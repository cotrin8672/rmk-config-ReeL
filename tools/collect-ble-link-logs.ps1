param(
    [string]$OutputPath = (Join-Path ([IO.Path]::GetTempPath()) ('ReeL-device-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')),
    [ValidateRange(1, 86400)][int]$Seconds = 1800,
    [switch]$SelfTest
)
$ErrorActionPreference = 'Stop'

function Complete-Lines([ref]$Pending, [string]$Chunk) {
    $Pending.Value += $Chunk
    while (($index = $Pending.Value.IndexOf("`n")) -ge 0) {
        $line = $Pending.Value.Substring(0, $index).TrimEnd("`r")
        $Pending.Value = $Pending.Value.Substring($index + 1)
        if ($line.Length) { $line }
    }
    if ($Pending.Value.Length -gt 65536) { throw 'Serial record exceeded 64 KiB without a newline.' }
}

if ($SelfTest) {
    $pending = ''
    $lines = @(Complete-Lines ([ref]$pending) "BLD1,100,INFO,HEARTBEAT`r")
    if ($lines.Count -ne 0) { throw 'Partial record emitted too early.' }
    $lines = @(Complete-Lines ([ref]$pending) "`nBLD1,200,INFO,[gatt] disconnected: ConnectionTimeout`nBLD1,0")
    if ($lines.Count -ne 2 -or $lines[0] -ne 'BLD1,100,INFO,HEARTBEAT' -or $lines[1] -notmatch 'ConnectionTimeout$') { throw 'Chunked records were lost or combined.' }
    if ($pending -ne 'BLD1,0') { throw 'Partial next record was lost.' }
    $lines = @(Complete-Lines ([ref]$pending) ",INFO,BOOT resetreas=0x00000002`r`n")
    if ($lines.Count -ne 1 -or $lines[0] -ne 'BLD1,0,INFO,BOOT resetreas=0x00000002' -or $pending) { throw 'Boot record after reconnect was damaged.' }
    'PASS: split serial records, disconnect reason and reset flags preserved.'
    return
}

Add-Type -AssemblyName System.IO.Ports
$writer = [IO.StreamWriter]::new($OutputPath, $false, [Text.UTF8Encoding]::new($false))
$writer.AutoFlush = $true
$deadline = (Get-Date).AddSeconds($Seconds)
$lastNotice = ''
$port = $null
function Write-Record([string]$Line) {
    $writer.WriteLine([DateTimeOffset]::Now.ToString('o') + ' ' + $Line)
}

try {
    Write-Record 'HOST collector started; no device or Windows settings are changed.'
    Write-Output "Log: $OutputPath"
    while ((Get-Date) -lt $deadline) {
        $devices = @(Get-PnpDevice -Class Ports -PresentOnly -ErrorAction SilentlyContinue |
            Where-Object { $_.InstanceId -match 'USB\\VID_C0DE&PID_CAFE' })
        if ($devices.Count -gt 1) { throw 'More than one Embassy USB logger is connected; cannot identify ReeL safely.' }
        if ($devices.Count -eq 0 -or $devices[0].FriendlyName -notmatch '\((COM\d+)\)') {
            if ($lastNotice -ne 'waiting') { Write-Record 'HOST waiting for diagnostic USB logger'; $lastNotice = 'waiting' }
            Start-Sleep -Seconds 2
            continue
        }
        $portName = $Matches[1]
        $pending = ''
        try {
            $port = [IO.Ports.SerialPort]::new($portName, 115200)
            $port.DtrEnable = $true
            $port.ReadTimeout = 500
            $port.Open()
            Write-Record "HOST serial opened: $portName"
            $lastNotice = 'connected'
            while ((Get-Date) -lt $deadline -and $port.IsOpen) {
                foreach ($line in @(Complete-Lines ([ref]$pending) ($port.ReadExisting()))) { Write-Record $line }
                Start-Sleep -Milliseconds 50
            }
        } catch [IO.IOException] {
            Write-Record ('HOST serial disconnected: ' + $_.Exception.Message)
        } catch [UnauthorizedAccessException] {
            Write-Record ('HOST serial unavailable: ' + $_.Exception.Message)
        } finally {
            if ($pending) { Write-Record ('HOST incomplete record: ' + $pending) }
            if ($port) { $port.Dispose(); $port = $null }
        }
        Start-Sleep -Seconds 1
    }
} finally {
    if ($port) { $port.Dispose() }
    Write-Record 'HOST collector stopped'
    $writer.Dispose()
}
