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
use crate::services::php_runtime::{nginx_fastcgi_pass, resolve_php, start_php, PhpRuntime};
use crate::services::process::{
    find_bin, is_running, read_pid, spawn_logged_attached, stop_pid, write_pid,
};
use crate::services::tls::ensure_site_cert;

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
        .or_else(|| runtimes.first().map(|(v, _)| v.clone()))?;
    runtimes
        .iter()
        .find(|(v, r)| *v == want || r.version == want || r.version.starts_with(&want))
        .map(|(_, r)| r)
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

    if sites_dir.exists() {
        for entry in fs::read_dir(&sites_dir)? {
            let _ = fs::remove_file(entry?.path());
        }
    }

    let http_port = state.stack.apache_http_port;
    let https_port = state.stack.apache_https_port;

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
    let Some(runtime) = php_for_site(site, runtimes, state) else {
        return Ok(());
    };
    let root = crate::paths_util::conf_path(&docroot);
    let php = php_proxy_block(runtime, &root);
    // VirtualHost address must match Listen (127.0.0.1) — `*:port` can attach the
    // SSL vhost to the wrong listener on Lounge/Homebrew builds.
    let mut conf = format!(
        r#"<VirtualHost 127.0.0.1:{http_port}>
  ServerName {domain}
  DocumentRoot "{root}"
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
        root = root,
        php = php,
        err = crate::paths_util::conf_path(
            &paths.logs.join(format!("apache/{}-error.log", site.id)),
        ),
        access = crate::paths_util::conf_path(
            &paths.logs.join(format!("apache/{}-access.log", site.id)),
        ),
    );

    if site.https {
        let certs = ensure_site_cert(paths, &site.domain)?;
        conf.push_str(&format!(
            r#"
<VirtualHost 127.0.0.1:{https_port}>
  ServerName {domain}
  DocumentRoot "{root}"
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
    // Reclaim a stale FOREGROUND httpd left from a prior crash / DETACHED spawn.
    if let Some(old) = read_pid(&paths.pid_file("apache")) {
        let _ = stop_pid(old);
        state.runtime.apache_pid = None;
        for _ in 0..20 {
            let http_free =
                std::net::TcpListener::bind(("127.0.0.1", state.stack.apache_http_port)).is_ok();
            let https_free =
                std::net::TcpListener::bind(("127.0.0.1", state.stack.apache_https_port)).is_ok();
            if http_free && https_free {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    crate::services::ports::ensure_ports_free(&state.stack, "apache")?;

    let needs_apache = state.sites.iter().any(site_uses_apache)
        || latest_installed(state, "apache").is_some();
    if !needs_apache {
        return Ok(());
    }

    let mut runtimes = Vec::new();
    if opts.start_php_runtimes {
        for ver in collect_php_versions(state) {
            match start_php(paths, state, &ver) {
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

    // Only start if we have at least one apache site, or force for smoke
    if !state.sites.iter().any(site_uses_apache) {
        return Ok(());
    }

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

    // Wait until the process is still alive and accepts TCP on the HTTPS port.
    // Do NOT use bind()-fails-as-ready: Windows SO_REUSEADDR lets bind succeed
    // while httpd is already listening, which false-negatived readiness.
    // Keep the poll budget tight so site setting toggles stay under ~2s.
    let https_port = state.stack.apache_https_port;
    let mut ready = false;
    for _ in 0..40 {
        if !is_running(pid) {
            let err = fs::read_to_string(paths.logs.join("apache.err")).unwrap_or_default();
            let log = fs::read_to_string(paths.logs.join("apache/error.log")).unwrap_or_default();
            return Err(DevStackError::Message(format!(
                "Apache exited immediately after start.\n--- apache.err ---\n{err}\n--- error.log ---\n{log}"
            )));
        }
        if std::net::TcpStream::connect(("127.0.0.1", https_port)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    if !ready {
        let err = fs::read_to_string(paths.logs.join("apache.err")).unwrap_or_default();
        let log = fs::read_to_string(paths.logs.join("apache/error.log")).unwrap_or_default();
        return Err(DevStackError::Message(format!(
            "Apache started (pid {pid}) but port {https_port} never became ready.\n--- apache.err ---\n{err}\n--- error.log ---\n{log}"
        )));
    }

    state.runtime.apache_pid = Some(pid);
    for site in state.sites.iter_mut().filter(|s| site_uses_apache(s)) {
        site.status = "running".into();
    }
    Ok(())
}

pub fn reload_apache(paths: &Paths, state: &mut AppState) -> Result<()> {
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
        // `-k graceful` looks for "Apache2.4" and no-ops. Restart the process instead.
        if cfg!(windows) {
            let _ = stop_pid(pid);
            if let Some(file_pid) = read_pid(&paths.pid_file("apache")) {
                let _ = stop_pid(file_pid);
            }
            state.runtime.apache_pid = None;
            // Brief poll — mpm_winnt children usually release ports within ~200ms.
            for _ in 0..20 {
                let http_free =
                    std::net::TcpListener::bind(("127.0.0.1", state.stack.apache_http_port)).is_ok();
                let https_free = std::net::TcpListener::bind((
                    "127.0.0.1",
                    state.stack.apache_https_port,
                ))
                .is_ok();
                if http_free && https_free {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
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
}
