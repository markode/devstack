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
use crate::services::process::{find_bin, is_running, read_pid, spawn_logged, stop_pid, write_pid};
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
        // Apache proxy_fcgi Unix socket form
        format!("unix:{path}|fcgi://localhost")
    } else {
        format!("fcgi://{pass}")
    }
}

fn module_line(root: &Path, name: &str) -> Option<String> {
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
                c.display()
            ));
        }
    }
    // Apache Lounge / static package: relative modules/ only when that dir exists.
    // Skip when missing — many distro builds compile log_config/unixd in.
    let rel = root.join("modules").join(format!("mod_{name}.so"));
    if root.join("modules").is_dir() {
        return Some(format!(
            "LoadModule {name}_module \"{}\"",
            rel.display()
        ));
    }
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
            // Skip modules that don't exist on this layout (e.g. unixd on Windows)
            let path_hint = line.split('"').nth(1).unwrap_or("");
            if !path_hint.is_empty() && !Path::new(path_hint).exists() && system_layout {
                continue;
            }
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
        root.display().to_string()
    };
    let runtime_dir = paths.apache.join("run");
    fs::create_dir_all(&runtime_dir)?;
    let mime = if root.join("conf").join("mime.types").exists() {
        root.join("conf").join("mime.types")
    } else if Path::new("/etc/mime.types").exists() {
        PathBuf::from("/etc/mime.types")
    } else {
        root.join("conf").join("mime.types")
    };
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
            runtime_dir.display()
        )
    };
    let body = format!(
        r#"ServerRoot "{server_root}"
Listen {http_port}
Listen {https_port}
{load_modules}
ServerName localhost
ServerAdmin admin@localhost
PidFile "{pid}"
ErrorLog "{error_log}"
LogLevel warn
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
        // Avoid a libc dependency — parse from /proc when available, else assume non-root.
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("Uid:"))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or(1000)
    }
    #[cfg(not(unix))]
    {
        0
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
    let fcgi = fcgi_target(runtime);
    let mut conf = format!(
        r#"<VirtualHost *:{http_port}>
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
  <FilesMatch "\.php$">
    SetHandler "proxy:{fcgi}"
  </FilesMatch>
  ProxyTimeout 3600
  ErrorLog "{err}"
  CustomLog "{access}" common
</VirtualHost>
"#,
        domain = site.domain,
        root = crate::paths_util::conf_path(&docroot),
        fcgi = fcgi,
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
<VirtualHost *:{https_port}>
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
  <FilesMatch "\.php$">
    SetHandler "proxy:{fcgi}"
  </FilesMatch>
  ProxyTimeout 3600
  ErrorLog "{err}"
  CustomLog "{access}" common
</VirtualHost>
"#,
            domain = site.domain,
            root = crate::paths_util::conf_path(&docroot),
            cert = crate::paths_util::conf_path(&certs.site_cert),
            key = crate::paths_util::conf_path(&certs.site_key),
            fcgi = fcgi,
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

pub fn start_apache(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.apache_pid {
        if is_running(pid) {
            return reload_apache(paths, state);
        }
    }
    crate::services::ports::ensure_ports_free(&state.stack, "apache")?;

    let needs_apache = state.sites.iter().any(site_uses_apache)
        || latest_installed(state, "apache").is_some();
    if !needs_apache {
        return Ok(());
    }

    let mut runtimes = Vec::new();
    for ver in collect_php_versions(state) {
        match start_php(paths, state, &ver) {
            Ok(rt) => runtimes.push((ver, rt)),
            Err(e) => {
                eprintln!("apache: skip php {ver}: {e}");
            }
        }
    }
    // Also resolve without starting if already up
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
    let child = spawn_logged(
        &bin,
        &["-f", &conf_s, "-D", "FOREGROUND"],
        Some(&root),
        &paths.logs.join("apache.out"),
        &paths.logs.join("apache.err"),
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file("apache"), pid)?;
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
