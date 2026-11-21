# One-time Windows set-up of the USB network of the IMX678 camera (black YIZHAN 4K); run in an administrator PowerShell,
# after Windows' "Remote NDIS Compatible Device" driver is on the camera's RNDIS function. Renames the adapter "Microscope", gives it
# 192.168.1.2/24 (the camera is 192.168.1.100), IPv4 MTU 400 and no IPv6: the camera's vendor USB driver is not safe
# receiving frames larger than one USB packet, and IPv6 cannot go below 1280.
$ErrorActionPreference = 'Stop'
$a = Get-NetAdapter | Where-Object { $_.InterfaceDescription -match 'Remote NDIS' -or $_.Name -eq 'Microscope' } | Select-Object -First 1
if (-not $a) { throw 'no Remote NDIS adapter: install the driver on the camera''s RNDIS function first' }
if ($a.Name -ne 'Microscope') { Rename-NetAdapter -Name $a.Name -NewName 'Microscope'; $a = Get-NetAdapter -Name 'Microscope' }
Disable-NetAdapterBinding -Name 'Microscope' -ComponentID ms_tcpip6
Set-NetIPInterface -InterfaceAlias 'Microscope' -AddressFamily IPv4 -Dhcp Disabled -NlMtuBytes 400
# the MTU again in the persistent store: Set-NetIPInterface's alone falls back to 1500 when the camera reboots
netsh interface ipv4 set subinterface 'Microscope' mtu=400 store=persistent | Out-Null
Get-NetIPAddress -InterfaceAlias 'Microscope' -AddressFamily IPv4 -ErrorAction SilentlyContinue | Remove-NetIPAddress -Confirm:$false
New-NetIPAddress -InterfaceAlias 'Microscope' -IPAddress 192.168.1.2 -PrefixLength 24 | Out-Null
Get-NetIPInterface -InterfaceAlias 'Microscope' -AddressFamily IPv4 | Format-Table InterfaceAlias, NlMtu, Dhcp, ConnectionState
Get-NetIPAddress -InterfaceAlias 'Microscope' -AddressFamily IPv4 | Format-Table IPAddress, PrefixLength
