//! Orchestrate DNS + TLS + database + PHP + Nginx/Apache for the whole stack.

use serde::Serialize;

use crate::config::{save_state, AppState, Paths};
use crate::error::Result;
use crate::services::database::{configure_phpmyadmin, start_database, stop_database};
use crate::services::dns::{start_dns, stop_dns, sync_hosts_and_trust_ca};
use crate::services::httpd::{reload_apache, start_apache, stop_apache};
use crate::services::nginx::{reload_nginx, start_nginx, stop_nginx};
use crate::services::php_runtime::register_system_php;
use crate::services::process::is_running;
use crate::services::sites::refresh_site_metadata;
use crate::services::tls::ensure_root_ca;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackStatus {
    pub nginx: bool,
    pub apache: bool,
    pub dns: bool,
    pub database: bool,
    pub database_engine: Option<String>,
    pub http_port: u16,
    pub https_port: u16,
    pub apache_http_port: u16,
    pub apache_https_port: u16,
    pub mysql_port: u16,
    pub mariadb_port: u16,
    pub postgresql_port: u16,
    pub mongodb_port: u16,
    pub php_fpm_port: u16,
    pub coexistence_mode: bool,
    pub php_versions: Vec<String>,
}

pub fn stack_status(state: &AppState) -> StackStatus {
    StackStatus {
        nginx: state
            .runtime
            .nginx_pid
            .map(is_running)
            .unwrap_or(false),
        apache: state
            .runtime
            .apache_pid
            .map(is_running)
            .unwrap_or(false),
        dns: state.runtime.dns_pid.map(is_running).unwrap_or(false),
        database: state
            .runtime
            .database_pid
            .map(is_running)
            .unwrap_or(false),
        database_engine: state.runtime.database_engine.clone(),
        http_port: state.stack.http_port,
        https_port: state.stack.https_port,
        apache_http_port: state.stack.apache_http_port,
        apache_https_port: state.stack.apache_https_port,
        mysql_port: state.stack.mysql_port,
        mariadb_port: state.stack.mariadb_port,
        postgresql_port: state.stack.postgresql_port,
        mongodb_port: state.stack.mongodb_port,
        php_fpm_port: state.stack.php_fpm_port,
        coexistence_mode: state.stack.coexistence_mode,
        php_versions: state.runtime.php_pids.keys().cloned().collect(),
    }
}

pub fn apply_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    refresh_site_metadata(state);
    ensure_root_ca(paths)?;
    // Single UAC on Windows for hosts + CA trust; unix uses helper/sudo separately.
    let _ = sync_hosts_and_trust_ca(paths, state);

    let _ = start_dns(paths, state, 5353);

    let engine = state.active_database.clone().or_else(|| {
        if state.installed.contains_key("mariadb") {
            Some("mariadb".into())
        } else if state.installed.contains_key("mysql") {
            Some("mysql".into())
        } else {
            None
        }
    });
    if let Some(engine) = engine {
        start_database(paths, state, &engine)?;
        let _ = configure_phpmyadmin(paths, state);
    }

    if !state.installed.contains_key("php") {
        let _ = register_system_php(state, paths);
    }

    // Both front-ends can run together on different ports (see docs/apache.md).
    // Surface the first hard failure so PortConflict / UI dialogs can react.
    let nginx_err = start_nginx(paths, state).err();
    let apache_err = start_apache(paths, state).err();
    save_state(&paths.state_file, state)?;
    if let Some(e) = nginx_err.or(apache_err) {
        return Err(e);
    }
    Ok(stack_status(state))
}

pub fn start_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    apply_stack(paths, state)
}

pub fn stop_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    let _ = stop_nginx(paths, state);
    let _ = stop_apache(paths, state);
    let _ = stop_database(paths, state);
    let _ = stop_dns(paths, state);
    for version in state.runtime.php_pids.keys().cloned().collect::<Vec<_>>() {
        let _ = crate::services::php_runtime::stop_php(paths, state, &version);
    }
    save_state(&paths.state_file, state)?;
    Ok(stack_status(state))
}

pub fn reload_sites(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    refresh_site_metadata(state);
    let _ = ensure_root_ca(paths);
    let _ = sync_hosts_and_trust_ca(paths, state);
    let nginx_err = reload_nginx(paths, state).err();
    let apache_err = reload_apache(paths, state).err();
    save_state(&paths.state_file, state)?;
    if let Some(e) = nginx_err.or(apache_err) {
        return Err(e);
    }
    Ok(stack_status(state))
}
