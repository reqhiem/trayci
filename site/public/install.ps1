# Installs Trayci on Windows from a GitHub release.
#
#   irm https://reqhiem.github.io/trayci/install.ps1 | iex
#
# Environment:
#   TRAYCI_VERSION  exact version to install, e.g. 0.4.5 (default: latest)
#
# Downloads the NSIS installer, checks it against the SHA-256 digest GitHub
# records for the asset, and runs it silently for the current user, so no UAC
# prompt. A running Trayci is closed by the installer and started again after.
# Everything lives in one function: `iex` runs in the caller's session, so this
# never calls `exit` or leaves variables behind. Keep this file ASCII-only.

function Install-Trayci {
  $ErrorActionPreference = 'Stop'
  # Windows PowerShell 5.1 crawls while it draws progress, and may default to TLS 1.0.
  $ProgressPreference = 'SilentlyContinue'
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

  Write-Host ''
  Write-Host '  Trayci installer'
  Write-Host ''
  if (-not [Environment]::Is64BitOperatingSystem) { throw 'Trayci needs 64-bit Windows.' }

  $repo = 'reqhiem/trayci'
  $releaseUrl = "https://api.github.com/repos/$repo/releases/latest"
  if ($env:TRAYCI_VERSION) {
    $releaseUrl = "https://api.github.com/repos/$repo/releases/tags/v$($env:TRAYCI_VERSION -replace '^v')"
  }
  Write-Host '  Finding your release...' -ForegroundColor DarkGray
  try {
    $release = Invoke-RestMethod $releaseUrl -Headers @{ 'User-Agent' = 'trayci-install' }
  } catch {
    throw "Could not find the Trayci release at $releaseUrl. $($_.Exception.Message)"
  }
  $version = $release.tag_name -replace '^v'
  $asset = $release.assets | Where-Object { $_.name -like '*_x64-setup.exe' } | Select-Object -First 1
  if (-not $asset) { throw "Trayci $version has no Windows installer." }
  if ("$($asset.digest)" -match '^sha256:([0-9a-f]{64})$') { $expected = $Matches[1] }
  else { throw "GitHub has no SHA-256 digest for $($asset.name), so it cannot be verified." }

  Write-Host "  Installing Trayci $version" -ForegroundColor White
  $installer = Join-Path ([IO.Path]::GetTempPath()) $asset.name
  try {
    Write-Host '  Downloading...' -ForegroundColor DarkGray
    Invoke-WebRequest $asset.browser_download_url -OutFile $installer -UseBasicParsing
    Write-Host '  Verifying the download...' -ForegroundColor DarkGray
    if ((Get-FileHash $installer -Algorithm SHA256).Hash -ne $expected) {
      throw "Checksum mismatch for $($asset.name)."
    }
    Write-Host '  Running the installer...' -ForegroundColor DarkGray
    $process = Start-Process $installer -ArgumentList '/S' -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "The installer exited with code $($process.ExitCode)." }
  } finally {
    Remove-Item $installer -Force -ErrorAction SilentlyContinue
  }

  Write-Host ''
  Write-Host "  Installed Trayci $version" -ForegroundColor Green
  $exe = Join-Path $env:LOCALAPPDATA 'Trayci\trayci.exe'
  if (Test-Path $exe) {
    Start-Process $exe
    Write-Host '  Trayci is starting in your tray.'
  } else {
    Write-Host '  Open Trayci from the Start menu.'
  }
  Write-Host ''
}

Install-Trayci
