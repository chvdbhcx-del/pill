// The Windows taskbar as the island inside it needs it (`edge.position` = taskbar): where its notification area is (the island never stands on
// the tray), and keeping the island above the bar. The taskbar is itself topmost and puts itself on top of the other topmost windows whenever it
// is clicked, so the island (a topmost overlay positioned over the bar, never a child of `Shell_TrayWnd`) raises itself back:
//  * at once, from an `EVENT_SYSTEM_FOREGROUND` hook that wakes only when the foreground window changes (zero idle cost, no timer);
//  * and from the guard poll (500 ms, already running) as a safety net for a raise that gave no focus event.
// The same hook also tells the island that follows the active monitor (`edge.monitor` = active) that the foreground window changed (`pill::monitor_poke`).
// Neither activates the island or takes the focus: `SWP_NOACTIVATE`, no move, no resize.
// The same hook also tells the fullscreen guard (`guard.rs`, via `set_listener`) that the foreground window changed, and a second hook, scoped
// to the foreground window's thread and moved on every foreground change, tells it when that window moved or resized (F11, a video going
// fullscreen): the guard re-checks at once instead of waiting for its slow poll.
// The standalone music widget (`media.widget.edge` = taskbar) stands in the same bar and rides the same hook: it is the second window the hook raises.
use crate::ui::host::geom::{self, Align, IRect, IslandBar, WidgetAsk};
use crate::ui::host::MonitorInfo;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, Ordering};
use std::sync::OnceLock;
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, SetWindowPos,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_FOREGROUND, GW_HWNDPREV, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
    SWP_NOSIZE, WINEVENT_OUTOFCONTEXT,
};

/// The window classes of the primary and of a secondary monitor's taskbar.
pub fn is_taskbar_class(c: &str) -> bool {
    matches!(c, "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// Is a taskbar among the windows above the island? `above` = the classes of the windows in front of it, nearest first. Pure.
pub fn covered_by_taskbar<'a>(above: impl IntoIterator<Item = &'a str>) -> bool {
    above.into_iter().any(is_taskbar_class)
}

pub(crate) fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn rect_of(h: HWND) -> Option<IRect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok()?;
    Some(IRect::new(r.left, r.top, r.right, r.bottom))
}

/// Every taskbar window: the primary one and the secondary ones (one per other monitor).
fn taskbars() -> Vec<HWND> {
    let mut v = Vec::new();
    unsafe {
        if let Ok(h) = FindWindowW(w!("Shell_TrayWnd"), None) {
            v.push(h);
        }
        let mut prev = HWND::default();
        while let Ok(h) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) {
            if h.0.is_null() || v.len() > 16 {
                break;
            }
            v.push(h);
            prev = h;
        }
    }
    v
}

/// The notification area (`TrayNotifyWnd`: the tray icons and the clock) of the taskbar that lies along the bottom of `mon` (physical px).
/// `None` when that bar has none (a secondary bar of some Windows builds) or it is not visible: the island then has the whole bar.
pub fn tray_rect(mon: IRect) -> Option<IRect> {
    for bar in taskbars() {
        let Some(r) = rect_of(bar) else { continue };
        let on_this_monitor = r.l >= mon.l - 2 && r.r <= mon.r + 2 && (r.b - mon.b).abs() <= 2;
        if !on_this_monitor {
            continue;
        }
        let tray = unsafe { FindWindowExW(bar, None, w!("TrayNotifyWnd"), None) }.ok().filter(|t| !t.0.is_null() && unsafe { IsWindowVisible(*t) }.as_bool());
        return tray.and_then(rect_of);
    }
    None
}

/// The taskbar window whose strip lies along the bottom of `mon` (physical px), if there is one.
fn bar_rect_on(mon: IRect) -> Option<IRect> {
    taskbars().into_iter().filter_map(rect_of).find(|r| r.l >= mon.l - 2 && r.r <= mon.r + 2 && (r.b - mon.b).abs() <= 2)
}

/// Why the strip the work area reserves at the bottom of `mon` is NOT a bottom taskbar the island can stand in (`None` = it is one, or there is no
/// strip at all and the plain bottom edge applies anyway). `bar` = the rectangle of the taskbar window along this monitor's bottom. Pure. A shell
/// replacement or tweaker that reserves a strip without a real `Shell_TrayWnd` there, a floating / inset bar, a bar that is mid-slide (auto-hide)
/// or a work area that does not match the bar all end up here, and the island then takes the plain bottom edge.
pub fn implausible_bar(mon: IRect, work: IRect, scale: f32, bar: Option<IRect>) -> Option<&'static str> {
    if mon.b - work.b <= 0 {
        return None;
    }
    let Some(b) = bar else { return Some("no taskbar window along the bottom of this monitor") };
    let tol = ((4.0 * scale).round() as i32).max(3);
    if b.r - b.l < mon.w() / 2 {
        return Some("the taskbar window is not a full-width strip");
    }
    if b.h() < geom::phys(geom::BAR_MIN_H, scale) || b.h() > mon.h() / 3 {
        return Some("the taskbar window has an implausible height");
    }
    if (b.t - work.b).abs() > tol {
        return Some("the taskbar window and the work area disagree (floating or sliding bar)");
    }
    None
}

