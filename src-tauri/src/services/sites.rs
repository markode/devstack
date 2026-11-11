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
    /// Short human-facing summary (never a raw log dump).
    pub message: Option<String>,
    /// Fresh error log excerpt for a "Show log" expander (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_excerpt: Option<String>,
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
    // Docroots such as `public/` or `web/` sit one level below the project root.
    let project = project_root_for_docroot(&root);
    let composer = fs::read_to_string(project.join("composer.json")).unwrap_or_default();
    if project.join("artisan").exists() && project.join("composer.json").exists() {
        if composer.contains("statamic/cms") {
            return ("statamic".into(), "ST".into());
        }
        return ("laravel".into(), "LV".into());
    }
    if composer.contains("symfony/framework-bundle") || project.join("bin/console").exists() && composer.contains("symfony/") {
        return ("symfony".into(), "SY".into());
    }
    if composer.contains("drupal/core") || project.join("web/core/lib/Drupal.php").exists() {
        return ("drupal".into(), "DR".into());
    }
    if root.join("libraries/src/Version.php").exists() || root.join("configuration.php").exists() && root.join("administrator").exists() {
        return ("joomla".into(), "JM".into());
    }
    if project.join("composer.json").exists() {
        return ("php".into(), "PHP".into());
    }
    ("other".into(), "?".into())
}

/// `{project}/public` → `{project}` when the folder is a conventional docroot.
pub fn project_root_for_docroot(root: &Path) -> PathBuf {
    let is_docroot = root
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| matches!(n, "public" | "web" | "public_html" | "webroot" | "www"))
        .unwrap_or(false);
    if is_docroot {
        if let Some(parent) = root.parent() {
            return parent.to_path_buf();
        }
    }
    root.to_path_buf()
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

/// Apache mpm_winnt lifecycle notices are not failures.
fn is_benign_log_line(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    if l.contains("ah00418") || l.contains("ah00354") || l.contains("ah00489") || l.contains("ah00094")
    {
        return true;
    }
    if l.contains("resuming normal operations") || l.contains("created child process") {
        return true;
    }
    if l.contains("starting ") && l.contains("worker threads") {
        return true;
    }
    if l.contains("sigusr1 received") || l.contains("graceful restart") {
        return true;
    }
    false
}

fn is_errorish_log_line(line: &str) -> bool {
    if is_benign_log_line(line) {
        return false;
    }
    let l = line.to_ascii_lowercase();
    l.contains("error")
        || l.contains("emerg")
        || l.contains("crit")
        || l.contains("alert")
        || l.contains("fatal")
        || l.contains("failed")
        || l.contains("refused")
        || l.contains("cannot")
        || l.contains("unable")
        || l.contains("bind() to")
        || l.contains("no listening sockets")
}

/// Fresh, non-benign log lines since `last_start_at` (best-effort).
fn fresh_error_log_excerpt(paths: &Paths, apache: bool, since: Option<&str>) -> Option<String> {
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
        let Ok(raw) = fs::read_to_string(&p) else {
            continue;
        };
        let mut lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        if let Some(since) = since {
            // Keep lines that appear after the start marker / timestamp substring when present.
            if let Some(idx) = lines.iter().rposition(|l| l.contains(since)) {
                lines = lines.split_off(idx.saturating_add(1));
            }
        }
        let errors: Vec<&str> = lines
            .into_iter()
            .filter(|l| is_errorish_log_line(l))
            .collect();
        if errors.is_empty() {
            continue;
        }
        let take = errors.len().saturating_sub(12);
        let excerpt = errors[take..].join("\n");
        if !excerpt.is_empty() {
            return Some(excerpt);
        }
    }
    None
}

fn humanize_start_failure(server: &str, raw: &str) -> String {
    let l = raw.to_ascii_lowercase();
    if l.contains("port") && (l.contains("busy") || l.contains("in use") || l.contains("bind")) {
        return format!("{server} could not bind — a port is already in use");
    }
    if l.contains("configtest") || l.contains("emerg") || l.contains("syntax") {
        return format!("{server} failed to start due to a configuration error");
    }
    if l.contains("vcruntime") || l.contains("dll") || l.contains("not found") {
        return format!("{server} failed to start — missing dependency or binary");
    }
    if l.contains("exited immediately") {
        return format!("{server} exited immediately after start");
    }
    format!("{server} failed to start")
}

