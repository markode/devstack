//! Shared download, checksum verification, and archive extraction.

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{DevStackError, Result};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub package_id: String,
    pub version: String,
    pub phase: String,
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub type ProgressFn = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

pub fn noop_progress() -> ProgressFn {
    Arc::new(|_| {})
}

pub async fn download_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    package_id: &str,
    version: &str,
    progress: &ProgressFn,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let response = client.get(url).send().await?.error_for_status()?;
    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = File::create(dest)?;
    let mut downloaded: u64 = 0;

    progress(ProgressEvent {
        package_id: package_id.into(),
        version: version.into(),
        phase: "downloading".into(),
        downloaded,
        total,
    });

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        progress(ProgressEvent {
            package_id: package_id.into(),
            version: version.into(),
            phase: "downloading".into(),
            downloaded,
            total,
        });
    }

    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1024 * 64];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    let expected = expected.trim().to_lowercase();
    if actual != expected {
        return Err(DevStackError::ChecksumMismatch { expected, actual });
    }
    Ok(())
}

pub async fn download_and_verify(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    package_id: &str,
    version: &str,
    progress: &ProgressFn,
) -> Result<()> {
    download_file(client, url, dest, package_id, version, progress).await?;
    if let Some(expected) = sha256 {
        progress(ProgressEvent {
            package_id: package_id.into(),
            version: version.into(),
            phase: "verifying".into(),
            downloaded: 0,
            total: None,
        });
        verify_sha256(dest, expected)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
    TarXz,
}

impl ArchiveKind {
    pub fn from_name(name: &str) -> Result<Self> {
        let lower = name.to_lowercase();
        if lower.ends_with(".zip") {
            Ok(Self::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Ok(Self::TarGz)
        } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
            Ok(Self::TarXz)
        } else {
            Err(DevStackError::Archive(format!(
                "unsupported archive format: {name}"
            )))
        }
    }
}

pub fn extract_archive(
    archive: &Path,
    dest: &Path,
    kind: ArchiveKind,
    strip_components: usize,
) -> Result<()> {
    fs::create_dir_all(dest)?;
    match kind {
        ArchiveKind::Zip => extract_zip(archive, dest, strip_components),
        ArchiveKind::TarGz => {
            let file = File::open(archive)?;
            let dec = flate2::read::GzDecoder::new(file);
            extract_tar(dec, dest, strip_components)
        }
        ArchiveKind::TarXz => {
            let file = File::open(archive)?;
            let dec = xz2::read::XzDecoder::new(file);
            extract_tar(dec, dest, strip_components)
        }
    }
}

fn extract_tar<R: Read>(reader: R, dest: &Path, strip_components: usize) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(|e| DevStackError::Archive(e.to_string()))? {
        let mut entry = entry.map_err(|e| DevStackError::Archive(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| DevStackError::Archive(e.to_string()))?;
        let stripped = match strip_path(&path, strip_components) {
            Some(p) => p,
            None => continue,
        };
        let out = dest.join(stripped);
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            entry
                .unpack(&out)
                .map_err(|e| DevStackError::Archive(e.to_string()))?;
        }
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path, strip_components: usize) -> Result<()> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| DevStackError::Archive(e.to_string()))?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| DevStackError::Archive(e.to_string()))?;
        let name = file.name().to_string();
        let rel = PathBuf::from(&name);
        let stripped = match strip_path(&rel, strip_components) {
            Some(p) => p,
            None => continue,
        };
        let out = dest.join(stripped);
        if file.is_dir() || name.ends_with('/') {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = File::create(&out)?;
            std::io::copy(&mut file, &mut outfile)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    fs::set_permissions(&out, fs::Permissions::from_mode(mode))?;
                }
            }
        }
    }
    Ok(())
}

fn strip_path(path: &Path, strip: usize) -> Option<PathBuf> {
    let components: Vec<_> = path.components().collect();
    if components.len() <= strip {
        return None;
    }
    Some(components[strip..].iter().collect())
}

pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("DevStack/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?)
}

pub async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client.get(url).send().await?.error_for_status()?.text().await?)
}

pub async fn fetch_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T> {
    Ok(client.get(url).send().await?.error_for_status()?.json().await?)
}

/// Host / arch helpers shared by package providers.
pub fn current_os() -> &'static str {
    std::env::consts::OS
}

pub fn current_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

pub fn node_platform_triple() -> Result<(&'static str, &'static str, ArchiveKind)> {
    let os = current_os();
    let arch = current_arch();
    match (os, arch) {
        ("macos", "x64") => Ok(("darwin", "x64", ArchiveKind::TarGz)),
        ("macos", "arm64") => Ok(("darwin", "arm64", ArchiveKind::TarGz)),
        ("linux", "x64") => Ok(("linux", "x64", ArchiveKind::TarXz)),
        ("linux", "arm64") => Ok(("linux", "arm64", ArchiveKind::TarXz)),
        ("windows", "x64") => Ok(("win", "x64", ArchiveKind::Zip)),
        ("windows", "arm64") => Ok(("win", "arm64", ArchiveKind::Zip)),
        _ => Err(DevStackError::UnsupportedPlatform(format!("{os}/{arch}"))),
    }
}
