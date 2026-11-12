//! PHP package provider.
//!
//! - **Windows:** live NTS x64 builds from
//!   `https://downloads.php.net/~windows/releases/releases.json`
//! - **macOS / Linux:** prefer `DEVSTACK_PHP_MANIFEST_URL` /
//!   `AppState.php_manifest_url` / baked `packages-macos-v1` manifest, then
//!   auto-discover static-php.dev cli+fpm builds for the host arch.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::macos_defaults::{
    arch_aliases, arch_label, os_aliases, DEFAULT_COMBINED_MANIFEST_URL, PHP_CHANNELS,
    STATIC_PHP_COMMON,
};
use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent, ProgressFn,
};
use crate::error::{DevStackError, Result};
use crate::php::{write_dev_php_ini, PhpIniOptions};

const WINDOWS_RELEASES: &str = "https://downloads.php.net/~windows/releases/releases.json";
const WINDOWS_BASE: &str = "https://downloads.php.net/~windows/releases/";

#[derive(Clone)]
pub struct PhpProvider {
    client: reqwest::Client,
}

impl PhpProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn manifest_url(state: &AppState) -> Option<String> {
        std::env::var("DEVSTACK_PHP_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| state.php_manifest_url.clone())
            .or_else(|| Some(DEFAULT_COMBINED_MANIFEST_URL.to_string()))
    }

    async fn list_windows(&self) -> Result<Vec<PackageVersion>> {
        let map: BTreeMap<String, Value> = self
            .client
            .get(WINDOWS_RELEASES)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut out = Vec::new();
        for (channel, entry) in map.iter().rev() {
            let version = entry
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or(channel)
                .to_string();

            let nts = find_nts_x64(entry);
            let (url, sha) = if let Some((path, sha)) = nts {
                (Some(format!("{WINDOWS_BASE}{path}")), Some(sha))
            } else {
                (None, None)
            };

            out.push(PackageVersion {
                version: version.clone(),
                channel: channel.clone(),
                label: if version == *channel {
                    channel.clone()
                } else {
                    format!("{channel} ({version})")
                },
                lts: false,
                download_url: url,
                sha256: sha,
                notes: Some("Windows NTS x64".into()),
            });
        }
        out.sort_by(|a, b| b.channel.cmp(&a.channel));
        Ok(out)
    }

    async fn list_manifest(&self, url: &str) -> Result<Vec<PackageVersion>> {
        let body = self
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let manifest: PhpManifest = serde_json::from_str(&body).map_err(|e| {
            DevStackError::Message(format!("Failed to parse PHP manifest at {url}: {e}"))
        })?;

        let os = current_os();

        let mut out = Vec::new();
        for pkg in manifest.packages {
            let id_ok = pkg
                .id
                .as_deref()
                .map(|id| id.eq_ignore_ascii_case("php"))
                .unwrap_or(true);
            if !id_ok {
                continue;
            }
            let os_ok = pkg
                .os
                .as_deref()
                .map(|o| os_aliases(o, os))
                .unwrap_or(true);
            let arch_ok = pkg
                .arch
                .as_deref()
                .map(arch_aliases)
                .unwrap_or(true);
            if !os_ok || !arch_ok {
                continue;
            }
            let channel = pkg
                .channel
                .clone()
                .unwrap_or_else(|| major_minor(&pkg.version));
            out.push(PackageVersion {
                label: format!("{channel} ({})", pkg.version),
                version: pkg.version,
                channel,
                lts: pkg.lts.unwrap_or(false),
                download_url: Some(pkg.url),
                sha256: pkg.sha256,
                notes: pkg.notes.or_else(|| {
                    pkg.fpm_url
                        .map(|u| format!("fpm:{u}"))
                }),
            });
        }
        out.sort_by(|a, b| cmp_ver(&b.version, &a.version));
        Ok(out)
    }

