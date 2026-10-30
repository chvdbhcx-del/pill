// The pill window: placement at the top center, collapsed/expanded frames, visibility (guard / auto-hide / tray), tray icon
// and hotkeys. Owner: pill agent (lane L1). The window itself is the native Overlay of ui/island.
//
// Window model: one transparent always-on-top window, never resized while the view animates.
// Expand: Rust enlarges the window -> emits `pill:expand` -> the view animates. Collapse: the view animates, then calls
// `pill_set_state("collapsed")` and Rust shrinks the window. The cursor (hover / top-edge / click-through) belongs to the
// host's proximity engine (ui/host/proximity.rs).
use crate::ui::bus;
use serde_json::{json, Map, Value};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::AppHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN};
use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, SetWindowDisplayAffinity, MSG, WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_HOTKEY};

// Frame sizes in logical px at 100 % user size (ui-core mirrors them for the island layout).
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
    expanded: bool,    // OS window is in the expanded frame
    hidden: u8,        // HIDE_* bits
    shown: bool,       // the OS window is currently visible
    rect: [f64; 4],    // pill body rect reported by the page, css px relative to the window
    mon: [i32; 4],     // target monitor rect, physical px
    scale: f64,        // target monitor scale factor (css px -> physical px)
    layout: [i32; 4],  // last placed x, y, w, h
}

static ST: Mutex<St> = Mutex::new(St {
    expanded: false, hidden: 0, shown: true, rect: [0.0; 4], mon: [0; 4], scale: 1.0,
    layout: [0; 4],
});

fn st() -> MutexGuard<'static, St> {
    ST.lock().unwrap_or_else(|e| e.into_inner())
}

fn me() -> HWND {
    native().map_or(HWND::default(), |n| HWND(n.hwnd() as *mut c_void))
}

// ---- the island window: the Overlay of `ui::island`, driven through the PillWin seam (placement, show / hide, capture affinity) ----
static NATIVE: std::sync::OnceLock<Box<dyn crate::ui::host::pillwin::PillWin>> = std::sync::OnceLock::new();

fn native() -> Option<&'static dyn crate::ui::host::pillwin::PillWin> {
    NATIVE.get().map(|b| b.as_ref())
}

/// `ui::island::start` hands the Overlay over right after queueing it.
pub fn attach_native(w: Box<dyn crate::ui::host::pillwin::PillWin>) {
    let _ = NATIVE.set(w);
}

