//! Config export / import: a single `.zip` with sanitized `state.json`, `settings.json`,
//! per-site metadata (php.ini overrides, …), hosting profiles (no secrets) and optional
//! database dumps. Import restores onto another machine/profile; machine-specific data
//! (installed package paths, running PIDs) is deliberately not carried over.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{
    AppState, BackupSettings, DatabaseFilesSettings, DeveloperSettings, Paths, PhpExtensionSettings,
    SiteRecord, SitesSettings, StackPreferences, TemplateRecord,
};
use crate::error::{DevStackError, Result};
use crate::services::progress::{emit, JobProgressFn};

pub const FORMAT: &str = "devstack-config";
pub const FORMAT_VERSION: u32 = 1;
const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveMeta {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    pub created_at: String,
    pub os: String,
    pub site_count: usize,
    pub site_ids: Vec<String>,
    pub includes_databases: bool,
    /// `{engine}/{database}` for each bundled dump.
    pub databases: Vec<String>,
    pub site_meta_files: usize,
    pub has_hosting_profiles: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsDoc {
    stack: StackPreferences,
    developer: DeveloperSettings,
    sites_settings: SitesSettings,
    backup_settings: BackupSettings,
    db_files: DatabaseFilesSettings,
    active_database: Option<String>,
    php_extensions: std::collections::HashMap<String, PhpExtensionSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateDoc {
    sites: Vec<SiteRecord>,
    templates: Vec<TemplateRecord>,
    welcome_site_seeded: bool,
    /// Informational only (id → versions); installs are re-done on the target machine.
    installed_packages: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub path: String,
    pub bytes: u64,
    pub meta: ArchiveMeta,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportOptions {
    /// Replace the site list instead of merging by id.
    pub replace_sites: bool,
    pub restore_settings: bool,
    pub restore_databases: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub sites_added: usize,
    pub sites_updated: usize,
    pub settings_applied: bool,
    pub site_meta_files: usize,
    pub databases_restored: Vec<String>,
    pub databases_skipped: Vec<String>,
    /// Sites whose folder does not exist on this machine.
    pub missing_paths: Vec<String>,
    pub installed_packages: BTreeMap<String, Vec<String>>,
    pub notes: Vec<String>,
}

fn zip_err(e: zip::result::ZipError) -> DevStackError {
    DevStackError::Archive(e.to_string())
}

pub fn sanitized_state_doc(state: &AppState) -> StateDocPublic {
    let installed_packages = state
        .installed
        .iter()
        .map(|(k, v)| (k.clone(), v.iter().map(|r| r.version.clone()).collect()))
        .collect();
    StateDocPublic(StateDoc {
        sites: state.sites.clone(),
        templates: state.templates.clone(),
        welcome_site_seeded: state.welcome_site_seeded,
        installed_packages,
    })
}

/// Opaque wrapper so `StateDoc` stays private while tests can inspect JSON.
pub struct StateDocPublic(StateDoc);

impl StateDocPublic {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.0)?)
    }
}

fn options() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
}

fn add_bytes(zip: &mut zip::ZipWriter<File>, name: &str, data: &[u8]) -> Result<()> {
    zip.start_file(name, options()).map_err(zip_err)?;
    zip.write_all(data)?;
    Ok(())
}

fn add_file(zip: &mut zip::ZipWriter<File>, name: &str, path: &Path) -> Result<u64> {
    zip.start_file(name, options().large_file(true)).map_err(zip_err)?;
    let mut f = File::open(path)?;
    Ok(std::io::copy(&mut f, zip)?)
}

fn collect_site_meta(paths: &Paths, site_ids: &[String]) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for id in site_ids {
        let base = paths.sites_meta.join(id);
        let Ok(rd) = fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            // Metadata only: small regular files, never merged/generated artefacts.
            if !p.is_file() || e.file_name() == "php-merged.ini" {
                continue;
            }
            if e.metadata().map(|m| m.len() > 5 * 1024 * 1024).unwrap_or(true) {
                continue;
            }
            out.push((format!("sites/{id}/{}", e.file_name().to_string_lossy()), p));
        }
    }
    out
}

