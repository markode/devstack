//! Plain-file database exports under a user-visible `Databases/` folder.
//!
//! We intentionally dump via `mysqldump` / `pg_dump` / `mongodump` into portable
//! archives instead of copying raw engine datadir files. Raw InnoDB / Postgres
//! data files are not portable across versions, require the server to be fully
//! stopped, and break if any page is mid-write — dumps are the safe format.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::config::{AppState, DatabaseFilesSettings, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::backup::{
    backup_database, restore_database, CompressKind,
};
use crate::services::database::{
    client_bin_for, engine_install_dir, mysql_root_args,
};
use crate::services::process::find_bin;
use crate::services::progress::{emit, JobProgressFn};
use crate::services::php_runtime::resolve_php;
use crate::services::wordpress::{ensure_wp_cli, run_wp_cli};
use crate::services::nginx::expand_home;

/// Folder names under Databases/
pub const ENGINE_FOLDERS: &[&str] = &["mysql", "mariadb", "postgres", "mongo"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbFileEntry {
    pub engine: String,
    pub database: String,
    pub path: String,
    pub bytes: u64,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbFileChangeEvent {
    pub engine: String,
    pub database: String,
    pub path: String,
    pub kind: String,
    pub import_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSiteDatabaseRequest {
    pub site_id: String,
    pub archive_path: String,
    /// Optional search→replace for non-WP sites (from, to).
    pub search_replace: Option<(String, String)>,
}

fn extras_install_root(state: &AppState, package_id: &str) -> Option<PathBuf> {
    state
        .installed
        .get(package_id)
        .and_then(|v| v.first())
        .map(|r| r.path.clone())
}

/// Resolve the configured Databases folder (create layout).
pub fn databases_folder(paths: &Paths, settings: &DatabaseFilesSettings) -> Result<PathBuf> {
    let root = if let Some(ref custom) = settings.folder {
        expand_home(custom)
    } else {
        paths.root.join("Databases")
    };
    ensure_layout(&root)?;
    Ok(root)
}

pub fn ensure_layout(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    for eng in ENGINE_FOLDERS {
        fs::create_dir_all(root.join(eng))?;
        fs::create_dir_all(root.join(eng).join(".safety"))?;
    }
    Ok(())
}

fn folder_for_engine(engine: &str) -> &'static str {
    match engine {
        "mysql" => "mysql",
        "mariadb" => "mariadb",
        "postgresql" | "postgres" => "postgres",
        "mongodb" | "mongo" => "mongo",
        _ => "mariadb",
    }
}

pub fn canonical_engine(folder_or_engine: &str) -> &'static str {
    match folder_or_engine {
        "mysql" => "mysql",
        "mariadb" => "mariadb",
        "postgres" | "postgresql" => "postgresql",
        "mongo" | "mongodb" => "mongodb",
        _ => "mariadb",
    }
}

pub fn export_path_for(root: &Path, engine: &str, db_name: &str) -> PathBuf {
    let folder = folder_for_engine(engine);
    let ext = match engine {
        "mongodb" | "mongo" => "archive.gz",
        _ => "sql.gz",
    };
    root.join(folder).join(format!("{db_name}.{ext}"))
}

pub fn safety_path_for(root: &Path, engine: &str, db_name: &str) -> PathBuf {
    let folder = folder_for_engine(engine);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let ext = match engine {
        "mongodb" | "mongo" => "archive.gz",
        _ => "sql.gz",
    };
    root.join(folder)
        .join(".safety")
        .join(format!("{db_name}-{stamp}.{ext}"))
}

pub fn list_db_files(paths: &Paths, state: &AppState) -> Result<Vec<DbFileEntry>> {
    let root = databases_folder(paths, &state.db_files)?;
    let mut out = Vec::new();
    for eng in ENGINE_FOLDERS {
        let dir = root.join(eng);
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name.starts_with('.') {
                continue;
            }
            let database = strip_dump_ext(&name).unwrap_or(name.clone());
            let meta = entry.metadata().ok();
            let modified_at = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| {
                    let dt: chrono::DateTime<chrono::Local> = t.into();
                    Some(dt.to_rfc3339())
                });
            out.push(DbFileEntry {
                engine: (*eng).into(),
                database,
                path: path.display().to_string(),
                bytes: meta.map(|m| m.len()).unwrap_or(0),
                modified_at,
            });
        }
    }
    out.sort_by(|a, b| a.engine.cmp(&b.engine).then(a.database.cmp(&b.database)));
    Ok(out)
}

