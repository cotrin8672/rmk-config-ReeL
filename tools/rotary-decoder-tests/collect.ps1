param(
    [string]$Port,
    [ValidateSet('Ambiguous','Next')][string]$Mode = 'Ambiguous',
    [switch]$NewCapture,
    [string]$OutputDirectory = (Join-Path ([Environment]::GetFolderPath('UserProfile')) 'Downloads/ReeL-rotary-logs'),
    [int]$TimeoutSeconds = 180,
    [string]$ValidateFile
)
$ErrorActionPreference = 'Stop'

function Test-RotaryLog([string[]]$Lines) {
    if ($Lines.Count -lt 4 -or $Lines[0] -notmatch '^BEGIN,1,(\d+),(\d+),(\d+),32768$') { throw 'Invalid or incomplete BEGIN record.' }
    $count = [int]$Matches[2]
    $dropped = [uint64]$Matches[3]
    if ($count -lt 1 -or $count -gt 512 -or $Lines.Count -ne $count + 3) { throw 'Sample count mismatch; keep the partial log.' }
    if ($Lines[-1] -notmatch '^END,(\d+),([0-9a-fA-F]{8})$' -or [int]$Matches[1] -ne $count) { throw 'Missing or invalid END record.' }
    $expected = $Matches[2].ToLowerInvariant()
    [uint32]$hash = 2166136261
    for ($i=1; $i -lt $Lines.Count-1; $i++) {
        foreach ($b in [Text.Encoding]::UTF8.GetBytes($Lines[$i] + "`n")) {
            $hash = [uint32](([uint64]($hash -bxor [uint32]$b) * 16777619) -band 4294967295)
        }
    }
    if ($hash.ToString('x8') -ne $expected) { throw 'Checksum mismatch; keep the partial log and use DUMP again.' }
    $rows = @($Lines[1..($Lines.Count-2)] | ConvertFrom-Csv)
    [uint64]$previousTicks = 0
    for ($i=0; $i -lt $rows.Count; $i++) {
        if ([uint64]$rows[$i].n -ne $dropped + $i + 1 -or [uint64]$rows[$i].ticks -lt $previousTicks) { throw 'Sample sequence or timestamp mismatch.' }
        $previousTicks = [uint64]$rows[$i].ticks
    }
    [PSCustomObject]@{ Samples=$count; Dropped=$dropped; Source=$rows[-1].source; Output=$rows[-1].output; Checksum=$expected }
}

if ($ValidateFile) {
    Test-RotaryLog ([IO.File]::ReadAllLines((Resolve-Path -LiteralPath $ValidateFile)))
    return
}
if (-not $Port) {
    $candidates = @(Get-CimInstance Win32_PnPEntity | Where-Object {
        $_.PNPDeviceID -match 'VID_4C4B&PID_524D' -and $_.Name -match '\(COM\d+\)'
    } | ForEach-Object { if ($_.Name -match '\((COM\d+)\)') { $Matches[1] } } | Sort-Object -Unique)
    if ($candidates.Count -ne 1) { throw "左側をUSB接続してください。候補: $($candidates -join ', ')。自動選択できない場合は -Port COM番号 を指定してください。" }
    $Port = $candidates[0]
}
$serial = [IO.Ports.SerialPort]::new($Port,115200,[IO.Ports.Parity]::None,8,[IO.Ports.StopBits]::One)
$serial.NewLine = "`n"
$serial.ReadTimeout = 3000
$serial.WriteTimeout = 3000
$serial.DtrEnable = $true
$lines = [Collections.Generic.List[string]]::new()
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$path = Join-Path $OutputDirectory ('rotary-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '.log')
try {
    $serial.Open()
    $serial.DiscardInBuffer()
    $serial.WriteLine('INFO')
    $identity = $serial.ReadLine().TrimEnd("`r")
    if ($identity -notmatch '^REEL_ROTARY_V1,(READY|FROZEN)$') { throw "診断ファームを確認できません: $identity" }
    if ($Matches[1] -eq 'FROZEN' -and -not $NewCapture) {
        Write-Host '保存済みの記録を回収します。エンコーダ操作は不要です。'
        $serial.WriteLine('DUMP')
    } else {
        $serial.WriteLine($(if ($Mode -eq 'Next') { 'NEXT' } else { 'ARM' }))
        if ($serial.ReadLine().TrimEnd("`r") -ne 'ARMED') { throw '収集開始を確認できません。' }
        Write-Host '収集中です。問題が出る位置で普段どおり上下に動かしてください。LCDがFROZENになったら記録確定です。'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $begun = $false
    $complete = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        try { $line = $serial.ReadLine().TrimEnd("`r") } catch [TimeoutException] { continue }
        if ($line.StartsWith('BEGIN,')) { $begun = $true; $lines.Clear() }
        if ($begun) { $lines.Add($line) }
        if ($line.StartsWith('END,')) { $complete = $true; break }
        if ($line.StartsWith('ERR') -or $line -eq 'BUSY') { throw $line }
    }
    if (-not $complete) { throw '収集が完了していません。再起動せず、同じコマンドでもう一度回収できます。' }
    $result = Test-RotaryLog $lines.ToArray()
    [IO.File]::WriteAllText($path,($lines -join "`n")+"`n",[Text.UTF8Encoding]::new($false))
    Write-Host "保存完了: $path"
    Write-Host "採用元=$($result.Source)、出力=$($result.Output)（1=CW、-1=CCW、0=None）、サンプル=$($result.Samples)、先頭側の上書き=$($result.Dropped)"
    Write-Host 'この.logファイルを送ってください。確定直前に回した方向と、PCで起きた動きを一言添えてください。'
    Write-Host '次の記録を取り直す場合だけ -NewCapture を付けてください。'
} catch {
    if ($lines.Count -gt 0) {
        [IO.File]::WriteAllText($path+'.partial',($lines -join "`n")+"`n",[Text.UTF8Encoding]::new($false))
        Write-Host "未完了データを保持: $path.partial"
    }
    throw
} finally {
    if ($serial.IsOpen) { $serial.Close() }
    $serial.Dispose()
}
