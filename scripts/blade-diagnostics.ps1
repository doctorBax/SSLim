param([Parameter(Mandatory = $true)][string]$OutputPath)

# Explicit allowlist only. No profiles, network addresses, routes, or raw logs.
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'sslim.exe'
$version = $null
if (Test-Path -LiteralPath $exe -PathType Leaf) {
    try {
        $candidate = & $exe --version 2>$null | Select-Object -First 1
        if ($LASTEXITCODE -eq 0 -and $candidate -match '^sslim [0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?$') { $version = $candidate }
    } catch { }
}
$adapters = @()
try {
    $adapters = @(Get-NetAdapter -Name 'Blade' -ErrorAction Stop | ForEach-Object {
        $adapter = $_
        $stats = Get-NetAdapterStatistics -Name $adapter.Name -ErrorAction SilentlyContinue
        [ordered]@{
            status = [string]$adapter.Status
            link_speed = [string]$adapter.LinkSpeed
            received_bytes = if ($stats) { [long]$stats.ReceivedBytes } else { $null }
            sent_bytes = if ($stats) { [long]$stats.SentBytes } else { $null }
        }
    })
} catch { }
$report = [ordered]@{
    schema = 1
    generated_utc = [DateTime]::UtcNow.ToString('o')
    blade_version = $version
    windows_version = [Environment]::OSVersion.Version.ToString()
    blade_adapters = $adapters
}
$report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
Write-Host 'Diagnostics saved. Review the file before sharing.'
