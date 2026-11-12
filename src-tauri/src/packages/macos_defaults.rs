//! Baked-in default manifest URLs for macOS/Linux one-click installs.
//!
//! Override any of these with `DEVSTACK_*_MANIFEST_URL` or the matching
//! `AppState` field. The `packages-macos-v1` release is produced by
//! `.github/workflows/packages-macos.yml` and contains relocatable prebuilt
//! trees (no Homebrew / Xcode required on the user's machine).

/// GitHub release that hosts prebuilt macOS package tarballs + `manifest.json`.
pub const PACKAGES_RELEASE_TAG: &str = "packages-macos-v1";

/// Combined manifest listing PHP / Apache / Nginx / MariaDB / Memcached / Redis.
pub const DEFAULT_COMBINED_MANIFEST_URL: &str =
    "https://github.com/markode/devstack/releases/download/packages-macos-v1/manifest.json";

/// static-php.dev common builds (cli + fpm). Used when the packages release is
/// unreachable so the PHP page still shows real download URLs.
pub const STATIC_PHP_COMMON: &str = "https://dl.static-php.dev/static-php-cli/common/";

/// Channels we always try to offer on macOS/Linux.
pub const PHP_CHANNELS: &[&str] = &["8.5", "8.4", "8.3", "8.2", "8.1", "8.0", "7.4"];

pub fn arch_label() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => other,
    }
}

pub fn arch_aliases(a: &str) -> bool {
    let host = std::env::consts::ARCH;
    a == host
        || (host == "x86_64" && matches!(a, "x64" | "amd64" | "x86_64"))
        || (host == "aarch64" && matches!(a, "arm64" | "aarch64"))
}

pub fn os_aliases(o: &str, host: &str) -> bool {
    o.eq_ignore_ascii_case(host)
        || (host == "macos" && matches!(o.to_ascii_lowercase().as_str(), "darwin" | "osx" | "macos"))
        || (host == "windows" && matches!(o.to_ascii_lowercase().as_str(), "win" | "windows"))
        || (host == "linux" && o.eq_ignore_ascii_case("linux"))
}
