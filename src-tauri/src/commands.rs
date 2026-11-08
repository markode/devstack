use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::config::{
    ensure_welcome_site, load_state, save_state, AppState, Paths, SiteRecord, TemplateRecord,
};
use crate::downloads::{noop_progress, ProgressEvent, ProgressFn};
use crate::error::Result;
use crate::packages::version_cache::{
    self, RefreshGuard, VersionListResponse, VersionsUpdatedEvent,
};
use crate::packages::{
    installed_from_state, InstalledPackage, PackageInfo, PackageRegistry, PackageVersion,
};
use crate::services::dns::{self, HostsStatus};
use crate::php::{list_php_extensions, set_xdebug, PhpExtensionView};
use crate::services::backup::{
    backup_database, default_backup_path, restore_database, seed_large_demo_table,
};
use crate::services::progress::{JobProgress, JobProgressFn};
use crate::services::sites::{
    create_site, delete_site, detect_site, refresh_site_metadata, SiteDetails,
};
use crate::services::stack::{
    apply_stack, reload_sites, stack_status, start_stack, stop_stack, StackStatus,
};
use crate::services::templates::{
    duplicate_from_source, list_templates, mark_site_as_template, DuplicateRequest,
    DuplicateResult,
};
use crate::services::extras::{
    list_managed_services, read_service_log, start_service, stop_service, ManagedServiceStatus,
};
use crate::services::ports::{
    apply_suggested_ports, check_service_ports, PortsConfig, PortCheckResult,
};
use crate::services::wordpress::{
    create_wordpress_site, CreateWordPressRequest, CreateWordPressResult,
};

pub struct AppCtx {
    pub paths: Paths,
    pub state: Mutex<AppState>,
    pub registry: PackageRegistry,
    pub version_refresh: Arc<RefreshGuard>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub data_dir: String,
    pub os: String,
    pub arch: String,
    pub php_manifest_url: Option<String>,
    pub stack: StackStatus,
}

#[tauri::command]
pub async fn get_app_info(ctx: State<'_, Arc<AppCtx>>) -> Result<AppInfo> {
    let state = ctx.state.lock().await;
    Ok(AppInfo {
        name: "DevStack".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: ctx.paths.root.display().to_string(),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        php_manifest_url: state
            .php_manifest_url
            .clone()
            .or_else(|| std::env::var("DEVSTACK_PHP_MANIFEST_URL").ok()),
        stack: stack_status(&state),
    })
}

/// Ensure the backups directory exists and return its absolute path.
#[tauri::command]
pub async fn get_backups_dir(ctx: State<'_, Arc<AppCtx>>) -> Result<String> {
    std::fs::create_dir_all(&ctx.paths.backups)?;
    Ok(ctx.paths.backups.display().to_string())
}

#[tauri::command]
pub async fn list_packages(ctx: State<'_, Arc<AppCtx>>) -> Result<Vec<PackageInfo>> {
    Ok(ctx.registry.list_packages())
}

#[tauri::command]
pub async fn list_package_versions(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    package_id: String,
) -> Result<VersionListResponse> {
    let cached = version_cache::cached_response(&ctx.paths, &package_id);

    // Cold cache: fetch synchronously so the UI is not empty on first open.
    if !cached.from_cache {
        let state_snapshot = ctx.state.lock().await.clone();
        return match version_cache::fetch_and_store(
            &ctx.registry,
            &ctx.paths,
            &state_snapshot,
            &package_id,
        )
        .await
        {
            Ok(entry) => Ok(VersionListResponse {
                package_id,
                versions: entry.versions,
                from_cache: false,
                refreshing: false,
                fetched_at: Some(entry.fetched_at),
                warning: None,
            }),
            Err(e) => Ok(VersionListResponse {
                package_id,
                versions: Vec::new(),
                from_cache: false,
                refreshing: false,
                fetched_at: None,
                warning: Some(e.to_string()),
            }),
        };
    }

    // Stale-while-revalidate: return cache now, refresh in the background.
    let refresh_guard = ctx.version_refresh.clone();
    if refresh_guard.try_begin(&package_id).await {
        let ctx_arc = ctx.inner().clone();
        let pkg = package_id.clone();
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            let state_snapshot = ctx_arc.state.lock().await.clone();
            let result = version_cache::fetch_and_store(
                &ctx_arc.registry,
                &ctx_arc.paths,
                &state_snapshot,
                &pkg,
            )
            .await;
            let event = match result {
                Ok(entry) => VersionsUpdatedEvent {
                    package_id: pkg.clone(),
                    versions: entry.versions,
                    fetched_at: entry.fetched_at,
                    error: None,
                },
                Err(e) => VersionsUpdatedEvent {
                    package_id: pkg.clone(),
                    versions: cached_versions_fallback(&ctx_arc.paths, &pkg),
                    fetched_at: String::new(),
                    error: Some(e.to_string()),
                },
            };
            let _ = app_handle.emit("versions-updated", &event);
            refresh_guard.end(&pkg).await;
        });
    }

    Ok(cached)
}

