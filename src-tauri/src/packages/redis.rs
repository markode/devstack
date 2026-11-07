//! Redis — official source tarballs (checksums from redis/redis-hashes) + build;
//! Windows uses tporadowski binary builds.

use async_trait::async_trait;
use regex::Regex;
use std::fs;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_os, download_and_verify, extract_archive, fetch_text, ArchiveKind, ProgressEvent,
    ProgressFn,
};
use crate::error::{DevStackError, Result};

const HASHES_URL: &str = "https://raw.githubusercontent.com/redis/redis-hashes/master/README";
const WIN_RELEASES: &str =
    "https://api.github.com/repos/tporadowski/redis/releases?per_page=15";

#[derive(Clone)]
pub struct RedisProvider {
    client: reqwest::Client,
}

impl RedisProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    async fn list_unix(&self) -> Result<Vec<PackageVersion>> {
        let body = fetch_text(&self.client, HASHES_URL).await?;
        let re = Regex::new(
            r#"hash\s+(redis-([0-9]+\.[0-9]+\.[0-9]+)\.tar\.gz)\s+sha256\s+([a-fA-F0-9]+)\s+(\S+)"#,
        )
        .map_err(|e| DevStackError::Message(e.to_string()))?;
        let mut out = Vec::new();
        for cap in re.captures_iter(&body) {
            let file = cap[1].to_string();
            let version = cap[2].to_string();
            let sha = cap[3].to_lowercase();
            let url = cap[4].replace("http://", "https://");
            let channel = version
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            out.push(PackageVersion {
                label: format!("Redis {version}"),
                version: version.clone(),
                channel,
                lts: version.starts_with('7') || version.starts_with('8'),
                download_url: Some(url),
                sha256: Some(sha),
                notes: Some(format!("source {file} — built locally")),
            });
        }
        // Newest first, but keep a few 7.x builds available for lean CI installs
        out.sort_by(|a, b| cmp_ver(&b.version, &a.version));
        let mut slim = Vec::new();
        let mut seen_major7 = 0;
        for v in out {
            let major7 = v.version.starts_with('7');
            if slim.len() < 12 || (major7 && seen_major7 < 4) {
                if major7 {
                    seen_major7 += 1;
                }
                slim.push(v);
            }
            if slim.len() >= 16 {
                break;
            }
        }
        Ok(slim)
    }

    async fn list_windows(&self) -> Result<Vec<PackageVersion>> {
        #[derive(serde::Deserialize)]
        struct Rel {
            tag_name: String,
            assets: Vec<Asset>,
            #[serde(default)]
            draft: bool,
            #[serde(default)]
            prerelease: bool,
        }
        #[derive(serde::Deserialize)]
        struct Asset {
            name: String,
            browser_download_url: String,
        }
        let releases: Vec<Rel> = self
            .client
            .get(WIN_RELEASES)
            .header("User-Agent", "DevStack")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut out = Vec::new();
        for r in releases {
            if r.draft || r.prerelease {
                continue;
            }
            let Some(asset) = r
                .assets
                .iter()
                .find(|a| a.name.ends_with(".zip") && a.name.to_lowercase().contains("redis"))
            else {
                continue;
            };
            let version = r.tag_name.trim_start_matches('v').to_string();
            out.push(PackageVersion {
                label: format!("Redis {version}"),
                version: version.clone(),
                channel: version.split('.').take(2).collect::<Vec<_>>().join("."),
                lts: true,
                download_url: Some(asset.browser_download_url.clone()),
                sha256: None,
                notes: Some("tporadowski Windows build".into()),
            });
            if out.len() >= 10 {
                break;
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl PackageProvider for RedisProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "redis".into(),
            name: "Redis".into(),
            description: "Redis in-memory store — install, start, stop.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        if current_os() == "windows" {
            self.list_windows().await
        } else {
            self.list_unix().await
        }
    }

    async fn install(
        &self,
        version: &str,
        paths: &Paths,
        state: &mut AppState,
        progress: ProgressFn,
    ) -> Result<InstalledPackage> {
        let versions = self.list_versions(state).await?;
        let meta = versions
            .into_iter()
            .find(|v| v.version == version || v.channel == version)
            .ok_or_else(|| DevStackError::VersionNotFound(version.into()))?;
        let url = meta
            .download_url
            .clone()
            .ok_or_else(|| DevStackError::Message("No Redis download URL".into()))?;
        let file_name = url.rsplit('/').next().unwrap_or("redis.tgz").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            "redis",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "redis".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("redis", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        let kind = ArchiveKind::from_name(&file_name)?;
        extract_archive(&archive, &dest, kind, if kind == ArchiveKind::Zip { 0 } else { 1 })?;
        let _ = fs::remove_file(&archive);

        if current_os() != "windows" {
            progress(ProgressEvent {
                package_id: "redis".into(),
                version: meta.version.clone(),
                phase: "building".into(),
                downloaded: 0,
                total: None,
            });
            // Source tree may be nested; find Makefile
            let src = if dest.join("src").join("Makefile").exists() || dest.join("Makefile").exists()
            {
                dest.clone()
            } else {
                fs::read_dir(&dest)?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.join("Makefile").exists() || p.join("src").join("Makefile").exists())
                    .unwrap_or(dest.clone())
            };
            // Build core binaries only (skip Redis 8 modules that need extra deps).
            // MALLOC=libc avoids jemalloc LTO link failures on some Linux toolchains.
            let src_dir = if src.join("src").join("Makefile").exists() {
                src.join("src")
            } else {
                src.clone()
            };
            let status = crate::services::process::command("make")
                .args([
                    "-j",
                    "2",
                    "BUILD_TLS=no",
                    "MALLOC=libc",
                    "redis-server",
                    "redis-cli",
                ])
                .current_dir(&src_dir)
                .status()
                .map_err(|e| DevStackError::Message(format!("make redis: {e}")))?;
            if !status.success() {
                return Err(DevStackError::Message("redis make failed".into()));
            }
            // Ensure binaries live under dest/src/redis-server (default) or copy to dest/bin
            let bin_dir = dest.join("bin");
            fs::create_dir_all(&bin_dir)?;
            for name in ["redis-server", "redis-cli"] {
                let candidates = [
                    src.join("src").join(name),
                    src.join(name),
                    dest.join("src").join(name),
                ];
                if let Some(found) = candidates.into_iter().find(|p| p.exists()) {
                    let _ = fs::copy(&found, bin_dir.join(name));
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = fs::set_permissions(bin_dir.join(name), fs::Permissions::from_mode(0o755));
                    }
                }
            }
            if !bin_dir.join("redis-server").exists() {
                return Err(DevStackError::Message(
                    "redis-server binary missing after build".into(),
                ));
            }
        }

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "redis", record.clone())?;
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "redis".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "redis".into(),
            version: record.version,
            channel: record.channel,
            path: record.path,
            installed_at: record.installed_at,
        })
    }

    async fn uninstall(
        &self,
        version: &str,
        paths: &Paths,
        state: &mut AppState,
    ) -> Result<()> {
        default_uninstall("redis", version, paths, state).await
    }
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c| c == '.' || c == '-')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(a).cmp(&parse(b))
}
