$ErrorActionPreference = 'Stop'

foreach ($name in @('CI_API_V4_URL', 'CI_PROJECT_ID', 'CI_COMMIT_TAG', 'CI_JOB_ID', 'CI_JOB_TOKEN')) {
    if (-not [Environment]::GetEnvironmentVariable($name)) {
        throw "Required GitLab CI variable is missing: $name"
    }
}

$projectApi = "$env:CI_API_V4_URL/projects/$env:CI_PROJECT_ID"
$tag = [Uri]::EscapeDataString($env:CI_COMMIT_TAG)
$releaseUri = "$projectApi/releases/$tag"
$headers = @{ 'JOB-TOKEN' = $env:CI_JOB_TOKEN }

# Only a missing release should trigger creation; authentication/server errors must fail.
$release = $null
try {
    $release = Invoke-RestMethod -Uri $releaseUri -Headers $headers -Method Get
} catch {
    if (-not $_.Exception.Response -or [int]$_.Exception.Response.StatusCode -ne 404) { throw }
}

$artifactRoot = Join-Path $PSScriptRoot 'tvrockdtv'
foreach ($architecture in @('x64', 'x86')) {
    if (-not (Test-Path -LiteralPath (Join-Path $artifactRoot "$architecture/TvRockDTV.dll") -PathType Leaf)) {
        throw "Release DLL is missing: $architecture"
    }
}
$archive = Join-Path $env:TEMP "tvrockdtv-$env:CI_JOB_ID.zip"
Compress-Archive -Path $artifactRoot -DestinationPath $archive -Force

# Each job gets its own package version, including retries. Tags can contain slashes.
$packageUri = "$projectApi/packages/generic/tvrockdtv/$env:CI_JOB_ID/tvrockdtv.zip"
Invoke-RestMethod -Uri $packageUri -Headers $headers -Method Put -InFile $archive -ContentType 'application/zip' | Out-Null

$link = @{
    name = 'tvrockdtv.zip'
    url = $packageUri
    direct_asset_path = '/tvrockdtv.zip'
    link_type = 'package'
}
if ($null -eq $release) {
    $body = @{
        tag_name = $env:CI_COMMIT_TAG
        name = $env:CI_COMMIT_TAG
        description = 'Windows x64/x86 DLLs. Download tvrockdtv.zip from Assets.'
        assets = @{ links = @($link) }
    } | ConvertTo-Json -Depth 6
    Invoke-RestMethod -Uri "$projectApi/releases" -Headers $headers -Method Post -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) | Out-Null
} else {
    # Preserve existing release notes and unrelated assets.
    $existingLink = $release.assets.links | Where-Object { $_.name -eq $link.name } | Select-Object -First 1
    $linkUri = "$releaseUri/assets/links"
    $method = 'Post'
    if ($existingLink) {
        $linkUri += "/$($existingLink.id)"
        $method = 'Put'
    }
    $body = $link | ConvertTo-Json
    Invoke-RestMethod -Uri $linkUri -Headers $headers -Method $method -ContentType 'application/json; charset=utf-8' -Body ([Text.Encoding]::UTF8.GetBytes($body)) | Out-Null
}

Write-Output "Published GitLab Release: $env:CI_COMMIT_TAG (tvrockdtv.zip)"