fn cached_versions_fallback(paths: &Paths, package_id: &str) -> Vec<PackageVersion> {
    version_cache::read_cache(paths, package_id)
        .map(|c| c.versions)
        .unwrap_or_default()
}

#[tauri::command]
pub async fn list_installed(
    ctx: State<'_, Arc<AppCtx>>,
    package_id: Option<String>,
) -> Result<Vec<InstalledPackage>> {
    let state = ctx.state.lock().await;
    if let Some(id) = package_id {
        Ok(installed_from_state(&state, &id))
    } else {
        let mut all = Vec::new();
        for id in state.installed.keys() {
            all.extend(installed_from_state(&state, id));
        }
        Ok(all)
    }
}

#[tauri::command]
pub async fn install_package(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    package_id: String,
    version: String,
) -> Result<InstalledPackage> {
    let provider = ctx.registry.get(&package_id)?;
    let progress: ProgressFn = {
        let app = app.clone();
        Arc::new(move |ev: ProgressEvent| {
            let _ = app.emit("install-progress", ev);
        })
    };

    let mut state = ctx.state.lock().await;
    provider
        .install(&version, &ctx.paths, &mut state, progress)
        .await
}

#[tauri::command]
pub async fn uninstall_package(
    ctx: State<'_, Arc<AppCtx>>,
    package_id: String,
    version: String,
) -> Result<()> {
    let provider = ctx.registry.get(&package_id)?;
    let mut state = ctx.state.lock().await;
    provider
        .uninstall(&version, &ctx.paths, &mut state)
        .await
}

#[tauri::command]
pub async fn get_sites(ctx: State<'_, Arc<AppCtx>>) -> Result<Vec<SiteRecord>> {
    let mut state = ctx.state.lock().await;
    ensure_welcome_site(&ctx.paths, &mut state);
    refresh_site_metadata(&mut state);
    save_state(&ctx.paths.state_file, &state)?;
    Ok(state.sites.clone())
}

/// Fast site list for UI: local config only — no TCP/HTTP probes.
/// Call `probe_site_reachability_cmd` (or list with probe=true) in the background.
#[tauri::command]
pub async fn list_site_details(ctx: State<'_, Arc<AppCtx>>) -> Result<Vec<SiteDetails>> {
    list_site_details_inner(ctx, false).await
}

#[tauri::command]
pub async fn list_site_details_probed(ctx: State<'_, Arc<AppCtx>>) -> Result<Vec<SiteDetails>> {
    list_site_details_inner(ctx, true).await
}

async fn list_site_details_inner(
    ctx: State<'_, Arc<AppCtx>>,
    probe: bool,
) -> Result<Vec<SiteDetails>> {
    let mut state = ctx.state.lock().await;
    ensure_welcome_site(&ctx.paths, &mut state);
    refresh_site_metadata(&mut state);
    let sites: Vec<_> = state.sites.clone();
    let mut details = Vec::with_capacity(sites.len());
    for s in sites {
        let mut d = detect_site(&s);
        if probe {
            crate::services::sites::enrich_site_details(&ctx.paths, &mut state, &mut d);
        } else {
            crate::services::sites::enrich_site_details_fast(&mut d, &state);
        }
        details.push(d);
    }
    if probe {
        save_state(&ctx.paths.state_file, &state)?;
    }
    Ok(details)
}

#[tauri::command]
pub async fn probe_all_site_reachability(
    ctx: State<'_, Arc<AppCtx>>,
) -> Result<Vec<SiteDetails>> {
    list_site_details_inner(ctx, true).await
}

