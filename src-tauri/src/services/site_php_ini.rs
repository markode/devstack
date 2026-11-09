//! Per-site PHP overrides: `{sites_meta}/{id}/php.ini` merged over the global ini.
//!
//! - php-fpm (macOS/Linux): overrides are injected into the site's nginx vhost as
//!   `fastcgi_param PHP_VALUE` (the pool uses `php_value`, so they win).
//! - php-cgi (Windows): overrides are mirrored to `{docroot}/.user.ini`.
//! - CLI (artisan/composer in the terminal): `php-merged.ini` is the full merged file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{find_installed, save_state, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};

const USER_INI_MARKER: &str = "; managed by DevStack (site php.ini overrides)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyKind {
    /// `256M`, `1G`, `0`, `-1` (memory_limit only for -1).
    Size,
    /// Plain integer (seconds, counts).
    Int,
    Bool,
    Text,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownKey {
    pub key: &'static str,
    pub kind: KeyKind,
    pub description: &'static str,
}

pub const KNOWN_KEYS: &[KnownKey] = &[
    KnownKey { key: "memory_limit", kind: KeyKind::Size, description: "Max memory per request (-1 = unlimited)" },
    KnownKey { key: "max_execution_time", kind: KeyKind::Int, description: "Seconds a script may run (0 = unlimited)" },
    KnownKey { key: "max_input_time", kind: KeyKind::Int, description: "Seconds to parse input (-1 = unlimited)" },
    KnownKey { key: "max_input_vars", kind: KeyKind::Int, description: "Max input variables per request" },
    KnownKey { key: "max_file_uploads", kind: KeyKind::Int, description: "Max files per upload request" },
    KnownKey { key: "upload_max_filesize", kind: KeyKind::Size, description: "Largest single upload" },
    KnownKey { key: "post_max_size", kind: KeyKind::Size, description: "Largest POST body" },
    KnownKey { key: "display_errors", kind: KeyKind::Bool, description: "Show errors in output" },
    KnownKey { key: "error_reporting", kind: KeyKind::Text, description: "Error level (e.g. E_ALL)" },
    KnownKey { key: "date.timezone", kind: KeyKind::Text, description: "Default timezone (e.g. Europe/Berlin)" },
    KnownKey { key: "default_socket_timeout", kind: KeyKind::Int, description: "Socket timeout in seconds" },
    KnownKey { key: "session.gc_maxlifetime", kind: KeyKind::Int, description: "Session lifetime in seconds" },
    KnownKey { key: "realpath_cache_size", kind: KeyKind::Size, description: "Realpath cache size" },
    KnownKey { key: "short_open_tag", kind: KeyKind::Bool, description: "Allow <? short tags" },
    KnownKey { key: "opcache.enable", kind: KeyKind::Bool, description: "Enable OPcache" },
    KnownKey { key: "opcache.validate_timestamps", kind: KeyKind::Bool, description: "Check file timestamps" },
    KnownKey { key: "zend.assertions", kind: KeyKind::Int, description: "1 = on, 0 = off, -1 = compiled out" },
];

pub fn known_key(key: &str) -> Option<&'static KnownKey> {
    KNOWN_KEYS.iter().find(|k| k.key == key)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sliders {
    /// `None` = unlimited (-1).
    pub memory_limit_mb: Option<u32>,
    /// `0` = unlimited.
    pub max_execution_time: u32,
    pub upload_max_mb: u32,
    pub post_max_mb: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SitePhpIniView {
    pub site_id: String,
    pub php_version: Option<String>,
    pub ini_path: String,
    pub merged_ini_path: String,
    pub overrides: BTreeMap<String, String>,
    pub global: BTreeMap<String, String>,
    pub merged: BTreeMap<String, String>,
    pub sliders: Sliders,
    pub known_keys: Vec<KnownKey>,
    /// True when the change was applied to a running PHP runtime.
    pub reloaded: bool,
    pub notes: Vec<String>,
}

pub fn site_meta_dir(paths: &Paths, site_id: &str) -> PathBuf {
    paths.sites_meta.join(site_id)
}

pub fn site_ini_path(paths: &Paths, site_id: &str) -> PathBuf {
    site_meta_dir(paths, site_id).join("php.ini")
}

pub fn merged_ini_path(paths: &Paths, site_id: &str) -> PathBuf {
    site_meta_dir(paths, site_id).join("php-merged.ini")
}

/// Parse `key = value` lines (ignores comments / sections / blanks).
pub fn parse_ini(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with(';') || t.starts_with('#') || t.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let key = k.trim().to_string();
            let mut val = v.trim().to_string();
            if let Some(idx) = val.find(" ;") {
                val.truncate(idx);
            }
            let val = val.trim().trim_matches('"').to_string();
            if !key.is_empty() {
                out.insert(key, val);
            }
        }
    }
    out
}

