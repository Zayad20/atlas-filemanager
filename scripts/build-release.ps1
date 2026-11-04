[CmdletBinding()]
param(
    # Stable is the supported production compiler. Nightly is an opt-in LLVM
    # experiment and uses a separate target directory.
    [switch]$Nightly,
    [switch]$Lean,
    [switch]$Run,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$ErrorActionPreference = "Stop"
if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

if ($Nightly) {
    $env:CARGO_TARGET_DIR = Join-Path $root "target\toolchains\nightly-llvm-release"
    $env:RUSTFLAGS = "-Zthreads=8"
    $cargoPrefix = @("+nightly")
    $compiler = "nightly LLVM"
    $binaryPath = Join-Path $env:CARGO_TARGET_DIR "release\atlas.exe"
}
else {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    $cargoPrefix = @("+1.97.1")
    $compiler = "stable LLVM"
    $binaryPath = Join-Path $root "target\release\atlas.exe"
}

$command = @("build", "--locked", "--release")
if (-not $Lean) {
    # Keep the provider protocol in sync with the host from this same checkout.
    $command += @("-p", "atlas", "-p", "atlas-cloud-extension")
}
if ($Lean) {
    $command += "--no-default-features"
}
if ($CargoArgs) {
    $command += $CargoArgs
}

Write-Host "Compiler: $compiler"
Write-Host "Linker:   rust-lld.exe (from .cargo/config.toml)"
Write-Host "Target:   $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' })"
Write-Host "Command:  cargo $($cargoPrefix + $command -join ' ')"

# Stop any orphan processes or running atlas before release build
foreach ($proc in Get-Process -Name atlas, atlas-search-service -ErrorAction SilentlyContinue) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
}

& cargo @cargoPrefix @command
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

if ($Run) {
    if (-not (Test-Path -LiteralPath $binaryPath)) {
        throw "Release binary was not found: $binaryPath"
    }
    Write-Host "Running:  $binaryPath"
    & $binaryPath
    exit $LASTEXITCODE
}

Write-Host "Built:    $binaryPath"
exit 0
