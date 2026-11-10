//! v0.3.0 managed services: Memcached, Meilisearch, Typesense, MinIO, Laravel Reverb.
//!
//! Lifecycle plugs into [`crate::services::extras`] (`start_service` / `stop_service` /
//! `service_status`). Binaries are never auto-started with the stack.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use crate::config::{latest_installed, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::process::{find_bin, is_running, read_pid, spawn_logged_env, stop_pid, write_pid};

pub const NEW_SERVICE_IDS: &[&str] = &["memcached", "meilisearch", "typesense", "minio", "reverb"];

pub const MINIO_USER: &str = "devstack";
pub const MINIO_PASSWORD: &str = "devstack123";
pub const TYPESENSE_API_KEY: &str = "devstack";

pub fn is_new_service(id: &str) -> bool {
    NEW_SERVICE_IDS.contains(&id)
}

pub fn display_name(id: &str) -> &'static str {
    match id {
        "memcached" => "Memcached",
        "meilisearch" => "Meilisearch",
        "typesense" => "Typesense",
        "minio" => "MinIO",
        "reverb" => "Laravel Reverb",
        _ => "Service",
    }
}

pub fn pid_of(state: &AppState, id: &str) -> Option<u32> {
    match id {
        "memcached" => state.runtime.memcached_pid,
        "meilisearch" => state.runtime.meilisearch_pid,
        "typesense" => state.runtime.typesense_pid,
        "minio" => state.runtime.minio_pid,
        "reverb" => state.runtime.reverb_pid,
        _ => None,
    }
}

pub fn set_pid(state: &mut AppState, id: &str, pid: Option<u32>) {
    match id {
        "memcached" => state.runtime.memcached_pid = pid,
        "meilisearch" => state.runtime.meilisearch_pid = pid,
        "typesense" => state.runtime.typesense_pid = pid,
        "minio" => state.runtime.minio_pid = pid,
        "reverb" => state.runtime.reverb_pid = pid,
        _ => {}
    }
}

/// Listen ports (API first, console second for MinIO).
pub fn ports_of(state: &AppState, id: &str) -> Vec<u16> {
    let s = &state.stack;
    match id {
        "memcached" => vec![s.memcached_port],
        "meilisearch" => vec![s.meilisearch_port],
        "typesense" => vec![s.typesense_port],
        "minio" => vec![s.minio_port, s.minio_console_port],
        "reverb" => vec![s.reverb_port],
        _ => vec![],
    }
}

pub fn ui_url(state: &AppState, id: &str) -> Option<String> {
    match id {
        "meilisearch" => Some(format!("http://127.0.0.1:{}", state.stack.meilisearch_port)),
        "minio" => Some(format!("http://127.0.0.1:{}", state.stack.minio_console_port)),
        "typesense" => Some(format!("http://127.0.0.1:{}/health", state.stack.typesense_port)),
        _ => None,
    }
}

/// Platform gate: Typesense does not publish Windows binaries.
pub fn is_available_on_platform(id: &str) -> bool {
    !(id == "typesense" && cfg!(windows))
}

pub fn unavailable_message(id: &str) -> String {
    match id {
        "typesense" => {
            "Not available on Windows — Typesense does not publish Windows binaries.".into()
        }
        other => format!("{other} is not available on this platform."),
    }
}

pub fn notes(state: &AppState, id: &str) -> Option<String> {
    if !is_available_on_platform(id) {
        return Some(unavailable_message(id));
    }
    let s = &state.stack;
    Some(match id {
        "memcached" => format!("TCP :{} — cache / session backend", s.memcached_port),
        "meilisearch" => format!("HTTP :{} · dev mode, no master key", s.meilisearch_port),
        "typesense" => format!(
            "HTTP :{} · API key `{TYPESENSE_API_KEY}`",
            s.typesense_port
        ),
        "minio" => format!(
            "S3 API :{} · console :{} · login {MINIO_USER} / {MINIO_PASSWORD}",
            s.minio_port, s.minio_console_port
        ),
        "reverb" => match reverb_site(state) {
            Some(site) => format!(
                "WebSocket :{} · runs `php artisan reverb:start` in {}",
                s.reverb_port, site.domain
            ),
            None => "Select a Laravel site with laravel/reverb installed (composer require laravel/reverb)".into(),
        },
        _ => return None,
    })
}

