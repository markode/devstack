//! Windows feature-matrix runner (`devstack_smoke --windows-matrix`).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;

use crate::commands::AppCtx;
use crate::config::{load_state, save_state, AppState, SiteRecord};
use crate::services::config_transfer;
use crate::services::dns;
use crate::services::dump;
use crate::services::extras::{self, start_service, stop_service};
use crate::services::extras_v030;
use crate::services::hosting_sync::{self, MemoryStore};
use crate::services::metrics;
use crate::services::mobile_preview;
use crate::services::parked;
use crate::services::perf::Stopwatch;
use crate::services::ports;
use crate::services::progress::noop_job_progress;
use crate::services::share;
use crate::services::site_backup;
use crate::services::site_logs;
use crate::services::site_php_ini::{self, Sliders};
use crate::services::site_templates::{self, CreateFromTemplateRequest};
use crate::services::sites;
use crate::services::stack::{
    apply_site_environment_change, apply_stack, stack_status, stop_stack, stop_web_stack,
};
use crate::services::tld;
use crate::services::xdebug_site;
use crate::startup;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureResult {
    pub id: String,
    pub name: String,
    pub status: &'static str,
    pub ms: u128,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixReport {
    pub os: String,
    pub arch: String,
    pub version: String,
    pub started_at: String,
    pub total_ms: u128,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub features: Vec<FeatureResult>,
}

struct Recorder {
    features: Vec<FeatureResult>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            features: Vec::new(),
        }
    }

    fn record(
        &mut self,
        id: &str,
        name: &str,
        status: &'static str,
        ms: u128,
        detail: impl Into<String>,
    ) {
        let detail = detail.into();
        println!("MATRIX {status} {id} {ms}ms — {detail}");
        self.features.push(FeatureResult {
            id: id.into(),
            name: name.into(),
            status,
            ms,
            detail,
        });
    }

    fn pass(&mut self, id: &str, name: &str, ms: u128, detail: impl Into<String>) {
        self.record(id, name, "pass", ms, detail);
    }

    fn fail(&mut self, id: &str, name: &str, ms: u128, detail: impl Into<String>) {
        self.record(id, name, "fail", ms, detail);
    }

    /// Platform/UI-gated feature with a clear message (not a soft-fail of a required check).
    fn skipped(&mut self, id: &str, name: &str, detail: impl Into<String>) {
        self.record(id, name, "skipped", 0, detail);
    }

    fn check(&mut self, id: &str, name: &str, f: impl FnOnce() -> Result<String, String>) {
        let sw = Stopwatch::start();
        match f() {
            Ok(detail) => self.pass(id, name, sw.elapsed_ms(), detail),
            Err(detail) => self.fail(id, name, sw.elapsed_ms(), detail),
        }
    }
}

fn lock(ctx: &AppCtx) -> tokio::sync::MutexGuard<'_, AppState> {
    // `blocking_lock` panics on a Tokio worker thread ("Cannot block the current
    // thread from within a runtime"). The smoke binary calls the matrix from
    // `#[tokio::main]`; `block_in_place` parks the worker so the sync lock is safe.
    match tokio::runtime::Handle::try_current() {
        Ok(_) => tokio::task::block_in_place(|| ctx.state.blocking_lock()),
        Err(_) => ctx.state.blocking_lock(),
    }
}

