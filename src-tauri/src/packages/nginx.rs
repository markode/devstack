//! Nginx package provider — official Windows zips; source build on macOS;
//! Ubuntu .deb on Linux. Optional override via `DEVSTACK_NGINX_MANIFEST_URL`.

use async_trait::async_trait;
use serde::Deserialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    default_uninstall, now_iso, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{mark_installed, save_state, AppState, InstalledRecord, Paths};
use crate::downloads::{
    current_os, download_and_verify, extract_archive, ArchiveKind, ProgressEvent, ProgressFn,
};
use crate::error::{DevStackError, Result};

/// Known-good Windows builds when nginx.org HTML scrape fails (offline / layout change).
const WINDOWS_FALLBACK: &[&str] = &[
    "1.31.8", "1.30.6", "1.28.3", "1.26.3", "1.24.0", "1.22.1",
];

#[derive(Clone)]
pub struct NginxProvider {
    client: reqwest::Client,
}

impl NginxProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    fn manifest_url(state: &AppState) -> Option<String> {
        std::env::var("DEVSTACK_NGINX_MANIFEST_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| state.nginx_manifest_url.clone())
    }

    fn windows_zip_url(version: &str) -> String {
        format!("https://nginx.org/download/nginx-{version}.zip")
    }

    fn source_tar_url(version: &str) -> String {
        format!("https://nginx.org/download/nginx-{version}.tar.gz")
    }

    async fn list_official(&self) -> Result<Vec<PackageVersion>> {
        let os = current_os();
        if os == "linux" {
            return self.list_linux_debs().await;
        }

        // Prefer parsing real asset links from the download page so we only
        // offer versions that nginx.org actually publishes for this OS.
        match self.list_from_download_page(os).await {
            Ok(v) if !v.is_empty() => Ok(v),
            Ok(_) => Ok(Self::fallback_versions(os)),
            Err(err) => {
                let fb = Self::fallback_versions(os);
                if fb.is_empty() {
                    Err(err)
                } else {
                    Ok(fb)
                }
            }
        }
    }

    async fn list_linux_debs(&self) -> Result<Vec<PackageVersion>> {
        let html = self
            .client
            .get("https://nginx.org/packages/ubuntu/pool/nginx/n/nginx/")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let re = regex::Regex::new(r"nginx_(\d+\.\d+\.\d+)-1~noble_amd64\.deb").unwrap();
        let mut versions: Vec<String> = re
            .captures_iter(&html)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect();
        versions.sort_by(|a, b| cmp_ver(b, a));
        versions.dedup();
        versions.truncate(12);
        Ok(versions
            .into_iter()
            .map(|version| PackageVersion {
                channel: version.split('.').take(2).collect::<Vec<_>>().join("."),
                label: version.clone(),
                download_url: Some(format!(
                    "https://nginx.org/packages/ubuntu/pool/nginx/n/nginx/nginx_{version}-1~noble_amd64.deb"
                )),
                version,
                lts: false,
                sha256: None,
                notes: Some("Official nginx.org Ubuntu noble amd64 .deb".into()),
            })
            .collect())
    }

    async fn list_from_download_page(&self, os: &str) -> Result<Vec<PackageVersion>> {
        let html = self
            .client
            .get("https://nginx.org/en/download.html")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let mut versions = match os {
            "windows" => {
                // Only versions that have a Windows .zip on the page.
                let re = regex::Regex::new(
                    r#"(?i)href=["'](?:https://nginx\.org)?/download/nginx-(\d+\.\d+\.\d+)\.zip["']"#,
                )
                .unwrap();
                re.captures_iter(&html)
                    .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                    .collect::<Vec<_>>()
            }
            "macos" => {
                // macOS has no official binaries — offer source tarballs we compile on install.
                let re = regex::Regex::new(
                    r#"(?i)href=["'](?:https://nginx\.org)?/download/nginx-(\d+\.\d+\.\d+)\.tar\.gz["']"#,
                )
                .unwrap();
                re.captures_iter(&html)
                    .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                    .collect::<Vec<_>>()
            }
            _ => Vec::new(),
        };

        versions.sort_by(|a, b| cmp_ver(b, a));
        versions.dedup();
        versions.truncate(12);

        Ok(versions
            .into_iter()
            .map(|version| Self::version_entry(os, &version))
            .collect())
    }

