# Seam B smoke test for Windows: given an Enforcement, blocked domains and their subdomains don't
# resolve, DNS can't bypass the filter (Windows Firewall), the hosts fallback is written, a
# blocked app is killed, the service comes back after it is killed, and all of it still holds
# after the service restarts.
#
# Usage, in an elevated PowerShell (Windows PowerShell 5.1 or PowerShell 7):
#   cargo build --release -p cloister-enforcer -p cloister-cli -p cloister-app
#   target\release\cloisterd.exe install --app "$PWD\target\release\cloister-app.exe"
#   powershell -ExecutionPolicy Bypass -File scripts\windows\smoke-test.ps1 [-Minutes 3] [-Cli target\release\cloister.exe]
#
# It starts a real Block of -Minutes with the Distraction List. That Block CAN'T be cancelled:
# YouTube, Reddit, etc. stay blocked until it ends.
param(
    [int]$Minutes = 3,
    [string]$Cli = "$PSScriptRoot\..\..\target\release\cloister.exe"
)
$ErrorActionPreference = 'Continue'
$Blocked = 'youtube.com', 'www.youtube.com', 'm.youtube.com', 'youtu.be', 'rr1---sn-abc.googlevideo.com', 'old.reddit.com', 'x.com'
$Allowed = 'example.com', 'wikipedia.org'
$Socket = 'C:\ProgramData\Cloister\cloister.sock'
$script:failures = 0

function Pass($m) { Write-Host "  ok    $m" }
function Fail($m) { Write-Host "  FAIL  $m" -ForegroundColor Red; $script:failures++ }

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { Write-Error 'Run in an elevated PowerShell.'; exit 1 }
if (-not (Test-Path $Cli)) { Write-Error "No CLI at $Cli (build it, or pass -Cli)."; exit 1 }
if ((Get-Service Cloister -ErrorAction SilentlyContinue).Status -ne 'Running') {
    Write-Error 'The Cloister service is not running. Install it first (see the header).'; exit 1
}

function Resolves($name, $server) {
    $args = @{ Name = $name; Type = 'A'; DnsOnly = $true; QuickTimeout = $true; ErrorAction = 'Stop' }
    if ($server) { $args.Server = $server }
    try { $r = Resolve-DnsName @args; return [bool]($r | Where-Object { $_.IPAddress }) } catch { return $false }
}

function Check($title) {
    Write-Host "== $title"
    foreach ($name in $Blocked) {
        if (Resolves $name '127.0.0.1') { Fail "filter: $name resolves" } else { Pass "filter: $name is NXDOMAIN" }
        if (Resolves $name $null) { Fail "system resolver: $name resolves" } else { Pass "system resolver: $name doesn't resolve" }
    }
    foreach ($name in $Allowed) {
        if (Resolves $name $null) { Pass "system resolver: $name resolves" } else { Fail "system resolver: $name doesn't resolve" }
    }
    # 1.1.1.1 and 9.9.9.9 are the service's upstreams, so they are reachable; any other resolver isn't.
    if (Resolves 'example.com' '8.8.8.8') { Fail 'firewall: DNS to 8.8.8.8 got through' } else { Pass 'firewall: DNS to 8.8.8.8 is dropped' }
    try { $tcp = Resolve-DnsName example.com -Server 8.8.8.8 -TcpOnly -DnsOnly -QuickTimeout -ErrorAction Stop } catch { $tcp = $null }
    if ($tcp) { Fail 'firewall: DNS over TCP to 8.8.8.8 got through' } else { Pass 'firewall: DNS over TCP to 8.8.8.8 is dropped' }
    $dot = New-Object Net.Sockets.TcpClient
    $ok = $dot.ConnectAsync('8.8.8.8', 853).Wait(2000) -and $dot.Connected
    $dot.Close()
    if ($ok) { Fail 'firewall: DNS-over-TLS to 8.8.8.8 got through' } else { Pass 'firewall: DNS-over-TLS to 8.8.8.8 is dropped' }
    foreach ($rule in 'Cloister-DNS', 'Cloister-blocked-sites') {
        if (Get-NetFirewallRule -DisplayName $rule -ErrorAction SilentlyContinue) { Pass "firewall: rule $rule present" } else { Fail "firewall: no rule $rule" }
    }
    if (Select-String -Path "$env:windir\System32\drivers\etc\hosts" -Pattern "0.0.0.0`tyoutube.com" -Quiet) { Pass 'hosts: fallback entries present' } else { Fail 'hosts: no fallback entries' }
    Get-NetAdapter | Where-Object Status -eq 'Up' | ForEach-Object {
        $dns = (Get-DnsClientServerAddress -InterfaceIndex $_.ifIndex -AddressFamily IPv4).ServerAddresses
        if ($dns -contains '127.0.0.1' -and $dns.Count -eq 1) { Pass "dns settings: $($_.Name) -> 127.0.0.1" } else { Fail "dns settings: $($_.Name) -> $($dns -join ',')" }
    }
}

