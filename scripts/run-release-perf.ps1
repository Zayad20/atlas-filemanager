[CmdletBinding()]
param(
    # Use the definitive release profile (Thin LTO + one codegen unit).
    # Omit for the faster iterative release-perf profile.
    [switch]$Exact,
    [switch]$Lean,
    [switch]$BuildOnly,
    [double]$ThresholdMs = 4.0,
    [switch]$NoEvents,
    [switch]$NoGpuiFrames,
    [switch]$NoFileLog,
    [switch]$SkipSidebar,
    [switch]$SkipOverlays,
    [switch]$SkipInspector,
    [string]$LogPath,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$ErrorActionPreference = "Stop"
if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
}

$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

# Keep this diagnostic path on the supported stable LLVM toolchain. Cargo still
# parallelizes crate builds normally; release-perf's 16 codegen units let LLVM
# work on the large atlas-ui crate in parallel without nightly Rust flags.
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue

$env:ATLAS_PERF = "1"
$env:ATLAS_PERF_EVENTS = if ($NoEvents) { "0" } else { "1" }
$env:ATLAS_PERF_MS = $ThresholdMs.ToString(
    [System.Globalization.CultureInfo]::InvariantCulture
)
if ($NoGpuiFrames) {
    Remove-Item Env:GPUI_FRAME_LOG -ErrorAction SilentlyContinue
} else {
    # GPUI's fork emits one [gpui-frame] line per Window::draw, including
    # render/layout/prepaint/paint CPU timings. GPU submission/presentation is
    # separate; these numbers capture CPU work outside Atlas spans.
    $env:GPUI_FRAME_LOG = "0"
}
if (-not $env:RUST_LOG) {
    $env:RUST_LOG = "info"
}
if ($SkipSidebar) {
    $env:ATLAS_PERF_SKIP_SIDEBAR = "1"
} else {
    Remove-Item Env:ATLAS_PERF_SKIP_SIDEBAR -ErrorAction SilentlyContinue
}
if ($SkipOverlays) {
    $env:ATLAS_PERF_SKIP_OVERLAYS = "1"
} else {
    Remove-Item Env:ATLAS_PERF_SKIP_OVERLAYS -ErrorAction SilentlyContinue
}
if ($SkipInspector) {
    $env:ATLAS_PERF_SKIP_INSPECTOR = "1"
} else {
    Remove-Item Env:ATLAS_PERF_SKIP_INSPECTOR -ErrorAction SilentlyContinue
}

$profile = if ($Exact) { "release" } else { "release-perf" }
$searchExe = Join-Path $root "target\$profile\atlas-search-service.exe"

