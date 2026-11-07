# Apache httpd (second web server)

DevStack can run **Apache** alongside **Nginx**. Each site picks one front-end; WordPress sites default to Apache so pretty permalinks and `.htaccess` work out of the box.

## Port design

| Front-end | HTTP | HTTPS |
| --- | --- | --- |
| Nginx | `stack.http_port` (default **80**) | `stack.https_port` (default **443**) |
| Apache | `stack.apache_http_port` (default **8088**) | `stack.apache_https_port` (default **8444**) |

Both processes can run at the same time. Site “Open in browser” URLs include the non-default port for Apache-backed sites (e.g. `https://blog.test:8444`).

If you want Apache on 80/443 exclusively, stop Nginx and set the Apache ports in stack prefs (or swap the defaults when only Apache sites exist).

## Install

Apache is a normal `PackageProvider` (`id: apache`).

### Windows

Official [Apache Lounge](https://www.apachelounge.com/) **VS17 Win64** builds:

- Listed from the VS17 download page
- Downloaded with a Lounge `Referer`
- SHA-256 verified from the adjacent `.zip.txt` checksum file

### macOS / Linux

Point at a prebuilt/static httpd manifest (same JSON shape as PHP/Nginx):

```bash
export DEVSTACK_APACHE_MANIFEST_URL="https://example.com/apache-manifest.json"
# or set apache_manifest_url in DevStack state
```

See `resources/apache-manifest.example.json`.

Without a manifest on macOS/Linux, the Services “Install” control stays disabled (`installReady: false`) and lists an actionable error.

## Per-site choice

Site detail → **Web server**: `Apache (full .htaccess)` or `Nginx`.

- New WordPress sites default to **Apache**
- New blank / PHP sites default to **Nginx**
- Changing the selector reloads both front-ends

## Apache feature set

- `AllowOverride All` on every site `DocumentRoot` (full `.htaccess`)
- `mod_rewrite` loaded for WordPress pretty permalinks
- PHP via `mod_proxy` + `mod_proxy_fcgi` to the same PHP-FPM / php-cgi workers Nginx uses
- HTTPS with the DevStack local CA (same cert helper as Nginx)
- `LimitRequestBody 0` / `LimitXMLRequestBody 0` and `Timeout` / `ProxyTimeout` **3600** (no upload size caps for local work)

## Smoke

```bash
cd src-tauri
DEVSTACK_RESET_SITES=1 DEVSTACK_DATA_DIR=/tmp/devstack-apache \
  cargo run --bin devstack_smoke -- --apache
```

This installs Apache when possible, creates a WordPress site on Apache, enables pretty permalinks, and GETs a post slug over HTTPS on the Apache port.