    /// Scrape static-php.dev common builds for cli (+ matching fpm) on this arch.
    async fn list_static_php(&self) -> Result<Vec<PackageVersion>> {
        let html = self
            .client
            .get(STATIC_PHP_COMMON)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let arch = arch_label();
        let arch_alt = if arch == "aarch64" { "arm64" } else { arch };
        let cli_re = regex::Regex::new(&format!(
            r"php-(\d+\.\d+\.\d+)-cli-macos-({arch}|{arch_alt})\.tar\.gz"
        ))
        .map_err(|e| DevStackError::Message(e.to_string()))?;
        // Also match linux builds when on linux.
        let cli_re_linux = regex::Regex::new(&format!(
            r"php-(\d+\.\d+\.\d+)-cli-linux-({arch}|{arch_alt})\.tar\.gz"
        ))
        .map_err(|e| DevStackError::Message(e.to_string()))?;

        let os = current_os();
        let re = if os == "linux" {
            &cli_re_linux
        } else {
            &cli_re
        };

        let mut by_channel: BTreeMap<String, String> = BTreeMap::new();
        for cap in re.captures_iter(&html) {
            let ver = cap[1].to_string();
            let channel = major_minor(&ver);
            if !PHP_CHANNELS.iter().any(|c| *c == channel) {
                continue;
            }
            by_channel
                .entry(channel)
                .and_modify(|cur| {
                    if cmp_ver(&ver, cur) == std::cmp::Ordering::Greater {
                        *cur = ver.clone();
                    }
                })
                .or_insert(ver);
        }

        let os_token = if os == "linux" { "linux" } else { "macos" };
        let mut out = Vec::new();
        for channel in PHP_CHANNELS {
            let Some(version) = by_channel.get(*channel) else {
                continue;
            };
            // Prefer aarch64/x86_64 token that appears in the listing.
            let mut url = None;
            for a in [arch, arch_alt] {
                let candidate = format!("{STATIC_PHP_COMMON}php-{version}-cli-{os_token}-{a}.tar.gz");
                if html.contains(&format!("php-{version}-cli-{os_token}-{a}.tar.gz")) {
                    url = Some(candidate);
                    break;
                }
            }
            let Some(url) = url else { continue };
            let fpm_url = url.replace("-cli-", "-fpm-");
            out.push(PackageVersion {
                label: format!("{channel} ({version})"),
                version: version.clone(),
                channel: (*channel).to_string(),
                lts: matches!(*channel, "8.4" | "8.3" | "8.1"),
                download_url: Some(url),
                sha256: None,
                notes: Some(format!(
                    "static-php.dev cli+fpm · companion fpm:{fpm_url}"
                )),
            });
        }
        Ok(out)
    }

    fn merge_prefer_urls(primary: Vec<PackageVersion>, fallback: Vec<PackageVersion>) -> Vec<PackageVersion> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut out = Vec::new();
        for v in primary.into_iter().chain(fallback) {
            if v.download_url.is_none() {
                continue;
            }
            let key = v.channel.clone();
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);
            out.push(v);
        }
        out.sort_by(|a, b| cmp_ver(&b.version, &a.version));
        out
    }
}

#[derive(Debug, Deserialize)]
struct PhpManifest {
    #[serde(default)]
    packages: Vec<PhpManifestPackage>,
}

#[derive(Debug, Deserialize)]
struct PhpManifestPackage {
    #[serde(default)]
    id: Option<String>,
    version: String,
    channel: Option<String>,
    os: Option<String>,
    arch: Option<String>,
    url: String,
    #[serde(default)]
    fpm_url: Option<String>,
    sha256: Option<String>,
    lts: Option<bool>,
    notes: Option<String>,
    #[allow(dead_code)]
    format: Option<String>,
}

fn find_nts_x64(entry: &Value) -> Option<(String, String)> {
    let obj = entry.as_object()?;
    let preferred = ["nts-vs17-x64", "nts-vs16-x64", "nts-vc15-x64"];
    for key in preferred {
        if let Some(build) = obj.get(key) {
            if let Some(zip) = build.get("zip") {
                let path = zip.get("path")?.as_str()?.to_string();
                let sha = zip.get("sha256")?.as_str()?.to_string();
                return Some((path, sha));
            }
        }
    }
    for (key, build) in obj {
        if key.starts_with("nts-") && key.ends_with("-x64") {
            if let Some(zip) = build.get("zip") {
                let path = zip.get("path")?.as_str()?.to_string();
                let sha = zip.get("sha256")?.as_str()?.to_string();
                return Some((path, sha));
            }
        }
    }
    None
}

