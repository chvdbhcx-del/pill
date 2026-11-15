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

## The bottom edge (`edge.position` = `bottom`)

The island can hang from the top edge (default) or stand on the bottom edge, at the taskbar, and grow UPWARD. The shell, the tabs and the
header keep their one layout, "hanging from the top of the window"; `IslandView` mirrors it about the window's middle line:

| piece | what is mirrored |
|---|---|
| window | `pill.rs::frame_for` -> `geom::dock_position(Dock, mon, work, under_taskbar, ..)`: the window's BOTTOM is the anchor (work-area bottom with `underTaskbar`, the monitor bottom without), so the folded (120) and the open (480) frame share the floor; `Placement::BottomCenter`, `geom::resize_keep_bottom_center` for a DPI change |
| body, hit region | `IslandView::win_rect` (`geom::mirror_y`): `h - (y + height)`; the click-through region, `locate`, `local_live` and the panel origin use it |
| silhouette | `shapes::paint_body` / `paint_edge` / the tail run unchanged under a vertical flip (`flipped`): flat against the screen edge, round on the free side, flares, the light shadow and the liquid edge on the free side |
| header | `ChromeLayout::docked_bottom`: tabs, buttons, VPN chip and the pet slot move to the bottom of the panel, the tab view grows above them |
| legs / eyes | the legs hang from the top of the body (flipped), the eyes stay upright |
| pointer | `shell_pt` mirrors `Zone` and `MouseMove` before they reach the shell (the liquid edge reaches for a pointer above the body) |
| edge dwell | `Cx::set_edge_bottom` -> `Proximity::set_edge_bottom`: `TopEdge` fires on the monitor's BOTTOM row; the fullscreen peek (`fspeek::on_edge`) too |
| command bar | `pill::body_bottom` is `None` at the bottom: the bar hangs from the top of the monitor as always |
| auto-hide | the `-130 %` slide runs through the flipped body, i.e. downward |

Side along the edge (`edge.align` = `center` | `left` | `right`, bottom edge only; the top edge is always centred): `geom::dock_position_aligned`
puts the WINDOW against that side of the monitor (of the work area with `underTaskbar`, so a side taskbar is not covered; `offsetX` does not
apply), and the view moves the body, which the shell lays out centred, to `geom::ALIGN_PAD` (20 px) from that side of the window
(`IslandView::dx_for`, `geom::align_body_dx`): the body keeps its edge while it grows, and the open panel (already 20 px inside the frame) is
where it always was. `win_rect` / `shell_pt` / `flipped` carry the shift, the reported `pill_rect` x is shifted too (the edge reveal watches
the corner).

Changing the setting live (edge or side): the view slides the body out through the old edge (`tuck`, `Slide::Out`), asks for the new edge (`pill_edge`, which
moves the window while nothing is visible), waits `EDGE_HOLD_MS` and slides the body in from the new edge (`Slide::In`). Reduce motion and a
hidden island take the edge at once. `pill.rs` keeps the edge the window is on (`pill::dock`), which follows `edge.position` only through
`pill_edge`. The settings preview (`settings_view/preview`) runs the same view in its sandbox; its camera stands the island on the floor of the
stage when `Live::at_bottom`.

## The music widget (`media.widget`): the player as its own window

`ui/musicwidget/` puts the island's player into a SEPARATE small window, so it can stand apart from the island: in the bottom left corner next to
the taskbar while the pill stays in the middle, at the top, or on either side of either edge. Settings (Media and sound page, section "Player widget"):

| key | default | meaning |
|---|---|---|
| `media.widget` | `false` | off = nothing changes and no window exists; on = the widget window (the island keeps its own player too) |
| `media.widget.edge` | `bottom` | `top` / `bottom` / `taskbar` (INSIDE the bottom taskbar strip, see "The widget inside the taskbar" below) |
| `media.widget.align` | `left` | `left` / `center` / `right` along that edge |
| `media.widget.offset` | `0` | -50..50 % of the monitor width, only for `center` (like `offsetX`) |

