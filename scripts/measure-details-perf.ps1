# Measure Details body Element path (ring + custom Element).
#
# Usage (from repo root):
#   powershell -NoProfile -File scripts/measure-details-perf.ps1
#   powershell -NoProfile -File scripts/measure-details-perf.ps1 -SkipBuild
#
# Interact ~20-40s:
#   1) Wait 2s after open (ignore cold start)
#   2) Scroll Details
#   3) Split panes several times
#   4) Close the window
#
# Logs (always written for later inspection):
#   perf-logs/details-YYYYMMDD-HHMMSS.log
#   perf-logs/details-YYYYMMDD-HHMMSS.summary.txt
#   perf-details-element.log          <- latest full log
#   perf-details-element.summary.txt  <- latest summary

param(
    [switch]$SkipBuild,
    # Isolation (safe defaults: do NOT skip overlays — menus/DnD helpers live there)
    [switch]$SkipSidebar,
    [switch]$SkipOverlays,
    [switch]$SkipInspector,
    # Sidebar + inspector only. Overlays stay on so splits/menus keep working.
    [switch]$SkipChrome
)

$ErrorActionPreference = "Continue"
# PS 7+: do not treat native stderr as terminating errors
if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logDir = Join-Path $root "perf-logs"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null

$log = Join-Path $logDir "details-$stamp.log"
$summary = Join-Path $logDir "details-$stamp.summary.txt"
$latest = Join-Path $root "perf-details-element.log"
$latestSummary = Join-Path $root "perf-details-element.summary.txt"
$marker = Join-Path $root "target\release\.atlas-perf-log-built"

$env:ATLAS_PERF = "1"
$env:ATLAS_PERF_EVENTS = "1"
if (-not $env:RUST_LOG) { $env:RUST_LOG = "info" }
# GPUI already has phase timers (prepaint/taffy/paint/present). 0 = every frame.
if (-not $env:GPUI_FRAME_LOG) { $env:GPUI_FRAME_LOG = "0" }

if ($SkipChrome -or $SkipSidebar) { $env:ATLAS_PERF_SKIP_SIDEBAR = "1" } else { Remove-Item Env:ATLAS_PERF_SKIP_SIDEBAR -ErrorAction SilentlyContinue }
# Overlays only if explicitly requested (-SkipOverlays). -SkipChrome does NOT strip them.
if ($SkipOverlays) { $env:ATLAS_PERF_SKIP_OVERLAYS = "1" } else { Remove-Item Env:ATLAS_PERF_SKIP_OVERLAYS -ErrorAction SilentlyContinue }
if ($SkipChrome -or $SkipInspector) { $env:ATLAS_PERF_SKIP_INSPECTOR = "1" } else { Remove-Item Env:ATLAS_PERF_SKIP_INSPECTOR -ErrorAction SilentlyContinue }

$isolationFlags = @()
if ($env:ATLAS_PERF_SKIP_SIDEBAR -eq "1") { $isolationFlags += "sidebar" }
if ($env:ATLAS_PERF_SKIP_OVERLAYS -eq "1") { $isolationFlags += "overlays" }
if ($env:ATLAS_PERF_SKIP_INSPECTOR -eq "1") { $isolationFlags += "inspector" }
$isolationLabel = if ($isolationFlags.Count) { $isolationFlags -join "," } else { "none" }

$exe = Join-Path $root "target\release\atlas.exe"

$needBuild = -not $SkipBuild
if (-not (Test-Path $exe)) { $needBuild = $true }
if (-not (Test-Path $marker)) { $needBuild = $true }

if ($needBuild) {
    Write-Host "Building release with --features perf-log (required for frame_summary)..."
    & cargo build --release --features perf-log -p atlas
    if ($LASTEXITCODE -ne 0) {
        Write-Host "Build failed."
        exit 1
    }
    Set-Content -Path $marker -Value $stamp -Encoding utf8
} else {
    Write-Host "Using existing $exe (marker present; pass nothing to rebuild, or -SkipBuild is set)"
}

