# SplitSys booth cold start (DEMO.md §1): host server + kiosk browser, after the board power cycle.
#   powershell -ExecutionPolicy Bypass -File demo\start_demo.ps1 [-Transport serial:COM5] [-Port 8080] [-Replay results\hw]
# Owner: host-dashboard. Uses the native Windows build of splitsys-host (USB strategy b).
param(
    [string]$Transport = "serial:auto",
    [int]$Port = 8080,
    [string]$Replay = "",
    [switch]$Attract,
    [switch]$NoBrowser
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$t0 = Get-Date

# 1. Find the host binary (prebuilt: building here would blow the 60 s budget).
$candidates = @($env:SPLITSYS_HOST,
    (Join-Path $root "sw\target\release\splitsys-host.exe"),
    (Join-Path $root "sw\target\debug\splitsys-host.exe")) | Where-Object { $_ -and (Test-Path $_) }
if (-not $candidates) {
    Write-Host "splitsys-host.exe not found. Build it once: cd sw; cargo build --release --locked -p splitsys-host" -ForegroundColor Red
    exit 1
}
$exe = $candidates[0]
Write-Host "host: $exe"

# 2. Board check (prints build_id, or the list of serial ports on failure).
& $exe --transport $Transport ping
if ($LASTEXITCODE -ne 0) {
    Write-Host "PING failed. Check the USB cable / COM port (Device Manager), or run with -Replay results\hw for the recorded session." -ForegroundColor Yellow
}

# 3. Server (keeps running; reconnects to the board on its own).
$srvArgs = @("--transport", $Transport, "--results", (Join-Path $root "results"), "serve", "--port", $Port)
if ($Replay) { $srvArgs += @("--replay", $Replay) }
if ($Attract) { $srvArgs += "--attract" }
$srv = Start-Process -FilePath $exe -ArgumentList $srvArgs -WorkingDirectory $root -PassThru -WindowStyle Minimized
Write-Host "server pid $($srv.Id) on http://127.0.0.1:$Port/"

# 4. Wait until the dashboard answers (max 15 s).
$url = "http://127.0.0.1:$Port/"
$ok = $false
for ($i = 0; $i -lt 30; $i++) {
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 1 "$url`api/state" | Out-Null; $ok = $true; break } catch { Start-Sleep -Milliseconds 500 }
}
if (-not $ok) { Write-Host "dashboard did not come up on $url" -ForegroundColor Red; exit 1 }

# 5. Kiosk browser (Edge ships with Windows; separate profile = no restore prompts).
if (-not $NoBrowser) {
    $edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
    $kioskProfile = Join-Path $env:TEMP "splitsys_kiosk"
    Start-Process -FilePath $edge -ArgumentList @("--kiosk", $url, "--edge-kiosk-type=fullscreen", "--no-first-run", "--user-data-dir=$kioskProfile")
}
$dt = [int]((Get-Date) - $t0).TotalSeconds
Write-Host "ready in $dt s. Keys: 1/2/3 scenes, Space demo, A attract, R replay, F fullscreen. Alt+F4 closes the kiosk." -ForegroundColor Green
