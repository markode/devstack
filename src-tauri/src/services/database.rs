//! MySQL / MariaDB process lifecycle + huge-DB tuned config.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{find_installed, latest_installed, AppState, Paths};
use crate::error::{DevStackError, Result};
use crate::paths_util::{posix_path, quoted_conf_path};
use crate::services::process::{
    find_bin, is_running, read_pid, spawn_logged_attached, stop_pid, write_pid,
};


pub fn engine_install_dir(state: &AppState, engine: &str) -> Result<PathBuf> {
    let preferred = match engine {
        "mysql" => state.stack.mysql_version.clone(),
        _ => state.stack.mariadb_version.clone(),
    };
    if let Some(ver) = preferred {
        if let Some(rec) = find_installed(state, engine, &ver) {
            return Ok(rec.path.clone());
        }
    }
    latest_installed(state, engine)
        .map(|r| r.path.clone())
        .ok_or_else(|| DevStackError::Message(format!("{engine} is not installed")))
}

pub fn write_db_config(
    paths: &Paths,
    engine: &str,
    datadir: &Path,
    port: u16,
) -> Result<PathBuf> {
    let conf_dir = paths.databases.join(engine);
    fs::create_dir_all(&conf_dir)?;
    fs::create_dir_all(datadir)?;
    let conf = conf_dir.join("my.cnf");
    let sock = paths.databases.join(format!("{engine}.sock"));
    // Forward slashes + quotes — Windows paths with spaces break unquoted datadir=.
    // Omit socket= on Windows (named-pipe confusion); TCP only.
    let socket_mysqld = if cfg!(windows) {
        String::new()
    } else {
        format!("socket={}\n", quoted_conf_path(&sock))
    };
    let socket_client = if cfg!(windows) {
        String::new()
    } else {
        format!("socket={}\n", quoted_conf_path(&sock))
    };
    let body = format!(
        r#"[mysqld]
port={port}
bind-address=127.0.0.1
datadir={datadir}
{socket_mysqld}max_allowed_packet=1G
innodb_buffer_pool_size=512M
innodb_log_file_size=256M
innodb_log_buffer_size=64M
innodb_flush_log_at_trx_commit=2
wait_timeout=28800
interactive_timeout=28800
net_read_timeout=600
net_write_timeout=600
max_connections=200
skip_name_resolve=ON
character-set-server=utf8mb4
collation-server=utf8mb4_unicode_ci

[client]
port={port}
host=127.0.0.1
{socket_client}default-character-set=utf8mb4
"#,
        datadir = quoted_conf_path(datadir),
        socket_mysqld = socket_mysqld,
        socket_client = socket_client,
    );
    fs::write(&conf, body)?;
    Ok(conf)
}

/// Root client connect args — TCP on Windows (no Unix sockets), socket elsewhere.
pub fn mysql_root_args(paths: &Paths, state: &AppState, engine: &str) -> Vec<String> {
    let mut args = mysql_root_connect_base(paths, state, engine);
    // Short connect timeout so readiness polls don't hang for minutes.
    // Only the mysql/mariadb *client* accepts this; mysqldump rejects it.
    args.insert(args.len().saturating_sub(1), "--connect-timeout=2".into());
    args
}

/// Connect args safe for mysqldump / mariadb-dump (no `--connect-timeout`).
pub fn mysql_dump_root_args(paths: &Paths, state: &AppState, engine: &str) -> Vec<String> {
    mysql_root_connect_base(paths, state, engine)
}

fn mysql_root_connect_base(paths: &Paths, state: &AppState, engine: &str) -> Vec<String> {
    let port = crate::services::ports::engine_db_port(&state.stack, engine);
    if cfg!(windows) {
        vec![
            "--protocol=TCP".into(),
            "--host=127.0.0.1".into(),
            format!("--port={port}"),
            "-uroot".into(),
        ]
    } else {
        let sock = socket_path(paths, engine);
        if sock.exists() {
            vec![format!("--socket={}", sock.display()), "-uroot".into()]
        } else {
            vec![
                "--host=127.0.0.1".into(),
                format!("--port={port}"),
                "-uroot".into(),
            ]
        }
    }
}

