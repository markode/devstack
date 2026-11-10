# DevStack v0.3.2 — portable dylib/DLL hotfix

Date: 2026-11-10  
Branch: `cursor/hotfix-0-3-2-static-b7b6`  
PR: https://github.com/markode/devstack/pull/27

## Symptom (v0.3.1 arm64)

On macOS 26.0.1 / Apple Silicon (Mac16,10), `/Applications/DevStack.app` crashed at launch:

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
  - Windows: `scripts/check-windows-dlls.ps1` (`dumpbin /dependents` — system DLLs + WebView2Loader only; blank-line parser fixed so the gate is real).
  - Release: same gates on produced `.dmg` / `devstack.exe`.

## CI evidence

| Check | Result | Notes |
|-------|--------|-------|
| macOS smoke otool gate | **PASS** | [run 42803142205](https://github.com/markode/devstack/actions/runs/42803142205) — no `/opt/homebrew`, no `liblzma` |
| macOS clean launch (xz hidden) | **PASS** | `OK: release binary alive 3s with Homebrew xz hidden` |
| Windows dumpbin gate | **PASS** (parser fixed after first green) | first green vacuously passed blank-line bug; fixed + re-verified |
| Windows / macOS smokes | **PASS** | macos-smoke + windows-smoke on PR #27 |
| Release 4 assets | _pending tag_ | arm64/x64 dmg + exe/msi |

### macOS `otool -L` — `./target/release/devstack` (smoke CI)

```
=== otool -L ./target/release/devstack ===
./target/release/devstack:
	/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit
	/System/Library/Frameworks/Security.framework/Versions/A/Security
	/System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation
	/usr/lib/libobjc.A.dylib
	/System/Library/Frameworks/IOKit.framework/Versions/A/IOKit
	/System/Library/Frameworks/CoreServices.framework/Versions/A/CoreServices
	/System/Library/Frameworks/WebKit.framework/Versions/A/WebKit
	/System/Library/Frameworks/ApplicationServices.framework/Versions/A/ApplicationServices
	/System/Library/Frameworks/CoreGraphics.framework/Versions/A/CoreGraphics
	/System/Library/Frameworks/Carbon.framework/Versions/A/Carbon
	/System/Library/Frameworks/CoreVideo.framework/Versions/A/CoreVideo
	/usr/lib/libSystem.B.dylib
	/usr/lib/libiconv.2.dylib
	/System/Library/Frameworks/QuartzCore.framework/Versions/A/QuartzCore
	/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation
	/System/Library/Frameworks/ColorSync.framework/Versions/A/ColorSync
OK: ./target/release/devstack has no non-system dylibs
OK: release binary alive 3s with Homebrew xz hidden
```

No `liblzma`, `/opt/homebrew`, or `/usr/local` paths.

### Windows `dumpbin /dependents` — `devstack.exe` (smoke CI)

```
=== dumpbin /dependents …\devstack.exe ===
  Image has the following dependencies:
    kernel32.dll
    bcryptprimitives.dll
    ntdll.dll
    api-ms-win-core-synch-l1-2-0.dll
    shell32.dll
    user32.dll
    ole32.dll
    oleaut32.dll
    gdi32.dll
    dwmapi.dll
    imm32.dll
    combase.dll
    comctl32.dll
    shlwapi.dll
    advapi32.dll
    api-ms-win-core-winrt-error-l1-1-0.dll
    ws2_32.dll
    bcrypt.dll
    pdh.dll
    psapi.dll
    powrprof.dll
    crypt32.dll
    api-ms-win-crt-*.dll
OK: …\devstack.exe dependents are system / WebView2 only
Launch OK: title=DevStack responding=True titleMs=909
```

## Expected otool shape (good)

```
/usr/lib/libSystem.B.dylib
/System/Library/Frameworks/...
@rpath/...
```

No `/opt/homebrew`, `/usr/local/opt`, or other absolute third-party paths.
