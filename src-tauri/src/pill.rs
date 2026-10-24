// The pill window: placement at the top center, collapsed/expanded frames, cursor loop, focus handling,
// visibility (guard / auto-hide / tray), tray icon and hotkeys. Owner: pill agent (lane L1).
//
// Window model (architecture 2.1): one transparent always-on-top window, never resized while the page animates.
// Expand: Rust enlarges the window -> emits `pill:expand` -> page animates. Collapse: page animates, then calls
// `pill_set_state("collapsed")` and Rust shrinks the window. Rust owns the cursor (hover / top-edge) and emits
// `pill:hover` / `pill:leave`.
use serde_json::{json, Map, Value};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS,
    MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, RegisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    VK_LBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId,
    IsWindow, SetForegroundWindow, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, HWND_TOPMOST, MSG, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SW_HIDE, SW_SHOWNOACTIVATE, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_HOTKEY, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

// Frame sizes in logical px at 100 % user size. The page mirrors them (pill.js CFG), keep both in sync.
// Collapsed height: 7 top gap + 36 capsule + 14 liquid edge + gap + a dropped card (48) fits in 120.
// every frame is as wide as EXPANDED: a width change moves the window's left edge while the page still lays the island out for the old width,
// and the island jumped sideways for a frame at each resize (the collapsed rest of the frame is click-through, see set_ignore)
const COLLAPSED: (f64, f64) = (720.0, 120.0);
const EXPANDED: (f64, f64) = (720.0, 480.0);

// Reasons the window is hidden; it is shown only when none is set.
pub const HIDE_USER: u8 = 1; // tray "Show / Hide"
pub const HIDE_GUARD: u8 = 2; // fullscreen / per-app (guard.rs)
pub const HIDE_AUTO: u8 = 4; // auto-hide slid the pill away

struct St {
    hwnd: isize,
    expanded: bool,    // OS window is in the expanded frame
    hidden: u8,        // HIDE_* bits
    shown: bool,       // the OS window is currently visible
    rect: [f64; 4],    // pill body rect reported by the page, css px relative to the window
    mon: [i32; 4],     // target monitor rect, physical px
    scale: f64,        // target monitor scale factor (css px -> physical px)
    layout: [i32; 4],  // last placed x, y, w, h
    prev_fg: isize,    // window that had focus before pill_activate(true)
    active: bool,
}

static ST: Mutex<St> = Mutex::new(St {
    hwnd: 0, expanded: false, hidden: 0, shown: true, rect: [0.0; 4], mon: [0; 4], scale: 1.0,
    layout: [0; 4], prev_fg: 0, active: false,
});

fn st() -> MutexGuard<'static, St> {
    ST.lock().unwrap_or_else(|e| e.into_inner())
}

fn me() -> HWND {
    HWND(st().hwnd as *mut c_void)
}

/// Run on the UI thread (window calls from helper threads).
fn on_main(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || f(&a));
}

struct Lay {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    mon: [i32; 4],
    scale: f64,
}

