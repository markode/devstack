//! `.test` domain resolution + privileged helper (hosts / resolver / CA trust).
//!
//! Windows writes `%SystemRoot%\System32\drivers\etc\hosts` (elevated via UAC
//! when needed). macOS prefers passwordless `sudo -n` (CI / admin shells), then
//! falls back to osascript administrator privileges. Linux keeps `sudo -n`.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::{AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::process::{is_running, read_pid, spawn_logged, stop_pid, write_pid};

/// Serialize privileged helper invokes — concurrent `sudo -n` / keychain access
/// has hung macos-smoke mid-apply (ca/hosts never logged) for the full job timeout.
fn helper_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Spawn + wait with a hard wall-clock timeout (kill on expiry).
#[cfg(unix)]
pub(crate) fn run_command_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("spawn: {e}")))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|e| DevStackError::Message(format!("wait: {e}")));
            }
            Ok(None) if start.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(DevStackError::Message(format!(
                    "command timed out after {}s",
                    timeout.as_secs()
                )));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(DevStackError::Message(format!("wait: {e}")));
            }
        }
    }
}

pub const MARKER_BEGIN: &str = "# BEGIN DevStack";
pub const MARKER_END: &str = "# END DevStack";

const HELPER_SCRIPT: &str = r##"#!/usr/bin/env bash
# DevStack privileged helper — isolated surface for hosts/resolver/CA.
set -euo pipefail
cmd="${1:-}"
shift || true
case "$cmd" in
  hosts-sync)
    marker_begin="# BEGIN DevStack"
    marker_end="# END DevStack"
    hosts_file="/etc/hosts"
    tmp="$(mktemp)"
    awk -v b="$marker_begin" -v e="$marker_end" '
      $0==b {skip=1; next}
      $0==e {skip=0; next}
      !skip {print}
    ' "$hosts_file" > "$tmp"
    {
      echo "$marker_begin"
      for d in "$@"; do
        echo "127.0.0.1 $d"
        echo "::1 $d"
      done
      echo "$marker_end"
    } >> "$tmp"
    cat "$tmp" > "$hosts_file"
    rm -f "$tmp"
    ;;
  resolver-macos)
    mkdir -p /etc/resolver
    printf "nameserver 127.0.0.1\nport %s\n" "${1:-5353}" > /etc/resolver/test
    ;;
  trust-ca)
    ca="${1:-}"
    if [[ -z "$ca" || ! -f "$ca" ]]; then
      echo "missing ca" >&2; exit 1
    fi
    if [[ "$(uname -s)" == "Darwin" ]]; then
      security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain "$ca"
    elif [[ -d /usr/local/share/ca-certificates ]]; then
      cp "$ca" /usr/local/share/ca-certificates/devstack-root.crt
      update-ca-certificates || true
    fi
    ;;
  *)
    echo "unknown helper command: $cmd" >&2
    exit 2
    ;;
esac
"##;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostsStatus {
    pub ok: bool,
    pub missing: Vec<String>,
    pub message: Option<String>,
    /// True only when the DevStack CA is verified in the OS trust store.
    pub ca_trusted: bool,
    /// Set when HTTPS will warn because the CA is not trusted (includes Retry hint).
    pub ca_message: Option<String>,
}

/// Build the DevStack hosts block (IPv4 + IPv6) for the given domains.
pub fn build_hosts_block(domains: &[String]) -> String {
    let mut lines = vec![MARKER_BEGIN.to_string()];
    for d in domains {
        let d = d.trim();
        if d.is_empty() {
            continue;
        }
        lines.push(format!("127.0.0.1 {d}"));
        lines.push(format!("::1 {d}"));
    }
    lines.push(MARKER_END.to_string());
    lines.join("\n") + "\n"
}

/// Merge a DevStack block into existing hosts file contents.
pub fn merge_hosts_contents(existing: &str, domains: &[String]) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in existing.lines() {
        if line.trim() == MARKER_BEGIN {
            skipping = true;
            continue;
        }
        if line.trim() == MARKER_END {
            skipping = false;
            continue;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&build_hosts_block(domains));
    out
}

pub fn site_domains(sites: &[SiteRecord]) -> Vec<String> {
    let mut d: Vec<_> = sites.iter().flat_map(|s| s.hostnames()).collect();
    d.push("phpmyadmin.test".into());
    d.push("mailpit.test".into());
    d.sort();
    d.dedup();
    d
}

pub fn windows_hosts_path() -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    PathBuf::from(root)
        .join("System32")
        .join("drivers")
        .join("etc")
        .join("hosts")
}

/// Platform hosts file path (`System32\...\hosts` on Windows, `/etc/hosts` elsewhere).
pub fn system_hosts_path() -> PathBuf {
    #[cfg(windows)]
    {
        windows_hosts_path()
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/hosts")
    }
}

