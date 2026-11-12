# DevStack v0.3.7 — test report

## Goal

Make macOS one-click installs work without Homebrew, Xcode CLT, or env vars, and
stop the macOS CI smoke from masking that gap with `brew install`.

## Changes

- **PHP (macOS/Linux):** baked `packages-macos-v1` manifest; relocatable Homebrew
  `php@8.4` cellar packaging with recursive dylib bundling (`@loader_path` and
  `@rpath`, including ICU/brotli). Manifest builds are authoritative over
  static-php.dev so listed installs include `php-fpm`. Windows NTS zip extract
  path restored (regression fix).
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
  `macos-latest`; `macos-smoke.yml` builds or downloads packages, builds the
  smoke binary, hides Homebrew from PATH, installs PHP/MariaDB/Nginx/Apache
  through the app (`--macos-managed`), fails if any of those packages lacks a
  download URL.

## MariaDB on macOS (explicit)

| Approach | Status |
| --- | --- |
| MariaDB.org REST macOS artifacts | Unavailable (no files published) |
| MySQL Community in the MariaDB slot | **Rejected** — not used |
| CI relocatable MariaDB (`mariadbd` + bundled dylibs) | **Required path** via `packages-macos-v1` / smoke builder |

The package tree is produced by installing the Homebrew `mariadb` formula on the
*builder*, copying the cellar, and rewriting Mach-O deps to `@loader_path` so
end users do not need Homebrew. The installed binary is still MariaDB Server.

## Verification (PR #32 / `cursor/macos-packages-0-3-7-b7b6`)

| Check | Result | Run |
| --- | --- | --- |
| packages-macos `build (macos-latest, aarch64)` | **pass** | [43220557285](https://github.com/markode/devstack/actions/runs/43220557285) |
| macOS smoke (managed, brew hidden, real MariaDB, `.test:8080`) | **pass** | [43220557289](https://github.com/markode/devstack/actions/runs/43220557289) |
| Windows HTTPS smoke | **pass** | [43220557283](https://github.com/markode/devstack/actions/runs/43220557283) |
| Windows ×2 + macOS ×2 release assets | published with `v0.3.7` after tag | — |

## Notes

- PHP 7.4 appears only when an artifact exists (packages workflow can add older
  channels later; current default package is Homebrew `php@8.4`).
- `packages-macos-v1` must be published (workflow on `main`) before a clean Mac
  can install without the smoke local HTTP server.
- Intel (`macos-13`) package builds were dropped from the required matrix (runner
  queue hang); arm64 (`macos-latest`) is required and ships first.
