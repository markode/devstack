//! One-click search / cache / object-store services (v0.3.0):
//! Meilisearch, Typesense, MinIO (community fork) and Memcached (system binary).

use async_trait::async_trait;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_arch, current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent,
    ProgressFn,
};
use crate::error::{DevStackError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    Meilisearch,
    Typesense,
    Minio,
    Memcached,
}

impl ServiceKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Meilisearch => "meilisearch",
            Self::Typesense => "typesense",
            Self::Minio => "minio",
            Self::Memcached => "memcached",
        }
    }

    pub fn display(self) -> &'static str {
        match self {
            Self::Meilisearch => "Meilisearch",
            Self::Typesense => "Typesense",
            Self::Minio => "MinIO",
            Self::Memcached => "Memcached",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Meilisearch => "Typo-tolerant search engine (Laravel Scout, WordPress). HTTP API on :7700.",
            Self::Typesense => "Fast in-memory search engine (Laravel Scout). macOS/Linux builds.",
            Self::Minio => "S3-compatible object storage with web console (community fork — upstream stopped shipping binaries).",
            Self::Memcached => "In-memory cache. Prebuilt macOS/Linux packages when published; otherwise uses memcached on PATH (brew/apt optional fallback).",
        }
    }

    fn releases_repo(self) -> &'static str {
        match self {
            Self::Meilisearch => "meilisearch/meilisearch",
            Self::Typesense => "typesense/typesense",
            Self::Minio => "pgsty/minio",
            Self::Memcached => "",
        }
    }

    fn fallback_versions(self) -> &'static [&'static str] {
        match self {
            Self::Meilisearch => &["1.15.2", "1.12.8"],
            Self::Typesense => &["29.0", "27.1"],
            Self::Minio => &["2026-10-26T00-00-00Z"],
            Self::Memcached => &["1.6.38", "system"],
        }
    }

    /// Executable names to look for inside an install dir.
    pub fn bin_names(self) -> &'static [&'static str] {
        match self {
            Self::Meilisearch => &["meilisearch", "meilisearch.exe"],
            Self::Typesense => &["typesense-server", "typesense-server.exe"],
            Self::Minio => &["minio", "minio.exe", "silo", "silo.exe"],
            Self::Memcached => &["memcached", "memcached.exe"],
        }
    }
}

/// Map host OS/arch → the release asset (or URL) name used by each service.
pub fn meili_asset(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("linux", "x64") => "meilisearch-linux-amd64",
        ("linux", "arm64") => "meilisearch-linux-aarch64",
        ("macos", "x64") => "meilisearch-macos-amd64",
        ("macos", "arm64") => "meilisearch-macos-apple-silicon",
        ("windows", "x64") => "meilisearch-windows-amd64.exe",
        _ => return None,
    })
}

/// Typesense publishes tarballs at dl.typesense.org (linux / darwin only).
pub fn typesense_url(version: &str, os: &str, arch: &str) -> Option<String> {
    let (o, a) = match (os, arch) {
        ("linux", "x64") => ("linux", "amd64"),
        ("linux", "arm64") => ("linux", "arm64"),
        ("macos", "x64") => ("darwin", "amd64"),
        ("macos", "arm64") => ("darwin", "arm64"),
        _ => return None,
    };
    Some(format!(
        "https://dl.typesense.org/releases/{version}/typesense-server-{version}-{o}-{a}.tar.gz"
    ))
}

/// MinIO fork asset suffix for the host (`_linux_amd64.tar.gz`, `_windows_amd64.zip`, …).
pub fn minio_asset_suffix(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("linux", "x64") => "_linux_amd64.tar.gz",
        ("linux", "arm64") => "_linux_arm64.tar.gz",
        ("macos", "x64") => "_darwin_amd64.tar.gz",
        ("macos", "arm64") => "_darwin_arm64.tar.gz",
        ("windows", "x64") => "_windows_amd64.zip",
        ("windows", "arm64") => "_windows_arm64.zip",
        _ => return None,
    })
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

