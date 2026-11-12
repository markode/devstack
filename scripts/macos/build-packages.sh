#!/usr/bin/env bash
# Build relocatable macOS package tarballs for DevStack (no Homebrew required
# on the *user* machine). Intended for CI runners that *do* have brew + Xcode CLT.
#
# Produces under $OUT:
#   php-<ver>-macos-<arch>.tar.gz
#   mariadb-<ver>-macos-<arch>.tar.gz   (MySQL Community Server tree)
#   nginx-<ver>-macos-<arch>.tar.gz
#   httpd-<ver>-macos-<arch>.tar.gz
#   memcached-<ver>-macos-<arch>.tar.gz
#   redis-<ver>-macos-<arch>.tar.gz
#   manifest.json
#   SHA256SUMS
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${OUT:-$ROOT/dist/macos-packages}"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) ARCH_LABEL=aarch64; MYSQL_ARCH=arm64 ;;
  x86_64)        ARCH_LABEL=x86_64;  MYSQL_ARCH=x86_64 ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

PHP_VERSION="${PHP_VERSION:-8.4.23}"
MYSQL_VERSION="${MYSQL_VERSION:-8.4.13}"
MYSQL_MAJOR="${MYSQL_MAJOR:-8.4}"
MYSQL_LABEL="${MYSQL_LABEL:-macos15}"
NGINX_VERSION="${NGINX_VERSION:-1.28.0}"
HTTPD_VERSION="${HTTPD_VERSION:-2.4.66}"
MEMCACHED_VERSION="${MEMCACHED_VERSION:-1.6.38}"
REDIS_VERSION="${REDIS_VERSION:-7.4.2}"
SPC_EXTS="${SPC_EXTS:-bcmath,calendar,ctype,curl,dom,exif,fileinfo,filter,gd,iconv,mbstring,mbregex,mysqli,mysqlnd,openssl,pcntl,pdo,pdo_mysql,pdo_sqlite,phar,posix,session,simplexml,soap,sockets,sqlite3,tokenizer,xml,xmlreader,xmlwriter,zip,zlib}"

mkdir -p "$OUT" "$OUT/staging"
cd "$OUT/staging"

sha256_file() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

pack_dir() {
  local name="$1" src="$2"
  local tar="$OUT/${name}-macos-${ARCH_LABEL}.tar.gz"
  rm -f "$tar"
  tar -C "$src" -czf "$tar" .
  echo "$tar"
}

# ---------- PHP via static-php-cli (cli + fpm, with mysqli) ----------
build_php() {
  local spc_arch
  case "$ARCH_LABEL" in
    aarch64) spc_arch=aarch64 ;;
    *)       spc_arch=x86_64 ;;
  esac
  local spc_url="https://dl.static-php.dev/static-php-cli/common/bin/spc-macos-${spc_arch}"
  curl -fsSL -o spc "$spc_url"
  chmod +x spc
  ./spc download --with-php="${PHP_VERSION}" --for-extensions="${SPC_EXTS}" || \
    ./spc download --with-php="$(echo "$PHP_VERSION" | cut -d. -f1-2)" --for-extensions="${SPC_EXTS}"
  ./spc build --build-cli --build-fpm "${SPC_EXTS}"
  local prefix="$OUT/staging/php-prefix"
  rm -rf "$prefix"
  mkdir -p "$prefix/bin" "$prefix/sbin"
  # spc outputs under buildroot / or cwd depending on version
  local php_bin fpm_bin
  php_bin="$(find . -type f -name php ! -path './spc' ! -path './php-prefix/*' | head -1 || true)"
  fpm_bin="$(find . -type f -name php-fpm ! -path './php-prefix/*' | head -1 || true)"
  if [[ -z "$php_bin" || -z "$fpm_bin" ]]; then
    # Fall back to downloading common static builds and hope mysqli is present
    # (common builds currently omit mysqli — CI should prefer spc success).
    echo "spc did not emit php/php-fpm; falling back to static-php.dev common builds" >&2
    curl -fsSL -o php-cli.tgz \
      "https://dl.static-php.dev/static-php-cli/common/php-${PHP_VERSION}-cli-macos-${ARCH_LABEL}.tar.gz"
    curl -fsSL -o php-fpm.tgz \
      "https://dl.static-php.dev/static-php-cli/common/php-${PHP_VERSION}-fpm-macos-${ARCH_LABEL}.tar.gz"
    tar -xzf php-cli.tgz -C "$prefix/bin"
    tar -xzf php-fpm.tgz -C "$prefix/sbin"
    [[ -f "$prefix/bin/php" ]] || mv "$prefix/bin/"*php* "$prefix/bin/php" 2>/dev/null || true
    [[ -f "$prefix/sbin/php-fpm" ]] || mv "$prefix/sbin/"*php-fpm* "$prefix/sbin/php-fpm" 2>/dev/null || true
  else
    cp "$php_bin" "$prefix/bin/php"
    cp "$fpm_bin" "$prefix/sbin/php-fpm"
    chmod 755 "$prefix/bin/php" "$prefix/sbin/php-fpm"
  fi
  # Record actual version
  local ver
  ver="$("$prefix/bin/php" -r 'echo PHP_VERSION;' 2>/dev/null || echo "$PHP_VERSION")"
  PHP_VERSION="$ver"
  pack_dir "php-${PHP_VERSION}" "$prefix"
}

