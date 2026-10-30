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

PARTS
