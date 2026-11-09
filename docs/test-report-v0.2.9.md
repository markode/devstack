# DevStack v0.2.9 — test report

Date: 2026-11-09  
Branch: `cursor/release-0-2-9-b7b6`  
Principles kept: isolation-first busy-port fallback (never touch foreign listeners; never auto-grab 80/443); **no stack autostart**.

## Features covered

| # | Feature | Implementation | Automated coverage |
|---|---------|----------------|--------------------|
| 1 | Sites status banner | `SitesStatusBanner` + stack `job-progress` phases | Vitest banner states; stack phase unit test |
| 2 | Site tile + quick actions | `SiteTile` (URL w/ port, PHP, server, Open/Folder/Terminal/DB/Logs/HTTPS) | Vitest tile + `formatSiteUrl` |
| 3 | Empty-state onboarding | `SitesEmptyState` (add folder / WP / import) | Vitest empty state |
| 4 | Parked folders | `parked.rs` sync + watcher; General UI | Unit sync; E2E `run_v029_features_smoke` |
| 5 | Tray / menu bar | `tray.rs` Start/Stop, sites, PHP switch, Quit→`stop_stack` | Compiles with `tray-icon`; Quit calls stop |
| 6 | Per-site PHP | Existing pools + tile selector | Existing PERF + E2E site settings |
| 7 | dump() + Xdebug | `dump.rs` receiver + helper; `xdebug_site.rs` | Unit parse/ring/launch.json; E2E xdebug toggle |
| 8 | Share (cloudflared) | `share.rs` quick tunnel + WP URL rewrite | Unit asset/URL/args; E2E URL parse |
| 9 | Site backups | `site_backup.rs` files+DB, schedule, list/restore/delete | Unit list/prune/due; E2E backup create/delete |
| 10 | Mailpit tab | `MailpitPage` iframe + start | Existing services smoke opens 8025 |
| 11 | Clone site | `clone_site` alias of `duplicate_site` | Existing WP duplicate E2E |
| 12 | Bundled tools | Composer/WP-CLI/Node shims in `tools/bin` | Unit shim contents; E2E shim files |

## Local results (agent VM, 2026-11-09)

```
cargo test --lib
→ 73 passed; 0 failed

npm test (vitest)
→ 13 files / 35 tests passed

npx tsc --noEmit
→ clean

cargo check --bins
→ ok (devstack + devstack_smoke)
```

## CI expectations

Windows HTTPS smoke + macOS smoke (`devstack_smoke --e2e`) must both pass, including the new `run_v029_features_smoke` section (parked sync, dump helper, tool shims, full-site backup, xdebug hints, cloudflared asset/URL parse).

## Isolation reminders re-verified by unit tests

- `auto_separate_busy_ports` remaps every service field including redis/mailpit/dns
- Foreign listeners are never stopped
- Classic 80/443 notice is informational only

## Manual / tray notes

Tray menus and Cloudflare live tunnels require a GUI session / network; covered by unit tests and compile checks. Quit path stops the stack before exit.