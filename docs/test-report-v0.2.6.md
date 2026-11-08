# Test report — v0.2.6 (WAMP ports + real site status)

## Problem

On Windows with WAMP owning `:80`, Sites showed a green status for Apache WordPress
sites whose Open URL used `:8088`, but the browser got `ERR_CONNECTION_REFUSED`.
Port 80 still served WAMP. Status was based on “process spawned / nginx started”
rather than reachability of the site’s actual listen URL.

## Fixes

1. **Reachability status** — Sites list/detail probe TCP + HTTP(S) against the real
   Open URL port. Dot is green only when both succeed; red/yellow with a message that
   includes the last lines of `apache.err` / nginx error log when useful.
2. **Nginx no longer marks Apache sites running** — starting Nginx only flips
   Nginx-backed sites; Apache sites keep Apache’s own lifecycle.
3. **Auto port separation** — on app launch and every `apply_stack` / start-stack,
   busy classic ports (`80`/`443`/`3306`/…) are remapped to free coexistence ports,
   persisted, and surfaced as a Sites banner
   (`WAMP detected on :80, DevStack uses :8080`).
4. **WordPress URL rewrite** — after a remap (and after DB is up), every WP site’s
   `siteurl`/`home` is search-replaced so redirects match the new ports.

## CI coverage

Windows `--e2e` smoke now includes:

- **WAMP coexistence** — bind dummy listeners on `80`/`443`/`3306`, reset stack to
  classic defaults, `apply_stack`, create WP, fetch over auto-picked HTTP/HTTPS,
  assert reachability `running`.
- **Status error** — stop Apache, enrich site details, assert reachability `error`
  (not green) with `tcp_ok=false`.

Unit: `auto_separate_remaps_busy_classic_ports` in `services::ports`.
