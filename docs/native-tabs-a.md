# Native panel tabs, set A (`src-tauri/src/ui/tabs/a*`)

Home (media + launcher strip), Weather, Controls, Notifications, Clipboard, Shelf as native `View`s drawn inside the expanded island panel.
Models are `pill_ui_core::panel::a::*` (pure, tested); the views here only own a model, feed it, execute its `BackendCall`s, play its `Anim`
cues and draw. Set B (note, text, chat, agents, ai) is `ui/tabs/b*`.

```
ui/tabs/mod.rs        create(id): a::create(id) or b::create(id)
ui/tabs/fx.rs         TabFx: what a tab asks of the island (toast, collapse, live row, pet reaction, badge ...)
ui/tabs/a.rs          create(id), W / CHROME_H / tab_h, the host contract (module doc)
ui/tabs/a/core.rs     Base: calls in flight, subscriptions, Ticker (model clock), Fx routing, Incoming classification
ui/tabs/a/play.rs     Player: Anim cues -> Sample (alpha, offset, scale, rotation, progress)
ui/tabs/a/draw.rs     gradients (linear / linear_deg / radial), WIC decode (bytes / file), ImageCache (LRU byte cap, lazy D2D bitmaps)
ui/tabs/a/icons.rs    the 66 line icons of src/icons.js via glasskit svgpath (icons_data.rs is generated from icons.js)
ui/tabs/a/snap.rs     test helper: OnBg backdrop + PNG snapshots ($PILL_TABS_DUMP or <target>/tabs-a-snaps/<tab>/<name>.png)
ui/tabs/a/<tab>.rs    one view per tab (+ a/<tab>/ helpers)
```

## 1. Host contract (for the island view)

* `ui::tabs::create(id) -> Option<Box<dyn View>>` builds the view on the UI thread. `size_hint()` = the tab's content box: 648 px wide
  (`a::W`: the 680 px panel minus 16 px padding on each side) and `tabH - 71` high (`a::CHROME_H`: 60 px header + 11 px bottom of `#views`):
  260 px for home, clipboard, shelf, controls and notifs (`tabH` 331), 346 px for weather (`tabH` 417).
* The tab draws into the `rect` it is given (any origin) and expects input events in the SAME coordinate space as the rect of the last
  `draw`. It paints no background. Use the island's theme (`Theme::pill_dark()` / `pill_light()`).
* The island drives the tab with its OWN `Cx`: `on_show` when the tab becomes the visible one, `on_hide` when it stops being it (the ticker
  stops; a hidden tab costs nothing). Dropping the view is fine at any time.
* Every `Msg` that is not a `TabFx` is forwarded to the active tab's `message`: backend events it subscribed to (`Cx::subscribe` posts them
  to the island's queue), replies of its `call_async` commands, its `Tick`, and its image-decode answers (`ImageReady`). A tab ignores
  messages that are not its own.
* `View::on_drop` is the tab's file drop (home launcher, shelf); `Enter` / `Over` over the tab area must be answered with the tab's response
  (`consumed` = accept). The island decides which tab area a drop lands in; the shelf and the launcher strip hit-test inside the tab.
* Island-bound requests come out as `Msg::Custom(Box<TabFx>)` posted with `cx.post` (so they land in the island's own `View::message`):

```rust
pub enum TabFx {
    Toast { id: &'static str, icon: &'static str, title: String, ttl_ms: u32 },
    Collapse { via: Option<&'static str> },          // Some("click") = a click that closes the island
    Expand { tab: &'static str },
    Live { key: &'static str, spec: Option<LiveSpec> },   // Pill.setLive; None removes the row
    React { key: &'static str, kind: &'static str, color: Option<String>, ttl_ms: Option<u32> },
    Unreact { key: &'static str },
    Badge { tab: &'static str, text: String },       // empty text hides the badge
    SetPinned(bool),
    OpenCommandBar,
    StartCapture { mode: &'static str },
    OpenClipAi { id: i64 },
    CloseClipAi,
    DragPull(bool),
    EdgeColor(Option<String>),
}
```

## 2. Rules every view here follows

* Animations: the model's `Fx::Anim { target, anim }` cues are played by `Player` (glasskit easing curves, the numbers of `motion.js` /
  `theme.css`). Reduce motion (`Cx::reduced_motion`, `ease::reduced()`) plays nothing and shows the end state. Frames are requested only
  while a cue or a control is moving; a settled tab renders nothing. No frame / tick rate is ever lowered.
