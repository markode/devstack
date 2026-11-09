//! Share a local site over a Cloudflare quick tunnel (`cloudflared tunnel --url …`).
//!
//! While shared, WordPress `siteurl`/`home` point at the public URL; they are
//! restored to the canonical local URL when the tunnel stops.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Serialize;

use crate::config::{save_state, AppState, Paths, SiteRecord};
use crate::downloads::{download_file, http_client, ProgressFn};
use crate::error::{DevStackError, Result};
use crate::services::database::{client_bin_for, engine_install_dir};
use crate::services::ports::canonical_site_url;
use crate::services::process::{is_running, spawn_logged, stop_pid};
use crate::services::progress::{emit, JobProgressFn};
use crate::services::wp_url::site_is_wordpress;

const RELEASE_BASE: &str = "https://github.com/cloudflare/cloudflared/releases/latest/download";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    pub active: bool,
    pub site_id: Option<String>,
    pub url: Option<String>,
    pub pid: Option<u32>,
}

/// Official release asset for an OS/arch pair (`std::env::consts` values).
pub fn asset_name(os: &str, arch: &str) -> Result<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("cloudflared-linux-amd64"),
        ("linux", "aarch64") => Ok("cloudflared-linux-arm64"),
        ("macos", "x86_64") => Ok("cloudflared-darwin-amd64.tgz"),
        ("macos", "aarch64") => Ok("cloudflared-darwin-arm64.tgz"),
        ("windows", "x86_64") => Ok("cloudflared-windows-amd64.exe"),
        ("windows", "aarch64") => Ok("cloudflared-windows-amd64.exe"),
        _ => Err(DevStackError::UnsupportedPlatform(format!("{os}/{arch}"))),
    }
}

pub fn cloudflared_path(paths: &Paths) -> PathBuf {
    paths
        .tools
        .join("cloudflared")
        .join(if cfg!(windows) { "cloudflared.exe" } else { "cloudflared" })
}

fn extract_tgz_binary(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let is_bin = entry
            .path()?
            .file_name()
            .map(|n| n == "cloudflared")
            .unwrap_or(false);
        if is_bin {
            let mut out = fs::File::create(dest)?;
            std::io::copy(&mut entry, &mut out)?;
            return Ok(());
        }
    }
    Err(DevStackError::Archive("cloudflared not found in archive".into()))
}

pub async fn ensure_cloudflared(
    paths: &Paths,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<PathBuf> {
    let dest = cloudflared_path(paths);
    if dest.exists() && fs::metadata(&dest).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        return Ok(dest);
    }
    let asset = asset_name(std::env::consts::OS, std::env::consts::ARCH)?;
    fs::create_dir_all(dest.parent().unwrap())?;
    emit(progress, job_id, "cloudflared", "Downloading cloudflared…", 0, None);
    let client = http_client()?;
    let dl: ProgressFn = {
        let progress = progress.clone();
        let job_id = job_id.to_string();
        std::sync::Arc::new(move |ev| {
            emit(&progress, &job_id, "cloudflared", "Downloading cloudflared…", ev.downloaded, ev.total);
        })
    };
    let url = format!("{RELEASE_BASE}/{asset}");
    if asset.ends_with(".tgz") {
        let tmp = paths.downloads.join(asset);
        download_file(&client, &url, &tmp, "cloudflared", "latest", &dl).await?;
        extract_tgz_binary(&tmp, &dest)?;
        let _ = fs::remove_file(&tmp);
    } else {
        download_file(&client, &url, &dest, "cloudflared", "latest", &dl).await?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms)?;
    }
    Ok(dest)
}

/// First `https://*.trycloudflare.com` URL in a cloudflared log line/blob.
pub fn parse_trycloudflare_url(text: &str) -> Option<String> {
    let re = Regex::new(r"https://[a-zA-Z0-9][a-zA-Z0-9-]*\.trycloudflare\.com").ok()?;
    re.find(text).map(|m| m.as_str().to_string())
}

/// Tunnel origin args: plain HTTP for non-HTTPS sites, local HTTPS (self-signed
/// CA, so verification off) otherwise — avoids http→https redirect loops.
pub fn tunnel_args(site: &SiteRecord, state: &AppState) -> Vec<String> {
    let apache = crate::services::httpd::site_uses_apache(site);
    let (http, https) = if apache {
        (state.stack.apache_http_port, state.stack.apache_https_port)
    } else {
        (state.stack.http_port, state.stack.https_port)
    };
    let mut args = vec!["tunnel".to_string(), "--no-autoupdate".into()];
    if site.https {
        args.extend([
            "--url".into(),
            format!("https://127.0.0.1:{https}"),
            "--no-tls-verify".into(),
            "--origin-server-name".into(),
            site.domain.clone(),
        ]);
    } else {
        args.extend(["--url".into(), format!("http://127.0.0.1:{http}")]);
    }
    args.extend(["--http-host-header".into(), site.domain.clone()]);
    args
}

fn wp_engine(site: &SiteRecord, state: &AppState) -> String {
    site.database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into())
}

fn rewrite_wp(paths: &Paths, state: &AppState, site: &SiteRecord, from: &str, to: &str) {
    if !site_is_wordpress(site) || from == to {
        return;
    }
    let Some(db) = site.database_name.as_deref() else {
        return;
    };
    let engine = wp_engine(site, state);
    if !matches!(engine.as_str(), "mysql" | "mariadb") {
        return;
    }
    let ready = engine_install_dir(state, &engine)
        .and_then(|root| client_bin_for(&root))
        .is_ok();
    if !ready {
        return;
    }
    if let Err(e) = crate::services::db_files::mysql_wp_options_url_replace(
        paths, state, &engine, db, from, to,
    ) {
        crate::applog::warn("share", format!("WP URL rewrite {from} → {to}: {e}"));
    }
}