#[tauri::command]
pub async fn get_site_details(
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
) -> Result<SiteDetails> {
    let mut state = ctx.state.lock().await;
    ensure_welcome_site(&ctx.paths, &mut state);
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?
        .clone();
    let mut details = detect_site(&site);
    crate::services::sites::enrich_site_details(&ctx.paths, &mut state, &mut details);
    save_state(&ctx.paths.state_file, &state)?;
    Ok(details)
}

#[tauri::command]
pub async fn create_site_cmd(
    ctx: State<'_, Arc<AppCtx>>,
    name: String,
    path: String,
    php_version: Option<String>,
    web_server: Option<String>,
    https: Option<bool>,
    document_root: Option<String>,
) -> Result<SiteRecord> {
    let mut state = ctx.state.lock().await;
    let mut site = create_site(
        &mut state,
        &ctx.paths,
        &name,
        crate::paths_util::expand_home(&PathBuf::from(path)),
        php_version,
    )?;
    if let Some(ws) = web_server {
        let normalized = match ws.to_ascii_lowercase().as_str() {
            "apache" | "httpd" => "apache".to_string(),
            _ => "nginx".to_string(),
        };
        if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
            s.web_server = Some(normalized.clone());
            site.web_server = Some(normalized);
        }
    }
    if let Some(h) = https {
        if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
            s.https = h;
            site.https = h;
        }
    }
    if let Some(sub) = document_root {
        let sub = sub.trim().trim_matches(['/', '\\']);
        if !sub.is_empty() {
            if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
                let base = crate::paths_util::expand_home(&s.path);
                let joined = base.join(sub);
                s.path = joined.clone();
                site.path = joined;
            }
        }
    }
    save_state(&ctx.paths.state_file, &state)?;
    sync_hosts_after_change(&ctx.paths, &mut state)?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(site)
}

#[tauri::command]
pub async fn update_site_environment(
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
    php_version: Option<String>,
    node_version: Option<String>,
    https: Option<bool>,
    web_server: Option<String>,
) -> Result<SiteRecord> {
    let mut state = ctx.state.lock().await;
    ensure_welcome_site(&ctx.paths, &mut state);
    let (prev_web_server, prev_https) = {
        let site = state
            .sites
            .iter()
            .find(|s| s.id == site_id)
            .ok_or_else(|| {
                crate::error::DevStackError::Message(format!("site not found: {site_id}"))
            })?;
        (site.web_server.clone(), site.https)
    };
    let site = state
        .sites
        .iter_mut()
        .find(|s| s.id == site_id)
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?;
    if php_version.is_some() {
        site.php_version = php_version;
    }
    if node_version.is_some() {
        site.node_version = node_version;
    }
    if let Some(h) = https {
        site.https = h;
    }
    if let Some(ws) = web_server {
        let normalized = match ws.to_ascii_lowercase().as_str() {
            "apache" | "httpd" => "apache".to_string(),
            _ => "nginx".to_string(),
        };
        site.web_server = Some(normalized);
    }
    let out = site.clone();
    save_state(&ctx.paths.state_file, &state)?;
    // Fast path: cert cache + hosts-if-needed + reload only affected front-end.
    // Do NOT re-run certutil/UAC or full stack restart on every toggle.
    crate::services::stack::apply_site_environment_change(
        &ctx.paths,
        &mut state,
        &site_id,
        prev_web_server,
        prev_https,
    )?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(out)
}

#[tauri::command]
pub async fn list_php_extension_settings(
    ctx: State<'_, Arc<AppCtx>>,
) -> Result<Vec<PhpExtensionView>> {
    let state = ctx.state.lock().await;
    Ok(list_php_extensions(&state, &ctx.paths))
}

#[tauri::command]
pub async fn set_php_xdebug(
    ctx: State<'_, Arc<AppCtx>>,
    version: String,
    enabled: bool,
) -> Result<PhpExtensionView> {
    let mut state = ctx.state.lock().await;
    set_xdebug(&mut state, &ctx.paths, &version, enabled)
}

#[tauri::command]
pub async fn set_php_manifest_url(
    ctx: State<'_, Arc<AppCtx>>,
    url: Option<String>,
) -> Result<()> {
    let mut state = ctx.state.lock().await;
    state.php_manifest_url = url.filter(|s| !s.trim().is_empty());
    save_state(&ctx.paths.state_file, &state)?;
    Ok(())
}

