# Native settings window (`src-tauri/src/ui/settings_view`)

> **Since 0.7.0 there is no web (WebView2) UI.** The native UI described here is the only one: the `ui.native.*` keys, `--native`, `--web-ui`, `PILL_NATIVE` / `PILL_WEB_UI`, `ui/flags.rs`, `ui/lazy.rs` and `ui/windows.rs` are gone, and every gate / "flag off" / "web" comparison below is history (v0.6.1 is the last release that still contains the web code).

The Settings window of Pill on the native host (`docs/native-host.md`), replacing `settings.html` + `settings.js` when the flag
`ui.native.settings` is on (and `--web-ui` / `PILL_WEB_UI=1` is not given). Pure logic stays in `pill_ui_core::settings` (schema, `SettingsModel`,
search, wizard, flows); this module only draws it and routes input.

## Gate

`settingswin::open` (tray, island, command bar, `pill.exe --open settings`, the first-run wizard request, AI setup links) and
`settingswin::settings_close` start with `if ui::settings_view::native_enabled()` (= `flags::native_settings()`, which already answers false while
the web UI is forced). Flag off = exactly the old code path. The window is the host's `Normal` singleton `settings`; a second open sends
`Msg::Open({page})` to the open view. Geometry memory keeps the old keys (`settingsWin.x|y|w|h|max`, debounced by the host), restored with
`Placement::Restore` (falls back to centring when the monitor is gone).

## Files

| file | what |
|---|---|
| `mod.rs` | `SettingsView`, `View` impl, `Local` messages |
| `open.rs` | host registration, geometry memory, facade `Api` over `ui::api::call`, ticker, file dialog |
| `nav.rs` `step.rs` `replies.rs` `acts.rs` `events.rs` | page building / switching, per-frame work, backend answers, clicks, input routing |
| `page.rs` `rows.rs` `cardpaint.rs` `ctlpaint.rs` `keys.rs` `pagepaint.rs` | page / row / control types and painting |
| `chrome.rs` `paint.rs` `wizard.rs` | title bar, sidebar, search, the frame, first-run wizard |
| `custom.rs` `cw1.rs` `cw2.rs` `cw3.rs` `cw3b.rs` `customacts.rs` `prim.rs` | the 12 `Custom` widgets and their drawing primitives |
| `picker*.rs` | pet cards and island-style cards with live previews |
| `ui.rs` `fields.rs` `txt.rs` `pal.rs` `icons.rs` | hit registry, deferred text fields, text helpers, palettes, icons |
| `preview/` | the live preview panel on the right (see "Live preview") |
| `tests_adv.rs` | the collapsed "Advanced" section: default closed, spring, no events / Tab stops while closed, search and old page names |

## Pages

**The sidebar is a list of functions: one short page per function** (35 pages, typically 3-12 rows; General is the one long page), under seven small non-clickable group headings
(`nav::NAV`: `NavItem::Head` / `NavItem::Page`). The list scrolls when it does not fit (wheel, scrollbar, keyboard; the chosen page is scrolled into view
when it is opened from search or from outside, and again when the window is resized), the selected item has the glide background plus an accent bar, and
every item has a small monoline icon left of its name (below). On a page, in this order: the function's on/off switch, its events (the `live.*` rows), its options, then (if any) a
trailing `advanced` section folded behind the page's own "Advanced" header (see below). The rows are defined once (`ui-core/src/settings/pages/{base,system,phone,ai}.rs`,
the old sixteen pages); `pages/layout.rs` takes them out of a pool by id and arranges them (`Pool::main` is the unnamed main card of a function page; `Pool::finish`
panics when a row is left over). No settings key was renamed, dropped or given another default.

