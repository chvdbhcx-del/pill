# Running the native UI

## Switches

| what | how |
|---|---|
| everything native for this run | `pill.exe --native` or env `PILL_NATIVE=1` (the four keys below are not touched) |
| per surface, persistent | settings.json keys `ui.native.pill` (island + panel tabs), `ui.native.bar` (command bar), `ui.native.settings` (settings window), `ui.native.capture` (screenshot overlay); all default `false` |
| back to the WebView UI | `pill.exe --web-ui` or env `PILL_WEB_UI=1`: wins over the keys AND over `--native` |
| software renderer | `PILL_FORCE_WARP=1` |
| layered-window present path (no DirectComposition) | `PILL_FORCE_LAYERED=1` |
| host loop trace on stderr | `PILL_HOST_TRACE=1` |

Flags off and no `--native`: nothing native starts, the app is the WebView app byte for byte. With any flag on there is no WebView window for that
surface (island: `build_island`; bar: `build_bar` / `lazy`; settings: `settingswin::open`; capture: `open_overlays`).

## How it is wired (glue)

* `ui::native::ensure_host(app)` (idempotent): `ui::api::init`, Medeo Sans, the facade as the host's `Api`, `host::start`. The island, the bar,
  the settings opener and the capture overlay all call it; `main.rs` calls `native::start_if_needed` first when any flag is on.
* `ui::tabs::fx::TabFx` is the only tab -> island message. A tabs post it with `cx.post(Msg::Custom)`, B tabs and overlays with `fx::emit`; the
  island drains `tabs::b::take_fx()` after every call into a view (`IslandView::drain_fx`) and applies it in `island/tabfx.rs`.

  | TabFx | effect |
  |---|---|
  | `Toast` / `Card` | card in the shell queue (+ recorded by the notifications tab through `ToastRec`) |
  | `Live` | ignored on purpose: music row / shelf chip / drop card belong to the hub providers (`MediaLive`, `ShelfLive`) fed by the same events |
  | `SetLive` | one-line text row with that key |
  | `React` / `Unreact` / `EdgePulse` / `EdgeColor` / `DragPull` | reaction resolver / edge halo / edge colour / drag-pull glow |
  | `Badge` | counter on the tab button (empty or 0 hides) |
  | `Collapse` / `Expand` / `SetPinned` | shell (`SetPinned` also persists `pinned`) |
  | `OpenCommandBar` / `VoiceToggle` / `GoPage(p)` | backend `cmd_toggle` / `voice_toggle` / `settings_open {page}` (native or web settings, by the flag) |
  | `StartCapture{mode}` | collapse, 450 ms later `capture_start {mode}` (at once for `record_screen` while recording) |
  | `OpenClipAi` / `CloseClipAi` / `OverlayClosed` | the `ai_actions` overlay over the tab body: Esc and clicks go to it first |
* Tabs with background duties (home, shelf, notifs, clipboard) are created at start (`TabHost::warm`: `on_show` + `on_hide`), so they keep
  receiving events and replies. `Msg::Custom` is routed by type (images to all views, cover colours to home, file picker to chat, meters to
  controls, ticks to the visible tab). Drops while the panel shows Home / Shelf go to that tab first (launcher strip, shelf), the rest lands via `shelf_add`.
* Weather: not a kept view. The backend's `net::bg_loop` publishes `weather:current` (also with the web UI closed) and the shell asks `weather_get`
  itself, so the folded clock row has temperature and icon while the weather tab was never opened.
* System monitor: the hub's `MetersView` is handed to the Controls view (`tabs::a::Meters`), which draws the CPU / RAM / GPU row and shortens the
  tiles to 116 px, as `sysmon-live.css` did.

## Known gaps

* `Live` rows of A tabs are not drawn by the tabs (hub-owned); if a hub provider is missing for a row, that row is missing.
* `StartCapture` uses a fixed 450 ms delay like the web.
* A B-tab `SetLive` row with the same id as a hub row replaces it (same as `Pill.setLive` key semantics on the web).
* Nothing here was verified on a real screen: tests are headless (Sim + WARP).

## On-screen checklist for the lead (prioritised, run `pill.exe --native`)

1. Start: no WebView window appears at all (Task Manager: no msedgewebview2 children), island sits top centre, no console window, no crash.
2. Hover opens the panel, leaving folds it; liquid edge reacts; click-through works around the capsule.
3. Every tab opens and looks like the web one: Home, Weather, Controls, Notifs, Clipboard, Shelf, Note, Text, Chat, Agents; slide direction right.
4. Keyboard in tabs: type in Note / Text / Chat / Clipboard search; Esc folds (or closes the AI overlay first); focus returns to the app below.
5. Folded music row: play something; row, cover, equaliser follow `media:changed` while the panel is closed; Home buttons work when open.
6. Toasts: screenshot done, timer done ("Again" button), notification card; they appear in Notifs history with a badge on the tab.
7. Clipboard: copy text, hotkey `clip:open` expands the tab with the search box focused; AI quick actions overlay opens, Esc closes, result Copy/Paste.
8. Drag a file onto the folded capsule: drop card + edge glow, lands on the shelf once; onto Home launcher strip: added to launcher; onto Shelf tab: added once.
9. Controls: CPU / RAM / GPU meter row above the tiles (needs Settings > Live > System monitor), bars ease, hot colour at limits; tile toggles and Bluetooth popover.
10. Command bar: hotkey and the Home button open the native bar under the island; Esc / blur hides it. Run a result.
11. Settings: tray / island button / Note tab "voice" and AI wizard buttons open the native settings on the right page; changes apply live.
12. Capture: Home tiles collapse the island, then the native overlay appears on every monitor; area / window / record work; recording capsule + Stop.
13. Weather: with weather enabled in Settings the folded clock shows temperature and icon without opening the weather tab; tab works when opened.
14. Reduce motion (Windows or setting) = no motion anywhere; light theme; DPI 125 / 150 / 200 % and two monitors; pets and characters.
15. Rollback: `pill.exe --web-ui` (and `--native --web-ui`) gives the WebView UI; idle CPU of the native run is near zero (no frames when nothing moves).
