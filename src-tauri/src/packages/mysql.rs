//! MySQL Community Server — version list + binary install (Windows/Linux).

use async_trait::async_trait;
use serde::Deserialize;
use std::fs;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_arch, current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent,
    ProgressFn,
};
use crate::error::{DevStackError, Result};

#[derive(Clone)]
pub struct MySqlProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct GhTag {
    name: String,
}

impl MySqlProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn artifact_for(version: &str) -> Result<(String, String, ArchiveKind)> {
        let os = current_os();
        let arch = current_arch();
        let major_minor = version
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".");
        match (os, arch) {
            ("windows", "x64") | ("windows", _) => Ok((
                format!("mysql-{version}-winx64.zip"),
                format!(
                    "https://cdn.mysql.com/Downloads/MySQL-{major_minor}/mysql-{version}-winx64.zip"
                ),
                ArchiveKind::Zip,
            )),
            ("linux", "x64") => Ok((
                format!("mysql-{version}-linux-glibc2.28-x86_64.tar.xz"),
                format!(
                    "https://cdn.mysql.com/Downloads/MySQL-{major_minor}/mysql-{version}-linux-glibc2.28-x86_64.tar.xz"
                ),
                ArchiveKind::TarXz,
            )),
            ("macos", "arm64") => Ok((
                format!("mysql-{version}-macos14-arm64.tar.gz"),
                format!(
                    "https://cdn.mysql.com/Downloads/MySQL-{major_minor}/mysql-{version}-macos14-arm64.tar.gz"
                ),
                ArchiveKind::TarGz,
            )),
            ("macos", "x64") => Ok((
                format!("mysql-{version}-macos14-x86_64.tar.gz"),
                format!(
                    "https://cdn.mysql.com/Downloads/MySQL-{major_minor}/mysql-{version}-macos14-x86_64.tar.gz"
                ),
                ArchiveKind::TarGz,
            )),
            _ => Err(DevStackError::UnsupportedPlatform(format!("{os}/{arch}"))),
        }
    }
}

#[async_trait]
impl PackageProvider for MySqlProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "mysql".into(),
            name: "MySQL".into(),
            description: "MySQL Community Server — install, start, stop via DevStack.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let tags: Vec<GhTag> = self
            .client
            .get("https://api.github.com/repos/mysql/mysql-server/tags?per_page=100")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut out = Vec::new();
        for tag in tags {
            let Some(version) = tag.name.strip_prefix("mysql-") else {
                continue;
            };
            if !version
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                continue;
            }
            let channel = version
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            let major = channel.split('.').next().unwrap_or("0");
            let (name, url, _) = match Self::artifact_for(version) {
                Ok(v) => v,
                Err(_) => continue,
            };
            out.push(PackageVersion {
                label: version.to_string(),
                version: version.to_string(),
                channel: channel.clone(),
                lts: (major == "8" && channel.ends_with(".4")) || channel == "8.0",
                download_url: Some(url),
                sha256: None,
                notes: Some(format!("Artifact {name}")),
            });
            if out.len() >= 30 {
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
        let (file_name, url, kind) = Self::artifact_for(&meta.version)?;
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            None,
            "mysql",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "mysql".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("mysql", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        extract_archive(&archive, &dest, kind, 1)?;
        let _ = fs::remove_file(&archive);

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "mysql", record.clone())?;
        state.stack.mysql_version = Some(meta.version.clone());
        state.active_database = Some("mysql".into());
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "mysql".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "mysql".into(),
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
        default_uninstall("mysql", version, paths, state).await
    }
}
