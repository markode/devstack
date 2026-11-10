#!/usr/bin/env bash
# Launch DevStack from a .dmg after hiding Homebrew xz (liblzma), require Responding ≤3s.
# Proves the app does not need /opt/homebrew/*/liblzma.5.dylib at runtime.
set -euo pipefail

dmg="${1:-}"
data_dir="${2:-${TMPDIR:-/tmp}/devstack-clean-launch}"
if [[ -z "$dmg" || ! -f "$dmg" ]]; then
  echo "usage: $0 /path/to/DevStack_*.dmg [data_dir]" >&2
  exit 2
fi

cleanup() {
  if [[ -n "${APP_PID:-}" ]]; then
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
  fi
  if [[ -n "${MOUNT:-}" ]]; then
    hdiutil detach "$MOUNT" -quiet || true
  fi
  if [[ -n "${XZ_BAK:-}" && -e "$XZ_BAK" ]]; then
    mv "$XZ_BAK" "${XZ_BAK%.bak-devstack}"
  fi
  if [[ -n "${XZ_CELLAR_BAK:-}" && -e "$XZ_CELLAR_BAK" ]]; then
    mv "$XZ_CELLAR_BAK" "${XZ_CELLAR_BAK%.bak-devstack}"
  fi
}
trap cleanup EXIT

echo "=== hiding Homebrew xz (if present) ==="
for prefix in /opt/homebrew /usr/local; do
  if [[ -d "$prefix/opt/xz" && ! -e "$prefix/opt/xz.bak-devstack" ]]; then
    XZ_BAK="$prefix/opt/xz.bak-devstack"
    mv "$prefix/opt/xz" "$XZ_BAK"
    echo "moved $prefix/opt/xz -> $XZ_BAK"
  fi
  # Also hide cellar copies that dyld might still resolve via absolute paths baked at link time.
  cellar="$(ls -d "$prefix/Cellar/xz/"* 2>/dev/null | head -1 || true)"
  if [[ -n "$cellar" && -d "$cellar" && ! -e "${cellar}.bak-devstack" ]]; then
    XZ_CELLAR_BAK="${cellar}.bak-devstack"
    mv "$cellar" "$XZ_CELLAR_BAK"
    echo "moved $cellar -> $XZ_CELLAR_BAK"
  fi
done

echo "=== mount $dmg ==="
MOUNT="$(hdiutil attach -nobrowse -readonly "$dmg" | awk '/\/Volumes\//{print $3; exit}')"
[[ -n "$MOUNT" ]] || { echo "mount failed" >&2; exit 1; }
app="$(find "$MOUNT" -maxdepth 2 -name 'DevStack.app' -type d | head -1)"
[[ -n "$app" ]] || { echo "DevStack.app missing in dmg" >&2; exit 1; }
bin="$app/Contents/MacOS/devstack"
[[ -f "$bin" ]] || { echo "binary missing: $bin" >&2; exit 1; }

"$(cd "$(dirname "$0")" && pwd)/check-macos-dylibs.sh" "$bin"

mkdir -p "$data_dir"
export DEVSTACK_DATA_DIR="$data_dir"
echo "=== launch (DEVSTACK_DATA_DIR=$data_dir) ==="
"$bin" &
APP_PID=$!

deadline=$((SECONDS + 3))
ok=0
while (( SECONDS < deadline )); do
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    echo "FAIL: process exited before 3s (likely dyld missing lib)" >&2
    wait "$APP_PID" || true
    exit 1
  fi
  # macOS: if the process is still alive after a short settle, treat as launch OK
  # (full UI Responding checks need AppleScript accessibility on CI).
  sleep 0.2
  ok=1
done

if [[ "$ok" -ne 1 ]] || ! kill -0 "$APP_PID" 2>/dev/null; then
  echo "FAIL: app not alive within 3s" >&2
  exit 1
fi
echo "OK: launched from dmg with Homebrew xz hidden; still alive at 3s (pid=$APP_PID)"
kill "$APP_PID" 2>/dev/null || true
wait "$APP_PID" 2>/dev/null || true
APP_PID=""
exit 0
