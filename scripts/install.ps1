#!/usr/bin/env pwsh
# Installs Valle on Windows:
#
#   irm https://openvalle.ai/install.ps1 | iex
#
# It downloads the latest release's executable for this machine, checks it against the
# release's SHA256SUMS, and puts it in ~\.valle\bin, which it adds to your PATH.
# $env:VALLE_VERSION installs another release (v0.1.0), $env:VALLE_INSTALL another folder than
# ~\.valle.
$ErrorActionPreference = 'Stop'
# Windows PowerShell 5 draws a progress bar that slows a download many times over.
$ProgressPreference = 'SilentlyContinue'

function Fail([string]$Message) {
  Write-Host "error: $Message" -ForegroundColor Red
  exit 1
}

$Repository = 'openvalle/valle'
$Install = if ($env:VALLE_INSTALL) { $env:VALLE_INSTALL } else { Join-Path $HOME '.valle' }
$Bin = Join-Path $Install 'bin'

if ($PSVersionTable.PSVersion.Major -ge 6 -and -not $IsWindows) {
  Fail 'this script installs Valle on Windows; on macOS and Linux run: curl -fsSL https://openvalle.ai/install.sh | sh'
}

# The machine's architecture, not this process's: PowerShell may be running emulated.
$Machine = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').PROCESSOR_ARCHITECTURE
$Arch = switch ($Machine) {
  'AMD64' { 'x86_64' }
  'ARM64' { 'arm64' }
  default { Fail "Valle has no package for Windows on $Machine" }
}

[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Version = $env:VALLE_VERSION
if (-not $Version) {
  # The latest release, from where its page redirects to: no API, so no rate limit.
  try {
    $Request = [Net.WebRequest]::Create("https://github.com/$Repository/releases/latest")
    $Request.Method = 'HEAD'
    $Response = $Request.GetResponse()
    $Version = $Response.ResponseUri.Segments[-1]
    $Response.Close()
  } catch {
    Fail "could not reach github.com to find the latest release: $($_.Exception.Message)"
  }
}
if ($Version -match '^\d') { $Version = "v$Version" }
if ($Version -notmatch '^v\d') {
  Fail "could not tell the latest release (got `"$Version`"); set `$env:VALLE_VERSION, such as v0.1.0"
}

$Archive = "valle-$Version-windows-$Arch.zip"
$Url = "https://github.com/$Repository/releases/download/$Version"
$Temporary = Join-Path ([IO.Path]::GetTempPath()) "valle-install-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $Temporary | Out-Null

try {
  Write-Host "Downloading Valle $Version for Windows $Arch"
  $Zip = Join-Path $Temporary $Archive
  try {
    Invoke-WebRequest -Uri "$Url/$Archive" -OutFile $Zip -UseBasicParsing
    $Sums = (Invoke-WebRequest -Uri "$Url/SHA256SUMS" -UseBasicParsing).Content
  } catch {
    Fail "could not download $Url/$Archive : $($_.Exception.Message)"
  }
  if ($Sums -is [byte[]]) { $Sums = [Text.Encoding]::UTF8.GetString($Sums) }

  $Expected = $null
  foreach ($Line in ($Sums -split "`n")) {
    $Parts = $Line.Trim() -split '\s+', 2
    if ($Parts.Count -eq 2 -and $Parts[1].TrimStart('*') -eq $Archive) { $Expected = $Parts[0].ToLower() }
  }
  if (-not $Expected) { Fail "SHA256SUMS of $Version does not list $Archive" }
  $Actual = (Get-FileHash -Algorithm SHA256 -Path $Zip).Hash.ToLower()
  if ($Actual -ne $Expected) {
    Fail "$Archive does not match its checksum: expected $Expected, got $Actual"
  }

  Expand-Archive -Path $Zip -DestinationPath $Temporary -Force
  New-Item -ItemType Directory -Force -Path $Bin | Out-Null
  $Exe = Join-Path $Bin 'valle.exe'
  try {
    # Over a running valle.exe, which Windows will not overwrite: it can be renamed aside.
    if (Test-Path $Exe) {
      $Old = "$Exe.old"
      Remove-Item -Force $Old -ErrorAction SilentlyContinue
      Rename-Item -Path $Exe -NewName 'valle.exe.old'
    }
    Move-Item -Force -Path (Join-Path $Temporary 'valle.exe') -Destination $Exe
    Remove-Item -Force "$Exe.old" -ErrorAction SilentlyContinue
  } catch {
    Fail "could not write $Exe : $($_.Exception.Message)"
  }

  $Installed = & $Exe --version
  if ($LASTEXITCODE -ne 0) { Fail "$Exe does not run on this machine" }
  Write-Host "$Installed was installed to $Exe" -ForegroundColor Green

  # The user's PATH, kept in the registry: new terminals have it, and this one from now.
  $User = [Environment]::GetEnvironmentVariable('Path', 'User')
  $Folders = @($User -split ';' | Where-Object { $_ })
  if ($Folders -notcontains $Bin) {
    if ($env:VALLE_NO_MODIFY_PATH -eq '1') {
      Write-Host "`nAdd $Bin to your PATH."
    } else {
      [Environment]::SetEnvironmentVariable('Path', (($Folders + $Bin) -join ';'), 'User')
      Write-Host "Added $Bin to your PATH"
    }
  }
  if (($env:Path -split ';') -notcontains $Bin) { $env:Path = "$env:Path;$Bin" }

  Write-Host "`nRun valle docs start to begin."
  if (-not (Get-Command ffmpeg -ErrorAction SilentlyContinue)) {
    Write-Host "`nVideo needs FFmpeg 7, 8 or 9: https://github.com/$Repository/blob/main/docs/install.md#ffmpeg" -ForegroundColor DarkGray
  }
} finally {
  Remove-Item -Recurse -Force $Temporary -ErrorAction SilentlyContinue
}
