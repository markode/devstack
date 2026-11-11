# DevStack v0.3.6 — delete modal portal / labels / red Delete hotfix

Date: 2026-11-11  
Branch: `cursor/hotfix-0-3-6-delete-modal-b7b6`  
Source: real Windows 11 on v0.3.5 (Delete dialog off-viewport after scroll; concatenated checkbox labels; faded pink Delete button).

## Symptoms (v0.3.5)

1. Clicking **Delete Application** at the bottom of a long site detail page opened the modal inside the scrollable content — only a grey backdrop was visible until the user scrolled up.
2. Checkbox labels concatenated title + detail with no space/line break (`Remove from DevStackvhosts…`, `…userwp_test / u_test`, `…diskC:\…`).
3. Delete controls looked disabled (faded pink) rather than clearly red/enabled; no in-dialog progress or result.

## Fix summary

| Area | Change |
|------|--------|
| Modal shell | Shared `Modal` portals to `document.body` with `position:fixed; inset:0`, Esc dismiss, focus trap, `body.modal-open` scroll lock; all app modals audited |
| Delete labels | Bold title + description on separate lines; whole row is the checkbox label; monospace DB names and path |
| Delete button | Solid red `danger-btn` / `btn-danger-solid`; progress while deleting; result / warnings then Done |
| Tests | `Modal.test.tsx` + `DeleteSiteModal.test.tsx` (portal after scroll, label layout, confirm flow) |

## CI results

_(filled after green Windows + macOS smoke)_

## Release assets

| Asset | Notes |
|-------|-------|
| `DevStack_0.3.6_aarch64.dmg` | macOS arm64 |
| `DevStack_0.3.6_x64.dmg` | macOS x64 |
| `DevStack_0.3.6_x64-setup.exe` | Windows NSIS |
| `DevStack_0.3.6_x64_en-US.msi` | Windows MSI |
