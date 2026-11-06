//! PostgreSQL — versions from postgresql.org/versions.json.

use async_trait::async_trait;
use serde::Deserialize;

use super::{
    default_uninstall, InstalledPackage, PackageInfo, PackageProvider, PackageVersion,
};
use crate::config::{AppState, Paths};
use crate::downloads::ProgressFn;
use crate::error::{DevStackError, Result};

const VERSIONS_JSON: &str = "https://www.postgresql.org/versions.json";

#[derive(Clone)]
pub struct PostgreSqlProvider {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PgVersion {
    major: String,
    latest_minor: String,
    supported: bool,
    current: bool,
    eol_date: Option<String>,
}

impl PostgreSqlProvider {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PackageProvider for PostgreSqlProvider {
    fn info(&self) -> PackageInfo {
        PackageInfo {
            id: "postgresql".into(),
            name: "PostgreSQL".into(),
            description: "PostgreSQL from postgresql.org version catalog.".into(),
            category: "service".into(),
            install_ready: false,
        }
    }

    async fn list_versions(&self, _state: &AppState) -> Result<Vec<PackageVersion>> {
        let rows: Vec<PgVersion> = self
            .client
            .get(VERSIONS_JSON)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let mut out: Vec<PackageVersion> = rows
            .into_iter()
            .filter(|v| v.supported || v.current)
            .map(|v| {
                let version = format!("{}.{}", v.major, v.latest_minor);
                PackageVersion {
                    label: format!("{} ({version})", v.major),
                    version,
                    channel: v.major.clone(),
                    lts: v.supported,
                    download_url: Some(format!(
                        "https://ftp.postgresql.org/pub/source/v{0}.{1}/postgresql-{0}.{1}.tar.bz2",
                        v.major, v.latest_minor
                    )),
                    sha256: None,
                    notes: v.eol_date.map(|d| format!("EOL {d}")).or_else(|| {
                        Some(
                            "Official version catalog; platform binary install in next milestone."
                                .into(),
                        )
                    }),
                }
            })
            .collect();

        out.sort_by(|a, b| b.channel.cmp(&a.channel));
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
            "PostgreSQL binary install lands in the next milestone (version list is live)."
                .into(),
        ))
    }

    async fn uninstall(
        &self,
        version: &str,
        paths: &Paths,
        state: &mut AppState,
    ) -> Result<()> {
        default_uninstall("postgresql", version, paths, state).await
    }
}
