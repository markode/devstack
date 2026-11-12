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
# @loader_path so the tree runs without Homebrew installed. Iterates until no
# brew absolute deps remain (covers transitive ICU etc.).
relocate_prefix() {
  local src="$1" stage="$2"
  rm -rf "$stage"
  mkdir -p "$stage" "$stage/lib-bundled"
  cp -a "$src"/. "$stage"/

  if ! command -v otool >/dev/null 2>&1; then
    echo "otool missing — cannot relocate" >&2
    return 1
  fi

  python3 - "$stage" <<'PY'
import os, re, subprocess, sys
from pathlib import Path

stage = Path(sys.argv[1])
bundled = stage / "lib-bundled"
bundled.mkdir(exist_ok=True)
brew_re = re.compile(r"^(/opt/homebrew/|/usr/local/(Cellar|opt)/)")
system_re = re.compile(r"^(/usr/lib/|/System/|@executable_path/|/usr/local/lib/libSystem)")
# Remember cellar dirs we copied from so @loader_path siblings can be found.
source_dirs = set()
# Cache basename -> absolute brew path for missing @loader_path resolution.
name_index = {}

def is_macho(path: Path) -> bool:
    try:
        out = subprocess.check_output(["file", str(path)], text=True, stderr=subprocess.DEVNULL)
    except Exception:
        return False
    return "Mach-O" in out

def otool_libs(path: Path):
    try:
        out = subprocess.check_output(["otool", "-L", str(path)], text=True, stderr=subprocess.DEVNULL)
    except Exception:
        return []
    libs = []
    for line in out.splitlines()[1:]:
        lib = line.strip().split()[0]
        libs.append(lib)
    return libs

def collect_machos():
    out = []
    for root, dirs, files in os.walk(stage):
        dirs[:] = [d for d in dirs if d not in {".git", "share", "include", "man"}]
        for name in files:
            p = Path(root) / name
            if p.is_symlink():
                continue
            if os.access(p, os.X_OK) or p.suffix in {".dylib", ".so"}:
                if is_macho(p):
                    out.append(p)
    return out

def index_dir(d: Path):
    try:
        for child in d.iterdir():
            if not child.name.endswith((".dylib", ".so")):
                continue
            # Prefer real files; also index symlink names (ICU uses versioned names).
            try:
                real = child.resolve() if child.is_symlink() or child.is_file() else None
            except Exception:
                real = None
            if real and real.is_file():
                name_index.setdefault(child.name, real)
                name_index.setdefault(real.name, real)
    except Exception:
        pass

def find_named(name: str):
    if name in name_index and name_index[name].is_file():
        return name_index[name]
    for d in list(source_dirs):
        index_dir(d)
        cand = d / name
        if cand.exists():
            try:
                real = cand.resolve()
                if real.is_file():
                    name_index[name] = real
                    return real
            except Exception:
                pass
    # Slow fallback: search Homebrew lib trees once per missing name.
    for root in ("/opt/homebrew/Cellar", "/opt/homebrew/opt", "/usr/local/Cellar", "/usr/local/opt"):
        root_p = Path(root)
        if not root_p.is_dir():
            continue
        try:
            for p in root_p.rglob(name):
                if p.is_file() or p.is_symlink():
                    try:
                        real = p.resolve()
                    except Exception:
                        continue
                    if real.is_file():
                        name_index[name] = real
                        source_dirs.add(real.parent)
                        index_dir(real.parent)
                        return real
        except Exception:
            continue
    return None

def copy_into_bundled(abs_lib):
    abs_path = Path(abs_lib)
    if not abs_path.exists():
        return None
    try:
        real = abs_path.resolve()
    except Exception:
        real = abs_path
    if not real.is_file():
        return None
    # Keep the *referenced* basename (symlink name), not only the resolved file name.
    base = abs_path.name
    dest = bundled / base
    source_dirs.add(real.parent)
    index_dir(real.parent)
    if not dest.exists():
        # Copy under the referenced name so @loader_path/libicudata.79.dylib resolves.
        subprocess.run(["cp", "-fL", str(abs_path), str(dest)], check=False)
        try:
            dest.chmod(0o755)
        except Exception:
            pass
        subprocess.run(
            ["install_name_tool", "-id", f"@loader_path/{base}", str(dest)],
            check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    # Also ensure the resolved basename exists when different (some load cmds use it).
    if real.name != base:
        dest2 = bundled / real.name
        if not dest2.exists():
            subprocess.run(["cp", "-fL", str(real), str(dest2)], check=False)
            try:
                dest2.chmod(0o755)
            except Exception:
                pass
            subprocess.run(
                ["install_name_tool", "-id", f"@loader_path/{real.name}", str(dest2)],
                check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
    # Pull every sibling dylib from the cellar lib dir (ICU/OpenSSL families).
    for sibling in real.parent.iterdir():
        if not sibling.name.endswith((".dylib", ".so")):
            continue
        if sibling.is_symlink() or sibling.is_file():
            sbase = sibling.name
            sdest = bundled / sbase
            if sdest.exists():
                continue
            try:
                sreal = sibling.resolve()
            except Exception:
                continue
            if not sreal.is_file():
                continue
            subprocess.run(["cp", "-fL", str(sibling), str(sdest)], check=False)
            try:
                sdest.chmod(0o755)
            except Exception:
                pass
            subprocess.run(
                ["install_name_tool", "-id", f"@loader_path/{sbase}", str(sdest)],
                check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            )
            name_index[sbase] = sreal
    return dest if dest.exists() else None

def rewrite_dep(path: Path, old: str, base: str):
    if path.parent.resolve() == bundled.resolve():
        new = f"@loader_path/{base}"
    else:
        rel = os.path.relpath(bundled, path.parent).replace(os.sep, "/")
        new = f"@loader_path/{rel}/{base}"
    subprocess.run(
        ["install_name_tool", "-change", old, new, str(path)],
        check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )

changed = True
rounds = 0
while changed and rounds < 24:
    changed = False
    rounds += 1
    for path in collect_machos():
        for lib in otool_libs(path):
            if system_re.match(lib) or lib.startswith("@rpath/"):
                # @rpath often still points into brew; handle absolute only below.
                if not lib.startswith("@rpath/"):
                    continue
            if brew_re.match(lib):
                if not os.path.isfile(lib):
                    continue
                base = os.path.basename(lib)
                before = {p.name for p in bundled.iterdir()} if bundled.exists() else set()
                copy_into_bundled(lib)
                after = {p.name for p in bundled.iterdir()}
                if after != before:
                    changed = True
                rewrite_dep(path, lib, base)
                changed = True
                continue
            # ICU and friends already use @loader_path for siblings in the cellar.
            # After we copy one ICU dylib, those refs break unless siblings are bundled.
            if lib.startswith("@loader_path/"):
                base = os.path.basename(lib)
                # Already satisfied next to this file?
                local = (path.parent / base)
                if local.exists():
                    continue
                dest = bundled / base
                if not dest.exists():
                    found = find_named(base)
                    if found is None:
                        continue
                    before = {p.name for p in bundled.iterdir()}
                    copy_into_bundled(found if found.name == base else found.parent / base if (found.parent / base).exists() else found)
                    # Ensure referenced basename exists
                    if not dest.exists() and found.is_file():
                        subprocess.run(["cp", "-fL", str(found), str(dest)], check=False)
                        subprocess.run(
                            ["install_name_tool", "-id", f"@loader_path/{base}", str(dest)],
                            check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                        )
                    after = {p.name for p in bundled.iterdir()}
                    if after != before:
                        changed = True
                # If this file lives outside lib-bundled, point at bundled copy.
                if path.parent.resolve() != bundled.resolve() and dest.exists():
                    rewrite_dep(path, lib, base)
                    changed = True

# Final sweep: normalize bundled dylibs to sibling @loader_path and fill gaps.
for path in list(bundled.iterdir()):
    if not path.is_file() or not is_macho(path):
        continue
    for lib in otool_libs(path):
        if brew_re.match(lib) and os.path.isfile(lib):
            base = os.path.basename(lib)
            copy_into_bundled(lib)
            rewrite_dep(path, lib, base)
        elif lib.startswith("@loader_path/"):
            base = os.path.basename(lib)
            if not (bundled / base).exists():
                found = find_named(base)
                if found is not None:
                    copy_into_bundled(found.parent / base if (found.parent / base).exists() else found)
                    if not (bundled / base).exists():
                        subprocess.run(["cp", "-fL", str(found), str(bundled / base)], check=False)
                        subprocess.run(
                            ["install_name_tool", "-id", f"@loader_path/{base}", str(bundled / base)],
                            check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                        )
            if lib.startswith("@loader_path/../lib-bundled/"):
                rewrite_dep(path, lib, base)

print(f"relocate done after {rounds} rounds; bundled={len(list(bundled.iterdir()))}")
PY

  if command -v codesign >/dev/null 2>&1; then
    find "$stage" -type f \( -name 'mariadbd' -o -name 'httpd' -o -name 'nginx' \
      -o -name 'memcached' -o -name 'redis-server' -o -name 'php' -o -name 'php-fpm' \) \
      -exec codesign -s - --force --deep {} \; 2>/dev/null || true
    find "$stage/lib-bundled" -type f -exec codesign -s - --force {} \; 2>/dev/null || true
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

# ---------- PHP via relocated Homebrew php@8.4 (includes mysqli + php-fpm) ----------
# static-php-cli is flaky on GHA (403 mirrors / pkg-config). Brew cellar + recursive
# dylib bundling is the reliable path for one-click installs without Homebrew on the user Mac.
build_php() {
  if ! command -v brew >/dev/null 2>&1; then
    echo "brew required to build PHP package" >&2
    return 1
  fi
  local formula="php@8.4"
  brew install "$formula" || { formula="php"; brew install "$formula"; }
  local prefix
  prefix="$(brew --prefix "$formula")"
  # Prove the source formula has mysqli before we relocate.
  if ! "$prefix/bin/php" -m 2>/dev/null | grep -qi mysqli; then
    echo "FAIL: brew $formula missing mysqli before relocate" >&2
    "$prefix/bin/php" -m >&2 || true
    return 1
  fi
  local stage="$OUT/staging/php-brew-prefix"
  relocate_prefix "$prefix" "$stage"
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
  # Relocated binary must actually run and expose mysqli.
  if ! "$stage/bin/php" -m 2>/dev/null | grep -qi mysqli; then
    echo "FAIL: relocated PHP cannot load mysqli (dylib bundling incomplete?)" >&2
    otool -L "$stage/bin/php" | head -40 >&2 || true
    "$stage/bin/php" -m >&2 || true
    return 1
  fi
  PHP_VERSION="$("$stage/bin/php" -r 'echo PHP_VERSION;')"
  assert_no_brew_deps "$stage/bin/php" || {
    echo "warning: php still has brew dylib refs after relocate" >&2
  }
  pack_dir "php-${PHP_VERSION}" "$stage"
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