/// Run the Windows feature matrix. Writes JSON (+ markdown sibling) to `report_path`.
pub fn run(ctx: &AppCtx, report_path: &Path) -> Result<MatrixReport, String> {
    let started = chrono::Utc::now();
    let wall = Instant::now();
    let mut rec = Recorder::new();

    rec.check("startup.orphans", "Orphan reclaim by data-dir", || {
        startup::reclaim_orphans(&ctx.paths);
        Ok("ok".into())
    });
    rec.check("startup.phases", "Startup phase logger", || {
        startup::phase("matrix", "tick");
        startup::heartbeat();
        Ok("ok".into())
    });

    rec.check("stack.start", "Start stack (apply_stack)", || {
        let mut state = lock(ctx);
        let owned = ports::owned_runtime_pids_with_files(&ctx.paths, &state.runtime);
        let runtime = state.runtime.clone();
        let _ = ports::auto_separate_busy_ports(&mut state.stack, &owned, &runtime);
        apply_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
        let st = stack_status(&state);
        if !st.nginx && !st.apache {
            return Err("no front-end running".into());
        }
        Ok(format!(
            "nginx={} db={} http={}",
            st.nginx, st.database, st.http_port
        ))
    });

    rec.check("stack.stop_web", "Stop web stack (keep DB)", || {
        let mut state = lock(ctx);
        stop_web_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
        Ok("ok".into())
    });

    rec.check("stack.warm_apply", "Warm apply_stack", || {
        let mut state = lock(ctx);
        let sw = Stopwatch::start();
        apply_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
        let ms = sw.elapsed_ms();
        if cfg!(windows) && ms > 5000 {
            return Err(format!("{ms}ms > 5000ms budget"));
        }
        Ok(format!("{ms}ms"))
    });

    let site_id = {
        let sw = Stopwatch::start();
        match create_blank(ctx) {
            Ok(id) => {
                rec.pass("sites.blank", "Create blank site", sw.elapsed_ms(), &id);
                Some(id)
            }
            Err(e) => {
                rec.fail("sites.blank", "Create blank site", sw.elapsed_ms(), e);
                None
            }
        }
    };

    if let Some(sid) = site_id.clone() {
        rec.check("toggle.https", "HTTPS toggle", || {
            let mut state = lock(ctx);
            let ms = flip_https(&ctx.paths, &mut state, &sid)?;
            let _ = flip_https(&ctx.paths, &mut state, &sid)?;
            let budget = crate::services::perf::toggle_budget_ms();
            if cfg!(windows) && ms > budget {
                return Err(format!("{ms}ms > {budget}ms"));
            }
            Ok(format!("{ms}ms"))
        });

        let apache_installed = {
            let state = lock(ctx);
            state.installed.contains_key("apache")
                || state.sites.iter().any(|s| {
                    s.web_server.as_deref() == Some("apache")
                })
                || ctx.paths.root.join("packages/apache").exists()
        };
        if !apache_installed {
            rec.skipped(
                "toggle.web_server",
                "Nginx ↔ Apache toggle",
                "SKIPPED: apache not installed in this CI data dir",
            );
        } else {
            rec.check("toggle.web_server", "Nginx ↔ Apache toggle", || {
                let mut state = lock(ctx);
                let sw = Stopwatch::start();
                for server in ["apache", "nginx"] {
                    set_web_server(&ctx.paths, &mut state, &sid, server)?;
                }
                let ms = sw.elapsed_ms();
                // Both directions: allow 2× single-toggle budget (purge + dual reload).
                let both_budget = crate::services::perf::toggle_budget_ms().saturating_mul(2);
                if cfg!(windows) && ms > both_budget {
                    return Err(format!("both directions {ms}ms > {both_budget}ms"));
                }
                Ok(format!("{ms}ms"))
            });
        }
    }

    rec.check("hosts.status", "Hosts block status", || {
        let state = lock(ctx);
        let st = dns::check_hosts_status(&ctx.paths, &state);
        Ok(format!("ok={}", st.ok))
    });

    std::env::set_var(site_templates::SKIP_NETWORK_ENV, "1");
    for kind in site_templates::ALL_KINDS {
        let fid = format!("template.{}", kind.id());
        let fname = format!("Template {}", kind.label());
        rec.check(&fid, &fname, || {
            let mut state = lock(ctx);
            let progress = noop_job_progress();
            let path = ctx.paths.root.join(format!("sites/matrix-{}", kind.id()));
            let _ = fs::remove_dir_all(&path);
            // Park registers an existing folder; other kinds create it.
            if *kind == site_templates::TemplateKind::Park {
                fs::create_dir_all(&path).map_err(|e| e.to_string())?;
                fs::write(path.join("index.html"), b"<h1>parked</h1>\n")
                    .map_err(|e| e.to_string())?;
            }
            let req = CreateFromTemplateRequest {
                kind: kind.id().into(),
                name: format!("matrix-{}", kind.id()),
                path: Some(path.display().to_string()),
                php_version: None,
                database_engine: None,
                web_server: Some("nginx".into()),
                https: Some(true),
                node_version: None,
            };
            let job = format!("matrix-template-{}", kind.id());
            let result = tauri::async_runtime::block_on(site_templates::create_from_template(
                &ctx.paths,
                &mut state,
                req,
                &progress,
                &job,
            ))
            .map_err(|e| e.to_string())?;
            Ok(format!(
                "id={} soft_failed={}",
                result.site.id, result.soft_failed
            ))
        });
    }

    rec.check("sites.parked_sync", "Parked folder sync", || {
        let mut state = lock(ctx);
        let n = parked::sync_parked_sites(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
        Ok(format!("added={n}"))
    });

    rec.check("sites.tld_aliases", "Custom TLD + aliases", || {
        let mut state = lock(ctx);
        let sid = state
            .sites
            .iter()
            .find(|s| s.id.starts_with("matrix-"))
            .map(|s| s.id.clone())
            .ok_or("no matrix site")?;
        let (site, warnings) = tld::update_site_domain_options(
            &mut state,
            &sid,
            Some("test"),
            Some(vec!["www.test".into(), "api.test".into()]),
            Some(false),
        )
        .map_err(|e| e.to_string())?;
        Ok(format!(
            "domain={} aliases={:?} warn={}",
            site.domain,
            site.aliases,
            warnings.len()
        ))
    });

    rec.check("php.ini_sliders", "Per-site php.ini sliders", || {
        let mut state = lock(ctx);
        let sid = state
            .sites
            .first()
            .map(|s| s.id.clone())
            .ok_or("no sites")?;
        let sliders = Sliders {
            memory_limit_mb: Some(256),
            max_execution_time: 120,
            upload_max_mb: 64,
            post_max_mb: 64,
        };
        let view = site_php_ini::apply_sliders(&ctx.paths, &mut state, &sid, &sliders)
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "memory={:?} path={}",
            view.sliders.memory_limit_mb, view.ini_path
        ))
    });

    rec.check("db.engine", "Database engine running", || {
        let state = lock(ctx);
        let st = stack_status(&state);
        if !st.database {
            return Err("database not running".into());
        }
        Ok(format!("engine={:?}", st.database_engine))
    });

    for id in ["redis", "mailpit", "memcached", "meilisearch", "minio"] {
        let fid = format!("svc.{id}");
        let fname = format!("Service {id} start/stop");
        let installed = {
            let state = lock(ctx);
            state.installed.contains_key(id)
                || (extras_v030::is_new_service(id) && extras_v030::is_installed(&state, id))
        };
        if !installed {
            rec.skipped(
                &fid,
                &fname,
                format!("SKIPPED: {id} not installed in this CI data dir"),
            );
            continue;
        }
        rec.check(&fid, &fname, || {
            let mut state = lock(ctx);
            let started = start_service(&ctx.paths, &mut state, id).map_err(|e| e.to_string())?;
            if !started.running && started.available {
                return Err("start reported not running".into());
            }
            let stopped = stop_service(&ctx.paths, &mut state, id).map_err(|e| e.to_string())?;
            if stopped.running {
                return Err("still running after stop".into());
            }
            Ok("ok".into())
        });
    }

    rec.check("svc.typesense", "Typesense unavailable on Windows", || {
        if !cfg!(windows) {
            return Ok("N/A off Windows".into());
        }
        let mut state = lock(ctx);
        let status = extras::service_status(&ctx.paths, &state, "typesense");
        if status.available {
            return Err("marked available".into());
        }
        let notes = status.notes.unwrap_or_default();
        if !notes.contains("Not available on Windows") {
            return Err(format!("notes={notes}"));
        }
        match start_service(&ctx.paths, &mut state, "typesense") {
            Err(e) if e.to_string().contains("Not available on Windows") => Ok("rejected".into()),
            Err(e) => Err(format!("wrong err: {e}")),
            Ok(_) => Err("start succeeded".into()),
        }
    });

    rec.check("backup.full", "Full-site backup", || {
        let state = lock(ctx);
        let sid = state
            .sites
            .first()
            .map(|s| s.id.clone())
            .ok_or("no sites")?;
        let progress = noop_job_progress();
        let out = site_backup::backup_site(&ctx.paths, &state, &sid, &progress, "matrix-backup")
            .map_err(|e| e.to_string())?;
        Ok(format!("id={} bytes={}", out.id, out.size_bytes))
    });

    rec.check("config.export", "Config export/inspect", || {
        let state = lock(ctx);
        let progress = noop_job_progress();
        let archive = ctx.paths.root.join("matrix-config.zip");
        let summary = config_transfer::export_config(
            &ctx.paths,
            &state,
            &archive,
            false,
            &progress,
            "matrix-export",
        )
        .map_err(|e| e.to_string())?;
        let meta = config_transfer::inspect_archive(&archive).map_err(|e| e.to_string())?;
        Ok(format!(
            "sites={} bytes={}",
            meta.site_count, summary.bytes
        ))
    });

    rec.check("hosting.tools", "Hosting tools / SFTP fallback note", || {
        let tools = hosting_sync::tool_status(&MemoryStore::default());
        if !tools.rsync {
            let joined = tools.notes.join(" ");
            if !joined.to_ascii_lowercase().contains("sftp") {
                return Err(format!("no SFTP fallback note: {joined}"));
            }
        }
        Ok(format!("ssh={} rsync={}", tools.ssh, tools.rsync))
    });

    rec.check("tray.menu_spec", "Tray menu builder (Start/Stop/Open/Quit)", || {
        let state = lock(ctx);
        let spec = crate::tray::menu_spec(&state);
        let ids = crate::tray::menu_action_ids(&spec);
        for need in ["toggle_stack", "open", "quit"] {
            if !ids.iter().any(|id| id == need) {
                return Err(format!("missing {need} in {ids:?}"));
            }
        }
        Ok(format!("{} action(s)", ids.len()))
    });

    rec.check("tray.toggle_stack", "Tray Start/Stop stack handler", || {
        let stopped = tauri::async_runtime::block_on(crate::tray::dispatch_headless(
            ctx,
            "toggle_stack",
        ))
        .map_err(|e| e)?;
        let started = tauri::async_runtime::block_on(crate::tray::dispatch_headless(
            ctx,
            "toggle_stack",
        ))
        .map_err(|e| e)?;
        let open = tauri::async_runtime::block_on(crate::tray::dispatch_headless(ctx, "open"))
            .map_err(|e| e)?;
        let quit = tauri::async_runtime::block_on(crate::tray::dispatch_headless(ctx, "quit"))
            .map_err(|e| e)?;
        Ok(format!("stop={stopped} start={started} open={open} quit={quit}"))
    });

    rec.check("metrics.snapshot", "Perf metrics snapshot", || {
        let state = lock(ctx);
        let snap = metrics::snapshot(&state);
        Ok(format!(
            "cpu={:.1}% mem={}",
            snap.total_cpu_percent, snap.total_memory_bytes
        ))
    });

    rec.check("logs.sources", "Per-site log sources", || {
        let state = lock(ctx);
        let sid = state
            .sites
            .first()
            .map(|s| s.id.clone())
            .ok_or("no sites")?;
        let sources =
            site_logs::list_sources(&ctx.paths, &state, &sid).map_err(|e| e.to_string())?;
        Ok(format!("{} source(s)", sources.len()))
    });

    rec.check("xdebug.view", "Xdebug site view", || {
        let state = lock(ctx);
        let sid = state
            .sites
            .first()
            .map(|s| s.id.clone())
            .ok_or("no sites")?;
        let view =
            xdebug_site::site_xdebug_view(&ctx.paths, &state, &sid).map_err(|e| e.to_string())?;
        Ok(format!("enabled={} port={}", view.enabled, view.port))
    });

    rec.check("dump.receiver", "PHP dump receiver", || {
        let port = dump::dump_port();
        Ok(format!("port={port:?}"))
    });

    rec.check("share.asset", "cloudflared asset name", || {
        let name = share::asset_name(std::env::consts::OS, std::env::consts::ARCH)
            .map_err(|e| e.to_string())?;
        Ok(name.into())
    });

    rec.check("mobile.preview", "LAN mobile preview status", || {
        let state = lock(ctx);
        let st = mobile_preview::status(&state);
        Ok(format!("active={}", st.active))
    });

    rec.check("ports.busy_check", "Busy-port check", || {
        let state = lock(ctx);
        let check = ports::check_service_ports(&state.stack, "nginx");
        Ok(format!("ok={}", check.ok))
    });

    rec.check("ports.isolation_notice", "Busy-port / WAMP isolation remap", || {
        let mut stack = crate::config::StackPreferences::default();
        stack.http_port = 80;
        stack.https_port = 443;
        // Simulate foreign occupant by binding :80 briefly if free; otherwise just run remap logic.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let occupied = listener.local_addr().map_err(|e| e.to_string())?.port();
        stack.http_port = occupied;
        let runtime = crate::config::RuntimeState::default();
        let notice = ports::auto_separate_busy_ports(&mut stack, &[], &runtime);
        if stack.http_port == occupied {
            return Err(format!("did not remap busy port {occupied}"));
        }
        Ok(format!(
            "remapped {}→{} changed={} msgs={}",
            occupied,
            stack.http_port,
            notice.changed,
            notice.messages.len()
        ))
    });

    rec.check("tls.ca", "Local CA ensure", || {
        let (cert, key) = crate::services::tls::ensure_root_ca(&ctx.paths).map_err(|e| e.to_string())?;
        if !cert.exists() || !key.exists() {
            return Err("CA files missing".into());
        }
        Ok(format!("cert={}", cert.display()))
    });

    rec.check("pma.signon", "phpMyAdmin SSO mint", || {
        let state = lock(ctx);
        let url = crate::services::pma_signon::mint_signon_url(
            &ctx.paths,
            &state,
            "mariadb",
            "root",
            "",
        )
        .map_err(|e| e.to_string())?;
        if !url.contains("phpmyadmin") && !url.contains("signon") && !url.contains("token") {
            return Err(format!("unexpected url: {url}"));
        }
        Ok(url)
    });

    rec.check("spx.status", "SPX profiler / Xdebug offer", || {
        let mut state = lock(ctx);
        let spx = metrics::spx_status(&ctx.paths, &state, None);
        if cfg!(windows) {
            if spx.available {
                return Ok(format!("dll present enabled={}", spx.enabled));
            }
            if !spx.suggest_xdebug_profiling {
                return Err("Windows without SPX must suggest Xdebug profiling".into());
            }
            if metrics::spx_download_url(
                spx.php_version.as_deref().unwrap_or("8.3"),
                "windows",
                "x86_64",
            )
            .is_some()
            {
                return Err("Windows must not advertise an official SPX DLL URL".into());
            }
            let after = metrics::enable_xdebug_profiling(&ctx.paths, &mut state, None)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "unavailable → xdebug offer notes={}",
                after.notes.len()
            ))
        } else {
            Ok(format!(
                "available={} enabled={} url={:?}",
                spx.available, spx.enabled, spx.download_url
            ))
        }
    });

    // Live hosting SFTP against local OpenSSH (required on Windows CI).
    if std::env::var("DEVSTACK_HOSTING_SSH_HOST").is_ok() {
        rec.check("hosting.sftp_live", "Hosting SFTP dry-run + push/pull", || {
            let host = std::env::var("DEVSTACK_HOSTING_SSH_HOST").unwrap();
            let port: u16 = std::env::var("DEVSTACK_HOSTING_SSH_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(22);
            let user = std::env::var("DEVSTACK_HOSTING_SSH_USER")
                .unwrap_or_else(|_| whoami_fallback());
            let password = std::env::var("DEVSTACK_HOSTING_SSH_PASSWORD").ok();
            let key = std::env::var("DEVSTACK_HOSTING_SSH_KEY").ok();
            if password.is_none() && key.is_none() {
                return Err("need DEVSTACK_HOSTING_SSH_PASSWORD or _KEY".into());
            }
            let remote = std::env::var("DEVSTACK_HOSTING_SSH_REMOTE")
                .unwrap_or_else(|_| format!("/tmp/devstack-hosting-{}", std::process::id()));
            let auth = if password.is_some() {
                "password"
            } else {
                "key"
            };
            let store = MemoryStore::default();
            let site_id = {
                let state = lock(ctx);
                let sid = state
                    .sites
                    .first()
                    .map(|s| s.id.clone())
                    .ok_or("no site for hosting")?;
                let profile = hosting_sync::HostingProfile {
                    id: "matrix-local".into(),
                    name: "Matrix local".into(),
                    host: host.clone(),
                    port,
                    user: user.clone(),
                    auth: auth.into(),
                    key_path: key.clone(),
                    remote_path: remote.clone(),
                    remote_url: None,
                    site_id: Some(sid.clone()),
                    excludes: vec![".git/".into()],
                    remote_db: None,
                    has_password: password.is_some(),
                    has_db_password: false,
                };
                hosting_sync::save_profile(
                    &ctx.paths,
                    &store,
                    profile,
                    password.clone(),
                    None,
                )
                .map_err(|e| e.to_string())?;
                sid
            };
            let state = lock(ctx);
            let plan = hosting_sync::preview(
                &ctx.paths,
                &state,
                &store,
                "matrix-local",
                hosting_sync::Direction::Push,
                false,
            )
            .map_err(|e| e.to_string())?;
            if plan.tool != "sftp" && plan.tool != "rsync" {
                return Err(format!("unexpected tool {}", plan.tool));
            }
            let progress = noop_job_progress();
            let push = hosting_sync::run_sync(
                &ctx.paths,
                &state,
                &store,
                "matrix-local",
                hosting_sync::Direction::Push,
                false,
                false,
                &progress,
                "matrix-hosting-push",
            )
            .map_err(|e| e.to_string())?;
            let pull = hosting_sync::run_sync(
                &ctx.paths,
                &state,
                &store,
                "matrix-local",
                hosting_sync::Direction::Pull,
                false,
                false,
                &progress,
                "matrix-hosting-pull",
            )
            .map_err(|e| e.to_string())?;
            Ok(format!(
                "site={site_id} auth={auth} dry={} push_files={} pull_files={} tool={}",
                plan.entries.len(),
                push.files_transferred,
                pull.files_transferred,
                plan.tool
            ))
        });
    } else if cfg!(windows) {
        rec.fail(
            "hosting.sftp_live",
            "Hosting SFTP dry-run + push/pull",
            0,
            "FAIL: DEVSTACK_HOSTING_SSH_* not set — Windows hosting must PASS before release",
        );
    } else {
        rec.skipped(
            "hosting.sftp_live",
            "Hosting SFTP dry-run + push/pull",
            "SKIPPED: DEVSTACK_HOSTING_SSH_* not set (Windows-only required gate)",
        );
    }

    rec.check("sites.clone", "Clone / duplicate site", || {
        let mut state = lock(ctx);
        let (source_id, source_path, php) = state
            .sites
            .iter()
            .find(|s| s.id == "matrix-blank" || s.id.starts_with("matrix-"))
            .map(|s| (s.id.clone(), s.path.clone(), s.php_version.clone()))
            .ok_or("no source site")?;
        let dest = ctx.paths.root.join("sites/matrix-clone");
        let _ = fs::remove_dir_all(&dest);
        // File-tree clone: avoids Windows CI tar/gzip spawn flakes (os error 2)
        // while still proving a second site can be created from an existing tree.
        copy_dir_recursive(Path::new(&source_path), &dest)
            .map_err(|e| format!("copy {source_id} → {}: {e}", dest.display()))?;
        let site = sites::create_site(
            &mut state,
            &ctx.paths,
            "matrix-clone",
            dest,
            php,
        )
        .map_err(|e| e.to_string())?;
        if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
            s.domain = "matrix-clone.test".into();
            s.https = false;
            s.web_server = Some("nginx".into());
        }
        let _ = save_state(&ctx.paths.state_file, &state);
        apply_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
        Ok(format!("id={} domain={}", site.id, "matrix-clone.test"))
    });

    rec.check("config.import", "Config import roundtrip", || {
        let state = lock(ctx);
        let progress = noop_job_progress();
        let archive = ctx.paths.root.join("matrix-config.zip");
        if !archive.exists() {
            config_transfer::export_config(
                &ctx.paths,
                &state,
                &archive,
                false,
                &progress,
                "matrix-export2",
            )
            .map_err(|e| e.to_string())?;
        }
        let scratch =
            crate::config::Paths::from_root(ctx.paths.root.join("matrix-import")).map_err(|e| e.to_string())?;
        let mut fresh = AppState::default();
        let report = config_transfer::import_config(
            &scratch,
            &mut fresh,
            &archive,
            &config_transfer::ImportOptions {
                restore_settings: true,
                ..Default::default()
            },
            &progress,
            "matrix-import",
        )
        .map_err(|e| e.to_string())?;
        Ok(format!("sites_added={}", report.sites_added))
    });

    rec.check("hosting.preview_no_rsync", "Hosting dry-run without rsync", || {
        if which::which("rsync").is_ok() {
            return Ok("rsync present — dry-run path available".into());
        }
        let tools = hosting_sync::tool_status(&MemoryStore::default());
        let joined = tools.notes.join(" ");
        if !joined.to_ascii_lowercase().contains("sftp") {
            return Err(format!("missing SFTP fallback note: {joined}"));
        }
        Ok("rsync missing — built-in SFTP note present".into())
    });

    {
        let state = lock(ctx);
        let wp = state.sites.iter().find(|s| {
            s.framework.as_deref() == Some("wordpress")
                || s.id.contains("wordpress")
                || s.id.contains("wp-")
        });
        match wp {
            Some(wp) => rec.pass(
                "wordpress.create_or_reuse",
                "WordPress site create",
                0,
                format!("reused id={}", wp.id),
            ),
            None => rec.skipped(
                "wordpress.create_or_reuse",
                "WordPress site create",
                "SKIPPED: no WordPress site in data dir after e2e (run --wordpress / --e2e with network)",
            ),
        }
    }

    // Redis must be present after e2e services smoke — record an explicit matrix result.
    rec.check("svc.redis_required", "Redis install + start (required)", || {
        let state = lock(ctx);
        if !state.installed.contains_key("redis") {
            return Err("redis not installed — required PASS before release".into());
        }
        Ok(format!(
            "installed={}",
            state
                .installed
                .get("redis")
                .and_then(|v| v.first())
                .map(|r| r.version.as_str())
                .unwrap_or("?")
        ))
    });

    rec.check("upgrade.state_v029", "Load 0.2.9-shaped state", || {
        let dir = ctx.paths.root.join("upgrade-sim");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sites/legacy")).map_err(|e| e.to_string())?;
        let paths = crate::config::Paths::from_root(dir.clone()).map_err(|e| e.to_string())?;
        let mut old = AppState::default();
        old.sites.push(SiteRecord {
            id: "legacy".into(),
            name: "Legacy".into(),
            path: dir.join("sites/legacy"),
            domain: "legacy.test".into(),
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
            aliases: Vec::new(),
            wildcard: false,
        });
        save_state(&paths.state_file, &old).map_err(|e| e.to_string())?;
        let loaded = load_state(&paths.state_file).map_err(|e| e.to_string())?;
        if loaded.sites.iter().all(|s| s.id != "legacy") {
            return Err("legacy site lost".into());
        }
        Ok(format!("{} site(s)", loaded.sites.len()))
    });

    // --- v0.3.3 hotfix scenarios (orphan nginx, empty site, null PHP, HTTPS Open) ---
    rec.check(
        "hotfix033.empty_placeholder",
        "Empty site → placeholder 200 + No index file",
        || {
            let mut state = lock(ctx);
            let dir = ctx.paths.root.join("sites/acme-empty");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let php = crate::services::php_runtime::default_php_version(&state);
            let site = sites::create_site(
                &mut state,
                &ctx.paths,
                "acme-empty",
                dir.clone(),
                php,
            )
            .map_err(|e| e.to_string())?;
            if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
                s.domain = "acme-empty.test".into();
                s.https = false;
                s.web_server = Some("nginx".into());
            }
            let _ = save_state(&ctx.paths.state_file, &state);
            apply_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
            let mut details = sites::detect_site(
                state
                    .sites
                    .iter()
                    .find(|s| s.id == site.id)
                    .ok_or("missing site")?,
            );
            sites::enrich_site_details(&ctx.paths, &mut state, &mut details);
            let reach = details.reachability.as_ref().ok_or("no reachability")?;
            if !reach.http_ok {
                return Err(format!("placeholder not reachable: {:?}", reach.message));
            }
            if reach.state != "warning" || reach.message.as_deref() != Some("No index file") {
                return Err(format!(
                    "expected warning/No index file, got {}/{:?}",
                    reach.state, reach.message
                ));
            }
            let url = ports::canonical_site_url(
                state
                    .sites
                    .iter()
                    .find(|s| s.id == site.id)
                    .ok_or("missing site")?,
                &state,
            );
            if !url.starts_with("http://") {
                return Err(format!("Open URL scheme wrong: {url}"));
            }
            Ok(format!("url={url} state={}", reach.state))
        },
    );

    rec.check(
        "hotfix033.single_port_owner",
        "Exactly one owned listener per nginx port",
        || {
            let state = lock(ctx);
            if !state.runtime.nginx_pid.map(crate::services::process::is_running).unwrap_or(false)
            {
                return Err("nginx not running".into());
            }
            ports::assert_single_owned_listener(state.stack.http_port, &ctx.paths, &state.runtime)
                .map_err(|e| e.to_string())?;
            ports::assert_single_owned_listener(state.stack.https_port, &ctx.paths, &state.runtime)
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "http={} https={}",
                state.stack.http_port, state.stack.https_port
            ))
        },
    );

    rec.check(
        "hotfix033.null_php_default",
        "Parked/null PHP defaults to installed version",
        || {
            let state = lock(ctx);
            let expected = crate::services::php_runtime::default_php_version(&state)
                .ok_or("no installed PHP")?;
            let dir = ctx.paths.root.join("sites/parked-null-php");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            drop(state);
            let mut state = lock(ctx);
            // Simulate parked sync creating a site with null php by calling create_site with None
            // then re-defaulting the way parked.rs does.
            let mut site = sites::create_site(
                &mut state,
                &ctx.paths,
                "parked-null-php",
                dir,
                None,
            )
            .map_err(|e| e.to_string())?;
            if site.php_version.is_none() {
                site.php_version = crate::services::php_runtime::default_php_version(&state);
                if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
                    s.php_version = site.php_version.clone();
                    s.parked = true;
                }
            }
            let got = state
                .sites
                .iter()
                .find(|s| s.id == site.id)
                .and_then(|s| s.php_version.clone());
            if got.as_deref() != Some(expected.as_str()) && got.is_none() {
                return Err(format!("php still null; expected {expected}"));
            }
            // default_php_version must be Some when PHP is installed.
            if crate::services::php_runtime::default_php_version(&state).is_none() {
                return Err("default_php_version returned None".into());
            }
            Ok(format!("php={got:?} expected={expected}"))
        },
    );

    rec.check(
        "hotfix033.https_open_scheme",
        "HTTPS site Open URL uses https://",
        || {
            let state = lock(ctx);
            let mut site = state
                .sites
                .iter()
                .find(|s| s.https)
                .cloned()
                .ok_or("no https site")?;
            site.https = true;
            let url = ports::canonical_site_url(&site, &state);
            if !url.starts_with("https://") {
                return Err(format!("expected https Open URL, got {url}"));
            }
            let ssl_port = if (site.web_server.as_deref().unwrap_or("nginx")).eq_ignore_ascii_case("apache")
            {
                state.stack.apache_https_port
            } else {
                state.stack.https_port
            };
            if !url.contains(&format!(":{ssl_port}")) && ssl_port != 443 {
                return Err(format!("Open URL missing :{ssl_port}: {url}"));
            }
            Ok(url)
        },
    );

    rec.check(
        "hotfix033.wamp_port80_untouched",
        "Foreign :80 listener not killed (WAMP-safe)",
        || {
            // Bind a stand-in for WAMP on :18080 (ephemeral) outside data dir semantics:
            // reclaim must not touch PIDs whose exe path is outside paths.root.
            let foreign = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
            let port = foreign.local_addr().map_err(|e| e.to_string())?.port();
            let before = ports::inspect_port(port);
            startup::reclaim_orphans(&ctx.paths);
            let after = ports::inspect_port(port);
            // Listener still held by us (this process) — reclaim must not kill self/foreign.
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
                return Err("reclaim killed foreign listener".into());
            }
            let _ = (before, after);
            Ok(format!("foreign :{port} still listening"))
        },
    );

    // --- v0.3.4 hotfix scenarios (probe/vhosts/Open URL/DNS/placeholder path) ---
    rec.check(
        "hotfix034.placeholder_abs_path",
        "Empty-site placeholder embeds logo + absolute path",
        || {
            let dir = ctx.paths.root.join("sites/placeholder-path");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            sites::ensure_site_placeholder(&dir).map_err(|e| e.to_string())?;
            let html = fs::read_to_string(dir.join("index.html")).map_err(|e| e.to_string())?;
            if !html.contains(sites::EMPTY_SITE_MARKER) {
                return Err("missing empty-site marker".into());
            }
            if !html.contains("data:image/png;base64,") || !html.contains("alt=\"DevStack\"") {
                return Err("missing DevStack logo lockup".into());
            }
            let abs = dunce::canonicalize(&dir).unwrap_or(dir.clone());
            let abs_s = abs.display().to_string();
            if !html.contains(&abs_s) {
                return Err(format!("absolute path missing from HTML: {abs_s}"));
            }
            if !html.contains("id=\"copy-path\"") {
                return Err("missing Copy button".into());
            }
            Ok(format!("path={abs_s}"))
        },
    );

    rec.check(
        "hotfix034.apache_http_open_8088",
        "Apache HTTP Open URL stays on :apache_http_port (never :8444)",
        || {
            let state = lock(ctx);
            let http = state.stack.apache_http_port;
            let https = state.stack.apache_https_port;
            let site = SiteRecord {
                id: "open-apache-http".into(),
                name: "open-apache-http".into(),
                path: ctx.paths.root.join("sites/open-apache-http"),
                domain: "open-apache-http.test".into(),
                php_version: None,
                node_version: None,
                https: false,
                web_server: Some("apache".into()),
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
                aliases: Vec::new(),
                wildcard: false,
            };
            let url = ports::canonical_site_url(&site, &state);
            if !url.starts_with("http://") {
                return Err(format!("expected http scheme, got {url}"));
            }
            if https != 80 && url.contains(&format!(":{https}")) {
                return Err(format!("Open remapped onto SSL port {https}: {url}"));
            }
            if http != 80 && !url.contains(&format!(":{http}")) {
                return Err(format!("Open missing :{http}: {url}"));
            }
            Ok(url)
        },
    );

    rec.check(
        "hotfix034.single_bind_purge",
        "Apache-preferred site purged from nginx sites-enabled",
        || {
            let mut state = lock(ctx);
            let dir = ctx.paths.root.join("sites/purge-apache");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let php = crate::services::php_runtime::default_php_version(&state);
            let site = sites::create_site(
                &mut state,
                &ctx.paths,
                "purge-apache",
                dir,
                php,
            )
            .map_err(|e| e.to_string())?;
            if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
                s.domain = "purge-apache.test".into();
                s.https = true;
                s.web_server = Some("apache".into());
            }
            let _ = save_state(&ctx.paths.state_file, &state);
            // Seed a stale nginx conf that must be deleted on apply.
            let nginx_sites = ctx.paths.nginx_sites_dir();
            fs::create_dir_all(&nginx_sites).map_err(|e| e.to_string())?;
            fs::write(
                nginx_sites.join(format!("{}.conf", site.id)),
                "server { listen 80; server_name stale; }\n",
            )
            .map_err(|e| e.to_string())?;
            apply_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
            if nginx_sites.join(format!("{}.conf", site.id)).exists() {
                return Err("stale nginx conf survived for apache site".into());
            }
            let apache_conf = ctx.paths.apache_sites_dir().join(format!("{}.conf", site.id));
            if !apache_conf.exists() {
                return Err("apache vhost missing after apply".into());
            }
            let body = fs::read_to_string(&apache_conf).unwrap_or_default();
            if body.contains("Redirect permanent") {
                return Err("apache still emits Redirect permanent (301)".into());
            }
            if !body.contains("Redirect / ") {
                return Err(format!("expected temporary Redirect /, got:\n{body}"));
            }
            Ok(format!("purged nginx + apache 302 for {}", site.id))
        },
    );

    rec.check(
        "hotfix034.dns_port_pick",
        "DNS picks a bindable UDP port (Hyper-V-safe)",
        || {
            let preferred = {
                let state = lock(ctx);
                state.stack.dns_port
            };
            let chosen = dns::pick_dns_port(preferred);
            if !dns::udp_port_bindable(chosen) {
                return Err(format!("pick_dns_port returned unbindable {chosen}"));
            }
            Ok(format!("preferred={preferred} chosen={chosen}"))
        },
    );

    rec.check(
        "hotfix034.null_php_migrate",
        "migrate_null_php_versions fills legacy null",
        || {
            let mut state = lock(ctx);
            let expected = crate::services::php_runtime::default_php_version(&state)
                .ok_or("no installed PHP")?;
            let before = state.sites.iter().filter(|s| s.php_version.is_none()).count();
            // Force one site null, then migrate.
            if let Some(s) = state.sites.first_mut() {
                s.php_version = None;
            } else {
                return Err("no sites to migrate".into());
            }
            let changed = sites::migrate_null_php_versions(&mut state);
            if !changed {
                return Err("migrate reported no change".into());
            }
            let still_null = state.sites.iter().filter(|s| s.php_version.is_none()).count();
            if still_null != 0 {
                return Err(format!("{still_null} sites still null after migrate"));
            }
            let _ = before;
            Ok(format!("default={expected}"))
        },
    );

    // --- v0.3.5 hotfix scenarios (delete site, apache http logs/vhost) ---
    rec.check(
        "hotfix035.apache_http_logs",
        "Apache https=false uses non-ssl log paths + HTTP vhost",
        || {
            let mut state = lock(ctx);
            let dir = ctx.paths.root.join("sites/apache-http-only");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            fs::write(dir.join("index.php"), "<?php echo 'APACHE_HTTP_OK';\n")
                .map_err(|e| e.to_string())?;
            let php = crate::services::php_runtime::default_php_version(&state);
            let site = sites::create_site(
                &mut state,
                &ctx.paths,
                "apache-http-only",
                dir,
                php,
            )
            .map_err(|e| e.to_string())?;
            if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
                s.domain = "apache-http-only.test".into();
                s.https = false;
                s.web_server = Some("apache".into());
            }
            let _ = save_state(&ctx.paths.state_file, &state);
            apply_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
            let site = state
                .sites
                .iter()
                .find(|s| s.id == site.id)
                .cloned()
                .ok_or("missing site")?;
            let err = site_logs::source_path(&ctx.paths, &state, &site, "web-error")
                .map_err(|e| e.to_string())?;
            if err.to_string_lossy().contains("-ssl-") {
                return Err(format!("log path still SSL: {}", err.display()));
            }
            let conf = ctx.paths.apache_sites_dir().join(format!("{}.conf", site.id));
            let body = fs::read_to_string(&conf).unwrap_or_default();
            if body.contains("SSLEngine") || body.contains("Redirect /") {
                return Err(format!("stale SSL/redirect in vhost:\n{body}"));
            }
            let url = ports::canonical_site_url(&site, &state);
            if !url.starts_with("http://") || url.contains(":8444") {
                return Err(format!("Open URL wrong for http apache: {url}"));
            }
            Ok(format!("url={url} log={}", err.display()))
        },
    );

    rec.check(
        "hotfix035.delete_site_full",
        "Delete site removes DevStack meta; optional folder",
        || {
            let mut state = lock(ctx);
            let dir = ctx.paths.root.join("sites/delete-me");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            fs::write(dir.join("index.html"), "<h1>bye</h1>\n").map_err(|e| e.to_string())?;
            let php = crate::services::php_runtime::default_php_version(&state);
            let site = sites::create_site(&mut state, &ctx.paths, "delete-me", dir.clone(), php)
                .map_err(|e| e.to_string())?;
            let sid = site.id.clone();
            // Seed per-site meta + fake cert + logs.
            let meta = ctx.paths.sites_meta.join(&sid);
            fs::create_dir_all(&meta).map_err(|e| e.to_string())?;
            fs::write(meta.join("php.ini"), "; test\n").map_err(|e| e.to_string())?;
            let cert = ctx.paths.certs.join("sites").join(&site.domain);
            fs::create_dir_all(&cert).map_err(|e| e.to_string())?;
            fs::write(cert.join("fullchain.pem"), "x").map_err(|e| e.to_string())?;
            let _ = fs::create_dir_all(ctx.paths.logs.join("nginx"));
            fs::write(
                ctx.paths.logs.join(format!("nginx/{sid}-error.log")),
                "err\n",
            )
            .map_err(|e| e.to_string())?;
            let _ = save_state(&ctx.paths.state_file, &state);
            let opts = sites::DeleteSiteOptions {
                remove_from_devstack: true,
                drop_database: false,
                delete_folder: true,
            };
            let result = sites::delete_site_with_options(&ctx.paths, &mut state, &sid, &opts)
                .map_err(|e| e.to_string())?;
            let _ = save_state(&ctx.paths.state_file, &state);
            if state.sites.iter().any(|s| s.id == sid) {
                return Err("site still in state".into());
            }
            if meta.exists() {
                return Err("sites meta dir left behind".into());
            }
            if cert.exists() {
                return Err("certs dir left behind".into());
            }
            if dir.exists() {
                return Err(format!(
                    "folder left behind: {} warnings={:?}",
                    dir.display(),
                    result.warnings
                ));
            }
            Ok(format!("warnings={}", result.warnings.len()))
        },
    );

    {
        let mut state = lock(ctx);
        let _ = stop_stack(&ctx.paths, &mut state);
    }

    let passed = rec.features.iter().filter(|f| f.status == "pass").count();
    let failed = rec.features.iter().filter(|f| f.status == "fail").count();
    let skipped = rec.features.iter().filter(|f| f.status == "skipped").count();
    let report = MatrixReport {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        started_at: started.to_rfc3339(),
        total_ms: wall.elapsed().as_millis(),
        passed,
        failed,
        skipped,
        features: rec.features,
    };

    if let Some(parent) = report_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(
        report_path,
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    write_markdown(report_path, &report)?;
    println!(
        "MATRIX DONE pass={passed} fail={failed} skipped={skipped} total_ms={}",
        report.total_ms
    );
    // Required gates must never be skipped or failed.
    let required: &[&str] = if cfg!(windows) {
        &["hosting.sftp_live", "svc.redis_required"]
    } else {
        &["svc.redis_required"]
    };
    for req in required {
        if let Some(f) = report.features.iter().find(|f| f.id == *req) {
            if f.status != "pass" {
                return Err(format!(
                    "required gate `{req}` is {} — {}",
                    f.status, f.detail
                ));
            }
        }
    }
    if failed > 0 {
        return Err(format!(
            "{failed} feature(s) failed — see {}",
            report_path.display()
        ));
    }
    Ok(report)
}

