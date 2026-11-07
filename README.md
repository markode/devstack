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
# Milestone 3: one-click WP → template → duplicate over HTTPS
cd src-tauri && DEVSTACK_RESET_SITES=1 DEVSTACK_DATA_DIR=/tmp/devstack-smoke \
  cargo run --bin devstack_smoke -- --wordpress
# Milestone 4: Postgres + MongoDB + Redis + Mailpit
cd src-tauri && DEVSTACK_DATA_DIR=/tmp/devstack-smoke \
  cargo run --bin devstack_smoke -- --services
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
| `mongodb` | downloads.mongodb.org `current.json` + SHA256 | Yes |
| `postgresql` | theseus-rs binaries + `.sha256` | Yes |
| `redis` | redis.io tarballs (redis-hashes SHA256); Windows zip builds | Yes |
| `mailpit` | GitHub releases + asset digest | Yes |

### Stack runtime (milestone 2)

- **One Nginx** serves all sites: `client_max_body_size 0`, long FastCGI timeouts
- **PHP-FPM** pool per PHP version (macOS/Linux); **php-cgi** workers on Windows
- **`.test` domains** via privileged helper (`hosts-sync` / macOS `/etc/resolver`) + tiny DNS daemon
- **Local CA** + per-site certs; HTTPS toggle per site
- **MySQL / MariaDB** start/stop with huge-DB tuning (`max_allowed_packet=1G`, larger InnoDB, long timeouts)
- **phpMyAdmin** on `phpmyadmin.test` with no import limits + WordPress sign-on deep-link

### WordPress & templates (milestone 3)

- **New WordPress** — downloads latest core (SHA1-verified) + official WP-CLI phar (SHA512), creates DB/user, `wp core install`, optional `pl_PL`, shows admin password once
- **Templates** — mark any site: streaming file archive + DB dump (gzip/zstd, no size limits)
- **Duplicate** — copy files, stream import, `wp search-replace` (serialized-safe, all tables), refresh credentials
- **Backup / restore** from site detail via mysqldump/mysql pipes
- **Databases folder** — portable dumps under `Databases/` (auto-export, folder watcher, site Download/Import with WP URL rewrite); see `docs/databases-folder.md`

### Extra services & packaging (milestone 4)

- **MongoDB / PostgreSQL / Redis / Mailpit** — full install + start/stop; Services page shows status, ports, logs
- **Mailpit** — SMTP `:1025`, UI on `mailpit.test` (Nginx proxy) + PHP SMTP hint ini
- **GitHub Actions** release workflow (`tauri-action`) → draft Release with macOS `.dmg` and Windows NSIS/MSI
- **Signing docs** in `docs/packaging.md` (optional; unsigned CI builds work)
- **Auto-updater stub** in `tauri.conf.json`
- App icon + polished **About** page

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

- **General / PHP / JavaScript / Services** — package tables + runtime status/start/stop/logs
- **Sites** — 3-column tile grid; **New WordPress** / **Duplicate**; detail view with template mark, streaming DB backup/restore, Open database for WordPress
- **About** — brand, version, stack summary, updater note
- Placeholders: AI, Pro, Sharing, Shortcuts

## Roadmap

1. ~~M1 package core + PHP/Node UI~~
2. ~~M2 Nginx, `.test` HTTPS, MySQL/MariaDB, Sites grid~~
3. ~~M3 One-click WordPress, templates/duplicate, streaming DB backup~~
4. ~~M4 Mongo/Postgres/Redis/Mailpit + packaging + About~~
5. Site PATH shims / CLI isolation
6. Sharing / Pro / AI surfaces

## License

Proprietary / TBD.
