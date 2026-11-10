# DevStack v0.3.1 — test report

Date: 2026-11-10  
Tip: `8e98136` on `cursor/release-0-3-1-b7b6` (PR #25)  
Principles: isolation-first busy-port fallback; no stack autostart; soft-skips are not passes.

## CI gates (required before publish)

| Check | Result | Run |
|-------|--------|-----|
| Windows HTTPS smoke | **success** | [42722349702](https://github.com/markode/devstack/actions/runs/42722349702) |
| macOS smoke | **success** | [42722349604](https://github.com/markode/devstack/actions/runs/42722349604) |

Windows feature matrix on that tip: **46 passed · 0 failed · 2 skipped** (`MATRIX DONE … total_ms=31513`).

Required gates:

| Gate | Status | Detail |
|------|--------|--------|
| `hosting.sftp_live` | **pass** | auth=password dry=2 push_files=2 pull_files=0 tool=sftp |
| `svc.redis_required` | **pass** | installed=5.0.14.1 |

Skipped (explicit, not passes): `svc.memcached`, `svc.minio` — not installed in this CI data dir.

## Performance (measured on tip)

| Metric | Windows | macOS | Budget |
|--------|---------|-------|--------|
| Warm `apply_stack` | **2516 ms** (warm_runs=[2516, 2524]) | **363 ms** (warm_runs=[410, 363]) | < 5000 / 4000 ms |
| HTTPS toggle | 325 ms | 84 ms | < 1000 ms |
| Apache ↔ nginx toggle | 303 ms | 109 ms | < 1000 ms |
| Cold `apply_stack` | 2528 ms | 1776 ms | — |
| Matrix warm apply | 2444 ms | — | < 5000 ms |

```
PERF_BENCH apply_stack_warm_ms=2516 https_toggle_ms=325 apache_toggle_ms=303 cold_apply_stack_ms=2528 stop_stack_ms=136 warm_runs=[2516, 2524]
PERF_BENCH apply_stack_warm_ms=363 https_toggle_ms=84 apache_toggle_ms=109 cold_apply_stack_ms=1776 stop_stack_ms=399 warm_runs=[410, 363]
```

## Hang / warm-path root causes fixed

1. **Win11 AppHangB1:** nested `async_runtime::block_on` in tray setup + missing window title → non-blocking tray, forced `DevStack` title, stall watchdog.
2. **macOS CI hang:** skip osascript admin elevation under CI.
3. **Warm apply ~47s:** keep DB/DNS/PHP up on `stop_web_stack`; reuse live MariaDB; skip headless UAC; bound `port_is_free`; CI-only Defender exclusions (never silent in the shipped app).
4. **CA trust honesty:** mark trusted only after OS store verification; UAC decline defers (Retry UI); no re-prompt every apply.
5. **Windows matrix:** run off Tokio worker; accept `C:/` SFTP remotes; raise nginx `server_names_hash_bucket_size`.

## Windows feature matrix (actual)

Generated: `2026-11-10T00:10:51Z` · os=`windows` arch=`x86_64` version=`0.3.1` · total `31513` ms

| Status | Feature | ms | Detail |
| --- | --- | ---: | --- |
| `pass` | Orphan reclaim (`startup.orphans`) | 1 | ok |
| `pass` | Startup phase logger (`startup.phases`) | 0 | ok |
| `pass` | Start stack (`stack.start`) | 3517 | nginx=true db=true http=8080 |
| `pass` | Stop web stack (`stack.stop_web`) | 124 | ok |
| `pass` | Warm apply_stack (`stack.warm_apply`) | 2444 | 2444ms |
| `pass` | Create blank site (`sites.blank`) | 2579 | matrix-blank |
| `pass` | HTTPS toggle (`toggle.https`) | 610 | 303ms |
| `pass` | Nginx ↔ Apache toggle (`toggle.web_server`) | 508 | 508ms |
| `pass` | Hosts block status (`hosts.status`) | 192 | ok=true |
| `pass` | Templates (Laravel/Symfony/Statamic/Drupal/Joomla/Woo) | ~46–53 each | soft_failed=false |
| `pass` | Parked folder sync (`sites.parked_sync`) | 0 | added=0 |
| `pass` | Custom TLD + aliases (`sites.tld_aliases`) | 0 | aliases=["www.test","api.test"] |
| `pass` | Per-site php.ini sliders (`php.ini_sliders`) | 345 | memory=256 |
| `pass` | Database engine (`db.engine`) | 4 | mariadb |
| `pass` | Redis start/stop (`svc.redis`) | 2673 | ok |
| `pass` | Mailpit start/stop (`svc.mailpit`) | 4692 | ok |
| `skipped` | Memcached (`svc.memcached`) | 0 | SKIPPED: not installed in this CI data dir |
| `pass` | Meilisearch (`svc.meilisearch`) | 6756 | ok |
| `skipped` | MinIO (`svc.minio`) | 0 | SKIPPED: not installed in this CI data dir |
| `pass` | Typesense unavailable on Windows (`svc.typesense`) | 102 | rejected |
| `pass` | Full-site backup (`backup.full`) | 32 | bytes=773 |
| `pass` | Config export (`config.export`) | 29 | sites=16 |
| `pass` | Hosting tools (`hosting.tools`) | 23 | ssh=true rsync=false |
| `pass` | Tray menu + handlers | 1 / 126 | 20 actions |
| `pass` | Metrics / logs / xdebug / dump / share / mobile | — | pass |
| `pass` | Busy-port isolation (`ports.isolation_notice`) | 1243 | remapped |
| `pass` | Local CA / PMA SSO / SPX→Xdebug offer | — | pass |
| `pass` | Hosting SFTP live (`hosting.sftp_live`) | 1409 | required gate |
| `pass` | Clone site (`sites.clone`) | 2697 | matrix-clone.test |
| `pass` | Config import (`config.import`) | 24 | sites_added=16 |
| `pass` | Hosting dry-run without rsync | 28 | built-in SFTP |
| `pass` | WordPress create/reuse | 0 | reused demo |
| `pass` | Redis required (`svc.redis_required`) | 0 | required gate |
| `pass` | Load 0.2.9-shaped state (`upgrade.state_v029`) | 1 | ok |

## Guardrails shipped in 0.3.1

- **Defender exclusions:** CI workflow only for automatic exclusions; Settings → Performance opt-in (default OFF, UAC, removed on toggle-off / uninstall).
- **CA trust:** verified in CurrentUser/LocalMachine Root (or platform store) before marking trusted; “HTTPS certificate not trusted” + Retry when deferred.
