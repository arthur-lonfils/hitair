# hitair installer for Windows (PowerShell).
#   irm https://raw.githubusercontent.com/arthur-lonfils/hitair/main/install.ps1 | iex
$ErrorActionPreference = 'Stop'

$repo  = 'arthur-lonfils/hitair'
$asset = 'hitair-windows-x86_64.zip'
$url   = "https://github.com/$repo/releases/latest/download/$asset"
$dir   = Join-Path $env:LOCALAPPDATA 'hitair'

Write-Host "Downloading $asset…"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$zip = Join-Path $env:TEMP $asset
Invoke-WebRequest -Uri $url -OutFile $zip
Expand-Archive -Path $zip -DestinationPath $dir -Force
Remove-Item $zip
Write-Host "Installed hitair.exe to $dir"

# Add the install dir to the user PATH if it isn't there yet.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$dir*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$dir", 'User')
    Write-Host "Added $dir to your PATH — restart your terminal to pick it up."
}

Write-Host "Done. Run: hitair"
