//! Streaming MySQL/MariaDB backup & restore (mysqldump | gzip/zstd).

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Stdio};
use std::thread;

use crate::config::{AppState, Paths};
use crate::error::{DevStackError, Result};
use crate::services::database::{
    client_bin_for, dump_bin_for, engine_install_dir, mysql_root_args,
};
use crate::services::progress::{emit, JobProgressFn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressKind {
    Gzip,
    Zstd,
}

impl CompressKind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gzip => "sql.gz",
            Self::Zstd => "sql.zst",
        }
    }

    pub fn detect(path: &Path) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        if name.ends_with(".zst") || name.ends_with(".zstd") {
            Self::Zstd
        } else {
            Self::Gzip
        }
    }

    pub fn preferred() -> Self {
        if which::which("zstd").is_ok() {
            Self::Zstd
        } else {
            Self::Gzip
        }
    }
}

pub fn backup_database(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db_name: &str,
    dest: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<u64> {
    let root = engine_install_dir(state, engine)?;
    let dump = dump_bin_for(&root)?;
    let connect = mysql_root_args(paths, state, engine);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let kind = if dest
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e == "zst" || e == "zstd")
        .unwrap_or(false)
        || dest
            .to_string_lossy()
            .ends_with(".sql.zst")
    {
        CompressKind::Zstd
    } else {
        CompressKind::preferred()
    };

    // Normalize dest extension if caller passed a stem
    let dest = if dest.extension().is_none() {
        PathBuf::from(format!("{}.{}", dest.display(), kind.extension()))
    } else {
        dest.to_path_buf()
    };

    emit(
        progress,
        job_id,
        "dump",
        &format!("Dumping `{db_name}`…"),
        0,
        None,
    );

    let mut dump_args = connect;
    dump_args.extend([
        "--single-transaction".into(),
        "--quick".into(),
        "--routines".into(),
        "--triggers".into(),
        "--hex-blob".into(),
        "--max_allowed_packet=1G".into(),
        db_name.to_string(),
    ]);
    let mut child = crate::services::process::command(&dump)
        .args(&dump_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("mysqldump spawn: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| DevStackError::Message("mysqldump stdout missing".into()))?;

    let out_file = File::create(&dest)?;
    let bytes = compress_stream(stdout, out_file, kind, progress, job_id)?;

    let status = child
        .wait()
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    if !status.success() {
        return Err(DevStackError::Message(
            "mysqldump failed — check database is running and the schema exists".into(),
        ));
    }

    emit(
        progress,
        job_id,
        "done",
        &format!("Backup written ({} bytes)", bytes),
        bytes,
        Some(bytes),
    );
    Ok(bytes)
}

pub fn restore_database(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db_name: &str,
    archive: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<()> {
    let root = engine_install_dir(state, engine)?;
    let client = client_bin_for(&root)?;
    let mut connect = mysql_root_args(paths, state, engine);
    connect.push(format!("--database={db_name}"));
    connect.push("--max_allowed_packet=1G".into());
    let kind = CompressKind::detect(archive);
    let total = fs::metadata(archive).map(|m| m.len()).unwrap_or(0);

    emit(
        progress,
        job_id,
        "restore",
        &format!("Importing into `{db_name}`…"),
        0,
        Some(total),
    );

    let mut child = crate::services::process::command(&client)
        .args(&connect)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("mysql spawn: {e}")))?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| DevStackError::Message("mysql stdin missing".into()))?;

    let file = File::open(archive)?;
    decompress_to_writer(file, stdin, kind, progress, job_id, total)?;

    let status = child
        .wait()
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    if !status.success() {
        return Err(DevStackError::Message(
            "mysql import failed — dump may be corrupt or credentials invalid".into(),
        ));
    }

    emit(
        progress,
        job_id,
        "done",
        "Restore complete",
        total,
        Some(total),
    );
    Ok(())
}

