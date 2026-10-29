# READ-ONLY memory report for a running Pill: pill.exe, mswidgets.exe and the msedgewebview2.exe processes that belong to Pill
# (their command line carries Pill's WebView2 data folder %LOCALAPPDATA%\com.pill.dev\EBWebView / --webview-exe-name=pill.exe).
# Metric: PRIVATE working set (WMI WorkingSetPrivate = perf counter "\Process V2(*)\Working Set - Private", the number Task Manager shows as "Memory");
# fallback when the counter is unavailable: Get-Process PrivateMemorySize64 (committed private bytes, usually higher; the header says which).
# Starts, stops and writes nothing. Usage: pwsh -File tools\dev\mem.ps1 [-Csv]
param([switch]$Csv)
$ErrorActionPreference = 'Stop'

$procs = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe' OR Name='pill.exe' OR Name='mswidgets.exe'")

function Classify($p) {
    switch ($p.Name) {
        'pill.exe'      { return 'pill.exe' }
        'mswidgets.exe' { return 'mswidgets.exe' }
    }
    $cl = [string]$p.CommandLine
    # only Pill's own WebView2 tree; SearchHost / widgets / other apps' webviews are skipped
    if ($cl -notmatch 'com\.pill\.dev' -and $cl -notmatch 'app\.island\.desktop' -and $cl -notmatch '--webview-exe-name=pill\.exe') { return $null }
    if ($cl -match '--type=([\w-]+)') {
        switch ($Matches[1]) {
            'renderer'        { return 'webview2 renderer' }
            'gpu-process'     { return 'webview2 gpu-process' }
            'utility'         { return 'webview2 utility' }
            'crashpad-handler'{ return 'webview2 crashpad-handler' }
            default           { return "webview2 $($Matches[1])" }
        }
    }
    return 'webview2 browser'
}

# private working set per pid. Win32_PerfFormattedData_PerfProc_Process.WorkingSetPrivate is the same number as the perf counter
# "\Process V2(*)\Working Set - Private" but locale independent (the counter path is translated on a Russian Windows).
$ws = @{}
$metric = 'Working Set - Private'
try {
    foreach ($s in Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -ErrorAction Stop) { $ws[[int]$s.IDProcess] = [double]$s.WorkingSetPrivate }
    if ($ws.Count -eq 0) { throw 'no samples' }
} catch {
    $metric = 'PrivateMemorySize64 (fallback)'
    foreach ($q in Get-Process -ErrorAction SilentlyContinue) { $ws[$q.Id] = [double]$q.PrivateMemorySize64 }
}
$rows = foreach ($p in $procs) {
    $k = Classify $p
    if (-not $k) { continue }
    [pscustomobject]@{ Group = $k; PID = [int]$p.ProcessId; MB = [math]::Round(($ws[[int]$p.ProcessId]) / 1MB, 1) }
}
$rows = @($rows)
if ($Csv) { $rows | Sort-Object Group, PID | ConvertTo-Csv -NoTypeInformation; return }

"Pill memory report  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')   metric: $metric"
if (-not $rows) { 'No Pill processes found.'; return }
''
$rows | Sort-Object Group, PID | Format-Table Group, PID, @{n = 'MB'; e = { '{0:N1}' -f $_.MB }; a = 'right' } -AutoSize
'Per group:'
$rows | Group-Object Group | Sort-Object Name | ForEach-Object {
    [pscustomobject]@{ Group = $_.Name; Count = $_.Count; MB = '{0:N1}' -f ($_.Group | Measure-Object MB -Sum).Sum }
} | Format-Table Group, Count, @{n = 'MB'; e = { $_.MB }; a = 'right' } -AutoSize
$wv = @($rows | Where-Object Group -like 'webview2*')
'WebView2 total : {0,8:N1} MB  ({1} processes)' -f ($wv | Measure-Object MB -Sum).Sum, $wv.Count
'TOTAL (all)    : {0,8:N1} MB  ({1} processes)' -f ($rows | Measure-Object MB -Sum).Sum, $rows.Count
