//! Developer CLI tooling: Composer + WP-CLI + Node shims in `{tools}/bin`.
//!
//! Shims prefer DevStack-managed installs and fall back to whatever is on the
//! user's PATH (minus the shim dir). The global PATH is only modified when
//! `SitesSettings.opt_in_path` is enabled.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{AppState, Paths};
use crate::downloads::{download_file, fetch_text, http_client, verify_sha256, ProgressFn};
use crate::error::{DevStackError, Result};
use crate::services::process::find_bin;
use crate::services::progress::{emit, JobProgressFn};

pub use crate::services::wordpress::ensure_wp_cli;

const COMPOSER_URL: &str = "https://getcomposer.org/download/latest-stable/composer.phar";
const COMPOSER_SHA_URL: &str = "https://getcomposer.org/download/latest-stable/composer.phar.sha256sum";

const PATH_BEGIN: &str = "# >>> devstack tools >>>";
const PATH_END: &str = "# <<< devstack tools <<<";

pub fn bin_dir(paths: &Paths) -> PathBuf {
    paths.tools.join("bin")
}

pub fn composer_phar(paths: &Paths) -> PathBuf {
    paths.tools.join("composer.phar")
}

pub async fn ensure_composer(
    paths: &Paths,
    progress: &JobProgressFn,
    job_id: &str,
) -> Result<PathBuf> {
    let dest = composer_phar(paths);
    if dest.exists() && fs::metadata(&dest).map(|m| m.len() > 100_000).unwrap_or(false) {
        return Ok(dest);
    }
    fs::create_dir_all(&paths.tools)?;
    emit(progress, job_id, "composer", "Downloading Composer…", 0, None);
    let client = http_client()?;
    let dl: ProgressFn = {
        let progress = progress.clone();
        let job_id = job_id.to_string();
        std::sync::Arc::new(move |ev| {
            emit(&progress, &job_id, "composer", "Downloading Composer…", ev.downloaded, ev.total);
        })
    };
    download_file(&client, COMPOSER_URL, &dest, "composer", "latest", &dl).await?;
    if let Ok(raw) = fetch_text(&client, COMPOSER_SHA_URL).await {
        if let Some(sha) = raw.split_whitespace().next() {
            if let Err(e) = verify_sha256(&dest, sha) {
                let _ = fs::remove_file(&dest);
                return Err(e);
            }
        }
    }
    Ok(dest)
}

/// Directory containing the installed `node` binary (and `npm`).
pub fn node_bin(state: &AppState) -> Option<PathBuf> {
    let rec = state.installed.get("node")?.first()?;
    find_bin(&rec.path, &["node", "node.exe"])
}

/// Resolve the managed Node binary or explain how to install it.
pub fn ensure_node(state: &AppState) -> Result<PathBuf> {
    node_bin(state).ok_or_else(|| {
        DevStackError::Message("Node.js is not installed — install it from Packages first".into())
    })
}

fn managed_php(paths: &Paths, state: &AppState) -> Option<(PathBuf, Option<PathBuf>)> {
    let list = state.installed.get("php")?;
    let rec = state
        .stack
        .php_version
        .as_ref()
        .and_then(|v| list.iter().find(|r| &r.version == v))
        .or_else(|| list.first())?;
    let root = if rec.path.exists() {
        rec.path.clone()
    } else {
        paths.package_dir("php", &rec.version)
    };
    let bin = find_bin(&root, &["php", "php.exe"])?;
    let ini = crate::packages::php::php_ini_path(&root);
    Some((bin, ini.exists().then_some(ini)))
}

fn unix_fallback(name: &str) -> String {
    format!(
        "BIN_DIR=\"$(cd \"$(dirname \"$0\")\" && pwd)\"\n\
         CLEAN_PATH=$(printf '%s' \"$PATH\" | tr ':' '\\n' | grep -vxF \"$BIN_DIR\" | paste -sd: -)\n\
         PATH=\"$CLEAN_PATH\" exec {name} \"$@\"\n"
    )
}

pub fn unix_php_phar_shim(tools: &Path, name: &str, phar: &str, php: Option<(&Path, Option<&Path>)>) -> String {
    let phar_path = tools.join(phar);
    let mut s = format!("#!/bin/sh\n# DevStack shim for `{name}` (tools dir: {})\n", tools.display());
    if let Some((bin, ini)) = php {
        let ini_arg = ini
            .map(|i| format!(" -c \"{}\"", i.display()))
            .unwrap_or_default();
        s.push_str(&format!(
            "if [ -x \"{bin}\" ] && [ -f \"{phar}\" ]; then\n  exec \"{bin}\"{ini_arg} \"{phar}\" \"$@\"\nfi\n",
            bin = bin.display(),
            phar = phar_path.display(),
        ));
    }
    s.push_str(&unix_fallback(name));
    s
}

