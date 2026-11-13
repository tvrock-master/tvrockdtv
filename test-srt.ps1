param(
    [Parameter(Mandatory=$true)][string]$BonDriverDLL,
    [Parameter(Mandatory=$true)][string]$BonDriverInclude,
    [ValidateSet('x64','x86')][string]$Architecture='x64'
)
$ErrorActionPreference='Stop'
$BonDriverDLL=(Resolve-Path -LiteralPath $BonDriverDLL).Path
$BonDriverInclude=(Resolve-Path -LiteralPath $BonDriverInclude).Path
& "$PSScriptRoot/build.ps1" -Architecture $Architecture
$target=if($Architecture -eq 'x86'){'i686-pc-windows-msvc'}else{'x86_64-pc-windows-msvc'}
$output=Join-Path $PSScriptRoot "target/$target/release"
Push-Location $output
try {
    & cl /nologo /EHsc /std:c++17 /utf-8 /MD /DUNICODE /D_UNICODE "/I$BonDriverInclude" "$PSScriptRoot/tests/srt_interop.cpp" tvrockdtv.lib /Fe:srt_interop.exe /link ws2_32.lib userenv.lib bcrypt.lib ntdll.lib crypt32.lib secur32.lib ncrypt.lib
    if($LASTEXITCODE){throw 'SRT integration compile failed'}
    & ./srt_interop.exe $BonDriverDLL
    if($LASTEXITCODE){throw 'SRT integration failed'}
} finally {Pop-Location}
