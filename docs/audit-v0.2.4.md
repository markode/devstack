# DevStack audit — v0.2.4

Full Windows + macOS audit of the Rust backend and React UI after the v0.2.3
hosts/HTTPS/version-cache fixes. Each item below was found and fixed in this
release unless marked *verified OK*.

## Command spawn / Windows console

| Bug | Fix |
| --- | --- |
| Helper/daemon processes could flash a console on Windows | All spawns go through `services::process::command` / `spawn_logged` with `CREATE_NO_WINDOW` (+ `DETACHED_PROCESS` for daemons). |
| `php-cgi` started without managed `php.ini` | `start_php_cgi_workers` now writes php.ini, passes `-c`, sets `PHPRC`, stops stale PID first (`spawn_logged_env`). |
| WP-CLI / `php -m` ignored DevStack ini | `apply_php_ini_to_command` (`PHPRC` + `-c`); `run_wp_cli_with_ini` used for WordPress create / search-replace. |

## Paths

| Bug | Fix |
| --- | --- |
| Mixed `Sites/acme` separators → “Not allowed” / bad dirs | New `paths_util`: `expand_home`, `normalize_separators`, `ensure_dir`, `conf_path`, `quoted_conf_path`. |
| Open folder via Tauri opener scope | New `open_path_cmd` / `reveal_path_cmd` using `explorer.exe` / `open`; UI `open.ts` invokes Rust. |
| Nginx/Apache/MySQL conf used raw `Path::display()` (backslashes, spaces) | Conf generation uses forward-slash `conf_path` / `quoted_conf_path` (e.g. `C:/Users/Jan Kowalski/...`). |
| Site create did not normalize path before persist | `create_site` expands/normalizes and `ensure_dir` before writing `index.php`. |

## Database lifecycle

| Bug | Fix |
| --- | --- |
| Windows waited on Unix socket; clients used `--socket=` | `mysql_root_args` uses TCP on Windows; readiness polls TCP `SELECT 1`. |
| First-run init missed `.exe` installers / MariaDB initialize | `mariadb-install-db.exe` / `mysql_install_db.exe`; fallback `mysqld`/`mariadbd --initialize-insecure`. |
| `my.cnf` datadir broke with spaces | Quoted forward-slash paths in `write_db_config`. |
| phpMyAdmin forced socket on Windows | TCP `127.0.0.1` + port + `connect_type=tcp`; Unix keeps socket. |
| Backup / restore / db_files / seed still socket-only | All switched to `mysql_root_args`. |
| WordPress / duplicate created DB without starting engine | `start_database` before `ensure_database`; WP uses `mysql_db_host(port)`. |

## Hosts / CA / ports

| Bug | Fix |
| --- | --- |
| Hosts banner never auto-synced; UAC cancel opaque | `elevate.ps1` Try/Catch + log/exit codes, `attrib -r`, verify block; `check_hosts_status` file-first; `ensure_hosts_if_needed` on launch. |
| Site CRUD discarded hosts errors (`let _ =`) | `sync_hosts_after_change` propagates errors after create / WP / duplicate / env update. |
| Start stack showed raw `\|DEVSTACK_PORT_BUSY:{json}\|` | Shared `parsePortBusy` / `humanizeError`; `PortConflictDialog` (suggested / coexistence / cancel) on Sites + Services. |

## PHP

| Bug | Fix |
| --- | --- |
| Relative `extension_dir=ext` → missing mysqli when cwd is a site | Absolute `extension_dir` + `WINDOWS_EXTENSIONS` in managed php.ini; regenerate on bootstrap/launch. |
| UI had no missing-module signal | `PhpExtensionView.missing_modules` / `loaded_modules` from `php -m`. |
| CI did not assert mysqli | Smoke `assert_php_mysqli`; Windows workflow runs php.ini unit tests + `--e2e`. |

## UI

| Bug | Fix |
| --- | --- |
| No Add site (empty / link folder) | Add site modal + empty state CTA. |
| Start stack ignored port conflicts | Preflight `checkServicePorts("nginx")` + dialog. |
| Disabled actions lacked tooltips | Tooltips on Add site / Start stack / port dialog buttons. |

## CI / release

| Item | Change |
| --- | --- |
| Windows smoke was HTTPS-probe only | Extended to `devstack_smoke --e2e` (install PHP+MariaDB+Nginx, plain + WP sites, http/https curl, DB export/import, stop). |
| No macOS smoke workflow | Added `.github/workflows/macos-smoke.yml` with the same `--e2e` path. |

## Verified OK (no code change)

- Version list SWR cache + `welcome.test` seed (v0.2.3).
- Postgres `initdb` / Mongo first-run paths in `extras.rs` already used `process::command` and `.exe` names.
- Node install package provider already Windows-aware.
- Xdebug toggle already rewrites managed php.ini.
