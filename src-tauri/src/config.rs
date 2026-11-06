use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{DevStackError, Result};

/// On-disk application settings and installed-package inventory.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppState {
    /// Optional URL for the macOS/Linux PHP static-binary manifest.
    pub php_manifest_url: Option<String>,
    /// package_id -> list of installed versions
    pub installed: HashMap<String, Vec<InstalledRecord>>,
    /// PHP version -> enabled extensions (e.g. xdebug)
    pub php_extensions: HashMap<String, PhpExtensionSettings>,
    /// Sites (milestone stub for Environment dropdowns)
    pub sites: Vec<SiteRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRecord {
    pub version: String,
    pub channel: Option<String>,
    pub path: PathBuf,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PhpExtensionSettings {
    pub xdebug: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteRecord {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub php_version: Option<String>,
    pub node_version: Option<String>,
    pub https: bool,
}

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub packages: PathBuf,
    pub downloads: PathBuf,
    pub state_file: PathBuf,
}

impl Paths {
    pub fn from_root(root: PathBuf) -> Result<Self> {
        let packages = root.join("packages");
        let downloads = root.join("downloads");
        let state_file = root.join("state.json");
        fs::create_dir_all(&packages)?;
        fs::create_dir_all(&downloads)?;
        Ok(Self {
            root,
            packages,
            downloads,
            state_file,
        })
    }

    pub fn package_dir(&self, package_id: &str, version: &str) -> PathBuf {
        self.packages.join(package_id).join(version)
    }
}

pub fn default_data_root() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("DevStack")
}

pub fn load_state(path: &Path) -> Result<AppState> {
    if !path.exists() {
        return Ok(AppState::default());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_state(path: &Path, state: &AppState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(state)?;
    fs::write(path, raw)?;
    Ok(())
}

pub fn mark_installed(
    state: &mut AppState,
    package_id: &str,
    record: InstalledRecord,
) -> Result<()> {
    let list = state.installed.entry(package_id.to_string()).or_default();
    list.retain(|r| r.version != record.version);
    list.push(record);
    list.sort_by(|a, b| b.version.cmp(&a.version));
    Ok(())
}

pub fn mark_uninstalled(state: &mut AppState, package_id: &str, version: &str) -> Result<()> {
    if let Some(list) = state.installed.get_mut(package_id) {
        list.retain(|r| r.version != version);
    }
    if package_id == "php" {
        state.php_extensions.remove(version);
    }
    Ok(())
}

pub fn ensure_demo_site(state: &mut AppState) {
    if state.sites.is_empty() {
        state.sites.push(SiteRecord {
            id: "acme-dashboard".into(),
            name: "acme-dashboard".into(),
            path: PathBuf::from("~/Sites/acme-dashboard"),
            php_version: None,
            node_version: None,
            https: true,
        });
    }
}

pub fn remove_dir_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(DevStackError::Io)?;
    }
    Ok(())
}
