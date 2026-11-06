use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::error::{DevStackError, Result};

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
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).status();
        std::thread::sleep(std::time::Duration::from_millis(400));
        if is_running(pid) {
            let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
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
    let mut cmd = Command::new(program);
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
    let mut cmd = Command::new(program);
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
