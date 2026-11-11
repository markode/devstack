//! Apache httpd front-end — full `.htaccess` / mod_rewrite, PHP via proxy_fcgi / FCGI.
//!
//! ## Port design
//! Nginx keeps `stack.http_port` / `https_port` (typically 80/443).
//! Apache listens on `stack.apache_http_port` / `apache_https_port` (defaults **8088 / 8444**)
//! so both can run at once. Site URLs for Apache-backed sites include the non-default
//! port. See `docs/apache.md`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{find_installed, latest_installed, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::nginx::expand_home;
use crate::services::php_runtime::{nginx_fastcgi_pass, resolve_php, start_php_many, PhpRuntime};
use crate::services::process::{
    find_bin, is_running, read_pid, spawn_logged_attached, stop_pid, write_pid,
};

pub fn site_uses_apache(site: &SiteRecord) -> bool {
    matches!(
        site.web_server.as_deref().map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("apache") | Some("httpd")
    )
}

pub fn apache_root(_paths: &Paths, state: &AppState) -> Result<PathBuf> {
    if let Some(ver) = &state.stack.apache_version {
        if let Some(rec) = find_installed(state, "apache", ver) {
            return Ok(rec.path.clone());
        }
    }
    if let Some(rec) = latest_installed(state, "apache") {
        return Ok(rec.path.clone());
    }
    if let Ok(bin) = which::which("httpd") {
        return Ok(bin.parent().unwrap_or_else(|| Path::new("/usr/sbin")).to_path_buf());
    }
    Err(DevStackError::Message(
        "Apache is not installed. Install it from Services.".into(),
    ))
}

pub fn apache_bin(root: &Path) -> Result<PathBuf> {
    if let Some(p) = find_bin(root, &["httpd", "httpd.exe", "apache2"]) {
        return Ok(p);
    }
    which::which("httpd")
        .or_else(|_| which::which("apache2"))
        .map_err(|_| DevStackError::Message("httpd/apache2 binary not found".into()))
}

fn php_for_site<'a>(
    site: &SiteRecord,
    runtimes: &'a [(String, PhpRuntime)],
    state: &AppState,
) -> Option<&'a PhpRuntime> {
    let want = site
        .php_version
        .clone()
        .or_else(|| state.stack.php_version.clone())
        .or_else(|| runtimes.first().map(|(v, _)| v.clone()));
    if let Some(want) = want.as_ref() {
        if let Some(rt) = runtimes
            .iter()
            .find(|(v, r)| *v == *want || r.version == *want || r.version.starts_with(want))
            .map(|(_, r)| r)
        {
            return Some(rt);
        }
    }
    // Requested PHP missing — fall back to any installed runtime so the site still serves.
    runtimes.first().map(|(_, r)| r)
}

fn fcgi_target(runtime: &PhpRuntime) -> String {
    let pass = nginx_fastcgi_pass(runtime);
    if let Some(path) = pass.strip_prefix("unix:") {
        // Trailing slash required — without it Apache glues SCRIPT_FILENAME onto the
        // authority (on Windows: `127.0.0.1:9000` + `D:\…` → DNS host `127.0.0.1:9000d`).
        format!("unix:{path}|fcgi://localhost/")
    } else {
        format!("fcgi://{pass}/")
    }
}

fn module_line(root: &Path, name: &str) -> Option<String> {
    // unixd is Unix-only; Windows Lounge builds do not ship it.
    if name == "unixd" && cfg!(windows) {
        return None;
    }
    let candidates = [
        root.join("modules").join(format!("mod_{name}.so")),
        root.join("modules").join(format!("mod_{name}.dll")),
        root.join("lib").join("httpd").join("modules").join(format!("mod_{name}.so")),
        // Debian/Ubuntu apache2 package layout
        PathBuf::from(format!("/usr/lib/apache2/modules/mod_{name}.so")),
        PathBuf::from(format!("/usr/lib64/httpd/modules/mod_{name}.so")),
    ];
    for c in &candidates {
        if c.exists() {
            // Prefer absolute path so ServerRoot can be the DevStack package dir
            // or a system prefix without a relative modules/ tree.
            return Some(format!(
                "LoadModule {name}_module \"{}\"",
                crate::paths_util::conf_path(c)
            ));
        }
    }
    // Never invent LoadModule lines for missing files (Lounge modules/ may omit unixd).
    None
}

