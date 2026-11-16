// Windows Focus Assist ("quiet time", "do not disturb") for the island. Owner: focus lane. Opt-in: `notify.respectFocus` (off by default; with it
// off nothing here runs and Windows is never asked).
//
// There is no event for "Focus Assist turned on / off", so the state (`SHQueryUserNotificationState`) is re-read only when something that can
// change it has just happened, never on a timer while idle:
//  * the foreground window changed (`EVENT_SYSTEM_FOREGROUND` hook, wakes only then; opening and closing the quick settings flyout is such a
//    change, and so is a game or video going fullscreen). The state lags a moment behind the window, so one more read follows 1.5 s later;
//  * the session was unlocked / reconnected, the PC woke up, the display setup changed (a hidden window, like watchdog.rs);
//  * the settings switch was flipped, and the island asked at start (`focus_state`).
// While the island HOLDS cards (and only then) it asks for `focus_watch` and the state is also re-read every 15 s, because the end of a
// Focus Assist period usually makes no event at all. Nothing is held in the normal case, so nothing ticks.
//
// Emits `focus:changed {state, quiet}` on the bus when the answer changed. The decision what to hold lives in pill_ui_core::live::focus.
use crate::ui::bus;
use pill_ui_core::live::focus::{qun_is_quiet, qun_name};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Shell::SHQueryUserNotificationState;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW, RegisterClassExW, SetTimer, TranslateMessage, EVENT_SYSTEM_FOREGROUND, MSG,
    WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT, WM_APP, WM_DISPLAYCHANGE, WM_TIMER, WNDCLASSEXW,
};

const KEY: &str = "notify.respectFocus";
const WM_POWERBROADCAST: u32 = 0x0218;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WM_WATCH: u32 = WM_APP + 7;
/// One more read after a foreground change: the state follows the window a moment later.
const SETTLE_MS: u32 = 1500;
/// While cards are held: how often Windows is asked whether the quiet time is over.
const WATCH_MS: u32 = 15_000;
const T_SETTLE: usize = 1;
const T_WATCH: usize = 2;

static STARTED: AtomicBool = AtomicBool::new(false);
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The last answer sent (-1 = none yet), so only a change is announced.
static LAST: AtomicI32 = AtomicI32::new(-1);

fn wanted() -> bool {
    crate::settings::get_bool(KEY, false)
}

/// The raw `QUERY_USER_NOTIFICATION_STATE`, `None` when Windows would not say.
fn query() -> Option<i32> {
    unsafe { SHQueryUserNotificationState().ok().map(|s| s.0) }
}

fn payload(q: Option<i32>) -> Value {
    match q {
        Some(q) => json!({ "state": qun_name(q), "quiet": qun_is_quiet(q) }),
        None => json!({ "state": "unknown", "quiet": false }),
    }
}

/// Reads the state and announces it when it changed. Does nothing while the switch is off.
fn refresh() {
    if !wanted() {
        return;
    }
    let q = query();
    if LAST.swap(q.unwrap_or(0), Ordering::SeqCst) != q.unwrap_or(0) {
        let _ = bus::emit("focus:changed", payload(q));
    }
}

/// Command `focus_state`: the answer for the island's start. Off: "open", and Windows is not asked.
pub fn focus_state() -> Value {
    if !wanted() {
        return payload(Some(5));
    }
    let q = query();
    LAST.store(q.unwrap_or(0), Ordering::SeqCst);
    payload(q)
}

/// Command `focus_watch {on}`: the island holds cards (`on`) or no longer does; sets or kills the 15 s re-read on the hook thread.
pub fn focus_watch(on: bool) {
    let h = WINDOW.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as *mut std::ffi::c_void), WM_WATCH, WPARAM(on as usize), LPARAM(0));
        }
    }
}

unsafe extern "system" fn on_foreground(_h: HWINEVENTHOOK, _ev: u32, _hwnd: HWND, _obj: i32, _child: i32, _thread: u32, _ms: u32) {
    if !wanted() {
        return;
    }
    refresh();
    let w = WINDOW.load(Ordering::Acquire);
    if w != 0 {
        SetTimer(HWND(w as *mut std::ffi::c_void), T_SETTLE, SETTLE_MS, None); // re-arming restarts it: a burst of changes reads once, after it
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // PBT_APMRESUMESUSPEND 7, PBT_APMRESUMEAUTOMATIC 0x12
        WM_POWERBROADCAST if matches!(wp.0, 0x7 | 0x12) => refresh(),
        // WTS_CONSOLE_CONNECT 1, WTS_REMOTE_CONNECT 3, WTS_SESSION_LOGON 5, WTS_SESSION_UNLOCK 8
        WM_WTSSESSION_CHANGE if matches!(wp.0, 0x1 | 0x3 | 0x5 | 0x8) => refresh(),
        WM_DISPLAYCHANGE => refresh(), // "when duplicating my display" is one of Focus Assist's automatic rules
        WM_TIMER => {
            if wp.0 == T_SETTLE {
                let _ = KillTimer(hwnd, T_SETTLE);
            }
            refresh();
        }
        WM_WATCH => {
            if wp.0 != 0 {
                SetTimer(hwnd, T_WATCH, WATCH_MS, None);
            } else {
                let _ = KillTimer(hwnd, T_WATCH);
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// The hook thread: a hidden window (power and session messages, the two timers) and the foreground hook, then the message loop.
fn run() {
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else { return };
        let cls = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(proc), hInstance: hinst.into(), lpszClassName: w!("PillFocusAssist"), ..Default::default() };
        RegisterClassExW(&cls);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PillFocusAssist"), w!("pill-focus-assist"), WINDOW_STYLE(0x8000_0000 /* WS_POPUP, never shown */), 0, 0, 0, 0, None, None, hinst, None) else {
            return;
        };
        let _ = WTSRegisterSessionNotification(hwnd, 0 /* NOTIFY_FOR_THIS_SESSION */);
        WINDOW.store(hwnd.0 as isize, Ordering::Release);
        let _ = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    STARTED.store(false, Ordering::SeqCst);
}

/// Starts the hook thread once, when the switch is on. It stays (asleep in GetMessage) after the switch goes off; the callbacks check the switch.
fn ensure() {
    if cfg!(test) || !wanted() || STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if std::thread::Builder::new().name("focus-assist".into()).spawn(run).is_err() {
        STARTED.store(false, Ordering::SeqCst);
    }
}

pub fn init(_app: &tauri::App) {
    ensure();
    refresh();
}

pub fn on_settings(patch: &Map<String, Value>) {
    if !patch.contains_key(KEY) {
        return;
    }
    LAST.store(-1, Ordering::SeqCst);
    if wanted() {
        ensure();
        refresh();
    } else {
        // the island lets go of what it holds (it also sees the setting itself); nothing is read any more
        let _ = bus::emit("focus:changed", payload(Some(5)));
        focus_watch(false);
    }
}