/// Target monitor + frame size for the current settings.
fn layout(app: &AppHandle, expanded: bool) -> Option<Lay> {
    let win = app.get_webview_window("pill")?;
    let mons = win.available_monitors().ok()?;
    let want = crate::settings::get_str("monitor", "primary");
    let mon = want
        .parse::<usize>()
        .ok()
        .and_then(|i| mons.get(i.saturating_sub(1)).cloned())
        .or_else(|| win.primary_monitor().ok().flatten())
        .or_else(|| mons.first().cloned())?;
    let (p, s, sf) = (mon.position(), mon.size(), mon.scale_factor());
    let (ml, mt, mw, mh) = (p.x, p.y, s.width as i32, s.height as i32);
    // Sit below a top taskbar / bar (setting `underTaskbar`, default on): use the work-area top of that monitor.
    let mut top = mt;
    if crate::settings::get_bool("underTaskbar", true) {
        unsafe {
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            let hm = MonitorFromPoint(POINT { x: ml + mw / 2, y: mt + 10 }, MONITOR_DEFAULTTONEAREST);
            if GetMonitorInfoW(hm, &mut mi).as_bool() {
                top = mi.rcWork.top.max(mt);
            }
        }
    }
    let k = sf * crate::settings::get_f64("size", 100.0).clamp(80.0, 200.0) / 100.0;
    let (fw, fh) = if expanded { EXPANDED } else if CARD.load(Ordering::Relaxed) { CARD_FRAME } else { COLLAPSED };
    let w = ((fw * k).round() as i32).min(mw);
    let h = (fh * k).round() as i32;
    // `offsetX` (-50..50 % of the monitor width, 0 = middle) moves the window centre; the window stays inside the monitor.
    let off = crate::settings::get_f64("offsetX", 0.0).clamp(-50.0, 50.0);
    let cx = ml as f64 + mw as f64 / 2.0 + off / 100.0 * mw as f64;
    let x = ((cx - w as f64 / 2.0).round() as i32).clamp(ml, (ml + mw - w).max(ml));
    Some(Lay { x, y: top, w, h, mon: [ml, mt, ml + mw, mt + mh], scale: sf })
}

/// Put the window into its current frame (collapsed/expanded) on the target monitor. UI thread only.
pub fn place(app: &AppHandle) {
    let exp = st().expanded;
    let Some(l) = layout(app, exp) else { return };
    {
        let mut s = st();
        s.mon = l.mon;
        s.scale = l.scale;
        s.layout = [l.x, l.y, l.w, l.h];
    }
    pass_through(app, exp);
    unsafe {
        let _ = SetWindowPos(me(), HWND_TOPMOST, l.x, l.y, l.w, l.h, SWP_NOACTIVATE);
    }
    HZ_DIRTY.store(true, Ordering::Relaxed); // monitor may have changed: cursor loop re-detects + re-emits `pill:display`
}

/// Setting `passThrough`: while the pill is folded the window lets mouse clicks fall through to what is beneath it (the cursor
/// loop still reads the cursor, but ignores hover / top-edge while this is on); the expanded frame is always clickable.
/// ponytail: an event card dropped from a folded pill is click-through too; split the window if its buttons must work in this mode.
static PT: AtomicBool = AtomicBool::new(false);

fn pass_through(_app: &AppHandle, expanded: bool) {
    let want = crate::settings::get_bool("passThrough", false) && !expanded;
    if PT.swap(want, Ordering::Relaxed) == want {
        return;
    }
    set_ignore(want);
}

