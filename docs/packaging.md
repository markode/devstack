# Packaging & releases

DevStack ships macOS and Windows installers via GitHub Actions (`/.github/workflows/release.yml`) using [`tauri-action`](https://github.com/tauri-apps/tauri-action).

## What CI builds

| Runner | Artifact |
| --- | --- |
| `macos-latest` + `universal-apple-darwin` | Universal `.dmg` / `.app` (arm64 + x64) |
| `windows-latest` | NSIS `.exe` and/or `.msi` |

Triggers: push of a `v*` tag (or manual `workflow_dispatch`). Artifacts upload to a **draft** GitHub Release.

**Unsigned builds are intentional.** The workflow succeeds without any signing secrets.

## Optional code signing / notarization

### macOS

| Secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 `.p12` Developer ID Application certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Password for the `.p12` |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: …` |
| `APPLE_ID` | Apple ID used for notarization |
| `APPLE_PASSWORD` | App-specific password |
| `APPLE_TEAM_ID` | Team ID |

Without these, `tauri build` still produces a usable (Gatekeeper-unnotarized) `.dmg`.

**Important:** do not create empty placeholder secrets. An empty `APPLE_CERTIFICATE` is still injected as an env var and makes the bundler attempt PKCS#12 import (and fail). The release workflow only forwards non-empty signing secrets; `tauri.conf.json` uses macOS `signingIdentity: "-"` (ad-hoc) for unsigned CI.

### Windows

| Secret | Purpose |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | Minisign / Tauri updater private key (PEM) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Key password |

Authenticode EV/OV certs can be wired later via `signtool` in a follow-up job; CI does not require them.

## Auto-updater stub

`src-tauri/tauri.conf.json` includes a `plugins.updater` stub pointing at GitHub Releases endpoints. Enable the `updater` feature and ship signed update manifests when ready — the stub keeps config shape stable without requiring keys in CI.

## Local build

```bash
npm install
npm run tauri build
```

Outputs land under `src-tauri/target/release/bundle/`.
