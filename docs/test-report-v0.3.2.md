# DevStack v0.3.2 — portable dylib/DLL hotfix

Date: 2026-11-10  
Branch: `cursor/hotfix-0-3-2-static-b7b6`

## Symptom (v0.3.1 arm64)

On macOS 26.0.1 / Apple Silicon, `/Applications/DevStack.app` crashed at launch:

```
dyld: Library not loaded: /opt/homebrew/*/liblzma.5.dylib
Referenced from: /Applications/DevStack.app/Contents/MacOS/devstack
Termination Reason: DYLD, Code 1 Library missing
```

Root cause: `xz2` → `lzma-sys` dynamically linked Homebrew’s liblzma from the GitHub Actions runner via pkg-config.

## Fix

- `xz2` / `lzma-sys` built with `static` (`LZMA_API_STATIC=1`).
- `libz-sys` forced `static`; `openssl-sys` `vendored` (alongside `ssh2`’s `vendored-openssl`).
- `src-tauri/.cargo/config.toml` sets `LZMA_API_STATIC` / `LIBZ_SYS_STATIC` / `OPENSSL_STATIC`.
- CI gates:
  - macOS: `scripts/check-macos-dylibs.sh` (`otool -L` must stay under `/usr/lib` + `/System` + `@rpath`).
  - macOS: launch with Homebrew `xz` moved aside for ≥3s.
  - Windows: `scripts/check-windows-dlls.ps1` (`dumpbin /dependents` — system DLLs + WebView2Loader only).
  - Release: same gates on produced `.dmg` / `devstack.exe`.

## CI evidence (fill after green runs)

| Check | Result | Notes |
|-------|--------|-------|
| macOS smoke otool gate | _pending_ | paste `otool -L` excerpt |
| macOS clean launch (xz hidden) | _pending_ | alive ≥3s |
| Windows dumpbin gate | _pending_ | paste dependents |
| Windows / macOS smokes | _pending_ | |
| Release 4 assets | _pending_ | arm64/x64 dmg + exe/msi |

## Expected otool shape (good)

```
/usr/lib/libSystem.B.dylib
/System/Library/Frameworks/...
@rpath/...
```

No `/opt/homebrew`, `/usr/local/opt`, or other absolute third-party paths.