#[tauri::command]
pub async fn get_stack_status(ctx: State<'_, Arc<AppCtx>>) -> Result<StackStatus> {
    let state = ctx.state.lock().await;
    Ok(stack_status(&state))
}

#[tauri::command]
pub async fn start_dev_stack(ctx: State<'_, Arc<AppCtx>>) -> Result<StackStatus> {
    let mut state = ctx.state.lock().await;
    start_stack(&ctx.paths, &mut state)
}

#[tauri::command]
pub async fn stop_dev_stack(ctx: State<'_, Arc<AppCtx>>) -> Result<StackStatus> {
    let mut state = ctx.state.lock().await;
    stop_stack(&ctx.paths, &mut state)
}

#[tauri::command]
pub async fn apply_dev_stack(ctx: State<'_, Arc<AppCtx>>) -> Result<StackStatus> {
    let mut state = ctx.state.lock().await;
    apply_stack(&ctx.paths, &mut state)
}

#[tauri::command]
pub async fn register_system_php_cmd(ctx: State<'_, Arc<AppCtx>>) -> Result<String> {
    let mut state = ctx.state.lock().await;
    let rt = crate::services::php_runtime::register_system_php(&mut state, &ctx.paths)?;
    Ok(rt.version)
}

fn job_progress_emitter(app: AppHandle) -> JobProgressFn {
    Arc::new(move |ev: JobProgress| {
        let _ = app.emit("job-progress", ev);
    })
}

#[tauri::command]
pub async fn create_wordpress(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    request: CreateWordPressRequest,
) -> Result<CreateWordPressResult> {
    let progress = job_progress_emitter(app);
    let mut state = ctx.state.lock().await;
    let result =
        create_wordpress_site(&ctx.paths, &mut state, request, &progress, "wordpress").await?;
    save_state(&ctx.paths.state_file, &state)?;
    sync_hosts_after_change(&ctx.paths, &mut state)?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(result)
}

#[tauri::command]
pub async fn mark_template(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
) -> Result<TemplateRecord> {
    let progress = job_progress_emitter(app);
    let mut state = ctx.state.lock().await;
    let tpl =
        mark_site_as_template(&ctx.paths, &mut state, &site_id, &progress, "template").await?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(tpl)
}

#[tauri::command]
pub async fn get_templates(ctx: State<'_, Arc<AppCtx>>) -> Result<Vec<TemplateRecord>> {
    let state = ctx.state.lock().await;
    Ok(list_templates(&state))
}

#[tauri::command]
pub async fn duplicate_site(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    request: DuplicateRequest,
) -> Result<DuplicateResult> {
    let progress = job_progress_emitter(app);
    let mut state = ctx.state.lock().await;
    let result =
        duplicate_from_source(&ctx.paths, &mut state, request, &progress, "duplicate").await?;
    save_state(&ctx.paths.state_file, &state)?;
    sync_hosts_after_change(&ctx.paths, &mut state)?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub path: String,
    pub bytes: u64,
}

#[tauri::command]
pub async fn backup_site_database(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
) -> Result<BackupResult> {
    let progress = job_progress_emitter(app);
    let state = ctx.state.lock().await;
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?;
    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into());
    let db = site
        .database_name
        .clone()
        .or_else(|| detect_site(&site).database_name)
        .ok_or_else(|| crate::error::DevStackError::Message("no database on site".into()))?;
    let dest = default_backup_path(&ctx.paths, &site.id, &db);
    let bytes = backup_database(
        &ctx.paths,
        &state,
        &engine,
        &db,
        &dest,
        &progress,
        "backup",
    )?;
    Ok(BackupResult {
        path: dest.display().to_string(),
        bytes,
    })
}

#[tauri::command]
pub async fn restore_site_database(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
    archive_path: String,
) -> Result<()> {
    let progress = job_progress_emitter(app);
    let state = ctx.state.lock().await;
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?;
    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into());
    let db = site
        .database_name
        .clone()
        .or_else(|| detect_site(&site).database_name)
        .ok_or_else(|| crate::error::DevStackError::Message("no database on site".into()))?;
    restore_database(
        &ctx.paths,
        &state,
        &engine,
        &db,
        PathBuf::from(archive_path).as_path(),
        &progress,
        "restore",
    )?;
    Ok(())
}

