# Pill Phone

The Android companion of the Pill desktop app. It mirrors the phone's notifications to your PC (with reply, mark as read and dismiss from the PC) and moves files in both directions. Everything runs on your local network, no cloud and no account.

Contract with the PC side: `docs/phone-protocol.md` in the Pill repository (protocol v1). Screens in `docs/screenshots/` are rendered by the test suite, in dark and light.

## What it does

* Onboarding (3 animated pages), then a home screen with the connection drop: it breathes while connected, wobbles while connecting, goes grey when the PC is not found.
* Pairing: the link `pillphone://pair?h=<ip>&p=<port>&c=<code>&n=<pc name>` (opened from the PC's pairing page), the in-app QR scanner (own screen: CameraX plus a bundled ML Kit QR reader, no Google Play services, camera only inside the square window), or typing `host:port` and the 8-letter code.
* A foreground service owns the WebSocket (ping every 20 s, back-off 1..30 s, restart after boot, follows network changes). "PC not found on this network" comes with a "Scan again" action.
* Notification mirroring with a per-app filter (messengers on, system and ongoing off by default). Nothing is queued while the PC is away; on reconnect the active notifications are replayed.
* Files: the share sheet entry "Send to PC", an in-app picker, and "File from PC" notifications (optional auto-receive) saved to `Downloads/Pill`.
* Status (battery, charging, network, Wi-Fi name when allowed, Do Not Disturb), a permission checklist with one-tap fixes, unpair.
* English and Russian, light and dark following the system.

## How pairing works

1. On the PC: Settings > Phone > Phone link shows a QR code for `http://<pc-ip>:<port>/p?c=<code>`.
2. The phone opens that page (or the app scans the QR itself) and calls `POST /api/pair` with the one-time code.
3. The PC answers with a per-device bearer token. It is kept in `EncryptedSharedPreferences` (Android Keystore) and sent as `Authorization: Bearer` / `?token=` afterwards.
4. The PC can revoke the token any time (socket closed with code 4001); the app then shows "Pairing was removed".

Traffic is plain HTTP/WebSocket on the LAN. Pairing hosts that are not private addresses are rejected by the app.

## Build

Requirements: JDK 17, Android SDK with platform 35 and build-tools 35 (`sdkmanager "platforms;android-35" "build-tools;35.0.0"`).

```
./gradlew assembleDebug            # app/build/outputs/apk/debug
./gradlew assembleRelease          # signed if keys/keystore.properties exists
./gradlew testDebugUnitTest        # protocol, filter and screenshot tests
./gradlew :app:recordRoborazziDebug --tests "app.pill.phone.ScreenshotTest"   # re-render docs/screenshots
```

If the checkout path contains non-ASCII characters (the JVM test worker cannot load classes from such a folder on Windows), the build output goes to `~/.pill-phone-build/app` automatically and `android.overridePathCheck=true` is set.

## Signing

`keys/` is git-ignored. Create a key once and keep it (an app update must be signed with the same key):

```
keytool -genkeypair -keystore keys/release.keystore -alias pillphone -keyalg RSA -keysize 4096 -validity 10950
```

Write `keys/keystore.properties`:

```
storeFile=../keys/release.keystore
storePassword=...
keyAlias=pillphone
keyPassword=...
```

`./gradlew assembleRelease` picks it up. Copy `app/build/outputs/apk/release/app-release.apk` to `dist/PillPhone.apk` (also git-ignored); the PC's pairing page links to a release asset named `PillPhone.apk`.

## Notes

* Foreground service type: `dataSync` up to Android 13, `specialUse` from Android 14, because Android 15 stops `dataSync` services after 6 hours a day and the link has to stay up.
* The scanner only accepts the Pill pairing link or `/p?c=` URL; any other QR code gives a gentle shake and a hint. Camera permission has a rationale screen and a settings shortcut when blocked.
* The APK is built for arm64-v8a and armeabi-v7a only (the ML Kit model is native code per ABI).
* On Android 13+ a sideloaded app needs "Allow restricted settings" (App info, three dots) before the notification access switch can be turned on; the checklist says so.

## Notice

Proprietary software. All rights reserved. Not licensed for copying, modification or redistribution without written permission.
