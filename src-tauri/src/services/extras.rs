//! MongoDB, PostgreSQL, Redis, Mailpit process lifecycle + status.

use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::config::{find_installed, latest_installed, save_state, AppState, Paths};
use crate::error::{DevStackError, Result};
use crate::services::process::{
    find_bin, is_running, read_pid, spawn_logged, stop_pid, write_pid,
};
use crate::services::tls::ensure_site_cert;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedServiceStatus {
    pub id: String,
    pub name: String,
    pub running: bool,
    pub installed: bool,
    pub version: Option<String>,
    pub ports: Vec<u16>,
    pub log_path: Option<String>,
    pub ui_url: Option<String>,
    pub notes: Option<String>,
}

pub fn default_ports(id: &str) -> Vec<u16> {
    let d = crate::config::StackPreferences::default();
    match id {
        "mongodb" => vec![d.mongodb_port],
        "postgresql" => vec![d.postgresql_port],
        "redis" => vec![6379],
        "mailpit" => vec![1025, 8025],
        "mariadb" => vec![d.mariadb_port],
        "mysql" => vec![d.mysql_port],
        "nginx" => vec![d.http_port, d.https_port],
        "apache" => vec![d.apache_http_port, d.apache_https_port],
        _ => vec![],
    }
}

fn install_root(state: &AppState, id: &str) -> Option<PathBuf> {
    latest_installed(state, id).map(|r| r.path.clone())
}

fn port_open(port: u16) -> bool {
    TcpStream::connect(("127.0.0.1", port)).is_ok()
}

pub fn service_status(paths: &Paths, state: &AppState, id: &str) -> ManagedServiceStatus {
    let mailpit_url =
        crate::services::ports::format_url("https", "mailpit.test", state.stack.https_port);
    let (name, pid, ports, ui) = match id {
        "mongodb" => (
            "MongoDB",
            state.runtime.mongodb_pid,
            vec![state.stack.mongodb_port],
            None,
        ),
        "postgresql" => (
            "PostgreSQL",
            state.runtime.postgresql_pid,
            vec![state.stack.postgresql_port],
            None,
        ),
        "redis" => ("Redis", state.runtime.redis_pid, vec![6379], None),
        "mailpit" => (
            "Mailpit",
            state.runtime.mailpit_pid,
            vec![1025, 8025],
            Some(mailpit_url),
        ),
        "mariadb" => (
            "MariaDB",
            state.runtime.database_pid.filter(|_| {
                state.runtime.database_engine.as_deref() == Some("mariadb")
            }),
            vec![state.stack.mariadb_port],
            None,
        ),
        "mysql" => (
            "MySQL",
            state.runtime.database_pid.filter(|_| {
                state.runtime.database_engine.as_deref() == Some("mysql")
            }),
            vec![state.stack.mysql_port],
            None,
        ),
        "nginx" => (
            "Nginx",
            state.runtime.nginx_pid,
            vec![state.stack.http_port, state.stack.https_port],
            None,
        ),
        "apache" => (
            "Apache",
            state.runtime.apache_pid,
            vec![
                state.stack.apache_http_port,
                state.stack.apache_https_port,
            ],
            None,
        ),
        other => (other, None, default_ports(other), None),
    };

    let running = pid.map(is_running).unwrap_or(false)
        || ports.iter().any(|p| port_open(*p));
    let installed = state.installed.contains_key(id)
        || matches!(id, "nginx" | "apache" | "mariadb" | "mysql");
    let version = latest_installed(state, id).map(|r| r.version.clone());
    let log_path = Some(paths.logs.join(format!("{id}.err")).display().to_string());

    ManagedServiceStatus {
        id: id.into(),
        name: name.into(),
        running,
        installed: installed || version.is_some(),
        version,
        ports,
        log_path,
        ui_url: ui,
        notes: match id {
            "mailpit" => Some("SMTP :1025 · UI :8025 / mailpit.test".into()),
            "redis" => Some("Use as PHP session / cache backend".into()),
            "apache" => Some(format!(
                "HTTP :{} · HTTPS :{} (alongside Nginx :{}/:{})",
                state.stack.apache_http_port,
                state.stack.apache_https_port,
                state.stack.http_port,
                state.stack.https_port
            )),
            _ => None,
        },
    }
}

