//! Central DevStack application log: ring buffer + rotating file + Tauri events.
//!
//! Development default: **no redaction** — full command lines, args, env, stdout/stderr,
//! and error chains. Redaction is opt-in via Developer settings (`redact_secrets`).

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

const RING_CAP: usize = 5000;
const FILE_ROTATE_BYTES: u64 = 4 * 1024 * 1024; // 4 MiB

/// Optional live emit sink (wired from Tauri setup — keeps applog free of UI crates).
type EmitFn = Box<dyn Fn(&LogEntry) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "warn" | "warning" => LogLevel::Warn,
            "error" | "err" => LogLevel::Error,
            _ => LogLevel::Info,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: u64,
    pub ts: String,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

struct AppLogInner {
    ring: VecDeque<LogEntry>,
    next_id: u64,
    error_count: u64,
    file: Option<File>,
    file_path: Option<PathBuf>,
    emit: Option<EmitFn>,
}

pub struct AppLog {
    inner: Mutex<AppLogInner>,
}

static APPLOG: OnceLock<AppLog> = OnceLock::new();
static REDACT: AtomicBool = AtomicBool::new(false); // default OFF for development
static TAIL_SEQ: AtomicU64 = AtomicU64::new(1);

fn global() -> &'static AppLog {
    APPLOG.get_or_init(|| AppLog {
        inner: Mutex::new(AppLogInner {
            ring: VecDeque::with_capacity(RING_CAP),
            next_id: 1,
            error_count: 0,
            file: None,
            file_path: None,
            emit: None,
        }),
    })
}

/// Sync redaction flag from persisted Developer settings.
pub fn set_redact_secrets(enabled: bool) {
    REDACT.store(enabled, Ordering::Relaxed);
}

pub fn redact_enabled() -> bool {
    REDACT.load(Ordering::Relaxed)
}

/// Initialize file sink under `{data}/logs/devstack.log`.
pub fn init(log_dir: &Path) {
    let _ = fs::create_dir_all(log_dir);
    let path = log_dir.join("devstack.log");
    rotate_if_needed(&path);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    if let Ok(mut g) = global().inner.lock() {
        g.file = file;
        g.file_path = Some(path);
    }
    info(
        "app",
        format!(
            "DevStack logger ready (redact_secrets={})",
            redact_enabled()
        ),
    );
}

/// Attach a live emitter (typically Tauri `app.emit("app-log", entry)`).
pub fn attach_emitter<F>(f: F)
where
    F: Fn(&LogEntry) + Send + Sync + 'static,
{
    if let Ok(mut g) = global().inner.lock() {
        g.emit = Some(Box::new(f));
    }
}

pub fn log_dir_path() -> Option<PathBuf> {
    global()
        .inner
        .lock()
        .ok()
        .and_then(|g| {
            g.file_path
                .as_ref()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        })
}

fn rotate_if_needed(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() < FILE_ROTATE_BYTES {
        return;
    }
    let rotated = path.with_extension("log.1");
    let _ = fs::remove_file(&rotated);
    let _ = fs::rename(path, &rotated);
}

/// Redact passwords/tokens when Developer → Redact secrets is ON.
/// When OFF (development default), returns the input unchanged.
pub fn maybe_redact(input: &str) -> String {
    if !redact_enabled() {
        return input.to_string();
    }
    redact(input)
}

/// Always-available redaction transform (used by tests and when flag is on).
pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    let patterns = [
        r#"(?i)(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|authorization|auth|bearer|private[_-]?key)\s*[=:]\s*["']?([^\s"'&,;]+)"#,
        r#"(?i)(password|passwd|pwd|secret|token)=([^\s&]+)"#,
        r#"(?i)(Bearer\s+)([A-Za-z0-9\-._~+/]+=*)"#,
        r#"(?i)(mysql://[^:]+:)([^@]+)(@"#,
        r#"(?i)(postgres(?:ql)?://[^:]+:)([^@]+)(@"#,
    ];
    for pat in patterns {
        if let Ok(re) = regex::Regex::new(pat) {
            out = re
                .replace_all(&out, |caps: &regex::Captures| {
                    let head = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    format!("{head}***")
                })
                .into_owned();
        }
    }
    out
}

