# DevStack v0.3.3 — Windows orphan nginx + empty-site hotfix

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-3-orphans-b7b6`  
Source diag: user Windows 11 machine on v0.3.2 (`devstack-diag-032.zip`) with WAMP on `:80`.

## Symptoms (v0.3.2)

1. **CRITICAL:** ~20 `nginx.exe` PIDs all `LISTENING` on `0.0.0.0:8080` / `:8443` (and IPv6). Orphans from prior runs never reclaimed (`wmic` missing → `orphan scan failed: program not found`). Double `start_dev_stack` within ~6 ms both succeeded.
2. Empty site `acme-dashboard` → nginx `403 Forbidden` (directory index forbidden); tile showed **Running**.
3. WordPress `test` (Apache `:8088`) → tile warning “Port open but … did not return a valid HTTP response”; probe did not follow redirects / surface PHP-FCGI reason.
4. Open of an HTTPS port used `http://` → Apache `400` “plain HTTP to an SSL-enabled server port”.
5. Parked `my-site` with `phpVersion: null` (“System”) + missing `my-site.test` hosts entry → HTTPS probe warning.
6. NSIS upgrade over a running app could leave the old `0.3.0` exe in place.

## Fix summary

| Area | Change |
|------|--------|
| Orphans | PowerShell `Get-CimInstance` / tasklist reclaim by **exe path under data dir** only (never WAMP) |
| Job objects | `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` on spawned nginx/httpd/php-cgi/mysqld |
| Start mutex | Serialize start/stop/apply; log call only after lock |
| Port owner | `assert_single_owned_listener` after nginx/apache ready — kill data-dir extras only |
| Empty site | Branded `index.html` placeholder (`<!-- devstack-empty-site -->`); tile **Warning / No index file** |
| Probe | curl `-L`, concrete failure / redirect / PHP-FCGI hints |
| PHP default | Parked / create_site default to installed PHP when null |
| Hosts | Auto `repairHosts` once on Open; `ensure_hosts_if_needed` after parked adds |
| HTTPS Open | Force `https://` when URL port is an SSL listener |
| NSIS | `PREINSTALL`/`PREUNINSTALL` `taskkill /IM DevStack.exe /T /F` before replace |

## CI scenarios (Windows matrix)

| Id | Expectation |
|----|-------------|
| `hotfix033.empty_placeholder` | Empty docroot → HTTP OK + tile warning `No index file` |
| `hotfix033.single_port_owner` | Exactly one owned listener on nginx http/https |
| `hotfix033.null_php_default` | Null PHP resolves to installed version |
| `hotfix033.https_open_scheme` | HTTPS site canonical Open URL starts with `https://` |
| `hotfix033.wamp_port80_untouched` | Foreign listener outside data dir survives reclaim |

## Results

| Check | Result | Notes |
|-------|--------|-------|
| Unit: orphan parse / placeholder / listeners | **PASS** | 5 + 3 + 1 Rust tests |
| Vitest: `siteUrl` HTTPS rewrite | **PASS** | 8 tests |
| Windows smoke + matrix | _pending CI_ | must include hotfix033.* |
| macOS smoke | _pending CI_ | static dylib gates still green |
| Release 4 assets | _pending tag_ | arm64/x64 dmg + exe/msi |

## Upgrade note

Install v0.3.3 over a running DevStack: the NSIS preinstall hook force-closes `DevStack.exe` (job object tears down children) before files are replaced, fixing silent “old exe left behind” upgrades from 0.3.0→0.3.2.
