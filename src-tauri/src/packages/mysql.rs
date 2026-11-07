//! MySQL Community Server — version list + binary install (Windows/macOS/Linux).
//!
//! Versions are discovered from official MySQL download/archive HTML when
//! reachable, then verified against `cdn.mysql.com` with HEAD requests.
//! A curated fallback list is used when the network fails. GitHub tags are
//! not used (the first page of tags is dominated by `mysql-cluster-*`).

use async_trait::async_trait;
use std::collections::BTreeSet;
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

const DOWNLOAD_PAGE: &str = "https://dev.mysql.com/downloads/mysql/";
const ARCHIVES_PAGE: &str = "https://downloads.mysql.com/archives/community/";

/// Recent GA versions used as CDN probe seeds and offline fallback.
/// Keep Innovation (9.x), 8.4 LTS, and 8.0 represented.
const FALLBACK_VERSIONS: &[&str] = &[
    "9.6.0", "9.5.0", "9.4.0", "9.3.0", "9.2.0", "9.1.0", "9.0.1", "8.4.13", "8.4.11",
    "8.4.10", "8.4.9", "8.4.8", "8.4.7", "8.4.6", "8.0.46", "8.0.45", "8.0.44", "8.0.43",
];

/// Channels to check for patches newer than the curated fallback list.
const PROBE_NEWER_CHANNELS: &[&str] = &["9.7", "9.6", "9.5", "8.4", "8.0"];

#[derive(Clone)]
pub struct MySqlProvider {
    client: reqwest::Client,
}