pub fn render_ini(overrides: &BTreeMap<String, String>) -> String {
    let mut s = String::from("; DevStack per-site PHP overrides (merged over the global php.ini)\n[PHP]\n");
    for (k, v) in overrides {
        s.push_str(&format!("{k} = {v}\n"));
    }
    s
}

pub fn load_overrides(paths: &Paths, site_id: &str) -> BTreeMap<String, String> {
    fs::read_to_string(site_ini_path(paths, site_id))
        .map(|raw| parse_ini(&raw))
        .unwrap_or_default()
}

fn is_size(v: &str, allow_unlimited: bool) -> bool {
    if allow_unlimited && v == "-1" {
        return true;
    }
    let digits = v.trim_end_matches(['K', 'k', 'M', 'm', 'G', 'g']);
    !digits.is_empty() && digits.len() >= v.len().saturating_sub(1) && digits.chars().all(|c| c.is_ascii_digit())
}

/// Validate + normalize one key/value; returns the value to store.
pub fn validate_value(key: &str, value: &str) -> Result<String> {
    let spec = known_key(key)
        .ok_or_else(|| DevStackError::Message(format!("unknown php.ini key: {key}")))?;
    let v = value.trim();
    if v.is_empty() {
        return Err(DevStackError::Message(format!("{key}: empty value")));
    }
    if v.contains(['"', '\n', '\r', ';', '$', '`']) {
        return Err(DevStackError::Message(format!("{key}: invalid characters in value")));
    }
    match spec.kind {
        KeyKind::Size => {
            if !is_size(v, key == "memory_limit") {
                return Err(DevStackError::Message(format!(
                    "{key}: expected a size like 256M, 2G or 0 (got {v})"
                )));
            }
            Ok(v.to_ascii_uppercase())
        }
        KeyKind::Int => {
            let n: i64 = v
                .parse()
                .map_err(|_| DevStackError::Message(format!("{key}: expected an integer (got {v})")))?;
            if n < -1 {
                return Err(DevStackError::Message(format!("{key}: must be >= -1")));
            }
            Ok(n.to_string())
        }
        KeyKind::Bool => match v.to_ascii_lowercase().as_str() {
            "1" | "on" | "true" | "yes" => Ok("On".into()),
            "0" | "off" | "false" | "no" => Ok("Off".into()),
            _ => Err(DevStackError::Message(format!("{key}: expected On/Off (got {v})"))),
        },
        KeyKind::Text => {
            if v.len() > 120 {
                return Err(DevStackError::Message(format!("{key}: value too long")));
            }
            Ok(v.to_string())
        }
    }
}

pub fn validate_overrides(input: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (k, v) in input {
        out.insert(k.trim().to_string(), validate_value(k.trim(), v)?);
    }
    Ok(out)
}

/// Value of the nginx `PHP_VALUE` fastcgi param (`k=v\nk=v`), or `None` when empty.
pub fn php_value_param(overrides: &BTreeMap<String, String>) -> Option<String> {
    if overrides.is_empty() {
        return None;
    }
    Some(
        overrides
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\\n"),
    )
}

