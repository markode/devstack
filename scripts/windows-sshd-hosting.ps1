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

$sshUser = 'devstackssh'
$plain = 'DevStack-CI-Ssh-1!'
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

$userProfile = Join-Path 'C:\Users' $sshUser
if (-not (Test-Path $userProfile)) {
  New-Item -ItemType Directory -Force -Path $userProfile | Out-Null
}

$sshdConfig = Join-Path $env:ProgramData 'ssh\sshd_config'
if (Test-Path $sshdConfig) {
  $raw = Get-Content $sshdConfig -Raw
  $changed = $false
  if ($raw -notmatch '(?m)^PubkeyAuthentication\s+yes') {
    Add-Content -Path $sshdConfig -Value "`nPubkeyAuthentication yes"
    $changed = $true
  }
  if ($raw -match '(?m)^PasswordAuthentication\s+no') {
    (Get-Content $sshdConfig) |
      ForEach-Object { $_ -replace '^PasswordAuthentication\s+no', 'PasswordAuthentication yes' } |
      Set-Content $sshdConfig
    $changed = $true
  } elseif ($raw -notmatch '(?m)^PasswordAuthentication\s+yes') {
    Add-Content -Path $sshdConfig -Value "`nPasswordAuthentication yes"
    $changed = $true
  }
  # Ensure Match Group administrators block doesn't disable password for our user.
  if ($changed) {
    Restart-Service sshd
    Start-Sleep -Seconds 3
  }
}

$remote = Join-Path $userProfile 'devstack-hosting-remote'
New-Item -ItemType Directory -Force -Path $remote | Out-Null
icacls $remote /grant:r "${sshUser}:(OI)(CI)F" /grant:r "Administrators:(OI)(CI)F" /grant:r "SYSTEM:(OI)(CI)F" | Out-Null
$remotePosix = ($remote -replace '\\', '/')

# Port must be open.
$listening = Get-NetTCPConnection -LocalPort 22 -State Listen -ErrorAction SilentlyContinue
if (-not $listening) {
  # Fallback probe
  try {
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.Connect('127.0.0.1', 22)
    $tcp.Close()
  } catch {
    throw "sshd is not listening on :22"
  }
}

# Password auth probe via plink-less PowerShell SSH (OpenSSH client).
# Use SSH_ASKPASS-style: echo password to ssh with PreferredAuthentications=password.
$ask = Join-Path $env:TEMP 'devstack-ssh-askpass.cmd'
# cmd script that prints the password once
"@echo off`r`necho $plain`r`n" | Set-Content -Path $ask -Encoding ascii
$env:SSH_ASKPASS = $ask
$env:SSH_ASKPASS_REQUIRE = 'force'
$env:DISPLAY = 'unused'
try {
  & ssh -o StrictHostKeyChecking=accept-new `
    -o PreferredAuthentications=password `
    -o PubkeyAuthentication=no `
    -o NumberOfPasswordPrompts=1 `
    -o BatchMode=no `
    "${sshUser}@127.0.0.1" "echo ok" | Out-Host
  if ($LASTEXITCODE -ne 0) {
    throw "ssh password probe to ${sshUser}@127.0.0.1 failed ($LASTEXITCODE)"
  }
}
finally {
  Remove-Item Env:SSH_ASKPASS -ErrorAction SilentlyContinue
  Remove-Item Env:SSH_ASKPASS_REQUIRE -ErrorAction SilentlyContinue
  Remove-Item Env:DISPLAY -ErrorAction SilentlyContinue
  Remove-Item $ask -ErrorAction SilentlyContinue
}

Write-HostingEnv -HostName '127.0.0.1' -Port '22' -User $sshUser -Password $plain -Remote $remotePosix
