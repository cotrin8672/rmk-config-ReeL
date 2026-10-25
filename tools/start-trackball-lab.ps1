$ErrorActionPreference = 'Stop'
$labUrl = 'http://127.0.0.1:4321/trackball-diagnostics.html'

function Test-TrackballLab {
    try {
        $page = Invoke-WebRequest -Uri $labUrl -UseBasicParsing -TimeoutSec 2
        return $page.StatusCode -eq 200 -and $page.Content.Contains('id="connectBle"') -and $page.Content.Contains('id="sensorImage"')
    } catch {
        return $false
    }
}

if (-not (Test-TrackballLab)) {
    $python = (Get-Command python -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
    $errorLog = Join-Path $env:TEMP 'reel-trackball-lab.stderr.log'
    $outputLog = Join-Path $env:TEMP 'reel-trackball-lab.stdout.log'
    $server = Start-Process -FilePath $python -ArgumentList @(
        '-u', '-m', 'http.server', '4321', '--bind', '127.0.0.1', '--directory', "`"$PSScriptRoot`""
    ) -WindowStyle Hidden -RedirectStandardOutput $outputLog -RedirectStandardError $errorLog -PassThru

    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        if (Test-TrackballLab) { break }
        if ($server.HasExited) { throw "Local server failed to start. See $errorLog" }
        Start-Sleep -Milliseconds 200
    }
    if (-not (Test-TrackballLab)) { throw "Local server did not become ready. See $errorLog" }
}

Write-Output $labUrl
