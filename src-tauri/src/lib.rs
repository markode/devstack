pub mod commands;
pub mod config;
pub mod downloads;
pub mod error;
pub mod packages;
pub mod php;

use std::sync::Arc;

use commands::{
    bootstrap_ctx, get_app_info, get_sites, install_package, list_installed,
    list_package_versions, list_packages, list_php_extension_settings, set_php_manifest_url,
    set_php_xdebug, uninstall_package, update_site_environment,
};
use config::default_data_root;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let ctx = bootstrap_ctx(default_data_root()).expect("failed to init DevStack data dir");
    let ctx = Arc::new(ctx);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(ctx)
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            list_packages,
            list_package_versions,
            list_installed,
            install_package,
            uninstall_package,
            get_sites,
            update_site_environment,
            list_php_extension_settings,
            set_php_xdebug,
            set_php_manifest_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DevStack");
}

/// Expose context construction for integration tests / CLI smoke helpers.
pub use commands::AppCtx as DevStackCtx;
pub use config::default_data_root as data_root;