/// Extract the DevStack-managed block from hosts file contents (inclusive markers).
pub fn extract_devstack_block(contents: &str) -> Option<String> {
    let mut out = String::new();
    let mut in_block = false;
    let mut found = false;
    for line in contents.lines() {
        let t = line.trim();
        if t == MARKER_BEGIN {
            in_block = true;
            found = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if t == MARKER_END {
            if in_block {
                out.push_str(line);
                out.push('\n');
            }
            in_block = false;
            continue;
        }
        if in_block {
            out.push_str(line);
            out.push('\n');
        }
    }
    found.then_some(out)
}

/// Domains that are missing a `127.0.0.1` entry inside the DevStack hosts block.
pub fn domains_missing_from_hosts(contents: &str, domains: &[String]) -> Vec<String> {
    let block = extract_devstack_block(contents).unwrap_or_default();
    let lower = block.to_ascii_lowercase();
    domains
        .iter()
        .filter(|d| {
            let needle = format!("127.0.0.1 {}", d.trim().to_ascii_lowercase());
            !lower.lines().any(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") == needle
                || l.contains(&needle))
        })
        .cloned()
        .collect()
}

/// True when every domain has both IPv4 and IPv6 lines in the DevStack block.
pub fn hosts_block_complete(contents: &str, domains: &[String]) -> bool {
    let Some(block) = extract_devstack_block(contents) else {
        return domains.is_empty();
    };
    let lower = block.to_ascii_lowercase();
    domains.iter().all(|d| {
        let d = d.trim().to_ascii_lowercase();
        if d.is_empty() {
            return true;
        }
        let v4 = format!("127.0.0.1 {d}");
        let v6 = format!("::1 {d}");
        lower.lines().any(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") == v4)
            && lower
                .lines()
                .any(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") == v6)
    })
}

pub fn ensure_helper_script(paths: &Paths) -> Result<PathBuf> {
    let path = paths.helper.join("devstack-helper.sh");
    fs::write(&path, HELPER_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

pub fn helper_run(paths: &Paths, args: &[&str]) -> Result<()> {
    if args.is_empty() {
        return Err(DevStackError::Message("helper: missing command".into()));
    }
    let _guard = helper_lock();
    #[cfg(windows)]
    {
        return helper_run_windows(paths, args);
    }
    #[cfg(target_os = "macos")]
    {
        return helper_run_macos(paths, args);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return helper_run_linux(paths, args);
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = paths;
        Err(DevStackError::Message("helper unsupported on this OS".into()))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn helper_run_linux(paths: &Paths, args: &[&str]) -> Result<()> {
    let script = ensure_helper_script(paths)?;
    let mut cmd = crate::services::process::command("sudo");
    cmd.arg("-n").arg(&script).args(args);
    let status = run_command_with_timeout(cmd, Duration::from_secs(45));
    match status {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(DevStackError::Message(format!(
                "privileged helper failed (exit {}). {stderr}",
                out.status
            )))
        }
        Err(e) => Err(DevStackError::Message(format!("helper invoke: {e}"))),
    }
}

#[cfg(target_os = "macos")]
fn helper_run_macos(paths: &Paths, args: &[&str]) -> Result<()> {
    let script = ensure_helper_script(paths)?;
    // Prefer passwordless sudo (GitHub Actions / admin shells) — same path as Linux.
    // Never fall through to osascript under CI / non-interactive sessions: the
    // "with administrator privileges" dialog blocks forever with no UI, which is
    // exactly what hung macos-smoke for 35+ minutes after the first stack stop.
    // Also hard-timeout sudo itself — concurrent sudo/keychain has wedged CI
    // with no stderr for the full 40m job limit.
    let mut sudo_cmd = crate::services::process::command("sudo");
    sudo_cmd.arg("-n").arg(&script).args(args);
    let sudo = run_command_with_timeout(sudo_cmd, Duration::from_secs(45));
    match &sudo {
        Ok(out) if out.status.success() => return Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!(
                "devstack helper: sudo -n failed for {:?} (exit {}): {stderr}",
                args, out.status
            );
        }
        Err(e) => eprintln!("devstack helper: sudo -n failed for {:?}: {e}", args),
    }

    if macos_skip_osascript() {
        let detail = match &sudo {
            Ok(out) => format!(
                "sudo -n exit {}; {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => format!("sudo -n error: {e}"),
        };
        return Err(DevStackError::Message(format!(
            "macOS privileged helper unavailable in CI/headless ({detail}). \
             Grant passwordless sudo for the DevStack helper or pre-install hosts/resolver/CA."
        )));
    }

    // Interactive desktop only: GUI elevation when sudo needs a password.
    let apple = macos_osascript_helper_command(&script, args);
    let output = run_osascript_with_timeout(&apple, Duration::from_secs(30))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(DevStackError::Message(format!(
            "macOS privileged helper failed: {stderr}"
        )))
    }
}

/// CI and other non-interactive sessions must not invoke osascript — it waits
/// forever on an admin dialog that never appears on GitHub-hosted runners.
#[cfg(target_os = "macos")]
fn macos_skip_osascript() -> bool {
    if std::env::var_os("CI").is_some() || std::env::var_os("DEVSTACK_NO_OSA").is_some() {
        return true;
    }
    // No TTY → headless / launchd agent; osascript auth cannot succeed.
    use std::io::IsTerminal;
    !std::io::stdin().is_terminal()
}

/// Run `osascript -e` with a hard wall-clock timeout so a stuck auth dialog
/// cannot freeze apply_stack / start_dns for the entire CI job.
#[cfg(target_os = "macos")]
fn run_osascript_with_timeout(
    apple: &str,
    timeout: Duration,
) -> Result<std::process::Output> {
    let mut child = crate::services::process::command("osascript")
        .args(["-e", apple])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DevStackError::Message(format!("osascript helper: {e}")))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|e| DevStackError::Message(format!("osascript helper: {e}")));
            }
            Ok(None) if start.elapsed() < timeout => {
                thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(DevStackError::Message(format!(
                    "osascript helper timed out after {}s waiting for administrator privileges",
                    timeout.as_secs()
                )));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(DevStackError::Message(format!("osascript helper: {e}")));
            }
        }
    }
}