fn site_has_reverb(site: &SiteRecord) -> bool {
    let root = crate::paths_util::expand_home(&site.path);
    if !root.join("artisan").exists() {
        return false;
    }
    root.join("vendor/laravel/reverb").exists()
        || fs::read_to_string(root.join("composer.json"))
            .map(|c| c.contains("laravel/reverb"))
            .unwrap_or(false)
}

/// Site Reverb should serve: the configured one, else the first Laravel site with the package.
pub fn reverb_site(state: &AppState) -> Option<&SiteRecord> {
    if let Some(id) = state.stack.reverb_site_id.as_deref() {
        if let Some(s) = state.sites.iter().find(|s| s.id == id) {
            return Some(s);
        }
    }
    state.sites.iter().find(|s| site_has_reverb(s))
}

/// Whether the service can be started (binary installed / reverb site resolvable).
pub fn is_installed(state: &AppState, id: &str) -> bool {
    if id == "reverb" {
        return reverb_site(state).is_some();
    }
    latest_installed(state, id).is_some()
}

fn install_bin(state: &AppState, id: &str) -> Result<(PathBuf, PathBuf)> {
    let root = latest_installed(state, id)
        .map(|r| r.path.clone())
        .ok_or_else(|| DevStackError::Message(format!("{} is not installed", display_name(id))))?;
    let names: &[&str] = match id {
        "memcached" => &["memcached", "memcached.exe"],
        "meilisearch" => &["meilisearch", "meilisearch.exe"],
        "typesense" => &["typesense-server", "typesense-server.exe"],
        "minio" => &["minio", "minio.exe", "silo", "silo.exe"],
        _ => &[],
    };
    let bin = find_bin(&root, names).ok_or_else(|| {
        DevStackError::Message(format!("{} binary not found under {}", display_name(id), root.display()))
    })?;
    Ok((bin, root))
}

/// Everything needed to spawn a service (pure, unit-tested).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPlan {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub data_dir: PathBuf,
}

pub fn spawn_plan(paths: &Paths, state: &AppState, id: &str) -> Option<SpawnPlan> {
    let s = &state.stack;
    let data = paths.databases.join(id);
    let data_s = data.display().to_string();
    Some(match id {
        "memcached" => SpawnPlan {
            args: vec!["-l".into(), "127.0.0.1".into(), "-p".into(), s.memcached_port.to_string(), "-U".into(), "0".into()],
            env: vec![],
            data_dir: data,
        },
        "meilisearch" => SpawnPlan {
            args: vec![
                "--http-addr".into(),
                format!("127.0.0.1:{}", s.meilisearch_port),
                "--db-path".into(),
                data.join("data.ms").display().to_string(),
                "--dump-dir".into(),
                data.join("dumps").display().to_string(),
                "--no-analytics".into(),
                "--env".into(),
                "development".into(),
            ],
            env: vec![],
            data_dir: data,
        },
        "typesense" => SpawnPlan {
            args: vec![
                format!("--data-dir={}", data.join("data").display()),
                format!("--api-key={TYPESENSE_API_KEY}"),
                "--listen-address=127.0.0.1".into(),
                format!("--listen-port={}", s.typesense_port),
                "--enable-cors".into(),
            ],
            env: vec![],
            data_dir: data,
        },
        "minio" => SpawnPlan {
            args: vec![
                "server".into(),
                data.join("data").display().to_string(),
                "--address".into(),
                format!("127.0.0.1:{}", s.minio_port),
                "--console-address".into(),
                format!("127.0.0.1:{}", s.minio_console_port),
            ],
            env: vec![
                ("MINIO_ROOT_USER".into(), MINIO_USER.into()),
                ("MINIO_ROOT_PASSWORD".into(), MINIO_PASSWORD.into()),
            ],
            data_dir: data,
        },
        "reverb" => SpawnPlan {
            args: vec![
                "artisan".into(),
                "reverb:start".into(),
                "--host=127.0.0.1".into(),
                format!("--port={}", s.reverb_port),
            ],
            env: vec![],
            data_dir: PathBuf::from(data_s),
        },
        _ => return None,
    })
}

