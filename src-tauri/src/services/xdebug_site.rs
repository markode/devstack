//! Per-site Xdebug toggle. Xdebug is configured per PHP version (php.ini), so
//! enabling it for a site switches it on for that site's PHP version and writes
//! IDE hints (VS Code `launch.json`, PhpStorm server/path mapping).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::{save_state, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::paths_util::expand_home;

pub const XDEBUG_PORT: u16 = 9003;
const CONFIG_NAME: &str = "Listen for Xdebug (DevStack)";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhpStormHint {
    pub server_name: String,
    pub host: String,
    pub port: u16,
    pub local_path: String,
    pub remote_path: String,
    pub ide_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteXdebugView {
    pub site_id: String,
    pub php_version: String,
    pub enabled: bool,
    pub port: u16,
    pub launch_json: Option<PathBuf>,
    pub phpstorm: PhpStormHint,
}

pub fn launch_json_config(site_root: &Path) -> Value {
    let root = site_root.display().to_string().replace('\\', "/");
    json!({
        "name": CONFIG_NAME,
        "type": "php",
        "request": "launch",
        "port": XDEBUG_PORT,
        "pathMappings": { root.clone(): "${workspaceFolder}" }
    })
}

pub fn launch_json(site_root: &Path) -> Value {
    json!({
        "version": "0.2.0",
        "configurations": [launch_json_config(site_root)]
    })
}

/// Merge our configuration into an existing launch.json (plain JSON only).
/// Returns `None` when the existing file cannot be parsed (e.g. contains comments).
pub fn merge_launch_json(existing: &str, site_root: &Path) -> Option<Value> {
    let mut doc: Value = serde_json::from_str(existing).ok()?;
    let cfgs = doc
        .as_object_mut()?
        .entry("configurations")
        .or_insert_with(|| json!([]))
        .as_array_mut()?;
    cfgs.retain(|c| c.get("name").and_then(|n| n.as_str()) != Some(CONFIG_NAME));
    cfgs.push(launch_json_config(site_root));
    Some(doc)
}

fn write_launch_json(site_root: &Path) -> Result<PathBuf> {
    let dir = site_root.join(".vscode");
    fs::create_dir_all(&dir)?;
    let file = dir.join("launch.json");
    let doc = match fs::read_to_string(&file) {
        Ok(existing) => match merge_launch_json(&existing, site_root) {
            Some(d) => d,
            None => {
                let alt = dir.join("launch.devstack.json");
                fs::write(&alt, serde_json::to_string_pretty(&launch_json(site_root))?)?;
                return Ok(alt);
            }
        },
        Err(_) => launch_json(site_root),
    };
    fs::write(&file, serde_json::to_string_pretty(&doc)?)?;
    Ok(file)
}

fn php_version_for(site: &SiteRecord, state: &AppState) -> Result<String> {
    site.php_version
        .clone()
        .or_else(|| state.stack.php_version.clone())
        .or_else(|| {
            state
                .installed
                .get("php")
                .and_then(|l| l.first())
                .map(|r| r.version.clone())
        })
        .ok_or_else(|| DevStackError::Message("no PHP version installed".into()))
}

pub fn phpstorm_hint(site: &SiteRecord) -> PhpStormHint {
    let local = expand_home(&site.path).display().to_string();
    PhpStormHint {
        server_name: site.domain.clone(),
        host: site.domain.clone(),
        port: if site.https { 443 } else { 80 },
        local_path: local.clone(),
        remote_path: local,
        ide_key: "PHPSTORM".into(),
    }
}

pub fn set_site_xdebug(
    paths: &Paths,
    state: &mut AppState,
    site_id: &str,
    enabled: bool,
) -> Result<SiteXdebugView> {
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| DevStackError::Message(format!("site not found: {site_id}")))?;
    let version = php_version_for(&site, state)?;
    crate::php::set_xdebug(state, paths, &version, enabled)?;

    if state.runtime.php_pids.contains_key(&version) {
        let _ = crate::services::php_runtime::stop_php(paths, state, &version);
        let _ = crate::services::php_runtime::start_php(paths, state, &version);
    }

    let root = expand_home(&site.path);
    let launch = if enabled && root.is_dir() {
        write_launch_json(&root).ok()
    } else {
        None
    };
    if let Some(s) = state.sites.iter_mut().find(|s| s.id == site_id) {
        s.xdebug = enabled;
    }
    save_state(&paths.state_file, state)?;
    Ok(SiteXdebugView {
        site_id: site.id.clone(),
        php_version: version,
        enabled,
        port: XDEBUG_PORT,
        launch_json: launch,
        phpstorm: phpstorm_hint(&site),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_json_shape() {
        let v = launch_json(Path::new("/home/me/Sites/blog"));
        assert_eq!(v["version"], "0.2.0");
        let cfg = &v["configurations"][0];
        assert_eq!(cfg["type"], "php");
        assert_eq!(cfg["request"], "launch");
        assert_eq!(cfg["port"], 9003);
        assert_eq!(cfg["pathMappings"]["/home/me/Sites/blog"], "${workspaceFolder}");
    }

    #[test]
    fn merge_keeps_other_configs_and_replaces_ours() {
        let existing = r#"{"version":"0.2.0","configurations":[{"name":"other"},{"name":"Listen for Xdebug (DevStack)","port":1}]}"#;
        let merged = merge_launch_json(existing, Path::new("/s")).unwrap();
        let cfgs = merged["configurations"].as_array().unwrap();
        assert_eq!(cfgs.len(), 2);
        assert_eq!(cfgs[0]["name"], "other");
        assert_eq!(cfgs[1]["port"], 9003);
        assert!(merge_launch_json("// comment\n{}", Path::new("/s")).is_none());
    }

    #[test]
    fn write_launch_json_creates_file() {
        let root = std::env::temp_dir().join(format!("devstack-xd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let f = write_launch_json(&root).unwrap();
        assert!(f.ends_with(".vscode/launch.json"));
        let _ = write_launch_json(&root).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(v["configurations"].as_array().unwrap().len(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
