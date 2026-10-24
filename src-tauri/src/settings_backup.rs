// Settings backup: export to Downloads (no secrets) and import through the normal settings path.
// Commands: settings_export() -> { ok, path, error? }, settings_import(json_text) -> { ok, applied, skipped, error? }.
// Import only accepts keys of the defaults table, a few known exact keys and the dynamic prefixes below; secrets
// (DPAPI blobs, tokens, passwords) and machine/runtime state are never exported and never imported.
use crate::settings;
use serde_json::{json, Map, Value};
use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

type Cfg = Map<String, Value>;

const MAX_IMPORT: usize = 1 << 20;
/// Dynamic key families (hotkey.<action>, module.<id>, live.<id>, ...) that settings.rs does not list in its defaults.
const PREFIXES: &[&str] = &[
    "hotkey.", "module.", "live.", "capture.", "voice.", "ai.", "weather.", "net.", "obs.", "sound.", "pet.", "audio.", "clipboard.", "everything.",
];
/// Exact keys written by the settings page that have no entry in the Rust defaults table.
const EXTRA: &[&str] = &["showTime", "showDate", "showTemp", "showSeconds", "showBattery", "opacityCol", "opacityExp", "fps", "edgeIdleFps"];

/// Secret-like keys: DPAPI blobs (`...Enc`, `voice.key.<provider>`), tokens, passwords, API keys.
pub fn is_secret(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    key.ends_with("Enc") || k.contains(".key.") || k.contains("token") || k.contains("secret") || k.contains("password") || k.contains("apikey")
}

/// Machine or runtime state that must not travel between PCs.
fn is_runtime(key: &str) -> bool {
    key == "aiSetupDone" || key.starts_with("aisetup.") || key.starts_with("settingsWin.")
}

fn exportable(key: &str) -> bool {
    !key.starts_with('_') && !is_secret(key) && !is_runtime(key)
}

/// `C:\...`, `C:/...` or a UNC path: it names a place on this PC (capture folder, ffmpeg, ...), so it neither travels nor is accepted.
fn looks_like_path(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || s.starts_with("\\\\")
}

fn plain_value(v: &Value) -> bool {
    match v {
        Value::Bool(_) | Value::Number(_) => true,
        Value::String(s) => s.len() <= 4096 && !looks_like_path(s),
        Value::Array(a) => a.len() <= 256 && a.iter().all(|x| x.as_str().is_some_and(|s| s.len() <= 4096 && !looks_like_path(s))),
        _ => false,
    }
}

fn same_kind(a: &Value, b: &Value) -> bool {
    matches!((a, b), (Value::Bool(_), Value::Bool(_)) | (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) | (Value::Array(_), Value::Array(_)))
}

/// Pure filter: returns the patch to apply and the number of skipped entries. The `_pill` header is ignored (not counted).
/// A key with a default must keep the default's JSON type; other accepted keys must be a bool, number, string or list of strings.
pub fn filter_import(input: &Value, defs: &Cfg) -> (Cfg, usize) {
    let mut patch = Cfg::new();
    let mut skipped = 0;
    let Some(obj) = input.as_object() else { return (patch, 0) };
    for (k, v) in obj {
        if k == "_pill" {
            continue;
        }
        let known = defs.contains_key(k) || EXTRA.contains(&k.as_str()) || PREFIXES.iter().any(|p| k.starts_with(p));
        let typed = defs.get(k).is_none_or(|d| same_kind(d, v));
        if !exportable(k) || !known || !typed || !plain_value(v) {
            skipped += 1;
            continue;
        }
        patch.insert(k.clone(), v.clone());
    }
    (patch, skipped)
}

fn downloads_dir() -> PathBuf {
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, HANDLE::default()) {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            if let Some(s) = s.filter(|s| !s.is_empty()) {
                return PathBuf::from(s);
            }
        }
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Downloads")
}

#[tauri::command]
pub fn settings_export() -> Value {
    let t = unsafe { GetLocalTime() };
    let u = unsafe { GetSystemTime() };
    let name = format!("pill-settings-{:04}{:02}{:02}-{:02}{:02}{:02}.json", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    let iso = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", u.wYear, u.wMonth, u.wDay, u.wHour, u.wMinute, u.wSecond);
    let mut out = Cfg::new();
    out.insert("_pill".into(), json!({ "version": env!("CARGO_PKG_VERSION"), "exported": iso }));
    for (k, v) in settings::merged() {
        if exportable(&k) && plain_value(&v) {
            out.insert(k, v);
        }
    }
    let dir = downloads_dir();
    let path = dir.join(name);
    let written = serde_json::to_vec_pretty(&out).map_err(|e| e.to_string()).and_then(|b| {
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(&path, b).map_err(|e| e.to_string())
    });
    if let Err(e) = written {
        return json!({ "ok": false, "error": e });
    }
    // one argv element for explorer.exe (no cmd.exe, no shell string); CREATE_NO_WINDOW
    let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())).creation_flags(0x0800_0000).spawn();
    json!({ "ok": true, "path": path.to_string_lossy() })
}