fn wait_ready(id: &str, state: &AppState, pid: u32) -> Result<()> {
    let port = ports_of(state, id).first().copied().unwrap_or(0);
    for _ in 0..50 {
        if crate::services::extras::port_open(port) {
            return Ok(());
        }
        if !is_running(pid) {
            return Err(DevStackError::Message(format!(
                "{} exited immediately — see its log",
                display_name(id)
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(DevStackError::Message(format!("timed out waiting for port {port}")))
}

/// Busy-port fallback: when a foreign process holds one of the service's ports, move to the
/// next free candidate (never touching the other process) and leave a notice for the UI.
pub fn auto_remap_if_busy(state: &mut AppState, id: &str) -> Option<String> {
    let check = crate::services::ports::check_service_ports(&state.stack, id);
    if check.ok || check.suggested.is_empty() {
        return None;
    }
    let msg = format!(
        "{}: {} — using {}",
        display_name(id),
        check.message.clone().unwrap_or_else(|| "port busy".into()),
        check
            .suggested
            .iter()
            .map(|(f, p)| format!("{f}→:{p}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    crate::services::ports::apply_suggested_ports(&mut state.stack, &check.suggested).ok()?;
    state.runtime.port_notice.push(msg.clone());
    Some(msg)
}

pub fn start(paths: &Paths, state: &mut AppState, id: &str) -> Result<()> {
    if let Some(pid) = pid_of(state, id) {
        if is_running(pid) {
            return Ok(());
        }
    }
    let plan = spawn_plan(paths, state, id)
        .ok_or_else(|| DevStackError::Message(format!("start not supported for {id}")))?;
    fs::create_dir_all(&plan.data_dir).ok();
    let out = paths.logs.join(format!("{id}.out"));
    let err = paths.logs.join(format!("{id}.err"));

    let (program, cwd, args): (PathBuf, PathBuf, Vec<String>) = if id == "reverb" {
        let site = reverb_site(state)
            .ok_or_else(|| {
                DevStackError::Message(
                    "Reverb needs a Laravel site with laravel/reverb installed. Pick one in Services → Reverb.".into(),
                )
            })?
            .clone();
        let php_version = site
            .php_version
            .clone()
            .or_else(|| state.stack.php_version.clone())
            .unwrap_or_else(|| "8.3".into());
        let rt = crate::services::php_runtime::resolve_php(paths, state, &php_version)?;
        let mut args = Vec::new();
        let merged = crate::services::site_php_ini::merged_ini_path(paths, &site.id);
        let ini = if merged.exists() {
            merged
        } else {
            crate::packages::php::php_ini_path(&rt.root)
        };
        if ini.exists() {
            args.push("-c".to_string());
            args.push(ini.display().to_string());
        }
        args.extend(plan.args.clone());
        (rt.php_bin, crate::paths_util::expand_home(&site.path), args)
    } else {
        let (bin, root) = install_bin(state, id)?;
        (bin, if id == "minio" || id == "meilisearch" { plan.data_dir.clone() } else { root }, plan.args.clone())
    };

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let env_os: Vec<(String, OsString)> = plan.env.iter().map(|(k, v)| (k.clone(), OsString::from(v))).collect();
    let env_refs: Vec<(&str, &std::ffi::OsStr)> = env_os.iter().map(|(k, v)| (k.as_str(), v.as_os_str())).collect();
    let child = spawn_logged_env(&program, &arg_refs, Some(&cwd), &out, &err, &env_refs, true)?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file(id), pid)?;
    set_pid(state, id, Some(pid));
    wait_ready(id, state, pid).map_err(|e| {
        let _ = stop_pid(pid);
        set_pid(state, id, None);
        e
    })
}

pub fn stop(paths: &Paths, state: &mut AppState, id: &str) -> Result<()> {
    let ports = ports_of(state, id);
    if let Some(pid) = pid_of(state, id) {
        // Meilisearch (and friends) can ignore a soft TERM briefly; always escalate.
        let _ = stop_pid(pid);
        #[cfg(unix)]
        {
            // Process-group kill covers binaries that re-exec / daemonize a child
            // while we still hold the original spawn pid.
            let _ = crate::services::process::command("kill")
                .args(["-KILL", &format!("-{pid}")])
                .status();
            let _ = crate::services::process::command("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
    }
    set_pid(state, id, None);
    if let Some(pid) = read_pid(&paths.pid_file(id)) {
        let _ = stop_pid(pid);
        #[cfg(unix)]
        {
            let _ = crate::services::process::command("kill")
                .args(["-KILL", &format!("-{pid}")])
                .status();
            let _ = crate::services::process::command("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
    }
    let _ = fs::remove_file(paths.pid_file(id));
    // service_status treats an open listen port as "running" even with no pid —
    // wait until our ports are free so stop looks stopped immediately.
    for port in ports {
        let _ = crate::services::perf::poll_backoff(
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(40),
            std::time::Duration::from_secs(3),
            || (!crate::services::extras::port_open(port)).then_some(()),
        );
        if crate::services::extras::port_open(port) {
            // Last resort: kill whoever still owns the port (only our data-dir
            // orphans should linger here after stop_pid).
            #[cfg(unix)]
            {
                let _ = crate::services::process::command("sh")
                    .args([
                        "-c",
                        &format!(
                            "pids=$(lsof -t -iTCP:{port} -sTCP:LISTEN 2>/dev/null); \
                             [ -n \"$pids\" ] && kill -KILL $pids"
                        ),
                    ])
                    .status();
            }
            #[cfg(windows)]
            {
                // Prefer netstat — Get-NetTCPConnection regularly hangs for minutes
                // on Windows CI (CIM). Parse `TCP  127.0.0.1:PORT ... LISTENING  PID`.
                kill_windows_listeners(port);
            }
            let _ = crate::services::perf::poll_backoff(
                std::time::Duration::from_millis(5),
                std::time::Duration::from_millis(40),
                std::time::Duration::from_millis(800),
                || (!crate::services::extras::port_open(port)).then_some(()),
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn kill_windows_listeners(port: u16) {
    let Ok(out) = crate::services::process::command("netstat")
        .args(["-ano", "-p", "tcp"])
        .output()
    else {
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut killed = std::collections::HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("TCP") || !line.contains("LISTENING") {
            continue;
        }
        // Columns: TCP  local  foreign  state  pid
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 {
            continue;
        }
        let local = cols[1];
        let Some(local_port) = local
            .rsplit_once(':')
            .map(|(_, p)| p.trim_end_matches(']'))
            .and_then(|p| p.parse::<u16>().ok())
        else {
            continue;
        };
        if local_port != port {
            continue;
        }
        let Ok(pid) = cols[4].parse::<u32>() else { continue };
        if pid == 0 || !killed.insert(pid) {
            continue;
        }
        let _ = stop_pid(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> (Paths, PathBuf) {
        let dir = std::env::temp_dir().join(format!("devstack-extras030-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (Paths::from_root(dir.clone()).unwrap(), dir)
    }

    #[test]
    fn ids_and_default_ports() {
        let st = AppState::default();
        assert_eq!(ports_of(&st, "memcached"), vec![11211]);
        assert_eq!(ports_of(&st, "meilisearch"), vec![7700]);
        assert_eq!(ports_of(&st, "typesense"), vec![8108]);
        assert_eq!(ports_of(&st, "minio"), vec![9100, 9101]);
        assert_eq!(ports_of(&st, "reverb"), vec![8085]);
        assert!(ports_of(&st, "nope").is_empty());
        assert!(is_new_service("minio") && !is_new_service("redis"));
    }

    #[test]
    fn service_ports_match_extras_ports() {
        let st = AppState::default();
        for id in NEW_SERVICE_IDS {
            let from_ports: Vec<u16> = crate::services::ports::service_ports(&st.stack, id)
                .into_iter()
                .map(|(_, p)| p)
                .collect();
            assert_eq!(from_ports, ports_of(&st, id), "{id}");
            assert_eq!(crate::services::extras::default_ports(id), ports_of(&st, id), "{id}");
        }
    }

    #[test]
    fn busy_port_auto_remaps_without_touching_holder() {
        let holder = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy = holder.local_addr().unwrap().port();
        let mut st = AppState::default();
        st.stack.meilisearch_port = busy;
        let msg = auto_remap_if_busy(&mut st, "meilisearch").expect("remap");
        assert!(msg.contains("Meilisearch"));
        assert_ne!(st.stack.meilisearch_port, busy);
        assert!(crate::services::ports::port_is_free(st.stack.meilisearch_port));
        assert!(st.runtime.port_notice.iter().any(|m| m.contains("Meilisearch")));
        assert!(auto_remap_if_busy(&mut st, "meilisearch").is_none());
        assert!(holder.local_addr().is_ok());
    }

    #[test]
    fn spawn_plans_use_configured_ports() {
        let (paths, dir) = temp_paths("plan");
        let mut st = AppState::default();
        st.stack.memcached_port = 11300;
        st.stack.meilisearch_port = 7710;
        st.stack.minio_port = 9200;
        st.stack.minio_console_port = 9201;
        let m = spawn_plan(&paths, &st, "memcached").unwrap();
        assert!(m.args.contains(&"11300".to_string()));
        let ms = spawn_plan(&paths, &st, "meilisearch").unwrap();
        assert!(ms.args.contains(&"127.0.0.1:7710".to_string()));
        assert!(ms.args.contains(&"--no-analytics".to_string()));
        let mi = spawn_plan(&paths, &st, "minio").unwrap();
        assert!(mi.args.contains(&"127.0.0.1:9200".to_string()));
        assert!(mi.args.contains(&"127.0.0.1:9201".to_string()));
        assert!(mi.env.iter().any(|(k, _)| k == "MINIO_ROOT_USER"));
        let ts = spawn_plan(&paths, &st, "typesense").unwrap();
        assert!(ts.args.iter().any(|a| a == "--listen-port=8108"));
        assert!(spawn_plan(&paths, &st, "redis").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reverb_resolves_laravel_site_with_package() {
        let (_paths, dir) = temp_paths("reverb");
        let app = dir.join("app");
        fs::create_dir_all(app.join("vendor/laravel/reverb")).unwrap();
        fs::write(app.join("artisan"), "").unwrap();
        let mut st = AppState::default();
        assert!(!is_installed(&st, "reverb"));
        let mut site = SiteRecord {
            id: "app".into(),
            name: "app".into(),
            path: app,
            domain: String::new(),
            php_version: None,
            node_version: None,
            https: true,
            web_server: None,
            framework: Some("laravel".into()),
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
        site.ensure_domain();
        st.sites.push(site);
        assert!(is_installed(&st, "reverb"));
        assert_eq!(reverb_site(&st).unwrap().id, "app");
        let plan = spawn_plan(&_paths, &st, "reverb").unwrap();
        assert_eq!(plan.args[1], "reverb:start");
        assert!(notes(&st, "reverb").unwrap().contains("app.test"));
        let _ = fs::remove_dir_all(&dir);
    }
}
