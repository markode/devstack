//! PHP `dump()` receiver: a tiny loopback HTTP server fed by an auto-prepended
//! helper (`{tools}/devstack_dump.php`) and an in-memory ring buffer for the UI.

use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::error::{DevStackError, Result};

pub const DEFAULT_DUMP_PORT: u16 = 9913;
const MAX_ENTRIES: usize = 500;
const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpEntry {
    pub id: u64,
    /// Unix epoch milliseconds.
    pub ts: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub payload: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

type Emitter = Box<dyn Fn(&DumpEntry) + Send + Sync>;

static BUFFER: Mutex<VecDeque<DumpEntry>> = Mutex::new(VecDeque::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static PORT: AtomicU16 = AtomicU16::new(0);
static EMITTER: OnceLock<Emitter> = OnceLock::new();

/// Register the callback used to forward each entry to the UI (`php-dump` event).
pub fn attach_emitter(f: impl Fn(&DumpEntry) + Send + Sync + 'static) {
    let _ = EMITTER.set(Box::new(f));
}

pub fn dump_port() -> Option<u16> {
    match PORT.load(Ordering::SeqCst) {
        0 => None,
        p => Some(p),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Parse the helper's JSON body. Non-JSON bodies are kept verbatim as a string payload.
pub fn parse_dump_body(body: &str) -> Result<DumpEntry> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(DevStackError::Message("empty dump body".into()));
    }
    let mut entry = DumpEntry {
        id: 0,
        ts: now_ms(),
        site: None,
        payload: serde_json::Value::Null,
        file: None,
        line: None,
    };
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Object(map)) if map.contains_key("payload") => {
            entry.payload = map.get("payload").cloned().unwrap_or_default();
            entry.site = map.get("site").and_then(|v| v.as_str()).map(String::from);
            entry.file = map.get("file").and_then(|v| v.as_str()).map(String::from);
            entry.line = map
                .get("line")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32);
            if let Some(ts) = map.get("ts").and_then(|v| v.as_u64()) {
                entry.ts = ts;
            }
        }
        Ok(other) => entry.payload = other,
        Err(_) => entry.payload = serde_json::Value::String(trimmed.to_string()),
    }
    Ok(entry)
}

pub fn push_entry(mut entry: DumpEntry) -> DumpEntry {
    entry.id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut buf) = BUFFER.lock() {
        buf.push_back(entry.clone());
        while buf.len() > MAX_ENTRIES {
            buf.pop_front();
        }
    }
    if let Some(emit) = EMITTER.get() {
        emit(&entry);
    }
    entry
}

pub fn list_dumps(limit: Option<usize>) -> Vec<DumpEntry> {
    let buf = BUFFER.lock().map(|b| b.clone()).unwrap_or_default();
    let skip = limit.map(|l| buf.len().saturating_sub(l)).unwrap_or(0);
    buf.into_iter().skip(skip).collect()
}

pub fn clear_dumps() {
    if let Ok(mut buf) = BUFFER.lock() {
        buf.clear();
    }
}

fn handle_connection(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut data = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut header_end = None;
    while header_end.is_none() && data.len() < MAX_BODY {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                data.extend_from_slice(&chunk[..n]);
                header_end = data.windows(4).position(|w| w == b"\r\n\r\n");
            }
        }
    }
    let Some(he) = header_end else {
        return;
    };
    let head = String::from_utf8_lossy(&data[..he]).to_string();
    let request_line = head.lines().next().unwrap_or("");
    if request_line.starts_with("GET ") {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        return;
    }
    let content_len = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0)
        .min(MAX_BODY);
    let mut body = data[he + 4..].to_vec();
    while body.len() < content_len {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(content_len);
    if let Ok(entry) = parse_dump_body(&String::from_utf8_lossy(&body)) {
        push_entry(entry);
    }
    let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
}

