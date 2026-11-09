# DevStack v0.3.1 — hotfix test report

## Windows 0.3.0 AppHangB1

**Symptom:** Win11 (build 26200, WebView2) — dark empty window, title `Tauri App (brak odpowiedzi)`, AppHangB1 after `DevStack logger ready` + dump receiver log.

**Root causes fixed:**
1. `tray::setup` called `async_runtime::block_on` inside Tauri `.setup()` → nested-runtime deadlock on the main thread.
2. `tauri.windows.conf.json` replaced the window array without `title`, so the product showed `Tauri App`.
3. Decorations/shadow and tray menu refresh could contend with the state lock held by the background launch thread.

**Mitigations:**
- Non-blocking setup: placeholder tray, async menu refresh, deferred chrome, phase logging, >2s stall watchdog.
- Window title forced to `DevStack`.
- Orphan reclaim for leftover `httpd`/`mysqld` whose cmdline references our data dir.
- Windows CI launches the real `devstack.exe` and asserts Responding + title within 3s.

## Performance (Windows before → after)

From Windows HTTPS smoke on v0.3.0 (before):

| Metric | Before (v0.3.0 CI) | Target (v0.3.1) |
| --- | --- | --- |
| Warm `apply_stack` | ~51490 ms (full stop incl. MariaDB) | < 5000 ms (`stop_web_stack` keeps DB) |
| HTTPS toggle | 127 ms | < 1000 ms |
| Apache ↔ nginx toggle | ~4462 ms (FOREGROUND stop+start) | < 1000 ms (dual-bound vhosts) |

Budgets: Windows warm **5000 ms**, toggles **1000 ms**.

## Other fixes

- Meilisearch (and other one-click services): hard kill + wait for listen ports to close; `stop_stack` now stops extras.
- `empty-smoke`: plain HTTP site + Host-match retry (fixes coexistence Host 404 on release-e2e).
