# Native window host (`src-tauri/src/ui/host`)

> **Since 0.7.0 there is no web (WebView2) UI.** The native UI described here is the only one: the `ui.native.*` keys, `--native`, `--web-ui`, `PILL_NATIVE` / `PILL_WEB_UI`, `ui/flags.rs`, `ui/lazy.rs` and `ui/windows.rs` are gone, and every gate / "flag off" / "web" comparison below is history (v0.6.1 is the last release that still contains the web code).

The one place every native Pill surface lives. Wave 2 of the WebView2 -> native-Rust rewrite. Views (bar, settings, capture, panel tabs,
island, pets) implement `ui::view::View`; the host owns windows, input, DPI, frames and presentation. `docs/native-ui-contract.md` lists the
backend names a view calls; this file is how a view runs.

```
ui/view.rs            View, Cx, Effects, Api / BusApi / Clipboard / Spawn traits + fakes, MsgQueue / Waker, DropEvent, ProximityEvent
ui/host/mod.rs        NativeWindow builder, WindowSpec, WindowKind, Placement, WinEvent, HostHandle, start / shutdown / set_api
ui/host/win32.rs      the UI thread: classes, message loop, window creation per kind, input, frames, proximity poll, focus, device lost
ui/host/present.rs    DirectComposition flip-model swap chain (+ waitable object) or the layered readback fallback
ui/host/dropt.rs      OLE IDropTarget -> View::on_drop
ui/host/sched.rs      frame scheduler (pure)      ui/host/proximity.rs  the island's cursor logic (pure)
ui/host/geom.rs       DPI / hit region / anchoring / resize hit test (pure)      ui/host/focus.rs  blur + watchdog policies (pure)
ui/host/pillwin.rs    PillWin trait (the pill.rs seam) + NativePillWin        ui/host/testing.rs  headless harness (WARP)
ui/host/demo.rs       DemoView + the tests below
```

## 1. Threading rules

* ONE UI thread (`pill-ui`) owns every native window, the Direct3D / Direct2D device and every view. It is started by `host::start()`
  (idempotent, first call) and runs a classic Win32 loop. It is per-monitor-DPI-v2 (`SetThreadDpiAwarenessContext`) and an OLE STA.
* Views are built ON that thread by a factory (`NativeWindow::spawn(&host, |cx| Box::new(MyView::new(cx)))`), so a view does not have to be
  `Send` and may hold `Rc`, glasskit controls, text layouts.