pub fn unix_node_shim(tools: &Path, name: &str, node_dir: Option<&Path>) -> String {
    let mut s = format!("#!/bin/sh\n# DevStack shim for `{name}` (tools dir: {})\n", tools.display());
    if let Some(dir) = node_dir {
        s.push_str(&format!(
            "if [ -x \"{dir}/{name}\" ]; then\n  PATH=\"{dir}:$PATH\" exec \"{dir}/{name}\" \"$@\"\nfi\n",
            dir = dir.display()
        ));
    }
    s.push_str(&unix_fallback(name));
    s
}

pub fn windows_php_phar_shim(tools: &Path, name: &str, phar: &str, php: Option<(&Path, Option<&Path>)>) -> String {
    let phar_path = tools.join(phar);
    let mut s = format!("@echo off\r\nREM DevStack shim for {name} (tools dir: {})\r\n", tools.display());
    if let Some((bin, ini)) = php {
        let ini_arg = ini
            .map(|i| format!(" -c \"{}\"", i.display()))
            .unwrap_or_default();
        s.push_str(&format!(
            "if exist \"{bin}\" if exist \"{phar}\" (\r\n  \"{bin}\"{ini_arg} \"{phar}\" %*\r\n  exit /b %ERRORLEVEL%\r\n)\r\n",
            bin = bin.display(),
            phar = phar_path.display(),
        ));
    }
    s.push_str(&format!("set \"PATH=%PATH:%~dp0;=%\"\r\n{name} %*\r\n"));
    s
}

pub fn windows_node_shim(tools: &Path, name: &str, node_dir: Option<&Path>) -> String {
    let mut s = format!("@echo off\r\nREM DevStack shim for {name} (tools dir: {})\r\n", tools.display());
    if let Some(dir) = node_dir {
        s.push_str(&format!(
            "if exist \"{dir}\\{name}.cmd\" (\r\n  set \"PATH={dir};%PATH%\"\r\n  call \"{dir}\\{name}.cmd\" %*\r\n  exit /b %ERRORLEVEL%\r\n)\r\n\
             if exist \"{dir}\\{name}.exe\" (\r\n  set \"PATH={dir};%PATH%\"\r\n  \"{dir}\\{name}.exe\" %*\r\n  exit /b %ERRORLEVEL%\r\n)\r\n",
            dir = dir.display()
        ));
    }
    s.push_str(&format!("set \"PATH=%PATH:%~dp0;=%\"\r\n{name} %*\r\n"));
    s
}

fn write_shim(dir: &Path, name: &str, contents: &str) -> Result<()> {
    let file = dir.join(if cfg!(windows) { format!("{name}.cmd") } else { name.to_string() });
    fs::write(&file, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&file)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&file, perms)?;
    }
    Ok(())
}

/// (Re)write `composer`, `wp`, `node`, `npm` shims into `{tools}/bin`.
pub fn write_shims(paths: &Paths, state: &AppState) -> Result<PathBuf> {
    let dir = bin_dir(paths);
    fs::create_dir_all(&dir)?;
    let tools = &paths.tools;
    let php = managed_php(paths, state);
    let php_ref = php.as_ref().map(|(b, i)| (b.as_path(), i.as_deref()));
    let node_dir = node_bin(state).and_then(|p| p.parent().map(Path::to_path_buf));
    let nd = node_dir.as_deref();
    if cfg!(windows) {
        write_shim(&dir, "composer", &windows_php_phar_shim(tools, "composer", "composer.phar", php_ref))?;
        write_shim(&dir, "wp", &windows_php_phar_shim(tools, "wp", "wp-cli.phar", php_ref))?;
        write_shim(&dir, "node", &windows_node_shim(tools, "node", nd))?;
        write_shim(&dir, "npm", &windows_node_shim(tools, "npm", nd))?;
    } else {
        write_shim(&dir, "composer", &unix_php_phar_shim(tools, "composer", "composer.phar", php_ref))?;
        write_shim(&dir, "wp", &unix_php_phar_shim(tools, "wp", "wp-cli.phar", php_ref))?;
        write_shim(&dir, "node", &unix_node_shim(tools, "node", nd))?;
        write_shim(&dir, "npm", &unix_node_shim(tools, "npm", nd))?;
    }
    Ok(dir)
}

pub fn path_block(bin: &Path) -> String {
    format!(
        "{PATH_BEGIN}\nexport PATH=\"{}:$PATH\"\n{PATH_END}\n",
        bin.display()
    )
}

/// Insert or replace the marked DevStack block in a shell profile.
pub fn upsert_marked_block(content: &str, block: &str) -> String {
    let stripped = remove_marked_block(content);
    let mut out = stripped;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    out
}

