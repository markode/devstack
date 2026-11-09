//! One-click project templates: Laravel, Symfony, Statamic, Drupal, Joomla, WooCommerce.
//!
//! Composer-based kinds use the managed `composer.phar` + managed PHP. WooCommerce
//! reuses the WordPress installer and adds the plugin through WP-CLI. A database is
//! created automatically (soft-fail when no engine is running); network failures
//! never lose the scaffold (`DEVSTACK_SKIP_TEMPLATE_NETWORK=1` skips network entirely).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{default_sites_root, slugify, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::progress::{emit, JobProgressFn};

pub const SKIP_NETWORK_ENV: &str = "DEVSTACK_SKIP_TEMPLATE_NETWORK";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    Laravel,
    Symfony,
    Statamic,
    Drupal,
    Joomla,
    Woocommerce,
}

pub const ALL_KINDS: &[TemplateKind] = &[
    TemplateKind::Laravel,
    TemplateKind::Symfony,
    TemplateKind::Statamic,
    TemplateKind::Drupal,
    TemplateKind::Joomla,
    TemplateKind::Woocommerce,
];

impl TemplateKind {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "laravel" => Self::Laravel,
            "symfony" => Self::Symfony,
            "statamic" => Self::Statamic,
            "drupal" => Self::Drupal,
            "joomla" => Self::Joomla,
            "woocommerce" | "woo" => Self::Woocommerce,
            other => return Err(DevStackError::Message(format!("unknown site template: {other}"))),
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Laravel => "laravel",
            Self::Symfony => "symfony",
            Self::Statamic => "statamic",
            Self::Drupal => "drupal",
            Self::Joomla => "joomla",
            Self::Woocommerce => "woocommerce",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Laravel => "Laravel",
            Self::Symfony => "Symfony",
            Self::Statamic => "Statamic",
            Self::Drupal => "Drupal",
            Self::Joomla => "Joomla",
            Self::Woocommerce => "WooCommerce",
        }
    }

    /// Sub-folder served as the web root (relative to the project).
    pub fn docroot(self) -> &'static str {
        match self {
            Self::Laravel | Self::Symfony | Self::Statamic => "public",
            Self::Drupal => "web",
            Self::Joomla | Self::Woocommerce => "",
        }
    }

    /// Framework label stored on the site.
    pub fn framework(self) -> &'static str {
        match self {
            Self::Woocommerce => "wordpress",
            other => other.id(),
        }
    }

    pub fn needs_composer(self) -> bool {
        matches!(self, Self::Laravel | Self::Symfony | Self::Statamic | Self::Drupal)
    }
}

/// `composer …` argument lists (without the leading `composer`) for a kind.
pub fn composer_commands(kind: TemplateKind, target: &Path) -> Vec<Vec<String>> {
    let t = target.display().to_string();
    let create = |pkg: &str| {
        vec![
            "create-project".to_string(),
            pkg.to_string(),
            t.clone(),
            "--no-interaction".to_string(),
            "--prefer-dist".to_string(),
        ]
    };
    match kind {
        TemplateKind::Laravel => vec![create("laravel/laravel")],
        TemplateKind::Symfony => vec![create("symfony/skeleton")],
        TemplateKind::Statamic => vec![create("statamic/statamic")],
        TemplateKind::Drupal => vec![create("drupal/recommended-project")],
        TemplateKind::Joomla | TemplateKind::Woocommerce => vec![],
    }
}

/// `wp …` argument lists for kinds that finish via WP-CLI.
pub fn wp_cli_commands(kind: TemplateKind) -> Vec<Vec<String>> {
    match kind {
        TemplateKind::Woocommerce => vec![vec![
            "plugin".into(),
            "install".into(),
            "woocommerce".into(),
            "--activate".into(),
        ]],
        _ => vec![],
    }
}

/// Human-readable list of the commands a kind runs (for UI + tests).
pub fn command_summary(kind: TemplateKind) -> Vec<String> {
    let dir = Path::new("<dir>");
    let mut out: Vec<String> = composer_commands(kind, dir)
        .into_iter()
        .map(|a| format!("composer {}", a.join(" ")))
        .collect();
    match kind {
        TemplateKind::Joomla => out.push("download Joomla full package (joomla/joomla-cms release) and unzip".into()),
        TemplateKind::Woocommerce => {
            out.push("wp core download + config + install (WordPress)".into());
            out.extend(wp_cli_commands(kind).into_iter().map(|a| format!("wp {}", a.join(" "))));
        }
        _ => {}
    }
    out
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    pub kind: String,
    pub label: String,
    pub docroot: String,
    pub commands: Vec<String>,
}

