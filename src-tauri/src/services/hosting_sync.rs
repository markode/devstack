//! Hosting sync: pull/push a site's files (and optionally its database) over SSH.
//!
//! Shells out to the system OpenSSH client (`ssh`, `sftp`, `scp`) and `rsync` rather than
//! linking libssh2 — no extra native build dependency, and it honours the user's
//! `~/.ssh/config` / agent. Passwords live in the OS keychain (`keyring`) and reach
//! OpenSSH through an `SSH_ASKPASS` helper (never on argv).

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::{AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::progress::{emit, JobProgressFn};

pub const KEYCHAIN_SERVICE: &str = "devstack-hosting";

// ---------------------------------------------------------------------------
// Profiles + secrets
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteDb {
    pub host: String,
    pub name: String,
    pub user: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct HostingProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// `key` (agent / identity file) or `password` (keychain).
    pub auth: String,
    pub key_path: Option<String>,
    /// Absolute remote folder that mirrors the site docroot (e.g. `/var/www/html`).
    pub remote_path: String,
    /// Public URL of the remote site (for WordPress URL search-replace).
    pub remote_url: Option<String>,
    pub site_id: Option<String>,
    pub excludes: Vec<String>,
    pub remote_db: Option<RemoteDb>,
    /// Filled when listing: a password exists in the keychain.
    #[serde(skip_deserializing)]
    pub has_password: bool,
    #[serde(skip_deserializing)]
    pub has_db_password: bool,
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str);
}

pub struct KeyringStore;

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Option<String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, key).ok()?.get_password().ok()
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        keyring::Entry::new(KEYCHAIN_SERVICE, key)
            .and_then(|e| e.set_password(value))
            .map_err(|e| DevStackError::Message(format!("OS keychain unavailable: {e}")))
    }
    fn delete(&self, key: &str) {
        if let Ok(e) = keyring::Entry::new(KEYCHAIN_SERVICE, key) {
            let _ = e.delete_credential();
        }
    }
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Option<String> {
        self.0.lock().ok()?.get(key).cloned()
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) {
        if let Ok(mut g) = self.0.lock() {
            g.remove(key);
        }
    }
}

pub fn ssh_secret_key(profile_id: &str) -> String {
    format!("{profile_id}:ssh")
}

pub fn db_secret_key(profile_id: &str) -> String {
    format!("{profile_id}:db")
}

pub fn profiles_path(paths: &Paths) -> PathBuf {
    paths.root.join("hosting-profiles.json")
}

