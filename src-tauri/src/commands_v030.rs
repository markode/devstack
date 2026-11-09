//! Tauri commands for v0.3.0 features (kept apart from `commands.rs`).
//!
//! One-click services reuse `start_managed_service` / `stop_managed_service` /
//! `list_managed_services_cmd` / `get_service_log` with the new service ids
//! (`memcached`, `meilisearch`, `typesense`, `minio`, `reverb`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::{job_progress_emitter, sync_hosts_after_change, AppCtx};
use crate::config::{save_state, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::config_transfer::{ArchiveMeta, ExportSummary, ImportOptions, ImportReport};
use crate::services::extras::ManagedServiceStatus;
use crate::services::hosting_sync::{
    Direction, HostingProfile, HostingTools, KeyringStore, SyncPlan, SyncResult,
};
use crate::services::metrics::{MetricsSnapshot, SlowQueryInfo, SpxStatus};
use crate::services::mobile_preview::MobilePreviewInfo;
use crate::services::site_logs::{LogSourceInfo, QueueStatus, SiteLogPage, SiteLogQuery};
use crate::services::site_php_ini::{SitePhpIniView, Sliders};
use crate::services::site_templates::{
    CreateFromTemplateRequest, CreateFromTemplateResult, TemplateInfo,
};
use crate::services::tld::TldSettings;

type Ctx<'a> = State<'a, Arc<AppCtx>>;

// --- 1. Services ---------------------------------------------------------------

/// Choose the Laravel site Reverb serves (`None` = auto-detect).
#[tauri::command]
pub async fn set_reverb_site(ctx: Ctx<'_>, site_id: Option<String>) -> Result<ManagedServiceStatus> {
    let mut state = ctx.state.lock().await;
    if let Some(id) = site_id.as_deref() {
        if !state.sites.iter().any(|s| s.id == id) {
            return Err(DevStackError::Message(format!("site not found: {id}")));
        }
    }
    state.stack.reverb_site_id = site_id;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(crate::services::extras::service_status(&ctx.paths, &state, "reverb"))
}

// --- 2. Per-site php.ini ---------------------------------------------------------

#[tauri::command]
pub async fn get_site_php_ini(ctx: Ctx<'_>, site_id: String) -> Result<SitePhpIniView> {
    let state = ctx.state.lock().await;
    crate::services::site_php_ini::get_site_php_ini(&ctx.paths, &state, &site_id)
}

#[tauri::command]
pub async fn set_site_php_ini(
    ctx: Ctx<'_>,
    site_id: String,
    overrides: BTreeMap<String, String>,
) -> Result<SitePhpIniView> {
    let mut state = ctx.state.lock().await;
    crate::services::site_php_ini::set_site_php_ini(&ctx.paths, &mut state, &site_id, overrides)
}

#[tauri::command]
pub async fn set_site_php_ini_sliders(
    ctx: Ctx<'_>,
    site_id: String,
    sliders: Sliders,
) -> Result<SitePhpIniView> {
    let mut state = ctx.state.lock().await;
    crate::services::site_php_ini::apply_sliders(&ctx.paths, &mut state, &site_id, &sliders)
}

// --- 3. Site templates -----------------------------------------------------------

#[tauri::command]
pub async fn list_site_template_kinds() -> Result<Vec<TemplateInfo>> {
    Ok(crate::services::site_templates::list_template_kinds())
}

#[tauri::command]
pub async fn create_site_from_template(
    app: AppHandle,
    ctx: Ctx<'_>,
    request: CreateFromTemplateRequest,
) -> Result<CreateFromTemplateResult> {
    let progress = job_progress_emitter(app);
    let mut state = ctx.state.lock().await;
    let result = crate::services::site_templates::create_from_template(
        &ctx.paths,
        &mut state,
        request,
        &progress,
        "site-template",
    )
    .await?;
    save_state(&ctx.paths.state_file, &state)?;
    let _ = sync_hosts_after_change(&ctx.paths, &mut state);
    save_state(&ctx.paths.state_file, &state)?;
    Ok(result)
}

// --- 4. Hosting sync -------------------------------------------------------------

#[tauri::command]
pub async fn hosting_tools_status() -> Result<HostingTools> {
    Ok(crate::services::hosting_sync::tool_status(&KeyringStore))
}

#[tauri::command]
pub async fn hosting_list_profiles(ctx: Ctx<'_>) -> Result<Vec<HostingProfile>> {
    Ok(crate::services::hosting_sync::list_profiles(&ctx.paths, &KeyringStore))
}

#[tauri::command]
pub async fn hosting_save_profile(
    ctx: Ctx<'_>,
    profile: HostingProfile,
    password: Option<String>,
    db_password: Option<String>,
) -> Result<HostingProfile> {
    crate::services::hosting_sync::save_profile(&ctx.paths, &KeyringStore, profile, password, db_password)
}

#[tauri::command]
pub async fn hosting_delete_profile(ctx: Ctx<'_>, profile_id: String) -> Result<()> {
    crate::services::hosting_sync::delete_profile(&ctx.paths, &KeyringStore, &profile_id)
}

#[tauri::command]
pub async fn hosting_test_connection(ctx: Ctx<'_>, profile_id: String) -> Result<String> {
    let paths = ctx.paths.clone();
    tokio::task::spawn_blocking(move || {
        crate::services::hosting_sync::test_connection(&paths, &KeyringStore, &profile_id)
    })
    .await
    .map_err(|e| DevStackError::Message(e.to_string()))?
}

#[tauri::command]
pub async fn hosting_preview(
    ctx: Ctx<'_>,
    profile_id: String,
    direction: String,
    delete: Option<bool>,
) -> Result<SyncPlan> {
    let dir = Direction::parse(&direction)?;
    let (paths, state) = (ctx.paths.clone(), ctx.state.lock().await.clone());
    tokio::task::spawn_blocking(move || {
        crate::services::hosting_sync::preview(&paths, &state, &KeyringStore, &profile_id, dir, delete.unwrap_or(false))
    })
    .await
    .map_err(|e| DevStackError::Message(e.to_string()))?
}

#[tauri::command]
pub async fn hosting_run_sync(
    app: AppHandle,
    ctx: Ctx<'_>,
    profile_id: String,
    direction: String,
    delete: Option<bool>,
    include_database: Option<bool>,
) -> Result<SyncResult> {
    let dir = Direction::parse(&direction)?;
    let progress = job_progress_emitter(app);
    let (paths, state) = (ctx.paths.clone(), ctx.state.lock().await.clone());
    tokio::task::spawn_blocking(move || {
        crate::services::hosting_sync::run_sync(
            &paths,
            &state,
            &KeyringStore,
            &profile_id,
            dir,
            delete.unwrap_or(false),
            include_database.unwrap_or(false),
            &progress,
            "hosting-sync",
        )
    })
    .await
    .map_err(|e| DevStackError::Message(e.to_string()))?
}

// --- 5. TLDs / aliases -----------------------------------------------------------

#[tauri::command]
pub async fn get_tld_settings(ctx: Ctx<'_>) -> Result<TldSettings> {
    let state = ctx.state.lock().await;
    Ok(crate::services::tld::tld_settings(&state.developer.default_tld))
}

#[tauri::command]
pub async fn set_default_tld(ctx: Ctx<'_>, tld: String) -> Result<TldSettings> {
    let mut state = ctx.state.lock().await;
    let settings = crate::services::tld::set_default_tld(&mut state, &tld)?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(settings)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteDomainUpdate {
    pub site: SiteRecord,
    pub warnings: Vec<String>,
    pub hosts_ok: bool,
    pub hosts_message: Option<String>,
}

/// Change a site's TLD / aliases / wildcard; regenerates cert, vhosts and hosts block.
#[tauri::command]
pub async fn update_site_domain(
    ctx: Ctx<'_>,
    site_id: String,
    tld: Option<String>,
    aliases: Option<Vec<String>>,
    wildcard: Option<bool>,
) -> Result<SiteDomainUpdate> {
    let mut state = ctx.state.lock().await;
    let (site, warnings) = crate::services::tld::update_site_domain_options(
        &mut state,
        &site_id,
        tld.as_deref(),
        aliases,
        wildcard,
    )?;
    save_state(&ctx.paths.state_file, &state)?;
    let hosts = sync_hosts_after_change(&ctx.paths, &mut state)?;
    save_state(&ctx.paths.state_file, &state)?;
    Ok(SiteDomainUpdate { site, warnings, hosts_ok: hosts.ok, hosts_message: hosts.message })
}

// --- 6. Logs + queue -------------------------------------------------------------

#[tauri::command]
pub async fn list_site_log_sources(ctx: Ctx<'_>, site_id: String) -> Result<Vec<LogSourceInfo>> {
    let state = ctx.state.lock().await;
    crate::services::site_logs::list_sources(&ctx.paths, &state, &site_id)
}

/// Filtered tail (`query.cursor` unset) or live poll (`query.cursor` = last page's `cursor`).
#[tauri::command]
pub async fn read_site_logs(ctx: Ctx<'_>, site_id: String, query: SiteLogQuery) -> Result<SiteLogPage> {
    let state = ctx.state.lock().await;
    crate::services::site_logs::read_logs(&ctx.paths, &state, &site_id, &query)
}

#[tauri::command]
pub async fn get_site_queue_status(ctx: Ctx<'_>, site_id: String) -> Result<QueueStatus> {
    let (paths, state) = (ctx.paths.clone(), ctx.state.lock().await.clone());
    tokio::task::spawn_blocking(move || crate::services::site_logs::queue_status(&paths, &state, &site_id))
        .await
        .map_err(|e| DevStackError::Message(e.to_string()))?
}

// --- 7. Mobile preview -----------------------------------------------------------

#[tauri::command]
pub async fn get_mobile_preview_status(ctx: Ctx<'_>) -> Result<MobilePreviewInfo> {
    let state = ctx.state.lock().await;
    Ok(crate::services::mobile_preview::status(&state))
}

#[tauri::command]
pub async fn start_mobile_preview(ctx: Ctx<'_>, site_id: String) -> Result<MobilePreviewInfo> {
    let mut state = ctx.state.lock().await;
    crate::services::mobile_preview::start(&ctx.paths, &mut state, &site_id)
}

#[tauri::command]
pub async fn stop_mobile_preview(ctx: Ctx<'_>) -> Result<MobilePreviewInfo> {
    let mut state = ctx.state.lock().await;
    crate::services::mobile_preview::stop(&ctx.paths, &mut state)
}

// --- 8. Performance --------------------------------------------------------------

#[tauri::command]
pub async fn get_metrics_snapshot(ctx: Ctx<'_>) -> Result<MetricsSnapshot> {
    let state = ctx.state.lock().await.clone();
    tokio::task::spawn_blocking(move || crate::services::metrics::snapshot(&state))
        .await
        .map_err(|e| DevStackError::Message(e.to_string()))
}

#[tauri::command]
pub async fn get_slow_query_info(ctx: Ctx<'_>, limit: Option<usize>) -> Result<SlowQueryInfo> {
    let (paths, state) = (ctx.paths.clone(), ctx.state.lock().await.clone());
    tokio::task::spawn_blocking(move || {
        crate::services::metrics::slow_query_info(&paths, &state, limit.unwrap_or(50))
    })
    .await
    .map_err(|e| DevStackError::Message(e.to_string()))
}

#[tauri::command]
pub async fn set_slow_query_log(
    ctx: Ctx<'_>,
    enabled: bool,
    threshold_secs: Option<f64>,
) -> Result<SlowQueryInfo> {
    let (paths, state) = (ctx.paths.clone(), ctx.state.lock().await.clone());
    tokio::task::spawn_blocking(move || {
        crate::services::metrics::set_slow_query_log(&paths, &state, enabled, threshold_secs.unwrap_or(1.0))
    })
    .await
    .map_err(|e| DevStackError::Message(e.to_string()))?
}

#[tauri::command]
pub async fn get_spx_status(ctx: Ctx<'_>, php_version: Option<String>) -> Result<SpxStatus> {
    let state = ctx.state.lock().await;
    Ok(crate::services::metrics::spx_status(&ctx.paths, &state, php_version.as_deref()))
}

#[tauri::command]
pub async fn set_spx_enabled(
    ctx: Ctx<'_>,
    enabled: bool,
    php_version: Option<String>,
) -> Result<SpxStatus> {
    let mut state = ctx.state.lock().await;
    crate::services::metrics::set_spx(&ctx.paths, &mut state, php_version.as_deref(), enabled)
}

// --- 9. Config export / import ---------------------------------------------------

#[tauri::command]
pub async fn export_config(
    app: AppHandle,
    ctx: Ctx<'_>,
    dest_path: String,
    include_databases: Option<bool>,
) -> Result<ExportSummary> {
    let progress = job_progress_emitter(app);
    let (paths, state) = (ctx.paths.clone(), ctx.state.lock().await.clone());
    let dest = PathBuf::from(dest_path);
    tokio::task::spawn_blocking(move || {
        crate::services::config_transfer::export_config(
            &paths,
            &state,
            &dest,
            include_databases.unwrap_or(false),
            &progress,
            "config-export",
        )
    })
    .await
    .map_err(|e| DevStackError::Message(e.to_string()))?
}

#[tauri::command]
pub async fn inspect_config_archive(path: String) -> Result<ArchiveMeta> {
    crate::services::config_transfer::inspect_archive(&PathBuf::from(path))
}

#[tauri::command]
pub async fn import_config(
    app: AppHandle,
    ctx: Ctx<'_>,
    path: String,
    options: Option<ImportOptions>,
) -> Result<ImportReport> {
    let progress = job_progress_emitter(app);
    let mut state = ctx.state.lock().await;
    let opts = options.unwrap_or(ImportOptions {
        replace_sites: false,
        restore_settings: true,
        restore_databases: false,
    });
    let report = crate::services::config_transfer::import_config(
        &ctx.paths,
        &mut state,
        &PathBuf::from(path),
        &opts,
        &progress,
        "config-import",
    )?;
    let _ = sync_hosts_after_change(&ctx.paths, &mut state);
    save_state(&ctx.paths.state_file, &state)?;
    Ok(report)
}