/// The Overlay exists now (its HWND is known): first placement and the capture affinity.
pub fn native_ready(app: &AppHandle) {
    on_main(app, |a| {
        affinity();
        place(a);
    });
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
fn layout(expanded: bool) -> Option<Lay> {
    layout_native(expanded)
}

/// The window frame on a monitor (physical px): size from the settings, centred plus `offsetX`, `top` = where the island hangs.
fn frame_for(ml: i32, mt: i32, mw: i32, mh: i32, top: i32, sf: f64, expanded: bool) -> Lay {
    let k = sf * crate::settings::get_f64("size", 100.0).clamp(80.0, 200.0) / 100.0;
    let (fw, fh) = if expanded { EXPANDED } else if CARD.load(Ordering::Relaxed) { CARD_FRAME } else { COLLAPSED };
    let w = ((fw * k).round() as i32).min(mw);
    let h = (fh * k).round() as i32;
    // `offsetX` (-50..50 % of the monitor width, 0 = middle) moves the window centre; the window stays inside the monitor.
    let off = crate::settings::get_f64("offsetX", 0.0).clamp(-50.0, 50.0);
    let cx = ml as f64 + mw as f64 / 2.0 + off / 100.0 * mw as f64;
    let x = ((cx - w as f64 / 2.0).round() as i32).clamp(ml, (ml + mw - w).max(ml));
    Lay { x, y: top, w, h, mon: [ml, mt, ml + mw, mt + mh], scale: sf }
}

/// Monitors come from the host (the same `monitor` setting, "1".. = list order, else the primary).
fn layout_native(expanded: bool) -> Option<Lay> {
    let mons = crate::ui::host::handle()?.monitors();
    let want = crate::settings::get_str("monitor", "primary");
    let m = want
        .parse::<usize>()
        .ok()
        .and_then(|i| mons.get(i.saturating_sub(1)))
        .or_else(|| mons.iter().find(|m| m.primary))
        .or_else(|| mons.first())?;
    let (ml, mt, mw, mh) = (m.rect.l, m.rect.t, m.rect.r - m.rect.l, m.rect.b - m.rect.t);
    let top = if crate::settings::get_bool("underTaskbar", true) { m.work.t.max(mt) } else { mt };
    Some(frame_for(ml, mt, mw, mh, top, m.scale as f64, expanded))
}

/// Put the window into its current frame (collapsed/expanded) on the target monitor. UI thread only.
pub fn place(_app: &AppHandle) {
    let exp = st().expanded;
    let Some(l) = layout(exp) else { return };
    {
        let mut s = st();
        s.mon = l.mon;
        s.scale = l.scale;
        s.layout = [l.x, l.y, l.w, l.h];
    }
    if let Some(n) = native() {
        n.set_bounds(l.x, l.y, l.w, l.h);
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
        if let Some(l) = layout(exp) {
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
        if let Some(n) = native() {
            n.set_visible(true);
        }
        place(app); // re-assert topmost + re-place after being hidden
        let _ = bus::emit("pill:show", json!({ "reason": reason }));
    } else {
        let _ = bus::emit("pill:hide", json!({ "reason": reason }));
        // auto-hide keeps the Overlay "shown" (the body slid out of its frame): the host's top-edge dwell must keep running to bring it back
        if let Some(n) = native() {
            n.set_visible(st().hidden & (HIDE_USER | HIDE_GUARD) == 0);
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
    let _ = bus::emit("pill:expand", json!({ "tab": tab, "via": via }));
}

pub fn toggle(app: &AppHandle) {
    if st().expanded {
        let _ = bus::emit("pill:collapse", ());
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
    if me().0.is_null() {
        return; // the native Overlay is not created yet (`native_ready` calls this again)
    }
    let a = if crate::settings::get_bool("showInCapture", true) { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE };
    unsafe {
        let _ = SetWindowDisplayAffinity(me(), a);
    }
}

/// Physical y just below the pill's body when it is on screen on the monitor at `(mon_x, mon_y)` (None while hidden / not measured):
/// the command bar hangs under it instead of covering it.
pub fn body_bottom(mon_x: i32, mon_y: i32) -> Option<i32> {
    let s = st();
    let on_screen = s.hidden == 0 && s.shown && s.rect[3] > 0.0 && s.mon[0] == mon_x && s.mon[1] == mon_y;
    on_screen.then(|| s.layout[1] + ((s.rect[1] + s.rect[3]) * s.scale).round() as i32)
}

// ---- commands ----

pub fn pill_rect(x: f64, y: f64, w: f64, h: f64) {
    st().rect = [x, y, w, h];
}

pub fn pill_set_state(app: AppHandle, state: String, tab: Option<String>, via: Option<String>) {
    if state == "expanded" {
        expand(&app, tab, via.as_deref().unwrap_or("page"));
    } else {
        pill_activate(app.clone(), false);
        st().expanded = false;
        place(&app);
    }
}

pub fn pill_toggle(app: AppHandle) {
    toggle(&app);
}

/// The page slid away (on=true: hide the window) or an event/click wants it back (on=false).
pub fn pill_auto_hide(app: AppHandle, on: bool) {
    set_hidden(&app, HIDE_AUTO, on, if on { "auto" } else { "event" });
}

/// Text fields need keyboard focus: the native view takes / gives it back itself (`Cx::set_focus`), so there is nothing to do here.
pub fn pill_activate(_app: AppHandle, _on: bool) {}

/// Monitor names for the settings tab ("1".."n" map to this order).
pub fn pill_monitors(_app: AppHandle) -> Vec<String> {
    let ms = crate::ui::host::handle().map(|h| h.monitors()).unwrap_or_default();
    ms.iter().enumerate().map(|(i, m)| format!("{} {}x{}", i + 1, m.rect.r - m.rect.l, m.rect.b - m.rect.t)).collect()
}

/// An event card hangs below the folded pill: while one is shown the folded frame is taller so a long notification fits.
static CARD: AtomicBool = AtomicBool::new(false);
const CARD_FRAME: (f64, f64) = (720.0, 200.0);

pub fn pill_cards(app: AppHandle, on: bool) {
    if CARD.swap(on, Ordering::Relaxed) != on {
        on_main(&app, place);
    }
}

/// The pets' look-at-the-cursor feed comes from the host's proximity engine; nothing to switch here.
pub fn pill_set_gaze(_on: bool) {}

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
    affinity();
    place(&h);
    tray(app);
    hotkeys(app);
}