Write-Host ""
Write-Host "=== Atlas Details perf measure ==="
Write-Host "Log:          $log"
Write-Host "Summary:      $summary"
Write-Host "Latest log:   $latest"
Write-Host "Latest sum:   $latestSummary"
if ($env:ATLAS_PERF_SKIP_SIDEBAR -eq "1") { Write-Host "ISOLATION: skip sidebar" }
if ($env:ATLAS_PERF_SKIP_OVERLAYS -eq "1") { Write-Host "ISOLATION: skip overlays" }
if ($env:ATLAS_PERF_SKIP_INSPECTOR -eq "1") { Write-Host "ISOLATION: skip inspector" }
Write-Host ""
Write-Host "1) Wait 2s after open"
Write-Host "2) Scroll Details folder"
Write-Host "3) Split panes several times"
Write-Host "4) Close the window"
Write-Host ""
Write-Host "Starting atlas... (close the window when done)"
Write-Host ""

# Start-Process redirects avoid:
# - Tee-Object UTF-16 + NativeCommandError on stderr
# - PowerShell wrapping rustc/tracing lines as terminating errors
$proc = Start-Process -FilePath $exe `
    -WorkingDirectory $root `
    -RedirectStandardOutput $log `
    -RedirectStandardError (Join-Path $logDir "details-$stamp.stderr.log") `
    -PassThru `
    -NoNewWindow

# Merge stderr into main log after exit (keeps one log file)
$proc.WaitForExit()
$exitCode = $proc.ExitCode
$stderrPath = Join-Path $logDir "details-$stamp.stderr.log"

# Combine stdout + stderr into a single UTF-8 log (no BOM, plain text)
$combined = New-Object System.Text.StringBuilder
foreach ($part in @($log, $stderrPath)) {
    if (Test-Path $part) {
        $bytes = [System.IO.File]::ReadAllBytes($part)
        if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) {
            $text = [System.Text.Encoding]::Unicode.GetString($bytes, 2, $bytes.Length - 2)
        } elseif ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
            $text = [System.Text.Encoding]::UTF8.GetString($bytes, 3, $bytes.Length - 3)
        } else {
            $text = [System.Text.Encoding]::UTF8.GetString($bytes)
        }
        # Strip ANSI color sequences
        $text = [regex]::Replace($text, '\x1B\[[0-9;]*m', '')
        [void]$combined.Append($text)
        if (-not $text.EndsWith("`n")) { [void]$combined.AppendLine() }
    }
}
$utf8NoBom = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($log, $combined.ToString(), $utf8NoBom)
Copy-Item -Force $log $latest
if (Test-Path $stderrPath) { Remove-Item -Force $stderrPath -ErrorAction SilentlyContinue }

