//! PostgreSQL — prebuilt binaries from theseus-rs/postgresql-binaries (SHA256).

use async_trait::async_trait;
use serde::Deserialize;
use std::fs;
use std::process::Command;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_arch, current_os, download_and_verify, fetch_text, ProgressEvent, ProgressFn,
};
use crate::error::{DevStackError, Result};

const RELEASES_API: &str =
    "https://api.github.com/repos/theseus-rs/postgresql-binaries/releases?per_page=40";

#[derive(Clone)]
pub struct PostgreSqlProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

impl PostgreSqlProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn target_triple() -> Result<&'static str> {
        match (current_os(), current_arch()) {
            ("linux", "x64") => Ok("x86_64-unknown-linux-gnu"),
            ("linux", "arm64") => Ok("aarch64-unknown-linux-gnu"),
            ("macos", "x64") => Ok("x86_64-apple-darwin"),
            ("macos", "arm64") => Ok("aarch64-apple-darwin"),
            ("windows", "x64") => Ok("x86_64-pc-windows-msvc"),
            ("windows", "arm64") => Ok("aarch64-pc-windows-msvc"),
            (os, arch) => Err(DevStackError::UnsupportedPlatform(format!("{os}/{arch}"))),
        }
    }

    fn asset_for(release: &GhRelease, triple: &str) -> Option<(String, String)> {
        let archive_name = format!("postgresql-{}-{triple}.tar.gz", release.tag_name);
        let url = release
            .assets
            .iter()
            .find(|a| a.name == archive_name)?
            .browser_download_url
            .clone();
        Some((archive_name, url))
    }
}

#[async_trait]
impl PackageProvider for PostgreSqlProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "postgresql".into(),
            name: "PostgreSQL".into(),
            description: "PostgreSQL server — install, start, stop.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let triple = Self::target_triple()?;
        let releases: Vec<GhRelease> = self
            .client
            .get(RELEASES_API)
            .header("Accept", "application/vnd.github+json")
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
            let Some((_name, url)) = Self::asset_for(&r, triple) else {
                continue;
            };
            let major = r.tag_name.split('.').next().unwrap_or(&r.tag_name).to_string();
            let sha_url = format!("{url}.sha256");
            let sha256 = fetch_text(&self.client, &sha_url)
                .await
                .ok()
                .and_then(|s| {
                    s.split_whitespace()
                        .next()
                        .map(|h| h.trim().to_lowercase())
                });
            out.push(PackageVersion {
                label: format!("PostgreSQL {}", r.tag_name),
                version: r.tag_name.clone(),
                channel: major,
                lts: true,
                download_url: Some(url),
                sha256,
                notes: Some(format!("theseus-rs binary ({triple})")),
            });
            if out.len() >= 12 {
                break;
            }
        }
        Ok(out)
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
            .ok_or_else(|| DevStackError::Message("No PostgreSQL binary for this OS/arch".into()))?;
        let file_name = url.rsplit('/').next().unwrap_or("postgresql.tar.gz").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            "postgresql",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "postgresql".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("postgresql", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        // System tar handles hard-linked timezone files that the Rust tar crate rejects.
        let status = Command::new("tar")
            .args([
                "-xzf",
                &archive.to_string_lossy(),
                "-C",
                &dest.to_string_lossy(),
                "--strip-components=1",
            ])
            .status()
            .map_err(|e| DevStackError::Message(format!("tar extract: {e}")))?;
        if !status.success() {
            // Retry without strip in case layout differs
            let status2 = Command::new("tar")
                .args([
                    "-xzf",
                    &archive.to_string_lossy(),
                    "-C",
                    &dest.to_string_lossy(),
                ])
                .status()
                .map_err(|e| DevStackError::Message(format!("tar extract: {e}")))?;
            if !status2.success() {
                return Err(DevStackError::Archive("postgresql tar extract failed".into()));
            }
            if !dest.join("bin").exists() {
                if let Ok(entries) = fs::read_dir(&dest) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.join("bin").exists() {
                            for child in fs::read_dir(&p)?.flatten() {
                                let name = child.file_name();
                                let _ = fs::rename(child.path(), dest.join(name));
                            }
                            let _ = fs::remove_dir_all(&p);
                            break;
                        }
                    }
                }
            }
        }
        let _ = fs::remove_file(&archive);

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "postgresql", record.clone())?;
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "postgresql".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "postgresql".into(),
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
        default_uninstall("postgresql", version, paths, state).await
    }
}
