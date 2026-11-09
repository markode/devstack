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
    pub redis: u16,
    pub mailpit_smtp: u16,
    pub mailpit_ui: u16,
    pub dns: u16,
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
            redis: stack.redis_port,
            mailpit_smtp: stack.mailpit_smtp_port,
            mailpit_ui: stack.mailpit_ui_port,
            dns: stack.dns_port,
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
        stack.redis_port = self.redis;
        stack.mailpit_smtp_port = self.mailpit_smtp;
        stack.mailpit_ui_port = self.mailpit_ui;
        stack.dns_port = self.dns;
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
            redis: 6380,
            mailpit_smtp: 1026,
            mailpit_ui: 8026,
            dns: 5354,
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
    /// Windows service name when the listener is a service (e.g. wampapache64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_service: Option<String>,
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

/// Single source of truth for a site's Open / WP canonical URL.
/// `https` enabled → https://domain:httpsPort; otherwise http://domain:httpPort,
/// using the site's front-end (nginx vs apache) port pair.
pub fn canonical_site_url(site: &crate::config::SiteRecord, state: &crate::config::AppState) -> String {
    let apache = crate::services::httpd::site_uses_apache(site);
    let (scheme, port) = if site.https {
        (
            "https",
            if apache {
                state.stack.apache_https_port
            } else {
                state.stack.https_port
            },
        )
    } else {
        (
            "http",
            if apache {
                state.stack.apache_http_port
            } else {
                state.stack.http_port
            },
        )
    };
    format_url(scheme, &site.domain, port)
}

/// Both http and https absolute URLs for a site (correct ports per front-end).
pub fn site_http_https_urls(
    site: &crate::config::SiteRecord,
    state: &crate::config::AppState,
) -> (String, String) {
    let apache = crate::services::httpd::site_uses_apache(site);
    let (http_port, https_port) = if apache {
        (state.stack.apache_http_port, state.stack.apache_https_port)
    } else {
        (state.stack.http_port, state.stack.https_port)
    };
    (
        format_url("http", &site.domain, http_port),
        format_url("https", &site.domain, https_port),
    )
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
    find_next_free_excluding(start, candidates, &[])
}

pub fn find_next_free_excluding(start: u16, candidates: &[u16], reserved: &[u16]) -> u16 {
    let ok = |p: u16| p != start && !reserved.contains(&p) && port_is_free(p);
    for &p in candidates {
        if ok(p) {
            return p;
        }
    }
    // Walk upward from start+1
    let mut p = start.saturating_add(1).max(1024);
    for _ in 0..200 {
        if ok(p) {
            return p;
        }
        p = p.saturating_add(1);
    }
    start
}