function Ipc($json) {
    # AF_UNIX from .NET needs PowerShell 7 (.NET Core).
    if ($PSVersionTable.PSEdition -ne 'Core') { return $null }
    $s = New-Object Net.Sockets.Socket([Net.Sockets.AddressFamily]::Unix, [Net.Sockets.SocketType]::Stream, [Net.Sockets.ProtocolType]::Unspecified)
    $s.Connect((New-Object Net.Sockets.UnixDomainSocketEndPoint($Socket)))
    $stream = New-Object Net.Sockets.NetworkStream($s, $true)
    $w = New-Object IO.StreamWriter($stream); $w.Write("$json`n"); $w.Flush()
    $reply = (New-Object IO.StreamReader($stream)).ReadLine()
    $s.Close(); return $reply
}

$answer = Read-Host "This starts a $Minutes-minute Block that can't be cancelled. Continue? [y/N]"
if ($answer -ne 'y') { exit 1 }

& $Cli block start --minutes $Minutes --plain
if ($LASTEXITCODE -ne 0) { Write-Error "Couldn't start a Block."; exit 1 }
Start-Sleep 8   # firewall and DNS settings are applied on the next ticks
Check 'while the Block is in force'

Write-Host '== restarting the service'
Restart-Service Cloister
Start-Sleep 8
Check 'after restarting the service'

Write-Host '== killing the service (the Service Control Manager must restart it)'
Stop-Process -Name cloisterd -Force
Start-Sleep 5
if ((Get-Service Cloister).Status -eq 'Running') { Pass 'the service was restarted' } else { Fail 'the service stayed stopped' }

Write-Host '== trying to cancel over IPC (must be refused)'
$status = & $Cli status
$id = ($status | Select-String -Pattern '^Block (blk-\S+) ends').Matches | Select-Object -First 1 | ForEach-Object { $_.Groups[1].Value }
$reply = Ipc "{`"request`":`"command`",`"command`":{`"type`":`"cancel_block`",`"block_id`":`"$id`"}}"
if ($null -eq $reply) { Write-Host '  skip  needs PowerShell 7 for AF_UNIX' }
elseif ($reply -like '*loosening_not_accepted*') { Pass 'IPC refused cancel_block' } else { Fail "IPC replied: $reply" }

Write-Host '== app blocker: a copy of PING.EXE named SmokeApp stands in for a blocked app'
$app = "$env:TEMP\SmokeApp.exe"
Copy-Item "$env:windir\System32\PING.EXE" $app -Force
$p = Start-Process $app -ArgumentList '-t', '127.0.0.1' -WindowStyle Hidden -PassThru
Start-Sleep 1
if ($p.HasExited) { Fail "the stand-in app didn't start" } else {
    & $Cli block start --minutes 2 --plain --extra-app SmokeApp | Out-Null
    Start-Sleep 8
    if (Get-Process -Id $p.Id -ErrorAction SilentlyContinue) { Fail 'a blocked app kept running'; Stop-Process -Id $p.Id -Force } else { Pass 'a blocked app was killed' }
}
Remove-Item $app -ErrorAction SilentlyContinue

Write-Host '== tampering: deleting the firewall rules (they must come back within ~35 s)'
netsh advfirewall firewall delete rule name=Cloister-DNS | Out-Null
$back = $false
foreach ($i in 1..45) {
    if (Get-NetFirewallRule -DisplayName 'Cloister-DNS' -ErrorAction SilentlyContinue) { $back = $true; break }
    Start-Sleep 1
}
if ($back) { Pass 'the rules were added again' } else { Fail 'the rules stayed deleted' }

& $Cli status
Write-Host ''
if ($script:failures -eq 0) {
    Write-Host 'PASS. Manual step: reboot, then run "cloister status" and "Resolve-DnsName youtube.com -Server 127.0.0.1"'
    Write-Host 'before the Block ends; it must still fail. Then work through docs/checklists/shutdown-ritual-kiosk-windows.md.'
} else {
    Write-Host "FAILED: $($script:failures) check(s)."
    exit 1
}
