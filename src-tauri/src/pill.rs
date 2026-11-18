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
use crate::ui::host::geom::{self, Align, Dock, IRect};
use crate::ui::host::MonitorInfo;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU8, Ordering};
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
    bar: i32,          // height of the taskbar strip the island stands in, physical px (0 = not in a taskbar)
    bar_scale: f64,    // the monitor scale the island was told that height in (the event carries logical px)
}

static ST: Mutex<St> = Mutex::new(St {
    expanded: false, hidden: 0, shown: true, rect: [0.0; 4], mon: [0; 4], scale: 1.0,
    layout: [0; 4], bar: 0, bar_scale: 1.0,
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
    /// height of the taskbar strip the window stands in (physical px, 0 = a plain edge placement)
    bar: i32,
}

/// Target monitor + frame size for the current settings.
fn layout(expanded: bool) -> Option<Lay> {
    layout_native(expanded)
}

/// The edge the island window hangs from RIGHT NOW (`Dock::code`). It follows `edge.position`, but only when the island view says so (`pill_edge`):
/// the view first slides the body off the old edge, so the window never jumps across the screen under a visible island.
static DOCK: AtomicU8 = AtomicU8::new(DOCK_UNSET);
const DOCK_UNSET: u8 = 255;

static ALIGN: AtomicU8 = AtomicU8::new(DOCK_UNSET);

/// Where along the edge the window stands right now (`edge.align`; the centre for the top edge). Follows the setting only through `pill_edge`.
pub fn align() -> Align {
    if let Some(a) = Align::from_code(ALIGN.load(Ordering::Relaxed)) {
        return a;
    }
    let a = if dock().at_bottom() { Align::from_setting(&crate::settings::get_str("edge.align", "center")) } else { Align::Center };
    ALIGN.store(a.code(), Ordering::Relaxed);
    a
}

pub fn dock() -> Dock {
    if let Some(d) = Dock::from_code(DOCK.load(Ordering::Relaxed)) {
        return d;
    }
    let d = Dock::from_setting(&crate::settings::get_str("edge.position", "top"));
    DOCK.store(d.code(), Ordering::Relaxed);
    d
}

/// The island view slid out and asks for the new edge (`bottom`, and `bar`: inside the taskbar) and side (`align`, the centre unless it is the
/// bottom edge): re-place the window there. `follow`: the view is also changing monitors (`pill:monitor`): the window goes to the monitor the
/// foreground window is on (or back to the `monitor` setting's when `edge.monitor` is no longer active).
pub fn pill_edge(app: AppHandle, bottom: bool, align: Option<String>, bar: Option<bool>, follow: Option<bool>) {
    let a = if bottom { Align::from_setting(align.as_deref().unwrap_or("center")) } else { Align::Center };
    let d = match (bottom, bar.unwrap_or(false)) {
        (false, _) => Dock::Top,
        (true, false) => Dock::Bottom,
        (true, true) => Dock::Taskbar,
    };
    let moved = DOCK.swap(d.code(), Ordering::Relaxed) != d.code();
    let moved = ALIGN.swap(a.code(), Ordering::Relaxed) != a.code() || moved;
    let target = if follow_on() { MON_TARGET.load(Ordering::Relaxed) } else { NO_MON };
    let followed = follow == Some(true) && MON_COMMITTED.swap(target, Ordering::Relaxed) != target;
    if moved || followed {
        on_main(&app, move |a| {
            place(a);
            if followed {
                crate::ui::musicwidget::relayout(a); // the widget stands where the island does
            }
        });
    }
}

/// The window frame on a monitor (physical px): size from the settings, centred plus `offsetX`, hanging from the `dock` edge (`underTaskbar`
/// keeps it inside the work area). In the taskbar (`Dock::Taskbar`, a plain visible bar along the bottom of THIS monitor) the window is the
/// frame plus the bar's height, stands on the monitor's bottom edge over the bar, and its body sits in the bar at the `edge.align` side (see
/// `geom::taskbar_position`); with no such bar it is the plain bottom edge.
fn frame_for(m: &crate::ui::host::MonitorInfo, expanded: bool) -> Lay {
    let sf = m.scale as f64;
    let (ml, mt, mw, mh) = (m.rect.l, m.rect.t, m.rect.r - m.rect.l, m.rect.b - m.rect.t);
    let k = sf * crate::settings::get_f64("size", 100.0).clamp(80.0, 200.0) / 100.0;
    let (fw, fh) = if expanded {
        EXPANDED
    } else if CARD.load(Ordering::Relaxed) {
        (CARD_FRAME.0, CARD_FRAME.1.max(f32::from_bits(CARD_H.load(Ordering::Relaxed)) as f64))
    } else {
        COLLAPSED
    };
    let w = ((fw * k).round() as i32).min(mw);
    let mut h = ((fh * k).round() as i32).min(mh);
    // `offsetX` (-50..50 % of the monitor width, 0 = middle) moves the window centre; the window stays inside the monitor.
    let off = crate::settings::get_f64("offsetX", 0.0).clamp(-50.0, 50.0);
    let dock = dock();
    let bar = if dock == Dock::Taskbar { geom::bottom_bar(m.rect, crate::taskbar::trusted_work(m.rect, m.work, m.scale), m.scale, crate::taskbar::tray_rect(m.rect)) } else { None };
    let (x, y) = match bar {
        Some(b) => {
            h += b.h;
            geom::taskbar_position(align(), m.rect, &b, m.scale, (w, h), geom::phys(geom::ALIGN_PAD, k as f32), geom::phys(pill_ui_core::shell::capsule::COL_MIN_W as f32, k as f32), off as f32)
        }
        None => {
            let under = crate::settings::get_bool("underTaskbar", true);
            geom::dock_position_aligned(dock, align(), m.rect, m.work, under, 0, (w, h), off as f32)
        }
    };
    Lay { x, y, w, h, mon: [ml, mt, ml + mw, mt + mh], scale: sf, bar: bar.map_or(0, |b| b.h) }
}

/// The monitor the window stands on: the one the island followed the foreground window to (`followed`, a `geom::monitor_key`) while it is still
/// there, else the `monitor` setting's ("1".. = list order), else the primary. Pure.
fn pick_for_layout<'a>(mons: &'a [MonitorInfo], want: &str, followed: Option<i64>) -> Option<&'a MonitorInfo> {
    followed
        .and_then(|k| mons.iter().find(|m| geom::monitor_key(&m.rect) == k))
        .or_else(|| want.parse::<usize>().ok().and_then(|i| mons.get(i.saturating_sub(1))))
        .or_else(|| mons.iter().find(|m| m.primary))
        .or_else(|| mons.first())
}

