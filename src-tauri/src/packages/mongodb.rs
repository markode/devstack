//! MongoDB Community — versions + install from downloads.mongodb.org/current.json.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
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

const CURRENT_JSON: &str = "https://downloads.mongodb.org/current.json";

#[derive(Clone)]
pub struct MongoDbProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct MongoCurrent {
    versions: Vec<MongoVersion>,
}

#[derive(Debug, Deserialize)]
struct MongoVersion {
    version: String,
    #[serde(default)]
    production_release: bool,
    #[serde(default)]
    development_release: bool,
    #[serde(default)]
    downloads: Vec<Value>,
}

impl MongoDbProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn prefer_download(downloads: &[Value]) -> Option<(String, Option<String>, String)> {
        let os = current_os();
        let arch = match current_arch() {
            "x64" => "x86_64",
            "arm64" => "aarch64",
            other => other,
        };

        let mut scored: Vec<(i32, String, Option<String>, String)> = Vec::new();
        for d in downloads {
            let edition = d.get("edition").and_then(|v| v.as_str()).unwrap_or("");
            let edition_score = match edition {
                "targeted" | "base" | "" => 10,
                "enterprise" => 0,
                _ => 5,
            };
            let target = d
                .get("target")
                .or_else(|| d.get("os"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let d_arch = d.get("arch").and_then(|v| v.as_str()).unwrap_or("");
            if d_arch != arch && d_arch != current_arch() {
                continue;
            }

            let os_score = match os {
                "macos" if target.contains("macos") || target.contains("osx") => 20,
                "windows" if target.contains("windows") || target == "windows" => 20,
                "linux"
                    if target.contains("ubuntu2404")
                        || target.contains("ubuntu2204")
                        || target.contains("ubuntu")
                        || target.contains("debian")
                        || target.contains("amazon")
                        || target.contains("rhel")
                        || target == "linux" =>
                {
                    let mut s = 15;
                    if target.contains("ubuntu2404") {
                        s += 5;
                    }
                    s
                }
                _ => 0,
            };
            if os_score == 0 {
                continue;
            }

            let archive = d.get("archive")?;
            let url = archive.get("url")?.as_str()?.to_string();
            let sha = archive
                .get("sha256")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            scored.push((edition_score + os_score, url, sha, target.to_string()));
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .next()
            .map(|(_, url, sha, note)| (url, sha, note))
    }
}

#[async_trait]
impl PackageProvider for MongoDbProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "mongodb".into(),
            name: "MongoDB".into(),
            description: "MongoDB Community Server — install, start, stop.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let data: MongoCurrent = self
            .client
            .get(CURRENT_JSON)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut out = Vec::new();
        for v in data.versions.into_iter().take(30) {
            if v.development_release && !v.production_release {
                continue;
            }
            let channel = v
                .version
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            let (download_url, sha256, note) = Self::prefer_download(&v.downloads)
                .map(|(u, s, n)| (Some(u), s, Some(n)))
                .unwrap_or((None, None, None));

            out.push(PackageVersion {
                label: v.version.clone(),
                version: v.version,
                channel,
                lts: v.production_release,
                download_url,
                sha256,
                notes: note,
            });
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
            .ok_or_else(|| DevStackError::Message("No MongoDB binary for this OS/arch".into()))?;
        let file_name = url.rsplit('/').next().unwrap_or("mongodb.tgz").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            "mongodb",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "mongodb".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("mongodb", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        let kind = ArchiveKind::from_name(&file_name)?;
        extract_archive(&archive, &dest, kind, 1)?;
        let _ = fs::remove_file(&archive);

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "mongodb", record.clone())?;
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "mongodb".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "mongodb".into(),
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
        default_uninstall("mongodb", version, paths, state).await
    }
}
