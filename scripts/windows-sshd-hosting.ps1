# Enable OpenSSH Server on the Windows runner and prepare a key-auth target for hosting SFTP E2E.
# Exports DEVSTACK_HOSTING_SSH_* for the subsequent smoke/matrix step (caller must re-invoke smoke
# or already have the env vars in the same shell).

$ErrorActionPreference = 'Stop'

$sshDir = Join-Path $env:USERPROFILE '.ssh'
New-Item -ItemType Directory -Force -Path $sshDir | Out-Null
$key = Join-Path $sshDir 'devstack_hosting_ed25519'
if (-not (Test-Path $key)) {
  & ssh-keygen -t ed25519 -N '""' -f $key | Out-Null
}
$pub = (Get-Content "$key.pub" -Raw).Trim()
$auth = Join-Path $sshDir 'authorized_keys'
if (-not (Test-Path $auth) -or -not (Select-String -Path $auth -Pattern 'devstack_hosting' -Quiet -ErrorAction SilentlyContinue)) {
  Add-Content -Path $auth -Value $pub
}
# Administrators on Windows OpenSSH read ProgramData keys, not ~/.ssh/authorized_keys.
$adminAuth = Join-Path $env:ProgramData 'ssh\administrators_authorized_keys'
New-Item -ItemType Directory -Force -Path (Split-Path $adminAuth) | Out-Null
if (-not (Test-Path $adminAuth) -or -not (Select-String -Path $adminAuth -Pattern 'devstack_hosting' -Quiet -ErrorAction SilentlyContinue)) {
  Add-Content -Path $adminAuth -Value $pub
  # Restrict ACL the way OpenSSH expects.
  icacls $adminAuth /inheritance:r /grant "Administrators:F" /grant "SYSTEM:F" | Out-Null
}

# Install / start OpenSSH Server if needed.
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

# Ensure public-key auth is allowed.
$sshdConfig = Join-Path $env:ProgramData 'ssh\sshd_config'
if (Test-Path $sshdConfig) {
  $raw = Get-Content $sshdConfig -Raw
  if ($raw -notmatch '(?m)^PubkeyAuthentication\s+yes') {
    Add-Content -Path $sshdConfig -Value "`nPubkeyAuthentication yes`nAuthorizedKeysFile .ssh/authorized_keys`n"
    Restart-Service sshd
    Start-Sleep -Seconds 2
  }
}

$remote = Join-Path $env:USERPROFILE "devstack-hosting-remote"
New-Item -ItemType Directory -Force -Path $remote | Out-Null
# Use forward slashes for SFTP remote path.
$remotePosix = ($remote -replace '\\', '/')

$env:DEVSTACK_HOSTING_SSH_HOST = '127.0.0.1'
$env:DEVSTACK_HOSTING_SSH_PORT = '22'
$env:DEVSTACK_HOSTING_SSH_USER = $env:USERNAME
$env:DEVSTACK_HOSTING_SSH_KEY = $key
$env:DEVSTACK_HOSTING_SSH_REMOTE = $remotePosix

Write-Host "Hosting SSH ready host=127.0.0.1 user=$env:USERNAME key=$key remote=$remotePosix"

# Smoke connectivity
& ssh -i $key -o StrictHostKeyChecking=accept-new -o BatchMode=yes "127.0.0.1" "echo ok" | Out-Host
if ($LASTEXITCODE -ne 0) {
  throw "ssh localhost failed ($LASTEXITCODE) — public-key auth not working"
}
