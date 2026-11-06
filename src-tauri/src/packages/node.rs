//! Node.js package provider — official dist index, no nvm.

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    download_and_verify, extract_archive, fetch_text, node_platform_triple, ProgressEvent,
    ProgressFn,
};
use crate::error::{DevStackError, Result};

const INDEX_URL: &str = "https://nodejs.org/dist/index.json";

#[derive(Clone)]
pub struct NodeProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct NodeIndexEntry {
    version: String,
    #[serde(default)]
    lts: serde_json::Value,
    files: Vec<String>,
}

impl NodeProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn archive_name(version: &str, os: &str, arch: &str, kind: crate::downloads::ArchiveKind) -> String {
        let ver = version.trim_start_matches('v');
        match kind {
            crate::downloads::ArchiveKind::Zip => format!("node-v{ver}-{os}-{arch}.zip"),
            crate::downloads::ArchiveKind::TarGz => format!("node-v{ver}-{os}-{arch}.tar.gz"),
            crate::downloads::ArchiveKind::TarXz => format!("node-v{ver}-{os}-{arch}.tar.xz"),
        }
    }

    fn file_token(os: &str, arch: &str, kind: crate::downloads::ArchiveKind) -> String {
        match (os, kind) {
            ("darwin", _) => format!("osx-{arch}-tar"),
            ("linux", crate::downloads::ArchiveKind::TarXz) | ("linux", _) => {
                format!("linux-{arch}")
            }
            ("win", _) => format!("win-{arch}-zip"),
            _ => format!("{os}-{arch}"),
        }
    }

    async fn sha256_for(
        &self,
        version: &str,
        file_name: &str,
    ) -> Result<String> {
        let ver = if version.starts_with('v') {
            version.to_string()
        } else {
            format!("v{version}")
        };
        let url = format!("https://nodejs.org/dist/{ver}/SHASUMS256.txt");
        let body = fetch_text(&self.client, &url).await?;
        for line in body.lines() {
            let mut parts = line.split_whitespace();
            let hash = parts.next();
            let name = parts.next();
            if let (Some(hash), Some(name)) = (hash, name) {
                if name == file_name || name.ends_with(file_name) {
                    return Ok(hash.to_string());
                }
            }
        }
        Err(DevStackError::Message(format!(
            "SHA256 not found for {file_name} in SHASUMS256.txt"
        )))
    }
}

#[async_trait]
impl PackageProvider for NodeProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "node".into(),
            name: "Node.js".into(),
            description: "Install and switch between Node.js versions (official builds).".into(),
            category: "runtime".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let (os, arch, kind) = node_platform_triple()?;
        let entries: Vec<NodeIndexEntry> = self
            .client
            .get(INDEX_URL)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let token = Self::file_token(os, arch, kind);
        let mut best: BTreeMap<u32, PackageVersion> = BTreeMap::new();

        for entry in entries {
            let ver = entry.version.trim_start_matches('v').to_string();
            let major: u32 = match ver.split('.').next().and_then(|s| s.parse().ok()) {
                Some(m) => m,
                None => continue,
            };
            // Skip odd non-LTS current lines older than needed? Keep latest per major.
            if !entry.files.iter().any(|f| f == &token || f.starts_with(&token)) {
                // windows zip token is exact; linux is "linux-x64"
                if !entry.files.iter().any(|f| f.contains(&format!("{os}-{arch}"))) {
                    continue;
                }
            }
            let is_lts = match &entry.lts {
                serde_json::Value::Bool(b) => *b,
                serde_json::Value::String(s) => !s.is_empty() && s != "false",
                _ => false,
            };
            let channel = major.to_string();
            let candidate = PackageVersion {
                label: format!("{channel} ({ver})"),
                version: ver.clone(),
                channel,
                lts: is_lts,
                download_url: Some(format!(
                    "https://nodejs.org/dist/v{ver}/{}",
                    Self::archive_name(&ver, os, arch, kind)
                )),
                sha256: None, // resolved at install time from SHASUMS256.txt
                notes: if is_lts {
                    Some("LTS".into())
                } else {
                    None
                },
            };
            best.entry(major)
                .and_modify(|existing| {
                    if ver_gt(&candidate.version, &existing.version) {
                        *existing = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }

        let mut out: Vec<_> = best.into_values().collect();
        out.sort_by(|a, b| b.channel.cmp(&a.channel));
        // Keep recent majors that matter for local AI/JS tooling
        out.retain(|v| {
            v.channel
                .parse::<u32>()
                .map(|m| m >= 16)
                .unwrap_or(false)
        });
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

        let (os, arch, kind) = node_platform_triple()?;
        let file_name = Self::archive_name(&meta.version, os, arch, kind);
        let url = format!("https://nodejs.org/dist/v{}/{file_name}", meta.version);
        let sha = self.sha256_for(&meta.version, &file_name).await?;

        let archive_path = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive_path,
            Some(&sha),
            "node",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "node".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("node", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        extract_archive(&archive_path, &dest, kind, 1)?;
        let _ = fs::remove_file(&archive_path);

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "node", record.clone())?;
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "node".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "node".into(),
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
        default_uninstall("node", version, paths, state).await
    }
}

fn ver_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(a) > parse(b)
}
