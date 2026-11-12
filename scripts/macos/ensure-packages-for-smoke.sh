#!/usr/bin/env bash
# Ensure local macOS package tarballs + a HTTP-served manifest exist for smoke.
# Prefer downloading packages-macos-v1; otherwise build a minimal set on the runner.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${OUT:-${RUNNER_TEMP:-/tmp}/devstack-macos-pkgs}"
PORT="${DEVSTACK_PKG_HTTP_PORT:-18765}"
BASE_URL="${PACKAGES_BASE_URL:-https://github.com/markode/devstack/releases/download/packages-macos-v1}"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64|aarch64) ARCH_LABEL=aarch64 ;;
  *) ARCH_LABEL=x86_64 ;;
esac

mkdir -p "$OUT"
cd "$OUT"

download_release() {
  echo "Trying packages release at $BASE_URL …"
  if curl -fsSL -o manifest.json "$BASE_URL/manifest.json"; then
    python3 - <<PY
import json, os, subprocess, sys
m=json.load(open("manifest.json"))
arch=os.environ.get("ARCH_LABEL","aarch64")
want={"php","mariadb","nginx","apache","httpd"}
got=set()
for p in m.get("packages") or []:
    if p.get("arch") not in (arch, "arm64" if arch=="aarch64" else arch):
        # also accept arm64 label for aarch64
        if not (arch=="aarch64" and p.get("arch")=="arm64"):
            continue
    pid=p.get("id")
    if pid=="httpd":
        pid="apache"
    if pid not in want:
        continue
    url=p["url"]
    name=os.path.basename(url)
    print("fetch", name)
    r=subprocess.run(["curl","-fsSL","-o",name,url])
    if r.returncode!=0:
        sys.exit(1)
    # rewrite URL to local server
    p["url"]=f"http://127.0.0.1:{os.environ['PORT']}/{name}"
    got.add(pid)
need=want - got
if need:
    print("missing from release:", sorted(need))
    sys.exit(2)
open("manifest.json","w").write(json.dumps({"schema_version":1,"packages":m["packages"]},indent=2)+"\n")
print("release packages OK")
PY
    return $?
  fi
  return 1
}

export ARCH_LABEL PORT
if ! download_release; then
  echo "Release incomplete — building minimal packages on runner…"
  chmod +x "$ROOT/scripts/macos/build-packages.sh"
  OUT="$OUT" TARGETS=php,mariadb,nginx,httpd \
    PACKAGES_BASE_URL="http://127.0.0.1:${PORT}" \
    "$ROOT/scripts/macos/build-packages.sh"
  # Rewrite manifest URLs to local server (build script already uses PACKAGES_BASE_URL).
fi

# Fail if any required package lacks a URL for this arch.
python3 - <<PY
import json, os, sys
arch=os.environ["ARCH_LABEL"]
m=json.load(open("manifest.json"))
need={"php","mariadb","nginx","apache"}
have=set()
for p in m["packages"]:
    a=p.get("arch")
    if a not in (arch, "arm64" if arch=="aarch64" else arch) and not (arch=="aarch64" and a=="arm64"):
        continue
    if not p.get("url"):
        print("FAIL: package without URL", p); sys.exit(1)
    pid=p.get("id")
    if pid=="httpd": pid="apache"
    if pid in need:
        have.add(pid)
missing=need-have
if missing:
    print("FAIL: missing packages", sorted(missing)); sys.exit(1)
print("manifest gate OK", sorted(have))
PY

# Start static file server for the app to download from (file:// not supported by reqwest easily).
if ! curl -fsS "http://127.0.0.1:${PORT}/manifest.json" >/dev/null 2>&1; then
  python3 -m http.server "$PORT" --bind 127.0.0.1 >/tmp/devstack-pkg-http.log 2>&1 &
  echo $! > /tmp/devstack-pkg-http.pid
  for i in $(seq 1 30); do
    curl -fsS "http://127.0.0.1:${PORT}/manifest.json" >/dev/null 2>&1 && break
    sleep 0.2
  done
fi

echo "DEVSTACK_PHP_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
echo "DEVSTACK_APACHE_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
echo "DEVSTACK_NGINX_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
echo "DEVSTACK_MARIADB_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
# Export for subsequent steps via GITHUB_ENV when present
if [[ -n "${GITHUB_ENV:-}" ]]; then
  {
    echo "DEVSTACK_PHP_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
    echo "DEVSTACK_APACHE_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
    echo "DEVSTACK_NGINX_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
    echo "DEVSTACK_MARIADB_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
    echo "DEVSTACK_MEMCACHED_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
    echo "DEVSTACK_REDIS_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
  } >> "$GITHUB_ENV"
fi