pub fn list_template_kinds() -> Vec<TemplateInfo> {
    ALL_KINDS
        .iter()
        .map(|k| TemplateInfo {
            kind: k.id().into(),
            label: k.label().into(),
            docroot: k.docroot().into(),
            commands: command_summary(*k),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFromTemplateRequest {
    pub kind: String,
    pub name: String,
    pub path: Option<String>,
    pub php_version: Option<String>,
    pub database_engine: Option<String>,
    pub web_server: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateStepResult {
    pub name: String,
    pub ok: bool,
    pub skipped: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCredentials {
    pub engine: String,
    pub name: String,
    pub user: String,
    pub password: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFromTemplateResult {
    pub site: SiteRecord,
    pub kind: String,
    pub project_path: String,
    pub docroot: String,
    pub url: String,
    pub database: Option<DatabaseCredentials>,
    pub steps: Vec<TemplateStepResult>,
    /// True when a network/composer step failed but the scaffold + site were kept.
    pub soft_failed: bool,
    pub notes: Vec<String>,
}

pub fn skip_network() -> bool {
    std::env::var(SKIP_NETWORK_ENV).map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false)
}

/// Replace/append `KEY=value` lines in a dotenv string.
pub fn patch_env(raw: &str, pairs: &[(&str, String)]) -> String {
    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();
    for (k, v) in pairs {
        let prefix = format!("{k}=");
        let commented = format!("# {k}=");
        let line = format!("{k}={v}");
        if let Some(i) = lines
            .iter()
            .position(|l| l.trim_start().starts_with(&prefix) || l.trim_start().starts_with(&commented))
        {
            lines[i] = line;
        } else {
            lines.push(line);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn is_sql_engine(engine: &str) -> bool {
    matches!(engine, "mysql" | "mariadb")
}

fn write_db_env(project: &Path, kind: TemplateKind, db: &DatabaseCredentials, domain: &str) -> Result<()> {
    match kind {
        TemplateKind::Laravel | TemplateKind::Statamic => {
            let env = project.join(".env");
            let base = fs::read_to_string(&env)
                .or_else(|_| fs::read_to_string(project.join(".env.example")))
                .unwrap_or_default();
            let next = patch_env(
                &base,
                &[
                    ("APP_URL", format!("https://{domain}")),
                    ("DB_CONNECTION", "mysql".into()),
                    ("DB_HOST", "127.0.0.1".into()),
                    ("DB_PORT", db.port.to_string()),
                    ("DB_DATABASE", db.name.clone()),
                    ("DB_USERNAME", db.user.clone()),
                    ("DB_PASSWORD", db.password.clone()),
                ],
            );
            fs::write(env, next)?;
        }
        TemplateKind::Symfony => {
            let env = project.join(".env.local");
            let url = format!(
                "mysql://{}:{}@127.0.0.1:{}/{}?serverVersion=8.0&charset=utf8mb4",
                db.user, db.password, db.port, db.name
            );
            let base = fs::read_to_string(&env).unwrap_or_default();
            fs::write(env, patch_env(&base, &[("DATABASE_URL", format!("\"{url}\""))]))?;
        }
        _ => {}
    }
    Ok(())
}

/// Minimal on-disk scaffold used when the network is skipped/unavailable.
pub fn scaffold_placeholder(kind: TemplateKind, project: &Path) -> Result<()> {
    fs::create_dir_all(project)?;
    let docroot = if kind.docroot().is_empty() {
        project.to_path_buf()
    } else {
        project.join(kind.docroot())
    };
    fs::create_dir_all(&docroot)?;
    let index = docroot.join("index.php");
    if !index.exists() {
        fs::write(
            index,
            format!(
                "<?php\n// DevStack placeholder for the {} template (network step was skipped).\nheader('Content-Type: text/plain');\necho \"{} template placeholder — PHP \" . PHP_VERSION . \"\\n\";\n",
                kind.label(),
                kind.label()
            ),
        )?;
    }
    match kind {
        TemplateKind::Laravel | TemplateKind::Statamic => {
            let _ = fs::write(project.join("artisan"), "#!/usr/bin/env php\n<?php\n");
            let name = if kind == TemplateKind::Statamic { "statamic/statamic" } else { "laravel/laravel" };
            let req = if kind == TemplateKind::Statamic { "\"statamic/cms\": \"*\"" } else { "\"laravel/framework\": \"*\"" };
            let _ = fs::write(
                project.join("composer.json"),
                format!("{{\n  \"name\": \"{name}\",\n  \"require\": {{ {req} }}\n}}\n"),
            );
            let _ = fs::write(project.join(".env.example"), "APP_NAME=DevStack\nAPP_ENV=local\nAPP_KEY=\nDB_CONNECTION=mysql\n");
        }
        TemplateKind::Symfony => {
            let _ = fs::create_dir_all(project.join("bin"));
            let _ = fs::write(project.join("bin/console"), "#!/usr/bin/env php\n<?php\n");
            let _ = fs::write(
                project.join("composer.json"),
                "{\n  \"name\": \"symfony/skeleton\",\n  \"require\": { \"symfony/framework-bundle\": \"*\" }\n}\n",
            );
        }
        TemplateKind::Drupal => {
            let _ = fs::write(
                project.join("composer.json"),
                "{\n  \"name\": \"drupal/recommended-project\",\n  \"require\": { \"drupal/core\": \"*\" }\n}\n",
            );
        }
        TemplateKind::Joomla | TemplateKind::Woocommerce => {}
    }
    let _ = fs::write(project.join(".devstack-template"), format!("{}\n", kind.id()));
    Ok(())
}

struct StepLog(Vec<TemplateStepResult>);

impl StepLog {
    fn ok(&mut self, name: &str, message: impl Into<String>) {
        self.0.push(TemplateStepResult { name: name.into(), ok: true, skipped: false, message: message.into() });
    }
    fn skipped(&mut self, name: &str, message: impl Into<String>) {
        self.0.push(TemplateStepResult { name: name.into(), ok: true, skipped: true, message: message.into() });
    }
    fn failed(&mut self, name: &str, message: impl Into<String>) {
        self.0.push(TemplateStepResult { name: name.into(), ok: false, skipped: false, message: message.into() });
    }
}

fn create_database(
    paths: &Paths,
    state: &mut AppState,
    engine: &str,
    kind: TemplateKind,
    id: &str,
) -> Result<DatabaseCredentials> {
    if !is_sql_engine(engine) {
        return Err(DevStackError::Message(format!("{engine} is not a SQL engine — skipping database")));
    }
    let name = crate::services::wordpress::safe_db_ident(&format!("{}_{id}", kind.framework()));
    let user = crate::services::wordpress::safe_db_ident(&format!("u_{id}"));
    let password = crate::services::wordpress::random_password(24);
    crate::services::database::start_database(paths, state, engine)?;
    crate::services::database::ensure_database(paths, state, engine, &name, &user, &password)?;
    let port = crate::services::ports::engine_db_port(&state.stack, engine);
    Ok(DatabaseCredentials {
        engine: engine.into(),
        name,
        user,
        password,
        host: "127.0.0.1".into(),
        port,
    })
}

fn pick_php_version(state: &AppState, requested: Option<String>) -> Option<String> {
    requested
        .or_else(|| state.stack.php_version.clone())
        .or_else(|| state.installed.get("php").and_then(|v| v.first()).map(|r| r.version.clone()))
}

fn run_composer_blocking(
    php_bin: PathBuf,
    php_root: PathBuf,
    composer: PathBuf,
    tools: PathBuf,
    cwd: PathBuf,
    args: Vec<String>,
) -> Result<String> {
    let mut cmd = crate::services::process::command(&php_bin);
    crate::php::apply_php_ini_to_command(&mut cmd, &php_root);
    cmd.arg(&composer)
        .args(&args)
        .current_dir(&cwd)
        .env("COMPOSER_HOME", tools.join("composer-home"))
        .env("COMPOSER_ALLOW_SUPERUSER", "1")
        .env("COMPOSER_NO_INTERACTION", "1")
        .env("COMPOSER_MEMORY_LIMIT", "-1");
    let out = cmd
        .output()
        .map_err(|e| DevStackError::Message(format!("composer failed to start: {e}")))?;
    if !out.status.success() {
        return Err(DevStackError::Message(format!(
            "composer {} failed: {}{}",
            args.first().map(|s| s.as_str()).unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim(),
            String::from_utf8_lossy(&out.stdout).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

async fn install_joomla(paths: &Paths, project: &Path, progress: &JobProgressFn, job: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        assets: Vec<Asset>,
    }
    let client = crate::downloads::http_client()?;
    let rel: Release = client
        .get("https://api.github.com/repos/joomla/joomla-cms/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let asset = rel
        .assets
        .iter()
        .find(|a| a.name.ends_with("Full_Package.zip"))
        .ok_or_else(|| DevStackError::Message("Joomla full package not found in latest release".into()))?;
    emit(progress, job, "download", &format!("Downloading Joomla {}…", rel.tag_name), 0, None);
    let zip = paths.downloads.join(&asset.name);
    crate::downloads::download_file(
        &client,
        &asset.browser_download_url,
        &zip,
        "joomla",
        &rel.tag_name,
        &crate::downloads::noop_progress(),
    )
    .await?;
    crate::downloads::extract_archive(&zip, project, crate::downloads::ArchiveKind::Zip, 0)?;
    let _ = fs::remove_file(zip);
    Ok(rel.tag_name)
}

pub async fn create_from_template(
    paths: &Paths,
    state: &mut AppState,
    req: CreateFromTemplateRequest,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<CreateFromTemplateResult> {
    create_from_template_with(paths, state, req, progress, job_id, skip_network()).await
}

pub async fn create_from_template_with(
    paths: &Paths,
    state: &mut AppState,
    req: CreateFromTemplateRequest,
    progress: &JobProgressFn,
    job_id: &str,
    skip_net: bool,
) -> Result<CreateFromTemplateResult> {
    let kind = TemplateKind::parse(&req.kind)?;
    let id = slugify(&req.name);
    if state.sites.iter().any(|s| s.id == id) {
        return Err(DevStackError::Message(format!("a site named `{id}` already exists")));
    }
    let tld = state.developer.default_tld.clone();
    let domain = format!("{id}.{tld}");
    let project = req
        .path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(|p| crate::paths_util::expand_home(&PathBuf::from(p)))
        .unwrap_or_else(|| default_sites_root().join(&id));
    fs::create_dir_all(&project)?;
    let mut steps = StepLog(Vec::new());
    let mut notes = Vec::new();
    let mut soft_failed = false;

    // WooCommerce delegates to the WordPress installer (own DB + install).
    if kind == TemplateKind::Woocommerce {
        return create_woocommerce(paths, state, &req, &project, progress, job_id, skip_net).await;
    }

    let engine = req
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .or_else(|| state.runtime.database_engine.clone())
        .unwrap_or_else(|| "mariadb".into());

    emit(progress, job_id, "database", &format!("Creating {} database…", engine), 0, None);
    let database = match create_database(paths, state, &engine, kind, &id) {
        Ok(db) => {
            steps.ok("database", format!("created `{}` on {engine}", db.name));
            Some(db)
        }
        Err(e) => {
            soft_failed = true;
            steps.failed("database", e.to_string());
            notes.push("Database was not created — start a MySQL/MariaDB engine and re-run, or create it from the Databases page.".into());
            None
        }
    };

    let mut scaffolded = false;
    if skip_net {
        steps.skipped("project", format!("{SKIP_NETWORK_ENV}=1 — wrote placeholder scaffold"));
        scaffold_placeholder(kind, &project)?;
        scaffolded = true;
    } else if kind == TemplateKind::Joomla {
        emit(progress, job_id, "project", "Installing Joomla…", 0, None);
        match install_joomla(paths, &project, progress, job_id).await {
            Ok(tag) => steps.ok("project", format!("Joomla {tag} unpacked")),
            Err(e) => {
                soft_failed = true;
                steps.failed("project", e.to_string());
            }
        }
    } else {
        emit(progress, job_id, "project", &format!("composer create-project ({})…", kind.label()), 0, None);
        let run = run_composer_for_kind(paths, state, kind, &project, &req, progress, job_id).await;
        match run {
            Ok(msg) => steps.ok("project", msg),
            Err(e) => {
                soft_failed = true;
                steps.failed("project", e.to_string());
                notes.push("composer create-project failed (offline?). A placeholder scaffold was written so the site still loads.".into());
            }
        }
    }
    if !scaffolded && !project.join(".devstack-template").exists() && !project_has_files(&project) {
        scaffold_placeholder(kind, &project)?;
    }
    let _ = fs::write(project.join(".devstack-template"), format!("{}\n", kind.id()));

    if let Some(db) = &database {
        match write_db_env(&project, kind, db, &domain) {
            Ok(()) => steps.ok("env", "database credentials written to .env"),
            Err(e) => steps.failed("env", e.to_string()),
        }
    }

    let docroot = if kind.docroot().is_empty() {
        project.clone()
    } else {
        project.join(kind.docroot())
    };
    fs::create_dir_all(&docroot)?;

    let php_version = pick_php_version(state, req.php_version.clone());
    let mut site = SiteRecord {
        id: id.clone(),
        name: req.name.clone(),
        path: docroot.clone(),
        domain: domain.clone(),
        php_version,
        node_version: None,
        https: true,
        web_server: Some(
            match req.web_server.as_deref().map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("apache") | Some("httpd") => "apache",
                _ => "nginx",
            }
            .into(),
        ),
        framework: Some(kind.framework().into()),
        database_engine: database.as_ref().map(|d| d.engine.clone()),
        database_name: database.as_ref().map(|d| d.name.clone()),
        database_user: database.as_ref().map(|d| d.user.clone()),
        database_password: database.as_ref().map(|d| d.password.clone()),
        status: "stopped".into(),
        is_template: false,
        parked: false,
        xdebug: false,
        tld,
        aliases: vec![],
        wildcard: false,
    };
    site.ensure_domain();
    state.sites.retain(|s| s.id != site.id);
    state.sites.push(site.clone());
    if let Some(db) = &database {
        state.active_database = Some(db.engine.clone());
    }
    let detected = crate::services::sites::detect_framework(&docroot).0;
    if detected != "other" && detected != "php" {
        if let Some(s) = state.sites.iter_mut().find(|s| s.id == id) {
            s.framework = Some(detected.clone());
        }
        site.framework = Some(detected);
    }

    emit(progress, job_id, "done", &format!("{} site ready", kind.label()), 100, Some(100));
    let url = crate::services::ports::canonical_site_url(&site, state);
    Ok(CreateFromTemplateResult {
        site,
        kind: kind.id().into(),
        project_path: project.display().to_string(),
        docroot: docroot.display().to_string(),
        url,
        database,
        steps: steps.0,
        soft_failed,
        notes,
    })
}

fn project_has_files(project: &Path) -> bool {
    fs::read_dir(project)
        .map(|mut rd| rd.any(|e| e.map(|e| e.file_name() != ".devstack-template").unwrap_or(false)))
        .unwrap_or(false)
}

async fn run_composer_for_kind(
    paths: &Paths,
    state: &mut AppState,
    kind: TemplateKind,
    project: &Path,
    req: &CreateFromTemplateRequest,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<String> {
    let version = pick_php_version(state, req.php_version.clone())
        .ok_or_else(|| DevStackError::Message("install a PHP version first".into()))?;
    let rt = crate::services::php_runtime::resolve_php(paths, state, &version)?;
    let composer = crate::services::tools::ensure_composer(paths, progress, job_id).await?;
    // composer create-project wants an empty directory.
    let _ = fs::remove_file(project.join(".devstack-template"));
    let mut log = Vec::new();
    for args in composer_commands(kind, project) {
        let out = tokio::task::spawn_blocking({
            let (php, root, comp, tools) = (rt.php_bin.clone(), rt.root.clone(), composer.clone(), paths.tools.clone());
            let cwd = project.parent().unwrap_or(project).to_path_buf();
            move || run_composer_blocking(php, root, comp, tools, cwd, args)
        })
        .await
        .map_err(|e| DevStackError::Message(e.to_string()))??;
        log.push(out.lines().last().unwrap_or("").to_string());
    }
    Ok(format!("composer create-project finished ({})", kind.label()))
}

async fn create_woocommerce(
    paths: &Paths,
    state: &mut AppState,
    req: &CreateFromTemplateRequest,
    project: &Path,
    progress: &JobProgressFn,
    job_id: &str,
    skip_net: bool,
) -> Result<CreateFromTemplateResult> {
    let kind = TemplateKind::Woocommerce;
    let id = slugify(&req.name);
    let mut steps = StepLog(Vec::new());
    let mut notes = Vec::new();
    let mut soft_failed = false;
    let tld = state.developer.default_tld.clone();
    let domain = format!("{id}.{tld}");

    if skip_net {
        steps.skipped("wordpress", format!("{SKIP_NETWORK_ENV}=1 — wrote placeholder scaffold"));
        scaffold_placeholder(kind, project)?;
        let engine = req
            .database_engine
            .clone()
            .or_else(|| state.active_database.clone())
            .unwrap_or_else(|| "mariadb".into());
        let database = match create_database(paths, state, &engine, kind, &id) {
            Ok(db) => {
                steps.ok("database", format!("created `{}` on {engine}", db.name));
                Some(db)
            }
            Err(e) => {
                soft_failed = true;
                steps.failed("database", e.to_string());
                None
            }
        };
        let mut site = SiteRecord {
            id: id.clone(),
            name: req.name.clone(),
            path: project.to_path_buf(),
            domain,
            php_version: pick_php_version(state, req.php_version.clone()),
            node_version: None,
            https: true,
            web_server: Some("apache".into()),
            framework: Some("wordpress".into()),
            database_engine: database.as_ref().map(|d| d.engine.clone()),
            database_name: database.as_ref().map(|d| d.name.clone()),
            database_user: database.as_ref().map(|d| d.user.clone()),
            database_password: database.as_ref().map(|d| d.password.clone()),
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld,
            aliases: vec![],
            wildcard: false,
        };
        site.ensure_domain();
        state.sites.retain(|s| s.id != site.id);
        state.sites.push(site.clone());
        let url = crate::services::ports::canonical_site_url(&site, state);
        return Ok(CreateFromTemplateResult {
            site,
            kind: kind.id().into(),
            project_path: project.display().to_string(),
            docroot: project.display().to_string(),
            url,
            database,
            steps: steps.0,
            soft_failed,
            notes,
        });
    }

    let wp = crate::services::wordpress::CreateWordPressRequest {
        name: req.name.clone(),
        domain: None,
        path: Some(project.display().to_string()),
        php_version: req.php_version.clone(),
        database_engine: req.database_engine.clone(),
        locale: None,
        admin_user: None,
        site_title: Some(req.name.clone()),
    };
    let result = crate::services::wordpress::create_wordpress_site(paths, state, wp, progress, job_id).await?;
    steps.ok("wordpress", format!("WordPress {} installed", result.wordpress_version));

    let site = result.site.clone();
    let version = site.php_version.clone().unwrap_or_else(|| "8.3".into());
    match crate::services::php_runtime::resolve_php(paths, state, &version) {
        Ok(rt) => {
            let wp_cli = crate::services::wordpress::ensure_wp_cli(paths, progress, job_id).await?;
            for args in wp_cli_commands(kind) {
                let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                match crate::services::wordpress::run_wp_cli_with_ini(
                    &rt.php_bin,
                    &wp_cli,
                    project,
                    &refs,
                    Some(&rt.root),
                ) {
                    Ok(_) => steps.ok("woocommerce", "WooCommerce plugin installed + activated"),
                    Err(e) => {
                        soft_failed = true;
                        steps.failed("woocommerce", e.to_string());
                        notes.push("WooCommerce plugin install failed (offline?) — WordPress itself is ready.".into());
                    }
                }
            }
        }
        Err(e) => {
            soft_failed = true;
            steps.failed("woocommerce", e.to_string());
        }
    }
    notes.push(format!("WP admin: {} / {}", result.admin_user, result.admin_password));
    let database = site.database_name.as_ref().map(|n| DatabaseCredentials {
        engine: site.database_engine.clone().unwrap_or_default(),
        name: n.clone(),
        user: site.database_user.clone().unwrap_or_default(),
        password: site.database_password.clone().unwrap_or_default(),
        host: "127.0.0.1".into(),
        port: crate::services::ports::engine_db_port(
            &state.stack,
            site.database_engine.as_deref().unwrap_or("mariadb"),
        ),
    });
    Ok(CreateFromTemplateResult {
        site,
        kind: kind.id().into(),
        project_path: project.display().to_string(),
        docroot: project.display().to_string(),
        url: result.url,
        database,
        steps: steps.0,
        soft_failed,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::progress::noop_job_progress;

    #[test]
    fn kind_parse_and_labels() {
        for k in ALL_KINDS {
            assert_eq!(TemplateKind::parse(k.id()).unwrap(), *k);
        }
        assert_eq!(TemplateKind::parse("Woo").unwrap(), TemplateKind::Woocommerce);
        assert!(TemplateKind::parse("rails").is_err());
    }

    #[test]
    fn kind_to_commands_mapping() {
        let t = Path::new("/sites/app");
        let l = composer_commands(TemplateKind::Laravel, t);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0][0], "create-project");
        assert_eq!(l[0][1], "laravel/laravel");
        assert_eq!(l[0][2], "/sites/app");
        assert_eq!(composer_commands(TemplateKind::Symfony, t)[0][1], "symfony/skeleton");
        assert_eq!(composer_commands(TemplateKind::Statamic, t)[0][1], "statamic/statamic");
        assert_eq!(composer_commands(TemplateKind::Drupal, t)[0][1], "drupal/recommended-project");
        assert!(composer_commands(TemplateKind::Joomla, t).is_empty());
        assert!(composer_commands(TemplateKind::Woocommerce, t).is_empty());
        assert_eq!(
            wp_cli_commands(TemplateKind::Woocommerce)[0],
            vec!["plugin", "install", "woocommerce", "--activate"]
        );
        assert!(command_summary(TemplateKind::Joomla)[0].contains("Joomla"));
        assert_eq!(TemplateKind::Laravel.docroot(), "public");
        assert_eq!(TemplateKind::Drupal.docroot(), "web");
        assert_eq!(TemplateKind::Woocommerce.framework(), "wordpress");
        assert_eq!(list_template_kinds().len(), 6);
    }

    #[test]
    fn patch_env_replaces_and_appends() {
        let raw = "APP_NAME=x\n# DB_HOST=old\nDB_PORT=3306\n";
        let out = patch_env(
            raw,
            &[("DB_HOST", "127.0.0.1".into()), ("DB_PORT", "3308".into()), ("DB_NAME", "app".into())],
        );
        assert!(out.contains("DB_HOST=127.0.0.1"));
        assert!(!out.contains("# DB_HOST"));
        assert!(out.contains("DB_PORT=3308"));
        assert!(out.contains("DB_NAME=app"));
        assert!(out.contains("APP_NAME=x"));
    }

    #[test]
    fn scaffold_detects_as_framework() {
        let dir = std::env::temp_dir().join(format!("devstack-tpl-scaffold-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (kind, want) in [
            (TemplateKind::Laravel, "laravel"),
            (TemplateKind::Statamic, "statamic"),
            (TemplateKind::Symfony, "symfony"),
            (TemplateKind::Drupal, "drupal"),
        ] {
            let p = dir.join(kind.id());
            scaffold_placeholder(kind, &p).unwrap();
            let docroot = if kind.docroot().is_empty() { p.clone() } else { p.join(kind.docroot()) };
            assert!(docroot.join("index.php").exists(), "{kind:?}");
            assert_eq!(crate::services::sites::detect_framework(&docroot).0, want);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn create_from_template_offline_scaffolds_site_and_soft_fails_db() {
        let dir = std::env::temp_dir().join(format!("devstack-tpl-create-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.join("data")).unwrap();
        let mut state = AppState::default();
        let progress = noop_job_progress();
        let req = CreateFromTemplateRequest {
            kind: "laravel".into(),
            name: "Shop App".into(),
            path: Some(dir.join("sites/shop-app").display().to_string()),
            php_version: None,
            database_engine: None,
            web_server: None,
        };
        let res = create_from_template_with(&paths, &mut state, req, &progress, "t", true)
            .await
            .unwrap();
        assert_eq!(res.site.id, "shop-app");
        assert_eq!(res.site.domain, "shop-app.test");
        assert!(res.docroot.ends_with("public"));
        assert!(Path::new(&res.docroot).join("index.php").exists());
        assert!(Path::new(&res.project_path).join(".devstack-template").exists());
        assert!(res.steps.iter().any(|s| s.name == "project" && s.skipped));
        // No DB engine installed in unit tests → database step soft-fails, site is kept.
        assert!(res.steps.iter().any(|s| s.name == "database" && !s.ok));
        assert!(res.soft_failed);
        assert_eq!(state.sites.len(), 1);
        assert_eq!(state.sites[0].framework.as_deref(), Some("laravel"));
        // Duplicate names are rejected.
        let dup = CreateFromTemplateRequest {
            kind: "symfony".into(),
            name: "shop app".into(),
            path: None,
            php_version: None,
            database_engine: None,
            web_server: None,
        };
        assert!(create_from_template_with(&paths, &mut state, dup, &progress, "t", true).await.is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