fn major_minor(version: &str) -> String {
    let parts: Vec<_> = version.split('.').collect();
    if parts.len() >= 2 {
        format!("{}.{}", parts[0], parts[1])
    } else {
        version.to_string()
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

fn companion_fpm_url(meta: &PackageVersion) -> Option<String> {
    if let Some(notes) = &meta.notes {
        if let Some(rest) = notes.split("fpm:").nth(1) {
            let url = rest.split_whitespace().next().unwrap_or(rest).trim();
            if url.starts_with("http") {
                return Some(url.to_string());
            }
        }
    }
    meta.download_url
        .as_ref()
        .map(|u| u.replace("-cli-", "-fpm-"))
}

fn place_binaries_from_extract(staging: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest.join("bin"))?;
    fs::create_dir_all(dest.join("sbin"))?;

    // Already a proper tree?
    if dest.join("bin/php").exists() || staging.join("bin/php").exists() {
        if staging.join("bin").exists() {
            copy_tree(staging, dest)?;
        }
        return Ok(());
    }

    // static-php single-file archives extract to `php` / `php-fpm` at root.
    for (name, sub) in [("php", "bin"), ("php-fpm", "sbin"), ("php-cgi", "bin")] {
        let candidates = [
            staging.join(name),
            dest.join(name),
            staging.join(sub).join(name),
        ];
        for c in candidates {
            if c.is_file() {
                let target = dest.join(sub).join(name);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                if c != target {
                    fs::copy(&c, &target)?;
                    let _ = fs::remove_file(&c);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
                }
                break;
            }
        }
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            fs::create_dir_all(&dst)?;
            copy_tree(&src, &dst)?;
        } else {
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[async_trait]
impl PackageProvider for PhpProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "php".into(),
            name: "PHP".into(),
            description: "Install and switch between PHP versions.".into(),
            category: "runtime".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, state: &AppState) -> Result<Vec<PackageVersion>> {
        match current_os() {
            "windows" => self.list_windows().await,
            _ => {
                let mut primary = Vec::new();
                if let Some(url) = Self::manifest_url(state) {
                    match self.list_manifest(&url).await {
                        Ok(v) if !v.is_empty() => primary = v,
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("[php] manifest {url} unavailable ({e}); trying static-php.dev");
                        }
                    }
                }
                // packages-macos-v1 (or DEVSTACK_PHP_MANIFEST_URL) is authoritative
                // when it lists builds. Merging static-php.dev on top lets a newer
                // cli-only channel (e.g. 8.5) win over our relocatable brew tree
                // that ships php-fpm — which breaks macOS managed smoke/E2E.
                let merged = if !primary.is_empty() {
                    let mut v = primary;
                    v.sort_by(|a, b| cmp_ver(&b.version, &a.version));
                    v
                } else {
                    let fallback = match self.list_static_php().await {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("[php] static-php discovery failed: {e}");
                            Vec::new()
                        }
                    };
                    Self::merge_prefer_urls(Vec::new(), fallback)
                };
                if merged.is_empty() {
                    return Err(DevStackError::Message(
                        "No PHP downloads available for this platform (packages manifest and static-php.dev both empty)."
                            .into(),
                    ));
                }
                // Fail loudly if any listed row lacks a URL (CI gate).
                if let Some(bad) = merged.iter().find(|v| v.download_url.is_none()) {
                    return Err(DevStackError::Message(format!(
                        "PHP {} has no download URL — refusing to list incomplete packages",
                        bad.version
                    )));
                }
                Ok(merged)
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
        let versions = self.list_versions(state).await?;
        let meta = versions
            .into_iter()
            .find(|v| v.version == version || v.channel == version)
            .ok_or_else(|| DevStackError::VersionNotFound(version.into()))?;

        let url = meta.download_url.clone().ok_or_else(|| {
            DevStackError::Message(format!(
                "No download URL for PHP {}. {}",
                meta.version,
                meta.notes.as_deref().unwrap_or("")
            ))
        })?;
        let sha = meta.sha256.clone();
        let fpm_url = companion_fpm_url(&meta);

        let file_name = url
            .rsplit('/')
            .next()
            .unwrap_or("php-archive.bin")
            .to_string();
        let archive_path = paths.downloads.join(&file_name);

        download_and_verify(
            &self.client,
            &url,
            &archive_path,
            sha.as_deref(),
            "php",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "php".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("php", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;

        let kind = ArchiveKind::from_name(&file_name)?;
        if current_os() == "windows" || matches!(kind, ArchiveKind::Zip) {
            // Windows NTS zip has php.exe at archive root — extract straight into dest.
            extract_archive(&archive_path, &dest, kind, 0)?;
            let _ = fs::remove_file(&archive_path);
        } else {
            let staging = dest.join(".extract-cli");
            fs::create_dir_all(&staging)?;
            extract_archive(&archive_path, &staging, kind, 0)?;
            place_binaries_from_extract(&staging, &dest)?;
            let _ = fs::remove_dir_all(&staging);
            let _ = fs::remove_file(&archive_path);

            // Companion FPM archive (static-php splits cli/fpm).
            if let Some(fpm) = fpm_url {
                if fpm != url {
                    let fpm_name = fpm.rsplit('/').next().unwrap_or("php-fpm.tgz").to_string();
                    let fpm_path = paths.downloads.join(&fpm_name);
                    match download_and_verify(
                        &self.client,
                        &fpm,
                        &fpm_path,
                        None,
                        "php",
                        &meta.version,
                        &progress,
                    )
                    .await
                    {
                        Ok(()) => {
                            let fpm_staging = dest.join(".extract-fpm");
                            fs::create_dir_all(&fpm_staging)?;
                            if let Ok(k) = ArchiveKind::from_name(&fpm_name) {
                                let _ = extract_archive(&fpm_path, &fpm_staging, k, 0);
                                let _ = place_binaries_from_extract(&fpm_staging, &dest);
                            }
                            let _ = fs::remove_dir_all(&fpm_staging);
                            let _ = fs::remove_file(&fpm_path);
                        }
                        Err(e) => {
                            eprintln!(
                                "[php] companion fpm download failed ({e}); continuing with cli only"
                            );
                        }
                    }
                }
            }

            // Flatten common `php-<ver>/` prefix trees into dest.
            if !dest.join("bin/php").exists() && !dest.join("php").exists() {
                if let Ok(entries) = fs::read_dir(&dest) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.is_dir() && (p.join("bin/php").exists() || p.join("php").exists()) {
                            place_binaries_from_extract(&p, &dest)?;
                            copy_tree(&p, &dest)?;
                            let _ = fs::remove_dir_all(&p);
                            break;
                        }
                    }
                }
            }

            if !dest.join("bin/php").exists() && dest.join("php").is_file() {
                fs::create_dir_all(dest.join("bin"))?;
                fs::rename(dest.join("php"), dest.join("bin/php"))?;
            }
            if !dest.join("sbin/php-fpm").exists() && dest.join("php-fpm").is_file() {
                fs::create_dir_all(dest.join("sbin"))?;
                fs::rename(dest.join("php-fpm"), dest.join("sbin/php-fpm"))?;
            }
        }

        if crate::services::process::find_bin(&dest, &["php", "php.exe"]).is_none() {
            return Err(DevStackError::Message(format!(
                "PHP {} extracted but no php binary found under {}",
                meta.version,
                dest.display()
            )));
        }

        write_dev_php_ini(&dest, &PhpIniOptions::default())?;

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest.clone(),
            installed_at: now_iso(),
        };
        mark_installed(state, "php", record.clone())?;
        state
            .php_extensions
            .entry(meta.version.clone())
            .or_default();
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "php".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "php".into(),
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
        default_uninstall("php", version, paths, state).await
    }
}

/// Resolve php.ini path inside an installed PHP tree.
pub fn php_ini_path(install_dir: &Path) -> std::path::PathBuf {
    let candidates = [
        install_dir.join("php.ini"),
        install_dir.join("etc/php.ini"),
        install_dir.join("conf/php.ini"),
    ];
    candidates
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| install_dir.join("php.ini"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_fpm_from_notes() {
        let meta = PackageVersion {
            version: "8.4.23".into(),
            channel: "8.4".into(),
            label: "8.4".into(),
            lts: true,
            download_url: Some(
                "https://dl.static-php.dev/static-php-cli/common/php-8.4.23-cli-macos-aarch64.tar.gz"
                    .into(),
            ),
            sha256: None,
            notes: Some(
                "static-php.dev cli+fpm · companion fpm:https://dl.static-php.dev/static-php-cli/common/php-8.4.23-fpm-macos-aarch64.tar.gz"
                    .into(),
            ),
        };
        let u = companion_fpm_url(&meta).unwrap();
        assert!(u.contains("-fpm-"));
        assert!(u.contains("8.4.23"));
    }
}
