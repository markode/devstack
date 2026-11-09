//! Orchestrate DNS + TLS + database + PHP + Nginx/Apache for the whole stack.

use serde::Serialize;

use crate::config::{save_state, AppState, Paths};
use crate::error::Result;
use crate::services::database::{configure_phpmyadmin, start_database, stop_database};
use crate::services::dns::{start_dns, stop_dns, sync_hosts_and_trust_ca};
use crate::services::httpd::{reload_apache, start_apache, stop_apache};
use crate::services::nginx::{reload_nginx, start_nginx, stop_nginx};
use crate::services::php_runtime::{register_system_php, start_php_many};
use crate::services::ports::{
    auto_separate_busy_ports, classic_web_port_notice, owned_runtime_pids_with_files,
    PortRemapNotice, PortSnapshot,
};
use crate::services::process::is_running;
use crate::services::progress::{emit, noop_job_progress, JobProgressFn};
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

/// Job id used for stack start/apply progress events.
pub const STACK_JOB_ID: &str = "stack";

/// Ordered phases emitted by [`apply_stack_with_progress`].
pub const STACK_PHASES: &[&str] = &["ports", "dns", "database", "php", "nginx", "apache", "done"];

fn stack_phase(progress: &JobProgressFn, phase: &str, message: &str) {
    let idx = STACK_PHASES.iter().position(|p| *p == phase).unwrap_or(0) as u64;
    emit(
        progress,
        STACK_JOB_ID,
        phase,
        message,
        idx,
        Some(STACK_PHASES.len() as u64 - 1),
    );
}

pub fn apply_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    apply_stack_with_progress(paths, state, &noop_job_progress())
}

pub fn apply_stack_with_progress(
    paths: &Paths,
    state: &mut AppState,
    progress: &JobProgressFn,
) -> Result<StackStatus> {
    let total_sw = crate::services::perf::Stopwatch::start();
    stack_phase(progress, "ports", "Checking ports…");
    refresh_site_metadata(state);

    state.runtime.stack_intentionally_stopped = false;
    state.runtime.last_start_at = Some(chrono::Utc::now().to_rfc3339());
    state.runtime.last_start_error = None;

    // Mandatory busy-port fallback: never stop foreign listeners; remap ours.
    let before = PortSnapshot::from_stack(&state.stack);
    let owned = owned_runtime_pids_with_files(paths, &state.runtime);
    let runtime_snap = state.runtime.clone();
    let remap: PortRemapNotice =
        auto_separate_busy_ports(&mut state.stack, &owned, &runtime_snap);
    let mut notice = remap.messages.clone();
    notice.extend(classic_web_port_notice(&state.stack));
    state.runtime.port_notice = notice;
    if remap.changed {
        for msg in &remap.messages {
            crate::applog::info("ports", msg);
        }
        save_state(&paths.state_file, state)?;
    }

    if !state.installed.contains_key("php") {
        let _ = register_system_php(state, paths);
    }
    let engine = state.active_database.clone().or_else(|| {
        if state.installed.contains_key("mariadb") {
            Some("mariadb".into())
        } else if state.installed.contains_key("mysql") {
            Some("mysql".into())
        } else {
            None
        }
    });
    let php_versions = crate::services::nginx::collect_php_versions(state);

    // Independent phases run concurrently on state snapshots; only the fields each
    // phase owns are merged back (dns_pid on `state`, database_* / php_pids below).
    stack_phase(progress, "dns", "Starting DNS and trusting certificates…");
    stack_phase(progress, "database", "Starting database…");
    stack_phase(progress, "php", "Preparing PHP…");
    let services_sw = crate::services::perf::Stopwatch::start();
    let ca_snapshot = state.clone();
    let db_snapshot = engine.as_ref().map(|_| state.clone());
    let mut php_snapshot = state.clone();
    let (ca_res, db_res, php_pids) = std::thread::scope(|scope| {
        let ca_h = scope.spawn(|| -> Result<()> {
            ensure_root_ca(paths)?;
            // Single UAC on Windows for hosts + CA trust; unix uses helper/sudo separately.
            let _ = sync_hosts_and_trust_ca(paths, &ca_snapshot);
            Ok(())
        });
        let db_h = scope.spawn(|| -> Option<(Result<()>, AppState)> {
            let engine = engine.as_deref()?;
            let mut st = db_snapshot?;
            let res = start_database(paths, &mut st, engine);
            if res.is_ok() {
                let _ = configure_phpmyadmin(paths, &st);
            }
            Some((res, st))
        });
        let php_h = scope.spawn(|| {
            let _ = start_php_many(paths, &mut php_snapshot, &php_versions);
            php_snapshot.runtime.php_pids.clone()
        });

        match start_dns(paths, state, state.stack.dns_port) {
            Ok(()) => {}
            Err(e) => {
                crate::applog::warn(
                    "dns",
                    &format!("{e} — continuing with hosts-file resolution only"),
                );
                state.runtime.dns_pid = None;
            }
        }
        (
            ca_h.join(),
            db_h.join(),
            php_h.join().ok(),
        )
    });
    if let Some(pids) = php_pids {
        state.runtime.php_pids = pids;
    }
    let mut db_result: Result<()> = Ok(());
    if let Ok(Some((res, st))) = db_res {
        state.runtime.database_pid = st.runtime.database_pid;
        state.runtime.database_engine = st.runtime.database_engine;
        state.active_database = st.active_database;
        db_result = res;
    }
    if let Err(e) = db_result {
        state.runtime.last_start_error = Some(e.to_string());
        save_state(&paths.state_file, state)?;
        return Err(e);
    }
    match ca_res {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            state.runtime.last_start_error = Some(e.to_string());
            save_state(&paths.state_file, state)?;
            return Err(e);
        }
        Err(_) => {
            return Err(crate::error::DevStackError::Message(
                "certificate setup thread panicked".into(),
            ))
        }
    }

    let services_ms = services_sw.elapsed_ms();

    // WP siteurl/home rewrite needs a live DB (now up) but not the web servers, so
    // overlap it with Nginx/Apache startup using a read-only snapshot.
    let wp_snapshot = state.clone();
    let remap_changed = remap.changed;
    let (nginx_err, apache_err) = std::thread::scope(|scope| {
        let wp_h = scope.spawn(|| {
            if remap_changed {
                let _ = rewrite_all_wp_urls_for_port_remap(paths, &wp_snapshot, before);
            }
            // Catch install/toggle mismatches even when ports did not remap this start.
            let _ = crate::services::wp_url::repair_mismatched_wp_urls(paths, &wp_snapshot);
        });
        // Both front-ends can run together on different ports (see docs/apache.md).
        // PHP is already up (start_php_many above) so these skip straight to spawn.
        // Never auto-open a browser from Start stack — the Sites UI owns that.
        stack_phase(progress, "nginx", "Starting Nginx…");
        let nginx_err = start_nginx(paths, state).err();
        stack_phase(progress, "apache", "Starting Apache…");
        let apache_err = start_apache(paths, state).err();
        let _ = wp_h.join();
        (nginx_err, apache_err)
    });
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
    stack_phase(progress, "done", "Stack ready");
    crate::applog::info(
        "perf",
        &format!(
            "stack ready in {}ms (db+php+dns+certs {services_ms}ms)",
            total_sw.elapsed_ms()
        ),
    );
    Ok(stack_status(state))
}

