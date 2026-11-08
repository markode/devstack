//! Single Nginx instance serving many DevStack sites.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{find_installed, latest_installed, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::php_runtime::{
    nginx_fastcgi_pass, resolve_php, start_php, PhpRuntime,
};
use crate::services::process::{find_bin, is_running, read_pid, spawn_logged, stop_pid, write_pid};
use crate::services::tls::ensure_site_cert;

pub fn nginx_root(_paths: &Paths, state: &AppState) -> Result<PathBuf> {
    if let Some(ver) = &state.stack.nginx_version {
        if let Some(rec) = find_installed(state, "nginx", ver) {
            return Ok(rec.path.clone());
        }
    }
    if let Some(rec) = latest_installed(state, "nginx") {
        return Ok(rec.path.clone());
    }
    if let Ok(bin) = which::which("nginx") {
        return Ok(bin
            .parent()
            .unwrap_or_else(|| Path::new("/usr/sbin"))
            .to_path_buf());
    }
    Err(DevStackError::Message(
        "Nginx is not installed. Install it from Services.".into(),
    ))
}

pub fn nginx_bin(root: &Path) -> Result<PathBuf> {
    if let Some(p) = find_bin(root, &["nginx", "nginx.exe"]) {
        return Ok(p);
    }
    which::which("nginx").map_err(|_| DevStackError::Message("nginx binary not found".into()))
}