    fn version_entry(os: &str, version: &str) -> PackageVersion {
        let channel = version.split('.').take(2).collect::<Vec<_>>().join(".");
        let (download_url, notes) = match os {
            "windows" => (
                Some(Self::windows_zip_url(version)),
                Some("Official Windows zip from nginx.org".into()),
            ),
            "macos" => (
                Some(Self::source_tar_url(version)),
                Some(
                    "Official source tarball — DevStack compiles nginx on install (needs Xcode CLT)"
                        .into(),
                ),
            ),
            _ => (None, Some(format!("No Nginx download configured for OS `{os}`"))),
        };
        PackageVersion {
            label: version.to_string(),
            version: version.to_string(),
            channel,
            lts: false,
            download_url,
            sha256: None,
            notes,
        }
    }

    fn fallback_versions(os: &str) -> Vec<PackageVersion> {
        match os {
            "windows" | "macos" => WINDOWS_FALLBACK
                .iter()
                .map(|v| Self::version_entry(os, v))
                .collect(),
            _ => Vec::new(),
        }
    }

    async fn list_manifest(&self, url: &str) -> Result<Vec<PackageVersion>> {
        #[derive(Deserialize)]
        struct Manifest {
            packages: Vec<ManPkg>,
        }
        #[derive(Deserialize)]
        struct ManPkg {
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
        let arch = std::env::consts::ARCH;
        let arch_aliases = |a: &str| {
            a == arch
                || (arch == "x86_64" && matches!(a, "x64" | "amd64" | "x86_64"))
                || (arch == "aarch64" && matches!(a, "arm64" | "aarch64"))
        };
        Ok(m.packages
            .into_iter()
            .filter(|p| {
                p.os
                    .as_deref()
                    .map(|o| {
                        o.eq_ignore_ascii_case(os)
                            || (os == "macos" && o.eq_ignore_ascii_case("darwin"))
                            || (os == "windows" && o.eq_ignore_ascii_case("win"))
                    })
                    .unwrap_or(true)
                    && p.arch.as_deref().map(arch_aliases).unwrap_or(true)
            })
            .map(|p| PackageVersion {
                channel: p.channel.unwrap_or_else(|| {
                    p.version.split('.').take(2).collect::<Vec<_>>().join(".")
                }),
                label: p.version.clone(),
                version: p.version,
                lts: false,
                download_url: Some(p.url),
                sha256: p.sha256,
                notes: p.notes,
            })
            .collect())
    }

    fn build_from_source(
        archive: &Path,
        dest: &Path,
        version: &str,
        progress: &ProgressFn,
    ) -> Result<()> {
        progress(ProgressEvent {
            package_id: "nginx".into(),
            version: version.into(),
            phase: "compiling".into(),
            downloaded: 0,
            total: None,
        });

        let staging = dest.join(".src-staging");
        crate::config::remove_dir_if_exists(&staging)?;
        fs::create_dir_all(&staging)?;
        extract_archive(archive, &staging, ArchiveKind::TarGz, 1)?;

        // Source lands either at staging/ or staging/nginx-VERSION/
        let src = if staging.join("configure").exists() {
            staging.clone()
        } else {
            fs::read_dir(&staging)?
                .flatten()
                .map(|e| e.path())
                .find(|p| p.join("configure").exists())
                .ok_or_else(|| {
                    DevStackError::Message(
                        "nginx source configure script not found after extract".into(),
                    )
                })?
        };

        let prefix = dest
            .canonicalize()
            .unwrap_or_else(|_| dest.to_path_buf());

        // Prefer Homebrew OpenSSL/PCRE when present (common on macOS).
        let mut args = vec![
            format!("--prefix={}", prefix.display()),
            "--with-http_ssl_module".into(),
            "--with-http_v2_module".into(),
            "--with-http_realip_module".into(),
            "--with-stream".into(),
        ];
        for brew_opt in [
            "/opt/homebrew/opt/openssl@3",
            "/opt/homebrew/opt/openssl",
            "/usr/local/opt/openssl@3",
            "/usr/local/opt/openssl",
        ] {
            let base = PathBuf::from(brew_opt);
            if base.join("include").is_dir() {
                args.push(format!("--with-cc-opt=-I{}/include", base.display()));
                args.push(format!("--with-ld-opt=-L{}/lib", base.display()));
                break;
            }
        }
        for pcre in [
            "/opt/homebrew/opt/pcre2",
            "/usr/local/opt/pcre2",
            "/opt/homebrew/opt/pcre",
            "/usr/local/opt/pcre",
        ] {
            let base = PathBuf::from(pcre);
            if base.join("include").is_dir() {
                let cc = args
                    .iter()
                    .find(|a| a.starts_with("--with-cc-opt="))
                    .cloned();
                let ld = args
                    .iter()
                    .find(|a| a.starts_with("--with-ld-opt="))
                    .cloned();
                if let Some(c) = cc {
                    args.retain(|a| !a.starts_with("--with-cc-opt="));
                    args.push(format!("{c} -I{}/include", base.display()));
                } else {
                    args.push(format!("--with-cc-opt=-I{}/include", base.display()));
                }
                if let Some(l) = ld {
                    args.retain(|a| !a.starts_with("--with-ld-opt="));
                    args.push(format!("{l} -L{}/lib", base.display()));
                } else {
                    args.push(format!("--with-ld-opt=-L{}/lib", base.display()));
                }
                break;
            }
        }

        let configure = Command::new("sh")
            .arg("./configure")
            .args(&args)
            .current_dir(&src)
            .output()
            .map_err(|e| {
                DevStackError::Message(format!(
                    "nginx ./configure failed to start ({e}). Install Xcode Command Line Tools."
                ))
            })?;
        if !configure.status.success() {
            return Err(DevStackError::Message(format!(
                "nginx ./configure failed:\n{}",
                String::from_utf8_lossy(&configure.stderr)
            )));
        }

        let jobs = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let make = Command::new("make")
            .arg(format!("-j{jobs}"))
            .current_dir(&src)
            .output()
            .map_err(|e| DevStackError::Message(format!("nginx make failed to start: {e}")))?;
        if !make.status.success() {
            return Err(DevStackError::Message(format!(
                "nginx make failed:\n{}",
                String::from_utf8_lossy(&make.stderr)
            )));
        }

        let install = Command::new("make")
            .arg("install")
            .current_dir(&src)
            .output()
            .map_err(|e| DevStackError::Message(format!("nginx make install: {e}")))?;
        if !install.status.success() {
            return Err(DevStackError::Message(format!(
                "nginx make install failed:\n{}",
                String::from_utf8_lossy(&install.stderr)
            )));
        }

        let _ = fs::remove_dir_all(&staging);
        Ok(())
    }
}

#[async_trait]
impl PackageProvider for NginxProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "nginx".into(),
            name: "Nginx".into(),
            description: "One Nginx for all sites — official Windows zip / source build on macOS."
                .into(),
            category: "service".into(),
            install_ready: true,
        }
    }

    async fn list_versions(&self, state: &AppState) -> Result<Vec<PackageVersion>> {
        if let Some(url) = Self::manifest_url(state) {
            let listed = self.list_manifest(&url).await?;
            if listed.is_empty() {
                return Err(DevStackError::Message(format!(
                    "Nginx manifest at {url} returned no packages for this OS/arch. Check os/arch filters or unset DEVSTACK_NGINX_MANIFEST_URL."
                )));
            }
            return Ok(listed);
        }
        let listed = self.list_official().await?;
        if listed.is_empty() {
            return Err(DevStackError::Message(format!(
                "No Nginx downloads found for {} (nginx.org returned no matching assets).",
                current_os()
            )));
        }
        Ok(listed)
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
                "No Nginx download URL for {} on {}. {}",
                meta.version,
                current_os(),
                meta.notes.unwrap_or_default()
            ))
        })?;

        let file_name = url.rsplit('/').next().unwrap_or("nginx.bin").to_string();
        let archive = paths.downloads.join(&file_name);
        download_and_verify(
            &self.client,
            &url,
            &archive,
            meta.sha256.as_deref(),
            "nginx",
            &meta.version,
            &progress,
        )
        .await?;

        progress(ProgressEvent {
            package_id: "nginx".into(),
            version: meta.version.clone(),
            phase: "extracting".into(),
            downloaded: 0,
            total: None,
        });

        let dest = paths.package_dir("nginx", &meta.version);
        crate::config::remove_dir_if_exists(&dest)?;
        fs::create_dir_all(&dest)?;

        let os = current_os();
        if file_name.ends_with(".deb") {
            extract_deb(&archive, &dest)?;
        } else if os == "macos"
            && (file_name.ends_with(".tar.gz") || file_name.ends_with(".tgz"))
        {
            Self::build_from_source(&archive, &dest, &meta.version, &progress)?;
        } else {
            let kind = ArchiveKind::from_name(&file_name)?;
            let strip = 1;
            extract_archive(&archive, &dest, kind, strip)?;
        }
        let _ = fs::remove_file(&archive);

        // Ensure we can find the binary after install
        let has_bin = find_nginx_bin(&dest).is_some();
        if !has_bin {
            return Err(DevStackError::Message(format!(
                "Nginx installed to {} but no nginx binary was found",
                dest.display()
            )));
        }

        let record = InstalledRecord {
            version: meta.version.clone(),
            channel: Some(meta.channel.clone()),
            path: dest.clone(),
            installed_at: now_iso(),
        };
        mark_installed(state, "nginx", record.clone())?;
        state.stack.nginx_version = Some(meta.version.clone());
        save_state(&paths.state_file, state)?;

        progress(ProgressEvent {
            package_id: "nginx".into(),
            version: meta.version.clone(),
            phase: "done".into(),
            downloaded: 0,
            total: None,
        });

        Ok(InstalledPackage {
            package_id: "nginx".into(),
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
        default_uninstall("nginx", version, paths, state).await
    }
}

