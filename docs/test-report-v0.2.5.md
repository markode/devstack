# DevStack test report — v0.2.5

Living report. Updated as CI iterations complete.

## Goals

Ship only after windows-latest + macos-latest E2E (real downloads, real backend) are green, covering user flows listed below, with site-setting toggles typically &lt;2s (CI assert &lt;3s).

## Bugs found and fixed in this release

| Bug | Fix |
| --- | --- |
| Create site loader hidden behind modal | In-dialog progress via `AsyncModal` / `useAsyncJob`; cancel blocked while busy; inline errors |
| Windows Apache WP: `No input file specified` | Trailing-slash `SetHandler` + `ProxyFCGISetEnvIf` rebuild of `SCRIPT_FILENAME`; `cgi.fix_pathinfo=1`; no `doc_root` |
| HTTPS toggle ~1 min on Windows | Skip hosts/CA when unchanged; CA trust marker; rcgen cert cache; reload only affected front-end |
| Apache→Nginx left `:8444` dead / wrong Open URL | Targeted reload + WP URL port rewrite; Open URL from live stack ports |
| phpMyAdmin password in URL + CGI failure | One-time token signon (`pma-tokens/`, 60s TTL); password never in query string |
| Double hosts/CA sync on every env change | `apply_site_environment_change` fast path |

## Automated coverage

### Unit (cargo test --lib)

- paths_util, php.ini (`cgi.fix_pathinfo`), httpd ProxyFCGISetEnvIf, tls cert cache, pma token mint, ports, dns hosts merge, process CREATE_NO_WINDOW flags

### Frontend (vitest)

- `AsyncModal`: busy aria, progress banner, inline error, cancel disabled while busy
- `useAsyncJob` surfaces thrown errors

### E2E (`devstack_smoke --e2e` on windows-latest + macos-latest)

| Flow | Status |
| --- | --- |
| Fresh data dir / reset sites | covered |
| Install PHP + Nginx + MariaDB + phpMyAdmin + Apache | covered |
| Start/stop stack | covered |
| WordPress create (Apache) HTTP+HTTPS 200, no “No input file specified” | covered |
| Template + duplicate | covered |
| DB backup streaming | covered |
| Site setting perf: HTTPS / server Apache↔Nginx / PHP &lt;3s | covered (`PERF SUMMARY`) |
| Install Node / Postgres / Mongo / Redis / Mailpit | `--services` (extend e2e follow-up) |
| Port conflict + coexistence | unit + Services UI (extend e2e follow-up) |
| Hosts sync + CA trust | launch + repair paths |
| welcome.test seed | bootstrap |
| Empty site / link folder | UI + create_site |
| phpMyAdmin token signon 200 | unit mint; e2e probe follow-up |
| Delete site cleans vhosts/hosts | `delete_site` command |
| App restart persists state | state.json round-trip |

## Perf timings (filled from CI logs)

| Platform | https=false | https=true | server=nginx | server=apache | php switch |
| --- | --- | --- | --- | --- | --- |
| windows-latest | _pending_ | _pending_ | _pending_ | _pending_ | _pending_ |
| macos-latest | _pending_ | _pending_ | _pending_ | _pending_ | _pending_ |

## CI runs

_Pending first green PR checks after push._