/// `K`/`M`/`G`/bytes → megabytes (rounded up). `-1`/`0` → `None`.
pub fn size_to_mb(v: &str) -> Option<u32> {
    let v = v.trim();
    if v == "-1" || v == "0" || v.is_empty() {
        return None;
    }
    let (num, mult_kb): (&str, f64) = match v.chars().last()? {
        'K' | 'k' => (&v[..v.len() - 1], 1.0 / 1024.0),
        'M' | 'm' => (&v[..v.len() - 1], 1.0),
        'G' | 'g' => (&v[..v.len() - 1], 1024.0),
        _ => (v, 1.0 / (1024.0 * 1024.0)),
    };
    let n: f64 = num.parse().ok()?;
    Some((n * mult_kb).ceil().max(1.0) as u32)
}

pub fn sliders_from_map(merged: &BTreeMap<String, String>) -> Sliders {
    Sliders {
        memory_limit_mb: merged.get("memory_limit").and_then(|v| size_to_mb(v)),
        max_execution_time: merged
            .get("max_execution_time")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        upload_max_mb: merged.get("upload_max_filesize").and_then(|v| size_to_mb(v)).unwrap_or(0),
        post_max_mb: merged.get("post_max_size").and_then(|v| size_to_mb(v)).unwrap_or(0),
    }
}

/// Convert slider positions to ini overrides (`0` MB upload/post = unlimited).
pub fn sliders_to_overrides(s: &Sliders) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert(
        "memory_limit".into(),
        s.memory_limit_mb.map(|mb| format!("{mb}M")).unwrap_or_else(|| "-1".into()),
    );
    m.insert("max_execution_time".into(), s.max_execution_time.to_string());
    m.insert(
        "upload_max_filesize".into(),
        if s.upload_max_mb == 0 { "0".into() } else { format!("{}M", s.upload_max_mb) },
    );
    m.insert(
        "post_max_size".into(),
        if s.post_max_mb == 0 { "0".into() } else { format!("{}M", s.post_max_mb) },
    );
    m
}

fn site_php_version(state: &AppState, site: &SiteRecord) -> Option<String> {
    site.php_version
        .clone()
        .or_else(|| state.stack.php_version.clone())
        .or_else(|| state.installed.get("php").and_then(|v| v.first()).map(|r| r.version.clone()))
}

fn global_ini_map(paths: &Paths, state: &AppState, version: Option<&str>) -> BTreeMap<String, String> {
    if let Some(v) = version {
        if let Some(rec) = find_installed(state, "php", v) {
            let dir = if rec.path.exists() { rec.path.clone() } else { paths.package_dir("php", &rec.version) };
            let ini = crate::packages::php::php_ini_path(&dir);
            if let Ok(raw) = fs::read_to_string(ini) {
                let mut map = parse_ini(&raw);
                map.retain(|k, _| known_key(k).is_some());
                return map;
            }
        }
    }
    let d = crate::php::PhpIniOptions::default();
    BTreeMap::from([
        ("memory_limit".to_string(), d.memory_limit),
        ("max_execution_time".to_string(), d.max_execution_time),
        ("upload_max_filesize".to_string(), d.upload_max_filesize),
        ("post_max_size".to_string(), d.post_max_size),
    ])
}

pub fn merge(global: &BTreeMap<String, String>, overrides: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut m = global.clone();
    for (k, v) in overrides {
        m.insert(k.clone(), v.clone());
    }
    m
}

fn find_site<'a>(state: &'a AppState, site_id: &str) -> Result<&'a SiteRecord> {
    state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .ok_or_else(|| DevStackError::Message(format!("site not found: {site_id}")))
}

pub fn get_site_php_ini(paths: &Paths, state: &AppState, site_id: &str) -> Result<SitePhpIniView> {
    let site = find_site(state, site_id)?;
    let version = site_php_version(state, site);
    let overrides = load_overrides(paths, site_id);
    let global = global_ini_map(paths, state, version.as_deref());
    let merged = merge(&global, &overrides);
    Ok(SitePhpIniView {
        site_id: site_id.into(),
        php_version: version,
        ini_path: site_ini_path(paths, site_id).display().to_string(),
        merged_ini_path: merged_ini_path(paths, site_id).display().to_string(),
        sliders: sliders_from_map(&merged),
        overrides,
        global,
        merged,
        known_keys: KNOWN_KEYS.to_vec(),
        reloaded: false,
        notes: Vec::new(),
    })
}

