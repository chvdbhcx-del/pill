<#
  Fills the winget and Scoop manifest templates for one released version of Pill.

    pwsh tools/update-manifests.ps1                    the latest GitHub release
    pwsh tools/update-manifests.ps1 -Version 0.9.0     a given release
    pwsh tools/update-manifests.ps1 -Only scoop        just one of the two (winget | scoop)

  Downloads Pill_<Version>_x64-setup.exe from the GitHub release v<Version> (only from https://github.com/chvdbhcx-del/pill/releases/),
  computes its SHA256 ONCE and writes copies of the templates with <VERSION>, <SHA256> and <DATE> filled in:
      packaging/winget/out/<Version>/*.yaml      (from packaging/winget/chvdbhcx-del.Pill*.yaml)
      packaging/scoop/out/<Version>/pill.json    (from packaging/scoop/pill.json)
  Nothing is submitted or published anywhere: see docs/DISTRIBUTION.md. No token is needed (public release, unauthenticated GETs).
#>
param(
    [string]$Version = '',
    [ValidateSet('all', 'winget', 'scoop')][string]$Only = 'all',
    [string]$Date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$repo = 'chvdbhcx-del/pill'
$allowed = "https://github.com/$repo/releases/"

if (-not $Version) {
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'pill-update-manifests' }
    $Version = ([string]$rel.tag_name).TrimStart('v')
    Write-Host "Latest release: v$Version"
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3, got '$Version'" }
if ($Date -notmatch '^\d{4}-\d{2}-\d{2}$') { throw "Date must look like 2026-10-24, got '$Date'" }

$url = "${allowed}download/v$Version/Pill_${Version}_x64-setup.exe"
if (-not $url.StartsWith($allowed)) { throw "Refusing to download from outside $allowed" }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("pill-manifests-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    $exe = Join-Path $tmp "Pill_${Version}_x64-setup.exe"
    Write-Host "Downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $exe -UseBasicParsing
    $sha = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToUpperInvariant()
    Write-Host "SHA256 $sha"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

$utf8 = New-Object Text.UTF8Encoding($false)
function Write-Filled([string]$srcFile, [string]$outDir, [string]$hash) {
    $text = (Get-Content $srcFile -Raw).Replace('<VERSION>', $Version).Replace('<SHA256>', $hash).Replace('<DATE>', $Date)
    if ($text -match '<[A-Z0-9]+>') { throw "Unfilled placeholder left in $(Split-Path -Leaf $srcFile): $($Matches[0])" }
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    [IO.File]::WriteAllText((Join-Path $outDir (Split-Path -Leaf $srcFile)), $text, $utf8)
}

if ($Only -in 'all', 'winget') {
    $out = Join-Path $root "packaging\winget\out\$Version"
    foreach ($f in Get-ChildItem (Join-Path $root 'packaging\winget') -Filter 'chvdbhcx-del.Pill*.yaml') { Write-Filled $f.FullName $out $sha }
    Write-Host "winget manifests: $out"
    Write-Host "  check: winget validate --manifest `"$out`""
}
if ($Only -in 'all', 'scoop') {
    $out = Join-Path $root "packaging\scoop\out\$Version"
    Write-Filled (Join-Path $root 'packaging\scoop\pill.json') $out $sha.ToLowerInvariant()   # Scoop compares lower-case hashes
    Write-Host "scoop manifest:   $out\pill.json"
    Write-Host "  check: scoop install $out\pill.json   (in a VM or Windows Sandbox: the install switches are untested)"
}
