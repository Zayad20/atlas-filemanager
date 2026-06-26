param(
    [string]$Version = "0.1.0"
)

$ErrorActionPreference = "Stop"

cargo build --release --features perf-log

$iscc = Get-Command ISCC.exe -ErrorAction SilentlyContinue
if (-not $iscc) {
    throw "Inno Setup compiler not found. Install Inno Setup, then run this again."
}

& $iscc.Source "/DAppVersion=$Version" "packaging\windows\atlas.iss"

$installer = "target\installer\AtlasSetup-$Version.exe"
if (Test-Path $installer) {
    Get-FileHash $installer -Algorithm SHA256
}
