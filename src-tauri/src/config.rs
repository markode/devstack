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
    /// Optional URL for macOS/Linux Nginx static-binary manifest.
    pub nginx_manifest_url: Option<String>,
    /// Optional URL for macOS/Linux Apache httpd prebuilt manifest.
    pub apache_manifest_url: Option<String>,
    /// package_id -> list of installed versions
    pub installed: HashMap<String, Vec<InstalledRecord>>,
    /// PHP version -> enabled extensions (e.g. xdebug)
    pub php_extensions: HashMap<String, PhpExtensionSettings>,
    /// Sites
    pub sites: Vec<SiteRecord>,
    /// Saved starter templates (file + DB snapshots)
    pub templates: Vec<TemplateRecord>,
    /// Active database engine for stack services: "mariadb" | "mysql"
    pub active_database: Option<String>,
    /// Preferred versions for stack services
    pub stack: StackPreferences,
    /// Runtime process bookkeeping (pids)
    pub runtime: RuntimeState,
    /// Plain-file database exports (Databases folder)
    pub db_files: DatabaseFilesSettings,
    /// Set after the first-launch `welcome.test` site has been seeded (or
    /// skipped). Prevents recreating the site after the user deletes it.
    pub welcome_site_seeded: bool,
}

/// Settings for the user-facing Databases folder (portable dumps, not raw datadirs).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DatabaseFilesSettings {
    /// Override root for Databases folder; default `{data}/Databases`.
    pub folder: Option<PathBuf>,
    /// Auto-export every N seconds while engines are running (0 = disabled).
    pub auto_export_interval_secs: u64,
    /// Export dumps when a database service is stopped.
    pub auto_export_on_stop: bool,
    /// `ask` = confirm in UI before import; `auto` = import on drop/replace.
    pub import_mode: String,
}