| group | pages (id: what) |
|---|---|
| Island | `look`: shape, theme, colour, size, monitor (Advanced: custom colour, transparency, position) / `edge`: edge style list + reactions / `pet`: pet gallery, side, size / `clock`: time, date, seconds, 12/24 h / `tabs`: island tabs, what is on Home, profiles / `animation`: Reduce motion (Advanced: frame rate, GPU) |
| Behaviour | `general`: ONE page of the general settings, in this order: Start and language (`language`, autostart), Hiding the pill (fullscreen, peek, auto-hide), Quiet hours, Command bar search (file search + browser engine), Settings window (the live-preview switch `settings.preview`); Advanced: hiding details and program lists, pass-through, Everything, settings backup (+ the data folder note), first run / `hotkeys`: every key on ONE page (the recorder checks a combo against all others) |
| Features | `weather` / `media` (Advanced: sound sources) / `volume`: volume, devices, mic and camera dots / `clipboard` (Advanced: pasting, exclusions, limits) / `screenshots` / `recording` (Advanced: encoder) / `notes`: notes and timer (Advanced: own sound) / `launcher` / `shelf` / `text` / `controls` / `vpn` (Advanced: your own client) / `voice`: voice to text + local Whisper (Advanced: recognition service) |
| Events | `notifications` (Advanced: display time, history) / `downloads` (+ Steam) / `battery` / `keyboard` / `desktops` / `usb` / `obs` / `calendar` / `mail` |
| Phone | `phone`: Phone link, apps, shared clipboard, files, relay as sections (Advanced: file details, relay address and cap) |
| AI | `ai`: assistant and chat / `agents`: AI agents in the terminal / `localai`: local AI chat on this computer |
| App | `updates`: version and updates, what is new, help |

Where a page fetches data when it shows up and which pages poll status lines is decided per page id (`settings_view/nav.rs::page_shown`, `live_status`): `start`
autostart, `search` Everything, `hotkeys` launcher list, `look` monitors, `volume` audio devices, `voice` key / Whisper, `clipboard` status, `ai` / `localai` setup,
`updates` info, and the polled lines of `notifications`, `downloads`, `obs`, `calendar`, `mail` (1 s ticker) and `phone` (2 s). The pet / edge previews loop only on `edge` and `pet`.

The overlapping switches are NOT merged, only placed side by side and labelled apart (`Pool::edit`, the ids are listed in `RELABELED` in `tests.rs`):
Weather = "Forecast from the internet" (`net.weather`, asks the network, off by default) + "Weather tab" (`module.weather`); Phone = "Phone link"
(`phone.enabled`, the main switch: the server on this PC) + "Phone tab" (`module.phone`) + "Cards from the phone" (`live.phone`); File shelf =
"File shelf" (`module.shelf`) + "File-count badge" (`live.shelf`). The wizard's step 2 collects all `module.*` rows from the schema in the old order
(`wizard::MODULE_ORDER`). Two key-less buttons that only jumped to a section right next to them are gone (`aisetup.cloud-chat`, `aisetup.cloud-voice`;
`DROPPED` in `tests.rs`).

**Old page ids keep working everywhere** (`nav::LEGACY`, `OPENABLE`, `settingswin.rs::PAGES`; a test holds the last two equal). The sixteen ids of the oldest
sidebar and the ten of the previous one: `appearance` -> `look`, `character` -> `edge`, `behavior` -> `general`, `sound` -> `volume`,
`live` -> `notifications`, `modules` -> `tabs`, `aisetup` -> `localai`; the ids of the 38-page sidebar `start`, `hiding`, `quiet`, `search` -> `general` (scrolled to their first row: `language`, `hideInFullscreen`, `quiet.on`, `module.fileSearch`); `general`, `hotkeys`, `weather`, `voice`, `clipboard`, `phone`, `ai`, `updates` are pages
of the new sidebar too. The window opens the new page and scrolls to the start of the old section (no flash). Ctrl+F finds every row, jumps to its page (and
opens the Advanced section when the row is in it); it also matches the old page names and the name of the function page (both are appended to the search
keywords of every row: "weather" finds the units, the city and the folded-island chips alike).

