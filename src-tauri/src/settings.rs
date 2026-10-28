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
        "module.text": true, "module.chat": true, "module.note": true, "module.weather": true, "module.controls": true, "module.uninstall": true,
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

// ---------- profiles ("scenes"): named on/off states of the island modules and live events ----------
// Stored in the same settings.json under `profiles` {name: {key: bool}}. Only boolean module.* / live.* keys travel; network opt-ins
// (net.*, the mail and calendar-link feeds), secrets, paths and everything else are never read into a profile nor written by one.
const MAX_PROFILES: usize = 8;
const NAME_MAX: usize = 24;
const PROFILE_DENY: &[&str] = &["live.mail", "live.calendar"];

fn profile_key(k: &str) -> bool {
    // `module.<id>` / `live.<id>` only: sub-settings such as live.calendar.path or live.obs.port are not toggles
    let id = k.strip_prefix("module.").or_else(|| k.strip_prefix("live."));
    id.is_some_and(|id| !id.contains('.')) && !PROFILE_DENY.contains(&k) && !crate::settings_backup::is_secret(k)
}

/// The profile-able part of a config: boolean toggles only.
fn profile_snapshot(cfg: &Cfg) -> Cfg {
    cfg.iter().filter(|(k, v)| profile_key(k) && v.is_boolean()).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// What applying a stored profile writes. Re-filtered on every apply, so a hand-edited settings.json cannot smuggle other keys in.
fn profile_patch(profile: &Value) -> Cfg {
    profile.as_object().map(|o| profile_snapshot(o)).unwrap_or_default()
}

fn profile_name(raw: &str) -> Option<String> {
    let n: String = raw.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(NAME_MAX).collect();
    let n = n.trim().to_string();
    (!n.is_empty()).then_some(n)
}

/// Add or replace `name`; a ninth new profile is refused (replacing an existing one is always allowed).
fn profiles_put(profiles: &mut Cfg, name: &str, snap: Cfg) -> Result<(), &'static str> {
    if !profiles.contains_key(name) && profiles.len() >= MAX_PROFILES {
        return Err("full");
    }
    profiles.insert(name.to_string(), Value::Object(snap));
    Ok(())
}

fn stored_profiles() -> Cfg {
    match raw("profiles") {
        Some(Value::Object(o)) => o,
        _ => Cfg::new(),
    }
}

#[tauri::command]
pub fn profile_save(app: tauri::AppHandle, name: String) -> Value {
    let Some(name) = profile_name(&name) else { return json!({ "ok": false, "error": "name" }) };
    let mut all = stored_profiles();
    if let Err(e) = profiles_put(&mut all, &name, profile_snapshot(&merged())) {
        return json!({ "ok": false, "error": e });
    }
    let _ = apply_patch(&app, Cfg::from_iter([("profiles".to_string(), Value::Object(all))]));
    json!({ "ok": true })
}

#[tauri::command]
pub fn profile_apply(app: tauri::AppHandle, name: String) -> Value {
    let Some(p) = stored_profiles().get(&name).map(profile_patch) else { return json!({ "ok": false, "error": "missing" }) };
    let n = p.len();
    let _ = apply_patch(&app, p);
    json!({ "ok": true, "applied": n })
}

#[tauri::command]
pub fn profile_delete(app: tauri::AppHandle, name: String) -> Value {
    let mut all = stored_profiles();
    all.remove(&name);
    let _ = apply_patch(&app, Cfg::from_iter([("profiles".to_string(), Value::Object(all))]));
    json!({ "ok": true })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Cfg {
        let Value::Object(m) = json!({
            "module.chat": false, "module.media": true, "live.agents": false, "live.sysmon": true,
            "live.mail": true, "live.calendar": true, "live.calendar.path": "C:/x.ics", "live.obs.port": 4455,
            "net.ai": true, "net.weather": true, "ai.apiKeyEnc": "blob", "voice.key.groq": true, "module.tokenX": true,
            "hotkey.toggle": "Ctrl+Alt+I", "size": 100, "theme": "dark", "capture.folder": "C:/v", "history.mail": true
        }) else { unreachable!() };
        m
    }

    #[test]
    fn profile_snapshot_takes_only_plain_module_and_live_toggles() {
        let mut k: Vec<_> = profile_snapshot(&cfg()).keys().cloned().collect();
        k.sort();
        assert_eq!(k, ["live.agents", "live.sysmon", "module.chat", "module.media"]);
    }

    #[test]
    fn applying_a_profile_never_touches_excluded_keys() {
        // a hand-edited / tampered profile: only the allowed boolean keys survive
        let evil = json!({ "module.chat": true, "net.ai": false, "live.mail": false, "live.calendar": false, "hotkey.toggle": "X", "size": 1,
            "ai.apiKeyEnc": true, "live.calendar.path": true, "module.media": "no", "live.agents": true });
        let mut k: Vec<_> = profile_patch(&evil).keys().cloned().collect();
        k.sort();
        assert_eq!(k, ["live.agents", "module.chat"]);
        assert!(profile_patch(&json!("not an object")).is_empty());
    }

    #[test]
    fn save_then_apply_round_trips_the_toggles() {
        let snap = profile_snapshot(&cfg());
        let mut now = cfg();
        now.insert("module.chat".into(), json!(true));
        now.insert("live.agents".into(), json!(true));
        for (k, v) in profile_patch(&Value::Object(snap)) {
            now.insert(k, v);
        }
        assert_eq!((&now["module.chat"], &now["live.agents"]), (&json!(false), &json!(false)));
        assert_eq!((&now["net.ai"], &now["live.mail"], &now["size"]), (&json!(true), &json!(true), &json!(100)));
    }

    #[test]
    fn profile_count_is_capped_but_replacing_is_allowed() {
        let mut p = Cfg::new();
        for i in 0..MAX_PROFILES {
            assert!(profiles_put(&mut p, &format!("p{i}"), Cfg::new()).is_ok());
        }
        assert_eq!(profiles_put(&mut p, "extra", Cfg::new()), Err("full"));
        assert!(profiles_put(&mut p, "p3", profile_snapshot(&cfg())).is_ok());
        assert_eq!(p.len(), MAX_PROFILES);
    }

    #[test]
    fn profile_names_are_cleaned() {
        assert_eq!(profile_name("  Work
 "), Some("Work".into()));
        assert_eq!(profile_name("   "), None);
        assert_eq!(profile_name(&"x".repeat(60)).unwrap().chars().count(), NAME_MAX);
    }

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
