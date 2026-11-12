# Native settings window (`src-tauri/src/ui/settings_view`)

> **Since 0.7.0 there is no web (WebView2) UI.** The native UI described here is the only one: the `ui.native.*` keys, `--native`, `--web-ui`, `PILL_NATIVE` / `PILL_WEB_UI`, `ui/flags.rs`, `ui/lazy.rs` and `ui/windows.rs` are gone, and every gate / "flag off" / "web" comparison below is history (v0.6.1 is the last release that still contains the web code).

The Settings window of Pill on the native host (`docs/native-host.md`), replacing `settings.html` + `settings.js` when the flag
`ui.native.settings` is on (and `--web-ui` / `PILL_WEB_UI=1` is not given). Pure logic stays in `pill_ui_core::settings` (schema, `SettingsModel`,
search, wizard, flows); this module only draws it and routes input.

## Gate

`settingswin::open` (tray, island, command bar, `pill.exe --open settings`, the first-run wizard request, AI setup links) and
`settingswin::settings_close` start with `if ui::settings_view::native_enabled()` (= `flags::native_settings()`, which already answers false while
the web UI is forced). Flag off = exactly the old code path. The window is the host's `Normal` singleton `settings`; a second open sends
`Msg::Open({page})` to the open view. Geometry memory keeps the old keys (`settingsWin.x|y|w|h|max`, debounced by the host), restored with
`Placement::Restore` (falls back to centring when the monitor is gone).

## Files

| file | what |
|---|---|
| `mod.rs` | `SettingsView`, `View` impl, `Local` messages |
| `open.rs` | host registration, geometry memory, facade `Api` over `ui::api::call`, ticker, file dialog |
| `nav.rs` `step.rs` `replies.rs` `acts.rs` `events.rs` | page building / switching, per-frame work, backend answers, clicks, input routing |
| `page.rs` `rows.rs` `cardpaint.rs` `ctlpaint.rs` `keys.rs` `pagepaint.rs` | page / row / control types and painting |
| `chrome.rs` `paint.rs` `wizard.rs` | title bar, sidebar, search, the frame, first-run wizard |
| `custom.rs` `cw1.rs` `cw2.rs` `cw3.rs` `cw3b.rs` `customacts.rs` `prim.rs` | the 12 `Custom` widgets and their drawing primitives |
| `picker*.rs` | pet cards and island-style cards with live previews |
| `ui.rs` `fields.rs` `txt.rs` `pal.rs` `icons.rs` | hit registry, deferred text fields, text helpers, palettes, icons |
| `tests_adv.rs` | the collapsed "Advanced" section: default closed, spring, no events / Tab stops while closed, search and old page names |

## Pages

Ten sidebar pages, each a list of named sections (`Group` titles) in the order people reach for them; rarely needed or technical sections come last,
flagged `advanced` and folded behind one "Advanced" header (see below). The rows are defined once (`ui-core/src/settings/pages/{base,system,phone,ai}.rs`,
the old sixteen pages); `pages/layout.rs` takes them out of a pool by id and arranges them, and `Pool::finish` panics when a row is left over.

| page (id) | what it holds | Advanced section |
|---|---|---|
| Appearance (`appearance`) | island look, edge style and character, pet, what the folded island shows (the old Character + Appearance pages) | position and transparency, animation and smoothness |
| General (`general`) | language, autostart, when to hide the island, Quiet hours, browser search engine (the old General + Behavior + Search pages) | hiding details, program lists, pass-through mode, capture visibility, Everything file search, settings backup, first-run wizard |
| Hotkeys (`hotkeys`) | main keys, screen keys, launcher shortcuts | actions that have no key yet |
| Modules (`modules`) | module switches (VPN included), profiles, weather (the old Weather page), tab strip, Home layout | VPN: your own client |
| Events (`live`) | live events by kind: media and screen, downloads and drives, system, messages, calendar, mail, AI agents | cards and history, download details, chips by the clock, OBS connection |
| Clipboard and screenshots (`clipboard`) | clipboard history, screenshots, screen recording | click-to-paste, exclusions and limits, encoder and reveal |
| Sound and voice (`sound`) | devices, music, reminder sound, voice to text (the old Sound + Voice pages) | player sources, own reminder file, voice service address / model |
| Phone (`phone`) | link and pairing, notifications, clipboard, files, relay switch | file cap and app address, relay address and cap |
| AI (`ai`) | assistant, local AI install, Whisper (the old AI + AI setup pages) | - |
| Updates (`updates`) | version, what is new, report a problem | - |

Old page ids keep working everywhere (`nav::LEGACY`): `character` -> Appearance, `behavior` / `search` -> General, `weather` -> Modules, `voice` -> Sound and
voice, `aisetup` -> AI. The window opens the new page and scrolls to the start of the old section (no flash); Ctrl+F also matches the old page names
(they are appended to the search keywords of every moved row).

### The Advanced section

