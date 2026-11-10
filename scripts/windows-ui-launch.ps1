# Launch the built DevStack exe and assert Responding + DevStack title within 3s.
# Also exercise minimize/restore (custom titlebar path) and record timings JSON.
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$DataDir,
  [string]$TimingsJson = ""
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Exe)) { throw "devstack.exe not found: $Exe" }
Write-Host "Launching $Exe"
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$proc = Start-Process -FilePath $Exe -PassThru -WindowStyle Normal
$deadline = (Get-Date).AddSeconds(3)
$okTitle = $false
$okResponding = $false
$titleMs = $null

try {
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 100
    $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
    if (-not $p) { throw "devstack.exe exited during startup (code $($proc.ExitCode))" }
    if ($p.Responding) { $okResponding = $true }
    if ($p.MainWindowTitle -match 'DevStack') {
      $okTitle = $true
      if (-not $titleMs) { $titleMs = $sw.ElapsedMilliseconds }
    }
    if ($okResponding -and $okTitle) { break }
  }

  $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
  if (-not $p) { throw "devstack.exe exited before checks completed" }
  if (-not $p.Responding) {
    throw "AppHang: process not Responding within 3s (title='$($p.MainWindowTitle)')"
  }
  if ($p.MainWindowTitle -notmatch 'DevStack') {
    throw "Window title was '$($p.MainWindowTitle)' (expected DevStack) within 3s"
  }

  $logFile = Join-Path $DataDir 'logs\devstack.log'
  $deadlineLog = (Get-Date).AddSeconds(5)
  $sawSetup = $false
  $sawTray = $false
  while ((Get-Date) -lt $deadlineLog) {
    if (Test-Path $logFile) {
      if (Select-String -Path $logFile -Pattern 'setup_complete|Tauri setup complete' -Quiet -ErrorAction SilentlyContinue) {
        $sawSetup = $true
      }
      if (Select-String -Path $logFile -Pattern 'tray|system tray|Tray' -Quiet -ErrorAction SilentlyContinue) {
        $sawTray = $true
      }
    }
    if ($sawSetup) { break }
    Start-Sleep -Milliseconds 100
  }
  if (-not $sawSetup) {
    Write-Host "WARN: setup_complete not yet in logs (window still responded)"
  }

  if ($p.MainWindowHandle -eq [IntPtr]::Zero) {
    throw "MainWindowHandle is zero — custom titlebar window missing"
  }

  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class NativeWin {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
}
'@

  $hwnd = $p.MainWindowHandle
  [void][NativeWin]::ShowWindow($hwnd, 6) # SW_MINIMIZE
  Start-Sleep -Milliseconds 200
  [void][NativeWin]::ShowWindow($hwnd, 9) # SW_RESTORE
  Start-Sleep -Milliseconds 200
  $p2 = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
  if (-not $p2 -or -not $p2.Responding) {
    throw "App hung after minimize/restore (titlebar control path)"
  }

  Write-Host "Launch OK: title=DevStack responding=True titleMs=$titleMs trayLog=$sawTray setup=$sawSetup"

  if (-not $TimingsJson) {
    if ($env:RUNNER_TEMP) {
      $TimingsJson = Join-Path $env:RUNNER_TEMP 'devstack-ui-launch.json'
    } else {
      $TimingsJson = Join-Path $env:TEMP 'devstack-ui-launch.json'
    }
  }
  @{
    titleMs = $titleMs
    responding = $true
    trayLogSeen = $sawTray
    setupComplete = $sawSetup
    mainWindowHandle = [int64]$hwnd
  } | ConvertTo-Json | Set-Content -Path $TimingsJson -Encoding utf8
  Write-Host "UI timings → $TimingsJson"
}
finally {
  if ($proc -and -not $proc.HasExited) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    Get-Process -Name 'devstack' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  }
}