#[tauri::command]
pub async fn seed_site_large_table(
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
    rows: Option<u32>,
) -> Result<()> {
    let state = ctx.state.lock().await;
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?;
    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into());
    let db = site
        .database_name
        .clone()
        .ok_or_else(|| crate::error::DevStackError::Message("no database on site".into()))?;
    seed_large_demo_table(&ctx.paths, &state, &engine, &db, rows.unwrap_or(2000))?;
    Ok(())
}

#[tauri::command]
pub async fn list_managed_services_cmd(
    ctx: State<'_, Arc<AppCtx>>,
) -> Result<Vec<ManagedServiceStatus>> {
    let state = ctx.state.lock().await;
    Ok(list_managed_services(&ctx.paths, &state))
}

#[tauri::command]
pub async fn start_managed_service(
    ctx: State<'_, Arc<AppCtx>>,
    service_id: String,
) -> Result<ManagedServiceStatus> {
    let mut state = ctx.state.lock().await;
    start_service(&ctx.paths, &mut state, &service_id)
}

#[tauri::command]
pub async fn stop_managed_service(
    ctx: State<'_, Arc<AppCtx>>,
    service_id: String,
) -> Result<ManagedServiceStatus> {
    let mut state = ctx.state.lock().await;
    stop_service(&ctx.paths, &mut state, &service_id)
}

#[tauri::command]
pub async fn get_service_log(
    ctx: State<'_, Arc<AppCtx>>,
    service_id: String,
    lines: Option<u32>,
) -> Result<String> {
    read_service_log(&ctx.paths, &service_id, lines.unwrap_or(80) as usize)
}

#[tauri::command]
pub async fn update_package(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    package_id: String,
) -> Result<InstalledPackage> {
    // Reinstall latest listed version (update stub for Services)
    let provider = ctx.registry.get(&package_id)?;
    let progress: ProgressFn = {
        let app = app.clone();
        Arc::new(move |ev: ProgressEvent| {
            let _ = app.emit("install-progress", ev);
        })
    };
    let mut state = ctx.state.lock().await;
    let versions = provider.list_versions(&state).await?;
    let latest = versions
        .first()
        .ok_or_else(|| crate::error::DevStackError::Message("no versions available".into()))?;
    provider
        .install(&latest.version, &ctx.paths, &mut state, progress)
        .await
}

/// Headless helper used by CI / smoke tests without a UI.
#[allow(dead_code)]
pub async fn smoke_list_node(ctx: &AppCtx) -> Result<Vec<PackageVersion>> {
    let provider = ctx.registry.get("node")?;
    let state = ctx.state.lock().await;
    let _ = noop_progress();
    provider.list_versions(&state).await
}

// —— Ports / coexistence ———————————————————————————————————————————————

#[tauri::command]
pub async fn get_ports_config(ctx: State<'_, Arc<AppCtx>>) -> Result<PortsConfig> {
    let state = ctx.state.lock().await;
    Ok(PortsConfig::from_stack(&state.stack))
}

#[tauri::command]
pub async fn update_ports_config(
    ctx: State<'_, Arc<AppCtx>>,
    config: PortsConfig,
) -> Result<PortsConfig> {
    let mut state = ctx.state.lock().await;
    let before = crate::services::ports::PortSnapshot::from_stack(&state.stack);
    // phpMyAdmin is served by Nginx — keep the displayed URL port in sync.
    let mut cfg = config;
    cfg.phpmyadmin = cfg.nginx_https;
    cfg.apply_to_stack(&mut state.stack);
    let _ = crate::services::stack::rewrite_all_wp_urls_for_port_remap(
        &ctx.paths, &state, before,
    );
    save_state(&ctx.paths.state_file, &state)?;
    // Regenerate configs so running services pick up new ports on next start/reload.
    let _ = reload_sites(&ctx.paths, &mut state);
    Ok(PortsConfig::from_stack(&state.stack))
}

