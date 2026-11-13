# Windows PowerShell 5.1-compatible runner with inherited output and closed stdin.
function Invoke-Checked {
    param(
        [string]$Program,
        [string[]]$Arguments,
        [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
        [string]$Label = $Program
    )
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = $Program
    # Quote according to Windows command-line argument parsing rules.
    $start.Arguments = (($Arguments | ForEach-Object {
        '"' + (($_ -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
    }) -join ' ')
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $timer = [Diagnostics.Stopwatch]::StartNew()
    Write-Host "Starting $Label (timeout: ${TimeoutSeconds}s): $Program"
    $process = [Diagnostics.Process]::Start($start)
    try {
        $process.StandardInput.Close()
        $nextReport = 30
        while (!$process.WaitForExit(1000)) {
            $elapsed = [int]$timer.Elapsed.TotalSeconds
            if ($elapsed -ge $TimeoutSeconds) {
                # Kill descendants too (CMake starts Ninja, which starts cl.exe).
                & "$env:SystemRoot\System32\taskkill.exe" /PID $process.Id /T /F | Out-Host
                if (!$process.HasExited) { $process.Kill() }
                throw "$Label timed out after ${TimeoutSeconds}s."
            }
            if ($elapsed -ge $nextReport) {
                Write-Host "Waiting for $Label (${elapsed}s elapsed, PID $($process.Id))"
                $nextReport += 30
            }
        }
        if ($process.ExitCode -ne 0) {
            throw "$Label failed with exit code $($process.ExitCode)."
        }
        Write-Host "Completed $Label ($([int]$timer.Elapsed.TotalSeconds)s)"
    } finally {
        $process.Dispose()
    }
}
