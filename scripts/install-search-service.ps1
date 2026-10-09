# Installs Atlas Search Service as an automatic Windows Service with Administrator privileges
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$serviceExe = Join-Path $scriptDir "..\target\release\atlas-search-service.exe"

if (-not (Test-Path $serviceExe)) {
    $serviceExe = Join-Path $scriptDir "..\target\debug\atlas-search-service.exe"
}

if (-not (Test-Path $serviceExe)) {
    Write-Host "atlas-search-service.exe not found. Building release..." -ForegroundColor Yellow
    Push-Location (Join-Path $scriptDir "..")
    try {
        cargo build --release -p atlas-search-service
    } finally {
        Pop-Location
    }
    $serviceExe = Join-Path $scriptDir "..\target\release\atlas-search-service.exe"
}

$fullPath = (Get-Item $serviceExe).FullName
Write-Host "Elevating to register AtlasSearchService as auto-start Windows Service..." -ForegroundColor Cyan
Start-Process powershell -Verb RunAs -Wait -ArgumentList "-NoProfile -ExecutionPolicy Bypass -Command `"`"$fullPath`" --install; Start-Sleep -Seconds 2`"

Write-Host "Done. Checking service status..." -ForegroundColor Green
sc.exe query AtlasSearchService
