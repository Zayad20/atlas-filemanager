# Uninstalls Atlas Search Service with Administrator privileges
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$serviceExe = Join-Path $scriptDir "..\target\release\atlas-search-service.exe"

if (-not (Test-Path $serviceExe)) {
    $serviceExe = Join-Path $scriptDir "..\target\debug\atlas-search-service.exe"
}

$fullPath = (Get-Item $serviceExe).FullName
Write-Host "Elevating to uninstall AtlasSearchService..." -ForegroundColor Cyan
Start-Process powershell -Verb RunAs -Wait -ArgumentList "-NoProfile -ExecutionPolicy Bypass -Command `"`"$fullPath`" --uninstall; Start-Sleep -Seconds 2`"

Write-Host "Done." -ForegroundColor Green
sc.exe query AtlasSearchService
