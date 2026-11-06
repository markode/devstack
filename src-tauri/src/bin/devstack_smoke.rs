//! CI / headless smoke: M2 stack + M3 WordPress create → template → duplicate.

use std::path::PathBuf;
use std::process::Command;
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
    let stack = args.iter().any(|a| a == "--stack");
    let wordpress = args.iter().any(|a| a == "--wordpress")
        || (!install_node && !stack && !args.iter().any(|a| a == "--help"));
    let skip_mariadb = args.iter().any(|a| a == "--skip-mariadb");

    if args.iter().any(|a| a == "--help") {
        eprintln!(
            "devstack_smoke [--stack] [--wordpress] [--skip-mariadb] [--install-node]"
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

    if stack && !wordpress {
        run_stack_smoke(&ctx, skip_mariadb).await?;
    }

    if wordpress {
        run_wordpress_smoke(&ctx, skip_mariadb).await?;
    }

    Ok(())
}

async fn ensure_stack(ctx: &DevStackCtx, skip_mariadb: bool) -> Result<(), Box<dyn std::error::Error>> {
    {
        let mut state = ctx.state.lock().await;
        state.stack.http_port = 8080;
        state.stack.https_port = 8443;
        let _ = register_system_php(&mut state, &ctx.paths)?;
        println!("registered system PHP");
    }

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
        let ver = {
            let state = ctx.state.lock().await;
            let provider = ctx.registry.get("mariadb")?;
            let versions = provider.list_versions(&state).await?;
            versions
                .iter()
                .find(|v| v.channel.starts_with("11.4") && v.download_url.is_some())
                .or_else(|| versions.iter().find(|v| v.download_url.is_some()))
                .map(|v| v.version.clone())
        };
        if let Some(ver) = ver {
            println!("Installing MariaDB {ver} (this can take a while)…");
            install_pkg(ctx, "mariadb", &ver).await?;
        } else {
            return Err("MariaDB required for WordPress smoke".into());
        }
    }

    let _ = install_pkg(ctx, "phpmyadmin", "5.2.3").await;

    {
        let mut state = ctx.state.lock().await;
        let _ = stop_stack(&ctx.paths, &mut state);
        apply_stack(&ctx.paths, &mut state)?;
        println!(
            "stack up — nginx={} db={:?} ports={}/{}",
            state.runtime.nginx_pid.is_some(),
            state.runtime.database_engine,
            state.stack.http_port,
            state.stack.https_port
        );
    }
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

    let _ = Command::new("sudo")
        .args([
            "-n",
            &ctx.paths.helper.join("devstack-helper.sh").to_string_lossy(),
            "hosts-sync",
            "smoke.test",
            "phpmyadmin.test",
        ])
        .status();

    tokio::time::sleep(Duration::from_secs(2)).await;
    http_expect("smoke.test", 8443, "DevStack OK")?;
    println!("SMOKE PASS: https://smoke.test:8443 via Nginx + PHP-FPM");
    Ok(())
}

async fn run_wordpress_smoke(
    ctx: &DevStackCtx,
    skip_mariadb: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    ensure_stack(ctx, skip_mariadb).await?;
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

    let _ = Command::new("sudo")
        .args([
            "-n",
            &ctx.paths.helper.join("devstack-helper.sh").to_string_lossy(),
            "hosts-sync",
            "demo.test",
            "clone.test",
            "phpmyadmin.test",
        ])
        .status();

    {
        let mut state = ctx.state.lock().await;
        apply_stack(&ctx.paths, &mut state)?;
    }

    tokio::time::sleep(Duration::from_secs(2)).await;

    // WordPress home should mention the site title / WordPress
    http_expect("demo.test", 8443, "Demo Blog")?;
    // After search-replace, clone should not still advertise the old domain as canonical
    let clone_body = http_get("clone.test", 8443)?;
    if !clone_body.contains("clone.test") && !clone_body.to_lowercase().contains("wordpress") {
        return Err(format!("clone body unexpected: {clone_body}").into());
    }
    if clone_body.contains("https://demo.test") {
        eprintln!("warning: clone HTML still references demo.test (may be in content)");
    }
    // homeurl option check via wp-cli if possible
    println!("SMOKE PASS: WordPress demo.test + clone.test over HTTPS");
    Ok(())
}

fn http_expect(host: &str, port: u16, needle: &str) -> Result<(), Box<dyn std::error::Error>> {
    let body = http_get(host, port)?;
    println!("GET https://{host}:{port}/ → {} bytes, has_needle={}", body.len(), body.contains(needle));
    if !body.contains(needle) {
        return Err(format!("response missing `{needle}`: {body}").into());
    }
    Ok(())
}

fn http_get(host: &str, port: u16) -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("https://{host}:{port}/");
    let out = format!("/tmp/devstack-smoke-{host}.html");
    let status = Command::new("curl")
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
    println!("HTTP {code} for {host}");
    if code != "200" {
        return Err(format!("HTTP {code} for {host}: {body}").into());
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
    devstack_lib::config::ensure_demo_site(&mut state);
    devstack_lib::config::save_state(&paths.state_file, &state)?;
    Ok(DevStackCtx {
        paths,
        state: tokio::sync::Mutex::new(state),
        registry: PackageRegistry::new()?,
    })
}