/// Build `do shell script "…" with administrator privileges` with correct quoting.
/// Outer `"` are AppleScript string delimiters; inner `'` quote for `/bin/sh`.
#[cfg(any(test, target_os = "macos"))]
fn macos_osascript_helper_command(script: &Path, args: &[&str]) -> String {
    let mut shell_cmd = shell_single_quote(&script.display().to_string());
    for a in args {
        shell_cmd.push(' ');
        shell_cmd.push_str(&shell_single_quote(a));
    }
    let escaped = shell_cmd.replace('\\', "\\\\").replace('"', "\\\"");
    format!("do shell script \"{escaped}\" with administrator privileges")
}

#[cfg(any(test, target_os = "macos", windows))]
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(windows)]
fn helper_run_windows(paths: &Paths, args: &[&str]) -> Result<()> {
    match args[0] {
        "hosts-sync" => {
            let domains: Vec<String> = args[1..].iter().map(|s| (*s).to_string()).collect();
            sync_hosts_windows(paths, &domains)
        }
        "trust-ca" => {
            let ca = args
                .get(1)
                .ok_or_else(|| DevStackError::Message("trust-ca missing path".into()))?;
            trust_ca_windows(paths, Path::new(ca))
        }
        other => Err(DevStackError::Message(format!(
            "unsupported Windows helper command: {other}"
        ))),
    }
}

#[cfg(windows)]
fn try_write_hosts_direct(hosts_path: &Path, merged: &str) -> Result<bool> {
    match fs::write(hosts_path, merged) {
        Ok(()) => {
            let _ = crate::services::process::command("ipconfig")
                .args(["/flushdns"])
                .status();
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(false),
        Err(e) => Err(DevStackError::Message(format!(
            "could not write hosts file {}: {e}",
            hosts_path.display()
        ))),
    }
}

#[cfg(windows)]
fn try_trust_ca_direct(ca: &Path) -> bool {
    let ca_disp = ca.display().to_string();
    crate::services::process::command("certutil")
        .args(["-addstore", "-f", "Root", &ca_disp])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn windows_hosts_copy_ps(tmp: &Path, hosts_path: &Path) -> String {
    format!(
        r#"
$hostsPath = {hosts}
$tmpPath = {tmp}
if (Test-Path -LiteralPath $hostsPath) {{
  attrib -r $hostsPath 2>$null | Out-Null
}}
try {{
  Copy-Item -LiteralPath $tmpPath -Destination $hostsPath -Force
}} catch {{
  throw ("Hosts write blocked (read-only attribute, antivirus HostsFileHijack, or ACL). " + $_.Exception.Message)
}}
$after = Get-Content -LiteralPath $hostsPath -Raw -ErrorAction Stop
if ($after -notmatch '# BEGIN DevStack') {{
  throw 'Hosts write appeared to succeed but the DevStack block is missing — antivirus may have blocked the change (HostsFileHijack).'
}}
ipconfig /flushdns | Out-Null
Write-DevStackLog 'hosts written and DNS flushed'
"#,
        hosts = ps_quote(&hosts_path.display().to_string()),
        tmp = ps_quote(&tmp.display().to_string()),
    )
}

#[cfg(windows)]
fn sync_hosts_windows(paths: &Paths, domains: &[String]) -> Result<()> {
    let hosts_path = windows_hosts_path();
    let existing = fs::read_to_string(&hosts_path).unwrap_or_default();
    let merged = merge_hosts_contents(&existing, domains);
    if try_write_hosts_direct(&hosts_path, &merged)? {
        verify_hosts_contain(&hosts_path, domains)?;
        return Ok(());
    }
    let tmp = paths.helper.join("hosts.devstack.tmp");
    fs::create_dir_all(&paths.helper)?;
    fs::write(&tmp, &merged)?;
    run_elevated_powershell(paths, &windows_hosts_copy_ps(&tmp, &hosts_path))?;
    let _ = fs::remove_file(&tmp);
    verify_hosts_contain(&hosts_path, domains)?;
    Ok(())
}

#[cfg(windows)]
fn trust_ca_windows(paths: &Paths, ca: &Path) -> Result<()> {
    if !ca.exists() {
        return Err(DevStackError::Message(format!(
            "CA certificate missing: {}",
            ca.display()
        )));
    }
    if crate::services::tls::is_ca_trusted(paths) {
        return Ok(());
    }
    if crate::services::tls::is_ca_trust_deferred(paths) {
        // User declined / timed out / CI skipped — do not re-prompt until Retry.
        return Ok(());
    }
    if try_trust_ca_direct(ca) {
        return finalize_ca_trust(paths);
    }
    if windows_skip_uac() {
        // CI/headless: never fake trust, never block on RunAs. Defer until Retry
        // (or an interactive session) so warm apply_stack stays within budget.
        eprintln!("devstack: skipping elevated CA trust under CI/headless (not marking trusted)");
        crate::services::tls::mark_ca_trust_deferred(paths)?;
        return Ok(());
    }
    let ps = format!(
        "certutil -addstore -f Root {}\nWrite-DevStackLog 'CA trusted'\n",
        ps_quote(&ca.display().to_string())
    );
    match run_elevated_powershell(paths, &ps) {
        Ok(()) => finalize_ca_trust(paths),
        Err(e) => {
            let msg = e.to_string();
            if uac_declined_or_timed_out(&msg) {
                crate::services::tls::mark_ca_trust_deferred(paths)?;
                return Ok(());
            }
            Err(e)
        }
    }
}

/// Mark trusted only after the CA is actually present in the cert store.
#[cfg(windows)]
fn finalize_ca_trust(paths: &Paths) -> Result<()> {
    if !crate::services::tls::is_ca_in_trust_store(paths) {
        crate::services::tls::mark_ca_trust_deferred(paths)?;
        return Err(DevStackError::Message(
            crate::services::tls::ca_untrusted_message().into(),
        ));
    }
    crate::services::tls::mark_ca_trusted(paths)?;
    Ok(())
}

fn uac_declined_or_timed_out(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("canceled by the user")
        || m.contains("cancelled by the user")
        || m.contains("uac cancelled")
        || m.contains("timed out")
        || m.contains("uac elevation unavailable")
}

/// Write hosts + trust the DevStack CA in a **single** UAC prompt on Windows.
#[cfg(windows)]
pub fn sync_hosts_and_trust_ca_windows(
    paths: &Paths,
    domains: &[String],
    ca: &Path,
) -> Result<()> {
    if !ca.exists() {
        return Err(DevStackError::Message(format!(
            "CA certificate missing: {}",
            ca.display()
        )));
    }
    let hosts_path = windows_hosts_path();
    let existing = fs::read_to_string(&hosts_path).unwrap_or_default();
    let merged = merge_hosts_contents(&existing, domains);
    // Skip rewriting hosts when the DevStack block is already complete.
    let hosts_complete = domains_missing_from_hosts(&existing, domains).is_empty();
    let hosts_ok = if hosts_complete {
        true
    } else {
        try_write_hosts_direct(&hosts_path, &merged)?
    };
    let ca_trusted = crate::services::tls::is_ca_trusted(paths);
    let ca_deferred = crate::services::tls::is_ca_trust_deferred(paths);
    let mut ca_ok = ca_trusted;
    if !ca_ok && !ca_deferred {
        if try_trust_ca_direct(ca) {
            match finalize_ca_trust(paths) {
                Ok(()) => ca_ok = true,
                Err(_) => ca_ok = false,
            }
        }
    } else if ca_deferred {
        // Do not elevate for CA again until Retry clears the deferred marker.
        ca_ok = true; // "ok" for prompting purposes — hosts may still need elevation
    }
    if hosts_ok && (ca_trusted || ca_deferred || ca_ok) {
        if !hosts_complete {
            verify_hosts_contain(&hosts_path, domains)?;
        }
        // If CA still untrusted but deferred, leave it — status API surfaces notice.
        return Ok(());
    }

    if windows_skip_uac() {
        if !hosts_ok {
            return Err(DevStackError::Message(
                "hosts write needs elevation but UAC is unavailable under CI/headless".into(),
            ));
        }
        eprintln!("devstack: skipping elevated hosts/CA under CI/headless (hosts already ok)");
        if !crate::services::tls::is_ca_trusted(paths) {
            crate::services::tls::mark_ca_trust_deferred(paths)?;
        }
        return Ok(());
    }

    let need_ca = !crate::services::tls::is_ca_trusted(paths)
        && !crate::services::tls::is_ca_trust_deferred(paths);
    let tmp = paths.helper.join("hosts.devstack.tmp");
    fs::create_dir_all(&paths.helper)?;
    fs::write(&tmp, &merged)?;
    let mut parts = String::new();
    if !hosts_ok {
        parts.push_str(&windows_hosts_copy_ps(&tmp, &hosts_path));
    }
    if need_ca {
        parts.push_str(&format!(
            "certutil -addstore -f Root {}\nWrite-DevStackLog 'CA trusted'\n",
            ps_quote(&ca.display().to_string())
        ));
    }
    match run_elevated_powershell(paths, &parts) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            if !hosts_ok {
                verify_hosts_contain(&hosts_path, domains)?;
            }
            if need_ca {
                let _ = finalize_ca_trust(paths);
            }
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            let msg = e.to_string();
            if need_ca && uac_declined_or_timed_out(&msg) {
                crate::services::tls::mark_ca_trust_deferred(paths)?;
                if hosts_ok {
                    return Ok(());
                }
            }
            Err(e)
        }
    }
}

