//! Starter templates: snapshot site files + streaming DB dump; duplicate with search-replace.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{slugify, AppState, Paths, SiteRecord, TemplateRecord};
use crate::error::{DevStackError, Result};
use crate::services::backup::{
    archive_directory, backup_database, extract_directory_archive, restore_database, CompressKind,
};
use crate::services::database::{ensure_database, socket_path};
use crate::services::nginx::expand_home;
use crate::services::php_runtime::resolve_php;
use crate::services::progress::{emit, JobProgressFn};
use crate::services::sites::{detect_site, parse_wp_config};
use crate::services::stack::reload_sites;
use crate::services::wordpress::{run_wp_cli, update_wp_config_credentials, ensure_wp_cli};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateRequest {
    /// Template id OR source site id
    pub source_id: String,
    pub name: String,
    pub domain: Option<String>,
    pub path: String,
    pub php_version: Option<String>,
    pub database_engine: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateResult {
    pub site: SiteRecord,
    pub url: String,
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

fn random_password(len: usize) -> String {
    use rand::{distributions::Alphanumeric, Rng};
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

pub async fn mark_site_as_template(
    paths: &Paths,
    state: &mut AppState,
    site_id: &str,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<TemplateRecord> {
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .cloned()
        .ok_or_else(|| DevStackError::Message(format!("site not found: {site_id}")))?;

    let abs = expand_home(&site.path);
    let tpl_id = format!(
        "{}-{}",
        site.id,
        chrono::Local::now().format("%Y%m%d%H%M%S")
    );
    let tpl_dir = paths.templates.join(&tpl_id);
    fs::create_dir_all(&tpl_dir)?;

    let kind = CompressKind::preferred();
    let files_name = if kind == CompressKind::Zstd {
        "files.tar.zst"
    } else {
        "files.tar.gz"
    };
    let files_archive = tpl_dir.join(files_name);

    emit(
        progress,
        job_id,
        "snapshot",
        "Archiving site files…",
        0,
        None,
    );
    let mut size = archive_directory(&abs, &files_archive, progress, job_id)?;

    let engine = site
        .database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .or_else(|| state.runtime.database_engine.clone())
        .unwrap_or_else(|| "mariadb".into());

    let (db_name, _, _) = parse_wp_config(&site.path);
    let db_name = db_name.or_else(|| site.database_name.clone());

    let db_archive = if let Some(db) = db_name {
        let db_path = tpl_dir.join(format!("database.{}", kind.extension()));
        emit(
            progress,
            job_id,
            "dump",
            &format!("Dumping database `{db}`…"),
            0,
            None,
        );
        size += backup_database(paths, state, &engine, &db, &db_path, progress, job_id)?;
        Some(db_path)
    } else {
        None
    };

    let source_url = if site.https {
        format!("https://{}", site.domain)
    } else {
        format!("http://{}", site.domain)
    };

    let record = TemplateRecord {
        id: tpl_id,
        name: site.name.clone(),
        source_site_id: site.id.clone(),
        source_domain: site.domain.clone(),
        source_url,
        framework: site
            .framework
            .clone()
            .unwrap_or_else(|| "wordpress".into()),
        created_at: chrono::Utc::now().to_rfc3339(),
        files_archive,
        db_archive,
        size_bytes: size,
        database_engine: Some(engine),
    };

    if let Some(s) = state.sites.iter_mut().find(|s| s.id == site.id) {
        s.is_template = true;
    }
    state.templates.retain(|t| t.id != record.id);
    state.templates.push(record.clone());

    emit(
        progress,
        job_id,
        "done",
        &format!("Template `{}` saved", record.name),
        size,
        Some(size),
    );
    Ok(record)
}

pub async fn duplicate_from_source(
    paths: &Paths,
    state: &mut AppState,
    req: DuplicateRequest,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<DuplicateResult> {
    let id = slugify(&req.name);
    if state.sites.iter().any(|s| s.id == id) {
        return Err(DevStackError::Message(format!(
            "a site named `{id}` already exists"
        )));
    }

    let domain = req
        .domain
        .filter(|d| !d.trim().is_empty())
        .map(|d| {
            if d.ends_with(".test") {
                d
            } else {
                format!("{d}.test")
            }
        })
        .unwrap_or_else(|| format!("{id}.test"));

    let dest = expand_home(&PathBuf::from(&req.path));
    fs::create_dir_all(&dest)?;

    // Resolve template or live site
    let template = state
        .templates
        .iter()
        .find(|t| t.id == req.source_id)
        .cloned();

    let (files_archive, db_archive, old_url, old_domain, framework, engine_hint) =
        if let Some(tpl) = template {
            (
                tpl.files_archive.clone(),
                tpl.db_archive.clone(),
                tpl.source_url.clone(),
                tpl.source_domain.clone(),
                tpl.framework.clone(),
                tpl.database_engine.clone(),
            )
        } else {
            // Live duplicate: snapshot on the fly into a temp template dir
            emit(
                progress,
                job_id,
                "snapshot",
                "Creating temporary snapshot of source site…",
                0,
                None,
            );
            let tmp = mark_site_as_template(paths, state, &req.source_id, progress, job_id).await?;
            (
                tmp.files_archive,
                tmp.db_archive,
                tmp.source_url,
                tmp.source_domain,
                tmp.framework,
                tmp.database_engine,
            )
        };

    emit(
        progress,
        job_id,
        "copy",
        "Copying files into target directory…",
        0,
        None,
    );
    extract_directory_archive(&files_archive, &dest, progress, job_id)?;

    let engine = req
        .database_engine
        .or(engine_hint)
        .or_else(|| state.active_database.clone())
        .or_else(|| state.runtime.database_engine.clone())
        .unwrap_or_else(|| "mariadb".into());

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

    let db_name = safe_db_ident(&format!("wp_{id}"));
    let db_user = safe_db_ident(&format!("u_{id}"));
    let db_pass = random_password(24);
    let sock = socket_path(paths, &engine);

    emit(
        progress,
        job_id,
        "database",
        &format!("Creating database `{db_name}`…"),
        0,
        None,
    );
    ensure_database(paths, state, &engine, &db_name, &db_user, &db_pass)?;

    if let Some(db_arch) = db_archive {
        if db_arch.exists() {
            emit(progress, job_id, "import", "Streaming database import…", 0, None);
            restore_database(
                paths, state, &engine, &db_name, &db_arch, progress, job_id,
            )?;
        }
    }

    update_wp_config_credentials(&dest, &db_name, &db_user, &db_pass, &sock)?;

    let https_port = state.stack.https_port;
    let new_url = if https_port == 443 {
        format!("https://{domain}")
    } else {
        format!("https://{domain}:{https_port}")
    };
    if framework == "wordpress" || dest.join("wp-config.php").exists() {
        emit(
            progress,
            job_id,
            "search-replace",
            "Running wp search-replace (serialized-safe)…",
            0,
            None,
        );
        let php_rt = resolve_php(paths, state, &php_version)?;
        let wp_cli = ensure_wp_cli(paths, progress, job_id).await?;

        // Replace both https and http variants + domain-only
        let replacements = [
            (old_url.clone(), new_url.clone()),
            (
                old_url.replace("https://", "http://"),
                new_url.replace("https://", "http://"),
            ),
            (format!("https://{old_domain}"), new_url.clone()),
            (format!("http://{old_domain}"), format!("http://{domain}")),
            (old_domain.clone(), domain.clone()),
        ];
        for (from, to) in replacements {
            if from == to || from.is_empty() {
                continue;
            }
            let _ = run_wp_cli(
                &php_rt.php_bin,
                &wp_cli,
                &dest,
                &[
                    "search-replace",
                    &from,
                    &to,
                    "--all-tables",
                    "--precise",
                    "--recurse-objects",
                    "--skip-columns=guid",
                    "--report-changed-only",
                ],
            );
        }
    }

    let mut site = SiteRecord {
        id: id.clone(),
        name: req.name.clone(),
        path: PathBuf::from(&req.path),
        domain: domain.clone(),
        php_version: Some(php_version),
        node_version: None,
        https: true,
        web_server: Some("apache".into()),
        framework: Some(framework),
        database_engine: Some(engine),
        database_name: Some(db_name),
        database_user: Some(db_user),
        database_password: Some(db_pass),
        status: "running".into(),
        is_template: false,
    };
    let details = detect_site(&site);
    site.framework = Some(details.framework);
    state.sites.push(site.clone());
    let _ = reload_sites(paths, state);

    emit(
        progress,
        job_id,
        "done",
        "Duplicate complete",
        100,
        Some(100),
    );

    Ok(DuplicateResult {
        site,
        url: new_url,
    })
}

pub fn list_templates(state: &AppState) -> Vec<TemplateRecord> {
    state.templates.clone()
}
