//! PHP-FPM pools (macOS/Linux) and php-cgi worker manager (Windows).

use std::collections::HashSet;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::config::{find_installed, AppState, Paths};
use crate::error::{DevStackError, Result};
use crate::services::process::{
    find_bin, is_running, read_pid, run_capture, spawn_logged, stop_pid, write_pid,
};

fn supervisor_stops() -> &'static Mutex<HashSet<String>> {
    static STOPS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    STOPS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn mark_supervisor_stop(version: &str) {
    if let Ok(mut g) = supervisor_stops().lock() {
        g.insert(version.to_string());
    }
}

fn clear_supervisor_stop(version: &str) {
    if let Ok(mut g) = supervisor_stops().lock() {
        g.remove(version);
    }
}

fn should_supervisor_stop(version: &str) -> bool {
    supervisor_stops()
        .lock()
        .map(|g| g.contains(version))
        .unwrap_or(true)
}

#[derive(Debug, Clone)]
pub struct PhpRuntime {
    pub version: String,
    pub root: PathBuf,
    pub php_bin: PathBuf,
    pub fpm_bin: Option<PathBuf>,
    pub cgi_bin: Option<PathBuf>,
    pub listen: String,
    pub is_system: bool,
}

pub fn resolve_php(paths: &Paths, state: &AppState, version: &str) -> Result<PhpRuntime> {
    if let Some(rec) = find_installed(state, "php", version) {
        let root = rec.path.clone();
        if let Some(php_bin) = find_bin(&root, &["php", "php.exe"]) {
            let ver_mm = rec
                .version
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            let fpm_name = format!("php-fpm{ver_mm}");
            let cgi_name = format!("php-cgi{ver_mm}");
            let fpm_bin = find_bin(
                &root,
                &["php-fpm", "php-fpm.exe", fpm_name.as_str()],
            )
            .or_else(|| which::which(&fpm_name).ok())
            .or_else(|| which::which("php-fpm").ok());
            let cgi_bin = find_bin(
                &root,
                &["php-cgi", "php-cgi.exe", cgi_name.as_str()],
            )
            .or_else(|| which::which(&cgi_name).ok())
            .or_else(|| which::which("php-cgi").ok());
            let listen = fpm_listen_addr_with_base(paths, &rec.version, state.stack.php_fpm_port);
            // If FPM isn't inside the package tree, treat as system hybrid.
            let is_system = fpm_bin
                .as_ref()
                .map(|p| !p.starts_with(&root))
                .unwrap_or(false);
            return Ok(PhpRuntime {
                version: rec.version.clone(),
                root,
                php_bin,
                fpm_bin,
                cgi_bin,
                listen,
                is_system,
            });
        }
        // Installed record may point at a system prefix (e.g. /usr/sbin from php-fpm).
        if let Ok(mut runtime) = resolve_system_php(paths, state, &rec.version) {
            runtime.version = rec.version.clone();
            return Ok(runtime);
        }
    }

    // System PHP fallback (useful on Linux CI / smoke)
    resolve_system_php(paths, state, version)
}

fn resolve_system_php(paths: &Paths, state: &AppState, version: &str) -> Result<PhpRuntime> {
    let candidates = [
        format!("php-fpm{version}"),
        format!("php-fpm{}", version.split('.').take(2).collect::<Vec<_>>().join(".")),
        "php-fpm8.3".into(),
        "php-fpm8.2".into(),
        "php-fpm8.1".into(),
        "php-fpm".into(),
    ];
    let fpm = candidates
        .iter()
        .find_map(|c| which::which(c).ok())
        .ok_or_else(|| {
            DevStackError::Message(format!(
                "PHP {version} is not installed in DevStack and no system php-fpm was found"
            ))
        })?;
    let php = which::which("php8.3")
        .or_else(|_| which::which("php"))
        .unwrap_or_else(|_| PathBuf::from("php"));
    let ver = detect_php_version(&php).unwrap_or_else(|| version.to_string());
    let listen = fpm_listen_addr_with_base(paths, &ver, state.stack.php_fpm_port);
    let root = php
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(Path::new("/usr"))
        .to_path_buf();
    Ok(PhpRuntime {
        version: ver,
        root,
        php_bin: php,
        fpm_bin: Some(fpm),
        cgi_bin: which::which("php-cgi").or_else(|_| which::which("php-cgi8.3")).ok(),
        listen,
        is_system: true,
    })
}

