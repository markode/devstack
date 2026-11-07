//! One-click WordPress: download core + WP-CLI, create DB, wp core install.

use std::fs;
use std::path::{Path, PathBuf};

use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};

use crate::config::{slugify, AppState, Paths, SiteRecord};
use crate::downloads::{
    download_file, extract_archive, fetch_text, http_client, verify_sha1, verify_sha512,
    ArchiveKind, ProgressFn,
};
use crate::error::{DevStackError, Result};
use crate::services::database::ensure_database;
use crate::services::nginx::expand_home;
use crate::services::php_runtime::resolve_php;
use crate::services::progress::{emit, JobProgressFn};
use crate::services::sites::detect_site;
use crate::services::stack::reload_sites;

const WP_CLI_URL: &str =
    "https://raw.githubusercontent.com/wp-cli/builds/gh-pages/phar/wp-cli.phar";
const WP_CLI_SHA_URL: &str =
    "https://raw.githubusercontent.com/wp-cli/builds/gh-pages/phar/wp-cli.phar.sha512";
const WP_LATEST_TGZ: &str = "https://wordpress.org/latest.tar.gz";
const WP_LATEST_SHA1: &str = "https://wordpress.org/latest.tar.gz.sha1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWordPressRequest {
    pub name: String,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub php_version: Option<String>,
    pub database_engine: Option<String>,
    pub locale: Option<String>,
    pub admin_user: Option<String>,
    pub site_title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWordPressResult {
    pub site: SiteRecord,
    pub admin_user: String,
    pub admin_password: String,
    pub admin_email: String,
    pub url: String,
    pub wordpress_version: String,
}

pub async fn ensure_wp_cli(paths: &Paths, progress: &JobProgressFn, job_id: &str) -> Result<PathBuf> {
    let dest = paths.tools.join("wp-cli.phar");
    if dest.exists() && fs::metadata(&dest).map(|m| m.len() > 100_000).unwrap_or(false) {
        return Ok(dest);
    }
    emit(progress, job_id, "wp-cli", "Downloading WP-CLI…", 0, None);
    let client = http_client()?;
    let sha_raw = fetch_text(&client, WP_CLI_SHA_URL).await?;
    let sha = sha_raw
        .split_whitespace()
        .next()
        .ok_or_else(|| DevStackError::Message("empty WP-CLI checksum".into()))?;
    let dl: ProgressFn = {
        let progress = progress.clone();
        let job_id = job_id.to_string();
        std::sync::Arc::new(move |ev| {
            emit(
                &progress,
                &job_id,
                "wp-cli",
                "Downloading WP-CLI…",
                ev.downloaded,
                ev.total,
            );
        })
    };
    download_file(&client, WP_CLI_URL, &dest, "wp-cli", "latest", &dl).await?;
    verify_sha512(&dest, sha)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms)?;
    }
    Ok(dest)
}

pub async fn download_wordpress_core(
    paths: &Paths,
    dest_dir: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<String> {
    fs::create_dir_all(dest_dir)?;
    let client = http_client()?;
    emit(
        progress,
        job_id,
        "wordpress",
        "Fetching WordPress checksum…",
        0,
        None,
    );
    let sha_raw = fetch_text(&client, WP_LATEST_SHA1).await?;
    let sha1 = sha_raw
        .split_whitespace()
        .next()
        .ok_or_else(|| DevStackError::Message("empty WordPress SHA1".into()))?
        .to_string();

    // Discover version label from version-check (best-effort)
    let version = fetch_wp_version(&client).await.unwrap_or_else(|| "latest".into());

    let archive = paths.downloads.join(format!("wordpress-{version}.tar.gz"));
    emit(
        progress,
        job_id,
        "wordpress",
        &format!("Downloading WordPress {version}…"),
        0,
        None,
    );
    let dl: ProgressFn = {
        let progress = progress.clone();
        let job_id = job_id.to_string();
        std::sync::Arc::new(move |ev| {
            emit(
                &progress,
                &job_id,
                "wordpress",
                "Downloading WordPress…",
                ev.downloaded,
                ev.total,
            );
        })
    };
    if !(archive.exists()
        && verify_sha1(&archive, &sha1).is_ok())
    {
        download_file(
            &client,
            WP_LATEST_TGZ,
            &archive,
            "wordpress",
            &version,
            &dl,
        )
        .await?;
        verify_sha1(&archive, &sha1)?;
    }

    emit(
        progress,
        job_id,
        "extract",
        "Extracting WordPress…",
        0,
        None,
    );
    extract_archive(&archive, dest_dir, ArchiveKind::TarGz, 1)?;
    Ok(version)
}

async fn fetch_wp_version(client: &reqwest::Client) -> Option<String> {
    #[derive(Deserialize)]
    struct Offer {
        version: Option<String>,
        response: Option<String>,
    }
    #[derive(Deserialize)]
    struct Resp {
        offers: Option<Vec<Offer>>,
    }
    let resp: Resp = client
        .get("https://api.wordpress.org/core/version-check/1.7/")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    resp.offers.and_then(|offers| {
        offers
            .into_iter()
            .find(|o| o.response.as_deref() == Some("upgrade") || o.version.is_some())
            .and_then(|o| o.version)
    })
}

fn random_password(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn safe_db_ident(input: &str) -> String {
    let mut s: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s = "wp".into();
    }
    if s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        s = format!("wp_{s}");
    }
    s.truncate(48);
    s
}

