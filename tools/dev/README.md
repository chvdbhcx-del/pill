# Live-check tools

Small PowerShell scripts for checking the running app. PowerShell 7 (`pwsh`; Windows PowerShell 5.1 also works for `jump-detect.ps1`). No Node, no Python.

| Tool | What it does |
| --- | --- |
| `pwsh tools/dev/mem.ps1` | read-only memory report for a running Pill: `pill.exe`, `mswidgets.exe` and, as a check, any `msedgewebview2.exe` that belongs to Pill (there should be none: Pill is native only) |
| `pwsh tools/dev/jump-detect.ps1` | captures one pixel row at the top of the screen for 8 s and reports every frame where the capsule's centre moves sideways |

### jump-detect.ps1

Needs no debug port. Run it in a second terminal and exercise the island (hover, switch tabs, collapse, notification):

```powershell
pwsh tools/dev/jump-detect.ps1 -DurationMs 15000
```

Defaults: whole width of the primary screen, pixel row 3 from the top, dark = R+G+B below 60. Use `-X`/`-Width` for another monitor, `-Row`/`-DarkThreshold` for a theme where the capsule is not black, `-MaxCenterShift` for the tolerance in pixels. It measures the longest dark run in that row, so a very dark wallpaper or pets/characters beside the capsule will disturb the result. Exit code 1 if a jump was found.