fn digest_sha(d: &Option<String>) -> Option<String> {
    d.as_ref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .map(|s| s.to_lowercase())
}

/// Parse `memcached 1.6.21` / `memcached 1.6.21\n` → `1.6.21`.
pub fn parse_memcached_version(out: &str) -> Option<String> {
    out.split_whitespace()
        .find(|t| t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .map(|s| s.to_string())
}

#[derive(Clone)]
pub struct ServiceBinProvider {
    kind: ServiceKind,
    client: reqwest::Client,
}

impl ServiceBinProvider {
    pub fn new(kind: ServiceKind, client: reqwest::Client) -> Self {
        Self { kind, client }
    }

    fn fallback(&self) -> Vec<PackageVersion> {
        self.kind
            .fallback_versions()
            .iter()
            .map(|v| PackageVersion {
                label: format!("{} {v}", self.kind.display()),
                version: (*v).to_string(),
                channel: (*v).to_string(),
                lts: true,
                download_url: self.direct_url(v),
                sha256: None,
                notes: Some("curated fallback".into()),
            })
            .collect()
    }

    fn direct_url(&self, version: &str) -> Option<String> {
        let (os, arch) = (current_os(), current_arch());
        match self.kind {
            ServiceKind::Meilisearch => meili_asset(os, arch).map(|a| {
                format!("https://github.com/meilisearch/meilisearch/releases/download/v{version}/{a}")
            }),
            ServiceKind::Typesense => typesense_url(version, os, arch),
            ServiceKind::Minio | ServiceKind::Memcached => None,
        }
    }

    async fn github_releases(&self) -> Result<Vec<GhRelease>> {
        let url = format!(
            "https://api.github.com/repos/{}/releases?per_page=15",
            self.kind.releases_repo()
        );
        let resp = self
            .client
            .get(url)
            .header("User-Agent", "DevStack")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    async fn list_remote(&self) -> Result<Vec<PackageVersion>> {
        let (os, arch) = (current_os(), current_arch());
        let releases = self.github_releases().await?;
        let mut out = Vec::new();
        for r in releases {
            if r.draft || r.prerelease {
                continue;
            }
            match self.kind {
                ServiceKind::Meilisearch => {
                    let Some(want) = meili_asset(os, arch) else { break };
                    let Some(a) = r.assets.iter().find(|a| a.name == want) else { continue };
                    let v = r.tag_name.trim_start_matches('v').to_string();
                    out.push(PackageVersion {
                        label: format!("Meilisearch {v}"),
                        version: v.clone(),
                        channel: v.split('.').take(2).collect::<Vec<_>>().join("."),
                        lts: true,
                        download_url: Some(a.browser_download_url.clone()),
                        sha256: digest_sha(&a.digest),
                        notes: Some(want.into()),
                    });
                }
                ServiceKind::Typesense => {
                    let Some(_) = typesense_url("0", os, arch) else { break };
                    let v = r.tag_name.trim_start_matches('v').to_string();
                    if v.contains('-') || !v.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        continue;
                    }
                    out.push(PackageVersion {
                        label: format!("Typesense {v}"),
                        version: v.clone(),
                        channel: v.clone(),
                        lts: true,
                        download_url: typesense_url(&v, os, arch),
                        sha256: None,
                        notes: Some("dl.typesense.org".into()),
                    });
                }
                ServiceKind::Minio => {
                    let Some(suffix) = minio_asset_suffix(os, arch) else { break };
                    let Some(a) = r
                        .assets
                        .iter()
                        .find(|a| a.name.ends_with(suffix) && !a.name.contains(".sbom"))
                    else {
                        continue;
                    };
                    let v = r.tag_name.trim_start_matches("RELEASE.").to_string();
                    out.push(PackageVersion {
                        label: format!("MinIO {v}"),
                        version: v.clone(),
                        channel: v.chars().take(7).collect(),
                        lts: true,
                        download_url: Some(a.browser_download_url.clone()),
                        sha256: digest_sha(&a.digest),
                        notes: Some(format!("{} (community fork)", a.name)),
                    });
                }
                ServiceKind::Memcached => break,
            }
            if out.len() >= 8 {
                break;
            }
        }
        Ok(out)
    }

    fn system_memcached() -> Option<(PathBuf, String)> {
        let bin = which::which("memcached").ok()?;
        let version = crate::services::process::command(&bin)
            .arg("-V")
            .output()
            .ok()
            .and_then(|o| parse_memcached_version(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_else(|| "system".into());
        Some((bin, version))
    }

    async fn install_memcached(
        &self,
        paths: &Paths,
        state: &mut AppState,
        progress: &ProgressFn,
    ) -> Result<InstalledPackage> {
        let (bin, version) = Self::system_memcached().ok_or_else(|| {
            DevStackError::Message(
                "memcached was not found on PATH. Install it first (macOS: `brew install memcached`, Debian/Ubuntu: `sudo apt install memcached`, Windows: `choco install memcached`) and retry.".into(),
            )
        })?;
        let dest = paths.package_dir("memcached", &version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        let name = bin.file_name().map(|n| n.to_owned()).unwrap_or_else(|| "memcached".into());
        let target = dest.join(&name);
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&bin, &target);
        }
        #[cfg(not(unix))]
        {
            fs::copy(&bin, &target)?;
        }
        finish_install(self.kind, &version, &version, dest, paths, state, progress)
    }
}

fn finish_install(
    kind: ServiceKind,
    version: &str,
    channel: &str,
    dest: PathBuf,
    paths: &Paths,
    state: &mut AppState,
    progress: &ProgressFn,
) -> Result<InstalledPackage> {
    let record = InstalledRecord {
        version: version.to_string(),
        channel: Some(channel.to_string()),
        path: dest,
        installed_at: now_iso(),
    };
    mark_installed(state, kind.id(), record.clone())?;
    save_state(&paths.state_file, state)?;
    progress(ProgressEvent {
        package_id: kind.id().into(),
        version: version.to_string(),
        phase: "done".into(),
        downloaded: 0,
        total: None,
    });
    Ok(InstalledPackage {
        package_id: kind.id().into(),
        version: record.version,
        channel: record.channel,
        path: record.path,
        installed_at: record.installed_at,
    })
}

fn chmod_exec(dir: &Path, names: &[&str]) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for n in names {
            let p = dir.join(n);
            if p.exists() {
                let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o755));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, names);
    }
}

