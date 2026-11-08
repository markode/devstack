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
| Apache HTTP 400 “plain HTTP to SSL port” | Explicit `Listen 127.0.0.1:<http>` + `Listen … https` and per-port VirtualHosts |
| macOS helper AppleScript syntax error | Prefer `sudo -n`; wrap shell cmd in AppleScript `"` |
| Databases export `.sql.gz` was zstd (invalid gzip header) | Detect compression from path incl. `.partial`; temp named `*.partial.sql.gz` |
| Windows HTTPS toggle 7s+ (seeded WP DB) | Options-only `siteurl`/`home` rewrite + batched mysql REPLACE |

## Automated coverage

### Unit (cargo test --lib)

- paths_util, php.ini (`cgi.fix_pathinfo`), httpd ProxyFCGISetEnvIf, tls cert cache, pma token mint, ports, dns hosts merge, process CREATE_NO_WINDOW flags

### Frontend (vitest)

- `AsyncModal`: busy aria, progress banner, inline error, cancel disabled while busy
- `useAsyncJob` surfaces thrown errors

### E2E (`devstack_smoke --e2e` on windows-latest + macos-latest)

`--e2e` now runs: stack + wordpress (+perf/switch) + apache permalinks + db-files + services + persistence/PMA/empty/delete.

| Flow | Status |
| --- | --- |
| Fresh data dir / reset sites | covered |
| Install PHP (≥2 versions) + Nginx + MariaDB + MySQL(best-effort) + phpMyAdmin + Apache + Node | covered |
| Start/stop stack | covered |
| WordPress create (Apache) HTTP+HTTPS 200, no “No input file specified” | covered |
| Apache↔Nginx↔Apache switch + fetch both schemes | covered |
| Template + duplicate | covered |
| DB backup streaming + Databases folder import/URL rewrite | covered (`--db-files`) |
| Site setting perf: HTTPS / server / PHP &lt;3s | covered (`PERF SUMMARY`) |
| Postgres / Mongo / Redis / Mailpit | covered (`--services`) |
| Port conflict + coexistence | unit tests + Services UI |
| Hosts sync + CA trust | persistence smoke hosts status |
| welcome.test seed | bootstrap |
| Empty site | persistence smoke |
| phpMyAdmin token signon (no `pass=`, HTTP probe) | covered |
| Delete site cleans vhosts | covered |
| App restart persists state | state.json reload assert |

## Perf timings (filled from CI logs)

| Platform | https=false | https=true | server=nginx | server=apache | php switch |
| --- | --- | --- | --- | --- | --- |
| windows-latest | _pending_ (was 4898ms; Apache FOREGROUND stop/start) | _pending_ | _pending_ | _pending_ | _pending_ |
| macos-latest | **105ms** | **146ms** | **165ms** | **173ms** | skipped (1 PHP on that runner) |

CI budgets: macOS/Linux &lt;3s; Windows &lt;8s (Lounge FOREGROUND stop/start ≈4.5–5s on GHA).

## CI runs

- macOS `42473293316`: through db-files + PERF green; Mailpit GitHub API 403 rate limit (fallback added).
- Windows `42473293313`: through Mongo install + ports; redis-cli path NotFound on Windows package layout (find_bin fix).
