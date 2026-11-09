# DevStack v0.3.0 — test report

Date: 2026-11-09  
Branch: `cursor/release-0-3-0-b7b6`  
Principles kept: isolation-first busy-port fallback (never touch foreign listeners; never auto-grab 80/443); **no stack autostart**.

## Features covered

| # | Feature | Backend | UI | Automated coverage |
|---|---------|---------|----|--------------------|
| 1 | One-click services (Redis, Memcached, Meilisearch, Typesense, MinIO, Reverb) | `packages/service_bins.rs`, `extras_v030.rs`, `ports.rs` | `ServicesPage` | Unit ports/remap; E2E Meilisearch start/stop |
| 2 | Per-site php.ini + sliders | `site_php_ini.rs` | `SitePhpIniPanel` | Unit merge/validate; E2E live probe |
| 3 | Site templates (Laravel/Symfony/Statamic/Drupal/Joomla/Woo) | `site_templates.rs` | `TemplatePickerModal` | Unit scaffold; E2E offline scaffold + DB |
| 4 | Hosting sync SFTP/SSH | `hosting_sync.rs` + keyring | `HostingPage` | Unit excludes/plan/rsync args |
| 5 | Custom TLDs + aliases/wildcard | `tld.rs`, `vhost_ext.rs`, `tls.rs` | `SiteDomainPanel` | Unit hosts/SAN; E2E hosts/cert |
| 6 | Per-site logs + Laravel queue | `site_logs.rs` | `SiteLogsPanel` | Unit parsers/filters/cursor |
| 7 | Mobile preview LAN + QR | `mobile_preview.rs` | `MobilePreviewModal` | Unit URL/QR; E2E bind |
| 8 | Perf dashboard (CPU/RAM, slow log, SPX) | `metrics.rs` | `PerformancePage` | Unit snapshot/slow-log/SPX |
| 9 | Config export/import | `config_transfer.rs` | General page | Unit + E2E archive roundtrip |
| P | Performance (top priority) | parallel stack, backoff polls, deferred bootstrap, AppData snapshot | instant tabs from cache | `devstack_smoke --perf` / `--e2e` gate |

## Local results (agent VM, Linux x86_64, 2026-11-09)

```
cargo test --lib
→ 148 passed; 0 failed

npm test (vitest)
→ 15 files / 60 tests passed

npx tsc --noEmit
→ clean

devstack_smoke --perf  (DEVSTACK_DATA_DIR=/tmp/perfwork/data, release binary)
→ SMOKE PASS: perf bench within budget
```

### Performance — before / after (same Linux agent VM, managed MariaDB + Nginx)

Methodology: warm-up start+stop, then best of two warm `apply_stack` runs. Before = commit `1627563` + bench harness; after = `2373ca5` lineage. See `docs/performance.md`.

| Metric | Target | CI budget (unix / Win) | Before | After (measured) |
|--------|--------|------------------------|--------|------------------|
| Warm `apply_stack` | < 3 s | 4000 / 8000 ms | ~664–679 ms | **260 ms** (warm_runs=[260, 354]) |
| Cold `apply_stack` (post warm-up stop) | — | — | ~710–1170 ms | **710 ms** |
| `stop_stack` | — | — | ~5880 ms | **1029 ms** |
| HTTPS toggle | < 1 s | 3000 / 8000 ms | 18–55 ms | **16 ms** |
| Web-server toggle (nginx↔apache) | < 1 s | 3000 / 8000 ms | ~425 ms | **54 ms** |

Machine-readable line from this run:

```
PERF_BENCH apply_stack_warm_ms=260 https_toggle_ms=16 apache_toggle_ms=54 cold_apply_stack_ms=710 stop_stack_ms=1029 warm_runs=[260, 354]
```

### Perf optimizations shipped

- Bootstrap: `regenerate_all_php_ini` / hosts sync deferred off the window-create path
- Stack start: DNS/CA then DB ∥ PHP versions in parallel; nginx/apache after; skip re-starting live PHP
- Readiness: short backoff polls (5→50 ms) instead of fixed 500 ms sleeps
- Stop: parallel service shutdown; short poll instead of 400 ms fixed sleeps
- Site toggles: skip WP URL repair when unchanged; reload-only front-end
- Frontend: `localStorage` sites/stack snapshot for instant paint; boot without probe then background refresh; `ensureSites()` avoids duplicate mount load
- CI: `run_perf_bench` fails on regression above budgets

## CI

| Check | Result |
|-------|--------|
| Windows HTTPS smoke (`devstack_smoke --e2e`, incl. v0.3.0 + perf bench) | _filled after CI_ |
| macOS smoke | _filled after CI_ |

## Known limitations

- **MinIO**: community fork (`pgsty/minio`); ports 9100/9101 (9000 collides with php-cgi base)
- **Memcached**: adopts PATH binary (no official static builds)
- **Typesense**: macOS/Linux only
- **Reverb**: runs `php artisan reverb:start` in a Laravel site with `laravel/reverb`
- **Hosting sync**: OpenSSH + rsync (no libssh2); secrets in OS keychain via `keyring`
- **SPX**: requires `php-spx` extension when present; otherwise status explains install
- **Wildcard DNS**: hosts file can't hold wildcards — local DNS daemon + cert SANs
