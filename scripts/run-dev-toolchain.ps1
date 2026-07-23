[CmdletBinding()]
param(
    [ValidateSet("llvm", "cranelift")]
    [string]$Backend = "llvm",
    [ValidateSet("devfast", "devfull")]
    [string]$Mode = "devfast",
    # Backwards-compatible shorthand for -Mode devfull.
    [switch]$Full,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$ErrorActionPreference = "Stop"
if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

$targetName = if ($Backend -eq "cranelift") { "nightly-cranelift" } else { "nightly-llvm" }
$env:CARGO_TARGET_DIR = Join-Path $root "target\toolchains\$targetName"
$env:RUSTFLAGS = "-Zthreads=8"
if ($Full) {
    $Mode = "devfull"
}

$command = @("+nightly")
if ($Backend -eq "cranelift") {
    $installed = (& rustup component list --toolchain nightly --installed 2>$null) -join "`n"
    if ($installed -notmatch "rustc-codegen-cranelift") {
        throw "Cranelift is missing. Run: rustup component add rustc-codegen-cranelift --toolchain nightly"
    }
    $command += @(
        "-Z", "codegen-backend",
        "--config", 'profile.dev.codegen-backend=\"cranelift\"'
    )
}

$command += "run"
if ($Mode -eq "devfast") {
    $command += "--no-default-features"
}
if ($CargoArgs) {
    $command += $CargoArgs
}

Write-Host "Toolchain: nightly ($Backend, $Mode, frontend threads=8)"
Write-Host "Target:    $env:CARGO_TARGET_DIR"
Write-Host "Command:   cargo $($command -join ' ')"
& cargo @command
exit $LASTEXITCODE