impl MySqlProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn major_minor(version: &str) -> String {
        version
            .split('.')
            .take(2)
            .collect::<Vec<_>>()
            .join(".")
    }

    fn is_lts(version: &str) -> bool {
        let channel = Self::major_minor(version);
        channel == "8.4" || channel == "8.0"
    }

    /// Prefer macos15 for recent GA builds; older builds shipped as macos14.
    fn macos_label(version: &str) -> &'static str {
        if cmp_ver(version, "9.2.0") != std::cmp::Ordering::Less {
            "macos15"
        } else if version.starts_with("9.") {
            "macos14"
        } else if version.starts_with("8.4.")
            && cmp_ver(version, "8.4.4") != std::cmp::Ordering::Less
        {
            "macos15"
        } else if version.starts_with("8.4.") {
            "macos14"
        } else if version.starts_with("8.0.")
            && cmp_ver(version, "8.0.41") != std::cmp::Ordering::Less
        {
            "macos15"
        } else {
            "macos14"
        }
    }

    /// Candidate artifact file names for the current (or given) platform.
    fn artifact_names(version: &str, os: &str, arch: &str) -> Result<Vec<(String, ArchiveKind)>> {
        match (os, arch) {
            ("windows", _) => Ok(vec![(
                format!("mysql-{version}-winx64.zip"),
                ArchiveKind::Zip,
            )]),
            ("linux", "x64") => Ok(vec![(
                format!("mysql-{version}-linux-glibc2.28-x86_64.tar.xz"),
                ArchiveKind::TarXz,
            )]),
            ("macos", "arm64") => {
                let preferred = Self::macos_label(version);
                let mut names = vec![format!("mysql-{version}-{preferred}-arm64.tar.gz")];
                let other = if preferred == "macos15" {
                    "macos14"
                } else {
                    "macos15"
                };
                names.push(format!("mysql-{version}-{other}-arm64.tar.gz"));
                Ok(names
                    .into_iter()
                    .map(|n| (n, ArchiveKind::TarGz))
                    .collect())
            }
            ("macos", "x64") => {
                let preferred = Self::macos_label(version);
                let mut names = vec![format!("mysql-{version}-{preferred}-x86_64.tar.gz")];
                let other = if preferred == "macos15" {
                    "macos14"
                } else {
                    "macos15"
                };
                names.push(format!("mysql-{version}-{other}-x86_64.tar.gz"));
                Ok(names
                    .into_iter()
                    .map(|n| (n, ArchiveKind::TarGz))
                    .collect())
            }
            _ => Err(DevStackError::UnsupportedPlatform(format!("{os}/{arch}"))),
        }
    }

    fn cdn_bases(version: &str) -> [String; 2] {
        let mm = Self::major_minor(version);
        [
            format!("https://cdn.mysql.com/Downloads/MySQL-{mm}"),
            format!("https://cdn.mysql.com/archives/mysql-{mm}"),
        ]
    }

    /// Best-effort artifact without network (offline fallback).
    fn artifact_for(version: &str) -> Result<(String, String, ArchiveKind)> {
        let os = current_os();
        let arch = current_arch();
        let names = Self::artifact_names(version, os, arch)?;
        let (name, kind) = names
            .into_iter()
            .next()
            .ok_or_else(|| DevStackError::Message("no MySQL artifact name".into()))?;
        let base = &Self::cdn_bases(version)[0];
        Ok((name.clone(), format!("{base}/{name}"), kind))
    }

    async fn url_exists(&self, url: &str) -> bool {
        match self
            .client
            .head(url)
            .header("User-Agent", concat!("DevStack/", env!("CARGO_PKG_VERSION")))
            .send()
            .await
        {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Resolve a live CDN URL for this version on the current platform.
    async fn resolve_artifact(
        &self,
        version: &str,
    ) -> Result<Option<(String, String, ArchiveKind)>> {
        let os = current_os();
        let arch = current_arch();
        let names = Self::artifact_names(version, os, arch)?;
        for base in Self::cdn_bases(version) {
            for (name, kind) in &names {
                let url = format!("{base}/{name}");
                if self.url_exists(&url).await {
                    return Ok(Some((name.clone(), url, kind.clone())));
                }
            }
        }
        Ok(None)
    }

    /// Fast existence probe using the Windows zip (always published for GA).
    async fn version_published(&self, version: &str) -> bool {
        let name = format!("mysql-{version}-winx64.zip");
        for base in Self::cdn_bases(version) {
            if self.url_exists(&format!("{base}/{name}")).await {
                return true;
            }
        }
        false
    }

    async fn fetch_page_versions(&self) -> Vec<String> {
        let mut out = BTreeSet::new();
        for url in [DOWNLOAD_PAGE, ARCHIVES_PAGE] {
            match self
                .client
                .get(url)
                .header(
                    "User-Agent",
                    concat!("DevStack/", env!("CARGO_PKG_VERSION")),
                )
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(html) = resp.text().await {
                        for v in parse_mysql_page_versions(&html) {
                            out.insert(v);
                        }
                    }
                }
                _ => {}
            }
        }
        out.into_iter().collect()
    }

    fn to_package_version(version: &str, url: String, name: &str) -> PackageVersion {
        let channel = Self::major_minor(version);
        PackageVersion {
            label: version.to_string(),
            version: version.to_string(),
            channel,
            lts: Self::is_lts(version),
            download_url: Some(url),
            sha256: None,
            notes: Some(format!("Artifact {name}")),
        }
    }

    fn fallback_list() -> Vec<PackageVersion> {
        FALLBACK_VERSIONS
            .iter()
            .filter_map(|version| {
                let (name, url, _) = Self::artifact_for(version).ok()?;
                Some(Self::to_package_version(version, url, &name))
            })
            .collect()
    }

    /// Curated seeds + download-page scrape + a short "anything newer?" CDN probe.
    async fn discover_published_versions(&self) -> Vec<String> {
        let mut set: BTreeSet<String> = BTreeSet::new();

        for v in FALLBACK_VERSIONS {
            set.insert((*v).to_string());
        }
        for v in self.fetch_page_versions().await {
            if community_version_ok(&v) {
                set.insert(v);
            }
        }

        // For each channel, walk a few patches above the highest seed we already
        // know so newly released GA builds appear without a wide CDN scan.
        for channel in PROBE_NEWER_CHANNELS {
            let highest = set
                .iter()
                .filter(|v| v.starts_with(&format!("{channel}.")))
                .max_by(|a, b| cmp_ver(a, b))
                .cloned();
            let start_patch = highest
                .as_deref()
                .and_then(|v| v.split('.').nth(2)?.parse::<u32>().ok())
                .map(|p| p + 1)
                .unwrap_or(0);
            for patch in start_patch..start_patch.saturating_add(4) {
                let version = format!("{channel}.{patch}");
                if self.version_published(&version).await {
                    set.insert(version);
                } else {
                    break;
                }
            }
        }

        // New Innovation minors often appear as N.0.0.
        if let Some(highest_minor) = set
            .iter()
            .filter_map(|v| {
                let mut p = v.split('.');
                (p.next() == Some("9"))
                    .then(|| p.next()?.parse::<u32>().ok())
                    .flatten()
            })
            .max()
        {
            for minor in (highest_minor + 1)..=(highest_minor + 2) {
                let version = format!("9.{minor}.0");
                if self.version_published(&version).await {
                    set.insert(version);
                } else {
                    break;
                }
            }
        }

        let mut out: Vec<_> = set.into_iter().collect();
        out.sort_by(|a, b| cmp_ver(b, a));
        out
    }
}