/// Marker embedded in the branded empty-site placeholder HTML.
pub const EMPTY_SITE_MARKER: &str = "<!-- devstack-empty-site -->";

const EMPTY_SITE_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>DevStack — empty site</title>
  <!-- devstack-empty-site -->
  <style>
    :root { color-scheme: light; }
    body {
      margin: 0; min-height: 100vh; display: grid; place-items: center;
      font-family: "Segoe UI", system-ui, sans-serif;
      background: linear-gradient(160deg, #0f172a 0%, #1e293b 45%, #0ea5e9 160%);
      color: #e2e8f0;
    }
    main { text-align: center; padding: 2rem; max-width: 28rem; }
    h1 { font-size: 1.35rem; font-weight: 650; margin: 0 0 0.5rem; letter-spacing: -0.02em; }
    p { margin: 0; opacity: 0.78; line-height: 1.45; font-size: 0.95rem; }
    .brand { font-size: 0.75rem; letter-spacing: 0.14em; text-transform: uppercase;
      opacity: 0.55; margin-bottom: 1rem; }
  </style>
</head>
<body>
  <main>
    <div class="brand">DevStack</div>
    <h1>No index file yet</h1>
    <p>Drop an <code>index.php</code> or <code>index.html</code> into this site folder to get started.</p>
  </main>
</body>
</html>
"#;

fn has_index_file(docroot: &Path) -> bool {
    docroot.join("index.php").exists()
        || docroot.join("index.html").exists()
        || docroot.join("index.htm").exists()
}

fn has_real_app_files(docroot: &Path) -> bool {
    let root = expand_home(docroot);
    let project = project_root_for_docroot(&root);
    const SIGNALS: &[&str] = &[
        "wp-config.php",
        "wp-config-sample.php",
        "composer.json",
        "package.json",
        "artisan",
        "Gemfile",
        "manage.py",
        "Cargo.toml",
        "go.mod",
        "web.config",
    ];
    SIGNALS
        .iter()
        .any(|f| root.join(f).exists() || project.join(f).exists())
}

/// True when the docroot is only our branded empty-site placeholder (no real app).
pub fn is_empty_site_placeholder(docroot: &Path) -> bool {
    let root = expand_home(docroot);
    if !root.is_dir() {
        return false;
    }
    let (framework, _) = detect_framework(&root);
    if framework != "other" || has_real_app_files(&root) {
        return false;
    }
    if root.join("index.php").exists() || root.join("index.htm").exists() {
        return false;
    }
    let index = root.join("index.html");
    let Ok(raw) = fs::read_to_string(&index) else {
        return false;
    };
    raw.contains(EMPTY_SITE_MARKER)
}

/// If `docroot` exists and has no index file, write a branded static placeholder.
pub fn ensure_site_placeholder(docroot: &Path) -> Result<()> {
    let root = expand_home(docroot);
    if !root.is_dir() {
        return Ok(());
    }
    if has_index_file(&root) {
        return Ok(());
    }
    fs::write(root.join("index.html"), EMPTY_SITE_HTML)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub ok: bool,
    pub code: String,
    pub final_url: String,
    pub error: Option<String>,
}

fn host_of_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let hostport = rest.split('/').next().unwrap_or(rest);
    let host = hostport.split(':').next().unwrap_or(hostport);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

fn http_probe(host: &str, port: u16, https: bool) -> ProbeResult {
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
            "-L",
            "--max-redirs",
            "5",
            "--max-time",
            "3",
            "--resolve",
            &format!("{host}:{port}:127.0.0.1"),
            "-o",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
            "-w",
            "%{http_code}\t%{url_effective}",
            &url,
        ])
        .output()
    {
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let mut parts = stdout.splitn(2, '\t');
        let code = parts.next().unwrap_or("").trim().to_string();
        let final_url = parts
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let final_url = if final_url.is_empty() {
            url.clone()
        } else {
            final_url
        };
        if code.is_empty() {
            return ProbeResult {
                ok: false,
                code: "000".into(),
                final_url,
                error: Some(if stderr.is_empty() {
                    "empty response".into()
                } else {
                    stderr
                }),
            };
        }
        let ok = code.starts_with('2') || code.starts_with('3') || code.starts_with('4');
        if ok {
            return ProbeResult {
                ok: true,
                code,
                final_url,
                error: None,
            };
        }
        let reason = if code == "000" {
            if stderr.is_empty() {
                "connection failed".into()
            } else {
                stderr
            }
        } else if !stderr.is_empty() {
            stderr
        } else {
            format!("HTTP {code}")
        };
        return ProbeResult {
            ok: false,
            code,
            final_url,
            error: Some(reason),
        };
    }
    if https {
        return ProbeResult {
            ok: false,
            code: "000".into(),
            final_url: url,
            error: Some("connection failed".into()),
        };
    }
    // Raw HTTP GET fallback (no redirect following).
    let Ok(mut stream) =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(400))
    else {
        return ProbeResult {
            ok: false,
            code: "000".into(),
            final_url: url,
            error: Some("connection failed".into()),
        };
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
    let req = format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).is_err() {
        return ProbeResult {
            ok: false,
            code: "000".into(),
            final_url: url,
            error: Some("connection failed".into()),
        };
    }
    let mut buf = [0u8; 64];
    let ok = matches!(stream.read(&mut buf), Ok(n) if n > 0);
    ProbeResult {
        ok,
        code: if ok { "200".into() } else { "000".into() },
        final_url: url,
        error: if ok {
            None
        } else {
            Some("empty response".into())
        },
    }
}

