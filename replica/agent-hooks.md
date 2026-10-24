# Agent status in the pill (Claude Code, Codex)

The pill shows `working` (spinner), `waiting` (amber "Needs your input") and a card when an agent finishes.
The app NEVER edits another tool's config. It only reads small JSON files from `%APPDATA%\pill\agents\`
that hooks YOU install write. Nothing leaves the machine.

## Status file (one per session, written by hooks)

```json
{"tool":"claude-code","session":"abc-123","state":"working","title":"Refactor the login flow","cwd":"C:\\proj","ts":1792776671881}
```

- `tool`: `claude-code` | `codex` | anything else is shown as `other`
- `state`: `working` | `waiting` | `done`; `session`: `[A-Za-z0-9._-]`, max 64 chars (anything else is dropped)
- `title`: text only, max 80 chars; `cwd`: optional; `ts`: epoch ms (missing or in the future -> file time)
- Reader rules (`src-tauri/src/agents.rs`): files over 4 KB, non-JSON, unknown states and symlinks are ignored; at most 64 files;
  control and bidi characters are stripped; the content is never executed or passed to a shell.
- `working`/`waiting` without an update for 30 min -> `stale` (not counted); `done` files are deleted after 10 min;
  stale files after 24 h. `agents:done` fires once per transition to `done`; old `done` files never ring on app start.

## The helper (the app's own file)

At start the app writes `%APPDATA%\pill\hooks\agent-hook.ps1` (its own folder, ASCII, ~30 lines: read it before you use it).
It reads the hook JSON (stdin for Claude Code, last argument for Codex), writes the status file atomically,
keeps the earlier title/cwd on later events, ignores a `waiting` that arrives after `done` (Claude Code's idle reminder),
prints nothing and always exits 0 (a non-zero hook exit can block the agent; UserPromptSubmit stdout goes to the model).
The Settings text comes from the Tauri command `agents_hook_snippets` (or `await PillAgentsHookText()` in the page).

## Claude Code: merge into `~/.claude/settings.json`

Replace `<HELPER>` with `C:/Users/<you>/AppData/Roaming/pill/hooks/agent-hook.ps1` (forward slashes: safe in cmd, bash and PowerShell).

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"<HELPER>\" -Tool claude-code -State working" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"<HELPER>\" -Tool claude-code -State waiting" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "powershell -NoProfile -ExecutionPolicy Bypass -File \"<HELPER>\" -Tool claude-code -State done" }] }]
  }
}
```

Optional: a `PostToolUse` hook with `"matcher": "*"` and `-State working` flips the pill back to "working" after you approve a
permission prompt (otherwise it stays "waiting" until the turn ends). Cost: one PowerShell start (~0.2 s) per tool call.

## Codex CLI: `~/.codex/config.toml` (above any `[table]`)

```toml
notify = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "<HELPER>", "-Tool", "codex", "-State", "done"]
```

Only the end of a turn is reported (no working/waiting states).

## Marked uncertain (written from memory, verify against your versions)

1. Claude Code hook stdin JSON: `session_id`, `transcript_path`, `cwd`, `hook_event_name` (+ `prompt` for UserPromptSubmit, `message` for Notification). `CLAUDE_SESSION_ID` as an env var is not documented, so the helper uses stdin first.
2. Which shell runs hook commands on Windows differs between Claude Code versions; the command contains no `$` and no backslashes on purpose.
3. Claude Code's Notification also fires as an idle reminder about a minute after the turn ended; handled in the helper.
4. Codex `notify` payload: `type: "agent-turn-complete"`, `thread-id`, `cwd`, `input-messages`, `last-assistant-message`; quoting of the JSON argument through `powershell -File` was tested only via bash here. Failure mode: session `default`, empty title.
5. The helper title for Claude Code is your prompt (first 80 chars), for Codex the start of the last assistant message. Both stay local; leave `title` empty in the file if you do not want them shown.

## Manual test (no agent needed)

```powershell
'{"tool":"claude-code","session":"t1","state":"working","title":"Manual test","ts":' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + '}' | Set-Content "$env:APPDATA\pill\agents\t1.json"
# later: change "working" to "done" -> the card appears; delete the file to clear
```
