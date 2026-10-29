// Global hotkeys of every module in ONE place (GateGuard note: new file of the Pill app; used by main.rs, settings.rs and
// settingswin.rs; user request: "делай все оставшееся" = rebinding a hotkey on the settings page works at once, no restart).
// Owner: settings lane. Handled here: toggle, command, clipboard, chat, record, shotArea, shotFull, ocr, pipette, qr, pastePlain,
// mic, passThrough and every `hotkey.launcher.<id>`. NOT handled here: `voice` (voice.rs follows settings:changed itself),
// `vpn` (no backend yet), Ctrl+Alt+Q = quit (raw RegisterHotKey thread in pill.rs, fixed on purpose).
//
// How it works
//  * desired() = the combo of every action from settings.json (defaults from settingswin::ACTIONS), skipping empty combos and the
//    actions of a module that is switched off (module.clipboard, module.chat, module.launcher).
//  * reload() registers what is missing and unregisters what is gone. Everything runs on the main thread (the shortcut plugin hops
//    there anyway). Each combo goes through the shortcut plugin first and, when the plugin refuses it, through a raw RegisterHotKey
//    owned by one extra thread (same pair as before: plugin -> RegisterHotKey fallback).
//  * strict reload (a `hotkey.*` key changed): the first combo that cannot be registered rolls everything back to the previous
//    bindings and returns Err(text), so the app never ends up without its hotkeys. The caller (settings.rs) then restores the old
//    setting values. Lenient reload (startup, module switches, hotkeys_reload): failures are only logged, the rest stays bound.
use crate::ui::bus::{self, Target};
use crate::settings;
use crate::settingswin::{parse_combo, ACTIONS};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcut, GlobalShortcutExt, Shortcut, ShortcutState};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY, WM_USER};

enum Via {
    Plugin(Shortcut),
    Raw(i32),
}

struct Reg {
    action: String,
    combo: String,
    via: Via,
}

static ST: Mutex<Vec<Reg>> = Mutex::new(Vec::new());
static APP: OnceLock<AppHandle> = OnceLock::new();
static NEXT_ID: AtomicI32 = AtomicI32::new(0x7E00);

fn st() -> std::sync::MutexGuard<'static, Vec<Reg>> {
    ST.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------------------------
// What each action does
// ---------------------------------------------------------------------------------------------------------------

/// Settings key of the module that must be on for the action to exist.
fn module_of(action: &str) -> Option<&'static str> {
    match action {
        "clipboard" => Some("module.clipboard"),
        "chat" => Some("module.chat"),
        a if a.starts_with("launcher.") => Some("module.launcher"),
        _ => None,
    }
}

fn run(app: &AppHandle, action: &str) {
    if module_of(action).is_some_and(|k| !settings::get_bool(k, true)) {
        return;
    }
    match action {
        "toggle" => crate::pill::toggle(app),
        "command" => crate::command::cmd_toggle(app.clone()),
        "clipboard" => {
            let _ = bus::emit(&app, Target::Label("pill"), "clip:open", ());
        }
        "chat" => {
            crate::pill::expand(app, Some("chat".into()), "hotkey");
            let _ = bus::emit(&app, Target::Label("pill"), "chat:focus", ());
        }
        "shotArea" => crate::capture::start(app, "area"),
        "shotFull" => crate::capture::start(app, "screen"),
        "ocr" => crate::capture::start(app, "text"),
        "pipette" => crate::capture::start(app, "color"),
        "qr" => crate::capture::start(app, "qr"),
        "record" => crate::capture::start(app, "record_screen"),
        "pastePlain" => crate::clipboard::paste_plain(),
        "mic" => crate::system::toggle_mic(),
        "passThrough" => {
            let mut m = Map::new();
            m.insert("passThrough".into(), json!(!settings::get_bool("passThrough", false)));
            settings::settings_set(app.clone(), m);
        }
        a if a.starts_with("launcher.") => {
            if let Ok(id) = a["launcher.".len()..].parse::<i64>() {
                std::thread::spawn(move || {
                    crate::launcher::launcher_open(id);
                });
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Raw RegisterHotKey fallback: one thread owns every raw registration (RegisterHotKey belongs to the thread that registers it)
// ---------------------------------------------------------------------------------------------------------------

enum RawOp {
    Reg { id: i32, mods: u32, vk: u32, reply: Sender<bool> },
    Unreg(i32),
}

struct RawThread {
    tid: u32,
    tx: Mutex<Sender<RawOp>>,
}

static RAW: OnceLock<RawThread> = OnceLock::new();
static RAW_MAP: Mutex<Vec<(i32, String)>> = Mutex::new(Vec::new()); // raw id -> action

fn raw() -> &'static RawThread {
    RAW.get_or_init(|| {
        let (tx, rx) = channel::<RawOp>();
        let (tid_tx, tid_rx) = channel::<u32>();
        std::thread::spawn(move || unsafe {
            let mut msg = MSG::default();
            let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE); // creates this thread's message queue
            let _ = tid_tx.send(GetCurrentThreadId());
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_APP {
                    while let Ok(op) = rx.try_recv() {
                        match op {
                            RawOp::Reg { id, mods, vk, reply } => {
                                let _ = reply.send(RegisterHotKey(None, id, HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT, vk).is_ok());
                            }
                            RawOp::Unreg(id) => {
                                let _ = UnregisterHotKey(None, id);
                            }
                        }
                    }
                } else if msg.message == WM_HOTKEY {
                    let id = msg.wParam.0 as i32;
                    let action = RAW_MAP.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(i, _)| *i == id).map(|(_, a)| a.clone());
                    if let (Some(action), Some(app)) = (action, APP.get()) {
                        let a = app.clone();
                        let _ = app.run_on_main_thread(move || run(&a, &action));
                    }
                }
            }
        });
        RawThread { tid: tid_rx.recv_timeout(Duration::from_secs(2)).unwrap_or(0), tx: Mutex::new(tx) }
    })
}

