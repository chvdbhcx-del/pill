# Taskbar compatibility

How Pill's "in the taskbar" mode (`edge.position` = `taskbar`) and the standalone music widget (`media.widget.edge` = `taskbar`) behave next to the
Windows 11 taskbar and the tools that change it. Nothing here was run against those tools: the cells come from reading the code (`taskbar.rs`,
`ui/host/geom.rs`, `pill.rs::frame_for`) and from the public notes of the tools. Every cell is one of:

* **by design** - the code does this on purpose, whatever the tool;
* **expected to work** - follows from the rules below, no live test yet;
* **UNVERIFIED - check live** - depends on how the tool behaves in a way the code cannot know. Use the checklist at the end.

## What Pill relies on

| # | assumption | where | when it fails |
|---|---|---|---|
| R1 | The taskbar RESERVES its strip: work area = monitor minus a strip at the bottom (full width, 24 logical px .. one third of the monitor) | `geom::bottom_bar` | auto-hide, bar on another edge, floating / inset bar: plain bottom edge |
| R2 | A `Shell_TrayWnd` (primary) or `Shell_SecondaryTrayWnd` (other monitors) window lies along the bottom of that monitor, is at least half the monitor wide and its top matches the work area's bottom (DPI-scaled tolerance, at least 3 px) | `taskbar::implausible_bar` (new) | no such window, a narrow / inset / sliding window: plain bottom edge, one log line |
| R3 | `TrayNotifyWnd` (tray and clock) is a visible child of that window; the island stays left of it | `taskbar::tray_rect` | not found / hidden: the island may use the whole bar |
| R4 | The taskbar is topmost and jumps in front of Pill when clicked; Pill raises itself back (`EVENT_SYSTEM_FOREGROUND`, no timer) | `taskbar::raise_over_taskbar` | a tool that parents / skins the bar differently: see matrix |
| R5 | Pill never reparents into the taskbar and never reads taskbar icons: it does NOT know where the Start button or the app icons are | by design | a centred icon group is covered if the island is centred too (set Left / Right) |

The fallback ("plain bottom edge") is the ordinary bottom island: it hangs from the bottom of the monitor, inside the work area when `underTaskbar` is on.
Whenever R1 or R2 stops holding, Pill logs ONE line per change of verdict to `%LOCALAPPDATA%\Pill\logs\pill.log`:
`taskbar: not used as the island's bar (<why>); using the plain bottom edge`.

## Matrix

