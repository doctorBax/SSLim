# Install Blade from an unpacked Windows release or fetch the latest release.
param(
    [string]$Destination = "$env:LOCALAPPDATA\sslim",
    [switch]$NoLaunch
)

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$temporary = $null
$required = @("sslim.exe", "blade-routes.ps1", "blade-tray.ps1", "blade-diagnostics.ps1", "wintun.dll")

try {
    if (-not $root -or @($required | Where-Object { -not (Test-Path -LiteralPath (Join-Path $root $_)) }).Count -gt 0) {
        Write-Host "Downloading the latest Blade release..."
        $release = Invoke-RestMethod -Headers @{ "User-Agent" = "SSLim-Blade-Installer" } `
            -Uri "https://api.github.com/repos/doctorBax/sslim/releases/latest"
        $asset = $release.assets | Where-Object { $_.name -eq "sslim-windows-x86_64.zip" } | Select-Object -First 1
        if (-not $asset) {
            throw "The latest GitHub release does not contain the Windows package yet."
        }
        $temporary = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
        New-Item -ItemType Directory -Path $temporary | Out-Null
        $archive = Join-Path $temporary "sslim-windows-x86_64.zip"
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $archive
        $root = Join-Path $temporary "package"
        Expand-Archive -LiteralPath $archive -DestinationPath $root
    }

    foreach ($name in $required) {
        $source = Join-Path $root $name
        if (-not (Test-Path $source)) {
            throw "The package is incomplete. Missing $source"
        }
    }

    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    foreach ($name in $required) {
        Copy-Item (Join-Path $root $name) $Destination -Force
    }

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathEntries = @($userPath -split ";" | Where-Object { $_ })
    if ($pathEntries -notcontains $Destination) {
        [Environment]::SetEnvironmentVariable("Path", (@($pathEntries + $Destination) -join ";"), "User")
        Write-Host "Added Blade to your user PATH."
    }

    Write-Host "Blade is installed. Confirm the Windows administrator prompt to connect."
    if (-not $NoLaunch) {
        Start-Process -FilePath "powershell.exe" -WindowStyle Hidden -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + (Join-Path $Destination 'blade-tray.ps1') + '"')) `
            -WorkingDirectory $Destination -Verb RunAs
    }
} finally {
    if ($temporary -and (Test-Path -LiteralPath $temporary)) {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}
