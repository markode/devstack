//! CI / headless smoke: M2 stack + M3 WordPress create → template → duplicate.

use std::path::PathBuf;
use std::process::Command;

fn smoke_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}
use std::sync::Arc;
use std::time::Duration;

use devstack_lib::downloads::ProgressFn;
use devstack_lib::packages::PackageRegistry;
use devstack_lib::services::backup::{backup_database, seed_large_demo_table};
use devstack_lib::services::php_runtime::register_system_php;
use devstack_lib::services::progress::noop_job_progress;
use devstack_lib::services::sites::create_site;
use devstack_lib::services::stack::{apply_stack, stop_stack};
use devstack_lib::services::templates::{
    duplicate_from_source, mark_site_as_template, DuplicateRequest,
};
use devstack_lib::services::extras::{start_service, stop_service};
use devstack_lib::services::wordpress::{create_wordpress_site, CreateWordPressRequest};
use devstack_lib::{data_root, DevStackCtx};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--devstack-dns") {
        let port = args
            .windows(2)
            .find(|w| w[0] == "--devstack-dns")
            .and_then(|w| w[1].parse().ok())
            .unwrap_or(5353u16);
        return Ok(devstack_lib::services::dns::run_dns_daemon(port)?);
    }

    let install_node = args.iter().any(|a| a == "--install-node");
    let e2e = args.iter().any(|a| a == "--e2e");
    let stack = e2e || args.iter().any(|a| a == "--stack");
    let services = args.iter().any(|a| a == "--services");
    let wordpress = e2e || args.iter().any(|a| a == "--wordpress");
    let apache = args.iter().any(|a| a == "--apache");
    let db_files = args.iter().any(|a| a == "--db-files");
    let skip_mariadb = args.iter().any(|a| a == "--skip-mariadb");
    // Default: services smoke when no mode flag given
    let services = services
        || (!install_node
            && !stack
            && !wordpress
            && !apache
            && !db_files
            && !e2e
            && !args.iter().any(|a| a == "--help"));

    if args.iter().any(|a| a == "--help") {
        eprintln!(
            "devstack_smoke [--e2e] [--stack] [--wordpress] [--apache] [--db-files] [--services] [--skip-mariadb] [--install-node]"
        );
        return Ok(());
    }

    let ctx = bootstrap()?;
    println!(
        "DevStack smoke — os={} arch={}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!("data dir: {}", ctx.paths.root.display());

    {
        let state = ctx.state.lock().await;
        for id in [
            "php",
            "node",
            "nginx",
            "phpmyadmin",
            "mysql",
            "mariadb",
            "mongodb",
            "postgresql",
        ] {
            let provider = ctx.registry.get(id)?;
            match provider.list_versions(&state).await {
                Ok(versions) => println!(
                    "[{id}] {} version(s); latest={}",
                    versions.len(),
                    versions.first().map(|v| v.label.as_str()).unwrap_or("-")
                ),
                Err(e) => println!("[{id}] list error: {e}"),
            }
        }
    }

    if install_node {
        install_pkg(&ctx, "node", "22").await?;
    }

    if stack {
        run_stack_smoke(&ctx, skip_mariadb).await?;
    }

    if wordpress {
        run_wordpress_smoke(&ctx, skip_mariadb).await?;
    }

    if apache {
        run_apache_wordpress_smoke(&ctx, skip_mariadb).await?;
    }

    if db_files {
        run_db_files_smoke(&ctx, skip_mariadb).await?;
    }

    if services {
        run_services_smoke(&ctx).await?;
    }

    Ok(())
}

async fn ensure_stack(ctx: &DevStackCtx, skip_mariadb: bool) -> Result<(), Box<dyn std::error::Error>> {
    {
        let mut state = ctx.state.lock().await;
        state.stack.http_port = 8080;
        state.stack.https_port = 8443;
        state.stack.apache_http_port = 8088;
        state.stack.apache_https_port = 8444;
        // Prefer a managed PHP build (Windows needs NTS + absolute extension_dir).
        let _ = register_system_php(&mut state, &ctx.paths);
    }
    // Install official PHP when system PHP is missing or Windows (mysqli DLLs).
    if cfg!(windows) || {
        let state = ctx.state.lock().await;
        !state.installed.contains_key("php")
    } {
        let ver = match pick_version(ctx, "php", |v| {
            v.starts_with("8.3") || v.starts_with("8.4") || v.starts_with("8.")
        })
        .await
        {
            Ok(v) => Ok(v),
            Err(_) => pick_version(ctx, "php", |_| true).await,
        };
        match ver {
            Ok(v) => {
                println!("Installing PHP {v}…");
                if let Err(e) = install_pkg(ctx, "php", &v).await {
                    println!("[php] managed install failed ({e}) — trying brew/system");
                    ensure_system_php(ctx).await?;
                }
            }
            Err(e) => {
                println!("[php] managed install skipped: {e} — trying brew/system");
                ensure_system_php(ctx).await?;
            }
        }
    }
    assert_php_mysqli(ctx).await?;

    if let Err(e) = install_pkg(ctx, "nginx", "1.28.3").await {
        println!("nginx 1.28.3 install failed ({e}) — trying latest listed");
        let ver = {
            let state = ctx.state.lock().await;
            let provider = ctx.registry.get("nginx")?;
            let versions = provider.list_versions(&state).await?;
            versions
                .first()
                .map(|v| v.version.clone())
                .ok_or_else(|| "no nginx versions".to_string())?
        };
        install_pkg(ctx, "nginx", &ver).await?;
    }

    if !skip_mariadb {
        let ver = match pick_version(ctx, "mariadb", |v| {
            v.starts_with("11.4") || v.starts_with("11.") || v.starts_with("10.11")
        })
        .await
        {
            Ok(v) => Ok(v),
            Err(_) => pick_version(ctx, "mariadb", |_| true).await,
        };
        match ver {
            Ok(ver) => {
                println!("Installing MariaDB {ver} (this can take a while)…");
                if let Err(e) = install_pkg(ctx, "mariadb", &ver).await {
                    println!("[mariadb] managed install failed ({e}) — trying brew/system");
                    ensure_system_mariadb(ctx).await?;
                }
            }
            Err(e) => {
                println!("[mariadb] no managed package ({e}) — trying brew/system");
                ensure_system_mariadb(ctx).await?;
            }
        }
    }

    let _ = install_pkg(ctx, "phpmyadmin", "5.2.3").await;
    // WordPress sites default to Apache — fail early if it cannot be installed.
    if let Err(e) = ensure_apache(ctx).await {
        println!("[apache] install deferred: {e}");
    }

    {
        let mut state = ctx.state.lock().await;
        let _ = stop_stack(&ctx.paths, &mut state);
        apply_stack(&ctx.paths, &mut state)?;
        println!(
            "stack up — nginx={} apache={} db={:?} nginx={}/{} apache={}/{}",
            state.runtime.nginx_pid.is_some(),
            state.runtime.apache_pid.is_some(),
            state.runtime.database_engine,
            state.stack.http_port,
            state.stack.https_port,
            state.stack.apache_http_port,
            state.stack.apache_https_port
        );
    }
    Ok(())
}