#[cfg(windows)]
fn verify_hosts_contain(hosts_path: &Path, domains: &[String]) -> Result<()> {
    let contents = fs::read_to_string(hosts_path).unwrap_or_default();
    let missing = domains_missing_from_hosts(&contents, domains);
    if missing.is_empty() {
        return Ok(());
    }
    Err(DevStackError::Message(format!(
        "Hosts file still missing DevStack entries for: {}. The elevated write may have been blocked.",
        missing.join(", ")
    )))
}

#[cfg(windows)]
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// CI / headless sessions must not spawn `Start-Process -Verb RunAs`: the UAC
/// dialog never appears and `-Wait` blocks apply_stack for ~45–60s every warm
/// start (Windows HTTPS smoke warm budget is 5s).
#[cfg(windows)]
fn windows_skip_uac() -> bool {
    if std::env::var_os("CI").is_some() || std::env::var_os("DEVSTACK_NO_UAC").is_some() {
        return true;
    }
    use std::io::IsTerminal;
    !std::io::stdin().is_terminal()
}

#[cfg(windows)]
pub(crate) fn run_elevated_powershell(paths: &Paths, command: &str) -> Result<()> {
    if windows_skip_uac() {
        return Err(DevStackError::Message(
            "UAC elevation unavailable under CI/headless (set DEVSTACK_NO_UAC=0 only on interactive desktops)"
                .into(),
        ));
    }
    fs::create_dir_all(&paths.helper)?;
    let script = paths.helper.join("elevate.ps1");
    let log_path = paths.helper.join("elevate.log");
    let exit_path = paths.helper.join("elevate.exit");
    let _ = fs::remove_file(&log_path);
    let _ = fs::remove_file(&exit_path);

    let body = format!(
        r#"$ErrorActionPreference = 'Stop'
$log = {log}
$exitFile = {exit}
function Write-DevStackLog([string]$Message) {{
  $line = '[{{0}}] {{1}}' -f (Get-Date -Format o), $Message
  Add-Content -LiteralPath $log -Value $line -ErrorAction SilentlyContinue
}}
try {{
  Write-DevStackLog 'start'
{command}
  Write-DevStackLog 'ok'
  Set-Content -LiteralPath $exitFile -Value '0' -Encoding ascii
  exit 0
}} catch {{
  Write-DevStackLog ('error: ' + $_.Exception.Message)
  Set-Content -LiteralPath $exitFile -Value '1' -Encoding ascii
  exit 1
}}
"#,
        log = ps_quote(&log_path.display().to_string()),
        exit = ps_quote(&exit_path.display().to_string()),
        command = command,
    );
    fs::write(&script, body)?;
    let script_disp = script.display().to_string();
    // Hard wall-clock timeout: never let RunAs -Wait freeze apply_stack.
    let status = crate::services::process::command("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "$p = Start-Process -FilePath powershell -Verb RunAs -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File',{script}; if (-not $p.WaitForExit(15000)) {{ try {{ $p.Kill() }} catch {{}}; throw 'UAC elevation timed out after 15s' }}",
                script = ps_quote(&script_disp)
            ),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| DevStackError::Message(format!("UAC elevation failed: {e}")))?;

    let stderr = String::from_utf8_lossy(&status.stderr);
    let stdout = String::from_utf8_lossy(&status.stdout);
    let combined = format!("{stdout}{stderr}");
    if combined.to_ascii_lowercase().contains("canceled by the user")
        || combined.to_ascii_lowercase().contains("cancelled by the user")
        || combined.to_ascii_lowercase().contains("timed out")
    {
        return Err(DevStackError::Message(format!(
            "UAC elevation unavailable: {combined}"
        )));
    }

    let log = fs::read_to_string(&log_path).unwrap_or_default();
    let exit_code = fs::read_to_string(&exit_path)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok());

    if status.status.success() && exit_code == Some(0) {
        return Ok(());
    }
    if exit_code == Some(0) && !status.status.success() {
        // Outer process quirks — trust elevate.exit when present.
        return Ok(());
    }

    let detail = if log.trim().is_empty() {
        combined
    } else {
        log
    };
    if detail.to_ascii_lowercase().contains("canceled by the user")
        || detail.to_ascii_lowercase().contains("cancelled by the user")
    {
        return Err(DevStackError::Message("UAC cancelled".into()));
    }
    Err(DevStackError::Message(format!(
        "Elevated hosts/CA update failed. {detail}"
    )))
}