fn php_fcgi_hint(paths: &Paths, state: &AppState, site: &SiteRecord) -> Option<String> {
    if !crate::services::httpd::site_uses_apache(site) {
        return None;
    }
    let ver = site
        .php_version
        .clone()
        .or_else(|| crate::services::php_runtime::default_php_version(state))?;
    let listen =
        crate::services::php_runtime::fpm_listen_addr_with_base(paths, &ver, state.stack.php_fpm_port);
    let ready = if let Some(sock) = listen.strip_prefix("unix:") {
        Path::new(sock).exists()
    } else {
        listen
            .parse::<std::net::SocketAddr>()
            .ok()
            .map(|addr| {
                TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok()
            })
            .unwrap_or(false)
    };
    if ready {
        None
    } else {
        Some(format!(
            "PHP/FCGI for {ver} does not appear to be listening ({listen})"
        ))
    }
}

fn format_probe_failure(
    probe: &ProbeResult,
    probed_url: &str,
    php_hint: Option<String>,
) -> String {
    let mut msg = if probe.code == "000" || probe.code.is_empty() {
        format!(
            "HTTP 000 ({})",
            probe.error.as_deref().unwrap_or("connection failed")
        )
    } else if let Some(err) = probe.error.as_deref() {
        if err.starts_with("HTTP ") {
            err.to_string()
        } else {
            format!("HTTP {} ({err})", probe.code)
        }
    } else {
        format!("HTTP {}", probe.code)
    };
    if !probe.final_url.is_empty() && probe.final_url != probed_url {
        if let (Some(expected), Some(actual)) =
            (host_of_url(probed_url), host_of_url(&probe.final_url))
        {
            if expected != actual {
                msg = format!("redirected to {}", probe.final_url);
            }
        }
    }
    if let Some(hint) = php_hint {
        msg = format!("{msg}; {hint}");
    }
    msg
}