#[tauri::command]
pub fn settings_import(app: tauri::AppHandle, json_text: String) -> Value {
    if json_text.len() > MAX_IMPORT {
        return json!({ "ok": false, "applied": 0, "skipped": 0, "error": "file is larger than 1 MB" });
    }
    let parsed = match serde_json::from_str::<Value>(json_text.trim_start_matches('\u{feff}')) {
        Ok(v @ Value::Object(_)) => v,
        Ok(_) => return json!({ "ok": false, "applied": 0, "skipped": 0, "error": "not a settings file (JSON object expected)" }),
        Err(e) => return json!({ "ok": false, "applied": 0, "skipped": 0, "error": format!("invalid JSON: {e}") }),
    };
    let (patch, skipped) = filter_import(&parsed, &settings::defaults());
    let mut applied = patch.len();
    if applied == 0 {
        return json!({ "ok": false, "applied": 0, "skipped": skipped, "error": "no known settings in the file" });
    }
    let hotkeys = patch.keys().filter(|k| k.starts_with("hotkey.")).count();
    let (_, res) = settings::apply_patch(&app, patch);
    match res {
        Ok(()) => json!({ "ok": true, "applied": applied, "skipped": skipped }),
        Err(e) => {
            // apply_patch rolled every hotkey.* key of the patch back
            applied -= hotkeys;
            json!({ "ok": true, "applied": applied, "skipped": skipped + hotkeys, "error": e })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(v: Value) -> (Cfg, usize) {
        filter_import(&v, &settings::defaults())
    }

    #[test]
    fn secrets_are_detected() {
        for k in ["ai.apiKeyEnc", "obs.passwordEnc", "voice.key.groq", "net.token", "x.secret"] {
            assert!(is_secret(k), "{k}");
        }
        for k in ["hotkey.voice", "hotkey.toggle", "size", "voice.model"] {
            assert!(!is_secret(k), "{k}");
        }
    }

    #[test]
    fn filter_keeps_known_typed_keys_only() {
        let (p, skipped) = run(json!({
            "_pill": { "version": "0.1.0" },
            "size": 120, "theme": "light", "clock24": false, "hideInApps": ["a.exe"],
            "hotkey.launcher.1": "Ctrl+Alt+1", "module.media": false, "everything.autostart": false, "showTime": true,
            "clock24x": true,                  // unknown
            "size_": 1,                         // unknown
            "autoHide": "yes",                  // wrong type (bool expected)
            "theme2": "x",                      // unknown
            "ai.apiKeyEnc": "blob",             // secret
            "voice.key.openai": "blob",         // secret
            "settingsWin.max": true,            // runtime
            "aiSetupDone": true,                // runtime
            "weather.city": { "a": 1 },         // nested object
            "hideInFullscreen": [1, 2]          // wrong type
        }));
        let mut keys: Vec<&str> = p.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["clock24", "everything.autostart", "hideInApps", "hotkey.launcher.1", "module.media", "showTime", "size", "theme"]);
        assert_eq!(skipped, 10);
    }

    #[test]
    fn non_object_and_header_only() {
        assert_eq!(run(json!([1, 2])).0.len(), 0);
        let (p, s) = run(json!({ "_pill": { "version": "x" } }));
        assert!(p.is_empty() && s == 0);
    }

    #[test]
    fn export_filter() {
        assert!(exportable("theme") && exportable("hotkey.toggle"));
        assert!(!exportable("ai.apiKeyEnc") && !exportable("_pill") && !exportable("settingsWin.max"));
    }

    #[test]
    fn machine_paths_do_not_travel() {
        for s in ["C:\\Users\\me\\Videos", "d:/clips", "\\\\nas\\share"] {
            assert!(!plain_value(&json!(s)), "{s}");
        }
        assert!(!plain_value(&json!(["chrome.exe", "C:\\tools\\x.exe"])));
        assert!(plain_value(&json!("mp4")) && plain_value(&json!(["chrome.exe", "obs64"])) && plain_value(&json!(true)));
        let (patch, skipped) = run(json!({ "capture.folder": "C:\\Videos", "capture.fps": 30 }));
        assert_eq!((patch.len(), skipped), (1, 1));
    }
}