**Sidebar icons.** Each page has one icon id (`Page::icon`, all listed in `nav::ICONS`, a test keeps page icons and the list equal and unique). `settings_view/icons.rs::glyph`
resolves an id to a 24-grid monoline glyph: the window's own few (`IC`: general, character, weather, updates, clipboard, phone, power, palette, paw, clock, motion,
desktops, obs, chip) first, then the island's line-icon set (`island::gpu::Gpu::glyph`: bell, download, battery, globe, usb, calendar, mail, keyboard, shield, mic, ...);
drawn with `glasskit::svgpath` at 16 px, stroke 1.6, left of the name (14 px in, name 10 px after the icon; long names get an ellipsis). Colour: dim idle, text colour
while hovered, accent while selected; the mix reads the hover flow and the glide of the selection indicator (`chrome.rs::paint_nav_item`), so nothing new is stepped and
Reduce motion makes it instant. Group headings stay text only. Icons per page: look palette, edge smiley, pet paw, clock clock, tabs home, animation easing curve, general
sliders, hotkeys keyboard, weather sun+cloud, media note, volume speaker, clipboard clipboard, screenshots dashed area, recording camera, notes note, launcher grid,
shelf tray, text text, controls sliders, vpn shield, voice mic, notifications bell, downloads arrow, battery battery, keyboard globe, desktops two windows, usb plug, obs
broadcast, calendar calendar, mail envelope, phone phone, ai sparkles, agents terminal prompt, localai chip, updates refresh.

**Pickers.** The edge styles are a compact list (a 112 x 48 live pictogram, the name and one or two lines of text per row); the pets are tiles, three per row,
a 62 px stage and at most three lines of text. Page heading to first card: 20 px; sections 24 px apart.


### The Advanced section

`Group::adv()` marks a group; the groups so marked are one trailing run of a page. `build_page` puts a `Block::Adv(n)` header before them (n = the blocks
that follow) and a `page::Adv { t: Flow, on, sub, fid }` on the page; the header lists the names of the sections inside. The section starts closed on every
window start and is **never stored** (`SettingsView::adv_open` remembers only which pages were opened while this window lives). A click or Enter / Space on the
header sets `Flow` `t` (target set in the event handler, ticked in `step_all`; instant under Reduce motion). `paint_blocks` scales the heights, the opacity
and the divider gaps of the following blocks by `t`, `paint_card` multiplies every row's `vis` by it and keeps it in `RowRt::ak`; rows with `ak < 0.5` take
no events and `focus_order` leaves them out while closed. A Ctrl+F hit inside a closed section calls `set_advanced(true, instant)` from `resolve_goto`
and waits for the next layout before scrolling to the row and flashing it. Idle cost is nil: only the one `Flow` moves, and only for a moment.

## Live preview (`preview/`)

