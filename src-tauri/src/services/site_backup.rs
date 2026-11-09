//! Full-site backups: files archive + database dump, stored under the backups folder.
//!
//! Layout: `{folder}/sites/{siteId}/{siteId}-{YYYYmmdd-HHMMSS}/{files.tar.gz, db.sql.gz, meta.json}`.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::config::{AppState, BackupSettings, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::paths_util::expand_home;
use crate::services::backup::{archive_directory, backup_database, extract_directory_archive, restore_database};
use crate::services::progress::{emit, JobProgressFn};

const META_FILE: &str = "meta.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SiteBackupRecord {
    pub id: String,
    pub site_id: String,
    pub site_name: String,
    pub domain: String,
    /// RFC 3339 timestamp.
    pub created_at: String,
    pub dir: PathBuf,
    pub files_archive: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_archive: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db_engine: Option<String>,
    pub size_bytes: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

pub fn backup_id(site_id: &str, stamp: &str) -> String {
    format!("{site_id}-{stamp}")
}

pub fn backups_root(paths: &Paths, settings: &BackupSettings) -> PathBuf {
    settings
        .folder
        .as_ref()
        .map(|p| expand_home(p))
        .unwrap_or_else(|| paths.backups.clone())
}

fn site_backups_dir(root: &Path, site_id: &str) -> PathBuf {
    root.join("sites").join(site_id)
}

fn engine_for(site: &SiteRecord, state: &AppState) -> String {
    site.database_engine
        .clone()
        .or_else(|| state.active_database.clone())
        .unwrap_or_else(|| "mariadb".into())
}