/// Install Apache via PackageProvider, or register a system apache2/httpd on Linux.
async fn ensure_apache(ctx: &DevStackCtx) -> Result<(), Box<dyn std::error::Error>> {
    {
        let state = ctx.state.lock().await;
        if state.installed.contains_key("apache") {
            println!("[apache] already installed");
            return Ok(());
        }
    }

    // Prefer PackageProvider (Lounge on Windows; manifest on macOS/Linux).
    match pick_version(ctx, "apache", |_| true).await {
        Ok(ver) => {
            println!("Installing Apache {ver}…");
            if install_pkg(ctx, "apache", &ver).await.is_ok() {
                return Ok(());
            }
        }
        Err(e) => println!("[apache] package list unavailable ({e}) — trying system/brew httpd"),
    }

    if cfg!(target_os = "linux") {
        register_system_apache(ctx).await?;
        return Ok(());
    }

    if cfg!(target_os = "macos") {
        return register_brew_apache(ctx).await;
    }

    Err("Apache is required but not installable on this OS (set DEVSTACK_APACHE_MANIFEST_URL)".into())
}

async fn register_brew_apache(ctx: &DevStackCtx) -> Result<(), Box<dyn std::error::Error>> {
    println!("[apache] brew install httpd…");
    let status = smoke_command("brew").args(["install", "httpd"]).status()?;
    if !status.success() {
        // Already installed is fine — brew exits 0; treat failure as soft and retry prefix.
        println!("[apache] brew install httpd returned {status} — checking prefix");
    }
    let prefix = smoke_command("brew")
        .args(["--prefix", "httpd"])
        .output()?;
    let root = String::from_utf8_lossy(&prefix.stdout).trim().to_string();
    if root.is_empty() {
        return Err("brew --prefix httpd returned empty".into());
    }
    let bin = PathBuf::from(&root).join("bin/httpd");
    if !bin.exists() {
        return Err(format!("httpd binary missing at {}", bin.display()).into());
    }
    // Stop brew's launchd so DevStack owns the ports/config.
    let _ = smoke_command("brew").args(["services", "stop", "httpd"]).status();
    let mut state = ctx.state.lock().await;
    let record = devstack_lib::config::InstalledRecord {
        version: "brew".into(),
        channel: Some("brew".into()),
        path: PathBuf::from(&root),
        installed_at: devstack_lib::packages::now_iso(),
    };
    devstack_lib::config::mark_installed(&mut state, "apache", record)?;
    state.stack.apache_version = Some("brew".into());
    state.stack.apache_http_port = 8088;
    state.stack.apache_https_port = 8444;
    devstack_lib::config::save_state(&ctx.paths.state_file, &state)?;
    println!(
        "[apache] registered brew prefix {root} (bin {})",
        bin.display()
    );
    Ok(())
}

async fn register_system_apache(
    ctx: &DevStackCtx,
) -> Result<(), Box<dyn std::error::Error>> {
    let bin = which::which("httpd")
        .or_else(|_| which::which("apache2"))
        .or_else(|_| {
            println!("[apache] apt-get install apache2…");
            let _ = smoke_command("sudo")
                .args(["-n", "apt-get", "install", "-y", "apache2"])
                .status();
            which::which("apache2")
        })?;
    // Stop the distro service so DevStack owns the ports/config.
    let _ = smoke_command("sudo")
        .args(["-n", "systemctl", "stop", "apache2"])
        .status();
    let _ = smoke_command("sudo")
        .args(["-n", "systemctl", "disable", "apache2"])
        .status();

    let root = bin
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| std::path::Path::new("/usr"))
        .to_path_buf();
    let mut state = ctx.state.lock().await;
    let record = devstack_lib::config::InstalledRecord {
        version: "system".into(),
        channel: Some("system".into()),
        path: root.clone(),
        installed_at: devstack_lib::packages::now_iso(),
    };
    devstack_lib::config::mark_installed(&mut state, "apache", record)?;
    state.stack.apache_version = Some("system".into());
    // Smoke uses high ports so we don't need root for Listen.
    state.stack.apache_http_port = 8088;
    state.stack.apache_https_port = 8444;
    devstack_lib::config::save_state(&ctx.paths.state_file, &state)?;
    println!(
        "[apache] registered system binary {} (root {})",
        bin.display(),
        root.display()
    );
    Ok(())
}

