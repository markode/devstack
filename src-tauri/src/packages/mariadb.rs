//! MariaDB — versions from downloads.mariadb.org REST API + full binary install.

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

    fn prefer_file(files: &[Value]) -> Option<(String, Option<String>, String)> {
        let os = current_os();
        let arch = match current_arch() {
            "x64" => "x86_64",
            "arm64" => "aarch64",
            other => other,
        };
        let mut scored = Vec::new();
        for file in files {
            let name = file.get("file_name")?.as_str()?;
            let fos = file.get("os").and_then(|v| v.as_str()).unwrap_or("");
            let fcpu = file.get("cpu").and_then(|v| v.as_str()).unwrap_or("");
            let url = file
                .get("file_download_url")?
                .as_str()?
                .replace("http://", "https://");
            let sha = file
                .pointer("/checksum/sha256sum")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let mut score = 0;
            match os {
                "windows" if fos.eq_ignore_ascii_case("Windows") && name.ends_with(".zip") => {
                    score += 50;
                }
                "linux"
                    if fos.eq_ignore_ascii_case("Linux")
                        && name.ends_with(".tar.gz")
                        && !name.contains("debug")
                        && fcpu.contains(arch) =>
                {
                    score += 40;
                    if name.contains("systemd") {
                        score += 5;
                    }
                }
                "macos"
                    if (fos.eq_ignore_ascii_case("MacOS") || fos.eq_ignore_ascii_case("Darwin"))
                        && !name.contains("debug") =>
                {
                    score += 40;
                }
                _ => {}
            }
            if score > 0 {
                scored.push((score, url, sha, name.to_string()));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored
            .into_iter()
            .next()
            .map(|(_, url, sha, name)| (url, sha, name))
    }
}

#[async_trait]
impl PackageProvider for MariaDbProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "mariadb".into(),
            name: "MariaDB".into(),
            description: "MariaDB Server — install, start, stop via DevStack.".into(),
            category: "service".into(),
            install_ready: true,
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

            let (download, sha, _name) = entry
                .get("files")
                .and_then(|f| f.as_array())
                .and_then(|files| Self::prefer_file(files))
                .map(|(u, s, n)| (Some(u), s, n))
                .unwrap_or((None, None, String::new()));

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
            .ok_or_else(|| DevStackError::Message("No MariaDB binary for this OS/arch".into()))?;
        let file_name = url.rsplit('/').next().unwrap_or("mariadb.tgz").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            "mariadb",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "mariadb".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("mariadb", &meta.version);
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
        mark_installed(state, "mariadb", record.clone())?;
        state.stack.mariadb_version = Some(meta.version.clone());
        state.active_database = Some("mariadb".into());
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "mariadb".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "mariadb".into(),
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
