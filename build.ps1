param([ValidateSet('x64','x86')][string]$Architecture='x64',[switch]$Debug,[switch]$CollectDrcs)
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$target=if($Architecture -eq 'x86'){'i686-pc-windows-msvc'}else{'x86_64-pc-windows-msvc'}
$profile=if($Debug){'debug'}else{'release'}
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$installation=& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if(!$installation){throw 'Visual Studio C++ Build Tools are required for the C++ ABI adapter.'}
$devshell=Join-Path $installation 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll'
Import-Module $devshell
# Keep paths added by CI (for example Rust), while removing duplicates from
# repeated shell initialization. Rebuild architecture-specific include/lib lists.
$env:PATH=($env:PATH -split ';' | Select-Object -Unique) -join ';'
foreach($name in @('INCLUDE','LIB','LIBPATH')){
    [Environment]::SetEnvironmentVariable($name,$null,'Process')
}
$vsArch=if($Architecture -eq 'x86'){'x86'}else{'amd64'}
Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -Arch $vsArch -HostArch amd64 -SkipExistingEnvironmentVariables:$false | Out-Null
# Select the compiler explicitly so a previously used x86/x64 compiler on PATH
# cannot compile the adapter for a different architecture than the Rust library.
$compiler=Join-Path $env:VCToolsInstallDir "bin\Hostx64\$Architecture\cl.exe"
if(!(Test-Path -LiteralPath $compiler)){throw "MSVC compiler not found: $compiler"}
# test.ps1 also invokes cl/dumpbin by name after this script returns.
$env:PATH=((@((Split-Path -Parent $compiler)) + ($env:PATH -split ';')) | Select-Object -Unique) -join ';'
# LIB is architecture-specific too. Do not let an inherited x86/x64 SDK or CRT
# directory supply libraries for the explicitly selected compiler.
$sdkLibVersion=$env:WindowsSDKLibVersion
if(!$sdkLibVersion){$sdkLibVersion=$env:WindowsSDKVersion}
if(!$env:WindowsSdkDir -or !$sdkLibVersion){throw 'Windows SDK library location is missing.'}
$libraryDirs=@(
    (Join-Path $env:VCToolsInstallDir "lib\$Architecture"),
    (Join-Path $env:WindowsSdkDir "Lib\$sdkLibVersion\ucrt\$Architecture"),
    (Join-Path $env:WindowsSdkDir "Lib\$sdkLibVersion\um\$Architecture")
)
foreach($libraryDir in $libraryDirs){
    if(!(Test-Path -LiteralPath $libraryDir -PathType Container)){throw "Library directory not found: $libraryDir"}
}
$env:LIB=$libraryDirs -join ';'
Write-Output "MSVC compiler ($Architecture): $compiler"
Write-Output "MSVC/SDK libraries ($Architecture): $env:LIB"
$argsList=@('build','--locked','--manifest-path',(Join-Path $root 'Cargo.toml'),'--target',$target)
if(!$Debug){$argsList+='--release'}
if($CollectDrcs){$argsList+=@('--features','drcs-collect')}
& cargo @argsList
if($LASTEXITCODE){throw 'Rust build failed. See the Cargo error above for the cause.'}
$output=Join-Path $root "target\$target\$profile"
$rustLibrary=Join-Path $output 'tvrockdtv.lib'
if($CollectDrcs){
    $output=Join-Path $output 'drcs-collector'
    New-Item -ItemType Directory -Force -Path $output | Out-Null
}
$flags=if($Debug){@('/Od','/Zi','/MD')}else{@('/O2','/MD','/DNDEBUG')}
Push-Location $output
try {
    & $compiler /nologo /LD /EHsc /std:c++17 /utf-8 /DUNICODE /D_UNICODE /D_USRDLL @flags "/I$root\compat" "$root\compat\TvRockDTV.cpp" "$rustLibrary" /link /INCREMENTAL:NO /OUT:TvRockDTV.dll /IMPLIB:TvRockDTV_abi.lib user32.lib advapi32.lib shell32.lib shlwapi.lib ws2_32.lib userenv.lib bcrypt.lib ntdll.lib crypt32.lib secur32.lib ncrypt.lib
    if($LASTEXITCODE){throw 'DLL link failed'}
} finally {Pop-Location}
$licenses=Join-Path $output 'licenses'
& (Join-Path $root 'tools/write-license-notices.ps1') -OutputDirectory $licenses
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination $output
Copy-Item -LiteralPath (Join-Path $root 'THIRD_PARTY.md') -Destination $output
Write-Output "Built: $output\TvRockDTV.dll"
