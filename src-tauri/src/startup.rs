//! Startup helpers: phase logging, main-thread stall watchdog, orphan reclaim.

use std::time::{Duration, Instant};

use crate::applog;
use crate::config::Paths;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static SETUP_DONE: AtomicBool = AtomicBool::new(false);
static LAST_HEARTBEAT_MS: AtomicU64 = AtomicU64::new(0);

/// Windows service process names that may be left behind from prior DevStack runs.
pub const ORPHAN_PROCESS_NAMES: &[&str] = &[
    "nginx.exe",
    "httpd.exe",
    "php-cgi.exe",
    "mysqld.exe",
    "mariadbd.exe",
];

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
/// whose executable path or command line clearly references `paths.root`;
/// never touches foreign WAMP.
pub fn reclaim_orphans(paths: &Paths) {
    reclaim_orphans_for_names(paths, ORPHAN_PROCESS_NAMES, &[]);
}

/// Like [`reclaim_orphans`], but limited to the given image names (e.g. `nginx.exe`).
///
/// `keep_pids` are never killed — pass live runtime PIDs so warm `apply_stack`
/// does not tear down MariaDB/PHP that `stop_web_stack` intentionally left up.
pub fn reclaim_orphans_for_names(paths: &Paths, names: &[&str], keep_pids: &[u32]) {
    let root = paths.root.to_string_lossy().to_string();
    if root.is_empty() || names.is_empty() {
        return;
    }
    phase(
        "orphans",
        format!("scanning for leftovers under {root} ({})", names.join(",")),
    );
    #[cfg(windows)]
    {
        reclaim_orphans_windows(&root, names, keep_pids);
    }
    #[cfg(not(windows))]
    {
        let _ = (root, names, keep_pids);
    }
}

/// Normalize a Windows/Unix path for case-insensitive substring matching.
pub fn normalize_root_marker(root: &str) -> String {
    root.replace('/', "\\").to_ascii_lowercase()
}

/// Pure helper: given scan text + data-root, return PIDs whose ExecutablePath
/// (preferred) or CommandLine contains the DevStack data root.
///
/// Accepted line formats (header lines ignored when they contain `ProcessId`):
/// - TSV: `pid<TAB>name<TAB>executablePath<TAB>commandLine`
/// - CSV: `ProcessId,Name,ExecutablePath,CommandLine` (quoted fields ok; cmdline
///   matching falls back to whole-line contains when CSV splits are ambiguous)
pub fn orphan_pids_from_scan_text(text: &str, root: &str) -> Vec<u32> {
    let root_l = normalize_root_marker(root);
    if root_l.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.contains("processid") && (lower.contains("executablepath") || lower.contains("commandline"))
        {
            continue;
        }
        if let Some(pid) = orphan_pid_from_scan_line(line, &root_l) {
            if !out.contains(&pid) {
                out.push(pid);
            }
        }
    }
    out
}

fn orphan_pid_from_scan_line(line: &str, root_l: &str) -> Option<u32> {
    // TSV first — PowerShell emit format (avoids comma-in-CommandLine issues).
    if line.contains('\t') {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() >= 1 {
            let pid = cols[0].trim().trim_matches('"').parse::<u32>().ok()?;
            let exe = cols.get(2).map(|s| s.trim().trim_matches('"')).unwrap_or("");
            let cmd = if cols.len() > 3 {
                cols[3..].join("\t")
            } else {
                String::new()
            };
            let cmd = cmd.trim().trim_matches('"');
            if path_or_cmd_matches_root(exe, cmd, root_l) {
                return Some(pid);
            }
        }
        return None;
    }

    // CSV: ProcessId,Name,ExecutablePath,CommandLine — ProcessId is first.
    let cols = parse_csv_loose(line);
    if cols.len() >= 3 {
        if let Ok(pid) = cols[0].trim().parse::<u32>() {
            let exe = cols.get(2).map(|s| s.as_str()).unwrap_or("");
            let cmd = if cols.len() > 3 {
                cols[3..].join(",")
            } else {
                String::new()
            };
            if path_or_cmd_matches_root(exe, &cmd, root_l) {
                return Some(pid);
            }
            // Ambiguous CSV: also accept whole-line match with trailing PID form
            // (legacy wmic: Node,CommandLine,ProcessId).
        }
    }
    // Legacy wmic CSV: ...,CommandLine,ProcessId
    if let Some(pid_s) = line.rsplit(',').next() {
        if let Ok(pid) = pid_s.trim().trim_matches('"').parse::<u32>() {
            let line_l = normalize_root_marker(line);
            if line_l.contains(root_l) {
                return Some(pid);
            }
        }
    }
    None
}

fn path_or_cmd_matches_root(exe: &str, cmd: &str, root_l: &str) -> bool {
    let exe_l = normalize_root_marker(exe);
    if !exe_l.is_empty() && exe_l.contains(root_l) {
        return true;
    }
    // Prefer ExecutablePath; only fall back to CommandLine when exe is empty
    // or did not match — still require an explicit root reference.
    let cmd_l = normalize_root_marker(cmd);
    !cmd_l.is_empty() && cmd_l.contains(root_l)
}