#[tauri::command]
pub async fn set_coexistence_mode(
    ctx: State<'_, Arc<AppCtx>>,
    enabled: bool,
) -> Result<PortsConfig> {
    let mut state = ctx.state.lock().await;
    let before = crate::services::ports::PortSnapshot::from_stack(&state.stack);
    if enabled {
        PortsConfig::coexistence().apply_to_stack(&mut state.stack);
        state.runtime.port_notice = vec![
            "Coexistence mode on — DevStack uses alternate ports beside WAMP/MAMP.".into(),
        ];
    } else {
        // Restore classic defaults for web + DB ports; keep Apache on 8088/8444.
        state.stack.coexistence_mode = false;
        state.stack.http_port = 80;
        state.stack.https_port = 443;
        state.stack.mysql_port = 3306;
        state.stack.mariadb_port = 3306;
        state.stack.postgresql_port = 5432;
        state.stack.mongodb_port = 27017;
        state.stack.php_fpm_port = 9000;
        state.runtime.port_notice.clear();
    }
    let _ = crate::services::stack::rewrite_all_wp_urls_for_port_remap(
        &ctx.paths, &state, before,
    );
    save_state(&ctx.paths.state_file, &state)?;
    let _ = reload_sites(&ctx.paths, &mut state);
    Ok(PortsConfig::from_stack(&state.stack))
}

#[tauri::command]
pub async fn check_service_ports_cmd(
    ctx: State<'_, Arc<AppCtx>>,
    service_id: String,
) -> Result<PortCheckResult> {
    let state = ctx.state.lock().await;
    Ok(check_service_ports(&state.stack, &service_id))
}

#[tauri::command]
pub async fn apply_next_free_ports(
    ctx: State<'_, Arc<AppCtx>>,
    service_id: String,
) -> Result<PortsConfig> {
    let mut state = ctx.state.lock().await;
    let before = crate::services::ports::PortSnapshot::from_stack(&state.stack);
    let check = check_service_ports(&state.stack, &service_id);
    if !check.suggested.is_empty() {
        apply_suggested_ports(&mut state.stack, &check.suggested)?;
        // Turning on individual alts should also flip coexistence when web/DB move off defaults.
        if state.stack.http_port != 80
            || state.stack.https_port != 443
            || state.stack.mysql_port != 3306
            || state.stack.mariadb_port != 3306
        {
            state.stack.coexistence_mode = true;
        }
        let _ = crate::services::stack::rewrite_all_wp_urls_for_port_remap(
            &ctx.paths, &state, before,
        );
        save_state(&ctx.paths.state_file, &state)?;
        let _ = reload_sites(&ctx.paths, &mut state);
    }
    Ok(PortsConfig::from_stack(&state.stack))
}

// —— Databases folder (plain-file dumps) ————————————————————————————————

#[tauri::command]
pub async fn get_db_files_settings(
    ctx: State<'_, Arc<AppCtx>>,
) -> Result<crate::config::DatabaseFilesSettings> {
    let state = ctx.state.lock().await;
    Ok(state.db_files.clone())
}

#[tauri::command]
pub async fn get_databases_folder(ctx: State<'_, Arc<AppCtx>>) -> Result<String> {
    let state = ctx.state.lock().await;
    let root = crate::services::db_files::databases_folder(&ctx.paths, &state.db_files)?;
    Ok(root.display().to_string())
}

#[tauri::command]
pub async fn update_db_files_settings(
    ctx: State<'_, Arc<AppCtx>>,
    folder: Option<String>,
    auto_export_interval_secs: Option<u64>,
    auto_export_on_stop: Option<bool>,
    import_mode: Option<String>,
) -> Result<crate::config::DatabaseFilesSettings> {
    let mut state = ctx.state.lock().await;
    if let Some(f) = folder {
        state.db_files.folder = if f.trim().is_empty() {
            None
        } else {
            Some(PathBuf::from(f))
        };
    }
    if let Some(secs) = auto_export_interval_secs {
        state.db_files.auto_export_interval_secs = secs;
    }
    if let Some(on_stop) = auto_export_on_stop {
        state.db_files.auto_export_on_stop = on_stop;
    }
    if let Some(mode) = import_mode {
        let normalized = match mode.to_ascii_lowercase().as_str() {
            "auto" => "auto".to_string(),
            _ => "ask".to_string(),
        };
        state.db_files.import_mode = normalized;
    }
    let _ = crate::services::db_files::databases_folder(&ctx.paths, &state.db_files)?;
    let out = state.db_files.clone();
    save_state(&ctx.paths.state_file, &state)?;
    Ok(out)
}

#[tauri::command]
pub async fn list_database_files(
    ctx: State<'_, Arc<AppCtx>>,
) -> Result<Vec<crate::services::db_files::DbFileEntry>> {
    let state = ctx.state.lock().await;
    crate::services::db_files::list_db_files(&ctx.paths, &state)
}

