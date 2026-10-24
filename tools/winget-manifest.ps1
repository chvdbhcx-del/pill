<#
  Fills the winget manifest templates in packaging/winget/ for one released version.

    pwsh tools/winget-manifest.ps1 -Version 0.1.0

  Downloads Pill_<Version>_x64-setup.exe from the GitHub release v<Version> (only from
  https://github.com/chvdbhcx-del/pill/releases/), computes its SHA256 and writes copies of the manifests with the version,
  URL, hash and date filled in to packaging/winget/out/<Version>/. Nothing is submitted anywhere: see packaging/winget/README.md.
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [string]$Date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd')
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$src = Join-Path $root 'packaging\winget'
$out = Join-Path $src "out\$Version"

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3, got '$Version'" }
if ($Date -notmatch '^\d{4}-\d{2}-\d{2}$') { throw "Date must look like 2026-10-24, got '$Date'" }

$allowed = 'https://github.com/chvdbhcx-del/pill/releases/'
$url = "${allowed}download/v$Version/Pill_${Version}_x64-setup.exe"
if (-not $url.StartsWith($allowed)) { throw "Refusing to download from outside $allowed" }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("pill-winget-" + [guid]::NewGuid().ToString('N'))
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

New-Item -ItemType Directory -Force -Path $out | Out-Null
$utf8 = New-Object Text.UTF8Encoding($false)
foreach ($f in Get-ChildItem $src -Filter 'chvdbhcx-del.Pill*.yaml') {
    $text = (Get-Content $f.FullName -Raw).Replace('<VERSION>', $Version).Replace('<SHA256>', $sha).Replace('<DATE>', $Date)
    if ($text -match '<[A-Z0-9]+>') { throw "Unfilled placeholder left in $($f.Name): $($Matches[0])" }
    [IO.File]::WriteAllText((Join-Path $out $f.Name), $text, $utf8)
}
Write-Host "Manifests written to $out"
Write-Host "Check them with: winget validate --manifest `"$out`""
