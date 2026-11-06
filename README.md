# DevStack

Local PHP development environment for **macOS** and **Windows** (Tauri 2 + Rust + React/TypeScript), inspired by Laravel Herd / MAMP PRO.

Milestone 1 delivers the app shell, a generic package manager core, live version lists, and one-click installs for Node.js (and PHP where binaries are available).

## Stack

- **UI:** React 19 + TypeScript + Vite
- **Shell:** Tauri 2 (macOS / Windows; Linux used for CI smoke)
- **Core:** Rust modules — `downloads`, `packages` (generic `PackageProvider`), `php`, `config`

## Develop

### Prerequisites

- Node.js 20+ and npm
- Rust stable (`rustup`)
- [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/) for your OS  
  - macOS: Xcode CLT  
  - Windows: WebView2 + MSVC  
  - Linux (optional): `libwebkit2gtk-4.1-dev`, `librsvg2-dev`

```bash
npm install
npm run tauri dev
```

Frontend-only preview (mock data, no Rust installs):

```bash
npm run dev
```

## Build

```bash
# Frontend
npm run build

# Full desktop bundle
npm run tauri build

# Rust typecheck
cd src-tauri && cargo check

# Headless smoke (lists live versions; optional Node install)
cd src-tauri && cargo run --bin devstack_smoke
cd src-tauri && cargo run --bin devstack_smoke -- --install-node --node-channel 22
```

## Package architecture

Every runtime/service implements `PackageProvider`:

| ID | Source | Install (M1) |
| --- | --- | --- |
| `php` | Windows: [downloads.php.net releases.json](https://downloads.php.net/~windows/releases/releases.json) (NTS x64). macOS/Linux: pluggable static-binary manifest | Yes (when URL + SHA256 available) |
| `node` | [nodejs.org/dist/index.json](https://nodejs.org/dist/index.json) + `SHASUMS256.txt` | Yes |
| `phpmyadmin` | [phpmyadmin.net version.json](https://www.phpmyadmin.net/home_page/version.json) + `.sha256` | Yes |
| `mysql` | `mysql/mysql-server` GitHub tags | List only |
| `mariadb` | [MariaDB REST API](https://downloads.mariadb.org/rest-api/mariadb/) | List only |
| `mongodb` | [downloads.mongodb.org/current.json](https://downloads.mongodb.org/current.json) | List only |
| `postgresql` | [postgresql.org/versions.json](https://www.postgresql.org/versions.json) | List only |

Shared path for all providers: **download → SHA256 verify → extract → record in app data**.

### PHP on macOS / Linux

Official Windows ZIPs are used on Windows. For macOS/Linux, point DevStack at a **static-php-cli style manifest**:

```bash
export DEVSTACK_PHP_MANIFEST_URL="https://example.com/php-manifest.json"
# or set php_manifest_url in the app state file
```

See [`resources/php-manifest.example.json`](resources/php-manifest.example.json) for the schema. Until a URL is configured, the PHP page shows channel stubs and Install stays disabled.

### Generated `php.ini`

Each installed PHP gets a managed `php.ini` with no practical limits for huge apps / DB imports:

- `upload_max_filesize=0`
- `post_max_size=0`
- `memory_limit=-1`
- `max_execution_time=0`
- `max_input_time=-1`
- OPcache enabled

Per-version **Xdebug** can be toggled on the PHP page (rewrites the same ini).

### Data directory

Packages and `state.json` live under the OS app-data folder (`DevStack/`), e.g. `~/Library/Application Support/DevStack` on macOS.

## UI

Settings-style window with sidebar:

- **General, PHP, JavaScript, Services, Sites** — implemented
- **AI, Pro, Sharing, Shortcuts, About** — placeholders

PHP / JavaScript pages mirror Herd’s header card + Available Versions table. Sites includes Environment dropdowns for PHP/Node and an HTTPS toggle.

## Roadmap (later milestones)

1. **Service installs** — finish MySQL / MariaDB / MongoDB / PostgreSQL one-click install/update/uninstall using the same provider path; keep phpMyAdmin updates.
2. **Nginx** — per-site vhosts, reloads, logs.
3. **`.test` domains + HTTPS** — local DNS / proxy and trusted certificates.
4. **phpMyAdmin hosting** — serve the installed UI against linked DB services.
5. **DB backup / import** — streaming dump/restore with no size limits (pairs with unlimited PHP upload settings).
6. **WordPress** — one-click install via WP-CLI; starter-template duplication to a new domain (files + DB + `wp search-replace`).
7. **Site isolation** — PATH / `herd`-style CLI shims so a site’s selected PHP/Node are used in the terminal.
8. **Extensions marketplace** — Xdebug and other PHP extensions as first-class package ops.

## License

Proprietary / TBD.
