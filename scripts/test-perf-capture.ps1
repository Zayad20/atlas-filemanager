$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'perf-capture.ps1')
$root = Split-Path -Parent $PSScriptRoot
$captureRoot = Join-Path $root 'artifacts'
$stdoutPath = Join-Path $captureRoot 'capture-test.stdout.tmp'
$stderrPath = Join-Path $captureRoot 'capture-test.stderr.tmp'
$exitCode = Invoke-PerfCapture -FilePath 'python.exe' `
    -Arguments @((Join-Path $captureRoot 'test-perf-emitter.py'), 'argument with spaces', 'trailing\', 'embedded"quote') `
    -WorkingDirectory $root -StdoutPath $stdoutPath -StderrPath $stderrPath
if ($exitCode -ne 7) { throw "Exit code was $exitCode, expected 7" }
foreach ($path in @($stdoutPath, $stderrPath)) {
    # Exclusive open verifies that the capture owns no surviving writer handle.
    $stream = [System.IO.File]::Open($path, 'Open', 'ReadWrite', 'None')
    $stream.Dispose()
    $text = [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::UTF8)
    if ($text.Split([char]10).Count -ne 4101) { throw "Truncated capture: $path" }
    if ($text -notmatch 'child-.*complete') { throw "Missing inherited-pipe output: $path" }
    if ($text.Contains([char]0xfffd)) { throw "Invalid UTF-8 in $path" }
}
Remove-Item -LiteralPath $stdoutPath, $stderrPath
Write-Host 'PASS: both streams drained, UTF-8 intact, arguments intact, exit code preserved, files unlocked.'
