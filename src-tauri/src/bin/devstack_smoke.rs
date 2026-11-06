//! CI / headless smoke: list versions from live sources and optionally install Node.

use std::sync::Arc;

use devstack_lib::packages::PackageRegistry;
use devstack_lib::{data_root, DevStackCtx};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let install_node = args.iter().any(|a| a == "--install-node");
    let channel = args
        .windows(2)
        .find(|w| w[0] == "--node-channel")
        .map(|w| w[1].clone())
        .unwrap_or_else(|| "22".into());

    let ctx = bootstrap()?;
    let state = ctx.state.lock().await;

    println!("DevStack smoke — os={} arch={}", std::env::consts::OS, std::env::consts::ARCH);
    println!("data dir: {}", ctx.paths.root.display());

    for id in [
        "php",
        "node",
        "phpmyadmin",
        "mysql",
        "mariadb",
        "mongodb",
        "postgresql",
    ] {
        let provider = ctx.registry.get(id)?;
        match provider.list_versions(&state).await {
            Ok(versions) => {
                println!(
                    "[{id}] {} version(s); latest={}",
                    versions.len(),
                    versions
                        .first()
                        .map(|v| v.label.as_str())
                        .unwrap_or("-")
                );
            }
            Err(e) => println!("[{id}] list error: {e}"),
        }
    }
    drop(state);

    if install_node {
        println!("Installing Node channel {channel}…");
        let provider = ctx.registry.get("node")?;
        let progress = Arc::new(|ev: devstack_lib::downloads::ProgressEvent| {
            if ev.phase == "downloading" {
                if let Some(total) = ev.total {
                    eprint!("\r  {} / {} bytes", ev.downloaded, total);
                }
            } else {
                eprintln!("\n  phase={}", ev.phase);
            }
        });
        let mut state = ctx.state.lock().await;
        let installed = provider
            .install(&channel, &ctx.paths, &mut state, progress)
            .await?;
        println!(
            "\nInstalled Node {} → {}",
            installed.version,
            installed.path.display()
        );
    }

    Ok(())
}

fn bootstrap() -> Result<DevStackCtx, Box<dyn std::error::Error>> {
    // Re-create the same wiring as the app without Tauri.
    let paths = devstack_lib::config::Paths::from_root(data_root())?;
    let mut state = devstack_lib::config::load_state(&paths.state_file)?;
    devstack_lib::config::ensure_demo_site(&mut state);
    devstack_lib::config::save_state(&paths.state_file, &state)?;
    Ok(DevStackCtx {
        paths,
        state: tokio::sync::Mutex::new(state),
        registry: PackageRegistry::new()?,
    })
}
