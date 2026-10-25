# SplitSys booth cold start (DEMO.md §1): host server + kiosk browser, after the board power cycle.
#   powershell -ExecutionPolicy Bypass -File demo\start_demo.ps1 [-Transport serial:COM5] [-Baud auto|1562500|115200] [-Port 8080] [-Replay results\hw]
# -Baud auto (default): PING at 1562500 (KG3+, spec BAUD) and, if no answer, at 115200 (KG2); the rate that
# answered is printed and pinned for the server. An explicit rate is used as is (no probe).
# Owner: host-dashboard. Uses the native Windows build of splitsys-host (USB strategy b).
param(
    [string]$Transport = "serial:auto",
    [string]$Baud = "auto",
    [int]$Port = 8080,
    [string]$Replay = "",
    [switch]$Attract,
    [switch]$NoBrowser,
    # Top-left of the monitor the kiosk should open on (Windows virtual-desktop coordinates).
    # "auto" (default) = the first non-primary display, so a replugged monitor that moved is still found;
    # "" = let Edge choose.
    [string]$WindowPosition = "auto"
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$t0 = Get-Date

# 1. Find the host binary (prebuilt: building here would blow the 60 s budget).
# Outer @() matters: with one match, Where-Object returns a bare string and [0] would be its first char.
$candidates = @(@($env:SPLITSYS_HOST,
    (Join-Path $root "sw\target\release\splitsys-host.exe"),
    (Join-Path $root "sw\target\debug\splitsys-host.exe")) | Where-Object { $_ -and (Test-Path $_) })
if (-not $candidates) {
    Write-Host "splitsys-host.exe not found. Build it once: cd sw; cargo build --release --locked -p splitsys-host" -ForegroundColor Red
    exit 1
}
$exe = $candidates[0]
Write-Host "host: $exe"

# 2. Board check (prints build_id and the baud that answered, or the list of serial ports on failure).
$srvBaud = $Baud
$pingOut = @(& $exe --transport $Transport --baud $Baud ping)
$pingOk = ($LASTEXITCODE -eq 0)
$pingOut | ForEach-Object { Write-Host $_ }
if ($pingOk) {
    $m = [regex]::Match(($pingOut -join "`n"), 'baud (\d+)')
    if ($m.Success) {
        $srvBaud = $m.Groups[1].Value
        $gen = if ($srvBaud -eq "115200") { "KG2 rate" } elseif ($srvBaud -eq "1562500") { "KG3 / spec rate" } else { "explicit rate" }
        Write-Host "board answered at $srvBaud baud ($gen); server pinned to --baud $srvBaud" -ForegroundColor Green
    }
} else {
    Write-Host "PING failed (tried -Baud $Baud). Check the USB cable / COM port (Device Manager); KG2 runs at 115200, KG3 at 1562500 (-Baud 115200 / -Baud 1562500); or run with -Replay results\hw for the recorded session. The server keeps probing (--baud $srvBaud)." -ForegroundColor Yellow
}

# 3. Server (keeps running; reconnects to the board on its own).
$srvArgs = @("--transport", $Transport, "--baud", $srvBaud, "--results", (Join-Path $root "results"), "serve", "--port", $Port)
if ($Replay) { $srvArgs += @("--replay", $Replay) }
if ($Attract) { $srvArgs += "--attract" }
$srv = Start-Process -FilePath $exe -ArgumentList $srvArgs -WorkingDirectory $root -PassThru -WindowStyle Minimized
Write-Host "server pid $($srv.Id) on http://127.0.0.1:$Port/"

# 4. Wait until the dashboard answers (max ~40 s). Probe the page, not /api/state: the state snapshot
#    is ~330 KB with the full hw session and the server's first answer can take well over 1 s (H-7, Sat).
$url = "http://127.0.0.1:$Port/"
$ok = $false
for ($i = 0; $i -lt 40; $i++) {
    try { Invoke-WebRequest -UseBasicParsing -TimeoutSec 3 $url | Out-Null; $ok = $true; break } catch { Start-Sleep -Milliseconds 500 }
}
if (-not $ok) { Write-Host "dashboard did not come up on $url" -ForegroundColor Red; exit 1 }

# 5. Kiosk browser (Edge ships with Windows; separate profile = no restore prompts).
if (-not $NoBrowser) {
    $edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
    $kioskProfile = Join-Path $env:TEMP "splitsys_kiosk"
    $edgeArgs = @("--kiosk", $url, "--edge-kiosk-type=fullscreen", "--no-first-run", "--user-data-dir=$kioskProfile")
    if ($WindowPosition -eq "auto") {
        Add-Type -AssemblyName System.Windows.Forms
        $ext = [System.Windows.Forms.Screen]::AllScreens | Where-Object { -not $_.Primary } | Select-Object -First 1
        if ($ext) { $WindowPosition = "$($ext.Bounds.X),$($ext.Bounds.Y)"; Write-Host "kiosk on $($ext.DeviceName) at $WindowPosition" }
        else { $WindowPosition = ""; Write-Host "no external monitor found: kiosk on the laptop screen" -ForegroundColor Yellow }
    }
    if ($WindowPosition) { $edgeArgs += "--window-position=$WindowPosition" }
    Start-Process -FilePath $edge -ArgumentList $edgeArgs
}
$dt = [int]((Get-Date) - $t0).TotalSeconds
Write-Host "ready in $dt s. Keys: 1/2/3 scenes, Space demo, A attract, R replay, F fullscreen. Alt+F4 closes the kiosk." -ForegroundColor Green
