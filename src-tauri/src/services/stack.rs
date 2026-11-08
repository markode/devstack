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
    // Fast path: do NOT re-trust CA or rewrite hosts here — callers that add
    // domains should use sync_hosts_if_needed / sync_hosts_and_trust_ca once.
    let _ = ensure_root_ca(paths);
    let nginx_err = reload_nginx(paths, state).err();
    let apache_err = reload_apache(paths, state).err();
    save_state(&paths.state_file, state)?;
    if let Some(e) = nginx_err.or(apache_err) {
        return Err(e);
    }
    Ok(stack_status(state))
}

/// Apply a site environment change with minimal work:
/// cert cache hit, hosts only if missing, reload only the affected front-end(s).
pub fn apply_site_environment_change(
    paths: &Paths,
    state: &mut AppState,
    site_id: &str,
    prev_web_server: Option<String>,
    prev_https: bool,
) -> Result<StackStatus> {
    refresh_site_metadata(state);
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?
        .clone();

    if site.https {
        // Cached after first mint — rcgen in-process, no openssl/UAC.
        crate::services::tls::ensure_site_cert(paths, &site.domain)?;
    }

    // Hosts only when the domain is missing; never re-prompt CA trust on toggles.
    crate::services::dns::sync_hosts_if_needed(paths, state)?;

    let now_apache = crate::services::httpd::site_uses_apache(&site);
    let was_apache = matches!(
        prev_web_server
            .as_deref()
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("apache") | Some("httpd")
    );

    // PHP runtime for the site (needed when version changes or server switches).
    if let Some(ver) = site.php_version.as_deref() {
        let _ = crate::services::php_runtime::start_php(paths, state, ver);
    }

    if now_apache != was_apache {
        // Front-end switch: regenerate both configs; stop Apache when unused.
        reload_nginx(paths, state)?;
        if now_apache || state.sites.iter().any(crate::services::httpd::site_uses_apache) {
            reload_apache(paths, state)?;
        } else {
            let _ = crate::services::httpd::stop_apache(paths, state);
        }
        rewrite_wp_urls_for_server_switch(paths, state, &site, was_apache, now_apache)?;
    } else if now_apache {
        reload_apache(paths, state)?;
        if site.https != prev_https {
            rewrite_wp_urls_for_https_toggle(paths, state, &site, prev_https)?;
        }
    } else {
        reload_nginx(paths, state)?;
        if site.https != prev_https {
            rewrite_wp_urls_for_https_toggle(paths, state, &site, prev_https)?;
        }
    }

    save_state(&paths.state_file, state)?;
    Ok(stack_status(state))
}

fn rewrite_wp_urls_for_server_switch(
    paths: &Paths,
    state: &AppState,
    site: &crate::config::SiteRecord,
    was_apache: bool,
    now_apache: bool,
) -> Result<()> {
    let Some(db) = site.database_name.as_deref() else {
        return Ok(());
    };
    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into());
    if !matches!(engine.as_str(), "mysql" | "mariadb") {
        return Ok(());
    }
    let old_port = if was_apache {
        state.stack.apache_https_port
    } else {
        state.stack.https_port
    };
    let new_port = if now_apache {
        state.stack.apache_https_port
    } else {
        state.stack.https_port
    };
    if old_port == new_port {
        return Ok(());
    }
    let from = format!(":{old_port}");
    let to = format!(":{new_port}");
    let _ = crate::services::db_files::mysql_simple_replace_pub(paths, state, &engine, db, &from, &to);
    Ok(())
}

fn rewrite_wp_urls_for_https_toggle(
    paths: &Paths,
    state: &AppState,
    site: &crate::config::SiteRecord,
    prev_https: bool,
) -> Result<()> {
    let Some(db) = site.database_name.as_deref() else {
        return Ok(());
    };
    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into());
    if !matches!(engine.as_str(), "mysql" | "mariadb") {
        return Ok(());
    }
    let (https_port, http_port) = if crate::services::httpd::site_uses_apache(site) {
        (state.stack.apache_https_port, state.stack.apache_http_port)
    } else {
        (state.stack.https_port, state.stack.http_port)
    };
    let https_url = crate::services::ports::format_url("https", &site.domain, https_port);
    let http_url = crate::services::ports::format_url("http", &site.domain, http_port);
    let (from, to) = if site.https && !prev_https {
        (http_url, https_url)
    } else if !site.https && prev_https {
        (https_url, http_url)
    } else {
        return Ok(());
    };
    // Options-only — full-table REPLACE scans seeded demo tables and blew the
    // <3s toggle budget on Windows. siteurl/home is what the app Open URL uses.
    let _ = crate::services::db_files::mysql_wp_options_url_replace(
        paths, state, &engine, db, &from, &to,
    );
    Ok(())
}
