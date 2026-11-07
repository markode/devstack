//! MySQL / MariaDB process lifecycle + huge-DB tuned config.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::config::{find_installed, latest_installed, AppState, Paths};
use crate::error::{DevStackError, Result};
use crate::services::process::{
    find_bin, is_running, read_pid, spawn_logged, stop_pid, write_pid,
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
    let body = format!(
        r#"[mysqld]
port={port}
bind-address=127.0.0.1
datadir={datadir}
socket={sock}
max_allowed_packet=1G
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
socket={sock}
default-character-set=utf8mb4
"#,
        datadir = datadir.display(),
        sock = paths.databases.join(format!("{engine}.sock")).display(),
    );
    fs::write(&conf, body)?;
    Ok(conf)
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
    // Prefer mariadb-install-db / mysql_install_db / mysqld --initialize-insecure
    if let Some(install) = find_bin(root, &["mariadb-install-db", "mysql_install_db"]) {
        let out = crate::services::process::command(&install)
            .args([
                format!("--datadir={}", datadir.display()),
                format!("--basedir={}", root.display()),
                "--auth-root-authentication-method=normal".into(),
            ])
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
    // MySQL-style initialize (not supported by MariaDB)
    let mysqld = server_bin(root)?;
    if mysqld.file_name().and_then(|n| n.to_str()) == Some("mysqld")
        || mysqld.file_name().and_then(|n| n.to_str()) == Some("mysqld.exe")
    {
        let _ = crate::services::process::command(&mysqld)
            .args([
                format!("--defaults-file={}", conf.display()),
                "--initialize-insecure".into(),
            ])
            .status();
    }
    if !datadir.join("mysql").exists() {
        return Err(DevStackError::Message(
            "failed to initialize database datadir".into(),
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
    let log_out = paths.logs.join(format!("{engine}.out"));
    let log_err = paths.logs.join(format!("{engine}.err"));
    let defaults = format!("--defaults-file={}", conf.display());
    let child = spawn_logged(
        &mysqld,
        &[&defaults],
        Some(&root),
        &log_out,
        &log_err,
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file(engine), pid)?;
    state.runtime.database_pid = Some(pid);
    state.runtime.database_engine = Some(engine.into());
    state.active_database = Some(engine.into());

    // Wait for socket + accept connections (InnoDB init can take >10s on cold start)
    let sock = paths.databases.join(format!("{engine}.sock"));
    let client = client_bin(&root).ok();
    for _ in 0..150 {
        if sock.exists() {
            if let Some(cli) = &client {
                let ok = crate::services::process::command(cli)
                    .args([
                        format!("--socket={}", sock.display()),
                        "-uroot".into(),
                        "-e".into(),
                        "SELECT 1".into(),
                    ])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if ok {
                    return Ok(());
                }
            } else {
                thread::sleep(Duration::from_millis(300));
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(DevStackError::Message(format!(
        "{engine} started but never became ready (no socket/accept)"
    )))
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
    if let Some(pid) = state.runtime.database_pid.take() {
        stop_pid(pid)?;
    }
    for engine in ["mariadb", "mysql"] {
        if let Some(pid) = read_pid(&paths.pid_file(engine)) {
            let _ = stop_pid(pid);
        }
    }
    state.runtime.database_engine = None;
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
    let sock = paths.databases.join(format!("{engine}.sock"));
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
    let status = crate::services::process::command(&client)
        .args([
            format!("--socket={}", sock.display()),
            "-uroot".into(),
            "-e".into(),
            sql,
        ])
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
        let status2 = crate::services::process::command(&client)
            .args([
                format!("--socket={}", sock.display()),
                "-uroot".into(),
                "-e".into(),
                sql2,
            ])
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
    let sock = paths.databases.join(format!("{engine}.sock"));
    let conf = pma.path.join("config.inc.php");
    let signon = pma.path.join("devstack-signon.php");
    fs::write(
        &signon,
        r#"<?php
session_start();
if (!empty($_GET['db'])) {
  $_SESSION['PMA_single_signon_user'] = $_GET['user'] ?? 'root';
  $_SESSION['PMA_single_signon_password'] = $_GET['pass'] ?? '';
  $_SESSION['PMA_single_signon_host'] = 'localhost';
  $_SESSION['PMA_single_signon_only_database'] = $_GET['db'];
}
header('Location: index.php');
"#,
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
$cfg['Servers'][$i]['host'] = 'localhost';
$cfg['Servers'][$i]['socket'] = '{sock}';
$cfg['Servers'][$i]['AllowNoPassword'] = true;
$cfg['Servers'][$i]['compress'] = false;
$cfg['Servers'][$i]['AllowRoot'] = true;
// Also keep a cookie server for manual login
$i++;
$cfg['Servers'][$i]['auth_type'] = 'cookie';
$cfg['Servers'][$i]['host'] = 'localhost';
$cfg['Servers'][$i]['socket'] = '{sock}';
$cfg['Servers'][$i]['AllowNoPassword'] = true;
"#,
        sock = sock.display()
    );
    fs::write(conf, cfg)?;
    Ok(())
}
