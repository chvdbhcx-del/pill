// User settings: flat JSON map in %APPDATA%\pill\settings.json, cached in memory, atomic writes.
// Owner: pill agent (lane L1). Other modules read their own keys through get_str/get_bool/get_f64.
// Keys: size (80-200), theme (dark|light|auto), clock24, autoHide, monitor ("primary"|"1".."n"), reduceMotion,
// showInCapture, hideInApps (list of exe names), hotkey.<action> (live: hotkeys.rs), module.<id>, live.<id>, capture.*.
use serde_json::{json, Map, Value};
use std::sync::{Mutex, OnceLock};

type Cfg = Map<String, Value>;
static CACHE: OnceLock<Mutex<Cfg>> = OnceLock::new();

fn path() -> std::path::PathBuf {
    crate::store::dir().join("settings.json")
}

pub(crate) fn defaults() -> Cfg {
    let Value::Object(mut m) = json!({
        "size": 100, "theme": "dark", "clock24": true, "autoHide": false, "monitor": "primary",
        "reduceMotion": false, "showInCapture": true, "hideInApps": [],
        "hotkey.toggle": "Ctrl+Alt+I", "hotkey.command": "Alt+Shift+Space", "hotkey.clipboard": "Ctrl+Alt+V",
        // settings window (settingswin.rs / settings.js)
        "language": "auto", "shape": "notch", "character": "liquid", "pet": "cat", "pet.side": "right", "pet.size": "m", "hideInFullscreen": true, "autoHideKeepDesktop": false, "updateAuto": true,
        "underTaskbar": true, "passThrough": false, "passThroughOpacity": 50, "offsetX": 0, "musicPill": "title",
        "audio.reactive": true, "sound.reminders": true, "sound.reminderTone": "chime"
    }) else { unreachable!() };
    // second literal: one json! macro would hit the recursion limit
    let Value::Object(more) = json!({
        // Settings > Modules (module.<id>) and > Live events (live.<id>): everything on except OBS, which needs its own WebSocket server
        "module.media": true, "module.shelf": true, "module.launcher": true, "module.fileSearch": true, "module.clipboard": true,
        "module.text": true, "module.chat": true, "module.note": true, "module.weather": true, "module.controls": true,
        "live.volume": true, "live.music": true, "live.micCam": true, "live.downloads": true, "live.recording": true, "live.notifications": true,
        "live.reminders": true, "live.keyboardLayout": true, "live.copy": true, "live.obs": false, "live.agents": true, "live.steam": true, "live.sysmon": true,
        "live.dlDone": true, "live.shelf": true, "live.usb": true, "live.battery": true, "live.calendar": false, "live.calendar.path": "",
        // screen capture (capture.rs)
        "capture.format": "mp4", "capture.fps": 30, "capture.encoder": "auto", "capture.shotAction": "both", "capture.revealAfterSave": false,
        // first-run AI setup wizard (aisetup.rs)
        "aiSetupDone": false, "aisetup.whisper.enabled": false, "aisetup.whisper.model": "base", "aisetup.whisper.port": 8179
    }) else { unreachable!() };
    m.extend(more);
    // third literal (recursion limit again): mail notifications (mail.rs), opt-in; the password lives DPAPI-encrypted in mail.passwordEnc, never plain
    let Value::Object(mail) = json!({ "live.mail": false, "mail.host": "", "mail.port": 993, "mail.user": "", "mail.mailbox": "INBOX" }) else { unreachable!() };
    m.extend(mail);
    // notification centre (notifhist.rs, tab-notifs.js): history.keep = store what the island showed; history.mail = also keep mail / Windows-notification bodies (off: title only)
    let Value::Object(hist) = json!({ "module.notifs": true, "history.keep": true, "history.mail": false }) else { unreachable!() };
    m.extend(hist);
    m
}

fn load() -> Cfg {
    let p = path();
    let Ok(text) = std::fs::read_to_string(&p) else { return Cfg::new() }; // missing file = first run
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => m,
        _ => {
            // Never overwrite a corrupt file: move it aside and start from defaults.
            eprintln!("settings: {} is corrupt, moved to settings.corrupt.json", p.display());
            let _ = std::fs::rename(&p, p.with_file_name("settings.corrupt.json"));
            Cfg::new()
        }
    }
}

