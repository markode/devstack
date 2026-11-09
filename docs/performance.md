# Performance (v0.3.0)

Targets and the CI regression gate for the hot paths.

## Budgets

| Metric | Target | CI budget (unix) | CI budget (Windows) |
| --- | --- | --- | --- |
| Warm `apply_stack` (start from stopped, after warm-up start+stop) | < 3 s | **4000 ms** | **20000 ms** |
| HTTPS toggle (`apply_site_environment_change`) | < 1 s | **3000 ms** | **8000 ms** |
| Web-server (nginx <-> apache) toggle | < 1 s | **3000 ms** | **8000 ms** |

Constants live in `src-tauri/src/services/perf.rs` (`WARM_APPLY_STACK_BUDGET_MS_*`,
`TOGGLE_BUDGET_MS_*`). Budgets are intentionally above the targets so CI jitter does
not flake the build while a real regression still fails it.

## CI bench

`devstack_smoke --e2e` ends with `run_perf_bench` (also runnable alone with
`devstack_smoke --perf`). It prints one machine-readable line:

```
PERF_BENCH apply_stack_warm_ms=… https_toggle_ms=… apache_toggle_ms=… cold_apply_stack_ms=… stop_stack_ms=… warm_runs=[…]
```

Methodology:

1. Create a throwaway `perfbench` site (nginx, HTTPS).
2. Warm-up: `stop_stack` then `apply_stack` (primes page cache, certs, php.ini, DB
   datadir) then `stop_stack`. The warm-up `apply_stack` is reported as
   `cold_apply_stack_ms`.
3. Measure `apply_stack` from the fully stopped stack **twice**; the reported
   `apply_stack_warm_ms` is the best of the two (a regression slows both runs, a
   scheduler hiccup only one).
4. With the stack up, time the HTTPS toggle (off then on) and the web-server toggle
   (apache then nginx); the slower direction is reported.
5. Remove the site, reload, stop the stack, then enforce the budgets above.

Before/after for a change: build the old commit with the bench file copied in
(`services/perf.rs` + `bin/devstack_smoke.rs` are self-contained), run `--perf` several
times on the same data dir, and compare `apply_stack_warm_ms` / `stop_stack_ms`.

## What changed

- **Bootstrap**: `bootstrap_ctx` only loads state, ensures the welcome site and builds the
  package registry. `regenerate_all_php_ini`, the `Databases/` layout, WP URL rewrites and
  the hosts/UAC check run once on the background launch thread, off the state lock
  (the lock is held only for the port remap + parked sync). Watcher setup no longer blocks
  window setup on the state mutex.
- **`apply_stack`**: DNS, CA/hosts trust, database and all PHP versions start
  concurrently (state snapshots, owned fields merged back). WordPress URL repair overlaps
  Nginx/Apache start. Nginx/Apache find PHP already listening and skip straight to spawn.
- **`stop_stack`**: database shutdown (the long pole) runs beside the front-ends/DNS/PHP.
- **Readiness polls** use `perf::poll_backoff` (5 ms doubling to 25-50 ms) instead of
  fixed 100-500 ms sleeps: PHP (<=2 s), database (cheap TCP/socket probe gates the client
  `SELECT 1`, 45 s ceiling for cold InnoDB init), Nginx (new, <=3 s), Apache (<=3 s).
- **`stop_pid`** (unix) polls for exit instead of a fixed 400 ms sleep, and exited-but-
  unreaped children (zombies) count as stopped.
- **`start_php`** is a no-op fast path when the pid is alive and the listener accepts;
  `start_php_many` starts distinct versions in parallel. `php -r` version detection for
  system PHP is memoised.
- **Site toggles**: only the edited site is re-scanned; WordPress siteurl repair is skipped
  for non-WordPress sites and when the stored siteurl already equals the canonical URL;
  only the affected front-end reloads.
- **Windows Apache**: `-k graceful` is a no-op for `-D FOREGROUND` (it targets the
  `Apache2.4` service), so a real config change still needs stop+start. The restart is now
  skipped when the rendered `httpd.conf` + vhosts + referenced cert/key files (len, mtime)
  fingerprint is unchanged for the running pid.
- **Watchers**: parked-folder debounce 500 ms (trailing edge, sleeps exactly until the
  window closes); `Databases/` watcher keeps 800 ms repeat debounce + 400 ms settle. Both
  must stay >= `perf::MIN_WATCH_DEBOUNCE_MS` (300 ms), enforced by unit tests.

## Frontend

- `AppData` hydrates a lightweight snapshot (sites list without reachability payloads +
  stack ports) from `localStorage` (`devstack.sitesSnapshot.v1`) for instant first paint,
  then refreshes in phases: sites + stack status (paint), installed PHP/Node + templates +
  hosts (after the first frame), reachability probe last.
- Boot loads with `probe: false` and runs the probe afterwards. `SitesPage` mounts through
  `ensureSites()`, which never starts a second load when real data is cached or a load is
  already in flight.
