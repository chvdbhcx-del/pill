#requires -Version 7
<#
.SYNOPSIS
  Writes PillPhone.json, the update manifest the Pill Phone app reads from the phone-latest release, for a built APK.
.DESCRIPTION
  versionName / versionCode / minSdk come from the APK itself (aapt2 from the Android SDK build-tools) and fall back to
  app/build.gradle.kts when aapt2 is not found. SHA-256 and size are computed from the file. Nothing is uploaded: the two
  `gh release upload` lines are printed for the owner to run.
.EXAMPLE
  pwsh android/tools/make-update-manifest.ps1 -Notes "Updates from inside the app."
#>
param(
    # The release APK; it must be called PillPhone.apk (the app downloads that asset name).
    [string]$Apk = (Join-Path $PSScriptRoot '..\dist\PillPhone.apk'),
    # Where to write the manifest (default: next to the APK).
    [string]$Out,
    # Short plain text shown in the update card, up to 600 characters.
    [string]$Notes = ''
)
$ErrorActionPreference = 'Stop'

$Apk = (Resolve-Path -LiteralPath $Apk).Path
if (-not $Out) { $Out = Join-Path (Split-Path $Apk) 'PillPhone.json' }
if ((Split-Path $Apk -Leaf) -ne 'PillPhone.apk') { Write-Warning 'The APK is not named PillPhone.apk: upload it under that name or the app will not find it.' }
if ($Notes.Length -gt 600) { throw 'Notes are longer than 600 characters.' }

$name = $null; $code = $null; $minSdk = $null; $source = 'APK (aapt2)'

# 1. the APK itself, through aapt2 of the newest installed build-tools
$sdk = @($env:ANDROID_HOME, $env:ANDROID_SDK_ROOT, (Join-Path $env:LOCALAPPDATA 'Android\Sdk')) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
$aapt2 = if ($sdk) {
    Get-ChildItem (Join-Path $sdk 'build-tools') -Directory -ErrorAction SilentlyContinue |
        Sort-Object { try { [version]$_.Name } catch { [version]'0.0' } } -Descending |
        ForEach-Object { Join-Path $_.FullName 'aapt2.exe' } | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if ($aapt2) {
    $badging = & $aapt2 dump badging $Apk 2>$null
    $pkg = $badging | Where-Object { $_ -like 'package:*' } | Select-Object -First 1
    if ($pkg -match "versionCode='(\d+)'") { $code = [long]$Matches[1] }
    if ($pkg -match "versionName='([^']+)'") { $name = $Matches[1] }
    $sdkLine = $badging | Where-Object { $_ -match '^(min)?[sS]dkVersion:' } | Select-Object -First 1
    if ($sdkLine -match "'(\d+)'") { $minSdk = [int]$Matches[1] }
}

# 2. fall back to the Gradle file
if (-not $name -or -not $code -or -not $minSdk) {
    $source = 'app/build.gradle.kts'
    $gradle = Get-Content -Raw (Join-Path $PSScriptRoot '..\app\build.gradle.kts')
    if ($gradle -match 'versionCode\s*=\s*(\d+)') { $code = [long]$Matches[1] }
    if ($gradle -match 'versionName\s*=\s*"([^"]+)"') { $name = $Matches[1] }
    if ($gradle -match 'minSdk\s*=\s*(\d+)') { $minSdk = [int]$Matches[1] }
}
if (-not $name -or -not $code -or -not $minSdk) { throw 'Could not read versionName / versionCode / minSdk.' }

$manifest = [ordered]@{
    versionName = $name
    versionCode = $code
    sha256      = (Get-FileHash -Algorithm SHA256 -LiteralPath $Apk).Hash.ToLowerInvariant()
    size        = (Get-Item -LiteralPath $Apk).Length
    minSdk      = $minSdk
    notes       = $Notes
}
# UTF-8 without BOM
[IO.File]::WriteAllText($Out, ($manifest | ConvertTo-Json) + "`n", [Text.UTF8Encoding]::new($false))

Write-Host "Wrote $Out (version $name, code $code, minSdk $minSdk, from $source)"
Write-Host ''
Write-Host 'Publish, in this order (the APK first, so the manifest never points at a file that is not there yet; versionCode must be higher than the published one):'
Write-Host "  gh release upload phone-latest `"$Apk`" --clobber --repo chvdbhcx-del/pill"
Write-Host "  gh release upload phone-latest `"$Out`" --clobber --repo chvdbhcx-del/pill"