/// TCP-connect the site's listen port and HTTP-probe the actual Open URL.
/// Updates `site.status` to match (`running` / `error` / `stopped`).
///
/// When the stack is stopped (intentionally or never started), returns neutral
/// `stopped` — never red with stale log tails. Errors only after a failed start.
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

    // Front-end process down.
    if !server_up {
        // Intentional Stop stack (or never started) → grey / neutral, no stale logs.
        let never_started = state.runtime.last_start_at.is_none()
            && state.runtime.last_start_error.is_none();
        if state.runtime.stack_intentionally_stopped || never_started {
            details.site.status = "stopped".into();
            return SiteReachability {
                state: "stopped".into(),
                message: Some("Stack stopped — Start".into()),
                log_excerpt: None,
                tcp_ok: false,
                http_ok: false,
                probed_url,
                probed_port: port,
            };
        }
        if let Some(err) = state.runtime.last_start_error.as_deref() {
            let excerpt = fresh_error_log_excerpt(
                paths,
                apache,
                state.runtime.last_start_at.as_deref(),
            );
            details.site.status = "error".into();
            return SiteReachability {
                state: "error".into(),
                message: Some(humanize_start_failure(server_name, err)),
                log_excerpt: excerpt.or_else(|| {
                    let t = err.trim();
                    if t.is_empty() {
                        None
                    } else {
                        Some(
                            t.chars()
                                .rev()
                                .take(800)
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect(),
                        )
                    }
                }),
                tcp_ok: false,
                http_ok: false,
                probed_url,
                probed_port: port,
            };
        }
        // Unexpected crash / stop of this site's front-end while the stack was started
        // (sibling front-end may still be up — still red for this site).
        let excerpt = fresh_error_log_excerpt(
            paths,
            apache,
            state.runtime.last_start_at.as_deref(),
        );
        details.site.status = "error".into();
        return SiteReachability {
            state: "error".into(),
            message: Some(format!("{server_name} is not running")),
            log_excerpt: excerpt,
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
        let excerpt = fresh_error_log_excerpt(
            paths,
            apache,
            state.runtime.last_start_at.as_deref(),
        );
        details.site.status = "error".into();
        return SiteReachability {
            state: "error".into(),
            message: Some(format!(
                "{server_name} is running but nothing listens on :{port}"
            )),
            log_excerpt: excerpt,
            tcp_ok: false,
            http_ok: false,
            probed_url,
            probed_port: port,
        };
    }

    // Ensure empty docroots have a static index before probing.
    let _ = ensure_site_placeholder(&details.absolute_path);
    let empty_placeholder = is_empty_site_placeholder(&details.absolute_path);

    let probe = http_probe(&details.site.domain, port, https);
    if !probe.ok {
        let php_hint = php_fcgi_hint(paths, state, &details.site);
        details.site.status = "warning".into();
        return SiteReachability {
            state: "warning".into(),
            message: Some(format_probe_failure(&probe, &probed_url, php_hint)),
            log_excerpt: fresh_error_log_excerpt(
                paths,
                apache,
                state.runtime.last_start_at.as_deref(),
            ),
            tcp_ok: true,
            http_ok: false,
            probed_url,
            probed_port: port,
        };
    }

    // Empty branded placeholder → warning (not "Running").
    if empty_placeholder {
        details.site.status = "warning".into();
        return SiteReachability {
            state: "warning".into(),
            message: Some("No index file".into()),
            log_excerpt: None,
            tcp_ok: true,
            http_ok: true,
            probed_url,
            probed_port: port,
        };
    }

    // Surfaced redirect host mismatch (e.g. WordPress siteurl) as a soft warning.
    if let (Some(expected), Some(actual)) =
        (host_of_url(&probed_url), host_of_url(&probe.final_url))
    {
        if expected != actual {
            details.site.status = "warning".into();
            return SiteReachability {
                state: "warning".into(),
                message: Some(format!("redirected to {}", probe.final_url)),
                log_excerpt: None,
                tcp_ok: true,
                http_ok: true,
                probed_url,
                probed_port: port,
            };
        }
    }

    details.site.status = "running".into();
    SiteReachability {
        state: "running".into(),
        message: None,
        log_excerpt: None,
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
    let php_version =
        php_version.or_else(|| crate::services::php_runtime::default_php_version(state));
    let mut site = SiteRecord {
        id: id.clone(),
        name: name.to_string(),
        path,
        domain: format!("{id}.{}", state.developer.default_tld),
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
        parked: false,
        xdebug: false,
        tld: state.developer.default_tld.clone(),
        aliases: Vec::new(),
        wildcard: false,
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
    ensure_site_placeholder(&abs)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("devstack-{label}-{n}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_placeholder_writes_marker_when_empty() {
        let dir = temp_dir("empty-site");
        ensure_site_placeholder(&dir).unwrap();
        let html = fs::read_to_string(dir.join("index.html")).unwrap();
        assert!(html.contains(EMPTY_SITE_MARKER));
        assert!(is_empty_site_placeholder(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_placeholder_skips_existing_index() {
        let dir = temp_dir("has-index");
        fs::write(dir.join("index.php"), "<?php echo 1;").unwrap();
        ensure_site_placeholder(&dir).unwrap();
        assert!(!dir.join("index.html").exists());
        assert!(!is_empty_site_placeholder(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn placeholder_false_when_app_files_present() {
        let dir = temp_dir("wp-empty");
        ensure_site_placeholder(&dir).unwrap();
        fs::write(dir.join("wp-config.php"), "<?php").unwrap();
        assert!(!is_empty_site_placeholder(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_of_url_parses_schemes() {
        assert_eq!(
            host_of_url("https://blog.test:8443/wp-admin"),
            Some("blog.test".into())
        );
        assert_eq!(host_of_url("http://LOCAL.TEST/"), Some("local.test".into()));
    }
}