fn detect_php_version(php: &Path) -> Option<String> {
    // Spawning `php -r` costs 20-80ms; the binary's version never changes within a
    // process lifetime, so memoise per path (hit on every resolve_php fallback).
    static CACHE: OnceLock<Mutex<std::collections::HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Some(v) = cache.lock().ok().and_then(|g| g.get(php).cloned()) {
        return Some(v);
    }
    let out = run_capture(php, &["-r", "echo PHP_MAJOR_VERSION.'.'.PHP_MINOR_VERSION.'.'.PHP_RELEASE_VERSION;"], None).ok()?;
    let ver = out.trim().to_string();
    if let Ok(mut g) = cache.lock() {
        g.insert(php.to_path_buf(), ver.clone());
    }
    Some(ver)
}

/// Preferred PHP version for new sites: stack preference, else first installed package.
pub fn default_php_version(state: &AppState) -> Option<String> {
    state
        .stack
        .php_version
        .clone()
        .or_else(|| {
            state.installed.get("php").and_then(|v| {
                v.first().map(|r| {
                    r.channel
                        .clone()
                        .filter(|c| !c.is_empty())
                        .unwrap_or_else(|| r.version.clone())
                })
            })
        })
}

pub fn fpm_listen_addr(paths: &Paths, version: &str) -> String {
    fpm_listen_addr_with_base(paths, version, 9000)
}

pub fn fpm_listen_addr_with_base(paths: &Paths, version: &str, base: u16) -> String {
    let sock = paths.php.join(format!("php-{version}.sock"));
    if cfg!(windows) {
        // TCP on Windows for php-cgi / fpm
        let port = base.saturating_add(version_port_offset(version));
        format!("127.0.0.1:{port}")
    } else {
        format!("unix:{}", sock.display())
    }
}

fn version_port_offset(version: &str) -> u16 {
    // Use major.minor only so "8.3" and "8.3.12" share the same php-cgi port.
    let parts: Vec<&str> = version.split('.').collect();
    let major = parts.first().and_then(|s| s.parse::<u16>().ok()).unwrap_or(8);
    let minor = parts.get(1).and_then(|s| s.parse::<u16>().ok()).unwrap_or(3);
    ((major * 10) + minor) % 100
}

pub fn ensure_fpm_pool(paths: &Paths, runtime: &PhpRuntime) -> Result<PathBuf> {
    let conf_dir = paths.php.join("fpm").join(&runtime.version);
    fs::create_dir_all(&conf_dir)?;
    let conf = conf_dir.join("php-fpm.conf");
    let pool = conf_dir.join("www.conf");
    let listen = if runtime.listen.starts_with("unix:") {
        runtime.listen.trim_start_matches("unix:").to_string()
    } else {
        runtime.listen.clone()
    };
    if let Some(parent) = Path::new(&listen).parent() {
        let _ = fs::create_dir_all(parent);
    }

    let conf_body = format!(
        "[global]\npid = {pid}\nerror_log = {err}\ninclude = {pool}\n",
        pid = paths.pid_file(&format!("php-fpm-{}", runtime.version)).display(),
        err = paths.logs.join(format!("php-fpm-{}.log", runtime.version)).display(),
        pool = pool.display()
    );
    fs::write(&conf, conf_body)?;

    let pool_body = format!(
        r#"[www]
user = {user}
group = {group}
listen = {listen}
listen.owner = {user}
listen.group = {group}
pm = dynamic
pm.max_children = 20
pm.start_servers = 2
pm.min_spare_servers = 1
pm.max_spare_servers = 5
clear_env = no
catch_workers_output = yes
php_value[memory_limit] = -1
php_value[upload_max_filesize] = 0
php_value[post_max_size] = 0
php_value[max_execution_time] = 0
php_admin_value[max_input_time] = -1
"#,
        user = current_user(),
        group = current_group(),
        listen = listen,
    );
    fs::write(&pool, pool_body)?;
    Ok(conf)
}

