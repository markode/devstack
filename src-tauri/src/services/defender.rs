//! Optional Windows Defender path exclusions (Settings → Performance).
//!
//! Isolation rule: the shipped app never adds Defender exclusions silently.
//! CI may call `Add-MpPreference` in the workflow only. Product path is an
//! explicit opt-in toggle (default OFF) that elevates via UAC, and exclusions
//! are removed on toggle-off or uninstall.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Paths;
use crate::error::{DevStackError, Result};

/// Manifest of paths we asked Defender to exclude (survives for uninstall).
fn exclusions_manifest(paths: &Paths) -> PathBuf {
    paths.helper.join("defender-exclusions.txt")
}

/// Paths covered when the user opts in (data root + hot spawn dirs).
pub fn exclusion_paths(paths: &Paths) -> Vec<PathBuf> {
    vec![
        paths.root.clone(),
        paths.packages.clone(),
        paths.run.clone(),
        paths.helper.clone(),
    ]
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSettingsView {
    /// Whether Defender path exclusions are enabled in saved settings.
    pub defender_exclusions: bool,
    /// Platform supports the toggle (Windows only).
    pub defender_exclusions_supported: bool,
    /// Paths that would be / are excluded when enabled.
    pub exclusion_paths: Vec<String>,
    /// Short explanation shown next to the toggle.
    pub defender_exclusions_help: String,
}

pub fn settings_view(paths: &Paths, enabled: bool) -> PerformanceSettingsView {
    PerformanceSettingsView {
        defender_exclusions: enabled,
        defender_exclusions_supported: cfg!(windows),
        exclusion_paths: exclusion_paths(paths)
            .into_iter()
            .map(|p| p.display().to_string())
            .collect(),
        defender_exclusions_help: "Speeds up PHP/Nginx respawns by skipping Windows Defender \
scans of the DevStack data folder. This is a system-wide security change (UAC required). \
Off by default; exclusions are removed when you turn this off or uninstall DevStack."
            .into(),
    }
}

/// PowerShell that adds ExclusionPath entries (elevated).
pub fn build_add_exclusions_ps(paths: &[PathBuf]) -> String {
    let mut s = String::from("$ErrorActionPreference = 'Stop'\n");
    for p in paths {
        s.push_str(&format!(
            "if (Test-Path -LiteralPath {path}) {{ Add-MpPreference -ExclusionPath {path} }}\n",
            path = ps_quote(&p.display().to_string()),
        ));
    }
    s.push_str("Write-DevStackLog 'defender exclusions added'\n");
    s
}

/// PowerShell that removes ExclusionPath entries (elevated).
pub fn build_remove_exclusions_ps(paths: &[PathBuf]) -> String {
    let mut s = String::from("$ErrorActionPreference = 'Continue'\n");
    for p in paths {
        s.push_str(&format!(
            "Remove-MpPreference -ExclusionPath {path} -ErrorAction SilentlyContinue\n",
            path = ps_quote(&p.display().to_string()),
        ));
    }
    s.push_str("Write-DevStackLog 'defender exclusions removed'\n");
    s
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub fn write_exclusions_manifest(paths: &Paths, listed: &[PathBuf]) -> Result<()> {
    fs::create_dir_all(&paths.helper)?;
    let body = listed
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(exclusions_manifest(paths), body)?;
    Ok(())
}

pub fn read_exclusions_manifest(paths: &Paths) -> Vec<PathBuf> {
    fs::read_to_string(exclusions_manifest(paths))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn clear_exclusions_manifest(paths: &Paths) {
    let _ = fs::remove_file(exclusions_manifest(paths));
}

/// Enable or disable Defender exclusions. Never called from apply_stack.
pub fn set_defender_exclusions(paths: &Paths, enable: bool) -> Result<()> {
    if !cfg!(windows) {
        if enable {
            return Err(DevStackError::Message(
                "Windows Defender exclusions are only available on Windows".into(),
            ));
        }
        clear_exclusions_manifest(paths);
        return Ok(());
    }
    #[cfg(windows)]
    {
        set_defender_exclusions_windows(paths, enable)
    }
    #[cfg(not(windows))]
    {
        let _ = paths;
        let _ = enable;
        Ok(())
    }
}

#[cfg(windows)]
fn set_defender_exclusions_windows(paths: &Paths, enable: bool) -> Result<()> {
    let listed = if enable {
        exclusion_paths(paths)
    } else {
        let mut v = read_exclusions_manifest(paths);
        if v.is_empty() {
            v = exclusion_paths(paths);
        }
        v
    };
    if listed.is_empty() {
        return Ok(());
    }
    let ps = if enable {
        build_add_exclusions_ps(&listed)
    } else {
        build_remove_exclusions_ps(&listed)
    };
    crate::services::dns::run_elevated_powershell(paths, &ps)?;
    if enable {
        write_exclusions_manifest(paths, &listed)?;
    } else {
        clear_exclusions_manifest(paths);
    }
    Ok(())
}

/// Best-effort remove during uninstall / cleanup (no UAC if already elevated).
pub fn remove_exclusions_script_for_uninstall(manifest: &Path) -> String {
    let paths: Vec<PathBuf> = fs::read_to_string(manifest)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect();
    let mut ps = build_remove_exclusions_ps(&paths);
    ps.push_str(&format!(
        "Remove-Item -LiteralPath {m} -Force -ErrorAction SilentlyContinue\n",
        m = ps_quote(&manifest.display().to_string()),
    ));
    ps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_default_view_is_off_and_explains_uac() {
        let dir = std::env::temp_dir().join(format!("devstack-def-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let v = settings_view(&paths, false);
        assert!(!v.defender_exclusions);
        assert_eq!(v.defender_exclusions_supported, cfg!(windows));
        assert!(v.defender_exclusions_help.contains("UAC"));
        assert!(v.defender_exclusions_help.to_ascii_lowercase().contains("off by default"));
        assert!(!v.exclusion_paths.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_and_remove_scripts_quote_paths_and_call_mppreference() {
        let paths = vec![
            PathBuf::from(r"C:\Users\me\AppData\Local\DevStack"),
            PathBuf::from(r"C:\Users\me\AppData\Local\DevStack\packages"),
        ];
        let add = build_add_exclusions_ps(&paths);
        assert!(add.contains("Add-MpPreference -ExclusionPath"));
        assert!(add.contains(r"'C:\Users\me\AppData\Local\DevStack'"));
        assert!(!add.contains("Remove-MpPreference"));

        let rem = build_remove_exclusions_ps(&paths);
        assert!(rem.contains("Remove-MpPreference -ExclusionPath"));
        assert!(!rem.contains("Add-MpPreference"));
    }

    #[test]
    fn manifest_roundtrip_and_uninstall_script() {
        let dir = std::env::temp_dir().join(format!("devstack-def-m-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths::from_root(dir.clone()).unwrap();
        let listed = exclusion_paths(&paths);
        write_exclusions_manifest(&paths, &listed).unwrap();
        let read = read_exclusions_manifest(&paths);
        assert_eq!(read.len(), listed.len());
        let script = remove_exclusions_script_for_uninstall(&exclusions_manifest(&paths));
        assert!(script.contains("Remove-MpPreference"));
        assert!(script.contains("Remove-Item"));
        clear_exclusions_manifest(&paths);
        assert!(read_exclusions_manifest(&paths).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn shipped_app_never_auto_enables_exclusions() {
        // Guardrail: default state bit is false; apply_stack must not call set_*.
        assert!(!crate::config::PerformanceSettings::default().defender_exclusions);
    }
}
