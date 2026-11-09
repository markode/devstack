//! Per-site logs (PHP / web server / Laravel) with level, text and time filters,
//! byte-cursor live polling, plus Laravel queue + Horizon status.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::config::{AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};

const TAIL_BYTES: u64 = 512 * 1024;
const MAX_ENTRIES: usize = 2000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSourceInfo {
    /// `php` | `web-error` | `web-access` | `laravel`
    pub id: String,
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiteLogEntry {
    pub ts: Option<String>,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteLogPage {
    pub source: String,
    pub path: String,
    pub entries: Vec<SiteLogEntry>,
    /// Byte offset to pass back for the next poll.
    pub cursor: u64,
    pub size: u64,
    /// True when the file was rotated/truncated since `cursor`.
    pub reset: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SiteLogQuery {
    pub source: String,
    pub lines: Option<usize>,
    /// Minimum severity: debug < info < notice < warning < error < critical.
    pub level: Option<String>,
    /// Case-insensitive substring.
    pub text: Option<String>,
    /// `YYYY-MM-DD HH:MM:SS` / ISO-8601 lower bound, or relative like `15m`, `2h`, `1d`.
    pub since: Option<String>,
    /// Byte cursor from a previous page (live poll).
    pub cursor: Option<u64>,
}

pub fn severity(level: &str) -> u8 {
    match level.to_ascii_lowercase().as_str() {
        "debug" => 0,
        "info" => 1,
        "notice" | "deprecated" => 2,
        "warning" | "warn" => 3,
        "error" | "err" => 4,
        "critical" | "alert" | "emergency" | "fatal" | "crit" | "emerg" => 5,
        _ => 1,
    }
}

fn norm_level(level: &str) -> String {
    match severity(level) {
        0 => "debug",
        1 => "info",
        2 => "notice",
        3 => "warning",
        4 => "error",
        _ => "critical",
    }
    .into()
}

fn project_root(site: &SiteRecord) -> PathBuf {
    crate::services::sites::project_root_for_docroot(&crate::paths_util::expand_home(&site.path))
}

fn latest_laravel_log(site: &SiteRecord) -> PathBuf {
    let dir = project_root(site).join("storage/logs");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("log") {
                continue;
            }
            let m = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().map(|(t, _)| m > *t).unwrap_or(true) {
                best = Some((m, p));
            }
        }
    }
    best.map(|(_, p)| p).unwrap_or_else(|| dir.join("laravel.log"))
}

pub fn source_path(paths: &Paths, state: &AppState, site: &SiteRecord, source: &str) -> Result<PathBuf> {
    let apache = crate::services::httpd::site_uses_apache(site);
    Ok(match source {
        "php" => {
            let docroot = crate::paths_util::expand_home(&site.path);
            let local = docroot.join("php_errors.log");
            if local.exists() {
                local
            } else {
                let ver = site
                    .php_version
                    .clone()
                    .or_else(|| state.stack.php_version.clone())
                    .or_else(|| state.installed.get("php").and_then(|v| v.first()).map(|r| r.version.clone()))
                    .unwrap_or_default();
                paths.logs.join(format!("php-fpm-{ver}.log"))
            }
        }
        "web-error" => {
            if apache {
                paths.logs.join(format!("apache/{}-ssl-error.log", site.id))
            } else {
                paths.logs.join(format!("nginx/{}-error.log", site.id))
            }
        }
        "web-access" => {
            if apache {
                paths.logs.join(format!("apache/{}-ssl-access.log", site.id))
            } else {
                paths.logs.join(format!("nginx/{}-access.log", site.id))
            }
        }
        "laravel" => latest_laravel_log(site),
        other => return Err(DevStackError::Message(format!("unknown log source: {other}"))),
    })
}

fn find_site<'a>(state: &'a AppState, id: &str) -> Result<&'a SiteRecord> {
    state
        .sites
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| DevStackError::Message(format!("site not found: {id}")))
}

