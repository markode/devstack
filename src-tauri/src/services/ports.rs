//! Configurable listen ports + coexistence helpers for WAMP / MAMP PRO.
//!
//! Defaults match classic Herd/MAMP layouts. Coexistence mode shifts web and DB
//! ports so DevStack can run beside system stacks that own 80/443/3306.

use std::collections::BTreeMap;
use std::net::TcpListener;

use serde::{Deserialize, Serialize};

use crate::config::StackPreferences;
use crate::error::{DevStackError, Result};

/// Suggested alternate when the preferred port is taken (coexistence / next-free).
pub const ALT_HTTP: u16 = 8080;
pub const ALT_HTTPS: u16 = 8443;
pub const ALT_MYSQL: u16 = 3307;
pub const ALT_MARIADB: u16 = 3308;
pub const ALT_POSTGRES: u16 = 5433;
pub const ALT_MONGO: u16 = 27018;
pub const ALT_PHP_FPM: u16 = 9001;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortsConfig {
    pub coexistence_mode: bool,
    pub nginx_http: u16,
    pub nginx_https: u16,
    pub apache_http: u16,
    pub apache_https: u16,
    pub mysql: u16,
    pub mariadb: u16,
    pub postgresql: u16,
    pub mongodb: u16,
    pub php_fpm: u16,
    /// URL port for phpMyAdmin (served by Nginx — mirrors nginx HTTPS).
    pub phpmyadmin: u16,
}

impl PortsConfig {
    pub fn from_stack(stack: &StackPreferences) -> Self {
        Self {
            coexistence_mode: stack.coexistence_mode,
            nginx_http: stack.http_port,
            nginx_https: stack.https_port,
            apache_http: stack.apache_http_port,
            apache_https: stack.apache_https_port,
            mysql: stack.mysql_port,
            mariadb: stack.mariadb_port,
            postgresql: stack.postgresql_port,
            mongodb: stack.mongodb_port,
            php_fpm: stack.php_fpm_port,
            phpmyadmin: stack.https_port,
        }
    }

    pub fn apply_to_stack(&self, stack: &mut StackPreferences) {
        stack.coexistence_mode = self.coexistence_mode;
        stack.http_port = self.nginx_http;
        stack.https_port = self.nginx_https;
        stack.apache_http_port = self.apache_http;
        stack.apache_https_port = self.apache_https;
        stack.mysql_port = self.mysql;
        stack.mariadb_port = self.mariadb;
        stack.postgresql_port = self.postgresql;
        stack.mongodb_port = self.mongodb;
        stack.php_fpm_port = self.php_fpm;
    }

    pub fn defaults() -> Self {
        Self::from_stack(&StackPreferences::default())
    }

