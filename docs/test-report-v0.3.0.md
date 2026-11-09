# DevStack v0.3.0 — test report (skeleton)

Branch: `cursor/release-0-3-0-b7b6`  
Principles kept: isolation-first busy-port fallback (never touch foreign listeners; never auto-grab 80/443); **no stack autostart** (new services only start from the UI / `start_managed_service`).

## Features covered

| # | Feature | Backend | UI | Automated coverage |
|---|---------|---------|----|--------------------|
| 1 | One-click services (Redis listed, Memcached, Meilisearch, Typesense, MinIO, Reverb) | `packages/service_bins.rs`, `services/extras_v030.rs`, `extras.rs`, `ports.rs` | `ServicesPage` (new blocks, MinIO API+console ports, Reverb site picker) | Unit: default ports, candidate ports, `service_ports` == `extras` ports, busy-port remap, spawn plans; E2E `run_v030_features_smoke` starts/stops Meilisearch |
| 2 | Per-site php.ini | `site_php_ini.rs` (`sites/{id}/php.ini` + `php-merged.ini`, vhost `PHP_VALUE`, `.user.ini` on php-cgi) | `SitePhpIniPanel` (sliders + advanced keys) | Unit: validation, merge, sliders, set/get; E2E live probe through nginx → php-fpm |
| 3 | Site templates | `site_templates.rs` (laravel, symfony, statamic, drupal, joomla, woocommerce) | `TemplatePickerModal`, Sites toolbar + empty state | Unit: kind→commands, scaffold, offline create (`DEVSTACK_SKIP_TEMPLATE_NETWORK=1`); E2E scaffold + DB creation |
| 4 | Hosting sync | `hosting_sync.rs` (OpenSSH + rsync, `keyring`) | `HostingPage` | Unit: exclude matching, dry-run plan serialization, rsync args, keychain-only secrets |
| 5 | Custom TLDs + aliases | `tld.rs`, `vhost_ext.rs`, `tls.rs` (SAN fingerprint), `dns.rs` | `SiteDomainPanel`, General → default TLD | Unit: hosts block includes aliases, cert SAN regen, vhost server names; E2E hosts/cert/vhost |
| 6 | Per-site logs + Laravel queue | `site_logs.rs` | `SiteLogsPanel` (+ `SiteQueuePanel`) | Unit: parsers, filters, cursor poll/rotation, queue/Horizon |
| 7 | Mobile preview | `mobile_preview.rs` (nginx LAN server block, QR) | `MobilePreviewModal`, tile + detail action | Unit: URL builder, conf, QR, port pool; E2E live bind (soft-skip without LAN) |
| 8 | Perf dashboard | `metrics.rs` (sysinfo, slow log, SPX) | `PerformancePage` | Unit: snapshot, slow-log parser, SPX block; E2E snapshot non-empty |
| 9 | Config export/import | `config_transfer.rs` | General page buttons | Unit: archive roundtrip + meta + traversal guard; E2E roundtrip into scratch profile |

## Local results

> Filled by the release run. Placeholders below.

```
cargo test --lib          → _pending_
npm test (vitest)         → _pending_
devstack_smoke --v030     → _pending_
```

### Timings (perf agent fills)

| Metric | Budget | Measured |
|--------|--------|----------|
| Warm `apply_stack` | see `docs/performance.md` | _pending_ |
| HTTPS toggle | see `docs/performance.md` | _pending_ |
| Per-site php.ini apply (PHP restart + nginx reload) | — | _pending_ |
| Metrics snapshot (`get_metrics_snapshot`) | ~250 ms (two CPU samples) | _pending_ |
| Meilisearch start → ready | — | _pending_ |

## Known limitations / notes

- **MinIO**: upstream stopped publishing binaries (`dl.min.io` returns 410), so the package installs the community fork's release (`pgsty/minio`; binary may be named `silo`). Defaults API `:9100`, console `:9101` (9000 collides with the Windows php-cgi base port).
- **Memcached**: no official binaries — the package adopts `memcached` found on `PATH` (`brew`/`apt`/`choco`), with an actionable error otherwise.
- **Typesense**: macOS/Linux only (`dl.typesense.org`).
- **Reverb**: no binary; `php artisan reverb:start --host=127.0.0.1 --port=<reverbPort>` is run in the selected Laravel site (`laravel/reverb` required). Default port `8085`.
- **Hosting sync**: shells out to OpenSSH/rsync (no `ssh2`/libssh2 native build). Passwords live in the OS keychain (`keyring`) and reach OpenSSH via `SSH_ASKPASS`; without rsync the preview is local-only and transfers use `scp` (no excludes).
- **Per-site php.ini** on Windows php-cgi is applied through `.user.ini` (PERDIR keys only, cached up to 5 min).
- **Wildcard sub-domains** rely on the hosts file (no wildcards) plus the local DNS daemon; on macOS the resolver only covers `.test`.