pub fn render_stack(
    paths: &Paths,
    state: &AppState,
    php_runtimes: &[(String, PhpRuntime)],
) -> Result<PathBuf> {
    let root = apache_root(paths, state)?;
    let conf_dir = paths.apache_conf_dir();
    let sites_dir = paths.apache_sites_dir();
    fs::create_dir_all(&conf_dir)?;
    fs::create_dir_all(&sites_dir)?;
    fs::create_dir_all(paths.logs.join("apache"))?;
    fs::create_dir_all(paths.apache.join("tmp"))?;

    // Drop nginx-only / deleted site leftovers before rewriting Apache vhosts.
    crate::services::vhost_ext::purge_stale_vhost_confs(paths, state);
    if sites_dir.exists() {
        for entry in fs::read_dir(&sites_dir)? {
            let path = entry?.path();
            // Full rewrite below — clear prior confs so nothing stale survives.
            if path.extension().and_then(|e| e.to_str()) == Some("conf") {
                let _ = fs::remove_file(&path);
            }
        }
    }

    let http_port = state.stack.apache_http_port;
    let https_port = state.stack.apache_https_port;

    // Neutral default first so the first custom site is never the catch-all on
    // :apache_http_port / :apache_https_port (avoids nginx sites' old Redirect
    // vhosts hijacking localhost Open).
    write_default_vhost(paths, &sites_dir, http_port, https_port)?;

    // Only Apache-preferred sites — nginx sites must not leave Redirect vhosts here.
    for site in state.sites.iter().filter(|s| site_uses_apache(s)) {
        write_vhost(paths, state, site, php_runtimes, &sites_dir, http_port, https_port)?;
    }

    let modules = [
        "authz_core",
        "authz_host",
        "headers",
        "mime",
        "rewrite",
        "setenvif",
        "ssl",
        "socache_shmcb",
        "proxy",
        "proxy_fcgi",
        "dir",
        "alias",
        "filter",
        "deflate",
        "log_config",
        "unixd",
    ];

    let system_layout = root.starts_with("/usr") || root.join("lib/apache2/modules").exists()
        || Path::new("/usr/lib/apache2/modules").exists()
            && find_bin(&root, &["apache2"]).is_some();

    let mut load_modules = String::new();
    // Only load an MPM for package layouts (Lounge / static builds). Distro
    // apache2 already selects mpm_event/prefork via its own modules.
    if !system_layout {
        if cfg!(windows) {
            if let Some(line) = module_line(&root, "mpm_winnt") {
                load_modules.push_str(&line);
                load_modules.push('\n');
            }
        } else if let Some(line) = module_line(&root, "mpm_event") {
            load_modules.push_str(&line);
            load_modules.push('\n');
        }
    } else if let Some(line) = module_line(&root, "mpm_event") {
        // Debian: mpm_event.so exists under /usr/lib/apache2/modules
        load_modules.push_str(&line);
        load_modules.push('\n');
    }
    for m in modules {
        if let Some(line) = module_line(&root, m) {
            load_modules.push_str(&line);
            load_modules.push('\n');
        }
    }

    let conf = conf_dir.join("httpd.conf");
    let server_root = if system_layout {
        // Debian apache2 expects ServerRoot /etc/apache2 for relative paths,
        // but we load modules by absolute path so /usr is fine.
        "/etc/apache2".to_string()
    } else {
        crate::paths_util::conf_path(&root)
    };
    let runtime_dir = paths.apache.join("run");
    fs::create_dir_all(&runtime_dir)?;
    let mime = [
        root.join("conf").join("mime.types"),
        // Homebrew httpd keeps mime.types under etc/ or share/, not conf/.
        root.join("share").join("httpd").join("mime.types"),
        PathBuf::from("/opt/homebrew/etc/httpd/mime.types"),
        PathBuf::from("/usr/local/etc/httpd/mime.types"),
        PathBuf::from("/etc/apache2/mime.types"),
        PathBuf::from("/etc/mime.types"),
        PathBuf::from("/etc/httpd/conf/mime.types"),
    ]
    .into_iter()
    .find(|p| p.exists())
    .unwrap_or_else(|| root.join("conf").join("mime.types"));
    let identity = if cfg!(windows) {
        String::new()
    } else if nix_geteuid() == 0 {
        "User www-data\nGroup www-data\n".into()
    } else {
        // Non-root smoke / user-mode: omit User/Group (can't drop privileges).
        String::new()
    };
    let runtime_lines = if cfg!(windows) {
        String::new()
    } else {
        format!(
            "DefaultRuntimeDir \"{}\"\nMutex default\n",
            crate::paths_util::conf_path(&runtime_dir)
        )
    };
    let body = format!(
        r#"ServerRoot "{server_root}"
# Bind address + protocol so HTTP/HTTPS listeners never share SSL state.
Listen 127.0.0.1:{http_port} http
Listen 127.0.0.1:{https_port} https
{load_modules}
ServerName localhost
ServerAdmin admin@localhost
PidFile "{pid}"
ErrorLog "{error_log}"
LogLevel warn
LogFormat "%h %l %u %t \"%r\" %>s %b" common
LogFormat "%h %l %u %t \"%r\" %>s %b \"%{{Referer}}i\" \"%{{User-Agent}}i\"" combined
Timeout 3600
KeepAlive On
MaxKeepAliveRequests 100
KeepAliveTimeout 5
LimitRequestBody 0
LimitXMLRequestBody 0
TypesConfig "{mime}"
AddType application/x-compress .Z
AddType application/x-gzip .gz .tgz
{runtime_lines}{identity}
SSLRandomSeed startup builtin
SSLRandomSeed connect builtin
SSLSessionCache "shmcb:{cache}"
SSLSessionCacheTimeout 300

IncludeOptional "{sites}/*.conf"
"#,
        pid = crate::paths_util::conf_path(&paths.pid_file("apache")),
        error_log = crate::paths_util::conf_path(&paths.logs.join("apache/error.log")),
        sites = crate::paths_util::conf_path(&sites_dir),
        mime = crate::paths_util::conf_path(&mime),
        cache = crate::paths_util::conf_path(&paths.apache.join("run/ssl_scache")),
    );
    fs::write(&conf, body)?;
    Ok(conf)
}

