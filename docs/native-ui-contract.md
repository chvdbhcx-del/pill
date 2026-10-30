# Native UI contract

The names and shapes the Rust backend and the native UI say to each other. There is no web (WebView2) front end any more: this is the backend contract of the native views (`ui/api`, `ui/bus`). Event producers are the `bus::emit` call sites and the small local `emit` helpers; commands are the arms of `src-tauri/src/ui/api/dispatch.rs`. The tests in `ui/api.rs` keep the command table and the event producers in step with this file.

- **Events** (backend to UI) go through `ui::bus::emit(name, payload)` (`src-tauri/src/ui/bus.rs`); native code `bus::subscribe(name, f)` to the same stream and gets the payload as JSON. "Payload as emitted" is the Rust expression at the producer; JSON object keys are the field names. "Surface" is the native window that consumes the event (`pill` = the island and its tabs).
- **Commands** (UI to backend) are plain `pub fn`s called through `ui::api::call` / `call_async` with camelCase JSON arguments; `AppHandle` arguments are injected and not listed. A `Result` command answers `Err(text)`.
- Window labels: `pill` (the island, always alive), `bar` (created on demand), `settings` (on demand), `capture-N` (one overlay per monitor, `capture.rs`).

## Events

| Event | Surface | Payload as emitted | Producer |
|---|---|---|---|
| `agents:changed` | pill | `snap` | agents.rs |
| `agents:done` | pill | `json!({ "session": s.session, "tool": s.tool, "title": s.title, "cwd": s.cwd })` | agents.rs |
| `aisetup:progress` | all windows | `json!({ "id": id, "item": item, "done": done, "total": total, "phase": phase, "message": message })` | aisetup.rs |
| `audio:bands` | pill | `&Frame { b, rms: (rms * 1000.0).round() / 1000.0, t }` | audio.rs |
| `capture:begin` | capture-N overlay | `json!({ "id": id })` | capture.rs |
| `capture:done` | pill | `json!({ "kind": "error", "message": msg })`<br>`json!({ "kind": "screenshot", "path": path, "copied": copied, "w": w, "h": h })`<br>`json!({ "kind": "text", "found": false })`<br>`json!({ "kind": "text", "found": true, "text": head, "chars": t.chars().count(), "copied": copied })`<br>`json!({ "kind": "qr", "found": true, "text": head, "isUrl": is_http_url(&t) })`<br>`json!({ "kind": "qr", "found": false })`<br>`json!({ "kind": "color", "hex": hex, "rgb": rgb, "copied": copied, "history": color_history() })`<br>`json!({ "kind": "record", "path": p, "copied": copied, "seconds": (now_ms() - r.started) / 1000 })` | capture.rs, capture.rs, capture.rs, capture.rs, capture.rs, capture.rs, capture.rs, capture.rs |
| `capture:end` | capture-N overlay | `json!({})` | capture.rs |
| `chat:delta` | pill | `json!({ "thread": thread, "run": run, "text": std::mem::take(&mut pending) })`<br>`json!({ "thread": thread, "run": run, "text": pending })` | chat.rs, chat.rs |
| `chat:done` | pill | `json!({ "thread": thread, "run": run, "ok": error.is_none(), "stopped": stopped, "error": error, "id": id })` | chat.rs |
| `chat:focus` | pill | `()` | hotkeys.rs |
| `clip:added` | pill | `item` | clipboard.rs |
| `clip:open` | pill | `()` | hotkeys.rs |
| `kbd:layout` | pill | `info(new)` | watch.rs |
| `launcher:changed` | pill | `serde_json::json!({ "count": v.len(), "items": &v })` | launcher.rs |
| `live:battery` | pill | `json!({ "kind": kind, "percent": percent })` | liveplus.rs |
| `live:calendar` | pill | `json!({ "title": title, "minutes": minutes })` | liveplus.rs |
| `live:download` | pill | `json!({ "state": "progress", "id": id, "name": shown_name, "bytes": t.size, "total": Value::Null, "path": dir.join(name).to_string_lossy(), })`<br>`json!({ "state": "done", "id": id, "name": c.name, "bytes": bytes, "total": Value::Null, "path": p.to_string_lossy(), })`<br>`json!({ "state": "gone", "id": id })` | livesys.rs, livesys.rs, livesys.rs |
| `live:file-done` | pill | `json!({ "name": first, "path": path.to_string_lossy(), "more": ready.len() - 1 })` | liveplus.rs |
| `live:mail` | pill | `json!({ "from": from, "subject": subject, "more": f.fresh - 1 })` | mail.rs |
| `live:notification` | pill | `p` | notif.rs |
| `live:obs` | pill | `json!({ "recording": false, "streaming": false, "recMs": 0, "streamMs": 0 })`<br>`p` | obs.rs, obs.rs, obs.rs |
| `live:privacy` | pill | `json!({ "mic": now.0, "cam": now.1, "apps": now.2 })` | livesys.rs |
| `live:steam` | pill | `json!({ "state": "idle" })`<br>`done`<br>`p.clone()` | steam.rs, steam.rs, steam.rs, steam.rs |
| `live:usb` | pill | `json!({ "letter": c.to_string(), "added": added })` | liveplus.rs |
| `live:volume` | pill | `json!({ "level": cur.0, "muted": cur.1 })` | livesys.rs |
| `media:changed` | pill | `&st` | media.rs |
| `pill:collapse` | pill | `()` | pill.rs |
| `pill:expand` | pill | `json!({ "tab": tab, "via": via })` | pill.rs |
| `pill:hide` | pill | `json!({ "reason": reason })` | pill.rs |
| `pill:open-tab` | pill | `p` | command.rs |
| `pill:show` | pill | `json!({ "reason": reason })` | pill.rs |
| `rec:state` | pill | `json!({ "recording": true, "startedAt": started, "mode": kind, "fps": opts.fps })`<br>`json!({ "recording": false, "startedAt": r.started })` | capture.rs, capture.rs |
| `remind:changed` | pill | `serde_json::Value::Null` | timers.rs, timers.rs |
| `remind:due` | pill | `serde_json::json!({ "id": d.id, "text": d.text, "kind": d.kind, "late": now - d.due_at > LATE_MS, "repeat": d.rule, "due_at": d.due_at })` | timers.rs |
| `settings:changed` | all windows | `&all` | settings.rs |
| `shelf:changed` | pill | `serde_json::json!({ "count": v.len(), "items": &v })` | shelf.rs |
| `sys:changed` | pill | `build_state()` | system.rs |
| `sysmon:tick` | pill | `sample()` | sysmon.rs |
| `timer:done` | pill | `serde_json::json!({ "id": d.id })` | timers.rs |
| `timer:tick` | pill | `serde_json::json!({ "id": t.id, "remaining_ms": t.remaining_ms, "total_s": t.total_s })` | timers.rs |
| `uninstall:focus` | bar | `json!({})` | uninstall.rs |
| `uninstall:item` | bar | `json!({ "shown": shown, "ok": ok, "error": err, "n": n, "total": total })` | uninstall.rs |
| `uninstall:ready` | all windows | `json!({ "name": name, "kind": kind, "count": count })` | uninstall.rs |
| `uninstall:state` | bar | `v` | uninstall.rs |
| `update:available` | all windows | `r` | updater.rs |
| `voice:level` | all windows, pill | `p` | voice.rs, voice.rs |
| `voice:state` | all windows | `p` | voice.rs |
| `weather:current` | pill | `v` | net.rs |

