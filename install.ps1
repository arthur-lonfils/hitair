# hitair installer for Windows (PowerShell).
#   irm https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.ps1 | iex
#
# Installs both the desktop GUI (hitair-gui.exe) and the terminal app (hitair.exe).
$ErrorActionPreference = 'Stop'

$repo = 'arthur-lonfils/hitair'
$dir  = Join-Path $env:LOCALAPPDATA 'hitair'
$base = "https://github.com/$repo/releases/latest/download"
New-Item -ItemType Directory -Force -Path $dir | Out-Null

function Install-Bin($name) {
    $asset = "$name-windows-x86_64.zip"
    try {
        Write-Host "Downloading $asset…"
        $zip = Join-Path $env:TEMP $asset
        Invoke-WebRequest -Uri "$base/$asset" -OutFile $zip
        Expand-Archive -Path $zip -DestinationPath $dir -Force
        Remove-Item $zip
        Write-Host "Installed $name.exe to $dir"
        return $true
    } catch {
        return $false
    }
}

if (-not (Install-Bin 'hitair')) { throw "could not download hitair for Windows" }
$gui = Install-Bin 'hitair-gui'

# Add the install dir to the user PATH if it isn't there yet.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$dir*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
    Write-Host "Added $dir to your PATH — restart your terminal to pick it up."
}

if ($gui) {
    Write-Host "Done. Run: hitair-gui (desktop) — or hitair (terminal)"
} else {
    Write-Host "Done. Run: hitair"
}
