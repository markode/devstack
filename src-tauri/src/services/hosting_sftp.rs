//! Pure-Rust SFTP fallback (libssh2 via `ssh2`) used when `rsync` is missing.
//!
//! Supports dry-run preview (local ↔ remote listing + size compare) and
//! pull/push with excludes. Prefer system `rsync` when available.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ssh2::Session;

use crate::error::{DevStackError, Result};
use crate::services::hosting_sync::{
    exclude_matches, local_push_listing, summarize, Direction, HostingProfile, PlanEntry,
    SecretStore, SyncPlan,
};
use crate::services::progress::{emit, JobProgressFn};

fn connect(profile: &HostingProfile, password: Option<&str>) -> Result<Session> {
    let addr = format!("{}:{}", profile.host, profile.port.max(1));
    let tcp = TcpStream::connect_timeout(
        &addr
            .parse()
            .map_err(|e| DevStackError::Message(format!("bad host/port {addr}: {e}")))?,
        Duration::from_secs(15),
    )
    .map_err(|e| DevStackError::Message(format!("SFTP connect {addr}: {e}")))?;
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = tcp.set_write_timeout(Some(Duration::from_secs(60)));
    let mut sess = Session::new().map_err(|e| DevStackError::Message(format!("ssh2 session: {e}")))?;
    sess.set_tcp_stream(tcp);
    sess.handshake()
        .map_err(|e| DevStackError::Message(format!("ssh handshake: {e}")))?;

    if profile.auth == "password" {
        let pw = password.unwrap_or("");
        sess.userauth_password(&profile.user, pw)
            .map_err(|e| DevStackError::Message(format!("SFTP password auth failed: {e}")))?;
    } else if let Some(key) = profile.key_path.as_deref().filter(|k| !k.is_empty()) {
        sess.userauth_pubkey_file(&profile.user, None, Path::new(key), password)
            .map_err(|e| DevStackError::Message(format!("SFTP key auth failed: {e}")))?;
    } else {
        // Agent / default identities.
        let _ = sess.userauth_agent(&profile.user);
        if !sess.authenticated() {
            // Try default id_rsa / id_ed25519 without passphrase.
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let mut ok = false;
            for name in ["id_ed25519", "id_rsa", "id_ecdsa"] {
                let p = home.join(".ssh").join(name);
                if p.exists()
                    && sess
                        .userauth_pubkey_file(&profile.user, None, &p, None)
                        .is_ok()
                {
                    ok = true;
                    break;
                }
            }
            if !ok {
                return Err(DevStackError::Message(
                    "SFTP auth failed: set a key path or password on the hosting profile".into(),
                ));
            }
        }
    }
    if !sess.authenticated() {
        return Err(DevStackError::Message("SFTP authentication failed".into()));
    }
    Ok(sess)
}

fn remote_join(base: &str, rel: &str) -> String {
    let b = base.trim_end_matches('/');
    let r = rel.trim_start_matches('/');
    if r.is_empty() {
        b.to_string()
    } else {
        format!("{b}/{r}")
    }
}

