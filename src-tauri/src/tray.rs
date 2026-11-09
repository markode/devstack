//! System tray: start/stop stack, open sites, switch PHP, show window, quit.

use std::sync::Arc;

use tauri::image::Image;
use tauri::menu::{CheckMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::commands::{run_apply_stack, run_start_stack, run_stop_stack, AppCtx};
use crate::config::{save_state, AppState};
use crate::services::ports::canonical_site_url;
use crate::services::process::is_running;

const TRAY_ID: &str = "devstack";
const MAX_SITES: usize = 30;

fn stack_running(state: &AppState) -> bool {
    [
        state.runtime.nginx_pid,
        state.runtime.apache_pid,
        state.runtime.database_pid,
    ]
    .into_iter()
    .flatten()
    .any(is_running)
}

fn php_versions(state: &AppState) -> Vec<String> {
    state
        .installed
        .get("php")
        .map(|l| l.iter().map(|r| r.version.clone()).collect())
        .unwrap_or_default()
}

pub fn build_menu(app: &AppHandle, state: &AppState) -> tauri::Result<Menu<tauri::Wry>> {
    let running = stack_running(state);
    let toggle = MenuItemBuilder::with_id(
        "toggle_stack",
        if running { "Stop Stack" } else { "Start Stack" },
    )
    .build(app)?;

    let mut sites = SubmenuBuilder::new(app, "Sites");
    if state.sites.is_empty() {
        let none = MenuItemBuilder::with_id("no_sites", "No sites")
            .enabled(false)
            .build(app)?;
        sites = sites.item(&none);
    }
    for site in state.sites.iter().take(MAX_SITES) {
        let item = MenuItemBuilder::with_id(format!("site:{}", site.id), &site.domain).build(app)?;
        sites = sites.item(&item);
    }
    let sites = sites.build()?;

    let mut php = SubmenuBuilder::new(app, "PHP Version");
    let versions = php_versions(state);
    if versions.is_empty() {
        let none = MenuItemBuilder::with_id("no_php", "No PHP installed")
            .enabled(false)
            .build(app)?;
        php = php.item(&none);
    }
    for v in versions {
        let checked = state.stack.php_version.as_deref() == Some(v.as_str());
        let item = CheckMenuItemBuilder::with_id(format!("php:{v}"), format!("PHP {v}"))
            .checked(checked)
            .build(app)?;
        php = php.item(&item);
    }
    let php = php.build()?;

    let open = MenuItemBuilder::with_id("open", "Open DevStack").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    MenuBuilder::new(app)
        .item(&toggle)
        .item(&sites)
        .item(&php)
        .separator()
        .item(&open)
        .item(&quit)
        .build()
}

/// Rebuild the tray menu from current state (no-op if the tray was not created).
pub async fn refresh(app: &AppHandle, ctx: &Arc<AppCtx>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = ctx.state.lock().await.clone();
    if let Ok(menu) = build_menu(app, &state) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

async fn graceful_quit(app: AppHandle, ctx: Arc<AppCtx>) {
    {
        let mut state = ctx.state.lock().await;
        let _ = crate::services::stack::stop_stack(&ctx.paths, &mut state);
    }
    app.exit(0);
}

async fn set_php_version(app: AppHandle, ctx: Arc<AppCtx>, version: String) {
    let running = {
        let mut state = ctx.state.lock().await;
        state.stack.php_version = Some(version);
        let _ = save_state(&ctx.paths.state_file, &state);
        stack_running(&state)
    };
    if running {
        let _ = run_apply_stack(&app, &ctx).await;
    } else {
        refresh(&app, &ctx).await;
    }
    let _ = app.emit("stack-preferences-changed", ());
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    let ctx: Arc<AppCtx> = app.state::<Arc<AppCtx>>().inner().clone();
    let app = app.clone();
    match id {
        "toggle_stack" => {
            tauri::async_runtime::spawn(async move {
                let running = stack_running(&*ctx.state.lock().await);
                let _ = if running {
                    run_stop_stack(&app, &ctx).await
                } else {
                    run_start_stack(&app, &ctx).await
                };
            });
        }
        "open" => show_main(&app),
        "quit" => {
            tauri::async_runtime::spawn(graceful_quit(app, ctx));
        }
        other => {
            if let Some(site_id) = other.strip_prefix("site:") {
                let site_id = site_id.to_string();
                tauri::async_runtime::spawn(async move {
                    let url = {
                        let state = ctx.state.lock().await;
                        state
                            .sites
                            .iter()
                            .find(|s| s.id == site_id)
                            .map(|s| canonical_site_url(s, &state))
                    };
                    if let Some(url) = url {
                        let _ = app.opener().open_url(url, None::<&str>);
                    }
                });
            } else if let Some(version) = other.strip_prefix("php:") {
                let version = version.to_string();
                tauri::async_runtime::spawn(set_php_version(app, ctx, version));
            }
        }
    }
}

/// Install the tray with a lightweight placeholder menu.
///
/// Never call `block_on` here — Tauri's `.setup()` already runs on the async
/// runtime, and nesting `block_on` deadlocks the main thread (Windows AppHangB1).
/// Populate the real menu asynchronously via [`refresh`].
pub fn setup(app: &AppHandle, _ctx: &Arc<AppCtx>) -> tauri::Result<()> {
    // Empty default state: Sites/PHP submenus show "none" until refresh fills them.
    let menu = build_menu(app, &AppState::default())?;
    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("DevStack")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .build(app)?;
    Ok(())
}
