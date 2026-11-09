//! Startup helpers: phase logging, main-thread stall watchdog, orphan reclaim.

use std::time::{Duration, Instant};

use crate::applog;
use crate::config::Paths;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static SETUP_DONE: AtomicBool = AtomicBool::new(false);
static LAST_HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Log a named startup phase (always, so hangs are diagnosable from applog).
pub fn phase(name: &str, detail: impl AsRef<str>) {
    applog::info("startup", format!("phase={name} {}", detail.as_ref()));
    eprintln!("[devstack-startup] {name}: {}", detail.as_ref());
    LAST_HEARTBEAT_MS.store(now_ms(), Ordering::SeqCst);
}

pub fn mark_setup_complete() {
    SETUP_DONE.store(true, Ordering::SeqCst);
    phase("setup_complete", "main window event loop ready");
}

/// Pulse so the watchdog knows the process is making progress.
pub fn heartbeat() {
    LAST_HEARTBEAT_MS.store(now_ms(), Ordering::SeqCst);
}

/// Watchdog: if heartbeats stop for >2s after setup begins, log a stall warning.
/// Logging only — does not kill the process (AppHangB1 diagnosis aid).
pub fn spawn_watchdog() {
    LAST_HEARTBEAT_MS.store(now_ms(), Ordering::SeqCst);
    std::thread::Builder::new()
        .name("devstack-startup-watchdog".into())
        .spawn(|| {
            let mut stalled = false;
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let last = LAST_HEARTBEAT_MS.load(Ordering::SeqCst);
                let age = now_ms().saturating_sub(last);
                if age > 2000 {
                    if !stalled {
                        stalled = true;
                        applog::warn(
                            "startup",
                            format!(
                                "main-thread stall detected ({}ms since last heartbeat; setup_done={})",
                                age,
                                SETUP_DONE.load(Ordering::SeqCst)
                            ),
                        );
                        eprintln!(
                            "[devstack-startup] STALL {}ms (setup_done={})",
                            age,
                            SETUP_DONE.load(Ordering::SeqCst)
                        );
                    }
                } else {
                    stalled = false;
                }
                if SETUP_DONE.load(Ordering::SeqCst) && age < 2000 {
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        })
        .ok();
}

/// Reclaim orphaned DevStack service processes from older versions that still
/// hold our data-dir paths (mysqld/httpd/nginx/php-cgi). Only kills processes
/// whose command line clearly references `paths.root`; never touches foreign WAMP.
pub fn reclaim_orphans(paths: &Paths) {
    let root = paths.root.to_string_lossy().to_string();
    if root.is_empty() {
        return;
    }
    phase("orphans", format!("scanning for leftovers under {root}"));
    #[cfg(windows)]
    {
        reclaim_orphans_windows(&root);
    }
    #[cfg(not(windows))]
    {
        let _ = root;
    }
}

#[cfg(windows)]
fn reclaim_orphans_windows(root: &str) {
    use crate::services::process::{is_running, stop_pid};
    let root_l = root.replace('/', "\\").to_ascii_lowercase();
    let out = match crate::services::process::command("wmic")
        .args([
            "process",
            "where",
            "name='mysqld.exe' or name='mariadbd.exe' or name='httpd.exe' or name='nginx.exe' or name='php-cgi.exe'",
            "get",
            "ProcessId,CommandLine",
            "/FORMAT:CSV",
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            applog::warn("startup", format!("orphan scan failed: {e}"));
            return;
        }
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut killed = 0usize;
    for line in text.lines().skip(1) {
        let line_l = line.to_ascii_lowercase();
        if !line_l.contains(&root_l) {
            continue;
        }
        // CSV: Node,CommandLine,ProcessId
        let pid = line
            .rsplit(',')
            .next()
            .and_then(|s| s.trim().parse::<u32>().ok());
        let Some(pid) = pid else { continue };
        if !is_running(pid) {
            continue;
        }
        applog::info(
            "startup",
            format!("reclaiming orphan pid={pid} (cmdline references data dir)"),
        );
        let _ = stop_pid(pid);
        killed += 1;
    }
    phase("orphans", format!("reclaimed {killed} process(es)"));
}

/// Run `f` on a background thread with a wall-clock timeout. Returns `None` on timeout.
#[allow(dead_code)]
pub fn spawn_timeout<T: Send + 'static>(
    name: &'static str,
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    std::thread::Builder::new()
        .name(format!("devstack-{name}"))
        .spawn(move || {
            let _ = tx.send(f());
        })
        .ok()?;
    match rx.recv_timeout(timeout) {
        Ok(v) => {
            phase(name, format!("ok in {}ms", started.elapsed().as_millis()));
            Some(v)
        }
        Err(_) => {
            applog::warn(
                "startup",
                format!(
                    "{name} still running after {}ms — continuing without it",
                    timeout.as_millis()
                ),
            );
            None
        }
    }
}

pub fn setup_done() -> bool {
    SETUP_DONE.load(Ordering::SeqCst)
}

/// Keep heartbeats alive after setup so stall detection remains useful.
pub fn arm_ui_heartbeat(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("devstack-ui-heartbeat".into())
        .spawn(move || {
            use tauri::Emitter;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                heartbeat();
                let _ = app.emit("devstack-heartbeat", now_ms());
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_updates_heartbeat() {
        let before = LAST_HEARTBEAT_MS.load(Ordering::SeqCst);
        phase("test", "unit");
        let after = LAST_HEARTBEAT_MS.load(Ordering::SeqCst);
        assert!(after >= before);
    }
}