fn dir_size(path: &Path) -> u64 {
    fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

pub fn backup_site(
    paths: &Paths,
    state: &AppState,
    site_id: &str,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<SiteBackupRecord> {
    let site = state
        .sites
        .iter()
        .find(|s| s.id == site_id)
        .ok_or_else(|| DevStackError::Message(format!("site not found: {site_id}")))?;
    let root = backups_root(paths, &state.backup_settings);
    let now = Local::now();
    let id = backup_id(&site.id, &now.format("%Y%m%d-%H%M%S").to_string());
    let dir = site_backups_dir(&root, &site.id).join(&id);
    fs::create_dir_all(&dir)?;

    emit(progress, job_id, "files", "Archiving site files…", 0, None);
    let files_archive = dir.join("files.tar.gz");
    archive_directory(&expand_home(&site.path), &files_archive, progress, job_id)?;

    let mut warnings = Vec::new();
    let mut db_archive = None;
    let mut db_engine = None;
    if let Some(db) = site.database_name.as_deref() {
        let engine = engine_for(site, state);
        if matches!(engine.as_str(), "mysql" | "mariadb") {
            emit(progress, job_id, "database", "Dumping database…", 0, None);
            let dest = dir.join("db.sql.gz");
            match backup_database(paths, state, &engine, db, &dest, progress, job_id) {
                Ok(_) => {
                    db_archive = Some(dest);
                    db_engine = Some(engine);
                }
                Err(e) => {
                    warnings.push(format!("database skipped: {e}"));
                    crate::applog::warn("backup", format!("{}: database skipped: {e}", site.domain));
                }
            }
        }
    }

    let record = SiteBackupRecord {
        id,
        site_id: site.id.clone(),
        site_name: site.name.clone(),
        domain: site.domain.clone(),
        created_at: now.to_rfc3339(),
        dir: dir.clone(),
        files_archive,
        db_name: db_archive.as_ref().and(site.database_name.clone()),
        db_archive,
        db_engine,
        size_bytes: dir_size(&dir),
        warnings,
    };
    fs::write(dir.join(META_FILE), serde_json::to_string_pretty(&record)?)?;
    prune_backups(paths, state, &site.id)?;
    emit(progress, job_id, "done", "Backup complete", 100, Some(100));
    Ok(record)
}

pub fn list_site_backups(
    paths: &Paths,
    state: &AppState,
    site_id: Option<&str>,
) -> Vec<SiteBackupRecord> {
    let root = backups_root(paths, &state.backup_settings).join("sites");
    let mut out = Vec::new();
    let Ok(sites) = fs::read_dir(&root) else {
        return out;
    };
    for site_dir in sites.flatten() {
        if let Some(want) = site_id {
            if site_dir.file_name().to_string_lossy() != want {
                continue;
            }
        }
        let Ok(items) = fs::read_dir(site_dir.path()) else {
            continue;
        };
        for item in items.flatten() {
            let meta = item.path().join(META_FILE);
            if let Ok(raw) = fs::read_to_string(&meta) {
                if let Ok(rec) = serde_json::from_str::<SiteBackupRecord>(&raw) {
                    out.push(rec);
                }
            }
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

fn find_backup(paths: &Paths, state: &AppState, backup_id: &str) -> Result<SiteBackupRecord> {
    list_site_backups(paths, state, None)
        .into_iter()
        .find(|b| b.id == backup_id)
        .ok_or_else(|| DevStackError::Message(format!("backup not found: {backup_id}")))
}

pub fn restore_site_backup(
    paths: &Paths,
    state: &AppState,
    backup_id: &str,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<SiteBackupRecord> {
    let rec = find_backup(paths, state, backup_id)?;
    let site = state
        .sites
        .iter()
        .find(|s| s.id == rec.site_id)
        .ok_or_else(|| DevStackError::Message(format!("site no longer exists: {}", rec.site_id)))?;
    emit(progress, job_id, "extract", "Restoring site files…", 0, None);
    extract_directory_archive(&rec.files_archive, &expand_home(&site.path), progress, job_id)?;
    if let (Some(archive), Some(db), Some(engine)) =
        (rec.db_archive.as_deref(), rec.db_name.as_deref(), rec.db_engine.as_deref())
    {
        restore_database(paths, state, engine, db, archive, progress, job_id)?;
    }
    emit(progress, job_id, "done", "Restore complete", 100, Some(100));
    Ok(rec)
}

pub fn delete_site_backup(paths: &Paths, state: &AppState, backup_id: &str) -> Result<()> {
    let rec = find_backup(paths, state, backup_id)?;
    fs::remove_dir_all(&rec.dir)?;
    Ok(())
}

/// Keep only the newest `retain` backups for a site (0 = keep all).
pub fn prune_backups(paths: &Paths, state: &AppState, site_id: &str) -> Result<usize> {
    let retain = state.backup_settings.retain as usize;
    if retain == 0 {
        return Ok(0);
    }
    let all = list_site_backups(paths, state, Some(site_id));
    let mut removed = 0;
    for old in all.into_iter().skip(retain) {
        if fs::remove_dir_all(&old.dir).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn is_due(last: Option<DateTime<Local>>, now: DateTime<Local>, interval_secs: u64) -> bool {
    if interval_secs == 0 {
        return false;
    }
    match last {
        None => true,
        Some(t) => (now - t).num_seconds() >= interval_secs as i64,
    }
}

/// Back up every site whose newest backup is older than `interval_secs`.
/// Intended to be polled (e.g. every minute) from a background thread.
pub fn run_due_backups(
    paths: &Paths,
    state: &AppState,
    progress: &JobProgressFn,
) -> Result<Vec<SiteBackupRecord>> {
    let interval = state.backup_settings.interval_secs;
    if interval == 0 {
        return Ok(Vec::new());
    }
    let now = Local::now();
    let all = list_site_backups(paths, state, None);
    let mut done = Vec::new();
    for site in &state.sites {
        let last = all
            .iter()
            .find(|b| b.site_id == site.id)
            .and_then(|b| DateTime::parse_from_rfc3339(&b.created_at).ok())
            .map(|t| t.with_timezone(&Local));
        if !is_due(last, now, interval) {
            continue;
        }
        match backup_site(paths, state, &site.id, progress, "site-backup-auto") {
            Ok(rec) => done.push(rec),
            Err(e) => crate::applog::warn("backup", format!("auto-backup {}: {e}", site.domain)),
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::progress::noop_job_progress;

    #[test]
    fn backup_id_and_record_serialization() {
        assert_eq!(backup_id("blog", "20261109-101500"), "blog-20261109-101500");
        let rec = SiteBackupRecord {
            id: "blog-20261109-101500".into(),
            site_id: "blog".into(),
            site_name: "Blog".into(),
            domain: "blog.test".into(),
            created_at: "2026-11-09T10:15:00+00:00".into(),
            dir: PathBuf::from("/b/sites/blog/blog-20261109-101500"),
            files_archive: PathBuf::from("/b/files.tar.gz"),
            db_archive: None,
            db_name: None,
            db_engine: None,
            size_bytes: 42,
            warnings: vec![],
        };
        let json = serde_json::to_value(&rec).unwrap();
        assert_eq!(json["siteId"], "blog");
        assert_eq!(json["filesArchive"], "/b/files.tar.gz");
        assert!(json.get("dbArchive").is_none());
        let back: SiteBackupRecord = serde_json::from_value(json).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn due_logic() {
        let now = Local::now();
        assert!(!is_due(None, now, 0));
        assert!(is_due(None, now, 60));
        assert!(!is_due(Some(now - chrono::Duration::seconds(10)), now, 60));
        assert!(is_due(Some(now - chrono::Duration::seconds(120)), now, 60));
    }

    #[test]
    fn list_delete_and_prune_from_disk() {
        let root = std::env::temp_dir().join(format!("devstack-sbk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths::from_root(root.clone()).unwrap();
        let mut state = AppState::default();
        state.backup_settings.retain = 2;
        for i in 0..3 {
            let id = backup_id("blog", &format!("2026110{i}-000000"));
            let dir = site_backups_dir(&paths.backups, "blog").join(&id);
            fs::create_dir_all(&dir).unwrap();
            let rec = SiteBackupRecord {
                id: id.clone(),
                site_id: "blog".into(),
                site_name: "Blog".into(),
                domain: "blog.test".into(),
                created_at: format!("2026-11-0{}T00:00:00+00:00", i + 1),
                dir: dir.clone(),
                files_archive: dir.join("files.tar.gz"),
                db_archive: None,
                db_name: None,
                db_engine: None,
                size_bytes: 1,
                warnings: vec![],
            };
            fs::write(dir.join(META_FILE), serde_json::to_string(&rec).unwrap()).unwrap();
        }
        assert_eq!(list_site_backups(&paths, &state, Some("blog")).len(), 3);
        assert_eq!(prune_backups(&paths, &state, "blog").unwrap(), 1);
        let left = list_site_backups(&paths, &state, None);
        assert_eq!(left.len(), 2);
        assert!(left[0].created_at > left[1].created_at);
        delete_site_backup(&paths, &state, &left[0].id).unwrap();
        assert_eq!(list_site_backups(&paths, &state, None).len(), 1);
        let _ = noop_job_progress();
        let _ = fs::remove_dir_all(&root);
    }
}
