#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// One module per feature; each owner edits only their own file (plus their marked lines below).
// init() is called once from setup().
mod clipboard;
mod command;
mod guard;
mod media;
mod pill;
mod settings;
mod settings_backup;
mod shelf;
mod store;
mod system;
mod net;
mod chat;
mod notes;
mod timers;
mod watch;
mod audio;
mod agents;
mod launcher;
mod settingswin;
mod capture;
mod voice;
mod aisetup;
mod everything;
mod hotkeys;
mod obs;
mod steam;
mod wake;
mod notif;
mod livesys;
mod sysmon;
mod updater;
mod aiactions;
mod liveplus;
mod mail;
mod notifhist;

fn main() {
    tauri::Builder::default()
        // @plugins-begin (add .plugin(...) lines here, one per line)
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // @plugins-end
        .invoke_handler(tauri::generate_handler![
            pill::pill_rect, pill::pill_set_state, pill::pill_toggle, pill::pill_activate, pill::pill_auto_hide,
            pill::pill_monitors, pill::pill_set_gaze, pill::pill_cards, guard::pill_desktop_fg,
            updater::update_check, updater::update_install,
            // @pill-commands
            // @guard-commands
            media::media_state, media::media_sessions, media::media_select, media::media_control,
            media::media_seek, media::media_volume_get, media::media_volume_set, media::media_mute,
            media::media_pause_updates, media::media_resume_updates, media::battery_state,
            media::media_devices, media::media_set_default_device, media::media_capture_devices,
            // @media-commands
            audio::audio_status,
            // @audio-commands
            clipboard::clip_list, clipboard::clip_image, clipboard::clip_paste, clipboard::clip_pin, clipboard::clip_delete,
            clipboard::clip_wipe, clipboard::clip_pause, clipboard::clip_status,
            clipboard::clip_paste_text,
            // @clipboard-commands
            shelf::shelf_add, shelf::shelf_list, shelf::shelf_remove, shelf::shelf_clear,
            shelf::shelf_open, shelf::shelf_reveal, shelf::shelf_copy_path, shelf::shelf_drag_out,
            // @shelf-commands
            command::cmd_toggle, command::cmd_query, command::cmd_run, command::cmd_icon,
            // @command-commands
            settings::settings_get, settings::settings_set,
            settings_backup::settings_export, settings_backup::settings_import,
            // @settings-commands
            system::sys_state, system::sys_set, system::sys_power,
            // @system-commands
            net::weather_get, net::weather_search_city, net::weather_set_city, net::net_features_status, net::weather_current,
            // @net-commands
            chat::chat_send, chat::chat_retry, chat::chat_stop, chat::chat_threads, chat::chat_messages, chat::chat_new,
            chat::chat_delete, chat::chat_open_link, chat::ai_test, chat::ai_models, chat::ai_probe, chat::ai_set_key, chat::ai_config,
            aiactions::ai_transform, aiactions::ai_transform_cancel, aiactions::ai_clip_text, aiactions::cmd_ask,
            // @chat-commands
            notes::note_list, notes::note_get, notes::note_save, notes::note_delete,
            // @notes-commands
            timers::remind_add, timers::remind_list, timers::remind_done, timers::remind_delete, timers::remind_snooze,
            timers::timer_start, timers::timer_cancel, timers::timer_state, timers::sound_play,
            // @timers-commands
            watch::kbd_state, watch::watch_pause, watch::watch_resume,
            livesys::livesys_visible, livesys::livesys_privacy, livesys::livesys_reveal,
            sysmon::sysmon_get,
            liveplus::liveplus_ics_status,
            mail::mail_status, mail::mail_set_password, mail::mail_test,
            // @watch-commands
            settings::profile_save, settings::profile_apply, settings::profile_delete,
            agents::agents_list, agents::agents_dismiss, agents::agents_hook_snippets,
            // @agents-commands
            launcher::launcher_list, launcher::launcher_add, launcher::launcher_remove, launcher::launcher_open, launcher::launcher_move,
            // @launcher-commands
            settingswin::settings_open, settingswin::settings_close, settingswin::settings_quit, settingswin::settings_info,
            settingswin::autostart_get, settingswin::autostart_set, settingswin::hotkey_check, settingswin::hotkey_set,
            everything::everything_status, everything::everything_download, everything::everything_run_installer,
            // @settingswin-commands
            capture::capture_start, capture::capture_commit, capture::capture_cancel, capture::capture_stop, capture::capture_settings,
            capture::capture_session, capture::capture_frame, capture::capture_ready, capture::capture_action,
            // @capture-commands
            voice::voice_toggle, voice::voice_start, voice::voice_stop, voice::voice_cancel, voice::voice_state,
            voice::voice_test_mic, voice::voice_test_service, voice::voice_set_key, voice::voice_has_key,
            // @voice-commands
            aisetup::aisetup_status, aisetup::aisetup_whisper_install, aisetup::aisetup_whisper_set, aisetup::aisetup_whisper_remove,
            aisetup::aisetup_ollama_download, aisetup::aisetup_ollama_run_installer, aisetup::aisetup_ollama_pull, aisetup::aisetup_ai_use,
            aisetup::aisetup_cancel,
            hotkeys::hotkeys_reload,
            obs::obs_status, obs::obs_set_password, obs::obs_reveal, steam::steam_status, notif::notif_status, notif::notif_request,
            notifhist::notif_history_add, notifhist::notif_history_list, notifhist::notif_history_clear, notifhist::notif_history_mark_read, notifhist::notif_history_remove,
        ])
        .setup(|app| {
            store::init(app);
            settings::init(app);
            pill::init(app);
            guard::init(app);
            media::init(app);
            audio::init(app);
            clipboard::init(app);
            updater::init(app);
            shelf::init(app);
            command::init(app);
            system::init(app);
            net::init(app);
            chat::init(app);
            notes::init(app);
            timers::init(app);
            watch::init(app);
            livesys::init(app);
            agents::init(app);
            launcher::init(app);
            settingswin::init(app);
            capture::init(app);
            voice::init(app);
            aisetup::init(app);
            obs::init(app);
            steam::init(app);
            notif::init(app);
            sysmon::init(app);
            liveplus::init(app);
            mail::init(app);
            notifhist::init(app);
            hotkeys::init(app); // last: every module's init() has run, so the actions it triggers exist
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running pill");
}
