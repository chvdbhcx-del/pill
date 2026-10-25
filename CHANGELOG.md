# Changelog

All notable changes to Pill. The `## [x.y.z]` sections are used as release notes by `ci-templates/release.yml`.

## [0.1.5] - 2026-10-25

### Added
- Text tab: the AI mode is connected. Translate, fix errors, shorten, summarize, formal / friendly tone, bullet list, explain, continue, or type your own request; it works on the selection (or the whole text) through the chat endpoint from Settings, and Ctrl+Z restores the original.

### Changed
- Typography: the whole UI uses the bundled Nunito typeface (SIL OFL).
- New installer: artwork in the welcome and header areas, English and Russian pages chosen from the system language.

### Fixed
- The command bar's key hint shows the real shortcut (`Alt+Shift+Space`) instead of dropping `Shift`.

## [0.1.4] - 2026-10-25

### Added
- The command bar shows real icons: an application's own icon, the type icon of a file, the folder icon of a folder.

### Changed
- The command bar opens under the island instead of covering it.

## [0.1.3] - 2026-10-25

### Changed
- "Check on start" is on by default: about 20 seconds after launch Pill checks GitHub for a newer release and shows a card (installing is still your click). Turn it off in Settings > Updates.

## [0.1.2] - 2026-10-25

### Changed
- The system monitor is on by default (Settings > Live events > System monitor turns it off).
- The island's coloured edge has a soft glow, so it reads on light and grey wallpapers.

### Fixed
- Tail and legs characters were never drawn (an SVG overlay of zero size is not rendered); they are now drawn as smooth shapes and match the reference: a tail that hangs down in a smooth arc, legs standing right under the eyes.
- The legs' eye slot now widens the island (the content used to be pushed against the right edge).
- The Claude 8-bit pet has three real sizes (14 / 21 / 28 px) at any island size, and the settings preview follows the size.

## [0.1.1] - 2026-10-24

### Added
- First-run "AI setup" page: local Whisper (whisper.cpp, hash-checked download) or Ollama models, or a cloud service.
- Push-to-talk: hold the voice hotkey while you speak (Settings > Voice to text > hotkey mode).
- AI actions on clipboard items: translate, shorten, fix spelling, explain simply (uses your configured chat model).
- System monitor: CPU, RAM and GPU load and temperature on the Controls tab, plus an overheating alert chip (Settings > Live events).
- Settings backup: export/import without secrets or machine-specific paths; Everything starts by itself for file search.
- GitHub Actions: CI (`cargo check`/`cargo test`, `node --check` of the front-end) and a tag-triggered release workflow.
- winget manifest templates (`packaging/winget`) and `tools/winget-manifest.ps1`.
- `docs/SIGNING.md` on code signing options.
- `tools/dev`: live-check scripts (CDP evaluate, CPU profile, transition audit, sideways-jump detector).

### Fixed
- The island no longer jumps sideways for a frame after collapsing (the window keeps one width), and the collapse spring no longer undershoots into a clipped sliver.
- The clipboard trash button now also clears the system clipboard and the Win+V history.
- Dictation survives a microphone that is reconfigured mid-recording (reopens it); each take is logged to `%APPDATA%\pill\voice.log` (durations only).

## [0.1.0] - 2026-10-24

First public release.

### Added
- Capsule with a liquid edge, pets and characters; music, clock, notifications and coding-agent status.
- Expanded panel: clipboard history, notes and reminders, weather, shelf, quick controls, launcher and command bar.
- AI chat (local models or a cloud provider), voice to text with an OpenAI-compatible Whisper endpoint, screen capture, OCR, colour picker, QR reader.
- Opt-in updater with signed updates; all network features off by default.