# ---------- MySQL Community (MariaDB slot on macOS) ----------
build_mariadb() {
  local name="mysql-${MYSQL_VERSION}-${MYSQL_LABEL}-${MYSQL_ARCH}.tar.gz"
  local url="https://cdn.mysql.com/Downloads/MySQL-${MYSQL_MAJOR}/${name}"
  curl -fsSL -o mysql.tgz "$url"
  rm -rf mysql-extract
  mkdir mysql-extract
  tar -xzf mysql.tgz -C mysql-extract
  local tree
  tree="$(find mysql-extract -maxdepth 1 -type d ! -path mysql-extract | head -1)"
  pack_dir "mariadb-${MYSQL_VERSION}" "$tree"
}

# ---------- Nginx (compile once on CI) ----------
build_nginx() {
  curl -fsSL -o nginx.tgz "https://nginx.org/download/nginx-${NGINX_VERSION}.tar.gz"
  rm -rf nginx-src
  mkdir nginx-src
  tar -xzf nginx.tgz -C nginx-src --strip-components=1
  local prefix="$OUT/staging/nginx-prefix"
  rm -rf "$prefix"
  mkdir -p "$prefix"
  (
    cd nginx-src
    local cc_opt="" ld_opt=""
    for openssl in /opt/homebrew/opt/openssl@3 /usr/local/opt/openssl@3 /opt/homebrew/opt/openssl /usr/local/opt/openssl; do
      if [[ -d "$openssl/include" ]]; then
        cc_opt="-I${openssl}/include"
        ld_opt="-L${openssl}/lib"
        break
      fi
    done
    for pcre in /opt/homebrew/opt/pcre2 /usr/local/opt/pcre2; do
      if [[ -d "$pcre/include" ]]; then
        cc_opt="${cc_opt} -I${pcre}/include"
        ld_opt="${ld_opt} -L${pcre}/lib"
        break
      fi
    done
    ./configure --prefix="$prefix" \
      --with-http_ssl_module --with-http_v2_module --with-http_realip_module --with-stream \
      ${cc_opt:+--with-cc-opt="$cc_opt"} ${ld_opt:+--with-ld-opt="$ld_opt"}
    make -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 2)"
    make install
  )
  pack_dir "nginx-${NGINX_VERSION}" "$prefix"
}

