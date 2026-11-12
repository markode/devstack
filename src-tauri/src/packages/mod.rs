//! Generic package providers (PHP, Node, databases, phpMyAdmin, …).

mod apache;
mod macos_defaults;
mod mailpit;
mod mariadb;
mod mongodb;
mod mysql;
mod nginx;
mod node;
pub mod php;
mod phpmyadmin;
mod postgresql;
mod redis;
pub mod service_bins;
pub mod version_cache;

pub use macos_defaults::{DEFAULT_COMBINED_MANIFEST_URL, PACKAGES_RELEASE_TAG};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::{AppState, InstalledRecord, Paths};
use crate::downloads::{http_client, ProgressFn};
use crate::error::{DevStackError, Result};

pub use apache::ApacheProvider;
pub use mailpit::MailpitProvider;
pub use mariadb::MariaDbProvider;
pub use mongodb::MongoDbProvider;
pub use mysql::MySqlProvider;
pub use nginx::NginxProvider;
pub use node::NodeProvider;
pub use php::PhpProvider;
pub use phpmyadmin::PhpMyAdminProvider;
pub use postgresql::PostgreSqlProvider;
pub use redis::RedisProvider;
pub use service_bins::{ServiceBinProvider, ServiceKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageVersion {
    /// Exact version string used for install (e.g. "22.14.0", "8.4.5").
    pub version: String,
    /// Grouping label shown in the UI (major channel), e.g. "22", "8.4".
    pub channel: String,
    pub label: String,
    pub lts: bool,
    pub download_url: Option<String>,
    pub sha256: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub install_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackage {
    pub package_id: String,
    pub version: String,
    pub channel: Option<String>,
    pub path: PathBuf,
    pub installed_at: String,
}

#[async_trait]
pub trait PackageProvider: Send + Sync {
    fn info(&self) -> PackageInfo;

    async fn list_versions(&self, state: &AppState) -> Result<Vec<PackageVersion>>;

    /// Full one-click install. Providers may return `install_ready: false`
    /// and refuse until a later milestone wires binary extraction.
    async fn install(
        &self,
        version: &str,
        paths: &Paths,
        state: &mut AppState,
        progress: ProgressFn,
    ) -> Result<InstalledPackage>;

    async fn uninstall(
        &self,
        version: &str,
        paths: &Paths,
        state: &mut AppState,
    ) -> Result<()>;
}

pub struct PackageRegistry {
    providers: HashMap<String, Arc<dyn PackageProvider>>,
    client: reqwest::Client,
}

impl PackageRegistry {
    pub fn new() -> Result<Self> {
        let client = http_client()?;
        let mut providers: HashMap<String, Arc<dyn PackageProvider>> = HashMap::new();

        let list: Vec<Arc<dyn PackageProvider>> = vec![
            Arc::new(PhpProvider::new(client.clone())),
            Arc::new(NodeProvider::new(client.clone())),
            Arc::new(NginxProvider::new(client.clone())),
            Arc::new(ApacheProvider::new(client.clone())),
            Arc::new(PhpMyAdminProvider::new(client.clone())),
            Arc::new(MySqlProvider::new(client.clone())),
            Arc::new(MariaDbProvider::new(client.clone())),
            Arc::new(MongoDbProvider::new(client.clone())),
            Arc::new(PostgreSqlProvider::new(client.clone())),
            Arc::new(RedisProvider::new(client.clone())),
            Arc::new(MailpitProvider::new(client.clone())),
            Arc::new(ServiceBinProvider::new(ServiceKind::Memcached, client.clone())),
            Arc::new(ServiceBinProvider::new(ServiceKind::Meilisearch, client.clone())),
            Arc::new(ServiceBinProvider::new(ServiceKind::Typesense, client.clone())),
            Arc::new(ServiceBinProvider::new(ServiceKind::Minio, client.clone())),
        ];

        for p in list {
            providers.insert(p.info().id.clone(), p);
        }

        Ok(Self { providers, client })
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn list_packages(&self) -> Vec<PackageInfo> {
        let mut infos: Vec<_> = self.providers.values().map(|p| p.info()).collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    pub fn get(&self, id: &str) -> Result<Arc<dyn PackageProvider>> {
        self.providers
            .get(id)
            .cloned()
            .ok_or_else(|| DevStackError::Message(format!("unknown package: {id}")))
    }
}

pub fn installed_from_state(state: &AppState, package_id: &str) -> Vec<InstalledPackage> {
    state
        .installed
        .get(package_id)
        .into_iter()
        .flatten()
        .map(|r| InstalledPackage {
            package_id: package_id.to_string(),
            version: r.version.clone(),
            channel: r.channel.clone(),
            path: r.path.clone(),
            installed_at: r.installed_at.clone(),
        })
        .collect()
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn record_to_installed(package_id: &str, r: &InstalledRecord) -> InstalledPackage {
    InstalledPackage {
        package_id: package_id.to_string(),
        version: r.version.clone(),
        channel: r.channel.clone(),
        path: r.path.clone(),
        installed_at: r.installed_at.clone(),
    }
}

/// Generic uninstall used by most providers.
pub async fn default_uninstall(
    package_id: &str,
    version: &str,
    paths: &Paths,
    state: &mut AppState,
) -> Result<()> {
    let dir = paths.package_dir(package_id, version);
    crate::config::remove_dir_if_exists(&dir)?;
    crate::config::mark_uninstalled(state, package_id, version)?;
    crate::config::save_state(&paths.state_file, state)?;
    Ok(())
}
