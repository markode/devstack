//! Nginx package provider — official Windows builds; pluggable static for macOS/Linux.

use async_trait::async_trait;
use serde::Deserialize;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent, ProgressFn,
};
use crate::error::{DevStackError, Result};

#[derive(Clone)]
pub struct NginxProvider {
    client: reqwest::Client,
}

impl NginxProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn manifest_url(state: &AppState) -> Option<String> {
        std::env::var("DEVSTACK_NGINX_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| state.nginx_manifest_url.clone())
    }

    async fn list_official(&self) -> Result<Vec<PackageVersion>> {
        let os = current_os();
        if os == "linux" {
            let html = self
                .client
                .get("https://nginx.org/packages/ubuntu/pool/nginx/n/nginx/")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            let re = regex::Regex::new(r"nginx_(\d+\.\d+\.\d+)-1~noble_amd64\.deb").unwrap();
            let mut versions: Vec<String> = re
                .captures_iter(&html)
                .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                .collect();
            versions.sort_by(|a, b| cmp_ver(b, a));
            versions.dedup();
            versions.truncate(12);
            return Ok(versions
                .into_iter()
                .map(|version| PackageVersion {
                    channel: version.split('.').take(2).collect::<Vec<_>>().join("."),
                    label: version.clone(),
                    download_url: Some(format!(
                        "https://nginx.org/packages/ubuntu/pool/nginx/n/nginx/nginx_{version}-1~noble_amd64.deb"
                    )),
                    version,
                    lts: false,
                    sha256: None,
                    notes: Some("Official nginx.org Ubuntu noble amd64 .deb".into()),
                })
                .collect());
        }

        let html = self
            .client
            .get("https://nginx.org/en/download.html")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let re = regex::Regex::new(r"nginx-(\d+\.\d+\.\d+)").unwrap();
        let mut versions: Vec<String> = re
            .captures_iter(&html)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect();
        versions.sort_by(|a, b| cmp_ver(b, a));
        versions.dedup();
        versions.truncate(12);

        Ok(versions
            .into_iter()
            .map(|version| {
                let channel = version.split('.').take(2).collect::<Vec<_>>().join(".");
                let (url, notes) = match os {
                    "windows" => (
                        Some(format!("https://nginx.org/download/nginx-{version}.zip")),
                        Some("Official Windows zip from nginx.org".into()),
                    ),
                    _ => (
                        None,
                        Some("Set DEVSTACK_NGINX_MANIFEST_URL for macOS static builds".into()),
                    ),
                };
                PackageVersion {
                    label: version.clone(),
                    version,
                    channel,
                    lts: false,
                    download_url: url,
                    sha256: None,
                    notes,
                }
            })
            .collect())
    }

    async fn list_manifest(&self, url: &str) -> Result<Vec<PackageVersion>> {
        #[derive(Deserialize)]
        struct Manifest {
            packages: Vec<ManPkg>,
        }
        #[derive(Deserialize)]
        struct ManPkg {
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
        let arch = std::env::consts::ARCH;
        Ok(m.packages
            .into_iter()
            .filter(|p| {
                p.os
                    .as_deref()
                    .map(|o| o.eq_ignore_ascii_case(os) || (os == "macos" && o == "darwin"))
                    .unwrap_or(true)
                    && p.arch
                        .as_deref()
                        .map(|a| a == arch || (arch == "x86_64" && a == "x64"))
                        .unwrap_or(true)
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
}

#[async_trait]
impl PackageProvider for NginxProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "nginx".into(),
            name: "Nginx".into(),
            description: "One Nginx for all sites — official / static builds.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, state: &AppState) -> Result<Vec<PackageVersion>> {
        if let Some(url) = Self::manifest_url(state) {
            return self.list_manifest(&url).await;
        }
        self.list_official().await
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
            .ok_or_else(|| DevStackError::Message("No Nginx download for this platform".into()))?;

        let file_name = url.rsplit('/').next().unwrap_or("nginx.bin").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            "nginx",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "nginx".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("nginx", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;

        if file_name.ends_with(".deb") {
            extract_deb(&archive, &dest)?;
        } else {
            let kind = ArchiveKind::from_name(&file_name)?;
            let strip = if matches!(kind, ArchiveKind::Zip) {
                1
            } else {
                1
            };
            extract_archive(&archive, &dest, kind, strip)?;
        }
        let _ = fs::remove_file(&archive);

        // If deb extracted to usr/sbin/nginx, flatten marker
        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest.clone(),
            installed_at: now_iso(),
        };
        mark_installed(state, "nginx", record.clone())?;
        state.stack.nginx_version = Some(meta.version.clone());
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "nginx".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "nginx".into(),
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
        default_uninstall("nginx", version, paths, state).await
    }
}

fn extract_deb(deb: &Path, dest: &Path) -> Result<()> {
    // Prefer `dpkg-deb -x` when available
    if which::which("dpkg-deb").is_ok() {
        let status = Command::new("dpkg-deb")
            .args(["-x", &deb.to_string_lossy(), &dest.to_string_lossy()])
            .status()
            .map_err(|e| DevStackError::Message(e.to_string()))?;
        if status.success() {
            return Ok(());
        }
    }
    // Fallback: ar x + tar
    let mut archive = ar::Archive::new(fs::File::open(deb)?);
    let staging = dest.join(".deb-staging");
    fs::create_dir_all(&staging)?;
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.map_err(|e| DevStackError::Archive(e.to_string()))?;
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if name.starts_with("data.tar") {
            let data_path = staging.join(&name);
            let mut out = fs::File::create(&data_path)?;
            std::io::copy(&mut entry, &mut out)?;
            if name.ends_with(".xz") {
                let file = fs::File::open(&data_path)?;
                let dec = xz2::read::XzDecoder::new(file);
                let mut tar = tar::Archive::new(dec);
                tar.unpack(dest)
                    .map_err(|e| DevStackError::Archive(e.to_string()))?;
            } else if name.ends_with(".gz") {
                let file = fs::File::open(&data_path)?;
                let dec = flate2::read::GzDecoder::new(file);
                let mut tar = tar::Archive::new(dec);
                tar.unpack(dest)
                    .map_err(|e| DevStackError::Archive(e.to_string()))?;
            }
        } else {
            let mut sink = Vec::new();
            let _ = entry.read_to_end(&mut sink);
        }
    }
    let _ = fs::remove_dir_all(staging);
    Ok(())
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(a).cmp(&parse(b))
}