/// Bind 127.0.0.1:9913 (or the next free port) and serve in a background thread.
/// Idempotent — returns the already-bound port on repeat calls.
pub fn start_dump_server(paths: &Paths) -> Result<u16> {
    if let Some(p) = dump_port() {
        return Ok(p);
    }
    let mut listener = None;
    for port in DEFAULT_DUMP_PORT..DEFAULT_DUMP_PORT + 40 {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", port)) {
            listener = Some((l, port));
            break;
        }
    }
    let (listener, port) = listener
        .ok_or_else(|| DevStackError::Message("no free port for the dump receiver".into()))?;
    PORT.store(port, Ordering::SeqCst);
    write_php_helper(paths, port)?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handle_connection(stream));
        }
    });
    crate::applog::info("dump", format!("dump receiver on 127.0.0.1:{port}"));
    Ok(port)
}

pub fn php_helper_path(paths: &Paths) -> PathBuf {
    paths.tools.join("devstack_dump.php")
}

pub fn php_helper_source(port: u16) -> String {
    format!(
        r#"<?php
// Generated by DevStack — forwards dump() output to the DevStack Dumps panel.
if (!function_exists('devstack_dump')) {{
    function devstack_dump(...$vars) {{
        $bt = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 1);
        foreach ($vars as $v) {{
            $body = json_encode([
                'site' => $_SERVER['HTTP_HOST'] ?? (PHP_SAPI === 'cli' ? 'cli' : null),
                'payload' => print_r($v, true),
                'file' => $bt[0]['file'] ?? null,
                'line' => $bt[0]['line'] ?? null,
                'ts' => (int) (microtime(true) * 1000),
            ], JSON_PARTIAL_OUTPUT_ON_ERROR | JSON_INVALID_UTF8_SUBSTITUTE);
            $ctx = stream_context_create(['http' => [
                'method' => 'POST',
                'header' => "Content-Type: application/json\r\n",
                'content' => $body,
                'timeout' => 0.5,
                'ignore_errors' => true,
            ]]);
            @file_get_contents('http://127.0.0.1:{port}/dump', false, $ctx);
        }}
        return $vars[0] ?? null;
    }}
    if (!function_exists('dump')) {{
        function dump(...$vars) {{ return devstack_dump(...$vars); }}
    }}
}}
"#
    )
}

pub fn write_php_helper(paths: &Paths, port: u16) -> Result<PathBuf> {
    fs::create_dir_all(&paths.tools)?;
    let path = php_helper_path(paths);
    fs::write(&path, php_helper_source(port))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_helper_json_body() {
        let body = r#"{"site":"blog.test","payload":"Array\n(\n)","file":"/srv/a.php","line":12,"ts":1700000000000}"#;
        let e = parse_dump_body(body).unwrap();
        assert_eq!(e.site.as_deref(), Some("blog.test"));
        assert_eq!(e.line, Some(12));
        assert_eq!(e.file.as_deref(), Some("/srv/a.php"));
        assert_eq!(e.ts, 1_700_000_000_000);
        assert!(e.payload.as_str().unwrap().starts_with("Array"));
    }

    #[test]
    fn parses_raw_text_and_rejects_empty() {
        let e = parse_dump_body("hello").unwrap();
        assert_eq!(e.payload, serde_json::json!("hello"));
        assert!(parse_dump_body("  ").is_err());
        let e = parse_dump_body(r#"{"a":1}"#).unwrap();
        assert_eq!(e.payload, serde_json::json!({"a":1}));
    }

    #[test]
    fn ring_buffer_assigns_ids_and_caps() {
        clear_dumps();
        for i in 0..(MAX_ENTRIES + 5) {
            push_entry(parse_dump_body(&format!("n{i}")).unwrap());
        }
        let all = list_dumps(None);
        assert_eq!(all.len(), MAX_ENTRIES);
        assert!(all.windows(2).all(|w| w[0].id < w[1].id));
        assert_eq!(list_dumps(Some(3)).len(), 3);
        clear_dumps();
        assert!(list_dumps(None).is_empty());
    }

    #[test]
    fn helper_targets_port() {
        let src = php_helper_source(9914);
        assert!(src.contains("127.0.0.1:9914/dump"));
        assert!(src.contains("function dump"));
    }
}
