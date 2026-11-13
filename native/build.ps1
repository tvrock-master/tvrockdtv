#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('x64', 'x86', 'Win32')][string]$Architecture = 'x64',
    [ValidateSet('Release', 'Debug')][string]$Configuration = 'Release',
    [ValidateRange(1, 64)][int]$Jobs = 4,
    [Parameter(Mandatory=$true)][string]$OutputDir,
    [switch]$Rebuild,
    [switch]$Offline
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

. (Join-Path $PSScriptRoot 'tools/Processes.ps1')

function Find-Tool([string]$Name, [string]$Fallback) {
    # Prefer the matched native Windows CMake/Ninja pair from C++ Build Tools.
    if (Test-Path -LiteralPath $Fallback) { return $Fallback }
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    throw "Required tool not found: $Name. Install CMake and Ninja, or the C++ CMake tools component."
}

$target = if ($Architecture -eq 'Win32') { 'x86' } else { $Architecture }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (!(Test-Path -LiteralPath $vswhere)) {
    throw 'Install Microsoft C++ Build Tools 2019+ with MSVC x86/x64 and a Windows SDK. The Visual Studio IDE is not required.'
}
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$installation) { throw 'No MSVC x86/x64 toolchain was found. Install the C++ Build Tools workload.' }
$cmake = Find-Tool 'cmake.exe' (Join-Path $installation 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe')
$ninja = Find-Tool 'ninja.exe' (Join-Path $installation 'Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja\ninja.exe')
$developerCommand = Join-Path $installation 'Common7\Tools\VsDevCmd.bat'
if (!(Test-Path -LiteralPath $developerCommand)) { throw "Missing toolchain setup script: $developerCommand" }

# Import the command-line compiler environment without opening an IDE/window.
# Paths come from vswhere; reject cmd metacharacters instead of interpreting them.
if ($developerCommand -match '["%&|<>^\r\n]') { throw 'The toolchain path contains unsupported shell characters.' }
$command = '""{0}" -no_logo -arch={1} -host_arch=x64 && set"' -f $developerCommand, $target
$start = New-Object System.Diagnostics.ProcessStartInfo
$start.FileName = $env:ComSpec
$start.Arguments = '/u /d /s /c ' + $command
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.RedirectStandardOutput = $true
$start.RedirectStandardError = $true
$start.StandardOutputEncoding = [Text.Encoding]::Unicode
$start.StandardErrorEncoding = [Text.Encoding]::Unicode
$process = [Diagnostics.Process]::Start($start)
try {
    $errorOutput = $process.StandardError.ReadToEndAsync()
    $environment = $process.StandardOutput.ReadToEnd() -split '\r?\n'
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) {
        throw "Cannot initialize the MSVC command-line environment: $($errorOutput.Result)"
    }
} finally { $process.Dispose() }
$savedEnvironment = @{}
try {
    foreach ($line in $environment) {
        $separator = $line.IndexOf('=')
        if ($separator -le 0) { continue }
        $name = $line.Substring(0, $separator)
        $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $line.Substring($separator + 1), 'Process')
    }
    $savedEnvironment['VSLANG'] = [Environment]::GetEnvironmentVariable('VSLANG', 'Process')
    [Environment]::SetEnvironmentVariable('VSLANG', '1033', 'Process')
    . (Join-Path $PSScriptRoot 'tools/Dependencies.ps1')
    $manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'dependencies.json') -Raw | ConvertFrom-Json
    $cache = Join-Path $PSScriptRoot '.deps'
    $srtSource = Restore-Source $manifest.srt $cache $Offline.IsPresent $cmake
    $mbedTlsSource = Restore-Source $manifest.mbedtls $cache $Offline.IsPresent $cmake
    Invoke-Checked $cmake @('-S', $PSScriptRoot, '-B', (Join-Path $OutputDir 'build'), '-G', 'Ninja',
        "-DCMAKE_MAKE_PROGRAM=$ninja", '-DCMAKE_C_COMPILER=cl.exe', '-DCMAKE_CXX_COMPILER=cl.exe',
        '-DCMAKE_BUILD_TYPE=Release', "-DSRT_SOURCE_DIR=$srtSource", "-DMBEDTLS_SOURCE_DIR=$mbedTlsSource") `
        -TimeoutSeconds 300 -Label 'CMake configure'
    Invoke-Checked $cmake @('--build', (Join-Path $OutputDir 'build'), '--target', 'tvrock_srt', '--parallel', "$Jobs") `
        -TimeoutSeconds 900 -Label 'CMake build'
    Invoke-Checked $cmake @('--install', (Join-Path $OutputDir 'build'), '--prefix', $OutputDir) `
        -TimeoutSeconds 120 -Label 'CMake install'
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
}