    pub fn coexistence() -> Self {
        Self {
            coexistence_mode: true,
            nginx_http: ALT_HTTP,
            nginx_https: ALT_HTTPS,
            apache_http: 8088,
            apache_https: 8444,
            mysql: ALT_MYSQL,
            mariadb: ALT_MARIADB,
            postgresql: ALT_POSTGRES,
            mongodb: ALT_MONGO,
            php_fpm: ALT_PHP_FPM,
            phpmyadmin: ALT_HTTPS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortOccupant {
    pub port: u16,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortCheckResult {
    pub service_id: String,
    pub ok: bool,
    pub conflicts: Vec<PortOccupant>,
    /// Human-readable summary, e.g. `port 80 used by httpd.exe (WAMP)`.
    pub message: Option<String>,
    /// Suggested replacement ports keyed by config field name.
    pub suggested: BTreeMap<String, u16>,
}

/// Format a public URL, omitting the port for 80/443.
pub fn format_url(scheme: &str, host: &str, port: u16) -> String {
    let default = match scheme {
        "https" => 443,
        _ => 80,
    };
    if port == default {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

/// MySQL/MariaDB TCP host string for WP-CLI / clients (`127.0.0.1` or `127.0.0.1:3307`).
pub fn mysql_db_host(port: u16) -> String {
    if port == 3306 {
        "127.0.0.1".into()
    } else {
        format!("127.0.0.1:{port}")
    }
}

pub fn engine_db_port(stack: &StackPreferences, engine: &str) -> u16 {
    match engine {
        "mysql" => stack.mysql_port,
        _ => stack.mariadb_port,
    }
}

/// True when nothing is accepting connections on 127.0.0.1:port AND we can bind it.
pub fn port_is_free(port: u16) -> bool {
    if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
        return false;
    }
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn find_next_free(start: u16, candidates: &[u16]) -> u16 {
    for &p in candidates {
        if p != start && port_is_free(p) {
            return p;
        }
    }
    // Walk upward from start+1
    let mut p = start.saturating_add(1).max(1024);
    for _ in 0..200 {
        if port_is_free(p) {
            return p;
        }
        p = p.saturating_add(1);
    }
    start
}

fn candidate_ports_for(field: &str, current: u16) -> Vec<u16> {
    let presets: &[u16] = match field {
        "nginxHttp" | "http" => &[8080, 8081, 8888, 8000, 9080],
        "nginxHttps" | "https" => &[8443, 8444, 9443, 8445],
        "apacheHttp" => &[8088, 8089, 9088, 8008],
        "apacheHttps" => &[8444, 8445, 9444, 8446],
        "mysql" => &[3307, 3308, 3309, 3310],
        "mariadb" => &[3308, 3307, 3309, 3311],
        "postgresql" => &[5433, 5434, 5435],
        "mongodb" => &[27018, 27019, 27020],
        "phpFpm" => &[9001, 9002, 9003, 9070],
        _ => &[],
    };
    let mut out: Vec<u16> = presets.iter().copied().filter(|p| *p != current).collect();
    if out.is_empty() {
        out.push(current.saturating_add(1));
        out.push(current.saturating_add(10));
    }
    out
}

/// Identify which process holds a listening TCP port (best-effort).
pub fn inspect_port(port: u16) -> PortOccupant {
    #[cfg(windows)]
    {
        return inspect_port_windows(port);
    }
    #[cfg(not(windows))]
    {
        inspect_port_unix(port)
    }
}

#[cfg(windows)]
fn inspect_port_windows(port: u16) -> PortOccupant {
    let mut occupant = PortOccupant {
        port,
        pid: None,
        process_name: None,
        hint: None,
    };
    let out = crate::services::process::command("netstat")
        .args(["-ano"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let needle = format!(":{port}");
    for line in out.lines() {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("listen") {
            continue;
        }
        if !(line.contains(&needle)
            || line.contains(&format!("0.0.0.0:{port}"))
            || line.contains(&format!("127.0.0.1:{port}"))
            || line.contains(&format!("[::]:{port}")))
        {
            continue;
        }
        let pid = line.split_whitespace().last().and_then(|s| s.parse().ok());
        occupant.pid = pid;
        if let Some(pid) = pid {
            if let Ok(task) = crate::services::process::command("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
                .output()
            {
                let text = String::from_utf8_lossy(&task.stdout);
                // "httpd.exe","1234",...
                if let Some(name) = text.split(',').next() {
                    let name = name.trim().trim_matches('"').to_string();
                    if !name.is_empty() && !name.starts_with("INFO:") {
                        occupant.process_name = Some(name);
                    }
                }
            }
        }
        break;
    }
    occupant.hint = coexistence_hint(occupant.process_name.as_deref());
    occupant
}

#[cfg(not(windows))]
fn inspect_port_unix(port: u16) -> PortOccupant {
    let mut occupant = PortOccupant {
        port,
        pid: None,
        process_name: None,
        hint: None,
    };
    let out = crate::services::process::command("lsof")
        .args([
            "-nP",
            &format!("-iTCP:{port}"),
            "-sTCP:LISTEN",
            "-Fpc",
        ])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            occupant.pid = rest.parse().ok();
        } else if let Some(rest) = line.strip_prefix('c') {
            if !rest.is_empty() {
                occupant.process_name = Some(rest.to_string());
            }
        }
    }
    // Fallback: lsof without -F
    if occupant.process_name.is_none() {
        if let Ok(raw) = crate::services::process::command("lsof")
            .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
            .output()
        {
            let text = String::from_utf8_lossy(&raw.stdout);
            for line in text.lines().skip(1) {
                let cols: Vec<_> = line.split_whitespace().collect();
                if cols.len() >= 2 {
                    occupant.process_name = Some(cols[0].to_string());
                    occupant.pid = cols[1].parse().ok();
                    break;
                }
            }
        }
    }
    occupant.hint = coexistence_hint(occupant.process_name.as_deref());
    occupant
}

/// Map process names to a friendly stack hint (WAMP / MAMP PRO / …).
pub fn coexistence_hint(process_name: Option<&str>) -> Option<String> {
    let name = process_name?.to_ascii_lowercase();
    let mamp_installed = std::path::Path::new("/Applications/MAMP").exists()
        || std::path::Path::new("/Applications/MAMP PRO.app").exists();

    if name.contains("wamp")
        || name == "httpd.exe"
        || name == "wampapache64.exe"
        || name == "wampmysqld.exe"
        || name == "wampmysqld64.exe"
    {
        return Some("WAMP".into());
    }
    if name.contains("mamp") {
        return Some("MAMP PRO".into());
    }
    // On macOS, plain `httpd` / bundled mysqld is often MAMP when installed.
    if mamp_installed
        && (name == "httpd" || name == "mysqld" || name.starts_with("mysql"))
    {
        return Some("MAMP PRO".into());
    }
    None
}

pub fn format_occupant(o: &PortOccupant) -> String {
    let proc = o
        .process_name
        .clone()
        .unwrap_or_else(|| "unknown process".into());
    match (&o.hint, o.pid) {
        (Some(hint), Some(pid)) => format!("port {} used by {proc} ({hint}) [pid {pid}]", o.port),
        (Some(hint), None) => format!("port {} used by {proc} ({hint})", o.port),
        (None, Some(pid)) => format!("port {} used by {proc} [pid {pid}]", o.port),
        (None, None) => format!("port {} is in use", o.port),
    }
}

/// Ports a service needs before it can start.
pub fn service_ports(stack: &StackPreferences, service_id: &str) -> Vec<(String, u16)> {
    match service_id {
        "nginx" => vec![
            ("nginxHttp".into(), stack.http_port),
            ("nginxHttps".into(), stack.https_port),
        ],
        "apache" => vec![
            ("apacheHttp".into(), stack.apache_http_port),
            ("apacheHttps".into(), stack.apache_https_port),
        ],
        "mysql" => vec![("mysql".into(), stack.mysql_port)],
        "mariadb" => vec![("mariadb".into(), stack.mariadb_port)],
        "postgresql" => vec![("postgresql".into(), stack.postgresql_port)],
        "mongodb" => vec![("mongodb".into(), stack.mongodb_port)],
        "php" | "php-fpm" => vec![("phpFpm".into(), stack.php_fpm_port)],
        "phpmyadmin" => vec![("nginxHttps".into(), stack.https_port)],
        _ => vec![],
    }
}

pub fn check_service_ports(stack: &StackPreferences, service_id: &str) -> PortCheckResult {
    let needed = service_ports(stack, service_id);
    let mut conflicts = Vec::new();
    let mut suggested = BTreeMap::new();

    for (field, port) in &needed {
        if port_is_free(*port) {
            continue;
        }
        // Port may be held by *our* already-running service — treat as ok when
        // we are restarting; callers that want a hard check pass running=false.
        let occ = inspect_port(*port);
        conflicts.push(occ);
        let next = find_next_free(*port, &candidate_ports_for(field, *port));
        suggested.insert(field.clone(), next);
    }

    // Pair suggestions for web servers when only one side conflicted.
    if service_id == "nginx" {
        if suggested.contains_key("nginxHttp") && !suggested.contains_key("nginxHttps") {
            suggested.insert(
                "nginxHttps".into(),
                find_next_free(stack.https_port, &candidate_ports_for("nginxHttps", stack.https_port)),
            );
        }
        if suggested.contains_key("nginxHttps") && !suggested.contains_key("nginxHttp") {
            suggested.insert(
                "nginxHttp".into(),
                find_next_free(stack.http_port, &candidate_ports_for("nginxHttp", stack.http_port)),
            );
        }
        // Prefer classic coexistence pair when both free.
        if conflicts.iter().any(|c| c.port == 80 || c.port == 443)
            && port_is_free(ALT_HTTP)
            && port_is_free(ALT_HTTPS)
        {
            suggested.insert("nginxHttp".into(), ALT_HTTP);
            suggested.insert("nginxHttps".into(), ALT_HTTPS);
        }
    }

    let message = if conflicts.is_empty() {
        None
    } else {
        Some(
            conflicts
                .iter()
                .map(format_occupant)
                .collect::<Vec<_>>()
                .join("; "),
        )
    };

    PortCheckResult {
        service_id: service_id.into(),
        ok: conflicts.is_empty(),
        conflicts,
        message,
        suggested,
    }
}

/// Apply suggested ports for a service onto stack preferences.
pub fn apply_suggested_ports(
    stack: &mut StackPreferences,
    suggested: &BTreeMap<String, u16>,
) -> Result<()> {
    for (field, port) in suggested {
        match field.as_str() {
            "nginxHttp" => stack.http_port = *port,
            "nginxHttps" => stack.https_port = *port,
            "apacheHttp" => stack.apache_http_port = *port,
            "apacheHttps" => stack.apache_https_port = *port,
            "mysql" => stack.mysql_port = *port,
            "mariadb" => stack.mariadb_port = *port,
            "postgresql" => stack.postgresql_port = *port,
            "mongodb" => stack.mongodb_port = *port,
            "phpFpm" => stack.php_fpm_port = *port,
            _ => {}
        }
    }
    Ok(())
}

/// Ensure required ports are free before starting; error with a clear message.
pub fn ensure_ports_free(stack: &StackPreferences, service_id: &str) -> Result<()> {
    let check = check_service_ports(stack, service_id);
    if check.ok {
        return Ok(());
    }
    let msg = check
        .message
        .unwrap_or_else(|| format!("ports for {service_id} are in use"));
    // Embed machine-readable JSON for the UI "Use next free port" button.
    let payload = serde_json::json!({
        "serviceId": service_id,
        "suggested": check.suggested,
        "conflicts": check.conflicts,
    });
    Err(DevStackError::Message(format!(
        "{msg} |DEVSTACK_PORT_BUSY:{payload}|"
    )))
}

pub fn parse_ports_patch(value: &serde_json::Value) -> Result<PortsConfig> {
    serde_json::from_value(value.clone())
        .map_err(|e| DevStackError::Message(format!("invalid ports config: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_url_omits_default_ports() {
        assert_eq!(format_url("http", "app.test", 80), "http://app.test");
        assert_eq!(format_url("https", "app.test", 443), "https://app.test");
        assert_eq!(
            format_url("https", "app.test", 8443),
            "https://app.test:8443"
        );
        assert_eq!(
            format_url("http", "app.test", 8080),
            "http://app.test:8080"
        );
    }

    #[test]
    fn mysql_db_host_includes_non_default_port() {
        assert_eq!(mysql_db_host(3306), "127.0.0.1");
        assert_eq!(mysql_db_host(3307), "127.0.0.1:3307");
    }

    #[test]
    fn coexistence_ports_match_spec() {
        let c = PortsConfig::coexistence();
        assert!(c.coexistence_mode);
        assert_eq!(c.nginx_http, 8080);
        assert_eq!(c.nginx_https, 8443);
        assert_eq!(c.mysql, 3307);
        assert_eq!(c.mariadb, 3308);
        assert_eq!(c.postgresql, 5433);
        assert_eq!(c.mongodb, 27018);
        assert_eq!(c.phpmyadmin, 8443);
    }

    #[test]
    fn apply_coexistence_to_stack_roundtrip() {
        let mut stack = StackPreferences::default();
        assert_eq!(stack.http_port, 80);
        PortsConfig::coexistence().apply_to_stack(&mut stack);
        assert!(stack.coexistence_mode);
        assert_eq!(stack.http_port, 8080);
        assert_eq!(stack.mysql_port, 3307);
        assert_eq!(stack.mariadb_port, 3308);
        let cfg = PortsConfig::from_stack(&stack);
        assert_eq!(cfg.nginx_https, 8443);
        assert_eq!(cfg.phpmyadmin, 8443);
    }

    #[test]
    fn format_occupant_includes_hint() {
        let o = PortOccupant {
            port: 80,
            pid: Some(4242),
            process_name: Some("httpd.exe".into()),
            hint: Some("WAMP".into()),
        };
        let s = format_occupant(&o);
        assert!(s.contains("port 80"));
        assert!(s.contains("httpd.exe"));
        assert!(s.contains("WAMP"));
        assert!(s.contains("4242"));
    }

    #[test]
    fn coexistence_hint_detects_wamp() {
        assert_eq!(
            coexistence_hint(Some("httpd.exe")).as_deref(),
            Some("WAMP")
        );
        assert_eq!(
            coexistence_hint(Some("wampmysqld64.exe")).as_deref(),
            Some("WAMP")
        );
    }

    #[test]
    fn service_ports_nginx_lists_both() {
        let stack = StackPreferences::default();
        let ports = service_ports(&stack, "nginx");
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].1, 80);
        assert_eq!(ports[1].1, 443);
    }

    #[test]
    fn apply_suggested_updates_stack() {
        let mut stack = StackPreferences::default();
        let mut suggested = BTreeMap::new();
        suggested.insert("nginxHttp".into(), 8080);
        suggested.insert("mysql".into(), 3307);
        apply_suggested_ports(&mut stack, &suggested).unwrap();
        assert_eq!(stack.http_port, 8080);
        assert_eq!(stack.mysql_port, 3307);
    }

    #[test]
    fn engine_db_port_picks_mysql_vs_mariadb() {
        let mut stack = StackPreferences::default();
        stack.mysql_port = 3307;
        stack.mariadb_port = 3308;
        assert_eq!(engine_db_port(&stack, "mysql"), 3307);
        assert_eq!(engine_db_port(&stack, "mariadb"), 3308);
    }
}