/// Makes the whole window let clicks fall through (pass-through mode, or the cursor is outside the pill's own rectangle: the
/// transparent rest of the 560x120 frame must not block what is underneath, e.g. a browser's toolbar buttons).
/// Done on the window style directly: tauri's `set_ignore_cursor_events` returned Ok but left the style untouched here (measured), and
/// the window layer may rewrite the style later, so the cursor loop calls this every pass and it only writes when the style differs.
fn set_ignore(want: bool) {
    use windows::Win32::Foundation::COLORREF;
    use windows::Win32::UI::WindowsAndMessaging::{SetLayeredWindowAttributes, LWA_ALPHA, WS_EX_LAYERED, WS_EX_TRANSPARENT};
    unsafe {
        let me = me();
        let ex = GetWindowLongPtrW(me, GWL_EXSTYLE);
        let tr = (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as isize;
        let new = if want { ex | tr } else { ex & !(WS_EX_TRANSPARENT.0 as isize) };
        if new != ex {
            SetWindowLongPtrW(me, GWL_EXSTYLE, new);
            if want {
                let _ = SetLayeredWindowAttributes(me, COLORREF(0), 255, LWA_ALPHA); // a layered window is invisible without attributes
            }
        }
    }
}

/// True while auto-hide has slid the pill away (guard.rs brings it back over the desktop when `autoHideKeepDesktop` is on).
pub fn auto_hidden() -> bool {
    st().hidden & HIDE_AUTO != 0
}

/// Called by the guard poll (500 ms): re-place when monitor / DPI / taskbar / size changed.
pub fn relayout(app: &AppHandle) {
    on_main(app, |a| {
        let (exp, last) = {
            let s = st();
            (s.expanded, s.layout)
        };
        if let Some(l) = layout(a, exp) {
            if [l.x, l.y, l.w, l.h] != last {
                place(a);
            }
        }
    });
}

fn apply_vis(app: &AppHandle, reason: &str) {
    let (want, was) = {
        let s = st();
        (s.hidden == 0, s.shown)
    };
    if want == was {
        return;
    }
    st().shown = want;
    if want {
        unsafe {
            let _ = ShowWindow(me(), SW_SHOWNOACTIVATE);
        }
        place(app); // re-assert topmost + re-place after being hidden
        let _ = app.emit_to("pill", "pill:show", json!({ "reason": reason }));
    } else {
        let _ = app.emit_to("pill", "pill:hide", json!({ "reason": reason }));
        unsafe {
            let _ = ShowWindow(me(), SW_HIDE);
        }
    }
}

/// True while the pill window is hidden (fullscreen guard, auto-hide, tray, hide-in-apps): background work may pause.
pub fn is_hidden() -> bool {
    st().hidden != 0
}

/// Set or clear HIDE_* bits (mask). UI thread only; helper threads use `set_hidden_bg`.
pub fn set_hidden(app: &AppHandle, mask: u8, on: bool, reason: &str) {
    {
        let mut s = st();
        if on {
            s.hidden |= mask;
        } else {
            s.hidden &= !mask;
        }
    }
    apply_vis(app, reason);
}

pub fn set_hidden_bg(app: &AppHandle, mask: u8, on: bool, reason: &'static str) {
    on_main(app, move |a| set_hidden(a, mask, on, reason));
}

/// Enlarge the window, then tell the page to animate.
pub fn expand(app: &AppHandle, tab: Option<String>, via: &str) {
    if st().hidden & HIDE_GUARD != 0 {
        return; // never pop up over a fullscreen app
    }
    set_hidden(app, HIDE_USER | HIDE_AUTO, false, "expand");
    st().expanded = true;
    place(app);
    let _ = app.emit_to("pill", "pill:expand", json!({ "tab": tab, "via": via }));
}

pub fn toggle(app: &AppHandle) {
    if st().expanded {
        let _ = app.emit_to("pill", "pill:collapse", ());
    } else {
        expand(app, None, "hotkey");
    }
}

/// Settings that need Rust to react right away.
pub fn on_settings(app: &AppHandle, patch: &Map<String, Value>) {
    if ["size", "monitor", "offsetX", "underTaskbar", "passThrough"].iter().any(|k| patch.contains_key(*k)) {
        place(app);
    }
    if patch.contains_key("showInCapture") {
        affinity();
    }
    if patch.get("autoHide").and_then(|v| v.as_bool()) == Some(false) {
        set_hidden(app, HIDE_AUTO, false, "settings");
    }
}

fn affinity() {
    let a = if crate::settings::get_bool("showInCapture", true) { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE };
    unsafe {
        let _ = SetWindowDisplayAffinity(me(), a);
    }
}

// ---- commands ----

#[tauri::command]
pub fn pill_rect(x: f64, y: f64, w: f64, h: f64) {
    st().rect = [x, y, w, h];
}

#[tauri::command]
pub fn pill_set_state(app: AppHandle, state: String, tab: Option<String>, via: Option<String>) {
    if state == "expanded" {
        expand(&app, tab, via.as_deref().unwrap_or("page"));
    } else {
        pill_activate(app.clone(), false);
        st().expanded = false;
        place(&app);
    }
}

#[tauri::command]
pub fn pill_toggle(app: AppHandle) {
    toggle(&app);
}

/// The page slid away (on=true: hide the window) or an event/click wants it back (on=false).
#[tauri::command]
pub fn pill_auto_hide(app: AppHandle, on: bool) {
    set_hidden(&app, HIDE_AUTO, on, if on { "auto" } else { "event" });
}

/// Text fields need keyboard focus: drop NOACTIVATE and take the foreground; restore on false.
#[tauri::command]
pub fn pill_activate(app: AppHandle, on: bool) {
    if st().active == on {
        return;
    }
    st().active = on;
    unsafe {
        let me = me();
        let ex = GetWindowLongPtrW(me, GWL_EXSTYLE);
        if on {
            let fg = GetForegroundWindow();
            if fg.0 != me.0 {
                st().prev_fg = fg.0 as isize;
            }
            SetWindowLongPtrW(me, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE.0 as isize));
            // Borrow the foreground thread's input state so SetForegroundWindow is allowed.
            let (mine, theirs) = (GetCurrentThreadId(), GetWindowThreadProcessId(fg, None));
            let attached = theirs != 0 && theirs != mine && AttachThreadInput(mine, theirs, true).as_bool();
            let _ = SetForegroundWindow(me);
            if attached {
                let _ = AttachThreadInput(mine, theirs, false);
            }
            if let Some(w) = app.get_webview_window("pill") {
                let _ = w.set_focus();
            }
        } else {
            SetWindowLongPtrW(me, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
            let prev = std::mem::take(&mut st().prev_fg);
            let prev = HWND(prev as *mut c_void);
            if !prev.0.is_null() && IsWindow(prev).as_bool() {
                let _ = SetForegroundWindow(prev);
            }
        }
    }
}

/// Monitor names for the settings tab ("1".."n" map to this order).
#[tauri::command]
pub fn pill_monitors(app: AppHandle) -> Vec<String> {
    let Some(w) = app.get_webview_window("pill") else { return vec![] };
    let list = w.available_monitors().unwrap_or_default();
    list.iter()
        .map(|m| format!("{} {}x{}", m.name().map(String::as_str).unwrap_or(""), m.size().width, m.size().height))
        .collect()
}

// ---- cursor loop ----

const LIQUID: f64 = 14.0; // css px of liquid edge under the body: part of the hover rect
const ZONE: f64 = 120.0; // logical px around the pill where `pill:cursor` is streamed

#[derive(serde::Serialize, Clone)]
struct Cur {
    x: Option<f64>,
    y: Option<f64>,
}

/// True when (px, py) lies within `m` of the rect l,t,r,b (a point inside the rect counts as near).
fn near_rect(px: i32, py: i32, l: i32, t: i32, r: i32, b: i32, m: i32) -> bool {
    px >= l - m && px < r + m && py >= t - m && py < b + m
}

static HZ_DIRTY: AtomicBool = AtomicBool::new(true); // force a `pill:display` emit (set by place())

/// Loop period for a refresh rate: one frame, floored at 6 ms (never spin faster than ~166 Hz), 60 Hz if unknown.
fn period(hz: u32) -> Duration {
    Duration::from_micros((1_000_000 / if hz == 0 { 60 } else { hz } as u64).max(6_000))
}

/// Current mode refresh rate of the monitor hosting the pill window; 60 when unknown.
/// ponytail: nominal mode rate (max rate on VRR panels), which is what rAF follows; DWM timing is per-system, not per-monitor.
fn display_hz(hwnd: HWND) -> u32 {
    unsafe {
        let hm = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(hm, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return 60;
        }
        let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        if EnumDisplaySettingsW(PCWSTR(mi.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool()
            && dm.dmDisplayFrequency > 1 // 0 / 1 = "hardware default"
        {
            return dm.dmDisplayFrequency.clamp(24, 1000);
        }
    }
    60
}

/// Global gaze feed (`pill:gaze`): set by the page (pets / eyes character active) through `pill_set_gaze`; default off.
static GAZE: AtomicBool = AtomicBool::new(false);

/// An event card hangs below the folded pill: while one is shown the folded frame is taller so a long notification fits.
static CARD: AtomicBool = AtomicBool::new(false);
const CARD_FRAME: (f64, f64) = (720.0, 200.0);

#[tauri::command]
pub fn pill_cards(app: AppHandle, on: bool) {
    if CARD.swap(on, Ordering::Relaxed) != on {
        on_main(&app, place);
    }
}

#[tauri::command]
pub fn pill_set_gaze(on: bool) {
    GAZE.store(on, Ordering::Relaxed);
}

/// Wait before the next poll while the gaze feed is on and the cursor is far from the pill:
/// one frame but never faster than 60 Hz while it moves (never slower than ~30 Hz), a relaxed 50 ms poll when it rests.
fn gaze_wait(hz: u32, moved: bool) -> Duration {
    if moved { period(hz).max(Duration::from_millis(16)) } else { Duration::from_millis(50) }
}

#[cfg(test)]
mod tests {
    use super::{gaze_wait, near_rect, period};
    use std::time::Duration;
    #[test]
    fn gaze() {
        assert_eq!(gaze_wait(144, true), Duration::from_millis(16)); // floor: far cursor is polled at <= 60 Hz
        assert_eq!(gaze_wait(30, true), period(30)); // slow panel: its own frame, still >= 30 Hz
        assert_eq!(gaze_wait(240, false), Duration::from_millis(50)); // idle
    }
    #[test]
    fn cadence() {
        assert_eq!(period(60), Duration::from_micros(16_666));
        assert_eq!(period(144), Duration::from_micros(6_944));
        assert_eq!(period(240), Duration::from_millis(6)); // floor
        assert_eq!(period(0), period(60)); // unknown -> 60
    }
    #[test]
    fn zone() {
        assert!(near_rect(50, 50, 0, 0, 100, 40, 120)); // below the rect, within the margin
        assert!(near_rect(-119, 0, 0, 0, 100, 40, 120));
        assert!(!near_rect(-121, 0, 0, 0, 100, 40, 120));
        assert!(!near_rect(50, 161, 0, 0, 100, 40, 120));
    }
}

fn emit(app: &AppHandle, name: &str) {
    let _ = app.emit_to("pill", name, ());
}

/// One thread owns the cursor: display-rate (deadline-paced) near the pill, 10 Hz far, nothing but a flag check
/// while guard-hidden. Also re-checks the monitor refresh rate ~1/s and emits `pill:display` {hz} on change.
fn cursor_loop(app: AppHandle) {
    let (mut inside, mut left_at, mut edge_at): (bool, Option<Instant>, Option<Instant>) = (false, None, None);
    let (mut zone_on, mut last) = (false, (i32::MIN, i32::MIN)); // `pill:cursor` stream state
    let mut gaze_last = (i32::MIN, i32::MIN); // `pill:gaze` stream state (MIN = nothing sent since it was enabled / the window came back)
    let nap = |ms| std::thread::sleep(Duration::from_millis(ms));
    let (mut hz, mut hz_at, mut page_up) = (60u32, Instant::now(), false); // HZ_DIRTY starts true -> first pass detects
    let mut next = Instant::now(); // frame deadline for the fast cadence
    loop {
        let (hidden, rect, mon, sc, hwnd) = {
            let s = st();
            (s.hidden, s.rect, s.mon, s.scale, s.hwnd)
        };
        // Refresh-rate watch: ~1/s, plus on place() and once when the page first reports a rect (it is listening then).
        let dirty = HZ_DIRTY.swap(false, Ordering::Relaxed);
        let first = rect[2] > 0.0 && !page_up;
        if hwnd != 0 && (dirty || first || hz_at.elapsed() >= Duration::from_secs(1)) {
            hz_at = Instant::now();
            let h = display_hz(HWND(hwnd as *mut c_void));
            if h != hz || dirty || first {
                hz = h;
                let _ = app.emit_to("pill", "pill:display", json!({ "hz": hz }));
            }
            page_up |= first;
        }
        if hidden & (HIDE_GUARD | HIDE_USER) != 0 || rect[2] <= 0.0 {
            inside = false;
            zone_on = false;
            gaze_last = (i32::MIN, i32::MIN);
            left_at = None;
            edge_at = None;
            nap(100);
            continue;
        }
        let (mut p, mut wr) = (POINT::default(), RECT::default());
        unsafe {
            if GetCursorPos(&mut p).is_err() || GetWindowRect(HWND(hwnd as *mut c_void), &mut wr).is_err() {
                nap(100);
                continue;
            }
        }
        let l = wr.left + (rect[0] * sc) as i32;
        let r = l + (rect[2] * sc) as i32;
        // Hit rect: from the window top (the body may hang a few px below it) to the bottom of the liquid strip.
        let t = wr.top;
        let b = wr.top + ((rect[1] + rect[3] + LIQUID) * sc) as i32;
        let pad = 4;
        let x_in = p.x >= l - pad && p.x < r + pad;
        let in_rect = hidden == 0 && x_in && p.y >= t - pad && p.y < b + pad;
        // Click-through everywhere but on the pill itself (unless pass-through already covers the whole window): the cursor loop is
        // the hit test, a few px of slack so the edge pixels and the liquid strip stay grabbable.
        if !PT.load(Ordering::Relaxed) {
            set_ignore(!in_rect);
        }        // Top-edge strip: cursor pinned on the monitor top inside the pill's x range for ~120 ms
        // (ignored while a mouse button is held: dragging a window to the top must not pop the pill).
        let held = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 };
        let on_edge = x_in && p.y <= mon[1] + 1 && !held;
        edge_at = if on_edge { edge_at.or(Some(Instant::now())) } else { None };
        let edge_ok = edge_at.is_some_and(|t| t.elapsed() >= Duration::from_millis(120));

        if hidden & HIDE_AUTO != 0 {
            if edge_ok {
                edge_at = None;
                set_hidden_bg(&app, HIDE_AUTO, false, "edge");
            }
            zone_on = false;
            gaze_last = (i32::MIN, i32::MIN);
            nap(100); // ponytail: 10 Hz is the reveal latency while auto-hidden
            continue;
        }

        // pass-through (folded pill): no hover / top-edge opening, the pill opens by its hotkey only
        if !PT.load(Ordering::Relaxed) && (in_rect || edge_ok) {
            left_at = None;
            if !inside {
                inside = true;
                emit(&app, "pill:hover");
            }
        } else if inside {
            match left_at {
                None => left_at = Some(Instant::now()),
                Some(t) if t.elapsed() >= Duration::from_millis(250) => {
                    inside = false;
                    left_at = None;
                    emit(&app, "pill:leave");
                }
                _ => {}
            }
        }
        // `pill:cursor` for the liquid edge: only inside the window frame or within ZONE of the pill rect;
        // one {null,null} when leaving, then silence. Nothing is sent while the position is unchanged.
        let in_frame = p.x >= wr.left && p.x < wr.right && p.y >= wr.top && p.y < wr.bottom;
        let zone = in_frame || near_rect(p.x, p.y, l, t, r, b, (ZONE * sc) as i32);
        if zone {
            if !zone_on || (p.x, p.y) != last {
                zone_on = true;
                last = (p.x, p.y);
                let c = Cur { x: Some((p.x - wr.left) as f64 / sc), y: Some((p.y - wr.top) as f64 / sc) };
                let _ = app.emit_to("pill", "pill:cursor", c);
            }
        } else if zone_on {
            zone_on = false;
            let _ = app.emit_to("pill", "pill:cursor", Cur { x: None, y: None });
        }
        // `pill:gaze` for the pets' / eyes' look: the cursor in PHYSICAL px relative to the window's top-left (negative / beyond the
        // window is fine: any distance, any monitor). Only while the page asked for it and the cursor moved; once on enable.
        let gaze_on = GAZE.load(Ordering::Relaxed);
        let mut moved = false;
        if gaze_on {
            if (p.x, p.y) != gaze_last {
                gaze_last = (p.x, p.y);
                moved = true;
                let _ = app.emit_to("pill", "pill:gaze", json!({ "x": p.x - wr.left, "y": p.y - wr.top }));
            }
        } else {
            gaze_last = (i32::MIN, i32::MIN);
        }
        if inside || zone {
            // Deadline pacing: next frame = previous deadline + period, sleep only the remainder; if we are late
            // (or just woke from the slow cadence) resync to now instead of bursting to catch up.
            next += period(hz);
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now;
            }
        } else if gaze_on {
            std::thread::sleep(gaze_wait(hz, moved));
            next = Instant::now();
        } else {
            nap(100);
            next = Instant::now();
        }
    }
}

// ---- tray + hotkeys ----

fn tray(app: &tauri::App) {
    let build = || -> tauri::Result<()> {
        let toggle_i = MenuItem::with_id(app, "toggle", "Show / Hide", true, None::<&str>)?;
        let settings_i = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
        let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&toggle_i, &settings_i, &quit_i])?;
        let mut b = TrayIconBuilder::with_id("pill").tooltip("Pill").menu(&menu).on_menu_event(|app, ev| {
            match ev.id.as_ref() {
                "toggle" => {
                    let hid = st().hidden & HIDE_USER != 0;
                    set_hidden(app, HIDE_USER, !hid, "tray");
                }
                "settings" => crate::settingswin::open(app, None),
                "quit" => app.exit(0),
                _ => {}
            }
        });
        if let Some(icon) = app.default_window_icon() {
            b = b.icon(icon.clone());
        }
        b.build(app)?;
        Ok(())
    };
    if let Err(e) = build() {
        eprintln!("pill: tray failed: {e}");
    }
}