#[async_trait]
impl PackageProvider for MySqlProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "mysql".into(),
            name: "MySQL".into(),
            description: "MySQL Community Server — install, start, stop via DevStack.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        // Probe CDN first. On total network failure fall back to the curated list
        // so the UI never goes blank silently.
        let probe = self.client.head("https://cdn.mysql.com/").send().await;
        let cdn_reachable = matches!(&probe, Ok(r) if r.status().as_u16() < 500);

        if !cdn_reachable {
            let fb = Self::fallback_list();
            if fb.is_empty() {
                return Err(DevStackError::Message(
                    "Could not reach cdn.mysql.com and no MySQL fallback versions are available for this platform"
                        .into(),
                ));
            }
            return Ok(fb);
        }

        let candidates = self.discover_published_versions().await;
        let mut out = Vec::new();
        for version in candidates.into_iter().take(40) {
            if !community_version_ok(&version) {
                continue;
            }
            match self.resolve_artifact(&version).await {
                Ok(Some((name, url, _))) => {
                    out.push(Self::to_package_version(&version, url, &name));
                }
                Ok(None) => {
                    // Version exists on CDN for another platform, or naming differs —
                    // still offer a heuristic URL so the list is not blank.
                    if FALLBACK_VERSIONS.contains(&version.as_str())
                        || self.version_published(&version).await
                    {
                        if let Ok((name, url, _)) = Self::artifact_for(&version) {
                            out.push(Self::to_package_version(&version, url, &name));
                        }
                    }
                }
                Err(_) => {}
            }
            if out.len() >= 30 {
                break;
            }
        }

        if out.is_empty() {
            let fb = Self::fallback_list();
            if fb.is_empty() {
                return Err(DevStackError::Message(
                    "No MySQL Community Server downloads found for this platform".into(),
                ));
            }
            return Ok(fb);
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

        let (file_name, url, kind) = match self.resolve_artifact(&meta.version).await? {
            Some(v) => v,
            None => Self::artifact_for(&meta.version)?,
        };
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            None,
            "mysql",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "mysql".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("mysql", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        extract_archive(&archive, &dest, kind, 1)?;
        let _ = fs::remove_file(&archive);

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "mysql", record.clone())?;
        state.stack.mysql_version = Some(meta.version.clone());
        state.active_database = Some("mysql".into());
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "mysql".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "mysql".into(),
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
        default_uninstall("mysql", version, paths, state).await
    }
}

/// Strip a GitHub / mirror tag down to a Community Server version.
/// Rejects `mysql-cluster-*` and non-numeric suffixes after the `mysql-` prefix.
pub fn community_version_from_tag(tag: &str) -> Option<&str> {
    let version = tag.strip_prefix("mysql-")?;
    community_version_ok(version).then_some(version)
}

