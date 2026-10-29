# Changelog

All notable changes to Pill. The `## [x.y.z]` sections are used as release notes by `ci-templates/release.yml`.

## [0.5.2] - 2026-10-29

### Fixed
- Legs' eyes now stand on the same line as the text of the folded island (they sat about 1-3 px lower than the time, the pet and the icon): the eyes' height is taken from the middle of the live row's first text.
- The "Eyes" pet drew its face lower than the island's text (about 3 px); the artwork now sits on the text line, both in the calm state and with the happy arcs.
- The command bar search works while an uninstall runs. Typing (any letter, Backspace, Delete) while the uninstall panel is up now steps the panel aside and searches for what you typed; before, the panel took the keyboard and nothing could be searched during the leftover scan, the wait for the uninstaller or the deletion. The job itself (waiting, scanning, deleting, the administrator step) always ran in the background and keeps running; Esc inside the panel only closes the view, "Stop searching" / "Stop waiting" / Close stop or drop the job.
- A status strip under the search results shows the running uninstall ("qBittorrent: searching for leftovers, found 812" with a thin progress bar and a count that glides). When the step finishes it turns into "leftovers to review" with a small pop; clicking it, Ctrl+U or `u <name>` bring the panel back exactly where it was (checklist and ticks kept). If the bar is closed when a step finishes, the island shows a card ("Uninstall of qBittorrent: leftovers to review") with an Open button instead of the bar jumping open over what you are doing (quiet hours apply). The panel's logic is a small pure state machine (`src/uninstall-state.js`, `node src/uninstall-state.test.mjs`).

## [0.5.1] - 2026-10-29

### Fixed
- Folded island spacing: the pet, the weather icon, the time and the legs' eyes now stand at even distances. When the pet is on the left the weather icon moves to the right side (and the other way round), the eyes stand in their side column next to the time when it is empty, an empty column next to the pet collapses, and the clock row is measured again once the typeface and the character are in (the width could stay stale).

## [0.5.0] - 2026-10-29

