; DevStack NSIS hooks — remove opt-in Defender exclusions on uninstall.
; Exclusions are never added by the installer; only Settings → Performance
; (user opt-in, UAC) writes helper\defender-exclusions.txt.

!macro NSIS_HOOK_PREUNINSTALL
  ; Best-effort: remove any ExclusionPath entries recorded while the app ran.
  ; Uses HKCU AppData\Local\DevStack (matches dirs::data_dir on Windows).
  nsExec::ExecToLog 'powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "$$m = Join-Path $$env:LOCALAPPDATA \"DevStack\\helper\\defender-exclusions.txt\"; if (Test-Path -LiteralPath $$m) { Get-Content -LiteralPath $$m | ForEach-Object { if ($$_) { Remove-MpPreference -ExclusionPath $$_ -ErrorAction SilentlyContinue } }; Remove-Item -LiteralPath $$m -Force -ErrorAction SilentlyContinue }"'
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