fn find_nginx_bin(root: &Path) -> Option<PathBuf> {
    crate::services::process::find_bin(root, &["nginx", "nginx.exe"])
}

fn extract_deb(deb: &Path, dest: &Path) -> Result<()> {
    if which::which("dpkg-deb").is_ok() {
        let status = Command::new("dpkg-deb")
            .args(["-x", &deb.to_string_lossy(), &dest.to_string_lossy()])
            .status()
            .map_err(|e| DevStackError::Message(e.to_string()))?;
        if status.success() {
            return Ok(());
        }
    }
    let mut archive = ar::Archive::new(fs::File::open(deb)?);
    let staging = dest.join(".deb-staging");
    fs::create_dir_all(&staging)?;
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.map_err(|e| DevStackError::Archive(e.to_string()))?;
        let name = String::from_utf8_lossy(entry.header().identifier()).to_string();
        if name.starts_with("data.tar") {
            let data_path = staging.join(&name);
            let mut out = fs::File::create(&data_path)?;
            std::io::copy(&mut entry, &mut out)?;
            if name.ends_with(".xz") {
                let file = fs::File::open(&data_path)?;
                let dec = xz2::read::XzDecoder::new(file);
                let mut tar = tar::Archive::new(dec);
                tar.unpack(dest)
                    .map_err(|e| DevStackError::Archive(e.to_string()))?;
            } else if name.ends_with(".gz") {
                let file = fs::File::open(&data_path)?;
                let dec = flate2::read::GzDecoder::new(file);
                let mut tar = tar::Archive::new(dec);
                tar.unpack(dest)
                    .map_err(|e| DevStackError::Archive(e.to_string()))?;
            }
        } else {
            let mut sink = Vec::new();
            let _ = entry.read_to_end(&mut sink);
        }
    }
    let _ = fs::remove_dir_all(staging);
    Ok(())
}

fn cmp_ver(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    parse(a).cmp(&parse(b))
}