/// Monitors come from the host (the same `monitor` setting, "1".. = list order, else the primary; `edge.monitor` = active overrides it).
fn layout_native(expanded: bool) -> Option<Lay> {
    let mons = crate::ui::host::handle()?.monitors();
    let want = crate::settings::get_str("monitor", "primary");
    let committed = MON_COMMITTED.load(Ordering::Relaxed);
    let m = pick_for_layout(&mons, &want, (committed != NO_MON).then_some(committed))?;
    if committed != NO_MON && geom::monitor_key(&m.rect) != committed {
        MON_COMMITTED.store(NO_MON, Ordering::Relaxed); // that monitor was unplugged: the island is home, and does not jump back when it returns
    }
    Some(frame_for(m, expanded))
}

/// Put the window into its current frame (collapsed/expanded) on the target monitor. UI thread only.
pub fn place(_app: &AppHandle) {
    let exp = st().expanded;
    let Some(l) = layout(exp) else { return };
    let bar_changed = {
        let mut s = st();
        s.mon = l.mon;
        s.scale = l.scale;
        s.layout = [l.x, l.y, l.w, l.h];
        let changed = s.bar != l.bar || (l.bar > 0 && s.bar_scale != l.scale);
        s.bar = l.bar;
        s.bar_scale = l.scale;
        changed
    };
    if let Some(n) = native() {
        n.set_bounds(l.x, l.y, l.w, l.h);
    }
    // the island view learns the bar's height (logical px; 0 = no bar) to centre the folded pill in it and to lift the open one above it
    if bar_changed {
        let _ = bus::emit("pill:taskbar", json!({ "h": l.bar as f64 / l.scale.max(0.25) }));
    }
    crate::taskbar::follow(me(), l.bar > 0);
    crate::taskbar::follow_monitor(follow_on());
    publish_move();
}