* No work in `draw`: no backend calls, no decoding (images decode on the worker pool and arrive as `ImageReady`).
* Image caches are capped (24 MB of decoded pixels per view, LRU).
* The model clock (seek bar, polling intervals, 3 s arms, debounce) advances from a ticker thread that posts one `Tick` per period while the
  tab is shown; the tick wakes the view, it never forces frames by itself.
* Weather stays opt-in (the model's consent / contact state is drawn as is); nothing fetches on its own.

## 3. Tabs with background duties (the island must keep them alive)

In the web UI every tab module is alive from start-up and listens to events while the panel is collapsed. Four of the models do real work then, so the
island should create these views at start (`ui::tabs::create(id)`), keep them (call `on_hide`, do not drop them) and forward every non-`TabFx` `Msg` to ALL
of the kept views, not only the visible one:

| tab | background duty | effect |
|---|---|---|
| home | `media:changed` drives the folded island's music row | `TabFx::Live { key: "music", .. }` |
| shelf | files dropped on the folded island, count chip, drag-pull glow (the island forwards the drop via `View::on_drop`) | `TabFx::Live`, `Badge`, `DragPull` |
| notifs | records every card the island showed: forward it as `Msg::Custom(Box<ToastRec>)` (`pill_ui_core::panel::a::notifs::ToastRec { id, title, sub, icon, tone, text }`); the view answers with `notif_history_add` (dedupe, unread badge) | `TabFx::Badge { tab: "notifs" }` |
| clipboard | `clip:open` (hotkey) expands the tab and focuses the search box | `TabFx::Expand { tab: "clipboard" }` |

A view that was never shown still answers messages; it only draws when the island asks.

## 4. Per-tab notes

### home (`a/home.rs`, `a/home/{layout,paint,css,tests}.rs`)
Media card (cover with the colour glow of `cover.rs`, source label, title, artist, seek, transport), volume row with mute, output-device list, 2 x 4 tile grid and
the launcher strip. The wiring table is in the module doc. Cues: `title` / `artist` (text swap), `play` (glyph swap), `art` (cover crossfade), `launcher` (FLIP of the
icons, drag-reorder shifts), `link:<id>`. The model clock (seek bar, sessions refresh, launcher removal) advances from a 250 ms ticker: a playing track costs
four wakes a second and no frames. Launcher drops go through `View::on_drop` (launcher hit first). Tile labels wrap like the web.

### weather (`a/weather.rs`, `a/weather/{field,wxicons,tests}.rs`)
Sky card (condition gradient with the 0.8 s crossfade, coloured icon from the `PillWxIcon` geometry, big temperature, stats, sun times), 8-slot hourly strip, 7-day
list with range bars, units switch, and the off / city / error / loading cards. **Opt-in is the model's:** nothing is fetched before `net.weather`; the contact
field travels with the enabling `settings_set`. Text fields queue their input and take it at the start of the next draw (glasskit `TextInput` needs a canvas).

### controls (`a/controls.rs`, `a/controls/{layout,paint,tests}.rs`)
Wi-Fi / Ethernet / Bluetooth (+ paired-device popover) / dark-theme tiles and the four power tiles (press twice; the red wash drains over 3 s). The CPU / RAM / GPU
meters the web shows above the tiles come from `sysmon-live.js`, not from `tab-controls.js` or the panel-A model: they are NOT in this view (the tiles then fill the box).

### notifs (`a/notifs.rs`, `a/notifs/tests.rs`)
History grouped by day (sticky day headers), tone tiles, hover swaps the time for the remove button, removed rows collapse while the rows below slide up, staggered
enter, clear-all fade, empty state. Scrolling is a glasskit `ScrollView`; its metrics follow the rect of each draw.

### clipboard (`a/clipboard.rs`, `a/clipboard/{paint,tests}.rs`)
Search (glasskit `TextInput`; the debounce is the model's), history rows with WIC-decoded thumbnails (`clip_image` data URL -> worker -> `ImageCache`), pin / copy /
AI / delete buttons on the hovered row, keyboard selection, wipe arming, row enter / collapse / FLIP cues. The tab takes the keyboard with `cx.set_focus(true)` while the
search box is focused. Clipboard contents are never logged.

### shelf (`a/shelf.rs`, `a/shelf/{layout,paint,tests}.rs`)
Chips (shell icon / thumbnail / folder / typed document tile), ctrl-click multi-select, context menu (open, show in folder, copy path, remove), remove / clear / drag all,
fly-in of dropped chips, ring / shake cues. Drag-out: the model returns `shelf_drag_out { ids }` and the view runs it through `call_async` while the left button is
still down (the OLE source blocks its worker until the drop; the reply resets the ghosts). Drop-in: `View::on_drop` Enter / Over accept files, Drop calls `shelf_add`.

## 5. Verification

`cd src-tauri && cargo test --release tabs::a` runs the unit, wiring (RecordingApi: exact command names and arguments), Sim-driven interaction, idle-frame and
reduced-motion tests, and writes PNG snapshots of every tab state (dark + light, scale 1 and 2) to `<target>/tabs-a-snaps/<tab>/<state>-<theme>-x<scale>.png`.
Comparison with the web version, same fixtures on both sides:

```
node tools/dev/tabs-a-web.mjs --scale 1            # headless Chrome + stubbed Tauri: <target>/snap/web-a/<state>-<theme>-<s>x.png (23 states)
cargo test --release tabs::a                       # the native PNGs
node tools/dev/tabs-a-diff.mjs --theme dark        # web over native in <target>/snap/diff-a, mean difference per state
```

Measured (dark and light, scale 1): 0.5 - 2.5 % mean absolute difference per state (anti-aliasing and font hinting alone give about 1 %). Known visible
differences: line wrap of the weather off card (font metrics), the selected clipboard row highlight is a touch lighter, shelf chips have a slightly larger gap between name and size.

## 6. Gaps

* The sysmon meters above the Controls tiles (`sysmon-live.js`) and `Pill.setLive` rows of other live modules are not part of panel set A.
* Weather polling and the weather icon of the folded island need the weather view (or its model) alive while collapsed; the island decides.
* Tooltips (the `title` attributes of the web chips) are not drawn; the models expose the text.
* Launcher icons arrive as data URLs of the shell icon; when absent a generic file / folder glyph is drawn.
* IME composition strings (the `Char` path works), touch and right-to-left text: as in docs/native-host.md section 11.

## 7. What only the lead can verify on screen

1. Home: play / pause / next with a real player (Spotify, a browser); the seek bar moves once a second at no CPU cost (Task Manager); drag the seek and volume sliders, wheel on the
   volume row; open the device list and pick a device; drop an .exe or .lnk on the launcher strip, drag an icon to reorder, right-click removes; tiles start the capture / command bar.
2. Weather: with `net.weather` off nothing is sent (network monitor); enable with and without a contact; change city; the refresh icon spins; units switch.
3. Controls: Wi-Fi / Bluetooth / dark theme toggles act (busy pulse while waiting for UAC); a power tile arms on the first click and fires on the second within 3 s; the popover lists real devices.
4. Notifications: a download / USB / battery card shows up in the list; unread badge on the tab; remove and clear animate; a long list scrolls smoothly.
5. Clipboard: copy text and a screenshot, both appear (the thumbnail a moment later); search, arrow keys, Enter pastes into the previous window; Shift+click copies only; wipe needs two clicks.
6. Shelf: drop files on the open tab and on the folded island; chips fly in; drag a chip into Explorer / a browser upload (copy cursor, ghost while dragging); selection drags several; context menu actions.
7. Reduce motion (Windows animation effects off): every tab shows end states at once. Scale 100 / 150 / 200 %: text and 1 px lines stay crisp.
8. Idle: a settled tab renders no frames (`PILL_HOST_TRACE=1`).
