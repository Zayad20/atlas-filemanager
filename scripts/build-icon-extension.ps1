param([switch]$SkipBuild)

$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent
$output = Join-Path $repo "target\extensions\icon-manager"

if (-not $SkipBuild) {
    Push-Location $repo
    try {
        cargo build --release --locked -p atlas-icon-extension
    } finally {
        Pop-Location
    }
}

$binary = Join-Path $repo "target\release\atlas-icon-extension.exe"
if (-not (Test-Path $binary)) {
    throw "Missing $binary. Build the icon extension first."
}

Remove-Item -LiteralPath $output -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $output | Out-Null
Copy-Item $binary $output
Copy-Item (Join-Path $repo "extensions\icon-manager\extension.toml") $output
Copy-Item (Join-Path $repo "extensions\icon-manager\icon.svg") $output

Get-ChildItem $output