pub fn sync_hosts(paths: &Paths, state: &AppState) -> Result<()> {
    let domains = site_domains(&state.sites);
    #[cfg(windows)]
    {
        return sync_hosts_windows(paths, &domains);
    }
    #[cfg(not(windows))]
    {
        let mut args: Vec<&str> = vec!["hosts-sync"];
        let owned: Vec<String> = domains.clone();
        for d in &owned {
            args.push(d.as_str());
        }
        helper_run(paths, &args)
    }
}

/// Sync hosts file and trust the local CA — one elevation on Windows.
///
/// Skips both steps when hosts already contain every domain and the CA is
/// verified in the OS trust store (avoids UAC / certutil on every toggle).
/// Never marks the CA trusted unless store verification succeeds. When UAC is
/// declined/timed out (or CI skips elevation), defers CA trust so apply_stack
/// does not re-prompt until the user hits Retry.
pub fn sync_hosts_and_trust_ca(paths: &Paths, state: &AppState) -> Result<()> {
    let ca = crate::services::tls::ca_cert_path(paths);
    if !ca.exists() {
        crate::services::tls::ensure_root_ca(paths)?;
    }
    let hosts_ok = check_hosts_status(paths, state).ok;
    let ca_ok = crate::services::tls::is_ca_trusted(paths);
    if hosts_ok && ca_ok {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let domains = site_domains(&state.sites);
        if hosts_ok && !ca_ok {
            return trust_ca_windows(paths, &ca);
        }
        if !hosts_ok && ca_ok {
            return sync_hosts_windows(paths, &domains);
        }
        sync_hosts_and_trust_ca_windows(paths, &domains, &ca)
    }
    #[cfg(not(windows))]
    {
        if !hosts_ok {
            sync_hosts(paths, state)?;
        }
        if !ca_ok && !crate::services::tls::is_ca_trust_deferred(paths) {
            match crate::services::tls::trust_ca_via_helper(paths) {
                Ok(()) => {}
                Err(e) => {
                    // Defer so we do not re-prompt sudo/osascript on every apply.
                    let _ = crate::services::tls::mark_ca_trust_deferred(paths);
                    eprintln!("devstack: CA trust deferred: {e}");
                }
            }
        }
        Ok(())
    }
}

/// Clear the CA deferred marker and attempt hosts + CA trust again (Retry).
pub fn retry_hosts_and_ca_trust(paths: &Paths, state: &AppState) -> Result<()> {
    crate::services::tls::clear_ca_trust_deferred(paths);
    sync_hosts_and_trust_ca(paths, state)
}