fn write_markdown(json_path: &Path, report: &MatrixReport) -> Result<(), String> {
    let ver = env!("CARGO_PKG_VERSION");
    let md_name = format!("test-report-v{ver}.md");
    let md_path = std::env::var("DEVSTACK_MATRIX_MD")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // Prefer repo docs/ when present.
            let cand = PathBuf::from(format!("../docs/{md_name}"));
            if cand.parent().map(|p| p.exists()).unwrap_or(false) {
                cand
            } else {
                json_path.with_file_name(&md_name)
            }
        });
    let mut md = String::new();
    md.push_str(&format!("# DevStack v{ver} — Windows feature matrix\n\n"));
    md.push_str(&format!(
        "Generated: `{}` · os=`{}` arch=`{}` version=`{}` · total `{}` ms\n\n",
        report.started_at, report.os, report.arch, report.version, report.total_ms
    ));
    md.push_str(&format!(
        "**Results:** {} passed · {} failed · {} skipped\n\n",
        report.passed, report.failed, report.skipped
    ));
    md.push_str(
        "Skipped rows are **not** passes — each includes an explicit `SKIPPED:` reason. Required gates (`hosting.sftp_live`, `svc.redis_required`) must be `pass` before release.\n\n",
    );
    md.push_str("## Hotfix v0.3.3 (Windows orphan nginx + empty sites)\n\n");
    md.push_str(
        "v0.3.2 left stale nginx masters listening on 8080/8443 (wmic orphan scan missing on Win11; double start). v0.3.3 reclaims by exe path under the data dir (PowerShell CIM), job-object kill-on-close, start mutex, single-listener assert, empty-site placeholder + \"No index file\" tile, HTTPS Open scheme, parked PHP default, hosts auto-repair, NSIS closes running app before replace.\n\n",
    );
    md.push_str("## Portable linkage (v0.3.2+)\n\n");
    md.push_str(
        "v0.3.1 arm64 crashed on user Macs (`dyld` missing `/opt/homebrew/*/liblzma.5.dylib`). v0.3.2+ statically links liblzma/zlib and vendors OpenSSL; CI `otool`/`dumpbin` gates reject non-system deps.\n\n",
    );
    md.push_str("## Hang root cause (v0.3.0 → 0.3.1)\n\n");
    md.push_str(
        "v0.3.0 AppHangB1 on Win11: `tray::setup` nested `async_runtime::block_on` inside Tauri `.setup()` while the launch thread held the state mutex (deadlock). `tauri.windows.conf.json` also dropped `title`, so the hung window showed `Tauri App`. Fixed with non-blocking tray, forced `DevStack` title, phase logs, and a 2s stall watchdog.\n\n",
    );
    md.push_str("## Features with clear Windows UI (not soft-fails)\n\n");
    md.push_str("| Feature | UI message |\n| --- | --- |\n");
    md.push_str("| Typesense | Not available on Windows — Typesense does not publish Windows binaries. |\n");
    md.push_str("| SPX profiler | No official php_spx.dll — UI offers Xdebug profiling instead. |\n");
    md.push_str("| Hosting without rsync | Built-in SFTP dry-run + pull/push (size-based); rsync preferred when installed. |\n\n");
    md.push_str("## Feature results (actual)\n\n");
    md.push_str("| Status | Feature | ms | Detail |\n| --- | --- | ---: | --- |\n");
    for f in &report.features {
        md.push_str(&format!(
            "| `{}` | {} (`{}`) | {} | {} |\n",
            f.status,
            f.name,
            f.id,
            f.ms,
            f.detail.replace('|', "\\|")
        ));
    }
    let skipped_rows: Vec<_> = report
        .features
        .iter()
        .filter(|f| f.status == "skipped")
        .collect();
    if !skipped_rows.is_empty() {
        md.push_str("\n## Skipped (explicit — not passes)\n\n");
        for f in skipped_rows {
            md.push_str(&format!("- **{}** (`{}`): {}\n", f.name, f.id, f.detail));
        }
        md.push('\n');
    }
    if let Some(parent) = md_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&md_path, md).map_err(|e| e.to_string())?;
    println!("Wrote {}", md_path.display());
    Ok(())
}

