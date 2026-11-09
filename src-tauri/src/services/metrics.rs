//! Performance dashboard backend: per-service CPU/RAM (sysinfo), MySQL/MariaDB slow-query
//! log, and SPX profiler wiring (enabled only when the `spx` extension is installed).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::config::{AppState, Paths};
use crate::error::{DevStackError, Result};

// ---------------------------------------------------------------------------
// Process metrics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetric {
    pub id: String,
    pub label: String,
    pub pid: u32,
    pub running: bool,
    /// CPU percent of one core, summed over the process tree.
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub processes: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMetric {
    pub cpu_percent: f32,
    pub cpu_count: usize,
    pub total_memory_bytes: u64,
    pub used_memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsSnapshot {
    pub timestamp_ms: u64,
    pub system: SystemMetric,
    pub processes: Vec<ProcessMetric>,
    pub total_cpu_percent: f32,
    pub total_memory_bytes: u64,
}

/// `(service id, label, pid)` for every PID DevStack tracks.
pub fn managed_pids(state: &AppState) -> Vec<(String, String, u32)> {
    let r = &state.runtime;
    let mut v: Vec<(String, String, u32)> = Vec::new();
    let mut push = |id: &str, label: &str, pid: Option<u32>| {
        if let Some(p) = pid {
            v.push((id.into(), label.into(), p));
        }
    };
    push("nginx", "Nginx", r.nginx_pid);
    push("apache", "Apache", r.apache_pid);
    push("dns", "DNS", r.dns_pid);
    let engine = r.database_engine.clone().unwrap_or_else(|| "database".into());
    push(&engine, if engine == "mysql" { "MySQL" } else if engine == "mariadb" { "MariaDB" } else { "Database" }, r.database_pid);
    push("mongodb", "MongoDB", r.mongodb_pid);
    push("postgresql", "PostgreSQL", r.postgresql_pid);
    push("redis", "Redis", r.redis_pid);
    push("mailpit", "Mailpit", r.mailpit_pid);
    push("memcached", "Memcached", r.memcached_pid);
    push("meilisearch", "Meilisearch", r.meilisearch_pid);
    push("typesense", "Typesense", r.typesense_pid);
    push("minio", "MinIO", r.minio_pid);
    push("reverb", "Reverb", r.reverb_pid);
    let mut php: Vec<_> = r.php_pids.iter().collect();
    php.sort();
    for (ver, pid) in php {
        v.push((format!("php-{ver}"), format!("PHP {ver}"), *pid));
    }
    v
}

fn subtree(sys: &System, root: Pid) -> Vec<Pid> {
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, p) in sys.processes() {
        if let Some(parent) = p.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let mut out = vec![root];
    let mut seen: HashSet<Pid> = HashSet::from([root]);
    let mut i = 0;
    while i < out.len() {
        if let Some(kids) = children.get(&out[i]) {
            for k in kids {
                if seen.insert(*k) {
                    out.push(*k);
                }
            }
        }
        i += 1;
    }
    out
}

/// Sample CPU/RAM for all managed PIDs. Takes ~250 ms (two CPU samples).
pub fn snapshot(state: &AppState) -> MetricsSnapshot {
    let mut sys = System::new();
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    std::thread::sleep(Duration::from_millis(250));
    sys.refresh_cpu_usage();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut processes = Vec::new();
    for (id, label, pid) in managed_pids(state) {
        let root = Pid::from_u32(pid);
        if sys.process(root).is_none() {
            processes.push(ProcessMetric { id, label, pid, running: false, cpu_percent: 0.0, memory_bytes: 0, processes: 0 });
            continue;
        }
        let tree = subtree(&sys, root);
        let cpu: f32 = tree.iter().filter_map(|p| sys.process(*p)).map(|p| p.cpu_usage()).sum();
        let mem: u64 = tree.iter().filter_map(|p| sys.process(*p)).map(|p| p.memory()).sum();
        processes.push(ProcessMetric { id, label, pid, running: true, cpu_percent: cpu, memory_bytes: mem, processes: tree.len() });
    }
    let total_cpu = processes.iter().map(|p| p.cpu_percent).sum();
    let total_mem = processes.iter().map(|p| p.memory_bytes).sum();
    MetricsSnapshot {
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        system: SystemMetric {
            cpu_percent: sys.global_cpu_usage(),
            cpu_count: sys.cpus().len(),
            total_memory_bytes: sys.total_memory(),
            used_memory_bytes: sys.used_memory(),
        },
        processes,
        total_cpu_percent: total_cpu,
        total_memory_bytes: total_mem,
    }
}

// ---------------------------------------------------------------------------
// Slow query log
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlowQuery {
    pub time: Option<String>,
    pub user_host: Option<String>,
    pub query_time: f64,
    pub lock_time: f64,
    pub rows_sent: u64,
    pub rows_examined: u64,
    pub sql: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlowQueryInfo {
    pub engine: Option<String>,
    pub running: bool,
    pub enabled: bool,
    pub threshold_secs: f64,
    pub path: String,
    pub exists: bool,
    pub entries: Vec<SlowQuery>,
    pub notes: Vec<String>,
}

pub fn slow_log_path(paths: &Paths, engine: &str) -> PathBuf {
    paths.logs.join(format!("{engine}-slow.log"))
}

/// Parse the MySQL/MariaDB slow query log text format.
pub fn parse_slow_log(raw: &str) -> Vec<SlowQuery> {
    let mut out: Vec<SlowQuery> = Vec::new();
    let mut cur: Option<SlowQuery> = None;
    let mut have_stats = false;
    let flush = |cur: &mut Option<SlowQuery>, out: &mut Vec<SlowQuery>| {
        if let Some(mut q) = cur.take() {
            q.sql = q.sql.trim().to_string();
            if !q.sql.is_empty() {
                out.push(q);
            }
        }
    };
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("# Time:") {
            flush(&mut cur, &mut out);
            cur = Some(SlowQuery {
                time: Some(rest.trim().to_string()),
                user_host: None,
                query_time: 0.0,
                lock_time: 0.0,
                rows_sent: 0,
                rows_examined: 0,
                sql: String::new(),
            });
            have_stats = false;
        } else if let Some(rest) = line.strip_prefix("# User@Host:") {
            if cur.is_none() || have_stats {
                flush(&mut cur, &mut out);
                cur = Some(SlowQuery { time: None, user_host: None, query_time: 0.0, lock_time: 0.0, rows_sent: 0, rows_examined: 0, sql: String::new() });
            }
            if let Some(q) = cur.as_mut() {
                q.user_host = Some(rest.trim().to_string());
            }
            have_stats = false;
        } else if let Some(rest) = line.strip_prefix("# Query_time:") {
            if let Some(q) = cur.as_mut() {
                let toks: Vec<&str> = rest.split_whitespace().collect();
                q.query_time = toks.first().and_then(|t| t.parse().ok()).unwrap_or(0.0);
                for w in toks.windows(2) {
                    match w[0] {
                        "Lock_time:" => q.lock_time = w[1].parse().unwrap_or(0.0),
                        "Rows_sent:" => q.rows_sent = w[1].parse().unwrap_or(0),
                        "Rows_examined:" => q.rows_examined = w[1].parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            have_stats = true;
        } else if line.starts_with('#') || line.starts_with("SET timestamp") || line.starts_with("use ") {
            continue;
        } else if let Some(q) = cur.as_mut() {
            if have_stats {
                q.sql.push_str(line);
                q.sql.push('\n');
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn db_sql(paths: &Paths, state: &AppState, engine: &str, sql: &str) -> Result<String> {
    let root = crate::services::database::engine_install_dir(state, engine)?;
    let client = crate::services::database::client_bin_for(&root)?;
    let mut args = crate::services::database::mysql_root_args(paths, state, engine);
    args.extend(["-N".into(), "-B".into(), "-e".into(), sql.into()]);
    let out = crate::services::process::command(client).args(args).output()?;
    if !out.status.success() {
        return Err(DevStackError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn active_sql_engine(state: &AppState) -> Option<String> {
    state
        .runtime
        .database_engine
        .clone()
        .filter(|e| matches!(e.as_str(), "mysql" | "mariadb"))
        .or_else(|| state.active_database.clone().filter(|e| matches!(e.as_str(), "mysql" | "mariadb")))
}

pub fn slow_query_info(paths: &Paths, state: &AppState, limit: usize) -> SlowQueryInfo {
    let engine = active_sql_engine(state);
    let running = state.runtime.database_pid.map(crate::services::process::is_running).unwrap_or(false);
    let path = slow_log_path(paths, engine.as_deref().unwrap_or("mysql"));
    let mut notes = Vec::new();
    let (mut enabled, mut threshold) = (false, 10.0);
    if let (Some(e), true) = (engine.as_deref(), running) {
        match db_sql(paths, state, e, "SELECT @@slow_query_log, @@long_query_time, @@slow_query_log_file") {
            Ok(row) => {
                let p: Vec<&str> = row.trim().split('\t').collect();
                enabled = p.first().map(|v| *v == "1").unwrap_or(false);
                threshold = p.get(1).and_then(|v| v.parse().ok()).unwrap_or(10.0);
            }
            Err(err) => notes.push(format!("could not read slow-log settings: {err}")),
        }
    } else {
        notes.push("Start the database to toggle the slow query log.".into());
    }
    let exists = path.exists();
    let mut entries = if exists {
        fs::read_to_string(&path).map(|r| parse_slow_log(&r)).unwrap_or_default()
    } else {
        vec![]
    };
    let n = entries.len();
    if n > limit {
        entries = entries.split_off(n - limit);
    }
    entries.reverse();
    SlowQueryInfo { engine, running, enabled, threshold_secs: threshold, path: path.display().to_string(), exists, entries, notes }
}

pub fn set_slow_query_log(paths: &Paths, state: &AppState, enabled: bool, threshold_secs: f64) -> Result<SlowQueryInfo> {
    let engine = active_sql_engine(state).ok_or_else(|| DevStackError::Message("no MySQL/MariaDB engine selected".into()))?;
    if !state.runtime.database_pid.map(crate::services::process::is_running).unwrap_or(false) {
        return Err(DevStackError::Message("start the database first".into()));
    }
    let t = threshold_secs.clamp(0.0, 3600.0);
    let path = slow_log_path(paths, &engine);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(&path, "")?;
    }
    let sql = format!(
        "SET GLOBAL slow_query_log_file='{}'; SET GLOBAL long_query_time={t}; SET GLOBAL slow_query_log={};",
        path.display().to_string().replace('\\', "/").replace('\'', ""),
        if enabled { 1 } else { 0 }
    );
    db_sql(paths, state, &engine, &sql)?;
    Ok(slow_query_info(paths, state, 100))
}

// ---------------------------------------------------------------------------
// SPX profiler
// ---------------------------------------------------------------------------

pub const SPX_BEGIN: &str = "; >>> devstack spx >>>";
pub const SPX_END: &str = "; <<< devstack spx <<<";
pub const SPX_KEY: &str = "dev";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpxStatus {
    pub php_version: Option<String>,
    /// The `spx` extension binary exists for this PHP.
    pub available: bool,
    pub enabled: bool,
    pub report_dir: String,
    /// Open on any site: `{site url}/?SPX_KEY=dev&SPX_UI_URI=/`.
    pub ui_query: String,
    pub notes: Vec<String>,
}

pub fn spx_report_dir(paths: &Paths) -> PathBuf {
    paths.root.join("spx-reports")
}

pub fn spx_block(paths: &Paths, windows: bool) -> String {
    let ext = if windows { "php_spx.dll" } else { "spx.so" };
    let dir = spx_report_dir(paths).display().to_string().replace('\\', "/");
    format!(
        "{SPX_BEGIN}\nextension={ext}\nspx.http_enabled=1\nspx.http_key=\"{SPX_KEY}\"\nspx.http_ip_whitelist=\"127.0.0.1\"\nspx.data_dir=\"{dir}\"\n{SPX_END}\n"
    )
}

pub fn upsert_spx_block(ini: &str, block: Option<&str>) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in ini.lines() {
        if line.trim() == SPX_BEGIN {
            skipping = true;
            continue;
        }
        if line.trim() == SPX_END {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    if let Some(b) = block {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(b);
    }
    out
}

fn php_install_for(paths: &Paths, state: &AppState, version: &str) -> Option<PathBuf> {
    let rec = crate::config::find_installed(state, "php", version)?;
    Some(if rec.path.exists() { rec.path.clone() } else { paths.package_dir("php", &rec.version) })
}

fn spx_extension_available(install: &Path) -> bool {
    let Some(php) = crate::services::process::find_bin(install, &["php", "php.exe"]) else {
        return false;
    };
    let mut cmd = crate::services::process::command(php);
    crate::php::apply_php_ini_to_command(&mut cmd, install);
    let Ok(out) = cmd.args(["-r", "echo ini_get('extension_dir');"]).output() else {
        return false;
    };
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim().to_string());
    ["spx.so", "php_spx.dll", "spx.dll"].iter().any(|n| dir.join(n).exists())
        || install.join("ext/php_spx.dll").exists()
}

fn default_php_version(state: &AppState) -> Option<String> {
    state
        .stack
        .php_version
        .clone()
        .or_else(|| state.installed.get("php").and_then(|v| v.first()).map(|r| r.version.clone()))
}

pub fn spx_status(paths: &Paths, state: &AppState, version: Option<&str>) -> SpxStatus {
    let ver = version.map(|v| v.to_string()).or_else(|| default_php_version(state));
    let mut notes = Vec::new();
    let install = ver.as_deref().and_then(|v| php_install_for(paths, state, v));
    let available = install.as_deref().map(spx_extension_available).unwrap_or(false);
    let enabled = state.php_extensions.get(ver.as_deref().unwrap_or("")).map(|s| s.spx).unwrap_or(false);
    if ver.is_none() {
        notes.push("Install a PHP version first.".into());
    } else if !available {
        notes.push("php-spx extension not installed for this PHP. Install it (pecl install spx, or your distro's php-spx package) and SPX turns on from here.".into());
    }
    SpxStatus {
        php_version: ver,
        available,
        enabled,
        report_dir: spx_report_dir(paths).display().to_string(),
        ui_query: format!("?SPX_KEY={SPX_KEY}&SPX_UI_URI=/"),
        notes,
    }
}

fn apply_spx_to_ini(paths: &Paths, install: &Path, enable: bool) -> Result<()> {
    let ini = crate::packages::php::php_ini_path(install);
    let current = fs::read_to_string(&ini).unwrap_or_default();
    let block = spx_block(paths, cfg!(windows));
    let next = upsert_spx_block(&current, enable.then_some(block.as_str()));
    fs::write(&ini, next)?;
    Ok(())
}

pub fn set_spx(paths: &Paths, state: &mut AppState, version: Option<&str>, enable: bool) -> Result<SpxStatus> {
    let ver = version
        .map(|v| v.to_string())
        .or_else(|| default_php_version(state))
        .ok_or_else(|| DevStackError::Message("install a PHP version first".into()))?;
    let install = php_install_for(paths, state, &ver)
        .ok_or_else(|| DevStackError::VersionNotFound(ver.clone()))?;
    if enable && !spx_extension_available(&install) {
        return Err(DevStackError::Message(
            "php-spx extension not found for this PHP — install it first (pecl install spx / php-spx package).".into(),
        ));
    }
    fs::create_dir_all(spx_report_dir(paths))?;
    apply_spx_to_ini(paths, &install, enable)?;
    state.php_extensions.entry(ver.clone()).or_default().spx = enable;
    crate::config::save_state(&paths.state_file, state)?;
    Ok(spx_status(paths, state, Some(&ver)))
}

/// Re-add the SPX block after `regenerate_all_php_ini` rewrote the ini files.
pub fn reapply_spx(paths: &Paths, state: &AppState) {
    for (ver, settings) in &state.php_extensions {
        if !settings.spx {
            continue;
        }
        if let Some(install) = php_install_for(paths, state, ver) {
            if spx_extension_available(&install) {
                let _ = apply_spx_to_ini(paths, &install, true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_has_system_numbers_and_tracks_own_pid() {
        let mut state = AppState::default();
        let me = std::process::id();
        state.runtime.redis_pid = Some(me);
        state.runtime.nginx_pid = Some(u32::MAX - 7); // dead
        let snap = snapshot(&state);
        assert!(snap.system.total_memory_bytes > 0);
        assert!(snap.system.cpu_count > 0);
        assert!(snap.timestamp_ms > 0);
        let redis = snap.processes.iter().find(|p| p.id == "redis").unwrap();
        assert!(redis.running);
        assert!(redis.memory_bytes > 0);
        assert!(redis.processes >= 1);
        let nginx = snap.processes.iter().find(|p| p.id == "nginx").unwrap();
        assert!(!nginx.running);
        assert_eq!(nginx.memory_bytes, 0);
        assert!(snap.total_memory_bytes >= redis.memory_bytes);
    }

    #[test]
    fn managed_pids_cover_v030_services_and_php() {
        let mut state = AppState::default();
        state.runtime.memcached_pid = Some(1);
        state.runtime.meilisearch_pid = Some(2);
        state.runtime.typesense_pid = Some(3);
        state.runtime.minio_pid = Some(4);
        state.runtime.reverb_pid = Some(5);
        state.runtime.php_pids.insert("8.3.1".into(), 6);
        state.runtime.database_pid = Some(7);
        state.runtime.database_engine = Some("mariadb".into());
        let ids: Vec<String> = managed_pids(&state).into_iter().map(|(id, _, _)| id).collect();
        for want in ["memcached", "meilisearch", "typesense", "minio", "reverb", "php-8.3.1", "mariadb"] {
            assert!(ids.contains(&want.to_string()), "{want} in {ids:?}");
        }
    }

    #[test]
    fn slow_log_parser_reads_mysql_format() {
        let raw = "# Time: 240501 12:00:00\n# User@Host: root[root] @ localhost []\n# Query_time: 2.500000  Lock_time: 0.000100 Rows_sent: 1  Rows_examined: 50000\nSET timestamp=1714564800;\nSELECT * FROM wp_posts\nWHERE post_status = 'publish';\n# Time: 240501 12:00:09\n# User@Host: wp[wp] @ localhost []\n# Query_time: 1.100000  Lock_time: 0.000000 Rows_sent: 0  Rows_examined: 3\nUPDATE x SET a=1;\n";
        let q = parse_slow_log(raw);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].query_time, 2.5);
        assert_eq!(q[0].rows_examined, 50000);
        assert!(q[0].sql.starts_with("SELECT * FROM wp_posts"));
        assert!(q[0].sql.contains("post_status"));
        assert_eq!(q[1].rows_examined, 3);
        assert_eq!(q[1].user_host.as_deref(), Some("wp[wp] @ localhost []"));
        // MariaDB repeats User@Host without a Time line between entries.
        let maria = "# User@Host: a[a] @ localhost []\n# Query_time: 1.0  Lock_time: 0.0 Rows_sent: 1  Rows_examined: 1\nSELECT 1;\n# User@Host: b[b] @ localhost []\n# Query_time: 2.0  Lock_time: 0.0 Rows_sent: 1  Rows_examined: 2\nSELECT 2;\n";
        let m = parse_slow_log(maria);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].query_time, 2.0);
    }

    #[test]
    fn slow_info_without_db_is_graceful() {
        let dir = std::env::temp_dir().join(format!("devstack-metrics-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        fs::write(slow_log_path(&paths, "mysql"), "# Time: 1\n# User@Host: a[a] @ h []\n# Query_time: 3.0 Lock_time: 0 Rows_sent: 1 Rows_examined: 9\nSELECT 1;\n").unwrap();
        let info = slow_query_info(&paths, &AppState::default(), 10);
        assert!(!info.running && !info.enabled);
        assert!(info.exists);
        assert_eq!(info.entries.len(), 1);
        assert!(!info.notes.is_empty());
        assert!(set_slow_query_log(&paths, &AppState::default(), true, 1.0).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn spx_block_upsert_is_idempotent_and_removable() {
        let dir = std::env::temp_dir().join(format!("devstack-spx-{}", std::process::id()));
        let paths = Paths::from_root(dir.clone()).unwrap();
        let base = "[PHP]\nmemory_limit=-1\n";
        let block = spx_block(&paths, false);
        assert!(block.contains("extension=spx.so"));
        assert!(block.contains("spx.http_key=\"dev\""));
        let on = upsert_spx_block(base, Some(&block));
        assert_eq!(on.matches(SPX_BEGIN).count(), 1);
        let again = upsert_spx_block(&on, Some(&block));
        assert_eq!(again.matches(SPX_BEGIN).count(), 1);
        let off = upsert_spx_block(&on, None);
        assert!(!off.contains("spx"));
        assert!(off.contains("memory_limit=-1"));
        assert!(spx_block(&paths, true).contains("php_spx.dll"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn spx_status_without_php_explains_next_step() {
        let dir = std::env::temp_dir().join(format!("devstack-spx2-{}", std::process::id()));
        let paths = Paths::from_root(dir.clone()).unwrap();
        let mut state = AppState::default();
        let st = spx_status(&paths, &state, None);
        assert!(!st.available && !st.enabled);
        assert!(st.notes[0].contains("PHP"));
        assert!(st.ui_query.contains("SPX_KEY=dev"));
        assert!(set_spx(&paths, &mut state, None, true).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