#[tauri::command]
pub async fn export_database_to_folder(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    engine: String,
    database: String,
) -> Result<crate::services::db_files::DbFileEntry> {
    let progress = job_progress_emitter(app);
    let state = ctx.state.lock().await;
    crate::services::db_files::export_database_file(
        &ctx.paths,
        &state,
        &engine,
        &database,
        &progress,
        "db-export",
    )
}

#[tauri::command]
pub async fn export_all_databases_to_folder(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
) -> Result<Vec<crate::services::db_files::DbFileEntry>> {
    let progress = job_progress_emitter(app);
    let state = ctx.state.lock().await;
    crate::services::db_files::export_all_running(&ctx.paths, &state, &progress, "db-export-all")
}

#[tauri::command]
pub async fn import_database_from_folder(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    engine: String,
    database: String,
    archive_path: String,
) -> Result<()> {
    let progress = job_progress_emitter(app);
    let state = ctx.state.lock().await;
    crate::services::db_files::import_database_file(
        &ctx.paths,
        &state,
        &engine,
        &database,
        PathBuf::from(archive_path).as_path(),
        &progress,
        "db-import",
    )
}

#[tauri::command]
pub async fn download_site_database(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
    dest_path: String,
) -> Result<BackupResult> {
    let progress = job_progress_emitter(app);
    let state = ctx.state.lock().await;
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?;
    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into());
    let db = site
        .database_name
        .clone()
        .or_else(|| detect_site(&site).database_name)
        .ok_or_else(|| crate::error::DevStackError::Message("no database on site".into()))?;
    let dest = PathBuf::from(dest_path);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = backup_database(
        &ctx.paths,
        &state,
        &engine,
        &db,
        &dest,
        &progress,
        "download-db",
    )?;
    Ok(BackupResult {
        path: dest.display().to_string(),
        bytes,
    })
}

#[tauri::command]
pub async fn import_site_database_file(
    app: AppHandle,
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
    archive_path: String,
    search: Option<String>,
    replace: Option<String>,
) -> Result<()> {
    let progress = job_progress_emitter(app);
    let (site, engine, db) = {
        let state = ctx.state.lock().await;
        let site = state
            .sites
            .iter()
            .find(|s| s.id == site_id)
            .cloned()
            .ok_or_else(|| {
                crate::error::DevStackError::Message(format!("site not found: {site_id}"))
            })?;
        let engine = site
            .database_engine
            .clone()
            .or_else(|| state.active_database.clone())
            .unwrap_or_else(|| "mariadb".into());
        let db = site
            .database_name
            .clone()
            .or_else(|| detect_site(&site).database_name)
            .ok_or_else(|| crate::error::DevStackError::Message("no database on site".into()))?;
        (site, engine, db)
    };
    {
        let state = ctx.state.lock().await;
        crate::services::db_files::import_database_file(
            &ctx.paths,
            &state,
            &engine,
            &db,
            PathBuf::from(&archive_path).as_path(),
            &progress,
            "site-import",
        )?;
    }
    let search_replace = match (search, replace) {
        (Some(s), Some(r)) if !s.is_empty() => Some((s, r)),
        _ => None,
    };
    let state = ctx.state.lock().await;
    crate::services::db_files::post_import_url_rewrite(
        &ctx.paths,
        &state,
        &site,
        search_replace,
        &progress,
        "site-import-sr",
    )
    .await?;
    Ok(())
}

pub fn bootstrap_ctx(data_root: std::path::PathBuf) -> Result<AppCtx> {
    let paths = Paths::from_root(data_root)?;
    let mut state = load_state(&paths.state_file)?;
    ensure_welcome_site(&paths, &mut state);
    let _ = crate::services::db_files::databases_folder(&paths, &state.db_files);
    let _ = crate::php::regenerate_all_php_ini(&paths, &state);
    save_state(&paths.state_file, &state)?;
    let ctx = AppCtx {
        paths,
        state: Mutex::new(state),
        registry: PackageRegistry::new()?,
        version_refresh: Arc::new(RefreshGuard::new()),
    };
    Ok(ctx)
}

