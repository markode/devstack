//! MongoDB Community — versions from downloads.mongodb.org/current.json.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{
    default_uninstall, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{AppState, Paths};
use crate::downloads::{current_arch, current_os, ProgressFn};
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
            // Prefer community / targeted over enterprise
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
                    if target.contains("ubuntu")
                        || target.contains("debian")
                        || target.contains("amazon")
                        || target.contains("rhel")
                        || target == "linux" =>
                {
                    15
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
            description: "MongoDB Community Server.".into(),
            category: "service".into(),
            install_ready: false,
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
                notes: note.or_else(|| {
                    Some("Checksums from downloads.mongodb.org; install in next milestone.".into())
                }),
            });
        }
        Ok(out)
    }

    async fn install(
        &self,
        _version: &str,
        _paths: &Paths,
        _state: &mut AppState,
        _progress: ProgressFn,
    ) -> Result<InstalledPackage> {
        Err(DevStackError::Message(
            "MongoDB archive install lands in the next milestone (listing + SHA256 already available)."
                .into(),
        ))
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