pub fn list_managed_services(paths: &Paths, state: &AppState) -> Vec<ManagedServiceStatus> {
    [
        "nginx",
        "apache",
        "mariadb",
        "mysql",
        "mongodb",
        "postgresql",
        "redis",
        "mailpit",
        "phpmyadmin",
    ]
    .into_iter()
    .map(|id| {
        if id == "phpmyadmin" {
            let version = latest_installed(state, id).map(|r| r.version.clone());
            let url = crate::services::ports::format_url(
                "https",
                "phpmyadmin.test",
                state.stack.https_port,
            );
            ManagedServiceStatus {
                id: id.into(),
                name: "phpMyAdmin".into(),
                running: state.runtime.nginx_pid.map(is_running).unwrap_or(false),
                installed: version.is_some(),
                version,
                ports: vec![state.stack.https_port],
                log_path: None,
                ui_url: Some(url),
                notes: Some(format!(
                    "Served by Nginx on :{}",
                    state.stack.https_port
                )),
            }
        } else {
            service_status(paths, state, id)
        }
    })
    .collect()
}

pub fn start_service(paths: &Paths, state: &mut AppState, id: &str) -> Result<ManagedServiceStatus> {
    // Skip conflict check only when *our* recorded PID is still alive
    // (idempotent re-start). A foreign process on the same port must error.
    let our_pid_alive = match id {
        "nginx" => state.runtime.nginx_pid.map(is_running).unwrap_or(false),
        "apache" => state.runtime.apache_pid.map(is_running).unwrap_or(false),
        "mongodb" => state.runtime.mongodb_pid.map(is_running).unwrap_or(false),
        "postgresql" => state.runtime.postgresql_pid.map(is_running).unwrap_or(false),
        "redis" => state.runtime.redis_pid.map(is_running).unwrap_or(false),
        "mailpit" => state.runtime.mailpit_pid.map(is_running).unwrap_or(false),
        "mysql" => {
            state.runtime.database_engine.as_deref() == Some("mysql")
                && state.runtime.database_pid.map(is_running).unwrap_or(false)
        }
        "mariadb" => {
            state.runtime.database_engine.as_deref() == Some("mariadb")
                && state.runtime.database_pid.map(is_running).unwrap_or(false)
        }
        _ => false,
    };
    if !our_pid_alive {
        crate::services::ports::ensure_ports_free(&state.stack, id)?;
    }
    match id {
        "mongodb" => start_mongodb(paths, state)?,
        "postgresql" => start_postgresql(paths, state)?,
        "redis" => start_redis(paths, state)?,
        "mailpit" => start_mailpit(paths, state)?,
        "mariadb" => crate::services::database::start_database(paths, state, "mariadb")?,
        "mysql" => crate::services::database::start_database(paths, state, "mysql")?,
        "nginx" => {
            crate::services::nginx::start_nginx(paths, state)?;
        }
        "apache" => {
            crate::services::httpd::start_apache(paths, state)?;
        }
        _ => {
            return Err(DevStackError::Message(format!(
                "start not supported for {id}"
            )))
        }
    }
    save_state(&paths.state_file, state)?;
    Ok(service_status(paths, state, id))
}

pub fn stop_service(paths: &Paths, state: &mut AppState, id: &str) -> Result<ManagedServiceStatus> {
    // Portable dump before stopping DB engines (Databases folder).
    if state.db_files.auto_export_on_stop {
        let progress = crate::services::progress::noop_job_progress();
        match id {
            "mariadb" | "mysql" | "mongodb" | "postgresql" => {
                let _ = crate::services::db_files::export_all_running(
                    paths,
                    state,
                    &progress,
                    "export-on-stop",
                );
            }
            _ => {}
        }
    }
    match id {
        "mongodb" => {
            if let Some(pid) = state.runtime.mongodb_pid.take() {
                stop_pid(pid)?;
            }
            if let Some(pid) = read_pid(&paths.pid_file("mongodb")) {
                let _ = stop_pid(pid);
            }
        }
        "postgresql" => {
            if let Some(root) = install_root(state, "postgresql") {
                if let Some(pg_ctl) = find_bin(&root, &["pg_ctl", "pg_ctl.exe"]) {
                    let data = paths.databases.join("postgresql/data");
                    let _ = crate::services::process::command(&pg_ctl)
                        .args(["-D", &data.to_string_lossy(), "stop", "-m", "fast"])
                        .status();
                }
            }
            if let Some(pid) = state.runtime.postgresql_pid.take() {
                let _ = stop_pid(pid);
            }
        }
        "redis" => {
            if let Some(pid) = state.runtime.redis_pid.take() {
                stop_pid(pid)?;
            }
            if let Some(pid) = read_pid(&paths.pid_file("redis")) {
                let _ = stop_pid(pid);
            }
        }
        "mailpit" => {
            if let Some(pid) = state.runtime.mailpit_pid.take() {
                stop_pid(pid)?;
            }
            if let Some(pid) = read_pid(&paths.pid_file("mailpit")) {
                let _ = stop_pid(pid);
            }
        }
        "mariadb" | "mysql" => crate::services::database::stop_database(paths, state)?,
        "nginx" => crate::services::nginx::stop_nginx(paths, state)?,
        "apache" => crate::services::httpd::stop_apache(paths, state)?,
        _ => {
            return Err(DevStackError::Message(format!(
                "stop not supported for {id}"
            )))
        }
    }
    save_state(&paths.state_file, state)?;
    Ok(service_status(paths, state, id))
}