/// Prefetch version lists into the disk cache (best-effort, non-blocking).
pub fn prefetch_version_lists(ctx: Arc<AppCtx>) {
    tauri::async_runtime::spawn(async move {
        let ids = version_cache::all_package_ids(&ctx.registry);
        for id in ids {
            if !ctx.version_refresh.try_begin(&id).await {
                continue;
            }
            let state = ctx.state.lock().await.clone();
            let _ = version_cache::fetch_and_store(&ctx.registry, &ctx.paths, &state, &id).await;
            ctx.version_refresh.end(&id).await;
        }
    });
}

#[tauri::command]
pub async fn get_hosts_status(ctx: State<'_, Arc<AppCtx>>) -> Result<HostsStatus> {
    let state = ctx.state.lock().await;
    Ok(dns::check_hosts_status(&ctx.paths, &state))
}

#[tauri::command]
pub async fn repair_hosts(ctx: State<'_, Arc<AppCtx>>) -> Result<HostsStatus> {
    let mut state = ctx.state.lock().await;
    ensure_welcome_site(&ctx.paths, &mut state);
    let _ = crate::services::tls::ensure_root_ca(&ctx.paths);
    dns::sync_hosts_and_trust_ca(&ctx.paths, &state)?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(dns::check_hosts_status(&ctx.paths, &state))
}

/// Mint a one-time phpMyAdmin sign-on URL (password never appears in the query string).
#[tauri::command]
pub async fn delete_site_cmd(
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
) -> Result<()> {
    let mut state = ctx.state.lock().await;
    delete_site(&ctx.paths, &mut state, &site_id)?;
    save_state(&ctx.paths.state_file, &state)?;
    // Rewrite hosts without the removed domain; reload front-ends so vhosts drop.
    dns::sync_hosts_and_trust_ca(&ctx.paths, &state)?;
    let _ = crate::services::nginx::reload_nginx(&ctx.paths, &mut state);
    let _ = crate::services::httpd::reload_apache(&ctx.paths, &mut state);
    if !state.sites.iter().any(crate::services::httpd::site_uses_apache) {
        let _ = crate::services::httpd::stop_apache(&ctx.paths, &mut state);
    }
    save_state(&ctx.paths.state_file, &state)?;
    Ok(())
}

#[tauri::command]
pub async fn mint_phpmyadmin_signon_url(
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
) -> Result<String> {
    let state = ctx.state.lock().await;
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?;
    let (db_name, db_user, db_pass) = crate::services::sites::parse_wp_config(&site.path);
    let db = db_name
        .or_else(|| site.database_name.clone())
        .ok_or_else(|| {
            crate::error::DevStackError::Message("site has no database name".into())
        })?;
    let user = db_user
        .or_else(|| site.database_user.clone())
        .unwrap_or_else(|| "root".into());
    let pass = db_pass.unwrap_or_default();
    // Ensure signon.php knows the current tokens dir / DB port.
    let _ = crate::services::database::configure_phpmyadmin(&ctx.paths, &state);
    crate::services::pma_signon::mint_signon_url(&ctx.paths, &state, &db, &user, &pass)
}

#[tauri::command]
pub async fn open_path_cmd(path: String) -> Result<()> {
    let p = PathBuf::from(&path);
    let abs = crate::paths_util::expand_home(&p);
    if abs.is_dir() || abs.extension().is_none() {
        // Site folders: create if missing (empty site path).
        if !abs.exists() {
            crate::paths_util::ensure_dir(&abs)?;
        }
    }
    crate::paths_util::open_path_os(&abs)
}

#[tauri::command]
pub async fn reveal_path_cmd(path: String) -> Result<()> {
    let abs = crate::paths_util::expand_home(&PathBuf::from(&path));
    if !abs.exists() {
        return Err(crate::error::DevStackError::Message(format!(
            "path doesn't exist: {}",
            abs.display()
        )));
    }
    crate::paths_util::reveal_path_os(&abs)
}

/// Sync hosts+CA after site mutations that add domains (create/duplicate/WP).
/// Skips when hosts + CA trust marker are already current; still reloads servers.
pub fn sync_hosts_after_change(paths: &Paths, state: &mut AppState) -> Result<HostsStatus> {
    let _ = crate::services::tls::ensure_root_ca(paths);
    dns::sync_hosts_and_trust_ca(paths, state)?;
    let _ = crate::services::nginx::reload_nginx(paths, state);
    let _ = crate::services::httpd::reload_apache(paths, state);
    Ok(dns::check_hosts_status(paths, state))
}