fn strip_dump_ext(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    for ext in [
        ".sql.gz",
        ".sql.zst",
        ".sql.zstd",
        ".archive.gz",
        ".sql",
        ".zip",
    ] {
        if lower.ends_with(ext) {
            return Some(name[..name.len() - ext.len()].to_string());
        }
    }
    None
}

/// List logical databases for a running engine.
pub fn list_engine_databases(paths: &Paths, state: &AppState, engine: &str) -> Result<Vec<String>> {
    match engine {
        "mysql" | "mariadb" => list_mysql_databases(paths, state, engine),
        "postgresql" | "postgres" => list_postgres_databases(paths, state),
        "mongodb" | "mongo" => list_mongo_databases(paths, state),
        _ => Ok(vec![]),
    }
}

fn list_mysql_databases(paths: &Paths, state: &AppState, engine: &str) -> Result<Vec<String>> {
    let root = engine_install_dir(state, engine)?;
    let client = client_bin_for(&root)?;
    let mut args = mysql_root_args(paths, state, engine);
    args.extend(["-N".into(), "-e".into(), "SHOW DATABASES".into()]);
    let out = crate::services::process::command(&client)
        .args(&args)
        .output()
        .map_err(|e| DevStackError::Message(format!("SHOW DATABASES: {e}")))?;
    if !out.status.success() {
        return Ok(vec![]);
    }
    let skip = HashSet::from([
        "information_schema",
        "mysql",
        "performance_schema",
        "sys",
        "phpmyadmin",
    ]);
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !skip.contains(l.as_str()))
        .collect())
}

fn list_postgres_databases(paths: &Paths, state: &AppState) -> Result<Vec<String>> {
    let Some(root) = extras_install_root(state, "postgresql") else {
        return Ok(vec![]);
    };
    let psql = find_bin(&root, &["psql", "psql.exe"])
        .ok_or_else(|| DevStackError::Message("psql not found".into()))?;
    let _ = paths;
    let port = state.stack.postgresql_port.to_string();
    let out = crate::services::process::command(&psql)
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-U",
            "postgres",
            "-Atc",
            "SELECT datname FROM pg_database WHERE datistemplate = false;",
        ])
        .output()
        .map_err(|e| DevStackError::Message(format!("psql list: {e}")))?;
    if !out.status.success() {
        return Ok(vec![]);
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && l != "postgres")
        .collect())
}

fn list_mongo_databases(paths: &Paths, state: &AppState) -> Result<Vec<String>> {
    let Some(root) = extras_install_root(state, "mongodb") else {
        return Ok(vec![]);
    };
    let _ = paths;
    let shell = find_bin(&root, &["mongosh", "mongosh.exe", "mongo", "mongo.exe"]);
    let Some(shell) = shell else {
        return Ok(vec![]);
    };
    let out = crate::services::process::command(&shell)
        .args([
            "--quiet",
            "--eval",
            "db.adminCommand('listDatabases').databases.map(d => d.name).join('\\n')",
        ])
        .output()
        .map_err(|e| DevStackError::Message(format!("mongo list: {e}")))?;
    if !out.status.success() {
        return Ok(vec![]);
    }
    let skip = HashSet::from(["admin", "local", "config"]);
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !skip.contains(l.as_str()))
        .collect())
}

