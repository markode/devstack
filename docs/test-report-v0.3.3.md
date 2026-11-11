# DevStack v0.3.3 — Windows orphan nginx + empty-site hotfix

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-3-orphans-b7b6`  
PR: https://github.com/markode/devstack/pull/28  
Head: `0a024dfb056d8be8f06db176f85564733db73e2b`  
Source diag: user Windows 11 machine on v0.3.2 (`devstack-diag-032.zip`) with WAMP on `:80`.

## Symptoms (v0.3.2)

1. **CRITICAL:** ~20 `nginx.exe` PIDs all `LISTENING` on `0.0.0.0:8080` / `:8443`. Orphans never reclaimed (`wmic` missing). Double `start_dev_stack` within ~6 ms both succeeded.
2. Empty site `acme-dashboard` → nginx `403 Forbidden`; tile showed **Running**.
3. WordPress `test` (Apache `:8088`) → generic probe warning; no redirect / PHP-FCGI reason.
4. Open of an HTTPS port used `http://` → Apache `400` “plain HTTP to an SSL-enabled server port”.
5. Parked `my-site` with `phpVersion: null` + missing hosts → HTTPS probe warning.
6. NSIS upgrade over a running app could leave the old `0.3.0` exe in place.

## Fix summary

| Area | Change |
|------|--------|
| Orphans | PowerShell CIM reclaim by exe path under data dir; `keep_pids` so warm apply does not kill live MariaDB/PHP |
| Job objects | kernel32 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` FFI |
| Start mutex | Serialize start/stop/apply |
| Port owner | Owned listeners only (workers OK); Windows orphan pile-up cap |
| Empty site | Placeholder + tile **No index file** |
| Probe | curl `-L` + concrete reasons |
| PHP / hosts | Default installed PHP; repair hosts on Open/park |
| HTTPS Open | Force `https://` on SSL ports |
| NSIS | `taskkill` DevStack before replace |

## CI results (PR #28 @ `0a024df`)

| Check | Result | Run |
|-------|--------|-----|
| macOS smoke | **PASS** | [43002686995](https://github.com/markode/devstack/actions/runs/43002686995) |
| Windows HTTPS smoke + matrix | **PASS** (51 pass / 0 fail / 2 skipped) | [43002686976](https://github.com/markode/devstack/actions/runs/43002686976) |
| Warm apply | **3374 ms** (< 5000 ms) | matrix `stack.warm_apply` |

### hotfix033 scenarios (actual)

| Id | Result | Detail |
|----|--------|--------|
| `hotfix033.empty_placeholder` | **PASS** 3595ms | `url=http://acme-empty.test:8080 state=warning` (No index file) |
| `hotfix033.single_port_owner` | **PASS** 75ms | `http=8080 https=8443` |
| `hotfix033.null_php_default` | **PASS** 0ms | `php=Some("8.5") expected=8.5` |
| `hotfix033.https_open_scheme` | **PASS** 0ms | `https://welcome.test:8443` |
| `hotfix033.wamp_port80_untouched` | **PASS** 730ms | foreign listener still listening after reclaim |

`MATRIX DONE pass=51 fail=0 skipped=2 total_ms=43932`

## Release assets

| Asset | Notes |
|-------|-------|
| `DevStack_0.3.3_aarch64.dmg` | macOS arm64 |
| `DevStack_0.3.3_x64.dmg` | macOS x64 |
| `DevStack_0.3.3_x64-setup.exe` | Windows NSIS |
| `DevStack_0.3.3_x64_en-US.msi` | Windows MSI |

## Upgrade note

Install over a running DevStack: NSIS preinstall force-closes `DevStack.exe` (job object tears down children) before file replace.