fn parse_hotkey(spec: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let (mut m, mut vk) = (MOD_NOREPEAT, None);
    for part in spec.split('+').map(|p| p.trim().to_ascii_uppercase()) {
        match part.as_str() {
            "CTRL" | "CONTROL" => m |= MOD_CONTROL,
            "ALT" => m |= MOD_ALT,
            "SHIFT" => m |= MOD_SHIFT,
            "WIN" | "SUPER" | "META" => m |= MOD_WIN,
            "SPACE" => vk = Some(0x20),
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => vk = Some(k.as_bytes()[0] as u32),
            _ => return None,
        }
    }
    vk.map(|v| (m, v))
}

/// Raw RegisterHotKey thread. id 1 = quit (always raw).
fn raw_hotkeys(app: AppHandle, keys: Vec<(i32, String)>) {
    std::thread::spawn(move || unsafe {
        let mut live = 0;
        for (id, spec) in &keys {
            match parse_hotkey(spec) {
                Some((m, vk)) if RegisterHotKey(None, *id, m, vk).is_ok() => live += 1,
                _ => eprintln!("pill: hotkey {spec} could not be registered (taken by another app?)"),
            }
        }
        if live == 0 {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY {
                match msg.wParam.0 {
                    1 => app.exit(0),
                    _ => {}
                }
            }
        }
    });
}

/// Only Ctrl+Alt+Q (quit) lives here and is fixed on purpose; every configurable hotkey, the toggle included, is registered
/// (and re-registered live) by hotkeys.rs.
fn hotkeys(app: &tauri::App) {
    raw_hotkeys(app.handle().clone(), vec![(1, "Ctrl+Alt+Q".to_string())]);
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let Some(w) = app.get_webview_window("pill") else {
        eprintln!("pill: window 'pill' missing");
        return;
    };
    let Ok(raw) = w.hwnd() else { return };
    st().hwnd = raw.0 as isize;
    unsafe {
        let me = me();
        let ex = GetWindowLongPtrW(me, GWL_EXSTYLE);
        SetWindowLongPtrW(me, GWL_EXSTYLE, ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize);
        let _ = SetWindowPos(me, HWND::default(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
    affinity();
    place(&h);
    tray(app);
    hotkeys(app);
    std::thread::spawn(move || cursor_loop(h));
}