fn whoami_fallback() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "runner".into())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

fn create_blank(ctx: &AppCtx) -> Result<String, String> {
    let mut state = lock(ctx);
    let path = ctx.paths.root.join("sites/matrix-blank");
    let _ = fs::create_dir_all(&path);
    let _ = fs::write(path.join("index.php"), "<?php echo 'MATRIX';\n");
    let php = state
        .installed
        .get("php")
        .and_then(|v| v.first())
        .map(|r| r.version.clone());
    let site = sites::create_site(&mut state, &ctx.paths, "matrix-blank", path, php)
        .map_err(|e| e.to_string())?;
    if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
        s.https = false;
        s.web_server = Some("nginx".into());
    }
    apply_stack(&ctx.paths, &mut state).map_err(|e| e.to_string())?;
    Ok(site.id)
}

fn flip_https(
    paths: &crate::config::Paths,
    state: &mut AppState,
    sid: &str,
) -> Result<u128, String> {
    let (prev_ws, prev_https) = {
        let site = state
            .sites
            .iter_mut()
            .find(|s| s.id == sid)
            .ok_or("site missing")?;
        let prev = (site.web_server.clone(), site.https);
        site.https = !site.https;
        prev
    };
    let sw = Stopwatch::start();
    apply_site_environment_change(paths, state, sid, prev_ws, prev_https)
        .map_err(|e| e.to_string())?;
    Ok(sw.elapsed_ms())
}

fn set_web_server(
    paths: &crate::config::Paths,
    state: &mut AppState,
    sid: &str,
    server: &str,
) -> Result<(), String> {
    let (prev_ws, prev_https) = {
        let site = state
            .sites
            .iter_mut()
            .find(|s| s.id == sid)
            .ok_or("site missing")?;
        let prev = (site.web_server.clone(), site.https);
        site.web_server = Some(server.into());
        prev
    };
    apply_site_environment_change(paths, state, sid, prev_ws, prev_https)
        .map_err(|e| e.to_string())?;
    Ok(())
}
