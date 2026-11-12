//! Redis — official source tarballs (checksums from redis/redis-hashes) + build;
//! Windows uses tporadowski binary builds.

use async_trait::async_trait;
use regex::Regex;
use std::fs;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_os, download_and_verify, extract_archive, fetch_text, ArchiveKind, ProgressEvent,
    ProgressFn,
};
use crate::error::{DevStackError, Result};

const HASHES_URL: &str = "https://raw.githubusercontent.com/redis/redis-hashes/master/README";
const WIN_RELEASES: &str =
    "https://api.github.com/repos/tporadowski/redis/releases?per_page=15";

#[derive(Clone)]
pub struct RedisProvider {
    client: reqwest::Client,
}

impl RedisProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    async fn list_unix(&self) -> Result<Vec<PackageVersion>> {
        let body = fetch_text(&self.client, HASHES_URL).await?;
        let re = Regex::new(
            r#"hash\s+(redis-([0-9]+\.[0-9]+\.[0-9]+)\.tar\.gz)\s+sha256\s+([a-fA-F0-9]+)\s+(\S+)"#,
        )
        .map_err(|e| DevStackError::Message(e.to_string()))?;
        let mut out = Vec::new();
        for cap in re.captures_iter(&body) {
            let file = cap[1].to_string();
            let version = cap[2].to_string();
            let sha = cap[3].to_lowercase();
            let url = cap[4].replace("http://", "https://");
            let channel = version
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            out.push(PackageVersion {
                label: format!("Redis {version}"),
                version: version.clone(),
                channel,
                lts: version.starts_with('7') || version.starts_with('8'),
                download_url: Some(url),
                sha256: Some(sha),
                notes: Some(format!("source {file} — built locally")),
            });
        }
        // Newest first, but keep a few 7.x builds available for lean CI installs
        out.sort_by(|a, b| cmp_ver(&b.version, &a.version));
        let mut slim = Vec::new();
        let mut seen_major7 = 0;
        for v in out {
            let major7 = v.version.starts_with('7');
            if slim.len() < 12 || (major7 && seen_major7 < 4) {
                if major7 {
                    seen_major7 += 1;
                }
                slim.push(v);
            }
            if slim.len() >= 16 {
                break;
            }
        }
        Ok(slim)
    }

    fn windows_fallback_versions() -> Vec<PackageVersion> {
        // Curated tporadowski tags — used when GitHub API returns 429/5xx.
        const FALLBACK: &[&str] = &["5.0.14.1", "5.0.10"];
        FALLBACK
            .iter()
            .map(|version| {
                let url = format!(
                    "https://github.com/tporadowski/redis/releases/download/v{version}/Redis-x64-{version}.zip"
                );
                PackageVersion {
                    label: format!("Redis {version}"),
                    version: (*version).to_string(),
                    channel: version.split('.').take(2).collect::<Vec<_>>().join("."),
                    lts: true,
                    download_url: Some(url),
                    sha256: None,
                    notes: Some("tporadowski Windows build (fallback)".into()),
                }
            })
            .collect()
    }

    async fn list_windows(&self) -> Result<Vec<PackageVersion>> {
        #[derive(serde::Deserialize)]
        struct Rel {
            tag_name: String,
            assets: Vec<Asset>,
            #[serde(default)]
            draft: bool,
            #[serde(default)]
            prerelease: bool,
        }
        #[derive(serde::Deserialize)]
        struct Asset {
            name: String,
            browser_download_url: String,
        }
        let releases: Vec<Rel> = match self
            .client
            .get(WIN_RELEASES)
            .header("User-Agent", "DevStack")
            .send()
            .await
        {
            Ok(resp) => match resp.error_for_status() {
                Ok(ok) => ok.json().await.map_err(|e| {
                    DevStackError::Message(format!("Failed to parse Redis Windows releases: {e}"))
                })?,
                Err(e) => {
                    eprintln!("[redis] Windows releases API failed ({e}); using curated fallback");
                    return Ok(Self::windows_fallback_versions());
                }
            },
            Err(e) => {
                eprintln!("[redis] Windows releases API unreachable ({e}); using curated fallback");
                return Ok(Self::windows_fallback_versions());
            }
        };
        let mut out = Vec::new();
        for r in releases {
            if r.draft || r.prerelease {
                continue;
            }
            let Some(asset) = r
                .assets
                .iter()
                .find(|a| a.name.ends_with(".zip") && a.name.to_lowercase().contains("redis"))
            else {
                continue;
            };
            let version = r.tag_name.trim_start_matches('v').to_string();
            out.push(PackageVersion {
                label: format!("Redis {version}"),
                version: version.clone(),
                channel: version.split('.').take(2).collect::<Vec<_>>().join("."),
                lts: true,
                download_url: Some(asset.browser_download_url.clone()),
                sha256: None,
                notes: Some("tporadowski Windows build".into()),
            });
            if out.len() >= 10 {
                break;
            }
        }
        if out.is_empty() {
            return Ok(Self::windows_fallback_versions());
        }
        Ok(out)
    }

    async fn list_from_combined_manifest(&self) -> Result<Vec<PackageVersion>> {
        #[derive(serde::Deserialize)]
        struct Manifest {
            packages: Vec<ManPkg>,
        }
        #[derive(serde::Deserialize)]
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
        let url = std::env::var("DEVSTACK_REDIS_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                super::macos_defaults::DEFAULT_COMBINED_MANIFEST_URL.to_string()
            });
        let m: Manifest = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let os = current_os();
        let arch = std::env::consts::ARCH;
        Ok(m.packages
            .into_iter()
            .filter(|p| {
                p.id
                    .as_deref()
                    .map(|i| i.eq_ignore_ascii_case("redis"))
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
                        || (arch == "aarch64" && matches!(a, "arm64" | "aarch64"))
                        || (arch == "x86_64" && matches!(a, "x86_64" | "x64" | "amd64"))
                }).unwrap_or(true);
                os_ok && arch_ok
            })
            .map(|p| PackageVersion {
                channel: p.channel.unwrap_or_else(|| {
                    p.version.split('.').take(2).collect::<Vec<_>>().join(".")
                }),
                label: format!("Redis {}", p.version),
                version: p.version,
                lts: true,
                download_url: Some(p.url),
                sha256: p.sha256,
                notes: p.notes.or_else(|| Some("prebuilt (no local compile)".into())),
            })
            .collect())
    }
}