## Commands (175)

| Command | Args | Returns | Producer |
|---|---|---|---|
| `pill_rect` | `x: f64, y: f64, w: f64, h: f64` | `()` | pill.rs |
| `pill_set_state` | `state: String, tab: Option<String>, via: Option<String>` | `()` | pill.rs |
| `pill_toggle` | - | `()` | pill.rs |
| `pill_activate` | `on: bool` | `()` | pill.rs |
| `pill_auto_hide` | `on: bool` | `()` | pill.rs |
| `pill_monitors` | - | `Vec<String>` | pill.rs |
| `pill_set_gaze` | `on: bool` | `()` | pill.rs |
| `pill_cards` | `on: bool` | `()` | pill.rs |
| `pill_desktop_fg` | - | `bool` | guard.rs |
| `update_check` | - | `Value` | updater.rs |
| `update_install` | - | `Value` | updater.rs |
| `whatsnew_check` | - | `Value` | whatsnew.rs |
| `report_problem` | - | `Value` | report.rs |
| `media_state` | `known: Option<u64>` | `MediaState` | media.rs |
| `media_sessions` | - | `Vec<SessionInfo>` | media.rs |
| `media_select` | `id: Option<String>` | `()` | media.rs |
| `media_control` | `action: String` | `()` | media.rs |
| `media_seek` | `ms: u64` | `()` | media.rs |
| `media_volume_get` | - | `Volume` | media.rs |
| `media_volume_set` | `v: u32` | `()` | media.rs |
| `media_mute` | `muted: bool` | `()` | media.rs |
| `media_pause_updates` | - | `()` | media.rs |
| `media_resume_updates` | - | `()` | media.rs |
| `battery_state` | - | `Battery` | media.rs |
| `media_devices` | - | `Vec<Device>` | media.rs |
| `media_set_default_device` | `id: String` | `&'static str` | media.rs |
| `media_capture_devices` | - | `Vec<Device>` | media.rs |
| `audio_status` | - | `Status` | audio.rs |
| `clip_list` | `query: Option<String>, kind: Option<String>, limit: Option<i64>, offset: Option<i64>` | `Vec<Value>` | clipboard.rs |
| `clip_image` | `id: i64` | `Option<String>` | clipboard.rs |
| `clip_paste` | `id: i64, copy_only: Option<bool>, plain: Option<bool>` | `&'static str` | clipboard.rs |
| `clip_pin` | `id: i64, pinned: bool` | `()` | clipboard.rs |
| `clip_delete` | `id: i64` | `()` | clipboard.rs |
| `clip_wipe` | - | `()` | clipboard.rs |
| `clip_pause` | `paused: bool` | `()` | clipboard.rs |
| `clip_status` | - | `Value` | clipboard.rs |
| `clip_paste_text` | `text: String, copy_only: Option<bool>` | `&'static str` | clipboard.rs |
| `shelf_add` | `paths: Vec<String>` | `usize` | shelf.rs |
| `shelf_list` | - | `Vec<Item>` | shelf.rs |
| `shelf_remove` | `id: i64` | `()` | shelf.rs |
| `shelf_clear` | - | `()` | shelf.rs |
| `shelf_open` | `id: i64` | `bool` | shelf.rs |
| `shelf_reveal` | `id: i64` | `bool` | shelf.rs |
| `shelf_copy_path` | `id: i64` | `bool` | shelf.rs |
| `shelf_drag_out` | `ids: Vec<i64>` | `String` | shelf.rs |
| `cmd_toggle` | - | `()` | command.rs |
| `cmd_query` | `text: String` | `Vec<Res>` | command.rs |
| `cmd_run` | `id: String, mode: String` | `Outcome` | command.rs |
| `cmd_icon` | `id: String` | `Option<String>` | command.rs |
| `settings_get` | - | `Cfg` | settings.rs |
| `settings_set` | `patch: Cfg` | `Cfg` | settings.rs |
| `settings_export` | - | `Value` | settings_backup.rs |
| `settings_import` | `json_text: String` | `Value` | settings_backup.rs |
| `sys_state` | - | `SysState` | system.rs |
| `sys_set` | `key: String, value: Value` | `Value` | system.rs |
| `sys_power` | `action: String, confirmed: bool` | `Value` | system.rs |
| `weather_get` | `force: Option<bool>, tz: Option<i64>` | `Value` | net.rs |
| `weather_search_city` | `query: String, lang: Option<String>` | `Value` | net.rs |
| `weather_set_city` | `name: String, lat: f64, lon: f64` | `bool` | net.rs |
| `net_features_status` | - | `Value` | net.rs |
| `weather_current` | - | `Value` | net.rs |
| `chat_send` | `thread: Option<i64>, text: String, model: Option<String>` | `Value` | chat.rs |
| `chat_retry` | `thread: i64, model: Option<String>` | `Value` | chat.rs |
| `chat_stop` | `thread: Option<i64>` | `bool` | chat.rs |
| `chat_threads` | - | `Value` | chat.rs |
| `chat_messages` | `thread: i64` | `Value` | chat.rs |
| `chat_new` | `title: Option<String>` | `Value` | chat.rs |
| `chat_delete` | `thread: i64` | `bool` | chat.rs |
| `chat_open_link` | `url: String` | `bool` | chat.rs |
| `ai_test` | - | `Value` | chat.rs |
| `ai_models` | - | `Value` | chat.rs |
| `ai_probe` | - | `Value` | chat.rs |
| `ai_set_key` | `key: String` | `Value` | chat.rs |
| `ai_config` | - | `Value` | chat.rs |
| `ai_transform` | `text: String, action: String, instruction: Option<String>` | `Value` | aiactions.rs |
| `ai_transform_cancel` | - | `bool` | aiactions.rs |
| `ai_clip_text` | `id: i64` | `Option<String>` | aiactions.rs |
| `cmd_ask` | `question: String, history: Option<Vec<Turn>>` | `Value` | aiactions.rs |
| `note_list` | `folder_id: Option<i64>` | `Vec<NoteMeta>` | notes.rs |
| `note_get` | `id: i64` | `Option<Note>` | notes.rs |
| `note_save` | `id: Option<i64>, body_html: String, title: Option<String>` | `Result<NoteMeta, String>` | notes.rs |
| `note_delete` | `id: i64` | `()` | notes.rs |
| `remind_add` | `text: String` | `Result<Reminder, String>` | timers.rs |
| `remind_list` | - | `Vec<Reminder>` | timers.rs |
| `remind_done` | `id: i64` | `()` | timers.rs |
| `remind_delete` | `id: i64` | `()` | timers.rs |
| `remind_snooze` | `id: i64, minutes: i64` | `()` | timers.rs |
| `timer_start` | `seconds: i64` | `TimerState` | timers.rs |
| `timer_cancel` | - | `TimerState` | timers.rs |
| `timer_state` | - | `TimerState` | timers.rs |
| `sound_play` | `tone: Option<String>` | `()` | timers.rs |
| `kbd_state` | - | `Option<Value>` | watch.rs |
| `watch_pause` | - | `()` | watch.rs |
| `watch_resume` | - | `()` | watch.rs |
| `livesys_visible` | `on: bool` | `()` | livesys.rs |
| `livesys_privacy` | - | `Value` | livesys.rs |
| `livesys_reveal` | `path: String` | `bool` | livesys.rs |
| `sysmon_get` | - | `Value` | sysmon.rs |
| `liveplus_ics_status` | - | `Value` | liveplus.rs |
| `mail_status` | - | `Value` | mail.rs |
| `mail_set_password` | `password: String` | `bool` | mail.rs |
| `mail_test` | - | `Value` | mail.rs |
| `profile_save` | `name: String` | `Value` | settings.rs |
| `profile_apply` | `name: String` | `Value` | settings.rs |
| `profile_delete` | `name: String` | `Value` | settings.rs |
| `agents_list` | - | `Value` | agents.rs |
| `agents_dismiss` | `session: String` | `bool` | agents.rs |
| `agents_hook_snippets` | - | `Value` | agents.rs |
| `agents_open_folder` | `session: String` | `bool` | agents.rs |
| `agents_open_code` | `session: String` | `bool` | agents.rs |
| `agents_tools` | - | `Value` | agents.rs |
| `launcher_list` | - | `Vec<Link>` | launcher.rs |
| `launcher_add` | `paths: Vec<String>` | `usize` | launcher.rs |
| `launcher_remove` | `id: i64` | `()` | launcher.rs |
| `launcher_open` | `id: i64` | `bool` | launcher.rs |
| `launcher_move` | `id: i64, new_index: usize` | `()` | launcher.rs |
| `settings_open` | `page: Option<String>` | `()` | settingswin.rs |
| `settings_close` | - | `()` | settingswin.rs |
| `settings_quit` | - | `()` | settingswin.rs |
| `settings_info` | - | `Value` | settingswin.rs |
| `autostart_get` | - | `bool` | settingswin.rs |
| `autostart_set` | `on: bool` | `Result<bool, String>` | settingswin.rs |
| `hotkey_check` | `combo: String, action: Option<String>` | `Value` | settingswin.rs |
| `hotkey_set` | `action: String, combo: String` | `Value` | settingswin.rs |
| `everything_status` | - | `Value` | everything.rs |
| `everything_download` | - | `Value` | everything.rs |
| `everything_run_installer` | - | `Value` | everything.rs |
| `capture_start` | `mode: String` | `()` | capture.rs |
| `capture_commit` | `request: tauri::ipc::Request<'_>` | `Result<(), String>` | capture.rs |
| `capture_cancel` | - | `()` | capture.rs |
| `capture_stop` | - | `()` | capture.rs |
| `capture_settings` | - | `Value` | capture.rs |
| `capture_session` | `label: String` | `Option<Value>` | capture.rs |
| `capture_frame` | `label: String` | `tauri::ipc::Response` | capture.rs |
| `capture_ready` | `label: String` | `()` | capture.rs |
| `capture_action` | `action: String, value: String` | `Result<(), String>` | capture.rs |
| `voice_toggle` | - | `Value` | voice.rs |
| `voice_start` | - | `Value` | voice.rs |
| `voice_stop` | - | `Value` | voice.rs |
| `voice_cancel` | - | `Value` | voice.rs |
| `voice_state` | - | `Value` | voice.rs |
| `voice_test_mic` | - | `Value` | voice.rs |
| `voice_test_service` | - | `Value` | voice.rs |
| `voice_set_key` | `provider: String, key: String` | `Value` | voice.rs |
| `voice_has_key` | `provider: Option<String>` | `Value` | voice.rs |
| `aisetup_status` | - | `Value` | aisetup.rs |
| `aisetup_whisper_install` | `model: String` | `Value` | aisetup.rs |
| `aisetup_whisper_set` | `on: bool, model: Option<String>` | `Value` | aisetup.rs |
| `aisetup_whisper_remove` | - | `Value` | aisetup.rs |
| `aisetup_ollama_download` | - | `Value` | aisetup.rs |
| `aisetup_ollama_run_installer` | - | `Value` | aisetup.rs |
| `aisetup_ollama_pull` | `model: String` | `Value` | aisetup.rs |
| `aisetup_ai_use` | `server: String, model: String` | `Value` | aisetup.rs |
| `aisetup_cancel` | - | `Value` | aisetup.rs |
| `uninstall_info` | `id: usize, name: String` | `Value` | uninstall.rs |
| `uninstall_start` | `silent: bool` | `Value` | uninstall.rs |
| `uninstall_scan` | - | `Value` | uninstall.rs |
| `uninstall_delete` | `ids: Vec<usize>` | `Value` | uninstall.rs |
| `uninstall_admin` | `mode: String, ids: Vec<usize>` | `Value` | uninstall.rs |
| `uninstall_cancel` | - | `()` | uninstall.rs |
| `uninstall_state` | - | `Value` | uninstall.rs |
| `uninstall_open_panel` | - | `()` | uninstall.rs |
| `uninstall_open_backups` | - | `()` | uninstall.rs |
| `hotkeys_reload` | - | `Result<Value, String>` | hotkeys.rs |
| `obs_status` | - | `Value` | obs.rs |
| `obs_set_password` | `password: String` | `bool` | obs.rs |
| `obs_reveal` | `path: String` | `bool` | obs.rs |
| `steam_status` | - | `Value` | steam.rs |
| `notif_status` | - | `Value` | notif.rs |
| `notif_request` | - | `Value` | notif.rs |
| `notif_history_add` | `entry: NewEntry, read: Option<bool>` | `Option<Entry>` | notifhist.rs |
| `notif_history_list` | - | `Vec<Entry>` | notifhist.rs |
| `notif_history_clear` | - | `()` | notifhist.rs |
| `notif_history_mark_read` | `ids: Option<Vec<i64>>` | `usize` | notifhist.rs |
| `notif_history_remove` | `id: i64` | `()` | notifhist.rs |

