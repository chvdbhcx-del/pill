// Settings window opener (the window itself is ui/settings_view), autostart (HKCU Run value "Pill", only on an explicit toggle),
// hotkey checks/storage and the few helpers the pages need. Owner: settings-window agent.
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::AppHandle;
use windows::core::w;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT};

const PAGES: &[&str] = &["character", "general", "appearance", "behavior", "search", "hotkeys", "ai", "voice", "weather", "sound", "live", "clipboard", "modules", "updates", "aisetup", "welcome"];

pub fn init(_app: &tauri::App) {
    // Nothing runs at startup: the window is created on demand (tray / pill / command), the autostart value is never touched here.
}

/// Open the settings window on `page` (one of PAGES), or focus it when it already exists. Never blocks the caller.
pub fn open(app: &AppHandle, page: Option<&str>) {
    let page = page.map(str::to_ascii_lowercase).filter(|p| PAGES.contains(&p.as_str()));
    crate::ui::settings_view::open_native(app, page.as_deref());
}

// ---------------------------------------------------------------------------------------------------------------
// Commands for the window itself
// ---------------------------------------------------------------------------------------------------------------

/// Open or focus the settings window, optionally on a page (character, general, hotkeys, ai, voice, weather, sound, live, clipboard, modules, updates, aisetup).
pub fn settings_open(app: AppHandle, page: Option<String>) {
    open(&app, page.as_deref());
}

pub fn settings_close(_app: AppHandle) {
    crate::ui::settings_view::close_native();
}

pub fn settings_quit(app: AppHandle) {
    app.exit(0);
}

/// Version and data folder for the Updates page.
pub fn settings_info(app: AppHandle) -> Value {
    json!({ "version": app.package_info().version.to_string(), "dir": crate::store::dir().to_string_lossy() })
}

// ---------------------------------------------------------------------------------------------------------------
// Autostart: HKCU\Software\Microsoft\Windows\CurrentVersion\Run, value "Pill". Only touched by an explicit toggle.
// ---------------------------------------------------------------------------------------------------------------

/// The command line written to the Run value: the quoted path of this executable.
pub fn run_value(exe: &str) -> String {
    format!("\"{}\"", exe.trim_matches('"'))
}

pub fn autostart_get() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), w!("Pill"), RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

/// Add (on) or remove (off) the Run value; returns the state read back from the registry.
pub fn autostart_set(on: bool) -> Result<bool, String> {
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    unsafe {
        if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let v: Vec<u16> = run_value(&exe.to_string_lossy()).encode_utf16().chain(Some(0)).collect();
            let r = RegSetKeyValueW(HKEY_CURRENT_USER, key, w!("Pill"), REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32);
            if r != ERROR_SUCCESS {
                return Err(format!("registry error {}", r.0));
            }
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, w!("Pill"));
            if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
                return Err(format!("registry error {}", r.0));
            }
        }
    }
    Ok(autostart_get())
}

// ---------------------------------------------------------------------------------------------------------------
// Hotkeys
// ---------------------------------------------------------------------------------------------------------------

/// (action id, default combo). Stored as settings key `hotkey.<id>`; "" = not assigned. Launcher shortcuts are `launcher.<id>`.
pub const ACTIONS: &[(&str, &str)] = &[
    ("toggle", "Ctrl+Alt+I"),
    ("command", "Alt+Shift+Space"),
    ("chat", "Ctrl+Alt+A"),
    ("voice", "Ctrl+Alt+D"),
    ("record", "Ctrl+Alt+R"),
    ("shotArea", "Ctrl+Alt+S"),
    ("shotFull", "Ctrl+Alt+Shift+S"),
    ("ocr", "Ctrl+Alt+O"),
    ("clipboard", "Ctrl+Alt+V"),
    ("pastePlain", "Ctrl+Alt+Shift+V"),
    ("mic", "Ctrl+Alt+M"),
    ("pipette", "Ctrl+Alt+C"),
    ("qr", "Ctrl+Alt+Shift+Q"), // Ctrl+Alt+Q is the always-on quit hotkey of pill.rs
    ("passThrough", ""),
    ("vpn", ""),
];
/// Combos owned by the app itself (pill.rs registers Ctrl+Alt+Q = quit with a raw RegisterHotKey).
const RESERVED: &[(&str, &str)] = &[("Ctrl+Alt+Q", "quit")];

