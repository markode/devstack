//! Site detection (WordPress / Laravel) and detail helpers.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use regex::Regex;
use serde::Serialize;

use crate::config::{slugify, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::nginx::expand_home;
use crate::services::process::is_running;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteReachability {
    /// `running` | `stopped` | `error` | `warning`
    pub state: String,
    pub message: Option<String>,
    pub tcp_ok: bool,
    pub http_ok: bool,
    pub probed_url: String,
    pub probed_port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteDetails {
    pub site: SiteRecord,
    pub framework: String,
    pub framework_icon: String,
    pub absolute_path: PathBuf,
    pub url: String,
    pub https_url: String,
    pub database_name: Option<String>,
    pub database_user: Option<String>,
    pub phpmyadmin_url: Option<String>,
    pub has_wp_config: bool,
    #[serde(default)]
    pub reachability: Option<SiteReachability>,
}

pub fn detect_framework(path: &Path) -> (String, String) {
    let root = expand_home(path);
    if root.join("wp-config.php").exists() || root.join("wp-config-sample.php").exists() {
        return ("wordpress".into(), "WP".into());
    }
    if root.join("artisan").exists() && root.join("composer.json").exists() {
        return ("laravel".into(), "LV".into());
    }
    if root.join("composer.json").exists() {
        return ("php".into(), "PHP".into());
    }
    ("other".into(), "?".into())
}

pub fn parse_wp_config(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let cfg = expand_home(path).join("wp-config.php");
    let Ok(raw) = fs::read_to_string(&cfg) else {
        return (None, None, None);
    };
    let name = capture_define(&raw, "DB_NAME");
    let user = capture_define(&raw, "DB_USER");
    let pass = capture_define(&raw, "DB_PASSWORD");
    (name, user, pass)
}

fn capture_define(raw: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(
        r#"define\(\s*['\"]{key}['\"]\s*,\s*['\"]([^'\"]*)['\"]\s*\)"#
    ))
    .ok()?;
    re.captures(raw)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

pub fn detect_site(site: &SiteRecord) -> SiteDetails {
    let (framework, icon) = detect_framework(&site.path);
    let abs = expand_home(&site.path);
    let has_wp = abs.join("wp-config.php").exists();
    let (db_name, db_user, db_pass) = if framework == "wordpress" {
        parse_wp_config(&site.path)
    } else {
        (site.database_name.clone(), site.database_user.clone(), None)
    };

    // Placeholder only — real Open database mints a one-time token via
    // `mint_phpmyadmin_signon_url` so the password never appears in the URL.
    let pma = if framework == "wordpress" {
        db_name.as_ref().map(|db| {
            format!(
                "https://phpmyadmin.test/devstack-signon.php?db={}",
                urlencoding_minimal(db)
            )
        })
    } else {
        None
    };
    let _ = db_pass;

    let mut updated = site.clone();
    updated.framework = Some(framework.clone());
    if updated.database_name.is_none() {
        updated.database_name = db_name.clone();
    }
    if updated.database_user.is_none() {
        updated.database_user = db_user.clone();
    }

    let (http_port, https_port) = if crate::services::httpd::site_uses_apache(site) {
        (
            // Ports come from stack prefs; detect_site has no AppState — use defaults
            // overridden by list_site_details which rewrites URLs when state is available.
            8088u16,
            8444u16,
        )
    } else {
        (80u16, 443u16)
    };
    let url = crate::services::ports::format_url("http", &site.domain, http_port);
    let https_url = crate::services::ports::format_url("https", &site.domain, https_port);

    SiteDetails {
        site: updated,
        framework,
        framework_icon: icon,
        absolute_path: abs,
        url,
        https_url,
        database_name: db_name,
        database_user: db_user,
        phpmyadmin_url: pma,
        has_wp_config: has_wp,
        reachability: None,
    }
}

/// Rewrite site detail URLs using live stack port preferences.
pub fn apply_stack_ports(details: &mut SiteDetails, state: &AppState) {
    let apache = crate::services::httpd::site_uses_apache(&details.site);
    let (http_port, https_port) = if apache {
        (
            state.stack.apache_http_port,
            state.stack.apache_https_port,
        )
    } else {
        (state.stack.http_port, state.stack.https_port)
    };
    details.url = crate::services::ports::format_url("http", &details.site.domain, http_port);
    details.https_url =
        crate::services::ports::format_url("https", &details.site.domain, https_port);
    if let Some(pma) = details.phpmyadmin_url.as_mut() {
        // Rebuild host so non-default Nginx HTTPS ports appear in the URL.
        if let Some(rest) = pma.strip_prefix("https://phpmyadmin.test") {
            let base = crate::services::ports::format_url(
                "https",
                "phpmyadmin.test",
                state.stack.https_port,
            );
            // rest begins with `/…` or `:port/…`
            let path = if let Some(idx) = rest.find('/') {
                &rest[idx..]
            } else {
                ""
            };
            *pma = format!("{base}{path}");
        }
    }
}

fn site_listen_port(site: &SiteRecord, state: &AppState) -> (u16, bool) {
    let apache = crate::services::httpd::site_uses_apache(site);
    if site.https {
        (
            if apache {
                state.stack.apache_https_port
            } else {
                state.stack.https_port
            },
            true,
        )
    } else {
        (
            if apache {
                state.stack.apache_http_port
            } else {
                state.stack.http_port
            },
            false,
        )
    }
}

fn tail_log(path: &Path, max: usize) -> String {
    let Ok(raw) = fs::read_to_string(path) else {
        return String::new();
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let suffix: String = trimmed
        .chars()
        .rev()
        .take(max)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    suffix
}

fn server_log_hint(paths: &Paths, apache: bool) -> Option<String> {
    let candidates = if apache {
        vec![
            paths.logs.join("apache.err"),
            paths.logs.join("apache/error.log"),
        ]
    } else {
        vec![
            paths.logs.join("nginx/stderr.log"),
            paths.logs.join("nginx/error.log"),
        ]
    };
    for p in candidates {
        let t = tail_log(&p, 240);
        if !t.is_empty() {
            return Some(format!("{}: {t}", p.file_name()?.to_string_lossy()));
        }
    }
    None
}

fn http_probe_ok(host: &str, port: u16, https: bool) -> bool {
    // Prefer curl when available (handles TLS + Host). Fall back to raw HTTP.
    let url = if https {
        format!("https://{host}:{port}/")
    } else {
        format!("http://{host}:{port}/")
    };
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    if let Ok(out) = crate::services::process::command(curl)
        .args([
            "-k",
            "-sS",
            "--max-time",
            "1",
            "--connect-timeout",
            "1",
            "--resolve",
            &format!("{host}:{port}:127.0.0.1"),
            "-o",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
            "-w",
            "%{http_code}",
            &url,
        ])
        .output()
    {
        let code = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if code.starts_with('2') || code.starts_with('3') {
            return true;
        }
        // 4xx still means the server answered — treat as reachable for status.
        if code.starts_with('4') {
            return true;
        }
    }
    if https {
        return false;
    }
    // Raw HTTP GET fallback.
    let Ok(mut stream) =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(400))
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    matches!(stream.read(&mut buf), Ok(n) if n > 0)
}

/// TCP-connect the site's listen port and HTTP-probe the actual Open URL.
/// Updates `site.status` to match (`running` / `error` / `stopped`).
pub fn probe_site_reachability(
    paths: &Paths,
    state: &AppState,
    details: &mut SiteDetails,
) -> SiteReachability {
    let apache = crate::services::httpd::site_uses_apache(&details.site);
    let (port, https) = site_listen_port(&details.site, state);
    let probed_url = if https {
        details.https_url.clone()
    } else {
        details.url.clone()
    };
    let server_up = if apache {
        state
            .runtime
            .apache_pid
            .map(is_running)
            .unwrap_or(false)
    } else {
        state
            .runtime
            .nginx_pid
            .map(is_running)
            .unwrap_or(false)
    };
    let server_name = if apache { "Apache" } else { "Nginx" };

    if !server_up {
        let log = server_log_hint(paths, apache);
        let message = match log {
            Some(l) => format!("{server_name} is not running. {l}"),
            None => format!("{server_name} process is not running"),
        };
        details.site.status = "error".into();
        return SiteReachability {
            state: "error".into(),
            message: Some(message),
            tcp_ok: false,
            http_ok: false,
            probed_url,
            probed_port: port,
        };
    }

    let tcp_ok =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(350))
            .is_ok();
    if !tcp_ok {
        let log = server_log_hint(paths, apache);
        let message = match log {
            Some(l) => format!(
                "{server_name} is up but nothing listens on :{port}. {l}"
            ),
            None => format!(
                "{server_name} is up but nothing listens on 127.0.0.1:{port}"
            ),
        };
        details.site.status = "error".into();
        return SiteReachability {
            state: "error".into(),
            message: Some(message),
            tcp_ok: false,
            http_ok: false,
            probed_url,
            probed_port: port,
        };
    }

    let http_ok = http_probe_ok(&details.site.domain, port, https);
    if !http_ok {
        details.site.status = "warning".into();
        return SiteReachability {
            state: "warning".into(),
            message: Some(format!(
                "Port :{port} is open but {probed_url} did not return a valid HTTP response"
            )),
            tcp_ok: true,
            http_ok: false,
            probed_url,
            probed_port: port,
        };
    }

    details.site.status = "running".into();
    SiteReachability {
        state: "running".into(),
        message: None,
        tcp_ok: true,
        http_ok: true,
        probed_url,
        probed_port: port,
    }
}

/// Attach URLs from stack ports without probing (instant list render).
pub fn enrich_site_details_fast(details: &mut SiteDetails, state: &AppState) {
    apply_stack_ports(details, state);
}

/// Attach reachability to details and sync the matching SiteRecord.status in state.
pub fn enrich_site_details(
    paths: &Paths,
    state: &mut AppState,
    details: &mut SiteDetails,
) {
    apply_stack_ports(details, state);
    let reach = probe_site_reachability(paths, state, details);
    if let Some(site) = state.sites.iter_mut().find(|s| s.id == details.site.id) {
        site.status = reach.state.clone();
    }
    details.site.status = reach.state.clone();
    details.reachability = Some(reach);
}

fn urlencoding_minimal(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u8),
        })
        .collect()
}