function Close-WorkspaceAtlas {
    $atlasExe = [System.IO.Path]::GetFullPath((Join-Path $root "target\$profile\atlas.exe"))
    foreach ($process in Get-Process -Name atlas -ErrorAction SilentlyContinue) {
        if (-not $process.Path -or -not $process.Path.Equals($atlasExe, [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        $process.Refresh()
        if ($process.MainWindowHandle -eq [IntPtr]::Zero) {
            Write-Host "Stopping background Atlas before rebuild: $atlasExe (PID $($process.Id))"
            Stop-Process -Id $process.Id -Force -ErrorAction Stop
            if (-not $process.WaitForExit(5000)) {
                throw "Background Atlas at $atlasExe is still exiting; retry the build."
            }
            continue
        }
        Write-Host "Closing Atlas before rebuild: $atlasExe (PID $($process.Id))"
        # Let the normal close handler save the workspace. Never force-close
        # an interactive instance or touch an app from another checkout.
        if (-not $process.CloseMainWindow() -or -not $process.WaitForExit(5000)) {
            throw "Close Atlas at $atlasExe before rebuilding; its executable is still in use."
        }
    }
}

function Stop-WorkspaceSearchConsole {
    param([string]$OnlyExecutable)

    $targetRoot = [System.IO.Path]::GetFullPath((Join-Path $root "target")) + [System.IO.Path]::DirectorySeparatorChar
    foreach ($process in Get-CimInstance Win32_Process -Filter "Name = 'atlas-search-service.exe'") {
        # Only replace development console processes from this checkout.
        # Installed services and other checkouts have their own lifecycle.
        if (-not $process.ExecutablePath -or $process.CommandLine -notmatch '(?:^|\s)--console(?:\s|$)') {
            continue
        }
        $executable = [System.IO.Path]::GetFullPath($process.ExecutablePath)
        if (-not $executable.StartsWith($targetRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        if ($OnlyExecutable -and -not $executable.Equals($OnlyExecutable, [System.StringComparison]::OrdinalIgnoreCase)) {
            continue
        }
        Write-Host "Stopping workspace search console: $executable (PID $($process.ProcessId))"
        Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
    }
}

# Cargo run builds only the Atlas package, so build its companion explicitly.
# Windows locks running executables: release only this destination before build.
Close-WorkspaceAtlas
Stop-WorkspaceSearchConsole -OnlyExecutable $searchExe
Write-Host "Search:    cargo +1.97.1 build --locked --profile $profile -p atlas-search-service"
& cargo +1.97.1 build --locked --profile $profile -p atlas-search-service
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
if (-not $BuildOnly) {
    # Replace a console from another profile before Atlas probes the shared pipe.
    Stop-WorkspaceSearchConsole
    $searchProcess = Start-Process -FilePath $searchExe -ArgumentList '--console' -WindowStyle Hidden -PassThru
    Write-Host "Search:    $searchExe (PID $($searchProcess.Id))"
}

$verb = if ($BuildOnly) { "build" } else { "run" }
$command = @("+1.97.1", $verb, "--locked", "--profile", $profile)
if ($Lean) {
    $command += "--no-default-features"
}
$command += @("--features", "perf-log")
if ($CargoArgs) {
    $command += $CargoArgs
}

Write-Host "Compiler:  stable LLVM 1.97.1"
Write-Host "Linker:    rust-lld.exe"
Write-Host "Profile:   $profile"
Write-Host "Perf logs: enabled (events=$(-not $NoEvents), threshold_ms=$env:ATLAS_PERF_MS)"
Write-Host "GPUI frames: $(-not $NoGpuiFrames)"
Write-Host "Isolation: sidebar=$SkipSidebar overlays=$SkipOverlays inspector=$SkipInspector"
Write-Host "Command:   cargo $($command -join ' ')"

$cargoExit = 0
if ($NoFileLog) {
    $previousErrorAction = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & cargo @command
        $cargoExit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorAction
    }
} else {
    $logRoot = Join-Path $root "perf-logs"
    New-Item -ItemType Directory -Force -Path $logRoot | Out-Null
    if ([string]::IsNullOrWhiteSpace($LogPath)) {
        $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
        $resolvedLogPath = Join-Path $logRoot "release-perf-$stamp.log"
    } elseif ([System.IO.Path]::IsPathRooted($LogPath)) {
        $resolvedLogPath = $LogPath
    } else {
        $resolvedLogPath = Join-Path $root $LogPath
    }
    $resolvedLogPath = [System.IO.Path]::GetFullPath($resolvedLogPath)
    $logParent = Split-Path -Parent $resolvedLogPath
    New-Item -ItemType Directory -Force -Path $logParent | Out-Null
    $utf8NoBom = New-Object System.Text.UTF8Encoding $false
    . (Join-Path $PSScriptRoot "perf-capture.ps1")
    $captureId = [guid]::NewGuid().ToString("N")
    $stdoutPath = "$resolvedLogPath.$captureId.stdout.tmp"
    $stderrPath = "$resolvedLogPath.$captureId.stderr.tmp"
    $gpuiFrameLog = if ($env:GPUI_FRAME_LOG) { $env:GPUI_FRAME_LOG } else { "unset" }
    $skipSidebarValue = if ($env:ATLAS_PERF_SKIP_SIDEBAR) { $env:ATLAS_PERF_SKIP_SIDEBAR } else { "0" }
    $skipOverlaysValue = if ($env:ATLAS_PERF_SKIP_OVERLAYS) { $env:ATLAS_PERF_SKIP_OVERLAYS } else { "0" }
    $skipInspectorValue = if ($env:ATLAS_PERF_SKIP_INSPECTOR) { $env:ATLAS_PERF_SKIP_INSPECTOR } else { "0" }
    $header = @(
        "# Atlas release-perf"
        "# ATLAS_PERF=$env:ATLAS_PERF ATLAS_PERF_EVENTS=$env:ATLAS_PERF_EVENTS ATLAS_PERF_MS=$env:ATLAS_PERF_MS"
        "# GPUI_FRAME_LOG=$gpuiFrameLog RUST_LOG=$env:RUST_LOG"
        "# ATLAS_PERF_SKIP_SIDEBAR=$skipSidebarValue ATLAS_PERF_SKIP_OVERLAYS=$skipOverlaysValue ATLAS_PERF_SKIP_INSPECTOR=$skipInspectorValue"
        "# ATLAS_CONFIG_DIR=$env:ATLAS_CONFIG_DIR ATLAS_PERF_AUTOMATE_SPLITS=$env:ATLAS_PERF_AUTOMATE_SPLITS ATLAS_PERF_SPLIT_COUNT=$env:ATLAS_PERF_SPLIT_COUNT ATLAS_PERF_SPLIT_INTERVAL_MS=$env:ATLAS_PERF_SPLIT_INTERVAL_MS"
        ""
    ) -join "`n"
    [System.IO.File]::WriteAllText($resolvedLogPath, $header, $utf8NoBom)
    $captureMerged = $false
    try {
        $cargoExit = Invoke-PerfCapture -FilePath "cargo.exe" `
            -Arguments $command `
            -WorkingDirectory $root `
            -StdoutPath $stdoutPath `
            -StderrPath $stderrPath
        foreach ($part in @($stdoutPath, $stderrPath)) {
            if (Test-Path -LiteralPath $part) {
                $partText = [System.IO.File]::ReadAllText($part, $utf8NoBom)
                [System.IO.File]::AppendAllText($resolvedLogPath, $partText, $utf8NoBom)
                if ($partText) {
                    Write-Output $partText.TrimEnd()
                }
            }
        }
        $captureMerged = $true
    } finally {
        if ($captureMerged) {
            Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
        } else {
            Write-Warning "Capture retained for recovery: $stdoutPath, $stderrPath"
        }
    }
    Write-Host "Log:       $resolvedLogPath"
}
exit $cargoExit