const MOD_ALT: u32 = 1;
const MOD_CTRL: u32 = 2;
const MOD_SHIFT: u32 = 4;
const MOD_WIN: u32 = 8;

fn key_vk(k: &str) -> Option<(String, u32)> {
    let up = k.to_ascii_uppercase();
    let b = up.as_bytes();
    if b.len() == 1 && b[0].is_ascii_alphanumeric() {
        return Some((up.clone(), b[0] as u32));
    }
    if let Some(n) = up.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some((format!("F{n}"), 0x70 + n - 1));
    }
    let (name, vk) = match up.as_str() {
        "SPACE" => ("Space", 0x20),
        "ENTER" | "RETURN" => ("Enter", 0x0D),
        "TAB" => ("Tab", 0x09),
        "UP" | "ARROWUP" => ("Up", 0x26),
        "DOWN" | "ARROWDOWN" => ("Down", 0x28),
        "LEFT" | "ARROWLEFT" => ("Left", 0x25),
        "RIGHT" | "ARROWRIGHT" => ("Right", 0x27),
        "HOME" => ("Home", 0x24),
        "END" => ("End", 0x23),
        "PAGEUP" => ("PageUp", 0x21),
        "PAGEDOWN" => ("PageDown", 0x22),
        "INSERT" => ("Insert", 0x2D),
        "DELETE" | "DEL" => ("Delete", 0x2E),
        _ => return None,
    };
    Some((name.to_string(), vk))
}

/// "alt+shift+space" -> Some(("Alt+Shift+Space", mods, vk)). Needs exactly one key and a modifier (a bare F1-F24 is allowed).
pub fn parse_combo(spec: &str) -> Option<(String, u32, u32)> {
    let (mut mods, mut key) = (0u32, None);
    for part in spec.split('+').map(str::trim) {
        match part.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CTRL,
            "ALT" => mods |= MOD_ALT,
            "SHIFT" => mods |= MOD_SHIFT,
            "WIN" | "SUPER" | "META" => mods |= MOD_WIN,
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(key_vk(part)?);
            }
        }
    }
    let (name, vk) = key?;
    let fkey = (0x70..=0x87).contains(&vk);
    if mods == 0 && !fkey {
        return None;
    }
    let mut s = String::new();
    for (m, n) in [(MOD_CTRL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")] {
        if mods & m != 0 {
            s.push_str(n);
            s.push('+');
        }
    }
    s.push_str(&name);
    Some((s, mods, vk))
}

/// Current combo of every known action (defaults overlaid by settings.json), plus launcher.* shortcuts.
fn bound() -> Vec<(String, String)> {
    let cfg = crate::settings::settings_get();
    let mut out: Vec<(String, String)> = ACTIONS
        .iter()
        .map(|(a, d)| (a.to_string(), cfg.get(&format!("hotkey.{a}")).and_then(Value::as_str).unwrap_or(d).to_string()))
        .collect();
    for (k, v) in &cfg {
        if let (Some(a), Some(s)) = (k.strip_prefix("hotkey.launcher."), v.as_str()) {
            out.push((format!("launcher.{a}"), s.to_string()));
        }
    }
    out
}

/// Pure part of the conflict check: who already holds `norm` (an action other than `action`, or a reserved combo).
pub fn conflict(norm: &str, action: &str, bound: &[(String, String)]) -> Option<String> {
    if let Some((_, by)) = RESERVED.iter().find(|(c, _)| c.eq_ignore_ascii_case(norm)) {
        return Some(by.to_string());
    }
    bound
        .iter()
        .find(|(a, c)| a != action && !c.is_empty() && c.eq_ignore_ascii_case(norm))
        .map(|(a, _)| a.clone())
}

/// Try the combo against the OS: RegisterHotKey + immediate UnregisterHotKey. False = another program owns it.
fn os_free(mods: u32, vk: u32) -> bool {
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        if RegisterHotKey(None, 0x7C5E, HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT, vk).is_ok() {
            let _ = UnregisterHotKey(None, 0x7C5E);
            true
        } else {
            false
        }
    }
}

fn check(combo: &str, action: &str) -> Value {
    let Some((norm, mods, vk)) = parse_combo(combo) else { return json!({ "ok": false, "reason": "invalid" }) };
    let b = bound();
    if let Some(by) = conflict(&norm, action, &b) {
        return json!({ "ok": false, "reason": if by == "quit" { "reserved" } else { "used" }, "by": by, "combo": norm });
    }
    // The app itself holds its current combo, so the OS would refuse it: only unchanged combos skip the OS test.
    if b.iter().any(|(a, c)| a == action && c.eq_ignore_ascii_case(&norm)) {
        return json!({ "ok": true, "combo": norm });
    }
    if os_free(mods, vk) {
        json!({ "ok": true, "combo": norm })
    } else {
        json!({ "ok": false, "reason": "taken", "combo": norm })
    }
}

