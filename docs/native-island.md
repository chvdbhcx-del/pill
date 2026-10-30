# The native island (`src-tauri/src/ui/island`)

> **Since 0.7.0 there is no web (WebView2) UI.** The native UI described here is the only one: the `ui.native.*` keys, `--native`, `--web-ui`, `PILL_NATIVE` / `PILL_WEB_UI`, `ui/flags.rs`, `ui/lazy.rs` and `ui/windows.rs` are gone, and every gate / "flag off" / "web" comparison below is history (v0.6.1 is the last release that still contains the web code).

The always-resident `pill` window as a `ui::view::View` on the native host (Overlay window), replacing `pill.html` + `pill.js` + `live-*.js` +
`pets*.js` + `characters.js` when the flag `ui.native.pill` is on. Flag off (default): nothing here runs and the WebView island is built exactly as
before. `pill.exe --web-ui` / `PILL_WEB_UI=1` beat the flag (`ui::flags`).

```
island/start.rs      enabled(), start(app): host thread, fonts, FacadeApi (ui::api::call), the Overlay, hand-over to pill.rs (PillWin)
island/view.rs       IslandView: the View impl: events, proximity, drops, step (animation + wake-ups), draw (silhouette, row, panel, pet, edge)
island/core.rs       Core: shell model + live hub + glue (event routing, LiveCmd application, notices -> outs). No window, no GPU: unit tested
island/rows.rs       live-row layout (Elem vocabulary -> boxes, generic over a text meter) and painting
island/live.rs       clock row (3 equal columns, digit roll), event cards, content measure / paint, row enter animations
island/shapes.rs     body with top / bottom radii, notch flares, light shadow, liquid edge (fill, gradient strokes, halo, ball), glow style
island/chrome.rs     tab strip (label open fraction animates), header buttons, VPN chip, pet slot, the rect of the tab view
island/panel.rs      TabHost: ui::tabs::create(id), slide / fade of pill.css, Placeholder view, `guarded` effect filter
island/companions.rs pets (PetEngine) and characters (tail, legs + eyes), gaze, energy, tint, reserve
island/gpu.rs        icon + forecast-icon SVGs (generated from icons.js / tab-weather.js), WIC cover decode + cache, value glides
island/fonts.rs      Medeo Sans embedded, `host::set_fonts`
island/sys.rs        SysClock (wall clock, local offset, test override), QueueBackend, registry reads (light mode, language)
island/snapshots.rs  headless PNG snapshots of the web reference states (tests)
```

## How events flow

```
bus (any thread) --Cx::subscribe--> Msg::Event --> IslandView::message
   settings:changed pill:* audio:bands weather:current ---> Core::bus_event ---> shell::Event (Shell::handle_at)
   every other name (live:*, rec:state, capture:done, ...) -> Hub::dispatch(IslandEnv, name, payload, wall) -> Vec<LiveCmd>
         SetRow   -> Shell::set_live(id, LiveInput{priority, key, ttl, accent, tall}) + typed RowSpec kept in Core::rows (what draw paints)
         ClearRow -> set_live(id, None)             Toast -> Shell::toast(shell CardSpec, actions "a0","a1"..) + typed CardSpec in Core::cards
         React/Unreact -> Shell::react / unreact    TabBadge -> set_badge     EdgePulse -> Core::pulse (halo on the edge, 800 ms)
         PrivacyDots / RecTile / SysMeters -> state  Run(Action) / Query / Broadcast -> Out (executed by the view)
   the same event also goes to every tab view that exists (Msg::Event); replies (Msg::Reply) go to the island's own tickets first
IslandView::exec (after every call): Shell commands (QueueBackend) and Outs
   pill_activate -> Cx::set_focus (never reaches the backend)      pill_rect -> backend (pill.rs keeps the body rect for the command bar)
   battery_state / weather_get / settings_open / pill_desktop_fg -> call_async, the answer becomes Event::BatteryState / WeatherGet / SettingsOpened / DesktopFg
   everything else (pill_set_state, pill_cards, pill_auto_hide, settings_set, ...) -> Cx::call_async(cmd, args) as caller `pill`
   Query -> call_async, the answer is dispatched back to the hub as `as_event` (delayed queries sleep on their own thread)
   Notice::CardAction -> Out::Run(typed action)   TabShown -> TabHost (created lazily, only while the panel is open)
host proximity -> on_proximity: Hover/Leave -> shell Hover/Leave, Zone -> Event::Cursor (liquid bulge), Gaze -> pets/eyes, TopEdge -> pill_auto_hide(false)
host drop -> on_drop: Enter -> ShelfLive drop card, Drop -> shelf_add (launcher_add while Home is open)
timers: the shell's deadlines and the hub tick set an `Alarm` thread (sleeps on a condvar, posts one message): idle windows cost nothing
```

