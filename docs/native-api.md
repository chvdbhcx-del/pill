# Native API facade (`src-tauri/src/ui/api.rs`)

> **Since 0.7.0 there is no web (WebView2) UI.** The native UI described here is the only one: the `ui.native.*` keys, `--native`, `--web-ui`, `PILL_NATIVE` / `PILL_WEB_UI`, `ui/flags.rs`, `ui/lazy.rs` and `ui/windows.rs` are gone, and every gate / "flag off" / "web" comparison below is history (v0.6.1 is the last release that still contains the web code).

The native UI has no WebView, so it cannot `invoke('name', {..})`. `ui::api` is the in-process equivalent: the same command names, the same
argument objects, the same results, calling the very same Rust functions the `#[tauri::command]` wrappers are. No command signature or behaviour
was changed; the old web UI keeps working while both exist. Events go the other way through `ui::bus::subscribe` (see "Events").

Files: `ui/api.rs` (public API, tests), `ui/api/dispatch.rs` (one arm per command), `ui/api/args.rs` (JS-style argument extraction),
`ui/api/pool.rs` (worker pool).

## Public API

```rust
pub fn init(app: &AppHandle);                                   // once, from setup(); stores the handle
pub fn call(cmd: &str, args: Value, caller: &str) -> Result<Value, String>;
pub fn call_raw(cmd: &str, args: Value, caller: &str, body: Vec<u8>) -> Result<Value, String>;
pub fn call_async(cmd: &str, args: Value, caller: &str, done: impl FnOnce(Result<Value, String>) + Send + 'static);
pub fn call_raw_async(cmd, args, caller, body, done);           // same, with a raw body
pub fn dispatch_calls(calls: impl IntoIterator<Item = (String, Value)>, caller: &str,
                      on_reply: impl Fn(usize, Result<Value, String>) + Send + Sync + 'static);
pub fn capture_frame(label: &str) -> Option<Arc<Vec<u8>>>;      // raw RGBA of one monitor, shared, no copy
pub fn to_camel(&str) -> String;  pub fn to_snake(&str) -> String;
```

`Value` is `serde_json::Value`. The host must call `ui::api::init(app.handle())` in `setup()` (nothing does it yet; before that every call is
`Err("ui::api is not initialised ..")`, never a panic).

### Result and error mapping

* `Ok(v)` is exactly what `await invoke(..)` resolves with: the command's return value serialised by serde (`()` and `Option::None` are `Value::Null`,
  `&'static str` a string, `Vec<_>` an array, `Cfg` an object ...).
* `Err(text)` is what the promise would reject with: the `Err(String)` of a `Result<_, String>` command (`note_save`, `remind_add`, `autostart_set`,
  `hotkeys_reload`, `capture_commit`, `capture_action`), or an argument error, or `command X not found`. The `{"__error": ..}` convention of the JS
  wrappers is not used here; the renderer decides what a failure means (the bar maps `Err` to `value: None`, panel B to `Reply::err(text)`).
* A panic inside a command is caught and returned as `Err("panic: command `name`: message")`; the pool worker survives.
* Commands run to completion; no timeout is added.

### Arguments

The argument object has the JS shape: camelCase keys of the Rust parameter names (`copy_only` is `copyOnly`, `json_text` is `jsonText`,
`new_index` is `newIndex`). The snake_case spelling is accepted as a fallback; camelCase wins if both are present. `Value::Null` means "no
arguments" (`invoke('cmd_toggle')`), any other non-object is an error. Extra keys are ignored. Rules (as Tauri applies them):

| Rust parameter | missing key | `null` | wrong type |
|---|---|---|---|
| `T` | Err ``command `c` missing required key `fooBar` `` | Err (unless `T` accepts null, e.g. `Value`) | Err ``invalid args `fooBar` for command `c`: expected i64, got string`` |
| `Option<T>` | `None` | `None` | Err (same text) |
| `AppHandle` | injected from the stored handle | | |