fn community_version_ok(version: &str) -> bool {
    if version.is_empty() || version.starts_with("cluster-") {
        return false;
    }
    let mut parts = version.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    if !major.chars().all(|c| c.is_ascii_digit()) || major.is_empty() {
        return false;
    }
    // Require at least major.minor.patch with digits only (optional trailing -label rejected).
    let rest: Vec<&str> = parts.collect();
    if rest.len() < 2 {
        return false;
    }
    rest.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Extract Community Server version numbers from MySQL download / archives HTML.
pub fn parse_mysql_page_versions(html: &str) -> Vec<String> {
    let mut set = BTreeSet::new();

    // option value="8.4.13" / selected values / plain text mysql-8.4.13-...
    let patterns = [
        regex::Regex::new(r#"(?i)(?:option[^>]*value|data-version|version)\s*=\s*["'](\d+\.\d+\.\d+)["']"#)
            .unwrap(),
        regex::Regex::new(r"(?i)mysql-(\d+\.\d+\.\d+)(?:-|$|[^0-9])").unwrap(),
        regex::Regex::new(r"(?i)MySQL(?:\s|&nbsp;)+(\d+\.\d+\.\d+)").unwrap(),
    ];
    for re in patterns {
        for cap in re.captures_iter(html) {
            if let Some(m) = cap.get(1) {
                let v = m.as_str();
                if community_version_ok(v) {
                    set.insert(v.to_string());
                }
            }
        }
    }

    let mut out: Vec<_> = set.into_iter().collect();
    out.sort_by(|a, b| cmp_ver(b, a));
    out
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split(|c| c == '.' || c == '-')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(a).cmp(&parse(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_cluster_tags() {
        assert_eq!(community_version_from_tag("mysql-cluster-8.0.36"), None);
        assert_eq!(community_version_from_tag("mysql-cluster-9.0.0"), None);
        assert_eq!(
            community_version_from_tag("mysql-8.4.13"),
            Some("8.4.13")
        );
        assert_eq!(community_version_from_tag("mysql-9.6.0"), Some("9.6.0"));
        assert_eq!(community_version_from_tag("v8.4.13"), None);
        assert_eq!(community_version_from_tag("mysql-boost-8.0.36"), None);
    }

    #[test]
    fn parses_download_page_versions() {
        let html = r#"
            <select name="version">
              <option value="9.6.0">9.6.0</option>
              <option value="8.4.13" selected>8.4.13</option>
              <option value="8.0.46">8.0.46</option>
            </select>
            <a href="/get/Downloads/MySQL-8.4/mysql-8.4.13-winx64.zip">win</a>
            <a href="/get/Downloads/MySQL-9.6/mysql-cluster-9.6.0-linux.tar.gz">cluster</a>
        "#;
        let versions = parse_mysql_page_versions(html);
        assert!(versions.contains(&"9.6.0".to_string()));
        assert!(versions.contains(&"8.4.13".to_string()));
        assert!(versions.contains(&"8.0.46".to_string()));
        // cluster file name still yields 9.6.0 via mysql-(\d+) — that's fine;
        // community_version_from_tag is what rejects cluster *tags*.
        assert!(!versions.iter().any(|v| v.contains("cluster")));
    }

    #[test]
    fn parses_archives_page_versions() {
        let html = r#"
            <form>
              <option value="8.4.6">8.4.6</option>
              <option value="8.0.36">8.0.36</option>
            </form>
            mysql-8.4.6-macos14-arm64.tar.gz
        "#;
        let versions = parse_mysql_page_versions(html);
        assert_eq!(versions[0], "8.4.6"); // newest first
        assert!(versions.contains(&"8.0.36".to_string()));
    }

    #[test]
    fn windows_artifact_is_winx64_zip() {
        let names = MySqlProvider::artifact_names("8.4.13", "windows", "x64").unwrap();
        assert_eq!(names[0].0, "mysql-8.4.13-winx64.zip");
    }

    #[test]
    fn macos_prefers_15_then_14_for_recent() {
        let names = MySqlProvider::artifact_names("9.6.0", "macos", "arm64").unwrap();
        assert_eq!(names[0].0, "mysql-9.6.0-macos15-arm64.tar.gz");
        assert_eq!(names[1].0, "mysql-9.6.0-macos14-arm64.tar.gz");

        let old = MySqlProvider::artifact_names("9.1.0", "macos", "x64").unwrap();
        assert_eq!(old[0].0, "mysql-9.1.0-macos14-x86_64.tar.gz");
        assert_eq!(old[1].0, "mysql-9.1.0-macos15-x86_64.tar.gz");
    }

    #[test]
    fn macos_label_transition_points() {
        assert_eq!(MySqlProvider::macos_label("9.2.0"), "macos15");
        assert_eq!(MySqlProvider::macos_label("9.1.0"), "macos14");
        assert_eq!(MySqlProvider::macos_label("8.4.4"), "macos15");
        assert_eq!(MySqlProvider::macos_label("8.4.3"), "macos14");
        assert_eq!(MySqlProvider::macos_label("8.0.41"), "macos15");
        assert_eq!(MySqlProvider::macos_label("8.0.40"), "macos14");
    }

    #[test]
    fn fallback_list_not_empty() {
        // artifact_for uses current OS — should succeed on CI linux/mac/windows.
        let list = MySqlProvider::fallback_list();
        assert!(
            !list.is_empty(),
            "fallback MySQL versions must never be empty"
        );
        assert!(list.iter().any(|v| v.version.starts_with("8.4.")));
        assert!(list.iter().any(|v| v.version.starts_with("8.0.")));
        assert!(list.iter().any(|v| v.version.starts_with("9.")));
        for v in &list {
            assert!(v.download_url.as_ref().unwrap().contains("cdn.mysql.com"));
            assert!(!v.version.contains("cluster"));
        }
    }
}
