#!/usr/bin/env bash
# Ensure local macOS package tarballs + a HTTP-served manifest exist for smoke.
# Prefer downloading packages-macos-v1; otherwise build a minimal set on the runner.
# Built packages include *real* MariaDB (mariadbd), not MySQL Community.
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
  if ! curl -fsSL -o manifest.json "$BASE_URL/manifest.json"; then
    return 1
  fi
  ARCH_LABEL="$ARCH_LABEL" PORT="$PORT" python3 - <<'PY'
import json, os, subprocess, sys
m=json.load(open("manifest.json"))
arch=os.environ.get("ARCH_LABEL","aarch64")
port=os.environ["PORT"]
want={"php","mariadb","nginx","apache"}
got=set()
kept=[]
for p in m.get("packages") or []:
    a=p.get("arch")
    if a not in (arch, "arm64" if arch=="aarch64" else arch) and not (arch=="aarch64" and a=="arm64"):
        kept.append(p)
        continue
    pid=p.get("id")
    if pid=="httpd":
        pid="apache"
    if pid not in want:
        kept.append(p)
        continue
    url=p["url"]
    name=os.path.basename(url)
    print("fetch", name)
    r=subprocess.run(["curl","-fsSL","-o",name,url])
    if r.returncode!=0:
        sys.exit(1)
    p=dict(p)
    p["url"]=f"http://127.0.0.1:{port}/{name}"
    kept.append(p)
    got.add(pid)
need=want - got
if need:
    print("missing from release:", sorted(need))
    sys.exit(2)
open("manifest.json","w").write(json.dumps({"schema_version":1,"packages":kept},indent=2)+"\n")
print("release packages OK", sorted(got))
PY
}

export ARCH_LABEL PORT
if ! download_release; then
  echo "Release incomplete — building minimal packages on runner (real MariaDB included)…"
  chmod +x "$ROOT/scripts/macos/build-packages.sh"
  OUT="$OUT" TARGETS=php,mariadb,nginx,httpd \
    PACKAGES_BASE_URL="http://127.0.0.1:${PORT}" \
    "$ROOT/scripts/macos/build-packages.sh"
  # build-packages writes into $OUT; ensure we're looking at the right manifest
  if [[ ! -f "$OUT/manifest.json" ]]; then
    echo "build-packages did not produce manifest.json" >&2
    exit 1
  fi
  # Rewrite any leftover remote URLs to the local server.
  PORT="$PORT" OUT="$OUT" python3 - <<'PY'
import json, os
out=os.environ["OUT"]; port=os.environ["PORT"]
m=json.load(open(os.path.join(out,"manifest.json")))
for p in m["packages"]:
    name=os.path.basename(p["url"])
    p["url"]=f"http://127.0.0.1:{port}/{name}"
open(os.path.join(out,"manifest.json"),"w").write(json.dumps(m,indent=2)+"\n")
PY
fi

# Fail if any required package lacks a URL for this arch.
ARCH_LABEL="$ARCH_LABEL" OUT="$OUT" python3 - <<'PY'
import json, os, sys
arch=os.environ["ARCH_LABEL"]
out=os.environ["OUT"]
m=json.load(open(os.path.join(out,"manifest.json")))
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

# Start static file server
if ! curl -fsS "http://127.0.0.1:${PORT}/manifest.json" >/dev/null 2>&1; then
  # Serve from OUT so tarballs are reachable
  (cd "$OUT" && python3 -m http.server "$PORT" --bind 127.0.0.1 >/tmp/devstack-pkg-http.log 2>&1) &
  echo $! > /tmp/devstack-pkg-http.pid
  for i in $(seq 1 50); do
    curl -fsS "http://127.0.0.1:${PORT}/manifest.json" >/dev/null 2>&1 && break
    sleep 0.2
  done
fi
curl -fsS "http://127.0.0.1:${PORT}/manifest.json" >/dev/null

echo "DEVSTACK_PHP_MANIFEST_URL=http://127.0.0.1:${PORT}/manifest.json"
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