/// {ok:true, combo} when the combo can be bound to `action`; {ok:false, reason: invalid|reserved|used|taken, by?}.
pub fn hotkey_check(combo: String, action: Option<String>) -> Value {
    check(&combo, action.as_deref().unwrap_or(""))
}

/// Store `hotkey.<action>` (empty combo = not assigned) after the same check and re-register the hotkey at once (hotkeys.rs);
/// on a refusal the previous binding keeps working and the answer is {ok:false, reason:"taken"}.
pub fn hotkey_set(app: AppHandle, action: String, combo: String) -> Value {
    let known = ACTIONS.iter().any(|(a, _)| *a == action) || (action.starts_with("launcher.") && action.len() < 60 && action.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'));
    if !known {
        return json!({ "ok": false, "reason": "action" });
    }
    let value = if combo.trim().is_empty() {
        String::new()
    } else {
        let r = check(&combo, &action);
        if r["ok"] != json!(true) {
            return r;
        }
        r["combo"].as_str().unwrap_or("").to_string()
    };
    let mut patch = serde_json::Map::new();
    patch.insert(format!("hotkey.{action}"), json!(value));
    // hotkeys.rs registers the new combo right away; if the OS refuses it the old binding stays and the old value is restored
    match crate::settings::apply_patch(&app, patch).1 {
        Ok(()) => json!({ "ok": true, "combo": value }),
        Err(e) => json!({ "ok": false, "reason": "taken", "combo": value, "message": e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_normalise() {
        assert_eq!(parse_combo("alt+shift+space").unwrap().0, "Alt+Shift+Space");
        assert_eq!(parse_combo("Shift + Ctrl + v").unwrap().0, "Ctrl+Shift+V");
        assert_eq!(parse_combo("Ctrl+Alt+i").unwrap(), ("Ctrl+Alt+I".into(), MOD_CTRL | MOD_ALT, 'I' as u32));
        assert_eq!(parse_combo("f9").unwrap(), ("F9".into(), 0, 0x78));
        assert_eq!(parse_combo("win+arrowup").unwrap().0, "Win+Up");
        assert!(parse_combo("A").is_none(), "a bare letter is not a hotkey");
        assert!(parse_combo("Ctrl+").is_none());
        assert!(parse_combo("Ctrl+A+B").is_none());
        assert!(parse_combo("Ctrl+F25").is_none());
        assert!(parse_combo("Ctrl+Alt").is_none());
        assert!(parse_combo("").is_none());
    }

    #[test]
    fn conflicts() {
        let b = vec![("toggle".to_string(), "Ctrl+Alt+I".to_string()), ("qr".to_string(), String::new()), ("chat".to_string(), "Ctrl+Alt+A".to_string())];
        assert_eq!(conflict("Ctrl+Alt+I", "chat", &b).as_deref(), Some("toggle"));
        assert_eq!(conflict("Ctrl+Alt+I", "toggle", &b), None, "own combo is no conflict");
        assert_eq!(conflict("ctrl+alt+a", "toggle", &b).as_deref(), Some("chat"));
        assert_eq!(conflict("Ctrl+Alt+Q", "qr", &b).as_deref(), Some("quit"));
        assert_eq!(conflict("Ctrl+Alt+Z", "qr", &b), None);
    }

    #[test]
    fn run_value_is_quoted() {
        assert_eq!(run_value(r"C:\Program Files\Pill\pill.exe"), r#""C:\Program Files\Pill\pill.exe""#);
        assert_eq!(run_value(r#""C:\a\pill.exe""#), r#""C:\a\pill.exe""#);
    }

    #[test]
    fn every_default_action_parses_and_is_unique() {
        let mut seen = std::collections::HashSet::new();
        for (a, d) in ACTIONS {
            if !d.is_empty() {
                assert!(parse_combo(d).is_some(), "{a}");
                assert!(seen.insert(d.to_ascii_lowercase()), "duplicate default {d}");
                assert!(conflict(d, a, &[]).is_none(), "{a} default hits a reserved combo");
            }
        }
    }
}