impl ServiceBinProvider {
    async fn list_memcached(&self) -> Result<Vec<PackageVersion>> {
        // Prefer prebuilt packages from the combined macOS/Linux manifest.
        let mut out = Vec::new();
        let url = std::env::var("DEVSTACK_MEMCACHED_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                super::macos_defaults::DEFAULT_COMBINED_MANIFEST_URL.to_string()
            });
        if let Ok(listed) = self.list_from_combined_manifest(&url, "memcached").await {
            out.extend(listed);
        }
        if let Some((bin, v)) = Self::system_memcached() {
            out.push(PackageVersion {
                label: format!("Memcached {v} (system PATH)"),
                version: format!("system-{v}"),
                channel: "system".into(),
                lts: true,
                // Synthetic URL so Install is clickable; install_memcached handles "system*".
                download_url: Some(format!("devstack-system://memcached/{v}")),
                sha256: None,
                notes: Some(format!("fallback · {}", bin.display())),
            });
        }
        if out.is_empty() {
            out.push(PackageVersion {
                label: "Memcached (no prebuilt + not on PATH)".into(),
                version: "system".into(),
                channel: "system".into(),
                lts: true,
                download_url: None,
                sha256: None,
                notes: Some(
                    "No prebuilt memcached for this OS/arch and none on PATH. \
                     Install via brew/apt or publish packages-macos-v1 assets."
                        .into(),
                ),
            });
        }
        Ok(out)
    }

    async fn list_from_combined_manifest(
        &self,
        url: &str,
        id: &str,
    ) -> Result<Vec<PackageVersion>> {
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
        let arch = current_arch();
        Ok(m.packages
            .into_iter()
            .filter(|p| {
                p.id
                    .as_deref()
                    .map(|i| i.eq_ignore_ascii_case(id))
                    .unwrap_or(false)
            })
            .filter(|p| {
                let os_ok = p
                    .os
                    .as_deref()
                    .map(|o| {
                        o.eq_ignore_ascii_case(os)
                            || (os == "macos" && o.eq_ignore_ascii_case("darwin"))
                    })
                    .unwrap_or(true);
                let arch_ok = p.arch.as_deref().map(|a| {
                    a == arch
                        || (arch == "arm64" && matches!(a, "aarch64" | "arm64"))
                        || (arch == "x64" && matches!(a, "x86_64" | "x64" | "amd64"))
                }).unwrap_or(true);
                os_ok && arch_ok
            })
            .map(|p| PackageVersion {
                channel: p.channel.unwrap_or_else(|| p.version.clone()),
                label: format!("Memcached {}", p.version),
                version: p.version,
                lts: true,
                download_url: Some(p.url),
                sha256: p.sha256,
                notes: p.notes,
            })
            .collect())
    }
}

