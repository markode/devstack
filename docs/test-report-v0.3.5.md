# DevStack v0.3.5 — delete site / Apache HTTP / stop dedupe hotfix

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-5-delete-apache-stop-b7b6`  
PR: https://github.com/markode/devstack/pull/30  
Head: `c4dfa7603fdf6203b3fe5ec145dc157da047568d`  
Source: real Windows 11 on v0.3.4 (Delete Application no-op; Apache `test` https=false still broken; double `stop_dev_stack`).

## Symptoms (v0.3.4)

1. Site detail **Delete Application** did nothing — button was `disabled` with no handler; no IPC in `devstack.log`.
2. Apache site `test` (`https=false`) still unhealthy; log sources pointed at `*-ssl-*.log` (stale SSL assumption).
3. **Stop** invoked `stop_dev_stack` twice per click (~3 ms apart).

## Fix summary

| Area | Change |
|------|--------|
| Delete | Confirm dialog with remove-from-DevStack / drop DB+user / delete folder; backend cleans vhosts, hosts, certs, `sites/<id>` meta, logs; Windows retry for locked files; temp-start DB to drop; parked folder only if checked |
| Apache HTTP | Log sources + vhost use non-ssl paths when `https=false`; assert no `SSLEngine` / redirect |
| Stop | Coalesce `stopDevStack` like Start |

## CI results (PR #30 @ `c4dfa76`)

| Check | Result | Run |
|-------|--------|-----|
| macOS smoke | **PASS** | [43053636496](https://github.com/markode/devstack/actions/runs/43053636496) |
| Windows HTTPS smoke + matrix | **PASS** (58 pass / 0 fail / 2 skipped) | [43053636690](https://github.com/markode/devstack/actions/runs/43053636690) |
| Warm apply (Windows) | **2969 ms** (< 5000 ms) | `PERF_BENCH apply_stack_warm_ms=2969` |
| Warm apply (macOS) | **2146 ms** (< 4000 ms) | `PERF_BENCH apply_stack_warm_ms=2146` |
| Delete e2e | **PASS** (Win + mac) | `delete_site(empty-smoke) OK (folder removed, warnings=0)` |

### hotfix035 scenarios (actual)

| Id | Result | Detail |
|----|--------|--------|
| `hotfix035.apache_http_logs` | **PASS** 3022ms | `url=http://apache-http-only.test:8088` non-ssl error log |
| `hotfix035.delete_site_full` | **PASS** 1ms | `warnings=0` |

`MATRIX DONE pass=58 fail=0 skipped=2 total_ms=48283`

## Release assets

| Asset | Notes |
|-------|-------|
| `DevStack_0.3.5_aarch64.dmg` | macOS arm64 |
| `DevStack_0.3.5_x64.dmg` | macOS x64 |
| `DevStack_0.3.5_x64-setup.exe` | Windows NSIS |
| `DevStack_0.3.5_x64_en-US.msi` | Windows MSI |
