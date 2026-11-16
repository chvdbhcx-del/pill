# Live activities from local programs

Any program on this PC can put **one ongoing, time-bound thing** on the island: a build, an export, a render, a download of its own. It sends one line of JSON to a named pipe, or runs `pill.exe live ...`. No plugin code is loaded into Pill and nothing leaves the PC.

Opt-in: **Settings > Calendar and mail > Live activities** (`live.local`, off by default). While the switch is off the pipe does not exist and no thread runs. The **Show a sample** button posts a sample activity; the settings preview shows the same sample.

```
src-tauri/ui-core/src/live/activity.rs   schema + limits (parse_line), Registry (3 shown, queue, dismiss lock, time-out), the island provider; unit tested
src-tauri/src/liveact.rs                 the pipe, the clock thread, the reply channel, the commands, `pill.exe live`
```

## What the user sees (three representations, like iOS Live Activities)

| | where | what |
|---|---|---|
| **expanded** | an event card on the island | title, subtitle, progress bar, chip and up to two buttons; shown when the activity starts (and when it moves up from the queue), 8 s (14 s with buttons) |
| **compact** | the capsule row `liveact` (priority 56) | icon, title, progress bar, chip of the oldest shown activity, and a dismiss (x) button |
| **minimal** | the same row, after a separator | the other shown activities as icon + chip; `+N` when some wait in the queue |

Everything animates with the island's own motion: the row pops in and fades out, the progress bar glides to each new value, the card slides in and its bar sweeps to the value, a soft edge pulse on a new activity. With Reduce motion on, the end state is shown at once.

## Rules

- **Only fixed fields.** No custom views, no images, no markup, no URLs. Text is shown as plain text (control and bidi characters are removed, spaces collapsed).
- **Hard limits** (a message that breaks one is refused whole, nothing is cut): line 2048 bytes; `id` 1..32 of `A-Z a-z 0-9 _ . - :`; `title` 1..40 characters; `subtitle` 1..80; `chip` 1..7; `progress` 0..1; `ttl_s` 1..3600; at most 2 `actions`, each `id` 1..16 (same characters) and `label` 1..16, ids distinct.
- **At most 3 shown at once.** More wait in a queue of 5 (first in, first out); a freed slot goes to the first waiting one and it gets its card. A sixth waiting one is refused with `full`.
- **The user can close any activity** (the x button on the capsule row). A dismissed `id` **cannot be posted again until it ends**: `start` and `update` of it are refused with `locked` until you send `end` (or its time-out passes). That is the only way to bring the same id back.
- **Every activity ends by itself.** `ttl_s` defaults to 600 (10 minutes), at most 3600 (1 hour). It is an inactivity time-out: each `start` / `update` restarts it, so a program that died never leaves a card behind. A program that works for longer than an hour keeps sending updates.
- **Rate limit.** 15 messages a second per connection (a burst of 30); more get `rate-limited`.
- **Quiet hours** hide the cards as they hide other cards; the capsule row stays.
- Only the same user can connect (see Security). Up to 8 connections at once.

## Schema

One JSON object per line, UTF-8, `\n` at the end. Unknown fields are an error.

| field | start | update | end | |
|---|---|---|---|---|
| `op` | required | required | required | `"start"`, `"update"` or `"end"` |
| `id` | required | required | required | names the activity; all later messages use it |
| `title` | required | optional | | text |
| `subtitle` | optional | optional | | text; empty or `null` clears (update) |
| `icon` | optional | optional | | a built-in glyph name (`area bell battery bolt calendar check clipboard copy download file folder keyboard mail mic monitor mute next note2 pause play plus prev qr quote quoteoff scantext shelf sparkles terminal timer trash usb video volume warn x`) or an emoji (up to 8 characters, none of them plain ASCII). Default `sparkles` |
| `progress` | optional | optional | | number 0..1; `null` clears |
| `chip` | optional | optional | | short text, e.g. `40%`, `3/9`; empty or `null` clears |
| `ttl_s` | optional | optional | | seconds, 1..3600 |
| `actions` | optional | optional | | `[{"id":"stop","label":"Stop"}]`, at most 2; `null` clears |
| `v` | optional | optional | optional | schema version, only `1` |

`update` changes only the fields it carries. An `update` or `end` for an id that is not there: `update` is refused with `unknown-id` (start it again), `end` answers `gone` (it is always safe to send twice). A `start` of an id that is already shown replaces it.

### Replies

The server answers every line with one line:

```
{"ok":true,"state":"started"}      started | queued | updated | ended | gone
{"ok":false,"error":"locked","detail":"..."}
```