# ---------- Apache httpd via brew bottle relocate ----------
build_httpd() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "brew required to build httpd package" >&2
    return 1
  fi
  brew fetch --force-bottle httpd || brew install --build-bottle httpd || true
  brew install httpd || true
  local prefix
  prefix="$(brew --prefix httpd)"
  local ver
  ver="$(httpd -v 2>/dev/null | head -1 | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo "$HTTPD_VERSION")"
  HTTPD_VERSION="$ver"
  # Copy cellar tree + dependent dylibs into a relocatable prefix.
  local stage="$OUT/staging/httpd-prefix"
  rm -rf "$stage"
  mkdir -p "$stage"
  cp -a "$prefix"/. "$stage"/
  # Bundle linked Homebrew dylibs next to bin and rewrite install names.
  mkdir -p "$stage/lib-bundled"
  if command -v otool >/dev/null 2>&1; then
    local bin="$stage/bin/httpd"
    if [[ -x "$bin" ]]; then
      while read -r lib; do
        [[ "$lib" == /opt/homebrew/* || "$lib" == /usr/local/* ]] || continue
        local base
        base="$(basename "$lib")"
        cp -f "$lib" "$stage/lib-bundled/$base" 2>/dev/null || true
        install_name_tool -change "$lib" "@loader_path/../lib-bundled/$base" "$bin" 2>/dev/null || true
      done < <(otool -L "$bin" | awk '/^\t/ {print $1}')
    fi
  fi
  pack_dir "httpd-${HTTPD_VERSION}" "$stage"
}

# ---------- Memcached via brew relocate ----------
build_memcached() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "brew required to build memcached package" >&2
    return 1
  fi
  brew install memcached || true
  local prefix
  prefix="$(brew --prefix memcached)"
  local ver
  ver="$(memcached -V 2>/dev/null | awk '{print $2}' || echo "$MEMCACHED_VERSION")"
  MEMCACHED_VERSION="$ver"
  local stage="$OUT/staging/memcached-prefix"
  rm -rf "$stage"
  mkdir -p "$stage/bin" "$stage/lib-bundled"
  cp -a "$prefix/bin/memcached" "$stage/bin/"
  if command -v otool >/dev/null 2>&1; then
    local bin="$stage/bin/memcached"
    while read -r lib; do
      [[ "$lib" == /opt/homebrew/* || "$lib" == /usr/local/* ]] || continue
      local base
      base="$(basename "$lib")"
      cp -f "$lib" "$stage/lib-bundled/$base" 2>/dev/null || true
      install_name_tool -change "$lib" "@loader_path/../lib-bundled/$base" "$bin" 2>/dev/null || true
    done < <(otool -L "$bin" | awk '/^\t/ {print $1}')
  fi
  pack_dir "memcached-${MEMCACHED_VERSION}" "$stage"
}

# ---------- Redis via brew relocate ----------
build_redis() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "brew required to build redis package" >&2
    return 1
  fi
  brew install redis || true
  local prefix
  prefix="$(brew --prefix redis)"
  local ver
  ver="$(redis-server --version 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo "$REDIS_VERSION")"
  REDIS_VERSION="$ver"
  local stage="$OUT/staging/redis-prefix"
  rm -rf "$stage"
  mkdir -p "$stage/bin" "$stage/lib-bundled"
  for b in redis-server redis-cli; do
    [[ -x "$prefix/bin/$b" ]] && cp -a "$prefix/bin/$b" "$stage/bin/"
  done
  if command -v otool >/dev/null 2>&1; then
    for bin in "$stage/bin/"*; do
      [[ -x "$bin" ]] || continue
      while read -r lib; do
        [[ "$lib" == /opt/homebrew/* || "$lib" == /usr/local/* ]] || continue
        local base
        base="$(basename "$lib")"
        cp -f "$lib" "$stage/lib-bundled/$base" 2>/dev/null || true
        install_name_tool -change "$lib" "@loader_path/../lib-bundled/$base" "$bin" 2>/dev/null || true
      done < <(otool -L "$bin" | awk '/^\t/ {print $1}')
    done
  fi
  pack_dir "redis-${REDIS_VERSION}" "$stage"
}

write_manifest() {
  local base="${PACKAGES_BASE_URL:-https://github.com/markode/devstack/releases/download/packages-macos-v1}"
  python3 - "$OUT" "$ARCH_LABEL" "$base" <<'PY'
import hashlib, json, os, sys
out, arch, base = sys.argv[1:4]
pkgs = []
for name in sorted(os.listdir(out)):
    if not name.endswith(f"-macos-{arch}.tar.gz"):
        continue
    path = os.path.join(out, name)
    sha = hashlib.sha256(open(path, "rb").read()).hexdigest()
    # php-8.4.23-macos-aarch64.tar.gz
    stem = name[: -(len(f"-macos-{arch}.tar.gz"))]
    kind, ver = stem.split("-", 1)
    id_map = {"httpd": "apache", "php": "php", "nginx": "nginx",
              "mariadb": "mariadb", "memcached": "memcached", "redis": "redis"}
    pid = id_map.get(kind, kind)
    pkgs.append({
        "id": pid,
        "version": ver,
        "channel": ".".join(ver.split(".")[:2]),
        "os": "macos",
        "arch": arch,
        "url": f"{base}/{name}",
        "sha256": sha,
        "notes": "DevStack packages-macos-v1 prebuilt (no Homebrew/Xcode on install)",
    })
manifest = {"schema_version": 1, "packages": pkgs}
open(os.path.join(out, "manifest.json"), "w").write(json.dumps(manifest, indent=2) + "\n")
with open(os.path.join(out, "SHA256SUMS"), "w") as f:
    for p in pkgs:
        f.write(f"{p['sha256']}  {os.path.basename(p['url'])}\n")
print(f"wrote {len(pkgs)} packages to manifest.json")
PY
}

TARGETS="${TARGETS:-php,mariadb,nginx,httpd,memcached,redis}"
IFS=',' read -ra PARTS <<< "$TARGETS"
for t in "${PARTS[@]}"; do
  case "$t" in
    php) build_php ;;
    mariadb|mysql) build_mariadb ;;
    nginx) build_nginx ;;
    httpd|apache) build_httpd ;;
    memcached) build_memcached ;;
    redis) build_redis ;;
    *) echo "unknown target: $t" >&2; exit 1 ;;
  esac
done

write_manifest
echo "Packages ready in $OUT"
ls -la "$OUT"/*.tar.gz "$OUT"/manifest.json 2>/dev/null || true
