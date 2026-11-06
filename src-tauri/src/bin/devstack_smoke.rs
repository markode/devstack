//! CI / headless smoke for milestone 2: Nginx + PHP-FPM + MariaDB + https://name.test

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use devstack_lib::downloads::ProgressFn;
use devstack_lib::packages::PackageRegistry;
use devstack_lib::services::php_runtime::register_system_php;
use devstack_lib::services::sites::create_site;
use devstack_lib::services::stack::{apply_stack, stop_stack};
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
    let stack = args.iter().any(|a| a == "--stack") || !install_node;
    let skip_mariadb = args.iter().any(|a| a == "--skip-mariadb");

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

    Ok(())
}

async fn run_stack_smoke(ctx: &DevStackCtx, skip_mariadb: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Use unprivileged ports for CI VMs.
    {
        let mut state = ctx.state.lock().await;
        state.stack.http_port = 8080;
        state.stack.https_port = 8443;
        let _ = register_system_php(&mut state, &ctx.paths)?;
        println!("registered system PHP");
    }

    // Nginx via provider (noble .deb extract)
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
        // Prefer a smaller/stable LTS line if listed
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
            println!("No MariaDB binary for this platform — continuing without DB");
        }
    }

    // phpMyAdmin optional for smoke HTTP check
    let _ = install_pkg(ctx, "phpmyadmin", "5.2.3").await;

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
            .or_else(|| state.stack.php_version.clone())
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
        }
        println!("site {} → {}", site.domain, site_path.display());
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

    // Hosts: best-effort privileged helper; always also prove with --resolve
    let _ = Command::new("sudo")
        .args([
            "-n",
            &ctx.paths.helper.join("devstack-helper.sh").to_string_lossy(),
            "hosts-sync",
            "smoke.test",
            "phpmyadmin.test",
        ])
        .status();

    // Give nginx/php a moment
    tokio::time::sleep(Duration::from_secs(2)).await;

    let url = "https://smoke.test:8443/";
    println!("GET {url}");
    let status = Command::new("curl")
        .args([
            "-k",
            "--resolve",
            "smoke.test:8443:127.0.0.1",
            "-sS",
            "-o",
            "/tmp/devstack-smoke-body.txt",
            "-w",
            "%{http_code}",
            url,
        ])
        .output()?;
    let code = String::from_utf8_lossy(&status.stdout).trim().to_string();
    let body = std::fs::read_to_string("/tmp/devstack-smoke-body.txt").unwrap_or_default();
    println!("HTTP {code}");
    println!("body: {body}");
    if code != "200" || !body.contains("DevStack OK") {
        // dump logs
        let _ = Command::new("bash")
            .args([
                "-lc",
                &format!(
                    "echo '--- nginx err ---'; tail -n 40 {0}/logs/nginx/error.log 2>/dev/null; echo '--- nginx stderr ---'; tail -n 40 {0}/logs/nginx/stderr.log 2>/dev/null; echo '--- php ---'; tail -n 40 {0}/logs/php-fpm-*-stderr.log 2>/dev/null",
                    ctx.paths.root.display()
                ),
            ])
            .status();
        return Err(format!("smoke failed: HTTP {code} body={body}").into());
    }
    println!("SMOKE PASS: https://smoke.test:8443 via Nginx + PHP-FPM");
    Ok(())
}

async fn install_pkg(
    ctx: &DevStackCtx,
    id: &str,
    version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Skip if already installed
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
    devstack_lib::config::ensure_demo_site(&mut state);
    devstack_lib::config::save_state(&paths.state_file, &state)?;
    Ok(DevStackCtx {
        paths,
        state: tokio::sync::Mutex::new(state),
        registry: PackageRegistry::new()?,
    })
}