fn raw_send(op: RawOp) -> bool {
    let r = raw();
    if r.tid == 0 {
        return false;
    }
    if r.tx.lock().unwrap_or_else(|e| e.into_inner()).send(op).is_err() {
        return false;
    }
    unsafe { PostThreadMessageW(r.tid, WM_APP, WPARAM(0), LPARAM(0)).is_ok() }
}

fn raw_register(action: &str, mods: u32, vk: u32) -> Option<i32> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (reply, rx) = channel();
    RAW_MAP.lock().unwrap_or_else(|e| e.into_inner()).push((id, action.to_string()));
    let ok = raw_send(RawOp::Reg { id, mods, vk, reply }) && rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false);
    if !ok {
        RAW_MAP.lock().unwrap_or_else(|e| e.into_inner()).retain(|(i, _)| *i != id);
    }
    ok.then_some(id)
}

// ---------------------------------------------------------------------------------------------------------------
// Register / unregister one binding
// ---------------------------------------------------------------------------------------------------------------

fn have_plugin(app: &AppHandle) -> bool {
    app.try_state::<GlobalShortcut<tauri::Wry>>().is_some() // global_shortcut() panics when the plugin is not managed
}

fn register(app: &AppHandle, action: &str, combo: &str) -> Result<Reg, String> {
    let (norm, mods, vk) = parse_combo(combo).ok_or_else(|| format!("'{combo}' is not a valid shortcut"))?;
    if have_plugin(app) {
        // the plugin writes "Super" where Windows people write "Win"
        let spec = norm.split('+').map(|p| if p == "Win" { "Super" } else { p }).collect::<Vec<_>>().join("+");
        if let Ok(sc) = spec.parse::<Shortcut>() {
            let act = action.to_string();
            let res = app.global_shortcut().on_shortcut(sc, move |app, _, ev| {
                if ev.state == ShortcutState::Pressed {
                    run(app, &act);
                }
            });
            match res {
                Ok(()) => return Ok(Reg { action: action.into(), combo: norm, via: Via::Plugin(sc) }),
                Err(e) => eprintln!("hotkeys: plugin refused '{norm}' for {action} ({e}), trying RegisterHotKey"),
            }
        }
    }
    match raw_register(action, mods, vk) {
        Some(id) => Ok(Reg { action: action.into(), combo: norm, via: Via::Raw(id) }),
        None => Err(format!("'{norm}' is taken by another program")),
    }
}

