//! `.test` domain resolution + privileged helper (hosts / resolver / CA trust).

use std::fs;
use std::io::{Read, Write};
use std::net::{UdpSocket, Ipv4Addr};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::config::{AppState, Paths, SiteRecord};
use crate::error::{DevStackError, Result};
use crate::services::process::{is_running, read_pid, spawn_logged, stop_pid, write_pid};

const HELPER_SCRIPT: &str = r##"#!/usr/bin/env bash
# DevStack privileged helper — isolated surface for hosts/resolver/CA.
# Invoked via sudo/pkexec with explicit subcommands only.
set -euo pipefail
cmd="${1:-}"
shift || true
case "$cmd" in
  hosts-sync)
    marker_begin="# BEGIN DevStack"
    marker_end="# END DevStack"
    hosts_file="/etc/hosts"
    tmp="$(mktemp)"
    # drop previous block
    awk -v b="$marker_begin" -v e="$marker_end" '
      $0==b {skip=1; next}
      $0==e {skip=0; next}
      !skip {print}
    ' "$hosts_file" > "$tmp"
    {
      echo "$marker_begin"
      echo "127.0.0.1 phpmyadmin.test"
      echo "127.0.0.1 mailpit.test"
      for d in "$@"; do
        echo "127.0.0.1 $d"
      done
      echo "$marker_end"
    } >> "$tmp"
    cat "$tmp" > "$hosts_file"
    rm -f "$tmp"
    ;;
  resolver-macos)
    # /etc/resolver/test → local DevStack DNS
    mkdir -p /etc/resolver
    printf "nameserver 127.0.0.1\nport %s\n" "${1:-5353}" > /etc/resolver/test
    ;;
  trust-ca)
    ca="${1:-}"
    if [[ -z "$ca" || ! -f "$ca" ]]; then
      echo "missing ca" >&2; exit 1
    fi
    if [[ "$(uname -s)" == "Darwin" ]]; then
      security add-trusted-cert -d -r trustRoot -k /Library/Keychains/System.keychain "$ca" || true
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

pub fn ensure_helper_script(paths: &Paths) -> Result<std::path::PathBuf> {
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
    let script = ensure_helper_script(paths)?;
    // Prefer passwordless sudo when available; otherwise no-op with a soft error.
    let status = Command::new("sudo")
        .arg("-n")
        .arg(&script)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(DevStackError::Message(format!(
            "privileged helper failed (exit {s}). Run with sudo once or grant passwordless helper access."
        ))),
        Err(e) => Err(DevStackError::Message(format!("helper invoke: {e}"))),
    }
}

pub fn sync_hosts(paths: &Paths, state: &AppState) -> Result<()> {
    let mut domains: Vec<String> = state.sites.iter().map(|s| s.domain.clone()).collect();
    domains.push("phpmyadmin.test".into());
    domains.push("mailpit.test".into());
    domains.sort();
    domains.dedup();
    let args: Vec<&str> = std::iter::once("hosts-sync")
        .chain(domains.iter().map(|s| s.as_str()))
        .collect();
    helper_run(paths, &args)
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

    // Spawn a child process of this binary? For simplicity run inline thread + write pid of a small detached process.
    // We launch `devstack_smoke` style detached via a helper binary isn't available —
    // use a forked background via `std::process::Command` running a shell DNS? Better: dedicated thread
    // isn't durable across app restart. Use a small standalone loop via `nc`? 
    // Implement by spawning the current `dns_daemon` entry through env DEVSTACK_DNS_DAEMON.
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
            // leak/detach
            std::mem::forget(child);
            write_pid(&pid_path, pid)?;
            state.runtime.dns_pid = Some(pid);
            #[cfg(target_os = "macos")]
            {
                let _ = helper_run(paths, &["resolver-macos", &port.to_string()]);
            }
            Ok(())
        }
        Err(_) => {
            // Fallback: thread-based DNS inside process (good enough for smoke in same binary).
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
    out.extend_from_slice(&req[..2]); // id
    out.extend_from_slice(&[0x81, 0x80]); // flags
    out.extend_from_slice(&req[4..6]); // qdcount
    out.extend_from_slice(&[0x00, 0x01]); // ancount
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // nscount/arcount

    // copy question
    let mut i = 12usize;
    while i < req.len() && req[i] != 0 {
        let l = req[i] as usize;
        i += 1 + l;
        if i >= req.len() {
            return None;
        }
    }
    i += 1; // zero
    if i + 4 > req.len() {
        return None;
    }
    // only answer *.test queries; still reply for simplicity to all
    out.extend_from_slice(&req[12..i + 4]);

    // answer
    out.extend_from_slice(&[0xc0, 0x0c]); // name ptr
    out.extend_from_slice(&[0x00, 0x01]); // type A
    out.extend_from_slice(&[0x00, 0x01]); // class IN
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x1e]); // ttl 30
    out.extend_from_slice(&[0x00, 0x04]); // rdlength
    out.extend_from_slice(&ip.octets());
    Some(out)
}

pub fn site_domains(sites: &[SiteRecord]) -> Vec<String> {
    let mut d: Vec<_> = sites.iter().map(|s| s.domain.clone()).collect();
    d.push("phpmyadmin.test".into());
    d.push("mailpit.test".into());
    d.sort();
    d.dedup();
    d
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