/// True while auto-hide has slid the pill away (guard.rs brings it back over the desktop when `autoHideKeepDesktop` is on).
pub fn auto_hidden() -> bool {
    st().hidden & HIDE_AUTO != 0
}

/// Called by the guard poll (500 ms): re-place when monitor / DPI / taskbar / size changed.
pub fn relayout(app: &AppHandle) {
    crate::ui::musicwidget::relayout(app); // the standalone music widget: its monitor / DPI / taskbar may have changed too
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

/// The fullscreen guard (or a peek over it, which the music widget must not follow) is what keeps the island away right now: the standalone
/// music widget (ui/musicwidget) hides with it.
pub fn widget_guard() -> bool {
    st().hidden & HIDE_GUARD != 0 || crate::fspeek::peek() != crate::fspeek::PEEK_NONE
}

static WIDGET_GUARD: AtomicBool = AtomicBool::new(false);

/// Tell the music widget when `widget_guard` changed (event `pill:guard`).
fn sync_guard() {
    let g = widget_guard();
    if WIDGET_GUARD.swap(g, Ordering::Relaxed) != g {
        let _ = bus::emit(crate::ui::musicwidget::GUARD_EVENT, json!({ "hidden": g }));
    }
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
    sync_guard();
}

pub fn set_hidden_bg(app: &AppHandle, mask: u8, on: bool, reason: &'static str) {
    on_main(app, move |a| set_hidden(a, mask, on, reason));
}

/// Enlarge the window, then tell the page to animate.
pub fn expand(app: &AppHandle, tab: Option<String>, via: &str) {
    if st().hidden & HIDE_GUARD != 0 && !peek_over_guard(app, via) {
        return; // never pop up over a fullscreen app by itself: only the hotkey, the top edge and the tray entry may
    }
    set_hidden(app, HIDE_USER | HIDE_AUTO, false, "expand");
    st().expanded = true;
    place(app);
    let _ = bus::emit("pill:expand", json!({ "tab": tab, "via": via }));
}

/// The guard hides the island (fullscreen app / hide list) and `via` is allowed to override it: lift the guard until the island folds.
fn peek_over_guard(app: &AppHandle, via: &str) -> bool {
    let Some(level) = crate::fspeek::peek_level(via, crate::fspeek::why(), crate::settings::get_bool("fullscreenPeek", true)) else { return false };
    st().expanded = true; // the guard poll folds a peek whose island is not expanded
    crate::fspeek::start_peek(level);
    set_hidden(app, HIDE_GUARD, false, "peek");
    true
}

/// From the guard thread: the cursor rested on the top edge of a fullscreen app.
pub fn peek(app: &AppHandle, via: &'static str) {
    on_main(app, move |a| expand(a, None, via));
}

/// A peek is over (the island folded): the guard hides the island again at once if it still wants to.
pub fn end_peek(app: &AppHandle) {
    if crate::fspeek::end_peek() {
        if crate::fspeek::hides(crate::fspeek::why(), crate::fspeek::PEEK_NONE) {
            set_hidden_bg(app, HIDE_GUARD, true, "guard");
        } else {
            sync_guard(); // the peek was all that kept the music widget away
        }
    }
}

pub fn is_expanded() -> bool {
    st().expanded
}

/// The guard (fullscreen / hide list) is what hides the island right now.
pub fn guard_hidden() -> bool {
    st().hidden & HIDE_GUARD != 0
}

/// Where the edge request counts: the island's monitor (physical px), the x range of its body (the whole frame until measured) and whether
/// the island hangs from the bottom edge (then the request is the cursor resting on the monitor's bottom row).
pub fn edge_geometry() -> Option<([i32; 4], (i32, i32), bool)> {
    let s = st();
    if s.mon == [0; 4] || s.layout[2] <= 0 {
        return None;
    }
    let x = if s.rect[2] > 0.0 {
        let l = s.layout[0] + (s.rect[0] * s.scale).round() as i32;
        (l, l + (s.rect[2] * s.scale).round() as i32)
    } else {
        (s.layout[0], s.layout[0] + s.layout[2])
    };
    Some((s.mon, x, dock().at_bottom()))
}

/// Guard poll: if another program hid, minimised or un-topmosted the island, or it ended up off every monitor, put it back.
pub fn keep_alive(app: &AppHandle) {
    if st().hidden != 0 {
        return; // hidden on purpose (tray, fullscreen guard, hide list, AUTO-HIDE...): never fight whatever hid it, or the two undo each other in a loop
    }
    on_main(app, |a| {
        if crate::keepalive::repair(me(), &monitor_rects()) {
            place(a);
        }
        // inside the taskbar the island is topmost together with the bar, which jumps in front of it whenever it is clicked
        if st().bar > 0 {
            crate::taskbar::raise_over_taskbar(me());
        }
    });
}

fn monitor_rects() -> Vec<[i32; 4]> {
    crate::ui::host::handle().map(|h| h.monitors().iter().map(|m| [m.rect.l, m.rect.t, m.rect.r, m.rect.b]).collect()).unwrap_or_default()
}

/// Tray "Open island": whatever hides it (tray toggle, auto-hide, a fullscreen app, the hide list, the OS) is undone and it opens.
fn open_island(app: &AppHandle) {
    set_hidden(app, HIDE_USER | HIDE_AUTO, false, "tray");
    if crate::keepalive::repair(me(), &monitor_rects()) {
        place(app);
    }
    expand(app, None, "tray");
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
    if ["size", "monitor", "edge.monitor", "offsetX", "underTaskbar", "passThrough"].iter().any(|k| patch.contains_key(*k)) {
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
    if dock().at_bottom() {
        return None; // the island hangs from the bottom edge: nothing of it is under the command bar at the top
    }
    let s = st();
    let on_screen = s.hidden == 0 && s.shown && s.rect[3] > 0.0 && s.mon[0] == mon_x && s.mon[1] == mon_y;
    on_screen.then(|| s.layout[1] + ((s.rect[1] + s.rect[3]) * s.scale).round() as i32)
}

// ---- following the active monitor (`edge.monitor` = active) ----
// The foreground hook (`taskbar::on_foreground`) and a display change only POKE: a debounced look (no timer of its own) finds the monitor that
// holds the foreground window and, when it is not the one the island stands on, says so to the island view (`pill:monitor`). The view decides WHEN
// (collapsed, no card, no drag, no recording; reduced motion: at once), slides out through its edge and asks for the window with `pill_edge`
// (`follow`); only then does `MON_COMMITTED` change and `layout_native` re-place the window with the new monitor's own work area, taskbar
// strip, DPI, side and size. The music widget follows through `MonitorSel::Island`.

const NO_MON: i64 = i64::MIN;
/// the monitor (`geom::monitor_key`) the last look found the foreground window on
static MON_TARGET: AtomicI64 = AtomicI64::new(NO_MON);
/// the monitor the view has sent the window to; `layout_native` stands it there while that monitor exists
static MON_COMMITTED: AtomicI64 = AtomicI64::new(NO_MON);
static MON_POKES: AtomicU32 = AtomicU32::new(0);
/// what `pill:monitor` last said
static MON_PENDING: AtomicBool = AtomicBool::new(false);
/// the look waits this long after the last focus change (alt-tab through several windows is one look)
const MON_DEBOUNCE_MS: u64 = 400;

fn follow_on() -> bool {
    crate::settings::get_str("edge.monitor", "primary").eq_ignore_ascii_case("active")
}

/// The monitor the island stands on now (physical px), `None` before it is first placed. The music widget's `MonitorSel::Island`.
pub fn island_monitor() -> Option<IRect> {
    let m = st().mon;
    (m != [0; 4]).then(|| IRect::new(m[0], m[1], m[2], m[3]))
}

/// Does the window have to change monitors? `target` = where the foreground window is, `at` = where the island stands, `committed` = where the
/// view last sent it (all `geom::monitor_key`s), `known` = the monitors there are. Following: yes when the foreground monitor exists and is not
/// this one. Not following (any more): yes while the window is still on a followed monitor, so it goes home. Pure.
fn move_pending(active: bool, target: i64, at: i64, committed: i64, known: &[i64]) -> bool {
    if active {
        target != NO_MON && target != at && known.contains(&target)
    } else {
        committed != NO_MON
    }
}

/// Tells the island view when [`move_pending`] changed (and again at every placement while it holds, so a view that was busy looks again).
fn publish_move() {
    let Some(h) = crate::ui::host::handle() else { return };
    let at = {
        let s = st();
        (s.mon != [0; 4]).then(|| geom::monitor_key(&IRect::new(s.mon[0], s.mon[1], s.mon[2], s.mon[3])))
    };
    if at.is_none() && !MON_PENDING.load(Ordering::Relaxed) {
        return;
    }
    let known: Vec<i64> = h.monitors().iter().map(|m| geom::monitor_key(&m.rect)).collect();
    let pending = at.is_some_and(|at| move_pending(follow_on(), MON_TARGET.load(Ordering::Relaxed), at, MON_COMMITTED.load(Ordering::Relaxed), &known));
    if MON_PENDING.swap(pending, Ordering::Relaxed) != pending || pending {
        let _ = bus::emit("pill:monitor", json!({ "move": pending }));
    }
}

/// The foreground window changed (or the follow switch turned): look again in [`MON_DEBOUNCE_MS`], once, whatever came in between. Any thread.
pub fn monitor_poke() {
    let n = MON_POKES.fetch_add(1, Ordering::Relaxed) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(MON_DEBOUNCE_MS));
        if MON_POKES.load(Ordering::Relaxed) == n {
            monitor_look();
        }
    });
}

