# DevStack v0.3.5 — delete site / Apache HTTP / stop dedupe hotfix

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-5-delete-apache-stop-b7b6`  
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

## CI results

_Pending Windows/macOS smoke on this PR._

### hotfix035 scenarios (expected)

| Id | Expect |
|----|--------|
| `hotfix035.apache_http_logs` | non-ssl logs + HTTP Open URL |
| `hotfix035.delete_site_full` | meta/certs/folder gone |

## Release assets

| Asset | Notes |
|-------|-------|
| `DevStack_0.3.5_aarch64.dmg` | macOS arm64 |
| `DevStack_0.3.5_x64.dmg` | macOS x64 |
| `DevStack_0.3.5_x64-setup.exe` | Windows NSIS |
| `DevStack_0.3.5_x64_en-US.msi` | Windows MSI |