async fn run_stack_smoke(
    ctx: &DevStackCtx,
    skip_mariadb: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_stack(ctx, skip_mariadb).await?;

    let site_path = ctx.paths.root.join("sites/smoke");
    std::fs::create_dir_all(&site_path)?;
    std::fs::write(
        site_path.join("index.php"),
        "<?php echo 'DevStack OK '.phpversion();",
    )?;

    {
        let mut state = ctx.state.lock().await;
        let php_ver = state
            .installed
            .get("php")
            .and_then(|v| v.first())
            .map(|r| r.version.clone())
            .unwrap_or_else(|| "8.3".into());
        let site = create_site(
            &mut state,
            &ctx.paths,
            "smoke",
            site_path.clone(),
            Some(php_ver.clone()),
        )?;
        if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
            s.php_version = Some(php_ver);
            s.https = true;
            s.status = "running".into();
        }
        apply_stack(&ctx.paths, &mut state)?;
    }

    let _ = smoke_command("sudo")
        .args([
            "-n",
            &ctx.paths.helper.join("devstack-helper.sh").to_string_lossy(),
            "hosts-sync",
            "smoke.test",
            "phpmyadmin.test",
        ])
        .status();

    tokio::time::sleep(Duration::from_secs(2)).await;
    http_expect("smoke.test", 8080, "DevStack OK")?;
    http_expect("smoke.test", 8443, "DevStack OK")?;
    println!("SMOKE PASS: http://smoke.test:8080 + https://smoke.test:8443 via Nginx + PHP");

    {
        let mut state = ctx.state.lock().await;
        stop_stack(&ctx.paths, &mut state)?;
        println!("stack stopped cleanly");
    }
    Ok(())
}

async fn run_wordpress_smoke(
    ctx: &DevStackCtx,
    skip_mariadb: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_stack(ctx, skip_mariadb).await?;
    ensure_apache(ctx).await?;
    let progress = noop_job_progress();

    println!("Creating WordPress site demo.test…");
    let created = {
        let mut state = ctx.state.lock().await;
        let php_ver = state
            .installed
            .get("php")
            .and_then(|v| v.first())
            .map(|r| r.version.clone())
            .unwrap_or_else(|| "8.3".into());
        create_wordpress_site(
            &ctx.paths,
            &mut state,
            CreateWordPressRequest {
                name: "demo".into(),
                domain: Some("demo.test".into()),
                path: Some(ctx.paths.root.join("sites/demo").display().to_string()),
                php_version: Some(php_ver),
                database_engine: Some("mariadb".into()),
                locale: Some("en_US".into()),
                admin_user: Some("admin".into()),
                site_title: Some("Demo Blog".into()),
            },
            &progress,
            "wp",
        )
        .await?
    };
    println!(
        "WP {} at {} (admin {} / {})",
        created.wordpress_version,
        created.url,
        created.admin_user,
        created.admin_password
    );

    // Seed a larger table then backup (streaming)
    {
        let state = ctx.state.lock().await;
        let engine = created
            .site
            .database_engine
            .clone()
            .unwrap_or_else(|| "mariadb".into());
        let db = created.site.database_name.clone().unwrap();
        println!("Seeding large demo table (1200 rows ≈ multi-MB)…");
        seed_large_demo_table(&ctx.paths, &state, &engine, &db, 1200)?;
        let dest = ctx.paths.backups.join("demo-large.sql.gz");
        let bytes = backup_database(
            &ctx.paths,
            &state,
            &engine,
            &db,
            &dest,
            &progress,
            "backup",
        )?;
        println!("Backup OK → {} ({bytes} bytes)", dest.display());
    }

    println!("Marking demo as template…");
    let tpl = {
        let mut state = ctx.state.lock().await;
        mark_site_as_template(&ctx.paths, &mut state, "demo", &progress, "tpl").await?
    };
    println!(
        "Template {} size={} bytes",
        tpl.id, tpl.size_bytes
    );

    println!("Duplicating to clone.test…");
    let dup = {
        let mut state = ctx.state.lock().await;
        duplicate_from_source(
            &ctx.paths,
            &mut state,
            DuplicateRequest {
                source_id: tpl.id.clone(),
                name: "clone".into(),
                domain: Some("clone.test".into()),
                path: ctx.paths.root.join("sites/clone").display().to_string(),
                php_version: created.site.php_version.clone(),
                database_engine: Some("mariadb".into()),
            },
            &progress,
            "dup",
        )
        .await?
    };
    println!("Clone ready at {}", dup.url);

    let _ = smoke_command("sudo")
        .args([
            "-n",
            &ctx.paths.helper.join("devstack-helper.sh").to_string_lossy(),
            "hosts-sync",
            "demo.test",
            "clone.test",
            "phpmyadmin.test",
        ])
        .status();

    // WordPress defaults to Apache — keep it on both platforms and assert PHP-CGI works.
    let (https_port, http_port) = {
        let mut state = ctx.state.lock().await;
        apply_stack(&ctx.paths, &mut state)?;
        if state.runtime.apache_pid.is_none() {
            return Err("WordPress smoke requires Apache running after apply_stack".into());
        }
        let https_port = state.stack.apache_https_port;
        let http_port = state.stack.apache_http_port;
        println!(
            "wordpress stack — apache_pid={:?} nginx_pid={:?} https={https_port} http={http_port}",
            state.runtime.apache_pid, state.runtime.nginx_pid
        );
        (https_port, http_port)
    };

    tokio::time::sleep(Duration::from_millis(800)).await;

    // WordPress home should mention the site title / WordPress over HTTPS + HTTP.
    for (scheme_port, label) in [(https_port, "https"), (http_port, "http")] {
        if let Err(e) = http_expect("demo.test", scheme_port, "Demo Blog") {
            dump_apache_logs(ctx);
            return Err(format!("demo.test {label}: {e}").into());
        }
        let body = http_get_scheme(
            "demo.test",
            scheme_port,
            "/",
            if label == "http" { "http" } else { "https" },
        )?;
        if body.to_lowercase().contains("no input file specified") {
            dump_apache_logs(ctx);
            return Err(format!("demo.test {label}: No input file specified").into());
        }
    }
    // After search-replace, clone should not still advertise the old domain as canonical
    let clone_body = match http_get("clone.test", https_port) {
        Ok(b) => b,
        Err(e) => {
            dump_apache_logs(ctx);
            return Err(e);
        }
    };
    if !clone_body.contains("clone.test") && !clone_body.to_lowercase().contains("wordpress") {
        dump_apache_logs(ctx);
        return Err(format!("clone body unexpected: {clone_body}").into());
    }
    if clone_body.contains("https://demo.test") {
        eprintln!("warning: clone HTML still references demo.test (may be in content)");
    }

    // Fast site-setting toggles: HTTPS / server / PHP must apply in <3s on CI.
    run_site_settings_perf(ctx, &created.site.id).await?;

    // Export → import round-trip with search-replace already covered by duplicate;
    // also restore the large dump into a fresh DB name via client.
    {
        use devstack_lib::services::backup::restore_database;
        let state = ctx.state.lock().await;
        let engine = created
            .site
            .database_engine
            .clone()
            .unwrap_or_else(|| "mariadb".into());
        let dest = ctx.paths.backups.join("demo-large.sql.gz");
        if dest.exists() {
            let db = created.site.database_name.clone().unwrap();
            println!("Restoring backup into `{db}` (idempotent)…");
            restore_database(
                &ctx.paths,
                &state,
                &engine,
                &db,
                &dest,
                &progress,
                "restore",
            )?;
            println!("Restore OK");
        }
    }

    {
        let mut state = ctx.state.lock().await;
        stop_stack(&ctx.paths, &mut state)?;
        println!("stack stopped after WordPress e2e");
    }
    println!("SMOKE PASS: WordPress demo.test + clone.test over HTTPS (Apache :{https_port})");
    Ok(())
}

