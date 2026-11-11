; DevStack NSIS hooks.
; - PREINSTALL: close a running DevStack (and job-object children) before file replace.
;   Silent upgrades over a running 0.3.x previously left the old exe locked/in place.
; - PREUNINSTALL: remove opt-in Defender exclusions (Settings → Performance only).

!include "LogicLib.nsh"
!include "WinMessages.nsh"

!macro DevStack_CloseRunningApp
  ; Soft close via WM_CLOSE on the main window (title "DevStack"), then force-kill.
  ; Job Object KILL_ON_JOB_CLOSE tears down nginx/httpd/php-cgi/mysqld children with the app.
  nsExec::ExecToLog 'taskkill /IM DevStack.exe /T /F'
  nsExec::ExecToLog 'taskkill /IM devstack.exe /T /F'
  ; Brief wait so file locks release before NSIS copies.
  Sleep 800
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro DevStack_CloseRunningApp
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro DevStack_CloseRunningApp
  ; Best-effort: remove any ExclusionPath entries recorded while the app ran.
  ; Uses HKCU AppData\Local\DevStack (matches dirs::data_dir on Windows).
  nsExec::ExecToLog 'powershell -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command "$$m = Join-Path $$env:LOCALAPPDATA \"DevStack\\helper\\defender-exclusions.txt\"; if (Test-Path -LiteralPath $$m) { Get-Content -LiteralPath $$m | ForEach-Object { if ($$_) { Remove-MpPreference -ExclusionPath $$_ -ErrorAction SilentlyContinue } }; Remove-Item -LiteralPath $$m -Force -ErrorAction SilentlyContinue }"'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
!macroend
