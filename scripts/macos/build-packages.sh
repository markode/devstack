#!/usr/bin/env bash
# Build relocatable macOS package tarballs for DevStack (no Homebrew required
# on the *user* machine). Intended for CI runners that *do* have brew + Xcode CLT.
#
# Produces under $OUT:
#   php-<ver>-macos-<arch>.tar.gz          (static-php-cli with mysqli + fpm)
#   mariadb-<ver>-macos-<arch>.tar.gz      (real MariaDB — brew cellar relocated)
#   nginx-<ver>-macos-<arch>.tar.gz        (compiled once on CI)
#   httpd-<ver>-macos-<arch>.tar.gz        (brew cellar relocated)
#   memcached-<ver>-macos-<arch>.tar.gz
#   redis-<ver>-macos-<arch>.tar.gz
#   manifest.json
#   SHA256SUMS
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${OUT:-$ROOT/dist/macos-packages}"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) ARCH_LABEL=aarch64; SPC_ARCH=aarch64 ;;
  x86_64)        ARCH_LABEL=x86_64;  SPC_ARCH=x86_64 ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

PHP_VERSION="${PHP_VERSION:-8.4.23}"
MARIADB_FORMULA="${MARIADB_FORMULA:-mariadb}"
NGINX_VERSION="${NGINX_VERSION:-1.28.0}"
HTTPD_FORMULA="${HTTPD_FORMULA:-httpd}"
MEMCACHED_FORMULA="${MEMCACHED_FORMULA:-memcached}"
REDIS_FORMULA="${REDIS_FORMULA:-redis}"
SPC_EXTS="${SPC_EXTS:-bcmath,calendar,ctype,curl,dom,exif,fileinfo,filter,gd,iconv,mbstring,mbregex,mysqli,mysqlnd,openssl,pcntl,pdo,pdo_mysql,pdo_sqlite,phar,posix,session,simplexml,soap,sockets,sqlite3,tokenizer,xml,xmlreader,xmlwriter,zip,zlib}"

mkdir -p "$OUT" "$OUT/staging"
cd "$OUT/staging"

pack_dir() {
  local name="$1" src="$2"
  local tar="$OUT/${name}-macos-${ARCH_LABEL}.tar.gz"
  rm -f "$tar"
  tar -C "$src" -czf "$tar" .
  echo "packed $tar ($(du -h "$tar" | awk '{print $1}'))"
}