fn parse_csv_loose(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

#[cfg(windows)]
fn reclaim_orphans_windows(root: &str, names: &[&str], keep_pids: &[u32]) {
    use crate::services::process::{is_running, stop_pid};

    let text = match scan_windows_processes(names) {
        Ok(t) => t,
        Err(e) => {
            applog::warn("startup", format!("orphan scan failed: {e}"));
            return;
        }
    };
    let pids = orphan_pids_from_scan_text(&text, root);
    let mut killed = 0usize;
    for pid in pids {
        if keep_pids.contains(&pid) {
            continue;
        }
        if !is_running(pid) {
            continue;
        }
        applog::info(
            "startup",
            format!("reclaiming orphan pid={pid} (path/cmdline references data dir)"),
        );
        let _ = stop_pid(pid);
        killed += 1;
    }
    phase("orphans", format!("reclaimed {killed} process(es)"));
}

#[cfg(windows)]
fn scan_windows_processes(names: &[&str]) -> std::result::Result<String, String> {
    // Prefer Get-CimInstance (wmic is removed on modern Windows 11).
    let name_list = names
        .iter()
        .map(|n| format!("'{}'", n.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$names = @({name_list}); \
         Get-CimInstance Win32_Process | \
         Where-Object {{ $names -contains $_.Name }} | \
         ForEach-Object {{ \
           $exe = if ($null -ne $_.ExecutablePath) {{ $_.ExecutablePath }} else {{ '' }}; \
           $cmd = if ($null -ne $_.CommandLine) {{ ($_.CommandLine -replace \"[`r`n`t]\",' ') }} else {{ '' }}; \
           \"{{0}}`t{{1}}`t{{2}}`t{{3}}\" -f $_.ProcessId, $_.Name, $exe, $cmd \
         }}"
    );
    let out = crate::services::process::command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .output()
        .map_err(|e| format!("powershell: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    // Fallback: tasklist CSV + per-PID ExecutablePath via Get-CimInstance.
    applog::warn(
        "startup",
        format!("Get-CimInstance bulk scan failed ({err}); trying tasklist fallback"),
    );
    scan_windows_processes_tasklist_fallback(names)
}

#[cfg(windows)]
fn scan_windows_processes_tasklist_fallback(names: &[&str]) -> std::result::Result<String, String> {
    let mut rows = String::new();
    for name in names {
        let out = crate::services::process::command("tasklist")
            .args(["/FI", &format!("IMAGENAME eq {name}"), "/FO", "CSV", "/NH"])
            .output()
            .map_err(|e| format!("tasklist: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("INFO:") {
                continue;
            }
            let cols = parse_csv_loose(line);
            // "nginx.exe","1234","Session","0","12,345 K"
            if cols.len() < 2 {
                continue;
            }
            let proc_name = cols[0].trim().to_string();
            let pid: u32 = match cols[1].trim().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let (exe, cmd) = query_windows_process_paths(pid).unwrap_or_default();
            rows.push_str(&format!("{pid}\t{proc_name}\t{exe}\t{cmd}\n"));
        }
    }
    Ok(rows)
}

#[cfg(windows)]
fn query_windows_process_paths(pid: u32) -> Option<(String, String)> {
    let script = format!(
        "$p = Get-CimInstance Win32_Process -Filter \"ProcessId={pid}\"; \
         if ($null -eq $p) {{ return }}; \
         $exe = if ($null -ne $p.ExecutablePath) {{ $p.ExecutablePath }} else {{ '' }}; \
         $cmd = if ($null -ne $p.CommandLine) {{ ($p.CommandLine -replace \"[`r`n`t]\",' ') }} else {{ '' }}; \
         \"{{0}}`t{{1}}\" -f $exe, $cmd"
    );
    let out = crate::services::process::command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        return Some((String::new(), String::new()));
    }
    let mut parts = line.splitn(2, '\t');
    let exe = parts.next().unwrap_or("").to_string();
    let cmd = parts.next().unwrap_or("").to_string();
    Some((exe, cmd))
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

    #[test]
    fn orphan_pids_from_tsv_matches_executable_path() {
        let root = r"C:\Users\me\AppData\Local\DevStack";
        let text = "\
1234\tnginx.exe\tC:\\Users\\me\\AppData\\Local\\DevStack\\packages\\nginx\\nginx.exe\tnginx -c conf
5678\thttpd.exe\tC:\\wamp64\\bin\\apache\\httpd.exe\thttpd.exe -f C:\\wamp64\\httpd.conf
";
        let pids = orphan_pids_from_scan_text(text, root);
        assert_eq!(pids, vec![1234]);
    }

    #[test]
    fn orphan_pids_from_tsv_falls_back_to_command_line() {
        let root = r"C:\Users\me\AppData\Local\DevStack";
        let text =
            "999\tmysqld.exe\t\t\"C:\\Users\\me\\AppData\\Local\\DevStack\\packages\\mysql\\mysqld.exe\" --datadir=...\n";
        let pids = orphan_pids_from_scan_text(text, root);
        assert_eq!(pids, vec![999]);
    }

    #[test]
    fn orphan_pids_ignores_wamp_and_empty_root() {
        let root = r"C:\Users\me\AppData\Local\DevStack";
        let text = "1\thttpd.exe\tC:\\wamp64\\bin\\apache\\httpd.exe\thttpd\n";
        assert!(orphan_pids_from_scan_text(text, root).is_empty());
        assert!(orphan_pids_from_scan_text(text, "").is_empty());
    }

    #[test]
    fn orphan_pids_legacy_wmic_csv() {
        let root = r"C:\Users\me\AppData\Local\DevStack";
        let text = "Node,CommandLine,ProcessId\n\
MYPC,\"C:\\Users\\me\\AppData\\Local\\DevStack\\packages\\nginx\\nginx.exe\" -p prefix,4242\n";
        assert_eq!(orphan_pids_from_scan_text(text, root), vec![4242]);
    }

    #[test]
    fn orphan_pids_csv_with_header() {
        let root = r"C:\DevStack";
        let text = "\"ProcessId\",\"Name\",\"ExecutablePath\",\"CommandLine\"\n\
\"100\",\"nginx.exe\",\"C:\\DevStack\\nginx\\nginx.exe\",\"nginx -c conf\"\n";
        assert_eq!(orphan_pids_from_scan_text(text, root), vec![100]);
    }
}
