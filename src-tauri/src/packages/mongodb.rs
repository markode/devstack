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
            .await
            .map_err(|e| {
                DevStackError::Message(format!("Failed to fetch MongoDB versions: {e}"))
            })?
            .error_for_status()
            .map_err(|e| {
                DevStackError::Message(format!("Failed to fetch MongoDB versions: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                DevStackError::Message(format!("Failed to parse MongoDB version list: {e}"))
            })?;

        let out = versions_from_current(
            &data.versions,
            30,
            |downloads| Self::prefer_download(downloads),
        );
        if out.is_empty() {
            return Err(DevStackError::Message(
                "No MongoDB versions available for this platform (downloads.mongodb.org/current.json)"
                    .into(),
            ));
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

/// Convert `current.json` version entries into UI rows, skipping pure development builds.
fn versions_from_current(
    versions: &[MongoVersion],
    limit: usize,
    prefer: impl Fn(&[Value]) -> Option<(String, Option<String>, String)>,
) -> Vec<PackageVersion> {
    let mut out = Vec::new();
    for v in versions.iter().take(limit) {
        if v.development_release && !v.production_release {
            continue;
        }
        let channel = v
            .version
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".");
        let (download_url, sha256, note) = prefer(&v.downloads)
            .map(|(u, s, n)| (Some(u), s, Some(n)))
            .unwrap_or((None, None, None));

        out.push(PackageVersion {
            label: v.version.clone(),
            version: v.version.clone(),
            channel,
            lts: v.production_release,
            download_url,
            sha256,
            notes: note,
        });
    }
    out
}

/// True when a MongoDB `current.json` entry should be shown in the UI.
fn is_listable_mongo_version(v: &MongoVersion) -> bool {
    !(v.development_release && !v.production_release)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ver(
        version: &str,
        production: bool,
        development: bool,
        downloads: Vec<Value>,
    ) -> MongoVersion {
        MongoVersion {
            version: version.into(),
            production_release: production,
            development_release: development,
            downloads,
        }
    }

    #[test]
    fn skips_development_only_builds() {
        let versions = vec![
            ver("8.0.4", true, false, vec![]),
            ver("8.1.0-rc0", false, true, vec![]),
            ver("7.0.16", true, false, vec![]),
        ];
        assert!(is_listable_mongo_version(&versions[0]));
        assert!(!is_listable_mongo_version(&versions[1]));
        let out = versions_from_current(&versions, 30, |_| None);
        assert_eq!(
            out.iter().map(|v| v.version.as_str()).collect::<Vec<_>>(),
            vec!["8.0.4", "7.0.16"]
        );
        assert!(out.iter().all(|v| v.channel.len() >= 3));
    }

    #[test]
    fn prefer_callback_sets_download_url() {
        let versions = vec![ver(
            "8.0.4",
            true,
            false,
            vec![json!({"edition": "base"})],
        )];
        let out = versions_from_current(&versions, 10, |_| {
            Some((
                "https://example/mongodb.tgz".into(),
                Some("deadbeef".into()),
                "ubuntu2404".into(),
            ))
        });
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].download_url.as_deref(),
            Some("https://example/mongodb.tgz")
        );
        assert_eq!(out[0].sha256.as_deref(), Some("deadbeef"));
        assert_eq!(out[0].notes.as_deref(), Some("ubuntu2404"));
        assert!(out[0].lts);
    }
}