static DISTRUSTED: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

/// `work` when the bottom strip it leaves is a real taskbar, else the whole monitor (so `geom::bottom_bar` / `widget_bar` find no bar and the
/// island / widget take the plain bottom edge). Logs one line per change of verdict.
pub fn trusted_work(mon: IRect, work: IRect, scale: f32) -> IRect {
    let why = implausible_bar(mon, work, scale, bar_rect_on(mon));
    let mut last = DISTRUSTED.lock().unwrap_or_else(|e| e.into_inner());
    if *last != why {
        *last = why;
        if let Some(w) = why {
            eprintln!("taskbar: not used as the island's bar ({w}); using the plain bottom edge");
        }
    }
    if why.is_some() { mon } else { work }
}

/// Puts the island back above a taskbar that got in front of it. True when it had to. No activation, no move.
pub fn raise_over_taskbar(island: HWND) -> bool {
    if island.0.is_null() {
        return false;
    }
    let mut classes = Vec::new();
    let mut w = island;
    for _ in 0..32 {
        match unsafe { GetWindow(w, GW_HWNDPREV) } {
            Ok(p) if !p.0.is_null() => {
                classes.push(class_of(p));
                w = p;
            }
            _ => break,
        }
    }
    let covered = covered_by_taskbar(classes.iter().map(String::as_str));
    if covered {
        // async: the island belongs to the host thread, never wait for it from the thread that runs the hook
        unsafe {
            let _ = SetWindowPos(island, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS);
        }
    }
    covered
}

static ISLAND: AtomicIsize = AtomicIsize::new(0);
static WIDGET: AtomicIsize = AtomicIsize::new(0);
static FOLLOW: AtomicBool = AtomicBool::new(false);
static FOLLOW_WIDGET: AtomicBool = AtomicBool::new(false);
static HOOKED: AtomicBool = AtomicBool::new(false);
static FOLLOW_MON: AtomicBool = AtomicBool::new(false);
/// The foreground window the location hook watches, and that hook (0 = none).
static WATCHED: AtomicIsize = AtomicIsize::new(0);
static LOC_HOOK: AtomicIsize = AtomicIsize::new(0);

/// What a listener is told: the foreground window changed / the foreground window moved or resized.
pub const EV_FOREGROUND: u8 = 1;
pub const EV_LOCATION: u8 = 2;
static LISTENER: OnceLock<fn(u8)> = OnceLock::new();

/// Registers the one listener of the foreground events (the fullscreen guard). It runs on the UI thread inside the hook: keep it to a flag and a wake-up.
pub fn set_listener(f: fn(u8)) {
    let _ = LISTENER.set(f);
}

fn notify(ev: u8) {
    if let Some(f) = LISTENER.get() {
        f(ev);
    }
}

/// Points the location hook at `fg`'s thread (events of that one thread only: the cheapest scope that still sees its window change size).
fn watch(fg: HWND) {
    if WATCHED.swap(fg.0 as isize, Ordering::Relaxed) == fg.0 as isize || LISTENER.get().is_none() {
        return;
    }
    let old = LOC_HOOK.swap(0, Ordering::Relaxed);
    if old != 0 {
        let _ = unsafe { UnhookWinEvent(HWINEVENTHOOK(old as *mut c_void)) };
    }
    if fg.0.is_null() {
        return;
    }
    let tid = unsafe { GetWindowThreadProcessId(fg, None) };
    if tid == 0 {
        return;
    }
    let h = unsafe { SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(on_location), 0, tid, WINEVENT_OUTOFCONTEXT) };
    LOC_HOOK.store(h.0 as isize, Ordering::Relaxed); // a failed hook (null) leaves the guard's poll as the only net
}

unsafe extern "system" fn on_location(_hook: HWINEVENTHOOK, _event: u32, hwnd: HWND, object: i32, _child: i32, _thread: u32, _ms: u32) {
    // OBJID_WINDOW (0) of the watched window only: not its carets, cursors or child controls
    if object == 0 && hwnd.0 as isize == WATCHED.load(Ordering::Relaxed) {
        notify(EV_LOCATION);
    }
}

