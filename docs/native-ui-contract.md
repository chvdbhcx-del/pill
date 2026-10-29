# Native UI contract (derived by grep, Phase 0)

Everything the Rust backend and the WebView front end say to each other today; the native UI has to implement the same names and shapes. Generated mechanically from the sources (event producers: `bus::emit` call sites and the small local `emit` helpers; consumers: string literals in `src/*.js`; commands: `generate_handler!` in `main.rs` plus the `#[tauri::command]` signatures). Regenerate rather than hand-edit.

- **Events** (backend to UI) go through `ui::bus::emit(app, Target, name, payload)` (`src-tauri/src/ui/bus.rs`); native code can `bus::subscribe(name, f)` to the same stream and gets the payload as JSON. "Payload as emitted" is the Rust expression at the producer; JSON object keys are the field names.
- **Commands** (UI to backend) are `#[tauri::command]` functions registered in `generate_handler!` (`src-tauri/src/main.rs`). On the JS side argument names are camelCase (Tauri converts); `AppHandle` / window arguments are injected and not listed. Errors: a `Result` command rejects the promise with the `Err` text; the pages use wrappers that turn that into `null`.
- Window labels: `pill` (always alive), `bar` (created on demand, `ui/lazy.rs`), `settings` (on demand, `settingswin.rs`), `capture-N` (one overlay per monitor, `capture.rs`). The first two are built by `ui/windows.rs`.

## Events (56)