| Case | Island in the taskbar | Music widget in the taskbar | Why |
|---|---|---|---|
| Stock Windows 11, icons centred | by design: the span is monitor-left .. tray-left; Left / Right align keep clear of Start and icons, Center sits in the middle of the span and can cover centred icons | by design: same slot, avoids the island | R5: Pill cannot see the icons. Default on first choice is Left |
| Stock Windows 11, icons left | expected to work (Left lands over the icons: use Right or Center, or an offset) | expected to work | same rule |
| Taskbar auto-hide on | by design: work area = monitor, so no bar: plain bottom edge, no log line | by design: plain bottom edge | R1 |
| Auto-hide on while the bar is slid out | expected to work: the work area stays whole, the plain bottom edge applies | expected to work | R1 |
| Bar on top / left / right (Start11, ExplorerPatcher, Windows 10 style) | by design: not a bottom bar, plain bottom edge | by design | R1 |
| Secondary monitor, bar shown on all monitors | expected to work: the matching `Shell_SecondaryTrayWnd` is found per monitor; a secondary bar may have no `TrayNotifyWnd`, then the span is the whole bar | expected to work | R2, R3 |
| Secondary monitor, bar only on the main one | by design: no strip reserved there, plain bottom edge | by design | R1 |
| DPI 100 / 125 / 150 / 175 / 200 % | expected to work: all sizes scale with the monitor's DPI, bar height comes from the work area; unit-tested at 100-200 % (`geom`, `taskbar` tests) | expected to work | - |
| Mixed DPI (monitors at different scale) | expected to work: placement is per monitor; moving a window across is handled by `WM_DPICHANGED` | UNVERIFIED - check live | not exercised with two scales |
| Taller bar (Windows "large" taskbar, 72+ px) | island: expected to work; strip must stay under one third of the monitor | widget: by design only up to 72 logical px, taller = plain bottom edge | `WIDGET_BAR_MAX_H` |
| Start11 (Windows 11 style taskbar kept) | UNVERIFIED - check live | UNVERIFIED - check live | Start11 adds its own start menu; whether it keeps `Shell_TrayWnd` full width and the work area reserved is its setting |
| Start11 (Windows 10 style / bar replaced) | UNVERIFIED - check live: with a replacement bar R2 may fail and Pill then falls back by design | same | the fallback is the safety net |
| ExplorerPatcher (classic Windows 10 bar) | UNVERIFIED - check live: the bar is still `Shell_TrayWnd` and reserves its strip, so it should behave like Windows 10 | same | R1, R2 |
| Windhawk taskbar mods (icon size, height, styler) | UNVERIFIED - check live: a changed height is read from the work area, so it follows; a mod that insets or floats the bar fails R2 and falls back by design | same | R2 |
| TranslucentTB | expected to work: only changes the bar's look, geometry and z-order stay | expected to work | R4 |
| TaskbarX / centring tools | expected to work for placement (the `Shell_TrayWnd` rect is unchanged, they move the icons inside it); the island can cover the moved icons, set Left / Right | same | R5 |
| Taskbar mod that stacks a second always-on-top bar | UNVERIFIED - check live: two topmost windows fight over z-order; Pill raises itself only when a `Shell_*TrayWnd` is above it | same | R4 |
| Fullscreen app / game | by design: the fullscreen guard hides the island and the widget (event driven, see `native-island.md`) | by design: hides with the island | `guard.rs` |
| Explorer restarted (crash, tweaker reload) | UNVERIFIED - check live: the taskbar window is re-created; Pill re-evaluates on the next guard wake-up (at most 1 s) and the foreground hook still fires | same | `relayout` poll |

## Five-minute live check

Do this once after installing Pill, and again after switching a taskbar tool. Settings > Look > Screen edge.

1. Choose "In the taskbar", align Left. The capsule should stand in the bar, left of the clock, not over Start. Click the bar (an empty spot, then Start): the capsule stays visible above it each time. (R4)
2. Align Center and Right: center covers centred icons (expected), Right stands next to the tray. The capsule must never overlap the clock or tray.
3. Turn on taskbar auto-hide: the capsule moves to the plain bottom edge on its own within a second; nothing flickers. Turn it off: it returns to the bar. Check `pill.log` for at most one `taskbar:` line per change.
4. Change Windows scale (Settings > System > Display) to 125 %, then 175 % or 200 %: the capsule keeps its place in the bar and stays centred vertically in it.
5. Second monitor: set "Monitor" to the second one. Its bar (if "show taskbar on all displays" is on) holds the capsule; with that off the capsule sits on the plain bottom edge.
6. Enable "Music player as a separate widget" at the taskbar: it and the island keep clear of each other, both stay above the bar after a bar click.
7. Open a video or game in fullscreen: the island hides at once (not half a second later) and returns about half a second after you leave it. Press F11 in a browser: same.
8. With a tweaker installed (Start11, Windhawk mod, ExplorerPatcher, TranslucentTB, TaskbarX): repeat 1 and 3. If the capsule is NOT in the bar, read the `taskbar:` line in the log: it names which rule failed. Report it with Settings > Updates > Report a bug.

Sources for the tools' behaviour: the [Ramen Software write-up of Windhawk taskbar mods](https://ramensoftware.com/windhawk-mods-for-the-windows-11-taskbar), the [ExplorerPatcher project](https://github.com/valinet/ExplorerPatcher) and each tool's own release notes. Overlapping taskbar mods are generally discouraged by their own authors, so test one at a time.