pub fn list_sources(paths: &Paths, state: &AppState, site_id: &str) -> Result<Vec<LogSourceInfo>> {
    let site = find_site(state, site_id)?;
    let (framework, _) = crate::services::sites::detect_framework(&site.path);
    let mut ids = vec![("php", "PHP errors"), ("web-error", "Web server errors"), ("web-access", "Access log")];
    if matches!(framework.as_str(), "laravel" | "statamic") {
        ids.push(("laravel", "Laravel (storage/logs)"));
    }
    ids.into_iter()
        .map(|(id, label)| {
            let p = source_path(paths, state, site, id)?;
            let meta = fs::metadata(&p).ok();
            Ok(LogSourceInfo {
                id: id.into(),
                label: label.into(),
                exists: meta.is_some(),
                size: meta.map(|m| m.len()).unwrap_or(0),
                path: p.display().to_string(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_ts(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    for fmt in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%d-%b-%Y %H:%M:%S",
        "%d/%b/%Y:%H:%M:%S",
        "%a %b %d %H:%M:%S%.f %Y",
        "%a %b %d %H:%M:%S %Y",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(t);
        }
    }
    // Trim trailing zone / fractional garbage and retry.
    let cut: String = s.chars().take(19).collect();
    NaiveDateTime::parse_from_str(&cut, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(&cut, "%Y-%m-%d %H:%M:%S"))
        .ok()
}

pub fn parse_since(spec: &str, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let spec = spec.trim();
    if spec.len() >= 2 {
        let (num, unit) = spec.split_at(spec.len() - 1);
        if let Ok(n) = num.parse::<i64>() {
            let secs = match unit {
                "s" => n,
                "m" => n * 60,
                "h" => n * 3600,
                "d" => n * 86400,
                _ => 0,
            };
            if secs > 0 {
                return Some(now - chrono::Duration::seconds(secs));
            }
        }
    }
    parse_ts(spec)
}

/// One physical log line → start of a new entry? (continuation lines return `None`).
fn parse_header(source: &str, line: &str) -> Option<SiteLogEntry> {
    match source {
        "laravel" => {
            // [2024-05-01 12:00:00] local.ERROR: message
            let rest = line.strip_prefix('[')?;
            let (ts, rest) = rest.split_once("] ")?;
            parse_ts(ts)?;
            let (chan_level, msg) = rest.split_once(": ")?;
            let level = chan_level.rsplit('.').next().unwrap_or("info");
            Some(SiteLogEntry { ts: Some(ts.chars().take(19).collect()), level: norm_level(level), message: msg.to_string() })
        }
        "php" => {
            // [01-May-2024 12:00:00 UTC] PHP Fatal error:  message
            let rest = line.strip_prefix('[')?;
            let (stamp, msg) = rest.split_once("] ")?;
            let ts: String = stamp.split(' ').take(2).collect::<Vec<_>>().join(" ");
            let ts_parsed = parse_ts(&ts)?;
            let m = msg.strip_prefix("PHP ").unwrap_or(msg);
            let lower = m.to_ascii_lowercase();
            let level = if lower.starts_with("fatal") || lower.starts_with("parse") {
                "critical"
            } else if lower.starts_with("warning") {
                "warning"
            } else if lower.starts_with("notice") || lower.starts_with("deprecated") {
                "notice"
            } else {
                "error"
            };
            Some(SiteLogEntry { ts: Some(ts_parsed.format("%Y-%m-%d %H:%M:%S").to_string()), level: level.into(), message: m.to_string() })
        }
        "web-access" => {
            // 127.0.0.1 - - [01/May/2024:12:00:00 +0000] "GET / HTTP/1.1" 200 12
            let l = line.find('[')?;
            let r = line[l..].find(']')? + l;
            let ts = parse_ts(&line[l + 1..r].split(' ').next().unwrap_or(""))?;
            let status: u16 = line[r..]
                .split('"')
                .nth(2)
                .and_then(|t| t.split_whitespace().next())
                .and_then(|t| t.parse().ok())
                .unwrap_or(200);
            let level = if status >= 500 {
                "error"
            } else if status >= 400 {
                "warning"
            } else {
                "info"
            };
            Some(SiteLogEntry { ts: Some(ts.format("%Y-%m-%d %H:%M:%S").to_string()), level: level.into(), message: line.to_string() })
        }
        _ => {
            // nginx: 2024/05/01 12:00:00 [error] 123#0: msg ; apache: [Wed May 01 12:00:00.123456 2024] [core:error] ...
            if let Some((ts, rest)) = line.split_once(" [") {
                if let Some(t) = parse_ts(ts) {
                    let (lvl, msg) = rest.split_once("] ")?;
                    return Some(SiteLogEntry { ts: Some(t.format("%Y-%m-%d %H:%M:%S").to_string()), level: norm_level(lvl), message: msg.to_string() });
                }
            }
            let rest = line.strip_prefix('[')?;
            let (stamp, rest) = rest.split_once("] ")?;
            let t = parse_ts(stamp)?;
            let level = rest
                .strip_prefix('[')
                .and_then(|r| r.split_once(']'))
                .map(|(m, _)| m.rsplit(':').next().unwrap_or("error").to_string())
                .unwrap_or_else(|| "error".into());
            Some(SiteLogEntry { ts: Some(t.format("%Y-%m-%d %H:%M:%S").to_string()), level: norm_level(&level), message: rest.to_string() })
        }
    }
}

/// Group physical lines into entries (stack traces fold into the preceding entry).
pub fn parse_entries(source: &str, raw: &str) -> Vec<SiteLogEntry> {
    let mut out: Vec<SiteLogEntry> = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_header(source, line) {
            Some(e) => out.push(e),
            None => match out.last_mut() {
                Some(last) if source != "web-access" => {
                    last.message.push('\n');
                    last.message.push_str(line);
                }
                _ => out.push(SiteLogEntry { ts: None, level: "info".into(), message: line.to_string() }),
            },
        }
    }
    out
}

pub fn filter_entries(
    entries: Vec<SiteLogEntry>,
    level: Option<&str>,
    text: Option<&str>,
    since: Option<NaiveDateTime>,
) -> Vec<SiteLogEntry> {
    let min = level.filter(|l| !l.is_empty() && *l != "all").map(severity);
    let needle = text.map(|t| t.to_lowercase()).filter(|t| !t.is_empty());
    entries
        .into_iter()
        .filter(|e| min.map(|m| severity(&e.level) >= m).unwrap_or(true))
        .filter(|e| needle.as_ref().map(|n| e.message.to_lowercase().contains(n)).unwrap_or(true))
        .filter(|e| match (since, e.ts.as_deref().and_then(parse_ts)) {
            (Some(s), Some(t)) => t >= s,
            _ => true,
        })
        .collect()
}

/// Read from `from` to EOF (bounded), returning text and the new end offset.
fn read_range(path: &Path, from: u64) -> Result<(String, u64, u64, bool)> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    let mut start = from;
    let mut reset = false;
    if from > size {
        start = 0;
        reset = true;
    }
    if size - start > TAIL_BYTES {
        start = size - TAIL_BYTES;
    }
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((size - start) as usize);
    f.read_to_end(&mut buf)?;
    let mut text = String::from_utf8_lossy(&buf).to_string();
    if start > 0 && from == 0 || (start > 0 && start != from) {
        // Drop the partial first line when we started mid-file.
        if let Some(i) = text.find('\n') {
            text = text[i + 1..].to_string();
        }
    }
    Ok((text, size, start, reset))
}

pub fn read_logs(paths: &Paths, state: &AppState, site_id: &str, q: &SiteLogQuery) -> Result<SiteLogPage> {
    let site = find_site(state, site_id)?;
    let source = if q.source.is_empty() { "php" } else { q.source.as_str() };
    let path = source_path(paths, state, site, source)?;
    if !path.exists() {
        return Ok(SiteLogPage { source: source.into(), path: path.display().to_string(), entries: vec![], cursor: 0, size: 0, reset: false });
    }
    let (text, size, _start, reset) = read_range(&path, q.cursor.unwrap_or(0))?;
    let since = q.since.as_deref().and_then(|s| parse_since(s, chrono::Local::now().naive_local()));
    let mut entries = filter_entries(parse_entries(source, &text), q.level.as_deref(), q.text.as_deref(), since);
    let want = q.lines.unwrap_or(200).clamp(1, MAX_ENTRIES);
    if entries.len() > want {
        entries = entries.split_off(entries.len() - want);
    }
    Ok(SiteLogPage { source: source.into(), path: path.display().to_string(), entries, cursor: size, size, reset })
}

// ---------------------------------------------------------------------------
// Laravel queue / Horizon
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailedJob {
    pub id: String,
    pub queue: String,
    pub failed_at: String,
    pub exception: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonInfo {
    pub installed: bool,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatus {
    pub is_laravel: bool,
    pub driver: Option<String>,
    pub pending: Option<u64>,
    pub failed: Option<u64>,
    pub failed_jobs: Vec<FailedJob>,
    pub horizon: HorizonInfo,
    pub notes: Vec<String>,
}

pub fn parse_env(raw: &str) -> HashMap<String, String> {
    raw.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().trim_matches(['"', '\'']).to_string()))
        })
        .collect()
}