pub fn start_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    apply_stack(paths, state)
}

pub fn start_stack_with_progress(
    paths: &Paths,
    state: &mut AppState,
    progress: &JobProgressFn,
) -> Result<StackStatus> {
    apply_stack_with_progress(paths, state, progress)
}

/// Stop nginx/apache/dns/PHP (and extras) but leave the database running.
/// Used for warm `apply_stack` measurements so Windows does not re-pay InnoDB start.
pub fn stop_web_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    let _ = crate::services::share::stop_share(paths, state);
    let extras_ids: Vec<&'static str> = crate::services::extras_v030::NEW_SERVICE_IDS
        .iter()
        .copied()
        .chain(["redis", "mailpit", "mongodb", "postgresql"])
        .collect();
    for id in &extras_ids {
        let _ = crate::services::extras::stop_service(paths, state, id);
    }
    let _ = stop_nginx(paths, state);
    let _ = stop_apache(paths, state);
    let _ = stop_dns(paths, state);
    for version in state.runtime.php_pids.keys().cloned().collect::<Vec<_>>() {
        let _ = crate::services::php_runtime::stop_php(paths, state, &version);
    }
    state.runtime.stack_intentionally_stopped = true;
    state.runtime.last_start_error = None;
    refresh_site_process_status(state);
    save_state(&paths.state_file, state)?;
    Ok(stack_status(state))
}

