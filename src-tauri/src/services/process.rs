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

/// Process-wide Windows job object with `KILL_ON_JOB_CLOSE` so DevStack children
/// die when the app exits (prevents orphan nginx/httpd pile-up).
#[cfg(windows)]
static DEVSTACK_JOB: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

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
    // Children we `mem::forget` are never reaped, so an exited server lingers as a
    // zombie. Treat zombies as stopped so stop/readiness paths don't wait on them.
    sys.process(Pid::from_u32(pid))
        .map(|p| {
            !matches!(p.status(), sysinfo::ProcessStatus::Zombie) || zombie_leader_has_threads(pid)
        })
        .unwrap_or(false)
}

/// Linux keeps the thread-group leader as a zombie while sibling threads still run
/// (e.g. a server whose main thread called pthread_exit) — that process is alive.
fn zombie_leader_has_threads(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        fs::read_dir(format!("/proc/{pid}/task"))
            .map(|rd| rd.count() > 1)
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        false
    }
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
        // Poll instead of a fixed 400ms sleep — well-behaved servers exit in <50ms.
        let exited = crate::services::perf::poll_backoff(
            std::time::Duration::from_millis(5),
            std::time::Duration::from_millis(40),
            std::time::Duration::from_millis(400),
            || (!is_running(pid)).then_some(()),
        )
        .is_some();
        if !exited {
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
    spawn_logged_env(program, args, cwd, log_out, log_err, &[], true)
}

/// Spawn with redirected logs but **without** `DETACHED_PROCESS` (needed for
/// Windows mysqld/`--console` so the process stays alive and writes stderr).
pub fn spawn_logged_attached(
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    log_out: &Path,
    log_err: &Path,
) -> Result<Child> {
    spawn_logged_env(program, args, cwd, log_out, log_err, &[], false)
}

/// Like [`spawn_logged`], with extra environment variables (e.g. `PHPRC` for php-cgi).
pub fn spawn_logged_env(
    program: &Path,
    args: &[&str],
    cwd: Option<&Path>,
    log_out: &Path,
    log_err: &Path,
    envs: &[(&str, &OsStr)],
    detached: bool,
) -> Result<Child> {
    if let Some(parent) = log_out.parent() {
        fs::create_dir_all(parent)?;
    }
    let out = fs::File::create(log_out)?;
    let err = fs::File::create(log_err)?;
    let mut cmd = command(program);
    if detached {
        no_window_detached(&mut cmd);
    } else {
        no_window(&mut cmd);
    }
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let child = cmd.spawn().map_err(|e| {
        crate::applog::error(
            "process",
            format!("failed to spawn {}: {e:#}", program.display()),
        );
        DevStackError::Message(format!("failed to spawn {}: {e}", program.display()))
    })?;
    assign_child_to_job_object(&child);
    let pid = child.id();
    let source = crate::applog::source_from_log_path(log_err);
    crate::applog::process_spawn(source, program, args, envs, pid);
    // Stream raw stdout/stderr into the live log ring.
    crate::applog::tail_file(log_out.to_path_buf(), source);
    crate::applog::tail_file(log_err.to_path_buf(), source);
    Ok(child)
}

/// Assign a spawned child to the DevStack job object (Windows only; no-op elsewhere).
pub fn assign_child_to_job_object(_child: &Child) {
    #[cfg(windows)]
    {
        assign_child_to_job_object_windows(_child);
    }
}

/// Minimal kernel32 job-object FFI — avoids windows-sys feature gaps
/// (`CreateJobObjectW` is missing from some `Win32_System_JobObjects` builds).
#[cfg(windows)]
mod job_ffi {
    use std::ffi::c_void;
    pub type HANDLE = *mut c_void;
    pub type BOOL = i32;

