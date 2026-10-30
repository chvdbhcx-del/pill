// The command table of the facade (ui/api.rs): one arm per `#[tauri::command]` registered in main.rs `generate_handler!`, in that order.
// An arm reads the arguments with the names of the Rust parameters (`a.req("copy_only")` looks up the JS key `copyOnly`), calls the SAME function
// the Tauri wrapper is (the commands are plain `pub fn`s; `AppHandle` is the one injected parameter and comes from the stored handle) and turns
// the result into JSON, which is what the JS `await invoke(..)` gets. `async fn` commands are driven by `blk` (callers are worker threads).
// Keep every arm on its own line as `"name" => ..`: ui/api.rs's coverage test reads this file and checks it against the contract and main.rs.
// No command takes the calling window (only `AppHandle`), so `caller` is not consulted by any arm today; see docs/native-api.md.
use super::args::Args;
use crate::*;
use serde::Serialize;
use serde_json::Value;
use std::future::Future;
use tauri::AppHandle;

pub type Reply = Result<Value, String>;

fn ok<T: Serialize>(v: T) -> Reply {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// For commands that return `Result<_, String>`: the Err text is the rejection the JS promise would get.
fn res<T: Serialize>(r: Result<T, String>) -> Reply {
    r.and_then(ok)
}

fn blk<F: Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

/// Runs one command. `body` is the raw request body (only `capture_commit` takes one).
pub fn dispatch(app: &AppHandle, cmd: &str, v: &Value, caller: &str, body: Option<Vec<u8>>) -> Reply {
    let _ = caller; // no command reads the calling window today; the parameter keeps the contract stable (docs/native-api.md)
    if body.is_some() && cmd != "capture_commit" {
        return Err(format!("command `{cmd}` does not take a raw body"));
    }
    let a = Args::new(cmd, v)?;
    match cmd {
        "pill_rect" => ok(pill::pill_rect(a.req("x")?, a.req("y")?, a.req("w")?, a.req("h")?)),
        "pill_set_state" => ok(pill::pill_set_state(app.clone(), a.req("state")?, a.opt("tab")?, a.opt("via")?)),
        "pill_toggle" => ok(pill::pill_toggle(app.clone())),
        "pill_activate" => ok(pill::pill_activate(app.clone(), a.req("on")?)),
        "pill_auto_hide" => ok(pill::pill_auto_hide(app.clone(), a.req("on")?)),
        "pill_monitors" => ok(pill::pill_monitors(app.clone())),
        "pill_set_gaze" => ok(pill::pill_set_gaze(a.req("on")?)),
        "pill_cards" => ok(pill::pill_cards(app.clone(), a.req("on")?)),
        "pill_desktop_fg" => ok(guard::pill_desktop_fg()),
        "update_check" => ok(blk(updater::update_check(app.clone()))),
        "update_install" => ok(blk(updater::update_install(app.clone()))),
        "whatsnew_check" => ok(whatsnew::whatsnew_check()),
        "report_problem" => ok(report::report_problem()),
        "media_state" => ok(blk(media::media_state(a.opt("known")?))),
        "media_sessions" => ok(blk(media::media_sessions())),
        "media_select" => ok(blk(media::media_select(a.opt("id")?))),
        "media_control" => ok(blk(media::media_control(a.req("action")?))),
        "media_seek" => ok(blk(media::media_seek(a.req("ms")?))),
        "media_volume_get" => ok(blk(media::media_volume_get())),
        "media_volume_set" => ok(blk(media::media_volume_set(a.req("v")?))),
        "media_mute" => ok(blk(media::media_mute(a.req("muted")?))),
        "media_pause_updates" => ok(media::media_pause_updates()),
        "media_resume_updates" => ok(media::media_resume_updates()),
        "battery_state" => ok(media::battery_state()),
        "media_devices" => ok(blk(media::media_devices())),
        "media_set_default_device" => ok(blk(media::media_set_default_device(a.req("id")?))),
        "media_capture_devices" => ok(blk(media::media_capture_devices())),
        "audio_status" => ok(audio::audio_status()),
        "clip_list" => ok(blk(clipboard::clip_list(a.opt("query")?, a.opt("kind")?, a.opt("limit")?, a.opt("offset")?))),
        "clip_image" => ok(blk(clipboard::clip_image(a.req("id")?))),
        "clip_paste" => ok(blk(clipboard::clip_paste(a.req("id")?, a.opt("copy_only")?, a.opt("plain")?))),
        "clip_pin" => ok(blk(clipboard::clip_pin(a.req("id")?, a.req("pinned")?))),
        "clip_delete" => ok(blk(clipboard::clip_delete(a.req("id")?))),
        "clip_wipe" => ok(blk(clipboard::clip_wipe())),
        "clip_pause" => ok(blk(clipboard::clip_pause(a.req("paused")?))),
        "clip_status" => ok(blk(clipboard::clip_status())),
        "clip_paste_text" => ok(blk(clipboard::clip_paste_text(a.req("text")?, a.opt("copy_only")?))),
        "shelf_add" => ok(shelf::shelf_add(app.clone(), a.req("paths")?)),
        "shelf_list" => ok(shelf::shelf_list()),
        "shelf_remove" => ok(shelf::shelf_remove(app.clone(), a.req("id")?)),
        "shelf_clear" => ok(shelf::shelf_clear(app.clone())),
        "shelf_open" => ok(shelf::shelf_open(a.req("id")?)),
        "shelf_reveal" => ok(shelf::shelf_reveal(a.req("id")?)),
        "shelf_copy_path" => ok(shelf::shelf_copy_path(a.req("id")?)),
        "shelf_drag_out" => ok(shelf::shelf_drag_out(app.clone(), a.req("ids")?)),
        "cmd_toggle" => ok(command::cmd_toggle(app.clone())),
        "cmd_query" => ok(blk(command::cmd_query(a.req("text")?))),
        "cmd_run" => ok(command::cmd_run(app.clone(), a.req("id")?, a.req("mode")?)),
        "cmd_icon" => ok(blk(command::cmd_icon(a.req("id")?))),
        "cmd_ready" => ok(command::cmd_ready(app.clone())),
        "settings_get" => ok(settings::settings_get()),
        "settings_set" => ok(settings::settings_set(app.clone(), a.req("patch")?)),
        "settings_export" => ok(settings_backup::settings_export()),
        "settings_import" => ok(settings_backup::settings_import(app.clone(), a.req("json_text")?)),
        "sys_state" => ok(blk(system::sys_state())),
        "sys_set" => ok(blk(system::sys_set(a.req("key")?, a.req("value")?))),
        "sys_power" => ok(system::sys_power(a.req("action")?, a.req("confirmed")?)),
        "weather_get" => ok(net::weather_get(a.opt("force")?, a.opt("tz")?)),
        "weather_search_city" => ok(net::weather_search_city(a.req("query")?, a.opt("lang")?)),
        "weather_set_city" => ok(net::weather_set_city(a.req("name")?, a.req("lat")?, a.req("lon")?)),
        "net_features_status" => ok(net::net_features_status()),
        "weather_current" => ok(net::weather_current()),
        "chat_send" => ok(chat::chat_send(a.opt("thread")?, a.req("text")?, a.opt("model")?)),
        "chat_retry" => ok(chat::chat_retry(a.req("thread")?, a.opt("model")?)),
        "chat_stop" => ok(chat::chat_stop(a.opt("thread")?)),
        "chat_threads" => ok(chat::chat_threads()),
        "chat_messages" => ok(chat::chat_messages(a.req("thread")?)),
        "chat_new" => ok(chat::chat_new(a.opt("title")?)),
        "chat_delete" => ok(chat::chat_delete(a.req("thread")?)),
        "chat_open_link" => ok(chat::chat_open_link(a.req("url")?)),
        "ai_test" => ok(chat::ai_test()),
        "ai_models" => ok(chat::ai_models()),
        "ai_probe" => ok(chat::ai_probe()),
        "ai_set_key" => ok(chat::ai_set_key(a.req("key")?)),
        "ai_config" => ok(chat::ai_config()),
        "ai_transform" => ok(aiactions::ai_transform(a.req("text")?, a.req("action")?, a.opt("instruction")?)),
        "ai_transform_cancel" => ok(aiactions::ai_transform_cancel()),
        "ai_clip_text" => ok(aiactions::ai_clip_text(a.req("id")?)),
        "cmd_ask" => ok(aiactions::cmd_ask(a.req("question")?, a.opt("history")?)),
        "note_list" => ok(notes::note_list(a.opt("folder_id")?)),
        "note_get" => ok(notes::note_get(a.req("id")?)),
        "note_save" => res(notes::note_save(a.opt("id")?, a.req("body_html")?, a.opt("title")?)),
        "note_delete" => ok(notes::note_delete(a.req("id")?)),
        "remind_add" => res(timers::remind_add(a.req("text")?)),
        "remind_list" => ok(timers::remind_list()),
        "remind_done" => ok(timers::remind_done(a.req("id")?)),
        "remind_delete" => ok(timers::remind_delete(a.req("id")?)),
        "remind_snooze" => ok(timers::remind_snooze(a.req("id")?, a.req("minutes")?)),
        "timer_start" => ok(timers::timer_start(a.req("seconds")?)),
        "timer_cancel" => ok(timers::timer_cancel()),
        "timer_state" => ok(timers::timer_state()),
        "sound_play" => ok(timers::sound_play(a.opt("tone")?)),
        "kbd_state" => ok(watch::kbd_state()),
        "watch_pause" => ok(watch::watch_pause()),
        "watch_resume" => ok(watch::watch_resume()),
        "livesys_visible" => ok(livesys::livesys_visible(a.req("on")?)),
        "livesys_privacy" => ok(livesys::livesys_privacy()),
        "livesys_reveal" => ok(livesys::livesys_reveal(a.req("path")?)),
        "sysmon_get" => ok(blk(sysmon::sysmon_get())),
        "liveplus_ics_status" => ok(liveplus::liveplus_ics_status()),
        "mail_status" => ok(mail::mail_status()),
        "mail_set_password" => ok(mail::mail_set_password(app.clone(), a.req("password")?)),
        "mail_test" => ok(mail::mail_test()),
        "profile_save" => ok(settings::profile_save(app.clone(), a.req("name")?)),
        "profile_apply" => ok(settings::profile_apply(app.clone(), a.req("name")?)),
        "profile_delete" => ok(settings::profile_delete(app.clone(), a.req("name")?)),
        "agents_list" => ok(agents::agents_list()),
        "agents_dismiss" => ok(agents::agents_dismiss(a.req("session")?)),
        "agents_hook_snippets" => ok(agents::agents_hook_snippets()),
        "agents_open_folder" => ok(agents::agents_open_folder(a.req("session")?)),
        "agents_open_code" => ok(agents::agents_open_code(a.req("session")?)),
        "agents_tools" => ok(agents::agents_tools()),
        "launcher_list" => ok(launcher::launcher_list()),
        "launcher_add" => ok(launcher::launcher_add(app.clone(), a.req("paths")?)),
        "launcher_remove" => ok(launcher::launcher_remove(app.clone(), a.req("id")?)),
        "launcher_open" => ok(launcher::launcher_open(a.req("id")?)),
        "launcher_move" => ok(launcher::launcher_move(app.clone(), a.req("id")?, a.req("new_index")?)),
        "settings_open" => ok(settingswin::settings_open(app.clone(), a.opt("page")?)),
        "settings_close" => ok(settingswin::settings_close(app.clone())),
        "settings_quit" => ok(settingswin::settings_quit(app.clone())),
        "settings_info" => ok(settingswin::settings_info(app.clone())),
        "autostart_get" => ok(settingswin::autostart_get()),
        "autostart_set" => res(settingswin::autostart_set(a.req("on")?)),
        "hotkey_check" => ok(settingswin::hotkey_check(a.req("combo")?, a.opt("action")?)),
        "hotkey_set" => ok(settingswin::hotkey_set(app.clone(), a.req("action")?, a.req("combo")?)),
        "everything_status" => ok(everything::everything_status()),
        "everything_download" => ok(everything::everything_download(app.clone())),
        "everything_run_installer" => ok(everything::everything_run_installer()),
        "capture_start" => ok(capture::capture_start(app.clone(), a.req("mode")?)),
        "capture_commit" => {
            // Tauri: raw body = the PNG of the marked-up area, the JSON arguments in header `x-pill-args` (here: `args`); otherwise the JSON body is `args`
            let ca: capture::CommitArgs = serde_json::from_value(if v.is_null() { Value::Object(Default::default()) } else { v.clone() }).map_err(|e| e.to_string())?;
            res(capture::capture_commit_impl(app, ca, body))
        }
        "capture_cancel" => ok(capture::capture_cancel(app.clone())),
        "capture_stop" => ok(capture::capture_stop(app.clone())),
        "capture_settings" => ok(capture::capture_settings()),
        "capture_session" => ok(capture::capture_session(a.req("label")?)),
        // the frame is w*h*4 raw bytes, not JSON: `ui::api::capture_frame(label)` returns it without a copy
        "capture_frame" => Err("capture_frame returns raw RGBA bytes: use ui::api::capture_frame(label)".into()),
        "capture_ready" => ok(capture::capture_ready(app.clone(), a.req("label")?)),
        "capture_action" => res(capture::capture_action(a.req("action")?, a.req("value")?)),
        "voice_toggle" => ok(voice::voice_toggle(app.clone())),
        "voice_start" => ok(voice::voice_start(app.clone())),
        "voice_stop" => ok(voice::voice_stop()),
        "voice_cancel" => ok(voice::voice_cancel()),
        "voice_state" => ok(voice::voice_state()),
        "voice_test_mic" => ok(blk(voice::voice_test_mic(app.clone()))),
        "voice_test_service" => ok(blk(voice::voice_test_service())),
        "voice_set_key" => ok(voice::voice_set_key(a.req("provider")?, a.req("key")?)),
        "voice_has_key" => ok(voice::voice_has_key(a.opt("provider")?)),
        "aisetup_status" => ok(aisetup::aisetup_status()),
        "aisetup_whisper_install" => ok(aisetup::aisetup_whisper_install(app.clone(), a.req("model")?)),
        "aisetup_whisper_set" => ok(aisetup::aisetup_whisper_set(app.clone(), a.req("on")?, a.opt("model")?)),
        "aisetup_whisper_remove" => ok(aisetup::aisetup_whisper_remove(app.clone())),
        "aisetup_ollama_download" => ok(aisetup::aisetup_ollama_download(app.clone())),
        "aisetup_ollama_run_installer" => ok(aisetup::aisetup_ollama_run_installer()),
        "aisetup_ollama_pull" => ok(aisetup::aisetup_ollama_pull(app.clone(), a.req("model")?)),
        "aisetup_ai_use" => ok(aisetup::aisetup_ai_use(app.clone(), a.req("server")?, a.req("model")?)),
        "aisetup_cancel" => ok(aisetup::aisetup_cancel()),
        "uninstall_info" => ok(uninstall::uninstall_info(a.req("id")?, a.req("name")?)),
        "uninstall_start" => ok(uninstall::uninstall_start(app.clone(), a.req("silent")?)),
        "uninstall_scan" => ok(uninstall::uninstall_scan(app.clone())),
        "uninstall_delete" => ok(uninstall::uninstall_delete(app.clone(), a.req("ids")?)),
        "uninstall_admin" => ok(uninstall::uninstall_admin(app.clone(), a.req("mode")?, a.req("ids")?)),
        "uninstall_cancel" => ok(uninstall::uninstall_cancel()),
        "uninstall_state" => ok(uninstall::uninstall_state()),
        "uninstall_open_panel" => ok(uninstall::uninstall_open_panel(app.clone())),
        "uninstall_open_backups" => ok(uninstall::uninstall_open_backups()),
        "hotkeys_reload" => res(hotkeys::hotkeys_reload(app.clone())),
        "obs_status" => ok(obs::obs_status()),
        "obs_set_password" => ok(obs::obs_set_password(app.clone(), a.req("password")?)),
        "obs_reveal" => ok(obs::obs_reveal(a.req("path")?)),
        "steam_status" => ok(steam::steam_status()),
        "notif_status" => ok(notif::notif_status()),
        "notif_request" => ok(notif::notif_request(app.clone())),
        "notif_history_add" => ok(notifhist::notif_history_add(a.req("entry")?, a.opt("read")?)),
        "notif_history_list" => ok(notifhist::notif_history_list()),
        "notif_history_clear" => ok(notifhist::notif_history_clear()),
        "notif_history_mark_read" => ok(notifhist::notif_history_mark_read(a.opt("ids")?)),
        "notif_history_remove" => ok(notifhist::notif_history_remove(a.req("id")?)),
        _ => Err(format!("command {cmd} not found")),
    }
}
