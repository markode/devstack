//! Stale-while-revalidate disk cache for package version lists.
//!
//! Files live at `{data}/cache/versions/<id>.json` with a `fetched_at` stamp.
//! Callers return the cache immediately, then refresh in the background and
//! emit a `versions-updated` event when fresh data arrives.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{PackageRegistry, PackageVersion};
use crate::config::{AppState, Paths};
use crate::error::{DevStackError, Result};

const REFRESH_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedVersionList {
    pub package_id: String,
    pub fetched_at: String,
    pub fetched_at_unix: u64,
    pub versions: Vec<PackageVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionListResponse {
    pub package_id: String,
    pub versions: Vec<PackageVersion>,
    pub from_cache: bool,
    pub refreshing: bool,
    pub fetched_at: Option<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionsUpdatedEvent {
    pub package_id: String,
    pub versions: Vec<PackageVersion>,
    pub fetched_at: String,
    pub error: Option<String>,
}

pub fn versions_cache_dir(paths: &Paths) -> PathBuf {
    paths.root.join("cache").join("versions")
}

pub fn cache_path(paths: &Paths, package_id: &str) -> PathBuf {
    versions_cache_dir(paths).join(format!("{package_id}.json"))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn read_cache(paths: &Paths, package_id: &str) -> Option<CachedVersionList> {
    let path = cache_path(paths, package_id);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_cache(paths: &Paths, package_id: &str, versions: &[PackageVersion]) -> Result<CachedVersionList> {
    let dir = versions_cache_dir(paths);
    fs::create_dir_all(&dir)?;
    let entry = CachedVersionList {
        package_id: package_id.to_string(),
        fetched_at: now_iso(),
        fetched_at_unix: now_unix(),
        versions: versions.to_vec(),
    };
    let path = cache_path(paths, package_id);
    let raw = serde_json::to_string_pretty(&entry)?;
    fs::write(path, raw)?;
    Ok(entry)
}

/// Load cached versions if present (does not hit the network).
pub fn cached_response(paths: &Paths, package_id: &str) -> VersionListResponse {
    match read_cache(paths, package_id) {
        Some(c) if !c.versions.is_empty() => VersionListResponse {
            package_id: package_id.into(),
            versions: c.versions,
            from_cache: true,
            refreshing: true,
            fetched_at: Some(c.fetched_at),
            warning: None,
        },
        Some(c) => VersionListResponse {
            package_id: package_id.into(),
            versions: c.versions,
            from_cache: true,
            refreshing: true,
            fetched_at: Some(c.fetched_at),
            warning: Some("Cached version list is empty — refreshing…".into()),
        },
        None => VersionListResponse {
            package_id: package_id.into(),
            versions: Vec::new(),
            from_cache: false,
            refreshing: true,
            fetched_at: None,
            warning: None,
        },
    }
}

pub async fn fetch_and_store(
    registry: &PackageRegistry,
    paths: &Paths,
    state: &AppState,
    package_id: &str,
) -> Result<CachedVersionList> {
    let provider = registry.get(package_id)?;
    let versions = tokio::time::timeout(REFRESH_TIMEOUT, provider.list_versions(state))
        .await
        .map_err(|_| {
            DevStackError::Message(format!(
                "Timed out fetching {package_id} versions after {}s",
                REFRESH_TIMEOUT.as_secs()
            ))
        })??;
    write_cache(paths, package_id, &versions)
}

/// Track in-flight refreshes so we do not stampede the network.
#[derive(Default)]
pub struct RefreshGuard {
    inflight: Mutex<std::collections::HashSet<String>>,
}

impl RefreshGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn try_begin(&self, package_id: &str) -> bool {
        let mut g = self.inflight.lock().await;
        g.insert(package_id.to_string())
    }

    pub async fn end(&self, package_id: &str) {
        let mut g = self.inflight.lock().await;
        g.remove(package_id);
    }
}

pub fn all_package_ids(registry: &PackageRegistry) -> Vec<String> {
    registry
        .list_packages()
        .into_iter()
        .map(|p| p.id)
        .collect()
}

/// Pure helper for tests: decide whether a response should keep cache on error.
pub fn keep_cache_on_refresh_error(cached: Option<&CachedVersionList>, err: &str) -> VersionListResponse {
    match cached {
        Some(c) => VersionListResponse {
            package_id: c.package_id.clone(),
            versions: c.versions.clone(),
            from_cache: true,
            refreshing: false,
            fetched_at: Some(c.fetched_at.clone()),
            warning: Some(format!("Could not refresh versions — showing cached list. ({err})")),
        },
        None => VersionListResponse {
            package_id: String::new(),
            versions: Vec::new(),
            from_cache: false,
            refreshing: false,
            fetched_at: None,
            warning: Some(err.to_string()),
        },
    }
}

pub fn parse_cache_json(raw: &str) -> Result<CachedVersionList> {
    serde_json::from_str(raw).map_err(|e| DevStackError::Message(format!("bad cache json: {e}")))
}

#[allow(dead_code)]
pub fn ensure_cache_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packages::PackageVersion;

    fn sample_version(v: &str) -> PackageVersion {
        PackageVersion {
            version: v.into(),
            channel: v.split('.').take(2).collect::<Vec<_>>().join("."),
            label: v.into(),
            lts: false,
            download_url: Some(format!("https://example/{v}")),
            sha256: None,
            notes: None,
        }
    }

    #[test]
    fn cache_roundtrip_on_disk() {
        let dir = std::env::temp_dir().join(format!("devstack-vcache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("cache").join("versions")).unwrap();
        let paths = Paths {
            root: dir.clone(),
            packages: dir.join("packages"),
            downloads: dir.join("downloads"),
            state_file: dir.join("state.json"),
            run: dir.join("run"),
            logs: dir.join("logs"),
            nginx: dir.join("nginx"),
            php: dir.join("php"),
            certs: dir.join("certs"),
            databases: dir.join("databases"),
            sites_meta: dir.join("sites"),
            helper: dir.join("helper"),
            templates: dir.join("templates"),
            backups: dir.join("backups"),
            tools: dir.join("tools"),
            apache: dir.join("apache"),
        };
        let versions = vec![sample_version("8.4.1"), sample_version("8.3.0")];
        write_cache(&paths, "php", &versions).unwrap();
        let loaded = read_cache(&paths, "php").unwrap();
        assert_eq!(loaded.package_id, "php");
        assert_eq!(loaded.versions.len(), 2);
        assert_eq!(loaded.versions[0].version, "8.4.1");
        assert!(!loaded.fetched_at.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_cache_on_error_preserves_versions() {
        let cached = CachedVersionList {
            package_id: "node".into(),
            fetched_at: "2026-01-01T00:00:00Z".into(),
            fetched_at_unix: 1,
            versions: vec![sample_version("22.0.0")],
        };
        let resp = keep_cache_on_refresh_error(Some(&cached), "network down");
        assert!(resp.from_cache);
        assert_eq!(resp.versions.len(), 1);
        assert!(resp.warning.unwrap().contains("cached"));
    }

    #[test]
    fn parse_cache_json_rejects_garbage() {
        assert!(parse_cache_json("not-json").is_err());
    }
}
