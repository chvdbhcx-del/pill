# Native panel set B (`src-tauri/src/ui/tabs/b`)

The native views of the Note (with the timer ring and the reminders), Text, Chat and Agents tabs and of the two overlays `ai_actions` and
`ai_setup`. Models are the pure ones in `ui-core/src/panel/b/*`; the views own them, run their `BackendCall`s through `Cx::call_async`, feed
the replies back with `on_reply`, subscribe to the backend events they handle, drain `take_effects()` / `take_anims()` and paint with glasskit.

```
b.rs            create(id) -> Option<Box<dyn View>>  (note text chat agents)    create_overlay(name)  (ai_actions ai_setup)
b/kit.rs        Calls (ticket -> tag), Clicks (hover / press / click), Cues (AnimCue player), Icons, button / chip / card / flash painters,
                Field (TextInput with deferred input), route_effects, lang_of, ms_of, snap::shoot (test PNGs)
b/fx_b.rs       TabFx + emit() / take(): what the island must do for a tab (cards, live rows, badge, collapse, voice, Settings page, overlay closed)
b/note.rs       NoteScreen   b/note_doc.rs  HTML <-> document   b/editor.rs  rich editor   b/timer_card.rs  TimerCardView (ring + reminders)
b/text.rs       TextScreen   b/chat.rs + chat_pop.rs  ChatScreen, history + AI settings popovers
b/agents.rs     AgentsScreen b/ai_actions.rs  AiActionsScreen (in-panel overlay over the clipboard tab)   b/ai_setup.rs  AiSetupScreen
```

## How the lead wires it
* `ui::tabs::b::create(id)` already matches `ui::tabs::create`'s contract. Overlays: `create_overlay("ai_actions")` (fills the 648 x 260 body,
  opened with `Msg::Open({"id": clipId})`, paints nothing while closed, Esc closes it: route keys to it first while it is open) and
  `create_overlay("ai_setup")` (fills any rect, designed for the Settings content column).
* The views build themselves lazily: models are mounted on the first `on_show`. They use the `Cx` the island hands them: **the island must
  forward `Msg::Event` / `Msg::Reply` / `Msg::Custom` to the tab view** (subscriptions made through a tab's `cx` arrive in the island's window),
  `on_show` / `on_hide`, and for the chat `on_drop`.
* Island effects: after every call forwarded to a tab, drain `ui::tabs::b::take_fx()` (`TabFx`). TABS-A defines its own `TabFx` in `ui/tabs/fx.rs`;
  the lead merges the variants (`Card SetLive React Unreact EdgePulse SetBadge CollapsePill VoiceToggle GoPage OverlayClosed`) and swaps the
  body of `fx_b::emit` for the shared sink.
* Text input is queued by `event()` and applied by the next `draw` (a `View::event` has no `Canvas`): the text is on screen in the same frame,
  the model sees it one frame later. `step` returns true while input is queued.

## Status against the web version (headless snapshots, `tools/dev/tabs-b-web.mjs` vs `kit::snap::shoot`, compared with `tools/dev/tabs-b-diff.mjs`)
Dark, 1x: mean channel difference 0.4 - 8 % (anti-aliasing alone ~0.5 %; most of the rest is different sample text / language, not layout).
* **Note**: chips, toolbar, editor, saved check, right card match in position and size. Bold is lighter than the web (glasskit `SpanStyle` has no weight, 600 over a 500 base vs 700).
* **Timer / reminders**: ring, buttons, presets, rows, hint, input match; rows enter / collapse / FLIP.
* **Text**: layout identical; the QR card is 190 px like the web canvas. Text content differs (fixtures), fonts match.
* **Chat**: header, bubbles, composer, popovers match within a pixel or two; answer text is 13 px / 19 px lines where the web has 14 / 22 and the code block header and padding are tighter: both are `MarkdownView` metrics of glasskit. The streaming snapshot shows the top of the answer (the live view follows the stream; the headless render has no step).
* **Agents**: list, state colours, pulse; same layout.
* **ai_actions**: menu, working, result, error; a few pixels of spacing difference in the buttons row.
* **ai_setup**: the whole page is built; the web page has its own Settings chrome (627 x 728 capture), the native one is the content column only, so the pictures differ in size.

## Tests
`cargo test --release`: 514 pass (399 before + 115 here: kit 10, note_doc 10 + editor 13 + note 10, timer_card 11, text 11, chat 22, agents 10, ai_actions 4,
ai_setup 14, fx / registry 2). Each view has wiring tests over `Services::fake()` / `testing::Sim` (calls, tags, events, effects, cues, idle = no frames
once settled), snapshot tests for dark + light at 1x + 2x and one reduced-motion pass. PNGs: `PILL_SNAP_DIR` (default `target-agent/snap/native`).
Scenarios shared with the web harness: `tools/dev/fixtures/tabs-b/*.json`.

