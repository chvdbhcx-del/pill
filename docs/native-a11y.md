# Keyboard-first use and accessibility (`ui/a11y`, `island/view_kbd.rs`, `musicwidget/view_kbd.rs`, `host/uia.rs`)

Three things share one model: **keyboard mode** (the island card and the music widget driven by keys, with an animated focus ring),
**UI Automation** (screen readers see the buttons, tabs, sliders and list items, not only the window) and **high contrast** (system colours,
no glass). A fourth, small one lives in the command bar: prefix templates and bookmarks.

## What was there before (the audit)

| area | state in 0.9.0 |
|---|---|
| global hotkeys | `hotkeys.rs`: every action of `settingswin::ACTIONS` + `launcher.<id>`; plugin first, raw `RegisterHotKey` fallback; conflict check (`hotkey_check`); the recorder control `Control::Hotkey` on the Hotkeys page |
| UIA | `host/uia.rs`: ONE read-only pane per window, named by `View::accessible()` ("Pill, 12:30"); no children, no patterns, no focus |
| focus | `Cx::set_focus` (`pill_activate`): a tab asks for the keyboard for a text field; the host remembers the user's window and gives it back; Esc / focus-out collapse the card. No Tab order, no focus ring on the island (glasskit's `FocusChain` / `focus_ring` exist, the tab views use them inside their own controls) |
| music widget | mouse only (hover, click, drag the bar), a pane named "Music: title - artist" |
| high contrast | none (`reduceMotion` is the only accessibility switch; the theme is dark / light / a chosen colour) |
| command bar | prefixes `c n f a s =`, `>`; the web search row (`search.engine`, `search.customUrl` with `{q}`) |

## The model (`ui/a11y/nav.rs`, pure)

`View::controls() -> Vec<Control>` lists what a keyboard or a screen reader can reach: `{ id, name, role (Button | Tab | Slider | ListItem |
CheckBox), rect (window logical px), value (Toggled | Selected | Range { min, max, cur, step }), enabled }`. Views build it from the rectangles
they hit-test with, so a new hit region is a new control by adding one line. `View::control_act(id, Act)` acts (`Invoke`, `Toggle`, `Select`,
`Set(v)`, `Step(n)`), `View::focused_control()` says where the keyboard focus is.

* `reading_order` (rows top to bottom, left to right), `tab_step` (Tab / Shift+Tab, wraps), `move_dir` (arrows: a control clear of the current one
  along the arrow that overlaps it across the arrow wins, else the nearest by distance + 2 x offset), `nth_tab` (Alt+N), `neighbour_tab`;
* `route(controls, focus, key) -> Focus(id) | Act(id, act) | Close | Pass`: Tab walks, Home / End jump, Left / Right on a tab switch tabs, on a
  slider they step it, elsewhere they move; Up / Down move; Enter / Space press (a slider ignores them); Esc closes; Alt+1..9 jump to a tab.
  `nav_key` maps only navigation keys (no Ctrl / Win chords, no letters, so typing and media keys are never touched).

## Keyboard mode

`hotkey.keys` (default **Win+Alt+J**, stored as `Alt+Win+J`; rebind it with the recorder on the Hotkeys page). `islandkeys::keyboard_mode`
walks the targets (`ui/a11y/target.rs`): first press = the island, second = the music widget (only while it is on screen), third = off. It opens
the card (`pill::expand`) and emits `a11y:keys { target }` on the bus; each view takes or drops the keys.

**Island** (`island/view_kbd.rs`): the view asks the shell for the keyboard exactly as a text field does (`TextFocus(true)` -> `pill_activate` ->
`Cx::set_focus`): the window becomes the foreground window, the user's window is remembered, and nothing else changes. The focus ring appears on
the selected tab. Controls: the tab strip, the header buttons (dictation, pin, settings, VPN chip), a toast card's buttons, a row's buttons, the
desktop chip, and the open tab's own controls (`TabHost::controls`; Home lists its player buttons, seek bar, mute, volume slider, device button,
tiles and launcher icons in `tabs/a/home/ctl.rs`; other tabs list none yet and keep their own keyboard handling). An action is a click at the control's
centre through the view's own `event` (so a tab, a button and a card action run the code their mouse path runs); a slider moves by a click at the
fraction of its track.

| key | does |
|---|---|
| Tab / Shift+Tab, arrows | move the ring |
| Enter / Space | press the control under the ring |
| Left / Right | on a tab: switch to the neighbouring tab (the ring follows); on a slider: step it |
| Alt+1..9 | the N-th tab |
| Esc | leave: the ring fades, the keyboard goes back to the user's window (`TextFocus(false)`), the card folds |
| the hotkey again | next target, or off (the card stays; it folds on its own when the cursor is away) |