The monitor (`monitor`), the taskbar rule (`underTaskbar`) and `showInCapture` are the island's.

| piece | how |
|---|---|
| window | ONE Overlay (`NativeWindow::overlay("music-widget")`, title `pill-music`), 344 x 208 logical px, never resized, created hidden, only while `media.widget` is on; topmost, tool window, no-activate, per-pixel alpha, click-through outside the body (`Cx::set_hit_region`) |
| placement | `Placement::Docked { monitor, dock, align, offset_pct, inside_work, margin }` -> `geom::dock_position_aligned` (the same maths as the island's corner); it is part of the window spec, so `musicwidget::sync` re-creates the window when a placement setting changes; `musicwidget::relayout` (guard poll via `pill::relayout`) re-places it when the monitor, its DPI or the taskbar changed; `WM_DPICHANGED` re-places a Docked overlay by its spec instead of keeping a centre |
| data | the island's own feed: `media:changed` (+ `media_state` once at the start and when a cover is missing), `media_control` (prev / play_pause / next), `media_seek`; no second media backend. `media.rs` keeps emitting while the island is hidden when the widget is on (`widget_on`); the equaliser capture still follows the island alone |
| privacy | `module.media` off = no track is read, nothing shows, and a visible widget leaves at once; nothing is stored |
| when | `logic::Presence`: while a track plays and for 45 s after a pause; a player first seen paused does not call it up; no session = gone |
| shapes | `logic.rs` (pure, tested): the capsule 320 x 56 and the card 320 x 184 stand flush against the dock edge and the body between them grows away from it (`mirror_y` for the bottom); the layouts and the hit test are functions of the expansion |
| motion | `Flow` / `Feel` only, targets in `step`/events: enter / leave (`shown` slide + `fade`), hover open / close (`expand`, `ProximityEvent::Hover` / `Leave`), track change (`swap` cross-fades the old and the new text, `Gpu::cover` fades the cover in), play / pause (`pp` cross-fade with a pop), press dip, hover glow, progress glide. Reduce motion lands every value at once |
| idle cost | hidden = no frames, no timer; showing = one 1 Hz `Ticker` (clock + the end of a pause) and frames only while a spring moves; the ticker stops when the widget is not wanted |
| guard | `pill.rs` emits `pill:guard { hidden }` when the fullscreen guard (or a peek over it) starts / stops keeping the island away; the widget hides with it |

### The widget inside the taskbar (`media.widget.edge` = `taskbar`)

The same idea as the island's taskbar placement (section "Inside the taskbar"), for the widget: the FOLDED capsule is shrunk to fit inside the bottom
taskbar strip and the card opens upward above it. It reuses `geom::bottom_bar`, the tray clamp (`taskbar::tray_rect`), `bar_fold_cap`, `bar_gap` and the
island's z-order hook.

| piece | how |
|---|---|
| the bar | `geom::widget_bar` = `bottom_bar` of the island, and not taller than `WIDGET_BAR_MAX_H` (72 logical px; its window has room for that much). A bar at the top / left / right, an auto-hidden bar, a monitor without a bar and a taller strip give `None`: the widget is then the plain bottom widget (`dock_position_aligned`, `underTaskbar` as usual), exactly like the island |
| window | `Placement::InBar { monitor, ask }` -> `geom::widget_place` (pure; `taskbar::widget_place` feeds it the real tray and the island's settings): the window is `logic::WIN_BAR` (344 x 276 logical px, never resized: pad + card + 8 px gap + the tallest bar), its floor is the MONITOR's bottom (over the bar), its x comes from `geom::widget_bar_window` (left / right: the body a side pad (8 px) from the span's side, window = body minus the 12 px pad; centre: the middle of the span, shifted by `media.widget.offset`, which is ONLY for the centre) and never reaches the tray |
| folded capsule | `logic::Stand { dock, align, bar }`: in the bar the capsule is `BAR_FOLD_W` (288) wide and `min(56, bar - 8)` tall, centred in the strip (`geom::bar_gap`), laid out by `logic::bar_capsule_layout`: a cover of `h - 8`, ONE line of text (the title, at most two thirds, then the artist dimmer after it), three small buttons (`h - 10`, 16..28) and the progress as a 1.5 px hairline under the buttons (dropped when `h < 26`) |
| opening | the body rises with the expansion spring `p`: its bottom follows `bar_gap(bar, fold_h, p)` from "centred in the bar" to "8 px above the bar" while it grows from the capsule (288 x fold) to the card (320 x 184) on its anchored side, so it never jumps; the hit region (`logic::hit_rect`) also covers the strip down to where the folded capsule stood, so a cursor resting on the capsule keeps the card open (no open / close flicker). Reduce motion lands it at once |
| coexistence | when the island is `edge.position` = `taskbar` too, `taskbar::island_bar` reads its `edge.align`, `offsetX` and `size` (no cross-window messaging) and `geom::widget_bar_slot` keeps the widget `ISLAND_BAR_W` (300 logical px x size) plus a side pad clear of it: the widget goes to the OPPOSITE side (left <-> right); a centred widget, or one whose opposite side is taken too, takes the larger free stretch beside the island's side, centred in it (the slot's `align` is then `Center`). It stays inside the bar even when there is no room at all. The island's real width is not measured: a capsule wider than the reserve may touch the widget |
| news to the view | `musicwidget::relayout` (the 500 ms guard poll, no timer of its own) compares `geometry_key` (monitor, DPI, work area, and in the bar the window position and slot, so a moved tray or island counts) and emits `music:slot { bar: logical px (0 = no bar), align }`; `on_settings` re-checks when `edge.position`, `edge.align`, `offsetX` or `size` change. Hidden: the view just takes the new place (a show re-places it). On screen: it slides out, takes the place, asks the host to re-place the window (`Cx::relayout_self`) and slides in, so the window never moves under a visible capsule. `music:slot` is internal to the widget and is not in the contract table |
| z-order | the bar is topmost too and jumps in front of the widget when it is clicked. `taskbar::follow_widget` points the island's ONE `EVENT_SYSTEM_FOREGROUND` hook at the widget's window as well (installed from the main thread; the callback raises both windows when the new foreground window is a taskbar), and `taskbar::keep_widget_on_top` runs from `musicwidget::relayout` in the guard poll as the safety net (only while the window is visible and in the bar). `SWP_NOACTIVATE`, no move, no resize; a hidden or off-bar widget costs nothing |

Coexistence: the widget and the island's own player show side by side. While `media.widget` is on, `ui-core` `live::media::update_live` still gives the title strip / `music`
edge reaction, `panel::a::home` keeps the player part and `command::tab_on("home")` counts it; nothing is taken away from the island. The settings preview keeps its sample
player (`media.widget*` are not shareable to the preview island).

### Inside the taskbar (`edge.position` = `taskbar`)

A third placement: the folded island lives INSIDE the bottom taskbar strip, like one of its widgets, and opens upward above it. It reuses the whole
bottom-edge machinery (mirrored view, `Dock::at_bottom`, the edge slide) and adds:

| piece | how |
|---|---|
| the bar | `geom::bottom_bar(monitor, work area, scale, tray)`: a plain bar along the BOTTOM of this monitor (work area = monitor minus a strip of at least 24 logical px, nothing else). A bar at the top / left / right, an auto-hidden bar (work area = monitor or a 2 px stub), a monitor without a bar and an absurdly tall strip give `None`: the island is then the plain bottom island (with `underTaskbar` as usual) |
| tray | `taskbar::tray_rect(monitor)`: the `TrayNotifyWnd` child of the `Shell_TrayWnd` / `Shell_SecondaryTrayWnd` that lies along the bottom of the monitor. The free span is monitor-left .. tray-left minus 8 px; a nonsensical or invisible tray is ignored (the span is then the whole bar) |
| window | `pill.rs::frame_for`: the usual frame (120 / 200 / 480 x size) PLUS the bar's height, its bottom on the MONITOR's bottom (over the bar), x from `geom::taskbar_position`: `edge.align` Left / Right = the body 8 px from the span's side (window = body minus the 20 px shell pad), Center = the middle of the span; `offsetX` shifts it away from the side (percent of the monitor width), clamped so the narrowest folded body (118 px) stays inside the span, never over the tray. `underTaskbar` is not used (greyed out in settings) |
| news to the view | `pill.rs::place` emits `pill:taskbar` `{ "h": bar height in logical px }` (0 = no bar) when the bar changes (taskbar resized / hidden / moved, DPI, the setting); the 500 ms guard poll (`relayout`) re-evaluates, no new timer. `pill_edge` carries `bar: true` for this dock |
| folded size | the view hands the shell `set_fold_cap(bar - 2 x 4 px)`: the plain capsule never outgrows the bar (the default 30 px capsule fits any bar of 38 px and more; a bigger `size` is cut down while it stands in the bar and gets its size back outside it) |
| body place | `geom::bar_gap`: folded = the capsule centred in the bar; open / an event card / a widget taller than the bar = floating 8 px above the bar. `IslandView::step_bar`: two springs (`bar_sp` in the bar or not, `raise_sp` folded or floating) give `lift_px`, added to the shell's own rest offset in `win_rect` / `shell_pt` / the drawing (`dy()`), so the folded pill glides up into the open one and sinks back without a jump; reduced motion jumps |
| look | in the bar the capsule is round on the edge side too (`radius_top` follows `radius_bottom` by `bar_w`) and the notch flares are gone |
| z-order | the bar is topmost too and puts itself in front of the island when clicked. `taskbar::follow` installs ONE `EVENT_SYSTEM_FOREGROUND` hook (out of context, delivered by the UI thread's message loop; it wakes only when the foreground window changes and only does anything when the new foreground window is a taskbar) that calls `raise_over_taskbar`: walk `GW_HWNDPREV` up from the island (32 steps at most), and when a `Shell_*TrayWnd` is above it `SetWindowPos(HWND_TOPMOST, NOMOVE | NOSIZE | NOACTIVATE | NOOWNERZORDER | ASYNCWINDOWPOS)`. The same call runs from the guard poll (`pill::keep_alive`) as a safety net. Nothing is reparented into `Shell_TrayWnd`; clicks outside the hit region still reach the bar |

Settings: choosing the taskbar when `edge.align` was never chosen (still the default "center") also sets it to "left" (`SettingsModel::try_set`):
beside Start, in the free area of a centred Win11 taskbar. The settings preview (`settings_view/preview/live.rs`) tells its sandbox island a 48 px
bar and the stage draws a strip under the island.

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
14. Music widget (`media.widget` on, Media and sound page): it appears when something plays (not when a player starts paused), in the corner you chose (try bottom / top, left / centre / right, a second monitor, 150 % DPI, a taskbar on the side); with edge "In the taskbar" the folded player sits inside the bar (left / middle / right, 100 / 125 / 150 / 200 % DPI, a 40 / 48 / 52 px bar), stays clear of the tray and, when the pill is in the bar too, on the other side of it, opens upward above the bar on hover and folds back without flicker, stays above the bar after you click the bar, and falls back to the plain bottom placement with the bar at a side, auto-hidden or absent; hovering opens the card upward (bottom) or downward (top); prev / play / next and the seek bar work without taking the focus from the app you are typing in; a paused track keeps it ~45 s; the island keeps its own player meanwhile (title strip, player on Home); a fullscreen game hides it; switching `media.widget` off removes the window and leaves the island as it was.