fn nix_geteuid() -> u32 {
    #[cfg(unix)]
    {
        // Linux: /proc. macOS/BSD: `id -u` (no /proc).
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            if let Some(uid) = s
                .lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
            {
                return uid;
            }
        }
        crate::services::process::command("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or(1000)
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn write_default_vhost(
    paths: &Paths,
    sites_dir: &Path,
    http_port: u16,
    https_port: u16,
) -> Result<()> {
    let docroot = paths.apache.join("default-docroot");
    fs::create_dir_all(&docroot)?;
    let root = crate::paths_util::conf_path(&docroot);
    let mut body = format!(
        r#"<VirtualHost 127.0.0.1:{http_port}>
  ServerName localhost
  DocumentRoot "{root}"
  <Directory "{root}">
    Require all denied
  </Directory>
  ErrorDocument 404 "DevStack: no site matched this Host header.\n"
  ErrorLog "{err}"
  CustomLog "{access}" common
</VirtualHost>
"#,
        root = root,
        err = crate::paths_util::conf_path(&paths.logs.join("apache/default-error.log")),
        access = crate::paths_util::conf_path(&paths.logs.join("apache/default-access.log")),
    );
    // HTTPS catch-all so unmatched Host on the SSL port never hits a site vhost.
    if let Ok(certs) = crate::services::tls::ensure_site_cert(paths, "localhost") {
        body.push_str(&format!(
            r#"
<VirtualHost 127.0.0.1:{https_port}>
  ServerName localhost
  DocumentRoot "{root}"
  SSLEngine on
  SSLCertificateFile "{cert}"
  SSLCertificateKeyFile "{key}"
  <Directory "{root}">
    Require all denied
  </Directory>
  ErrorDocument 404 "DevStack: no site matched this Host header.\n"
  ErrorLog "{err}"
  CustomLog "{access}" common
</VirtualHost>
"#,
            root = root,
            cert = crate::paths_util::conf_path(&certs.site_cert),
            key = crate::paths_util::conf_path(&certs.site_key),
            err = crate::paths_util::conf_path(&paths.logs.join("apache/default-ssl-error.log")),
            access = crate::paths_util::conf_path(
                &paths.logs.join("apache/default-ssl-access.log"),
            ),
        ));
    }
    fs::write(sites_dir.join("00-default.conf"), body)?;
    Ok(())
}

/// PHP proxy directives for a vhost.
///
/// Windows php-cgi needs a real filesystem `SCRIPT_FILENAME`:
/// - `SetHandler proxy:fcgi://host:port` (no slash) glues `D:` onto the authority
/// - trailing-slash SetHandler alone leaves a URL path → "No input file specified"
/// - `ProxyPassMatch …/C:/docroot/$1` often yields `/C:/…` (leading slash) which
///   php-cgi also rejects
///
/// Fix: trailing-slash SetHandler + `ProxyFCGISetEnvIf` rebuild from
/// `DOCUMENT_ROOT` + `SCRIPT_NAME` (forward-slash docroots via `conf_path`).
///
/// Unix php-fpm uses SetHandler with a trailing slash.
fn php_proxy_block(runtime: &PhpRuntime, _docroot_conf: &str) -> String {
    let fcgi = fcgi_target(runtime);
    if cfg!(windows) {
        format!(
            r#"  ProxyFCGIBackendType GENERIC
  <FilesMatch "\.php$">
    SetHandler "proxy:{fcgi}"
  </FilesMatch>
  # Rebuild SCRIPT_FILENAME as DOCUMENT_ROOT + SCRIPT_NAME (Windows drive-safe).
  ProxyFCGISetEnvIf "true" SCRIPT_FILENAME "%{{reqenv:DOCUMENT_ROOT}}%{{reqenv:SCRIPT_NAME}}"
  ProxyTimeout 3600
"#
        )
    } else {
        format!(
            r#"  <FilesMatch "\.php$">
    SetHandler "proxy:{fcgi}"
  </FilesMatch>
  ProxyTimeout 3600
"#
        )
    }
}

fn write_vhost(
    paths: &Paths,
    state: &AppState,
    site: &SiteRecord,
    runtimes: &[(String, PhpRuntime)],
    sites_dir: &Path,
    http_port: u16,
    https_port: u16,
) -> Result<()> {
    let docroot = expand_home(&site.path);
    let _ = crate::services::sites::ensure_site_placeholder(&docroot);
    let Some(runtime) = php_for_site(site, runtimes, state) else {
        return Ok(());
    };
    let root = crate::paths_util::conf_path(&docroot);
    let php = php_proxy_block(runtime, &root);
    // VirtualHost address must match Listen (127.0.0.1) — `*:port` can attach the
    // SSL vhost to the wrong listener on Lounge/Homebrew builds.
    let mut conf = if site.https {
        let dest = crate::services::ports::format_url("https", &site.domain, https_port);
        format!(
            r#"<VirtualHost 127.0.0.1:{http_port}>
  ServerName {domain}
{aliases}  Redirect / {dest}/
  ErrorLog "{err}"
  CustomLog "{access}" common
</VirtualHost>
"#,
            domain = site.domain,
            aliases = crate::services::vhost_ext::apache_server_alias(site),
            dest = dest.trim_end_matches('/'),
            err = crate::paths_util::conf_path(
                &paths.logs.join(format!("apache/{}-error.log", site.id)),
            ),
            access = crate::paths_util::conf_path(
                &paths.logs.join(format!("apache/{}-access.log", site.id)),
            ),
        )
    } else {
        format!(
            r#"<VirtualHost 127.0.0.1:{http_port}>
  ServerName {domain}
{aliases}  DocumentRoot "{root}"
  <Directory "{root}">
    Options FollowSymLinks
    AllowOverride All
    Require all granted
  </Directory>
  DirectoryIndex index.php index.html
  LimitRequestBody 0
  Timeout 3600
{php}  ErrorLog "{err}"
  CustomLog "{access}" common
</VirtualHost>
"#,
            domain = site.domain,
            aliases = crate::services::vhost_ext::apache_server_alias(site),
            root = root,
            php = php,
            err = crate::paths_util::conf_path(
                &paths.logs.join(format!("apache/{}-error.log", site.id)),
            ),
            access = crate::paths_util::conf_path(
                &paths.logs.join(format!("apache/{}-access.log", site.id)),
            ),
        )
    };

    if site.https {
        let certs = crate::services::tls::ensure_site_cert_for(paths, site)?;
        conf.push_str(&format!(
            r#"
<VirtualHost 127.0.0.1:{https_port}>
  ServerName {domain}
{aliases}  DocumentRoot "{root}"
  SSLEngine on
  SSLCertificateFile "{cert}"
  SSLCertificateKeyFile "{key}"
  <Directory "{root}">
    Options FollowSymLinks
    AllowOverride All
    Require all granted
  </Directory>
  DirectoryIndex index.php index.html
  LimitRequestBody 0
  Timeout 3600
{php}  ErrorLog "{err}"
  CustomLog "{access}" common
</VirtualHost>
"#,
            domain = site.domain,
            aliases = crate::services::vhost_ext::apache_server_alias(site),
            root = root,
            cert = crate::paths_util::conf_path(&certs.site_cert),
            key = crate::paths_util::conf_path(&certs.site_key),
            php = php,
            err = crate::paths_util::conf_path(
                &paths.logs.join(format!("apache/{}-ssl-error.log", site.id)),
            ),
            access = crate::paths_util::conf_path(
                &paths.logs.join(format!("apache/{}-ssl-access.log", site.id)),
            ),
        ));
    }

    fs::write(sites_dir.join(format!("{}.conf", site.id)), conf)?;
    Ok(())
}

fn collect_php_versions(state: &AppState) -> Vec<String> {
    let mut versions: Vec<String> = state
        .sites
        .iter()
        .filter(|s| site_uses_apache(s))
        .filter_map(|s| s.php_version.clone())
        .collect();
    if let Some(v) = &state.stack.php_version {
        versions.push(v.clone());
    }
    if versions.is_empty() {
        if let Some(rec) = latest_installed(state, "php") {
            versions.push(rec.version.clone());
        } else {
            versions.push("8.3".into());
        }
    }
    versions.sort();
    versions.dedup();
    versions
}

struct StartApacheOpts {
    run_configtest: bool,
    start_php_runtimes: bool,
}

pub fn start_apache(paths: &Paths, state: &mut AppState) -> Result<()> {
    crate::services::vhost_ext::purge_stale_vhost_confs(paths, state);
    start_apache_inner(
        paths,
        state,
        StartApacheOpts {
            run_configtest: true,
            start_php_runtimes: true,
        },
    )
}

fn start_apache_inner(paths: &Paths, state: &mut AppState, opts: StartApacheOpts) -> Result<()> {
    if let Some(pid) = state.runtime.apache_pid {
        if is_running(pid) {
            return reload_apache(paths, state);
        }
    }
    // Kill stale DevStack httpd left by prior runs (never touches WAMP / live PHP).
    let keep: Vec<u32> = state.runtime.php_pids.values().copied().collect();
    crate::startup::reclaim_orphans_for_names(paths, &["httpd.exe"], &keep);
    // Reclaim a stale FOREGROUND httpd left from a prior crash / DETACHED spawn.
    if let Some(old) = read_pid(&paths.pid_file("apache")) {
        let _ = stop_pid(old);
        state.runtime.apache_pid = None;
        wait_apache_ports_free(state.stack.apache_http_port, state.stack.apache_https_port);
    }
    crate::services::ports::ensure_ports_free(&state.stack, "apache")?;

    let needs_apache = state.sites.iter().any(site_uses_apache)
        || latest_installed(state, "apache").is_some();
    if !needs_apache {
        return Ok(());
    }

    let mut runtimes = Vec::new();
    if opts.start_php_runtimes {
        let versions = collect_php_versions(state);
        for (ver, res) in start_php_many(paths, state, &versions) {
            match res {
                Ok(rt) => runtimes.push((ver, rt)),
                Err(e) => {
                    eprintln!("apache: skip php {ver}: {e}");
                }
            }
        }
    }
    // Resolve without starting if already up / reload fast-path.
    if runtimes.is_empty() {
        for ver in collect_php_versions(state) {
            if let Ok(rt) = resolve_php(paths, state, &ver) {
                runtimes.push((ver, rt));
            }
        }
    }

    let root = apache_root(paths, state)?;
    let bin = apache_bin(&root)?;
    let conf = render_stack(paths, state, &runtimes)?;

    // Start whenever Apache is installed (needs_apache above), even with zero
    // apache-preferred sites — dual-bound vhosts keep toggles under 1s.
    let conf_s = conf.display().to_string();
    // Configtest on cold start so we surface syntax/module errors. Reload skips
    // it — the conf was just rendered and the extra httpd -t was ~1–2s on CI.
    if opts.run_configtest {
        let test = crate::services::process::command(&bin)
            .args(["-f", &conf_s, "-t"])
            .current_dir(&root)
            .output();
        if let Ok(out) = &test {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                return Err(DevStackError::Message(format!(
                    "Apache configtest failed: {stderr}{stdout}"
                )));
            }
        }
    }

    // Attached (not DETACHED_PROCESS): we must own the real FOREGROUND pid so
    // later reload/stop can taskkill the listener. Detached spawns orphaned
    // httpd.exe that still hold 8088/8444 and look like "WAMP" conflicts.
    let child = spawn_logged_attached(
        &bin,
        &["-f", &conf_s, "-D", "FOREGROUND"],
        Some(&root),
        &paths.logs.join("apache.out"),
        &paths.logs.join("apache.err"),
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file("apache"), pid)?;

    // Wait until the process is still alive and accepts TCP on both listen ports.
    // Do NOT use bind()-fails-as-ready: Windows SO_REUSEADDR lets bind succeed
    // while httpd is already listening, which false-negatived readiness.
    // Keep the poll budget tight so site setting toggles stay under ~2s.
    let http_port = state.stack.apache_http_port;
    let https_port = state.stack.apache_https_port;
    let http_addr = std::net::SocketAddr::from(([127, 0, 0, 1], http_port));
    let https_addr = std::net::SocketAddr::from(([127, 0, 0, 1], https_port));
    let mut died = false;
    // 5ms → 25ms backoff; 3s ceiling covers Windows mpm_winnt's extra warm-up beat.
    let ready = crate::services::perf::poll_backoff(
        std::time::Duration::from_millis(5),
        std::time::Duration::from_millis(25),
        std::time::Duration::from_secs(3),
        || {
            if !is_running(pid) {
                died = true;
                return Some(());
            }
            let http_ok = std::net::TcpStream::connect_timeout(
                &http_addr,
                std::time::Duration::from_millis(100),
            )
            .is_ok();
            let https_ok = std::net::TcpStream::connect_timeout(
                &https_addr,
                std::time::Duration::from_millis(100),
            )
            .is_ok();
            (http_ok && https_ok).then_some(())
        },
    )
    .is_some();
    if died {
        let err = fs::read_to_string(paths.logs.join("apache.err")).unwrap_or_default();
        let log = fs::read_to_string(paths.logs.join("apache/error.log")).unwrap_or_default();
        return Err(DevStackError::Message(format!(
            "Apache exited immediately after start.\n--- apache.err ---\n{err}\n--- error.log ---\n{log}"
        )));
    }
    if !ready {
        let err = fs::read_to_string(paths.logs.join("apache.err")).unwrap_or_default();
        let log = fs::read_to_string(paths.logs.join("apache/error.log")).unwrap_or_default();
        return Err(DevStackError::Message(format!(
            "Apache started (pid {pid}) but ports {http_port}/{https_port} never became ready.\n--- apache.err ---\n{err}\n--- error.log ---\n{log}"
        )));
    }
    state.runtime.apache_pid = Some(pid);
    crate::services::ports::assert_single_owned_listener(http_port, paths, &state.runtime)?;
    crate::services::ports::assert_single_owned_listener(https_port, paths, &state.runtime)?;
    record_apache_fingerprint(paths, pid);
    for site in state.sites.iter_mut().filter(|s| site_uses_apache(s)) {
        site.status = "running".into();
    }
    // Clear stale error/warning once the listener is confirmed ready.
    Ok(())
}

/// Poll (2ms → 15ms, ≤1s) until both Apache listen ports can be bound again.
fn wait_apache_ports_free(http_port: u16, https_port: u16) {
    let _ = crate::services::perf::poll_backoff(
        std::time::Duration::from_millis(2),
        std::time::Duration::from_millis(15),
        std::time::Duration::from_secs(1),
        || {
            let http_free = std::net::TcpListener::bind(("127.0.0.1", http_port)).is_ok();
            let https_free = std::net::TcpListener::bind(("127.0.0.1", https_port)).is_ok();
            (http_free && https_free).then_some(())
        },
    );
}

fn apache_fingerprints() -> &'static std::sync::Mutex<std::collections::HashMap<u32, u64>> {
    static FP: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<u32, u64>>> =
        std::sync::OnceLock::new();
    FP.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Hash of httpd.conf + every vhost conf + (len, mtime) of each referenced cert/key.
/// Apache only reads certs at start, so a changed cert file must change the hash.
fn apache_config_fingerprint(paths: &Paths) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut files = vec![paths.apache_conf_dir().join("httpd.conf")];
    if let Ok(rd) = fs::read_dir(paths.apache_sites_dir()) {
        let mut sites: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        sites.sort();
        files.extend(sites);
    }
    for f in files {
        let body = fs::read_to_string(&f).unwrap_or_default();
        f.hash(&mut hasher);
        body.hash(&mut hasher);
        for line in body.lines() {
            let t = line.trim();
            if let Some(rest) = t
                .strip_prefix("SSLCertificateFile")
                .or_else(|| t.strip_prefix("SSLCertificateKeyFile"))
            {
                let p = rest.trim().trim_matches('"');
                if let Ok(meta) = fs::metadata(p) {
                    meta.len().hash(&mut hasher);
                    if let Ok(m) = meta.modified() {
                        m.hash(&mut hasher);
                    }
                }
            }
        }
    }
    hasher.finish()
}

