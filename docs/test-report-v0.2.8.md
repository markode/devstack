# DevStack test report — v0.2.8

## Goals

Ship after windows-latest + macos-latest E2E are green. Focus: WordPress
canonical URLs, mandatory busy-port fallback beside WAMP/MAMP/XAMPP/IIS,
native desktop chrome, and **strict isolation** from other stacks.

## Isolation (non-negotiable)

DevStack must never affect WAMP, MAMP PRO, XAMPP, IIS, or other system services.

| Rule | How we enforce it |
| --- | --- |
| Own ports only | `auto_separate_busy_ports` remaps DevStack to the next free port in its own range; foreign listeners are never stopped or reconfigured |
| Never auto-grab 80/443 | Remap **away** when busy; informational banner only (`classic_web_port_notice`) — no “claim port 80” action |
| Own data dirs | Packages, datadirs, logs, pid files under the DevStack data root |
| Hosts block markers | `merge_hosts_contents` only rewrites inside DevStack begin/end markers; other entries preserved |
| Domains | Only DevStack site domains are written; never steal domains other stacks use unless the user creates them here |
| PATH / env | Scoped to spawned children (`PHPRC`, per-process PATH); no global PHP/MySQL on system PATH |
| Own CA | Local DevStack CA under `certs/`; never replaces system trust stores beyond optional user-approved trust helper |
| Own php.ini | Written under each managed PHP install / PHPRC |
| No Windows services | All runtimes are child processes (or user-scoped helpers); uninstall leaves user data only |

### Automated isolation coverage

- `busy_port_fallback_each_service_field` — foreign TCP holders stay alive; each web/DB/php field remaps
- `mysql_mariadb_ports_stay_unique_after_collision`
- `auto_separate_ignores_owned_pids` — never remaps our own listeners
- `classic_web_notice_is_informational_only` — coexistence stack unchanged by :80 notice
- DNS hosts merge unit tests — non-DevStack lines preserved
- WP install writes `canonical_site_url`; Repair site URL + auto-repair on start/toggle

## Bugs fixed in 0.2.8

| Bug | Fix |
| --- | --- |
| WP siteurl/home wrong at install (`:8444` redirect from `:8088`) | Single `canonical_site_url` builder; install + repair via managed PHP+mysqli (serialized-safe); Repair action in site detail; auto-repair on port/https/server change and app start |
| WAMP tray “off” still owns 80 via `wampapache64` | Informational ownership notice (process + Windows service); mandatory fallback to free DevStack ports |
| DNS bind panic on Windows 10013 | `run_dns_daemon` returns Err; hosts-only fallback; no `.expect` |
| php-cgi supervisor respawns after Stop stack | Supervisor stop flag cleared on start / set on stop |
| Own mysqld treated as foreign | Pid-file aware `owned_runtime_pids_with_files`; mysql≠mariadb uniqueness |
| DB hard-killed | `mysqladmin shutdown` then wait before force stop |
| `databasePassword` in list/probe IPC | Stripped from `get_sites` / list / detail payloads |
| Website-like UI chrome | Edge-to-edge native shell; custom Win11 title bar; macOS Overlay traffic lights |

## UI

- Windows: `decorations: false`, custom caption buttons, `set_shadow`, drag region
- macOS: `titleBarStyle: Overlay`, `hiddenTitle`, traffic lights inset in sidebar
- Compact section headers (no hero cards); Playwright screenshots for Windows/macOS/PHP fixtures

## CI

Unit (`cargo test --lib`) + vitest + Playwright shell fixtures + `devstack_smoke --e2e` on windows-latest and macos-latest.
