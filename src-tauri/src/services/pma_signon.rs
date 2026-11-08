//! One-time phpMyAdmin sign-on tokens (no DB password in the URL).

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::config::{Paths, AppState};
use crate::error::{DevStackError, Result};
use crate::paths_util::conf_path;

const TTL_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PmaTokenPayload {
    pub user: String,
    pub pass: String,
    pub db: String,
    pub host: String,
    pub port: u16,
    pub created_at: u64,
}

fn tokens_dir(paths: &Paths) -> PathBuf {
    paths.root.join("pma-tokens")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn random_token() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Drop expired token files (best-effort).
pub fn cleanup_expired_tokens(paths: &Paths) {
    let dir = tokens_dir(paths);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let now = now_secs();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let stale = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<PmaTokenPayload>(&raw).ok())
            .map(|p| now.saturating_sub(p.created_at) > TTL_SECS)
            .unwrap_or(true);
        if stale {
            let _ = fs::remove_file(path);
        }
    }
}

/// Mint a short-lived token and return the sign-on URL (no password query param).
pub fn mint_signon_url(
    paths: &Paths,
    state: &AppState,
    db: &str,
    user: &str,
    pass: &str,
) -> Result<String> {
    cleanup_expired_tokens(paths);
    let engine = state
        .active_database
        .clone()
        .or_else(|| state.runtime.database_engine.clone())
        .unwrap_or_else(|| "mariadb".into());
    let port = crate::services::ports::engine_db_port(&state.stack, &engine);
    let host = if cfg!(windows) {
        "127.0.0.1".to_string()
    } else {
        "localhost".to_string()
    };
    let token = random_token();
    let dir = tokens_dir(paths);
    fs::create_dir_all(&dir)?;
    let payload = PmaTokenPayload {
        user: user.to_string(),
        pass: pass.to_string(),
        db: db.to_string(),
        host,
        port,
        created_at: now_secs(),
    };
    fs::write(
        dir.join(format!("{token}.json")),
        serde_json::to_string(&payload)?,
    )?;
    let base = crate::services::ports::format_url(
        "https",
        "phpmyadmin.test",
        state.stack.https_port,
    );
    Ok(format!("{base}/devstack-signon.php?token={token}"))
}

/// Absolute forward-slash path injected into `devstack-signon.php`.
pub fn tokens_dir_conf(paths: &Paths) -> String {
    conf_path(&tokens_dir(paths))
}

pub fn ensure_tokens_dir(paths: &Paths) -> Result<()> {
    fs::create_dir_all(tokens_dir(paths)).map_err(|e| {
        DevStackError::Message(format!("could not create pma-tokens dir: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_writes_token_without_password_in_url() {
        let dir = std::env::temp_dir().join(format!("devstack-pma-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let state = AppState::default();
        let url = mint_signon_url(&paths, &state, "wp_demo", "root", "s3cret").unwrap();
        assert!(url.contains("token="), "{url}");
        assert!(!url.contains("pass="), "{url}");
        assert!(!url.contains("s3cret"), "{url}");
        let token = url.split("token=").nth(1).unwrap();
        let file = tokens_dir(&paths).join(format!("{token}.json"));
        let raw = fs::read_to_string(&file).unwrap();
        assert!(raw.contains("s3cret"));
        assert!(raw.contains("wp_demo"));
        let _ = fs::remove_dir_all(&dir);
    }
}
