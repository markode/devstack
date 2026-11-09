//! Cross-platform path normalization — never concatenate with `/` by hand.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{DevStackError, Result};

/// Expand `~/…` / `~\…` and normalize separators into a platform [`PathBuf`].
pub fn expand_home(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    let expanded = if let Some(rest) = raw
        .strip_prefix("~/")
        .or_else(|| raw.strip_prefix("~\\"))
        .or_else(|| (raw == "~").then_some(""))
    {
        let mut out = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        push_normalized(&mut out, rest);
        out
    } else {
        normalize_separators(path)
    };
    simplify(&expanded)
}

/// Replace `/` and `\` with native components (handles mixed Windows paths).
pub fn normalize_separators(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    // Absolute Windows drive path: `C:\…` or `C:/…`
    if cfg!(windows) {
        let bytes = s.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' {
            // `PathBuf::from("D:")` is "cwd on D:", not the drive root — use `D:\`.
            let drive = (bytes[0] as char).to_ascii_uppercase();
            let mut out = PathBuf::from(format!("{drive}:\\"));
            let rest = &s[2..];
            push_normalized(&mut out, rest.trim_start_matches(['/', '\\']));
            return out;
        }
        if s.starts_with("\\\\") || s.starts_with("//") {
            // UNC — keep as-is after separator normalize
            let trimmed = s.trim_start_matches(['/', '\\']);
            let mut out = PathBuf::from("\\\\");
            push_normalized(&mut out, trimmed);
            return out;
        }
    }
    let mut out = PathBuf::new();
    if s.starts_with('/') {
        out.push(Component::RootDir.as_os_str());
    }
    push_normalized(&mut out, &s);
    out
}

fn push_normalized(out: &mut PathBuf, rest: &str) {
    for part in rest.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        out.push(part);
    }
}

/// `dunce::simplified` when available — strip `\\?\` verbatim prefix on Windows.
pub fn simplify(path: &Path) -> PathBuf {
    dunce::simplified(path).to_path_buf()
}

/// Canonicalize when the path exists; otherwise return the normalized absolute form.
pub fn normalize_existing(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    if let Ok(canon) = fs::canonicalize(&expanded) {
        return simplify(&canon);
    }
    expanded
}

/// Ensure a directory exists (create parents). Returns the normalized absolute path.
pub fn ensure_dir(path: &Path) -> Result<PathBuf> {
    let abs = expand_home(path);
    fs::create_dir_all(&abs).map_err(|e| {
        DevStackError::Message(format!("could not create directory {}: {e}", abs.display()))
    })?;
    Ok(normalize_existing(&abs))
}

