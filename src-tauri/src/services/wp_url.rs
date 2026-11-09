//! Canonical WordPress siteurl/home repair via managed PHP + mysqli.
//!
//! Serialized-safe search-replace of the old absolute URL → canonical URL,
//! plus exact writes of `siteurl` / `home`. Logged to the app Logs tab.

use std::fs;
use std::path::PathBuf;
use std::process::Stdio;

use crate::config::{AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::database::{client_bin_for, engine_install_dir, mysql_root_args};
use crate::services::nginx::expand_home;
use crate::services::php_runtime::resolve_php;
use crate::services::ports::{canonical_site_url, engine_db_port, mysql_db_host};

/// PHP script: set siteurl/home exactly, then serialized-safe REPLACE of $from → $to.
const REPAIR_PHP: &str = r#"<?php
declare(strict_types=1);
$host = getenv('DS_DB_HOST') ?: '127.0.0.1';
$port = (int)(getenv('DS_DB_PORT') ?: '3306');
$user = getenv('DS_DB_USER') ?: 'root';
$pass = getenv('DS_DB_PASS') ?: '';
$db   = getenv('DS_DB_NAME') ?: '';
$from = getenv('DS_URL_FROM') ?: '';
$to   = getenv('DS_URL_TO') ?: '';
if ($db === '' || $to === '') { fwrite(STDERR, "missing db/to\n"); exit(2); }
$mysqli = @new mysqli($host, $user, $pass, $db, $port);
if ($mysqli->connect_errno) { fwrite(STDERR, $mysqli->connect_error."\n"); exit(3); }
$mysqli->set_charset('utf8mb4');

function ds_replace_recursive($data, $from, $to) {
  if (is_string($data)) {
    return str_replace($from, $to, $data);
  }
  if (is_array($data)) {
    $out = [];
    foreach ($data as $k => $v) {
      $nk = is_string($k) ? str_replace($from, $to, $k) : $k;
      $out[$nk] = ds_replace_recursive($v, $from, $to);
    }
    return $out;
  }
  return $data;
}
function ds_serialized_replace($value, $from, $to) {
  if (!is_string($value) || $from === '' || strpos($value, $from) === false) {
    return [$value, false];
  }
  $un = @unserialize($value);
  if ($un !== false || $value === 'b:0;') {
    $repl = ds_replace_recursive($un, $from, $to);
    return [serialize($repl), true];
  }
  return [str_replace($from, $to, $value), true];
}

// Exact siteurl / home.
$stmt = $mysqli->prepare("UPDATE wp_options SET option_value=? WHERE option_name IN ('siteurl','home')");
$stmt->bind_param('s', $to);
$stmt->execute();
$stmt->close();

$changed = 0;
if ($from !== '' && $from !== $to) {
  $tables = $mysqli->query("SHOW TABLES");
  while ($tables && ($row = $tables->fetch_row())) {
    $table = $row[0];
    $cols = $mysqli->query("SHOW COLUMNS FROM `".$mysqli->real_escape_string($table)."`");
    $textCols = [];
    $pk = null;
    while ($cols && ($c = $cols->fetch_assoc())) {
      $type = strtolower($c['Type']);
      if (preg_match('/char|text|blob|json/', $type)) {
        $textCols[] = $c['Field'];
      }
      if ($c['Key'] === 'PRI') { $pk = $c['Field']; }
    }
    if (!$pk || !$textCols) { continue; }
    $sel = "`$pk`, `".implode('`, `', $textCols)."`";
    $res = $mysqli->query("SELECT $sel FROM `$table`");
    if (!$res) { continue; }
    while ($r = $res->fetch_assoc()) {
      $sets = [];
      foreach ($textCols as $col) {
        $val = $r[$col];
        if ($val === null) { continue; }
        [$new, $did] = ds_serialized_replace($val, $from, $to);
        if ($did && $new !== $val) {
          $sets[] = "`$col`='".$mysqli->real_escape_string($new)."'";
        }
      }
      if ($sets) {
        $id = $mysqli->real_escape_string((string)$r[$pk]);
        if ($mysqli->query("UPDATE `$table` SET ".implode(',', $sets)." WHERE `$pk`='$id'")) {
          $changed++;
        }
      }
    }
  }
}
echo "ok siteurl=$to changed_rows=$changed\n";
"#;

fn write_repair_script(paths: &Paths) -> Result<PathBuf> {
    let dir = paths.tools.join("scripts");
    fs::create_dir_all(&dir)?;
    let path = dir.join("wp-repair-url.php");
    fs::write(&path, REPAIR_PHP)?;
    Ok(path)
}

pub fn site_is_wordpress(site: &SiteRecord) -> bool {
    if site.framework.as_deref() == Some("wordpress") {
        return true;
    }
    let abs = expand_home(&site.path);
    abs.join("wp-config.php").exists()
}

fn read_wp_option(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db: &str,
    option: &str,
) -> Result<String> {
    let root = engine_install_dir(state, engine)?;
    let client = client_bin_for(&root)?;
    let mut args = mysql_root_args(paths, state, engine);
    args.push(format!("--database={db}"));
    args.extend([
        "-N".into(),
        "-e".into(),
        format!("SELECT option_value FROM wp_options WHERE option_name='{option}' LIMIT 1;"),
    ]);
    let out = crate::services::process::command(&client)
        .args(&args)
        .output()?;
    if !out.status.success() {
        return Err(DevStackError::Message(format!(
            "failed to read wp_options.{option}"
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Update wp-config.php DB_HOST when the engine listen port changes.
pub fn sync_wp_config_db_host(
    paths: &Paths,
    state: &AppState,
    site: &SiteRecord,
) -> Result<()> {
    let Some(engine) = site
        .database_engine
        .as_deref()
        .or(state.active_database.as_deref())
    else {
        return Ok(());
    };
    if !matches!(engine, "mysql" | "mariadb") {
        return Ok(());
    }
    let abs = expand_home(&site.path);
    let cfg = abs.join("wp-config.php");
    if !cfg.exists() {
        return Ok(());
    }
    let _ = paths;
    let host = mysql_db_host(engine_db_port(&state.stack, engine));
    let raw = fs::read_to_string(&cfg)?;
    let re = regex::Regex::new(
        r#"define\(\s*['"]DB_HOST['"]\s*,\s*['"][^'"]*['"]\s*\)"#,
    )
    .map_err(|e| DevStackError::Message(e.to_string()))?;
    let next = re.replace(&raw, format!("define('DB_HOST', '{host}')"));
    if next.as_ref() != raw.as_str() {
        fs::write(&cfg, next.as_ref())?;
        crate::applog::info(
            "wordpress",
            &format!("{}: wp-config DB_HOST → {host}", site.domain),
        );
    }
    Ok(())
}

/// Repair one site's siteurl/home (+ serialized URL rewrite) to the canonical URL.
pub fn repair_site_url(
    paths: &Paths,
    state: &AppState,
    site: &SiteRecord,
) -> Result<String> {
    if !site_is_wordpress(site) {
        return Err(DevStackError::Message(
            "Repair site URL is only available for WordPress sites".into(),
        ));
    }
    let db = site
        .database_name
        .as_deref()
        .ok_or_else(|| DevStackError::Message("site has no database".into()))?;
    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into());
    if !matches!(engine.as_str(), "mysql" | "mariadb") {
        return Err(DevStackError::Message(
            "URL repair requires MySQL/MariaDB".into(),
        ));
    }

    let canonical = canonical_site_url(site, state);
    let old = read_wp_option(paths, state, &engine, db, "siteurl").unwrap_or_default();
    let from = if old.is_empty() || old == canonical {
        // Still force siteurl/home even when already matching (no table scan).
        String::new()
    } else {
        old.clone()
    };

    let php_version = site
        .php_version
        .clone()
        .or_else(|| state.stack.php_version.clone())
        .unwrap_or_else(|| "8.3".into());
    let php_rt = resolve_php(paths, state, &php_version)?;
    let script = write_repair_script(paths)?;
    let port = engine_db_port(&state.stack, &engine);

    let mut cmd = crate::services::process::command(&php_rt.php_bin);
    let ini = php_rt.root.join("php.ini");
    if ini.exists() {
        cmd.arg("-c").arg(&ini);
    }
    cmd.arg(&script)
        .env("DS_DB_HOST", "127.0.0.1")
        .env("DS_DB_PORT", port.to_string())
        .env("DS_DB_USER", "root")
        .env("DS_DB_PASS", "")
        .env("DS_DB_NAME", db)
        .env("DS_URL_FROM", &from)
        .env("DS_URL_TO", &canonical)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(parent) = php_rt.php_bin.parent() {
        cmd.env(
            "PATH",
            format!(
                "{}{}{}",
                parent.display(),
                std::path::MAIN_SEPARATOR,
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    }
    let out = cmd.output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(DevStackError::Message(format!(
            "wp URL repair failed: {stderr} {stdout}"
        )));
    }

    let _ = sync_wp_config_db_host(paths, state, site);
    let msg = if from.is_empty() {
        format!("{}: siteurl/home set to {canonical}", site.domain)
    } else {
        format!("{}: {from} → {canonical}", site.domain)
    };
    crate::applog::info("wordpress", &msg);
    if !stdout.is_empty() {
        crate::applog::info("wordpress", &stdout);
    }
    Ok(msg)
}

/// On app start / after port remap: repair any WP site whose siteurl ≠ canonical.
pub fn repair_mismatched_wp_urls(paths: &Paths, state: &AppState) -> Result<Vec<String>> {
    let mut msgs = Vec::new();
    for site in &state.sites {
        if !site_is_wordpress(site) || site.database_name.is_none() {
            continue;
        }
        let engine = site
            .database_engine
            .clone()
            .or_else(|| state.active_database.clone())
            .unwrap_or_else(|| "mariadb".into());
        if !matches!(engine.as_str(), "mysql" | "mariadb") {
            continue;
        }
        let db = site.database_name.as_deref().unwrap();
        let canonical = canonical_site_url(site, state);
        let current = match read_wp_option(paths, state, &engine, db, "siteurl") {
            Ok(v) => v,
            Err(_) => continue,
        };
        if current.is_empty() || current == canonical {
            let _ = sync_wp_config_db_host(paths, state, site);
            continue;
        }
        match repair_site_url(paths, state, site) {
            Ok(m) => msgs.push(m),
            Err(e) => {
                crate::applog::warn(
                    "wordpress",
                    &format!("{}: auto URL repair skipped: {e}", site.domain),
                );
            }
        }
    }
    Ok(msgs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn canonical_uses_apache_ports_for_wp_shape() {
        let mut state = AppState::default();
        state.stack.apache_https_port = 8444;
        state.stack.apache_http_port = 8088;
        let site = SiteRecord {
            id: "x".into(),
            name: "x".into(),
            path: PathBuf::from("/tmp/x"),
            domain: "test.test".into(),
            php_version: None,
            node_version: None,
            https: true,
            web_server: Some("apache".into()),
            framework: Some("wordpress".into()),
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: crate::config::default_tld(),
            aliases: Vec::new(),
            wildcard: false,
        };
        assert_eq!(
            canonical_site_url(&site, &state),
            "https://test.test:8444"
        );
        let mut http_site = site;
        http_site.https = false;
        assert_eq!(
            canonical_site_url(&http_site, &state),
            "http://test.test:8088"
        );
    }
}
