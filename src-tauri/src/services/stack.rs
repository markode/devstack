//! Orchestrate DNS + TLS + database + PHP + Nginx for the whole stack.

use serde::Serialize;

use crate::config::{save_state, AppState, Paths};
use crate::error::Result;
use crate::services::database::{configure_phpmyadmin, start_database, stop_database};
use crate::services::dns::{start_dns, stop_dns, sync_hosts};
use crate::services::nginx::{reload_nginx, start_nginx, stop_nginx};
use crate::services::php_runtime::register_system_php;
use crate::services::process::is_running;
use crate::services::sites::refresh_site_metadata;
use crate::services::tls::{ensure_root_ca, trust_ca_via_helper};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackStatus {
    pub nginx: bool,
    pub dns: bool,
    pub database: bool,
    pub database_engine: Option<String>,
    pub http_port: u16,
    pub https_port: u16,
    pub php_versions: Vec<String>,
}

pub fn stack_status(state: &AppState) -> StackStatus {
    StackStatus {
        nginx: state
            .runtime
            .nginx_pid
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
        php_versions: state.runtime.php_pids.keys().cloned().collect(),
    }
}

pub fn apply_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    refresh_site_metadata(state);
    ensure_root_ca(paths)?;
    let _ = trust_ca_via_helper(paths);
    let _ = sync_hosts(paths, state);

    // Prefer high ports when unprivileged bind likely fails
    if state.stack.http_port == 80 {
        // keep 80/443; start_nginx will fail loudly — smoke can set 8080/8443
    }

    // DNS on 5353 (macOS resolver) — hosts file covers Linux/Windows
    let _ = start_dns(paths, state, 5353);

    let engine = state
        .active_database
        .clone()
        .or_else(|| {
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

    // Ensure at least system PHP is registered on Linux smoke hosts
    if !state.installed.contains_key("php") {
        let _ = register_system_php(state, paths);
    }

    start_nginx(paths, state)?;
    save_state(&paths.state_file, state)?;
    Ok(stack_status(state))
}

pub fn start_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    apply_stack(paths, state)
}

pub fn stop_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    let _ = stop_nginx(paths, state);
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
    let _ = sync_hosts(paths, state);
    reload_nginx(paths, state)?;
    save_state(&paths.state_file, state)?;
    Ok(stack_status(state))
}
