# Adds the pill agent-status hooks to ~/.claude/settings.json (backup: settings.json.bak-pill-hooks).
# Also repairs the file: it currently holds two JSON objects back to back; the second one (permissions) is merged into the first.
$p = "$env:USERPROFILE\.claude\settings.json"
Copy-Item $p "$p.bak-pill-hooks" -Force
$raw = Get-Content $p -Raw
$i = $raw.LastIndexOf("}`n`n{")
if ($i -ge 0) {
  $o1 = $raw.Substring(0, $i + 1) | ConvertFrom-Json -AsHashtable
  $o2 = $raw.Substring($i + 3) | ConvertFrom-Json -AsHashtable
  $o1.permissions = $o2.permissions
} else { $o1 = $raw | ConvertFrom-Json -AsHashtable }
$h = ($env:APPDATA -replace '\\', '/') + '/pill/hooks/agent-hook.ps1'
function cmd($s) { @{ hooks = @(@{ type = 'command'; command = "powershell -NoProfile -ExecutionPolicy Bypass -File `"$h`" -Tool claude-code -State $s"; timeout = 10 }) } }
if ($PSVersionTable.PSVersion.Major -lt 7 -or -not $o1 -or -not $o1.ContainsKey('env')) { throw 'needs PowerShell 7 (pwsh) and a parsable settings.json; nothing was written' }
if (-not $o1.hooks) { $o1.hooks = @{} }
$o1.hooks.UserPromptSubmit = @(cmd 'working')
# Only real questions (permission / elicitation dialogs). The idle reminder ("waiting for your input") is not a question to the user.
$n = cmd 'waiting'; $n.matcher = 'permission_prompt|elicitation_dialog'
$o1.hooks.Notification = @($n)
$o1.hooks.Stop = @(cmd 'done')
[IO.File]::WriteAllText($p, ($o1 | ConvertTo-Json -Depth 20), (New-Object Text.UTF8Encoding($false)))
$chk = Get-Content $p -Raw | ConvertFrom-Json
"ok; hooks: " + (($chk.hooks.PSObject.Properties.Name) -join ',') + "; allow rules: " + $chk.permissions.allow.Count