/// Forward-slash path for Nginx/Apache/MySQL conf (handles Windows + spaces).
/// Does not add quotes — callers that need a quoted absolute path should wrap it.
pub fn conf_path(path: &Path) -> String {
    expand_home(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Alias of [`conf_path`] (PHP ini `extension_dir`, phpMyAdmin, etc.).
pub fn posix_path(path: &Path) -> String {
    conf_path(path)
}

/// Quote a conf path when it contains spaces (MySQL `datadir="…"`).
pub fn quoted_conf_path(path: &Path) -> String {
    let s = conf_path(path);
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s
    }
}

/// Open a file or folder with the OS shell (bypasses Tauri opener path scope).
pub fn open_path_os(path: &Path) -> Result<()> {
    let abs = normalize_existing(path);
    if !abs.exists() {
        return Err(DevStackError::Message(format!(
            "path doesn't exist: {}",
            abs.display()
        )));
    }
    #[cfg(windows)]
    {
        // `explorer.exe` opens folders; for files use `cmd /C start`.
        let status = if abs.is_dir() {
            crate::services::process::command("explorer.exe")
                .arg(&abs)
                .status()
        } else {
            crate::services::process::command("cmd.exe")
                .args(["/C", "start", "", &abs.to_string_lossy()])
                .status()
        }
        .map_err(|e| DevStackError::Message(format!("could not open path: {e}")))?;
        if !status.success() {
            // explorer returns 1 even on success sometimes — only error if path missing.
            if !abs.exists() {
                return Err(DevStackError::Message(format!(
                    "could not open path: {}",
                    abs.display()
                )));
            }
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let status = crate::services::process::command("open")
            .arg(&abs)
            .status()
            .map_err(|e| DevStackError::Message(format!("could not open path: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(DevStackError::Message(format!(
                "open failed for {}",
                abs.display()
            )))
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let status = crate::services::process::command("xdg-open")
            .arg(&abs)
            .status()
            .map_err(|e| DevStackError::Message(format!("could not open path: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(DevStackError::Message(format!(
                "xdg-open failed for {}",
                abs.display()
            )))
        }
    }
}

/// Reveal a path in the file manager (select file / open parent folder).
pub fn reveal_path_os(path: &Path) -> Result<()> {
    let abs = normalize_existing(path);
    if !abs.exists() {
        return Err(DevStackError::Message(format!(
            "path doesn't exist: {}",
            abs.display()
        )));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer /select,C:\path — raw_arg preserves commas/spaces correctly.
        let mut cmd = crate::services::process::command("explorer.exe");
        cmd.raw_arg(format!("/select,{}", abs.display()));
        cmd.status()
            .map_err(|e| DevStackError::Message(format!("could not reveal path: {e}")))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let status = crate::services::process::command("open")
            .args(["-R"])
            .arg(&abs)
            .status()
            .map_err(|e| DevStackError::Message(format!("could not reveal path: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(DevStackError::Message(format!(
                "open -R failed for {}",
                abs.display()
            )))
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let parent = abs.parent().unwrap_or(&abs);
        open_path_os(parent)
    }
}

/// Open a terminal/shell with cwd set to `dir` (DevStack Terminal action).
pub fn open_terminal_in_dir(dir: &Path) -> Result<()> {
    let abs = expand_home(dir);
    if !abs.exists() {
        ensure_dir(&abs)?;
    }
    let abs = normalize_existing(&abs);
    #[cfg(windows)]
    {
        let status = crate::services::process::command("cmd.exe")
            .args(["/C", "start", "DevStack", "cmd.exe", "/K", "cd", "/D"])
            .arg(&abs)
            .status()
            .map_err(|e| DevStackError::Message(format!("could not open terminal: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            // Fallback: just open the folder.
            open_path_os(&abs)
        }
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"Terminal\" to do script \"cd \" & quoted form of \"{}\"",
            abs.display()
        );
        let status = crate::services::process::command("osascript")
            .args(["-e", &script])
            .status()
            .map_err(|e| DevStackError::Message(format!("could not open terminal: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            open_path_os(&abs)
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            if which::which(term).is_ok() {
                let status = crate::services::process::command(term)
                    .arg("--working-directory")
                    .arg(&abs)
                    .status();
                if status.map(|s| s.success()).unwrap_or(false) {
                    return Ok(());
                }
            }
        }
        open_path_os(&abs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_splits_forward_slashes() {
        let p = expand_home(Path::new("~/Sites/acme-dashboard"));
        // Must be separate components — never one component containing a slash.
        let parts: Vec<_> = p
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        assert!(
            parts.iter().any(|p| p == "Sites"),
            "missing Sites in {parts:?}"
        );
        assert!(
            parts.iter().any(|p| p == "acme-dashboard"),
            "missing acme-dashboard in {parts:?}"
        );
        assert!(
            parts.iter().all(|p| !p.contains('/') && !p.contains('\\')),
            "slash inside component: {parts:?}"
        );
    }

    #[test]
    fn normalize_mixed_windows_style() {
        let p = normalize_separators(Path::new(r"C:\Users\Marcin\Sites/acme-dashboard"));
        let s = p.to_string_lossy();
        assert!(s.contains("acme-dashboard"));
        // Should not keep a single component with an embedded slash after normalize.
        let has_mixed_component = p.components().any(|c| {
            let st = c.as_os_str().to_string_lossy();
            st.contains('/') && st.contains('\\')
        });
        assert!(!has_mixed_component, "{p:?}");
    }

    #[test]
    fn windows_drive_root_is_absolute() {
        let p = normalize_separators(Path::new(r"D:\a\_temp\devstack\data"));
        let s = conf_path(&p);
        // Must keep the slash after the drive letter — `D:a` is cwd-relative.
        assert!(
            s.starts_with("D:/a/") || s.starts_with(r"D:\a\"),
            "drive root lost: {s} ({p:?})"
        );
        assert!(!s.starts_with("D:a"), "relative drive path: {s}");
    }
}