Keyboard mode ends **without touching the focus** when the user takes the mouse (a click), the window loses the keyboard, or a tab asks for the keyboard for
a text field (so typing in Chat or Note is never intercepted). If the card does not open within 1.8 s (a fullscreen guard hides it) the keyboard is given back.

**Music widget** (`musicwidget/view_kbd.rs`): the widget takes the keyboard (`Cx::set_focus`) and opens its card. Space = play / pause, Left / Right =
5 s back / forward (previous / next track when the track has no length), Shift+Left / Shift+Right = previous / next track, Tab and Enter = the ring
over the three buttons and the position bar, Esc = leave. Media keys are not touched. It also ends when the widget goes away.

**The ring** (`ui/a11y/ring.rs`): ONE glasskit focus ring (2 px accent outline, 1 px gap) whose rectangle is four `Flow` springs (`Feel::FLOW`
within 300 ms): it glides from control to control, resizing on the way, fades in / out on `Feel::FADE`, appears in place (no flight from the
origin) when keyboard mode starts. Targets are set in `step` (`aim`), `paint` only reads; Reduce motion lands it at once; it costs frames only while it
moves, and `kbd_step` is a few comparisons when the mode is off. Under high contrast it gets a second outer line.

## UI Automation (`host/uia.rs`)

The window's pane keeps its name. Its **children** are the view's controls: control type from the role (button, tab item, slider, list item, check
box), name, `IsEnabled`, `HasKeyboardFocus`, `AutomationId` `pill.<id>`, bounds in screen px (client origin + logical x scale), runtime id
`[UiaAppendRuntimeId, id]`. Patterns by role / value: **Invoke** (plain buttons), **Toggle** (switches such as the pin), **SelectionItem** (tabs, list
items; the container is the pane), **RangeValue** (sliders: min / max / value, small change = the step). `Navigate` walks first / last child, siblings
and the parent; `ElementProviderFromPoint` finds the smallest control under a point; `GetFocus` returns the keyboard-mode focus, and a focus change
raises `UIA_AutomationFocusChangedEventId` (only while a client listens: `UiaClientsAreListening`).

Threading is unchanged: a provider call is a `SendMessageTimeoutW(WM_A11Y_REFRESH)` to the UI thread, which stores the text and the control list
(`store_controls`); an action is a parked `(id, Act)` plus `WM_A11Y_ACT`, answered by the view's `control_act`. Nothing is built unless a client is
connected.

## High contrast (`ui/a11y/contrast.rs`)

`a11y.contrast` = `auto` (default) | `on` | `off` (Look > Animation and contrast). `read_system()` asks Windows (`SPI_GETHIGHCONTRAST` + `GetSysColor`:
window, window text, highlight, highlight text, gray text, button face / text, hot-light). `pick(mode, sys)`: Auto = Windows' palette while a high-contrast theme
is on, otherwise the normal look; On = Windows' palette, or the built-in black one (`SysColors::BLACK`) without a theme; Off = never. `host/win32.rs` turns
`WM_SETTINGCHANGE` (SPI_SETHIGHCONTRAST / "HighContrast" / a broadcast), `WM_SYSCOLORCHANGE` and `WM_THEMECHANGED` into the bus event `a11y:contrast`
(debounced), and the views re-read.

While it is on: the island's body is the window colour and fully opaque (`ShellSettings::hc_bg` via the shell setting `a11y.hc.bg`: it beats `island.color`
and the transparency sliders, and decides the light / dark text), the toolkit theme is `contrast::theme` (flat, opaque, system colours; selection =
highlight; no shadow; links = hot-light), the body and the music widget get a solid 2 px outline in the button-text colour, the widget ignores the cover's
accent, and the focus ring doubles. Reduce motion is a separate setting and stays separate.

## Command bar shortcuts (`command/shortcuts.rs`)

`command.shortcuts` (Settings > General > Command bar search, "Your prefixes and bookmarks"): one entry per line, `prefix = address`, at most 20,
http / https only. An address with `{query}` is a template (`g = https://www.google.com/search?q={query}`: `g rust lifetimes` opens that page, the words
percent-encoded); without it a bookmark (`gh = https://github.com`: `gh` opens it). The row replaces the other results for that input (prefix match is
case-insensitive and needs the whole first word; a template needs words, a bookmark takes none; the first of two equal prefixes wins). A prefix of yours beats the
built-in `c n f a s`. Local only: nothing leaves the PC until Enter, and then only the default browser opens the address. The list is a `Lines` row, so entries
cannot contain a comma or `;`.

## Not verified on screen

See the report of the change: the ring over every tab, a real screen reader (Narrator, NVDA) walking the tree, the Windows contrast themes switching live, the
foreground hand-over of `Win+Alt+J` while a full-screen app is in front, the music widget's keys in the taskbar placement.