pub fn parse_failed_jobs(tsv: &str) -> Vec<FailedJob> {
    tsv.lines()
        .filter_map(|l| {
            let mut p = l.splitn(4, '\t');
            Some(FailedJob {
                id: p.next()?.to_string(),
                queue: p.next()?.to_string(),
                failed_at: p.next()?.to_string(),
                exception: p.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

fn sql_query(paths: &Paths, state: &AppState, engine: &str, db: &str, sql: &str) -> Result<String> {
    let root = crate::services::database::engine_install_dir(state, engine)?;
    let client = crate::services::database::client_bin_for(&root)?;
    let mut args = crate::services::database::mysql_root_args(paths, state, engine);
    args.push(format!("--database={db}"));
    args.extend(["-N".into(), "-B".into(), "-e".into(), sql.to_string()]);
    let out = crate::services::process::command(client).args(args).output()?;
    if !out.status.success() {
        return Err(DevStackError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn horizon_info(paths_state: &AppState, site: &SiteRecord) -> HorizonInfo {
    let root = project_root(site);
    let installed = root.join("vendor/laravel/horizon").exists()
        || fs::read_to_string(root.join("composer.json")).map(|c| c.contains("laravel/horizon")).unwrap_or(false);
    let url = installed.then(|| {
        let apache = crate::services::httpd::site_uses_apache(site);
        let port = if apache { paths_state.stack.apache_https_port } else { paths_state.stack.https_port };
        crate::services::ports::format_url(if site.https { "https" } else { "http" }, &site.domain, if site.https { port } else if apache { paths_state.stack.apache_http_port } else { paths_state.stack.http_port }) + "/horizon"
    });
    HorizonInfo { installed, url }
}

pub fn queue_status(paths: &Paths, state: &AppState, site_id: &str) -> Result<QueueStatus> {
    let site = find_site(state, site_id)?;
    let (framework, _) = crate::services::sites::detect_framework(&site.path);
    let is_laravel = matches!(framework.as_str(), "laravel" | "statamic");
    let mut status = QueueStatus {
        is_laravel,
        driver: None,
        pending: None,
        failed: None,
        failed_jobs: vec![],
        horizon: horizon_info(state, site),
        notes: vec![],
    };
    if !is_laravel {
        status.notes.push("Queue panel is available for Laravel sites.".into());
        return Ok(status);
    }
    let root = project_root(site);
    let env = parse_env(&fs::read_to_string(root.join(".env")).unwrap_or_default());
    let driver = env.get("QUEUE_CONNECTION").cloned().unwrap_or_else(|| "sync".into());
    status.driver = Some(driver.clone());
    match driver.as_str() {
        "database" => {
            let engine = site.database_engine.clone().or_else(|| state.active_database.clone()).unwrap_or_else(|| "mariadb".into());
            let db = env.get("DB_DATABASE").cloned().or_else(|| site.database_name.clone());
            match db {
                None => status.notes.push("No DB_DATABASE configured.".into()),
                Some(db) => {
                    let run = |sql: &str| sql_query(paths, state, &engine, &db, sql);
                    match run("SELECT COUNT(*) FROM jobs") {
                        Ok(n) => status.pending = n.trim().parse().ok(),
                        Err(e) => status.notes.push(format!("jobs table unavailable: {e}")),
                    }
                    match run("SELECT COUNT(*) FROM failed_jobs") {
                        Ok(n) => status.failed = n.trim().parse().ok(),
                        Err(e) => status.notes.push(format!("failed_jobs table unavailable: {e}")),
                    }
                    if status.failed.unwrap_or(0) > 0 {
                        if let Ok(rows) = run("SELECT id, queue, failed_at, REPLACE(REPLACE(LEFT(exception,240),CHAR(10),' '),CHAR(9),' ') FROM failed_jobs ORDER BY id DESC LIMIT 20") {
                            status.failed_jobs = parse_failed_jobs(&rows);
                        }
                    }
                }
            }
        }
        "redis" => {
            if let Some(rec) = crate::config::latest_installed(state, "redis") {
                if let Some(cli) = crate::services::process::find_bin(&rec.path, &["redis-cli", "redis-cli.exe"]) {
                    let out = crate::services::process::command(cli)
                        .args(["-p", &state.stack.redis_port.to_string(), "LLEN", "queues:default"])
                        .output();
                    if let Ok(o) = out {
                        status.pending = String::from_utf8_lossy(&o.stdout).trim().parse().ok();
                    }
                }
            }
            if status.pending.is_none() {
                status.notes.push("Redis queue depth unavailable (start Redis).".into());
            }
        }
        other => status.notes.push(format!("Queue driver `{other}` has no inspectable backlog.")),
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn laravel_entries_fold_stack_traces() {
        let raw = "[2024-05-01 12:00:00] local.ERROR: Boom {\"a\":1}\n#0 /app/x.php(1): foo()\n#1 {main}\n[2024-05-01 12:00:05] local.INFO: ok\n";
        let e = parse_entries("laravel", raw);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].level, "error");
        assert!(e[0].message.contains("#1 {main}"));
        assert_eq!(e[1].level, "info");
        assert_eq!(e[1].ts.as_deref(), Some("2024-05-01 12:00:05"));
    }

    #[test]
    fn php_nginx_apache_and_access_parsing() {
        let php = parse_entries("php", "[01-May-2024 12:00:00 UTC] PHP Fatal error:  Uncaught x in /a.php:3\n[01-May-2024 12:00:01 UTC] PHP Warning:  undefined var\n[01-May-2024 12:00:02 UTC] PHP Deprecated:  old\n");
        assert_eq!(php.iter().map(|e| e.level.as_str()).collect::<Vec<_>>(), vec!["critical", "warning", "notice"]);
        let ng = parse_entries("web-error", "2024/05/01 12:00:00 [error] 12#0: *1 open() failed\n2024/05/01 12:00:01 [warn] 12#0: x\n");
        assert_eq!(ng[0].level, "error");
        assert_eq!(ng[1].level, "warning");
        let ap = parse_entries("web-error", "[Wed May 01 12:00:00.123456 2024] [core:error] [pid 1] AH00124: loop\n");
        assert_eq!(ap[0].level, "error");
        assert_eq!(ap[0].ts.as_deref(), Some("2024-05-01 12:00:00"));
        let acc = parse_entries("web-access", "127.0.0.1 - - [01/May/2024:12:00:00 +0000] \"GET /x HTTP/1.1\" 500 12\n127.0.0.1 - - [01/May/2024:12:00:01 +0000] \"GET / HTTP/1.1\" 404 1\n127.0.0.1 - - [01/May/2024:12:00:02 +0000] \"GET / HTTP/1.1\" 200 1\n");
        assert_eq!(acc.iter().map(|e| e.level.as_str()).collect::<Vec<_>>(), vec!["error", "warning", "info"]);
    }

    #[test]
    fn filters_by_level_text_and_time() {
        let raw = "[2024-05-01 10:00:00] local.DEBUG: dbg\n[2024-05-01 11:00:00] local.WARNING: Slow query\n[2024-05-01 12:00:00] local.ERROR: Boom\n";
        let all = parse_entries("laravel", raw);
        assert_eq!(filter_entries(all.clone(), Some("warning"), None, None).len(), 2);
        assert_eq!(filter_entries(all.clone(), Some("error"), None, None).len(), 1);
        assert_eq!(filter_entries(all.clone(), None, Some("SLOW"), None).len(), 1);
        let since = parse_ts("2024-05-01 11:30:00");
        assert_eq!(filter_entries(all.clone(), None, None, since).len(), 1);
        assert_eq!(filter_entries(all, Some("all"), Some(""), None).len(), 3);
        let now = parse_ts("2024-05-01 12:00:00").unwrap();
        assert_eq!(parse_since("30m", now), parse_ts("2024-05-01 11:30:00"));
        assert_eq!(parse_since("2024-05-01T10:00:00Z", now), parse_ts("2024-05-01 10:00:00"));
    }

    fn site_in(dir: &Path, framework_files: bool) -> (Paths, AppState) {
        let paths = Paths::from_root(dir.join("data")).unwrap();
        let root = dir.join("app");
        fs::create_dir_all(root.join("public")).unwrap();
        fs::create_dir_all(root.join("storage/logs")).unwrap();
        if framework_files {
            fs::write(root.join("artisan"), "").unwrap();
            fs::write(root.join("composer.json"), "{\"require\":{\"laravel/horizon\":\"*\"}}").unwrap();
        }
        let mut state = AppState::default();
        let mut s = SiteRecord {
            id: "app".into(),
            name: "app".into(),
            path: root.join("public"),
            domain: String::new(),
            php_version: Some("8.3".into()),
            node_version: None,
            https: true,
            web_server: Some("nginx".into()),
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: String::new(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: vec![],
            wildcard: false,
        };
        s.ensure_domain();
        state.sites.push(s);
        (paths, state)
    }

    #[test]
    fn read_logs_tail_poll_cursor_and_rotation() {
        let dir = std::env::temp_dir().join(format!("devstack-sitelogs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (paths, state) = site_in(&dir, true);
        let log = dir.join("app/storage/logs/laravel.log");
        fs::write(&log, "[2024-05-01 12:00:00] local.ERROR: first\n").unwrap();
        let q = SiteLogQuery { source: "laravel".into(), ..Default::default() };
        let page = read_logs(&paths, &state, "app", &q).unwrap();
        assert_eq!(page.entries.len(), 1);
        let cursor = page.cursor;
        // append → poll returns only the new entry
        let mut body = fs::read_to_string(&log).unwrap();
        body.push_str("[2024-05-01 12:00:09] local.INFO: second\n");
        fs::write(&log, body).unwrap();
        let poll = read_logs(&paths, &state, "app", &SiteLogQuery { cursor: Some(cursor), ..q.clone() }).unwrap();
        assert_eq!(poll.entries.len(), 1);
        assert_eq!(poll.entries[0].message, "second");
        assert!(!poll.reset);
        // rotation: file shrinks below cursor
        fs::write(&log, "[2024-05-02 00:00:00] local.INFO: rotated\n").unwrap();
        let rot = read_logs(&paths, &state, "app", &SiteLogQuery { cursor: Some(poll.cursor), ..q.clone() }).unwrap();
        assert!(rot.reset);
        assert_eq!(rot.entries[0].message, "rotated");
        // missing source → empty page
        let missing = read_logs(&paths, &state, "app", &SiteLogQuery { source: "web-access".into(), ..Default::default() }).unwrap();
        assert!(missing.entries.is_empty());
        let sources = list_sources(&paths, &state, "app").unwrap();
        assert!(sources.iter().any(|s| s.id == "laravel" && s.exists));
        assert!(read_logs(&paths, &state, "app", &SiteLogQuery { source: "nope".into(), ..Default::default() }).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn queue_status_detects_horizon_and_non_laravel() {
        let dir = std::env::temp_dir().join(format!("devstack-queue-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (paths, state) = site_in(&dir, true);
        fs::write(dir.join("app/.env"), "QUEUE_CONNECTION=sync\nDB_DATABASE=x\n").unwrap();
        let q = queue_status(&paths, &state, "app").unwrap();
        assert!(q.is_laravel);
        assert_eq!(q.driver.as_deref(), Some("sync"));
        assert!(q.horizon.installed);
        assert!(q.horizon.url.as_deref().unwrap().ends_with("/horizon"));
        let _ = fs::remove_dir_all(&dir);
        let dir2 = std::env::temp_dir().join(format!("devstack-queue2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir2);
        let (paths2, state2) = site_in(&dir2, false);
        let q2 = queue_status(&paths2, &state2, "app").unwrap();
        assert!(!q2.is_laravel);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn env_and_failed_job_parsing() {
        let env = parse_env("A=1\n# c\nB=\"two\"\nC='x y'\n");
        assert_eq!(env["B"], "two");
        assert_eq!(env["C"], "x y");
        let jobs = parse_failed_jobs("7\tdefault\t2024-05-01 12:00:00\tIlluminate\\Exception: boom\n");
        assert_eq!(jobs[0].id, "7");
        assert_eq!(jobs[0].queue, "default");
        assert!(jobs[0].exception.contains("boom"));
    }
}