Two leniencies for native callers: a whole float where an integer is expected (`1500.0` for `u64`) is accepted, and the error text never contains
the argument value (API keys and passwords travel through here, so serde's own `invalid type: string "sk-.."` is replaced). Nothing in the facade
logs arguments or results.

### Raw bodies

Only `capture_commit` takes one: Tauri sends the marked-up PNG as a raw request body and the JSON arguments in header `x-pill-args`. Natively:
`call_raw("capture_commit", <the x-pill-args JSON>, caller, png_bytes)`. Without markup, `call("capture_commit", <same JSON>, caller)`. `call_raw` of any
other command is `Err("command `x` does not take a raw body")`.

`capture_frame` (raw RGBA, `w*h*4` bytes; `capture_session` gives `w`/`h`) is not JSON: `call("capture_frame", ..)` is an `Err` pointing to
`ui::api::capture_frame(label)`, which returns the session's own `Arc<Vec<u8>>` without copying 8 MB per monitor.

## The caller label

`caller` is the label of the native window that asks: `pill`, `bar`, `settings`, `capture-N`. The rule: **the facade never guesses the calling window;
the renderer says who it is.** Today every `#[tauri::command]` takes only `AppHandle` (the sole injected parameter; none takes `Window`,
`WebviewWindow`, `Webview` or `State`), and the label-dependent behaviours already receive their window as an explicit argument (`capture_ready` /
`capture_session` / `capture_frame` / `capture_commit` take `label`; `bar:shown` and friends are emitted to whichever window shows). So no arm reads
`caller` now and no command needed a window-based `_impl` refactor. The parameter is in the signature so that when a command does need the window, only
`dispatch.rs` and one `_impl(app, label, ..)` change, not every renderer. Pass the real label anyway; do not invent labels.

### `_impl` refactors (the only two)

`tauri::ipc::Request` / `Response` cannot be built outside a webview, so their bodies were extracted; the wrappers keep identical signatures:

| Command | Extracted | Wrapper now |
|---|---|---|
| `capture_commit` | `capture::capture_commit_impl(&AppHandle, CommitArgs, Option<Vec<u8>>) -> Result<(), String>` (`CommitArgs` became `pub(crate)`) | parses the request, calls the impl |
| `capture_frame` | `capture::capture_frame_arc(&str) -> Option<Arc<Vec<u8>>>` | `Response::new(frame bytes)` |

## Threading rules

* `call` / `call_raw` block (a sync command may take seconds; `async fn` commands are driven with `tauri::async_runtime::block_on`). Call them on
  **worker threads only**: never on the native UI thread (it would freeze the capsule), never inside a tokio task (`block_on` panics there; the panic
  would be caught and returned as an error, but the call would not run).
* `call_async` / `call_raw_async` / `dispatch_calls` run on a pool of plain std threads (`pill-api`, min 2, max 16, idle workers exit after 30 s). The pool
  grows when no worker is idle, so a quick `ai_transform_cancel`, `chat_stop`, `capture_cancel` or `aisetup_cancel` is never queued behind a long
  `ai_transform` / `aisetup_*` / `voice_test_mic`. Only above 16 simultaneous long calls does a call wait.
* `done` / `on_reply` run on the worker thread that finished the command, in completion order. Keep them short: post a message to the UI thread, never
  draw or touch UI state in them.
* The old UI ran sync commands on Tauri's main thread; on a worker they do the same work. Window creation (`settings_open`, the lazy bar) from a worker is
  the safe direction on Windows (from a sync command on the main thread it can deadlock).
* Event handlers (below) run on the emitting thread; calling `ui::api::call` from inside one can deadlock on locks the emitter holds. Use `call_async`.

## `dispatch_calls` and the ui-core `BackendCall` types

`dispatch_calls(calls, caller, on_reply)` starts every `(cmd, args)` at once on the pool and reports `on_reply(i, result)` with `i` = position in `calls`.
The ui-core crates each have their own call type; none was edited. Mapping (all use the camelCase arg object the JS sent, `Null` = no args):

| ui-core type | Shape | Map to `dispatch_calls` | Reply goes back as |
|---|---|---|---|
| `bar::BackendCall` | `{cmd: &'static str, args}` inside `Action::Call { ticket: Option<u64>, call }` | `(cmd.to_string(), args)`; keep `ticket` by index | `BarEvent::Reply { ticket, value }`: `Ok(v)` is `Some(v)`, `Err` is `None` (the JS wrappers' `null`); no ticket means fire-and-forget (pass a no-op `on_reply`) |
| `panel::a::BackendCall` | `{cmd, args}` (no tag) | same | `on_result(cmd, &args, &res)` of the owning tab; keep `(cmd, args)` by index; on `Err` skip the call (the JS pages did nothing on a rejected invoke) |
| `panel::b::BackendCall` | `{cmd, args, tag: u64}` | same; keep `tag` by index | `on_reply(tag, Reply { ok, v, err })`: `Ok(v)` is `Reply::ok(v)`, `Err(e)` is `Reply::err(&e)` |
| `settings::BackendCall` | enum; `.command()` and `.args()` give the pair | `(c.command().to_string(), c.args())` | handler per variant (`on_set_result(&merged)`, `on_hotkey_result`, `on_autostart(Result<bool,String>)`, `flows::*`): `autostart_set` is the one that wants the raw `Err` |
| `capture::BackendCall` | `{cmd: String, args}`; `ARGS_HEADER` = `x-pill-args` | `(cmd, args)`; for a markup commit use `call_raw_async` with the PNG; `capture_frame` through `ui::api::capture_frame` | capture model `Input`s; `is_registered()` is the web-backend list |
| `shell::Backend::invoke(cmd, args)` | fire-and-forget sink; results come back as `Event::BatteryState` / `WeatherGet` / `SettingsOpened` / `DesktopFg` | `dispatch_calls([(cmd, args)], "pill", ..)` and translate the reply for the four commands whose result matters | those `Event`s |
| `live::Query` | `{cmd, args, as_event, delay_ms}` | `(cmd, args)` after `delay_ms` (sleep on the renderer's timer, not in the pool); keep `as_event` by index | `Hub::dispatch(as_event, payload)`; `Err` is `Value::Null` |

Native-only name: `capture_pin` (ui-core `capture`) is not a backend command; the facade answers ``command capture_pin not found``. It needs a Rust
command first (and an arm plus a contract row; the coverage test then keeps all three lists equal).

## Events

The native UI subscribes with `ui::bus::subscribe(name, f)` (`f(&Value)`, runs on the emitting thread, returns an id for `unsubscribe`). Every one of
the 56 events of `docs/native-ui-contract.md` reaches it, verified by two tests in `api.rs`:

* `every_contract_event_has_a_producer_literal`: each event name is a string literal in a backend source file;
* `no_event_bypasses_the_bus`: no `.emit(` / `.emit_to(` / `.emit_str(` / `.emit_str_to(` / `.emit_filter(` call exists outside `ui/bus.rs`.

Things to know: the subscriber gets the payload but **not the target label** (`Target::All` or `Label("pill")` is invisible), so an event meant for
one window is seen by every native subscriber; a native window filters by what it needs (`capture:begin`/`capture:end` carry no monitor, the capture
host treats them as global, one session at a time). The Rust-side listeners (`notif`, `obs`, `steam`, `voice` on `settings:changed`) still get the
Tauri copy, because `bus::emit` forwards to the webviews first.

**`tauri://drag-enter|over|drop|leave` do not exist natively.** They are WebView events (WebView2 file drag); the bus never carries them and no
facade helper exists. The native shelf / home tab must register an OLE `IDropTarget` on its host window and call `shelf_add {paths}` /
`launcher_add {paths}` itself. The reverse, `shelf_drag_out {ids}`, is already a native OLE drag source: it blocks its worker until the drop and returns
`"copied"` or `"cancelled"`, so call it from `call_async` while the left button is still down.

Also never emitted: `bar:ready-barrier` (the bar page registers and drops it). `cmd_ready` has to be called by the native bar once its subscriptions
are in place, exactly like the page does: opening the bar waits for it (with a timeout) and replays `bar:shown` if it comes late.

## Tests (`cargo test --release`)

* `every_contract_command_has_a_dispatch_arm`: parses `docs/native-ui-contract.md` at test time; fails with the missing names.
* `every_arm_is_registered_in_main`, `contract_and_handler_agree`, `no_arm_is_listed_twice`: arms == `generate_handler!` == contract (176).
* `args.rs`: camel/snake helpers, required/optional/null/missing/wrong-type error texts, float-to-integer leniency, values never in errors.
* `pool.rs`: off-thread, a blocked job does not block the next, panics do not kill the pool.
* `api.rs`: pre-`init` calls are errors, `call_async` and `dispatch_calls` answer every call, panic wrapping.

Not unit-testable without a running app: the commands themselves. Every command needs the `AppHandle` (`tauri::test` needs a Tauri cargo feature this
crate does not enable, and most commands touch the real SQLite store, the registry, audio or the network), so no command is smoke-called. Run a
facade call against the live app in a real session (e.g. `call("settings_get", Null, "pill")`) once the host calls `init`.

## What cannot work without a WebView

* `tauri://drag-*` events (above): replaced by OLE `IDropTarget` in the host.
* Nothing else in the command or event contract: all 176 commands run without a WebView. The backend still creates the `pill` WebView window at start
  (`ui::windows::build_pill`) and the lazy `bar` / `settings` windows; removing those is the host's job (commands that show/hide them, `cmd_toggle`,
  `settings_open`, `pill_set_state`, still drive the WebView windows today).
