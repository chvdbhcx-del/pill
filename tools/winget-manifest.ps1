<#
  Fills the winget manifest templates in packaging/winget/ for one released version.

    pwsh tools/winget-manifest.ps1 -Version 0.9.0

  Kept for the documented command: it is tools/update-manifests.ps1 -Only winget (which also does the Scoop manifest).
  Output: packaging/winget/out/<Version>/. Nothing is submitted anywhere: see docs/DISTRIBUTION.md.
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [string]$Date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd')
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'update-manifests.ps1') -Version $Version -Only winget -Date $Date