fn push(level: LogLevel, source: &str, message: impl AsRef<str>) {
    let message = maybe_redact(message.as_ref());
    let source = source.to_string();
    let ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let entry = {
        let Ok(mut g) = global().inner.lock() else {
            return;
        };
        let id = g.next_id;
        g.next_id += 1;
        if level == LogLevel::Error {
            g.error_count = g.error_count.saturating_add(1);
        }
        let entry = LogEntry {
            id,
            ts,
            level,
            source,
            message,
        };
        if g.ring.len() >= RING_CAP {
            g.ring.pop_front();
        }
        g.ring.push_back(entry.clone());
        if let Some(file) = g.file.as_mut() {
            let line = format!(
                "{} [{}] [{}] {}\n",
                entry.ts,
                entry.level.as_str(),
                entry.source,
                entry.message
            );
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
        let path = g.file_path.clone();
        let need_rotate = path
            .as_ref()
            .and_then(|p| p.metadata().ok())
            .map(|m| m.len() >= FILE_ROTATE_BYTES)
            .unwrap_or(false);
        if need_rotate {
            g.file = None;
        }
        (entry, path, need_rotate)
    };
    let (entry, path, need_rotate) = entry;
    if need_rotate {
        if let Some(path) = path {
            rotate_if_needed(&path);
            if let Ok(mut g2) = global().inner.lock() {
                g2.file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .ok();
            }
        }
    }
    emit_entry(&entry);
}

fn emit_entry(entry: &LogEntry) {
    if let Ok(g) = global().inner.lock() {
        if let Some(emit) = g.emit.as_ref() {
            emit(entry);
        }
    }
}

pub fn info(source: &str, message: impl AsRef<str>) {
    push(LogLevel::Info, source, message);
}

pub fn warn(source: &str, message: impl AsRef<str>) {
    push(LogLevel::Warn, source, message);
}

pub fn error(source: &str, message: impl AsRef<str>) {
    push(LogLevel::Error, source, message);
}

pub fn command_call(name: &str, args_json: &str) {
    info("command", format!("→ {name} {args_json}"));
}

pub fn command_ok(name: &str, result_summary: &str) {
    info("command", format!("← {name} ok {result_summary}"));
}

pub fn command_err(name: &str, err: &str) {
    error("command", format!("← {name} error {err}"));
}

pub fn process_spawn(
    source: &str,
    program: &Path,
    args: &[&str],
    envs: &[(&str, &std::ffi::OsStr)],
    pid: u32,
) {
    let cmdline = format!(
        "{} {}",
        program.display(),
        args.iter().cloned().collect::<Vec<_>>().join(" ")
    );
    let env_s = if envs.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = envs
            .iter()
            .map(|(k, v)| format!("{k}={}", v.to_string_lossy()))
            .collect();
        format!(" env=[{}]", parts.join(", "))
    };
    info(
        source,
        format!("spawn pid={pid} cmdline=[{cmdline}]{env_s}"),
    );
}

pub fn process_exit(source: &str, pid: u32, code: Option<i32>, duration_ms: u64) {
    let code_s = code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into());
    info(
        source,
        format!("exit pid={pid} code={code_s} duration_ms={duration_ms}"),
    );
}

pub fn snapshot(limit: Option<usize>) -> Vec<LogEntry> {
    let Ok(g) = global().inner.lock() else {
        return Vec::new();
    };
    let n = limit.unwrap_or(RING_CAP).min(g.ring.len());
    g.ring
        .iter()
        .rev()
        .take(n)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn clear() {
    if let Ok(mut g) = global().inner.lock() {
        g.ring.clear();
        g.error_count = 0;
        if let Some(file) = g.file.as_mut() {
            let _ = file.write_all(b"--- cleared ---\n");
            let _ = file.flush();
        }
    }
    info("app", "log buffer cleared");
}

pub fn error_count() -> u64 {
    global()
        .inner
        .lock()
        .map(|g| g.error_count)
        .unwrap_or(0)
}

/// Reset the error badge counter without clearing the ring.
pub fn clear_error_count() {
    if let Ok(mut g) = global().inner.lock() {
        g.error_count = 0;
    }
}

/// Tail a service log file into the ring buffer (raw lines; redact only if enabled).
pub fn tail_file(path: PathBuf, source: &str) {
    let source = source.to_string();
    let id = TAIL_SEQ.fetch_add(1, Ordering::Relaxed);
    std::thread::Builder::new()
        .name(format!("applog-tail-{id}"))
        .spawn(move || {
            let mut offset = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let Ok(mut f) = File::open(&path) else {
                    continue;
                };
                let Ok(len) = f.seek(SeekFrom::End(0)) else {
                    continue;
                };
                if len < offset {
                    offset = 0;
                }
                if len == offset {
                    continue;
                }
                if f.seek(SeekFrom::Start(offset)).is_err() {
                    continue;
                }
                let mut chunk = Vec::new();
                if f.read_to_end(&mut chunk).is_err() {
                    continue;
                }
                offset = len;
                let text = String::from_utf8_lossy(&chunk);
                for line in text.lines() {
                    let t = line.trim_end();
                    if t.is_empty() {
                        continue;
                    }
                    let lower = t.to_ascii_lowercase();
                    if lower.contains("error")
                        || lower.contains("emerg")
                        || lower.contains("fatal")
                        || lower.contains("crit")
                        || lower.contains("panic")
                    {
                        error(&source, t);
                    } else if lower.contains("warn") {
                        warn(&source, t);
                    } else {
                        info(&source, t);
                    }
                }
            }
        })
        .ok();
}

pub fn source_from_log_path(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.contains("nginx") {
        "nginx"
    } else if name.contains("apache") || name.contains("httpd") {
        "apache"
    } else if name.contains("php") {
        "php"
    } else if name.contains("maria") || name.contains("mysql") || name.contains("postgres") || name.contains("mongo")
    {
        "db"
    } else if name.contains("mailpit") {
        "mailpit"
    } else if name.contains("redis") {
        "redis"
    } else {
        "app"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_helper_masks_secrets() {
        // Test pure `redact()` — avoids flaky shared AtomicBool across parallel tests.
        let s = redact("password=hunter2 token=abc123");
        assert!(!s.contains("hunter2"), "got {s}");
        assert!(!s.contains("abc123"), "got {s}");
        assert!(s.contains("***"), "got {s}");
    }

    #[test]
    fn ring_snapshot_and_error_count() {
        clear();
        clear_error_count();
        let before = error_count();
        info("app", "hello");
        error("app", "boom");
        assert!(error_count() > before);
        let snap = snapshot(Some(10));
        assert!(!snap.is_empty());
    }
}
