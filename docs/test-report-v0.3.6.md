# DevStack v0.3.6 — modal portal + Add-site Starter picker

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-6-delete-modal-b7b6`  
PR: https://github.com/markode/devstack/pull/31  
Head: `9f39389c4f22e0163b04fa5374143d9f27757fbe`

Combines the v0.3.5 Delete-dialog UI hotfix with the Add-site Starter picker (plus recommended stacks) in one release.

## A) Delete modal / labels / red Delete (Windows feedback)

| Area | Change |
|------|--------|
| Modal shell | Shared `Modal` portals to `document.body` (`fixed` + `inset:0`), Esc, focus trap, scroll lock |
| Delete labels | Bold title + description on separate lines; whole row is the checkbox label |
| Delete button | Solid red controls; progress + result/warnings with Done |
| Tests | `Modal.test.tsx`, `DeleteSiteModal.test.tsx` |

## B) Add-site Starter picker

Starter step inside **Add site** (reuses `site_templates` engine):

Empty folder · Blank index.html · Blank index.php · Park existing folder · WordPress · WordPress + WooCommerce · Laravel · Next.js · React (Vite) · Vue (Vite) · Angular · plus the user’s saved templates.

| Requirement | Implementation |
|-------------|----------------|
| Managed tools only | Composer / WP-CLI / Node / npm from DevStack packages — never system WAMP |
| DB + config | Auto DB/user; Laravel `.env` with remapped `APP_URL` + `key:generate`; WP via WP-CLI with canonical URL (ports) |
| WooCommerce | `wp plugin install woocommerce --activate` |
| JS frameworks | Scaffold with managed Node, then `npm run build` static output (`dist` / `out`) so `.test` serves immediately |
| Progress / cancel / rollback | Live job progress + log panel; `cancel_site_template_job`; hard rollback (drop DB + remove folder) on failure |
| Recommended stack | `src/starterRecommendations.json` — per-starter web server / PHP / Node / DB from official floors; Recommended badges; override warnings; one-click install or nearest installed compatible |
| E2E | Offline placeholder for every `STARTER_KINDS` in smoke; Windows matrix loops `ALL_KINDS` with `DEVSTACK_SKIP_TEMPLATE_NETWORK=1`; Vitest covers catalog + prefills/warnings/install CTA |

## CI results (PR #31 @ `9f39389`)

| Check | Result | Run |
|-------|--------|-----|
| macOS smoke | **PASS** | [43079475070](https://github.com/markode/devstack/actions/runs/43079475070) |
| Windows HTTPS smoke + matrix | **PASS** (67 pass / 0 fail / 2 skipped) | [43079474772](https://github.com/markode/devstack/actions/runs/43079474772) |
| Warm apply (Windows) | **4112 ms** | `PERF_BENCH apply_stack_warm_ms=4112` |
| Warm apply (macOS) | **765 ms** | `PERF_BENCH apply_stack_warm_ms=765` |
| Delete e2e | **PASS** (Win + mac) | `delete_site(empty-smoke) OK` |
| All Add-site starters (offline) | **PASS** (Win + mac) | empty/html/php/park/wordpress/woocommerce/laravel/nextjs/react/vue/angular |
| Matrix templates | **PASS** (15 kinds) | `template.*` including new starters |

`MATRIX DONE pass=67 fail=0 skipped=2 total_ms=59003`

## Release assets

| Asset | Notes |
|-------|-------|
| `DevStack_0.3.6_aarch64.dmg` | macOS arm64 |
| `DevStack_0.3.6_x64.dmg` | macOS x64 |
| `DevStack_0.3.6_x64-setup.exe` | Windows NSIS |
| `DevStack_0.3.6_x64_en-US.msi` | Windows MSI |
