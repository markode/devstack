//! MariaDB — versions from downloads.mariadb.org REST API.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{
    default_uninstall, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{AppState, Paths};
use crate::downloads::ProgressFn;
use crate::error::{DevStackError, Result};

const MAJOR_URL: &str = "https://downloads.mariadb.org/rest-api/mariadb/";

#[derive(Clone)]
pub struct MariaDbProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct MajorResponse {
    major_releases: Vec<MajorRelease>,
}

#[derive(Debug, Deserialize)]
struct MajorRelease {
    release_id: String,
    release_status: Option<String>,
    release_support_type: Option<String>,
}

impl MariaDbProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PackageProvider for MariaDbProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "mariadb".into(),
            name: "MariaDB".into(),
            description: "MariaDB Server from downloads.mariadb.org.".into(),
            category: "service".into(),
            install_ready: false,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let majors: MajorResponse = self
            .client
            .get(MAJOR_URL)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut out = Vec::new();
        for major in majors
            .major_releases
            .into_iter()
            .filter(|m| {
                matches!(
                    m.release_status.as_deref(),
                    Some("Stable") | Some("RC") | Some("Preview")
                )
            })
            .take(12)
        {
            let url = format!("{MAJOR_URL}{}/", major.release_id);
            let detail: Value = self
                .client
                .get(&url)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            let Some(releases) = detail.get("releases").and_then(|v| v.as_object()) else {
                continue;
            };

            // Latest patch for this major line
            let mut versions: Vec<_> = releases.keys().cloned().collect();
            versions.sort_by(|a, b| cmp_ver(b, a));
            let Some(latest) = versions.first() else {
                continue;
            };
            let entry = &releases[latest];
            let notes = [
                major.release_status.clone(),
                major.release_support_type.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");

            // Prefer a binary package checksum when present
            let mut sha = None;
            let mut download = None;
            if let Some(files) = entry.get("files").and_then(|f| f.as_array()) {
                for file in files {
                    let os = file.get("os").and_then(|v| v.as_str()).unwrap_or("");
                    let name = file
                        .get("file_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let is_interesting = name.ends_with(".zip")
                        || name.ends_with(".tar.gz")
                        || os.eq_ignore_ascii_case("Linux")
                        || os.eq_ignore_ascii_case("Windows")
                        || os.eq_ignore_ascii_case("MacOS");
                    if !is_interesting {
                        continue;
                    }
                    if let Some(url) = file.get("file_download_url").and_then(|v| v.as_str()) {
                        download = Some(url.replace("http://", "https://"));
                    }
                    if let Some(sum) = file
                        .pointer("/checksum/sha256sum")
                        .and_then(|v| v.as_str())
                    {
                        sha = Some(sum.to_string());
                    }
                    if download.is_some() {
                        break;
                    }
                }
            }

            out.push(PackageVersion {
                label: format!("{} ({latest})", major.release_id),
                version: latest.clone(),
                channel: major.release_id.clone(),
                lts: major
                    .release_support_type
                    .as_deref()
                    .map(|s| s.contains("Long Term"))
                    .unwrap_or(false),
                download_url: download,
                sha256: sha,
                notes: if notes.is_empty() { None } else { Some(notes) },
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
            "MariaDB binary install lands in the next milestone (REST listing + checksums already wired)."
                .into(),
        ))
    }

    async fn uninstall(
        &self,
        version: &str,
        paths: &Paths,
        state: &mut AppState,
    ) -> Result<()> {
        default_uninstall("mariadb", version, paths, state).await
    }
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c| c == '.' || c == '-')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(a).cmp(&parse(b))
}