fn current_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "nobody".into())
}

fn current_group() -> String {
    // Prefer primary group name when available
    #[cfg(unix)]
    {
        if let Ok(out) = command_id_gn() {
            if !out.is_empty() {
                return out;
            }
        }
    }
    current_user()
}

#[cfg(unix)]
fn command_id_gn() -> Result<String> {
    Ok(run_capture(Path::new("id"), &["-gn"], None)?
        .trim()
        .to_string())
}

pub fn start_php(paths: &Paths, state: &mut AppState, version: &str) -> Result<PhpRuntime> {
    let runtime = resolve_php(paths, state, version)?;
    let existing = state.runtime.php_pids.get(&runtime.version).copied();
    match ensure_php_running(paths, state, &runtime, existing) {
        Ok(Some(pid)) => {
            state.runtime.php_pids.insert(runtime.version.clone(), pid);
        }
        Ok(None) => {}
        Err(e) => {
            if existing.is_some() {
                state.runtime.php_pids.remove(&runtime.version);
            }
            return Err(e);
        }
    }
    Ok(runtime)
}

/// Start several PHP versions concurrently (each spawn + listen wait is ~100ms+).
/// Results keep the order of `versions`; duplicates resolving to the same runtime
/// are started once. Safe to call when everything is already running (no-op fast).
pub fn start_php_many(
    paths: &Paths,
    state: &mut AppState,
    versions: &[String],
) -> Vec<(String, Result<PhpRuntime>)> {
    let mut out: Vec<(String, Result<PhpRuntime>)> = Vec::with_capacity(versions.len());
    let mut jobs: Vec<(usize, PhpRuntime, Option<u32>)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (i, v) in versions.iter().enumerate() {
        match resolve_php(paths, state, v) {
            Ok(rt) => {
                if seen.insert(rt.version.clone()) {
                    let existing = state.runtime.php_pids.get(&rt.version).copied();
                    jobs.push((i, rt.clone(), existing));
                }
                out.push((v.clone(), Ok(rt)));
            }
            Err(e) => out.push((v.clone(), Err(e))),
        }
    }

    let state_ro: &AppState = state;
    let results: Vec<(usize, Result<Option<u32>>)> = if jobs.len() <= 1 {
        jobs.iter()
            .map(|(i, rt, existing)| (*i, ensure_php_running(paths, state_ro, rt, *existing)))
            .collect()
    } else {
        std::thread::scope(|scope| {
            let handles: Vec<_> = jobs
                .iter()
                .map(|(i, rt, existing)| {
                    let (i, existing) = (*i, *existing);
                    scope.spawn(move || (i, ensure_php_running(paths, state_ro, rt, existing)))
                })
                .collect();
            handles
                .into_iter()
                .zip(jobs.iter())
                .map(|(h, (i, _, _))| {
                    h.join().unwrap_or_else(|_| {
                        (*i, Err(DevStackError::Message("php start thread panicked".into())))
                    })
                })
                .collect()
        })
    };

    for (i, res) in results {
        let rt_version = jobs
            .iter()
            .find(|(j, _, _)| *j == i)
            .map(|(_, rt, _)| rt.version.clone())
            .unwrap_or_default();
        match res {
            Ok(Some(pid)) => {
                state.runtime.php_pids.insert(rt_version, pid);
            }
            Ok(None) => {}
            Err(e) => {
                state.runtime.php_pids.remove(&rt_version);
                let msg = e.to_string();
                for (_, r) in out.iter_mut() {
                    if matches!(r, Ok(rt) if rt.version == rt_version) {
                        *r = Err(DevStackError::Message(msg.clone()));
                    }
                }
            }
        }
    }
    out
}