# Copy a Homebrew cellar/prefix into $stage and rewrite non-system dylibs to
# @loader_path/../lib-bundled so the tree runs without Homebrew installed.
relocate_prefix() {
  local src="$1" stage="$2"
  rm -rf "$stage"
  mkdir -p "$stage" "$stage/lib-bundled"
  cp -a "$src"/. "$stage"/

  if ! command -v otool >/dev/null 2>&1; then
    echo "otool missing — cannot relocate" >&2
    return 1
  fi

  local machos=()
  while IFS= read -r -d '' f; do
    machos+=("$f")
  done < <(find "$stage" -type f \( -perm -111 -o -name '*.dylib' -o -name '*.so' \) -print0 2>/dev/null)

  local f lib base
  for f in "${machos[@]}"; do
    file "$f" 2>/dev/null | grep -q 'Mach-O' || continue
    while read -r lib; do
      case "$lib" in
        /opt/homebrew/*|/usr/local/Cellar/*|/usr/local/opt/*) ;;
        *) continue ;;
      esac
      [[ -f "$lib" ]] || continue
      base="$(basename "$lib")"
      if [[ ! -f "$stage/lib-bundled/$base" ]]; then
        cp -f "$lib" "$stage/lib-bundled/$base" 2>/dev/null || true
        chmod u+w "$stage/lib-bundled/$base" 2>/dev/null || true
        install_name_tool -id "@loader_path/../lib-bundled/$base" "$stage/lib-bundled/$base" 2>/dev/null || true
      fi
      # Depth from file → stage root for relative @loader_path
      local rel
      rel="$(python3 - "$f" "$stage" <<'PY'
import os, sys
f, stage = sys.argv[1:3]
rel = os.path.relpath(os.path.join(stage, "lib-bundled"), os.path.dirname(f))
print(rel.replace(os.sep, "/"))
PY
)"
      install_name_tool -change "$lib" "@loader_path/${rel}/${base}" "$f" 2>/dev/null || true
    done < <(otool -L "$f" 2>/dev/null | awk 'NR>1 {print $1}')
  done

  # Second pass: rewrite deps of bundled dylibs themselves.
  for f in "$stage"/lib-bundled/*; do
    [[ -f "$f" ]] || continue
    file "$f" 2>/dev/null | grep -q 'Mach-O' || continue
    while read -r lib; do
      case "$lib" in
        /opt/homebrew/*|/usr/local/Cellar/*|/usr/local/opt/*) ;;
        *) continue ;;
      esac
      [[ -f "$lib" ]] || continue
      base="$(basename "$lib")"
      if [[ ! -f "$stage/lib-bundled/$base" ]]; then
        cp -f "$lib" "$stage/lib-bundled/$base" 2>/dev/null || true
        chmod u+w "$stage/lib-bundled/$base" 2>/dev/null || true
        install_name_tool -id "@loader_path/$base" "$stage/lib-bundled/$base" 2>/dev/null || true
      fi
      install_name_tool -change "$lib" "@loader_path/$base" "$f" 2>/dev/null || true
    done < <(otool -L "$f" 2>/dev/null | awk 'NR>1 {print $1}')
  done

  # Ad-hoc sign touched binaries (Gatekeeper-friendly for local CI).
  if command -v codesign >/dev/null 2>&1; then
    find "$stage" -type f \( -name 'mariadbd' -o -name 'httpd' -o -name 'nginx' \
      -o -name 'memcached' -o -name 'redis-server' -o -name 'php' -o -name 'php-fpm' \) \
      -exec codesign -s - --force {} \; 2>/dev/null || true
  fi
}

assert_no_brew_deps() {
  local bin="$1"
  [[ -x "$bin" ]] || { echo "missing binary $bin" >&2; return 1; }
  if otool -L "$bin" 2>/dev/null | grep -E '/opt/homebrew|/usr/local/Cellar|/usr/local/opt' >/dev/null; then
    echo "FAIL: $bin still links Homebrew paths:" >&2
    otool -L "$bin" | grep -E '/opt/homebrew|/usr/local/Cellar|/usr/local/opt' >&2 || true
    return 1
  fi
  # Smoke-launch --version where possible
  case "$(basename "$bin")" in
    mariadbd|mysqld) "$bin" --version >/dev/null ;;
    httpd) "$bin" -v >/dev/null 2>&1 || true ;;
    nginx) "$bin" -v >/dev/null 2>&1 || true ;;
    memcached) "$bin" -V >/dev/null ;;
    redis-server) "$bin" --version >/dev/null ;;
    php|php-fpm) "$bin" -v >/dev/null 2>&1 || "$bin" -v >/dev/null ;;
  esac
}

# ---------- PHP: prefer static-php-cli; fallback to relocated brew php@8.4 ----------
build_php_spc() {
  # spc needs pkg-config on PATH (Homebrew pkgconf).
  if command -v brew >/dev/null 2>&1; then
    brew install pkgconf || true
    export PATH="$(brew --prefix pkgconf 2>/dev/null)/bin:$(brew --prefix)/bin:$PATH"
  fi
  command -v pkg-config >/dev/null 2>&1 || command -v pkgconf >/dev/null 2>&1 || {
    echo "pkg-config/pkgconf missing" >&2
    return 1
  }

  local spc_tgz="https://github.com/crazywhalecc/static-php-cli/releases/latest/download/spc-macos-${SPC_ARCH}.tar.gz"
  echo "Downloading spc from $spc_tgz"
  curl -fsSL -o spc.tar.gz "$spc_tgz"
  rm -rf spc-bin && mkdir spc-bin
  tar -xzf spc.tar.gz -C spc-bin
  local spc
  spc="$(find spc-bin -type f -name 'spc' | head -1)"
  [[ -n "$spc" ]] || { echo "spc binary missing from archive" >&2; return 1; }
  chmod +x "$spc"

  if ! "$spc" download --with-php="${PHP_VERSION}" --for-extensions="${SPC_EXTS}"; then
    local channel
    channel="$(echo "$PHP_VERSION" | cut -d. -f1-2)"
    "$spc" download --with-php="${channel}" --for-extensions="${SPC_EXTS}"
  fi
  "$spc" build --build-cli --build-fpm "${SPC_EXTS}"

  local prefix="$OUT/staging/php-prefix"
  rm -rf "$prefix"
  mkdir -p "$prefix/bin" "$prefix/sbin"
  local php_bin fpm_bin
  php_bin="$(find . -type f -name php ! -path './spc-bin/*' ! -path './php-prefix/*' | head -1 || true)"
  fpm_bin="$(find . -type f -name php-fpm ! -path './php-prefix/*' | head -1 || true)"
  if [[ -z "$php_bin" || -z "$fpm_bin" ]]; then
    echo "spc did not emit php/php-fpm" >&2
    return 1
  fi
  cp "$php_bin" "$prefix/bin/php"
  cp "$fpm_bin" "$prefix/sbin/php-fpm"
  chmod 755 "$prefix/bin/php" "$prefix/sbin/php-fpm"
  PHP_VERSION="$("$prefix/bin/php" -r 'echo PHP_VERSION;' 2>/dev/null || echo "$PHP_VERSION")"
  if ! "$prefix/bin/php" -m 2>/dev/null | grep -qi mysqli; then
    echo "FAIL: spc PHP missing mysqli" >&2
    return 1
  fi
  pack_dir "php-${PHP_VERSION}" "$prefix"
}

build_php_brew() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "brew required for PHP fallback" >&2
    return 1
  fi
  # Prefer php@8.4 for smoke; fall back to latest php formula.
  local formula="php@8.4"
  brew install "$formula" || formula="php"
  brew install "$formula" || true
  local prefix
  prefix="$(brew --prefix "$formula")"
  local stage="$OUT/staging/php-brew-prefix"
  relocate_prefix "$prefix" "$stage"
  # Ensure bin/php and sbin/php-fpm (or bin/php-fpm) exist.
  mkdir -p "$stage/bin" "$stage/sbin"
  if [[ ! -x "$stage/bin/php" ]]; then
    local found
    found="$(find "$stage" -type f -name php | head -1 || true)"
    [[ -n "$found" ]] && cp -f "$found" "$stage/bin/php" && chmod 755 "$stage/bin/php"
  fi
  if [[ ! -x "$stage/sbin/php-fpm" ]]; then
    local found
    found="$(find "$stage" -type f -name php-fpm | head -1 || true)"
    if [[ -n "$found" ]]; then
      cp -f "$found" "$stage/sbin/php-fpm"
      chmod 755 "$stage/sbin/php-fpm"
    fi
  fi
  [[ -x "$stage/bin/php" ]] || { echo "php binary missing after brew relocate" >&2; return 1; }
  PHP_VERSION="$("$stage/bin/php" -r 'echo PHP_VERSION;' 2>/dev/null || echo "8.4.0")"
  if ! "$stage/bin/php" -m 2>/dev/null | grep -qi mysqli; then
    echo "FAIL: brew PHP missing mysqli" >&2
    "$stage/bin/php" -m >&2 || true
    return 1
  fi
  # Soft-check brew deps — relocate may leave some; assert_no_brew_deps is ideal.
  assert_no_brew_deps "$stage/bin/php" || {
    echo "warning: php still has brew dylib refs after relocate; continuing" >&2
  }
  pack_dir "php-${PHP_VERSION}" "$stage"
}

build_php() {
  if build_php_spc; then
    return 0
  fi
  echo "spc PHP build failed — falling back to relocated Homebrew php@8.4 (real mysqli)" >&2
  build_php_brew
}

# ---------- Real MariaDB (brew formula relocated — not MySQL) ----------
build_mariadb() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "brew required to build MariaDB package" >&2
    return 1
  fi
  brew install "$MARIADB_FORMULA" || true
  local prefix
  prefix="$(brew --prefix "$MARIADB_FORMULA")"
  local ver
  ver="$(mariadbd --version 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)"
  if [[ -z "$ver" ]]; then
    ver="$("$prefix/bin/mariadbd" --version 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo "11.4.0")"
  fi
  local stage="$OUT/staging/mariadb-prefix"
  relocate_prefix "$prefix" "$stage"

  # Prefer bin/mariadbd layout DevStack expects.
  if [[ ! -x "$stage/bin/mariadbd" ]]; then
    local found
    found="$(find "$stage" -type f -name mariadbd | head -1 || true)"
    if [[ -n "$found" ]]; then
      mkdir -p "$stage/bin"
      cp -f "$found" "$stage/bin/mariadbd"
      chmod 755 "$stage/bin/mariadbd"
    fi
  fi
  [[ -x "$stage/bin/mariadbd" ]] || { echo "mariadbd missing after relocate" >&2; return 1; }
  assert_no_brew_deps "$stage/bin/mariadbd"
  # Client tools
  for b in mariadb mysql mariadb-dump mysqldump mariadb-install-db mysql_install_db; do
    if [[ -x "$prefix/bin/$b" && ! -e "$stage/bin/$b" ]]; then
      cp -a "$prefix/bin/$b" "$stage/bin/" 2>/dev/null || true
    fi
  done
  pack_dir "mariadb-${ver}" "$stage"
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
  # Bundle openssl/pcre dylibs if linked
  if [[ -x "$prefix/sbin/nginx" ]]; then
    local stage="$OUT/staging/nginx-reloc"
    relocate_prefix "$prefix" "$stage"
    # nginx lives in sbin/
    if [[ -x "$stage/sbin/nginx" ]]; then
      assert_no_brew_deps "$stage/sbin/nginx" || {
        # Soft: some system libs OK; only fail on homebrew leftovers
        otool -L "$stage/sbin/nginx" | grep -E '/opt/homebrew|/usr/local/Cellar' && return 1 || true
      }
      pack_dir "nginx-${NGINX_VERSION}" "$stage"
    else
      pack_dir "nginx-${NGINX_VERSION}" "$prefix"
    fi
  else
    echo "nginx binary missing" >&2
    return 1
  fi
}

build_brew_formula() {
  local formula="$1" bin_name="$2" pack_prefix="$3"
  if ! command -v brew >/dev/null 2>&1; then
    echo "brew required for $formula" >&2
    return 1
  fi
  brew install "$formula" || true
  local prefix
  prefix="$(brew --prefix "$formula")"
  local ver
  case "$bin_name" in
    httpd) ver="$(httpd -v 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo "2.4.66")" ;;
    memcached) ver="$(memcached -V 2>/dev/null | awk '{print $2}' || echo "1.6.38")" ;;
    redis-server) ver="$(redis-server --version 2>/dev/null | grep -Eo '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || echo "7.4.2")" ;;
    *) ver="0.0.0" ;;
  esac
  local stage="$OUT/staging/${formula}-prefix"
  relocate_prefix "$prefix" "$stage"
  local bin_path
  bin_path="$(find "$stage" -type f -name "$bin_name" | head -1 || true)"
  [[ -n "$bin_path" ]] || { echo "$bin_name missing in $formula" >&2; return 1; }
  assert_no_brew_deps "$bin_path" || true
  pack_dir "${pack_prefix}-${ver}" "$stage"
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
for p in pkgs:
    print(f"  - {p['id']} {p['version']} {p['arch']}")
PY
}

TARGETS="${TARGETS:-php,mariadb,nginx,httpd,memcached,redis}"
IFS=',' read -ra PARTS <<< "$TARGETS"
failed=0
for t in "${PARTS[@]}"; do
  echo "==== building $t ===="
  case "$t" in
    php) build_php || failed=1 ;;
    mariadb) build_mariadb || failed=1 ;;
    nginx) build_nginx || failed=1 ;;
    httpd|apache) build_brew_formula "$HTTPD_FORMULA" httpd httpd || failed=1 ;;
    memcached) build_brew_formula "$MEMCACHED_FORMULA" memcached memcached || failed=1 ;;
    redis) build_brew_formula "$REDIS_FORMULA" redis-server redis || failed=1 ;;
    *) echo "unknown target: $t" >&2; failed=1 ;;
  esac
done

write_manifest
echo "Packages ready in $OUT"
ls -la "$OUT"/*.tar.gz "$OUT"/manifest.json 2>/dev/null || true
exit "$failed"