/// Hosts-only sync when entries are missing (never re-trusts the CA).
pub fn sync_hosts_if_needed(paths: &Paths, state: &AppState) -> Result<()> {
    if check_hosts_status(paths, state).ok {
        return Ok(());
    }
    sync_hosts(paths, state)
}

/// True when `domain` resolves to a loopback address (hosts entry present).
pub fn domain_resolves_locally(domain: &str) -> bool {
    let candidates = [format!("{domain}:80"), format!("{domain}:443")];
    for c in candidates {
        if let Ok(addrs) = c.to_socket_addrs() {
            for addr in addrs {
                if addr.ip().is_loopback() {
                    return true;
                }
            }
        }
    }
    false
}

fn with_ca_status(paths: &Paths, mut status: HostsStatus) -> HostsStatus {
    let ca_trusted = crate::services::tls::is_ca_trusted(paths);
    status.ca_trusted = ca_trusted;
    status.ca_message = if ca_trusted {
        None
    } else {
        Some(crate::services::tls::ca_untrusted_message().into())
    };
    status
}

/// Prefer parsing the hosts file (avoids DNS-cache false negatives), then fall back to resolution.
pub fn check_hosts_status(paths: &Paths, state: &AppState) -> HostsStatus {
    let domains = site_domains(&state.sites);
    let hosts_path = system_hosts_path();
    if let Ok(contents) = fs::read_to_string(&hosts_path) {
        let missing = domains_missing_from_hosts(&contents, &domains);
        if missing.is_empty() {
            return with_ca_status(
                paths,
                HostsStatus {
                    ok: true,
                    missing,
                    message: None,
                    ca_trusted: false,
                    ca_message: None,
                },
            );
        }
        return with_ca_status(
            paths,
            HostsStatus {
                ok: false,
                message: Some(format!(
                    "Hosts file is missing DevStack entries for: {}. .test domains will not open until repaired.",
                    missing.join(", ")
                )),
                missing,
                ca_trusted: false,
                ca_message: None,
            },
        );
    }

    // Unreadable hosts file — fall back to DNS resolution.
    let mut missing = Vec::new();
    for d in &domains {
        if !domain_resolves_locally(d) {
            missing.push(d.clone());
        }
    }
    if missing.is_empty() {
        with_ca_status(
            paths,
            HostsStatus {
                ok: true,
                missing,
                message: None,
                ca_trusted: false,
                ca_message: None,
            },
        )
    } else {
        with_ca_status(
            paths,
            HostsStatus {
                ok: false,
                message: Some(format!(
                    "Could not read {} and these .test domains do not resolve locally: {}.",
                    hosts_path.display(),
                    missing.join(", ")
                )),
                missing,
                ca_trusted: false,
                ca_message: None,
            },
        )
    }
}

/// Sync hosts + CA when the DevStack block is incomplete.
pub fn ensure_hosts_if_needed(paths: &Paths, state: &AppState) -> Result<HostsStatus> {
    let status = check_hosts_status(paths, state);
    if status.ok {
        return Ok(status);
    }
    sync_hosts_and_trust_ca(paths, state)?;
    Ok(check_hosts_status(paths, state))
}

/// True when we can bind UDP 127.0.0.1:port (Hyper-V excluded ranges fail here).
pub fn udp_port_bindable(port: u16) -> bool {
    UdpSocket::bind(("127.0.0.1", port)).is_ok()
}