fn record_apache_fingerprint(paths: &Paths, pid: u32) {
    let fp = apache_config_fingerprint(paths);
    if let Ok(mut g) = apache_fingerprints().lock() {
        g.clear();
        g.insert(pid, fp);
    }
}

/// True when `pid` was started with exactly the config currently on disk and
/// is still accepting connections.
fn apache_fingerprint_matches(paths: &Paths, pid: u32) -> bool {
    let current = apache_config_fingerprint(paths);
    apache_fingerprints()
        .lock()
        .ok()
        .and_then(|g| g.get(&pid).copied())
        == Some(current)
}

pub fn reload_apache(paths: &Paths, state: &mut AppState) -> Result<()> {
    crate::services::vhost_ext::purge_stale_vhost_confs(paths, state);
    let mut runtimes = Vec::new();
    for ver in collect_php_versions(state) {
        if let Ok(rt) = resolve_php(paths, state, &ver) {
            runtimes.push((ver, rt));
        }
    }
    let conf = render_stack(paths, state, &runtimes)?;
    let root = apache_root(paths, state)?;
    let bin = apache_bin(&root)?;
    if let Some(pid) = state.runtime.apache_pid.filter(|p| is_running(*p)) {
        // Windows Lounge builds started with -D FOREGROUND are not a Windows service;
        // `-k graceful` looks for "Apache2.4" and no-ops, so a real config change
        // needs stop+start. We only pay that when the rendered config (httpd.conf,
        // every vhost, and the cert files they reference) differs from what the
        // running process was started with; otherwise the reload is a no-op.
        if cfg!(windows) {
            if apache_fingerprint_matches(paths, pid) {
                return Ok(());
            }
            let _ = stop_pid(pid);
            if let Some(file_pid) = read_pid(&paths.pid_file("apache")) {
                let _ = stop_pid(file_pid);
            }
            state.runtime.apache_pid = None;
            wait_apache_ports_free(state.stack.apache_http_port, state.stack.apache_https_port);
            // Skip configtest + PHP re-start — conf just rendered, PHP-CGI already up.
            return start_apache_inner(paths, state, StartApacheOpts {
                run_configtest: false,
                start_php_runtimes: false,
            });
        }
        let _ = crate::services::process::command(&bin)
            .args([
                "-f",
                &conf.display().to_string(),
                "-k",
                "graceful",
            ])
            .current_dir(&root)
            .status();
        let _ = pid;
        return Ok(());
    }
    start_apache(paths, state)
}