pub fn stop_stack(paths: &Paths, state: &mut AppState) -> Result<StackStatus> {
    let _ = crate::services::share::stop_share(paths, state);
    // The database shutdown (export-on-stop + mysqladmin) is the long pole, so run
    // it on a snapshot while the front-ends, DNS and PHP are stopped here.
    let mut db_snapshot = state.clone();
    // One-click extras (meilisearch etc.) are independent of the web stack but must
    // still die with "Stop Stack" so they cannot outlive the session.
    let extras_ids: Vec<&'static str> = crate::services::extras_v030::NEW_SERVICE_IDS
        .iter()
        .copied()
        .chain(["redis", "mailpit", "mongodb", "postgresql"])
        .collect();
    // Clone before the scoped threads so front-end stops can mutate `state` freely.
    let mut extras_snapshot = state.clone();
    std::thread::scope(|scope| {
        let db_h = scope.spawn(|| {
            let _ = stop_database(paths, &mut db_snapshot);
        });
        let extras_h = scope.spawn(|| {
            for id in &extras_ids {
                let _ = crate::services::extras::stop_service(paths, &mut extras_snapshot, id);
            }
        });
        let _ = stop_nginx(paths, state);
        let _ = stop_apache(paths, state);
        let _ = stop_dns(paths, state);
        for version in state.runtime.php_pids.keys().cloned().collect::<Vec<_>>() {
            let _ = crate::services::php_runtime::stop_php(paths, state, &version);
        }
        let _ = db_h.join();
        let _ = extras_h.join();
    });
    // Merge cleared extras pids (front-end/DB fields come from the other paths).
    state.runtime.redis_pid = extras_snapshot.runtime.redis_pid;
    state.runtime.mailpit_pid = extras_snapshot.runtime.mailpit_pid;
    state.runtime.mongodb_pid = extras_snapshot.runtime.mongodb_pid;
    state.runtime.postgresql_pid = extras_snapshot.runtime.postgresql_pid;
    state.runtime.memcached_pid = extras_snapshot.runtime.memcached_pid;
    state.runtime.meilisearch_pid = extras_snapshot.runtime.meilisearch_pid;
    state.runtime.typesense_pid = extras_snapshot.runtime.typesense_pid;
    state.runtime.minio_pid = extras_snapshot.runtime.minio_pid;
    state.runtime.reverb_pid = extras_snapshot.runtime.reverb_pid;
    state.runtime.database_pid = db_snapshot.runtime.database_pid;
    state.runtime.database_engine = db_snapshot.runtime.database_engine;
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

fn refresh_one_site_metadata(state: &mut AppState, site_id: &str) {
    if let Some(site) = state.sites.iter_mut().find(|s| s.id == site_id) {
        site.ensure_domain();
        let details = crate::services::sites::detect_site(site);
        site.framework = Some(details.framework);
        if site.database_name.is_none() {
            site.database_name = details.database_name;
        }
        if site.database_user.is_none() {
            site.database_user = details.database_user;
        }
    }
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
    // Only the edited site needs a metadata re-scan; the rest were refreshed when
    // they were last saved/started (avoids N filesystem walks per toggle).
    refresh_one_site_metadata(state, site_id);
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?
        .clone();

    if site.https {
        // Cached after first mint — rcgen in-process, no openssl/UAC.
        crate::services::tls::ensure_site_cert_for(paths, &site)?;
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

    // PHP runtime for the site. start_php is a fast no-op when this version is
    // already running and listening, so unchanged versions cost ~1ms.
    if let Some(ver) = site.php_version.as_deref() {
        let _ = crate::services::php_runtime::start_php(paths, state, ver);
    }

    if now_apache != was_apache {
        // Dual-bound vhosts: both front-ends already serve every site. Ensure the
        // destination is up, then rewrite WP URLs — no Windows FOREGROUND stop+start.
        if now_apache {
            if state.runtime.apache_pid.filter(|p| is_running(*p)).is_none() {
                start_apache(paths, state)?;
            } else {
                // Config already has this site; fingerprint no-op on Windows.
                let _ = reload_apache(paths, state);
            }
        } else if state.runtime.nginx_pid.filter(|p| is_running(*p)).is_none() {
            start_nginx(paths, state)?;
        } else {
            let _ = reload_nginx(paths, state);
        }
        rewrite_wp_urls_for_server_switch(paths, state, &site, was_apache, now_apache)?;
        refresh_site_process_status(state);
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
    _was_apache: bool,
    _now_apache: bool,
) -> Result<()> {
    // Full canonical repair (scheme + port + serialized) via managed PHP, skipped
    // for non-WordPress sites and when the stored siteurl is already canonical.
    match crate::services::wp_url::repair_site_url_if_changed(paths, state, site) {
        Ok(Some(msg)) => crate::applog::info("wordpress", &msg),
        Ok(None) => {}
        Err(e) => crate::applog::warn("wordpress", &format!("server-switch URL repair: {e}")),
    }
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
    _prev_https: bool,
) -> Result<()> {
    match crate::services::wp_url::repair_site_url_if_changed(paths, state, site) {
        Ok(Some(msg)) => crate::applog::info("wordpress", &msg),
        Ok(None) => {}
        Err(e) => crate::applog::warn("wordpress", &format!("https-toggle URL repair: {e}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn stack_phases_are_ordered_and_end_with_done() {
        assert_eq!(STACK_PHASES.first(), Some(&"ports"));
        assert_eq!(STACK_PHASES.last(), Some(&"done"));
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let progress: JobProgressFn = Arc::new(move |ev| sink.lock().unwrap().push(ev.phase));
        stack_phase(&progress, "ports", "x");
        stack_phase(&progress, "done", "y");
        assert_eq!(*seen.lock().unwrap(), vec!["ports", "done"]);
    }
}