/// Export one database into the Databases folder (atomic write).
pub fn export_database_file(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db_name: &str,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<DbFileEntry> {
    let root = databases_folder(paths, &state.db_files)?;
    let final_path = export_path_for(&root, engine, db_name);
    // Deterministic .partial next to target for atomic rename.
    let tmp = final_path.with_file_name(format!(
        "{}.partial",
        final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("dump")
    ));
    if let Some(parent) = tmp.parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = fs::remove_file(&tmp);

    let bytes = match engine {
        "mysql" | "mariadb" => {
            backup_database(paths, state, engine, db_name, &tmp, progress, job_id)?
        }
        "postgresql" | "postgres" => export_postgres(paths, state, db_name, &tmp, progress, job_id)?,
        "mongodb" | "mongo" => export_mongo(paths, state, db_name, &tmp, progress, job_id)?,
        other => {
            return Err(DevStackError::Message(format!(
                "export not supported for engine `{other}`"
            )))
        }
    };

    // backup_database may append extension — resolve actual tmp
    let written = if tmp.exists() {
        tmp
    } else if PathBuf::from(format!("{}.sql.gz", tmp.display())).exists() {
        PathBuf::from(format!("{}.sql.gz", tmp.display()))
    } else {
        // search for partial
        tmp
    };

    if written != final_path {
        fs::rename(&written, &final_path).or_else(|_| {
            fs::copy(&written, &final_path)?;
            fs::remove_file(&written)?;
            Ok::<(), DevStackError>(())
        })?;
    }

    let meta = fs::metadata(&final_path)?;
    Ok(DbFileEntry {
        engine: folder_for_engine(engine).into(),
        database: db_name.into(),
        path: final_path.display().to_string(),
        bytes: meta.len().max(bytes),
        modified_at: Some(chrono::Local::now().to_rfc3339()),
    })
}

pub fn export_all_running(
    paths: &Paths,
    state: &AppState,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<Vec<DbFileEntry>> {
    let mut out = Vec::new();
    let engines: Vec<&str> = {
        let mut e = Vec::new();
        if state.runtime.database_pid.is_some() || state.runtime.database_engine.is_some() {
            e.push(
                state
                    .runtime
                    .database_engine
                    .as_deref()
                    .or(state.active_database.as_deref())
                    .unwrap_or("mariadb"),
            );
        }
        if state.runtime.postgresql_pid.is_some() {
            e.push("postgresql");
        }
        if state.runtime.mongodb_pid.is_some() {
            e.push("mongodb");
        }
        e
    };
    for engine in engines {
        match list_engine_databases(paths, state, engine) {
            Ok(dbs) => {
                for db in dbs {
                    match export_database_file(paths, state, engine, &db, progress, job_id) {
                        Ok(entry) => out.push(entry),
                        Err(err) => {
                            emit(
                                progress,
                                job_id,
                                "warn",
                                &format!("skip {engine}/{db}: {err}"),
                                0,
                                None,
                            );
                        }
                    }
                }
            }
            Err(err) => {
                emit(
                    progress,
                    job_id,
                    "warn",
                    &format!("list {engine}: {err}"),
                    0,
                    None,
                );
            }
        }
    }
    Ok(out)
}

fn export_postgres(
    paths: &Paths,
    state: &AppState,
    db_name: &str,
    dest: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<u64> {
    let root = extras_install_root(state, "postgresql")
        .ok_or_else(|| DevStackError::Message("PostgreSQL not installed".into()))?;
    let pg_dump = find_bin(&root, &["pg_dump", "pg_dump.exe"])
        .ok_or_else(|| DevStackError::Message("pg_dump not found".into()))?;
    let _ = paths;
    emit(
        progress,
        job_id,
        "dump",
        &format!("pg_dump `{db_name}`…"),
        0,
        None,
    );
    let port = state.stack.postgresql_port.to_string();
    let mut child = crate::services::process::command(&pg_dump)
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-U",
            "postgres",
            "--no-owner",
            "--no-acl",
            db_name,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("pg_dump spawn: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DevStackError::Message("pg_dump stdout missing".into()))?;
    let out = File::create(dest)?;
    let bytes = gzip_stream(stdout, out, progress, job_id)?;
    let status = child.wait()?;
    if !status.success() {
        return Err(DevStackError::Message("pg_dump failed".into()));
    }
    Ok(bytes)
}

fn export_mongo(
    paths: &Paths,
    state: &AppState,
    db_name: &str,
    dest: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<u64> {
    let root = extras_install_root(state, "mongodb")
        .ok_or_else(|| DevStackError::Message("MongoDB not installed".into()))?;
    let mongodump = find_bin(&root, &["mongodump", "mongodump.exe"])
        .ok_or_else(|| DevStackError::Message("mongodump not found".into()))?;
    let _ = paths;
    emit(
        progress,
        job_id,
        "dump",
        &format!("mongodump `{db_name}`…"),
        0,
        None,
    );
    let port = format!("--port={}", state.stack.mongodb_port);
    let mut child = crate::services::process::command(&mongodump)
        .args([
            "--host=127.0.0.1",
            &port,
            &format!("--db={db_name}"),
            "--archive",
            "--gzip",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("mongodump spawn: {e}")))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| DevStackError::Message("mongodump stdout missing".into()))?;
    let mut out = File::create(dest)?;
    let mut buf = [0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = stdout.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        total += n as u64;
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(DevStackError::Message("mongodump failed".into()));
    }
    emit(
        progress,
        job_id,
        "done",
        &format!("Mongo archive written ({total} bytes)"),
        total,
        Some(total),
    );
    Ok(total)
}

fn gzip_stream<R: Read + Send + 'static, W: Write>(
    mut reader: R,
    writer: W,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<u64> {
    if which::which("gzip").is_ok() {
        let mut child = crate::services::process::command("gzip")
            .args(["-c", "-1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| DevStackError::Message(format!("gzip: {e}")))?;
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let progress_c = progress.clone();
        let job = job_id.to_string();
        let t = thread::spawn(move || -> Result<u64> {
            let mut buf = [0u8; 256 * 1024];
            let mut total = 0u64;
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                stdin.write_all(&buf[..n])?;
                total += n as u64;
                if total % (8 * 1024 * 1024) < n as u64 {
                    emit(
                        &progress_c,
                        &job,
                        "streaming",
                        &format!("Processed {total} bytes…"),
                        total,
                        None,
                    );
                }
            }
            Ok(total)
        });
        let mut out = writer;
        let mut buf = [0u8; 256 * 1024];
        let mut written = 0u64;
        loop {
            let n = stdout.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            written += n as u64;
        }
        let _ = t.join().map_err(|_| DevStackError::Message("gzip thread".into()))??;
        let status = child.wait()?;
        if !status.success() {
            return Err(DevStackError::Message("gzip failed".into()));
        }
        Ok(written)
    } else {
        let mut enc = flate2::write::GzEncoder::new(writer, flate2::Compression::fast());
        let mut buf = [0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            enc.write_all(&buf[..n])?;
            total += n as u64;
        }
        enc.finish()?;
        Ok(total)
    }
}

/// Safety-backup current DB then stream-import `archive` into `db_name`.
pub fn import_database_file(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db_name: &str,
    archive: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<()> {
    let root = databases_folder(paths, &state.db_files)?;
    // Safety backup of current DB (best-effort)
    emit(
        progress,
        job_id,
        "safety",
        "Writing safety backup of current database…",
        0,
        None,
    );
    let safety = safety_path_for(&root, engine, db_name);
    let _ = match engine {
        "mysql" | "mariadb" => {
            backup_database(paths, state, engine, db_name, &safety, progress, &format!("{job_id}-safety"))
        }
        "postgresql" | "postgres" => {
            export_postgres(paths, state, db_name, &safety, progress, &format!("{job_id}-safety"))
        }
        "mongodb" | "mongo" => {
            export_mongo(paths, state, db_name, &safety, progress, &format!("{job_id}-safety"))
        }
        _ => Ok(0),
    };

    let prepared = prepare_import_archive(archive)?;
    match engine {
        "mysql" | "mariadb" => {
            restore_database(paths, state, engine, db_name, &prepared, progress, job_id)?;
        }
        "postgresql" | "postgres" => {
            import_postgres(paths, state, db_name, &prepared, progress, job_id)?;
        }
        "mongodb" | "mongo" => {
            import_mongo(paths, state, db_name, &prepared, progress, job_id)?;
        }
        other => {
            return Err(DevStackError::Message(format!(
                "import not supported for `{other}`"
            )))
        }
    }
    if prepared != archive {
        let _ = fs::remove_file(&prepared);
    }
    Ok(())
}

/// Normalize .zip / plain .sql into a path restore_database understands.
fn prepare_import_archive(archive: &Path) -> Result<PathBuf> {
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if name.ends_with(".zip") {
        let tmp_dir = std::env::temp_dir().join(format!(
            "devstack-import-{}",
            chrono::Local::now().format("%Y%m%d%H%M%S")
        ));
        fs::create_dir_all(&tmp_dir)?;
        let file = File::open(archive)?;
        let mut zip = zip::ZipArchive::new(file)
            .map_err(|e| DevStackError::Message(format!("zip open: {e}")))?;
        let mut chosen: Option<PathBuf> = None;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| DevStackError::Message(format!("zip entry: {e}")))?;
            if entry.is_dir() {
                continue;
            }
            let ename = entry.name().to_lowercase();
            if !(ename.ends_with(".sql")
                || ename.ends_with(".sql.gz")
                || ename.ends_with(".sql.zst")
                || ename.ends_with(".gz"))
            {
                continue;
            }
            let out_name = Path::new(entry.name())
                .file_name()
                .map(|n| n.to_owned())
                .unwrap_or_else(|| std::ffi::OsString::from("dump.sql"));
            let out_path = tmp_dir.join(out_name);
            let mut out = File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out)?;
            chosen = Some(out_path);
            break;
        }
        chosen.ok_or_else(|| DevStackError::Message("zip contains no .sql dump".into()))
    } else if name.ends_with(".sql")
        && !name.ends_with(".sql.gz")
        && !name.ends_with(".sql.zst")
    {
        // Compress to temp .sql.gz for the existing restore pipeline
        let tmp = std::env::temp_dir().join(format!(
            "devstack-plain-{}.sql.gz",
            chrono::Local::now().format("%Y%m%d%H%M%S")
        ));
        let mut input = File::open(archive)?;
        let out = File::create(&tmp)?;
        let mut enc = flate2::write::GzEncoder::new(out, flate2::Compression::fast());
        std::io::copy(&mut input, &mut enc)?;
        enc.finish()?;
        Ok(tmp)
    } else {
        Ok(archive.to_path_buf())
    }
}

fn import_postgres(
    paths: &Paths,
    state: &AppState,
    db_name: &str,
    archive: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<()> {
    let root = extras_install_root(state, "postgresql")
        .ok_or_else(|| DevStackError::Message("PostgreSQL not installed".into()))?;
    let psql = find_bin(&root, &["psql", "psql.exe"])
        .ok_or_else(|| DevStackError::Message("psql not found".into()))?;
    let _ = paths;
    let total = fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    emit(
        progress,
        job_id,
        "restore",
        &format!("Importing into Postgres `{db_name}`…"),
        0,
        Some(total),
    );
    let port = state.stack.postgresql_port.to_string();
    // Ensure DB exists
    let _ = crate::services::process::command(&psql)
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-U",
            "postgres",
            "-c",
            &format!("CREATE DATABASE \"{db_name}\";"),
        ])
        .output();

    let mut child = crate::services::process::command(&psql)
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            &port,
            "-U",
            "postgres",
            "-d",
            db_name,
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("psql spawn: {e}")))?;
    let stdin = child.stdin.take().unwrap();
    let file = File::open(archive)?;
    let kind = CompressKind::detect(archive);
    crate::services::backup::decompress_import(file, stdin, kind, progress, job_id, total)?;
    let status = child.wait()?;
    if !status.success() {
        return Err(DevStackError::Message("psql import failed".into()));
    }
    Ok(())
}