/// A display was added, removed or re-arranged.
pub fn monitor_event() {
    if follow_on() || MON_COMMITTED.load(Ordering::Relaxed) != NO_MON {
        monitor_poke();
    }
}

enum Foreground {
    /// one of our own windows (settings, the command bar, the island): the island stays where it is
    Own,
    Rect(IRect),
    /// no window to go by (the desktop, a minimised or hidden one)
    Nothing,
}

fn foreground() -> Foreground {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible};
    unsafe {
        let h = GetForegroundWindow();
        if h.0.is_null() || !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return Foreground::Nothing;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == std::process::id() {
            return Foreground::Own;
        }
        if matches!(crate::taskbar::class_of(h).as_str(), "Progman" | "WorkerW") {
            return Foreground::Nothing; // the desktop covers the whole workspace: it says nothing about where the user is
        }
        let mut r = windows::Win32::Foundation::RECT::default();
        if GetWindowRect(h, &mut r).is_err() {
            return Foreground::Nothing;
        }
        Foreground::Rect(IRect::new(r.left, r.top, r.right, r.bottom))
    }
}

/// The debounced look, off the UI thread: where is the foreground window, and does the island have to go there?
fn monitor_look() {
    use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    let Some(h) = crate::ui::host::handle() else { return };
    unsafe {
        SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2); // physical px, like the monitors' rects
    }
    if follow_on() {
        let mons = h.monitors();
        let fg = match foreground() {
            Foreground::Own => return,
            Foreground::Rect(r) => Some(r),
            Foreground::Nothing => None,
        };
        let cursor = {
            let mut p = windows::Win32::Foundation::POINT::default();
            unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p) }.is_ok().then_some((p.x, p.y))
        };
        let rects: Vec<IRect> = mons.iter().map(|m| m.rect).collect();
        let primary = mons.iter().position(|m| m.primary).unwrap_or(0);
        let i = geom::follow_monitor(&rects, fg, cursor, primary);
        if let Some(m) = mons.get(i) {
            MON_TARGET.store(geom::monitor_key(&m.rect), Ordering::Relaxed);
        }
    }
    publish_move();
}

