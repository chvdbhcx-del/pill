# Native command bar (`src-tauri/src/ui/bar_view`)

> **Since 0.7.0 there is no web (WebView2) UI.** The native UI described here is the only one: the `ui.native.*` keys, `--native`, `--web-ui`, `PILL_NATIVE` / `PILL_WEB_UI`, `ui/flags.rs`, `ui/lazy.rs` and `ui/windows.rs` are gone, and every gate / "flag off" / "web" comparison below is history (v0.6.1 is the last release that still contains the web code).

The native replacement of `bar.html` + `bar.js` + `bar-ready.js` + `uninstall-ui.js` + `md-lite.js`. `BarView` is the `View` of the host Popup window
`bar` (`docs/native-host.md`). All behaviour is the pure model `pill_ui_core::bar` (`Bar::on_event / tick / view`, 185 tests); this module translates
host input into `BarEvent`s, executes the model's `Action`s and draws its `BarView` view-model with glasskit.

```
mod.rs     BarView: View impl, event routing, action executor, hover/press tweens, flex layout (`flow`), measuring canvas for the text box
paint.rs   theme, body layout, search row, answer (MarkdownView), result list, message, context menu, liquid outline (ui-core wave -> geometry)
panel.rs   uninstall panel (flow layout + paint of every step page), status strip
anim.rs    plays the model's `Anim` cues (enter / exit / collapse / expand / pop / flash / shake / draw-tick) from glasskit's CSS curves
keys.rs    InputEvent -> DOM-like KeyEvent, "does the bar consume this key", panel focus order (pure, tested)
icons.rs   web line glyphs via glasskit `Svg` (glyphs.rs = src/icons.js verbatim) + shell icons (PNG data URL -> premultiplied bitmap, per device)
native.rs  popup spec, lazy window, `bar_toggle / bar_hide / bar_is_visible`, `ui::api` facade wiring, OS light-theme probe
tests.rs   key routing, typing-during-uninstall invariant, gates, idle, headless snapshots
```

## Flow

* `on_show` -> `OsReduceMotion`, `BodyWidth`, `Shown`, `Visibility(true)`; `on_hide` -> `Visibility(false)`, `Hidden`. The host's show / hide replace
  `bar:shown` / `bar:hidden`. The view subscribes to `voice:state`, `uninstall:state|item|focus` and `ask:chunk` (forward compatible, nothing emits it
  yet) in `BarView::new` and then calls `cmd_ready` (what `bar-ready.js` did).