The shell asks the backend for `pill_set_state` and the backend answers with `pill:expand` (unchanged protocol): the native window is resized by
`pill.rs::place` (physical px through `NativePillWin::set_bounds`), the view only sees its client rect change. `LiveMeasured`: `draw` measures the
live content (DirectWrite widths), keeps the result and posts `Own::Remeasure` to itself (`Cx::post` works inside `draw`); the next `step` tells the
shell, which sizes the capsule exactly as `renderLive` + the ResizeObserver did.

Frames: `step` returns true while anything moves (shell springs / liquid, swap / ghost / digit rolls / card phases, looping row elements, tab
tweens, pets, edge pulse, panel in / out); nothing caps the rate (the shell's own `fps` setting is the user's choice and unchanged). Reduce motion:
`Cx::reduced_motion` (OS or the `reduceMotion` setting, pushed with `host::set_reduce_motion_setting`) gives end states everywhere.

## The pill.rs gates (flag off = the old code path, byte for byte)

`pill.rs` got a `NATIVE: OnceLock<Box<dyn PillWin>>` (`attach_native`, `native_ready`) and these branches:

| function | native behaviour |
|---|---|
| `me()` | the Overlay's HWND from `PillWin::hwnd` |
| `layout()` | `layout_native`: monitors from `HostHandle::monitors()` (same `monitor`, `underTaskbar`, `size`, `offsetX` rules; shared `frame_for`) |
| `place()` | `PillWin::set_bounds` instead of `SetWindowPos` |
| `set_ignore()` | no-op: the host's proximity engine toggles click-through, the view sets pass-through (`Cx::set_pass_through`) |
| `apply_vis()` | `PillWin::set_visible`; auto-hide alone keeps the window shown (the host's top-edge dwell must run to reveal it) |
| `affinity()` | skipped until the HWND exists (`native_ready` repeats it) |
| `pill_activate` | returns at once (the view calls `Cx::set_focus`) |
| `pill_monitors` | host monitor list |
| `init()` | no WebView lookup, no `cursor_loop` thread (hover / leave / cursor / gaze / top edge come from the host) |

`ui/windows.rs::build_island` = `island::start` when `island::enabled()` else `build_pill`; `main.rs` setup calls it where it called `build_pill`
(the flag is read from settings.json before `settings::init`, which is fine: `settings::get_bool` loads lazily).
Not wired natively: the `pill:display` event (no consumer), `tauri://drag-*` (replaced by `View::on_drop`), `window.__TAURI__`-style tab events
(tabs subscribe through the shared `Cx`).

## Differences from the web island (known)

* Blur in the enter / swap animations is replaced by opacity + scale only (no Gaussian effect on the live row).
* The glow edge style is a stack of translucent ellipses (no blur effect); the liquid edge is the same polyline / gradient / passes.
* `Slot` rows of providers that put their own markup in a row (not in `Elem`) do not exist here: the vocabulary is the `live` crate's.
* `monitor` index 1.. follows the host's order (primary first, then left to right), the web used Tauri's list.
* Tab views are the other agents' (`ui/tabs/*`); until they exist the panel shows the Placeholder.

## What only the lead can verify on screen

Build with the flag on: put `"ui.native.pill": true` into `%APPDATA%\...\settings.json` (the file `settings.rs` uses), start `pill.exe`; `--web-ui` returns
to the old island. Checklist:

1. The island appears top centre below the taskbar at the right size; `size`, `offsetX`, `monitor`, `underTaskbar` change it live.
2. Click-through: the transparent frame never blocks a browser under it; the capsule takes clicks; no console / no flicker on hover.
3. Hover opens after ~250 ms, leaving collapses after ~380 ms; the liquid edge bulges toward the cursor within 120 px.
4. Clock row, weather icon / temperature, date and battery switches (Settings) update the capsule width with the digit roll once a minute.
5. Music: cover / equaliser / title modes (`musicPill`), edge tint = cover accent; recording capsule + Stop; volume HUD (wheel on the capsule is not ported).
6. Toasts (screenshot, notification, uninstall:ready, battery, update...) drop in, buttons work, ttl folds them; quiet hours mute them.
7. Expanded panel: tab strip animation, header buttons (dictation, pin persists, settings), tab switching slide direction, Escape / focus-out collapse.
8. Text fields in tabs get keyboard focus (`pill_activate` becomes `set_focus`) and give it back.
9. Pets: side, size, tint per reaction, look at the cursor on any monitor, click pets it; eyes / legs / tail characters and the weather icon side.
10. Drag a file to the capsule: drop card, drop lands on the shelf (Home open: launcher).
11. Auto-hide slides the body away; pushing the cursor to the monitor's top edge brings it back; fullscreen guard hides / shows it; tray Show / Hide.
12. Reduce motion on (Windows or the setting): no motion anywhere. Light theme: body, shadow, eyes colour. DPI 125 / 150 / 200 %, two monitors.
13. Tail / legs overlap with the capsule when the pet is on the left and a card or the music row is shown.