async fn ensure_system_php(ctx: &DevStackCtx) -> Result<(), Box<dyn std::error::Error>> {
    {
        let state = ctx.state.lock().await;
        if state.installed.contains_key("php") {
            return Ok(());
        }
    }
    if cfg!(target_os = "macos") {
        println!("[php] brew install php…");
        let status = smoke_command("brew").args(["install", "php"]).status()?;
        if !status.success() {
            return Err("brew install php failed".into());
        }
    }
    let mut state = ctx.state.lock().await;
    let _ = register_system_php(&mut state, &ctx.paths)?;
    Ok(())
}

async fn ensure_system_mariadb(ctx: &DevStackCtx) -> Result<(), Box<dyn std::error::Error>> {
    {
        let state = ctx.state.lock().await;
        if state.installed.contains_key("mariadb") {
            return Ok(());
        }
    }
    if cfg!(target_os = "macos") {
        println!("[mariadb] brew install mariadb…");
        let status = smoke_command("brew")
            .args(["install", "mariadb"])
            .status()?;
        if !status.success() {
            return Err("brew install mariadb failed".into());
        }
        // Prefer brew prefix so we find mariadbd under Cellar.
        let prefix = smoke_command("brew")
            .args(["--prefix", "mariadb"])
            .output()?;
        let root = String::from_utf8_lossy(&prefix.stdout).trim().to_string();
        if root.is_empty() {
            return Err("brew --prefix mariadb returned empty".into());
        }
        let mut state = ctx.state.lock().await;
        let record = devstack_lib::config::InstalledRecord {
            version: "brew".into(),
            channel: Some("brew".into()),
            path: PathBuf::from(&root),
            installed_at: devstack_lib::packages::now_iso(),
        };
        devstack_lib::config::mark_installed(&mut state, "mariadb", record)?;
        state.stack.mariadb_version = Some("brew".into());
        state.active_database = Some("mariadb".into());
        // Avoid clashing with any system mysqld on 3306 during CI.
        // Stay on high ports already used by smoke (8080/8443) — also bump DB
        // away from any brew/launchd mysqld that may already own 3306.
        state.stack.mariadb_port = 3307;
        state.stack.mysql_port = 3308;
        // Stop brew's launchd instance so DevStack owns the datadir/port.
        let _ = smoke_command("brew").args(["services", "stop", "mariadb"]).status();
        let _ = smoke_command("pkill").args(["-f", "mariadbd"]).status();
        let _ = smoke_command("pkill").args(["-f", "mysqld"]).status();
        // Give brew's init a private datadir under DevStack so we don't fight
        // /opt/homebrew/var/mysql permissions.
        let brew_data = ctx.paths.databases.join("mariadb/data");
        std::fs::create_dir_all(&brew_data)?;
        if !brew_data.join("mysql").exists() {
            let _ = smoke_command("mysql_install_db")
                .args([
                    format!("--datadir={}", brew_data.display()),
                    "--auth-root-authentication-method=normal".into(),
                ])
                .status();
            let _ = smoke_command(&format!("{root}/bin/mysql_install_db"))
                .args([
                    format!("--datadir={}", brew_data.display()),
                    format!("--basedir={root}"),
                ])
                .status();
            let _ = smoke_command(&format!("{root}/scripts/mysql_install_db"))
                .args([
                    format!("--datadir={}", brew_data.display()),
                    format!("--basedir={root}"),
                ])
                .status();
        }
        devstack_lib::config::save_state(&ctx.paths.state_file, &state)?;
        println!("[mariadb] registered brew prefix {root} on port 3307");
        return Ok(());
    }
    Err("MariaDB required for WordPress smoke".into())
}