unsafe extern "system" fn on_foreground(_hook: HWINEVENTHOOK, _event: u32, hwnd: HWND, _object: i32, _child: i32, _thread: u32, _ms: u32) {
    watch(hwnd);
    notify(EV_FOREGROUND);
    if FOLLOW_MON.load(Ordering::Relaxed) {
        crate::pill::monitor_poke();
    }
    let (island, widget) = (FOLLOW.load(Ordering::Relaxed), FOLLOW_WIDGET.load(Ordering::Relaxed));
    if (island || widget) && is_taskbar_class(&class_of(hwnd)) {
        if island {
            raise_over_taskbar(HWND(ISLAND.load(Ordering::Relaxed) as *mut c_void));
        }
        if widget {
            raise_over_taskbar(HWND(WIDGET.load(Ordering::Relaxed) as *mut c_void));
        }
    }
}

/// Installs the foreground hook once, from the thread that calls this (the UI thread: the out-of-context callback is delivered through its
/// message loop).
fn hook() {
    if !HOOKED.swap(true, Ordering::Relaxed) {
        let h = unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT) };
        if h.0.is_null() {
            HOOKED.store(false, Ordering::Relaxed); // the guard poll still keeps the island on top
        } else {
            watch(unsafe { GetForegroundWindow() });
        }
    }
}

/// The island stands in the taskbar (`on`) or not any more. The raise does nothing while `on` is false; the hook itself is installed with the
/// first island window either way (the fullscreen guard listens on it).
pub fn follow(island: HWND, on: bool) {
    FOLLOW.store(on && !island.0.is_null(), Ordering::Relaxed);
    if island.0.is_null() {
        return;
    }
    ISLAND.store(island.0 as isize, Ordering::Relaxed);
    hook();
}

/// The island follows the active monitor (`on`) or not any more. Call from the UI thread (the hook is installed from its message loop). Turning it on
/// asks for a first look at once (`pill::monitor_poke`): nothing else would tell the island where the foreground window is until it changes.
pub fn follow_monitor(on: bool) {
    if FOLLOW_MON.swap(on, Ordering::Relaxed) == on {
        return;
    }
    if on {
        hook();
    }
    crate::pill::monitor_poke(); // on: look now; off: the island goes home
}

/// The same for the music widget's window (`on` while it stands in the bar). Call from the UI thread.
pub fn follow_widget(widget: HWND, on: bool) {
    FOLLOW_WIDGET.store(on && !widget.0.is_null(), Ordering::Relaxed);
    if !on || widget.0.is_null() {
        return;
    }
    WIDGET.store(widget.0 as isize, Ordering::Relaxed);
    hook();
}

/// The widget keeps itself on top like the island: `raise_over_taskbar` when it is on screen (a hidden window is none of the bar's business).
pub fn keep_widget_on_top(widget: HWND) {
    if !widget.0.is_null() && unsafe { IsWindowVisible(widget) }.as_bool() {
        raise_over_taskbar(widget);
    }
}

/// The folded island's real width in the bar (window px; 0 = it has not said, or it is not in a bar), published by the island's view when its capsule
/// grows or shrinks (the live row, a notification's text, the clock): the width the music widget keeps clear of.
static ISLAND_W: AtomicI32 = AtomicI32::new(0);

/// The island says how wide its folded capsule is now (0 = it left the bar). Only a change wakes the widget (`musicwidget::island_width_changed`):
/// no timer, nothing while the width stays.
pub fn publish_island_w(w: i32) {
    if ISLAND_W.swap(w.max(0), Ordering::Relaxed) != w.max(0) {
        crate::ui::musicwidget::island_width_changed();
    }
}

/// The width the widget keeps clear of: the island's published one, or (before it has said, or off a bar) the old ceiling `ISLAND_BAR_W` x `size`. Pure.
pub fn island_slot_w(published: i32, size_pct: f64, scale: f32) -> i32 {
    if published > 0 {
        published
    } else {
        geom::phys((geom::ISLAND_BAR_W as f64 * size_pct.clamp(80.0, 200.0) / 100.0) as f32, scale)
    }
}

/// Where the island stands in the bar, as the widget must keep clear of it (`None` = the island is not in the taskbar). The side and the offset
/// are read from the settings; the width is the island's real one ([`publish_island_w`]). `monitor` is the same setting for both, so they are in the same bar.
pub fn island_bar(scale: f32) -> Option<IslandBar> {
    use crate::settings::{get_f64, get_str};
    if !get_str("edge.position", "top").eq_ignore_ascii_case("taskbar") {
        return None;
    }
    Some(IslandBar {
        align: Align::from_setting(&get_str("edge.align", "center")),
        offset_pct: get_f64("offsetX", 0.0).clamp(-50.0, 50.0) as f32,
        w: island_slot_w(ISLAND_W.load(Ordering::Relaxed), get_f64("size", 100.0), scale),
    })
}