/// Fast path: already running + listening → `Ok(None)`. Otherwise (re)spawn and
/// return the new pid. Does not touch `state.runtime` so callers can run it from
/// several threads and merge pids afterwards.
fn ensure_php_running(
    paths: &Paths,
    state: &AppState,
    runtime: &PhpRuntime,
    existing_pid: Option<u32>,
) -> Result<Option<u32>> {
    if let Some(pid) = existing_pid {
        if is_running(pid) && php_listen_ready(runtime) {
            return Ok(None);
        }
        // Stale pid or dead listener — restart.
        mark_supervisor_stop(&runtime.version);
        let _ = stop_pid(pid);
        for kind in ["php-fpm", "php-cgi"] {
            if let Some(p) = read_pid(&paths.pid_file(&format!("{kind}-{}", runtime.version))) {
                let _ = stop_pid(p);
            }
        }
    }
    let pid = if cfg!(windows) {
        start_php_cgi_workers(paths, state, runtime)?
    } else {
        start_php_fpm(paths, runtime)?
    };
    Ok(Some(pid))
}

fn php_listen_ready(runtime: &PhpRuntime) -> bool {
    if runtime.listen.starts_with("unix:") {
        return Path::new(runtime.listen.trim_start_matches("unix:")).exists();
    }
    parse_tcp_listen(&runtime.listen)
        .map(|addr| {
            std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(100))
                .is_ok()
        })
        .unwrap_or(false)
}

fn start_php_fpm(paths: &Paths, runtime: &PhpRuntime) -> Result<u32> {
    let fpm = runtime
        .fpm_bin
        .as_ref()
        .ok_or_else(|| DevStackError::Message("php-fpm binary not found".into()))?;
    let conf = ensure_fpm_pool(paths, runtime)?;
    let log_out = paths.logs.join(format!("php-fpm-{}-stdout.log", runtime.version));
    let log_err = paths.logs.join(format!("php-fpm-{}-stderr.log", runtime.version));

    // Stop any stale pid file process
    let pid_file = paths.pid_file(&format!("php-fpm-{}", runtime.version));
    if let Some(pid) = read_pid(&pid_file) {
        let _ = stop_pid(pid);
    }
    // A leftover socket from a killed master makes "socket exists" a false ready.
    if let Some(sock) = runtime.listen.strip_prefix("unix:") {
        let _ = fs::remove_file(sock);
    }

    let args = ["-y", &conf.to_string_lossy(), "--nodaemonize"];
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
    let child = spawn_logged(fpm, &args_ref, None, &log_out, &log_err)?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&pid_file, pid)?;
    // Short backoff poll (5ms → 25ms, ≤2s): sockets usually appear in <50ms.
    let _ = crate::services::perf::poll_backoff(
        std::time::Duration::from_millis(5),
        std::time::Duration::from_millis(25),
        std::time::Duration::from_secs(2),
        || (php_listen_ready(runtime) || !is_running(pid)).then_some(()),
    );
    Ok(pid)
}

fn parse_tcp_listen(listen: &str) -> Option<std::net::SocketAddr> {
    listen.parse().ok().or_else(|| {
        // Accept "127.0.0.1:9083" already; also bare ":9083" is invalid.
        None
    })
}