| Event | Target | Payload as emitted | Producer | JS consumer |
|---|---|---|---|---|
| `agents:changed` | pill | `snap` | agents.rs:361 | agents-live.js |
| `agents:done` | pill | `json!({ "session": s.session, "tool": s.tool, "title": s.title, "cwd": s.cwd })` | agents.rs:364 | agents-live.js |
| `aisetup:progress` | all windows | `json!({ "id": id, "item": item, "done": done, "total": total, "phase": phase, "message": message })` | aisetup.rs:275 | ai-setup.js, settings.js |
| `audio:bands` | pill | `&Frame { b, rms: (rms * 1000.0).round() / 1000.0, t }` | audio.rs:279 | characters.js, pets.js, pill.js |
| `bar:hidden` | bar / settings (own window) | `()` | command.rs:229 | bar.js |
| `bar:shown` | bar / settings (own window) | `()` | command.rs:204, command.rs:317 | bar.js, uninstall-ui.js |
| `capture:begin` | capture-N overlay | `json!({ "id": id })` | capture.rs:725 | capture.js |
| `capture:done` | pill | `json!({ "kind": "error", "message": msg })`<br>`json!({ "kind": "screenshot", "path": path, "copied": copied, "w": w, "h": h })`<br>`json!({ "kind": "text", "found": false })`<br>`json!({ "kind": "text", "found": true, "text": head, "chars": t.chars().count(), "copied": copied })`<br>`json!({ "kind": "qr", "found": true, "text": head, "isUrl": is_http_url(&t) })`<br>`json!({ "kind": "qr", "found": false })`<br>`json!({ "kind": "color", "hex": hex, "rgb": rgb, "copied": copied, "history": color_history() })`<br>`json!({ "kind": "record", "path": p, "copied": copied, "seconds": (now_ms() - r.started) / 1000 })` | capture.rs:649, capture.rs:891, capture.rs:896, capture.rs:900, capture.rs:910, capture.rs:912, capture.rs:932, capture.rs:1191 | capture-live.js |
| `capture:end` | capture-N overlay | `json!({})` | capture.rs:771 | capture.js |
| `chat:delta` | pill | `json!({ "thread": thread, "run": run, "text": std::mem::take(&mut pending) })`<br>`json!({ "thread": thread, "run": run, "text": pending })` | chat.rs:192, chat.rs:229 | tab-chat.js |
| `chat:done` | pill | `json!({ "thread": thread, "run": run, "ok": error.is_none(), "stopped": stopped, "error": error, "id": id })` | chat.rs:243 | tab-chat.js |
| `chat:focus` | pill | `()` | hotkeys.rs:76 | pill.js |
| `clip:added` | pill | `item` | clipboard.rs:365 | live-events.js, tab-clipboard.js |
| `clip:open` | pill | `()` | hotkeys.rs:72 | tab-clipboard.js |
| `kbd:layout` | pill | `info(new)` | watch.rs:102 | live-events.js |
| `launcher:changed` | pill | `serde_json::json!({ "count": v.len(), "items": &v })` | launcher.rs:97 | tab-home.js |
| `live:battery` | pill | `json!({ "kind": kind, "percent": percent })` | liveplus.rs:734 | live-plus.js |
| `live:calendar` | pill | `json!({ "title": title, "minutes": minutes })` | liveplus.rs:693 | live-plus.js |
| `live:download` | pill | `json!({ "state": "progress", "id": id, "name": shown_name, "bytes": t.size, "total": Value::Null, "path": dir.join(name).to_string_lossy(), })`<br>`json!({ "state": "done", "id": id, "name": c.name, "bytes": bytes, "total": Value::Null, "path": p.to_string_lossy(), })`<br>`json!({ "state": "gone", "id": id })` | livesys.rs:374, livesys.rs:403, livesys.rs:408 | live-plus.js, live-system.js |
| `live:file-done` | pill | `json!({ "name": first, "path": path.to_string_lossy(), "more": ready.len() - 1 })` | liveplus.rs:712 | live-plus.js |
| `live:mail` | pill | `json!({ "from": from, "subject": subject, "more": f.fresh - 1 })` | mail.rs:676 | live-plus.js |
| `live:notification` | pill | `p` | notif.rs:143 | live-ext.js |
| `live:obs` | pill | `json!({ "recording": false, "streaming": false, "recMs": 0, "streamMs": 0 })`<br>`p` | obs.rs:188, obs.rs:267, obs.rs:340 | live-ext.js |
| `live:privacy` | pill | `json!({ "mic": now.0, "cam": now.1, "apps": now.2 })` | livesys.rs:300 | live-system.js |
| `live:steam` | pill | `json!({ "state": "idle" })`<br>`done`<br>`p.clone()` | steam.rs:455, steam.rs:493, steam.rs:548, steam.rs:575 | live-ext.js |
| `live:usb` | pill | `json!({ "letter": c.to_string(), "added": added })` | liveplus.rs:720 | live-plus.js |
| `live:volume` | pill | `json!({ "level": cur.0, "muted": cur.1 })` | livesys.rs:134 | live-system.js, tab-home.js |
| `media:changed` | pill | `&st` | media.rs:446 | tab-home.js |
| `pill:collapse` | pill | `()` | pill.rs:252 | pill.js |
| `pill:cursor` | pill | `c`<br>`Cur { x: None, y: None }` | pill.rs:651, pill.rs:655 | characters.js, pets.js, pill.js |
| `pill:display` | pill | `json!({ "hz": hz })` | pill.rs:562 | - |
| `pill:expand` | pill | `json!({ "tab": tab, "via": via })` | pill.rs:247 | pill.js |
| `pill:gaze` | pill | `json!({ "x": p.x - wr.left, "y": p.y - wr.top })` | pill.rs:665 | characters.js, pets.js |
| `pill:hide` | pill | `json!({ "reason": reason })` | pill.rs:210 | capture-live.js, characters.js, live-events.js, live-ext.js, live-system.js, pets.js, pill.js, tab-controls.js, tab-home.js, tab-note.js, tab-shelf.js, voice-live.js |
| `pill:hover` | pill | `()` | pill.rs:629 | pill.js |
| `pill:leave` | pill | `()` | pill.rs:637 | characters.js, pets.js, pill.js |
| `pill:open-tab` | pill | `p` | command.rs:1398 | pill.js, tab-note.js |
| `pill:show` | pill | `json!({ "reason": reason })` | pill.rs:208 | capture-live.js, characters.js, live-events.js, live-ext.js, live-system.js, pets.js, pill.js, tab-controls.js, tab-home.js, tab-note.js, voice-live.js |
| `rec:state` | pill | `json!({ "recording": true, "startedAt": started, "mode": kind, "fps": opts.fps })`<br>`json!({ "recording": false, "startedAt": r.started })` | capture.rs:1101, capture.rs:1176 | capture-live.js |
| `remind:changed` | pill | `serde_json::Value::Null` | timers.rs:476, timers.rs:499 | tab-note.js |
| `remind:due` | pill | `serde_json::json!({ "id": d.id, "text": d.text, "kind": d.kind, "late": now - d.due_at > LATE_MS, "repeat": d.rule, "due_at": d.due_at })` | timers.rs:474 | tab-note.js |
| `settings:changed` | all windows | `&all` | settings.rs:226 | agents-live.js, capture-live.js, characters.js, live-ext.js, live-system.js, pets.js, pill.js, settings.js, sysmon-live.js, tab-chat.js, tab-home.js, tab-notifs.js, tab-shelf.js, tab-weather.js (+ Rust listeners: notif.rs, obs.rs, steam.rs, voice.rs) |
| `settings:page` | bar / settings (own window) | `p` | settingswin.rs:30 | settings.js |
| `shelf:changed` | pill | `serde_json::json!({ "count": v.len(), "items": &v })` | shelf.rs:91 | tab-shelf.js |
| `sys:changed` | pill | `build_state()` | system.rs:215 | tab-controls.js |
| `sysmon:tick` | pill | `sample()` | sysmon.rs:263 | sysmon-live.js |
| `timer:done` | pill | `serde_json::json!({ "id": d.id })` | timers.rs:475 | - |
| `timer:tick` | pill | `serde_json::json!({ "id": t.id, "remaining_ms": t.remaining_ms, "total_s": t.total_s })` | timers.rs:486 | tab-note.js |
| `uninstall:focus` | bar | `json!({})` | uninstall.rs:2017 | uninstall-ui.js |
| `uninstall:item` | bar | `json!({ "shown": shown, "ok": ok, "error": err, "n": n, "total": total })` | uninstall.rs:1943 | uninstall-ui.js |
| `uninstall:ready` | all windows | `json!({ "name": name, "kind": kind, "count": count })` | uninstall.rs:2030 | live-ext.js |
| `uninstall:state` | bar | `v` | uninstall.rs:1761 | uninstall-state.js, uninstall-ui.js |
| `update:available` | all windows | `r` | updater.rs:53 | live-system.js |
| `voice:level` | all windows, pill | `p` | voice.rs:365, voice.rs:367 | voice-live.js, voice-ui.js |
| `voice:state` | all windows | `p` | voice.rs:550 | bar.js, tab-chat.js, voice-live.js |
| `weather:current` | pill | `v` | net.rs:1150 | pill.js |