pub fn export_config(
    paths: &Paths,
    state: &AppState,
    dest: &Path,
    include_databases: bool,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<ExportSummary> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let mut warnings = Vec::new();

    emit(progress, job_id, "state", "Writing settings…", 0, None);
    add_bytes(&mut zip, "state.json", sanitized_state_doc(state).to_json()?.as_bytes())?;
    let settings = SettingsDoc {
        stack: state.stack.clone(),
        developer: state.developer.clone(),
        sites_settings: state.sites_settings.clone(),
        backup_settings: state.backup_settings.clone(),
        db_files: state.db_files.clone(),
        active_database: state.active_database.clone(),
        php_extensions: state.php_extensions.clone(),
    };
    add_bytes(&mut zip, "settings.json", serde_json::to_string_pretty(&settings)?.as_bytes())?;

    let site_ids: Vec<String> = state.sites.iter().map(|s| s.id.clone()).collect();
    let meta_files = collect_site_meta(paths, &site_ids);
    for (name, path) in &meta_files {
        add_file(&mut zip, name, path)?;
    }
    let hosting = crate::services::hosting_sync::profiles_path(paths);
    let has_hosting = hosting.exists();
    if has_hosting {
        add_file(&mut zip, "hosting-profiles.json", &hosting)?;
    }

    let mut databases = Vec::new();
    if include_databases {
        for site in &state.sites {
            let (Some(db), Some(engine)) = (
                site.database_name.clone(),
                site.database_engine.clone().or_else(|| state.active_database.clone()),
            ) else {
                continue;
            };
            if !matches!(engine.as_str(), "mysql" | "mariadb") {
                continue;
            }
            emit(progress, job_id, "database", &format!("Dumping {db}…"), 0, None);
            let tmp = paths.downloads.join(format!("config-export-{db}.sql.gz"));
            match crate::services::backup::backup_database(paths, state, &engine, &db, &tmp, progress, job_id) {
                Ok(_) => {
                    let name = format!("databases/{engine}/{db}.sql.gz");
                    add_file(&mut zip, &name, &tmp)?;
                    databases.push(format!("{engine}/{db}"));
                    let _ = fs::remove_file(&tmp);
                }
                Err(e) => warnings.push(format!("skipped database {db}: {e}")),
            }
        }
    }

    let meta = ArchiveMeta {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        os: std::env::consts::OS.into(),
        site_count: state.sites.len(),
        site_ids,
        includes_databases: !databases.is_empty(),
        databases,
        site_meta_files: meta_files.len(),
        has_hosting_profiles: has_hosting,
    };
    add_bytes(&mut zip, "meta.json", serde_json::to_string_pretty(&meta)?.as_bytes())?;
    zip.finish().map_err(zip_err)?;
    let bytes = fs::metadata(dest)?.len();
    emit(progress, job_id, "done", "Export complete", bytes, Some(bytes));
    Ok(ExportSummary { path: dest.display().to_string(), bytes, meta, warnings })
}

fn open_archive(path: &Path) -> Result<zip::ZipArchive<File>> {
    zip::ZipArchive::new(File::open(path)?).map_err(zip_err)
}

fn read_entry(ar: &mut zip::ZipArchive<File>, name: &str) -> Result<Option<Vec<u8>>> {
    match ar.by_name(name) {
        Ok(mut f) => {
            if f.size() > MAX_ENTRY_BYTES {
                return Err(DevStackError::Archive(format!("{name} is too large")));
            }
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)?;
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(zip_err(e)),
    }
}

