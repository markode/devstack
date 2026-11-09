pub mod applog;
pub mod commands;
pub mod config;
pub mod downloads;
pub mod error;
pub mod packages;
pub mod paths_util;
pub mod php;
pub mod services;
pub mod tray;

use std::sync::Arc;

use commands::{
    append_app_log, apply_dev_stack, apply_next_free_ports, backup_site_database, bootstrap_ctx,
    check_service_ports_cmd, clear_app_log_error_count, clear_app_logs, create_site_cmd,
    create_wordpress, delete_site_cmd, download_site_database, duplicate_site,
    export_all_databases_to_folder, export_database_to_folder, get_app_info, get_app_log_error_count,
    get_app_logs, get_backups_dir, get_databases_folder, get_db_files_settings,
    get_developer_settings, get_hosts_status, get_ports_config, get_service_log, get_site_details,
    get_sites, get_stack_status, get_templates, import_database_from_folder,
    import_site_database_file, install_package, list_database_files, list_installed,
    list_managed_services_cmd, list_package_versions, list_packages, list_php_extension_settings,
    list_site_details, list_site_details_probed, mark_template, mint_phpmyadmin_signon_url,
    open_app_log_folder, open_path_cmd, prefetch_version_lists, probe_all_site_reachability,
    register_system_php_cmd, repair_hosts, repair_site_url_cmd, restore_site_database,
    reveal_path_cmd, seed_site_large_table, set_coexistence_mode, set_php_manifest_url,
    set_php_xdebug,
    start_dev_stack, start_managed_service, stop_dev_stack, stop_managed_service,
    uninstall_package, update_db_files_settings, update_developer_settings, update_package,
    update_ports_config, update_site_environment,
    backup_site_full, clear_php_dumps, clone_site, delete_site_backup_cmd, get_backup_settings,
    get_dump_status, get_share_status, get_sites_settings, get_tools_status, get_xdebug_launch_hints,
    install_dev_tools, list_php_dumps, list_site_backups_cmd, open_site_terminal,
    restore_site_backup_cmd, run_due_site_backups_cmd, set_site_xdebug, start_site_share,
    stop_site_share, sync_parked_sites_cmd, update_backup_settings, update_sites_settings,
};
use config::default_data_root;
use services::db_files::{
    databases_folder, export_all_running, new_suppress_set, spawn_folder_watcher,
};
use services::progress::noop_job_progress;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Tiny DNS daemon mode for detached helper process.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--devstack-dns") {
        let port = args
            .get(i + 1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(5353u16);
        // Never panic the helper — Windows bind 10013 (access denied / in use)
        // must fall back to hosts-only DNS without taking down the process tree.
        if let Err(e) = services::dns::run_dns_daemon(port) {
            eprintln!("devstack-dns: {e}");
            std::process::exit(1);
        }
        return;
    }

    let ctx = bootstrap_ctx(default_data_root()).expect("failed to init DevStack data dir");
    let ctx = Arc::new(ctx);
    if let Err(e) = services::dump::start_dump_server(&ctx.paths) {
        eprintln!("devstack dump receiver: {e}");
    }
    prefetch_version_lists(ctx.clone());
    // Auto-sync hosts+CA on launch when the DevStack block is missing (may prompt UAC),
    // and regenerate managed php.ini (Windows extension_dir + modules).
    {
        let launch_ctx = ctx.clone();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async {
                let mut state = launch_ctx.state.lock().await;
                // Persist free ports beside WAMP/MAMP/IIS before the UI opens.
                let before = crate::services::ports::PortSnapshot::from_stack(&state.stack);
                let owned = crate::services::ports::owned_runtime_pids_with_files(
                    &launch_ctx.paths,
                    &state.runtime,
                );
                let runtime_snap = state.runtime.clone();
                let remap = crate::services::ports::auto_separate_busy_ports(
                    &mut state.stack,
                    &owned,
                    &runtime_snap,
                );
                let mut notice = remap.messages.clone();
                notice.extend(crate::services::ports::classic_web_port_notice(&state.stack));
                state.runtime.port_notice = notice;
                if remap.changed {
                    let _ = crate::config::save_state(&launch_ctx.paths.state_file, &state);
                    // Best-effort WP URL rewrite (DB may not be up yet on cold launch).
                    let _ = crate::services::stack::rewrite_all_wp_urls_for_port_remap(
                        &launch_ctx.paths,
                        &state,
                        before,
                    );
                }
                if let Ok(n) = crate::services::parked::sync_parked_sites(&launch_ctx.paths, &mut state) {
                    if n > 0 {
                        let _ = crate::config::save_state(&launch_ctx.paths.state_file, &state);
                    }
                }
                let _ = crate::php::regenerate_all_php_ini(&launch_ctx.paths, &state);
                match crate::services::dns::ensure_hosts_if_needed(&launch_ctx.paths, &state) {
                    Ok(_) => {}
                    Err(e) => eprintln!("devstack hosts auto-sync: {e}"),
                }
            });
        });
    }

    // Folder watcher + scheduled auto-export for Databases/
    {
        let watch_ctx = ctx.clone();
        let suppress = new_suppress_set();
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok();
        let folder = rt.as_ref().and_then(|rt| {
            rt.block_on(async {
                let state = watch_ctx.state.lock().await;
                databases_folder(&watch_ctx.paths, &state.db_files).ok()
            })
        });
        if let Some(root) = folder {
            let emit_ctx = watch_ctx.clone();
            let _ = spawn_folder_watcher(root, suppress, running.clone(), move |mut ev| {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                rt.block_on(async {
                    let mode = emit_ctx.state.lock().await.db_files.import_mode.clone();
                    ev.import_mode = mode.clone();
                    if mode == "auto" {
                        let progress = noop_job_progress();
                        let state = emit_ctx.state.lock().await;
                        let _ = crate::services::db_files::import_database_file(
                            &emit_ctx.paths,
                            &state,
                            &ev.engine,
                            &ev.database,
                            std::path::Path::new(&ev.path),
                            &progress,
                            "watch-import",
                        );
                    }
                    let _ = ev;
                });
            });
        }

        // Interval auto-export
        let export_ctx = ctx.clone();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            loop {
                let secs = rt.block_on(async {
                    export_ctx.state.lock().await.db_files.auto_export_interval_secs
                });
                if secs == 0 {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    continue;
                }
                std::thread::sleep(std::time::Duration::from_secs(secs.max(30)));
                let progress = noop_job_progress();
                rt.block_on(async {
                    let state = export_ctx.state.lock().await;
                    let _ = export_all_running(
                        &export_ctx.paths,
                        &state,
                        &progress,
                        "auto-export",
                    );
                });
            }
        });
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(ctx)
        .setup(|app| {
            let handle = app.handle().clone();
            {
                let dump_handle = handle.clone();
                crate::services::dump::attach_emitter(move |entry| {
                    use tauri::Emitter;
                    let _ = dump_handle.emit("php-dump", entry);
                });
            }
            {
                use tauri::Manager;
                let ctx: Arc<commands::AppCtx> = app.state::<Arc<commands::AppCtx>>().inner().clone();
                if let Err(e) = tray::setup(&handle, &ctx) {
                    crate::applog::warn("tray", format!("tray unavailable: {e}"));
                }
                spawn_background_jobs(handle.clone(), ctx);
            }
            crate::applog::attach_emitter(move |entry| {
                use tauri::Emitter;
                let _ = handle.emit("app-log", entry);
            });
            // Windows: undecorated chrome needs an explicit shadow so Win11 keeps
            // rounded corners + DWM border (resize / Snap still work via system).
            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_decorations(false);
                    let _ = win.set_shadow(true);
                }
            }
            crate::applog::info("app", "Tauri setup complete");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_info,
            get_backups_dir,
            list_packages,
            list_package_versions,
            list_installed,
            install_package,
            uninstall_package,
            get_sites,
            list_site_details,
            list_site_details_probed,
            probe_all_site_reachability,
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
            list_managed_services_cmd,
            start_managed_service,
            stop_managed_service,
            get_service_log,
            update_package,
            get_db_files_settings,
            get_databases_folder,
            update_db_files_settings,
            list_database_files,
            export_database_to_folder,
            export_all_databases_to_folder,
            import_database_from_folder,
            download_site_database,
            import_site_database_file,
            get_ports_config,
            update_ports_config,
            set_coexistence_mode,
            check_service_ports_cmd,
            apply_next_free_ports,
            get_hosts_status,
            repair_hosts,
            repair_site_url_cmd,
            mint_phpmyadmin_signon_url,
            delete_site_cmd,
            open_path_cmd,
            reveal_path_cmd,
            get_app_logs,
            get_app_log_error_count,
            clear_app_logs,
            clear_app_log_error_count,
            open_app_log_folder,
            append_app_log,
            get_developer_settings,
            update_developer_settings,
            get_sites_settings,
            update_sites_settings,
            sync_parked_sites_cmd,
            get_share_status,
            start_site_share,
            stop_site_share,
            get_dump_status,
            list_php_dumps,
            clear_php_dumps,
            get_tools_status,
            install_dev_tools,
            get_backup_settings,
            update_backup_settings,
            backup_site_full,
            list_site_backups_cmd,
            restore_site_backup_cmd,
            delete_site_backup_cmd,
            run_due_site_backups_cmd,
            set_site_xdebug,
            get_xdebug_launch_hints,
            open_site_terminal,
            clone_site,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DevStack");
}