pub fn read_service_log(paths: &Paths, id: &str, lines: usize) -> Result<String> {
    let path = paths.logs.join(format!("{id}.err"));
    if !path.exists() {
        let alt = paths.logs.join(format!("{id}.out"));
        if alt.exists() {
            return tail_file(&alt, lines);
        }
        return Ok(String::new());
    }
    tail_file(&path, lines)
}

fn tail_file(path: &Path, lines: usize) -> Result<String> {
    let raw = fs::read_to_string(path)?;
    let collected: Vec<&str> = raw.lines().rev().take(lines).collect();
    Ok(collected.into_iter().rev().collect::<Vec<_>>().join("\n"))
}

fn start_mongodb(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.mongodb_pid {
        if is_running(pid) {
            return Ok(());
        }
    }
    let root = install_root(state, "mongodb")
        .ok_or_else(|| DevStackError::Message("MongoDB is not installed".into()))?;
    let mongod = find_bin(&root, &["mongod", "mongod.exe"]).ok_or_else(|| {
        DevStackError::Message(format!("mongod not found under {}", root.display()))
    })?;
    let dbpath = paths.databases.join("mongodb/data");
    fs::create_dir_all(&dbpath)?;
    let conf = paths.databases.join("mongodb/mongod.conf");
    let port = state.stack.mongodb_port;
    fs::write(
        &conf,
        format!(
            "storage:\n  dbPath: {}\nnet:\n  bindIp: 127.0.0.1\n  port: {port}\n",
            dbpath.display()
        ),
    )?;
    let child = spawn_logged(
        &mongod,
        &[&format!("--config={}", conf.display())],
        Some(&root),
        &paths.logs.join("mongodb.out"),
        &paths.logs.join("mongodb.err"),
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file("mongodb"), pid)?;
    state.runtime.mongodb_pid = Some(pid);
    wait_port(port, 40)?;
    Ok(())
}

