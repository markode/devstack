//! MariaDB — versions from downloads.mariadb.org REST API + full binary install.
//!
//! MariaDB.org no longer publishes macOS binaries. On macOS/Linux the baked
//! `packages-macos-v1` manifest (CI-built relocatable MariaDB trees) is required
//! for one-click install — we do **not** substitute MySQL Community into the
//! MariaDB slot.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

use super::macos_defaults::{
    arch_aliases, os_aliases, DEFAULT_COMBINED_MANIFEST_URL,
};
use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_arch, current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent,
    ProgressFn,
};
use crate::error::{DevStackError, Result};
use crate::services::process::find_bin;

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

    fn manifest_url(_state: &AppState) -> Option<String> {
        std::env::var("DEVSTACK_MARIADB_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| Some(DEFAULT_COMBINED_MANIFEST_URL.to_string()))
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
        Ok(m.packages
            .into_iter()
            .filter(|p| {
                p.id
                    .as_deref()
                    .map(|id| id.eq_ignore_ascii_case("mariadb"))
                    .unwrap_or(false)
                    && p.os
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
                lts: true,
                download_url: Some(p.url),
                sha256: p.sha256,
                notes: p.notes,
            })
            .collect())
    }

    fn flatten_db_tree(dest: &Path) -> Result<()> {
        if find_bin(dest, &["mariadbd", "mysqld", "mysqld.exe", "mariadbd.exe"]).is_some() {
            return Ok(());
        }
        for entry in fs::read_dir(dest)?.flatten() {
            let p = entry.path();
            if p.is_dir()
                && find_bin(&p, &["mariadbd", "mysqld", "mysqld.exe", "mariadbd.exe"]).is_some()
            {
                for child in fs::read_dir(&p)?.flatten() {
                    let from = child.path();
                    let name = from.file_name().unwrap();
                    let to = dest.join(name);
                    if !to.exists() {
                        let _ = fs::rename(&from, &to);
                    }
                }
                let _ = fs::remove_dir_all(&p);
                break;
            }
        }
        if find_bin(dest, &["mariadbd", "mysqld", "mysqld.exe", "mariadbd.exe"]).is_none() {
            return Err(DevStackError::Message(format!(
                "mysqld/mariadbd not found after extract in {}",
                dest.display()
            )));
        }
        Ok(())
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
                    if (fos.eq_ignore_ascii_case("MacOS")
                        || fos.eq_ignore_ascii_case("Darwin")
                        || fos.eq_ignore_ascii_case("OSX")
                        || name.contains("macos")
                        || name.contains("osx")
                        || name.contains("darwin"))
                        && !name.contains("debug")
                        && (name.ends_with(".tar.gz")
                            || name.ends_with(".tgz")
                            || name.ends_with(".pkg")) =>
                {
                    score += 40;
                    if fcpu.contains(arch)
                        || name.contains(arch)
                        || (arch == "aarch64"
                            && (name.contains("arm64") || fcpu.contains("arm64")))
                        || (arch == "x86_64"
                            && (name.contains("x86_64") || name.contains("amd64")))
                    {
                        score += 20;
                    }
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

    async fn list_versions(&self, state: &AppState) -> Result<Vec<PackageVersion>> {
        if let Some(url) = Self::manifest_url(state) {
            match self.list_manifest(&url).await {
                Ok(listed) if !listed.is_empty() => return Ok(listed),
                Ok(_) => {
                    eprintln!("[mariadb] manifest {url} returned no packages for this OS/arch");
                }
                Err(e) => {
                    eprintln!("[mariadb] manifest {url} unavailable ({e})");
                }
            }
        }

        // MariaDB.org dropped macOS artifacts — without packages-macos-v1 there
        // is no legitimate one-click MariaDB for macOS (we refuse MySQL substitutes).
        if current_os() == "macos" {
            return Err(DevStackError::Message(
                "No MariaDB macOS packages available. MariaDB.org no longer ships macOS \
                 binaries — install via the DevStack packages-macos-v1 release (or set \
                 DEVSTACK_MARIADB_MANIFEST_URL to a manifest that includes real mariadbd trees)."
                    .into(),
            ));
        }

        let majors: MajorResponse = self
            .client
            .get(MAJOR_URL)
            .send()
            .await
            .map_err(|e| {
                DevStackError::Message(format!("Failed to fetch MariaDB versions: {e}"))
            })?
            .error_for_status()
            .map_err(|e| {
                DevStackError::Message(format!("Failed to fetch MariaDB versions: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                DevStackError::Message(format!("Failed to parse MariaDB version list: {e}"))
            })?;

        let mut out = Vec::new();
        for major in select_major_releases(&majors.major_releases).into_iter().take(12) {
            let url = format!("{MAJOR_URL}{}/", major.release_id);
            let detail: Value = match self.client.get(&url).send().await {
                Ok(resp) => match resp.error_for_status() {
                    Ok(ok) => match ok.json().await {
                        Ok(v) => v,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            if let Some(pkg) = package_from_major_detail(&major, &detail, |files| {
                Self::prefer_file(files)
            }) {
                // Skip rows with no URL — they render as "cannot be installed".
                if pkg.download_url.is_some() {
                    out.push(pkg);
                }
            }
        }
        if out.is_empty() {
            return Err(DevStackError::Message(
                "No MariaDB versions available (downloads.mariadb.org returned no usable releases)"
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
        // Official trees nest under one top folder; relocated packages may not.
        extract_archive(&archive, &dest, kind, 0)?;
        if find_bin(&dest, &["mariadbd", "mysqld", "mysqld.exe", "mariadbd.exe"]).is_none() {
            // Retry mental model: strip 1 by flattening first child dir.
            Self::flatten_db_tree(&dest)?;
        } else {
            let _ = Self::flatten_db_tree(&dest);
        }
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

fn select_major_releases(majors: &[MajorRelease]) -> Vec<&MajorRelease> {
    majors
        .iter()
        .filter(|m| {
            matches!(
                m.release_status.as_deref(),
                Some("Stable") | Some("RC") | Some("Preview")
            )
        })
        .collect()
}

/// Build a [`PackageVersion`] from one major-release detail payload.
fn package_from_major_detail(
    major: &MajorRelease,
    detail: &Value,
    prefer: impl Fn(&[Value]) -> Option<(String, Option<String>, String)>,
) -> Option<PackageVersion> {
    let releases = detail.get("releases")?.as_object()?;
    let mut versions: Vec<_> = releases.keys().cloned().collect();
    versions.sort_by(|a, b| cmp_ver(b, a));
    let latest = versions.first()?;
    let entry = &releases[latest];
    let notes = [major.release_status.clone(), major.release_support_type.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");

    let (download, sha, _name) = entry
        .get("files")
        .and_then(|f| f.as_array())
        .and_then(|files| prefer(files))
        .map(|(u, s, n)| (Some(u), s, n))
        .unwrap_or((None, None, String::new()));

    Some(PackageVersion {
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
    })
}

/// Pick the newest release id from a MariaDB major-detail JSON object.
fn latest_release_id(detail: &Value) -> Option<String> {
    let releases = detail.get("releases")?.as_object()?;
    let mut versions: Vec<_> = releases.keys().cloned().collect();
    versions.sort_by(|a, b| cmp_ver(b, a));
    versions.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selects_stable_rc_preview_only() {
        let majors = vec![
            MajorRelease {
                release_id: "11.4".into(),
                release_status: Some("Stable".into()),
                release_support_type: Some("Long Term Support".into()),
            },
            MajorRelease {
                release_id: "11.5".into(),
                release_status: Some("Old".into()),
                release_support_type: None,
            },
            MajorRelease {
                release_id: "11.6".into(),
                release_status: Some("RC".into()),
                release_support_type: None,
            },
        ];
        let selected: Vec<_> = select_major_releases(&majors)
            .into_iter()
            .map(|m| m.release_id.as_str())
            .collect();
        assert_eq!(selected, vec!["11.4", "11.6"]);
    }

    #[test]
    fn parses_latest_release_from_detail() {
        let detail = json!({
            "releases": {
                "11.4.3": { "files": [] },
                "11.4.5": { "files": [] },
                "11.4.4": { "files": [] }
            }
        });
        assert_eq!(latest_release_id(&detail).as_deref(), Some("11.4.5"));
    }

    #[test]
    fn builds_package_from_detail_with_preferred_file() {
        let major = MajorRelease {
            release_id: "11.4".into(),
            release_status: Some("Stable".into()),
            release_support_type: Some("Long Term Support".into()),
        };
        let detail = json!({
            "releases": {
                "11.4.5": {
                    "files": [
                        {
                            "file_name": "mariadb-11.4.5-linux.tar.gz",
                            "os": "Linux",
                            "cpu": "x86_64",
                            "file_download_url": "https://example/m.tar.gz",
                            "checksum": { "sha256sum": "abc" }
                        }
                    ]
                }
            }
        });
        let pkg = package_from_major_detail(&major, &detail, |files| {
            let f = &files[0];
            Some((
                f["file_download_url"].as_str().unwrap().into(),
                Some("abc".into()),
                f["file_name"].as_str().unwrap().into(),
            ))
        })
        .unwrap();
        assert_eq!(pkg.version, "11.4.5");
        assert_eq!(pkg.channel, "11.4");
        assert!(pkg.lts);
        assert_eq!(pkg.download_url.as_deref(), Some("https://example/m.tar.gz"));
    }
}
