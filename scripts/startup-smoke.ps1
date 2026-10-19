param(
    [string]$Exe = 'target/release-perf/atlas.exe',
    [string]$Output = 'artifacts/startup-smoke.json',
    [switch]$KeepRunning
)

$ErrorActionPreference = 'Stop'
$executable = (Resolve-Path -LiteralPath $Exe).Path
$started = [System.Diagnostics.Stopwatch]::StartNew()
# Atlas is the interactive application under test; launch it as the user does.
$atlasProcess = Start-Process -FilePath $executable -WindowStyle Normal -PassThru
$startupPassed = $false
try {
    do {
        Start-Sleep -Milliseconds 100
        $atlasProcess.Refresh()
        if ($atlasProcess.HasExited) {
            throw "Atlas exited during startup with code $($atlasProcess.ExitCode)"
        }
        $windowHandle = $atlasProcess.MainWindowHandle
    } while ($windowHandle -eq 0 -and $started.Elapsed.TotalSeconds -lt 20)
    if ($windowHandle -eq 0) {
        throw 'Atlas is running without a visible main window after 20 seconds'
    }
    $visibleAfterMs = $started.ElapsedMilliseconds
    # Cover the bounded startup callbacks and asynchronous directory loading.
    Start-Sleep -Seconds 3
    $atlasProcess.Refresh()
    if ($atlasProcess.HasExited -or $atlasProcess.MainWindowHandle -eq 0 -or !$atlasProcess.Responding) {
        throw 'Atlas lost its main window or stopped responding after startup'
    }
    $report = [ordered]@{
        executable = $executable
        pid = $atlasProcess.Id
        visible_after_ms = $visibleAfterMs
        main_window_handle = $atlasProcess.MainWindowHandle.ToInt64()
        title = $atlasProcess.MainWindowTitle
        responding = $atlasProcess.Responding
        checked_at = (Get-Date).ToString('o')
    } | ConvertTo-Json
    $report | Set-Content -LiteralPath $Output -Encoding utf8
    $startupPassed = $true
    $report
} finally {
    if ((!$KeepRunning -or !$startupPassed) -and !$atlasProcess.HasExited) {
        Stop-Process -Id $atlasProcess.Id
    }
}