pub fn stop_apache(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.apache_pid.take() {
        let _ = stop_pid(pid);
    }
    if let Some(pid) = read_pid(&paths.pid_file("apache")) {
        let _ = stop_pid(pid);
    }
    for site in state.sites.iter_mut().filter(|s| site_uses_apache(s)) {
        site.status = "stopped".into();
    }
    Ok(())
}

/// Public URL for an Apache-backed site (includes port when not 443/80).
pub fn apache_site_url(state: &AppState, site: &SiteRecord) -> String {
    let https = site.https;
    let port = if https {
        state.stack.apache_https_port
    } else {
        state.stack.apache_http_port
    };
    let scheme = if https { "https" } else { "http" };
    crate::services::ports::format_url(scheme, &site.domain, port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppState, Paths, SiteRecord};
    use crate::services::php_runtime::PhpRuntime;
    use std::path::PathBuf;

    #[test]
    fn windows_php_proxy_rebuilds_script_filename() {
        let rt = PhpRuntime {
            version: "8.4".into(),
            root: PathBuf::from("."),
            php_bin: PathBuf::from("php"),
            fpm_bin: None,
            cgi_bin: None,
            listen: "127.0.0.1:9084".into(),
            is_system: false,
        };
        let block = php_proxy_block(&rt, "C:/Users/Marcin/Sites/test");
        if cfg!(windows) {
            assert!(
                block.contains("ProxyFCGISetEnvIf"),
                "expected ProxyFCGISetEnvIf, got:\n{block}"
            );
            assert!(
                block.contains("SCRIPT_FILENAME"),
                "expected SCRIPT_FILENAME rebuild, got:\n{block}"
            );
            assert!(
                block.contains("proxy:fcgi://127.0.0.1:9084/"),
                "expected trailing-slash SetHandler, got:\n{block}"
            );
            assert!(
                !block.contains("ProxyPassMatch"),
                "ProxyPassMatch embeds /C:/… which php-cgi rejects:\n{block}"
            );
        } else {
            assert!(block.contains("SetHandler"));
            assert!(!block.contains("ProxyFCGISetEnvIf"));
        }
    }

    #[test]
    fn fcgi_target_tcp_has_trailing_slash() {
        let rt = PhpRuntime {
            version: "8.3".into(),
            root: PathBuf::from("."),
            php_bin: PathBuf::from("php"),
            fpm_bin: None,
            cgi_bin: None,
            listen: "127.0.0.1:9085".into(),
            is_system: false,
        };
        assert_eq!(fcgi_target(&rt), "fcgi://127.0.0.1:9085/");
    }

    #[test]
    fn fcgi_target_unix_has_trailing_slash() {
        let rt = PhpRuntime {
            version: "8.3".into(),
            root: PathBuf::from("."),
            php_bin: PathBuf::from("php"),
            fpm_bin: None,
            cgi_bin: None,
            listen: "unix:/tmp/php.sock".into(),
            is_system: false,
        };
        assert_eq!(fcgi_target(&rt), "unix:/tmp/php.sock|fcgi://localhost/");
    }

    #[test]
    fn https_redirect_is_temporary_not_permanent() {
        let dir = std::env::temp_dir().join(format!("devstack-apache-302-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        // Minimal fake apache tree so apache_root resolves.
        let apache_pkg = dir.join("packages/apache/2.4");
        fs::create_dir_all(apache_pkg.join("modules")).unwrap();
        fs::write(apache_pkg.join("httpd.exe"), b"").unwrap();
        fs::write(apache_pkg.join("httpd"), b"").unwrap();
        let site_path = dir.join("sites/blog");
        fs::create_dir_all(&site_path).unwrap();
        let mut state = AppState::default();
        state.stack.apache_version = Some("2.4".into());
        crate::config::mark_installed(
            &mut state,
            "apache",
            crate::config::InstalledRecord {
                version: "2.4".into(),
                path: apache_pkg,
                channel: None,
                installed_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .unwrap();
        state.sites.push(SiteRecord {
            id: "blog".into(),
            name: "blog".into(),
            path: site_path,
            domain: "blog.test".into(),
            php_version: Some("8.3".into()),
            node_version: None,
            https: true,
            web_server: Some("apache".into()),
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: Vec::new(),
            wildcard: false,
        });
        let rt = PhpRuntime {
            version: "8.3".into(),
            root: PathBuf::from("."),
            php_bin: PathBuf::from("php"),
            fpm_bin: None,
            cgi_bin: None,
            listen: "127.0.0.1:9083".into(),
            is_system: false,
        };
        let _ = render_stack(&paths, &state, &[("8.3".into(), rt)]);
        let vhost = fs::read_to_string(paths.apache_sites_dir().join("blog.conf")).unwrap_or_default();
        assert!(
            vhost.contains("Redirect / "),
            "expected temporary Redirect, got:\n{vhost}"
        );
        assert!(
            !vhost.contains("Redirect permanent"),
            "301 Redirect permanent caches in browsers:\n{vhost}"
        );
        assert!(paths.apache_sites_dir().join("00-default.conf").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