fn start_postgresql(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.postgresql_pid {
        if is_running(pid) {
            return Ok(());
        }
    }
    let root = install_root(state, "postgresql")
        .ok_or_else(|| DevStackError::Message("PostgreSQL is not installed".into()))?;
    let initdb = find_bin(&root, &["initdb", "initdb.exe"])
        .ok_or_else(|| DevStackError::Message("initdb not found".into()))?;
    let pg_ctl = find_bin(&root, &["pg_ctl", "pg_ctl.exe"])
        .ok_or_else(|| DevStackError::Message("pg_ctl not found".into()))?;
    let data = paths.databases.join("postgresql/data");
    if !data.join("PG_VERSION").exists() {
        fs::create_dir_all(data.parent().unwrap())?;
        let status = crate::services::process::command(&initdb)
            .args([
                "-D",
                &data.to_string_lossy(),
                "-U",
                "postgres",
                "--auth=trust",
                "-E",
                "UTF8",
            ])
            .status()
            .map_err(|e| DevStackError::Message(e.to_string()))?;
        if !status.success() {
            return Err(DevStackError::Message("initdb failed".into()));
        }
        // Listen on localhost only
        let conf = data.join("postgresql.conf");
        if conf.exists() {
            let mut raw = fs::read_to_string(&conf)?;
            raw.push_str(&format!(
                "\nlisten_addresses = '127.0.0.1'\nport = {}\n",
                state.stack.postgresql_port
            ));
            fs::write(&conf, raw)?;
        }
    } else {
        // Keep port in sync with stack prefs on subsequent starts.
        let conf = data.join("postgresql.conf");
        if conf.exists() {
            let raw = fs::read_to_string(&conf)?;
            let port = state.stack.postgresql_port;
            let updated = if raw.lines().any(|l| l.trim_start().starts_with("port")) {
                raw.lines()
                    .map(|l| {
                        if l.trim_start().starts_with("port") {
                            format!("port = {port}")
                        } else {
                            l.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                format!("{raw}\nport = {port}\n")
            };
            fs::write(&conf, updated)?;
        }
    }
    let log = paths.logs.join("postgresql.err");
    let status = crate::services::process::command(&pg_ctl)
        .args([
            "-D",
            &data.to_string_lossy(),
            "-l",
            &log.to_string_lossy(),
            "start",
        ])
        .status()
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    if !status.success() {
        return Err(DevStackError::Message("pg_ctl start failed".into()));
    }
    // Resolve postmaster pid
    let pidfile = data.join("postmaster.pid");
    for _ in 0..40 {
        if let Ok(raw) = fs::read_to_string(&pidfile) {
            if let Some(line) = raw.lines().next() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    state.runtime.postgresql_pid = Some(pid);
                    write_pid(&paths.pid_file("postgresql"), pid)?;
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    wait_port(state.stack.postgresql_port, 40)?;
    Ok(())
}

fn start_redis(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.redis_pid {
        if is_running(pid) {
            return Ok(());
        }
    }
    let root = install_root(state, "redis")
        .ok_or_else(|| DevStackError::Message("Redis is not installed".into()))?;
    let server = find_bin(
        &root,
        &["redis-server", "redis-server.exe", "redis-server.bat"],
    )
    .ok_or_else(|| DevStackError::Message("redis-server not found".into()))?;
    let conf_dir = paths.databases.join("redis");
    fs::create_dir_all(&conf_dir)?;
    let conf = conf_dir.join("redis.conf");
    fs::write(
        &conf,
        "bind 127.0.0.1\nport 6379\nprotected-mode yes\ndir .\ndaemonize no\n",
    )?;
    let child = spawn_logged(
        &server,
        &[&conf.to_string_lossy()],
        Some(&conf_dir),
        &paths.logs.join("redis.out"),
        &paths.logs.join("redis.err"),
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file("redis"), pid)?;
    state.runtime.redis_pid = Some(pid);
    wait_port(6379, 40)?;
    Ok(())
}

fn start_mailpit(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.mailpit_pid {
        if is_running(pid) {
            return Ok(());
        }
    }
    let root = install_root(state, "mailpit")
        .ok_or_else(|| DevStackError::Message("Mailpit is not installed".into()))?;
    let bin = find_bin(&root, &["mailpit", "mailpit.exe"])
        .ok_or_else(|| DevStackError::Message("mailpit binary not found".into()))?;
    let ui = "8025";
    let smtp = "1025";
    let child = spawn_logged(
        &bin,
        &[
            "--smtp",
            &format!("127.0.0.1:{smtp}"),
            "--listen",
            &format!("127.0.0.1:{ui}"),
        ],
        Some(&root),
        &paths.logs.join("mailpit.out"),
        &paths.logs.join("mailpit.err"),
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file("mailpit"), pid)?;
    state.runtime.mailpit_pid = Some(pid);
    wait_port(8025, 40)?;

    // Ensure mailpit.test nginx reverse-proxy exists when nginx is up
    let _ = write_mailpit_nginx(paths, state);
    // PHP sendmail hint file
    let hint = paths.php.join("mailpit-sendmail.ini");
    let _ = fs::write(
        hint,
        "; DevStack Mailpit — point PHP SMTP here\n\
         SMTP = 127.0.0.1\n\
         smtp_port = 1025\n\
         sendmail_path = \"\"\n",
    );
    Ok(())
}

pub fn write_mailpit_nginx(paths: &Paths, state: &AppState) -> Result<()> {
    let sites = paths.nginx_sites_dir();
    fs::create_dir_all(&sites)?;
    let http = state.stack.http_port;
    let https = state.stack.https_port;
    let certs = ensure_site_cert(paths, "mailpit.test")?;
    let body = format!(
        r#"server {{
  listen {http};
  listen {https} ssl;
  server_name mailpit.test;
  ssl_certificate {cert};
  ssl_certificate_key {key};
  location / {{
    proxy_pass http://127.0.0.1:8025;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }}
}}
"#,
        cert = certs.site_cert.display(),
        key = certs.site_key.display(),
    );
    fs::write(sites.join("mailpit.conf"), body)?;
    Ok(())
}

fn wait_port(port: u16, attempts: u32) -> Result<()> {
    for _ in 0..attempts {
        if port_open(port) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(DevStackError::Message(format!(
        "timed out waiting for port {port}"
    )))
}

#[allow(dead_code)]
pub fn find_installed_version<'a>(state: &'a AppState, id: &str, version: &str) -> Option<&'a crate::config::InstalledRecord> {
    find_installed(state, id, version)
}
