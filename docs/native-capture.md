# Native capture overlay (`src-tauri/src/ui/capture_view`)

The Rust replacement of `capture.html` + `capture.js` + `capture.css` (the screenshot / OCR / QR / colour-picker / record-area overlay).
Logic = `pill_ui_core::capture::CaptureModel` (60 tests); this module is the renderer and the wiring. The recording capsule, timer and
result cards are `live` rows of the island, not part of this.

```
capture_view/mod.rs      CaptureView: View. glasskit input -> model Input, Reaction -> backend / toasts / jobs, per-frame animation state
capture_view/paint.rs    the scene (z-order of capture.css): frame, dim + cut-out, handles, size badge, markup, text box, hint, toolbar, tip, marker, loupe, flash, toast
capture_view/tools.rs    Direct2D execution of ui-core DrawCmd (shared with the export), pixelation, text metrics, colour parsing
capture_view/flatten.rs  marked-up PNG for capture_commit: crop + ops on a WARP device, WIC encode to memory, on a worker thread
capture_view/io.rs       CaptureIo for the UI thread (what is sync, what is async, the raw-body job)
capture_view/windows.rs  host windows for capture.rs: create / reuse / hide / close, the gate
capture_view/icons.rs    toolbar icons (inner SVG of capture.js + icons.js, glasskit Svg)
tools/dev/capture-diff.mjs  pixel comparison with the web page (headless Chrome)
```

## Gate (capture.rs)

`capture_view::windows::enabled()` = `flags::native_capture() && !flags::web_ui_forced()` (`ui.native.capture` in settings.json, `--web-ui` /
`PILL_WEB_UI=1` always wins). Flag off: capture.rs runs the WebView code path unchanged. Flag on, three edits in `capture.rs`:

* `open_overlays`: `open_native` creates one host `Fullscreen` window per monitor (`capture-N`, `MonitorSel::Rect` of the monitor, hidden,
  hidden from screen capture, no per-window focus watchdog) or reuses the idle one when the monitor rectangle is unchanged. A NEW window
  arms itself in its factory (`capture_session` + frame + `capture_ready`, then asks the host to show it); a REUSED one gets one
  `capture:begin` on the bus (every native view listens, each arms for its own label; duplicates are ignored by the model's session id).
* `hide_overlays`: emits `capture:end` once; the views hide themselves after their exit animations (copy flash, toolbar fade, a toast, at
  most 420 ms; a cancel hides at once). Backstop: 700 ms later `windows::hide_all()` unless a new session started. After 90 s idle
  `windows::close_all()` (same as the WebView windows' destroy).
* the foreground watchdog (`fg_ok`) asks `windows::foreground_ok()` (any overlay is the foreground window) instead of the WebView list.
  ESC / right click / foreground loss (1.5 s) / the 6 s "overlay did not start" guard / the 600 s failsafe are all unchanged.

`windows::open` also wires the host: `ui::api::init(app)`, `host::set_api(facade)`, the UI language (`language` setting, else the user's
locale) and the Reduce motion setting. Both calls are idempotent, so the lead's own wiring and this one agree.

## Per-overlay behaviour

* **Frame**: `ui::api::capture_frame(label)` (the session's `Arc`) -> premultiplied BGRA -> one `ID2D1Bitmap1` on the host device, drawn 1:1
  (no PNG round trip). Rebuilt when the host rebuilds its device; dropped on hide.
* **Selection**: dim 38 % until a selection exists; then the cut-out (46 % around it), 1 px outline, 8 handles (9 px white dot, 1.5 px accent
  ring) that pop after a pointer selection, size badge (above the box, inside it within 30 px of the top).
* **Markup**: every `DrawCmd` (pen, highlighter, arrow, rect, ellipse, pixelate, text, numbered step) drawn in frame px under a `1/rx`
  transform, clipped to the selection; insert animation (fade + scale); the selected op (select tool, `capture.extras`) gets a dashed outline
  measured with real font metrics. Text editing: dashed box, real caret position, paste (Ctrl+V), IME rectangle follows the caret.
* **Toolbar**: item rectangles from the model; hover / press / active / disabled as in CSS (120 ms colour tween, 0.92 press scale that
  springs back, swatch 1.15 hover spring); tooltips after 480 ms (the web page used the browser's `title`); enter = 120 ms fade + 4 px rise.
* **Loupe** (colour mode): 15 x 15 lens (hex / rgb / hsl / x y), marker, follow ease 90 ms, show fade + scale .94.
* **Exit**: white copy flash over the confirmed area while the dim fades out; toolbar fade.
* **Toasts**: pin has no backend command (`capture_pin` does not exist): the model still saves; the view says "Pin to screen is not
  available yet - the shot was saved". A failed markup export says the plain shot was sent.
* **Animations** all honour Reduce motion (model options + glasskit `Tween` + `cx.reduced_motion`); nothing caps a frame rate. Idle = zero
  frames (tested). The overlay fades its dim in over 140 ms and the hint over 200 ms (not in the web page).
* **Keyboard / mouse**: Esc, right click, Enter, Ctrl+C / S / Z (Ctrl+Y / Ctrl+Shift+Z with extras), double click, arrows in colour mode,
  nudge / Shift-square (default options keep them off: `Options::web_parity()`); `capture.extras = true` in settings.json turns on
  highlighter / ellipse / steps / redo / op editing / pin + shelf buttons.

## Commit

`Commit` -> `capture_commit {label, vw, vh, action, rect | x, y}` through `Cx::call_async`. With markup the model asks for a flatten: a worker
thread renders crop + ops (same `Tools::draw_cmd` as the screen) on a WARP device, encodes PNG with WIC into an `IStream`, then
`ui::api::call_raw_async("capture_commit", <x-pill-args JSON>, label, png)`. If the render fails an empty body goes out and the backend crops its
own copy (no markup). The export is software-only on purpose: a second hardware device created on a worker next to WARP devices hung the
test process; WARP never competes with the UI thread's GPU. `after_save` shelf: the view listens to `capture:done` and calls `shelf_add`.
Safety: if no `capture:end` arrives 1.8 s after a commit / cancel (or the call replies `Err`), the overlay closes itself.

## Cross-monitor

Same as the web version: one independent model per monitor window. A selection lives on the monitor where it was drawn; the other monitors
stay dimmed and take their own pointer events (a click there starts a selection there). The backend commits for the label that sends it.
Mixed DPI: logical viewport = physical / the monitor's scale, `rx = fw / vw` per axis (`MonitorGeom`, `Desktop` in tests).

## Tests (`cargo test --release capture_view`, 41)

Arming / session / show / focus, idle dim + hint, fade-in vs Reduce motion, drag cut-out + badge, handles + toolbar + pop, toolbar placement
flip, cancel (Esc, right click, button), Enter / Ctrl+C / Ctrl+S / double click / record commits with the right JSON, resize and move,
cursor mapping, toolbar states + tip + press + swatch, Russian copy, every markup tool (8 ops, pixels asserted), clip to the selection, op
select + delete, text box typing / paste / Esc, the flattened PNG decoded (size, red stroke, untouched pixels) and a flatten of every command
kind, pixelation maths, copy flash + exit, stuck guard, pin toast, shelf, OCR / QR / record, colour loupe + pick + ease, two mixed-DPI monitors
with negative offsets, 2x DPI, Reduce motion, idle = 0 frames, the export worker next to a live device, key mapping, the gate.
`PILL_CAPTURE_DUMP=<dir>` writes every snapshot as PNG; `web_parity_scenes` + `node tools/dev/capture-diff.mjs <dir>` compare idle / adjusting
/ hover / markup / loupe with the web page: 0.2 - 1.0 % mean difference (anti-aliasing, shadow softness, the lens grid, the pixelate
averaging method; the web's typed text did not commit in the scripted run, a script artefact).

## Known gaps

* The host has no diagonal-resize cursors (corner handles show the four-way arrow) and no hidden cursor (colour mode shows the crosshair under
  the marker instead of `cursor: none`).
* The model's op hit-test estimates text width (0.6 em); the view measures real width only for drawing and the selection outline.
* No `capture_pin` backend: toast only. No cross-monitor selection sync (as in the web version). Tooltips are drawn by the view.
* Not verified on a real screen: see the checklist.

## Checklist for the lead (needs eyes; set `ui.native.capture` true, restart, hotkey for area capture)

1. **Freeze timing**: press the hotkey while a video / clock plays: the picture must stop at once, no empty black window first, no flash of
   the desktop between the freeze and the overlay. First capture after start creates the windows (slower), the second reuses them.
2. **Multi-monitor**: every monitor dims; start a selection on monitor 2, then click on monitor 1 (a new selection there, keyboard follows
   the click); commit from either; the result is the monitor that committed. Unplug / re-arrange a monitor between captures: windows
   are recreated.
3. **Mixed DPI**: 100 % next to 150 %: selection size badge equals physical pixels, handles / toolbar are the same logical size, the saved
   PNG has the physical size of the selection, text is sharp on both.
4. **Fullscreen apps**: capture over a borderless-fullscreen game / video: overlay appears on top (topmost); over exclusive fullscreen it may
   not (host gap).
5. **Hidden from Snipping Tool / OBS**: with the overlay up press Win+Shift+S or start a screen recording: the overlay must not be in it.
6. **Keyboard**: Esc, Enter, Ctrl+C, Ctrl+S, Ctrl+Z, arrows (colour mode); the overlay under the cursor has the keyboard; clicking the other
   monitor moves it. Alt+Tab away: the overlay closes ~1.5 s later.
7. **Markup**: each tool, undo, text typing (also Cyrillic), paste, colour swatches; copy and paste into Paint: markup present, size right.
8. **Colour mode**: lens follows smoothly, hex / rgb / hsl match, Enter / click copies.
9. **Animations**: handles pop, toolbar rise, hover / press, copy flash; with Windows animations off or `reduceMotion` they are instant.
10. **Flag off** (`ui.native.capture` false or `--web-ui`): the WebView overlay behaves exactly as before.