fn list_remote(
    sess: &Session,
    remote_root: &str,
    excludes: &[String],
) -> Result<Vec<PlanEntry>> {
    let sftp = sess
        .sftp()
        .map_err(|e| DevStackError::Message(format!("sftp channel: {e}")))?;
    let mut out = Vec::new();
    fn walk(
        sftp: &ssh2::Sftp,
        abs: &str,
        rel: &str,
        excludes: &[String],
        out: &mut Vec<PlanEntry>,
    ) -> Result<()> {
        let entries = sftp
            .readdir(Path::new(abs))
            .map_err(|e| DevStackError::Message(format!("sftp readdir {abs}: {e}")))?;
        for (path, stat) in entries {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if name == "." || name == ".." || name.is_empty() {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let is_dir = stat.is_dir();
            if exclude_matches(excludes, &child_rel, is_dir) {
                continue;
            }
            let child_abs = format!("{}/{}", abs.trim_end_matches('/'), name);
            if is_dir {
                out.push(PlanEntry {
                    path: format!("{child_rel}/"),
                    action: "update".into(),
                    size: 0,
                    is_dir: true,
                });
                walk(sftp, &child_abs, &child_rel, excludes, out)?;
            } else {
                out.push(PlanEntry {
                    path: child_rel,
                    action: "update".into(),
                    size: stat.size.unwrap_or(0),
                    is_dir: false,
                });
            }
        }
        Ok(())
    }
    walk(&sftp, remote_root, "", excludes, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn plan_from_listings(
    _profile: &HostingProfile,
    direction: Direction,
    local: &[PlanEntry],
    remote: &[PlanEntry],
) -> Vec<PlanEntry> {
    use std::collections::BTreeMap;
    let mut lmap: BTreeMap<&str, &PlanEntry> = BTreeMap::new();
    for e in local {
        if !e.is_dir {
            lmap.insert(e.path.trim_end_matches('/'), e);
        }
    }
    let mut rmap: BTreeMap<&str, &PlanEntry> = BTreeMap::new();
    for e in remote {
        if !e.is_dir {
            rmap.insert(e.path.trim_end_matches('/'), e);
        }
    }
    let mut out = Vec::new();
    match direction {
        Direction::Push => {
            for (path, le) in &lmap {
                match rmap.get(path) {
                    None => out.push(PlanEntry {
                        path: (*path).into(),
                        action: "add".into(),
                        size: le.size,
                        is_dir: false,
                    }),
                    Some(re) if re.size != le.size => out.push(PlanEntry {
                        path: (*path).into(),
                        action: "update".into(),
                        size: le.size,
                        is_dir: false,
                    }),
                    _ => {}
                }
            }
        }
        Direction::Pull => {
            for (path, re) in &rmap {
                match lmap.get(path) {
                    None => out.push(PlanEntry {
                        path: (*path).into(),
                        action: "add".into(),
                        size: re.size,
                        is_dir: false,
                    }),
                    Some(le) if le.size != re.size => out.push(PlanEntry {
                        path: (*path).into(),
                        action: "update".into(),
                        size: re.size,
                        is_dir: false,
                    }),
                    _ => {}
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn preview(
    profile: &HostingProfile,
    secrets: &dyn SecretStore,
    direction: Direction,
    local_root: &Path,
) -> Result<SyncPlan> {
    let pw = secrets.get(&crate::services::hosting_sync::ssh_secret_key(&profile.id));
    let sess = connect(profile, pw.as_deref())?;
    let remote_root = profile.remote_path.trim_end_matches('/').to_string();
    let remote = list_remote(&sess, &remote_root, &profile.excludes)?;
    let local = local_push_listing(local_root, &profile.excludes);
    let entries = plan_from_listings(profile, direction, &local, &remote);
    let mut plan = summarize(
        &profile.id,
        direction,
        true,
        entries,
        profile.excludes.clone(),
        "sftp",
    );
    plan.notes.push(
        "rsync not found — used built-in SFTP (size-based diff; timestamps not compared).".into(),
    );
    Ok(plan)
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    Ok(())
}

fn upload_file(sess: &Session, local: &Path, remote: &str) -> Result<u64> {
    let data = fs::read(local)?;
    let sftp = sess
        .sftp()
        .map_err(|e| DevStackError::Message(format!("sftp: {e}")))?;
    if let Some(parent) = Path::new(remote).parent() {
        let mut cur = PathBuf::new();
        for c in parent.components() {
            cur.push(c);
            let _ = sftp.mkdir(&cur, 0o755);
        }
    }
    let mut remote_f = sftp
        .create(Path::new(remote))
        .map_err(|e| DevStackError::Message(format!("sftp create {remote}: {e}")))?;
    remote_f
        .write_all(&data)
        .map_err(|e| DevStackError::Message(format!("sftp write {remote}: {e}")))?;
    Ok(data.len() as u64)
}

fn download_file(sess: &Session, remote: &str, local: &Path) -> Result<u64> {
    ensure_parent(local)?;
    let sftp = sess
        .sftp()
        .map_err(|e| DevStackError::Message(format!("sftp: {e}")))?;
    let mut remote_f = sftp
        .open(Path::new(remote))
        .map_err(|e| DevStackError::Message(format!("sftp open {remote}: {e}")))?;
    let mut buf = Vec::new();
    remote_f
        .read_to_end(&mut buf)
        .map_err(|e| DevStackError::Message(format!("sftp read {remote}: {e}")))?;
    fs::write(local, &buf)?;
    Ok(buf.len() as u64)
}

pub fn transfer(
    profile: &HostingProfile,
    secrets: &dyn SecretStore,
    direction: Direction,
    local_root: &Path,
    delete: bool,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<(usize, usize, u64, Vec<String>)> {
    let pw = secrets.get(&crate::services::hosting_sync::ssh_secret_key(&profile.id));
    let sess = connect(profile, pw.as_deref())?;
    let remote_root = profile.remote_path.trim_end_matches('/').to_string();
    let plan = preview(profile, secrets, direction, local_root)?;
    let total = plan.entries.iter().filter(|e| !e.is_dir).count();
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut deleted = 0usize;
    let mut notes = plan.notes;
    notes.push("Transferred via built-in SFTP (no rsync).".into());

    for entry in plan.entries.iter().filter(|e| !e.is_dir) {
        let local = local_root.join(&entry.path);
        let remote = remote_join(&remote_root, &entry.path);
        match direction {
            Direction::Push => {
                bytes += upload_file(&sess, &local, &remote)?;
            }
            Direction::Pull => {
                bytes += download_file(&sess, &remote, &local)?;
            }
        }
        files += 1;
        emit(
            progress,
            job_id,
            "sftp",
            &format!("{} {}", entry.action, entry.path),
            files as u64,
            Some(total as u64),
        );
    }

    if delete {
        // Best-effort: remove remote files not present locally (push) or local files not remote (pull).
        let remote = list_remote(&sess, &remote_root, &profile.excludes)?;
        let local = local_push_listing(local_root, &profile.excludes);
        let sftp = sess.sftp().ok();
        match direction {
            Direction::Push => {
                let local_set: std::collections::HashSet<_> = local
                    .iter()
                    .filter(|e| !e.is_dir)
                    .map(|e| e.path.trim_end_matches('/').to_string())
                    .collect();
                if let Some(sftp) = sftp.as_ref() {
                    for e in remote.iter().filter(|e| !e.is_dir) {
                        let p = e.path.trim_end_matches('/');
                        if !local_set.contains(p) {
                            let remote_path = remote_join(&remote_root, p);
                            if sftp.unlink(Path::new(&remote_path)).is_ok() {
                                deleted += 1;
                            }
                        }
                    }
                }
            }
            Direction::Pull => {
                let remote_set: std::collections::HashSet<_> = remote
                    .iter()
                    .filter(|e| !e.is_dir)
                    .map(|e| e.path.trim_end_matches('/').to_string())
                    .collect();
                for e in local.iter().filter(|e| !e.is_dir) {
                    let p = e.path.trim_end_matches('/');
                    if !remote_set.contains(p) {
                        let path = local_root.join(p);
                        if fs::remove_file(&path).is_ok() {
                            deleted += 1;
                        }
                    }
                }
            }
        }
    }

    Ok((files, deleted, bytes, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_join_trims_slashes() {
        assert_eq!(remote_join("/var/www/", "/a/b"), "/var/www/a/b");
        assert_eq!(remote_join("/var/www", "a"), "/var/www/a");
    }

    #[test]
    fn plan_detects_add_and_update() {
        let profile = HostingProfile {
            id: "p".into(),
            ..Default::default()
        };
        let local = vec![
            PlanEntry {
                path: "a.txt".into(),
                action: "add".into(),
                size: 10,
                is_dir: false,
            },
            PlanEntry {
                path: "b.txt".into(),
                action: "add".into(),
                size: 20,
                is_dir: false,
            },
        ];
        let remote = vec![PlanEntry {
            path: "b.txt".into(),
            action: "add".into(),
            size: 5,
            is_dir: false,
        }];
        let push = plan_from_listings(&profile, Direction::Push, &local, &remote);
        assert!(push.iter().any(|e| e.path == "a.txt" && e.action == "add"));
        assert!(push
            .iter()
            .any(|e| e.path == "b.txt" && e.action == "update"));
    }
}
