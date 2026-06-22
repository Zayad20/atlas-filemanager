param(
    [string]$Version = "0.1.0",
    [string]$Notes = "Atlas update",
    [string]$Installer = "target\installer\AtlasSetup-$Version.exe"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $Installer)) {
    throw "Installer not found: $Installer. Run scripts\build-windows-installer.ps1 first."
}

cargo run -p atlas-publisher -- publish-app $Installer --version $Version --notes $Notes
