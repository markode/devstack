# Enable OpenSSH Server + non-admin user for hosting SFTP E2E (password auth).
# Hard-fails if sshd or the probe cannot be established — hosting must PASS before release.

$ErrorActionPreference = 'Stop'

function Write-HostingEnv {
  param(
    [string]$HostName,
    [string]$Port,
    [string]$User,
    [string]$Password,
    [string]$Remote,
    [string]$Key = ''
  )
  $env:DEVSTACK_HOSTING_SSH_HOST = $HostName
  $env:DEVSTACK_HOSTING_SSH_PORT = $Port
  $env:DEVSTACK_HOSTING_SSH_USER = $User
  $env:DEVSTACK_HOSTING_SSH_PASSWORD = $Password
  $env:DEVSTACK_HOSTING_SSH_REMOTE = $Remote
  if ($Key) { $env:DEVSTACK_HOSTING_SSH_KEY = $Key }
  if ($env:GITHUB_ENV) {
    @(
      "DEVSTACK_HOSTING_SSH_HOST=$HostName"
      "DEVSTACK_HOSTING_SSH_PORT=$Port"
      "DEVSTACK_HOSTING_SSH_USER=$User"
      "DEVSTACK_HOSTING_SSH_PASSWORD=$Password"
      "DEVSTACK_HOSTING_SSH_REMOTE=$Remote"
    ) | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
    if ($Key) {
      "DEVSTACK_HOSTING_SSH_KEY=$Key" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
    }
  }
  Write-Host "Hosting SSH ready host=$HostName user=$User remote=$Remote (password auth)"
}

function Set-SshdPasswordAuth {
  param([string]$ConfigPath, [string]$UserName)
  $lines = @(Get-Content -LiteralPath $ConfigPath -ErrorAction Stop)
  $out = New-Object System.Collections.Generic.List[string]
  $skipMatch = $false
  foreach ($line in $lines) {
    $trim = $line.Trim()
    if ($trim -match "^Match\s+") {
      # Entering any Match block ends our previous Match User skip.
      $skipMatch = $trim -match "^Match User\s+$([regex]::Escape($UserName))\b"
      if ($skipMatch) { continue }
    } elseif ($skipMatch) {
      # Skip body of our previous Match User block (indented directives).
      if ($trim -eq '' -or $line -match '^\s+') { continue }
      $skipMatch = $false
    }
    # Drop global PasswordAuthentication / KbdInteractive lines (commented or not).
    if ($trim -match '^(#\s*)?PasswordAuthentication\b') { continue }
    if ($trim -match '^(#\s*)?KbdInteractiveAuthentication\b') { continue }
    if ($trim -match '^(#\s*)?ChallengeResponseAuthentication\b') { continue }
    $out.Add($line) | Out-Null
  }
  $out.Add('') | Out-Null
  $out.Add('PasswordAuthentication yes') | Out-Null
  $out.Add('KbdInteractiveAuthentication yes') | Out-Null
  $out.Add("Match User $UserName") | Out-Null
  $out.Add('    PasswordAuthentication yes') | Out-Null
  $out.Add('    PubkeyAuthentication yes') | Out-Null
  Set-Content -LiteralPath $ConfigPath -Value $out -Encoding ascii
}

$sshUser = 'devstackssh'
# Avoid '!' — cmd.exe delayed-expansion corrupts askpass echo of passwords containing it.
$plain = 'DevStack-CI-Ssh-9'
$secure = ConvertTo-SecureString $plain -AsPlainText -Force

# Install / start OpenSSH Server.
$svc = Get-Service -Name sshd -ErrorAction SilentlyContinue
if (-not $svc) {
  Write-Host 'Installing OpenSSH.Server…'
  Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0 | Out-Null
  $svc = Get-Service -Name sshd -ErrorAction Stop
}
if ($svc.Status -ne 'Running') {
  Start-Service sshd
}
Set-Service -Name sshd -StartupType Automatic

