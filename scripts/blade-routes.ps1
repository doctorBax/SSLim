# Blade route setup for the Wintun adapter. Run elevated.
# Apply: -InterfaceIndex <n> -ServerIP <ip> -Prefixes "a.b.c.d/1,...,x.y.z.w/32" [-BlockIPv6]
# Revert: -Revert (interface routes die with the adapter)

param(
    [int]$InterfaceIndex,
    [string]$ServerIP,
    [ipaddress]$ClientIP = '10.253.0.2',
    [string]$Prefixes,
    [switch]$BlockIPv6,
    [switch]$Revert
)

$ErrorActionPreference = "Stop"
$LegacyRule = "Blade IPv6 outbound block"
if ($Revert) {
    Get-NetFirewallRule -DisplayName $LegacyRule -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue
    exit 0
}

Get-NetFirewallRule -DisplayName $LegacyRule -ErrorAction SilentlyContinue | Remove-NetFirewallRule -ErrorAction SilentlyContinue

if (-not $InterfaceIndex -or -not $ServerIP -or -not $Prefixes) {
    Write-Error "usage: blade-routes.ps1 -InterfaceIndex <n> -ServerIP <ip> -Prefixes <csv> [-BlockIPv6] | -Revert"
    exit 1
}

$existing = Get-NetIPAddress -InterfaceIndex $InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue
Set-NetIPInterface -InterfaceIndex $InterfaceIndex -AddressFamily IPv4 -NlMtuBytes 1280 -PolicyStore ActiveStore
Set-NetIPInterface -InterfaceIndex $InterfaceIndex -AddressFamily IPv4 -AutomaticMetric Disabled -InterfaceMetric 5 -PolicyStore ActiveStore
if (-not $existing) {
    New-NetIPAddress -InterfaceIndex $InterfaceIndex -IPAddress $ClientIP.IPAddressToString -PrefixLength 32 -PolicyStore ActiveStore -ErrorAction Stop | Out-Null
}

foreach ($prefix in $Prefixes -split ',') {
    New-NetRoute -DestinationPrefix $prefix -InterfaceIndex $InterfaceIndex -NextHop "0.0.0.0" -RouteMetric 1 -PolicyStore ActiveStore -ErrorAction Stop | Out-Null
}
Set-DnsClientServerAddress -InterfaceIndex $InterfaceIndex -ServerAddresses @("1.1.1.1", "9.9.9.9")

if ($BlockIPv6) {
    $ipv6 = Get-NetIPInterface -InterfaceIndex $InterfaceIndex -AddressFamily IPv6 -ErrorAction SilentlyContinue
    if ($ipv6) {
        Set-NetIPInterface -InterfaceIndex $InterfaceIndex -AddressFamily IPv6 -AutomaticMetric Disabled -InterfaceMetric 5 -PolicyStore ActiveStore
        if (-not (Get-NetIPAddress -InterfaceIndex $InterfaceIndex -IPAddress "fd54:253::2" -ErrorAction SilentlyContinue)) {
            New-NetIPAddress -InterfaceIndex $InterfaceIndex -IPAddress "fd54:253::2" -PrefixLength 128 -PolicyStore ActiveStore -ErrorAction Stop | Out-Null
        }
        if (-not (Get-NetRoute -InterfaceIndex $InterfaceIndex -DestinationPrefix "2000::/3" -ErrorAction SilentlyContinue)) {
            New-NetRoute -DestinationPrefix "2000::/3" -InterfaceIndex $InterfaceIndex -NextHop "::" -RouteMetric 1 -PolicyStore ActiveStore -ErrorAction Stop | Out-Null
        }
    }
}

exit 0