Errors: `bad-json`, `too-long`, `unknown-field`, `bad-op`, `bad-field` (the `detail` names the field), `unknown-id`, `locked`, `full`, `rate-limited`, `busy` (too many connections), `disabled`.

### Buttons: the reply channel

When the user presses a button, or closes the activity, or it ends by itself, the island writes **one JSON line on the connection that posted that activity last**:

```
{"event":"action","id":"build","action":"stop"}
{"event":"dismissed","id":"build"}
{"event":"ended","id":"build","reason":"ttl"}        reason: ttl | disabled
```

Why the same connection and not a callback pipe per id: a client that wants clicks already holds a connection (it is posting updates on it), it needs no second server, no second name to choose and nothing else on the PC can be made to connect to it. The price: **a client that closed its connection gets no clicks** (they are dropped), so a one-shot `start` followed by exit cannot receive buttons. Keep the connection open for the life of the activity, or use `pill live start ... --wait`. Only a button the activity declared is passed on. A client that never reads cannot block anything: its events are dropped when its queue (64 lines) is full.

## The pipe

`\\.\pipe\pill-live-v1`, byte mode, line-delimited.

## The command line

`pill.exe live start|update|end --id ID [--title T] [--subtitle S] [--icon NAME|EMOJI] [--progress 0..1|NN%] [--chip C] [--ttl SECONDS] [--action ID:LABEL]... [--clear subtitle|icon|progress|chip|actions] [--wait]`

It checks the message with the same rules, writes it, prints the reply line and exits: `0` accepted, `1` refused by the rules, `2` bad arguments, `3` Pill is not running or the switch is off. Nothing else of Pill starts in that process. `--wait` keeps the connection open and prints each event line (a button press) until the activity ends.

`pill.exe` has no console of its own, so from PowerShell pipe it to make the shell wait and keep the exit code: `& pill.exe live start --id b1 --title Build --progress 0 | Out-Host; $LASTEXITCODE`.

```powershell
pill live start  --id build --title "Building" --subtitle "app.sln" --icon terminal --progress 0 --chip 0%
pill live update --id build --progress 0.5 --chip 50%
pill live end    --id build
```

## Examples

PowerShell, keeping the connection (so buttons work):

```powershell
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'pill-live-v1', 'InOut')
$pipe.Connect(2000)
$w = New-Object System.IO.StreamWriter($pipe); $w.AutoFlush = $true
$r = New-Object System.IO.StreamReader($pipe)
function Send($o) { $w.WriteLine(($o | ConvertTo-Json -Compress -Depth 4)); $r.ReadLine() }

Send @{ op='start'; id='export'; title='Exporting video'; subtitle='clip.mp4'; icon='video'; progress=0; chip='0%'; ttl_s=120
        actions=@(@{id='cancel'; label='Cancel'}) }
foreach ($i in 1..10) { Start-Sleep 1; Send @{ op='update'; id='export'; progress=$i/10; chip="$($i*10)%" } }
Send @{ op='end'; id='export' }
```

Python (no packages, `open` works on a Windows pipe):

```python
import json, threading

pipe = open(r"\\.\pipe\pill-live-v1", "r+b", buffering=0)

def send(**msg):
    pipe.write((json.dumps(msg) + "\n").encode())
    return pipe.readline().decode()          # the reply line (replies and events come on this one stream)

print(send(op="start", id="job", title="Training", icon="bolt", progress=0.0, chip="0%",
           actions=[{"id": "stop", "label": "Stop"}]))
for i in range(1, 11):
    print(send(op="update", id="job", progress=i / 10, chip=f"{i * 10}%"))
print(send(op="end", id="job"))
```

If you read button events too, read one line per message you send *and* handle lines that start with `{"event":`: they can arrive between replies. A small reader thread that routes lines by their `ok` / `event` key is the usual shape.

## Security and privacy

- **Local only, this user only.** The pipe is created with an access list built from the process token: this user and SYSTEM, nobody else; remote clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`). If the access list cannot be built the pipe stays closed. The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a program that already owns the name makes Pill fail instead of receiving its clients.
- **Opt-in.** Off by default; turning it off closes every connection and ends every activity.
- **Nothing is stored or logged.** Messages live in memory until the activity ends. (The card, like any other island card, is recorded in the notification history if that history is on.)
- **A program can only show text.** It cannot draw, load an image, open a link or run anything: a button press only sends its id back to the program that declared it.
- Any program of the same user can post, and two programs share the id namespace: pick ids that are unlikely to clash (`myapp.build`).

## Not verified live

The pipe, the access list, the `pill live` console output, emoji icons and the compact row's look were written and unit tested, not seen on a running Pill.
