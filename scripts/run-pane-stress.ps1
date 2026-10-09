#requires -Version 7.0
# Rapid split/close capture using an isolated copy of preferences.
[CmdletBinding()]
param(
    [ValidateRange(1, 64)][int]$SplitCount = 24,
    [ValidateRange(16, 2000)][int]$IntervalMs = 40,
    [string]$Theme = 'Atlas Graphite Glass',
    [switch]$Preview,
    [switch]$DisableAnimations,
    [string]$LogPath = '.\perf-logs\pane-stress.log'
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$captureId = [guid]::NewGuid().ToString('N')
$configPath = Join-Path $root "artifacts\perf-profile-$captureId"
New-Item -ItemType Directory -Path $configPath | Out-Null
$userConfig = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'atlas'
$utf8 = [System.Text.UTF8Encoding]::new($false)
$settingsPath = Join-Path $userConfig 'settings.json'
$settings = if (Test-Path -LiteralPath $settingsPath) {
    Get-Content -LiteralPath $settingsPath -Raw | ConvertFrom-Json -AsHashtable
} else { @{} }
foreach ($pair in @(
    @('getting_started_seen', $true), @('disable_animations', [bool]$DisableAnimations),
    @('show_inspector', [bool]$Preview), @('inspector_placement', 'Global'), @('autosave_session', $false)
)) {
    $settings[$pair[0]] = $pair[1]
}
[IO.File]::WriteAllText((Join-Path $configPath 'settings.json'), ($settings | ConvertTo-Json -Depth 30), $utf8)
$profilesPath = Join-Path $userConfig 'profiles.json'
if (Test-Path -LiteralPath $profilesPath) {
    $profiles = Get-Content -LiteralPath $profilesPath -Raw | ConvertFrom-Json -AsHashtable
    $active = $profiles.profiles[$profiles.active_idx]
    $active.theme = $Theme
    $active.tree = $null
    $active.workspace.show_inspector = [bool]$Preview
    [IO.File]::WriteAllText((Join-Path $configPath 'profiles.json'), ($profiles | ConvertTo-Json -Depth 100), $utf8)
} else {
    [IO.File]::WriteAllText((Join-Path $configPath 'profiles.json'),
        (@{profiles=@(@{name='Perf';theme=$Theme});active_idx=0;global_tags=@()} | ConvertTo-Json -Depth 10), $utf8)
}
$scopedEnvironment = @('ATLAS_CONFIG_DIR', 'ATLAS_PERF_AUTOMATE_SPLITS', 'ATLAS_PERF_SPLIT_COUNT', 'ATLAS_PERF_SPLIT_INTERVAL_MS')
$previous = @{}
foreach ($key in $scopedEnvironment) { $previous[$key] = [Environment]::GetEnvironmentVariable($key) }
$env:ATLAS_CONFIG_DIR = $configPath
$env:ATLAS_PERF_AUTOMATE_SPLITS = '1'
$env:ATLAS_PERF_SPLIT_COUNT = "$SplitCount"
$env:ATLAS_PERF_SPLIT_INTERVAL_MS = "$IntervalMs"
Write-Host "Stress: $SplitCount splits + closes, $IntervalMs ms apart, theme=$Theme preview=$Preview animations=$(-not $DisableAnimations)"
Write-Host "Isolated preferences: $configPath"
# Start from one pane in a known directory; session.json is deliberately absent.
try {
    & (Join-Path $PSScriptRoot 'run-release-perf.ps1') -LogPath $LogPath -CargoArgs @('-j', '2', '--', $root)
} finally {
    foreach ($key in $scopedEnvironment) { [Environment]::SetEnvironmentVariable($key, $previous[$key]) }
}
