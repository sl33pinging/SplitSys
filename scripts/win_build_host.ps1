# Build splitsys-host natively on Windows (USB strategy b) and check that it runs.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\win_build_host.ps1 [-Ping] [-Transport serial:COM5]
#
# Output: sw\target\release\splitsys-host.exe (the path demo\start_demo.ps1 looks for).
# Needs Windows Rust (rustup, 1.99.1-x86_64-pc-windows-msvc, same version as WSL) and the MSVC linker
# (VS Build Tools, already installed on this laptop). See docs/ENV.md
# "Windows host build (USB strategy b)". Owner: toolchain-engineer.
# ASCII only: Windows PowerShell 5.1 reads BOM-less scripts as ANSI.
param(
    [switch]$Ping,
    [string]$Transport = "serial:auto",
    [string]$TargetDir = ""
)
# "Continue", not "Stop": under 5.1, cargo's progress output on stderr becomes
# NativeCommandError records when the output is captured, and "Stop" would abort.
# Every native call checks $LASTEXITCODE instead.
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$sw = Join-Path $root "sw"
if (-not $TargetDir) { $TargetDir = Join-Path $sw "target" }

function Fail([string]$msg, [int]$code) {
    Write-Host "win_build_host: $msg" -ForegroundColor Red
    exit $code
}

# 1. cargo (rustup puts it in %USERPROFILE%\.cargo\bin; a shell opened before
#    the install may not have it on PATH yet).
$cargo = $null
$cmd = Get-Command cargo -ErrorAction SilentlyContinue
if ($cmd) { $cargo = $cmd.Source }
elseif (Test-Path "$env:USERPROFILE\.cargo\bin\cargo.exe") { $cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe" }
if (-not $cargo) {
    Write-Host "win_build_host: cargo not found on Windows." -ForegroundColor Red
    Write-Host "Install Rust once (about 125 MB download, a few minutes):"
    Write-Host "  1. Download https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe"
    Write-Host "  2. Run:  rustup-init.exe -y --profile minimal --default-toolchain 1.99.1-x86_64-pc-windows-msvc"
    Write-Host "  3. Open a NEW PowerShell window and re-run this script."
    Write-Host "Details: docs\ENV.md, section 'Windows host build (USB strategy b)'."
    exit 2
}
Write-Host "cargo: $cargo"
& $cargo --version
if ($LASTEXITCODE -ne 0) { Fail "cargo --version failed (is a toolchain installed? run: rustup default 1.99.1-x86_64-pc-windows-msvc)" 2 }

# 2. MSVC linker (informational: rustc locates it through the VS installer itself).
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    $vc = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($vc) { Write-Host "MSVC build tools: $vc" }
    else { Write-Host "WARNING: no VS install with the C++ x64 build tools; linking will fail with 'link.exe not found'. See docs\ENV.md." -ForegroundColor Yellow }
} else {
    Write-Host "WARNING: vswhere.exe not found; Visual Studio Build Tools may be missing. See docs\ENV.md." -ForegroundColor Yellow
}

# 3. Build. Windows build products stay in sw\target (NTFS, Windows-only);
#    the WSL side uses its own CARGO_TARGET_DIR, so the two never collide.
$env:CARGO_TARGET_DIR = $TargetDir
Write-Host "building in $sw (CARGO_TARGET_DIR=$TargetDir)"
Push-Location $sw
try {
    & $cargo build --release --locked -p splitsys-host
    $rc = $LASTEXITCODE
} finally {
    Pop-Location
}
if ($rc -ne 0) { Fail "cargo build failed (exit $rc). If the error mentions link.exe, install the VS Build Tools C++ workload (docs\ENV.md)." $rc }

# 4. Check the binary runs.
$exe = Join-Path $TargetDir "release\splitsys-host.exe"
if (-not (Test-Path $exe)) { Fail "build reported success but $exe is missing" 1 }
& $exe --help | Select-Object -First 5
if ($LASTEXITCODE -ne 0) { Fail "$exe --help exited $LASTEXITCODE" 1 }
& $exe --version
Write-Host "OK: $exe" -ForegroundColor Green

# 5. Serial ports (informational) and optional board ping.
$ports = [System.IO.Ports.SerialPort]::GetPortNames()
if ($ports) { Write-Host "COM ports now: $($ports -join ', ')" } else { Write-Host "COM ports now: none (board not plugged in?)" }
if ($Ping) {
    & $exe --transport $Transport ping
    if ($LASTEXITCODE -ne 0) { Fail "ping over $Transport failed (exit $LASTEXITCODE); see docs\ENV.md 'Finding the COM port'" $LASTEXITCODE }
}
exit 0
