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

/// Parse a timestamp from an Apache or Nginx error-log line (best-effort).
fn parse_log_line_time(line: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    // Nginx: `2023/11/08 14:22:01 [error] …`
    if let Some(head) = line.get(..19) {
        if head.as_bytes().get(4) == Some(&b'/') {
            if let Ok(naive) =
                chrono::NaiveDateTime::parse_from_str(head, "%Y/%m/%d %H:%M:%S")
            {
                return Some(chrono::DateTime::from_naive_utc_and_offset(naive, chrono::Utc));
            }
        }
    }
    // Apache: `[Wed Nov 08 14:22:01.123456 2023] …` or `[Wed Nov 08 14:22:01 2023]`
    if let Some(start) = line.find('[') {
        if let Some(end) = line[start + 1..].find(']') {
            let inner = &line[start + 1..start + 1 + end];
            // Drop fractional seconds: `Wed Nov 08 14:22:01.123456 2023` → `…01 2023`
            let cleaned = if let Some(dot) = inner.find('.') {
                let after = inner[dot + 1..]
                    .find(' ')
                    .map(|i| &inner[dot + 1 + i..])
                    .unwrap_or("");
                format!("{}{}", &inner[..dot], after)
            } else {
                inner.to_string()
            };
            if let Ok(dt) = chrono::DateTime::parse_from_str(
                &format!("{cleaned} +0000"),
                "%a %b %d %H:%M:%S %Y %z",
            ) {
                return Some(dt.with_timezone(&chrono::Utc));
            }
            if let Ok(naive) =
                chrono::NaiveDateTime::parse_from_str(&cleaned, "%a %b %d %H:%M:%S %Y")
            {
                return Some(chrono::DateTime::from_naive_utc_and_offset(naive, chrono::Utc));
            }
        }
    }
    None
}

fn line_is_after_start(line: &str, since: &chrono::DateTime<chrono::Utc>) -> bool {
    match parse_log_line_time(line) {
        // Only keep lines we can date to after the current stack start.
        // Undated / unparseable lines are treated as stale (never attach Nov-08
        // mailpit noise when the stack just came up).
        Some(ts) => ts >= *since,
        None => false,
    }
}

