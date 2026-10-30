# Note tab (native) — notes for `docs/native-tabs-b.md`

Files: `ui/tabs/b/note_doc.rs` (HTML <-> document, pure), `editor.rs` (rich editor widget), `note.rs` (`NoteScreen`, prefs). The timer ring and the
reminders card are `timer_card.rs` (`TimerCardView`), driven by the same `NoteTab` model.

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

## Only the lead can verify on screen
Typing feel and latency (input is applied at draw), IME (candidate window position follows the caret, no composition string), selection with the mouse
at 125-200 % DPI, scroll glide in a long note, the caret blink pace, Ctrl+click on a link, the checkbox pop.
