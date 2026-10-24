<#
  Builds the Pill installer, signs it for the updater and (unless -NoPublish) publishes a GitHub release.

    pwsh tools/release.ps1                      build + sign + publish release v<version> from tauri.conf.json
    pwsh tools/release.ps1 -NoPublish           build + sign only (everything lands in src-tauri/target/release/bundle/nsis)
    pwsh tools/release.ps1 -Notes "What is new"  release notes shown in the Updates page

  Needs: node/npm, Rust, the NSIS toolchain Tauri downloads on its own, `gh` logged in (publishing only), and the updater signing key
  (default %USERPROFILE%\.tauri\pill-updater.key; generate once with: npx tauri signer generate -w <path>; the matching public key
  is in src-tauri/tauri.conf.json -> plugins.updater.pubkey). Never commit the private key: losing it means installed copies can no longer update.
  To ship a new version, bump "version" in src-tauri/tauri.conf.json first.
#>
param(
    [string]$Notes = '',
    [string]$Repo = 'chvdbhcx-del/pill',
    [string]$KeyPath = (Join-Path $env:USERPROFILE '.tauri\pill-updater.key'),
    [switch]$NoPublish
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$conf = Get-Content (Join-Path $root 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$ver = $conf.version
if (-not (Test-Path $KeyPath)) { throw "Signing key not found: $KeyPath (see the header of this script)" }
$env:TAURI_SIGNING_PRIVATE_KEY_PATH = $KeyPath
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''

Write-Host "Building Pill $ver ..."
npm run tauri build -- --bundles nsis
if ($LASTEXITCODE -ne 0) { throw 'tauri build failed' }

$dir = Join-Path $root 'src-tauri\target\release\bundle\nsis'
$exe = Join-Path $dir "Pill_${ver}_x64-setup.exe"
$sig = "$exe.sig"
if (-not (Test-Path $exe) -or -not (Test-Path $sig)) { throw "Missing $exe or its .sig (is bundle.createUpdaterArtifacts on and the key readable?)" }

$latest = [ordered]@{
    version  = $ver
    notes    = $Notes
    pub_date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = [ordered]@{
        'windows-x86_64' = [ordered]@{
            signature = (Get-Content $sig -Raw).Trim()
            url       = "https://github.com/$Repo/releases/download/v$ver/Pill_${ver}_x64-setup.exe"
        }
    }
}
$json = Join-Path $dir 'latest.json'
[IO.File]::WriteAllText($json, ($latest | ConvertTo-Json -Depth 5), (New-Object Text.UTF8Encoding($false)))
Write-Host "Installer: $exe"
Write-Host "Manifest:  $json"

if ($NoPublish) { Write-Host 'Not published (-NoPublish).'; return }

$tag = "v$ver"
gh release view $tag --repo $Repo *> $null
if ($LASTEXITCODE -eq 0) {
    Write-Host "Release $tag exists: replacing its files"
    gh release upload $tag $exe $sig $json --repo $Repo --clobber
} else {
    $body = if ($Notes) { $Notes } else { "Pill $ver" }
    gh release create $tag $exe $sig $json --repo $Repo --title "Pill $ver" --notes $body
}
if ($LASTEXITCODE -ne 0) { throw 'gh release failed' }
Write-Host "Published: https://github.com/$Repo/releases/tag/$tag"
