[CmdletBinding()]
param(
    [ValidateSet("stable", "nightly-llvm", "nightly-cranelift")]
    [string]$Configuration = "stable",
    [ValidateRange(1, 16)]
    [int]$Runs = 3,
    [string]$CodegenUnits = "64",
    [switch]$Full
)

$ErrorActionPreference = "Stop"
if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

$targetName = switch ($Configuration) {
    "stable" { "stable-1.97.1" }
    "nightly-llvm" { "nightly-llvm" }
    "nightly-cranelift" { "nightly-cranelift" }
}
$env:CARGO_TARGET_DIR = Join-Path $root "target\toolchains\$targetName"
$env:CARGO_INCREMENTAL = "1"

[string[]]$cargoPrefix = if ($Configuration -eq "stable") { "+1.97.1" } else { "+nightly" }
if ($Configuration -eq "stable") {
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
} else {
    $env:RUSTFLAGS = "-Zthreads=8"
}

if ($Configuration -eq "nightly-cranelift") {
    $installed = (& rustup component list --toolchain nightly --installed 2>$null) -join "`n"
    if ($installed -notmatch "rustc-codegen-cranelift") {
        throw "Cranelift is missing. Run: rustup component add rustc-codegen-cranelift --toolchain nightly"
    }
    $cargoPrefix += @(
        "-Z", "codegen-backend",
        "--config", 'profile.dev.codegen-backend=\"cranelift\"'
    )
}

$featureArgs = if ($Full) { @() } else { @("--no-default-features") }
$unitValues = @($CodegenUnits.Split(',') | ForEach-Object {
    $parsed = 0
    if (-not [int]::TryParse($_.Trim(), [ref]$parsed) -or $parsed -lt 1) {
        throw "Invalid codegen-units value: $_"
    }
    $parsed
})
$results = [System.Collections.Generic.List[object]]::new()

function Invoke-Cargo {
    param([string[]]$Arguments)
    Write-Host "cargo $((@($cargoPrefix) + @($Arguments)) -join ' ')"
    & cargo @cargoPrefix @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo failed with exit code $LASTEXITCODE"
    }
}

function Read-UiSeconds {
    $report = Join-Path $env:CARGO_TARGET_DIR "cargo-timings\cargo-timing.html"
    if (-not (Test-Path $report)) { return $null }
    $html = Get-Content -Raw $report
    $match = [regex]::Match(
        $html,
        '<td>atlas-ui v[^<]+</td>\s*<td>([0-9.]+)s</td>',
        [System.Text.RegularExpressions.RegexOptions]::Singleline
    )
    if (-not $match.Success) { return $null }
    return [double]$match.Groups[1].Value
}

foreach ($units in $unitValues) {
    $env:CARGO_PROFILE_DEV_CODEGEN_UNITS = [string]$units

    Write-Host ""
    Write-Host "=== $Configuration / codegen-units=$units ==="
    Write-Host "Warming dependencies in $env:CARGO_TARGET_DIR ..."
    $warmArgs = @("build", "--locked", "-p", "atlas") + $featureArgs
    Invoke-Cargo $warmArgs

    for ($run = 1; $run -le $Runs; $run++) {
        Write-Host ""
        Write-Host "Run $run/${Runs}: rebuilding atlas-ui and launcher"
        Invoke-Cargo @("clean", "-p", "atlas-ui", "--profile", "dev")
        Invoke-Cargo @("clean", "-p", "atlas", "--profile", "dev")

        $buildArgs = @(
            "build", "--locked", "-p", "atlas", "--timings"
        ) + $featureArgs
        $elapsed = Measure-Command { Invoke-Cargo $buildArgs }
        $uiSeconds = Read-UiSeconds
        $results.Add([pscustomobject]@{
            configuration = $Configuration
            codegen_units = $units
            run = $run
            total_seconds = [math]::Round($elapsed.TotalSeconds, 2)
            atlas_ui_seconds = $uiSeconds
            target_dir = $env:CARGO_TARGET_DIR
            rustc = (& rustc $cargoPrefix[0] --version)
        })
    }
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$outputDir = Join-Path $root "target\build-benchmarks"
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
$base = Join-Path $outputDir "$Configuration-$stamp"
$results | Export-Csv -NoTypeInformation "$base.csv"
$results | ConvertTo-Json -Depth 3 | Set-Content -Encoding utf8 "$base.json"

Write-Host ""
Write-Host "=== Results ==="
$results | Format-Table configuration, codegen_units, run, total_seconds, atlas_ui_seconds -AutoSize
Write-Host "CSV:  $base.csv"
Write-Host "JSON: $base.json"
Write-Host "Cleanup cache: Remove-Item -Recurse -Force '$env:CARGO_TARGET_DIR'"
