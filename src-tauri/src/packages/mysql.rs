//! MySQL Community Server — version list from GitHub tags + CDN notes.
//!
//! Install of platform archives lands in a later milestone; listing is live.

use async_trait::async_trait;
use serde::Deserialize;

use super::{
    default_uninstall, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{AppState, Paths};
use crate::downloads::ProgressFn;
use crate::error::{DevStackError, Result};

#[derive(Clone)]
pub struct MySqlProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
struct GhTag {
    name: String,
}

impl MySqlProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PackageProvider for MySqlProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "mysql".into(),
            name: "MySQL".into(),
            description: "MySQL Community Server (official archives).".into(),
            category: "service".into(),
            // Binary install wiring follows the same download+SHA path as Node;
            // deferred to keep this milestone focused on the provider surface.
            install_ready: false,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        // Official mysql/mysql-server tags: mysql-8.4.5, mysql-8.0.40, mysql-9.x, …
        let tags: Vec<GhTag> = self
            .client
            .get("https://api.github.com/repos/mysql/mysql-server/tags?per_page=100")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut out = Vec::new();
        for tag in tags {
            let Some(version) = tag.name.strip_prefix("mysql-") else {
                continue;
            };
            // Skip cluster / innovation noise prefixes already stripped; keep semver-ish.
            if !version
                .chars()
                .next()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                continue;
            }
            let channel = version
                .split('.')
                .take(2)
                .collect::<Vec<_>>()
                .join(".");
            let major = channel.split('.').next().unwrap_or("0");
            // CDN layout uses MySQL-<major.minor>/ for classic lines; Innovation uses Downloads/MySQL-<major>/
            let url = format!(
                "https://cdn.mysql.com/Downloads/MySQL-{channel}/mysql-{version}-INFO.txt"
            );
            out.push(PackageVersion {
                label: version.to_string(),
                version: version.to_string(),
                channel: channel.clone(),
                lts: (major == "8" && channel.ends_with(".4")) || channel == "8.0",
                download_url: Some(url),
                sha256: None,
                notes: Some(
                    "Version listing from mysql/mysql-server tags. Full archive install in a later milestone."
                        .into(),
                ),
            });
            if out.len() >= 40 {
                break;
            }
        }
        Ok(out)
    }

    async fn install(
        &self,
        _version: &str,
        _paths: &Paths,
        _state: &mut AppState,
        _progress: ProgressFn,
    ) -> Result<InstalledPackage> {
        Err(DevStackError::Message(
            "MySQL archive install lands in the next milestone (provider + checksum path already shared)."
                .into(),
        ))
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