fn import_mongo(
    paths: &Paths,
    state: &AppState,
    db_name: &str,
    archive: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<()> {
    let root = extras_install_root(state, "mongodb")
        .ok_or_else(|| DevStackError::Message("MongoDB not installed".into()))?;
    let mongorestore = find_bin(&root, &["mongorestore", "mongorestore.exe"])
        .ok_or_else(|| DevStackError::Message("mongorestore not found".into()))?;
    let _ = paths;
    let total = fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    emit(
        progress,
        job_id,
        "restore",
        &format!("mongorestore `{db_name}`…"),
        0,
        Some(total),
    );
    let port = format!("--port={}", state.stack.mongodb_port);
    let status = crate::services::process::command(&mongorestore)
        .args([
            "--host=127.0.0.1",
            &port,
            &format!("--nsInclude={db_name}.*"),
            "--gzip",
            &format!("--archive={}", archive.display()),
            "--drop",
        ])
        .status()
        .map_err(|e| DevStackError::Message(format!("mongorestore: {e}")))?;
    if !status.success() {
        return Err(DevStackError::Message("mongorestore failed".into()));
    }
    emit(progress, job_id, "done", "Mongo restore complete", total, Some(total));
    Ok(())
}

/// After importing into a site DB, rewrite URLs for WordPress (or optional pair).
pub async fn post_import_url_rewrite(
    paths: &Paths,
    state: &AppState,
    site: &SiteRecord,
    search_replace: Option<(String, String)>,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<()> {
    let abs = expand_home(&site.path);
    let is_wp = site.framework.as_deref() == Some("wordpress") || abs.join("wp-config.php").exists();
    let new_url = site_https_url(state, site);

    if is_wp {
        emit(
            progress,
            job_id,
            "search-replace",
            "Detecting old siteurl and running wp search-replace…",
            0,
            None,
        );
        let php_version = site
            .php_version
            .clone()
            .or_else(|| state.stack.php_version.clone())
            .unwrap_or_else(|| "8.3".into());
        let php_rt = resolve_php(paths, state, &php_version)?;
        let wp_cli = ensure_wp_cli(paths, progress, job_id).await?;
        let old_url = run_wp_cli(
            &php_rt.php_bin,
            &wp_cli,
            &abs,
            &["option", "get", "siteurl"],
        )
        .unwrap_or_default();
        let old_url = old_url.trim().to_string();
        if !old_url.is_empty() && old_url != new_url {
            wp_search_replace_urls(
                &php_rt.php_bin,
                &wp_cli,
                &abs,
                &old_url,
                &new_url,
                &site.domain,
            )?;
        }
    } else if let Some((from, to)) = search_replace {
        if !from.is_empty() && from != to {
            // Best-effort SQL REPLACE for MySQL/MariaDB sites
            if let Some(engine) = site
                .database_engine
                .as_deref()
                .or(state.active_database.as_deref())
            {
                if matches!(engine, "mysql" | "mariadb") {
                    if let Some(db) = site.database_name.as_deref() {
                        let _ = mysql_simple_replace(paths, state, engine, db, &from, &to);
                    }
                }
            }
        }
    }
    Ok(())
}

fn site_https_url(state: &AppState, site: &SiteRecord) -> String {
    let port = if crate::services::httpd::site_uses_apache(site) {
        state.stack.apache_https_port
    } else {
        state.stack.https_port
    };
    crate::services::ports::format_url("https", &site.domain, port)
}

pub fn wp_search_replace_urls(
    php: &Path,
    wp_cli: &Path,
    cwd: &Path,
    old_url: &str,
    new_url: &str,
    new_domain: &str,
) -> Result<()> {
    let old_domain = url::Url::parse(old_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| {
            old_url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or("")
                .to_string()
        });
    let replacements = [
        (old_url.to_string(), new_url.to_string()),
        (
            old_url.replace("https://", "http://"),
            new_url.replace("https://", "http://"),
        ),
        (
            old_url.replace("http://", "https://"),
            new_url.to_string(),
        ),
        (format!("https://{old_domain}"), new_url.to_string()),
        (
            format!("http://{old_domain}"),
            format!("http://{new_domain}"),
        ),
        (old_domain.clone(), new_domain.to_string()),
    ];
    for (from, to) in replacements {
        if from.is_empty() || from == to {
            continue;
        }
        let _ = run_wp_cli(
            php,
            wp_cli,
            cwd,
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
    Ok(())
}

pub fn mysql_simple_replace_pub(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db: &str,
    from: &str,
    to: &str,
) -> Result<()> {
    mysql_simple_replace(paths, state, engine, db, from, to)
}

fn mysql_simple_replace(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db: &str,
    from: &str,
    to: &str,
) -> Result<()> {
    let root = engine_install_dir(state, engine)?;
    let client = client_bin_for(&root)?;
    let connect = mysql_root_args(paths, state, engine);
    // Escape single quotes
    let from_e = from.replace('\'', "''");
    let to_e = to.replace('\'', "''");
    let sql = format!(
        "SELECT CONCAT('UPDATE `', table_name, '` SET `', column_name,
         '`=REPLACE(`', column_name, '`, ''{from_e}'', ''{to_e}'')')
         FROM information_schema.columns
         WHERE table_schema='{db}' AND data_type IN ('varchar','text','mediumtext','longtext','char');"
    );
    let mut args = connect.clone();
    args.extend(["-N".into(), "-e".into(), sql]);
    let out = crate::services::process::command(&client)
        .args(&args)
        .output()?;
    let stmts = String::from_utf8_lossy(&out.stdout);
    for line in stmts.lines().filter(|l| !l.trim().is_empty()) {
        let mut run = connect.clone();
        run.push(format!("--database={db}"));
        run.push("-e".into());
        run.push(line.to_string());
        let _ = crate::services::process::command(&client).args(&run).status();
    }
    Ok(())
}

pub fn parse_change_from_path(root: &Path, path: &Path) -> Option<(String, String)> {
    let rel = path.strip_prefix(root).ok()?;
    let mut comps = rel.components();
    let engine = comps.next()?.as_os_str().to_str()?.to_string();
    if !ENGINE_FOLDERS.contains(&engine.as_str()) {
        return None;
    }
    let file = comps.next()?.as_os_str().to_str()?.to_string();
    if file.starts_with('.') || file.ends_with(".partial") {
        return None;
    }
    // Ignore nested .safety
    if file == ".safety" {
        return None;
    }
    let db = strip_dump_ext(&file)?;
    Some((engine, db))
}

/// Shared suppress set so our own atomic exports don't trigger imports.
pub type SuppressSet = Arc<Mutex<HashSet<String>>>;

pub fn new_suppress_set() -> SuppressSet {
    Arc::new(Mutex::new(HashSet::new()))
}

pub fn suppress_path(set: &SuppressSet, path: &Path) {
    if let Ok(mut g) = set.lock() {
        g.insert(path.display().to_string());
    }
}

pub fn clear_suppress(set: &SuppressSet, path: &Path) {
    if let Ok(mut g) = set.lock() {
        g.remove(&path.display().to_string());
    }
}

pub fn is_suppressed(set: &SuppressSet, path: &Path) -> bool {
    set.lock()
        .map(|g| g.contains(&path.display().to_string()))
        .unwrap_or(false)
}

/// Spawn a background folder watcher. Emits callbacks on the provided channel-like Fn.
pub fn spawn_folder_watcher<F>(
    root: PathBuf,
    suppress: SuppressSet,
    running: Arc<AtomicBool>,
    mut on_change: F,
) -> Result<()>
where
    F: FnMut(DbFileChangeEvent) + Send + 'static,
{
    ensure_layout(&root)?;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| DevStackError::Message(format!("watcher: {e}")))?;

    for eng in ENGINE_FOLDERS {
        let dir = root.join(eng);
        let _ = watcher.watch(&dir, RecursiveMode::NonRecursive);
    }

    thread::spawn(move || {
        let _watcher = watcher; // keep alive
        let mut last: Option<(PathBuf, Instant)> = None;
        while running.load(Ordering::SeqCst) {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(event) => {
                    use notify::EventKind;
                    let interesting = matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Any
                    );
                    if !interesting {
                        continue;
                    }
                    for path in event.paths {
                        if is_suppressed(&suppress, &path) {
                            continue;
                        }
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("");
                        if name.ends_with(".partial") || name.starts_with('.') {
                            continue;
                        }
                        if let Some((eng, db)) = parse_change_from_path(&root, &path) {
                            // Debounce
                            let now = Instant::now();
                            if let Some((ref prev, t)) = last {
                                if prev == &path && now.duration_since(t) < Duration::from_millis(800)
                                {
                                    continue;
                                }
                            }
                            last = Some((path.clone(), now));
                            // Wait briefly for writers to finish
                            thread::sleep(Duration::from_millis(400));
                            if !path.exists() {
                                continue;
                            }
                            on_change(DbFileChangeEvent {
                                engine: eng,
                                database: db,
                                path: path.display().to_string(),
                                kind: "changed".into(),
                                import_mode: String::new(), // filled by caller
                            });
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Ok(())
}

/// Used by smoke tests: copy/replace a dump into the folder without going through watcher suppress.
pub fn write_dump_for_test(dest: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(dest, bytes)?;
    Ok(())
}

pub fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}