Built-in Tauri events the pages also use: `tauri://drag-enter`, `tauri://drag-over`, `tauri://drag-drop`, `tauri://drag-leave` (tab-home.js, tab-shelf.js: files dropped on the pill). The bar page also registers and immediately drops a listener named `bar:ready-barrier` (bar-ready.js; never emitted) and calls `cmd_ready` once its listeners are in place.

## Commands (176)

| Command | Args | Returns | Producer | JS callers |
|---|---|---|---|---|
| `pill_rect` | `x: f64, y: f64, w: f64, h: f64` | `()` | pill.rs:289 | pill.js |
| `pill_set_state` | `state: String, tab: Option<String>, via: Option<String>` | `()` | pill.rs:294 | pill.js |
| `pill_toggle` | - | `()` | pill.rs:305 | - |
| `pill_activate` | `on: bool` | `()` | pill.rs:317 | pill.js |
| `pill_auto_hide` | `on: bool` | `()` | pill.rs:311 | pill.js |
| `pill_monitors` | - | `Vec<String>` | pill.rs:354 | pill.js, settings.js |
| `pill_set_gaze` | `on: bool` | `()` | pill.rs:420 | characters.js, pets.js |
| `pill_cards` | `on: bool` | `()` | pill.rs:413 | pill.js |
| `pill_desktop_fg` | - | `bool` | guard.rs:59 | pill.js |
| `update_check` | - | `Value` | updater.rs:32 | settings.js |
| `update_install` | - | `Value` | updater.rs:38 | settings.js |
| `whatsnew_check` | - | `Value` | whatsnew.rs:34 | whatsnew-live.js |
| `report_problem` | - | `Value` | report.rs:125 | settings.js |
| `media_state` | `known: Option<u64>` | `MediaState` | media.rs:477 | tab-home.js |
| `media_sessions` | - | `Vec<SessionInfo>` | media.rs:487 | tab-home.js |
| `media_select` | `id: Option<String>` | `()` | media.rs:509 | tab-home.js |
| `media_control` | `action: String` | `()` | media.rs:516 | tab-home.js |
| `media_seek` | `ms: u64` | `()` | media.rs:530 | tab-home.js |
| `media_volume_get` | - | `Volume` | media.rs:572 | live-system.js, tab-home.js |
| `media_volume_set` | `v: u32` | `()` | media.rs:581 | live-system.js, tab-home.js |
| `media_mute` | `muted: bool` | `()` | media.rs:590 | live-system.js, tab-home.js |
| `media_pause_updates` | - | `()` | media.rs:153 | tab-home.js |
| `media_resume_updates` | - | `()` | media.rs:158 | tab-home.js |
| `battery_state` | - | `Battery` | media.rs:741 | pill.js |
| `media_devices` | - | `Vec<Device>` | media.rs:704 | settings.js, tab-home.js |
| `media_set_default_device` | `id: String` | `&'static str` | media.rs:717 | tab-home.js |
| `media_capture_devices` | - | `Vec<Device>` | media.rs:710 | settings.js |
| `audio_status` | - | `Status` | audio.rs:245 | - |
| `clip_list` | `query: Option<String>, kind: Option<String>, limit: Option<i64>, offset: Option<i64>` | `Vec<Value>` | clipboard.rs:479 | tab-clipboard.js |
| `clip_image` | `id: i64` | `Option<String>` | clipboard.rs:489 | tab-clipboard.js |
| `clip_paste` | `id: i64, copy_only: Option<bool>, plain: Option<bool>` | `&'static str` | clipboard.rs:624 | tab-clipboard.js |
| `clip_pin` | `id: i64, pinned: bool` | `()` | clipboard.rs:497 | tab-clipboard.js |
| `clip_delete` | `id: i64` | `()` | clipboard.rs:502 | tab-clipboard.js |
| `clip_wipe` | - | `()` | clipboard.rs:509 | settings.js, tab-clipboard.js |
| `clip_pause` | `paused: bool` | `()` | clipboard.rs:529 | pill.js, settings.js |
| `clip_status` | - | `Value` | clipboard.rs:534 | pill.js, settings.js, tab-clipboard.js |
| `clip_paste_text` | `text: String, copy_only: Option<bool>` | `&'static str` | clipboard.rs:657 | ai-actions.js |
| `shelf_add` | `paths: Vec<String>` | `usize` | shelf.rs:96 | tab-shelf.js |
| `shelf_list` | - | `Vec<Item>` | shelf.rs:120 | tab-shelf.js |
| `shelf_remove` | `id: i64` | `()` | shelf.rs:125 | tab-shelf.js |
| `shelf_clear` | - | `()` | shelf.rs:131 | tab-shelf.js |
| `shelf_open` | `id: i64` | `bool` | shelf.rs:137 | tab-shelf.js |
| `shelf_reveal` | `id: i64` | `bool` | shelf.rs:146 | tab-shelf.js |
| `shelf_copy_path` | `id: i64` | `bool` | shelf.rs:158 | tab-shelf.js |
| `shelf_drag_out` | `ids: Vec<i64>` | `String` | shelf.rs:220 | tab-shelf.js |
| `cmd_toggle` | - | `()` | command.rs:355 | bar.js, pill.js |
| `cmd_query` | `text: String` | `Vec<Res>` | command.rs:360 | bar.js |
| `cmd_run` | `id: String, mode: String` | `Outcome` | command.rs:879 | bar.js |
| `cmd_icon` | `id: String` | `Option<String>` | command.rs:373 | bar.js |
| `cmd_ready` | - | `()` | command.rs:200 | bar-ready.js |
| `settings_get` | - | `Cfg` | settings.rs:175 | bar.js, characters.js, pets.js, pill.js, settings.js, tab-home.js, tab-weather.js |
| `settings_set` | `patch: Cfg` | `Cfg` | settings.rs:180 | pill.js, settings.js, tab-chat.js, tab-weather.js |
| `settings_export` | - | `Value` | settings_backup.rs:108 | settings.js |
| `settings_import` | `json_text: String` | `Value` | settings_backup.rs:127 | settings.js |
| `sys_state` | - | `SysState` | system.rs:220 | tab-controls.js |
| `sys_set` | `key: String, value: Value` | `Value` | system.rs:303 | tab-controls.js |
| `sys_power` | `action: String, confirmed: bool` | `Value` | system.rs:382 | tab-controls.js |
| `weather_get` | `force: Option<bool>, tz: Option<i64>` | `Value` | net.rs:980 | pill.js, settings.js, tab-weather.js |
| `weather_search_city` | `query: String, lang: Option<String>` | `Value` | net.rs:1011 | settings.js, tab-weather.js |
| `weather_set_city` | `name: String, lat: f64, lon: f64` | `bool` | net.rs:1065 | settings.js, tab-weather.js |
| `net_features_status` | - | `Value` | net.rs:91 | settings.js, tab-chat.js, voice-ui.js |
| `weather_current` | - | `Value` | net.rs:1174 | - |
| `chat_send` | `thread: Option<i64>, text: String, model: Option<String>` | `Value` | chat.rs:250 | tab-chat.js |
| `chat_retry` | `thread: i64, model: Option<String>` | `Value` | chat.rs:280 | tab-chat.js |
| `chat_stop` | `thread: Option<i64>` | `bool` | chat.rs:302 | tab-chat.js |
| `chat_threads` | - | `Value` | chat.rs:311 | tab-chat.js |
| `chat_messages` | `thread: i64` | `Value` | chat.rs:327 | tab-chat.js |
| `chat_new` | `title: Option<String>` | `Value` | chat.rs:338 | - |
| `chat_delete` | `thread: i64` | `bool` | chat.rs:346 | tab-chat.js |
| `chat_open_link` | `url: String` | `bool` | chat.rs:357 | settings.js, tab-chat.js, tab-note.js |
| `ai_test` | - | `Value` | chat.rs:372 | settings.js, tab-chat.js |
| `ai_models` | - | `Value` | chat.rs:388 | settings.js, tab-chat.js |
| `ai_probe` | - | `Value` | chat.rs:413 | tab-chat.js |
| `ai_set_key` | `key: String` | `Value` | chat.rs:447 | settings.js, tab-chat.js |
| `ai_config` | - | `Value` | chat.rs:467 | settings.js, tab-chat.js |
| `ai_transform` | `text: String, action: String, instruction: Option<String>` | `Value` | aiactions.rs:132 | ai-actions.js, tab-text.js |
| `ai_transform_cancel` | - | `bool` | aiactions.rs:269 | ai-actions.js, bar.js, tab-text.js |
| `ai_clip_text` | `id: i64` | `Option<String>` | aiactions.rs:276 | ai-actions.js |
| `cmd_ask` | `question: String, history: Option<Vec<Turn>>` | `Value` | aiactions.rs:243 | bar.js |
| `note_list` | `folder_id: Option<i64>` | `Vec<NoteMeta>` | notes.rs:22 | tab-note.js |
| `note_get` | `id: i64` | `Option<Note>` | notes.rs:31 | tab-note.js |
| `note_save` | `id: Option<i64>, body_html: String, title: Option<String>` | `Result<NoteMeta, String>` | notes.rs:41 | tab-note.js |
| `note_delete` | `id: i64` | `()` | notes.rs:55 | tab-note.js |
| `remind_add` | `text: String` | `Result<Reminder, String>` | timers.rs:503 | tab-note.js |
| `remind_list` | - | `Vec<Reminder>` | timers.rs:518 | tab-note.js |
| `remind_done` | `id: i64` | `()` | timers.rs:528 | tab-note.js |
| `remind_delete` | `id: i64` | `()` | timers.rs:531 | tab-note.js |
| `remind_snooze` | `id: i64, minutes: i64` | `()` | timers.rs:535 | tab-note.js |
| `timer_start` | `seconds: i64` | `TimerState` | timers.rs:567 | tab-note.js |
| `timer_cancel` | - | `TimerState` | timers.rs:579 | tab-note.js |
| `timer_state` | - | `TimerState` | timers.rs:563 | tab-note.js |
| `sound_play` | `tone: Option<String>` | `()` | timers.rs:454 | settings.js |
| `kbd_state` | - | `Option<Value>` | watch.rs:119 | - |
| `watch_pause` | - | `()` | watch.rs:124 | live-events.js |
| `watch_resume` | - | `()` | watch.rs:129 | live-events.js |
| `livesys_visible` | `on: bool` | `()` | livesys.rs:474 | live-system.js |
| `livesys_privacy` | - | `Value` | livesys.rs:480 | live-system.js |
| `livesys_reveal` | `path: String` | `bool` | livesys.rs:490 | live-plus.js, live-system.js |
| `sysmon_get` | - | `Value` | sysmon.rs:254 | sysmon-live.js |
| `liveplus_ics_status` | - | `Value` | liveplus.rs:769 | settings-live-ext.js |
| `mail_status` | - | `Value` | mail.rs:714 | settings-live-ext.js |
| `mail_set_password` | `password: String` | `bool` | mail.rs:725 | settings-live-ext.js |
| `mail_test` | - | `Value` | mail.rs:739 | settings-live-ext.js |
| `profile_save` | `name: String` | `Value` | settings.rs:276 | settings.js |
| `profile_apply` | `name: String` | `Value` | settings.rs:287 | settings.js |
| `profile_delete` | `name: String` | `Value` | settings.rs:295 | settings.js |
| `agents_list` | - | `Value` | agents.rs:473 | agents-live.js |
| `agents_dismiss` | `session: String` | `bool` | agents.rs:479 | agents-live.js |
| `agents_hook_snippets` | - | `Value` | agents.rs:549 | agents-live.js, settings.js |
| `agents_open_folder` | `session: String` | `bool` | agents.rs:531 | agents-live.js |
| `agents_open_code` | `session: String` | `bool` | agents.rs:537 | agents-live.js |
| `agents_tools` | - | `Value` | agents.rs:544 | agents-live.js |
| `launcher_list` | - | `Vec<Link>` | launcher.rs:102 | settings.js, tab-home.js |
| `launcher_add` | `paths: Vec<String>` | `usize` | launcher.rs:108 | tab-home.js |
| `launcher_remove` | `id: i64` | `()` | launcher.rs:162 | tab-home.js |
| `launcher_open` | `id: i64` | `bool` | launcher.rs:170 | tab-home.js |
| `launcher_move` | `id: i64, new_index: usize` | `()` | launcher.rs:187 | tab-home.js |
| `settings_open` | `page: Option<String>` | `()` | settingswin.rs:154 | ai-actions.js, bar.js, live-system.js, pill.js, voice-live.js, whatsnew-live.js |
| `settings_close` | - | `()` | settingswin.rs:159 | ai-setup.js, settings.js |
| `settings_quit` | - | `()` | settingswin.rs:166 | settings.js |
| `settings_info` | - | `Value` | settingswin.rs:172 | settings.js |
| `autostart_get` | - | `bool` | settingswin.rs:186 | settings.js |
| `autostart_set` | `on: bool` | `Result<bool, String>` | settingswin.rs:192 | settings.js |
| `hotkey_check` | `combo: String, action: Option<String>` | `Value` | settingswin.rs:362 | - |
| `hotkey_set` | `action: String, combo: String` | `Value` | settingswin.rs:369 | settings.js |
| `everything_status` | - | `Value` | everything.rs:90 | settings.js |
| `everything_download` | - | `Value` | everything.rs:105 | settings.js |
| `everything_run_installer` | - | `Value` | everything.rs:130 | settings.js |
| `capture_start` | `mode: String` | `()` | capture.rs:1199 | capture-live.js |
| `capture_commit` | `request: tauri::ipc::Request<'_>` | `Result<(), String>` | capture.rs:1311 | capture.js |
| `capture_cancel` | - | `()` | capture.rs:1204 | capture.js |
| `capture_stop` | - | `()` | capture.rs:1209 | capture-live.js |
| `capture_settings` | - | `Value` | capture.rs:1214 | capture-live.js |
| `capture_session` | `label: String` | `Option<Value>` | capture.rs:1234 | capture.js |
| `capture_frame` | `label: String` | `tauri::ipc::Response` | capture.rs:1243 | capture.js |
| `capture_ready` | `label: String` | `()` | capture.rs:1263 | capture.js |
| `capture_action` | `action: String, value: String` | `Result<(), String>` | capture.rs:1366 | capture-live.js, voice-live.js |
| `voice_toggle` | - | `Value` | voice.rs:817 | voice-live.js |
| `voice_start` | - | `Value` | voice.rs:811 | bar.js |
| `voice_stop` | - | `Value` | voice.rs:823 | bar.js, voice-live.js |
| `voice_cancel` | - | `Value` | voice.rs:829 | - |
| `voice_state` | - | `Value` | voice.rs:835 | bar.js, voice-live.js |
| `voice_test_mic` | - | `Value` | voice.rs:848 | voice-ui.js |
| `voice_test_service` | - | `Value` | voice.rs:875 | voice-ui.js |
| `voice_set_key` | `provider: String, key: String` | `Value` | voice.rs:892 | voice-ui.js |
| `voice_has_key` | `provider: Option<String>` | `Value` | voice.rs:912 | ai-setup.js, voice-ui.js |
| `aisetup_status` | - | `Value` | aisetup.rs:632 | ai-setup.js |
| `aisetup_whisper_install` | `model: String` | `Value` | aisetup.rs:539 | ai-setup.js |
| `aisetup_whisper_set` | `on: bool, model: Option<String>` | `Value` | aisetup.rs:561 | ai-setup.js |
| `aisetup_whisper_remove` | - | `Value` | aisetup.rs:584 | ai-setup.js |
| `aisetup_ollama_download` | - | `Value` | aisetup.rs:674 | ai-setup.js |
| `aisetup_ollama_run_installer` | - | `Value` | aisetup.rs:690 | ai-setup.js |
| `aisetup_ollama_pull` | `model: String` | `Value` | aisetup.rs:706 | ai-setup.js |
| `aisetup_ai_use` | `server: String, model: String` | `Value` | aisetup.rs:756 | ai-setup.js |
| `aisetup_cancel` | - | `Value` | aisetup.rs:308 | ai-setup.js, settings.js |
| `uninstall_info` | `id: usize, name: String` | `Value` | uninstall.rs:1784 | uninstall-ui.js |
| `uninstall_start` | `silent: bool` | `Value` | uninstall.rs:1854 | uninstall-ui.js |
| `uninstall_scan` | - | `Value` | uninstall.rs:1913 | uninstall-ui.js |
| `uninstall_delete` | `ids: Vec<usize>` | `Value` | uninstall.rs:1920 | uninstall-ui.js |
| `uninstall_admin` | `mode: String, ids: Vec<usize>` | `Value` | uninstall.rs:2088 | uninstall-ui.js |
| `uninstall_cancel` | - | `()` | uninstall.rs:1982 | uninstall-ui.js |
| `uninstall_state` | - | `Value` | uninstall.rs:2002 | uninstall-ui.js |
| `uninstall_open_panel` | - | `()` | uninstall.rs:2014 | live-ext.js |
| `uninstall_open_backups` | - | `()` | uninstall.rs:1974 | uninstall-ui.js |
| `hotkeys_reload` | - | `Result<Value, String>` | hotkeys.rs:318 | - |
| `obs_status` | - | `Value` | obs.rs:374 | settings-live-ext.js |
| `obs_set_password` | `password: String` | `bool` | obs.rs:381 | settings-live-ext.js |
| `obs_reveal` | `path: String` | `bool` | obs.rs:391 | live-ext.js |
| `steam_status` | - | `Value` | steam.rs:586 | settings-live-ext.js |
| `notif_status` | - | `Value` | notif.rs:162 | settings-live-ext.js |
| `notif_request` | - | `Value` | notif.rs:169 | settings-live-ext.js |
| `notif_history_add` | `entry: NewEntry, read: Option<bool>` | `Option<Entry>` | notifhist.rs:85 | tab-notifs.js |
| `notif_history_list` | - | `Vec<Entry>` | notifhist.rs:93 | tab-notifs.js |
| `notif_history_clear` | - | `()` | notifhist.rs:98 | tab-notifs.js |
| `notif_history_mark_read` | `ids: Option<Vec<i64>>` | `usize` | notifhist.rs:102 | tab-notifs.js |
| `notif_history_remove` | `id: i64` | `()` | notifhist.rs:107 | tab-notifs.js |

Invoked by the pages but not registered in `generate_handler!`: none (the same check runs in `node src/contract.test.mjs`).