pub fn load_profiles(paths: &Paths) -> Vec<HostingProfile> {
    fs::read_to_string(profiles_path(paths))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_profiles(paths: &Paths, list: &[HostingProfile]) -> Result<()> {
    fs::write(profiles_path(paths), serde_json::to_string_pretty(list)?)?;
    Ok(())
}

pub fn list_profiles(paths: &Paths, secrets: &dyn SecretStore) -> Vec<HostingProfile> {
    load_profiles(paths)
        .into_iter()
        .map(|mut p| {
            p.has_password = secrets.get(&ssh_secret_key(&p.id)).is_some();
            p.has_db_password = secrets.get(&db_secret_key(&p.id)).is_some();
            p
        })
        .collect()
}

pub fn default_excludes() -> Vec<String> {
    [
        ".git/",
        ".svn/",
        "node_modules/",
        ".DS_Store",
        ".env",
        ".env.*",
        "*.log",
        "wp-content/cache/",
        "wp-content/upgrade/",
        "storage/logs/",
        "storage/framework/cache/",
        ".devstack-template",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub fn validate_profile(p: &HostingProfile) -> Result<()> {
    if p.host.trim().is_empty() || p.host.contains(char::is_whitespace) || p.host.starts_with('-') {
        return Err(DevStackError::Message("invalid host".into()));
    }
    if p.user.trim().is_empty() || p.user.contains(char::is_whitespace) || p.user.starts_with('-') {
        return Err(DevStackError::Message("invalid user".into()));
    }
    if !p.remote_path.starts_with('/') && !p.remote_path.starts_with('~') {
        return Err(DevStackError::Message("remote path must be absolute (or start with ~)".into()));
    }
    if p.remote_path.contains(['\n', '\r', '\'', '"', '`', '$', ';']) {
        return Err(DevStackError::Message("remote path contains unsupported characters".into()));
    }
    if !matches!(p.auth.as_str(), "key" | "password") {
        return Err(DevStackError::Message("auth must be `key` or `password`".into()));
    }
    Ok(())
}

pub fn save_profile(
    paths: &Paths,
    secrets: &dyn SecretStore,
    mut profile: HostingProfile,
    password: Option<String>,
    db_password: Option<String>,
) -> Result<HostingProfile> {
    if profile.port == 0 {
        profile.port = 22;
    }
    if profile.excludes.is_empty() {
        profile.excludes = default_excludes();
    }
    if profile.id.trim().is_empty() {
        profile.id = crate::config::slugify(&format!("{}-{}", profile.name, profile.host));
    }
    validate_profile(&profile)?;
    if let Some(pw) = password.filter(|p| !p.is_empty()) {
        secrets.set(&ssh_secret_key(&profile.id), &pw)?;
    }
    if let Some(pw) = db_password.filter(|p| !p.is_empty()) {
        secrets.set(&db_secret_key(&profile.id), &pw)?;
    }
    let mut list = load_profiles(paths);
    list.retain(|p| p.id != profile.id);
    list.push(profile.clone());
    write_profiles(paths, &list)?;
    profile.has_password = secrets.get(&ssh_secret_key(&profile.id)).is_some();
    profile.has_db_password = secrets.get(&db_secret_key(&profile.id)).is_some();
    Ok(profile)
}

pub fn delete_profile(paths: &Paths, secrets: &dyn SecretStore, id: &str) -> Result<()> {
    let mut list = load_profiles(paths);
    list.retain(|p| p.id != id);
    write_profiles(paths, &list)?;
    secrets.delete(&ssh_secret_key(id));
    secrets.delete(&db_secret_key(id));
    Ok(())
}

fn find_profile(paths: &Paths, id: &str) -> Result<HostingProfile> {
    load_profiles(paths)
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| DevStackError::Message(format!("hosting profile not found: {id}")))
}

// ---------------------------------------------------------------------------
// Exclude matching
// ---------------------------------------------------------------------------

fn glob_match(pat: &[char], text: &[char]) -> bool {
    match (pat.first(), text.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            // `**` crosses directories, `*` does not.
            if pat.get(1) == Some(&'*') {
                let rest = &pat[2..];
                let rest = if rest.first() == Some(&'/') { &rest[1..] } else { rest };
                (0..=text.len()).any(|i| glob_match(rest, &text[i..]))
            } else {
                let rest = &pat[1..];
                (0..=text.len())
                    .take_while(|i| *i == 0 || text[i - 1] != '/')
                    .any(|i| glob_match(rest, &text[i..]))
            }
        }
        (Some('?'), Some(c)) if *c != '/' => glob_match(&pat[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => glob_match(&pat[1..], &text[1..]),
        _ => false,
    }
}

/// rsync-style exclude matching on a `/`-separated relative path.
/// - trailing `/` matches directories (and everything beneath them)
/// - a pattern without `/` matches any path component (`*.log`, `.env`)
/// - a pattern with `/` is anchored at the root (`wp-content/cache/`)
pub fn exclude_matches(patterns: &[String], rel_path: &str, is_dir: bool) -> bool {
    let rel = rel_path.trim_start_matches('/').replace('\\', "/");
    let comps: Vec<&str> = rel.split('/').filter(|c| !c.is_empty()).collect();
    for raw in patterns {
        let mut pat = raw.trim().to_string();
        if pat.is_empty() || pat.starts_with('#') {
            continue;
        }
        let dir_only = pat.ends_with('/');
        if dir_only {
            pat.pop();
        }
        let anchored = pat.contains('/');
        let pat = pat.trim_start_matches('/').to_string();
        let pchars: Vec<char> = pat.chars().collect();
        if anchored {
            // Match against every prefix of the path (a matching dir excludes its children).
            for n in 1..=comps.len() {
                let prefix = comps[..n].join("/");
                let prefix_is_dir = n < comps.len() || is_dir;
                if dir_only && !prefix_is_dir {
                    continue;
                }
                if glob_match(&pchars, &prefix.chars().collect::<Vec<_>>()) {
                    return true;
                }
            }
        } else {
            for (i, c) in comps.iter().enumerate() {
                let comp_is_dir = i + 1 < comps.len() || is_dir;
                if dir_only && !comp_is_dir {
                    continue;
                }
                if glob_match(&pchars, &c.chars().collect::<Vec<_>>()) {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Plan (dry-run) model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Pull,
    Push,
}

impl Direction {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pull" | "download" => Ok(Self::Pull),
            "push" | "upload" => Ok(Self::Push),
            o => Err(DevStackError::Message(format!("unknown direction: {o}"))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub path: String,
    /// `add` | `update` | `delete`
    pub action: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub profile_id: String,
    pub direction: Direction,
    pub dry_run: bool,
    pub entries: Vec<PlanEntry>,
    pub add: usize,
    pub update: usize,
    pub delete: usize,
    pub bytes: u64,
    pub excludes: Vec<String>,
    pub tool: String,
    pub notes: Vec<String>,
}

pub fn summarize(
    profile_id: &str,
    direction: Direction,
    dry_run: bool,
    entries: Vec<PlanEntry>,
    excludes: Vec<String>,
    tool: &str,
) -> SyncPlan {
    let files = entries.iter().filter(|e| !e.is_dir);
    let add = files.clone().filter(|e| e.action == "add").count();
    let update = files.clone().filter(|e| e.action == "update").count();
    let delete = entries.iter().filter(|e| e.action == "delete").count();
    let bytes = files.filter(|e| e.action != "delete").map(|e| e.size).sum();
    SyncPlan {
        profile_id: profile_id.into(),
        direction,
        dry_run,
        entries,
        add,
        update,
        delete,
        bytes,
        excludes,
        tool: tool.into(),
        notes: vec![],
    }
}

/// Parse `rsync --itemize-changes --out-format=%i|%l|%n` lines.
pub fn parse_itemize(output: &str) -> Vec<PlanEntry> {
    let mut out = Vec::new();
    for line in output.lines() {
        let mut parts = line.splitn(3, '|');
        let (Some(flags), Some(size), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let flags = flags.trim();
        let name = name.trim_end_matches('/').to_string();
        if name.is_empty() || name == "." {
            continue;
        }
        let size: u64 = size.trim().parse().unwrap_or(0);
        if flags.starts_with("*deleting") {
            out.push(PlanEntry { path: name, action: "delete".into(), size: 0, is_dir: false });
            continue;
        }
        let mut chars = flags.chars();
        let _dir = chars.next();
        let kind = chars.next();
        let is_dir = kind == Some('d');
        if !matches!(kind, Some('f') | Some('d') | Some('L')) {
            continue;
        }
        let rest: String = chars.collect();
        let action = if rest.starts_with('+') { "add" } else { "update" };
        // Attribute-only changes on directories (".d..t......") are noise.
        if is_dir && action == "update" {
            continue;
        }
        out.push(PlanEntry { path: name, action: action.into(), size, is_dir });
    }
    out
}

// ---------------------------------------------------------------------------
// Tooling / command building
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostingTools {
    pub ssh: bool,
    pub sftp: bool,
    pub scp: bool,
    pub rsync: bool,
    pub keychain: bool,
    pub notes: Vec<String>,
}

pub fn tool_status(secrets: &dyn SecretStore) -> HostingTools {
    let has = |n: &str| which::which(n).is_ok();
    let probe = "__devstack_probe__";
    let keychain = secrets.set(probe, "1").is_ok();
    secrets.delete(probe);
    let mut notes = Vec::new();
    if !has("rsync") {
        notes.push(
            "rsync not found — using built-in SFTP for dry-run and pull/push (excludes applied; size-based diff). Install rsync/cwRsync for faster delta sync.".into(),
        );
    }
    if !has("ssh") {
        notes.push("OpenSSH client not found — install it to use hosting sync.".into());
    }
    if !keychain {
        notes.push("OS keychain unavailable: use key-based auth (passwords cannot be stored).".into());
    }
    HostingTools { ssh: has("ssh"), sftp: has("sftp"), scp: has("scp"), rsync: has("rsync"), keychain, notes }
}

fn quote_if_needed(s: &str) -> String {
    if s.contains(char::is_whitespace) {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

/// `ssh` option list shared by ssh/scp/sftp/rsync -e (no secrets).
pub fn ssh_options(profile: &HostingProfile) -> Vec<String> {
    let mut o = vec![
        "-p".to_string(),
        profile.port.max(1).to_string(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "ConnectTimeout=15".into(),
    ];
    if let Some(k) = profile.key_path.as_deref().filter(|k| !k.is_empty()) {
        o.push("-i".into());
        o.push(k.to_string());
    }
    if profile.auth == "password" {
        o.extend(["-o".into(), "PreferredAuthentications=password,keyboard-interactive".into()]);
        o.extend(["-o".into(), "NumberOfPasswordPrompts=1".into()]);
        o.extend(["-o".into(), "PubkeyAuthentication=no".into()]);
    } else {
        o.extend(["-o".into(), "BatchMode=yes".into()]);
    }
    o
}

pub fn ssh_command_string(profile: &HostingProfile) -> String {
    let mut parts = vec!["ssh".to_string()];
    parts.extend(ssh_options(profile).iter().map(|s| quote_if_needed(s)));
    parts.join(" ")
}

pub fn remote_spec(profile: &HostingProfile) -> String {
    format!(
        "{}@{}:{}/",
        profile.user,
        profile.host,
        profile.remote_path.trim_end_matches('/')
    )
}

fn local_dir_arg(local: &Path) -> String {
    let mut s = local.display().to_string().replace('\\', "/");
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

pub fn rsync_args(
    profile: &HostingProfile,
    direction: Direction,
    dry_run: bool,
    local: &Path,
    delete: bool,
) -> Vec<String> {
    let mut a = vec![
        "-rltz".to_string(),
        "--partial".into(),
        "--itemize-changes".into(),
        "--out-format=%i|%l|%n".into(),
        "-e".into(),
        ssh_command_string(profile),
    ];
    if dry_run {
        a.push("--dry-run".into());
    }
    if delete {
        a.push("--delete".into());
    }
    for ex in &profile.excludes {
        a.push(format!("--exclude={ex}"));
    }
    let (src, dst) = match direction {
        Direction::Pull => (remote_spec(profile), local_dir_arg(local)),
        Direction::Push => (local_dir_arg(local), remote_spec(profile)),
    };
    a.push(src);
    a.push(dst);
    a
}

/// `SSH_ASKPASS` helper so OpenSSH can read the password without it touching argv.
pub fn ensure_askpass(paths: &Paths) -> Result<PathBuf> {
    let name = if cfg!(windows) { "ssh-askpass.bat" } else { "ssh-askpass.sh" };
    let path = paths.tools.join(name);
    fs::create_dir_all(&paths.tools)?;
    let body = if cfg!(windows) {
        "@echo off\r\necho %DEVSTACK_SSH_PASSWORD%\r\n"
    } else {
        "#!/bin/sh\nprintf '%s\\n' \"$DEVSTACK_SSH_PASSWORD\"\n"
    };
    fs::write(&path, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

fn apply_auth_env(
    cmd: &mut Command,
    paths: &Paths,
    profile: &HostingProfile,
    password: Option<&str>,
) -> Result<()> {
    if profile.auth == "password" {
        let pw = password.ok_or_else(|| {
            DevStackError::Message("no stored password for this profile — save it again".into())
        })?;
        let askpass = ensure_askpass(paths)?;
        cmd.env("SSH_ASKPASS", askpass)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()))
            .env("DEVSTACK_SSH_PASSWORD", pw);
    }
    Ok(())
}

fn ssh_command(paths: &Paths, profile: &HostingProfile, password: Option<&str>, remote_cmd: &str) -> Result<Command> {
    let mut cmd = crate::services::process::command("ssh");
    cmd.args(ssh_options(profile))
        .arg(format!("{}@{}", profile.user, profile.host))
        .arg(remote_cmd)
        .stdin(Stdio::null());
    apply_auth_env(&mut cmd, paths, profile, password)?;
    Ok(cmd)
}

pub fn test_connection(
    paths: &Paths,
    secrets: &dyn SecretStore,
    profile_id: &str,
) -> Result<String> {
    let profile = find_profile(paths, profile_id)?;
    let pw = secrets.get(&ssh_secret_key(&profile.id));
    let remote_cmd = format!("test -d '{}' && echo devstack-ok || echo devstack-missing-dir", profile.remote_path);
    let out = ssh_command(paths, &profile, pw.as_deref(), &remote_cmd)?
        .output()
        .map_err(|e| DevStackError::Message(format!("ssh failed to start: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        return Err(DevStackError::Message(format!(
            "SSH connection failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    if stdout.contains("devstack-missing-dir") {
        return Ok(format!("Connected, but {} does not exist on the server yet.", profile.remote_path));
    }
    Ok(format!("Connected to {}@{}", profile.user, profile.host))
}

// ---------------------------------------------------------------------------
// Preview + run
// ---------------------------------------------------------------------------

fn site_for_profile<'a>(state: &'a AppState, profile: &HostingProfile) -> Result<&'a SiteRecord> {
    let id = profile
        .site_id
        .as_deref()
        .ok_or_else(|| DevStackError::Message("profile is not linked to a site".into()))?;
    state
        .sites
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| DevStackError::Message(format!("linked site not found: {id}")))
}

/// Walk `root` and return files that would be pushed (excludes applied) — used for
/// the preview when rsync is not installed.
pub fn local_push_listing(root: &Path, excludes: &[String]) -> Vec<PlanEntry> {
    fn walk(base: &Path, dir: &Path, ex: &[String], out: &mut Vec<PlanEntry>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for ent in rd.flatten() {
            let path = ent.path();
            let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let is_dir = path.is_dir();
            if exclude_matches(ex, &rel, is_dir) {
                continue;
            }
            if is_dir {
                walk(base, &path, ex, out);
            } else {
                let size = ent.metadata().map(|m| m.len()).unwrap_or(0);
                out.push(PlanEntry { path: rel, action: "add".into(), size, is_dir: false });
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, excludes, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn preview(
    paths: &Paths,
    state: &AppState,
    secrets: &dyn SecretStore,
    profile_id: &str,
    direction: Direction,
    delete: bool,
) -> Result<SyncPlan> {
    let profile = find_profile(paths, profile_id)?;
    let site = site_for_profile(state, &profile)?;
    let local = crate::paths_util::expand_home(&site.path);
    if which::which("rsync").is_err() {
        return crate::services::hosting_sftp::preview(&profile, secrets, direction, &local);
    }
    let pw = secrets.get(&ssh_secret_key(&profile.id));
    let args = rsync_args(&profile, direction, true, &local, delete);
    let mut cmd = crate::services::process::command("rsync");
    cmd.args(&args).stdin(Stdio::null());
    apply_auth_env(&mut cmd, paths, &profile, pw.as_deref())?;
    let out = cmd
        .output()
        .map_err(|e| DevStackError::Message(format!("rsync failed to start: {e}")))?;
    if !out.status.success() {
        return Err(DevStackError::Message(format!(
            "rsync dry-run failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let entries = parse_itemize(&String::from_utf8_lossy(&out.stdout));
    Ok(summarize(&profile.id, direction, true, entries, profile.excludes.clone(), "rsync"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub profile_id: String,
    pub direction: Direction,
    pub files_transferred: usize,
    pub deleted: usize,
    pub bytes: u64,
    pub database_synced: bool,
    pub url_rewritten: Option<String>,
    pub notes: Vec<String>,
}

fn run_streaming(
    mut cmd: Command,
    total: usize,
    progress: &JobProgressFn,
    job_id: &str,
    label: &str,
) -> Result<(usize, usize, u64)> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    let mut child = cmd
        .spawn()
        .map_err(|e| DevStackError::Message(format!("rsync failed to start: {e}")))?;
    let stdout = child.stdout.take().ok_or_else(|| DevStackError::Message("no stdout".into()))?;
    let (mut files, mut deleted, mut bytes) = (0usize, 0usize, 0u64);
    for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
        for e in parse_itemize(&line) {
            if e.action == "delete" {
                deleted += 1;
            } else if !e.is_dir {
                files += 1;
                bytes += e.size;
            }
            emit(
                progress,
                job_id,
                label,
                &format!("{} {}", e.action, e.path),
                files as u64,
                if total > 0 { Some(total as u64) } else { None },
            );
        }
    }
    let status = child.wait()?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut s) = child.stderr.take() {
            use std::io::Read;
            let _ = s.read_to_string(&mut err);
        }
        return Err(DevStackError::Message(format!("sync failed: {}", err.trim())));
    }
    Ok((files, deleted, bytes))
}

fn scp_fallback(
    paths: &Paths,
    profile: &HostingProfile,
    secrets: &dyn SecretStore,
    direction: Direction,
    local: &Path,
) -> Result<()> {
    let mut cmd = crate::services::process::command("scp");
    cmd.arg("-r").arg("-P").arg(profile.port.max(1).to_string());
    for (i, o) in ssh_options(profile).iter().enumerate() {
        // `-p` is the port flag for ssh but preserve-times for scp; skip the first pair.
        if i < 2 {
            continue;
        }
        cmd.arg(o);
    }
    let remote = format!("{}@{}:{}/.", profile.user, profile.host, profile.remote_path.trim_end_matches('/'));
    match direction {
        Direction::Pull => cmd.arg(remote).arg(local),
        Direction::Push => cmd.arg(format!("{}/.", local.display())).arg(format!(
            "{}@{}:{}",
            profile.user,
            profile.host,
            profile.remote_path.trim_end_matches('/')
        )),
    };
    apply_auth_env(&mut cmd, paths, profile, secrets.get(&ssh_secret_key(&profile.id)).as_deref())?;
    let out = cmd.stdin(Stdio::null()).output()?;
    if !out.status.success() {
        return Err(DevStackError::Message(format!("scp failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(())
}

fn db_host(db: &RemoteDb) -> &str {
    if db.host.trim().is_empty() {
        "127.0.0.1"
    } else {
        db.host.trim()
    }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The remote shell reads the DB password from the first stdin line, so it never
/// appears in any process argument list (local or remote).
const READ_PW: &str = "IFS= read -r MYSQL_PWD; export MYSQL_PWD;";

fn remote_dump_cmd(db: &RemoteDb) -> String {
    format!(
        "{READ_PW} mysqldump --single-transaction --no-tablespaces -h {} -u {} {}",
        sh_quote(db_host(db)),
        sh_quote(&db.user),
        sh_quote(&db.name)
    )
}

fn remote_import_cmd(db: &RemoteDb) -> String {
    format!(
        "{READ_PW} gunzip -c | mysql -h {} -u {} {}",
        sh_quote(db_host(db)),
        sh_quote(&db.user),
        sh_quote(&db.name)
    )
}

/// Run a remote command, optionally feeding `stdin_line` (+ a file) and redirecting stdout to a file.
fn ssh_run(
    paths: &Paths,
    profile: &HostingProfile,
    ssh_pw: Option<&str>,
    remote_cmd: &str,
    stdin_line: Option<&str>,
    stdin_file: Option<&Path>,
    stdout_file: Option<&Path>,
) -> Result<()> {
    use std::io::Write;
    let mut cmd = ssh_command(paths, profile, ssh_pw, remote_cmd)?;
    cmd.stdin(Stdio::piped()).stderr(Stdio::piped());
    match stdout_file {
        Some(f) => cmd.stdout(fs::File::create(f)?),
        None => cmd.stdout(Stdio::null()),
    };
    let mut child = cmd.spawn().map_err(|e| DevStackError::Message(format!("ssh failed to start: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(line) = stdin_line {
            let _ = writeln!(stdin, "{line}");
        }
        if let Some(f) = stdin_file {
            let mut file = fs::File::open(f)?;
            let _ = std::io::copy(&mut file, &mut stdin);
        }
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(DevStackError::Message(String::from_utf8_lossy(&out.stderr).trim().to_string()));
    }
    Ok(())
}

fn pull_database(
    paths: &Paths,
    state: &AppState,
    profile: &HostingProfile,
    site: &SiteRecord,
    secrets: &dyn SecretStore,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<Option<String>> {
    let db = profile.remote_db.as_ref().ok_or_else(|| DevStackError::Message("profile has no remote database configured".into()))?;
    let dbpw = secrets.get(&db_secret_key(&profile.id)).unwrap_or_default();
    let local_db = site.database_name.clone().ok_or_else(|| DevStackError::Message("site has no local database".into()))?;
    let engine = site.database_engine.clone().or_else(|| state.active_database.clone()).unwrap_or_else(|| "mariadb".into());
    emit(progress, job_id, "database", "Dumping remote database…", 0, None);
    let dump = paths.downloads.join(format!("hosting-{}-pull.sql", profile.id));
    let pw = secrets.get(&ssh_secret_key(&profile.id));
    ssh_run(paths, profile, pw.as_deref(), &remote_dump_cmd(db), Some(&dbpw), None, Some(&dump))
        .map_err(|e| DevStackError::Message(format!("remote mysqldump failed: {e}")))?;
    emit(progress, job_id, "database", "Importing into local database…", 0, None);
    crate::services::backup::restore_database(paths, state, &engine, &local_db, &dump, progress, job_id)?;
    let _ = fs::remove_file(&dump);
    // Serialized-safe remote URL → local URL rewrite (WordPress).
    if let Some(remote_url) = profile.remote_url.as_deref().filter(|u| !u.is_empty()) {
        if crate::services::wp_url::site_is_wordpress(site) {
            let local_url = crate::services::ports::canonical_site_url(site, state);
            emit(progress, job_id, "url", &format!("Rewriting {remote_url} → {local_url}"), 0, None);
            return crate::services::wp_url::repair_site_url_to(paths, state, site, &local_url, Some(remote_url.trim_end_matches('/')), None).map(Some);
        }
    }
    Ok(None)
}

fn push_database(
    paths: &Paths,
    state: &AppState,
    profile: &HostingProfile,
    site: &SiteRecord,
    secrets: &dyn SecretStore,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<Option<String>> {
    let db = profile.remote_db.as_ref().ok_or_else(|| DevStackError::Message("profile has no remote database configured".into()))?;
    let dbpw = secrets.get(&db_secret_key(&profile.id)).unwrap_or_default();
    let local_db = site.database_name.clone().ok_or_else(|| DevStackError::Message("site has no local database".into()))?;
    let engine = site.database_engine.clone().or_else(|| state.active_database.clone()).unwrap_or_else(|| "mariadb".into());
    let pw = secrets.get(&ssh_secret_key(&profile.id));

    // 1. Dump local, load into a scratch DB, rewrite URLs there (never touch the live local DB).
    emit(progress, job_id, "database", "Preparing database copy…", 0, None);
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let first = paths.downloads.join(format!("hosting-{}-push-a-{stamp}.sql.gz", profile.id));
    crate::services::backup::backup_database(paths, state, &engine, &local_db, &first, progress, job_id)?;
    let scratch = format!("{local_db}__push");
    let (user, pass) = (
        site.database_user.clone().unwrap_or_else(|| "root".into()),
        site.database_password.clone().unwrap_or_default(),
    );
    crate::services::database::ensure_database(paths, state, &engine, &scratch, &user, &pass)?;
    crate::services::backup::restore_database(paths, state, &engine, &scratch, &first, progress, job_id)?;
    let mut note = None;
    if let Some(remote_url) = profile.remote_url.as_deref().filter(|u| !u.is_empty()) {
        if crate::services::wp_url::site_is_wordpress(site) {
            let local_url = crate::services::ports::canonical_site_url(site, state);
            note = Some(crate::services::wp_url::repair_site_url_to(
                paths,
                state,
                site,
                remote_url.trim_end_matches('/'),
                Some(&local_url),
                Some(&scratch),
            )?);
        }
    }
    let second = paths.downloads.join(format!("hosting-{}-push-b-{stamp}.sql.gz", profile.id));
    crate::services::backup::backup_database(paths, state, &engine, &scratch, &second, progress, job_id)?;
    drop_database(paths, state, &engine, &scratch);
    let _ = fs::remove_file(&first);

    // 2. Back up the remote DB, then import.
    emit(progress, job_id, "database", "Backing up remote database…", 0, None);
    let backup_cmd = format!(
        "{} | gzip > \"$HOME/devstack-remote-backup-{stamp}.sql.gz\"",
        remote_dump_cmd(db)
    );
    if let Err(e) = ssh_run(paths, profile, pw.as_deref(), &backup_cmd, Some(&dbpw), None, None) {
        let _ = fs::remove_file(&second);
        return Err(DevStackError::Message(format!("remote backup failed (push aborted): {e}")));
    }
    emit(progress, job_id, "database", "Uploading database…", 0, None);
    let res = ssh_run(paths, profile, pw.as_deref(), &remote_import_cmd(db), Some(&dbpw), Some(&second), None);
    let _ = fs::remove_file(&second);
    res.map_err(|e| DevStackError::Message(format!("remote import failed: {e}")))?;
    Ok(note)
}

fn drop_database(paths: &Paths, state: &AppState, engine: &str, name: &str) {
    if let Ok(root) = crate::services::database::engine_install_dir(state, engine) {
        if let Ok(client) = crate::services::database::client_bin_for(&root) {
            let mut args = crate::services::database::mysql_root_args(paths, state, engine);
            args.push("-e".into());
            args.push(format!("DROP DATABASE IF EXISTS `{name}`"));
            let _ = crate::services::process::command(client).args(args).status();
        }
    }
}

pub fn run_sync(
    paths: &Paths,
    state: &AppState,
    secrets: &dyn SecretStore,
    profile_id: &str,
    direction: Direction,
    delete: bool,
    include_database: bool,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<SyncResult> {
    let profile = find_profile(paths, profile_id)?;
    let site = site_for_profile(state, &profile)?.clone();
    let local = crate::paths_util::expand_home(&site.path);
    fs::create_dir_all(&local)?;
    let mut notes = Vec::new();
    let (mut files, mut deleted, mut bytes) = (0usize, 0usize, 0u64);

    if which::which("rsync").is_ok() {
        emit(progress, job_id, "plan", "Comparing files…", 0, None);
        let total = preview(paths, state, secrets, profile_id, direction, delete)
            .map(|p| p.entries.iter().filter(|e| !e.is_dir).count())
            .unwrap_or(0);
        let pw = secrets.get(&ssh_secret_key(&profile.id));
        let mut cmd = crate::services::process::command("rsync");
        cmd.args(rsync_args(&profile, direction, false, &local, delete));
        apply_auth_env(&mut cmd, paths, &profile, pw.as_deref())?;
        let label = if direction == Direction::Pull { "pull" } else { "push" };
        let (f, d, b) = run_streaming(cmd, total, progress, job_id, label)?;
        files = f;
        deleted = d;
        bytes = b;
    } else {
        emit(progress, job_id, "transfer", "Transferring with SFTP…", 0, None);
        match crate::services::hosting_sftp::transfer(
            &profile,
            secrets,
            direction,
            &local,
            delete,
            progress,
            job_id,
        ) {
            Ok((f, d, b, n)) => {
                files = f;
                deleted = d;
                bytes = b;
                notes.extend(n);
            }
            Err(e) => {
                // Last resort: legacy scp (no excludes).
                emit(progress, job_id, "transfer", "SFTP failed — falling back to scp…", 0, None);
                scp_fallback(paths, &profile, secrets, direction, &local)?;
                notes.push(format!("SFTP failed ({e}); used scp without excludes."));
            }
        }
    }

    let mut database_synced = false;
    let mut url_rewritten = None;
    if include_database {
        let res = match direction {
            Direction::Pull => pull_database(paths, state, &profile, &site, secrets, progress, job_id),
            Direction::Push => push_database(paths, state, &profile, &site, secrets, progress, job_id),
        };
        match res {
            Ok(msg) => {
                database_synced = true;
                url_rewritten = msg;
            }
            Err(e) => notes.push(format!("database sync failed: {e}")),
        }
    }
    emit(progress, job_id, "done", "Sync complete", 100, Some(100));
    Ok(SyncResult {
        profile_id: profile.id,
        direction,
        files_transferred: files,
        deleted,
        bytes,
        database_synced,
        url_rewritten,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn profile() -> HostingProfile {
        HostingProfile {
            id: "prod".into(),
            name: "Prod".into(),
            host: "example.com".into(),
            port: 2222,
            user: "deploy".into(),
            auth: "key".into(),
            key_path: Some("/home/me/my key".into()),
            remote_path: "/var/www/html".into(),
            remote_url: Some("https://example.com".into()),
            site_id: Some("blog".into()),
            excludes: s(&[".git/", "*.log"]),
            remote_db: None,
            has_password: false,
            has_db_password: false,
        }
    }

    #[test]
    fn exclude_matching_rules() {
        let ex = s(&[".git/", "node_modules/", "*.log", ".env", "wp-content/cache/", "build/**/*.map"]);
        assert!(exclude_matches(&ex, ".git", true));
        assert!(exclude_matches(&ex, ".git/config", false));
        assert!(!exclude_matches(&ex, ".gitignore", false));
        assert!(exclude_matches(&ex, "app/node_modules/x/index.js", false));
        assert!(exclude_matches(&ex, "storage/laravel.log", false));
        assert!(exclude_matches(&ex, ".env", false));
        assert!(!exclude_matches(&ex, "env.php", false));
        assert!(exclude_matches(&ex, "wp-content/cache/a/b.html", false));
        assert!(!exclude_matches(&ex, "other/wp-content/cache/a.html", false));
        assert!(exclude_matches(&ex, "build/js/app.js.map", false));
        assert!(!exclude_matches(&ex, "build/js/app.js", false));
        // dir-only pattern must not exclude a *file* of the same name
        assert!(!exclude_matches(&s(&["cache/"]), "cache", false));
        assert!(exclude_matches(&s(&["cache/"]), "cache", true));
        assert!(!exclude_matches(&[], "anything", false));
    }

    #[test]
    fn rsync_args_never_carry_secrets_and_apply_excludes() {
        let local = Path::new("/home/me/Sites/blog");
        let a = rsync_args(&profile(), Direction::Pull, true, local, false);
        assert!(a.contains(&"--dry-run".to_string()));
        assert!(a.contains(&"--exclude=.git/".to_string()));
        assert!(a.contains(&"--exclude=*.log".to_string()));
        assert!(!a.contains(&"--delete".to_string()));
        let e = a.iter().position(|x| x == "-e").unwrap();
        assert!(a[e + 1].starts_with("ssh -p 2222"));
        assert!(a[e + 1].contains("-i \"/home/me/my key\""));
        assert!(a[e + 1].contains("BatchMode=yes"));
        assert_eq!(a[a.len() - 2], "deploy@example.com:/var/www/html/");
        assert_eq!(a[a.len() - 1], "/home/me/Sites/blog/");
        let push = rsync_args(&profile(), Direction::Push, false, local, true);
        assert!(push.contains(&"--delete".to_string()));
        assert!(!push.contains(&"--dry-run".to_string()));
        assert_eq!(push[push.len() - 1], "deploy@example.com:/var/www/html/");
        let mut pw = profile();
        pw.auth = "password".into();
        let opts = ssh_options(&pw).join(" ");
        assert!(opts.contains("PreferredAuthentications=password"));
        assert!(!opts.contains("BatchMode"));
    }

    #[test]
    fn parse_itemize_and_plan_serialization() {
        let out = ">f+++++++++|1200|index.php\n>f.st......|50|wp-config.php\ncd+++++++++|0|wp-content/uploads/\n.d..t......|0|wp-content/\n*deleting|0|old.txt\ngarbage line\n";
        let entries = parse_itemize(out);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0], PlanEntry { path: "index.php".into(), action: "add".into(), size: 1200, is_dir: false });
        assert_eq!(entries[1].action, "update");
        assert!(entries[2].is_dir);
        assert_eq!(entries[3].action, "delete");
        let plan = summarize("prod", Direction::Pull, true, entries, s(&[".git/"]), "rsync");
        assert_eq!((plan.add, plan.update, plan.delete), (1, 1, 1));
        assert_eq!(plan.bytes, 1250);
        let v = serde_json::to_value(&plan).unwrap();
        assert_eq!(v["direction"], "pull");
        assert_eq!(v["dryRun"], true);
        assert_eq!(v["profileId"], "prod");
        assert_eq!(v["entries"][0]["action"], "add");
        assert_eq!(v["entries"][0]["isDir"], false);
        let back: SyncPlan = serde_json::from_value(v).unwrap();
        assert_eq!(back, plan);
    }

    #[test]
    fn profile_roundtrip_with_memory_keychain() {
        let dir = std::env::temp_dir().join(format!("devstack-hosting-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let store = MemoryStore::default();
        let mut p = profile();
        p.auth = "password".into();
        p.id = String::new();
        p.excludes.clear();
        let saved = save_profile(&paths, &store, p, Some("hunter2".into()), Some("dbpw".into())).unwrap();
        assert!(saved.has_password && saved.has_db_password);
        assert_eq!(saved.port, 2222);
        assert_eq!(saved.excludes, default_excludes());
        let raw = fs::read_to_string(profiles_path(&paths)).unwrap();
        assert!(!raw.contains("hunter2") && !raw.contains("dbpw"), "secrets must stay in the keychain");
        assert_eq!(list_profiles(&paths, &store).len(), 1);
        delete_profile(&paths, &store, &saved.id).unwrap();
        assert!(list_profiles(&paths, &store).is_empty());
        assert!(store.get(&ssh_secret_key(&saved.id)).is_none());
        let mut bad = profile();
        bad.remote_path = "relative".into();
        assert!(validate_profile(&bad).is_err());
        bad = profile();
        bad.host = "-oProxyCommand=evil".into();
        assert!(validate_profile(&bad).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_listing_respects_excludes() {
        let dir = std::env::temp_dir().join(format!("devstack-hosting-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join(".git/HEAD"), "x").unwrap();
        fs::write(dir.join("src/a.php"), "<?php").unwrap();
        fs::write(dir.join("debug.log"), "x").unwrap();
        let list = local_push_listing(&dir, &s(&[".git/", "*.log"]));
        assert_eq!(list.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), vec!["src/a.php"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn askpass_helper_reads_password_from_env_only() {
        let dir = std::env::temp_dir().join(format!("devstack-hosting-askpass-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let p = ensure_askpass(&paths).unwrap();
        let body = fs::read_to_string(&p).unwrap();
        assert!(body.contains("DEVSTACK_SSH_PASSWORD"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_db_commands_read_password_from_stdin() {
        let db = RemoteDb { host: "".into(), name: "wp".into(), user: "wp'u".into() };
        let c = remote_dump_cmd(&db);
        assert!(c.starts_with("IFS= read -r MYSQL_PWD; export MYSQL_PWD;"));
        assert!(c.contains("-h '127.0.0.1' -u 'wp'\\''u' 'wp'"));
        assert!(!c.contains("-p"), "no password flag: {c}");
        assert!(remote_import_cmd(&db).contains("gunzip -c | mysql -h"));
    }
}
