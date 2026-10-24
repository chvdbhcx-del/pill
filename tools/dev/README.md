# Live-check tools

Small scripts for checking the running app: evaluate JS in a page, profile it, audit the island's size transitions, and catch sideways jumps on the real screen. Node 22+ (uses the global `WebSocket` and `fetch`) and PowerShell 7 (`pwsh`; Windows PowerShell 5.1 also works for `jump-detect.ps1`). No Python, no npm packages.

## Start the app with the debug port

WebView2 only exposes the Chrome DevTools Protocol when asked. Set the variable for the process that starts the app, never globally:

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9333 --remote-allow-origins=*'
npm run tauri dev
```

The port listens on `127.0.0.1` only. `--remote-allow-origins=*` is required, otherwise the WebSocket handshake is refused. Another port: pass `--port N` to a tool or set `CDP_PORT`. Do not leave the app running like this when you are not debugging: anything on your machine can then drive the page.

## Tools

| Tool | What it does |
| --- | --- |
| `node tools/dev/cdp.mjs list` | lists the pages (`pill.html`, `bar.html`, `settings.html`, ...) |
| `node tools/dev/cdp.mjs pill.html "Pill.state"` | evaluates an expression in the page whose URL contains `pill.html` and prints the JSON result (promises are awaited) |
| `node tools/dev/prof.mjs pill.html --ms 10000` | JS CPU profile: busy share of one core and the top functions by self time. Leave the app idle (or in the state you want to measure) while it runs |
| `node tools/dev/audit-transitions.mjs` | steps through expand, collapse, tab switches and an event card, sampling the island body every frame, and prints a table (`under`, `over`, `shift`, `settleMs`) |
| `pwsh tools/dev/jump-detect.ps1` | captures one pixel row at the top of the screen for 8 s and reports every frame where the capsule's centre moves sideways |

### audit-transitions.mjs

Start from an idle, collapsed island and keep the mouse away (hover expands it). Options: `--page pill.html`, `--selector "#body"`, `--max-under-w 14`, `--max-under-h 14`, `--max-shift 3`. Exit code 1 when a transition dips below its content by more than the limits (the island "vanishing" during a collapse) or the centre moves sideways between two frames more than `--max-shift`; 2 if it could not run. The defaults come from the author's measurements, not from a specification: tune them to your build. The audit calls `Pill.expand/collapse/toast`, so it needs those in the page (they exist in `src/pill.js`).

### jump-detect.ps1

Needs no debug port. Run it in a second terminal and exercise the island (hover, switch tabs, collapse, notification) or run `audit-transitions.mjs` at the same time:

```powershell
pwsh tools/dev/jump-detect.ps1 -DurationMs 15000
```

Defaults: whole width of the primary screen, pixel row 3 from the top, dark = R+G+B below 60. Use `-X`/`-Width` for another monitor, `-Row`/`-DarkThreshold` for a theme where the capsule is not black, `-MaxCenterShift` for the tolerance in pixels. It measures the longest dark run in that row, so a very dark wallpaper or pets/characters beside the capsule will disturb the result. Exit code 1 if a jump was found.

## Notes

- These tools only read: the audit moves the island by calling its public API, nothing is written to disk.
- Do not paste real data into `cdp.mjs` expressions you share: the output may contain clipboard or notes content.