    #[repr(C)]
    pub struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
        pub per_process_user_time_limit: i64,
        pub per_job_user_time_limit: i64,
        pub limit_flags: u32,
        pub minimum_working_set_size: usize,
        pub maximum_working_set_size: usize,
        pub active_process_limit: u32,
        pub affinity: usize,
        pub priority_class: u32,
        pub scheduling_class: u32,
    }

    #[repr(C)]
    pub struct IO_COUNTERS {
        pub read_operation_count: u64,
        pub write_operation_count: u64,
        pub other_operation_count: u64,
        pub read_transfer_count: u64,
        pub write_transfer_count: u64,
        pub other_transfer_count: u64,
    }

    #[repr(C)]
    pub struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        pub basic_limit_information: JOBOBJECT_BASIC_LIMIT_INFORMATION,
        pub io_info: IO_COUNTERS,
        pub process_memory_limit: usize,
        pub job_memory_limit: usize,
        pub peak_process_memory_used: usize,
        pub peak_job_memory_used: usize,
    }

    pub const JobObjectExtendedLimitInformation: i32 = 9;
    pub const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateJobObjectW(lp_job_attributes: *const c_void, lp_name: *const u16) -> HANDLE;
        pub fn AssignProcessToJobObject(h_job: HANDLE, h_process: HANDLE) -> BOOL;
        pub fn SetInformationJobObject(
            h_job: HANDLE,
            job_object_information_class: i32,
            lp_job_object_information: *const c_void,
            cb_job_object_information_length: u32,
        ) -> BOOL;
        pub fn CloseHandle(h_object: HANDLE) -> BOOL;
    }
}

#[cfg(windows)]
fn assign_child_to_job_object_windows(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use job_ffi::*;

    let job = *DEVSTACK_JOB.get_or_init(|| {
        // SAFETY: null attrs/name → anonymous job object.
        let handle: HANDLE = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            crate::applog::warn("process", "CreateJobObjectW failed; orphans may linger on exit");
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: handle is a valid job we just created.
        let ok: BOOL = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            )
        };
        if ok == 0 {
            crate::applog::warn(
                "process",
                "SetInformationJobObject(KILL_ON_JOB_CLOSE) failed",
            );
            unsafe {
                let _ = CloseHandle(handle);
            }
            return 0;
        }
        handle as isize
    });
    if job == 0 {
        return;
    }
    let process = child.as_raw_handle() as HANDLE;
    // SAFETY: process is the OS handle of a live Child; job is our process-wide job.
    let ok: BOOL = unsafe { AssignProcessToJobObject(job as HANDLE, process) };
    if ok == 0 {
        crate::applog::warn(
            "process",
            format!(
                "AssignProcessToJobObject failed for pid={} (may already be in a job)",
                child.id()
            ),
        );
    }
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

    #[cfg(unix)]
    #[test]
    fn exited_unreaped_child_is_not_running_and_stop_is_fast() {
        let child = command("true").spawn().expect("spawn true");
        let pid = child.id();
        std::mem::forget(child);
        let sw = crate::services::perf::Stopwatch::start();
        let _ = crate::services::perf::poll_backoff(
            std::time::Duration::from_millis(2),
            std::time::Duration::from_millis(10),
            std::time::Duration::from_secs(2),
            || (!is_running(pid)).then_some(()),
        );
        assert!(!is_running(pid), "zombie must count as stopped");
        let _ = stop_pid(pid);
        assert!(sw.elapsed_ms() < 1500);
    }

    #[cfg(unix)]
    #[test]
    fn stop_pid_returns_quickly_for_sigterm_friendly_process() {
        let child = command("sleep").arg("30").spawn().expect("spawn sleep");
        let pid = child.id();
        std::mem::forget(child);
        assert!(is_running(pid));
        let sw = crate::services::perf::Stopwatch::start();
        stop_pid(pid).unwrap();
        assert!(!is_running(pid));
        assert!(sw.elapsed_ms() < 350, "stop took {}ms", sw.elapsed_ms());
    }
}