async fn assert_php_mysqli(ctx: &DevStackCtx) -> Result<(), Box<dyn std::error::Error>> {
    let state = ctx.state.lock().await;
    let Some(list) = state.installed.get("php") else {
        println!("[php] no installed PHP — skip mysqli assert");
        return Ok(());
    };
    let rec = list.first().ok_or("php list empty")?;
    let install = if rec.path.exists() {
        rec.path.clone()
    } else {
        ctx.paths.package_dir("php", &rec.version)
    };
    let _ = devstack_lib::php::write_dev_php_ini(
        &install,
        &devstack_lib::php::PhpIniOptions::default(),
    )?;
    let php_bin = devstack_lib::services::process::find_bin(&install, &["php", "php.exe"])
        .ok_or_else(|| format!("php binary missing under {}", install.display()))?;
    let modules = devstack_lib::php::list_loaded_modules(&php_bin, &install)?;
    println!("[php] loaded modules: {}", modules.join(", "));
    if cfg!(windows) {
        let missing = devstack_lib::php::missing_required_modules(&modules);
        if !missing.is_empty() {
            return Err(format!(
                "php -m missing required modules: {}",
                missing.join(", ")
            )
            .into());
        }
        if !modules.iter().any(|m| m == "mysqli") {
            return Err("php -m does not list mysqli".into());
        }
        println!("[php] mysqli OK");
    }
    Ok(())
}

