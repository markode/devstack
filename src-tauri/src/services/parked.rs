//! Valet-style parked folders: every immediate subfolder of a parked directory
//! becomes a `{slug}.test` site (unless a managed site already owns that path).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{slugify, AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::paths_util::expand_home;
use crate::services::php_runtime::default_php_version;
use crate::services::sites::{detect_site, ensure_site_placeholder};

pub fn slug_for_folder(name: &str) -> String {
    slugify(name)
}

pub fn domain_for_folder(name: &str) -> String {
    format!("{}.test", slug_for_folder(name))
}

/// Immediate subfolders (non-hidden) of each parked dir as `(folder name, path)`.
pub fn scan_parked_dirs(dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for dir in dirs {
        let dir = expand_home(dir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            out.push((name.to_string(), path));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn same_path(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b
}

fn unique_id(taken: &HashSet<String>, base: &str) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken.contains(c))
        .unwrap()
}

/// Add parked sites for new subfolders and drop parked sites whose folder vanished.
/// Returns the number of sites added. Does not touch hosts/vhosts — callers reload.
pub fn sync_parked_sites(_paths: &Paths, state: &mut AppState) -> Result<usize> {
    let dirs = state.sites_settings.parked_dirs.clone();
    let found = scan_parked_dirs(&dirs);

    let before = state.sites.len();
    state
        .sites
        .retain(|s| !(s.parked && !expand_home(&s.path).exists()));
    let removed = before - state.sites.len();

    let mut taken_ids: HashSet<String> = state.sites.iter().map(|s| s.id.clone()).collect();
    let mut taken_domains: HashSet<String> =
        state.sites.iter().map(|s| s.domain.clone()).collect();
    let mut added = 0usize;

    for (name, path) in found {
        if state
            .sites
            .iter()
            .any(|s| same_path(&expand_home(&s.path), &path))
        {
            continue;
        }
        let slug = slug_for_folder(&name);
        let id = unique_id(&taken_ids, &slug);
        let tld = state.developer.default_tld.clone();
        let domain = format!("{id}.{tld}");
        if taken_domains.contains(&domain) {
            continue;
        }
        let mut site = SiteRecord {
            id: id.clone(),
            name: name.clone(),
            path: path.clone(),
            domain: domain.clone(),
            php_version: default_php_version(state),
            node_version: None,
            https: true,
            web_server: Some("nginx".into()),
            framework: None,
            database_engine: state.active_database.clone(),
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: true,
            xdebug: false,
            tld,
            aliases: Vec::new(),
            wildcard: false,
        };
        let details = detect_site(&site);
        site.framework = Some(details.framework);
        site.database_name = details.database_name;
        site.database_user = details.database_user;
        let _ = ensure_site_placeholder(&path);
        taken_ids.insert(id);
        taken_domains.insert(domain);
        state.sites.push(site);
        added += 1;
    }
    if removed > 0 {
        crate::applog::info("parked", format!("removed {removed} parked site(s) with missing folders"));
    }
    if added > 0 {
        crate::applog::info("parked", format!("added {added} parked site(s)"));
    }
    Ok(added)
}

/// Trailing-edge debounce for parked-folder events (never below `perf::MIN_WATCH_DEBOUNCE_MS`).
pub const PARKED_DEBOUNCE_MS: u64 = 500;

/// Watch parked directories; `on_change` fires (debounced) when a subfolder is
/// created/removed/renamed. Dropping `running` to false stops the thread.
pub fn spawn_parked_watcher<F>(
    dirs: Vec<PathBuf>,
    running: Arc<AtomicBool>,
    mut on_change: F,
) -> Result<()>
where
    F: FnMut() + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher: RecommendedWatcher = Watcher::new(
        move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        notify::Config::default(),
    )
    .map_err(|e| DevStackError::Message(format!("parked watcher: {e}")))?;
    let mut watching = 0;
    for dir in &dirs {
        let dir = expand_home(dir);
        if dir.is_dir() && watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
            watching += 1;
        }
    }
    if watching == 0 {
        return Ok(());
    }
    std::thread::spawn(move || {
        let _watcher = watcher;
        let mut pending: Option<Instant> = None;
        while running.load(Ordering::SeqCst) {
            // Sleep exactly until the debounce window closes (or block idle for 1s).
            let wait = match pending {
                Some(t) => Duration::from_millis(PARKED_DEBOUNCE_MS)
                    .saturating_sub(t.elapsed())
                    .max(Duration::from_millis(1)),
                None => Duration::from_secs(1),
            };
            match rx.recv_timeout(wait) {
                Ok(event) => {
                    use notify::EventKind;
                    if matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_)
                    ) {
                        pending = Some(Instant::now());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
            if let Some(t) = pending {
                if t.elapsed() >= Duration::from_millis(PARKED_DEBOUNCE_MS) {
                    pending = None;
                    on_change();
                }
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parked_debounce_not_below_minimum() {
        assert!(PARKED_DEBOUNCE_MS >= crate::services::perf::MIN_WATCH_DEBOUNCE_MS);
    }

    #[test]
    fn slug_and_domain_from_folder_name() {
        assert_eq!(slug_for_folder("My Cool_App"), "my-cool-app");
        assert_eq!(domain_for_folder("My Cool_App"), "my-cool-app.test");
        assert_eq!(domain_for_folder("blog"), "blog.test");
    }

    #[test]
    fn sync_adds_parked_subfolders_and_skips_managed_paths() {
        let root = std::env::temp_dir().join(format!("devstack-parked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let parked = root.join("Sites");
        std::fs::create_dir_all(parked.join("alpha")).unwrap();
        std::fs::create_dir_all(parked.join("Beta Site")).unwrap();
        std::fs::create_dir_all(parked.join(".hidden")).unwrap();
        std::fs::write(parked.join("file.txt"), "x").unwrap();

        let paths = Paths::from_root(root.join("data")).unwrap();
        let mut state = AppState::default();
        state.stack.php_version = Some("8.4".into());
        state.sites_settings.parked_dirs = vec![parked.clone()];
        state.sites.push(SiteRecord {
            id: "manual".into(),
            name: "manual".into(),
            path: parked.join("alpha"),
            domain: "manual.test".into(),
            php_version: None,
            node_version: None,
            https: true,
            web_server: None,
            framework: None,
            database_engine: None,
            database_name: None,
            database_user: None,
            database_password: None,
            status: "stopped".into(),
            is_template: false,
            parked: false,
            xdebug: false,
            tld: crate::config::default_tld(),
            aliases: Vec::new(),
            wildcard: false,
        });

        let added = sync_parked_sites(&paths, &mut state).unwrap();
        assert_eq!(added, 1);
        let beta = state.sites.iter().find(|s| s.parked).unwrap();
        assert_eq!(beta.domain, "beta-site.test");
        assert_eq!(beta.php_version.as_deref(), Some("8.4"));
        assert!(parked
            .join("Beta Site")
            .join("index.html")
            .exists());
        assert_eq!(sync_parked_sites(&paths, &mut state).unwrap(), 0);

        std::fs::remove_dir_all(parked.join("Beta Site")).unwrap();
        sync_parked_sites(&paths, &mut state).unwrap();
        assert!(!state.sites.iter().any(|s| s.parked));
        let _ = std::fs::remove_dir_all(&root);
    }
}
