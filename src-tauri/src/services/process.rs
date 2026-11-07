//! Process helpers — PID files, spawn/kill, and Windows console suppression.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::error::{DevStackError, Result};

/// Win32 `CREATE_NO_WINDOW` — hide console windows for GUI-launched children.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Win32 `DETACHED_PROCESS` — detach from the parent console.
#[cfg(windows)]
pub const DETACHED_PROCESS: u32 = 0x0000_0008;

/// Apply Windows creation flags so helper/server processes never flash a cmd window.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Like [`no_window`], plus `DETACHED_PROCESS` for long-lived daemons.
pub fn no_window_detached(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
    cmd
}

/// Build a [`Command`] with Windows console suppression already applied.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    no_window(&mut cmd);
    cmd
}

pub fn write_pid(path: &Path, pid: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, pid.to_string())?;
    Ok(())
}

pub fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

pub fn is_running(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    sys.process(Pid::from_u32(pid)).is_some()
}

pub fn stop_pid(pid: u32) -> Result<()> {
    if !is_running(pid) {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let _ = command("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
        std::thread::sleep(std::time::Duration::from_millis(400));
        if is_running(pid) {
            let _ = command("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
    }
    #[cfg(windows)]
    {
        let _ = command("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
    Ok(())
}

pub fn spawn_logged(
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    log_out: &Path,
    log_err: &Path,
) -> Result<Child> {
    if let Some(parent) = log_out.parent() {
        fs::create_dir_all(parent)?;
    }
    let out = fs::File::create(log_out)?;
    let err = fs::File::create(log_err)?;
    let mut cmd = command(program);
    no_window_detached(&mut cmd);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn().map_err(|e| {
        DevStackError::Message(format!("failed to spawn {}: {e}", program.display()))
    })
}

pub fn run_capture(program: &Path, args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let mut cmd = command(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().map_err(|e| {
        DevStackError::Message(format!("failed to run {}: {e}", program.display()))
    })?;
    if !out.status.success() {
        return Err(DevStackError::Message(format!(
            "{} failed: {}",
            program.display(),
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Find first existing executable among candidates.
pub fn find_bin(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        for candidate in [
            dir.join(name),
            dir.join("bin").join(name),
            dir.join("sbin").join(name),
            dir.join("usr/sbin").join(name),
            dir.join("usr/bin").join(name),
            dir.join("scripts").join(name),
        ] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_builder_returns_program() {
        let cmd = command("echo");
        // Just ensure construction does not panic on any platform.
        let _ = format!("{cmd:?}");
    }
}
