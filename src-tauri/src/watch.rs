// Keyboard-layout watcher (GateGuard note: new file for the Pill app, used by main.rs + src/live-events.js;
// user request: "keyboard layout indicator and copy reaction for the pill"). Owner: live-events lane.
// One poller thread: 250 ms while the pill is visible, 1000 ms while hidden (the page calls watch_pause / watch_resume from
// its pill:hide / pill:show handlers). Emits `kbd:layout` {lang:'RU', name:'Russian'} to window "pill" only when the layout of
// the SAME foreground thread changed and stayed changed for 2 polls. Never on startup, never for a null layout.
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};
use windows::Win32::Globalization::{GetLocaleInfoW, LOCALE_SENGLISHLANGUAGENAME, LOCALE_SISO639LANGNAME};
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

static HIDDEN: AtomicBool = AtomicBool::new(false);

// ---------- pure parts (unit-tested) ----------
/// ISO 639-1 code -> what users write on the indicator. Everything else is just upper-cased.
const ALIAS: &[(&str, &str)] = &[("uk", "UA"), ("el", "GR"), ("cs", "CZ"), ("da", "DK"), ("sv", "SE"), ("ja", "JP"), ("ko", "KR")];

fn display_code(iso: &str) -> String {
    let iso = iso.trim().to_lowercase();
    ALIAS.iter().find(|(k, _)| *k == iso).map_or_else(|| iso.to_uppercase(), |(_, v)| v.to_string())
}

/// Decides when a layout change is a real one. Feed it (thread id, language id) every poll.
/// The first poll of a thread, and every switch of the foreground thread, only set the baseline.
#[derive(Default)]
struct Tracker {
    tid: u32,
    lid: u16,
    pending: Option<(u16, u8)>, // candidate layout of the same thread + consecutive polls seen
}

impl Tracker {
    fn step(&mut self, tid: u32, lid: u16) -> Option<u16> {
        if tid == 0 || lid == 0 {
            *self = Tracker::default();
            return None;
        }
        if tid != self.tid {
            *self = Tracker { tid, lid, pending: None };
            return None;
        }
        if lid == self.lid {
            self.pending = None;
            return None;
        }
        let n = match self.pending {
            Some((p, n)) if p == lid => n + 1,
            _ => 1,
        };
        if n >= 2 {
            self.lid = lid;
            self.pending = None;
            return Some(lid);
        }
        self.pending = Some((lid, n));
        None
    }
}

// ---------- Win32 ----------
/// (foreground thread id, language id of its layout); None when there is no foreground window or the layout is null.
fn current() -> Option<(u32, u16)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        let lid = (hkl.0 as usize & 0xFFFF) as u16;
        (tid != 0 && lid != 0).then_some((tid, lid))
    }
}

fn locale_str(lid: u16, kind: u32) -> String {
    let mut b = [0u16; 80];
    let n = unsafe { GetLocaleInfoW(lid as u32, kind, Some(&mut b)) };
    if n <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&b[..n as usize - 1])
}

fn info(lid: u16) -> Value {
    let iso = locale_str(lid, LOCALE_SISO639LANGNAME);
    json!({ "lang": if iso.is_empty() { format!("{lid:04X}") } else { display_code(&iso) }, "name": locale_str(lid, LOCALE_SENGLISHLANGUAGENAME) })
}

fn run(app: AppHandle) {
    let mut t = Tracker::default();
    loop {
        if !crate::settings::get_bool("live.keyboardLayout", true) {
            // Settings > Live events > Keyboard layout is off: no polling of the foreground window at all
            t = Tracker::default();
            std::thread::sleep(std::time::Duration::from_millis(1000));
            continue;
        }
        if let Some((tid, lid)) = current() {
            if let Some(new) = t.step(tid, lid) {
                let _ = app.emit_to("pill", "kbd:layout", info(new));
            }
        } else {
            t.step(0, 0);
        }
        std::thread::sleep(std::time::Duration::from_millis(if HIDDEN.load(Ordering::Relaxed) { 1000 } else { 250 }));
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    std::thread::spawn(move || run(h));
}

// ---------- commands ----------
/// Current layout of the foreground window, for rendering on demand: {lang, name} or null.
#[tauri::command]
pub fn kbd_state() -> Option<Value> {
    current().map(|(_, lid)| info(lid))
}

#[tauri::command]
pub fn watch_pause() {
    HIDDEN.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub fn watch_resume() {
    HIDDEN.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping() {
        assert_eq!(display_code("ru"), "RU");
        assert_eq!(display_code("en"), "EN");
        assert_eq!(display_code("uk"), "UA");
        assert_eq!(display_code(" UK "), "UA");
        assert_eq!(display_code("de"), "DE");
    }

    #[test]
    fn same_thread_change_only() {
        let (en, ru) = (0x0409, 0x0419);
        let mut t = Tracker::default();
        assert_eq!(t.step(1, en), None); // baseline
        assert_eq!(t.step(1, en), None);
        assert_eq!(t.step(1, ru), None); // first poll of the new layout: not stable yet
        assert_eq!(t.step(1, ru), Some(ru)); // second: fire
        assert_eq!(t.step(1, ru), None); // no repeat
        assert_eq!(t.step(2, en), None); // other window with another layout: baseline only
        assert_eq!(t.step(2, en), None);
        assert_eq!(t.step(1, ru), None); // back to thread 1: baseline again, no flash
        assert_eq!(t.step(1, en), None);
        assert_eq!(t.step(1, ru), None); // flicker is not stable
        assert_eq!(t.step(1, en), None);
        assert_eq!(t.step(0, 0), None); // desktop / null layout
        assert_eq!(t.step(1, en), None); // re-baseline after null
    }
}