# Non-admin local user (avoids administrators_authorized_keys).
if (-not (Get-LocalUser -Name $sshUser -ErrorAction SilentlyContinue)) {
  New-LocalUser -Name $sshUser -Password $secure -PasswordNeverExpires -UserMayNotChangePassword |
    Out-Null
  Add-LocalGroupMember -Group 'Users' -Member $sshUser -ErrorAction SilentlyContinue
} else {
  Set-LocalUser -Name $sshUser -Password $secure
}
# Ensure account is active and password is what we expect (net user is authoritative on Windows).
& net user $sshUser $plain /active:yes | Out-Host

$userProfile = Join-Path 'C:\Users' $sshUser
if (-not (Test-Path -LiteralPath $userProfile)) {
  New-Item -ItemType Directory -Force -Path $userProfile | Out-Null
  # Seed a minimal profile so OpenSSH can resolve the home directory.
  New-Item -ItemType Directory -Force -Path (Join-Path $userProfile '.ssh') | Out-Null
}

$sshdConfig = Join-Path $env:ProgramData 'ssh\sshd_config'
if (-not (Test-Path -LiteralPath $sshdConfig)) {
  throw "sshd_config missing at $sshdConfig"
}
Set-SshdPasswordAuth -ConfigPath $sshdConfig -UserName $sshUser
Write-Host 'sshd_config password auth enforced; restarting sshd…'
Restart-Service sshd -Force
Start-Sleep -Seconds 3

$remote = Join-Path $userProfile 'devstack-hosting-remote'
New-Item -ItemType Directory -Force -Path $remote | Out-Null
icacls $remote /grant:r "${sshUser}:(OI)(CI)F" /grant:r "Administrators:(OI)(CI)F" /grant:r "SYSTEM:(OI)(CI)F" | Out-Null
# Home + .ssh must be traversable by the user for password sessions.
icacls $userProfile /grant:r "${sshUser}:(OI)(CI)F" | Out-Null
$remotePosix = ($remote -replace '\\', '/')

# Port must be open.
$listening = Get-NetTCPConnection -LocalPort 22 -State Listen -ErrorAction SilentlyContinue
if (-not $listening) {
  try {
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.Connect('127.0.0.1', 22)
    $tcp.Close()
  } catch {
    throw "sshd is not listening on :22"
  }
}

# Password auth probe. Use a .cmd askpass with delayed-expansion disabled so the
# password is echoed literally (prior failure: Permission denied with askpass).
$ask = Join-Path $env:TEMP 'devstack-ssh-askpass.cmd'
@(
  '@echo off'
  'setlocal DisableDelayedExpansion'
  "echo $plain"
) | Set-Content -Path $ask -Encoding ascii
$env:SSH_ASKPASS = $ask
$env:SSH_ASKPASS_REQUIRE = 'force'
$env:DISPLAY = 'unused'
$probeOk = $false
$lastErr = ''
try {
  for ($attempt = 1; $attempt -le 5; $attempt++) {
    Write-Host "SSH password probe attempt $attempt…"
    $out = & ssh -o StrictHostKeyChecking=accept-new `
      -o UserKnownHostsFile=NUL `
      -o PreferredAuthentications=password `
      -o PubkeyAuthentication=no `
      -o NumberOfPasswordPrompts=1 `
      -o BatchMode=no `
      -o ConnectTimeout=10 `
      "${sshUser}@127.0.0.1" "echo ok" 2>&1
    $code = $LASTEXITCODE
    Write-Host ($out | Out-String)
    if ($code -eq 0) {
      $probeOk = $true
      break
    }
    $lastErr = "exit $code :: $out"
    Start-Sleep -Seconds 2
  }
  if (-not $probeOk) {
    Write-Host '--- sshd_config (tail) ---'
    Get-Content -LiteralPath $sshdConfig -Tail 40 | Out-Host
    throw "ssh password probe to ${sshUser}@127.0.0.1 failed after retries ($lastErr)"
  }
}
finally {
  Remove-Item Env:SSH_ASKPASS -ErrorAction SilentlyContinue
  Remove-Item Env:SSH_ASKPASS_REQUIRE -ErrorAction SilentlyContinue
  Remove-Item Env:DISPLAY -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $ask -ErrorAction SilentlyContinue
}

Write-HostingEnv -HostName '127.0.0.1' -Port '22' -User $sshUser -Password $plain -Remote $remotePosix