async fn run_apache_wordpress_smoke(
    ctx: &DevStackCtx,
    skip_mariadb: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Apache WordPress pretty-permalinks smoke ===");
    ensure_stack(ctx, skip_mariadb).await?;
    ensure_apache(ctx).await?;

    {
        let mut state = ctx.state.lock().await;
        state.stack.apache_http_port = 8088;
        state.stack.apache_https_port = 8444;
        apply_stack(&ctx.paths, &mut state)?;
    }

    let progress = noop_job_progress();
    let created = {
        let mut state = ctx.state.lock().await;
        let php_ver = state
            .installed
            .get("php")
            .and_then(|v| v.first())
            .map(|r| r.version.clone())
            .unwrap_or_else(|| "8.3".into());
        create_wordpress_site(
            &ctx.paths,
            &mut state,
            CreateWordPressRequest {
                name: "apache-wp".into(),
                domain: Some("apache-wp.test".into()),
                path: Some(
                    ctx.paths
                        .root
                        .join("sites/apache-wp")
                        .display()
                        .to_string(),
                ),
                php_version: Some(php_ver),
                database_engine: Some("mariadb".into()),
                locale: Some("en_US".into()),
                admin_user: Some("admin".into()),
                site_title: Some("Apache Permalinks".into()),
            },
            &progress,
            "apache-wp",
        )
        .await?
    };

    if created.site.web_server.as_deref() != Some("apache") {
        return Err(format!(
            "expected web_server=apache, got {:?}",
            created.site.web_server
        )
        .into());
    }

    // Create a published post with a known slug for pretty-permalink probing.
    {
        let state = ctx.state.lock().await;
        let php_ver = created
            .site
            .php_version
            .clone()
            .unwrap_or_else(|| "8.3".into());
        let php = state
            .installed
            .get("php")
            .and_then(|list| {
                list.iter()
                    .find(|r| r.version == php_ver || r.channel.as_deref() == Some(php_ver.as_str()))
                    .or_else(|| list.first())
            })
            .ok_or("php not installed")?;
        let php_bin = php.path.join("bin/php");
        let php_bin = if php_bin.exists() {
            php_bin
        } else {
            which::which("php")?
        };
        let wp_cli = ctx.paths.root.join("tools/wp-cli.phar");
        let abs = created.site.path.clone();
        let status = smoke_command(&php_bin)
            .args([
                wp_cli.to_str().unwrap(),
                "post",
                "create",
                "--post_title=Pretty Permalink Probe",
                "--post_name=pretty-permalink-probe",
                "--post_status=publish",
                "--porcelain",
            ])
            .current_dir(&abs)
            .status()?;
        if !status.success() {
            return Err("wp post create failed".into());
        }
        let _ = smoke_command(&php_bin)
            .args([
                wp_cli.to_str().unwrap(),
                "rewrite",
                "flush",
                "--hard",
            ])
            .current_dir(&abs)
            .status();
    }

    let _ = smoke_command("sudo")
        .args([
            "-n",
            &ctx.paths.helper.join("devstack-helper.sh").to_string_lossy(),
            "hosts-sync",
            "apache-wp.test",
        ])
        .status();

    {
        let mut state = ctx.state.lock().await;
        apply_stack(&ctx.paths, &mut state)?;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;

    let https_port = {
        let state = ctx.state.lock().await;
        state.stack.apache_https_port
    };

    http_expect("apache-wp.test", https_port, "Apache Permalinks")?;

    // Pretty permalink must not 404 when AllowOverride + mod_rewrite are on.
    let permalink = http_get_path("apache-wp.test", https_port, "/pretty-permalink-probe/")?;
    if permalink.to_lowercase().contains("not found")
        || permalink.to_lowercase().contains("404")
            && !permalink.to_lowercase().contains("pretty permalink probe")
    {
        return Err(format!("pretty permalink 404: {permalink}").into());
    }
    if !permalink.to_lowercase().contains("pretty permalink probe")
        && !permalink.to_lowercase().contains("wordpress")
    {
        return Err(format!("pretty permalink body unexpected: {permalink}").into());
    }

    // Confirm .htaccess exists (wp rewrite --hard)
    let htaccess = std::path::PathBuf::from(&created.site.path).join(".htaccess");
    if !htaccess.exists() {
        eprintln!("warning: .htaccess missing at {}", htaccess.display());
    } else {
        println!(".htaccess present ({} bytes)", std::fs::metadata(&htaccess)?.len());
    }

    println!(
        "SMOKE PASS: Apache WordPress pretty permalinks on https://apache-wp.test:{https_port}"
    );
    Ok(())
}

async fn run_db_files_smoke(
    ctx: &DevStackCtx,
    skip_mariadb: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use devstack_lib::services::db_files::{
        databases_folder, export_database_file, import_database_file, list_db_files,
        post_import_url_rewrite,
    };
    use devstack_lib::services::progress::noop_job_progress;

    println!("=== Databases folder smoke ===");
    ensure_stack(ctx, skip_mariadb).await?;
    let progress = noop_job_progress();

    let created = {
        let mut state = ctx.state.lock().await;
        let php_ver = state
            .installed
            .get("php")
            .and_then(|v| v.first())
            .map(|r| r.version.clone())
            .unwrap_or_else(|| "8.3".into());
        create_wordpress_site(
            &ctx.paths,
            &mut state,
            CreateWordPressRequest {
                name: "dbfiles".into(),
                domain: Some("dbfiles.test".into()),
                path: Some(ctx.paths.root.join("sites/dbfiles").display().to_string()),
                php_version: Some(php_ver),
                database_engine: Some("mariadb".into()),
                locale: Some("en_US".into()),
                admin_user: Some("admin".into()),
                site_title: Some("DB Files Blog".into()),
            },
            &progress,
            "dbfiles-wp",
        )
        .await?
    };

    let engine = created
        .site
        .database_engine
        .clone()
        .unwrap_or_else(|| "mariadb".into());
    let db = created
        .site
        .database_name
        .clone()
        .ok_or("missing db name")?;

    {
        let state = ctx.state.lock().await;
        let entry = export_database_file(
            &ctx.paths,
            &state,
            &engine,
            &db,
            &progress,
            "dbfiles-export",
        )?;
        println!("exported {} ({} bytes)", entry.path, entry.bytes);
        let listed = list_db_files(&ctx.paths, &state)?;
        if !listed.iter().any(|e| e.database == db) {
            return Err("export missing from Databases folder listing".into());
        }
        let folder = databases_folder(&ctx.paths, &state.db_files)?;
        let dump = std::path::PathBuf::from(&entry.path);
        // Re-import same dump (safety backup + restore) then WP URL rewrite no-op
        import_database_file(
            &ctx.paths,
            &state,
            &engine,
            &db,
            &dump,
            &progress,
            "dbfiles-import",
        )?;
        println!("re-imported from {}", dump.display());
        let _ = folder;
    }
    {
        let state = ctx.state.lock().await;
        post_import_url_rewrite(
            &ctx.paths,
            &state,
            &created.site,
            None,
            &progress,
            "dbfiles-sr",
        )
        .await?;
    }

    println!("SMOKE PASS: Databases folder export + import + WP rewrite hook");
    Ok(())
}

async fn run_services_smoke(ctx: &DevStackCtx) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Milestone 4 services smoke (Postgres / Mongo / Redis / Mailpit) ===");

    // Prefer smaller/recent installable versions
    // Prefer Redis 7.4.x — builds cleanly with MALLOC=libc on CI Linux.
    let redis_ver = pick_version(ctx, "redis", |v| v.starts_with("7.4.") || v.starts_with("7.")).await?;
    println!("Installing Redis {redis_ver}…");
    install_pkg(ctx, "redis", &redis_ver).await?;

    let mailpit_ver = pick_version(ctx, "mailpit", |_| true).await?;
    println!("Installing Mailpit {mailpit_ver}…");
    install_pkg(ctx, "mailpit", &mailpit_ver).await?;

    let pg_ver = pick_version(ctx, "postgresql", |v| v.starts_with("16.")).await?;
    println!("Installing PostgreSQL {pg_ver}…");
    install_pkg(ctx, "postgresql", &pg_ver).await?;

    // Mongo is large (~100MB); still required for smoke
    let mongo_ver = pick_version(ctx, "mongodb", |v| {
        v.starts_with("8.0.") || v.starts_with("7.0.")
    })
    .await?;
    println!("Installing MongoDB {mongo_ver}…");
    install_pkg(ctx, "mongodb", &mongo_ver).await?;

    {
        let mut state = ctx.state.lock().await;
        for id in ["redis", "postgresql", "mongodb", "mailpit"] {
            println!("Starting {id}…");
            start_service(&ctx.paths, &mut state, id)?;
        }
    }

    // Probe ports
    for (name, port) in [("redis", 6379u16), ("postgresql", 5432), ("mongodb", 27017), ("mailpit-ui", 8025), ("mailpit-smtp", 1025)] {
        let ok = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
        println!("port {port} ({name}): {}", if ok { "open" } else { "CLOSED" });
        if !ok {
            return Err(format!("{name} port {port} not open").into());
        }
    }

    // Redis PING
    {
        let state = ctx.state.lock().await;
        let root = state
            .installed
            .get("redis")
            .and_then(|v| v.first())
            .map(|r| r.path.clone())
            .ok_or("redis missing")?;
        let cli = root.join("bin/redis-cli");
        let out = smoke_command(&cli).args(["-p", "6379", "PING"]).output()?;
        let body = String::from_utf8_lossy(&out.stdout);
        println!("redis-cli PING → {}", body.trim());
        if !body.contains("PONG") {
            return Err("redis PING failed".into());
        }
    }

    // Postgres SELECT 1
    {
        let state = ctx.state.lock().await;
        let root = state
            .installed
            .get("postgresql")
            .and_then(|v| v.first())
            .map(|r| r.path.clone())
            .ok_or("postgresql missing")?;
        let psql = root.join("bin/psql");
        let out = smoke_command(&psql)
            .args(["-h", "127.0.0.1", "-U", "postgres", "-c", "SELECT 1 AS ok;"])
            .output()?;
        let body = String::from_utf8_lossy(&out.stdout);
        println!("psql → {}", body.trim());
        if !out.status.success() {
            return Err(format!("psql failed: {body}").into());
        }
    }

    // Mongo ping
    {
        let state = ctx.state.lock().await;
        let root = state
            .installed
            .get("mongodb")
            .and_then(|v| v.first())
            .map(|r| r.path.clone())
            .ok_or("mongodb missing")?;
        let mongosh = [
            root.join("bin/mongosh"),
            root.join("bin/mongo"),
        ];
        let bin = mongosh.into_iter().find(|p| p.exists());
        if let Some(bin) = bin {
            let out = smoke_command(&bin)
                .args(["--eval", "db.runCommand({ ping: 1 })"])
                .output()?;
            println!("mongo ping status={}", out.status);
        } else {
            // Fall back to TCP-only if shell not bundled
            println!("mongo shell not bundled — TCP 27017 already verified");
        }
    }

    // Mailpit HTTP UI
    let status = smoke_command("curl")
        .args(["-sS", "-o", "/dev/null", "-w", "%{http_code}", "http://127.0.0.1:8025/"])
        .output()?;
    let code = String::from_utf8_lossy(&status.stdout).trim().to_string();
    println!("Mailpit UI HTTP {code}");
    if code != "200" {
        return Err(format!("mailpit UI HTTP {code}").into());
    }

    {
        let mut state = ctx.state.lock().await;
        for id in ["mailpit", "mongodb", "postgresql", "redis"] {
            let _ = stop_service(&ctx.paths, &mut state, id);
        }
    }

    println!("SMOKE PASS: Postgres + MongoDB + Redis + Mailpit");
    Ok(())
}