/// Wait until php-cgi / FPM accepts TCP (or unix socket exists).
/// Polls 5ms → 25ms (≈20×25ms steady state) within a 2s ceiling.
fn wait_php_listen(runtime: &PhpRuntime, pid: u32, log_err: &Path) -> Result<()> {
    use std::time::Duration;

    let mut died = false;
    let ready = crate::services::perf::poll_backoff(
        Duration::from_millis(5),
        Duration::from_millis(25),
        Duration::from_secs(2),
        || {
            if !is_running(pid) {
                died = true;
                return Some(());
            }
            php_listen_ready(runtime).then_some(())
        },
    )
    .is_some();
    if ready && !died {
        return Ok(());
    }
    let err = fs::read_to_string(log_err).unwrap_or_default();
    if died {
        return Err(DevStackError::Message(format!(
            "php-cgi {} exited before becoming ready.\n--- stderr ---\n{err}",
            runtime.version
        )));
    }
    Err(DevStackError::Message(format!(
        "php-cgi {} started (pid {pid}) but {} never accepted connections.\n--- stderr ---\n{err}",
        runtime.version, runtime.listen
    )))
}

fn start_php_cgi_workers(paths: &Paths, state: &AppState, runtime: &PhpRuntime) -> Result<u32> {
    let cgi = runtime
        .cgi_bin
        .as_ref()
        .ok_or_else(|| DevStackError::Message("php-cgi binary not found".into()))?;
    let listen = runtime.listen.clone();
    // Ensure managed php.ini exists (absolute extension_dir on Windows; honor xdebug).
    let mut opts = crate::php::PhpIniOptions::default().with_dump_helper(paths);
    if let Some(settings) = state.php_extensions.get(&runtime.version) {
        opts.xdebug = settings.xdebug;
    }
    crate::php::write_dev_php_ini(&runtime.root, &opts)?;
    let ini = crate::packages::php::php_ini_path(&runtime.root);
    let log_out = paths.logs.join(format!("php-cgi-{}-stdout.log", runtime.version));
    let log_err = paths.logs.join(format!("php-cgi-{}-stderr.log", runtime.version));
    // Stop stale worker for this version first.
    let pid_file = paths.pid_file(&format!("php-cgi-{}", runtime.version));
    if let Some(pid) = read_pid(&pid_file) {
        let _ = stop_pid(pid);
    }
    // Free the listen port if a zombie worker still holds it.
    if let Some(addr) = parse_tcp_listen(&listen) {
        let _ = crate::services::perf::poll_backoff(
            std::time::Duration::from_millis(2),
            std::time::Duration::from_millis(25),
            std::time::Duration::from_secs(1),
            || TcpListener::bind(addr).is_ok().then_some(()),
        );
    }
    let ini_s = ini.to_string_lossy().to_string();
    let args: Vec<&str> = if ini.exists() {
        vec!["-c", ini_s.as_str(), "-b", listen.as_str()]
    } else {
        vec!["-b", listen.as_str()]
    };
    let phprc = ini
        .parent()
        .map(|p| p.as_os_str())
        .unwrap_or_else(|| std::ffi::OsStr::new(""));
    // PHP_FCGI_MAX_REQUESTS=0 → never self-exit after N requests (Windows default is 500).
    let max_req = std::ffi::OsString::from("0");
    let mut envs: Vec<(&str, &std::ffi::OsStr)> = vec![("PHP_FCGI_MAX_REQUESTS", max_req.as_os_str())];
    if ini.exists() {
        envs.push(("PHPRC", phprc));
    }
    let child = crate::services::process::spawn_logged_env(
        cgi,
        &args,
        Some(&runtime.root),
        &log_out,
        &log_err,
        &envs,
        // Attached: we own the FOREGROUND pid for restart/stop; DETACHED orphans
        // workers that die after PHP_FCGI_MAX_REQUESTS with no supervisor.
        false,
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&pid_file, pid)?;
    wait_php_listen(runtime, pid, &log_err)?;

    // Lightweight supervisor: if the worker dies, respawn once the port frees.
    clear_supervisor_stop(&runtime.version);
    spawn_php_cgi_supervisor(
        paths.clone(),
        runtime.clone(),
        ini.clone(),
        cgi.clone(),
        log_out,
        log_err,
        pid_file,
    );
    Ok(pid)
}