#[cfg(test)]
mod follow_tests {
    use super::*;

    fn mon(l: i32, t: i32, r: i32, b: i32, primary: bool) -> MonitorInfo {
        let rect = IRect::new(l, t, r, b);
        MonitorInfo { rect, work: rect, primary, scale: 1.0, refresh_hz: 60 }
    }
    fn two() -> Vec<MonitorInfo> {
        vec![mon(0, 0, 1920, 1080, true), mon(1920, 0, 3840, 1080, false)]
    }
    fn key(m: &MonitorInfo) -> i64 {
        geom::monitor_key(&m.rect)
    }

    #[test]
    fn the_window_stands_on_the_followed_monitor_while_it_exists_else_where_the_setting_says() {
        let m = two();
        assert_eq!(pick_for_layout(&m, "primary", None).map(key), Some(key(&m[0])));
        assert_eq!(pick_for_layout(&m, "2", None).map(key), Some(key(&m[1])), "an explicit choice stays the default");
        assert_eq!(pick_for_layout(&m, "primary", Some(key(&m[1]))).map(key), Some(key(&m[1])), "followed beats the setting");
        assert_eq!(pick_for_layout(&m, "2", Some(key(&m[0]))).map(key), Some(key(&m[0])), "even an explicit other monitor");
        assert_eq!(pick_for_layout(&m[..1], "primary", Some(key(&m[1]))).map(key), Some(key(&m[0])), "unplugged: back to the primary");
        assert_eq!(pick_for_layout(&m[..1], "2", Some(key(&m[1]))).map(key), Some(key(&m[0])), "and an explicit one that is not there is the primary too");
        assert!(pick_for_layout(&[], "primary", None).is_none());
    }

