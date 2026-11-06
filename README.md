# DevStack

Local PHP development environment for **macOS** and **Windows** (Tauri 2 + Rust + React/TypeScript), inspired by Laravel Herd / MAMP PRO.

## Stack

- **UI:** React 19 + TypeScript + Vite
- **Shell:** Tauri 2 (macOS / Windows; Linux used for CI smoke)
- **Core:** Rust — `downloads`, `packages` (`PackageProvider`), `php`, `config`, `services` (Nginx, PHP-FPM/CGI, DB, DNS, TLS)

## Develop

### Prerequisites

- Node.js 20+ and npm
- Rust stable (`rustup`)
- [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/) for your OS

```bash
npm install
npm run tauri dev
```

Frontend-only preview (mock data):

```bash
npm run dev
```

## Build

```bash
npm run build
npm run tauri build
cd src-tauri && cargo check

# Headless smoke
cd src-tauri && cargo run --bin devstack_smoke -- --install-node --node-channel 22
# Milestone 2 stack: Nginx + PHP-FPM + https://smoke.test (+ MariaDB)
cd src-tauri && DEVSTACK_DATA_DIR=/tmp/devstack-smoke cargo run --bin devstack_smoke -- --stack
# Faster web-only smoke:
cd src-tauri && DEVSTACK_DATA_DIR=/tmp/devstack-smoke cargo run --bin devstack_smoke -- --stack --skip-mariadb
```

## Package architecture

Every runtime/service implements `PackageProvider` (list → download → SHA256 → extract → state):

| ID | Source | Install |
| --- | --- | --- |
| `php` | Windows NTS x64 JSON; macOS/Linux pluggable static manifest | Yes |
| `node` | nodejs.org `index.json` + `SHASUMS256.txt` | Yes |
| `nginx` | Windows zip / Linux noble `.deb` from nginx.org; macOS via `DEVSTACK_NGINX_MANIFEST_URL` | Yes |
| `phpmyadmin` | phpmyadmin.net + `.sha256` | Yes |
| `mysql` | Community archives (CDN) | Yes |
| `mariadb` | downloads.mariadb.org REST + binary tarball/zip | Yes |
| `mongodb` / `postgresql` | Official catalogs | List (install later) |

### Stack runtime (milestone 2)

- **One Nginx** serves all sites: `client_max_body_size 0`, long FastCGI timeouts
- **PHP-FPM** pool per PHP version (macOS/Linux); **php-cgi** workers on Windows
- **`.test` domains** via privileged helper (`hosts-sync` / macOS `/etc/resolver`) + tiny DNS daemon
- **Local CA** + per-site certs; HTTPS toggle per site
- **MySQL / MariaDB** start/stop with huge-DB tuning (`max_allowed_packet=1G`, larger InnoDB, long timeouts)
- **phpMyAdmin** on `phpmyadmin.test` with no import limits + WordPress sign-on deep-link

Privileged operations are isolated in `helper/devstack-helper.sh` (sudo/pkexec only for hosts/resolver/CA trust).

### PHP / Nginx manifests (macOS)

```bash
export DEVSTACK_PHP_MANIFEST_URL="https://example.com/php-manifest.json"
export DEVSTACK_NGINX_MANIFEST_URL="https://example.com/nginx-manifest.json"
```

See `resources/php-manifest.example.json`.

### Generated `php.ini`

Unlimited upload/memory/time limits + OPcache; per-version Xdebug toggle on the PHP page.

## UI

- **General / PHP / JavaScript / Services** — package tables
- **Sites** — 3-column tile grid (name, domain, framework icon, PHP, status); click opens full-window detail (environment, paths, Open database for WordPress)
- Placeholders: AI, Pro, Sharing, Shortcuts, About

## Roadmap

1. ~~M1 package core + PHP/Node UI~~
2. ~~M2 Nginx, `.test` HTTPS, MySQL/MariaDB, Sites grid~~
3. MongoDB / PostgreSQL full install path
4. Streaming DB backup/import (no size limits)
5. One-click WordPress via WP-CLI + starter-template duplication (`wp search-replace`)
6. Site PATH shims / CLI isolation
7. Sharing / Pro / AI surfaces

## License

Proprietary / TBD.