A panel on the right of the window (320 px) shows the real island, miniature, while a setting is being changed: `showSeconds` adds the seconds to the
folded clock, `net.weather` opens the island on the weather tab, `live.copy` plays the "Copied" flash, `clipClickPaste` opens the clipboard tab. The setting
is applied to the preview's own copy **at once** (it reads `SettingsModel::values()` every frame, not what the backend has saved), and the island animates
the change itself. Caption under the stage says what is shown; **Replay** plays the scene again, **Hide** turns `settings.preview` off (Updates, "Live
preview on the right"; default on, the only settings key that the preview added).

| file | what |
|---|---|
| `preview/mod.rs` | `Preview` (state), `step_preview` (panel width, sync of the settings, sandbox step, camera), `paint_preview` (stage, caption, the two buttons) |
| `preview/live.rs` | the sandbox: a second `IslandView` on a `Cx` of its own |
| `preview/scene.rs` | key -> `Scene` lookup (`of_key`), page start (`of_page`), `shareable` filter, captions |
| `preview/sample.rs` | `SampleApi` (the sandbox's backend), sample weather / clipboard, the payloads of the reactions |
| `preview/tests.rs` | headless tests (see below) |

**Reuse, not a copy.** The sandbox is the island's own `IslandView`, so every drawing path (pill, panel, tabs, cards, pets) is the real one. Its `Cx` is built
by hand: its own `FakeBus` (nobody else publishes on it), its own `MsgQueue` (the queue's wake-up posts `Local::Pv` to the window), and `SampleApi` as the
backend. Everything the island asks (`settings_get`, `weather_get`, `clip_list`, ...) is answered from sample data or with `null`; nothing is forwarded to the
real backend, and the preview gets no mouse or key events, so it cannot touch the real island. The window's settings go in as `settings:changed` patches,
filtered by `scene::shareable`: only switches, choices and sliders of the schema (no typed text such as the city, folders, addresses, mail user, device ids,
hotkeys), and never the ones that would hide or mute it (`autoHide`, `passThrough`, `quiet.*`, `hideInApps`, ...). Weather is a "Sample City" forecast, the
clipboard has invented lines, a notification is "Sample sender: ...".

**Which setting shows what** (`scene::of_key`, keyed by the settings key, so regrouping the rows changes nothing):

| setting(s) | scene |
|---|---|
| `shape` `theme` `island.color` `island.colorCustom` `size` `opacityCol` `opacityExp` `language` | the folded island |
| `showTime` `showDate` `showWxIcon` `showTemp` `showSeconds` `clock24` `showBattery` | the folded clock row (weather and 76 % battery are samples) |
| `pet` `pet.*` / `character` `character.liquid` | the folded island with the pet / the edge style |
| `reduceMotion` `fps` `edgeIdleFps` | the folded island, Replay shows the motion |
| `net.weather` `weather.units` | the weather tab with the sample forecast |
| `clipboard.*` `clipClickPaste` | the clipboard tab with sample items |
| `module.*` `home.*` `music.lyrics` | the open island on Home (the tab strip changes) |
| `live.copy` `live.keyboardLayout` `live.virtualDesktop*` `live.volume` `live.notifications` `live.battery` `live.usb` `live.dlDone` `live.reminders` `sound.reminder*` `live.recording` `live.music` `musicPill` `live.micCam` `cards.duration` `capture.shotAction` | the folded island with that reaction played from sample events; with the switch off the caption says it will not appear |

A setting that is not in the table keeps the scene on show; opening a page starts its own scene (`of_page`: Weather -> weather tab, Tabs and Home, Launcher, Shelf ... -> Home tab, Clipboard -> clipboard tab,
Notifications -> a notification, Media player -> music, Volume, Recording, Downloads, Battery ... -> their own reaction, Look, Edge, Pet, Clock, Animation -> the matching look of the folded island, the rest -> the folded island).

**Layout.** `parts_pv` takes `Preview::width()` off the right of the page area, so the page, its scrolling and its hit tests shrink with the panel. The panel is
only wanted when `window - sidebar - 320 >= 580` (a card plus its margins), i.e. a window of about 1113 px (`MIN_WINDOW_W`); narrower, or with the switch off, it
slides away (`vis` Flow) and the sandbox is dropped. The default window is now 1180 x 760 (`nav::DEFAULT_SIZE`); a saved geometry still wins, so a narrower
saved window simply has no panel until it is widened or maximised. The stage is a desktop-coloured box that follows the island: the camera (`zoom`, `stage_h`
Flows, targets set in `step`) looks at the island body of the sandbox window, so the folded island is shown big and the open one scaled down to fit.

**Idle cost.** The sandbox runs `SETTLE_MS` (3 s) after the last change, longer only while something really moves (`MAX_MS` 9 s; a pet's idle loop or the
weather sky do not count). Then `Live::sleep`: its queue stops waking the window (flag checked in the wake-up closure), the open tab stops its ticker
(`IslandView::preview_sleep`), its clock stops (time spent parked does not count, so a card keeps its remaining life), and no frame is requested until the next
change, Replay or page. A drawn re-measure posted while parked wakes it once. Reduce motion: the camera and the panel land at once and the island itself
is told (`Cx::reduced_motion`).

Tests (`preview/tests.rs`, all pixel tests run on one long-lived thread, see `gfx`): the panel follows `showSeconds` / `net.weather` / `live.copy` /
`clipClickPaste` in the dark theme (PNG per step), light theme at 2x, Reduce motion, no panel below the threshold, Hide / the switch, Replay, parked and idle
once settled, nothing typed by the user reaches the sandbox and the real backend is never called, no mouse / wheel effect, every reaction has a scene and
sample events.

## How it works

* One pass per frame inside `draw` measures and paints (layout cached for the next input event, so tests render once before sending events).
* Each schema row owns one glasskit control (Toggle, Slider with value bubble, Segmented, Select, Button, TextInput); a changed control goes
  `Change -> SettingsModel::try_set -> Vec<BackendCall> -> Cx::call_async`; the answer returns as `Msg::Reply`. Hotkeys only go through the recorder
  flow (`hotkey_set`, never `settings_set`). Secrets (API keys, passwords) are password fields that call their own command and are cleared.
* `TextInput::event` needs a `Canvas`; `fields::TextField` queues the events and replays them at the next `draw`.
* Animations (all collapse to instant under Reduce motion): page exit slide 120 ms + enter rise 240 ms, row expand / collapse 340 / 300 ms and the
  disabled fade, switch spring, segment pill, chip pop (weekdays, providers), message line rise, search popup pop + 26 ms stagger + jump glide +
  1.6 s wash, sidebar indicator spring, wizard step slide, picker card selection, key-capture look, window entrance 140 ms.
* Frames run only while something moves; the Edge and character and Pet pages loop its previews (per-card `PetEngine` / `Characters`). A ticker thread runs only
  on Notifications, Downloads, OBS, Calendar, Mail (status polling) and Phone, while Everything is being installed and so on.

## The 12 `Custom` widgets

`ai.providers`, `voice.providers` (chips, help, region warning, "Get a key" link) / `character.reactions` (reaction chips driving every preview) /
`hotkeys.launcher` (list from `launcher_list`, one recorder each) / `modules.profiles` (list, Apply, Delete, name field + Save) / `weather.city`
(field, results list, `weather_set_city`) / `search.everything` (status, Download, Run installer, progress box, 5 s status poll) /
`general.backup` (Export, Import through the system file dialog) / `updates.whatsnew` (animated disclosure with the CHANGELOG notes) /
`aisetup.intro` (text) / `aisetup.whisper` (model chips, plan, progress, switch, delete-with-confirm) / `aisetup.chat` (found servers, Ollama installer
and model catalog). Download jobs come from `aisetup:progress`.

## Tests

`cargo test --release settings_view`: every page renders in dark and light; toggle / segmented / slider write the right patch; the recorder commits
through `hotkey_set` (Esc cancels, Backspace clears); sidebar navigation fetches page data; Reduce motion switches at once; Ctrl+F search jumps and
flashes; the wizard steps and writes `firstRunDone`. `PILL_SNAP_DIR=<dir>` also writes the PNG snapshots (`<page>-<theme>-1x.png`).

## Known gaps

No light theme existed on the web page (the light palette is a native derivative); pet / style cards are not keyboard-navigable yet (Tab skips
them); Clipboard "Wipe" has no Cancel button (second click confirms, Esc is not wired); `net_features_status` host line is not shown; no
screen-reader tree (host limit); scale-2 and web-vs-native pixel comparisons were not run.

## Lead's on-screen checklist (flag on: `ui.native.settings` = true in settings.json, run `pill.exe`)

1. Tray -> Settings opens the native window on Look; `pill.exe --open settings` focuses it; with `--web-ui` the old window opens.
2. Drag by the title bar, double-click maximizes, resize from every edge, snap (Win+arrows), minimum 700x520; close, reopen: same place / size.
3. Move between monitors of different DPI: text stays crisp, window keeps its logical size.
4. Switch pages (click and Ctrl+F search incl. a row on another page: scrolls to the middle and flashes); the indicator glides.
5. Toggle, slider (bubble), segmented, select popup, text fields (type, paste, Enter / blur commit), Quiet hours chips and time fields.
6. Hotkeys: click a box, hold Ctrl+Alt, press a letter -> saved; a used / reserved / taken combo shows the red reason; Esc cancels; Backspace clears.
7. Autostart switch writes / removes the Run value; Reduce motion setting makes every animation instant.
8. General -> Advanced -> First run -> Run again shows the wizard (Skip, Back, Next, dots, language change inside it); first start shows it once.
9. AI (with the install part) / Voice to text / Updates / Notifications pages: buttons, progress boxes, status lines; nothing downloads without a click.
10. Advanced: on a page with the header at the end (General, Clipboard...) it is closed; click it, the section opens on a spring, click again, it closes; Ctrl+F "video encoder" opens Screen recording, opens the section and flashes the row.
11. Idle: window at 0 % CPU (Task Manager) on every page except Edge and character and Pet (previews loop); Reduce motion stops those too.
