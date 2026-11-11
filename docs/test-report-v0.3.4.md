# DevStack v0.3.4 — probe / vhosts / Open URL / empty-site path hotfix

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-4-probe-vhosts-b7b6`  
PR: https://github.com/markode/devstack/pull/29  
Head: `b14ac0f3507110019c07116709f3cdb0dc9fcae4`  
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
| Probe | 10s curl + one retry; warm PHP before probe (not inside `apply_stack`); 3xx OK; WP `install.php` + missing PHP hints |
| Vhosts | Single-bind apache/nginx; purge foreign/orphan confs; `00-default`; **302** not 301 |
| Open | `siteOpenUrl` / `formatSiteUrl` only — never remap `:8088`→`:8444` |
| Start | Coalesce `startDevStack` promise; stack Start guard while in flight |
| DNS | Bindable UDP pick + Hyper-V excluded ranges; warn not error badge; macOS helper/security hard timeouts |
| PHP | Migrate null → installed default |
| Placeholder | Inline DevStack logo + HTML-escaped absolute path + Copy |
| Perf CI | Windows toggle budget **2500 ms** for single-bind dual reload |

## CI results (PR #29 @ `b14ac0f`)

| Check | Result | Run |
|-------|--------|-----|
| macOS smoke | **PASS** | [43028848405](https://github.com/markode/devstack/actions/runs/43028848405) |
| Windows HTTPS smoke + matrix | **PASS** (56 pass / 0 fail / 2 skipped) | [43028848485](https://github.com/markode/devstack/actions/runs/43028848485) |
| Warm apply (Windows) | **4189 ms** (< 5000 ms) | `PERF_BENCH apply_stack_warm_ms=4189` |
| Warm apply (macOS) | **743 ms** (< 4000 ms) | `PERF_BENCH apply_stack_warm_ms=743` |
| Web-server toggle (Windows) | **1494 ms** (< 2500 ms) | `apache_toggle_ms=1494` |

### hotfix034 scenarios (actual)

| Id | Result | Detail |
|----|--------|--------|
| `hotfix034.placeholder_abs_path` | **PASS** 0ms | `path=D:\a\_temp\devstack-e2e\sites\placeholder-path` |
| `hotfix034.apache_http_open_8088` | **PASS** 0ms | `http://open-apache-http.test:8088` |
| `hotfix034.single_bind_purge` | **PASS** 6524ms | purged nginx + apache 302 for `purge-apache` |
| `hotfix034.dns_port_pick` | **PASS** 71ms | `preferred=5354 chosen=5356` |
| `hotfix034.null_php_migrate` | **PASS** 0ms | `default=8.5` |

`MATRIX DONE pass=56 fail=0 skipped=2 total_ms=55385`

## Release assets

| Asset | Notes |
|-------|-------|
| `DevStack_0.3.4_aarch64.dmg` | macOS arm64 |
| `DevStack_0.3.4_x64.dmg` | macOS x64 |
| `DevStack_0.3.4_x64-setup.exe` | Windows NSIS |
| `DevStack_0.3.4_x64_en-US.msi` | Windows MSI |