fn compress_stream<R: Read + Send + 'static, W: Write + Send + 'static>(
    reader: R,
    writer: W,
    kind: CompressKind,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<u64> {
    match kind {
        CompressKind::Gzip => {
            // Prefer system gzip for true streaming / low memory
            if which::which("gzip").is_ok() {
                pipe_via_compressor(reader, writer, "gzip", &["-c", "-1"], progress, job_id)
            } else {
                let mut enc = flate2::write::GzEncoder::new(writer, flate2::Compression::fast());
                let bytes = copy_counting(reader, &mut enc, progress, job_id, None)?;
                enc.finish()?;
                Ok(bytes)
            }
        }
        CompressKind::Zstd => {
            if which::which("zstd").is_ok() {
                pipe_via_compressor(reader, writer, "zstd", &["-T0", "-3", "-c"], progress, job_id)
            } else {
                // Fall back to gzip in-process if zstd binary missing
                let mut enc = flate2::write::GzEncoder::new(writer, flate2::Compression::fast());
                let bytes = copy_counting(reader, &mut enc, progress, job_id, None)?;
                enc.finish()?;
                Ok(bytes)
            }
        }
    }
}

pub fn decompress_import<R: Read + Send + 'static, W: Write + Send + 'static>(
    reader: R,
    writer: W,
    kind: CompressKind,
    progress: &JobProgressFn,
    job_id: &str,
    total: u64,
) -> Result<u64> {
    decompress_to_writer(reader, writer, kind, progress, job_id, total)
}

fn decompress_to_writer<R: Read + Send + 'static, W: Write + Send + 'static>(
    reader: R,
    writer: W,
    kind: CompressKind,
    progress: &JobProgressFn,
    job_id: &str,
    total: u64,
) -> Result<u64> {
    match kind {
        CompressKind::Gzip => {
            if which::which("gzip").is_ok() {
                pipe_via_compressor(reader, writer, "gzip", &["-dc"], progress, job_id)
            } else {
                let mut dec = flate2::read::GzDecoder::new(reader);
                copy_counting(&mut dec, writer, progress, job_id, Some(total))
            }
        }
        CompressKind::Zstd => {
            if which::which("zstd").is_ok() {
                pipe_via_compressor(reader, writer, "zstd", &["-dc"], progress, job_id)
            } else {
                Err(DevStackError::Message(
                    "zstd binary required to restore .zst backups (or use .sql.gz)".into(),
                ))
            }
        }
    }
}

fn pipe_via_compressor<R: Read + Send + 'static, W: Write + Send + 'static>(
    mut reader: R,
    mut writer: W,
    bin: &str,
    args: &[&str],
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<u64> {
    let mut child = crate::services::process::command(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("{bin} spawn: {e}")))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| DevStackError::Message(format!("{bin} stdin missing")))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| DevStackError::Message(format!("{bin} stdout missing")))?;

    let progress_in = progress.clone();
    let job_in = job_id.to_string();
    let input_thread = thread::spawn(move || -> Result<u64> {
        let mut buf = [0u8; 1024 * 256];
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
                    &progress_in,
                    &job_in,
                    "streaming",
                    &format!("Processed {total} bytes…"),
                    total,
                    None,
                );
            }
        }
        Ok(total)
    });

    let mut out_buf = [0u8; 1024 * 256];
    let mut written = 0u64;
    loop {
        let n = stdout.read(&mut out_buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&out_buf[..n])?;
        written += n as u64;
    }

    let _ = input_thread
        .join()
        .map_err(|_| DevStackError::Message("compress input thread panicked".into()))??;
    let status = child
        .wait()
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    if !status.success() {
        return Err(DevStackError::Message(format!("{bin} failed")));
    }
    Ok(written)
}

fn copy_counting<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    progress: &JobProgressFn,
    job_id: &str,
    total: Option<u64>,
) -> Result<u64> {
    let mut buf = [0u8; 1024 * 256];
    let mut n_total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        n_total += n as u64;
        if n_total % (8 * 1024 * 1024) < n as u64 {
            emit(
                progress,
                job_id,
                "streaming",
                &format!("Processed {n_total} bytes…"),
                n_total,
                total,
            );
        }
    }
    Ok(n_total)
}