pub fn render_stack(
    paths: &Paths,
    state: &AppState,
    php_runtimes: &[(String, PhpRuntime)],
) -> Result<PathBuf> {
    let conf_dir = paths.nginx_conf_dir();
    let sites_dir = paths.nginx_sites_dir();
    fs::create_dir_all(&conf_dir)?;
    fs::create_dir_all(&sites_dir)?;
    fs::create_dir_all(paths.nginx.join("temp"))?;
    fs::create_dir_all(paths.logs.join("nginx"))?;
    // Some nginx builds also probe `$prefix/logs/error.log` before reading conf.
    fs::create_dir_all(paths.nginx.join("logs"))?;

    if sites_dir.exists() {
        for entry in fs::read_dir(&sites_dir)? {
            let entry = entry?;
            let _ = fs::remove_file(entry.path());
        }
    }

    let http_port = state.stack.http_port;
    let https_port = state.stack.https_port;

    // Catch-all first (lexicographically `00-`) so unmatched Host never hits
    // mailpit/phpMyAdmin proxy blocks.
    write_default_catch_all(&sites_dir, http_port, https_port)?;

    for site in &state.sites {
        if crate::services::httpd::site_uses_apache(site) {
            continue;
        }
        write_site_conf(
            paths,
            state,
            site,
            php_runtimes,
            &sites_dir,
            http_port,
            https_port,
        )?;
    }
    write_phpmyadmin_conf(
        paths,
        state,
        php_runtimes,
        &sites_dir,
        http_port,
        https_port,
    )?;
    // Mailpit vhost only when the process is actually running — never as a
    // default_server / catch-all for other Host headers.
    let mailpit_up = state
        .runtime
        .mailpit_pid
        .map(crate::services::process::is_running)
        .unwrap_or(false);
    if mailpit_up && latest_installed(state, "mailpit").is_some() {
        let _ = crate::services::extras::write_mailpit_nginx(paths, state);
    }

    let temp = paths.nginx.join("temp");
    for sub in ["client_body", "proxy", "fastcgi", "uwsgi", "scgi"] {
        fs::create_dir_all(temp.join(sub))?;
    }

    let conf = conf_dir.join("nginx.conf");
    let body = format!(
        r#"worker_processes auto;
error_log {error_log} warn;
pid {pid};
events {{ worker_connections 1024; }}
http {{
  include       {mime};
  default_type  application/octet-stream;
  sendfile on;
  keepalive_timeout 65;
  client_max_body_size 0;
  client_body_timeout 3600s;
  proxy_read_timeout 3600s;
  fastcgi_read_timeout 3600s;
  fastcgi_send_timeout 3600s;
  fastcgi_connect_timeout 60s;
  client_body_temp_path {temp}/client_body;
  proxy_temp_path {temp}/proxy;
  fastcgi_temp_path {temp}/fastcgi;
  uwsgi_temp_path {temp}/uwsgi;
  scgi_temp_path {temp}/scgi;
  access_log {access_log};
  include {sites}/*.conf;
}}
"#,
        error_log = crate::paths_util::conf_path(&paths.logs.join("nginx/error.log")),
        pid = crate::paths_util::conf_path(&paths.pid_file("nginx")),
        access_log = crate::paths_util::conf_path(&paths.logs.join("nginx/access.log")),
        sites = crate::paths_util::conf_path(&sites_dir),
        mime = crate::paths_util::conf_path(&conf_dir.join("mime.types")),
        temp = crate::paths_util::conf_path(&temp),
    );
    fs::write(&conf, body)?;

    let mime = conf_dir.join("mime.types");
    if !mime.exists() {
        fs::write(
            &mime,
            "types {\n  text/html html htm shtml;\n  text/css css;\n  application/javascript js;\n  application/json json;\n  image/png png;\n  image/jpeg jpeg jpg;\n  image/gif gif;\n  image/svg+xml svg;\n  application/pdf pdf;\n  text/plain txt;\n}\n",
        )?;
    }
    ensure_fastcgi_params(paths)?;
    Ok(conf)
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

fn write_default_catch_all(sites_dir: &Path, http_port: u16, _https_port: u16) -> Result<()> {
    // HTTP-only default_server — never proxies. Unmatched Host → clear 404.
    // (Avoid ssl default_server here: needs certs and varies by nginx build.)
    let body = format!(
        r#"server {{
  listen {http_port} default_server;
  listen [::]:{http_port} default_server;
  server_name _;
  return 404 "DevStack: no site matched this Host header.\n";
}}
"#
    );
    fs::write(sites_dir.join("00-default.conf"), body)?;
    Ok(())
}

fn write_site_conf(
    paths: &Paths,
    state: &AppState,
    site: &SiteRecord,
    runtimes: &[(String, PhpRuntime)],
    sites_dir: &Path,
    http_port: u16,
    https_port: u16,
) -> Result<()> {
    let root = expand_home(&site.path);
    // Always write a server block for the site domain — never leave Host
    // unmatched so nginx falls through to mailpit/phpMyAdmin.
    let php_block = if let Some(runtime) = php_for_site(site, runtimes, state) {
        let fastcgi = nginx_fastcgi_pass(runtime);
        format!(
            r#"  location ~ \.php$ {{
    include {fcgi};
    fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
    fastcgi_pass {fastcgi};
    fastcgi_read_timeout 3600s;
    fastcgi_send_timeout 3600s;
  }}
"#,
            fastcgi = fastcgi_pass_value(&fastcgi),
            fcgi = crate::paths_util::quoted_conf_path(
                &paths.nginx_conf_dir().join("fastcgi_params"),
            ),
        )
    } else {
        String::new()
    };
    let mut server = format!(
        r#"server {{
  listen {http_port};
  listen [::]:{http_port};
  server_name {domain};
  root {root};
  index index.php index.html;
  client_max_body_size 0;
  location / {{
    try_files $uri $uri/ /index.php?$query_string;
  }}
{php_block}}}
"#,
        domain = site.domain,
        root = crate::paths_util::quoted_conf_path(&root),
        php_block = php_block,
    );

    if site.https {
        let certs = ensure_site_cert(paths, &site.domain)?;
        let php_ssl = if let Some(runtime) = php_for_site(site, runtimes, state) {
            let fastcgi = nginx_fastcgi_pass(runtime);
            format!(
                r#"  location ~ \.php$ {{
    include {fcgi};
    fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
    fastcgi_pass {fastcgi};
    fastcgi_read_timeout 3600s;
    fastcgi_send_timeout 3600s;
  }}
"#,
                fastcgi = fastcgi_pass_value(&fastcgi),
                fcgi = crate::paths_util::quoted_conf_path(
                    &paths.nginx_conf_dir().join("fastcgi_params"),
                ),
            )
        } else {
            String::new()
        };
        server.push_str(&format!(
            r#"
server {{
  listen {https_port} ssl;
  listen [::]:{https_port} ssl;
  server_name {domain};
  root {root};
  index index.php index.html;
  client_max_body_size 0;
  ssl_certificate     {cert};
  ssl_certificate_key {key};
  location / {{
    try_files $uri $uri/ /index.php?$query_string;
  }}
{php_ssl}}}
"#,
            domain = site.domain,
            root = crate::paths_util::quoted_conf_path(&root),
            cert = crate::paths_util::quoted_conf_path(&certs.site_cert),
            key = crate::paths_util::quoted_conf_path(&certs.site_key),
            php_ssl = php_ssl,
        ));
    }

    fs::write(sites_dir.join(format!("{}.conf", site.id)), server)?;
    Ok(())
}

fn write_phpmyadmin_conf(
    paths: &Paths,
    state: &AppState,
    runtimes: &[(String, PhpRuntime)],
    sites_dir: &Path,
    http_port: u16,
    https_port: u16,
) -> Result<()> {
    let Some(pma) = latest_installed(state, "phpmyadmin") else {
        return Ok(());
    };
    let Some((_, runtime)) = runtimes.first() else {
        return Ok(());
    };
    let root = crate::paths_util::quoted_conf_path(&pma.path);
    let fastcgi = nginx_fastcgi_pass(runtime);
    let certs = ensure_site_cert(paths, "phpmyadmin.test")?;
    let body = format!(
        r#"server {{
  listen {http_port};
  listen {https_port} ssl;
  server_name phpmyadmin.test;
  root {root};
  index index.php;
  client_max_body_size 0;
  ssl_certificate {cert};
  ssl_certificate_key {key};
  location / {{
    try_files $uri $uri/ /index.php?$query_string;
  }}
  location ~ \.php$ {{
    include {fcgi};
    fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
    fastcgi_pass {fastcgi};
    fastcgi_read_timeout 3600s;
  }}
}}
"#,
        root = root,
        cert = crate::paths_util::quoted_conf_path(&certs.site_cert),
        key = crate::paths_util::quoted_conf_path(&certs.site_key),
        fastcgi = fastcgi_pass_value(&fastcgi),
        fcgi = crate::paths_util::quoted_conf_path(
            &paths.nginx_conf_dir().join("fastcgi_params"),
        ),
    );
    fs::write(sites_dir.join("phpmyadmin.conf"), body)?;
    Ok(())
}

fn fastcgi_pass_value(listen: &str) -> String {
    listen.to_string()
}

fn ensure_fastcgi_params(paths: &Paths) -> Result<()> {
    let p = paths.nginx_conf_dir().join("fastcgi_params");
    if p.exists() {
        return Ok(());
    }
    fs::write(
        p,
        r#"fastcgi_param  QUERY_STRING       $query_string;
fastcgi_param  REQUEST_METHOD     $request_method;
fastcgi_param  CONTENT_TYPE       $content_type;
fastcgi_param  CONTENT_LENGTH     $content_length;
fastcgi_param  SCRIPT_NAME        $fastcgi_script_name;
fastcgi_param  REQUEST_URI        $request_uri;
fastcgi_param  DOCUMENT_URI       $document_uri;
fastcgi_param  DOCUMENT_ROOT      $document_root;
fastcgi_param  SERVER_PROTOCOL    $server_protocol;
fastcgi_param  REQUEST_SCHEME     $scheme;
fastcgi_param  HTTPS              $https if_not_empty;
fastcgi_param  GATEWAY_INTERFACE  CGI/1.1;
fastcgi_param  SERVER_SOFTWARE    nginx;
fastcgi_param  REMOTE_ADDR        $remote_addr;
fastcgi_param  REMOTE_PORT        $remote_port;
fastcgi_param  SERVER_ADDR        $server_addr;
fastcgi_param  SERVER_PORT        $server_port;
fastcgi_param  SERVER_NAME        $server_name;
"#,
    )?;
    Ok(())
}

pub fn expand_home(path: &Path) -> PathBuf {
    crate::paths_util::expand_home(path)
}

fn collect_php_versions(state: &AppState) -> Vec<String> {
    let mut versions: Vec<String> = state
        .sites
        .iter()
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

pub fn start_nginx(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.nginx_pid {
        if is_running(pid) {
            return reload_nginx(paths, state);
        }
    }
    crate::services::ports::ensure_ports_free(&state.stack, "nginx")?;

    let versions = collect_php_versions(state);
    let mut runtimes = Vec::new();
    for v in &versions {
        match start_php(paths, state, v) {
            Ok(rt) => runtimes.push((v.clone(), rt)),
            Err(e) => eprintln!("php start {v}: {e}"),
        }
    }
    if runtimes.is_empty() {
        return Err(DevStackError::Message("no PHP runtime available".into()));
    }

    let conf = render_stack(paths, state, &runtimes)?;
    let root = nginx_root(paths, state)?;
    let bin = nginx_bin(&root)?;
    let prefix = paths.nginx.clone();
    let log_out = paths.logs.join("nginx/stdout.log");
    let log_err = paths.logs.join("nginx/stderr.log");
    let conf_s = conf.to_string_lossy().to_string();
    let prefix_s = prefix.to_string_lossy().to_string();
    let child = spawn_logged(
        &bin,
        &["-c", &conf_s, "-p", &prefix_s, "-g", "daemon off;"],
        Some(&root),
        &log_out,
        &log_err,
    )?;
    let pid = child.id();
    std::mem::forget(child);
    write_pid(&paths.pid_file("nginx"), pid)?;
    state.runtime.nginx_pid = Some(pid);
    // Only mark Nginx-backed sites running — Apache sites keep their own status.
    for site in &mut state.sites {
        if !crate::services::httpd::site_uses_apache(site) {
            site.status = "running".into();
        }
    }
    Ok(())
}

pub fn reload_nginx(paths: &Paths, state: &mut AppState) -> Result<()> {
    let versions = collect_php_versions(state);
    let mut runtimes = Vec::new();
    for v in versions {
        match start_php(paths, state, &v) {
            Ok(rt) => runtimes.push((v, rt)),
            Err(_) => {
                if let Ok(rt) = resolve_php(paths, state, &v) {
                    runtimes.push((v, rt));
                }
            }
        }
    }
    if runtimes.is_empty() {
        if let Ok(rt) = resolve_php(paths, state, "8.3") {
            runtimes.push(("8.3".into(), rt));
        }
    }
    let conf = render_stack(paths, state, &runtimes)?;
    let root = nginx_root(paths, state)?;
    let bin = nginx_bin(&root)?;
    let conf_s = conf.display().to_string();
    let prefix_s = paths.nginx.display().to_string();

    // Validate before signalling the master — surface syntax errors in the UI.
    let test = crate::services::process::command(&bin)
        .args(["-t", "-c", &conf_s, "-p", &prefix_s])
        .output();
    if let Ok(out) = &test {
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            return Err(DevStackError::Message(format!(
                "Nginx config test failed: {stderr}{stdout}"
            )));
        }
    }

    if state
        .runtime
        .nginx_pid
        .filter(|p| is_running(*p))
        .is_none()
    {
        return start_nginx(paths, state);
    }

    let status = crate::services::process::command(&bin)
        .args(["-c", &conf_s, "-p", &prefix_s, "-s", "reload"])
        .status();
    if status.map(|s| s.success()).unwrap_or(false) {
        return Ok(());
    }
    stop_nginx(paths, state)?;
    start_nginx(paths, state)
}

pub fn stop_nginx(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.nginx_pid.take() {
        stop_pid(pid)?;
    }
    if let Some(pid) = read_pid(&paths.pid_file("nginx")) {
        let _ = stop_pid(pid);
    }
    for site in &mut state.sites {
        if !crate::services::httpd::site_uses_apache(site) {
            site.status = "stopped".into();
        }
    }
    Ok(())
}
