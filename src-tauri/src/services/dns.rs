//! `.test` domain resolution + privileged helper (hosts / resolver / CA trust).
//!
//! Windows writes `%SystemRoot%\System32\drivers\etc\hosts` (elevated via UAC
//! when needed). macOS uses osascript administrator privileges. Linux keeps
//! `sudo -n` / the bash helper script.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, ToSocketAddrs, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::config::{AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::process::{is_running, read_pid, spawn_logged, stop_pid, write_pid};

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
    let mut d: Vec<_> = sites.iter().map(|s| s.domain.clone()).collect();
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
    let status = crate::services::process::command("sudo")
        .arg("-n")
        .arg(&script)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
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
    // Escape for embedding inside an AppleScript string.
    let mut cmd = format!("'{}'", script.display());
    for a in args {
        cmd.push(' ');
        cmd.push_str(&shell_single_quote(a));
    }
    let apple = format!("do shell script {cmd} with administrator privileges");
    let output = crate::services::process::command("osascript")
        .args(["-e", &apple])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| DevStackError::Message(format!("osascript helper: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(DevStackError::Message(format!(
            "macOS privileged helper failed: {stderr}"
        )))
    }
}

#[cfg(any(target_os = "macos", windows))]
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
    if try_trust_ca_direct(ca) {
        return Ok(());
    }
    let ps = format!(
        "certutil -addstore -f Root {}\nWrite-DevStackLog 'CA trusted'\n",
        ps_quote(&ca.display().to_string())
    );
    run_elevated_powershell(paths, &ps)
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
    let hosts_ok = try_write_hosts_direct(&hosts_path, &merged)?;
    let ca_ok = try_trust_ca_direct(ca);
    if hosts_ok && ca_ok {
        verify_hosts_contain(&hosts_path, domains)?;
        return Ok(());
    }

    let tmp = paths.helper.join("hosts.devstack.tmp");
    fs::create_dir_all(&paths.helper)?;
    fs::write(&tmp, &merged)?;
    let mut parts = String::new();
    if !hosts_ok {
        parts.push_str(&windows_hosts_copy_ps(&tmp, &hosts_path));
    }
    if !ca_ok {
        parts.push_str(&format!(
            "certutil -addstore -f Root {}\nWrite-DevStackLog 'CA trusted'\n",
            ps_quote(&ca.display().to_string())
        ));
    }
    run_elevated_powershell(paths, &parts)?;
    let _ = fs::remove_file(&tmp);
    if !hosts_ok {
        verify_hosts_contain(&hosts_path, domains)?;
    }
    Ok(())
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

#[cfg(windows)]
fn run_elevated_powershell(paths: &Paths, command: &str) -> Result<()> {
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
    let status = crate::services::process::command("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "try {{ Start-Process -FilePath powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-WindowStyle','Hidden','-ExecutionPolicy','Bypass','-File',{script} }} catch {{ Write-Error $_.Exception.Message; exit 2 }}",
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
    {
        return Err(DevStackError::Message("UAC cancelled".into()));
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
pub fn sync_hosts_and_trust_ca(paths: &Paths, state: &AppState) -> Result<()> {
    let ca = crate::services::tls::ca_cert_path(paths);
    if !ca.exists() {
        crate::services::tls::ensure_root_ca(paths)?;
    }
    #[cfg(windows)]
    {
        let domains = site_domains(&state.sites);
        return sync_hosts_and_trust_ca_windows(paths, &domains, &ca);
    }
    #[cfg(not(windows))]
    {
        let _ = ca;
        sync_hosts(paths, state)?;
        crate::services::tls::trust_ca_via_helper(paths)
    }
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

/// Prefer parsing the hosts file (avoids DNS-cache false negatives), then fall back to resolution.
pub fn check_hosts_status(paths: &Paths, state: &AppState) -> HostsStatus {
    let domains = site_domains(&state.sites);
    let hosts_path = system_hosts_path();
    if let Ok(contents) = fs::read_to_string(&hosts_path) {
        let missing = domains_missing_from_hosts(&contents, &domains);
        if missing.is_empty() {
            return HostsStatus {
                ok: true,
                missing,
                message: None,
            };
        }
        return HostsStatus {
            ok: false,
            message: Some(format!(
                "Hosts file is missing DevStack entries for: {}. .test domains will not open until repaired.",
                missing.join(", ")
            )),
            missing,
        };
    }

    // Unreadable hosts file — fall back to DNS resolution.
    let _ = paths;
    let mut missing = Vec::new();
    for d in &domains {
        if !domain_resolves_locally(d) {
            missing.push(d.clone());
        }
    }
    if missing.is_empty() {
        HostsStatus {
            ok: true,
            missing,
            message: None,
        }
    } else {
        HostsStatus {
            ok: false,
            message: Some(format!(
                "Could not read {} and these .test domains do not resolve locally: {}.",
                hosts_path.display(),
                missing.join(", ")
            )),
            missing,
        }
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

    let exe = std::env::current_exe().unwrap_or_else(|_| Path::new("devstack").to_path_buf());
    let child = spawn_logged(
        &exe,
        &["--devstack-dns", &port.to_string()],
        None,
        &log_out,
        &log_err,
    );
    match child {
        Ok(child) => {
            let pid = child.id();
            std::mem::forget(child);
            write_pid(&pid_path, pid)?;
            state.runtime.dns_pid = Some(pid);
            #[cfg(target_os = "macos")]
            {
                if let Err(e) = helper_run(paths, &["resolver-macos", &port.to_string()]) {
                    eprintln!("devstack resolver-macos: {e}");
                }
            }
            Ok(())
        }
        Err(_) => {
            start_dns_thread(port)?;
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
        DevStackError::Message(format!("DNS bind 127.0.0.1:{port} failed: {e}"))
    })?;
    sock.set_read_timeout(Some(Duration::from_secs(1))).ok();
    thread::Builder::new()
        .name("devstack-dns".into())
        .spawn(move || dns_loop(sock))
        .map_err(|e| DevStackError::Message(e.to_string()))?;
    Ok(())
}

/// Entry used when the process is launched with `--devstack-dns <port>`.
pub fn run_dns_daemon(port: u16) -> Result<()> {
    let sock = UdpSocket::bind(("127.0.0.1", port))
        .map_err(|e| DevStackError::Message(format!("DNS bind failed: {e}")))?;
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
        }];
        let d = site_domains(&sites);
        assert!(d.contains(&"a.test".into()));
        assert!(d.contains(&"phpmyadmin.test".into()));
        assert!(d.contains(&"mailpit.test".into()));
    }
}