/// Snapshot a directory tree via streaming `tar | gzip/zstd` (no size limit).
pub fn archive_directory(
    src: &Path,
    dest: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<u64> {
    if !src.is_dir() {
        return Err(DevStackError::Message(format!(
            "not a directory: {}",
            src.display()
        )));
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let use_zstd = dest.to_string_lossy().contains(".zst") && which::which("zstd").is_ok();
    emit(
        progress,
        job_id,
        "files",
        &format!("Archiving {}…", src.display()),
        0,
        None,
    );

    let mut tar = crate::services::process::command("tar")
        .args(["-C", &src.to_string_lossy(), "-cf", "-", "."])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("tar spawn: {e}")))?;

    let stdout = tar
        .stdout
        .take()
        .ok_or_else(|| DevStackError::Message("tar stdout missing".into()))?;
    let out = File::create(dest)?;
    let kind = if use_zstd {
        CompressKind::Zstd
    } else {
        CompressKind::Gzip
    };
    let bytes = compress_stream(stdout, BufWriter::new(out), kind, progress, job_id)?;
    let status = tar.wait()?;
    if !status.success() {
        return Err(DevStackError::Message("tar archive failed".into()));
    }
    Ok(bytes)
}

pub fn extract_directory_archive(
    archive: &Path,
    dest: &Path,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<()> {
    fs::create_dir_all(dest)?;
    emit(
        progress,
        job_id,
        "extract",
        &format!("Extracting into {}…", dest.display()),
        0,
        None,
    );

    let kind = if archive.to_string_lossy().contains(".zst") {
        CompressKind::Zstd
    } else {
        CompressKind::Gzip
    };

    let mut tar = crate::services::process::command("tar")
        .args(["-C", &dest.to_string_lossy(), "-xf", "-"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("tar extract spawn: {e}")))?;

    let stdin = tar
        .stdin
        .take()
        .ok_or_else(|| DevStackError::Message("tar stdin missing".into()))?;
    let file = File::open(archive)?;
    let total = fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    decompress_to_writer(BufReader::new(file), stdin, kind, progress, job_id, total)?;
    let status = tar.wait()?;
    if !status.success() {
        return Err(DevStackError::Message("tar extract failed".into()));
    }
    Ok(())
}

pub fn default_backup_path(paths: &Paths, site_id: &str, db_name: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let kind = CompressKind::preferred();
    paths
        .backups
        .join(site_id)
        .join(format!("{db_name}-{stamp}.{}", kind.extension()))
}

/// Generate a large-ish demo table for smoke/restore testing.
pub fn seed_large_demo_table(
    paths: &Paths,
    state: &AppState,
    engine: &str,
    db_name: &str,
    rows: u32,
) -> Result<()> {
    let root = engine_install_dir(state, engine)?;
    let client = client_bin_for(&root)?;
    let connect = mysql_root_args(paths, state, engine);
    let sql = format!(
        "USE `{db_name}`;
CREATE TABLE IF NOT EXISTS devstack_big (
  id BIGINT AUTO_INCREMENT PRIMARY KEY,
  payload LONGTEXT NOT NULL
) ENGINE=InnoDB;
"
    );
    let mut create_args = connect.clone();
    create_args.push("-e".into());
    create_args.push(sql);
    let _ = crate::services::process::command(&client)
        .args(&create_args)
        .status();

    // Insert via temp SQL files to avoid ARG_MAX limits on huge -e strings.
    let batch = 40u32;
    let mut left = rows;
    let tmp = paths.backups.join(format!("seed-{db_name}.sql"));
    if let Some(parent) = tmp.parent() {
        fs::create_dir_all(parent)?;
    }
    while left > 0 {
        let n = left.min(batch);
        let mut sql = String::from("INSERT INTO `devstack_big` (payload) VALUES ");
        for i in 0..n {
            if i > 0 {
                sql.push(',');
            }
            // ~1KB payload → multi-MB dumps with a few thousand rows
            let payload = format!("row-{}-{}", rows - left + i, "x".repeat(1024));
            sql.push_str(&format!("('{}')", payload.replace('\'', "''")));
        }
        sql.push(';');
        fs::write(&tmp, &sql)?;
        let mut insert_args = connect.clone();
        insert_args.push(format!("--database={db_name}"));
        let status = crate::services::process::command(&client)
            .args(&insert_args)
            .stdin(Stdio::from(File::open(&tmp)?))
            .status()
            .map_err(|e| DevStackError::Message(e.to_string()))?;
        if !status.success() {
            return Err(DevStackError::Message("failed seeding large table".into()));
        }
        left -= n;
    }
    let _ = fs::remove_file(&tmp);
    Ok(())
}