`Group::adv()` marks a group; the groups so marked are one trailing run of a page. `build_page` puts a `Block::Adv(n)` header before them (n = the blocks
that follow) and a `page::Adv { t: Flow, on, sub, fid }` on the page; the header lists the names of the sections inside. The section starts closed on every
window start and is **never stored** (`SettingsView::adv_open` remembers only which pages were opened while this window lives). A click or Enter / Space on the
header sets `Flow` `t` (target set in the event handler, ticked in `step_all`; instant under Reduce motion). `paint_blocks` scales the heights, the opacity
and the divider gaps of the following blocks by `t`, `paint_card` multiplies every row's `vis` by it and keeps it in `RowRt::ak`; rows with `ak < 0.5` take
no events and `focus_order` leaves them out while closed. A Ctrl+F hit inside a closed section calls `set_advanced(true, instant)` from `resolve_goto`
and waits for the next layout before scrolling to the row and flashing it. Idle cost is nil: only the one `Flow` moves, and only for a moment.

## How it works

* One pass per frame inside `draw` measures and paints (layout cached for the next input event, so tests render once before sending events).
* Each schema row owns one glasskit control (Toggle, Slider with value bubble, Segmented, Select, Button, TextInput); a changed control goes
  `Change -> SettingsModel::try_set -> Vec<BackendCall> -> Cx::call_async`; the answer returns as `Msg::Reply`. Hotkeys only go through the recorder
  flow (`hotkey_set`, never `settings_set`). Secrets (API keys, passwords) are password fields that call their own command and are cleared.
* `TextInput::event` needs a `Canvas`; `fields::TextField` queues the events and replays them at the next `draw`.
* Animations (all collapse to instant under Reduce motion): page exit slide 120 ms + enter rise 240 ms, row expand / collapse 340 / 300 ms and the
  disabled fade, switch spring, segment pill, chip pop (weekdays, providers), message line rise, search popup pop + 26 ms stagger + jump glide +
  1.6 s wash, sidebar indicator spring, wizard step slide, picker card selection, key-capture look, window entrance 140 ms.
* Frames run only while something moves; the Appearance page loops its previews (per-card `PetEngine` / `Characters`). A ticker thread runs only
  on the Events page (status polling), while Everything is being installed and so on.

## The 12 `Custom` widgets

`ai.providers`, `voice.providers` (chips, help, region warning, "Get a key" link) / `character.reactions` (reaction chips driving every preview) /
`hotkeys.launcher` (list from `launcher_list`, one recorder each) / `modules.profiles` (list, Apply, Delete, name field + Save) / `weather.city`
(field, results list, `weather_set_city`) / `search.everything` (status, Download, Run installer, progress box, 5 s status poll) /
`general.backup` (Export, Import through the system file dialog) / `updates.whatsnew` (animated disclosure with the CHANGELOG notes) /
`aisetup.intro` (text) / `aisetup.whisper` (model chips, plan, progress, switch, delete-with-confirm) / `aisetup.chat` (found servers, Ollama installer
and model catalog). Download jobs come from `aisetup:progress`.

## Tests

`cargo test --release settings_view`: every page renders in dark and light; toggle / segmented / slider write the right patch; the recorder commits
through `hotkey_set` (Esc cancels, Backspace clears); sidebar navigation fetches page data; Reduce motion switches at once; Ctrl+F search jumps and
flashes; the wizard steps and writes `firstRunDone`. `PILL_SNAP_DIR=<dir>` also writes the PNG snapshots (`<page>-<theme>-1x.png`).

## Known gaps

No light theme existed on the web page (the light palette is a native derivative); pet / style cards are not keyboard-navigable yet (Tab skips
them); Clipboard "Wipe" has no Cancel button (second click confirms, Esc is not wired); `net_features_status` host line is not shown; no
screen-reader tree (host limit); scale-2 and web-vs-native pixel comparisons were not run.

## Lead's on-screen checklist (flag on: `ui.native.settings` = true in settings.json, run `pill.exe`)

1. Tray -> Settings opens the native window on Appearance; `pill.exe --open settings` focuses it; with `--web-ui` the old window opens.
2. Drag by the title bar, double-click maximizes, resize from every edge, snap (Win+arrows), minimum 700x520; close, reopen: same place / size.
3. Move between monitors of different DPI: text stays crisp, window keeps its logical size.
4. Switch pages (click and Ctrl+F search incl. a row on another page: scrolls to the middle and flashes); the indicator glides.
5. Toggle, slider (bubble), segmented, select popup, text fields (type, paste, Enter / blur commit), Quiet hours chips and time fields.
6. Hotkeys: click a box, hold Ctrl+Alt, press a letter -> saved; a used / reserved / taken combo shows the red reason; Esc cancels; Backspace clears.
7. Autostart switch writes / removes the Run value; Reduce motion setting makes every animation instant.
8. General -> Advanced -> First run -> Run again shows the wizard (Skip, Back, Next, dots, language change inside it); first start shows it once.
9. AI (with the install part) / Sound and voice / Updates / Events pages: buttons, progress boxes, status lines; nothing downloads without a click.
10. Advanced: on a page with the header at the end (General, Clipboard...) it is closed; click it, the section opens on a spring, click again, it closes; Ctrl+F "video encoder" opens Clipboard, opens the section and flashes the row.
11. Idle: window at 0 % CPU (Task Manager) on every page except Appearance (previews loop); Reduce motion stops those too.
