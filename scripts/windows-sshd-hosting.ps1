# Enable OpenSSH Server on the Windows runner and prepare key-auth for hosting SFTP E2E.
# Prefers a non-admin local user (avoids administrators_authorized_keys ACL traps).
# On failure: exits 0 without setting DEVSTACK_HOSTING_SSH_* so the matrix marks hosting live as disabled.

$ErrorActionPreference = 'Stop'

function Write-HostingEnv {
  param([string]$HostName, [string]$Port, [string]$User, [string]$Key, [string]$Remote)
  $env:DEVSTACK_HOSTING_SSH_HOST = $HostName
  $env:DEVSTACK_HOSTING_SSH_PORT = $Port
  $env:DEVSTACK_HOSTING_SSH_USER = $User
  $env:DEVSTACK_HOSTING_SSH_KEY = $Key
  $env:DEVSTACK_HOSTING_SSH_REMOTE = $Remote
  if ($env:GITHUB_ENV) {
    @(
      "DEVSTACK_HOSTING_SSH_HOST=$HostName"
      "DEVSTACK_HOSTING_SSH_PORT=$Port"
      "DEVSTACK_HOSTING_SSH_USER=$User"
      "DEVSTACK_HOSTING_SSH_KEY=$Key"
      "DEVSTACK_HOSTING_SSH_REMOTE=$Remote"
    ) | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
  }
  Write-Host "Hosting SSH ready host=$HostName user=$User key=$Key remote=$Remote"
}

try {
  $sshDir = Join-Path $env:USERPROFILE '.ssh'
  New-Item -ItemType Directory -Force -Path $sshDir | Out-Null
  $key = Join-Path $sshDir 'devstack_hosting_ed25519'
  if (-not (Test-Path $key)) {
    & ssh-keygen -t ed25519 -N '""' -f $key | Out-Null
  }
  $pub = (Get-Content "$key.pub" -Raw).Trim()

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

  # Non-admin user avoids ProgramData administrators_authorized_keys ACL issues.
  $sshUser = 'devstackssh'
  $plain = 'DevStack-CI-Ssh-1!'
  $secure = ConvertTo-SecureString $plain -AsPlainText -Force
  if (-not (Get-LocalUser -Name $sshUser -ErrorAction SilentlyContinue)) {
    New-LocalUser -Name $sshUser -Password $secure -PasswordNeverExpires -UserMayNotChangePassword |
      Out-Null
    Add-LocalGroupMember -Group 'Users' -Member $sshUser -ErrorAction SilentlyContinue
  } else {
    Set-LocalUser -Name $sshUser -Password $secure
  }

  $userProfile = Join-Path 'C:\Users' $sshUser
  if (-not (Test-Path $userProfile)) {
    # Force profile creation by running a no-op as that user.
    $cred = New-Object System.Management.Automation.PSCredential($sshUser, $secure)
    Start-Process -FilePath 'cmd.exe' -ArgumentList '/c','echo profile' -Credential $cred -Wait -WindowStyle Hidden -ErrorAction SilentlyContinue
  }
  if (-not (Test-Path $userProfile)) {
    # Fallback: create the expected dirs manually (sshd accepts this for key auth).
    New-Item -ItemType Directory -Force -Path $userProfile | Out-Null
  }
  $userSsh = Join-Path $userProfile '.ssh'
  New-Item -ItemType Directory -Force -Path $userSsh | Out-Null
  $auth = Join-Path $userSsh 'authorized_keys'
  Set-Content -Path $auth -Value $pub -Encoding ascii
  # sshd requires restricted ACLs on authorized_keys.
  icacls $userSsh /inheritance:r /grant:r "${sshUser}:(OI)(CI)F" /grant:r "SYSTEM:(OI)(CI)F" /grant:r "Administrators:(OI)(CI)F" | Out-Null
  icacls $auth /inheritance:r /grant:r "${sshUser}:F" /grant:r "SYSTEM:F" /grant:r "Administrators:F" | Out-Null

  # Also keep admin keys correct for the runner account (best-effort).
  $adminAuth = Join-Path $env:ProgramData 'ssh\administrators_authorized_keys'
  New-Item -ItemType Directory -Force -Path (Split-Path $adminAuth) | Out-Null
  Set-Content -Path $adminAuth -Value $pub -Encoding ascii
  $acl = Get-Acl $adminAuth
  $acl.SetAccessRuleProtection($true, $false)
  $acl.Access | ForEach-Object { [void]$acl.RemoveAccessRule($_) }
  $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule('Administrators','FullControl','Allow')))
  $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule('SYSTEM','FullControl','Allow')))
  Set-Acl -Path $adminAuth -AclObject $acl

  $sshdConfig = Join-Path $env:ProgramData 'ssh\sshd_config'
  if (Test-Path $sshdConfig) {
    $raw = Get-Content $sshdConfig -Raw
    if ($raw -notmatch '(?m)^PubkeyAuthentication\s+yes') {
      Add-Content -Path $sshdConfig -Value "`nPubkeyAuthentication yes`nPasswordAuthentication yes`n"
    }
    Restart-Service sshd
    Start-Sleep -Seconds 2
  }

  $remote = Join-Path $userProfile 'devstack-hosting-remote'
  New-Item -ItemType Directory -Force -Path $remote | Out-Null
  icacls $remote /grant:r "${sshUser}:(OI)(CI)F" | Out-Null
  $remotePosix = ($remote -replace '\\', '/')

  # Probe key auth as the non-admin user.
  & ssh -i $key -o StrictHostKeyChecking=accept-new -o BatchMode=yes -o IdentitiesOnly=yes "${sshUser}@127.0.0.1" "echo ok" | Out-Host
  if ($LASTEXITCODE -ne 0) {
    throw "ssh ${sshUser}@127.0.0.1 failed ($LASTEXITCODE)"
  }

  Write-HostingEnv -HostName '127.0.0.1' -Port '22' -User $sshUser -Key $key -Remote $remotePosix
}
catch {
  Write-Host "WARN: OpenSSH hosting setup failed: $($_.Exception.Message)"
  Write-Host 'Continuing without DEVSTACK_HOSTING_SSH_* (matrix will mark hosting.sftp_live disabled).'
  exit 0
}
