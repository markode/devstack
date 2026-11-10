#!/usr/bin/env bash
# Fail if a Mach-O binary dynamically links anything outside Apple system paths.
# Allowed: /usr/lib/*, /System/*, @rpath/*, @executable_path/*, @loader_path/*
# Forbidden examples: /opt/homebrew/*, /usr/local/*, /Users/*, relative non-@ paths.
set -euo pipefail

bin="${1:-}"
if [[ -z "$bin" || ! -f "$bin" ]]; then
  echo "usage: $0 /path/to/Mach-O" >&2
  exit 2
fi

echo "=== otool -L $bin ==="
otool -L "$bin"

bad=0
while IFS= read -r line; do
  # otool lines look like: $'\t'/path/to/lib (compatibility ...)
  path="$(sed -E 's/^[[:space:]]+//; s/ \(compatibility.*//; s/[[:space:]]+$//' <<<"$line")"
  [[ -z "$path" ]] && continue
  # First line is the binary id ("…:") — skip.
  [[ "$path" == *: ]] && continue
  case "$path" in
    /usr/lib/*|/System/*|@rpath/*|@executable_path/*|@loader_path/*)
      ;;
    *)
      echo "FORBIDDEN dylib: $path" >&2
      bad=1
      ;;
  esac
done < <(otool -L "$bin" | tail -n +2)

if [[ "$bad" -ne 0 ]]; then
  echo "FAIL: $bin links non-system dylibs (Homebrew /usr/local etc.). Ship static/vendored natives." >&2
  exit 1
fi
echo "OK: $bin has no non-system dylibs"
