//! Apache httpd package provider.
//!
//! - **Windows:** official [Apache Lounge](https://www.apachelounge.com/) VS17 Win64 zips
//!   with SHA-256 from the adjacent `.zip.txt` checksum file.
//! - **macOS / Linux:** pluggable static/prebuilt manifest (`DEVSTACK_APACHE_MANIFEST_URL`
//!   / `apache_manifest_url`), same shape as the Nginx/PHP manifests.

use async_trait::async_trait;
use serde::Deserialize;
use std::fs;
use std::path::Path;

use super::macos_defaults::{arch_aliases, os_aliases, DEFAULT_COMBINED_MANIFEST_URL};
use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent, ProgressFn,
};
use crate::error::{DevStackError, Result};
use crate::services::process::find_bin;

const LOUNGE_VS17: &str = "https://www.apachelounge.com/download/VS17/";
const LOUNGE_VS17_BIN: &str = "https://www.apachelounge.com/download/VS17/binaries/";

#[derive(Clone)]
pub struct ApacheProvider {
    client: reqwest::Client,
}

impl ApacheProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn manifest_url(state: &AppState) -> Option<String> {
        std::env::var("DEVSTACK_APACHE_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| state.apache_manifest_url.clone())
            .or_else(|| {
                // Baked default for macOS/Linux — packages-macos-v1 release.
                match current_os() {
                    "macos" | "linux" => Some(DEFAULT_COMBINED_MANIFEST_URL.to_string()),
                    _ => None,
                }
            })
    }

    async fn list_windows_lounge(&self) -> Result<Vec<PackageVersion>> {
        let html = self
            .client
            .get(LOUNGE_VS17)
            .header("User-Agent", "DevStack/0.1 (package-manager)")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        // e.g. /download/VS17/binaries/httpd-2.4.66-251206-Win64-VS17.zip
        let re = regex::Regex::new(
            r#"(?i)href="(/download/VS17/binaries/(httpd-(\d+\.\d+\.\d+)-\d+-Win64-VS17\.zip))""#,
        )
        .unwrap();
        let mut found: Vec<(String, String, String)> = Vec::new();
        for cap in re.captures_iter(&html) {
            let rel = cap.get(1).unwrap().as_str().to_string();
            let file = cap.get(2).unwrap().as_str().to_string();
            let version = cap.get(3).unwrap().as_str().to_string();
            found.push((version, file, rel));
        }
        found.sort_by(|a, b| cmp_ver(&b.0, &a.0));
        found.dedup_by(|a, b| a.0 == b.0);
        found.truncate(8);

        let mut out = Vec::new();
        for (version, file, _rel) in found {
            let url = format!("{LOUNGE_VS17_BIN}{file}");
            let sha = self.fetch_lounge_sha256(&format!("{url}.txt")).await.ok();
            out.push(PackageVersion {
                channel: version.split('.').take(2).collect::<Vec<_>>().join("."),
                label: format!("{version} (Apache Lounge VS17 x64)"),
                version: version.clone(),
                lts: false,
                download_url: Some(url),
                sha256: sha,
                notes: Some("Official Apache Lounge Win64 VS17 build".into()),
            });
        }

        if out.is_empty() {
            // Curated fallback if HTML layout changes
            let version = "2.4.66";
            let file = "httpd-2.4.66-251206-Win64-VS17.zip";
            let url = format!("{LOUNGE_VS17_BIN}{file}");
            let sha = self.fetch_lounge_sha256(&format!("{url}.txt")).await.ok();
            out.push(PackageVersion {
                channel: "2.4".into(),
                label: format!("{version} (Apache Lounge VS17 x64)"),
                version: version.into(),
                lts: true,
                download_url: Some(url),
                sha256: sha,
                notes: Some("Fallback Apache Lounge Win64 VS17 build".into()),
            });
        }
        Ok(out)
    }