fn cache() -> &'static Mutex<Cfg> {
    CACHE.get_or_init(|| Mutex::new(load()))
}

fn lock() -> std::sync::MutexGuard<'static, Cfg> {
    cache().lock().unwrap_or_else(|e| e.into_inner())
}

fn save(m: &Cfg) {
    let p = path();
    let tmp = p.with_extension("json.tmp");
    let ok = serde_json::to_vec_pretty(m).ok().is_some_and(|b| std::fs::write(&tmp, b).is_ok());
    if !ok || std::fs::rename(&tmp, &p).is_err() {
        eprintln!("settings: could not write {}", p.display());
    }
}

pub fn init(_app: &tauri::App) {
    let _ = cache();
}

fn raw(key: &str) -> Option<Value> {
    lock().get(key).cloned()
}

pub fn get_str(key: &str, default: &str) -> String {
    match raw(key) {
        Some(Value::String(s)) => s,
        Some(Value::Number(n)) => n.to_string(),
        _ => default.to_string(),
    }
}

pub fn get_bool(key: &str, default: bool) -> bool {
    raw(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

pub fn get_f64(key: &str, default: f64) -> f64 {
    match raw(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(default),
        _ => default,
    }
}

/// A list of strings (hideInApps); anything else reads as empty.
pub fn get_list(key: &str) -> Vec<String> {
    match raw(key) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    }
}

#[allow(dead_code)] // public API for other modules
pub fn set(key: &str, value: Value) {
    let mut m = lock();
    m.insert(key.to_string(), value);
    save(&m);
}

pub(crate) fn merged() -> Cfg {
    let mut m = defaults();
    m.extend(lock().clone());
    m
}

#[tauri::command]
pub fn settings_get() -> Cfg {
    merged()
}

#[tauri::command]
pub fn settings_set(app: tauri::AppHandle, patch: Cfg) -> Cfg {
    let (all, res) = apply_patch(&app, patch);
    if let Err(e) = res {
        eprintln!("settings: hotkey change refused and rolled back: {e}");
    }
    all
}

/// Save a patch, run the Rust-side reactions and broadcast `settings:changed`. When a `hotkey.*` key cannot be registered
/// (taken by another program, invalid) every `hotkey.*` key of the patch goes back to its old value, the previous bindings stay
/// registered and the second value is Err(text). `module.*` keys re-sync the hotkeys of that module (never fails the patch).
pub fn apply_patch(app: &tauri::AppHandle, mut patch: Cfg) -> (Cfg, Result<(), String>) {
    use tauri::Emitter;
    if let Some(v) = patch.get("size").and_then(|v| v.as_f64()) {
        patch.insert("size".into(), json!(v.clamp(80.0, 200.0).round()));
    }
    let old_keys: Vec<(String, Option<Value>)> = patch.keys().filter(|k| k.starts_with("hotkey.")).map(|k| (k.clone(), raw(k))).collect();
    {
        let mut m = lock();
        for (k, v) in &patch {
            m.insert(k.clone(), v.clone());
        }
        save(&m);
    }
    let mut res = Ok(());
    if !old_keys.is_empty() || patch.keys().any(|k| k.starts_with("module.")) {
        let strict: Vec<String> = old_keys.iter().filter_map(|(k, _)| k.strip_prefix("hotkey.").map(str::to_string)).collect();
        if let Err(e) = crate::hotkeys::reload(app, &strict) {
            if !old_keys.is_empty() {
                {
                    let mut m = lock();
                    for (k, v) in old_keys {
                        match v {
                            Some(v) => m.insert(k, v),
                            None => m.remove(&k),
                        };
                    }
                    save(&m);
                }
                let _ = crate::hotkeys::reload(app, &[]);
                res = Err(e);
            }
        }
    }
    crate::pill::on_settings(app, &patch);
    crate::media::on_settings(&patch);
    let all = merged();
    let _ = app.emit("settings:changed", &all);
    (all, res)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pet_defaults() {
        let d = super::defaults();
        assert_eq!(d["character"], "liquid");
        assert_eq!(d["pet"], "cat");
        assert_eq!(d["pet.side"], "right");
        assert_eq!(d["pet.size"], "m");
        assert_eq!((&d["history.keep"], &d["history.mail"], &d["module.notifs"]), (&serde_json::json!(true), &serde_json::json!(false), &serde_json::json!(true)));
    }
}