/// Background respawn for Windows php-cgi (single listener per version).
fn spawn_php_cgi_supervisor(
    paths: Paths,
    runtime: PhpRuntime,
    ini: PathBuf,
    cgi: PathBuf,
    log_out: PathBuf,
    log_err: PathBuf,
    pid_file: PathBuf,
) {
    let version = runtime.version.clone();
    std::thread::Builder::new()
        .name(format!("php-cgi-supervise-{version}"))
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if should_supervisor_stop(&version) {
                    break;
                }
                let pid = read_pid(&pid_file);
                let alive = pid.map(is_running).unwrap_or(false);
                let listening = if let Some(addr) = parse_tcp_listen(&runtime.listen) {
                    std::net::TcpStream::connect_timeout(
                        &addr,
                        std::time::Duration::from_millis(80),
                    )
                    .is_ok()
                } else {
                    true
                };
                if alive && listening {
                    continue;
                }
                if should_supervisor_stop(&version) {
                    break;
                }
                // Worker gone or port dead — respawn.
                if let Some(p) = pid {
                    let _ = stop_pid(p);
                }
                let ini_s = ini.to_string_lossy().to_string();
                let listen = runtime.listen.clone();
                let args: Vec<&str> = if ini.exists() {
                    vec!["-c", ini_s.as_str(), "-b", listen.as_str()]
                } else {
                    vec!["-b", listen.as_str()]
                };
                let max_req = std::ffi::OsString::from("0");
                let phprc = ini
                    .parent()
                    .map(|p| p.as_os_str())
                    .unwrap_or_else(|| std::ffi::OsStr::new(""));
                let mut envs: Vec<(&str, &std::ffi::OsStr)> =
                    vec![("PHP_FCGI_MAX_REQUESTS", max_req.as_os_str())];
                if ini.exists() {
                    envs.push(("PHPRC", phprc));
                }
                let Ok(child) = crate::services::process::spawn_logged_env(
                    &cgi,
                    &args,
                    Some(&runtime.root),
                    &log_out,
                    &log_err,
                    &envs,
                    false,
                ) else {
                    continue;
                };
                let new_pid = child.id();
                std::mem::forget(child);
                let _ = write_pid(&pid_file, new_pid);
                let _ = paths; // keep Paths alive for pid_file path consistency
            }
        })
        .ok();
}

pub fn stop_php(paths: &Paths, state: &mut AppState, version: &str) -> Result<()> {
    mark_supervisor_stop(version);
    if let Some(pid) = state.runtime.php_pids.remove(version) {
        stop_pid(pid)?;
    }
    for kind in ["php-fpm", "php-cgi"] {
        if let Some(pid) = read_pid(&paths.pid_file(&format!("{kind}-{version}"))) {
            let _ = stop_pid(pid);
        }
    }
    Ok(())
}

pub fn nginx_fastcgi_pass(runtime: &PhpRuntime) -> String {
    if runtime.listen.starts_with("unix:") {
        runtime.listen.clone()
    } else {
        runtime.listen.clone()
    }
}

pub fn register_system_php(state: &mut AppState, paths: &Paths) -> Result<PhpRuntime> {
    let runtime = resolve_system_php(paths, state, "8.3")?;
    // Record as installed pointing at system paths so UI can select it
    use crate::config::{mark_installed, save_state, InstalledRecord};
    use crate::packages::now_iso;
    mark_installed(
        state,
        "php",
        InstalledRecord {
            version: runtime.version.clone(),
            channel: Some(
                runtime
                    .version
                    .split('.')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            path: runtime.root.clone(),
            installed_at: now_iso(),
        },
    )?;
    save_state(&paths.state_file, state)?;
    Ok(runtime)
}

#[cfg(test)]
mod perf_tests {
    use super::*;

    #[test]
    fn start_php_many_with_no_versions_is_a_noop() {
        let dir = std::env::temp_dir().join(format!("devstack-php-many-{}", std::process::id()));
        let paths = Paths::from_root(dir.clone()).unwrap();
        let mut state = AppState::default();
        assert!(start_php_many(&paths, &mut state, &[]).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
