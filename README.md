# Pill

A small "dynamic island" for the top of a Windows screen: a black capsule that shows what is playing, the clock, what your tools are doing and notifications, and grows into a panel with a clipboard history, notes, weather, an AI chat, quick controls and a command bar. Built with Tauri v2 (Rust) and plain HTML/CSS/JS.

> Pill is an independent, clean-room project. It is not affiliated with, endorsed by or derived from the code or artwork of any commercial app; it only follows the general idea of a notch-style widget.

*По-русски: небольшой «остров» в верхней части экрана Windows: музыка, часы, уведомления, статус ИИ-агентов; раскрывается в панель с историей буфера, заметками, погодой, ИИ-чатом и командной строкой.*

## Features

- Capsule with a liquid edge that reacts to music, notifications and recordings; pets/characters beside it
- Clipboard history (text, images, files) with search, pinning, paste-as-plain-text and a "wipe everything" button
- Notes, reminders and timer; weather; shelf for files; quick system controls; launcher and command bar (uses [Everything](https://www.voidtools.com/) for file search when installed)
- AI chat with local models (Ollama, LM Studio, llama.cpp, FreeLLMAPI) or a cloud provider
- Voice to text with any OpenAI-compatible Whisper endpoint, local or cloud
- Screen capture, OCR, colour picker and QR reader
- Status of coding agents (hooks for Claude Code)

## Privacy

Network features are **off by default**. Nothing is sent anywhere until you switch a feature on (weather, AI, voice, update check). The microphone is used only while you dictate. Clipboard history stays on your machine and skips password managers and secret-looking text by default.

## Install

Download `Pill_<version>_x64-setup.exe` from the [latest release](../../releases/latest) and run it. Updates are optional: *Settings → Updates → Check now* (or "Check on start"). Every update is signed and verified before it is installed.

## Build from source

Requirements: Windows 10/11, [Rust](https://rustup.rs/), Node.js 18+, WebView2 (included in Windows 11).

```powershell
npm install
npm run tauri dev                 # run with live front-end
npm run tauri build -- --bundles nsis
```

### Publishing a release (maintainers)

`tools/release.ps1` builds the installer, signs it with the updater key and uploads the installer, its signature and `latest.json` to a GitHub release:

```powershell
pwsh tools/release.ps1 -Notes "What is new"   # add -NoPublish to only build
```

The signing key is generated once (`npx tauri signer generate -w %USERPROFILE%\.tauri\pill-updater.key`); its public half is in `src-tauri/tauri.conf.json`. Keep the private key out of the repository and back it up: without it installed copies cannot update.

## License

[MIT](LICENSE)