pub fn remove_marked_block(content: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in content.lines() {
        if line.trim() == PATH_BEGIN {
            skipping = true;
            continue;
        }
        if skipping {
            if line.trim() == PATH_END {
                skipping = false;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Add/remove `{tools}/bin` on the user's PATH. Only called when the user opts in/out.
pub fn apply_path_opt_in(paths: &Paths, enabled: bool) -> Result<()> {
    let bin = bin_dir(paths);
    #[cfg(windows)]
    {
        let b = bin.display().to_string().replace('\'', "''");
        let script = if enabled {
            format!(
                "$p=[Environment]::GetEnvironmentVariable('Path','User'); \
                 if (($p -split ';') -notcontains '{b}') {{ [Environment]::SetEnvironmentVariable('Path', ('{b};'+$p), 'User') }}"
            )
        } else {
            format!(
                "$p=[Environment]::GetEnvironmentVariable('Path','User'); \
                 [Environment]::SetEnvironmentVariable('Path', (($p -split ';' | Where-Object {{ $_ -ne '{b}' }}) -join ';'), 'User')"
            )
        };
        let status = crate::services::process::command("powershell")
            .args(["-NoProfile", "-Command", &script])
            .status()?;
        if !status.success() {
            return Err(DevStackError::Message("failed to update user PATH".into()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let home = dirs::home_dir()
            .ok_or_else(|| DevStackError::Message("no home directory".into()))?;
        for name in [".zshrc", ".bashrc", ".profile"] {
            let file = home.join(name);
            if !file.exists() {
                continue;
            }
            let current = fs::read_to_string(&file)?;
            let next = if enabled {
                upsert_marked_block(&current, &path_block(&bin))
            } else {
                remove_marked_block(&current)
            };
            if next != current {
                fs::write(&file, next)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsStatus {
    pub bin_dir: String,
    pub composer: bool,
    pub wp_cli: bool,
    pub node: bool,
    pub shims: bool,
    pub opt_in_path: bool,
}

pub fn tools_status(paths: &Paths, state: &AppState) -> ToolsStatus {
    let dir = bin_dir(paths);
    let shim = |n: &str| dir.join(if cfg!(windows) { format!("{n}.cmd") } else { n.to_string() });
    ToolsStatus {
        bin_dir: dir.display().to_string(),
        composer: composer_phar(paths).exists(),
        wp_cli: paths.tools.join("wp-cli.phar").exists(),
        node: node_bin(state).is_some(),
        shims: shim("composer").exists() && shim("wp").exists(),
        opt_in_path: state.sites_settings.opt_in_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_shims_reference_tools_dir_and_fall_back() {
        let tools = PathBuf::from("/data/DevStack/tools");
        let phar = tools.join("composer.phar");
        let s = unix_php_phar_shim(&tools, "composer", "composer.phar", Some((Path::new("/p/php"), None)));
        // Use Path::display so the assertion matches Windows `\…` vs Unix `/…`.
        assert!(s.contains(&phar.display().to_string()), "{s}");
        assert!(s.contains(&Path::new("/p/php").display().to_string()));
        assert!(s.contains("exec composer"));
        let n = unix_node_shim(&tools, "npm", Some(Path::new("/n/bin")));
        assert!(n.contains(&format!("{}/npm", Path::new("/n/bin").display())));
        assert!(n.contains(&tools.display().to_string()));
    }

    #[test]
    fn windows_shims_reference_tools_dir() {
        let tools = PathBuf::from("C:/DevStack/tools");
        let s = windows_php_phar_shim(&tools, "wp", "wp-cli.phar", Some((Path::new("C:/php/php.exe"), None)));
        assert!(s.contains("wp-cli.phar"));
        assert!(s.contains(&tools.display().to_string()), "{s}");
        assert!(s.contains("%*"));
    }

    #[test]
    fn write_shims_creates_four_executables() {
        let root = std::env::temp_dir().join(format!("devstack-tools-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths::from_root(root.clone()).unwrap();
        let dir = write_shims(&paths, &AppState::default()).unwrap();
        for n in ["composer", "wp", "node", "npm"] {
            let f = dir.join(if cfg!(windows) { format!("{n}.cmd") } else { n.into() });
            let body = fs::read_to_string(&f).unwrap();
            assert!(body.contains(&paths.tools.display().to_string()), "{n}");
        }
        assert!(tools_status(&paths, &AppState::default()).shims);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn marked_path_block_is_idempotent_and_removable() {
        let block = path_block(Path::new("/t/bin"));
        let once = upsert_marked_block("export A=1\n", &block);
        let twice = upsert_marked_block(&once, &block);
        assert_eq!(once, twice);
        assert!(once.contains("/t/bin"));
        assert_eq!(remove_marked_block(&twice), "export A=1\n");
    }
}