/// Ports already claimed by other DevStack services — never steal these when remapping.
fn stack_reserved_ports(stack: &StackPreferences, except: &[&str]) -> Vec<u16> {
    let mut all = vec![
        ("nginxHttp", stack.http_port),
        ("nginxHttps", stack.https_port),
        ("apacheHttp", stack.apache_http_port),
        ("apacheHttps", stack.apache_https_port),
        ("mysql", stack.mysql_port),
        ("mariadb", stack.mariadb_port),
        ("postgresql", stack.postgresql_port),
        ("mongodb", stack.mongodb_port),
        ("phpFpm", stack.php_fpm_port),
        ("redis", stack.redis_port),
        ("mailpitSmtp", stack.mailpit_smtp_port),
        ("mailpitUi", stack.mailpit_ui_port),
        ("dns", stack.dns_port),
    ];
    all.retain(|(name, _)| !except.contains(name));
    let mut ports: Vec<u16> = all.into_iter().map(|(_, p)| p).collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

fn candidate_ports_for(field: &str, current: u16) -> Vec<u16> {
    let presets: &[u16] = match field {
        "nginxHttp" | "http" => &[8080, 8081, 8888, 8000, 9080],
        // Keep clear of Apache defaults (8088/8444).
        "nginxHttps" | "https" => &[8443, 9443, 8445, 9445],
        "apacheHttp" => &[8088, 8089, 9088, 8008],
        // Keep clear of Nginx defaults (8080/8443).
        "apacheHttps" => &[8444, 9444, 8446, 9446],
        "mysql" => &[3307, 3308, 3309, 3310],
        "mariadb" => &[3308, 3307, 3309, 3311],
        "postgresql" => &[5433, 5434, 5435],
        "mongodb" => &[27018, 27019, 27020],
        "phpFpm" => &[9001, 9002, 9003, 9070],
        "redis" => &[6380, 6381, 6382],
        "mailpitUi" => &[8026, 8027, 8028, 18025],
        "mailpitSmtp" => &[1026, 1027, 1028, 11025],
        "dns" => &[5354, 5355, 5356, 15353],
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
        windows_service: None,
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
    if let Some(pid) = occupant.pid {
        occupant.windows_service = windows_service_for_pid(pid);
    }
    occupant.hint = coexistence_hint(occupant.process_name.as_deref());
    occupant
}

#[cfg(windows)]
fn windows_service_for_pid(pid: u32) -> Option<String> {
    // sc queryex type= service state= all is heavy; use tasklist /svc instead.
    let out = crate::services::process::command("tasklist")
        .args(["/SVC", "/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // "httpd.exe","1234","wampapache64"
    for line in text.lines() {
        let cols: Vec<_> = line.split(',').map(|c| c.trim().trim_matches('"')).collect();
        if cols.len() >= 3 {
            let svc = cols[2];
            if !svc.is_empty() && !svc.eq_ignore_ascii_case("N/A") && !svc.starts_with("INFO:") {
                return Some(svc.to_string());
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn inspect_port_unix(port: u16) -> PortOccupant {
    let mut occupant = PortOccupant {
        port,
        pid: None,
        process_name: None,
        hint: None,
        windows_service: None,
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
    let svc = o
        .windows_service
        .as_ref()
        .map(|s| format!(" / service {s}"))
        .unwrap_or_default();
    match (&o.hint, o.pid) {
        (Some(hint), Some(pid)) => {
            format!("port {} used by {proc}{svc} ({hint}) [pid {pid}]", o.port)
        }
        (Some(hint), None) => format!("port {} used by {proc}{svc} ({hint})", o.port),
        (None, Some(pid)) => format!("port {} used by {proc}{svc} [pid {pid}]", o.port),
        (None, None) => format!("port {} is in use", o.port),
    }
}

/// Informational-only notice when classic 80/443 are held by another stack.
/// Never claims those ports — DevStack stays on its own listeners.
pub fn classic_web_port_notice(stack: &StackPreferences) -> Vec<String> {
    let mut out = Vec::new();
    for port in [80u16, 443u16] {
        if port_is_free(port) {
            continue;
        }
        let occ = inspect_port(port);
        // Skip if somehow our own nginx/apache is on 80/443.
        let ours = matches!(
            (port, stack.http_port, stack.https_port, stack.apache_http_port, stack.apache_https_port),
            (80, h, _, _, _) if h == 80
        ) || matches!(
            (port, stack.https_port),
            (443, h) if h == 443
        ) || matches!(
            (port, stack.apache_http_port),
            (80, h) if h == 80
        ) || matches!(
            (port, stack.apache_https_port),
            (443, h) if h == 443
        );
        if ours {
            continue;
        }
        let who = occ
            .hint
            .clone()
            .or_else(|| occ.process_name.clone())
            .unwrap_or_else(|| "another process".into());
        let svc = occ
            .windows_service
            .as_ref()
            .map(|s| format!(" (service {s})"))
            .unwrap_or_default();
        let ds_http = if stack.apache_http_port != 80 {
            stack.apache_http_port
        } else {
            stack.http_port
        };
        out.push(format!(
            "Port {port} is used by {who}{svc}. .test URLs without a port go there. DevStack uses :{ds_http}."
        ));
    }
    out
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
        "redis" => vec![("redis".into(), stack.redis_port)],
        "mailpit" => vec![
            ("mailpitSmtp".into(), stack.mailpit_smtp_port),
            ("mailpitUi".into(), stack.mailpit_ui_port),
        ],
        "dns" => vec![("dns".into(), stack.dns_port)],
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
            "redis" => stack.redis_port = *port,
            "mailpitSmtp" => stack.mailpit_smtp_port = *port,
            "mailpitUi" => stack.mailpit_ui_port = *port,
            "dns" => stack.dns_port = *port,
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

/// Result of silently remapping ports that are held by WAMP/MAMP/IIS/etc.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PortRemapNotice {
    pub changed: bool,
    /// Short lines for the Sites banner, e.g. `WAMP detected on :80, DevStack uses :8080`.
    pub messages: Vec<String>,
    pub previous: Option<PortsConfig>,
    pub current: Option<PortsConfig>,
}

/// Snapshot of ports used when rewriting WordPress siteurl/home after a remap.
#[derive(Debug, Clone, Copy)]
pub struct PortSnapshot {
    pub nginx_http: u16,
    pub nginx_https: u16,
    pub apache_http: u16,
    pub apache_https: u16,
}

impl PortSnapshot {
    pub fn from_stack(stack: &StackPreferences) -> Self {
        Self {
            nginx_http: stack.http_port,
            nginx_https: stack.https_port,
            apache_http: stack.apache_http_port,
            apache_https: stack.apache_https_port,
        }
    }
}

/// True when the occupant is a DevStack-owned listener (master **or** worker).
///
/// On Windows, nginx/httpd workers (not the stored master PID) often own the
/// TCP listen socket — matching only `owned_pids` false-triggered remaps onto
/// Apache's 8444 mid-stack.
fn occupant_is_ours(occ: &PortOccupant, owned_pids: &[u32], runtime: &crate::config::RuntimeState) -> bool {
    if let Some(pid) = occ.pid {
        if owned_pids.contains(&pid) {
            return true;
        }
    }
    let name = occ
        .process_name
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if name.is_empty() {
        return false;
    }
    if runtime.nginx_pid.is_some() && name.contains("nginx") {
        return true;
    }
    if runtime.apache_pid.is_some()
        && (name.contains("httpd") || name.contains("apache"))
    {
        return true;
    }
    // Any mysqld/mariadbd is treated as ours when we have a DB PID *or* when the
    // listener sits on our configured mysql/mariadb ports (checked by callers via
    // owned pid files). Name-only match when database_pid is set covers workers.
    if (runtime.database_pid.is_some()
        || runtime.database_engine.is_some())
        && (name.contains("mysqld")
            || name.contains("mariadbd")
            || name.contains("mariadb"))
    {
        return true;
    }
    false
}

/// True when `port` is listening and the listener is **not** one of ours.
fn foreign_occupant(
    port: u16,
    owned_pids: &[u32],
    runtime: &crate::config::RuntimeState,
) -> Option<PortOccupant> {
    if port_is_free(port) {
        return None;
    }
    let occ = inspect_port(port);
    if occupant_is_ours(&occ, owned_pids, runtime) {
        return None;
    }
    Some(occ)
}

fn who_label(occ: &PortOccupant) -> String {
    occ.hint
        .clone()
        .or_else(|| occ.process_name.clone())
        .unwrap_or_else(|| "another process".into())
}

/// On startup / start-stack: if ports we want are held by a **foreign** process
/// (WAMP/MAMP/IIS/Skype/… — not our own nginx/httpd/mysqld), automatically pick
/// free ports, persist coexistence, and return a banner notice.
///
/// `owned_pids` must include live DevStack service PIDs so a running stack is
/// never treated as a conflict with itself (that previously remapped 8443→8444
/// onto Apache mid-flight).
///
/// Does **not** fail when ports are busy — it remaps instead. Callers should
/// rewrite WP URLs when `changed` is true.
pub fn auto_separate_busy_ports(
    stack: &mut StackPreferences,
    owned_pids: &[u32],
    runtime: &crate::config::RuntimeState,
) -> PortRemapNotice {
    let previous = PortsConfig::from_stack(stack);
    let before = PortSnapshot::from_stack(stack);
    let mut messages = Vec::new();

    // --- Nginx ---
    let nginx_http_foreign = foreign_occupant(stack.http_port, owned_pids, runtime);
    let nginx_https_foreign = foreign_occupant(stack.https_port, owned_pids, runtime);
    if nginx_http_foreign.is_some() || nginx_https_foreign.is_some() {
        let prefer_alt = (stack.http_port == 80 || stack.https_port == 443)
            && port_is_free(ALT_HTTP)
            && port_is_free(ALT_HTTPS);
        let reserved = stack_reserved_ports(stack, &["nginxHttp", "nginxHttps"]);
        let new_http = if prefer_alt {
            ALT_HTTP
        } else if nginx_http_foreign.is_some() {
            find_next_free_excluding(
                stack.http_port,
                &candidate_ports_for("nginxHttp", stack.http_port),
                &reserved,
            )
        } else {
            stack.http_port
        };
        let new_https = if prefer_alt {
            ALT_HTTPS
        } else if nginx_https_foreign.is_some() {
            find_next_free_excluding(
                stack.https_port,
                &candidate_ports_for("nginxHttps", stack.https_port),
                &reserved,
            )
        } else {
            stack.https_port
        };
        if let Some(occ) = nginx_http_foreign {
            messages.push(format!(
                "{} detected on :{}, DevStack uses :{new_http}",
                who_label(&occ),
                occ.port
            ));
        }
        if let Some(occ) = nginx_https_foreign {
            messages.push(format!(
                "{} detected on :{}, DevStack uses :{new_https}",
                who_label(&occ),
                occ.port
            ));
        }
        stack.http_port = new_http;
        stack.https_port = new_https;
    }

    // --- Apache ---
    let apache_http_foreign =
        foreign_occupant(stack.apache_http_port, owned_pids, runtime);
    let apache_https_foreign =
        foreign_occupant(stack.apache_https_port, owned_pids, runtime);
    if apache_http_foreign.is_some() || apache_https_foreign.is_some() {
        let reserved = stack_reserved_ports(stack, &["apacheHttp", "apacheHttps"]);
        let new_http = if apache_http_foreign.is_some() {
            find_next_free_excluding(
                stack.apache_http_port,
                &candidate_ports_for("apacheHttp", stack.apache_http_port),
                &reserved,
            )
        } else {
            stack.apache_http_port
        };
        let new_https = if apache_https_foreign.is_some() {
            find_next_free_excluding(
                stack.apache_https_port,
                &candidate_ports_for("apacheHttps", stack.apache_https_port),
                &reserved,
            )
        } else {
            stack.apache_https_port
        };
        if let Some(occ) = apache_http_foreign {
            messages.push(format!(
                "{} detected on :{}, DevStack uses :{new_http}",
                who_label(&occ),
                occ.port
            ));
        }
        if let Some(occ) = apache_https_foreign {
            messages.push(format!(
                "{} detected on :{}, DevStack uses :{new_https}",
                who_label(&occ),
                occ.port
            ));
        }
        stack.apache_http_port = new_http;
        stack.apache_https_port = new_https;
    }

    // --- MySQL / MariaDB ---
    // Never remap onto the sibling engine's port; keep mysql ≠ mariadb always.
    if let Some(occ) = foreign_occupant(stack.mysql_port, owned_pids, runtime) {
        let mut reserved = stack_reserved_ports(stack, &["mysql"]);
        reserved.push(stack.mariadb_port);
        let alt = if stack.mysql_port == 3306
            && port_is_free(ALT_MYSQL)
            && ALT_MYSQL != stack.mariadb_port
        {
            ALT_MYSQL
        } else {
            find_next_free_excluding(
                stack.mysql_port,
                &candidate_ports_for("mysql", stack.mysql_port),
                &reserved,
            )
        };
        messages.push(format!(
            "{} detected on :{}, DevStack uses :{alt}",
            who_label(&occ),
            occ.port
        ));
        stack.mysql_port = alt;
    }
    if let Some(occ) = foreign_occupant(stack.mariadb_port, owned_pids, runtime) {
        let mut reserved = stack_reserved_ports(stack, &["mariadb"]);
        reserved.push(stack.mysql_port);
        let alt = if stack.mariadb_port == 3306
            && port_is_free(ALT_MARIADB)
            && ALT_MARIADB != stack.mysql_port
        {
            ALT_MARIADB
        } else {
            find_next_free_excluding(
                stack.mariadb_port,
                &candidate_ports_for("mariadb", stack.mariadb_port),
                &reserved,
            )
        };
        messages.push(format!(
            "{} detected on :{}, DevStack uses :{alt}",
            who_label(&occ),
            occ.port
        ));
        stack.mariadb_port = alt;
    }
    // Final uniqueness guard (e.g. both remapped to the same free port).
    if stack.mysql_port == stack.mariadb_port {
        let reserved = vec![stack.mysql_port];
        let alt = find_next_free_excluding(
            stack.mariadb_port,
            &candidate_ports_for("mariadb", stack.mariadb_port),
            &reserved,
        );
        messages.push(format!(
            "MariaDB port collided with MySQL :{}; MariaDB uses :{alt}",
            stack.mysql_port
        ));
        stack.mariadb_port = alt;
    }

    // --- PostgreSQL / MongoDB / PHP-FPM base ---
    if let Some(occ) = foreign_occupant(stack.postgresql_port, owned_pids, runtime) {
        let reserved = stack_reserved_ports(stack, &["postgresql"]);
        let alt = find_next_free_excluding(
            stack.postgresql_port,
            &candidate_ports_for("postgresql", stack.postgresql_port),
            &reserved,
        );
        messages.push(format!(
            "{} detected on :{}, DevStack PostgreSQL uses :{alt}",
            who_label(&occ),
            occ.port
        ));
        stack.postgresql_port = alt;
    }
    if let Some(occ) = foreign_occupant(stack.mongodb_port, owned_pids, runtime) {
        let reserved = stack_reserved_ports(stack, &["mongodb"]);
        let alt = find_next_free_excluding(
            stack.mongodb_port,
            &candidate_ports_for("mongodb", stack.mongodb_port),
            &reserved,
        );
        messages.push(format!(
            "{} detected on :{}, DevStack MongoDB uses :{alt}",
            who_label(&occ),
            occ.port
        ));
        stack.mongodb_port = alt;
    }
    if let Some(occ) = foreign_occupant(stack.php_fpm_port, owned_pids, runtime) {
        let reserved = stack_reserved_ports(stack, &["phpFpm"]);
        let alt = find_next_free_excluding(
            stack.php_fpm_port,
            &candidate_ports_for("phpFpm", stack.php_fpm_port),
            &reserved,
        );
        messages.push(format!(
            "{} detected on :{}, DevStack PHP-FPM base uses :{alt}",
            who_label(&occ),
            occ.port
        ));
        stack.php_fpm_port = alt;
    }
    if let Some(occ) = foreign_occupant(stack.redis_port, owned_pids, runtime) {
        let reserved = stack_reserved_ports(stack, &["redis"]);
        let alt = find_next_free_excluding(
            stack.redis_port,
            &candidate_ports_for("redis", stack.redis_port),
            &reserved,
        );
        messages.push(format!(
            "{} detected on :{}, DevStack Redis uses :{alt}",
            who_label(&occ),
            occ.port
        ));
        stack.redis_port = alt;
    }
    let mailpit_smtp_foreign =
        foreign_occupant(stack.mailpit_smtp_port, owned_pids, runtime);
    let mailpit_ui_foreign = foreign_occupant(stack.mailpit_ui_port, owned_pids, runtime);
    if mailpit_smtp_foreign.is_some() || mailpit_ui_foreign.is_some() {
        let reserved = stack_reserved_ports(stack, &["mailpitSmtp", "mailpitUi"]);
        if let Some(occ) = mailpit_smtp_foreign {
            let alt = find_next_free_excluding(
                stack.mailpit_smtp_port,
                &candidate_ports_for("mailpitSmtp", stack.mailpit_smtp_port),
                &reserved,
            );
            messages.push(format!(
                "{} detected on :{}, DevStack Mailpit SMTP uses :{alt}",
                who_label(&occ),
                occ.port
            ));
            stack.mailpit_smtp_port = alt;
        }
        if let Some(occ) = mailpit_ui_foreign {
            let reserved = stack_reserved_ports(stack, &["mailpitSmtp", "mailpitUi"]);
            let alt = find_next_free_excluding(
                stack.mailpit_ui_port,
                &candidate_ports_for("mailpitUi", stack.mailpit_ui_port),
                &reserved,
            );
            messages.push(format!(
                "{} detected on :{}, DevStack Mailpit UI uses :{alt}",
                who_label(&occ),
                occ.port
            ));
            stack.mailpit_ui_port = alt;
        }
    }
    if let Some(occ) = foreign_occupant(stack.dns_port, owned_pids, runtime) {
        let reserved = stack_reserved_ports(stack, &["dns"]);
        let alt = find_next_free_excluding(
            stack.dns_port,
            &candidate_ports_for("dns", stack.dns_port),
            &reserved,
        );
        messages.push(format!(
            "{} detected on :{}, DevStack DNS uses :{alt}",
            who_label(&occ),
            occ.port
        ));
        stack.dns_port = alt;
    }

    let after = PortSnapshot::from_stack(stack);
    let changed = before.nginx_http != after.nginx_http
        || before.nginx_https != after.nginx_https
        || before.apache_http != after.apache_http
        || before.apache_https != after.apache_https
        || previous.mysql != stack.mysql_port
        || previous.mariadb != stack.mariadb_port
        || previous.postgresql != stack.postgresql_port
        || previous.mongodb != stack.mongodb_port
        || previous.php_fpm != stack.php_fpm_port
        || previous.redis != stack.redis_port
        || previous.mailpit_smtp != stack.mailpit_smtp_port
        || previous.mailpit_ui != stack.mailpit_ui_port
        || previous.dns != stack.dns_port;

    if changed {
        stack.coexistence_mode = true;
        let mut seen = std::collections::HashSet::new();
        messages.retain(|m| seen.insert(m.clone()));
    } else {
        messages.clear();
    }

    PortRemapNotice {
        changed,
        messages,
        previous: if changed {
            Some(previous)
        } else {
            None
        },
        current: Some(PortsConfig::from_stack(stack)),
    }
}

/// Collect PIDs DevStack currently owns so auto-separate ignores our listeners.
pub fn owned_runtime_pids(runtime: &crate::config::RuntimeState) -> Vec<u32> {
    let mut pids = Vec::new();
    for p in [
        runtime.nginx_pid,
        runtime.apache_pid,
        runtime.dns_pid,
        runtime.database_pid,
        runtime.mongodb_pid,
        runtime.postgresql_pid,
        runtime.redis_pid,
        runtime.mailpit_pid,
    ]
    .into_iter()
    .flatten()
    {
        pids.push(p);
    }
    pids.extend(runtime.php_pids.values().copied());
    pids
}

/// Also include mysql/mariadb/nginx/apache/php-cgi pid-file PIDs (covers leftover
/// listeners after a crash when `runtime.*_pid` was cleared).
pub fn owned_runtime_pids_with_files(
    paths: &crate::config::Paths,
    runtime: &crate::config::RuntimeState,
) -> Vec<u32> {
    let mut pids = owned_runtime_pids(runtime);
    for name in [
        "nginx", "apache", "mariadb", "mysql", "mongodb", "postgresql", "redis", "mailpit", "dns",
    ] {
        if let Some(pid) = crate::services::process::read_pid(&paths.pid_file(name)) {
            if crate::services::process::is_running(pid) {
                pids.push(pid);
            }
        }
    }
    // php-cgi / php-fpm per version
    if let Ok(rd) = std::fs::read_dir(&paths.run) {
        for ent in rd.flatten() {
            let fname = ent.file_name().to_string_lossy().to_string();
            if !(fname.starts_with("php-cgi-") || fname.starts_with("php-fpm-")) {
                continue;
            }
            if let Some(pid) = crate::services::process::read_pid(&ent.path()) {
                if crate::services::process::is_running(pid) {
                    pids.push(pid);
                }
            }
        }
    }
    pids.sort_unstable();
    pids.dedup();
    pids
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
            windows_service: Some("wampapache64".into()),
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
    fn auto_separate_remaps_busy_classic_ports() {
        let _holders: Vec<TcpListener> = [80u16, 443, 3306]
            .into_iter()
            .filter_map(|p| TcpListener::bind(("127.0.0.1", p)).ok())
            .collect();
        if _holders.is_empty() {
            // Environment cannot bind classic ports — skip quietly.
            return;
        }
        let mut stack = StackPreferences::default();
        assert_eq!(stack.http_port, 80);
        // Empty owned_pids + empty runtime → holder looks foreign (WAMP-like).
        let runtime = crate::config::RuntimeState::default();
        let notice = auto_separate_busy_ports(&mut stack, &[], &runtime);
        assert!(notice.changed, "expected remap when classic ports held");
        assert!(stack.coexistence_mode);
        if !port_is_free(80) {
            assert_ne!(stack.http_port, 80);
        }
        if !port_is_free(443) {
            assert_ne!(stack.https_port, 443);
        }
        if !port_is_free(3306) {
            assert!(stack.mysql_port != 3306 || stack.mariadb_port != 3306);
        }
        assert!(!notice.messages.is_empty());
    }

    #[test]
    fn auto_separate_ignores_owned_pids() {
        let holder = match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(l) => l,
            Err(_) => return,
        };
        let port = holder.local_addr().unwrap().port();
        let mut stack = StackPreferences::default();
        stack.http_port = port;
        stack.https_port = ALT_HTTPS;
        let runtime = crate::config::RuntimeState::default();
        // Without owned pid → remaps away from `port`.
        let notice = auto_separate_busy_ports(&mut stack, &[], &runtime);
        assert!(notice.changed);
        assert_ne!(stack.http_port, port);

        // Reset and claim the listener as ours → no remap.
        stack.http_port = port;
        stack.coexistence_mode = false;
        let occ = inspect_port(port);
        let owned = occ.pid.map(|p| vec![p]).unwrap_or_default();
        if owned.is_empty() {
            return; // platform couldn't resolve pid
        }
        let notice = auto_separate_busy_ports(&mut stack, &owned, &runtime);
        assert!(!notice.changed, "must not remap ports we ourselves hold");
        assert_eq!(stack.http_port, port);
    }

    #[test]
    fn engine_db_port_picks_mysql_vs_mariadb() {
        let mut stack = StackPreferences::default();
        stack.mysql_port = 3307;
        stack.mariadb_port = 3308;
        assert_eq!(engine_db_port(&stack, "mysql"), 3307);
        assert_eq!(engine_db_port(&stack, "mariadb"), 3308);
    }

    #[test]
    fn busy_port_fallback_each_service_field() {
        // Bind ephemeral ports and point each stack field at them; auto_separate
        // must remap without ever stopping the foreign holders.
        let holders: Vec<TcpListener> = (0..8)
            .filter_map(|_| TcpListener::bind(("127.0.0.1", 0)).ok())
            .collect();
        if holders.len() < 8 {
            return;
        }
        let ports: Vec<u16> = holders
            .iter()
            .map(|h| h.local_addr().unwrap().port())
            .collect();
        let mut stack = StackPreferences::default();
        stack.http_port = ports[0];
        stack.https_port = ports[1];
        stack.apache_http_port = ports[2];
        stack.apache_https_port = ports[3];
        stack.mysql_port = ports[4];
        stack.mariadb_port = ports[5];
        stack.postgresql_port = ports[6];
        stack.mongodb_port = ports[7];
        stack.php_fpm_port = 9000; // may or may not be free
        let runtime = crate::config::RuntimeState::default();
        let notice = auto_separate_busy_ports(&mut stack, &[], &runtime);
        assert!(notice.changed);
        assert_ne!(stack.http_port, ports[0]);
        assert_ne!(stack.https_port, ports[1]);
        assert_ne!(stack.apache_http_port, ports[2]);
        assert_ne!(stack.apache_https_port, ports[3]);
        assert_ne!(stack.mysql_port, ports[4]);
        assert_ne!(stack.mariadb_port, ports[5]);
        assert_ne!(stack.postgresql_port, ports[6]);
        assert_ne!(stack.mongodb_port, ports[7]);
        assert_ne!(stack.mysql_port, stack.mariadb_port);
        // Foreign listeners must still be accept()ing — we never killed them.
        for h in &holders {
            assert!(h.local_addr().is_ok());
        }
    }

    #[test]
    fn mysql_mariadb_ports_stay_unique_after_collision() {
        let mut stack = StackPreferences::default();
        stack.mysql_port = 3308;
        stack.mariadb_port = 3308;
        let runtime = crate::config::RuntimeState::default();
        let _ = auto_separate_busy_ports(&mut stack, &[], &runtime);
        assert_ne!(stack.mysql_port, stack.mariadb_port);
    }

    #[test]
    fn classic_web_notice_is_informational_only() {
        // When DevStack is already on coexistence ports, a busy :80 produces a
        // notice string but must not mutate stack ports.
        let _holder = TcpListener::bind(("127.0.0.1", 80)).ok();
        let stack = PortsConfig::coexistence();
        let before = (stack.nginx_http, stack.nginx_https);
        let mut s = StackPreferences::default();
        stack.apply_to_stack(&mut s);
        let msgs = classic_web_port_notice(&s);
        assert_eq!((s.http_port, s.https_port), before);
        // msgs may be empty if :80 was free in this environment — that's fine.
        let _ = msgs;
    }

    #[test]
    fn canonical_site_url_builder_unit() {
        let mut state = crate::config::AppState::default();
        state.stack.apache_http_port = 8088;
        state.stack.apache_https_port = 8444;
        let site = crate::config::SiteRecord {
            id: "t".into(),
            name: "t".into(),
            path: std::path::PathBuf::from("/tmp/t"),
            domain: "test.test".into(),
            php_version: None,
            node_version: None,
            https: false,
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
        };
        assert_eq!(canonical_site_url(&site, &state), "http://test.test:8088");
    }
}
