#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// One module per feature; each owner edits only their own file (plus their marked lines below).
// init() is called once from setup().
mod clipboard;
mod command;
mod fspeek;
mod guard;
mod keepalive;
mod media;
mod lyrics;
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
mod alert;
mod dlguard;
mod watch;
mod audio;
mod player_audio;
mod proc_loopback;
mod extplayer;
mod agents;
mod agent_bridge;
mod launcher;
mod settingswin;
mod capture;
mod recaudio;
mod captranslate;
mod voice;
mod aisetup;
mod everything;
mod hotkeys;
mod islandkeys;
mod vdesk;
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
mod whatsnew;
mod watchdog;
mod report;
mod openarg;
mod uninstall;
mod ui;
mod applog;
mod vpn;
mod wifi;

fn main() {
    watchdog::wait_for_predecessor(); // restarted by the watchdog: the hung instance must be gone before the single-instance check
    applog::init(); // stderr -> %LOCALAPPDATA%\Pill\logs\pill.log (the release build has no console)
    // `pill.exe --uninstall-admin <port>:<token>`: the elevated helper of the uninstaller (uninstall/admin.rs); nothing else starts in that process
    if let Some(code) = uninstall::admin_main() {
        std::process::exit(code);
    }
    uninstall::pin_exe(); // keep pill.exe open (read sharing only) for the whole run: the admin step verifies exactly this file
    tauri::Builder::default()
        // @plugins-begin (add .plugin(...) lines here, one per line)
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| openarg::handle(app, args))) // first: a second launch is forwarded here and exits
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // @plugins-end
        .setup(|app| {
            // The native UI host + the island window first, before every init() below (pill::init needs the island).
            ui::island::start(app.handle());
            store::init(app);
            settings::init(app);
            pill::init(app);
            guard::init(app);
            watchdog::init();
            media::init(app);
            audio::init(app);
            clipboard::init(app);
            updater::init(app);
            shelf::init(app);
            command::init(app);
            system::init(app);
            vpn::init(app);
            net::init(app);
            chat::init(app);
            notes::init(app);
            timers::init(app);
            watch::init(app);
            vdesk::init(app);
            livesys::init(app);
            agents::init(app);
            agent_bridge::init();
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
        .build(tauri::generate_context!())
        .expect("error while building pill")
        .run(|_app, event| {
            // The app ends only through an explicit exit (tray Quit, Ctrl+Alt+Q, settings Quit, updater restart: code is Some). Closing the last
            // window (code None) must not end it: windows come and go (the bar is torn down while idle) and the island lives in the tray.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}