/// Fresh, non-benign log lines since `last_start_at` (best-effort).
fn fresh_error_log_excerpt(paths: &Paths, apache: bool, since: Option<&str>) -> Option<String> {
    let since_dt = since.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
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
        if let Some(since_dt) = since_dt.as_ref() {
            lines.retain(|l| line_is_after_start(l, since_dt));
        } else if let Some(since) = since {
            // Legacy fallback: keep lines after an embedded start marker.
            if let Some(idx) = lines.iter().rposition(|l| l.contains(since)) {
                lines = lines.split_off(idx.saturating_add(1));
            } else {
                // No marker and no parseable since → refuse stale tails.
                lines.clear();
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

const EMPTY_SITE_LOGO_DATA_URI: &str =
    "data:image/png;base64,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";

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Branded empty-site `index.html` with inline app logo + absolute folder path.
pub fn empty_site_placeholder_html(absolute_path: &Path) -> String {
    let path_disp = absolute_path.display().to_string();
    let path_esc = html_escape(&path_disp);
    let path_json =
        serde_json::to_string(&path_disp).unwrap_or_else(|_| format!("\"{path_esc}\""));
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>DevStack — empty site</title>
  <!-- devstack-empty-site -->
  <style>
    :root {{ color-scheme: light; }}
    body {{
      margin: 0; min-height: 100vh; display: grid; place-items: center;
      font-family: "Segoe UI", system-ui, sans-serif;
      background: linear-gradient(160deg, #0f172a 0%, #1e293b 45%, #0ea5e9 160%);
      color: #e2e8f0;
    }}
    main {{ text-align: center; padding: 2rem; max-width: 34rem; }}
    .lockup {{
      display: inline-flex; align-items: center; gap: 0.7rem;
      margin: 0 auto 1.25rem;
    }}
    .lockup img {{
      width: 40px; height: 40px; border-radius: 10px;
      box-shadow: 0 4px 18px rgba(14, 165, 233, 0.35);
    }}
    .lockup span {{
      font-weight: 650; font-size: 1.05rem; letter-spacing: -0.02em;
    }}
    h1 {{ font-size: 1.35rem; font-weight: 650; margin: 0 0 0.5rem; letter-spacing: -0.02em; }}
    p {{ margin: 0; opacity: 0.78; line-height: 1.45; font-size: 0.95rem; }}
    .path-row {{
      margin-top: 1.1rem; display: flex; flex-wrap: wrap; align-items: stretch;
      justify-content: center; gap: 0.45rem;
    }}
    .path-row code {{
      flex: 1 1 12rem; max-width: 100%;
      font-family: ui-monospace, "Cascadia Code", "SF Mono", Consolas, monospace;
      font-size: 0.78rem; line-height: 1.35; text-align: left;
      background: rgba(15, 23, 42, 0.55); color: #e2e8f0;
      border: 1px solid rgba(148, 163, 184, 0.28); border-radius: 8px;
      padding: 0.55rem 0.7rem; user-select: all; overflow-wrap: anywhere;
    }}
    .path-row button {{
      border: 1px solid rgba(148, 163, 184, 0.35); background: rgba(30, 41, 59, 0.9);
      color: #e2e8f0; border-radius: 8px; padding: 0.45rem 0.75rem;
      font: inherit; font-size: 0.8rem; cursor: pointer;
    }}
    .path-row button:hover {{ background: rgba(51, 65, 85, 0.95); }}
  </style>
</head>
<body>
  <main>
    <div class="lockup">
      <img src="{logo}" width="40" height="40" alt="DevStack" />
      <span>DevStack</span>
    </div>
    <h1>No index file yet</h1>
    <p>Drop an <code>index.php</code> or <code>index.html</code> into this site folder to get started.</p>
    <div class="path-row">
      <code id="site-path">{path}</code>
      <button type="button" id="copy-path" title="Copy path">Copy</button>
    </div>
  </main>
  <script>
    (function () {{
      var path = {path_json};
      var btn = document.getElementById("copy-path");
      if (!btn) return;
      btn.addEventListener("click", function () {{
        var done = function () {{
          btn.textContent = "Copied";
          setTimeout(function () {{ btn.textContent = "Copy"; }}, 1200);
        }};
        if (navigator.clipboard && navigator.clipboard.writeText) {{
          navigator.clipboard.writeText(path).then(done).catch(function () {{
            try {{
              var t = document.createElement("textarea");
              t.value = path; document.body.appendChild(t); t.select();
              document.execCommand("copy"); document.body.removeChild(t); done();
            }} catch (e) {{}}
          }});
        }} else {{
          try {{
            var t = document.createElement("textarea");
            t.value = path; document.body.appendChild(t); t.select();
            document.execCommand("copy"); document.body.removeChild(t); done();
          }} catch (e) {{}}
        }}
      }});
    }})();
  </script>
</body>
</html>
"##,
        logo = EMPTY_SITE_LOGO_DATA_URI,
        path = path_esc,
        path_json = path_json,
    )
}


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

/// If `docroot` exists and has no index (or only our placeholder), write/update the branded page.
pub fn ensure_site_placeholder(docroot: &Path) -> Result<()> {
    let root = expand_home(docroot);
    if !root.is_dir() {
        return Ok(());
    }
    let index = root.join("index.html");
    let only_ours = index.exists()
        && !root.join("index.php").exists()
        && !root.join("index.htm").exists()
        && fs::read_to_string(&index)
            .map(|s| s.contains(EMPTY_SITE_MARKER))
            .unwrap_or(false);
    if has_index_file(&root) && !only_ours {
        return Ok(());
    }
    let abs = dunce::canonicalize(&root).unwrap_or_else(|_| root.clone());
    fs::write(index, empty_site_placeholder_html(&abs))?;
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

fn probe_looks_transient(probe: &ProbeResult) -> bool {
    if probe.code == "000" || probe.code.is_empty() {
        return true;
    }
    let err = probe.error.as_deref().unwrap_or("").to_ascii_lowercase();
    err.contains("timed out")
        || err.contains("timeout")
        || err.contains("operation timed out")
        || err.contains("error code 28")
}

/// True when the effective URL is WordPress's installer (not a generic timeout).
pub fn is_wordpress_install_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let path = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .and_then(|rest| rest.find('/').map(|i| &rest[i..]))
        .unwrap_or("/");
    path.contains("wp-admin/install.php") || path.contains("/install.php")
}

fn http_probe_once(host: &str, port: u16, https: bool, max_time_secs: &str) -> ProbeResult {
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
            max_time_secs,
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
        let final_url = parts.next().unwrap_or("").trim().to_string();
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
        // 2xx / 3xx / 4xx = reachable (app responded).
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

fn http_probe(host: &str, port: u16, https: bool) -> ProbeResult {
    let first = http_probe_once(host, port, https, "10");
    if first.ok || !probe_looks_transient(&first) {
        return first;
    }
    // One retry on timeout / HTTP 000 — cold php-cgi often needs a second hit.
    http_probe_once(host, port, https, "10")
}

/// Hit each site's Open URL once so php-cgi / FPM workers are warm before probing.
pub fn warm_php_runtimes(paths: &Paths, state: &AppState) {
    let _ = paths;
    for site in &state.sites {
        let (port, https) = site_listen_port(site, state);
        // Short warm curl — failures are fine; probe_all will report properly.
        let _ = http_probe_once(&site.domain, port, https, "5");
    }
}

/// Fill `php_version: null` with the installed default and report whether state changed.
pub fn migrate_null_php_versions(state: &mut AppState) -> bool {
    crate::services::php_runtime::migrate_null_php_versions(state)
}

fn php_missing_hint(state: &AppState, site: &SiteRecord) -> Option<String> {
    let ver = site.php_version.as_deref()?;
    if crate::config::find_installed(state, "php", ver).is_some() {
        return None;
    }
    Some(format!("PHP {ver} not installed"))
}

fn php_fcgi_hint(paths: &Paths, state: &AppState, site: &SiteRecord) -> Option<String> {
    if let Some(missing) = php_missing_hint(state, site) {
        return Some(missing);
    }
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

fn php_probe_hint(paths: &Paths, state: &AppState, site: &SiteRecord) -> Option<String> {
    php_fcgi_hint(paths, state, site).or_else(|| php_missing_hint(state, site))
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
        // WP installer redirect can also surface as a non-ok code on some setups.
        if is_wordpress_install_url(&probe.final_url) {
            details.site.status = "warning".into();
            return SiteReachability {
                state: "warning".into(),
                message: Some("WordPress not installed".into()),
                log_excerpt: None,
                tcp_ok: true,
                http_ok: false,
                probed_url,
                probed_port: port,
            };
        }
        let php_hint = php_probe_hint(paths, state, &details.site);
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

    // WordPress landed on the installer — reachable, but not a working site yet.
    if is_wordpress_install_url(&probe.final_url) {
        details.site.status = "warning".into();
        return SiteReachability {
            state: "warning".into(),
            message: Some("WordPress not installed".into()),
            log_excerpt: None,
            tcp_ok: true,
            http_ok: true,
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

    // Soft warning when a missing PHP version is configured (site may still serve via fallback).
    if let Some(hint) = php_missing_hint(state, &details.site) {
        details.site.status = "warning".into();
        return SiteReachability {
            state: "warning".into(),
            message: Some(hint),
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
        assert!(html.contains("data:image/png;base64,"));
        assert!(html.contains("alt=\"DevStack\""));
        let abs = dunce::canonicalize(&dir).unwrap();
        assert!(html.contains(&html_escape(&abs.display().to_string())));
        assert!(is_empty_site_placeholder(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_site_html_escapes_path() {
        let p = PathBuf::from(r#"C:\Users\Marcin\Sites\acme & <dash>"#);
        let html = empty_site_placeholder_html(&p);
        assert!(
            html.contains(r#"<code id="site-path">C:\Users\Marcin\Sites\acme &amp; &lt;dash&gt;</code>"#)
        );
        // Clipboard payload is a JS string (JSON-escaped), not raw HTML text nodes.
        assert!(html.contains(r#"var path = "C:\\Users\\Marcin\\Sites\\acme & <dash>";"#));
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

    #[test]
    fn wordpress_install_url_detection() {
        assert!(is_wordpress_install_url(
            "https://blog.test:8444/wp-admin/install.php"
        ));
        assert!(is_wordpress_install_url(
            "http://blog.test/wp-admin/install.php?step=1"
        ));
        assert!(is_wordpress_install_url("https://blog.test/install.php"));
        assert!(!is_wordpress_install_url("https://blog.test/wp-admin/"));
        assert!(!is_wordpress_install_url("https://blog.test/"));
    }

    #[test]
    fn migrate_null_php_sets_default_and_persists_shape() {
        let mut state = AppState::default();
        state.stack.php_version = Some("8.4".into());
        state.sites.push(SiteRecord {
            id: "parked".into(),
            name: "parked".into(),
            path: PathBuf::from("/tmp/parked"),
            domain: "parked.test".into(),
            php_version: None,
            node_version: None,
            https: true,
            web_server: Some("nginx".into()),
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: true,
            xdebug: false,
            tld: "test".into(),
            aliases: Vec::new(),
            wildcard: false,
        });
        assert!(migrate_null_php_versions(&mut state));
        assert_eq!(state.sites[0].php_version.as_deref(), Some("8.4"));
        assert!(!migrate_null_php_versions(&mut state));
    }

    #[test]
    fn fresh_log_excerpt_ignores_pre_start_lines() {
        let dir = temp_dir("log-fresh");
        let paths = Paths::from_root(dir.clone()).unwrap();
        fs::create_dir_all(paths.logs.join("nginx")).unwrap();
        let log = paths.logs.join("nginx/error.log");
        fs::write(
            &log,
            "2023/11/08 10:00:00 [error] mailpit upstream timed out\n\
             2026/11/11 12:00:05 [error] connect() failed\n",
        )
        .unwrap();
        let since = "2026-11-11T12:00:00Z";
        let excerpt = fresh_error_log_excerpt(&paths, false, Some(since)).unwrap();
        assert!(excerpt.contains("connect() failed"));
        assert!(!excerpt.contains("mailpit"));
        let _ = fs::remove_dir_all(&dir);
    }
}
