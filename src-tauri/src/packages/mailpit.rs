//! Mailpit — catch-all SMTP + web UI (GitHub releases, SHA256 digests).

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

const RELEASES_API: &str =
    "https://api.github.com/repos/axllent/mailpit/releases?per_page=15";

#[derive(Clone)]
pub struct MailpitProvider {
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
    #[serde(default)]
    digest: Option<String>,
}

impl MailpitProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn asset_name() -> Result<&'static str> {
        match (current_os(), current_arch()) {
            ("linux", "x64") => Ok("mailpit-linux-amd64.tar.gz"),
            ("linux", "arm64") => Ok("mailpit-linux-arm64.tar.gz"),
            ("macos", "x64") => Ok("mailpit-darwin-amd64.tar.gz"),
            ("macos", "arm64") => Ok("mailpit-darwin-arm64.tar.gz"),
            ("windows", "x64") => Ok("mailpit-windows-amd64.zip"),
            ("windows", "arm64") => Ok("mailpit-windows-arm64.zip"),
            (os, arch) => Err(DevStackError::UnsupportedPlatform(format!("{os}/{arch}"))),
        }
    }
}

#[async_trait]
impl PackageProvider for MailpitProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "mailpit".into(),
            name: "Mailpit".into(),
            description: "Catch-all SMTP for local PHP mail + web UI on mailpit.test.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let want = Self::asset_name()?;
        let releases: Vec<GhRelease> = self
            .client
            .get(RELEASES_API)
            .header("User-Agent", "DevStack")
            .header("Accept", "application/vnd.github+json")
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
            let Some(asset) = r.assets.iter().find(|a| a.name == want) else {
                continue;
            };
            let version = r.tag_name.trim_start_matches('v').to_string();
            let sha256 = asset.digest.as_ref().and_then(|d| {
                d.strip_prefix("sha256:")
                    .map(|s| s.to_lowercase())
                    .or_else(|| {
                        if d.len() == 64 {
                            Some(d.to_lowercase())
                        } else {
                            None
                        }
                    })
            });
            out.push(PackageVersion {
                label: format!("Mailpit {version}"),
                version: version.clone(),
                channel: version.clone(),
                lts: true,
                download_url: Some(asset.browser_download_url.clone()),
                sha256,
                notes: Some(want.into()),
            });
            if out.len() >= 10 {
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
            .find(|v| v.version == version || v.channel == version || format!("v{version}") == v.version)
            .ok_or_else(|| DevStackError::VersionNotFound(version.into()))?;
        let url = meta
            .download_url
            .clone()
            .ok_or_else(|| DevStackError::Message("No Mailpit binary for this OS/arch".into()))?;
        let file_name = url.rsplit('/').next().unwrap_or("mailpit.tgz").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            "mailpit",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "mailpit".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("mailpit", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        let kind = ArchiveKind::from_name(&file_name)?;
        extract_archive(&archive, &dest, kind, 0)?;
        let _ = fs::remove_file(&archive);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["mailpit", "mailpit.exe"] {
                let p = dest.join(name);
                if p.exists() {
                    let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o755));
                }
            }
        }

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "mailpit", record.clone())?;
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "mailpit".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "mailpit".into(),
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
        default_uninstall("mailpit", version, paths, state).await
    }
}