pub fn open_phpmyadmin_url(details: &SiteDetails) -> Option<String> {
    details.phpmyadmin_url.clone()
}

pub fn refresh_site_metadata(state: &mut AppState) {
    for site in &mut state.sites {
        site.ensure_domain();
        let details = detect_site(site);
        site.framework = Some(details.framework);
        if site.database_name.is_none() {
            site.database_name = details.database_name;
        }
        if site.database_user.is_none() {
            site.database_user = details.database_user;
        }
    }
}

pub fn create_site(
    state: &mut AppState,
    _paths: &Paths,
    name: &str,
    path: PathBuf,
    php_version: Option<String>,
) -> Result<SiteRecord> {
    let id = slugify(name);
    let mut site = SiteRecord {
        id: id.clone(),
        name: name.to_string(),
        path,
        domain: format!("{id}.test"),
        php_version,
        node_version: None,
        https: true,
        web_server: Some("nginx".into()),
        framework: None,
        database_engine: state.active_database.clone(),
        database_name: None,
        database_user: None,
        database_password: None,
        status: "stopped".into(),
        is_template: false,
    };
    let details = detect_site(&site);
    site.framework = Some(details.framework);
    site.database_name = details.database_name;
    site.database_user = details.database_user;
    // Normalize mixed separators / ~ before persisting.
    site.path = crate::paths_util::expand_home(&site.path);
    state.sites.retain(|s| s.id != site.id);
    state.sites.push(site.clone());
    let abs = crate::paths_util::ensure_dir(&site.path)?;
    site.path = abs.clone();
    if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
        s.path = abs.clone();
    }
    if !abs.join("index.php").exists() && !abs.join("index.html").exists() {
        fs::write(abs.join("index.php"), "<?php phpinfo();")?;
    }
    Ok(site)
}

/// Remove a site from state and drop its nginx/apache vhost snippets.
/// Does not delete the site files on disk.
pub fn delete_site(paths: &Paths, state: &mut AppState, site_id: &str) -> Result<()> {
    let existed = state.sites.iter().any(|s| s.id == site_id);
    if !existed {
        return Err(DevStackError::Message(format!("site not found: {site_id}")));
    }
    state.sites.retain(|s| s.id != site_id);
    state.templates.retain(|t| t.source_site_id != site_id && t.id != site_id);
    let nginx_conf = paths.nginx_sites_dir().join(format!("{site_id}.conf"));
    let apache_conf = paths.apache_sites_dir().join(format!("{site_id}.conf"));
    let _ = fs::remove_file(nginx_conf);
    let _ = fs::remove_file(apache_conf);
    Ok(())
}
