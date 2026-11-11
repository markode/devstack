# DevStack v0.3.4 — probe / vhosts / Open URL / empty-site path hotfix

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-4-probe-vhosts-b7b6`  
Source: real Windows 11 follow-up on v0.3.3 (Open→8444, cold probe, double Start, DNS 5353, null PHP, empty-site path).

## Symptoms (v0.3.3)

1. Cold probe timed out (~3s); first Open after Start flaky.
2. Apache Open for `https=false` remapped onto `:8444` via stale `Redirect permanent` / dual-bound nginx leftovers.
3. Double UI Start / Strict-mode double invoke ran two `start_dev_stack` IPCs.
4. DNS on UDP 5353 failed under Hyper-V excluded ranges → Logs error badge.
5. Legacy `phpVersion: null` still showed as System / missing.
6. Empty-site placeholder lacked app logo + absolute folder path.

## Fix summary

| Area | Change |
|------|--------|
| Probe | 10s curl + one retry; warm PHP before probe; 3xx OK; WP `install.php` + missing PHP hints |
| Vhosts | Single-bind apache/nginx; purge foreign/orphan confs; `00-default`; **302** not 301 |
| Open | `siteOpenUrl` / `formatSiteUrl` only — never remap `:8088`→`:8444` |
| Start | Coalesce `startDevStack` promise; stack Start guard while in flight |
| DNS | Bindable UDP pick + Hyper-V excluded ranges; warn not error badge |
| PHP | Migrate null → installed default |
| Placeholder | Inline DevStack logo + HTML-escaped absolute path + Copy |

## CI results

_Pending Windows/macOS smoke on this PR._

### hotfix034 scenarios (expected)

| Id | Expect |
|----|--------|
| `hotfix034.placeholder_abs_path` | logo + absolute path + Copy |
| `hotfix034.apache_http_open_8088` | `http://…:apache_http_port` |
| `hotfix034.single_bind_purge` | apache site purged from nginx; Redirect temporary |
| `hotfix034.dns_port_pick` | bindable UDP chosen |
| `hotfix034.null_php_migrate` | null → default |

## Release assets

| Asset | Notes |
|-------|-------|
| `DevStack_0.3.4_aarch64.dmg` | macOS arm64 |
| `DevStack_0.3.4_x64.dmg` | macOS x64 |
| `DevStack_0.3.4_x64-setup.exe` | Windows NSIS |
| `DevStack_0.3.4_x64_en-US.msi` | Windows MSI |