/// The widget window of `size` (physical) on `mon`: its position, and its bar and slot when it stands in the bar ([`geom::widget_place`] with the
/// real tray and the island's settings).
pub fn widget_place(mon: &MonitorInfo, ask: &WidgetAsk, size: (i32, i32)) -> ((i32, i32), Option<(geom::Bar, geom::BarSlot)>) {
    // the slot asked of the bar is as wide as the folded capsule at the current size
    let ask = WidgetAsk { body_w: ask.body_w * crate::ui::musicwidget::size_k(), ..*ask };
    geom::widget_place(mon.rect, trusted_work(mon.rect, mon.work, mon.scale), mon.scale, tray_rect(mon.rect), island_bar(mon.scale), &ask, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_taskbar_classes_cover_the_island() {
        assert!(is_taskbar_class("Shell_TrayWnd") && is_taskbar_class("Shell_SecondaryTrayWnd"));
        for c in ["Progman", "WorkerW", "TrayNotifyWnd", "Chrome_WidgetWin_1", "", "shell_traywnd"] {
            assert!(!is_taskbar_class(c), "{c:?}");
        }
    }

    #[test]
    fn a_taskbar_anywhere_above_the_island_means_it_is_covered() {
        assert!(!covered_by_taskbar([]), "the island is at the top");
        assert!(!covered_by_taskbar(["Chrome_WidgetWin_1", "Notepad"]), "other topmost windows are none of our business");
        assert!(covered_by_taskbar(["Shell_TrayWnd"]));
        assert!(covered_by_taskbar(["Notepad", "Shell_SecondaryTrayWnd", "Chrome_WidgetWin_1"]));
    }

    #[test]
    fn the_island_slot_is_its_published_width_or_the_old_ceiling_until_it_says() {
        assert_eq!(island_slot_w(173, 100.0, 1.0), 173);
        assert_eq!(island_slot_w(173, 200.0, 1.5), 173, "the published width is physical already: neither the size nor the DPI touch it");
        assert_eq!(island_slot_w(0, 100.0, 1.0), 300);
        assert_eq!(island_slot_w(0, 150.0, 2.0), 900, "the ceiling is 300 logical px x size x DPI");
        assert_eq!(island_slot_w(-5, 100.0, 1.0), 300, "garbage counts as not said");
    }

    #[test]
    fn a_real_bottom_bar_is_trusted_at_every_dpi_and_a_dubious_one_is_not() {
        for (s, h) in [(1.0f32, 48), (1.25, 60), (1.5, 72), (2.0, 96)] {
            let (mon, work) = (IRect::new(0, 0, 2560, 1440), IRect::new(0, 0, 2560, 1440 - h));
            let bar = IRect::new(0, 1440 - h, 2560, 1440);
            assert_eq!(implausible_bar(mon, work, s, Some(bar)), None, "{s}");
            assert!(implausible_bar(mon, work, s, None).is_some(), "no taskbar window: a shell replacement reserved the strip");
            assert!(implausible_bar(mon, work, s, Some(IRect::new(800, 1440 - h, 1760, 1440))).is_some(), "a floating / inset bar");
            assert!(implausible_bar(mon, work, s, Some(IRect::new(0, 1440 - h + 30, 2560, 1440 + 30))).is_some(), "a bar that is sliding out");
            assert!(implausible_bar(mon, work, s, Some(IRect::new(0, 1440 - 6, 2560, 1440))).is_some(), "a sliver is no bar");
        }
        let mon = IRect::new(0, 0, 2560, 1440);
        assert_eq!(implausible_bar(mon, mon, 1.0, None), None, "no strip reserved (auto-hide, no bar): the plain bottom edge applies already");
        // a secondary monitor left of the primary, negative coordinates
        let m2 = IRect::new(-1920, -300, 0, 780);
        assert_eq!(implausible_bar(m2, IRect::new(-1920, -300, 0, 732), 1.0, Some(IRect::new(-1920, 732, 0, 780))), None);
    }

    #[test]
    fn there_is_nothing_to_raise_without_a_window() {
        assert!(!raise_over_taskbar(HWND::default()));
        follow(HWND::default(), true); // no window: the hook is not installed, nothing is followed
        follow_widget(HWND::default(), true);
        keep_widget_on_top(HWND::default());
        assert!(!FOLLOW.load(Ordering::Relaxed) && !FOLLOW_WIDGET.load(Ordering::Relaxed) && !HOOKED.load(Ordering::Relaxed));
    }
}