    #[test]
    fn a_move_is_pending_only_for_a_monitor_that_exists_and_is_not_this_one() {
        let m = two();
        let (a, b) = (key(&m[0]), key(&m[1]));
        let known = [a, b];
        assert!(move_pending(true, b, a, NO_MON, &known), "the foreground window is on the other monitor");
        assert!(!move_pending(true, a, a, NO_MON, &known), "already here");
        assert!(!move_pending(true, NO_MON, a, NO_MON, &known), "no look yet");
        assert!(!move_pending(true, 12345, a, NO_MON, &known), "a monitor that was unplugged is no target: no slide loop");
        assert!(move_pending(false, b, a, b, &known), "following was switched off while the island stands on a followed monitor: home");
        assert!(!move_pending(false, b, a, NO_MON, &known), "not following, never moved: nothing to do");
    }
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
        end_peek(&app);
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
/// The frame height (css px) the card on screen asks for when it is taller than `CARD_FRAME` (f32 bits; 0 = the plain card frame).
static CARD_H: AtomicU32 = AtomicU32::new(0);

/// `on`: an event card is up. `h`: the frame height it needs (css px), grown to fit a long question (the frame never shrinks while the card is up).
pub fn pill_cards(app: AppHandle, on: bool, h: Option<f64>) {
    let want = if on { h.unwrap_or(0.0).max(0.0) as f32 } else { 0.0 };
    let grew = CARD_H.swap(want.to_bits(), Ordering::Relaxed) != want.to_bits();
    if CARD.swap(on, Ordering::Relaxed) != on || grew {
        on_main(&app, place);
    }
}

/// The tallest an event card may be (css px): 60 % of the height of the monitor the island is on. None while no monitor is known.
pub fn card_max_h() -> Option<f32> {
    let (mh, sc) = {
        let s = st();
        ((s.mon[3] - s.mon[1]) as f64, s.scale)
    };
    let k = sc * crate::settings::get_f64("size", 100.0).clamp(80.0, 200.0) / 100.0;
    (mh > 0.0 && k > 0.0).then(|| ((mh * 0.6 / k) as f32).clamp(200.0, 900.0))
}

/// The pets' look-at-the-cursor feed comes from the host's proximity engine; nothing to switch here.
pub fn pill_set_gaze(_on: bool) {}

// ---- tray + hotkeys ----

fn tray(app: &tauri::App) {
    let build = || -> tauri::Result<()> {
        let ru = crate::ui::capture_view::windows::is_russian();
        let t = |ru_text: &'static str, en: &'static str| if ru { ru_text } else { en };
        let open_i = MenuItem::with_id(app, "open", t("Открыть остров", "Open island"), true, None::<&str>)?;
        let toggle_i = MenuItem::with_id(app, "toggle", t("Показать / скрыть", "Show / Hide"), true, None::<&str>)?;
        let settings_i = MenuItem::with_id(app, "settings", t("Настройки", "Settings"), true, None::<&str>)?;
        let quit_i = MenuItem::with_id(app, "quit", t("Выход", "Quit"), true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open_i, &toggle_i, &settings_i, &quit_i])?;
        let mut b = TrayIconBuilder::with_id("pill").tooltip("Pill").menu(&menu).on_menu_event(|app, ev| {
            match ev.id.as_ref() {
                "open" => open_island(app),
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
