//! System tray: start/stop stack, open sites, switch PHP, show window, quit.

use std::sync::Arc;

use serde::Serialize;
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

/// Platform-agnostic tray menu model (unit-tested; built into a Tauri menu at runtime).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayMenuSpec {
    pub items: Vec<TrayItemSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TrayItemSpec {
    Action {
        id: String,
        label: String,
        enabled: bool,
    },
    Check {
        id: String,
        label: String,
        checked: bool,
    },
    Submenu {
        label: String,
        children: Vec<TrayItemSpec>,
    },
    Separator,
}

/// Build the logical tray menu for the current app state.
pub fn menu_spec(state: &AppState) -> TrayMenuSpec {
    let running = stack_running(state);
    let mut items = vec![TrayItemSpec::Action {
        id: "toggle_stack".into(),
        label: if running {
            "Stop Stack".into()
        } else {
            "Start Stack".into()
        },
        enabled: true,
    }];

    let site_children = if state.sites.is_empty() {
        vec![TrayItemSpec::Action {
            id: "no_sites".into(),
            label: "No sites".into(),
            enabled: false,
        }]
    } else {
        state
            .sites
            .iter()
            .take(MAX_SITES)
            .map(|site| TrayItemSpec::Action {
                id: format!("site:{}", site.id),
                label: site.domain.clone(),
                enabled: true,
            })
            .collect()
    };
    items.push(TrayItemSpec::Submenu {
        label: "Sites".into(),
        children: site_children,
    });

    let versions = php_versions(state);
    let php_children = if versions.is_empty() {
        vec![TrayItemSpec::Action {
            id: "no_php".into(),
            label: "No PHP installed".into(),
            enabled: false,
        }]
    } else {
        versions
            .into_iter()
            .map(|v| TrayItemSpec::Check {
                id: format!("php:{v}"),
                label: format!("PHP {v}"),
                checked: state.stack.php_version.as_deref() == Some(v.as_str()),
            })
            .collect()
    };
    items.push(TrayItemSpec::Submenu {
        label: "PHP Version".into(),
        children: php_children,
    });
    items.push(TrayItemSpec::Separator);
    items.push(TrayItemSpec::Action {
        id: "open".into(),
        label: "Open DevStack".into(),
        enabled: true,
    });
    items.push(TrayItemSpec::Action {
        id: "quit".into(),
        label: "Quit".into(),
        enabled: true,
    });
    TrayMenuSpec { items }
}

