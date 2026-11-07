//! PHP-FPM pools (macOS/Linux) and php-cgi worker manager (Windows).

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use crate::config::{find_installed, AppState, Paths};
use crate::error::{DevStackError, Result};
use crate::services::process::{
    find_bin, is_running, read_pid, run_capture, spawn_logged, stop_pid, write_pid,
};

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
    let out = run_capture(php, &["-r", "echo PHP_MAJOR_VERSION.'.'.PHP_MINOR_VERSION.'.'.PHP_RELEASE_VERSION;"], None).ok()?;
    Some(out.trim().to_string())
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
php_admin_value[memory_limit] = -1
php_admin_value[upload_max_filesize] = 0
php_admin_value[post_max_size] = 0
php_admin_value[max_execution_time] = 0
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
    if let Some(pid) = state.runtime.php_pids.get(&runtime.version).copied() {
        if is_running(pid) {
            return Ok(runtime);
        }
    }

    if cfg!(windows) {
        start_php_cgi_workers(paths, state, &runtime)?;
    } else {
        start_php_fpm(paths, state, &runtime)?;
    }
    Ok(runtime)
}

fn start_php_fpm(paths: &Paths, state: &mut AppState, runtime: &PhpRuntime) -> Result<()> {
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

    let args = ["-y", &conf.to_string_lossy(), "--nodaemonize"];
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_ref()).collect();
    let child = spawn_logged(fpm, &args_ref, None, &log_out, &log_err)?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&pid_file, pid)?;
    state.runtime.php_pids.insert(runtime.version.clone(), pid);
    // Wait briefly for socket
    for _ in 0..20 {
        if runtime.listen.starts_with("unix:") {
            let sock = runtime.listen.trim_start_matches("unix:");
            if Path::new(sock).exists() {
                break;
            }
        } else if TcpListener::bind(runtime.listen.trim_start_matches("127.0.0.1:").parse::<std::net::SocketAddr>().ok().unwrap_or_else(|| "127.0.0.1:9".parse().unwrap())).is_err() {
            // port in use = listening
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}

fn start_php_cgi_workers(paths: &Paths, state: &mut AppState, runtime: &PhpRuntime) -> Result<()> {
    let cgi = runtime
        .cgi_bin
        .as_ref()
        .ok_or_else(|| DevStackError::Message("php-cgi binary not found".into()))?;
    let listen = runtime.listen.clone();
    // Ensure managed php.ini exists (absolute extension_dir on Windows; honor xdebug).
    let mut opts = crate::php::PhpIniOptions::default();
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
    let envs: Vec<(&str, &std::ffi::OsStr)> = if ini.exists() {
        vec![("PHPRC", phprc)]
    } else {
        vec![]
    };
    let child = crate::services::process::spawn_logged_env(
        cgi,
        &args,
        Some(&runtime.root),
        &log_out,
        &log_err,
        &envs,
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&pid_file, pid)?;
    state.runtime.php_pids.insert(runtime.version.clone(), pid);
    Ok(())
}

pub fn stop_php(paths: &Paths, state: &mut AppState, version: &str) -> Result<()> {
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