fn unregister(app: &AppHandle, r: &Reg) {
    match r.via {
        Via::Plugin(sc) => {
            if have_plugin(app) {
                let _ = app.global_shortcut().unregister(sc);
            }
        }
        Via::Raw(id) => {
            raw_send(RawOp::Unreg(id));
            RAW_MAP.lock().unwrap_or_else(|e| e.into_inner()).retain(|(i, _)| *i != id);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// desired state + reload
// ---------------------------------------------------------------------------------------------------------------

/// (action, normalised combo) for everything that should be bound right now. Same combo twice: the first action keeps it.
fn desired() -> Vec<(String, String)> {
    let cfg = settings::settings_get();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut want = |action: String, combo: &str| {
        if combo.trim().is_empty() || module_of(&action).is_some_and(|k| !settings::get_bool(k, true)) {
            return;
        }
        match parse_combo(combo) {
            Some((norm, _, _)) if !out.iter().any(|(_, c)| c.eq_ignore_ascii_case(&norm)) => out.push((action, norm)),
            Some((norm, _, _)) => eprintln!("hotkeys: '{norm}' is used twice, {action} stays unbound"),
            None => eprintln!("hotkeys: '{combo}' for {action} is not a valid shortcut"),
        }
    };
    for (a, d) in ACTIONS {
        if matches!(*a, "voice" | "vpn") {
            continue; // voice.rs registers its own; vpn has no backend
        }
        let c = cfg.get(&format!("hotkey.{a}")).and_then(Value::as_str).unwrap_or(d).to_string();
        want(a.to_string(), &c);
    }
    let mut ids: Vec<(&String, &str)> = cfg.iter().filter_map(|(k, v)| Some((k, k.strip_prefix("hotkey.launcher.").and(v.as_str())?))).collect();
    ids.sort();
    for (k, c) in ids {
        want(k["hotkey.".len()..].to_string(), c);
    }
    out
}

/// `strict` = the actions whose `hotkey.*` key was just changed by the user: the first of them that cannot be registered
/// rolls everything back and returns Err. Every other failure (combo busy since startup, ...) is only logged.
fn apply(app: &AppHandle, strict: &[String]) -> Result<(), String> {
    let want = desired();
    let cfg = settings::settings_get();
    for a in strict {
        let set = cfg.get(&format!("hotkey.{a}")).and_then(Value::as_str).unwrap_or("");
        if !set.trim().is_empty() && module_of(a).map_or(true, |k| settings::get_bool(k, true)) && !want.iter().any(|(x, _)| x == a) {
            return Err(format!("{a}: '{set}' is not a valid shortcut or is already used by another action"));
        }
    }
    let mut regs = st();
    let old = std::mem::take(&mut *regs);
    let (keep, gone): (Vec<Reg>, Vec<Reg>) = old.into_iter().partition(|r| want.iter().any(|(a, c)| *a == r.action && c.eq_ignore_ascii_case(&r.combo)));
    for r in &gone {
        unregister(app, r);
    }
    let mut added: Vec<Reg> = Vec::new();
    let mut failed: Option<String> = None;
    for (a, c) in &want {
        if keep.iter().any(|r| r.action == *a) {
            continue;
        }
        match register(app, a, c) {
            Ok(r) => added.push(r),
            Err(e) if strict.contains(a) => {
                failed = Some(format!("{a}: {e}"));
                break;
            }
            Err(e) => eprintln!("hotkeys: {a} ({c}): {e}"),
        }
    }
    if let Some(e) = failed {
        // roll back: drop what this round added, give the previous bindings back
        for r in &added {
            unregister(app, r);
        }
        let mut back = keep;
        for r in gone {
            match register(app, &r.action, &r.combo) {
                Ok(n) => back.push(n),
                Err(e2) => eprintln!("hotkeys: could not restore {} ({}): {e2}", r.action, r.combo),
            }
        }
        *regs = back;
        return Err(e);
    }
    *regs = keep.into_iter().chain(added).collect();
    Ok(())
}

/// Bring the OS registrations in line with the settings (see the file header for strict / lenient; `strict` empty = lenient).
pub fn reload(app: &AppHandle, strict: &[String]) -> Result<(), String> {
    let (tx, rx) = channel();
    let (a, strict) = (app.clone(), strict.to_vec());
    app.run_on_main_thread(move || {
        let _ = tx.send(apply(&a, &strict));
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(Duration::from_secs(15)).map_err(|e| e.to_string())?
}

/// Re-read the hotkey settings and re-register what changed; also retries combos that were busy before.
#[tauri::command]
pub fn hotkeys_reload(app: AppHandle) -> Result<Value, String> {
    reload(&app, &[])?;
    let s = st();
    Ok(json!({ "bound": s.iter().map(|r| json!({ "action": r.action, "combo": r.combo })).collect::<Vec<_>>() }))
}

pub fn init(app: &tauri::App) {
    let _ = APP.set(app.handle().clone());
    if let Err(e) = reload(app.handle(), &[]) {
        eprintln!("hotkeys: first registration failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_gate_their_actions() {
        assert_eq!(module_of("clipboard"), Some("module.clipboard"));
        assert_eq!(module_of("launcher.12"), Some("module.launcher"));
        assert_eq!(module_of("toggle"), None);
    }

    #[test]
    fn every_default_is_registrable_text() {
        // the plugin spelling of a normalised combo must parse: Win -> Super
        for (a, d) in ACTIONS {
            if let Some((norm, _, _)) = parse_combo(d) {
                let spec = norm.split('+').map(|p| if p == "Win" { "Super" } else { p }).collect::<Vec<_>>().join("+");
                assert!(spec.parse::<Shortcut>().is_ok(), "{a}: {spec}");
            }
        }
    }
}
