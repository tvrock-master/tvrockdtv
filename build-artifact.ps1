$ErrorActionPreference = 'Stop'

& (Join-Path $PSScriptRoot 'build.ps1') -Architecture x64
& (Join-Path $PSScriptRoot 'build.ps1') -Architecture x86

$artifactRoot = Join-Path $PSScriptRoot 'tvrockdtv'
New-Item -ItemType Directory -Force -Path (Join-Path $artifactRoot 'x64'), (Join-Path $artifactRoot 'x86') | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'target\x86_64-pc-windows-msvc\release\TvRockDTV.dll') -Destination (Join-Path $artifactRoot 'x64\TvRockDTV.dll') -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'target\i686-pc-windows-msvc\release\TvRockDTV.dll') -Destination (Join-Path $artifactRoot 'x86\TvRockDTV.dll') -Force

foreach($entry in @(@('x64','x86_64-pc-windows-msvc'),@('x86','i686-pc-windows-msvc'))) {
    $source=Join-Path $PSScriptRoot "target/$($entry[1])/release"
    $destination=Join-Path $artifactRoot $entry[0]
    Copy-Item -LiteralPath (Join-Path $source 'LICENSE') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $source 'THIRD_PARTY.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $source 'licenses') -Destination $destination -Recurse -Force
}
Write-Output "Artifacts: $artifactRoot"
