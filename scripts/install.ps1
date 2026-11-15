# SSLim Blade installer: copies the release files and adds the directory to the user PATH.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File install.ps1
param(
    [string]$Destination = "$env:LOCALAPPDATA\sslim"
)

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot

foreach ($name in @("sslim.exe", "blade-routes.ps1", "wintun.dll")) {
    $source = Join-Path $root $name
    if (-not (Test-Path $source)) {
        Write-Error "missing $source - run this script from the unpacked release"
    }
}

New-Item -ItemType Directory -Force -Path $Destination | Out-Null
foreach ($name in @("sslim.exe", "blade-routes.ps1", "wintun.dll")) {
    Copy-Item (Join-Path $root $name) $Destination -Force
}

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$Destination*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$Destination", "User")
    Write-Host "Added $Destination to the user PATH; open a new terminal for it to apply."
}

Write-Host "Installed. Try: sslim blade import `"sslim://...`""; sslim blade --stats"