#[async_trait]
impl PackageProvider for RedisProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "redis".into(),
            name: "Redis".into(),
            description: "Redis in-memory store — install, start, stop.".into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        if current_os() == "windows" {
            return self.list_windows().await;
        }
        if let Ok(prebuilt) = self.list_from_combined_manifest().await {
            if !prebuilt.is_empty() {
                return Ok(prebuilt);
            }
        }
        self.list_unix().await
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
            .or_else(|| {
                if current_os() == "windows" {
                    Self::windows_fallback_versions()
                        .into_iter()
                        .find(|v| v.version == version || v.channel == version)
                        .or_else(|| {
                            Some(PackageVersion {
                                label: format!("Redis {version}"),
                                version: version.to_string(),
                                channel: version
                                    .split('.')
                                    .take(2)
                                    .collect::<Vec<_>>()
                                    .join("."),
                                lts: true,
                                download_url: Some(format!(
                                    "https://github.com/tporadowski/redis/releases/download/v{version}/Redis-x64-{version}.zip"
                                )),
                                sha256: None,
                                notes: Some("tporadowski Windows build (direct)".into()),
                            })
                        })
                } else {
                    None
                }
            })
            .ok_or_else(|| DevStackError::VersionNotFound(version.into()))?;
        let primary = meta
            .download_url
            .clone()
            .ok_or_else(|| DevStackError::Message("No Redis download URL".into()))?;
        // download.redis.io occasionally DNS-fails on CI — GitHub source tags are a reliable mirror.
        let github_mirror = if current_os() != "windows" {
            Some(format!(
                "https://github.com/redis/redis/archive/refs/tags/{}.tar.gz",
                meta.version
            ))
        } else {
            None
        };
        let mut urls: Vec<(String, Option<&str>)> = vec![(primary.clone(), meta.sha256.as_deref())];
        if let Some(m) = github_mirror {
            if m != primary {
                // GitHub archive checksum differs from redis-hashes — skip sha for mirror.
                urls.push((m, None));
            }
        }
        let mut archive = paths.downloads.join("redis-download.tgz");
        let mut last_err = None;
        let mut used_url = primary.clone();
        for (attempt, (url, sha)) in urls.into_iter().enumerate() {
            let file_name = url.rsplit('/').next().unwrap_or("redis.tgz").to_string();
            archive = paths.downloads.join(&file_name);
            match download_and_verify(
                &self.client,
                &url,
                &archive,
                sha,
                "redis",
                &meta.version,
                &progress,
            )
            .await
            {
                Ok(()) => {
                    used_url = url;
                    last_err = None;
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "[redis] download attempt {} failed for {url}: {e}",
                        attempt + 1
                    );
                    last_err = Some(e);
                    tokio::time::sleep(std::time::Duration::from_secs(2 * (attempt as u64 + 1))).await;
                }
            }
        }
        if let Some(e) = last_err {
            return Err(e);
        }
        let _ = used_url;
        let file_name = archive
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("redis.tgz")
            .to_string();

        progress(ProgressEvent {
            package_id: "redis".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("redis", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;
        let kind = ArchiveKind::from_name(&file_name)?;
        extract_archive(&archive, &dest, kind, if kind == ArchiveKind::Zip { 0 } else { 0 })?;
        let _ = fs::remove_file(&archive);

        // Flatten one top-level folder when present.
        if crate::services::process::find_bin(&dest, &["redis-server", "redis-server.exe"]).is_none()
        {
            for entry in fs::read_dir(&dest)?.flatten() {
                let p = entry.path();
                if p.is_dir()
                    && crate::services::process::find_bin(
                        &p,
                        &["redis-server", "redis-server.exe"],
                    )
                    .is_some()
                {
                    for child in fs::read_dir(&p)?.flatten() {
                        let from = child.path();
                        let to = dest.join(child.file_name());
                        if !to.exists() {
                            let _ = fs::rename(&from, &to);
                        }
                    }
                    let _ = fs::remove_dir_all(&p);
                    break;
                }
            }
        }

        let already_built = crate::services::process::find_bin(
            &dest,
            &["redis-server", "redis-server.exe"],
        )
        .is_some();

        if current_os() != "windows" && !already_built {
            progress(ProgressEvent {
                package_id: "redis".into(),
                version: meta.version.clone(),
                phase: "building".into(),
                downloaded: 0,
                total: None,
            });
            // Source tree may be nested; find Makefile
            let src = if dest.join("src").join("Makefile").exists() || dest.join("Makefile").exists()
            {
                dest.clone()
            } else {
                fs::read_dir(&dest)?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.join("Makefile").exists() || p.join("src").join("Makefile").exists())
                    .unwrap_or(dest.clone())
            };
            // Build core binaries only (skip Redis 8 modules that need extra deps).
            // MALLOC=libc avoids jemalloc LTO link failures on some Linux toolchains.
            let src_dir = if src.join("src").join("Makefile").exists() {
                src.join("src")
            } else {
                src.clone()
            };
            let status = crate::services::process::command("make")
                .args([
                    "-j",
                    "2",
                    "BUILD_TLS=no",
                    "MALLOC=libc",
                    "redis-server",
                    "redis-cli",
                ])
                .current_dir(&src_dir)
                .status()
                .map_err(|e| DevStackError::Message(format!("make redis: {e}")))?;
            if !status.success() {
                return Err(DevStackError::Message("redis make failed".into()));
            }
            // Ensure binaries live under dest/src/redis-server (default) or copy to dest/bin
            let bin_dir = dest.join("bin");
            fs::create_dir_all(&bin_dir)?;
            for name in ["redis-server", "redis-cli"] {
                let candidates = [
                    src.join("src").join(name),
                    src.join(name),
                    dest.join("src").join(name),
                ];
                if let Some(found) = candidates.into_iter().find(|p| p.exists()) {
                    let _ = fs::copy(&found, bin_dir.join(name));
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = fs::set_permissions(bin_dir.join(name), fs::Permissions::from_mode(0o755));
                    }
                }
            }
            if !bin_dir.join("redis-server").exists() {
                return Err(DevStackError::Message(
                    "redis-server binary missing after build".into(),
                ));
            }
        }

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest,
            installed_at: now_iso(),
        };
        mark_installed(state, "redis", record.clone())?;
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "redis".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "redis".into(),
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
        default_uninstall("redis", version, paths, state).await
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
