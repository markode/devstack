# Fail if a PE binary depends on non-system DLLs (Homebrew-style portable gate).
# Allowed: KERNEL32/USER32/… system DLLs, VCRUNTIME/MSVCP/ucrtbase, WEBVIEW2LOADER*.
param(
  [Parameter(Mandatory = $true)][string]$Exe
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Exe)) {
  throw "exe not found: $Exe"
}

function Get-Dumpbin {
  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  if (Test-Path -LiteralPath $vswhere) {
    $root = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($root) {
      $cand = Get-ChildItem -Path (Join-Path $root 'VC\Tools\MSVC') -Recurse -Filter dumpbin.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '\\Hostx64\\x64\\dumpbin\.exe$' } |
        Select-Object -First 1
      if ($cand) { return $cand.FullName }
    }
  }
  $cmd = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  return $null
}

$dumpbin = Get-Dumpbin
if (-not $dumpbin) {
  throw 'dumpbin.exe not found — install VS Build Tools / run from Developer PowerShell'
}

Write-Host "=== dumpbin /dependents $Exe ==="
$out = & $dumpbin /dependents $Exe 2>&1 | Out-String
Write-Host $out

$allowed = @(
  'KERNEL32.DLL','USER32.DLL','GDI32.DLL','ADVAPI32.DLL','SHELL32.DLL','OLE32.DLL',
  'OLEAUT32.DLL','COMDLG32.DLL','COMCTL32.DLL','SHLWAPI.DLL','WS2_32.DLL','WINMM.DLL',
  'IMM32.DLL','VERSION.DLL','SETUPAPI.DLL','CRYPT32.DLL','BCRYPT.DLL','NCRYPT.DLL',
  'SECUR32.DLL','NETAPI32.DLL','WINTRUST.DLL','IPHLPI.DLL','IPHLAPI.DLL','NSI.DLL',
  'DNSAPI.DLL','WINHTTP.DLL','WININET.DLL','URLMON.DLL','PROPSYS.DLL','DWMAPI.DLL',
  'UXTHME.DLL','UXTHEME.DLL','D3D11.DLL','DXGI.DLL','DWRITE.DLL','CFGMGR32.DLL',
  'POWRPROF.DLL','RSTRTMGR.DLL','NTDLL.DLL','RPCRT4.DLL','MSVCRT.DLL','UCRTBASE.DLL',
  'VCRUNTIME140.DLL','VCRUNTIME140_1.DLL','MSVCP140.DLL','MSVCP140_1.DLL','MSVCP140_2.DLL',
  'CONCRT140.DLL','API-MS-WIN-*.DLL','WEBVIEW2LOADER.DLL','WEBVIEW2LOADER.DLL.DLL',
  'WINDOWSCODECSEXT.DLL','WINDOWSCODECS.DLL','MFPLAT.DLL','MF.DLL','MFREADWRITE.DLL',
  'WLDAP32.DLL','NORMALIZ.DLL','DBGHELP.DLL','BCYPTPRIMITIVES.DLL',
  # Common system DLLs seen on Tauri/WebView2 release builds:
  'COMBASE.DLL','PDH.DLL','PSAPI.DLL','USERENV.DLL','NETUTILS.DLL','AUTHZ.DLL',
  'WINSPOOL.DRV','IPHLPAPI.DLL','ONDEMANDCONNROUTEHELPER.DLL'
)

$deps = @()
$in = $false
foreach ($line in ($out -split "`r?`n")) {
  $t = $line.Trim()
  if ($t -match '^Image has the following dependencies') { $in = $true; continue }
  # dumpbin inserts a blank line after the header — do NOT end the section on blank lines.
  if ($in -and ($t -match '^Summary' -or $t -match '^Image has the following delay')) { $in = $false; continue }
  if ($in -and $t -match '\.DLL$') { $deps += $t.ToUpperInvariant() }
}

if ($deps.Count -eq 0) {
  Write-Error 'FAIL: dumpbin reported zero DLL dependents — parser bug or non-PE input'
  exit 1
}
Write-Host ("Parsed $($deps.Count) dependents: " + ($deps -join ', '))

$bad = @()
foreach ($d in $deps) {
  $ok = $false
  foreach ($a in $allowed) {
    if ($a.Contains('*')) {
      $pat = '^' + [regex]::Escape($a).Replace('\*','.*') + '$'
      if ($d -match $pat) { $ok = $true; break }
    } elseif ($d -eq $a) { $ok = $true; break }
  }
  # Also allow api-ms-win-* and ext-ms-* umbrella DLLs.
  if ($d -match '^API-MS-WIN-' -or $d -match '^EXT-MS-') { $ok = $true }
  if ($d -match '^WEBVIEW2LOADER') { $ok = $true }
  if (-not $ok) { $bad += $d }
}

if ($bad.Count -gt 0) {
  Write-Error ("FAIL: non-system DLL dependents:`n  " + ($bad -join "`n  "))
  exit 1
}
Write-Host "OK: $Exe dependents are system / WebView2 only"
exit 0
