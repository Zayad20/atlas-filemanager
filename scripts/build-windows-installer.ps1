param(
    [string]$Version = "0.1.0",
    [switch]$PerfLog
)

$ErrorActionPreference = "Stop"

if ($PerfLog) {
    cargo build --release --locked --features perf-log
} else {
    cargo build --release --locked
}

$iscc = Get-Command ISCC.exe -ErrorAction SilentlyContinue
$isccPath = if ($iscc) { $iscc.Source } else { $null }
if (-not $isccPath) {
    $commonPath = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    if (Test-Path $commonPath) {
        $isccPath = $commonPath
    } else {
        throw "Inno Setup compiler not found. Install Inno Setup, then run this again."
    }
}

& $isccPath "/DAppVersion=$Version" "packaging\windows\atlas.iss"

$installer = "target\installer\AtlasSetup-$Version.exe"
if (Test-Path $installer) {
    Get-FileHash $installer -Algorithm SHA256
}
