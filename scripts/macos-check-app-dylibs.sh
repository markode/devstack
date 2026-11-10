#!/usr/bin/env bash
# Locate DevStack.app Mach-O (from a .app path, .dmg, or raw binary) and run check-macos-dylibs.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHECK="$ROOT/scripts/check-macos-dylibs.sh"
target="${1:-}"

if [[ -z "$target" ]]; then
  echo "usage: $0 /path/to/DevStack.app|/path/to/*.dmg|/path/to/binary" >&2
  exit 2
fi

cleanup() {
  if [[ -n "${MOUNT:-}" ]]; then
    hdiutil detach "$MOUNT" -quiet || true
  fi
}
trap cleanup EXIT

bin=""
if [[ -f "$target" && "$target" == *.dmg ]]; then
  MOUNT="$(hdiutil attach -nobrowse -readonly "$target" | awk '/\/Volumes\//{print $3; exit}')"
  if [[ -z "$MOUNT" ]]; then
    echo "failed to mount $target" >&2
    exit 1
  fi
  app="$(find "$MOUNT" -maxdepth 2 -name 'DevStack.app' -type d | head -1)"
  [[ -n "$app" ]] || { echo "DevStack.app not found in $target" >&2; exit 1; }
  bin="$app/Contents/MacOS/devstack"
elif [[ -d "$target" && "$target" == *.app ]]; then
  bin="$target/Contents/MacOS/devstack"
elif [[ -f "$target" ]]; then
  bin="$target"
else
  echo "unrecognized target: $target" >&2
  exit 2
fi

[[ -f "$bin" ]] || { echo "binary missing: $bin" >&2; exit 1; }
exec "$CHECK" "$bin"
