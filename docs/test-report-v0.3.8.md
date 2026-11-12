# DevStack v0.3.8 — test report

## Goal

Hotfix macOS PHP install from `packages-macos-v1` after v0.3.7, and keep
one-click installs working without Homebrew / env vars.

## Cause of v0.3.7 macOS smoke failure (main + tag)

Not “packages missing”. Both failing runs (`43229551060` on `main`,
`43229600872` on `v0.3.7`) reached E2E **after** `packages-macos-v1` existed
(published ~13:55Z; ensure-packages ran ~14:05Z / ~14:18Z) and successfully
fetched `php` / `mariadb` / `nginx` / `apache` into the local smoke HTTP server.

Exact E2E error:

```text
Installing PHP 8.4.27…
[php] installing 8.4.27…
  phase=extracting
Error: "[php] managed install required on macOS: No such file or directory (os error 2)"
```

Root cause: the relocatable PHP tarball still contained a dangling absolute
Homebrew symlink `pecl → /opt/homebrew/lib/php/pecl`. Install extracted to a
staging dir then `copy_tree`/`fs::copy` followed that symlink and failed with
ENOENT on a brew-hidden / clean Mac.

## Fixes in 0.3.8

- PHP install moves full prefix trees without following absolute brew symlinks;
  drops absolute links instead of materializing them.
- `build-packages.sh` strips absolute/dangling symlinks before packing.
- Version bump to 0.3.8; republish `packages-macos-v1` from `main` after merge.

## Baked manifest (clean arm64 Mac, no env vars)

`DEFAULT_COMBINED_MANIFEST_URL` remains:

`https://github.com/markode/devstack/releases/download/packages-macos-v1/manifest.json`

Used by PHP, MariaDB, Apache, Nginx (and Memcached/Redis) when no
`DEVSTACK_*_MANIFEST_URL` / AppState override is set. No local HTTP server
required for end users.

## Verification

| Check | Result |
| --- | --- |
| packages-macos | see PR CI |
| macOS smoke (managed, brew hidden) | see PR CI |
| Windows HTTPS smoke | see PR CI |
| Release assets ×4 | published with `v0.3.8` |