    async fn fetch_lounge_sha256(&self, txt_url: &str) -> Result<String> {
        let body = self
            .client
            .get(txt_url)
            .header("User-Agent", "DevStack/0.1 (package-manager)")
            .header("Referer", LOUNGE_VS17)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        // "SHA256-Checksum for: …:\n<HEX>"
        let re = regex::Regex::new(r"(?i)SHA256-Checksum[^:]*:\s*([A-Fa-f0-9]{64})").unwrap();
        re.captures(&body)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_ascii_lowercase()))
            .ok_or_else(|| DevStackError::Message(format!("no SHA256 in {txt_url}")))
    }

    async fn list_manifest(&self, url: &str) -> Result<Vec<PackageVersion>> {
        #[derive(Deserialize)]
        struct Manifest {
            packages: Vec<ManPkg>,
        }
        #[derive(Deserialize)]
        struct ManPkg {
            #[serde(default)]
            id: Option<String>,
            version: String,
            channel: Option<String>,
            os: Option<String>,
            arch: Option<String>,
            url: String,
            sha256: Option<String>,
            notes: Option<String>,
        }
        let m: Manifest = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let os = current_os();
        let has_ids = m.packages.iter().any(|p| p.id.is_some());
        Ok(m.packages
            .into_iter()
            .filter(|p| {
                if has_ids {
                    p.id
                        .as_deref()
                        .map(|id| id.eq_ignore_ascii_case("apache") || id.eq_ignore_ascii_case("httpd"))
                        .unwrap_or(false)
                } else {
                    true
                }
            })
            .filter(|p| {
                p.os
                    .as_deref()
                    .map(|o| os_aliases(o, os))
                    .unwrap_or(true)
                    && p.arch.as_deref().map(arch_aliases).unwrap_or(true)
            })
            .map(|p| PackageVersion {
                channel: p.channel.unwrap_or_else(|| {
                    p.version.split('.').take(2).collect::<Vec<_>>().join(".")
                }),
                label: p.version.clone(),
                version: p.version,
                lts: false,
                download_url: Some(p.url),
                sha256: p.sha256,
                notes: p.notes,
            })
            .collect())
    }

    fn flatten_apache_tree(dest: &Path) -> Result<()> {
        // Apache Lounge zips usually contain Apache24/…
        if find_bin(dest, &["httpd", "httpd.exe"]).is_some() {
            return Ok(());
        }
        for entry in fs::read_dir(dest)?.flatten() {
            let p = entry.path();
            if p.is_dir() && find_bin(&p, &["httpd", "httpd.exe"]).is_some() {
                // Move children up
                for child in fs::read_dir(&p)?.flatten() {
                    let from = child.path();
                    let name = from.file_name().unwrap();
                    let to = dest.join(name);
                    if to.exists() {
                        continue;
                    }
                    let _ = fs::rename(&from, &to);
                }
                let _ = fs::remove_dir_all(&p);
                break;
            }
        }
        if find_bin(dest, &["httpd", "httpd.exe"]).is_none() {
            return Err(DevStackError::Message(format!(
                "httpd binary not found after extract in {}",
                dest.display()
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl PackageProvider for ApacheProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "apache".into(),
            name: "Apache".into(),
            description: "Apache HTTP Server — full .htaccess / mod_rewrite for WordPress."
                .into(),
            category: "service".into(),
            // Windows: Lounge builds. macOS/Linux: set DEVSTACK_APACHE_MANIFEST_URL
            // (list_versions explains when missing).
            install_ready: matches!(current_os(), "windows" | "macos" | "linux"),
        }
    }

    async fn list_versions(&self, state: &AppState) -> Result<Vec<PackageVersion>> {
        if let Some(url) = Self::manifest_url(state) {
            match self.list_manifest(&url).await {
                Ok(listed) if !listed.is_empty() => return Ok(listed),
                Ok(_) => {
                    eprintln!("[apache] manifest {url} returned no packages for this OS/arch");
                }
                Err(e) => {
                    eprintln!("[apache] manifest {url} unavailable ({e})");
                }
            }
        }
        match current_os() {
            "windows" => self.list_windows_lounge().await,
            "macos" | "linux" => Err(DevStackError::Message(
                "No prebuilt Apache httpd packages for this OS/arch yet. \
                 The packages-macos-v1 release (or DEVSTACK_APACHE_MANIFEST_URL) must provide download URLs."
                    .into(),
            )),
            other => Err(DevStackError::Message(format!(
                "Apache installs are not configured for OS `{other}`"
            ))),
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
        let url = meta.download_url.clone().ok_or_else(|| {
            DevStackError::Message(format!(
                "No Apache download URL for {} on {}",
                meta.version,
                current_os()
            ))
        })?;

        let file_name = url.rsplit('/').next().unwrap_or("httpd.zip").to_string();
        let archive = paths.downloads.join(&file_name);
        // Lounge may require a Referer — use a dedicated download for Windows
        if current_os() == "windows" && url.contains("apachelounge.com") {
            progress(ProgressEvent {
                package_id: "apache".into(),
                version: meta.version.clone(),
                phase: "downloading".into(),
                downloaded: 0,
                total: None,
            });
            let resp = self
                .client
                .get(&url)
                .header("User-Agent", "DevStack/0.1 (package-manager)")
                .header("Referer", LOUNGE_VS17)
                .send()
                .await?
                .error_for_status()?;
            let total = resp.content_length();
            let bytes = resp.bytes().await?;
            if let Some(parent) = archive.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&archive, &bytes)?;
            progress(ProgressEvent {
                package_id: "apache".into(),
                version: meta.version.clone(),
                phase: "downloading".into(),
                downloaded: bytes.len() as u64,
                total,
            });
            if let Some(expected) = &meta.sha256 {
                let got = crate::downloads::sha256_file(&archive)?;
                if !got.eq_ignore_ascii_case(expected) {
                    return Err(DevStackError::Message(format!(
                        "Apache SHA-256 mismatch: expected {expected}, got {got}"
                    )));
                }
            }
        } else {
            download_and_verify(
                &self.client,
                &url,
                &archive,
                meta.sha256.as_deref(),
                "apache",
                &meta.version,
                &progress,
            )
            .await?;
        }

        progress(ProgressEvent {
            package_id: "apache".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("apache", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        let kind = ArchiveKind::from_name(&file_name)?;
        extract_archive(&archive, &dest, kind, 0)?;
        Self::flatten_apache_tree(&dest)?;
        let _ = fs::remove_file(&archive);

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest.clone(),
            installed_at: now_iso(),
        };
        mark_installed(state, "apache", record.clone())?;
        state.stack.apache_version = Some(meta.version.clone());
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "apache".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "apache".into(),
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
        default_uninstall("apache", version, paths, state).await
    }
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(a).cmp(&parse(b))
}
