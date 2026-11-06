pub mod commands;
pub mod config;
pub mod downloads;
pub mod error;
pub mod packages;
pub mod php;
pub mod services;

use std::sync::Arc;

use commands::{
    apply_dev_stack, backup_site_database, bootstrap_ctx, create_site_cmd, create_wordpress,
    duplicate_site, get_app_info, get_site_details, get_sites, get_stack_status, get_templates,
    install_package, list_installed, list_package_versions, list_packages,
    list_php_extension_settings, list_site_details, mark_template, register_system_php_cmd,
    restore_site_database, seed_site_large_table, set_php_manifest_url, set_php_xdebug,
    start_dev_stack, stop_dev_stack, uninstall_package, update_site_environment,
};
use config::default_data_root;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Tiny DNS daemon mode for detached helper process.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--devstack-dns") {
        let port = args
            .get(i + 1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(5353u16);
        services::dns::run_dns_daemon(port).expect("dns daemon failed");
        return;
    }

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
            list_site_details,
            get_site_details,
            create_site_cmd,
            update_site_environment,
            list_php_extension_settings,
            set_php_xdebug,
            set_php_manifest_url,
            get_stack_status,
            start_dev_stack,
            stop_dev_stack,
            apply_dev_stack,
            register_system_php_cmd,
            create_wordpress,
            mark_template,
            get_templates,
            duplicate_site,
            backup_site_database,
            restore_site_database,
            seed_site_large_table,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DevStack");
}

pub use commands::AppCtx as DevStackCtx;
pub use config::default_data_root as data_root;
