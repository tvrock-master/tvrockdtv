#Requires -Version 5.1
param([string]$Archive, [string]$Destination)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Expand-Archive handles the Unix symlink entries in SRT's ZIP on Windows.
Expand-Archive -LiteralPath $Archive -DestinationPath $Destination -Force