pub use commands::AppCtx as DevStackCtx;
pub use config::default_data_root as data_root;

/// Parked-folder watcher + scheduled full-site backups.
fn spawn_background_jobs(app: tauri::AppHandle, ctx: Arc<commands::AppCtx>) {
    let (watch, dirs) = tauri::async_runtime::block_on(async {
        let s = ctx.state.lock().await;
        (s.sites_settings.watch_enabled, s.sites_settings.parked_dirs.clone())
    });
    if watch {
        let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (app, ctx) = (app.clone(), ctx.clone());
        let _ = services::parked::spawn_parked_watcher(dirs, running, move || {
            let (app, ctx) = (app.clone(), ctx.clone());
            tauri::async_runtime::spawn(async move {
                let _ = commands::sync_parked_now(Some(&app), &ctx).await;
            });
        });
    }

    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let snapshot = tauri::async_runtime::block_on(async {
            let s = ctx.state.lock().await;
            (s.backup_settings.interval_secs > 0).then(|| s.clone())
        });
        let Some(state) = snapshot else { continue };
        let emit_app = app.clone();
        let progress: services::progress::JobProgressFn = Arc::new(move |ev| {
            use tauri::Emitter;
            let _ = emit_app.emit("job-progress", ev);
        });
        let _ = services::site_backup::run_due_backups(&ctx.paths, &state, &progress);
    });
}
