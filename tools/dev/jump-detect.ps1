<#
  Detects the island jumping sideways, from the real screen (no debug port needed, no Python).

    pwsh tools/dev/jump-detect.ps1                       watch 8 s, whole width of the primary screen, pixel row 3
    pwsh tools/dev/jump-detect.ps1 -DurationMs 15000 -MaxCenterShift 6
    pwsh tools/dev/jump-detect.ps1 -X 1920 -Width 1920   a second monitor to the right of the primary one

  Grabs one pixel row of the top of the screen as fast as it can (usually 100+ frames/s), finds the longest run of dark pixels
  (the black capsule) in every frame and reports each time the CENTRE of that run moves by more than -MaxCenterShift pixels
  between two frames. A capsule that grows symmetrically keeps its centre, so a moving centre means a sideways jump (if pets or
  characters sit beside your capsule they join the run and move the centre: hide them for this check). While it runs, make the island
  do things (hover to expand, switch tabs, collapse, trigger a notification) or run audit-transitions.mjs next to it.
  Exit code 1 if any jump was found. Works best on a light wallpaper; a black wallpaper makes the run meaningless (raise -Row or
  -DarkThreshold). The capsule must be at the top edge of the captured area.
#>
param(
    [int]$X = 0,                 # left edge of the captured strip (screen pixels)
    [int]$Y = 0,                 # top edge of the screen area
    [int]$Width = 0,             # 0 = full width of the primary screen
    [int]$Row = 3,               # pixel row below Y that crosses the capsule
    [int]$DurationMs = 8000,
    [int]$DarkThreshold = 60,    # R+G+B below this counts as capsule
    [int]$MinRun = 40,           # shorter dark runs are not the capsule
    [double]$MaxCenterShift = 6,
    [double]$MaxJump = 400       # bigger moves are a different dark object entering the row, not the capsule: not counted
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
if (-not ('PillJump' -as [type])) {
    $refs = if ($PSVersionTable.PSEdition -eq 'Core') { 'System.Drawing.Common', 'System.Drawing.Primitives' } else { 'System.Drawing' }
    Add-Type -ReferencedAssemblies $refs -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

public static class PillJump {
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    public static int ScreenWidth() { SetProcessDPIAware(); return GetSystemMetrics(0); }

    public static int Frames;
    public static int Jumps;
    public static string Log = "";

    // longest run of dark pixels in the row: start and length
    static void Longest(byte[] buf, int w, int dark, out int start, out int len) {
        start = -1; len = 0;
        int cur = -1;
        for (int i = 0; i <= w; i++) {
            bool isDark = i < w && (buf[i * 4] + buf[i * 4 + 1] + buf[i * 4 + 2]) < dark;
            if (isDark) { if (cur < 0) cur = i; }
            else if (cur >= 0) { if (i - cur > len) { len = i - cur; start = cur; } cur = -1; }
        }
    }

    public static void Run(int x, int y, int w, int ms, int dark, int minRun, double maxShift, double maxJump) {
        SetProcessDPIAware();
        var bmp = new Bitmap(w, 1, PixelFormat.Format32bppArgb);
        var g = Graphics.FromImage(bmp);
        var buf = new byte[w * 4];
        var sb = new StringBuilder();
        var sw = Stopwatch.StartNew();
        double prevC = -1; int prevL = 0, prevR = 0;
        Frames = 0; Jumps = 0;
        while (sw.ElapsedMilliseconds < ms) {
            g.CopyFromScreen(x, y, 0, 0, new Size(w, 1));
            var d = bmp.LockBits(new Rectangle(0, 0, w, 1), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            Marshal.Copy(d.Scan0, buf, 0, buf.Length);
            bmp.UnlockBits(d);
            Frames++;
            int s, l; Longest(buf, w, dark, out s, out l);
            if (l < minRun) { prevC = -1; continue; }
            double c = s + l / 2.0;
            double dc = Math.Abs(c - prevC);
            if (prevC >= 0 && dc > maxShift && dc < maxJump) {
                Jumps++;
                sb.AppendFormat("t={0,5} ms  centre {1:F1} -> {2:F1}   left {3} -> {4}   right {5} -> {6}\n",
                    sw.ElapsedMilliseconds, prevC, c, prevL, s, prevR, s + l);
            }
            prevC = c; prevL = s; prevR = s + l;
        }
        Log = sb.ToString();
    }
}
'@
}

if ($Width -le 0) { $Width = [PillJump]::ScreenWidth() }
Write-Host "Watching x=$X..$($X + $Width), y=$($Y + $Row) for $DurationMs ms: use the island now."
[PillJump]::Run($X, $Y + $Row, $Width, $DurationMs, $DarkThreshold, $MinRun, $MaxCenterShift, $MaxJump)
$fps = [math]::Round([PillJump]::Frames * 1000.0 / $DurationMs)
if ([PillJump]::Log) { Write-Host ([PillJump]::Log).TrimEnd() }
Write-Host "frames: $([PillJump]::Frames) (~$fps/s), sideways jumps > $MaxCenterShift px: $([PillJump]::Jumps)"
if ([PillJump]::Jumps -gt 0) { exit 1 }