pub fn share_info(state: &AppState) -> ShareInfo {
    let pid = state.runtime.cloudflared_pid.filter(|p| is_running(*p));
    ShareInfo {
        active: pid.is_some() && state.runtime.share_url.is_some(),
        site_id: state.runtime.share_site_id.clone(),
        url: state.runtime.share_url.clone(),
        pid,
    }
}

/// Start a quick tunnel for `site_id` using an already-installed `cloudflared`.
pub fn start_share(
    paths: &Paths,
    state: &mut AppState,
    site_id: &str,
    cloudflared: &Path,
) -> Result<ShareInfo> {
    let _ = stop_share(paths, state);
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| DevStackError::Message(format!("site not found: {site_id}")))?;
    if state.runtime.nginx_pid.map(is_running).unwrap_or(false) == false
        && state.runtime.apache_pid.map(is_running).unwrap_or(false) == false
    {
        return Err(DevStackError::Message(
            "Start the stack before sharing a site".into(),
        ));
    }

    let args = tunnel_args(&site, state);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = paths.logs.join("cloudflared.out.log");
    let err = paths.logs.join("cloudflared.err.log");
    let mut child = spawn_logged(cloudflared, &arg_refs, None, &out, &err)?;
    let pid = child.id();

    let deadline = Instant::now() + Duration::from_secs(45);
    let mut url = None;
    while Instant::now() < deadline {
        let mut blob = fs::read_to_string(&err).unwrap_or_default();
        blob.push_str(&fs::read_to_string(&out).unwrap_or_default());
        if let Some(u) = parse_trycloudflare_url(&blob) {
            url = Some(u);
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(DevStackError::Message(format!(
                "cloudflared exited early ({status}); see logs/cloudflared.err.log"
            )));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let Some(url) = url else {
        let _ = stop_pid(pid);
        return Err(DevStackError::Message(
            "Timed out waiting for the trycloudflare.com URL".into(),
        ));
    };

    let local = canonical_site_url(&site, state);
    rewrite_wp(paths, state, &site, &local, &url);

    state.runtime.cloudflared_pid = Some(pid);
    state.runtime.share_url = Some(url);
    state.runtime.share_site_id = Some(site_id.to_string());
    save_state(&paths.state_file, state)?;
    crate::applog::info("share", format!("sharing {} at {}", site.domain, share_info(state).url.unwrap_or_default()));
    Ok(share_info(state))
}

/// Stop the tunnel (if any) and restore WordPress URLs. Always clears runtime.
pub fn stop_share(paths: &Paths, state: &mut AppState) -> Result<ShareInfo> {
    let url = state.runtime.share_url.take();
    let site_id = state.runtime.share_site_id.take();
    let pid = state.runtime.cloudflared_pid.take();
    if let (Some(url), Some(site_id)) = (url.as_deref(), site_id.as_deref()) {
        if let Some(site) = state.sites.iter().find(|s| s.id == site_id).cloned() {
            let local = canonical_site_url(&site, state);
            rewrite_wp(paths, state, &site, url, &local);
        }
    }
    if let Some(pid) = pid {
        stop_pid(pid)?;
    }
    if pid.is_some() || url.is_some() {
        save_state(&paths.state_file, state)?;
    }
    Ok(share_info(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_trycloudflare_url_from_log_lines() {
        let log = "2026-11-09T10:00:00Z INF Requesting new quick Tunnel on trycloudflare.com...\n\
2026-11-09T10:00:02Z INF +--------------------------------------------------------------------------------------------+\n\
2026-11-09T10:00:02Z INF |  Your quick Tunnel has been created! Visit it at (it may take some time to be reachable):  |\n\
2026-11-09T10:00:02Z INF |  https://quiet-river-1234.trycloudflare.com                                                |\n";
        assert_eq!(
            parse_trycloudflare_url(log).as_deref(),
            Some("https://quiet-river-1234.trycloudflare.com")
        );
        assert_eq!(parse_trycloudflare_url("INF Starting tunnel"), None);
        assert_eq!(parse_trycloudflare_url("https://api.trycloudflare.com/tunnel"), Some("https://api.trycloudflare.com".into()));
    }

    #[test]
    fn asset_names_are_platform_specific() {
        assert_eq!(asset_name("linux", "x86_64").unwrap(), "cloudflared-linux-amd64");
        assert_eq!(asset_name("macos", "aarch64").unwrap(), "cloudflared-darwin-arm64.tgz");
        assert!(asset_name("windows", "x86_64").unwrap().ends_with(".exe"));
        assert!(asset_name("plan9", "x86_64").is_err());
    }

    #[test]
    fn tunnel_args_use_http_or_https_origin() {
        let mut state = AppState::default();
        state.stack.http_port = 8080;
        state.stack.https_port = 8443;
        let mut site = SiteRecord {
            id: "a".into(),
            name: "a".into(),
            path: PathBuf::from("/tmp/a"),
            domain: "a.test".into(),
            php_version: None,
            node_version: None,
            https: false,
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
            tld: crate::config::default_tld(),
            aliases: Vec::new(),
            wildcard: false,
        };
        let args = tunnel_args(&site, &state);
        assert!(args.contains(&"http://127.0.0.1:8080".to_string()));
        site.https = true;
        let args = tunnel_args(&site, &state);
        assert!(args.contains(&"https://127.0.0.1:8443".to_string()));
        assert!(args.contains(&"--no-tls-verify".to_string()));
    }
}