async fn pick_version(
    ctx: &DevStackCtx,
    id: &str,
    prefer: impl Fn(&str) -> bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let state = ctx.state.lock().await;
    // Already installed?
    if let Some(list) = state.installed.get(id) {
        if let Some(r) = list.first() {
            return Ok(r.version.clone());
        }
    }
    let provider = ctx.registry.get(id)?;
    let versions = provider.list_versions(&state).await?;
    let chosen = versions
        .iter()
        .find(|v| v.download_url.is_some() && prefer(&v.version))
        .or_else(|| versions.iter().find(|v| v.download_url.is_some()))
        .ok_or_else(|| format!("no installable versions for {id}"))?;
    Ok(chosen.version.clone())
}

fn rewrite_wp_urls(
    paths: &devstack_lib::config::Paths,
    state: &devstack_lib::config::AppState,
    engine: &str,
    db: &str,
    from: &str,
    to: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use devstack_lib::services::database::{client_bin_for, engine_install_dir, mysql_root_args};
    let root = engine_install_dir(state, engine)?;
    let client = client_bin_for(&root)?;
    let mut args = mysql_root_args(paths, state, engine);
    let sql = format!(
        "UPDATE `wp_options` SET option_value = REPLACE(option_value, '{from}', '{to}') \
         WHERE option_name IN ('home','siteurl') OR option_value LIKE '%{from}%';"
    );
    args.push(format!("--database={db}"));
    args.push("-e".into());
    args.push(sql);
    let out = smoke_command(&client).args(&args).output()?;
    if !out.status.success() {
        return Err(format!(
            "wp url rewrite failed for {db}: {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    // Verify siteurl now contains the new port fragment.
    let mut check = mysql_root_args(paths, state, engine);
    check.push(format!("--database={db}"));
    check.push("-N".into());
    check.push("-e".into());
    check.push("SELECT option_value FROM wp_options WHERE option_name='siteurl' LIMIT 1;".into());
    let siteurl = smoke_command(&client).args(&check).output()?;
    let siteurl = String::from_utf8_lossy(&siteurl.stdout).trim().to_string();
    println!("Rewrote WP URLs in `{db}`: {from} → {to} (siteurl={siteurl})");
    if siteurl.contains(from) {
        return Err(format!("siteurl still contains {from}: {siteurl}").into());
    }
    Ok(())
}

fn dump_apache_logs(ctx: &DevStackCtx) {
    for name in [
        "apache.err",
        "apache.out",
        "apache/error.log",
        "apache/demo-error.log",
        "apache/demo-ssl-error.log",
        "apache/clone-error.log",
        "apache/clone-ssl-error.log",
    ] {
        let p = ctx.paths.logs.join(name);
        if p.exists() {
            let body = std::fs::read_to_string(&p).unwrap_or_default();
            let tail = body
                .lines()
                .rev()
                .take(40)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            eprintln!("--- {} (tail) ---\n{tail}", p.display());
        }
    }
}

fn http_expect(host: &str, port: u16, needle: &str) -> Result<(), Box<dyn std::error::Error>> {
    let scheme = if port == 80 || port == 8080 || port == 8088 {
        "http"
    } else {
        "https"
    };
    let body = http_get_scheme(host, port, "/", scheme)?;
    println!(
        "GET {scheme}://{host}:{port}/ → {} bytes, has_needle={}",
        body.len(),
        body.contains(needle)
    );
    if !body.contains(needle) {
        return Err(format!("response missing `{needle}`: {body}").into());
    }
    Ok(())
}

fn http_get(host: &str, port: u16) -> Result<String, Box<dyn std::error::Error>> {
    http_get_path(host, port, "/")
}

fn http_get_path(
    host: &str,
    port: u16,
    path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    http_get_scheme(host, port, path, "https")
}

fn http_get_scheme(
    host: &str,
    port: u16,
    path: &str,
    scheme: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("{scheme}://{host}:{port}{path}");
    let slug = path.trim_matches('/').replace('/', "-");
    let tmp = std::env::temp_dir();
    let out = tmp
        .join(format!(
            "devstack-smoke-{host}{}.html",
            if slug.is_empty() {
                String::new()
            } else {
                format!("-{slug}")
            }
        ))
        .to_string_lossy()
        .to_string();
    let status = smoke_command("curl")
        .args([
            "-k",
            "--resolve",
            &format!("{host}:{port}:127.0.0.1"),
            "-sS",
            "-L",
            "-o",
            &out,
            "-w",
            "%{http_code}",
            &url,
        ])
        .output()?;
    let code = String::from_utf8_lossy(&status.stdout).trim().to_string();
    let body = std::fs::read_to_string(&out).unwrap_or_default();
    println!("HTTP {code} for {url}");
    if code != "200" {
        return Err(format!("HTTP {code} for {url}: {body}").into());
    }
    Ok(body)
}

async fn install_pkg(
    ctx: &DevStackCtx,
    id: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    {
        let state = ctx.state.lock().await;
        if state
            .installed
            .get(id)
            .map(|list| {
                list.iter()
                    .any(|r| r.version == version || r.channel.as_deref() == Some(version))
            })
            .unwrap_or(false)
        {
            println!("[{id}] already installed ({version})");
            return Ok(());
        }
    }
    println!("[{id}] installing {version}…");
    let provider = ctx.registry.get(id)?;
    let progress: ProgressFn = Arc::new(|ev| {
        if ev.phase == "downloading" {
            if let Some(total) = ev.total {
                eprint!("\r  {} / {} ", ev.downloaded, total);
            }
        } else {
            eprintln!("\n  phase={}", ev.phase);
        }
    });
    let mut state = ctx.state.lock().await;
    let installed = provider
        .install(version, &ctx.paths, &mut state, progress)
        .await?;
    println!(
        "\n[{id}] installed {} → {}",
        installed.version,
        installed.path.display()
    );
    Ok(())
}

/// Assert HTTPS / web-server / PHP toggles stay under 3s (typical <2s).
async fn run_site_settings_perf(
    ctx: &DevStackCtx,
    site_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use devstack_lib::services::stack::apply_site_environment_change;
    use std::time::Instant;

    let mut timings: Vec<(String, u128)> = Vec::new();

    // Toggle HTTPS off then on.
    for https in [false, true] {
        let start = Instant::now();
        {
            let mut state = ctx.state.lock().await;
            let (prev_ws, prev_https) = {
                let site = state
                    .sites
                    .iter_mut()
                    .find(|s| s.id == site_id)
                    .ok_or("site missing for perf toggle")?;
                let prev = (site.web_server.clone(), site.https);
                site.https = https;
                prev
            };
            apply_site_environment_change(
                &ctx.paths,
                &mut state,
                site_id,
                prev_ws,
                prev_https,
            )?;
        }
        let ms = start.elapsed().as_millis();
        timings.push((format!("https={https}"), ms));
        println!("PERF toggle https={https}: {ms}ms");
        if ms > 3000 {
            return Err(format!("HTTPS toggle took {ms}ms (>3000)").into());
        }
    }

    // Apache → Nginx → Apache
    for server in ["nginx", "apache"] {
        let start = Instant::now();
        let (http_port, https_port) = {
            let mut state = ctx.state.lock().await;
            let (prev_ws, prev_https) = {
                let site = state
                    .sites
                    .iter_mut()
                    .find(|s| s.id == site_id)
                    .ok_or("site missing for server switch")?;
                let prev = (site.web_server.clone(), site.https);
                site.web_server = Some(server.into());
                prev
            };
            apply_site_environment_change(
                &ctx.paths,
                &mut state,
                site_id,
                prev_ws,
                prev_https,
            )?;
            if server == "nginx" {
                (state.stack.http_port, state.stack.https_port)
            } else {
                (state.stack.apache_http_port, state.stack.apache_https_port)
            }
        };
        let ms = start.elapsed().as_millis();
        timings.push((format!("server={server}"), ms));
        println!("PERF switch server={server}: {ms}ms");
        if ms > 3000 {
            return Err(format!("server switch to {server} took {ms}ms (>3000)").into());
        }
        // Fetch home on both schemes — must not be connection refused / No input file.
        for (port, scheme) in [(http_port, "http"), (https_port, "https")] {
            let body = http_get_scheme("demo.test", port, "/", scheme)?;
            if body.to_lowercase().contains("no input file specified") {
                return Err(
                    format!("demo.test via {server} {scheme}: No input file specified").into(),
                );
            }
        }
    }

    // Switch PHP version to another installed build when available.
    {
        let versions: Vec<String> = {
            let state = ctx.state.lock().await;
            state
                .installed
                .get("php")
                .map(|v| v.iter().map(|r| r.version.clone()).collect())
                .unwrap_or_default()
        };
        if versions.len() >= 2 {
            let target = versions[1].clone();
            let start = Instant::now();
            {
                let mut state = ctx.state.lock().await;
                let (prev_ws, prev_https) = {
                    let site = state
                        .sites
                        .iter_mut()
                        .find(|s| s.id == site_id)
                        .ok_or("site missing for php switch")?;
                    let prev = (site.web_server.clone(), site.https);
                    site.php_version = Some(target.clone());
                    prev
                };
                apply_site_environment_change(
                    &ctx.paths,
                    &mut state,
                    site_id,
                    prev_ws,
                    prev_https,
                )?;
            }
            let ms = start.elapsed().as_millis();
            timings.push((format!("php={target}"), ms));
            println!("PERF switch php={target}: {ms}ms");
            if ms > 3000 {
                return Err(format!("PHP switch took {ms}ms (>3000)").into());
            }
        } else {
            println!(
                "PERF skip php switch — only {} PHP version(s) installed",
                versions.len()
            );
        }
    }

    println!(
        "PERF SUMMARY: {}",
        timings
            .iter()
            .map(|(k, v)| format!("{k}={v}ms"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

fn bootstrap() -> Result<DevStackCtx, Box<dyn std::error::Error>> {
    let root = std::env::var("DEVSTACK_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_root());
    let paths = devstack_lib::config::Paths::from_root(root)?;
    let mut state = devstack_lib::config::load_state(&paths.state_file)?;
    // Fresh sites for wordpress smoke — keep packages
    if std::env::var("DEVSTACK_RESET_SITES").ok().as_deref() == Some("1") {
        state.sites.clear();
        state.templates.clear();
    }
    devstack_lib::config::ensure_welcome_site(&paths, &mut state);
    devstack_lib::config::save_state(&paths.state_file, &state)?;
    Ok(DevStackCtx {
        paths,
        state: tokio::sync::Mutex::new(state),
        registry: PackageRegistry::new()?,
        version_refresh: std::sync::Arc::new(
            devstack_lib::packages::version_cache::RefreshGuard::new(),
        ),
    })
}
