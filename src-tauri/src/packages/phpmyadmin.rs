//! phpMyAdmin — versions from phpmyadmin.net, zip + sha256 install.

use async_trait::async_trait;
use serde::Deserialize;
use std::fs;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{download_and_verify, extract_archive, fetch_text, ArchiveKind, ProgressEvent, ProgressFn};
use crate::error::{DevStackError, Result};

const VERSION_JSON: &str = "https://www.phpmyadmin.net/home_page/version.json";

#[derive(Clone)]
pub struct PhpMyAdminProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct PmaVersionJson {
    releases: Vec<PmaRelease>,
}

#[derive(Debug, Deserialize)]
struct PmaRelease {
    version: String,
    date: Option<String>,
    php_versions: Option<String>,
}

impl PhpMyAdminProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn artifact(version: &str) -> (String, String, String) {
        let name = format!("phpMyAdmin-{version}-all-languages.zip");
        let url = format!("https://files.phpmyadmin.net/phpMyAdmin/{version}/{name}");
        let sha_url = format!("{url}.sha256");
        (name, url, sha_url)
    }
}

#[async_trait]
impl PackageProvider for PhpMyAdminProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "phpmyadmin".into(),
            name: "phpMyAdmin".into(),
            description: "Web UI for MySQL/MariaDB with one-click updates.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let data: PmaVersionJson = self
            .client
            .get(VERSION_JSON)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(data
            .releases
            .into_iter()
            .map(|r| {
                let (_name, url, _) = Self::artifact(&r.version);
                let channel = r
                    .version
                    .split('.')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(".");
                PackageVersion {
                    label: r.version.clone(),
                    version: r.version,
                    channel,
                    lts: false,
                    download_url: Some(url),
                    sha256: None,
                    notes: r
                        .php_versions
                        .map(|p| format!("PHP {p}"))
                        .or(r.date.map(|d| format!("Released {d}"))),
                }
            })
            .collect())
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
            .find(|v| v.version == version)
            .ok_or_else(|| DevStackError::VersionNotFound(version.into()))?;

        let (name, url, sha_url) = Self::artifact(&meta.version);
        let sha_body = fetch_text(&self.client, &sha_url).await?;
        let sha = sha_body
            .split_whitespace()
            .next()
            .ok_or_else(|| DevStackError::Message("empty phpMyAdmin sha256 file".into()))?
            .to_string();

        let archive_path = paths.downloads.join(&name);
        download_and_verify(
            &self.client,
            &url,
            &archive_path,
            Some(&sha),
            "phpmyadmin",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "phpmyadmin".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("phpmyadmin", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        extract_archive(&archive_path, &dest, ArchiveKind::Zip, 1)?;
        let _ = fs::remove_file(&archive_path);

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "phpmyadmin", record.clone())?;
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "phpmyadmin".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "phpmyadmin".into(),
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
        default_uninstall("phpmyadmin", version, paths, state).await
    }
}
