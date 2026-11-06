use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::config::{
    ensure_demo_site, load_state, save_state, AppState, Paths, SiteRecord,
};
use crate::downloads::{noop_progress, ProgressEvent, ProgressFn};
use crate::error::Result;
use crate::packages::{
    installed_from_state, InstalledPackage, PackageInfo, PackageRegistry, PackageVersion,
};
use crate::php::{list_php_extensions, set_xdebug, PhpExtensionView};
use crate::services::sites::{create_site, detect_site, SiteDetails};
use crate::services::stack::{
    apply_stack, reload_sites, stack_status, start_stack, stop_stack, StackStatus,
};
use crate::services::sites::refresh_site_metadata;

pub struct AppCtx {
    pub paths: Paths,
    pub state: Mutex<AppState>,
    pub registry: PackageRegistry,
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

#[tauri::command]
pub async fn list_packages(ctx: State<'_, Arc<AppCtx>>) -> Result<Vec<PackageInfo>> {
    Ok(ctx.registry.list_packages())
}

#[tauri::command]
pub async fn list_package_versions(
    ctx: State<'_, Arc<AppCtx>>,
    package_id: String,
) -> Result<Vec<PackageVersion>> {
    let provider = ctx.registry.get(&package_id)?;
    let state = ctx.state.lock().await;
    provider.list_versions(&state).await
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
    ensure_demo_site(&mut state);
    refresh_site_metadata(&mut state);
    save_state(&ctx.paths.state_file, &state)?;
    Ok(state.sites.clone())
}

#[tauri::command]
pub async fn list_site_details(ctx: State<'_, Arc<AppCtx>>) -> Result<Vec<SiteDetails>> {
    let mut state = ctx.state.lock().await;
    ensure_demo_site(&mut state);
    refresh_site_metadata(&mut state);
    let details: Vec<_> = state.sites.iter().map(detect_site).collect();
    save_state(&ctx.paths.state_file, &state)?;
    Ok(details)
}

#[tauri::command]
pub async fn get_site_details(
    ctx: State<'_, Arc<AppCtx>>,
    site_id: String,
) -> Result<SiteDetails> {
    let mut state = ctx.state.lock().await;
    ensure_demo_site(&mut state);
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .ok_or_else(|| crate::error::DevStackError::Message(format!("site not found: {site_id}")))?
        .clone();
    Ok(detect_site(&site))
}

#[tauri::command]
pub async fn create_site_cmd(
    ctx: State<'_, Arc<AppCtx>>,
    name: String,
    path: String,
    php_version: Option<String>,
) -> Result<SiteRecord> {
    let mut state = ctx.state.lock().await;
    let site = create_site(
        &mut state,
        &ctx.paths,
        &name,
        PathBuf::from(path),
        php_version,
    )?;
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
) -> Result<SiteRecord> {
    let mut state = ctx.state.lock().await;
    ensure_demo_site(&mut state);
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
    let out = site.clone();
    save_state(&ctx.paths.state_file, &state)?;
    // Best-effort reload if stack is up
    let _ = reload_sites(&ctx.paths, &mut state);
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

/// Headless helper used by CI / smoke tests without a UI.
#[allow(dead_code)]
pub async fn smoke_list_node(ctx: &AppCtx) -> Result<Vec<PackageVersion>> {
    let provider = ctx.registry.get("node")?;
    let state = ctx.state.lock().await;
    let _ = noop_progress();
    provider.list_versions(&state).await
}

pub fn bootstrap_ctx(data_root: std::path::PathBuf) -> Result<AppCtx> {
    let paths = Paths::from_root(data_root)?;
    let mut state = load_state(&paths.state_file)?;
    ensure_demo_site(&mut state);
    save_state(&paths.state_file, &state)?;
    Ok(AppCtx {
        paths,
        state: Mutex::new(state),
        registry: PackageRegistry::new()?,
    })
}