/// Flatten action/check ids for assertions (skips separators).
pub fn menu_action_ids(spec: &TrayMenuSpec) -> Vec<String> {
    fn walk(items: &[TrayItemSpec], out: &mut Vec<String>) {
        for it in items {
            match it {
                TrayItemSpec::Action { id, .. } | TrayItemSpec::Check { id, .. } => {
                    out.push(id.clone());
                }
                TrayItemSpec::Submenu { children, .. } => walk(children, out),
                TrayItemSpec::Separator => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&spec.items, &mut out);
    out
}

pub fn build_menu(app: &AppHandle, state: &AppState) -> tauri::Result<Menu<tauri::Wry>> {
    let spec = menu_spec(state);
    let mut builder = MenuBuilder::new(app);
    for item in &spec.items {
        match item {
            TrayItemSpec::Separator => {
                builder = builder.separator();
            }
            TrayItemSpec::Action { id, label, enabled } => {
                let mi = MenuItemBuilder::with_id(id, label)
                    .enabled(*enabled)
                    .build(app)?;
                builder = builder.item(&mi);
            }
            TrayItemSpec::Check { id, label, checked } => {
                let mi = CheckMenuItemBuilder::with_id(id, label)
                    .checked(*checked)
                    .build(app)?;
                builder = builder.item(&mi);
            }
            TrayItemSpec::Submenu { label, children } => {
                let mut sub = SubmenuBuilder::new(app, label);
                for child in children {
                    match child {
                        TrayItemSpec::Action { id, label, enabled } => {
                            let mi = MenuItemBuilder::with_id(id, label)
                                .enabled(*enabled)
                                .build(app)?;
                            sub = sub.item(&mi);
                        }
                        TrayItemSpec::Check { id, label, checked } => {
                            let mi = CheckMenuItemBuilder::with_id(id, label)
                                .checked(*checked)
                                .build(app)?;
                            sub = sub.item(&mi);
                        }
                        _ => {}
                    }
                }
                builder = builder.item(&sub.build()?);
            }
        }
    }
    builder.build()
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

/// Dispatch a tray menu action by id. Used by the live tray and by Windows smoke/matrix.
pub async fn dispatch_action(app: &AppHandle, ctx: &Arc<AppCtx>, id: &str) -> Result<(), String> {
    match id {
        "toggle_stack" => {
            let running = stack_running(&*ctx.state.lock().await);
            if running {
                run_stop_stack(app, ctx)
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                run_start_stack(app, ctx)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        "open" => {
            show_main(app);
            Ok(())
        }
        "quit" => {
            // Smoke/matrix callers should not exit the process; live tray uses handle_menu_event.
            Err("quit must be handled by the live tray (would exit the process)".into())
        }
        "no_sites" | "no_php" => Ok(()),
        other => {
            if let Some(site_id) = other.strip_prefix("site:") {
                let url = {
                    let state = ctx.state.lock().await;
                    state
                        .sites
                        .iter()
                        .find(|s| s.id == site_id)
                        .map(|s| canonical_site_url(s, &state))
                };
                if let Some(url) = url {
                    app.opener()
                        .open_url(url, None::<&str>)
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            } else if let Some(version) = other.strip_prefix("php:") {
                set_php_version(app.clone(), ctx.clone(), version.to_string()).await;
                Ok(())
            } else {
                Err(format!("unknown tray action: {other}"))
            }
        }
    }
}

/// Headless tray action for smoke/matrix (no AppHandle): toggle stack / set PHP only.
pub async fn dispatch_headless(ctx: &AppCtx, id: &str) -> Result<String, String> {
    match id {
        "toggle_stack" => {
            let mut state = ctx.state.lock().await;
            let running = stack_running(&state);
            if running {
                crate::services::stack::stop_web_stack(&ctx.paths, &mut state)
                    .map_err(|e| e.to_string())?;
                Ok("stopped".into())
            } else {
                crate::services::stack::apply_stack(&ctx.paths, &mut state)
                    .map_err(|e| e.to_string())?;
                Ok("started".into())
            }
        }
        other if other.starts_with("php:") => {
            let version = other.trim_start_matches("php:").to_string();
            let mut state = ctx.state.lock().await;
            state.stack.php_version = Some(version.clone());
            save_state(&ctx.paths.state_file, &state).map_err(|e| e.to_string())?;
            Ok(format!("php={version}"))
        }
        "open" => Ok("open-noop-headless".into()),
        "quit" => Ok("quit-noop-headless".into()),
        other => Ok(format!("noop:{other}")),
    }
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    let ctx: Arc<AppCtx> = app.state::<Arc<AppCtx>>().inner().clone();
    let app = app.clone();
    match id {
        "quit" => {
            tauri::async_runtime::spawn(graceful_quit(app, ctx));
        }
        other => {
            let id = other.to_string();
            tauri::async_runtime::spawn(async move {
                let _ = dispatch_action(&app, &ctx, &id).await;
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SiteRecord;
    use std::path::PathBuf;

    #[test]
    fn menu_spec_has_core_actions() {
        let mut state = AppState::default();
        state.sites.push(SiteRecord {
            id: "demo".into(),
            name: "Demo".into(),
            path: PathBuf::from("/tmp/demo"),
            domain: "demo.test".into(),
            php_version: Some("8.3".into()),
            node_version: None,
            https: true,
            web_server: Some("nginx".into()),
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: vec![],
            wildcard: false,
        });
        state.installed.insert(
            "php".into(),
            vec![crate::config::InstalledRecord {
                version: "8.3.0".into(),
                path: PathBuf::from("/tmp/php"),
                channel: None,
                installed_at: "2026-01-01T00:00:00Z".into(),
            }],
        );
        state.stack.php_version = Some("8.3.0".into());
        let spec = menu_spec(&state);
        let ids = menu_action_ids(&spec);
        assert!(ids.contains(&"toggle_stack".into()));
        assert!(ids.contains(&"open".into()));
        assert!(ids.contains(&"quit".into()));
        assert!(ids.contains(&"site:demo".into()));
        assert!(ids.iter().any(|id| id.starts_with("php:")));
        assert_eq!(
            spec.items
                .iter()
                .find_map(|i| match i {
                    TrayItemSpec::Action { id, label, .. } if id == "toggle_stack" => {
                        Some(label.clone())
                    }
                    _ => None,
                })
                .as_deref(),
            Some("Start Stack")
        );
    }
}
