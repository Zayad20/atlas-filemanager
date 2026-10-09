# Drain both pipes before releasing the capture files. Start-Process's redirect
# writers can outlive WaitForExit(), leaving ReadAllText with a sharing violation.
function Invoke-PerfCapture {
    param(
        [string]$FilePath,
        [string[]]$Arguments,
        [string]$WorkingDirectory,
        [string]$StdoutPath,
        [string]$StderrPath
    )
    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $FilePath
    $info.WorkingDirectory = $WorkingDirectory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    if ($info.PSObject.Properties['ArgumentList']) {
        foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    } else {
        # Windows command-line quoting for Windows PowerShell / .NET Framework.
        $quoted = foreach ($argument in $Arguments) {
            '"' + (($argument -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
        }
        $info.Arguments = $quoted -join ' '
    }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $info
    $stdout = $null
    $stderr = $null
    try {
        $stdout = [System.IO.File]::Create($StdoutPath)
        $stderr = [System.IO.File]::Create($StderrPath)
        if (-not $process.Start()) { throw "Could not start $FilePath" }
        $stdoutCopy = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
        $stderrCopy = $process.StandardError.BaseStream.CopyToAsync($stderr)
        $process.WaitForExit()
        $null = $stdoutCopy.GetAwaiter().GetResult()
        $null = $stderrCopy.GetAwaiter().GetResult()
        return $process.ExitCode
    } finally {
        if ($stdout) { $stdout.Dispose() }
        if ($stderr) { $stderr.Dispose() }
        $process.Dispose()
    }
}