fn server_bin(root: &Path) -> Result<PathBuf> {
    find_bin(root, &["mariadbd", "mysqld", "mysqld.exe", "mariadbd.exe"]).ok_or_else(|| {
        DevStackError::Message(format!("mysqld/mariadbd not found under {}", root.display()))
    })
}

pub fn client_bin_for(root: &Path) -> Result<PathBuf> {
    find_bin(root, &["mariadb", "mysql", "mysql.exe", "mariadb.exe"]).ok_or_else(|| {
        DevStackError::Message(format!("mysql client not found under {}", root.display()))
    })
}

pub fn dump_bin_for(root: &Path) -> Result<PathBuf> {
    find_bin(
        root,
        &[
            "mariadb-dump",
            "mysqldump",
            "mysqldump.exe",
            "mariadb-dump.exe",
        ],
    )
    .ok_or_else(|| {
        DevStackError::Message(format!(
            "mysqldump not found under {}",
            root.display()
        ))
    })
}

pub fn socket_path(paths: &Paths, engine: &str) -> PathBuf {
    paths.databases.join(format!("{engine}.sock"))
}

fn client_bin(root: &Path) -> Result<PathBuf> {
    client_bin_for(root)
}

fn install_db_if_needed(root: &Path, datadir: &Path, conf: &Path) -> Result<()> {
    if datadir.join("mysql").exists() {
        return Ok(());
    }
    fs::create_dir_all(datadir)?;
    // Prefer mariadb-install-db / mysql_install_db (include .exe for Windows).
    if let Some(install) = find_bin(
        root,
        &[
            "mysql_install_db.exe",
            "mariadb-install-db.exe",
            "mariadb-install-db",
            "mysql_install_db",
        ],
    ) {
        // Windows install-db rejects Unix-only auth flags — keep args minimal.
        let mut args = vec![
            format!("--datadir={}", datadir.display()),
            format!("--basedir={}", root.display()),
        ];
        if !cfg!(windows) {
            args.push("--auth-root-authentication-method=normal".into());
        }
        // Windows mysql_install_db.exe rejects --basedir= as an unknown variable.
        let args: Vec<String> = if cfg!(windows) {
            vec![format!("--datadir={}", datadir.display())]
        } else {
            args
        };
        let out = crate::services::process::command(&install)
            .args(&args)
            .current_dir(root)
            .output();
        if let Ok(o) = &out {
            if !o.status.success() {
                eprintln!(
                    "mariadb-install-db: {}",
                    String::from_utf8_lossy(&o.stderr)
                );
            }
        }
        if datadir.join("mysql").exists() {
            return Ok(());
        }
    }
    // MySQL-only: mysqld --initialize-insecure (MariaDB rejects this flag).
    let mysqld = server_bin(root)?;
    let name = mysqld
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == "mysqld" || name == "mysqld.exe" {
        let out = crate::services::process::command(&mysqld)
            .args([
                format!("--defaults-file={}", conf.display()),
                "--initialize-insecure".into(),
            ])
            .current_dir(root)
            .output()
            .map_err(|e| DevStackError::Message(format!("initialize-insecure failed: {e}")))?;
        if !out.status.success() && !datadir.join("mysql").exists() {
            return Err(DevStackError::Message(format!(
                "mysqld --initialize-insecure failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
    }
    if !datadir.join("mysql").exists() {
        return Err(DevStackError::Message(
            "failed to initialize database datadir (install-db / initialize-insecure)".into(),
        ));
    }
    Ok(())
}

pub fn start_database(paths: &Paths, state: &mut AppState, engine: &str) -> Result<()> {
    if let Some(pid) = state.runtime.database_pid {
        if is_running(pid) && state.runtime.database_engine.as_deref() == Some(engine) {
            return Ok(());
        }
        let _ = stop_database(paths, state);
    }
    crate::services::ports::ensure_ports_free(&state.stack, engine)?;

    let root = engine_install_dir(state, engine)?;
    let datadir = paths.databases.join(engine).join("data");
    let port = crate::services::ports::engine_db_port(&state.stack, engine);
    let conf = write_db_config(paths, engine, &datadir, port)?;
    install_db_if_needed(&root, &datadir, &conf)?;

    let mysqld = server_bin(&root)?;
    if !cfg!(windows) {
        // Stale socket from a killed server would satisfy the readiness probe early.
        let _ = fs::remove_file(socket_path(paths, engine));
    }
    let log_out = paths.logs.join(format!("{engine}.out"));
    let log_err = paths.logs.join(format!("{engine}.err"));
    let defaults = format!("--defaults-file={}", conf.display());
    // Attached (no DETACHED_PROCESS) + --console on Windows so mysqld stays up
    // and writes to our redirected stderr.
    let child = if cfg!(windows) {
        spawn_logged_attached(
            &mysqld,
            &[&defaults, "--console"],
            Some(&root),
            &log_out,
            &log_err,
        )?
    } else {
        spawn_logged_attached(&mysqld, &[&defaults], Some(&root), &log_out, &log_err)?
    };
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file(engine), pid)?;
    state.runtime.database_pid = Some(pid);
    state.runtime.database_engine = Some(engine.into());
    state.active_database = Some(engine.into());

    // Wait for accept. Cold InnoDB init can take >10s so the ceiling stays 45s, but
    // the poll starts at 5ms and backs off to 50ms so warm restarts return fast.
    // A cheap TCP/socket probe gates the (slower) client `SELECT 1` confirmation.
    let client = client_bin(&root).ok();
    let connect = mysql_root_args(paths, state, engine);
    let sock = socket_path(paths, engine);
    let mut died = false;
    let ready = crate::services::perf::poll_backoff(
        Duration::from_millis(5),
        Duration::from_millis(50),
        Duration::from_secs(45),
        || {
            if !is_running(pid) {
                died = true;
                return Some(());
            }
            if !db_accepting(&sock, port) {
                return None;
            }
            match &client {
                Some(cli) => {
                    let mut args = connect.clone();
                    args.push("-e".into());
                    args.push("SELECT 1".into());
                    crate::services::process::command(cli)
                        .args(&args)
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false)
                        .then_some(())
                }
                None => Some(()),
            }
        },
    )
    .is_some();
    if died {
        let err = fs::read_to_string(&log_err).unwrap_or_default();
        let out = fs::read_to_string(&log_out).unwrap_or_default();
        return Err(DevStackError::Message(format!(
            "{engine} exited during startup.\nstderr:\n{err}\nstdout:\n{out}"
        )));
    }
    if ready {
        return Ok(());
    }
    let err = fs::read_to_string(&log_err).unwrap_or_default();
    Err(DevStackError::Message(format!(
        "{engine} started but never became ready (no accept on port {port}).\nstderr:\n{err}"
    )))
}

/// Cheap readiness probe: unix socket file present or TCP port accepting.
/// mysqld/mariadbd only create both once they are ready for connections.
fn db_accepting(sock: &Path, port: u16) -> bool {
    if !cfg!(windows) && sock.exists() {
        return true;
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(50)).is_ok()
}

pub fn stop_database(paths: &Paths, state: &mut AppState) -> Result<()> {
    if state.db_files.auto_export_on_stop {
        let progress = crate::services::progress::noop_job_progress();
        let _ = crate::services::db_files::export_all_running(
            paths,
            state,
            &progress,
            "export-on-stop",
        );
    }
    // Prefer graceful shutdown so InnoDB flushes cleanly (never kill foreign PIDs).
    let engine = state.runtime.database_engine.clone();
    if let Some(ref eng) = engine {
        let _ = mysqladmin_shutdown(paths, state, eng);
    }
    if let Some(pid) = state.runtime.database_pid.take() {
        // Give mysqld a moment after mysqladmin; force only if still alive.
        let _ = crate::services::perf::poll_backoff(
            Duration::from_millis(5),
            Duration::from_millis(50),
            Duration::from_secs(2),
            || (!is_running(pid)).then_some(()),
        );
        if is_running(pid) {
            let _ = stop_pid(pid);
        }
    }
    for eng in ["mariadb", "mysql"] {
        if let Some(pid) = read_pid(&paths.pid_file(eng)) {
            if is_running(pid) {
                let _ = stop_pid(pid);
            }
        }
    }
    state.runtime.database_engine = None;
    Ok(())
}

fn mysqladmin_shutdown(paths: &Paths, state: &AppState, engine: &str) -> Result<()> {
    let root = engine_install_dir(state, engine)?;
    let admin = find_bin(
        &root,
        &["mysqladmin", "mariadb-admin", "mysqladmin.exe", "mariadb-admin.exe"],
    )
    .ok_or_else(|| DevStackError::Message("mysqladmin not found".into()))?;
    let mut args = mysql_root_args(paths, state, engine);
    args.push("shutdown".into());
    let _ = crate::services::process::command(&admin).args(&args).status();
    Ok(())
}

pub fn ensure_database(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db_name: &str,
    user: &str,
    pass: &str,
) -> Result<()> {
    let root = engine_install_dir(state, engine)?;
    let client = client_bin(&root)?;
    let connect = mysql_root_args(paths, state, engine);
    // Always (re)set password — CREATE USER IF NOT EXISTS leaves stale passwords around.
    let sql = format!(
        "CREATE DATABASE IF NOT EXISTS `{db_name}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n\
         CREATE USER IF NOT EXISTS '{user}'@'localhost' IDENTIFIED BY '{pass}';\n\
         CREATE USER IF NOT EXISTS '{user}'@'127.0.0.1' IDENTIFIED BY '{pass}';\n\
         ALTER USER '{user}'@'localhost' IDENTIFIED BY '{pass}';\n\
         ALTER USER '{user}'@'127.0.0.1' IDENTIFIED BY '{pass}';\n\
         GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{user}'@'localhost';\n\
         GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{user}'@'127.0.0.1';\n\
         FLUSH PRIVILEGES;"
    );
    let mut args = connect.clone();
    args.push("-e".into());
    args.push(sql);
    let status = crate::services::process::command(&client)
        .args(&args)
        .status()
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    if !status.success() {
        // try without IF NOT EXISTS user syntax for older servers
        let sql2 = format!(
            "CREATE DATABASE IF NOT EXISTS `{db_name}`;\n\
             GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{user}'@'localhost' IDENTIFIED BY '{pass}';\n\
             GRANT ALL PRIVILEGES ON `{db_name}`.* TO '{user}'@'127.0.0.1' IDENTIFIED BY '{pass}';\n\
             FLUSH PRIVILEGES;"
        );
        let mut args2 = connect;
        args2.push("-e".into());
        args2.push(sql2);
        let status2 = crate::services::process::command(&client)
            .args(&args2)
            .status()
            .map_err(|e| DevStackError::Message(e.to_string()))?;
        if !status2.success() {
            return Err(DevStackError::Message(format!(
                "failed to create database `{db_name}`"
            )));
        }
    }
    Ok(())
}

pub fn configure_phpmyadmin(paths: &Paths, state: &AppState) -> Result<()> {
    let Some(pma) = latest_installed(state, "phpmyadmin") else {
        return Ok(());
    };
    let engine = state
        .active_database
        .clone()
        .or_else(|| state.runtime.database_engine.clone())
        .unwrap_or_else(|| "mariadb".into());
    let port = crate::services::ports::engine_db_port(&state.stack, &engine);
    let sock = paths.databases.join(format!("{engine}.sock"));
    let conf = pma.path.join("config.inc.php");
    let signon = pma.path.join("devstack-signon.php");
    // Windows: TCP only. Unix: socket when available.
    let (host, connect_type, socket_line) = if cfg!(windows) {
        (
            "127.0.0.1".to_string(),
            "tcp".to_string(),
            String::new(),
        )
    } else {
        (
            "localhost".to_string(),
            "socket".to_string(),
            format!(
                "$cfg['Servers'][$i]['socket'] = '{}';\n",
                posix_path(&sock)
            ),
        )
    };
    crate::services::pma_signon::ensure_tokens_dir(paths)?;
    let tokens_dir = crate::services::pma_signon::tokens_dir_conf(paths);
    fs::write(
        &signon,
        format!(
            r#"<?php
// DevStack one-time sign-on — credentials come from a short-lived token file,
// never from ?pass= in the URL.
session_name('SignonSession');
session_start();
$token = isset($_GET['token']) ? preg_replace('/[^a-f0-9]/', '', strtolower((string)$_GET['token'])) : '';
$dir = '{tokens_dir}';
$ttl = 60;
if ($token !== '' && is_dir($dir)) {{
  $file = $dir . DIRECTORY_SEPARATOR . $token . '.json';
  if (is_file($file)) {{
    $age = time() - (int)@filemtime($file);
    $raw = @file_get_contents($file);
    @unlink($file);
    if ($raw !== false && $age >= 0 && $age <= $ttl) {{
      $data = json_decode($raw, true);
      if (is_array($data)) {{
        $_SESSION['PMA_single_signon_user'] = $data['user'] ?? 'root';
        $_SESSION['PMA_single_signon_password'] = $data['pass'] ?? '';
        $_SESSION['PMA_single_signon_host'] = $data['host'] ?? '{host}';
        $_SESSION['PMA_single_signon_port'] = (string)($data['port'] ?? '{port}');
        if (!empty($data['db'])) {{
          $_SESSION['PMA_single_signon_only_database'] = $data['db'];
        }}
      }}
    }}
  }}
}}
header('Location: index.php');
exit;
"#,
            tokens_dir = tokens_dir.replace('\\', "\\\\").replace('\'', "\\'"),
            host = host,
            port = port,
        ),
    )?;
    let cfg = format!(
        r#"<?php
$cfg['blowfish_secret'] = 'devstack-local-secret-change-me-32b';
$cfg['ExecTimeLimit'] = 0;
$cfg['MemoryLimit'] = '0';
$cfg['UploadDir'] = '';
$cfg['SaveDir'] = '';
$cfg['LoginCookieValidity'] = 36000;
$cfg['MaxSizeForInputField'] = 0;
$i = 0;
$i++;
$cfg['Servers'][$i]['auth_type'] = 'signon';
$cfg['Servers'][$i]['SignonSession'] = 'SignonSession';
$cfg['Servers'][$i]['SignonURL'] = 'devstack-signon.php';
$cfg['Servers'][$i]['host'] = '{host}';
$cfg['Servers'][$i]['port'] = '{port}';
$cfg['Servers'][$i]['connect_type'] = '{connect_type}';
{socket_line}$cfg['Servers'][$i]['AllowNoPassword'] = true;
$cfg['Servers'][$i]['compress'] = false;
$cfg['Servers'][$i]['AllowRoot'] = true;
// Also keep a cookie server for manual login
$i++;
$cfg['Servers'][$i]['auth_type'] = 'cookie';
$cfg['Servers'][$i]['host'] = '{host}';
$cfg['Servers'][$i]['port'] = '{port}';
$cfg['Servers'][$i]['connect_type'] = '{connect_type}';
{socket_line}$cfg['Servers'][$i]['AllowNoPassword'] = true;
"#,
        host = host,
        port = port,
        connect_type = connect_type,
        socket_line = socket_line,
    );
    fs::write(conf, cfg)?;
    Ok(())
}

#[cfg(test)]
mod perf_tests {
    use super::*;

    #[test]
    fn db_accepting_detects_listening_tcp_port() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let missing = std::path::Path::new("/definitely/not/here.sock");
        assert!(db_accepting(missing, port));
        drop(listener);
        assert!(!db_accepting(missing, port));
    }
}
