//! Orchestrate DNS + TLS + database + PHP + Nginx/Apache for the whole stack.

use serde::Serialize;

use crate::config::{save_state, AppState, Paths};
use crate::error::Result;
use crate::services::database::{configure_phpmyadmin, start_database, stop_database};
use crate::services::dns::{start_dns, stop_dns, sync_hosts_and_trust_ca};
use crate::services::httpd::{reload_apache, start_apache, stop_apache};
use crate::services::nginx::{reload_nginx, start_nginx, stop_nginx};
use crate::services::php_runtime::register_system_php;
use crate::services::ports::{
    auto_separate_busy_ports, owned_runtime_pids, PortRemapNotice, PortSnapshot,
};
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
    /// Banner lines when ports were auto-remapped beside WAMP/MAMP/etc.
    #[serde(default)]
    pub port_notice: Vec<String>,
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
        port_notice: state.runtime.port_notice.clone(),
    }
}

/// Mark each site running/stopped from the front-end that actually serves it.
pub fn refresh_site_process_status(state: &mut AppState) {
    let nginx_up = state
        .runtime
        .nginx_pid
        .map(is_running)
        .unwrap_or(false);
    let apache_up = state
        .runtime
        .apache_pid
        .map(is_running)
        .unwrap_or(false);
    for site in &mut state.sites {
        let uses_apache = crate::services::httpd::site_uses_apache(site);
        // Don't overwrite a more specific error/warning set by reachability probes.
        if site.status == "error" || site.status == "warning" {
            if (uses_apache && apache_up) || (!uses_apache && nginx_up) {
                // Leave error/warning until the next probe refreshes it.
                continue;
            }
        }
        site.status = if uses_apache {
            if apache_up {
                "running"
            } else {
                "stopped"
            }
        } else if nginx_up {
            "running"
        } else {
            "stopped"
        }
        .into();
    }
}

pub fn apply_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    refresh_site_metadata(state);

    state.runtime.stack_intentionally_stopped = false;
    state.runtime.last_start_at = Some(chrono::Utc::now().to_rfc3339());
    state.runtime.last_start_error = None;

    // Auto-pick free ports when WAMP/MAMP/IIS hold 80/443/3306 — no modal required.
    // Ignore our own nginx/httpd/mysqld PIDs so a live stack never remaps onto itself.
    let before = PortSnapshot::from_stack(&state.stack);
    let owned = owned_runtime_pids(&state.runtime);
    let runtime_snap = state.runtime.clone();
    let remap: PortRemapNotice =
        auto_separate_busy_ports(&mut state.stack, &owned, &runtime_snap);
    if remap.changed {
        state.runtime.port_notice = remap.messages.clone();
        for msg in &remap.messages {
            crate::applog::info("ports", msg);
        }
        save_state(&paths.state_file, state)?;
    }

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
        if let Err(e) = start_database(paths, state, &engine) {
            state.runtime.last_start_error = Some(e.to_string());
            save_state(&paths.state_file, state)?;
            return Err(e);
        }
        let _ = configure_phpmyadmin(paths, state);
    }

    // WP siteurl/home rewrite needs a live DB — run after start_database.
    if remap.changed {
        rewrite_all_wp_urls_for_port_remap(paths, state, before)?;
    }

    if !state.installed.contains_key("php") {
        let _ = register_system_php(state, paths);
    }

    // Both front-ends can run together on different ports (see docs/apache.md).
    // Surface the first hard failure so PortConflict / UI dialogs can react.
    // Never auto-open a browser from Start stack — the Sites UI owns that.
    let nginx_err = start_nginx(paths, state).err();
    let apache_err = start_apache(paths, state).err();
    refresh_site_process_status(state);
    if let Some(ref e) = nginx_err.as_ref().or(apache_err.as_ref()) {
        state.runtime.last_start_error = Some(e.to_string());
    } else {
        state.runtime.last_start_error = None;
    }
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
    // Intentional stop → grey sites; clear start errors so probes never show stale logs.
    state.runtime.stack_intentionally_stopped = true;
    state.runtime.last_start_error = None;
    refresh_site_process_status(state);
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
    refresh_site_process_status(state);
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

/// After auto port remap, update every WordPress site's siteurl/home so Open URL
/// and canonical redirects match the new listen ports.
pub fn rewrite_all_wp_urls_for_port_remap(
    paths: &Paths,
    state: &AppState,
    before: PortSnapshot,
) -> Result<()> {
    let after = PortSnapshot::from_stack(&state.stack);
    for site in &state.sites {
        let Some(db) = site.database_name.as_deref() else {
            continue;
        };
        if !site
            .path
            .join("wp-config.php")
            .exists()
            && !crate::services::nginx::expand_home(&site.path)
                .join("wp-config.php")
                .exists()
        {
            continue;
        }
        let engine = site
            .database_engine
            .clone()
            .or_else(|| state.active_database.clone())
            .unwrap_or_else(|| "mariadb".into());
        if !matches!(engine.as_str(), "mysql" | "mariadb") {
            continue;
        }
        let apache = crate::services::httpd::site_uses_apache(site);
        let (old_http, old_https, new_http, new_https) = if apache {
            (
                before.apache_http,
                before.apache_https,
                after.apache_http,
                after.apache_https,
            )
        } else {
            (
                before.nginx_http,
                before.nginx_https,
                after.nginx_http,
                after.nginx_https,
            )
        };
        let pairs = [
            (
                crate::services::ports::format_url("https", &site.domain, old_https),
                crate::services::ports::format_url("https", &site.domain, new_https),
            ),
            (
                crate::services::ports::format_url("http", &site.domain, old_http),
                crate::services::ports::format_url("http", &site.domain, new_http),
            ),
        ];
        for (from, to) in pairs {
            if from == to {
                continue;
            }
            let _ = crate::services::db_files::mysql_wp_options_url_replace(
                paths, state, &engine, db, &from, &to,
            );
        }
    }
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
