param([ValidateSet('x64','x86')][string]$Architecture='x64')
$ErrorActionPreference='Stop'
& "$PSScriptRoot\build.ps1" -Architecture $Architecture
if($LASTEXITCODE){throw 'Build failed'}
$target=if($Architecture -eq 'x86'){'i686-pc-windows-msvc'}else{'x86_64-pc-windows-msvc'}
& cargo test --manifest-path "$PSScriptRoot\Cargo.toml" --target $target
if($LASTEXITCODE){throw 'Rust regression tests failed'}
$output=Join-Path $PSScriptRoot "target\$target\release"
Push-Location $output
try {
    & cl /nologo /MD /EHsc /std:c++17 /utf-8 /DUNICODE /D_UNICODE "/I$PSScriptRoot\compat" "$PSScriptRoot\tests\arib_regression.cpp" "$PSScriptRoot\compat\AribString.cpp" tvrockdtv.lib /Fe:arib_regression.exe /link ws2_32.lib userenv.lib bcrypt.lib ntdll.lib crypt32.lib secur32.lib ncrypt.lib
    if($LASTEXITCODE){throw 'ARIB test compile failed'}
    & .\arib_regression.exe
    if($LASTEXITCODE){throw 'Original ARIB regression failed'}
    & cl /nologo /MD /EHsc /std:c++17 /utf-8 /DUNICODE /D_UNICODE "/I$PSScriptRoot\compat" "$PSScriptRoot\tests\integration.cpp" tvrockdtv.lib /Fe:integration.exe /link user32.lib advapi32.lib shell32.lib shlwapi.lib ws2_32.lib userenv.lib bcrypt.lib ntdll.lib crypt32.lib secur32.lib ncrypt.lib
    if($LASTEXITCODE){throw 'Integration test compile failed'}
    & .\integration.exe
    if($LASTEXITCODE){throw 'Integration test failed'}
    $testOutput=Join-Path $output 'integrated-dll-smoke'
    New-Item -ItemType Directory -Force -Path $testOutput | Out-Null
    Copy-Item -LiteralPath (Join-Path $output 'TvRockDTV.dll') -Destination $testOutput
    Push-Location $testOutput
    try {
        & cl /nologo /MD /EHsc /utf-8 /DUNICODE /D_UNICODE "/I$PSScriptRoot\compat" "$PSScriptRoot\tests\dll_load.cpp" /Fe:dll_load.exe /link ws2_32.lib
        if($LASTEXITCODE){throw 'DLL loader compile failed'}
        & cargo build --manifest-path "$PSScriptRoot\Cargo.toml" --target $target --release --features test-harness --bin protocol_harness
        if($LASTEXITCODE){throw 'Protocol test harness build failed'}
        & "$output\protocol_harness.exe" (Join-Path $testOutput 'dll_load.exe')
        if($LASTEXITCODE){throw 'DLL network/lifecycle test failed'}
        & "$output\protocol_harness.exe" (Join-Path $testOutput 'dll_load.exe') sibling
        if($LASTEXITCODE){throw 'DLL sibling headers/lifecycle test failed'}
        & "$output\protocol_harness.exe" (Join-Path $testOutput 'dll_load.exe') self-delete
        if($LASTEXITCODE){throw 'Reentrant callback disposal test failed'}
        & "$output\protocol_harness.exe" (Join-Path $testOutput 'dll_load.exe') self-delete sibling
        if($LASTEXITCODE){throw 'Sibling PowerOff/Capture callback test failed'}
        & dumpbin /nologo /exports TvRockDTV.dll
    } finally {Pop-Location}
} finally {Pop-Location}
