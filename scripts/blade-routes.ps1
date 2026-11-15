# Blade route setup for the Wintun adapter. Run elevated.
# Apply: -InterfaceIndex <n> -ServerIP <ip> -Prefixes "a.b.c.d/1,...,x.y.z.w/32" [-BlockIPv6]
# Revert: -Revert (removes the IPv6 firewall rule; interface routes die with the adapter)

param(
    [int]$InterfaceIndex,
    [string]$ServerIP,
    [string]$Prefixes,
    [switch]$BlockIPv6,
    [switch]$Revert
)

$ErrorActionPreference = "Stop"
$Rule = "Blade IPv6 outbound block"

if ($Revert) {
    Get-NetFirewallRule -DisplayName $Rule -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue
    exit 0
}

if (-not $InterfaceIndex -or -not $ServerIP -or -not $Prefixes) {
    Write-Error "usage: blade-routes.ps1 -InterfaceIndex <n> -ServerIP <ip> -Prefixes <csv> [-BlockIPv6] | -Revert"
    exit 1
}

$existing = Get-NetIPAddress -InterfaceIndex $InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue
if (-not $existing) {
    New-NetIPAddress -InterfaceIndex $InterfaceIndex -IPAddress $ServerIP -PrefixLength 32 -ErrorAction Stop | Out-Null
}

foreach ($prefix in $Prefixes -split ',') {
    New-NetRoute -DestinationPrefix $prefix -InterfaceIndex $InterfaceIndex -RouteMetric 1 -ErrorAction SilentlyContinue | Out-Null
}

if ($BlockIPv6) {
    if (-not (Get-NetFirewallRule -DisplayName $Rule -ErrorAction SilentlyContinue)) {
        New-NetFirewallRule -DisplayName $Rule -Direction Outbound -Action Block -Protocol Any -RemoteAddress "::/0" -ErrorAction Stop | Out-Null
    }
}

exit 0