#[async_trait]
impl PackageProvider for ServiceBinProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: self.kind.id().into(),
            name: self.kind.display().into(),
            description: self.kind.description().into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        if self.kind == ServiceKind::Memcached {
            return self.list_memcached().await;
        }
        match self.list_remote().await {
            Ok(v) if !v.is_empty() => Ok(v),
            Ok(_) => Ok(self.fallback()),
            Err(e) => {
                eprintln!("[{}] releases lookup failed ({e}); using curated fallback", self.kind.id());
                Ok(self.fallback())
            }
        }
    }

    async fn install(
        &self,
        version: &str,
        paths: &Paths,
        state: &mut AppState,
        progress: ProgressFn,
    ) -> Result<InstalledPackage> {
        if self.kind == ServiceKind::Memcached {
            // Prebuilt download?
            let versions = self.list_versions(state).await.unwrap_or_default();
            if let Some(meta) = versions.iter().find(|v| v.version == version || v.channel == version)
            {
                if let Some(url) = &meta.download_url {
                    if url.starts_with("devstack-system://") || version.starts_with("system") {
                        return self.install_memcached(paths, state, &progress).await;
                    }
                    let file_name = url.rsplit('/').next().unwrap_or("memcached.bin").to_string();
                    let archive = paths.downloads.join(&file_name);
                    download_and_verify(
                        &self.client,
                        url,
                        &archive,
                        meta.sha256.as_deref(),
                        "memcached",
                        &meta.version,
                        &progress,
                    )
                    .await?;
                    let dest = paths.package_dir("memcached", &meta.version);
                    crate::config::remove_dir_if_exists(&dest)?;
                    fs::create_dir_all(&dest)?;
                    if file_name.ends_with(".tar.gz")
                        || file_name.ends_with(".tgz")
                        || file_name.ends_with(".zip")
                    {
                        let kind = ArchiveKind::from_name(&file_name)?;
                        extract_archive(&archive, &dest, kind, 0)?;
                    } else {
                        let target = dest.join("memcached");
                        fs::copy(&archive, &target)?;
                        chmod_exec(&dest, &["memcached"]);
                    }
                    let _ = fs::remove_file(&archive);
                    return finish_install(
                        self.kind,
                        &meta.version,
                        meta.channel.as_str(),
                        dest,
                        paths,
                        state,
                        &progress,
                    );
                }
            }
            return self.install_memcached(paths, state, &progress).await;
        }
        let versions = self.list_versions(state).await?;
        let meta = versions
            .into_iter()
            .find(|v| v.version == version || v.channel == version)
            .or_else(|| {
                self.direct_url(version).map(|url| PackageVersion {
                    label: format!("{} {version}", self.kind.display()),
                    version: version.to_string(),
                    channel: version.to_string(),
                    lts: true,
                    download_url: Some(url),
                    sha256: None,
                    notes: None,
                })
            })
            .ok_or_else(|| {
                DevStackError::UnsupportedPlatform(format!(
                    "{} has no build for {}/{}",
                    self.kind.display(),
                    current_os(),
                    current_arch()
                ))
            })?;
        let url = meta.download_url.clone().ok_or_else(|| {
            DevStackError::UnsupportedPlatform(format!(
                "{} has no build for {}/{}",
                self.kind.display(),
                current_os(),
                current_arch()
            ))
        })?;
        let file_name = url.rsplit('/').next().unwrap_or("download").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            self.kind.id(),
            &meta.version,
            &progress,
        )
        .await?;
        progress(ProgressEvent {
            package_id: self.kind.id().into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });
        let dest = paths.package_dir(self.kind.id(), &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        match ArchiveKind::from_name(&file_name) {
            Ok(kind) => {
                // Typesense / MinIO tarballs ship the binary at the archive root.
                extract_archive(&archive, &dest, kind, 0)?;
                let _ = fs::remove_file(&archive);
            }
            Err(_) => {
                // Raw executable (Meilisearch).
                let bin_name = if cfg!(windows) { "meilisearch.exe" } else { "meilisearch" };
                fs::rename(&archive, dest.join(bin_name)).or_else(|_| {
                    fs::copy(&archive, dest.join(bin_name)).map(|_| ())?;
                    let _ = fs::remove_file(&archive);
                    Ok::<(), std::io::Error>(())
                })?;
            }
        }
        chmod_exec(&dest, self.kind.bin_names());
        if crate::services::process::find_bin(&dest, self.kind.bin_names()).is_none() {
            return Err(DevStackError::Message(format!(
                "{} archive did not contain an executable",
                self.kind.display()
            )));
        }
        finish_install(self.kind, &meta.version, &meta.channel, dest, paths, state, &progress)
    }

    async fn uninstall(&self, version: &str, paths: &Paths, state: &mut AppState) -> Result<()> {
        default_uninstall(self.kind.id(), version, paths, state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meili_assets_per_platform() {
        assert_eq!(meili_asset("linux", "x64"), Some("meilisearch-linux-amd64"));
        assert_eq!(meili_asset("linux", "arm64"), Some("meilisearch-linux-aarch64"));
        assert_eq!(meili_asset("macos", "arm64"), Some("meilisearch-macos-apple-silicon"));
        assert_eq!(meili_asset("windows", "x64"), Some("meilisearch-windows-amd64.exe"));
        assert_eq!(meili_asset("windows", "arm64"), None);
    }

    #[test]
    fn typesense_url_shape_and_windows_unsupported() {
        assert_eq!(
            typesense_url("27.1", "linux", "x64").unwrap(),
            "https://dl.typesense.org/releases/27.1/typesense-server-27.1-linux-amd64.tar.gz"
        );
        assert!(typesense_url("27.1", "macos", "arm64").unwrap().ends_with("darwin-arm64.tar.gz"));
        assert!(typesense_url("27.1", "windows", "x64").is_none());
    }

    #[test]
    fn minio_suffixes() {
        assert_eq!(minio_asset_suffix("linux", "x64"), Some("_linux_amd64.tar.gz"));
        assert_eq!(minio_asset_suffix("windows", "x64"), Some("_windows_amd64.zip"));
    }

    #[test]
    fn memcached_version_parse() {
        assert_eq!(parse_memcached_version("memcached 1.6.21\n").as_deref(), Some("1.6.21"));
        assert_eq!(parse_memcached_version("garbage"), None);
    }

    #[test]
    fn provider_info_ids() {
        let c = reqwest::Client::new();
        for (k, id) in [
            (ServiceKind::Meilisearch, "meilisearch"),
            (ServiceKind::Typesense, "typesense"),
            (ServiceKind::Minio, "minio"),
            (ServiceKind::Memcached, "memcached"),
        ] {
            let p = ServiceBinProvider::new(k, c.clone());
            assert_eq!(p.info().id, id);
            assert_eq!(p.info().category, "service");
            assert!(!p.fallback().is_empty());
        }
    }
}