* Nothing touches a view from another thread, ever. Other threads use exactly two handles, both mailboxes drained by the loop:
  * `HostHandle` (`Clone + Send + Sync`): `spawn_with`, `open`, `show`, `show_below`, `hide`, `toggle`, `close`, `set_bounds`, `relayout`,
    `force_ignore`, `set_theme`, `request_frame`, `activate`, `post`, `waker`, `set_lang`; queries `hwnd`, `is_visible`, `backend`,
    `open_windows`, `monitors`.
  * `Waker` (`Clone + Send + Sync`, one per window): `post(Msg)` / `post_custom(v)`. The message arrives in `View::message` on the UI thread.
  Commands and messages share one queue, so `spawn` followed by `post` keeps its order. The queue wakes the loop once per burst (a posted
  message to a hidden message-only window, not a thread message: it survives Win32's modal move/size loops), is bounded (8192) and
  coalesces hot feeds (`Cx::subscribe_latest`).
* `draw` never blocks and never calls the backend. `Cx::call` is for in-memory commands only; anything slower is `call_async`.

## 2. How a view reaches the backend

```rust
// UI thread, inside View::event / on_show / message:
let page = cx.call("settings_get", json!({ "key": "size" }))?;            // blocking, fast commands only
let t = cx.call_async("cmd_query", json!({ "text": q }));                // reply: Msg::Reply { ticket: t, result }
cx.subscribe("media:changed");                                           // Msg::Event { name, payload, .. }; dies with the window
cx.subscribe_latest("audio:bands");                                      // bursts collapse to the newest payload
let w = cx.waker();                                                      // for your own worker threads: w.post_custom(MyResult { .. })
```

* `Api` (`ui/view.rs`) is `call(caller, name, args) -> Result<Value, String>`: command name = the `#[tauri::command]` name from the
  contract, `caller` = the window label (`WindowSpec::label`). The real implementation is the `ui::api` facade (being written
  separately); wire it ONCE at startup with `host::set_api(Arc::new(facade))`. Until then every call answers `Err("no backend wired ...")`.
* Events come from `ui::bus` (`GlobalBus`); handlers run on the emitting thread and only post to the window's `Waker`.
* Tests use `Services::fake()`: `RecordingApi` (records calls, answers from a table), `FakeBus` (`emit`), `MemClipboard`, inline jobs.
  See `view::tests` and `host::testing`.

## 3. Window kinds

| kind | style | for | placement default | notes |
|---|---|---|---|---|
| `Overlay` | `WS_POPUP`, `TOPMOST TOOLWINDOW NOACTIVATE LAYERED NOREDIRECTIONBITMAP` | the island | top centre of the primary monitor, below a top taskbar, 720x120 | click-through outside the view's hit region, proximity events, optional drop target, keyboard only on `cx.set_focus(true)` |
| `Popup` | `WS_POPUP`, `TOPMOST TOOLWINDOW NOREDIRECTIONBITMAP` | command bar | top centre of the monitor under the cursor, 7 px down, 560x350, created hidden | activatable: `show` takes the foreground; `hide_on_blur`; `show_below(y)` hangs it under the island; `toggle` swallows the click that just blurred it |
| `Normal` | `WS_OVERLAPPEDWINDOW` minus visible chrome, `NOREDIRECTIONBITMAP` | settings | centred, 840x760, min 700x520, opaque `#111013` | see "Chrome decision"; geometry memory via `WinEvent::Geometry` + `Placement::Restore` |
| `Fullscreen` | `WS_POPUP`, `TOPMOST TOOLWINDOW NOREDIRECTIONBITMAP` | capture overlay, one per monitor | covers exactly the monitor (`Placement::Cover(MonitorSel)`) | opaque black, crosshair cursor, takes the keyboard, hidden from screen capture, closes itself 1.5 s after losing the foreground |

**Chrome decision (Normal).** The window keeps `WS_THICKFRAME | WS_CAPTION | WS_SYSMENU | WS_MIN/MAXIMIZEBOX` but the host answers
`WM_NCCALCSIZE` with "client = the whole window" (and trims the overhang when maximized), `WM_NCACTIVATE`/`WM_NCPAINT` with "nothing to
draw", and `WM_NCHITTEST` with the 6 px resize edges plus the view's caption region (`cx.set_caption_region(rects)` -> `HTCAPTION`).
So the VIEW draws the title bar and the min / maximize / close buttons (`cx.window_cmd(Minimize | ToggleMaximize | DragMove)`,
`cx.close_self()`), exactly as `settings.html` does today (it already runs `decorations(false)`), while the SYSTEM still provides Aero
Snap, the DWM shadow (`DwmExtendFrameIntoClientArea`), Win11 rounded corners, min / max animations and Alt+F4. A system frame was
rejected: it cannot be themed to the dark `#111013` look and would need a second visual language. Caption-region rectangles must not
cover the view's buttons (clicks there would drag the window).

Builder: `NativeWindow::{overlay, popup, normal, fullscreen}(label)` then `.size .min_size .placement .restore(geometry) .hidden .activate_on_show
.hide_on_blur .focus_watchdog_ms .accept_drops .exclude_from_capture .topmost .resizable .fps_cap .title .theme .clear .singleton .on_event`;
finish with `.spawn(&host, factory)` or `.register(&host, "name", factory)` (then `cx.open_window("name", args)` / `host.open("name", args)`;
a singleton that is open is shown, focused and gets `Msg::Open(args)`).

`WinEvent` (`on_event`, runs on the UI thread, keep it a flag flip or a channel send): `Created, Shown, Hidden(Requested|Blur|FocusLost),
Closed, Geometry(Geometry), Blur`. It is how `ui/lazy.rs` (`note_hidden` / `note_shown`) and settings' size memory plug in.

## 4. Rendering and pacing

* **Composition (default).** `WS_EX_NOREDIRECTIONBITMAP` window + DXGI flip-model swap chain (`FLIP_SEQUENTIAL`, 2 buffers, premultiplied
  alpha, `FRAME_LATENCY_WAITABLE_OBJECT`, max latency 1) on a DirectComposition visual. DWM blends it with the desktop; no readback.
  Direct2D draws into the back buffer (`glasskit::Canvas`, logical px, transform = DPI scale), `Present(1)`.
* **Pacing.** A window that wants frames makes the loop wait on its swap chain's waitable object: one signal per vblank, so frames run at the
  display's refresh rate (60 / 75 / 144 / 240 Hz...), driven by the display, not by a timer. Nothing in the host caps the rate. The only
  throttles: `fps_cap` / `cx.set_fps_cap` (a caller's explicit choice), and a runaway guard that engages only if frames arrive closer
  than half a refresh period (an off-screen / covered / asleep swap chain that stopped pacing; it can never fire on a healthy window).
* **Idle = zero.** A window renders only after input that returned `redraw`, a `Msg`, `cx.request_frame()`, or while the last `step`
  returned `true`. Settled windows are not even in the loop's wait set; the loop then sleeps on messages (the cursor poll of an Overlay
  aside, see 5).
* `dt` for `step`: real time since the previous frame during an animation (clamped to 100 ms), ONE nominal frame after an idle gap.
* **Reduced motion.** `cx.reduced_motion` = Windows "show animations" off OR the app setting (`host::set_reduce_motion_setting(bool)`,
  call it from the `reduceMotion` setting). glasskit `Spring` / `Tween` follow the same flag; views must not add motion on top.
* **Device lost.** `Present` / `EndDraw` reporting removed / reset / hung / `RECREATE_TARGET`: the host rebuilds `Gfx`, the text cache and
  every swap chain and requests a frame everywhere; views are untouched.
* **Fallbacks.** `PILL_FORCE_WARP=1` software device (hardware failure falls back to WARP by itself). `PILL_FORCE_LAYERED=1` (or a failed
  composition set-up, per window) uses a layered window + DIB + `UpdateLayeredWindow` (the `mswidgets` method), paced by a display-period
  timer. `host.backend(id)` says which one a window got.
* Occluded (`DXGI_STATUS_OCCLUDED`): the window retries in 100 ms instead of spinning. The first frame is drawn before a window is shown
  and once more right after, so it never appears empty.
* The modal move / size loop of Windows blocks our loop; a 4 ms `WM_TIMER` (the OS clamps it to ~10 ms) keeps the window painting while
  dragging or resizing, so animations run at ~100 Hz there. That is an OS limit, not a cap.
* Diagnostics: `PILL_HOST_TRACE=1` prints the loop's sleeps, wake reasons and dispatched messages to stderr.

## 5. The island: click-through, proximity, drops

`pill.rs` toggles `WS_EX_TRANSPARENT` from a cursor thread because a click-through window gets no mouse messages. The native Overlay does
the same on the UI thread, but as a pure, tested machine (`proximity.rs`, ported with its numbers):

* the view declares what takes clicks: `cx.set_hit_region(rects)` (logical px); `HitRegion::All` (the default) means a normal window;
* the host polls the cursor (display period floored at 6 ms near the island, 30 ms when it rests, 100 ms far) and sets click-through when the
  cursor is more than 4 px from every rect; `WM_NCHITTEST` answers `HTTRANSPARENT` outside the region too (that part only helps clicks that
  would land on another window of the SAME thread; the style toggle is the cross-process mechanism);
* events to `View::on_proximity`: `Hover` (once per visit; never while a button held since before arrival = a drag; none right after a drop
  on the island), `Leave` (250 ms after the cursor left), `Zone(Some(pos)|None)` (window frame or 120 px around the region),
  `TopEdge` (120 ms on the monitor's top row inside the island's x range, no button), `Gaze(pos)` (`cx.set_gaze(true)`: cursor anywhere,
  physical px relative to the window, only when it moved);
* `cx.set_pass_through(true)` = the `passThrough` setting (whole window click-through, no hover);
* `cx.set_drop_zone(Some(rect))` + `accept_drops()`: a left-button drag that started elsewhere and enters the zone makes the window
  hit-testable (hysteresis +16 px) so OLE can deliver `DropEvent { phase: Enter|Over|Leave|Drop, pos, payload: Files|Text }`; accept an
  `Enter`/`Over` by returning `Response::consumed()`.
* `HostHandle::force_ignore(id, Some(b))` lets the old cursor thread keep deciding (the `PillWin::set_ignore` seam); `None` releases it.
* The Overlay is `NOACTIVATE`; `cx.set_focus(true)` removes it, takes the foreground (`AttachThreadInput` borrow, as `pill_activate`) and
  remembers the previous foreground window; `set_focus(false)` restores both. `Response::focus` is ignored: ask explicitly.

`PillWin` (`pillwin.rs`): `hwnd, monitor_rect, scale, set_size, set_bounds, set_visible, set_ignore, release_ignore`, physical px. `NativePillWin`
implements it over a `HostHandle` + `WindowId`. `pill.rs` is untouched; a later change swaps its `st().hwnd` global for a `Box<dyn PillWin>`.
`pill::layout` becomes `Placement::TopCenter { offset_pct, below_taskbar: underTaskbar }` + `host.relayout(id)` (the guard's 500 ms poll).

## 6. Focus, popups, input

* Popup: `show` places (monitor under the cursor), draws, shows without activation, then takes the foreground. `hide_on_blur` hides on
  `WM_ACTIVATE(WA_INACTIVE)` except in the first 400 ms after a show (our own focus hand-over); `toggle` ignores a show request within
  300 ms of a blur-hide (`focus::BlurGuard`, from `command.rs`). `WinEvent::Blur` fires first, then `Hidden(Blur)`.
* Fullscreen: `focus_watchdog_ms` closes the window after that long out of the foreground (`focus::FocusWatch`, from `capture.rs`).
* Input is translated by `glasskit::input::win32::Translator`: mouse (move / buttons / double-click counting / wheel with `precise` for
  touchpads, `WHEEL_NOTCH_PX` per notch), `WM_MOUSELEAVE` via `TrackMouseEvent`, keys with modifiers, `WM_CHAR` (surrogates joined,
  controls dropped), focus. `Response { capture }` -> `SetCapture` (released on mouse-up). Cursor: `Response::cursor`, `cx.set_cursor`
  (`CursorShape` adds `Crosshair`, `Move`, `Wait`); a Fullscreen window defaults to the crosshair. IME: the `Char` path is complete; the
  candidate window follows `cx.set_ime_rect(caret)`; no inline composition string yet. Tooltips are the view's job.

## 7. DPI

Everything above the host is logical px. Each window carries its monitor's scale (`GetDpiForWindow`), `Cx::scale` / `View::on_dpi`
follow `WM_DPICHANGED` (the suggested rect is applied; Overlay / Popup recompute their size from the logical size and keep the top centre;
Fullscreen stays glued to its monitor), the swap chain is resized to the client area, hit regions and the proximity engine rescale.
Placement is computed per monitor with that monitor's scale before the window exists, so a window is born at the right size.

## 8. Lifecycle, `ExitRequested`, `ui/windows.rs`

* `main.rs` answers `RunEvent::ExitRequested { code: None }` with `prevent_exit()`: closing the last WebView window never ends the app.
  Native windows are invisible to Tauri (they neither keep it alive nor block `app.exit(0)`). On `app.exit` call `host::shutdown()`
  first (destroys the windows, stops the thread, waits up to 2 s) or let the process end. Do not call `start()` after `shutdown()`.
* `ui/windows.rs` (`build_pill`, `build_bar`) keeps building the WebView windows; a native build (flag `ui::flags`) skips them and spawns
  `NativeWindow::overlay("pill")` / `popup("bar")` instead. `ui/lazy.rs` idle teardown maps to `close` / `spawn` (native windows are cheap
  to keep: a hidden Popup costs a swap chain, no renderer process).
* `host.open_windows()` counts live native windows for a future "exit when none left" rule.

## 9. Testing views headlessly

```rust
use crate::ui::host::testing::{render_view, drive, Sim};
let img = render_view(&mut MyView::new(), (400.0, 200.0), 2.0, &Theme::pill_dark());   // WARP, premultiplied BGRA; img.at(x, y), img.count(..)
let mut sim = Sim::new(Box::new(MyView::new()));                                       // same FrameState + Cx as the real host
sim.send(&InputEvent::MouseMove { .. });  assert!(sim.run(600) > 0);  assert!(!sim.frame.wants_frame());   // animated, then idle
let fx = sim.take_fx();                                                                // what the view asked of the host (focus, close, hit region...)
sim.send_proximity(&ProximityEvent::Hover);  sim.send_drop(&DropEvent { .. });  sim.deliver(Msg::Open(json!({})));
let (svc, fakes) = Services::fake(); let mut sim = Sim::with_services(view, svc);      // fakes.api.calls(), fakes.bus.emit("evt", json)
```
Tests that flip glasskit's process-wide reduced-motion override must hold `testing::motion_lock()`. `PILL_HOST_DUMP=<dir>` makes the demo
test write PNGs (`testing::save_png` for your own).

## 10. Running the host by hand (the lead)

`cargo test --release` runs the headless tests, 56 of them new (335 in all; scheduler, proximity, geometry, focus, queue, Cx, harness, placement, styles).
The `#[ignore]`d tests create real windows; run EACH in its own invocation (the host is a process-wide singleton):

| command | what it does | visible? |
|---|---|---|
| `cargo test --release -- --ignored --nocapture host_smoke_hidden` | creates the 4 kinds hidden, checks handles + backend, posts, closes, shuts down | no |
| `cargo test --release -- --ignored --nocapture host_pacing_offscreen` | animates an Overlay placed at (-30000, -30000) for 2 s, asserts idle = 0 frames and the runaway guard | no |
| `set PILL_PACING_VISIBLE=1` then the same | the window sits 60 px inside the primary monitor for 3 s and asserts fps within +-10 % of the refresh rate | a small box |
| `cargo test --release -- --ignored --nocapture host_demo` | one window of each kind (Overlay, Popup, Normal, Fullscreen); `PILL_DEMO_SECS` (default 120) ends it; Esc closes the focused one | YES, takes over the primary monitor |

Add `PILL_FORCE_LAYERED=1` or `PILL_FORCE_WARP=1` to repeat any of them on the fallbacks, `PILL_HOST_TRACE=1` for the loop trace.

### What to eyeball in `host_demo`

1. **Transparency.** Overlay: a dark rounded card (380x64) at the top centre, below the taskbar; everything around it in the 720x120 frame shows the desktop. No black / white slab, no 1 px border, soft edges anti-aliased. (Slab = composition + `WS_EX_LAYERED` combination failed: repeat with `PILL_FORCE_LAYERED=1` and report.)
2. **Click-through.** Park a browser / Explorer window under the transparent part of the frame: its buttons / tabs click normally; the card itself takes clicks. Hover the card: it lightens within a frame; leaving darkens it ~250 ms later. Cursor 100 px below the card still updates `zone x,y`. Pin the cursor on the monitor's top row above the card: `top edge xN` counts up every 120 ms.
3. **No focus steal.** Typing into another app keeps working while the cursor is over the card. Click the card, type: `typed "..."` echoes; Esc closes it.
4. **Drops.** Drag a file from Explorer from far away to the top area of the frame (y < 90): copy cursor, drop shows `1 file(s): C:\...`; drag selected text too. Dropping must not pop `hover`.
5. **Popup.** Appears top-centre of the monitor under the cursor, 7 px down; typing echoes; clicking another window hides it (log `Hidden(Blur)`); it does NOT hide in its first 400 ms; the scripted toggle at 6 s hides it, and a hotkey-style second toggle within 300 ms of a blur-hide must not re-open it.
6. **Normal.** Opaque `#111013`, no system caption, DWM shadow, rounded corners on Win11. Resize from all 8 edges / corners (6 px), drag by the top 36 px strip, double-click it to maximize, snap to screen edges, snap layouts via Win+Z; minimum 700x520; the window keeps painting during drag / resize (~100 Hz is the OS limit); "close" at the top right closes; maximized it must not overhang the monitor.
7. **DPI mix.** Drag the Normal window and the Overlay between monitors of different scales: the title line shows the new `N.NNx`, text stays crisp, size follows without a jump, hit region stays on the card.
8. **Pacing (needs a 240 Hz panel).** `host_pacing_offscreen` with `PILL_PACING_VISIBLE=1` must report ~240 fps on the 240 Hz monitor (and ~display Hz elsewhere). In `host_demo` move the pointer in the Normal window: the dot glides; stop: `frames drawn` stops increasing and the test process sits at 0 % CPU in Task Manager (the Overlay's cursor poll aside).
9. **Fullscreen.** Black over the primary monitor, crosshair, dragging draws a translucent selection rectangle, Esc closes it. Alt+Tab away: it closes ~1.5 s later. It must NOT appear in a Win+Shift+S / Snipping Tool capture (exclude-from-capture).
10. **Reduced motion.** Turn Windows "Animation effects" off: new windows say `reduced motion`, the dot jumps to the pointer. (On: glides.)
11. **Fallback.** Repeat 1-3 with `PILL_FORCE_LAYERED=1`: same look, click-through works by alpha too.
12. **Device lost (optional).** Win+Ctrl+Shift+B (driver reset) while the demo runs: windows must come back after a flicker.

## 11. Known gaps

* Click-through is the polled style toggle (latency <= one poll, 6 ms near the island); `HTTRANSPARENT` alone cannot cross processes. A
  raw-input (`RIDEV_INPUTSINK`) wake-up could replace the far 10 Hz poll later.
* `WS_EX_LAYERED` together with `WS_EX_NOREDIRECTIONBITMAP` and a DComp visual is what the Overlay uses for the style toggle; it is the one
  combination only a real screen can confirm (checklist 1-2). The layered fallback exists for exactly this.
* No UI Automation tree (screen readers), no touch / pen (`WM_POINTER`), no inline IME composition string, no system tooltips.
* A view-drawn title bar has no Win11 snap-layout flyout over its maximize button (needs `HTMAXBUTTON` for a declared region).
* One host per process (global singleton, no restart after `shutdown`). Windows are capped at ~60 animating at once (wait-set limit).
* Default GPU adapter only; no per-monitor adapter choice. Exclusive-fullscreen games hide topmost windows (the guard handles that).
* Hit regions are rectangles (pad 4 px); rounded hit shapes are the view's job (`glasskit::input::hit_rounded`).