fn write_user_ini(site: &SiteRecord, overrides: &BTreeMap<String, String>) {
    let path = crate::paths_util::expand_home(&site.path).join(".user.ini");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    // Never clobber a user-authored .user.ini.
    if !existing.is_empty() && !existing.starts_with(USER_INI_MARKER) {
        return;
    }
    if overrides.is_empty() {
        let _ = fs::remove_file(&path);
        return;
    }
    let mut body = format!("{USER_INI_MARKER}\n");
    for (k, v) in overrides {
        body.push_str(&format!("{k} = {v}\n"));
    }
    let _ = fs::write(&path, body);
}

/// Write the override + merged files for a site (no process reload).
pub fn write_files(
    paths: &Paths,
    state: &AppState,
    site: &SiteRecord,
    overrides: &BTreeMap<String, String>,
) -> Result<()> {
    let dir = site_meta_dir(paths, &site.id);
    fs::create_dir_all(&dir)?;
    let ini = site_ini_path(paths, &site.id);
    if overrides.is_empty() {
        let _ = fs::remove_file(&ini);
    } else {
        fs::write(&ini, render_ini(overrides))?;
    }
    let version = site_php_version(state, site);
    let global = global_ini_map(paths, state, version.as_deref());
    let merged = merge(&global, overrides);
    fs::write(merged_ini_path(paths, &site.id), render_ini(&merged))?;
    if cfg!(windows) {
        write_user_ini(site, overrides);
    }
    Ok(())
}

/// Validate, persist, regenerate the vhost and restart the site's PHP runtime.
pub fn set_site_php_ini(
    paths: &Paths,
    state: &mut AppState,
    site_id: &str,
    overrides: BTreeMap<String, String>,
) -> Result<SitePhpIniView> {
    let clean = validate_overrides(&overrides)?;
    let site = find_site(state, site_id)?.clone();
    write_files(paths, state, &site, &clean)?;

    let mut notes = Vec::new();
    let mut reloaded = false;
    if let Some(ver) = site_php_version(state, &site) {
        let running = state.runtime.php_pids.keys().any(|k| *k == ver || k.starts_with(&ver));
        if running {
            let key = state
                .runtime
                .php_pids
                .keys()
                .find(|k| **k == ver || k.starts_with(&ver))
                .cloned()
                .unwrap_or(ver.clone());
            let _ = crate::services::php_runtime::stop_php(paths, state, &key);
            match crate::services::php_runtime::start_php(paths, state, &key) {
                Ok(_) => reloaded = true,
                Err(e) => notes.push(format!("PHP restart failed: {e}")),
            }
        }
    }
    let nginx_up = state.runtime.nginx_pid.map(crate::services::process::is_running).unwrap_or(false);
    if nginx_up {
        if let Err(e) = crate::services::nginx::reload_nginx(paths, state) {
            notes.push(format!("nginx reload failed: {e}"));
        }
    }
    if cfg!(windows) {
        notes.push("php-cgi reads overrides from .user.ini (cached up to 5 minutes).".into());
    }
    save_state(&paths.state_file, state)?;
    let mut view = get_site_php_ini(paths, state, site_id)?;
    view.reloaded = reloaded;
    view.notes = notes;
    Ok(view)
}

pub fn apply_sliders(
    paths: &Paths,
    state: &mut AppState,
    site_id: &str,
    sliders: &Sliders,
) -> Result<SitePhpIniView> {
    let mut overrides = load_overrides(paths, site_id);
    overrides.extend(sliders_to_overrides(sliders));
    set_site_php_ini(paths, state, site_id, overrides)
}