## Editor: what is supported
The web editor is a contenteditable with the whitelist `b i s mark ul ol li div br a input[checkbox]`; the native editor covers exactly that.

| feature | native |
|---|---|
| bold / italic / strike / highlight (`mark`) | on a selection, or pending at the caret; Ctrl+B / Ctrl+I; toolbar toggles |
| links | existing ones kept, underlined, Ctrl+click opens (`chat_open_link`); cannot be created (neither can the web editor) |
| lines | plain, bullet, numbered (levels kept and drawn), checklist (click the box; Enter continues; Enter on an empty item ends the list; Backspace at the start makes the line plain) |
| soft break | Shift+Enter (`<br>`) |
| editing | typing incl. Cyrillic / AltGr, Backspace / Delete (Ctrl = word), Ctrl+A/C/X/V (paste = plain text), Ctrl+Z / Y / Shift+Z (200 steps, typing coalesced) |
| mouse | click, drag, shift-click, double-click word, triple-click line, drag past the edge scrolls, scrollbar |
| not supported | underline, headings, colours, rich paste, IME composition string (finished text arrives as `Char`), text drag-and-drop, spell-check |

## How it round-trips to the stored HTML
* Opening a note never writes it: `NoteTab` keeps the loaded (sanitized) HTML and the view sends `Action::EditorChanged` only after a user edit.
  Navigation, selection, scrolling, clicking a link, focus changes: no save. Tested (`mount_loads_..._without_saving`).
* After an edit the document is written by `Doc::to_html()` in the shape Chromium writes: bare first line, `<div>` lines (`<div><br></div>` when
  empty), consecutive list lines share one `<ul>` / `<ol>`, checklist lines `<input type="checkbox">&nbsp;text` (`checked=""` when ticked), formatting
  nests `<a><b><i><s><mark>` with minimal tag changes. Chromium-written samples round-trip byte for byte; any other HTML round-trips semantically
  (`from_html(to_html(d)) == d`), 33 samples in `note_doc::tests`.
* Normalisations (invisible in the web view): a literal `\n` in a text node becomes `<br>`; nested `<div>` becomes sibling lines; the nbsp after a
  checkbox is always written; empty inline elements vanish; fixed tag nesting order. A checkbox that is not the first thing of a plain line is kept as
  an inline object (`☐` / `☑`, flag `CHK`) so no markup is lost.

## Wiring
* `View::event` has no `Canvas`, so editor input is queued and applied by the next `draw` (same frame as the paint). `step` returns true while input
  is queued, so the result (`EdOut::Changed`, ...) is drained one frame later.
* The caret blinks for 12 s after the last input and then stays solid (a view cannot schedule a wake, so a permanently blinking caret would keep
  frames coming for as long as the editor is focused). Reduced motion: solid caret, no pop on the checkbox, no fades.
* Prefs `pill.note.cur` / `pill.timer.sel` live in `%APPDATA%\pill\ui-prefs.json` (`PrefsFile`, one map shared by both models).
* Subscribes: `pill:hide pill:show pill:open-tab remind:due remind:changed timer:tick timer:done settings:changed`; the island must forward `Msg` to the view.

## Note: only the lead can verify on screen
Typing feel and latency (input is applied at draw), IME (candidate window position follows the caret, no composition string), selection with the mouse
at 125-200 % DPI, scroll glide in a long note, the caret blink pace, Ctrl+click on a link, the checkbox pop.

## Only the lead can verify on screen (all views)
* Typing feel and latency everywhere (one-frame deferral), IME composition (only the candidate window position is set, no inline string),
  selection feel while the chat streams, chat follow-the-stream at 144 - 240 Hz, wheel / touchpad scrolling in every list.
* Chat: the file dialog (compile-checked only), dropping files onto the island, copy button hover + "Copied" fade, the model dropdown in the settings popover (clipped by its panel).
* Text: the tool buttons on a real selection in another font size, QR crispness at 125 - 200 % DPI.
* ai_setup: every download / install flow (nothing was run against a real server; downloads only start from an explicit click).
* ai_actions: Paste into the app below (focus hand-back timing).
* The `Msg` forwarding and `take_fx` draining by the island.

## Gaps
No selection of the result text in ai_actions (Copy takes all); chat selection is within one message; bold weight; chat markdown metrics (glasskit);
caret blink stops after 12 s in the note editor (a view cannot schedule a wake: `Cx::wake_in` would fix it); `NoteTab::mount` issues `timer_state` twice (model behaviour).
Kit additions worth promoting: `Cues::is_playing`, a transparent `Field` mode, a local-time-zone helper, `PrefsFile` (in `note.rs`), a horizontal chip scroller.
