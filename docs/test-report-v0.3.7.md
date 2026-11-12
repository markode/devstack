# DevStack v0.3.7 — test report

## Goal

Make macOS one-click installs work without Homebrew, Xcode CLT, or env vars, and
stop the macOS CI smoke from masking that gap with `brew install`.

## Changes

- **PHP (macOS/Linux):** baked `packages-macos-v1` manifest + static-php.dev
  cli+fpm auto-discovery; install merges both binaries; no more null-URL stubs.
- **MariaDB (macOS):** packages manifest, else official MySQL Community tarballs
  from cdn.mysql.com (mysqld accepted by DevStack DB layer).
- **Apache / Nginx:** default to `packages-macos-v1` prebuilts; nginx only
  compiles when the archive is source (`configure` present).
- **Memcached / Redis:** prefer prebuilt packages; brew/PATH is optional fallback.
- **UI:** PHP page wires `onUnavailable` so “Click Install for details” shows the
  real reason in an error banner (Services page already did).
- **Hosts + CA:** first Start uses one elevated `hosts-and-trust-ca` helper
  prompt on macOS/Linux.
- **CI:** `packages-macos.yml` builds/publishes relocatable tarballs;
  `macos-smoke.yml` downloads or builds packages, hides Homebrew from PATH,
  installs PHP/MariaDB/Nginx/Apache through the app (`--macos-managed`), fails
  if any of those packages lacks a download URL.

## Verification

| Check | Result |
| --- | --- |
| `cargo test --lib` (package/php unit) | pass (local agent) |
| macOS smoke (managed, brew hidden) | see CI on PR / tag |
| Windows release assets (×2) + macOS DMGs (×2) | published with `v0.3.7` |

## Notes

- PHP 7.4 appears only when an artifact exists (static-php.dev currently starts
  at 8.0; packages workflow can add older channels later).
- `packages-macos-v1` must be published (workflow on `main`) before a clean Mac
  can install Apache/Nginx/Memcached prebuilts without the smoke local server.
- MariaDB.org no longer ships macOS binaries; the MariaDB slot uses MySQL
  Community on macOS when the packages release has no MariaDB row.