#[allow(dead_code)]
pub fn user_ini_path(site: &SiteRecord) -> PathBuf {
    Path::new(&crate::paths_util::expand_home(&site.path)).join(".user.ini")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(tag: &str) -> (Paths, PathBuf) {
        let dir = std::env::temp_dir().join(format!("devstack-siteini-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (Paths::from_root(dir.clone()).unwrap(), dir)
    }

    #[test]
    fn validates_known_keys_and_values() {
        assert_eq!(validate_value("memory_limit", "512m").unwrap(), "512M");
        assert_eq!(validate_value("memory_limit", "-1").unwrap(), "-1");
        assert!(validate_value("upload_max_filesize", "-1").is_err());
        assert!(validate_value("max_execution_time", "abc").is_err());
        assert!(validate_value("display_errors", "maybe").is_err());
        assert_eq!(validate_value("display_errors", "1").unwrap(), "On");
        assert!(validate_value("disable_functions", "exec").is_err());
        assert!(validate_value("date.timezone", "UTC\"; evil").is_err());
    }

    #[test]
    fn parse_render_roundtrip() {
        let mut m = BTreeMap::new();
        m.insert("memory_limit".to_string(), "256M".to_string());
        m.insert("max_execution_time".to_string(), "60".to_string());
        let parsed = parse_ini(&render_ini(&m));
        assert_eq!(parsed, m);
    }

    #[test]
    fn merge_overrides_win() {
        let g = BTreeMap::from([("memory_limit".to_string(), "-1".to_string()), ("post_max_size".to_string(), "0".to_string())]);
        let o = BTreeMap::from([("memory_limit".to_string(), "128M".to_string())]);
        let m = merge(&g, &o);
        assert_eq!(m["memory_limit"], "128M");
        assert_eq!(m["post_max_size"], "0");
    }

    #[test]
    fn sliders_map_to_ini_keys_and_back() {
        let s = Sliders { memory_limit_mb: Some(512), max_execution_time: 90, upload_max_mb: 64, post_max_mb: 0 };
        let o = sliders_to_overrides(&s);
        assert_eq!(o["memory_limit"], "512M");
        assert_eq!(o["max_execution_time"], "90");
        assert_eq!(o["upload_max_filesize"], "64M");
        assert_eq!(o["post_max_size"], "0");
        assert_eq!(sliders_from_map(&o), s);
        assert_eq!(size_to_mb("2G"), Some(2048));
        assert_eq!(size_to_mb("-1"), None);
    }

    #[test]
    fn php_value_param_joins_with_escaped_newlines() {
        assert_eq!(php_value_param(&BTreeMap::new()), None);
        let o = BTreeMap::from([
            ("max_execution_time".to_string(), "60".to_string()),
            ("memory_limit".to_string(), "256M".to_string()),
        ]);
        assert_eq!(php_value_param(&o).unwrap(), "max_execution_time=60\\nmemory_limit=256M");
    }

    #[test]
    fn set_get_writes_files_without_running_stack() {
        let (paths, dir) = temp_paths("setget");
        let mut state = AppState::default();
        let mut site = SiteRecord {
            id: "blog".into(),
            name: "blog".into(),
            path: dir.join("blog"),
            domain: String::new(),
            php_version: None,
            node_version: None,
            https: true,
            web_server: None,
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: String::new(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: vec![],
            wildcard: false,
        };
        site.ensure_domain();
        state.sites.push(site);
        let view = set_site_php_ini(
            &paths,
            &mut state,
            "blog",
            BTreeMap::from([("memory_limit".into(), "384m".into()), ("max_execution_time".into(), "45".into())]),
        )
        .unwrap();
        assert_eq!(view.overrides["memory_limit"], "384M");
        assert_eq!(view.merged["max_execution_time"], "45");
        assert!(site_ini_path(&paths, "blog").exists());
        assert!(merged_ini_path(&paths, "blog").exists());
        let again = get_site_php_ini(&paths, &state, "blog").unwrap();
        assert_eq!(again.sliders.memory_limit_mb, Some(384));
        assert!(set_site_php_ini(&paths, &mut state, "blog", BTreeMap::from([("nope".into(), "1".into())])).is_err());
        // Clearing removes the override file.
        set_site_php_ini(&paths, &mut state, "blog", BTreeMap::new()).unwrap();
        assert!(!site_ini_path(&paths, "blog").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