### Fixed
- Safety hardening of the uninstaller (from an independent review). Folders: an install record can no longer make a generic or system folder deletable (`ProgramData\Microsoft`, `Common Files\Services`, Windows Defender, WindowsApps, `Microsoft*` and similar are denied outright, also after resolving links); a symlink or junction anywhere on a path, including the install folder and its parents, is refused; only fixed local drives are accepted; Windows, Program Files and the profile folders come from the system, never from environment variables. Recycle Bin: files are recycled with a guarantee (a mapped drive, a path over 259 characters, a drive set to "remove immediately" or an item bigger than the bin end in an error and nothing is deleted). Elevation: Pill keeps its own exe locked from the start and re-checks its SHA-256 and path before the Windows permission prompt, starts system tools by absolute path, no longer sends GUIDs to the elevated helper (it finds them itself), shows its own OK / Cancel dialog with the exact items before removing anything, ignores strangers on its local socket and notices a helper that stops. Matching: a GUID alone is only Probable for COM / Installer keys, everything at the top of a drive is Probable, a registry key shared with other versions is Probable, an install folder holding several programs is Probable. Deleting: tasks and services go first and a service that is still running is not reported deleted; registry backups are complete or the item is not deleted (no partial backups, line breaks and quotes are written exactly, the file is read back); a panic in a worker thread no longer leaves the panel spinning.
- The app windows now run under a strict Content-Security-Policy (no inline scripts, no `eval`, no network from the pages, `connect-src` limited to Tauri's own IPC); `node src/csp.test.mjs` keeps the front end compatible with it.

### Added
- Deep clean after an uninstall: the leftover search now covers the whole PC, not only the usual folders. It looks in every fixed drive's top levels, every readable user profile (AppData, LocalLow, Documents, Saved Games, Start Menu, Desktop, taskbar pins, Temp, Downloads installers), ProgramData, Program Files and Common Files, and in the registry (software keys, file associations and shell handlers, COM classes, App Paths, autostart entries, history lists, Windows Installer records, services, scheduled tasks, firewall rules). Every item shows why it matched and is either Certain (points into the install folder, carries the product GUID, or is named exactly like the app: pre-ticked) or Probable (the app's full name as words, or a shared publisher folder: unticked), with counts and sizes per group.
- Leftovers that need administrator rights (HKLM keys, services, scheduled tasks, firewall rules, Program Files, other users' profiles) can be removed with a second button, "Remove system leftovers (administrator)": Pill restarts itself elevated through the normal Windows permission prompt, re-derives the list itself from its own scan, and deletes only what you ticked. "Search more as administrator" shows what only an elevated scan can see. Registry items are saved to a `.reg` backup first (importable back); "Open backups folder" is in the summary.

### Changed
- Animations for the newer features. Command bar: the selection highlight glides between rows, row buttons pop in and press down, results that disappear fade out, the right-click menu opens from the cursor with its items in a stagger, pinning pops the pin icon, flashes the row and slides the message in and out. Island: the launcher strip slides aside for a newly pinned icon, the Agents tab slides rows when the order changes and pulses the dot of working sessions, the clock row fades and pops when the weather icon, temperature, date or battery appear or go. Settings: pages cross-fade, the sidebar indicator glides, search results stagger in and the target row pulses, quiet-hours day chips pop, the first-run wizard slides between steps with progress dots, status messages slide in. Reduce motion shows every end state at once.
- The uninstall panel animates every step: steps slide in and out, rows drop in one by one, groups expand and collapse, ticks draw their check, numbers glide, removed rows slide out as they are deleted, a failure shakes once, and the elevated step has its own progress animation. With Reduce motion everything appears at once.
- A name that only contains the app's name as words (for example `Foo Editor Data` for Foo Editor) is now offered as Probable instead of being ignored; it is never pre-ticked.

## [0.4.3] - 2026-10-29

### Added
- Command bar: an app row has a trash button and "Uninstall..." in its right-click menu; it puts `u <app name>` in the box, so the uninstall row (with its confirm step) shows right there.

### Changed
- Folded island: the weather icon has its own switch ("Weather icon"); "Temperature" now hides only the number, no longer the icon too.

## [0.4.2] - 2026-10-29

### Fixed
- Settings: a row whose switch has long labels (for example "Hotkey mode" on Voice to text) no longer squeezes its title and description into a narrow strip; the switch goes under the text.

## [0.4.1] - 2026-10-29

### Changed
- The pin button in a command-bar row (and the right-click menu and Ctrl+P) now also pins Microsoft Store apps to the island; they open and show their own icon like any other link.

## [0.4.0] - 2026-10-29

### Added
- Pin an app, file or folder to the island from the command bar: the pin button on the row, the right-click menu or Ctrl+P. It goes to the launcher strip (module "Launcher"); Microsoft Store apps cannot be pinned yet.
- After an update the island shows a "Pill <version>" card with that version's notes (taken from the changelog built into the app, no network) and an "All changes" button that opens Settings > Updates. Never on a fresh install; Settings > Updates has a switch for it.
- Settings > Updates > Report a problem: creates `Pill-report-<date>` in Downloads (versions, enabled modules, the settings export without secrets, the end of the voice log) and opens it. Nothing is sent anywhere.
- The "Agent finished" card has "Folder" (opens the session's folder in Explorer) and "VS Code" (only when `code` is installed) next to "Path". The app looks the folder up by session, and only an existing local directory is opened.
- `pill.exe --open agents` (or `--open settings`) started a second time is forwarded to the running Pill, which opens the Agents tab (or Settings) and the second process exits; Pill is now single-instance. Other arguments are ignored.
- Uninstall apps completely from the command bar: type `u`, `del`, `uninstall` or `удалить` and a name (switch: Settings > Modules > Complete app uninstall). Pill asks to confirm, runs the app's own uninstaller (its own window and Windows permission prompt; Pill never closes it), then scans for leftovers with strict exact-name matching and shows a checklist: the install folder, folders and Start Menu / Desktop shortcuts named like the app, HKCU keys and autostart entries. Files go to the Recycle Bin, registry keys are exported to a `.reg` backup under `%LOCALAPPDATA%\Pill\uninstall-backups` first, and a log is written beside it. Microsoft Store apps of the current user can be removed too. Anything in HKLM, services and scheduled tasks is only listed for you to remove by hand. System components, updates, runtimes and drivers stay hidden unless the query includes `all`.
- First-run wizard in Settings for a brand-new install only (language, modules, hotkeys, a pointer to AI setup; nothing downloaded or turned on, Skip always visible). Existing installs never see it; General has "Run again".
- Settings search: a field above the sidebar (Ctrl+F) finds pages and rows by their titles and descriptions in the current language; arrows and Enter jump to the page and highlight the row.
- Quiet hours (Settings > Behavior): from/to time (may cross midnight) and days of the week. While quiet, event cards, agent flashes and live-event pulses are not shown or queued; your timers, reminders and anything urgent still ring. Windows Focus Assist is not read.

### Changed
- Settings pages are regrouped: General (language, autostart, backup), Appearance (the island's looks, folded-island content, motion, frame rate), Behavior (auto-hide, pass-through, screenshots) and a new File search page (Everything). "Pill during music" moved to Sound. Long descriptions are shorter.

### Fixed
- Event cards (for example "Agent finished"): while a card is on screen, and for a moment after it closes, resting the cursor on the island no longer opens it, so pressing Dismiss does not pop the island open.

## [0.3.7] - 2026-10-28

### Changed
- The folder button on the "Agent finished" card is shortened to "Path" ("Путь").
- Medeo Sans (SIL OFL, Latin + Cyrillic subset) is the UI font in the island, the command bar, the capture overlay and Settings; it replaces Inter and the Segoe-based face.

### Added
- Settings > General can download the official Everything installer and es.exe from voidtools.com (size and SHA-256 checked, only on your click); the installer starts on a second click, with Windows' own permission prompt, and the command bar finds the downloaded es.exe.

## [0.3.6] - 2026-10-28

### Changed
- The "Agent finished" card and the short "Done" flash name the project of the session that finished, and the folder button reads "Copy folder path" in Russian too.
- Accessibility: every setting row names its control for screen readers, toggles expose a switch role, icon-only buttons have names, and keyboard focus is visible everywhere. Settings follows the island's Reduce motion switch, not only the system setting.

### Internal
- A CI self-check fails when the front end calls a command that is not registered in Rust.
- The calculator and unit conversion live in `command/calc.rs`.

## [0.3.5] - 2026-10-27

### Added
- Agents tab in the expanded island: every tracked Claude Code session with its project, state and time in that state, waiting ones first. The pill says which project needs your input ("2 need your input · project" when several wait).
- Settings > Modules: named profiles. Save the current module and live-event switches under a name and apply them with one click (up to 8; network opt-ins, keys and paths are never part of a profile).

### Changed
- Steam manifests are re-read only when the file changed; the local database uses `synchronous=NORMAL` with WAL.
- Settings message lines, hidden elements and long paths are laid out consistently.

### Fixed
- The AI setup page opened slowly when a local server port hung instead of refusing; ports that do not accept within 300 ms are treated as off.
- Remaining poison-panicking mutex locks in the system and updater code.
- "Needs your input" no longer stays on the island after you answered: while a session waits, the island watches the modification time of that session's transcript (its content is never read) and goes back to "working" once it changes. The status helper now also records the transcript path.
- Settings > Updates: the result line ("You are up to date") sits inside the card with the same inset and text size as the rows, and a failed install is shown in red.

## [0.3.4] - 2026-10-27

### Fixed
- Command bar no longer closes right after it opens (the focus hand-over blur is ignored for 400 ms) and no longer flashes the previous frame before its pop animation.
- Smoother side flares of the collapsed island: they are drawn from an anti-aliased vector mask instead of a hard gradient edge, so the curve no longer stair-steps on light wallpapers.

## [0.3.3] - 2026-10-27

### Added
- Notifications tab: a history of the toasts of other apps (cap 200 entries / 14 days; OCR and QR content is never stored; mail previews off by default). Switches in Settings: module and "keep history".
- Shelf is a first-class feature: dragging files towards the island shows a drop card, the island shows a file-count badge, the same files can be dropped again, dragging away cancels.
- Steam: the percent of a big download is read from the Steam content log when the manifest does not move yet; the chip is re-sent every 10 s so a restarted island picks it up.

### Changed
- Command bar is faster: one file-search process per query with a result cap, timeout and cancel, cached settings, notes index, app list and icons. Files are not searched for fewer than 2 characters unless the `f ` filter is used.
- Home tab: the volume slider eases to its target, the wheel steps 2 %, the arrows 1 %, a drag writes every percent, and an outside change moves the slider.
- Liquid edge of the command bar and the closer eyes pet.

### Fixed
- The command bar now takes the OS foreground and the WebView focus (with a retry), so the keyboard layout hotkeys switch the layout inside the search field.
- Updates: the code comment said the check was off by default; it is on by default and only checks, installing is always your click.
## [0.3.2] - 2026-10-26

### Changed
- Lower idle cost: GPU stats come from one long-lived `nvidia-smi` process (stopped when nobody needs them) instead of a new process every 2 s; the cursor loop backs off while the pointer rests on the folded island; the Steam, notification, OBS, window-watch and audio pollers wake far less often (shared condvar naps, 250 ms / 1 s / 16 ms); the running-timer chip is edited in place once a second instead of being re-rendered; tickers of the hidden island are paused and resume on show; fewer per-frame allocations, a size-limited icon cache in the command bar and an unused refresh-rate probe removed.

## [0.3.1] - 2026-10-26

### Added
- Command bar: the selected app / file / folder row has buttons, and a right-click menu of its own: open, show in folder, copy the path (Ctrl+Shift+C), run as administrator (Ctrl+Shift+Enter). The WebView's default context menu (Back / Reload / Save as / Print) is gone from the bar.

### Changed
- Command bar is smaller (560 x 350, compact rows, 15 px input).
- Type: Segoe UI Variable first (the metrics the layout is built on), Noto Sans bundled as a fallback; every bold label is weight 600; smaller text in the collapsed island, the temperature sits on the clock's line, "Agent working" is lighter.
- Settings window: a page fades in on change, the selected segment pill slides, dependent rows fade.

### Fixed
- A transparent window could turn into an opaque black slab when something switched DWM blur-behind off after it was created (for example a Windhawk mod unloading); the bar re-asserts it every time it opens and the island every 10 s.
- Steam downloads: the chip follows the app-state phases (downloading / staging / committing) and the live size of steamapps\downloading, so it no longer sits at 0 % while Steam has not rewritten the manifest yet.

## [0.3.0] - 2026-10-25

### Added
- Command bar: AI answers are formatted (paragraphs, lists, **bold**, *italic*, `code`, code blocks with a copy button; links stay plain text). Built from DOM nodes, never from raw HTML.
- Command bar: after an answer the next line you type is a follow-up in the same conversation (last 6 turns, kept in memory only; Esc ends it, «Новый разговор» starts afresh). A line starting with `>` or `=` goes back to normal search.
- Live events: new-mail cards over IMAP («Новое письмо: sender — subject», «и ещё N»). Opt-in, off by default, own switch on the «Живые события» page. TLS only (port 993, plain IMAP is refused), read-only EXAMINE, only `BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)]` is fetched, so mail is never marked as read and no body or attachment is downloaded. The mailbox password is encrypted with Windows DPAPI, never shown again after saving; a «Проверить подключение» button reports the result in plain words. Polls every 2 minutes and when the settings change, with back-off after errors; only the highest seen UID is remembered (`mail-state.json`, no mail content).
- Calendar: the .ics field also accepts an `https://` or `webcal://` subscription link (fetched every 30 minutes, 2 MB and 15 s limits, last good copy cached next to the settings so it works offline, only the host is ever logged; other schemes and plain `http://` are rejected).

## [0.2.0] - 2026-10-25

### Added
- Command bar: a query starting with `? `, `ии ` or `ai ` is a question for the chat model from Settings > AI. Enter sends it, the answer appears under the input (selectable, scrollable), Enter again copies it.
- Command bar: a microphone button (and F4) dictates with the same voice-to-text pipeline as the Text tab; the transcript is typed into the input.
- Live events: a card «Загружено: file name» when a file finishes in Downloads (click opens the folder with the file selected), connected / removed removable drives, battery warnings at 20 % and 10 % plus «Charging» / «Fully charged» (laptops only), and «In 10 min: title» for the next event of a local .ics file (off by default; path in Settings > Live events). Each has its own switch; everything is local and read-only.

## [0.1.6] - 2026-10-25

### Changed
- The typeface is now Inter (bundled, SIL OFL) with smooth unhinted rendering instead of Nunito, which looked blocky on small text.

## [0.1.5] - 2026-10-25

### Added
- Text tab: the AI mode is connected. Translate, fix errors, shorten, summarize, formal / friendly tone, bullet list, explain, continue, or type your own request; it works on the selection (or the whole text) through the chat endpoint from Settings, and Ctrl+Z restores the original.

### Changed
- Typography: the whole UI uses a bundled typeface (SIL OFL).
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