pub fn inspect_archive(path: &Path) -> Result<ArchiveMeta> {
    let mut ar = open_archive(path)?;
    let raw = read_entry(&mut ar, "meta.json")?
        .ok_or_else(|| DevStackError::Archive("not a DevStack config archive (meta.json missing)".into()))?;
    let meta: ArchiveMeta = serde_json::from_slice(&raw)?;
    if meta.format != FORMAT {
        return Err(DevStackError::Archive(format!("unexpected archive format: {}", meta.format)));
    }
    if meta.format_version > FORMAT_VERSION {
        return Err(DevStackError::Archive(format!(
            "archive format v{} is newer than this DevStack understands (v{FORMAT_VERSION})",
            meta.format_version
        )));
    }
    Ok(meta)
}

/// Only `sites/<id>/<file>` with plain path components is ever written to disk.
fn safe_site_meta_target(paths: &Paths, name: &str) -> Option<PathBuf> {
    let rest = name.strip_prefix("sites/")?;
    let rel = Path::new(rest);
    let comps: Vec<_> = rel.components().collect();
    if comps.len() != 2 || !comps.iter().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(paths.sites_meta.join(rel))
}

pub fn import_config(
    paths: &Paths,
    state: &mut AppState,
    archive: &Path,
    opts: &ImportOptions,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<ImportReport> {
    let meta = inspect_archive(archive)?;
    let mut ar = open_archive(archive)?;
    let mut report = ImportReport::default();

    emit(progress, job_id, "state", "Restoring sites…", 0, None);
    if let Some(raw) = read_entry(&mut ar, "state.json")? {
        let doc: StateDoc = serde_json::from_slice(&raw)?;
        report.installed_packages = doc.installed_packages;
        if opts.replace_sites {
            state.sites.clear();
            state.templates.clear();
        }
        for mut site in doc.sites {
            site.ensure_domain();
            site.status = "stopped".into();
            if !crate::paths_util::expand_home(&site.path).exists() {
                report.missing_paths.push(format!("{} → {}", site.id, site.path.display()));
            }
            match state.sites.iter_mut().find(|s| s.id == site.id) {
                Some(existing) => {
                    *existing = site;
                    report.sites_updated += 1;
                }
                None => {
                    state.sites.push(site);
                    report.sites_added += 1;
                }
            }
        }
        for t in doc.templates {
            if !state.templates.iter().any(|x| x.id == t.id) {
                state.templates.push(t);
            }
        }
        state.welcome_site_seeded |= doc.welcome_site_seeded;
    } else {
        return Err(DevStackError::Archive("state.json missing from archive".into()));
    }

    if opts.restore_settings {
        if let Some(raw) = read_entry(&mut ar, "settings.json")? {
            let s: SettingsDoc = serde_json::from_slice(&raw)?;
            state.stack = s.stack;
            state.developer = s.developer;
            state.sites_settings = s.sites_settings;
            state.backup_settings = s.backup_settings;
            state.db_files = s.db_files;
            state.active_database = s.active_database;
            state.php_extensions = s.php_extensions;
            report.settings_applied = true;
        }
    }

    let names: Vec<String> = (0..ar.len()).filter_map(|i| ar.by_index(i).ok().map(|f| f.name().to_string())).collect();
    for name in &names {
        if let Some(target) = safe_site_meta_target(paths, name) {
            if let Some(data) = read_entry(&mut ar, name)? {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(target, data)?;
                report.site_meta_files += 1;
            }
        }
    }
    if let Some(raw) = read_entry(&mut ar, "hosting-profiles.json")? {
        let target = crate::services::hosting_sync::profiles_path(paths);
        if !target.exists() || opts.replace_sites {
            fs::write(target, raw)?;
            report.notes.push("Hosting profiles restored — re-enter SSH passwords (they are never exported).".into());
        }
    }

    if opts.restore_databases {
        for entry in &meta.databases {
            let Some((engine, db)) = entry.split_once('/') else { continue };
            let site = state.sites.iter().find(|s| s.database_name.as_deref() == Some(db)).cloned();
            let running = state.runtime.database_pid.map(crate::services::process::is_running).unwrap_or(false)
                && state.runtime.database_engine.as_deref() == Some(engine);
            let Some(site) = site.filter(|_| running) else {
                report.databases_skipped.push(format!("{entry} (start {engine} and re-import)"));
                continue;
            };
            let name = format!("databases/{engine}/{db}.sql.gz");
            let tmp = paths.downloads.join(format!("config-import-{db}.sql.gz"));
            let Some(data) = read_entry(&mut ar, &name)? else { continue };
            fs::write(&tmp, data)?;
            emit(progress, job_id, "database", &format!("Importing {db}…"), 0, None);
            let user = site.database_user.clone().unwrap_or_else(|| "root".into());
            let pass = site.database_password.clone().unwrap_or_default();
            let res = crate::services::database::ensure_database(paths, state, engine, db, &user, &pass)
                .and_then(|_| crate::services::backup::restore_database(paths, state, engine, db, &tmp, progress, job_id));
            let _ = fs::remove_file(&tmp);
            match res {
                Ok(()) => report.databases_restored.push(entry.clone()),
                Err(e) => report.databases_skipped.push(format!("{entry}: {e}")),
            }
        }
    } else if meta.includes_databases {
        report.databases_skipped.extend(meta.databases.iter().map(|d| format!("{d} (not requested)")));
    }

    crate::config::save_state(&paths.state_file, state)?;
    emit(progress, job_id, "done", "Import complete", 100, Some(100));
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::progress::noop_job_progress;

    fn site(id: &str, path: &Path) -> SiteRecord {
        let mut s = SiteRecord {
            id: id.into(),
            name: id.into(),
            path: path.to_path_buf(),
            domain: String::new(),
            php_version: Some("8.3".into()),
            node_version: None,
            https: true,
            web_server: Some("nginx".into()),
            framework: Some("laravel".into()),
            database_engine: Some("mariadb".into()),
            database_name: Some(format!("db_{id}")),
            database_user: Some("u".into()),
            database_password: Some("p".into()),
            status: "running".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: "test".into(),
            aliases: vec![format!("{id}-alias.test")],
            wildcard: true,
        };
        s.ensure_domain();
        s
    }

    #[test]
    fn archive_roundtrip_restores_sites_settings_and_meta_files() {
        let root = std::env::temp_dir().join(format!("devstack-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = Paths::from_root(root.join("src")).unwrap();
        let dst = Paths::from_root(root.join("dst")).unwrap();
        let site_dir = root.join("sites/blog");
        fs::create_dir_all(&site_dir).unwrap();

        let mut state = AppState::default();
        state.sites.push(site("blog", &site_dir));
        state.sites.push(site("gone", &root.join("missing-folder")));
        state.stack.memcached_port = 11300;
        state.stack.http_port = 8080;
        state.developer.default_tld = "dev".into();
        state.runtime.nginx_pid = Some(4242);
        state.installed.insert(
            "php".into(),
            vec![crate::config::InstalledRecord {
                version: "8.3.1".into(),
                channel: None,
                path: "/abs/php".into(),
                installed_at: "x".into(),
            }],
        );
        fs::create_dir_all(src.sites_meta.join("blog")).unwrap();
        fs::write(src.sites_meta.join("blog/php.ini"), "[PHP]\nmemory_limit = 512M\n").unwrap();
        fs::write(src.sites_meta.join("blog/php-merged.ini"), "generated").unwrap();

        let zip_path = root.join("export/devstack-config.zip");
        let progress = noop_job_progress();
        let summary = export_config(&src, &state, &zip_path, false, &progress, "t").unwrap();
        assert!(summary.bytes > 0);
        assert_eq!(summary.meta.site_count, 2);
        assert_eq!(summary.meta.site_meta_files, 1, "merged ini is excluded");
        assert!(!summary.meta.includes_databases);

        let meta = inspect_archive(&zip_path).unwrap();
        assert_eq!(meta, summary.meta);
        assert_eq!(meta.format, FORMAT);
        assert_eq!(meta.site_ids, vec!["blog", "gone"]);

        // Sanitized state never leaks machine-local runtime data.
        let json = sanitized_state_doc(&state).to_json().unwrap();
        assert!(!json.contains("4242") && !json.contains("/abs/php"));

        let mut fresh = AppState::default();
        let report = import_config(
            &dst,
            &mut fresh,
            &zip_path,
            &ImportOptions { restore_settings: true, ..Default::default() },
            &progress,
            "t",
        )
        .unwrap();
        assert_eq!(report.sites_added, 2);
        assert!(report.settings_applied);
        assert_eq!(report.site_meta_files, 1);
        assert_eq!(report.missing_paths.len(), 1);
        assert!(report.missing_paths[0].starts_with("gone"));
        assert_eq!(report.installed_packages["php"], vec!["8.3.1"]);
        assert_eq!(fresh.stack.memcached_port, 11300);
        assert_eq!(fresh.stack.http_port, 8080);
        assert_eq!(fresh.developer.default_tld, "dev");
        assert!(fresh.runtime.nginx_pid.is_none());
        assert!(fresh.installed.is_empty());
        let blog = fresh.sites.iter().find(|s| s.id == "blog").unwrap();
        assert_eq!(blog.aliases, vec!["blog-alias.test"]);
        assert!(blog.wildcard);
        assert_eq!(blog.status, "stopped");
        assert_eq!(
            fs::read_to_string(dst.sites_meta.join("blog/php.ini")).unwrap(),
            "[PHP]\nmemory_limit = 512M\n"
        );
        assert!(!dst.sites_meta.join("blog/php-merged.ini").exists());
        assert!(dst.state_file.exists());

        // Re-import without settings merges instead of duplicating.
        let again = import_config(&dst, &mut fresh, &zip_path, &ImportOptions::default(), &progress, "t").unwrap();
        assert_eq!((again.sites_added, again.sites_updated), (0, 2));
        assert!(!again.settings_applied);
        assert_eq!(fresh.sites.len(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_foreign_or_newer_archives_and_path_traversal() {
        let root = std::env::temp_dir().join(format!("devstack-cfg-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let bogus = root.join("bogus.zip");
        {
            let mut z = zip::ZipWriter::new(File::create(&bogus).unwrap());
            z.start_file("hello.txt", options()).unwrap();
            z.write_all(b"hi").unwrap();
            z.finish().unwrap();
        }
        assert!(inspect_archive(&bogus).is_err());
        let newer = root.join("newer.zip");
        {
            let meta = ArchiveMeta {
                format: FORMAT.into(),
                format_version: FORMAT_VERSION + 1,
                app_version: "9".into(),
                created_at: "now".into(),
                os: "linux".into(),
                site_count: 0,
                site_ids: vec![],
                includes_databases: false,
                databases: vec![],
                site_meta_files: 0,
                has_hosting_profiles: false,
            };
            let mut z = zip::ZipWriter::new(File::create(&newer).unwrap());
            z.start_file("meta.json", options()).unwrap();
            z.write_all(serde_json::to_string(&meta).unwrap().as_bytes()).unwrap();
            z.finish().unwrap();
        }
        assert!(inspect_archive(&newer).unwrap_err().to_string().contains("newer"));
        let paths = Paths::from_root(root.join("p")).unwrap();
        assert!(safe_site_meta_target(&paths, "sites/blog/php.ini").is_some());
        assert!(safe_site_meta_target(&paths, "sites/../../etc/passwd").is_none());
        assert!(safe_site_meta_target(&paths, "sites/a/b/c").is_none());
        assert!(safe_site_meta_target(&paths, "other/x").is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