/// Best-effort parse of `netsh … excludedportrange` (Windows Hyper-V reservations).
fn windows_excluded_udp_ranges() -> Vec<(u16, u16)> {
    #[cfg(windows)]
    {
        let out = crate::services::process::command("netsh")
            .args([
                "interface",
                "ipv4",
                "show",
                "excludedportrange",
                "protocol=udp",
            ])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        parse_excluded_port_ranges(&out)
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn parse_excluded_port_ranges(netsh_out: &str) -> Vec<(u16, u16)> {
    let mut ranges = Vec::new();
    for line in netsh_out.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let Ok(start) = parts[0].parse::<u16>() else {
            continue;
        };
        let Ok(end) = parts[1].parse::<u16>() else {
            continue;
        };
        if start > 0 && end >= start {
            ranges.push((start, end));
        }
    }
    ranges
}

fn port_in_excluded_ranges(port: u16, ranges: &[(u16, u16)]) -> bool {
    ranges.iter().any(|(a, b)| port >= *a && port <= *b)
}

/// Pick a free UDP DNS port, preferring `preferred` then known candidates.
pub fn pick_dns_port(preferred: u16) -> u16 {
    let excluded = windows_excluded_udp_ranges();
    let mut candidates = vec![preferred];
    candidates.extend(
        crate::services::ports::candidate_ports_for("dns", preferred)
            .into_iter()
            .filter(|p| *p != preferred),
    );
    // Extra walk when Hyper-V has carved large ranges out of the low 5xxx space.
    for p in [5357, 5358, 5359, 5454, 5553, 15354, 25353] {
        if !candidates.contains(&p) {
            candidates.push(p);
        }
    }
    for p in candidates {
        if port_in_excluded_ranges(p, &excluded) {
            continue;
        }
        if udp_port_bindable(p) {
            return p;
        }
    }
    // Last resort: scan upward.
    let mut p = preferred.saturating_add(1).max(1024);
    for _ in 0..400 {
        if !port_in_excluded_ranges(p, &excluded) && udp_port_bindable(p) {
            return p;
        }
        p = p.saturating_add(1);
    }
    preferred
}

/// Start a tiny UDP DNS that answers `*.test` / `*.test.` with 127.0.0.1.
pub fn start_dns(paths: &Paths, state: &mut AppState, port: u16) -> Result<()> {
    if let Some(pid) = state.runtime.dns_pid {
        if is_running(pid) {
            return Ok(());
        }
    }
    let pid_path = paths.pid_file("dns");
    let log_out = paths.logs.join("dns.out");
    let log_err = paths.logs.join("dns.err");

    // Windows 10013 / Hyper-V excluded ranges: probe UDP bind and remapped before spawn.
    let chosen = pick_dns_port(port);
    if chosen != port {
        // Expected fallback — warn (not error) so the Logs badge stays clean.
        crate::applog::warn(
            "dns",
            format!(
                "DNS port {port} unavailable (bind denied or excluded); using {chosen}"
            ),
        );
        state.stack.dns_port = chosen;
    }

    let exe = std::env::current_exe().unwrap_or_else(|_| Path::new("devstack").to_path_buf());
    let child = spawn_logged(
        &exe,
        &["--devstack-dns", &chosen.to_string()],
        None,
        &log_out,
        &log_err,
    );
    match child {
        Ok(child) => {
            let pid = child.id();
            std::mem::forget(child);
            // Brief settle — if bind failed the helper exits immediately.
            thread::sleep(Duration::from_millis(80));
            if !is_running(pid) {
                return Err(DevStackError::Message(format!(
                    "DNS helper exited after bind on 127.0.0.1:{chosen}"
                )));
            }
            write_pid(&pid_path, pid)?;
            state.runtime.dns_pid = Some(pid);
            #[cfg(target_os = "macos")]
            {
                if let Err(e) = helper_run(paths, &["resolver-macos", &chosen.to_string()]) {
                    eprintln!("devstack resolver-macos: {e}");
                }
            }
            Ok(())
        }
        Err(_) => {
            start_dns_thread(chosen)?;
            write_pid(&pid_path, std::process::id())?;
            state.runtime.dns_pid = Some(std::process::id());
            Ok(())
        }
    }
}

pub fn stop_dns(paths: &Paths, state: &mut AppState) -> Result<()> {
    if let Some(pid) = state.runtime.dns_pid.take() {
        if pid != std::process::id() {
            stop_pid(pid)?;
        }
    }
    if let Some(pid) = read_pid(&paths.pid_file("dns")) {
        if pid != std::process::id() {
            let _ = stop_pid(pid);
        }
    }
    Ok(())
}

fn start_dns_thread(port: u16) -> Result<()> {
    let sock = UdpSocket::bind(("127.0.0.1", port)).map_err(|e| {
        let code = e.raw_os_error().unwrap_or(0);
        DevStackError::Message(format!("DNS bind 127.0.0.1:{port} failed ({code}): {e}"))
    })?;
    sock.set_read_timeout(Some(Duration::from_secs(1))).ok();
    thread::Builder::new()
        .name("devstack-dns".into())
        .spawn(move || dns_loop(sock))
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    Ok(())
}

/// Entry used when the process is launched with `--devstack-dns <port>`.
/// Returns Err on bind failure (e.g. Windows 10013) — caller must not panic;
/// the main app falls back to hosts-file resolution only.
pub fn run_dns_daemon(port: u16) -> Result<()> {
    let sock = match UdpSocket::bind(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(e) => {
            let code = e.raw_os_error().unwrap_or(0);
            // Parent start_dns already tried alternatives; keep this message concrete
            // for dns.err without going through applog (no error badge from the helper).
            return Err(DevStackError::Message(format!(
                "DNS bind 127.0.0.1:{port} failed ({code}): {e}. Using hosts-file resolution only."
            )));
        }
    };
    dns_loop(sock);
    Ok(())
}

fn dns_loop(sock: UdpSocket) {
    let mut buf = [0u8; 512];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((len, src)) => {
                if let Some(resp) = build_a_response(&buf[..len], Ipv4Addr::LOCALHOST) {
                    let _ = sock.send_to(&resp, src);
                }
            }
            Err(_) => continue,
        }
    }
}

fn build_a_response(req: &[u8], ip: Ipv4Addr) -> Option<Vec<u8>> {
    if req.len() < 12 {
        return None;
    }
    let mut out = Vec::with_capacity(req.len() + 16);
    out.extend_from_slice(&req[..2]);
    out.extend_from_slice(&[0x81, 0x80]);
    out.extend_from_slice(&req[4..6]);
    out.extend_from_slice(&[0x00, 0x01]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

    let mut i = 12usize;
    while i < req.len() && req[i] != 0 {
        let l = req[i] as usize;
        i += 1 + l;
        if i >= req.len() {
            return None;
        }
    }
    i += 1;
    if i + 4 > req.len() {
        return None;
    }
    out.extend_from_slice(&req[12..i + 4]);

    out.extend_from_slice(&[0xc0, 0x0c]);
    out.extend_from_slice(&[0x00, 0x01]);
    out.extend_from_slice(&[0x00, 0x01]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x1e]);
    out.extend_from_slice(&[0x00, 0x04]);
    out.extend_from_slice(&ip.octets());
    Some(out)
}

#[allow(dead_code)]
fn _read_all(mut r: impl Read) -> Result<Vec<u8>> {
    let mut b = Vec::new();
    r.read_to_end(&mut b)?;
    Ok(b)
}