* `Action::Call { ticket, call }` -> `Cx::call_async(call.cmd, call.args)`; the `Msg::Reply` maps back to `BarEvent::Reply` (`Err` and `null` are both
  the JS wrappers' `null`). `Copy` -> clipboard, `SetInput` -> text box (no echo back), `ScrollToRow` -> `ScrollList`, `Anim` -> `AnimPlayer`.
* Keys: `Key`/`Char` become `KeyEvent`s; the model sees every key first (the uninstall panel's capture handler is part of it), then the text box gets
  what the bar does not consume (`keys::bar_consumes`: arrows / Enter / Esc / Tab / F4 / Ctrl+P / Ctrl+Shift+C / Ctrl+U / PageUp,Down / Ctrl+Home,End;
  caret keys, Backspace, Ctrl+A/C/V/X/Z are the box's). Typing is a `Char`: the model steps the panel aside and asks for the caret, the character lands
  in the box. Space / Enter / Tab / arrows on a focused panel control act on it; a focus ring shows only after Tab.
* Text box = glasskit `TextInput` (single line, 15/22). `View::event` has no canvas, and `TextInput` measures text with one, so the view keeps a private
  software (WARP) canvas on the UI thread for that (`with_measure`); nothing is painted with it. The field's own ring / background are clipped away.
* Frames: while the bar is open and motion is allowed the liquid outline runs every frame at the display rate (as the web `requestAnimationFrame` loop
  did); loops (thinking spinner, progress bars, recording dot) and cues keep frames coming; model timers (80 ms debounce, 3 s confirm, copied labels)
  run inside a 3.3 s keep-alive window after every event, so reduced motion idles after that. Nothing is ever throttled.
* Reduced motion: the model drops its cues, the wave and loops are drawn still (progress at 100 % width / 50 % opacity), glasskit tweens jump.

## Window and gates

`native::spec()` = `NativeWindow::popup("bar")` (560 x 350, top centre of the monitor under the cursor, 7 px down, hidden, activates on show, hide on blur
with the host's 400 ms grace, `toggle` swallows the re-open echo within 300 ms). `command::toggle` hangs it under the island with `toggle_below`.
The `Hidden` window event runs `command::bar_hidden_cleanup` (stop the running query, drop the in-memory rows and the note index).

Gated by `flags::native_bar()` (setting `ui.native.bar`, and never with `--web-ui` / `PILL_WEB_UI=1`); flag off = exactly the old behaviour:

| where | native |
|---|---|
| `command::toggle / hide / bar_visible` | `bar_view::bar_toggle / bar_hide / bar_is_visible` (PREV_FG still stored; `index_apps_async` still runs) |
| `command::cmd_ready` | marks ready, no `bar:shown` replay |
| `ui/lazy.rs get_or_create("bar")` | `None` |
| `ui/windows.rs build_bar` | `Err(WindowNotFound)` |

`bar_view::register(&host)` registers the window under `bar` (for `open_window`); `main.rs` (island agent) may call it. `bar_toggle` creates the window on
first use and wires `ui::api::init` + `host::set_api` itself (both idempotent).

## Tests

`cargo test --release bar_view`. The headless snapshots write PNGs when `PILL_BAR_DUMP=<dir>` is set (`empty results answer u-confirm u-scanning
u-checklist u-deleting u-summary u-strip light scale2 reduced`); `node tools/dev/bar-shot.mjs <dir> --segoe` renders the same scenes from the web page in
headless Chrome (stubbed `__TAURI__`, same fixtures). The uninstall tests use a fake backend only; no real uninstall command is reachable.

## Known differences from the web page / gaps

* Font: native text is the host's `FontSet` (system Segoe UI until `host::set_fonts` is given Medeo Sans); the keycap uses the proportional face (web: mono).
* Answer: glasskit's `MarkdownView` look (13 px body, code block with language header and always-visible Copy button) instead of md-lite's 14 px; the answer
  box therefore scrolls a little earlier.
* No "group header" rows in the list (the web never drew any; Tab only jumps between groups); "Recent" label is drawn when the model provides it.
* Text-box context menu (cut / copy / paste on right click) and IME composition string are not drawn (shortcuts and finished IME text work).
* Wrapped paragraphs use DirectWrite's line spacing, not the CSS 16 / 20 px line heights (heights of the boxes follow the CSS values).
* The keyboard `Selected: N` footer text is one weight (web bolds the numbers).

## Lead's on-screen checklist (nothing here was run against the real app)

1. Set `ui.native.bar` = true, restart; the global hotkey opens a dark bar top-centre 7 px below the top (under the island when it shows), typing works
   at once (caret in the box), the bar pops in; hotkey again closes it; `--web-ui` brings the WebView bar back.
2. Click another window: the bar hides; clicking the hotkey within ~300 ms does not re-open it; Esc closes; a click on the transparent margin closes.
3. Search `calc`, apps with real icons, hover/arrow selection glides, hover buttons on the selected app row (copy path, admin, pin, uninstall, folder),
   right click menu, Ctrl+P pins, Enter on a clipboard row pastes into the previous window.
4. `? question` (with `net.ai` on) shows the thinking spinner, the answer with a code block and a working Copy button, the dialogue rows, Esc ends it.
5. Typing during an uninstall: start the real flow ONLY on a throw-away app. While the uninstaller / scan runs type something: the panel steps aside at the
   first key, the strip shows the job, Ctrl+U or a click on the strip brings the panel back with the ticks kept. (The real uninstall dry-run is the lead's.)
6. Reduced motion (Windows animations off): everything appears at once, the outline is a still curve, the process idles at 0 % CPU after a few seconds.
7. Light theme (`theme = light` / `auto` with a light OS), 150 % / 200 % DPI, a second monitor: the bar opens on the monitor under the cursor.
