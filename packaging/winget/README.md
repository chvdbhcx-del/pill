# winget package

Manifest templates (schema 1.6.0) for the package `chvdbhcx-del.Pill`: the Tauri NSIS installer (`nullsoft`, silent switch `/S`, per-user scope, because `tauri.conf.json` does not set `bundle.windows.nsis.installMode`, whose default is `currentUser`).

The files here are templates with the placeholders `<VERSION>`, `<SHA256>` and `<DATE>`. Do not submit them as they are.

## Steps (done by the maintainer, by hand)

1. Publish the release first (`pwsh tools/release.ps1`, or push a `vX.Y.Z` tag so `ci-templates/release.yml` does it). The installer must be downloadable at
   `https://github.com/chvdbhcx-del/pill/releases/download/v<version>/Pill_<version>_x64-setup.exe`.
2. Fill the templates:
   ```powershell
   pwsh tools/winget-manifest.ps1 -Version 0.1.0
   ```
   This downloads that one release asset (only from `github.com/chvdbhcx-del/pill/releases`), hashes it and writes the finished manifests to `packaging/winget/out/<version>/` (git-ignored).
3. Validate and test locally:
   ```powershell
   winget validate --manifest packaging\winget\out\0.1.0
   winget settings --enable LocalManifestFiles        # once, needs an elevated terminal
   winget install --manifest packaging\winget\out\0.1.0
   ```
4. Submit to [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs): fork it, copy the three files from `out/<version>/` to
   `manifests/c/chvdbhcx-del/Pill/<version>/`, and open a pull request. The pull-request template and automated checks (validation, installing the package, a malware scan) run there; fix whatever the bot reports. Current rules: <https://github.com/microsoft/winget-pkgs/blob/master/CONTRIBUTING.md>. The [Windows Package Manager Manifest Creator](https://github.com/microsoft/winget-create) (`wingetcreate update chvdbhcx-del.Pill --version <v> --urls <url> --submit`) can do the update PR for later versions.

## Notes

- The installer is not code-signed (see `docs/SIGNING.md`), so the winget pipeline or SmartScreen may flag it; unsigned packages are not forbidden by the manifest schema, but check the pull-request feedback.
- Every new release needs a new hash, so repeat steps 1 to 4 per version. The app updates itself in place (Settings, Updates); winget only needs the new manifest if you want `winget upgrade` to see it.
- Nothing in this repository submits anything to winget automatically.