#[allow(dead_code)]
fn _write_all(mut w: impl Write, data: &[u8]) -> Result<()> {
    w.write_all(data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_netsh_excluded_port_ranges() {
        let sample = r#"
Protocol tcp Port Exclusion Ranges

Start Port    End Port
----------    --------
      5353        5452
     50000       50059     *

* - Administered port exclusions.
"#;
        let ranges = parse_excluded_port_ranges(sample);
        assert!(ranges.contains(&(5353, 5452)));
        assert!(ranges.contains(&(50000, 50059)));
        assert!(port_in_excluded_ranges(5353, &ranges));
        assert!(port_in_excluded_ranges(5400, &ranges));
        assert!(!port_in_excluded_ranges(5453, &ranges));
    }

    #[test]
    fn hosts_block_includes_ipv4_and_ipv6() {
        let block = build_hosts_block(&[
            "welcome.test".into(),
            "phpmyadmin.test".into(),
        ]);
        assert!(block.contains(MARKER_BEGIN));
        assert!(block.contains(MARKER_END));
        assert!(block.contains("127.0.0.1 welcome.test"));
        assert!(block.contains("::1 welcome.test"));
        assert!(block.contains("127.0.0.1 phpmyadmin.test"));
        assert!(block.contains("::1 phpmyadmin.test"));
    }

    #[test]
    fn merge_replaces_previous_block() {
        let existing = "127.0.0.1 localhost\n# BEGIN DevStack\n127.0.0.1 old.test\n# END DevStack\n";
        let merged = merge_hosts_contents(existing, &["new.test".into()]);
        assert!(merged.contains("127.0.0.1 localhost"));
        assert!(merged.contains("127.0.0.1 new.test"));
        assert!(!merged.contains("old.test"));
        assert_eq!(merged.matches(MARKER_BEGIN).count(), 1);
    }

    #[test]
    fn hosts_block_includes_site_aliases() {
        let mut site = SiteRecord {
            id: "shop".into(),
            name: "shop".into(),
            path: PathBuf::from("/tmp/shop"),
            domain: "shop.test".into(),
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
            tld: "test".into(),
            aliases: vec!["store.test".into(), "Shop.local".into()],
            wildcard: true,
        };
        let domains = site_domains(&[site.clone()]);
        let block = build_hosts_block(&domains);
        assert!(block.contains("127.0.0.1 shop.test"));
        assert!(block.contains("127.0.0.1 store.test"));
        assert!(block.contains("::1 shop.local"));
        assert!(!block.contains('*'), "hosts files cannot hold wildcards");
        assert!(hosts_block_complete(&block, &domains));
        site.aliases.clear();
        assert!(!site_domains(&[site]).contains(&"store.test".to_string()));
    }

    #[test]
    fn site_domains_includes_helpers() {
        let sites = vec![SiteRecord {
            id: "a".into(),
            name: "a".into(),
            path: PathBuf::from("/tmp/a"),
            domain: "a.test".into(),
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
        }];
        let d = site_domains(&sites);
        assert!(d.contains(&"a.test".into()));
        assert!(d.contains(&"phpmyadmin.test".into()));
        assert!(d.contains(&"mailpit.test".into()));
    }

    #[test]
    fn osascript_helper_command_uses_applescript_double_quotes() {
        let script = PathBuf::from(
            "/Users/runner/Library/Application Support/devstack/helper/devstack-helper.sh",
        );
        let apple = macos_osascript_helper_command(&script, &["hosts-sync", "mailpit.test"]);
        assert!(
            apple.starts_with("do shell script \""),
            "AppleScript string must use double quotes: {apple}"
        );
        assert!(apple.contains("with administrator privileges"));
        assert!(apple.contains("Application Support"));
        assert!(apple.contains("'hosts-sync'"));
        assert!(apple.contains("'mailpit.test'"));
        // Must not produce the old broken form: do shell script '/path' 'arg'
        assert!(
            !apple.starts_with("do shell script '"),
            "bare single-quoted args break AppleScript: {apple}"
        );
    }

    #[test]
    fn uac_decline_detection_covers_cancel_and_timeout() {
        assert!(uac_declined_or_timed_out("UAC cancelled"));
        assert!(uac_declined_or_timed_out("operation canceled by the user"));
        assert!(uac_declined_or_timed_out("UAC elevation timed out after 15s"));
        assert!(uac_declined_or_timed_out("UAC elevation unavailable under CI"));
        assert!(!uac_declined_or_timed_out("permission denied writing hosts"));
    }

    #[test]
    fn hosts_status_reports_untrusted_ca_without_store() {
        let dir = std::env::temp_dir().join(format!("devstack-hosts-ca-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        crate::services::tls::ensure_root_ca(&paths).unwrap();
        // Marker alone must not count.
        let _ = fs::write(paths.certs.join("ca/.trusted-sha256"), "fake");
        let state = AppState::default();
        let status = check_hosts_status(&paths, &state);
        assert!(!status.ca_trusted);
        assert!(
            status
                .ca_message
                .as_deref()
                .unwrap_or("")
                .contains("HTTPS certificate not trusted"),
            "expected clear notice, got {:?}",
            status.ca_message
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn retry_clears_deferred_ca_marker() {
        let dir = std::env::temp_dir().join(format!("devstack-ca-retry-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        crate::services::tls::ensure_root_ca(&paths).unwrap();
        crate::services::tls::mark_ca_trust_deferred(&paths).unwrap();
        assert!(crate::services::tls::is_ca_trust_deferred(&paths));
        // retry clears deferred before attempting sync (which may still defer on CI).
        crate::services::tls::clear_ca_trust_deferred(&paths);
        assert!(!crate::services::tls::is_ca_trust_deferred(&paths));
        let _ = fs::remove_dir_all(&dir);
    }
}