impl Default for DatabaseFilesSettings {
    fn default() -> Self {
        Self {
            folder: None,
            auto_export_interval_secs: 300,
            auto_export_on_stop: true,
            import_mode: "ask".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StackPreferences {
    pub nginx_version: Option<String>,
    pub apache_version: Option<String>,
    pub php_version: Option<String>,
    pub mariadb_version: Option<String>,
    pub mysql_version: Option<String>,
    pub phpmyadmin_version: Option<String>,
    /// Nginx HTTP listen port (80 preferred; falls back when unprivileged)
    pub http_port: u16,
    pub https_port: u16,
    /// Apache HTTP/HTTPS ports (defaults leave 80/443 free for Nginx, or
    /// swap when Apache is the only front-end — see docs/apache.md).
    pub apache_http_port: u16,
    pub apache_https_port: u16,
    /// MySQL Community Server TCP port (default 3306).
    pub mysql_port: u16,
    /// MariaDB TCP port (default 3306; coexistence uses 3308).
    pub mariadb_port: u16,
    pub postgresql_port: u16,
    pub mongodb_port: u16,
    /// Base PHP-FPM / php-cgi TCP port on Windows (`php_fpm_port + version offset`).
    pub php_fpm_port: u16,
    /// When true, prefer non-default ports so DevStack can sit beside WAMP/MAMP.
    pub coexistence_mode: bool,
}

impl Default for StackPreferences {
    fn default() -> Self {
        Self {
            nginx_version: None,
            apache_version: None,
            php_version: None,
            mariadb_version: None,
            mysql_version: None,
            phpmyadmin_version: None,
            http_port: 80,
            https_port: 443,
            apache_http_port: 8088,
            apache_https_port: 8444,
            mysql_port: 3306,
            mariadb_port: 3306,
            postgresql_port: 5432,
            mongodb_port: 27017,
            php_fpm_port: 9000,
            coexistence_mode: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RuntimeState {
    pub nginx_pid: Option<u32>,
    pub apache_pid: Option<u32>,
    pub dns_pid: Option<u32>,
    /// php version -> fpm/cgi master pid
    pub php_pids: HashMap<String, u32>,
    pub database_pid: Option<u32>,
    pub database_engine: Option<String>,
    pub mongodb_pid: Option<u32>,
    pub postgresql_pid: Option<u32>,
    pub redis_pid: Option<u32>,
    pub mailpit_pid: Option<u32>,
    /// Ephemeral banner after auto port remap (not required across restarts).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port_notice: Vec<String>,
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
    /// FQDN under .test, e.g. acme-dashboard.test
    #[serde(default)]
    pub domain: String,
    pub php_version: Option<String>,
    pub node_version: Option<String>,
    pub https: bool,
    /// `"nginx"` | `"apache"` — which front-end serves this site.
    #[serde(default)]
    pub web_server: Option<String>,
    #[serde(default)]
    pub framework: Option<String>,
    #[serde(default)]
    pub database_engine: Option<String>,
    #[serde(default)]
    pub database_name: Option<String>,
    #[serde(default)]
    pub database_user: Option<String>,
    #[serde(default)]
    pub database_password: Option<String>,
    #[serde(default)]
    pub status: String,
    /// True when this site is also exposed as a reusable starter template.
    #[serde(default)]
    pub is_template: bool,
}

impl SiteRecord {
    pub fn ensure_domain(&mut self) {
        if self.domain.is_empty() {
            let slug = slugify(&self.name);
            self.domain = format!("{slug}.test");
        }
        if self.status.is_empty() {
            self.status = "stopped".into();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateRecord {
    pub id: String,
    pub name: String,
    pub source_site_id: String,
    pub source_domain: String,
    pub source_url: String,
    pub framework: String,
    pub created_at: String,
    pub files_archive: PathBuf,
    pub db_archive: Option<PathBuf>,
    pub size_bytes: u64,
    #[serde(default)]
    pub database_engine: Option<String>,
}

#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub packages: PathBuf,
    pub downloads: PathBuf,
    pub state_file: PathBuf,
    pub run: PathBuf,
    pub logs: PathBuf,
    pub nginx: PathBuf,
    pub php: PathBuf,
    pub certs: PathBuf,
    pub databases: PathBuf,
    pub sites_meta: PathBuf,
    pub helper: PathBuf,
    pub templates: PathBuf,
    pub backups: PathBuf,
    pub tools: PathBuf,
    pub apache: PathBuf,
}

impl Paths {
    pub fn from_root(root: PathBuf) -> Result<Self> {
        let packages = root.join("packages");
        let downloads = root.join("downloads");
        let state_file = root.join("state.json");
        let run = root.join("run");
        let logs = root.join("logs");
        let nginx = root.join("nginx");
        let php = root.join("php");
        let certs = root.join("certs");
        let databases = root.join("databases");
        let sites_meta = root.join("sites");
        let helper = root.join("helper");
        let templates = root.join("templates");
        let backups = root.join("backups");
        let tools = root.join("tools");
        let apache = root.join("apache");
        for p in [
            &packages,
            &downloads,
            &run,
            &logs,
            &nginx,
            &php,
            &certs,
            &databases,
            &sites_meta,
            &helper,
            &templates,
            &backups,
            &tools,
            &apache,
        ] {
            fs::create_dir_all(p)?;
        }
        Ok(Self {
            root,
            packages,
            downloads,
            state_file,
            run,
            logs,
            nginx,
            php,
            certs,
            databases,
            sites_meta,
            helper,
            templates,
            backups,
            tools,
            apache,
        })
    }

    pub fn apache_conf_dir(&self) -> PathBuf {
        self.apache.join("conf")
    }

    pub fn apache_sites_dir(&self) -> PathBuf {
        self.apache.join("sites")
    }

    pub fn package_dir(&self, package_id: &str, version: &str) -> PathBuf {
        self.packages.join(package_id).join(version)
    }

    pub fn nginx_conf_dir(&self) -> PathBuf {
        self.nginx.join("conf")
    }

    pub fn nginx_sites_dir(&self) -> PathBuf {
        self.nginx.join("sites")
    }

    pub fn pid_file(&self, name: &str) -> PathBuf {
        self.run.join(format!("{name}.pid"))
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
    let mut state: AppState = serde_json::from_str(&raw)?;
    for site in &mut state.sites {
        site.ensure_domain();
    }
    Ok(state)
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

/// Ensure every site has a `.test` domain. Seed `welcome.test` once on first launch.
pub fn ensure_welcome_site(paths: &Paths, state: &mut AppState) {
    for site in &mut state.sites {
        site.ensure_domain();
    }
    if state.welcome_site_seeded {
        return;
    }
    // Already have sites from a previous install / import — don't inject welcome.
    if !state.sites.is_empty() {
        state.welcome_site_seeded = true;
        return;
    }

    let site_path = default_sites_root().join("welcome");
    let _ = fs::create_dir_all(&site_path);
    let index = site_path.join("index.php");
    if !index.exists() {
        let _ = fs::write(
            &index,
            r#"<?php
header('Content-Type: text/html; charset=utf-8');
?><!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Welcome · DevStack</title>
  <style>
    body { font-family: system-ui, sans-serif; max-width: 40rem; margin: 4rem auto; padding: 0 1.25rem; color: #1a1a1a; }
    h1 { font-size: 1.75rem; margin-bottom: 0.5rem; }
    p { line-height: 1.55; color: #444; }
    code { background: #f3f3f3; padding: 0.1em 0.35em; border-radius: 4px; }
  </style>
</head>
<body>
  <h1>Welcome to DevStack</h1>
  <p>Your local stack is ready. This site lives at <code>welcome.test</code> —
     create more sites from the Sites panel.</p>
  <p>PHP <?= PHP_VERSION ?></p>
</body>
</html>
"#,
        );
    }

    let mut site = SiteRecord {
        id: "welcome".into(),
        name: "welcome".into(),
        path: site_path,
        domain: "welcome.test".into(),
        php_version: None,
        node_version: None,
        https: true,
        web_server: Some("nginx".into()),
        framework: Some("other".into()),
        database_engine: None,
        database_name: None,
        database_user: None,
        database_password: None,
        status: "stopped".into(),
        is_template: false,
    };
    site.ensure_domain();
    state.sites.push(site);
    state.welcome_site_seeded = true;
    let _ = paths; // reserved for future cert/hosts bootstrap at seed time
}

/// Back-compat alias used by older call sites / smoke tests.
pub fn ensure_demo_site(state: &mut AppState) {
    for site in &mut state.sites {
        site.ensure_domain();
    }
}

fn default_sites_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Sites")
}

pub fn remove_dir_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(DevStackError::Io)?;
    }
    Ok(())
}

pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' || ch.is_whitespace() {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "site".into()
    } else {
        trimmed
    }
}

pub fn latest_installed<'a>(
    state: &'a AppState,
    package_id: &str,
) -> Option<&'a InstalledRecord> {
    state.installed.get(package_id).and_then(|v| v.first())
}

pub fn find_installed<'a>(
    state: &'a AppState,
    package_id: &str,
    version: &str,
) -> Option<&'a InstalledRecord> {
    state.installed.get(package_id).and_then(|list| {
        list.iter()
            .find(|r| r.version == version || r.channel.as_deref() == Some(version))
    })
}
