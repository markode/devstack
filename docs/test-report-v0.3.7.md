# DevStack v0.3.7 — test report

## Goal

Make macOS one-click installs work without Homebrew, Xcode CLT, or env vars, and
stop the macOS CI smoke from masking that gap with `brew install`.

## Changes

- **PHP (macOS/Linux):** baked `packages-macos-v1` manifest + static-php.dev
  cli+fpm auto-discovery; install merges both binaries; no more null-URL stubs.
  Windows NTS zip extract path restored (regression fix).
- **MariaDB (macOS):** **real MariaDB** (`mariadbd`) from CI-built relocatable
  packages (`packages-macos-v1` / smoke local build). MariaDB.org no longer ships
  macOS binaries. **MySQL Community is not used as a MariaDB substitute.**
- **Apache / Nginx:** default to `packages-macos-v1` prebuilts; nginx only
  compiles when the archive is source (`configure` present).
- **Memcached / Redis:** prefer prebuilt packages; brew/PATH is optional fallback.
- **UI:** PHP page wires `onUnavailable` so “Click Install for details” shows the
  real reason in an error banner (Services page already did).
- **Hosts + CA:** first Start uses one elevated `hosts-and-trust-ca` helper
  prompt on macOS/Linux.
- **CI:** `packages-macos.yml` builds/publishes relocatable tarballs on
  `macos-latest`; `macos-smoke.yml` downloads or builds packages, hides Homebrew
  from PATH, installs PHP/MariaDB/Nginx/Apache through the app (`--macos-managed`),
  fails if any of those packages lacks a download URL.

## MariaDB on macOS (explicit)

| Approach | Status |
| --- | --- |
| MariaDB.org REST macOS artifacts | Unavailable (no files published) |
| MySQL Community in the MariaDB slot | **Rejected** — not used |
| CI relocatable MariaDB (`mariadbd` + bundled dylibs) | **Required path** via `packages-macos-v1` / smoke builder |

The package tree is produced by installing the Homebrew `mariadb` formula on the
*builder*, copying the cellar, and rewriting Mach-O deps to `@loader_path` so
end users do not need Homebrew. The installed binary is still MariaDB Server.

## Verification

| Check | Result |
| --- | --- |
| `cargo test --lib` | pass (local agent) |
| Windows HTTPS smoke | see CI |
| macOS smoke (managed, brew hidden, real MariaDB) | see CI |
| Windows ×2 + macOS ×2 release assets | published with `v0.3.7` |

## Notes

- PHP 7.4 appears only when an artifact exists (static-php.dev currently starts
  at 8.0; packages workflow can add older channels later).
- `packages-macos-v1` must be published (workflow on `main`) before a clean Mac
  can install without the smoke local HTTP server.
- Intel (`macos-13`) package builds are best-effort (`continue-on-error`); arm64
  (`macos-latest`) is required.
