//! PHP package provider.
//!
//! - **Windows:** live NTS x64 builds from
//!   `https://downloads.php.net/~windows/releases/releases.json`
//! - **macOS / Linux:** pluggable static-binary manifest (static-php-cli style).
//!   Configure via `AppState.php_manifest_url` or `DEVSTACK_PHP_MANIFEST_URL`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent, ProgressFn,
};
use crate::error::{DevStackError, Result};
use crate::php::{write_dev_php_ini, PhpIniOptions};

const WINDOWS_RELEASES: &str = "https://downloads.php.net/~windows/releases/releases.json";
const WINDOWS_BASE: &str = "https://downloads.php.net/~windows/releases/";

#[derive(Clone)]
pub struct PhpProvider {
    client: reqwest::Client,
}

impl PhpProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn manifest_url(state: &AppState) -> Option<String> {
        std::env::var("DEVSTACK_PHP_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| state.php_manifest_url.clone())
    }

    async fn list_windows(&self) -> Result<Vec<PackageVersion>> {
        let map: BTreeMap<String, Value> =
            self.client
                .get(WINDOWS_RELEASES)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

        let mut out = Vec::new();
        for (channel, entry) in map.iter().rev() {
            let version = entry
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or(channel)
                .to_string();

            let nts = find_nts_x64(entry);
            let (url, sha) = if let Some((path, sha)) = nts {
                (
                    Some(format!("{WINDOWS_BASE}{path}")),
                    Some(sha),
                )
            } else {
                (None, None)
            };

            out.push(PackageVersion {
                version: version.clone(),
                channel: channel.clone(),
                label: if version == *channel {
                    channel.clone()
                } else {
                    format!("{channel} ({version})")
                },
                lts: false,
                download_url: url,
                sha256: sha,
                notes: Some("Windows NTS x64".into()),
            });
        }
        out.sort_by(|a, b| b.channel.cmp(&a.channel));
        Ok(out)
    }

    async fn list_manifest(&self, url: &str) -> Result<Vec<PackageVersion>> {
        let manifest: PhpManifest = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let os = current_os();
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64",
            "aarch64" => "aarch64",
            other => other,
        };
        let os_aliases = match os {
            "macos" => vec!["macos", "darwin", "osx"],
            "linux" => vec!["linux"],
            "windows" => vec!["windows", "win"],
            other => vec![other],
        };

        let mut out = Vec::new();
        for pkg in manifest.packages {
            let os_ok = pkg
                .os
                .as_deref()
                .map(|o| os_aliases.iter().any(|a| a.eq_ignore_ascii_case(o)))
                .unwrap_or(true);
            let arch_ok = pkg
                .arch
                .as_deref()
                .map(|a| a.eq_ignore_ascii_case(arch) || (arch == "x86_64" && a == "x64"))
                .unwrap_or(true);
            if !os_ok || !arch_ok {
                continue;
            }
            let channel = pkg
                .channel
                .clone()
                .unwrap_or_else(|| major_minor(&pkg.version));
            out.push(PackageVersion {
                label: format!("{channel} ({})", pkg.version),
                version: pkg.version,
                channel,
                lts: pkg.lts.unwrap_or(false),
                download_url: Some(pkg.url),
                sha256: pkg.sha256,
                notes: pkg.notes,
            });
        }
        out.sort_by(|a, b| b.version.cmp(&a.version));
        Ok(out)
    }

    fn stub_versions() -> Vec<PackageVersion> {
        // Documented placeholder channels when no manifest is configured.
        ["8.5", "8.4", "8.3", "8.2", "8.1", "8.0", "7.4"]
            .into_iter()
            .map(|c| PackageVersion {
                version: c.to_string(),
                channel: c.to_string(),
                label: c.to_string(),
                lts: false,
                download_url: None,
                sha256: None,
                notes: Some(
                    "Set DEVSTACK_PHP_MANIFEST_URL (or php_manifest_url) to a static-php-cli style manifest to enable install on macOS/Linux."
                        .into(),
                ),
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct PhpManifest {
    #[serde(default)]
    packages: Vec<PhpManifestPackage>,
}

#[derive(Debug, Deserialize)]
struct PhpManifestPackage {
    version: String,
    channel: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    url: String,
    sha256: Option<String>,
    lts: Option<bool>,
    notes: Option<String>,
    #[allow(dead_code)]
    format: Option<String>,
}

fn find_nts_x64(entry: &Value) -> Option<(String, String)> {
    let obj = entry.as_object()?;
    // Prefer vs17 → vs16 → vc15 NTS x64.
    let preferred = ["nts-vs17-x64", "nts-vs16-x64", "nts-vc15-x64"];
    for key in preferred {
        if let Some(build) = obj.get(key) {
            if let Some(zip) = build.get("zip") {
                let path = zip.get("path")?.as_str()?.to_string();
                let sha = zip.get("sha256")?.as_str()?.to_string();
                return Some((path, sha));
            }
        }
    }
    // Fallback: any nts-*-x64
    for (key, build) in obj {
        if key.starts_with("nts-") && key.ends_with("-x64") {
            if let Some(zip) = build.get("zip") {
                let path = zip.get("path")?.as_str()?.to_string();
                let sha = zip.get("sha256")?.as_str()?.to_string();
                return Some((path, sha));
            }
        }
    }
    None
}

fn major_minor(version: &str) -> String {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        version.to_string()
    }
}

#[async_trait]
impl PackageProvider for PhpProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "php".into(),
            name: "PHP".into(),
            description: "Install and switch between PHP versions.".into(),
            category: "runtime".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, state: &AppState) -> Result<Vec<PackageVersion>> {
        match current_os() {
            "windows" => self.list_windows().await,
            _ => {
                if let Some(url) = Self::manifest_url(state) {
                    self.list_manifest(&url).await
                } else {
                    Ok(Self::stub_versions())
                }
            }
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
            .ok_or_else(|| {
                DevStackError::Message(
                    "No download URL for this PHP build. On macOS/Linux configure DEVSTACK_PHP_MANIFEST_URL."
                        .into(),
                )
            })?;
        let sha = meta.sha256.clone();

        let file_name = url
            .rsplit('/')
            .next()
            .unwrap_or("php-archive.bin")
            .to_string();
        let archive_path = paths.downloads.join(&file_name);

        download_and_verify(
            &self.client,
            &url,
            &archive_path,
            sha.as_deref(),
            "php",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "php".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("php", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;

        let kind = ArchiveKind::from_name(&file_name)?;
        // Windows ZIP has files at root; macOS/Linux tarballs often have one top folder.
        let strip = match kind {
            ArchiveKind::Zip => 0,
            _ => 1,
        };
        extract_archive(&archive_path, &dest, kind, strip)?;
        let _ = fs::remove_file(&archive_path);

        write_dev_php_ini(&dest, &PhpIniOptions::default())?;

        // Detect format from manifest if present
        let _ = meta;

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest.clone(),
            installed_at: now_iso(),
        };
        mark_installed(state, "php", record.clone())?;
        state
            .php_extensions
            .entry(meta.version.clone())
            .or_default();
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "php".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "php".into(),
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
        default_uninstall("php", version, paths, state).await
    }
}

/// Resolve php.ini path inside an installed PHP tree.
pub fn php_ini_path(install_dir: &Path) -> std::path::PathBuf {
    let candidates = [
        install_dir.join("php.ini"),
        install_dir.join("etc/php.ini"),
        install_dir.join("conf/php.ini"),
    ];
    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| install_dir.join("php.ini"))
}
