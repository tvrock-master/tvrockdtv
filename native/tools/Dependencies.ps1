. (Join-Path $PSScriptRoot 'Processes.ps1')

function Restore-Source([object]$Dependency, [string]$Cache, [bool]$Offline, [string]$CMake = 'cmake.exe') {
    $source = Join-Path $Cache $Dependency.directory
    $marker = Join-Path $source '.source-sha256'
    if ((Test-Path -LiteralPath $marker) -and
        (Test-Path -LiteralPath (Join-Path $source 'CMakeLists.txt')) -and
        ((Get-Content -LiteralPath $marker -Raw).Trim() -eq $Dependency.sha256)) {
        Write-Host "Using cached source: $($Dependency.directory)"
        return $source
    }

    New-Item -ItemType Directory -Path $Cache -Force | Out-Null
    $archive = Join-Path $Cache $Dependency.archive
    if (!(Test-Path -LiteralPath $archive)) {
        if ($Offline) { throw "Offline archive is missing: $archive" }
        $previousTls = [Net.ServicePointManager]::SecurityProtocol
        $previousProgress = $ProgressPreference
        $download = "$archive.$PID.download"
        try {
            [Net.ServicePointManager]::SecurityProtocol = $previousTls -bor [Net.SecurityProtocolType]::Tls12
            # Windows PowerShell's download progress rendering can be very slow in CI.
            $ProgressPreference = 'SilentlyContinue'
            Write-Host "Downloading $($Dependency.directory) from $($Dependency.url) (timeout: 180s)"
            # Keep interrupted downloads separate from the reusable archive.
            Invoke-WebRequest -UseBasicParsing -Uri $Dependency.url -OutFile $download -TimeoutSec 180
            Move-Item -LiteralPath $download -Destination $archive -Force
        } finally {
            if ($download -and (Test-Path -LiteralPath $download)) {
                Remove-Item -LiteralPath $download -Force
            }
            $ProgressPreference = $previousProgress
            [Net.ServicePointManager]::SecurityProtocol = $previousTls
        }
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $Dependency.sha256) {
        throw "Dependency checksum mismatch: $archive"
    }

    # Remove a stale success marker before attempting to repair an extraction.
    if (Test-Path -LiteralPath $marker) { Remove-Item -LiteralPath $marker -Force }
    if ($Dependency.archive.EndsWith('.zip')) {
        Invoke-Checked "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" @(
            '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
            '-File', (Join-Path $PSScriptRoot 'ExpandZip.ps1'), $archive, $Cache
        ) -TimeoutSeconds 180 -Label "Extract $($Dependency.directory)"
    } else {
        # Windows Server's tar stalled on the Mbed TLS archive in CI. Use
        # CMake's archive reader, as BonDriver_SRT does, without external tar.
        Invoke-Checked $CMake @('-E', 'chdir', $Cache, $CMake, '-E', 'tar', 'xf', ([IO.Path]::GetFullPath($archive))) `
            -TimeoutSeconds 180 -Label "Extract $($Dependency.directory)"
    }
    if (!(Test-Path -LiteralPath (Join-Path $source 'CMakeLists.txt'))) {
        throw "Dependency extraction did not produce the expected source directory: $source"
    }
    Set-Content -LiteralPath $marker -Value $Dependency.sha256 -Encoding ascii
    return $source
}
