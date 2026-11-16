# Pill Phone updates (over the air)

Pill Phone 1.1.0 and later look for newer builds on the public rolling release `phone-latest` of `chvdbhcx-del/pill`
and install them from inside the app. There is no change to the protocol with the PC.

## What the app does

* Fetches `PillPhone.json` from `https://github.com/chvdbhcx-del/pill/releases/download/phone-latest/PillPhone.json`
  (plain HTTPS GET, no account, no identifiers, `User-Agent: PillPhone/<versionName>`), on app start at most every 12 hours
  (Settings > Updates > "Check for updates automatically", on by default) and on "Check now". Nothing runs while the app is closed.
* Compares `versionCode` with the installed one. Only a strictly higher code is offered, and only if `minSdk` fits the phone.
* On the user's tap downloads `PillPhone.apk` from the same release into `cacheDir/updates/`, checking the size and the SHA-256
  of the manifest while streaming (a mismatch deletes the file), then checks that the file is this app with the installed
  certificate, and hands it to the system installer (`PackageInstaller` session). Android may show its own confirmation; the app never
  bypasses it. Android itself refuses an APK signed with another key, which is the real trust anchor.
* Only HTTPS to `github.com`, `objects.githubusercontent.com` and `release-assets.githubusercontent.com` (where GitHub redirects
  release downloads today) is accepted; every redirect hop is checked.
* The first time, the user has to allow "Install unknown apps" for Pill Phone (the app opens that Android page and carries on when the user is back).

## The manifest `PillPhone.json`

```json
{ "versionName": "1.1.0", "versionCode": 2, "sha256": "<hex of the apk>", "size": 11777637, "minSdk": 26, "notes": "short plain text" }
```

`versionName`, `versionCode` (integer, at least 1), `sha256` (64 hex) and `size` are required; `minSdk` and `notes` (up to 600 characters) are optional;
unknown fields are ignored; anything malformed makes the whole manifest unusable (the app shows "couldn't be read").

## Releasing a new phone build

1. Raise `versionCode` (strictly, every published APK) and `versionName` in `android/app/build.gradle.kts`.
2. Build: `cd android && ./gradlew assembleRelease` (needs `android/keys/keystore.properties`, never committed, always the same key).
3. Copy `app-release.apk` (under `~/.pill-phone-build/app/outputs/apk/release/` for a checkout with a non-ASCII path, else `app/build/outputs/apk/release/`) to `android/dist/PillPhone.apk`.
4. `pwsh android/tools/make-update-manifest.ps1 -Notes "What changed, one or two sentences."` writes `android/dist/PillPhone.json`
   (version data read from the APK with `aapt2`, SHA-256 and size from the file) and prints the upload commands.
5. Check the signing certificate against the published APK: `apksigner verify --print-certs` on both, the SHA-256 digests must be equal.
6. Upload, the APK first, then the manifest:
   `gh release upload phone-latest android/dist/PillPhone.apk --clobber --repo chvdbhcx-del/pill`
   `gh release upload phone-latest android/dist/PillPhone.json --clobber --repo chvdbhcx-del/pill`

Phones that still run 1.0.0 cannot update themselves: install 1.1.0 once by hand (the pairing page's "Download the app" button),
after that updates come from inside the app.