function Write-Summary {
    param([string]$OutPath, [string]$LogPath, [int]$Exit)

    function pickMs([string]$line, [string]$key) {
        if ($line -match "$([regex]::Escape($key))_ms=([\d.]+)") { return [double]$Matches[1] }
        return 0.0
    }

    $frames = @()
    if (Test-Path $LogPath) {
        Get-Content $LogPath -Encoding utf8 -ErrorAction SilentlyContinue | ForEach-Object {
            if ($_ -match '\[atlas-perf \+\s*([\d.]+)ms\] frame_summary frame=(\d+) gap_ms=([\d.]+)') {
                $t = [double]$Matches[1]
                $g = [double]$Matches[3]
                $hit = if ($_ -match 'details\.ring\.hit=(\d+)') { [int]$Matches[1] } else { 0 }
                $miss = if ($_ -match 'details\.ring\.miss=(\d+)') { [int]$Matches[1] } else { 0 }
                $el = if ($_ -match 'details\.element\.prepaint=(\d+)') { [int]$Matches[1] } else { 0 }
                $canvas = if ($_ -match 'details\.canvas\.prepaint=(\d+)') { [int]$Matches[1] } else { 0 }
                $cause = if ($_ -match 'cause=([^\s]+)') { $Matches[1] } else { "?" }
                $panes = if ($_ -match '\bpanes=(\d+)') { [int]$Matches[1] } else { 0 }
                $frames += [pscustomobject]@{
                    T = $t; Gap = $g; Hit = $hit; Miss = $miss
                    El = $el; Canvas = $canvas; Cause = $cause; Panes = $panes
                    AppMs = (pickMs $_ "app.render")
                    DockMs = (pickMs $_ "dock.render")
                    SideMs = (pickMs $_ "sidebar.render")
                    CenterMs = (pickMs $_ "app.render.center_dock")
                    OverlayMs = (pickMs $_ "app.render.overlays")
                    ElPreMs = (pickMs $_ "details.element.prepaint")
                    ElPaintMs = (pickMs $_ "details.element.paint")
                    ElRingMs = (pickMs $_ "details.element.ring")
                    PaneContentMs = (pickMs $_ "pane.content.render")
                    FilePanelMs = (pickMs $_ "file_panel.render")
                }
            }
        }
    }

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("Details perf measure $stamp")
    $lines.Add("exe_exit=$Exit")
    $lines.Add("log=$LogPath")
    $lines.Add("log_bytes=$((Get-Item $LogPath -ErrorAction SilentlyContinue).Length)")
    $lines.Add("isolation=$isolationLabel")
    $lines.Add("GPUI_FRAME_LOG=$($env:GPUI_FRAME_LOG)")
    $lines.Add("")

    if ($frames.Count -eq 0) {
        $lines.Add("No frame_summary lines found.")
        $lines.Add("Check: cargo build --release --features perf-log -p atlas")
        $lines.Add("Check: ATLAS_PERF=1 was set (script sets it).")
        $lines.Add("Check: you interacted and closed the window (not killed early).")
        $text = $lines -join "`n"
        [System.IO.File]::WriteAllText($OutPath, $text, $utf8NoBom)
        [System.IO.File]::WriteAllText($latestSummary, $text, $utf8NoBom)
        Write-Host $text
        return
    }

    function pct([double[]]$a, [double]$p) {
        if ($null -eq $a -or $a.Count -eq 0) { return 0 }
        $sorted = @($a | Sort-Object)
        $sorted[[int][math]::Floor(($sorted.Count - 1) * $p)]
    }

    $warm = @($frames | Where-Object { $_.T -gt 2000 })
    $gapsAll = @($frames | ForEach-Object { $_.Gap })
    $gapsWarm = @($warm | ForEach-Object { $_.Gap })
    $anim = @($warm | Where-Object { $_.Cause -match 'anim' })
    $gapsAnim = @($anim | ForEach-Object { $_.Gap })

    $lines.Add(("frames_total={0}" -f $frames.Count))
    $lines.Add(("frames_warm_gt2s={0}" -f $warm.Count))
    $lines.Add(("path_element_prepaint_frames={0}" -f @($frames | Where-Object { $_.El -gt 0 }).Count))
    $lines.Add(("path_canvas_prepaint_frames={0}" -f @($frames | Where-Object { $_.Canvas -gt 0 }).Count))
    $lines.Add("")
    $lines.Add(("gap_all p50={0} p90={1} max={2}" -f (pct $gapsAll 0.5), (pct $gapsAll 0.9), ($gapsAll | Measure-Object -Maximum).Maximum))
    if ($gapsWarm.Count -gt 0) {
        $lines.Add(("gap_warm p50={0} p90={1} max={2}" -f (pct $gapsWarm 0.5), (pct $gapsWarm 0.9), ($gapsWarm | Measure-Object -Maximum).Maximum))
    }
    $hitSum = ($warm | Measure-Object Hit -Sum).Sum
    $missSum = ($warm | Measure-Object Miss -Sum).Sum
    if ($null -eq $hitSum) { $hitSum = 0 }
    if ($null -eq $missSum) { $missSum = 0 }
    $lines.Add(("ring_warm hitSum={0} missSum={1}" -f $hitSum, $missSum))
    if ($anim.Count -gt 0) {
        $aHit = ($anim | Measure-Object Hit -Sum).Sum
        $aMiss = ($anim | Measure-Object Miss -Sum).Sum
        if ($null -eq $aHit) { $aHit = 0 }
        if ($null -eq $aMiss) { $aMiss = 0 }
        $lines.Add(("anim_warm n={0} p50={1} p90={2} max={3} hitSum={4} missSum={5}" -f `
            $anim.Count, (pct $gapsAnim 0.5), (pct $gapsAnim 0.9), ($gapsAnim | Measure-Object -Maximum).Maximum, `
            $aHit, $aMiss))
    } else {
        $lines.Add("anim_warm n=0 (no cause=anim_* frames after t>2s)")
    }

    function meanMs($arr, $prop) {
        $v = @($arr | ForEach-Object { $_.$prop } | Where-Object { $_ -gt 0 })
        if ($v.Count -eq 0) { return 0 }
        [math]::Round(($v | Measure-Object -Average).Average, 2)
    }
    function p50Ms($arr, $prop) {
        $v = @($arr | ForEach-Object { $_.$prop } | Sort-Object)
        if ($v.Count -eq 0) { return 0 }
        $v[[int](($v.Count - 1) * 0.5)]
    }

    $lines.Add("")
    $lines.Add("--- phase timings on anim_warm (ms, summed across panes/callers; span parents include children) ---")
    if ($anim.Count -gt 0) {
        foreach ($pair in @(
            @("gap", "Gap"),
            @("app.render", "AppMs"),
            @("dock.render", "DockMs"),
            @("app.render.center_dock", "CenterMs"),
            @("app.render.overlays", "OverlayMs"),
            @("pane.content.render", "PaneContentMs"),
            @("file_panel.render", "FilePanelMs"),
            @("details.element.prepaint", "ElPreMs"),
            @("details.element.ring", "ElRingMs"),
            @("details.element.paint", "ElPaintMs")
        )) {
            $label = $pair[0]; $prop = $pair[1]
            $lines.Add(("  {0,-28} p50={1} mean={2}" -f $label, (p50Ms $anim $prop), (meanMs $anim $prop)))
        }
        # Approx cost per pane for element phases
        $withPanes = @($anim | Where-Object { $_.Panes -gt 0 -and $_.ElPreMs -gt 0 })
        if ($withPanes.Count -gt 0) {
            $perPanePre = @($withPanes | ForEach-Object { $_.ElPreMs / $_.Panes } | Sort-Object)
            $perPanePaint = @($withPanes | ForEach-Object { if ($_.ElPaintMs -gt 0) { $_.ElPaintMs / $_.Panes } else { 0 } } | Sort-Object)
            $lines.Add(("  element.prepaint per pane   p50={0}" -f $perPanePre[[int](($perPanePre.Count-1)*0.5)]))
            $lines.Add(("  element.paint per pane      p50={0}" -f $perPanePaint[[int](($perPanePaint.Count-1)*0.5)]))
        }
    } else {
        $lines.Add("  (no anim frames)")
    }

    $lines.Add("")
    # Aggregate [gpui-frame] lines (framework prepaint/paint/present)
    $gpuiTotals = @(); $gpuiPre = @(); $gpuiPaint = @(); $gpuiPresent = @(); $gpuiTaffy = @()
    Get-Content $LogPath -Encoding utf8 -ErrorAction SilentlyContinue | ForEach-Object {
        # Full line form from gpui window.rs draw_roots (do not match "paint=" inside "prepaint=")
        if ($_ -match '\[gpui-frame\] total=([\d.]+)ms prepaint=([\d.]+)ms \(taffy=([\d.]+)ms measure=([\d.]+)ms/\d+ nodes=\d+\) paint=([\d.]+)ms') {
            $gpuiTotals += [double]$Matches[1]
            $gpuiPre += [double]$Matches[2]
            $gpuiTaffy += [double]$Matches[3]
            $gpuiPaint += [double]$Matches[5]
        }
        if ($_ -match '\[gpui-frame\] present_ms=([\d.]+)') {
            $gpuiPresent += [double]$Matches[1]
        }
    }
    function pctList([double[]]$a, [double]$p) {
        if ($null -eq $a -or $a.Count -eq 0) { return 0 }
        $s = @($a | Sort-Object)
        $s[[int][math]::Floor(($s.Count - 1) * $p)]
    }
    $lines.Add("")
    $lines.Add("--- gpui-frame (framework draw; residual outside atlas spans) ---")
    if ($gpuiTotals.Count -gt 0) {
        $lines.Add(("  gpui total    n={0} p50={1} p90={2}" -f $gpuiTotals.Count, (pctList $gpuiTotals 0.5), (pctList $gpuiTotals 0.9)))
        $lines.Add(("  gpui prepaint p50={0} p90={1}" -f (pctList $gpuiPre 0.5), (pctList $gpuiPre 0.9)))
        $lines.Add(("  gpui taffy    p50={0} p90={1}" -f (pctList $gpuiTaffy 0.5), (pctList $gpuiTaffy 0.9)))
        if ($gpuiPaint.Count -gt 0) {
            $lines.Add(("  gpui paint    p50={0} p90={1}" -f (pctList $gpuiPaint 0.5), (pctList $gpuiPaint 0.9)))
        }
    } else {
        $lines.Add("  (no [gpui-frame] lines - set GPUI_FRAME_LOG=0)")
    }
    if ($gpuiPresent.Count -gt 0) {
        $lines.Add(("  gpui present  n={0} p50={1} p90={2} max={3}" -f $gpuiPresent.Count, (pctList $gpuiPresent 0.5), (pctList $gpuiPresent 0.9), ($gpuiPresent | Measure-Object -Maximum).Maximum))
    } else {
        $lines.Add("  (no present_ms - rebuild gpui after present always-log patch, or no presents)")
    }

    # [gpui-phases] root_prepaint=.. deferred_prepaint=.. root_paint=..
    $phaseSums = @{}
    $phaseN = @{}
    Get-Content $LogPath -Encoding utf8 -ErrorAction SilentlyContinue | ForEach-Object {
        if ($_ -match '\[gpui-phases\]\s+(.*)$') {
            [regex]::Matches($Matches[1], '([\w.]+)=([\d.]+)') | ForEach-Object {
                $k = $_.Groups[1].Value
                $v = [double]$_.Groups[2].Value
                if (-not $phaseSums.ContainsKey($k)) { $phaseSums[$k] = 0.0; $phaseN[$k] = 0 }
                $phaseSums[$k] = [double]$phaseSums[$k] + $v
                $phaseN[$k] = [int]$phaseN[$k] + 1
            }
        }
    }
    $lines.Add("")
    $lines.Add("--- gpui-phases (mean ms inside Window::draw_roots) ---")
    if ($phaseN.Count -eq 0) {
        $lines.Add("  (no [gpui-phases] - rebuild gpui with phase timers + GPUI_FRAME_LOG=0)")
    } else {
        $keys = @($phaseSums.Keys | Sort-Object { -($phaseSums[$_] / [math]::Max(1, $phaseN[$_])) })
        foreach ($k in $keys) {
            $mean = [math]::Round($phaseSums[$k] / [math]::Max(1, $phaseN[$k]), 2)
            $lines.Add(("  {0,-22} mean_ms={1}" -f $k, $mean))
        }
        $lines.Add("  (root_prepaint = layout+prepaint of App tree; root_paint = scene build)")
    }

    # [gpui-layout] nodes_created=N sidebar=.. details.chrome=.. details.body=..
    $regionSums = @{}
    $regionSamples = 0
    Get-Content $LogPath -Encoding utf8 -ErrorAction SilentlyContinue | ForEach-Object {
        if ($_ -match '\[gpui-layout\] nodes_created=(\d+)\s+(.*)$') {
            $regionSamples++
            $rest = $Matches[2]
            [regex]::Matches($rest, '([\w.]+)=(\d+)') | ForEach-Object {
                $k = $_.Groups[1].Value
                $v = [int64]$_.Groups[2].Value
                if (-not $regionSums.ContainsKey($k)) { $regionSums[$k] = [int64]0; $regionSums["${k}__n"] = [int64]0 }
                $regionSums[$k] = [int64]$regionSums[$k] + $v
                $regionSums["${k}__n"] = [int64]$regionSums["${k}__n"] + 1
            }
        }
    }
    $lines.Add("")
    $lines.Add("--- gpui-layout region nodes (mean per logged frame) ---")
    if ($regionSamples -eq 0) {
        $lines.Add("  (no [gpui-layout] lines - rebuild with layout regions + GPUI_FRAME_LOG)")
    } else {
        $lines.Add(("  samples={0}" -f $regionSamples))
        $keys = @($regionSums.Keys | Where-Object { $_ -notmatch '__n$' } | Sort-Object { -($regionSums[$_] / [math]::Max(1, $regionSums["${_}__n"])) })
        foreach ($k in $keys) {
            $n = [math]::Max(1, [double]$regionSums["${k}__n"])
            $mean = [math]::Round([double]$regionSums[$k] / $n, 1)
            $lines.Add(("  {0,-22} mean_nodes={1}" -f $k, $mean))
        }
    }

    $lines.Add("")
    $lines.Add("Baseline reference (old perf.log, monolitico+ring): warm p50~16ms, anim p50~18-20ms")
    $lines.Add("Expect: details.element.* present; details.canvas.* absent")
    $lines.Add("Flags: -SkipChrome / -SkipSidebar / -SkipOverlays / -SkipInspector")

    $text = $lines -join "`n"
    [System.IO.File]::WriteAllText($OutPath, $text, $utf8NoBom)
    [System.IO.File]::WriteAllText($latestSummary, $text, $utf8NoBom)
    Write-Host ""
    Write-Host "=== Quick summary ==="
    Write-Host $text
    Write-Host ""
    Write-Host "Saved logs:"
    Write-Host "  $LogPath"
    Write-Host "  $OutPath"
    Write-Host "  $latest"
    Write-Host "  $latestSummary"
}

Write-Summary -OutPath $summary -LogPath $log -Exit $exitCode
exit $exitCode