pub fn run_wp_cli(
    php: &Path,
    wp_cli: &Path,
    cwd: &Path,
    args: &[&str],
) -> Result<String> {
    run_wp_cli_with_ini(php, wp_cli, cwd, args, None)
}

pub fn run_wp_cli_with_ini(
    php: &Path,
    wp_cli: &Path,
    cwd: &Path,
    args: &[&str],
    php_install_dir: Option<&Path>,
) -> Result<String> {
    let mut cmd = crate::services::process::command(php);
    if let Some(dir) = php_install_dir {
        crate::php::apply_php_ini_to_command(&mut cmd, dir);
    } else if let Some(parent) = php.parent() {
        crate::php::apply_php_ini_to_command(&mut cmd, parent);
    }
    cmd.arg(wp_cli).args(args).current_dir(cwd);
    let out = cmd
        .output()
        .map_err(|e| DevStackError::Message(format!("wp-cli failed to start: {e}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        return Err(DevStackError::Message(format!(
            "wp-cli error: {stderr}{stdout}"
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub async fn create_wordpress_site(
    paths: &Paths,
    state: &mut AppState,
    req: CreateWordPressRequest,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<CreateWordPressResult> {
    let id = slugify(&req.name);
    let domain = req
        .domain
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| format!("{id}.test"));
    let domain = if domain.ends_with(".test") {
        domain
    } else {
        format!("{domain}.test")
    };

    let site_path = req
        .path
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.sites_meta.join(&id));
    let abs = expand_home(&site_path);
    fs::create_dir_all(&abs)?;

    let php_version = req
        .php_version
        .or_else(|| state.stack.php_version.clone())
        .or_else(|| {
            state
                .installed
                .get("php")
                .and_then(|v| v.first())
                .map(|r| r.version.clone())
        })
        .unwrap_or_else(|| "8.3".into());

    let engine = req
        .database_engine
        .or_else(|| state.active_database.clone())
        .or_else(|| state.runtime.database_engine.clone())
        .unwrap_or_else(|| "mariadb".into());

    let wp_cli = ensure_wp_cli(paths, progress, job_id).await?;
    let version = download_wordpress_core(paths, &abs, progress, job_id).await?;

    let php_rt = resolve_php(paths, state, &php_version)?;
    let db_name = safe_db_ident(&format!("wp_{id}"));
    let db_user = safe_db_ident(&format!("u_{id}"));
    let db_pass = random_password(24);

    emit(
        progress,
        job_id,
        "database",
        &format!("Starting {engine} and creating `{db_name}`…"),
        0,
        None,
    );
    // WP-CLI needs a live server + mysqli — start DB and regenerate php.ini first.
    let _ = crate::php::write_dev_php_ini(&php_rt.root, &crate::php::PhpIniOptions::default());
    crate::services::database::start_database(paths, state, &engine)?;
    ensure_database(paths, state, &engine, &db_name, &db_user, &db_pass)?;

    // WordPress defaults to Apache — use Apache HTTPS port in siteurl.
    let https_port = state.stack.apache_https_port;
    let public_url =
        crate::services::ports::format_url("https", &domain, https_port);
    let db_host = crate::services::ports::mysql_db_host(
        crate::services::ports::engine_db_port(&state.stack, &engine),
    );

    emit(progress, job_id, "config", "Writing wp-config.php…", 0, None);
    // Use TCP 127.0.0.1[:port] — custom sockets confuse WP's host:port parsing
    run_wp_cli_with_ini(
        &php_rt.php_bin,
        &wp_cli,
        &abs,
        &[
            "config",
            "create",
            &format!("--dbname={db_name}"),
            &format!("--dbuser={db_user}"),
            &format!("--dbpass={db_pass}"),
            &format!("--dbhost={db_host}"),
            "--dbcharset=utf8mb4",
            "--skip-check",
            "--force",
        ],
        Some(&php_rt.root),
    )?;

    let admin_user = req
        .admin_user
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "admin".into());
    let admin_password = random_password(16);
    let admin_email = format!("admin@{domain}");
    let title = req
        .site_title
        .unwrap_or_else(|| req.name.clone());
    let locale = req.locale.unwrap_or_else(|| "en_US".into());

    emit(
        progress,
        job_id,
        "install",
        "Running wp core install…",
        0,
        None,
    );
    let mut install_args = vec![
        "core".into(),
        "install".into(),
        format!("--url={public_url}"),
        format!("--title={title}"),
        format!("--admin_user={admin_user}"),
        format!("--admin_password={admin_password}"),
        format!("--admin_email={admin_email}"),
        "--skip-email".into(),
    ];
    if locale != "en_US" {
        install_args.push(format!("--locale={locale}"));
    }
    let arg_refs: Vec<&str> = install_args.iter().map(|s| s.as_str()).collect();
    run_wp_cli_with_ini(
        &php_rt.php_bin,
        &wp_cli,
        &abs,
        &arg_refs,
        Some(&php_rt.root),
    )?;

    // Pretty permalinks need Apache AllowOverride + mod_rewrite (.htaccess).
    let _ = run_wp_cli_with_ini(
        &php_rt.php_bin,
        &wp_cli,
        &abs,
        &["rewrite", "structure", "/%postname%/", "--hard"],
        Some(&php_rt.root),
    );
    ensure_wordpress_htaccess(&abs)?;

    let mut site = SiteRecord {
        id: id.clone(),
        name: req.name.clone(),
        path: site_path,
        domain: domain.clone(),
        php_version: Some(php_version),
        node_version: None,
        https: true,
        // WordPress defaults to Apache for full .htaccess / pretty permalinks.
        web_server: Some("apache".into()),
        framework: Some("wordpress".into()),
        database_engine: Some(engine),
        database_name: Some(db_name),
        database_user: Some(db_user),
        database_password: Some(db_pass),
        status: "running".into(),
        is_template: false,
    };
    let details = detect_site(&site);
    site.framework = Some(details.framework);

    state.sites.retain(|s| s.id != site.id);
    state.sites.push(site.clone());
    state.active_database = site.database_engine.clone();

    let _ = reload_sites(paths, state);

    emit(
        progress,
        job_id,
        "done",
        "WordPress site ready",
        100,
        Some(100),
    );

    Ok(CreateWordPressResult {
        site,
        admin_user,
        admin_password,
        admin_email,
        url: public_url,
        wordpress_version: version,
    })
}

/// Standard WordPress rewrite rules — WP-CLI `--hard` may skip writing when it
/// cannot detect Apache; DevStack Apache vhosts always use `AllowOverride All`.
pub fn ensure_wordpress_htaccess(site_path: &Path) -> Result<()> {
    let path = expand_home(site_path).join(".htaccess");
    let body = "\
# BEGIN WordPress
<IfModule mod_rewrite.c>
RewriteEngine On
RewriteRule .* - [E=HTTP_AUTHORIZATION:%{HTTP:Authorization}]
RewriteBase /
RewriteRule ^index\\.php$ - [L]
RewriteCond %{REQUEST_FILENAME} !-f
RewriteCond %{REQUEST_FILENAME} !-d
RewriteRule . /index.php [L]
</IfModule>
# END WordPress
";
    if path.exists() {
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if existing.contains("RewriteRule . /index.php") || existing.contains("RewriteRule ./index.php") {
            return Ok(());
        }
    }
    fs::write(path, body)?;
    Ok(())
}

pub fn update_wp_config_credentials(
    site_path: &Path,
    db_name: &str,
    db_user: &str,
    db_pass: &str,
    db_host: &str,
    _sock: &Path,
) -> Result<()> {
    let cfg = expand_home(site_path).join("wp-config.php");
    let mut raw = fs::read_to_string(&cfg)?;
    for (key, val) in [
        ("DB_NAME", db_name),
        ("DB_USER", db_user),
        ("DB_PASSWORD", db_pass),
        ("DB_HOST", db_host),
    ] {
        let re = regex::Regex::new(&format!(
            r#"define\(\s*['\"]{key}['\"]\s*,\s*['\"][^'\"]*['\"]\s*\)"#
        ))
        .map_err(|e| DevStackError::Message(e.to_string()))?;
        raw = re
            .replace(&raw, format!("define('{key}', '{val}')"))
            .to_string();
    }
    fs::write(cfg, raw)?;
    Ok(())
}

