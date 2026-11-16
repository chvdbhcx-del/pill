//! The music widget: the island's player as its OWN small window, so it can be parked apart from the island (the bottom left corner next to the
//! taskbar while the pill stays in the middle, or the top, or any side of either edge). Settings (Media and sound page):
//!
//! | key | meaning |
//! |---|---|
//! | `media.widget` | off (default: nothing changes, no window exists) / on |
//! | `media.widget.edge` | `top` / `bottom` / `taskbar` (default `bottom`; `taskbar` = INSIDE the bottom taskbar strip, see below) |
//! | `media.widget.align` | `left` / `center` / `right` along that edge (default `left`) |
//! | `media.widget.offset` | -50..50 % of the monitor width, only for `center` (like `offsetX`) |
//!
//! * One Overlay window (`ui::host`, `Placement::Docked`), never resized: topmost, tool window, no-activate, per-pixel alpha, click-through
//!   outside the body. It is as big as the largest size needs (`logic::WIN_REAL`, 688 x 448 logical px); the body stands in it by `Stand::origin`.
//! * Size: the widget follows the island's `size` (80..200 %, `logic::size_k`): it is laid out in base px at 100 % and drawn scaled by the factor, so
//!   the capsule, the card, the fonts, the cover, the paddings, the radii and the shadow all scale, and so do the hit region and the placement
//!   numbers. A change of `size` re-targets the view's factor (a Flow: Reduce motion lands at once); the window is not re-created or moved, the
//!   body grows from its corner / side. In the taskbar the capsule is still capped by the bar's height (`Stand::fold_h`) and the slot asked of the
//!   bar is as wide as the scaled capsule (`taskbar::widget_place`). It uses the island's monitor (`monitor`; with `edge.monitor` = active the monitor the
//!   island stands on right now, `MonitorSel::Island`: the island moves first, then `pill_edge` re-places the widget with it), the work area (`underTaskbar`) and `showInCapture`.
//! * `view.rs` draws it and `logic.rs` decides everything (when it shows, the geometry, what a click hits); both are fed by the SAME backend
//!   the island's player uses (`media:changed`, `media_state`, `media_control`, `media_seek`): there is no second media backend.
//! * De-duplication: while `media.widget` is on, the island does not show the player (`ui-core` `live::media` gives no row and no edge
//!   reaction, `panel::a::home` has no player part and `command::tab_on` ignores it). Turning it off gives all of it back at once.
//! * The window exists only while the setting is on. Its placement is part of its spec, so a change of the placement settings re-creates it
//!   (`sync`); a change of the monitors, DPI or the taskbar re-places it (`relayout`, from the guard poll like the island's).
//! * In the taskbar (`edge` = `taskbar`): the window is taller ([`logic::WIN_BAR`]), its floor is the monitor's bottom (over the bar), the folded
//!   capsule is a compact one that fits the strip (its own prev / play / next: hovering does not open it, a click on its cover or title opens the card, which floats above the bar) (`Placement::InBar` -> `geom::widget_place`). Where the
//!   monitor has no bar it can live in (top / left / right / auto-hidden / too tall) it is the plain bottom placement, like the island. When the
//!   island stands in the same bar the widget keeps clear of it (`geom::widget_bar_slot`: the other side, from the settings alone). The window is
//!   raised over the bar like the island (`taskbar::follow_widget`, `keep_widget_on_top` from the guard poll). `SLOT_EVENT` tells the view which bar
//!   and side the window got, whenever that changes.
//! * The fullscreen guard hides it with the island (`pill.rs` emits `pill:guard`); nothing plays, nothing shows: no window, no timer, no frames.
pub mod logic;
mod view;

#[cfg(test)]
mod tests;

pub use view::{GUARD_EVENT, SLOT_EVENT};

use crate::ui::host::geom::{self, Align, Dock};
use crate::ui::host::{self, MonitorSel, NativeWindow, Placement, WinEvent};
use crate::ui::view::WindowId;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// The settings that decide where the window stands and how it is made. A change of any of them re-creates the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Sig {
    pub dock: Dock,
    pub align: Align,
    /// `media.widget.offset`, whole percent (only for the centre)
    pub offset: i32,
    /// `monitor` of the island: "primary" or "1".."n"; "active" while the island follows the active monitor (`edge.monitor`): it stands where the island does
    pub monitor: String,
    /// `underTaskbar`
    pub inside_work: bool,
    /// `showInCapture` (off = the window is hidden from screenshots and recordings)
    pub in_capture: bool,
}

/// Every key that matters to the window (what `on_settings` watches).
pub const KEYS: &[&str] = &["media.widget", "media.widget.edge", "media.widget.align", "media.widget.offset", "monitor", "edge.monitor", "underTaskbar", "showInCapture"];

/// The island's settings the widget keeps clear of while both stand in the taskbar: a change re-checks the place, it does not re-create the window.
/// (`size` is the island's and also the widget's own: the view scales, see [`size_k`].)
const ISLAND_KEYS: &[&str] = &["edge.position", "edge.align", "offsetX", "size"];

/// The widget's size factor (`size` / 100, `logic::size_k`) as the placement code sees it (`taskbar::widget_place` widens the slot it asks for).
static SIZE_K: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32

pub fn size_k() -> f32 {
    f32::from_bits(SIZE_K.load(Ordering::Relaxed))
}

fn read_size() {
    SIZE_K.store(logic::size_k(crate::settings::get_f64("size", 100.0)).to_bits(), Ordering::Relaxed);
}

/// The window for these settings: `None` when the widget is off. Pure: `get` is the settings reader (value or `Null`).
pub fn sig_from(get: &dyn Fn(&str) -> Value) -> Option<Sig> {
    if get("media.widget") != Value::Bool(true) {
        return None;
    }
    let s = |k: &str, d: &str| get(k).as_str().map_or_else(|| d.to_string(), str::to_string);
    Some(Sig {
        dock: Dock::from_setting(&s("media.widget.edge", "bottom")),
        align: match s("media.widget.align", "left").as_str() {
            "center" => Align::Center,
            "right" => Align::Right,
            _ => Align::Left,
        },
        offset: get("media.widget.offset").as_f64().unwrap_or(0.0).clamp(-50.0, 50.0).round() as i32,
        monitor: if s("edge.monitor", "primary") == "active" { "active".to_string() } else { s("monitor", "primary") },
        inside_work: get("underTaskbar") != Value::Bool(false),
        in_capture: get("showInCapture") != Value::Bool(false),
    })
}

/// The monitor of the island's `monitor` setting ("1".. = list order, else the primary): the same rule as `pill::layout_native`.
pub fn monitor_sel(setting: &str) -> MonitorSel {
    if setting == "active" {
        return MonitorSel::Island;
    }
    match setting.parse::<usize>() {
        Ok(i) if i >= 1 => MonitorSel::Index(i - 1),
        _ => MonitorSel::Primary,
    }
}

/// What the widget asks of a place in the bar.
fn ask(s: &Sig) -> geom::WidgetAsk {
    // body_w at 100 % (`taskbar::widget_place` scales it by the size); the pad is the same at every size, so the window never moves with it
    geom::WidgetAsk { align: s.align, offset_pct: s.offset as f32, body_w: logic::BAR_FOLD_W, pad: logic::BAR_PAD, inside_work: s.inside_work }
}

pub fn placement(s: &Sig) -> Placement {
    if s.dock == Dock::Taskbar {
        return Placement::InBar { monitor: monitor_sel(&s.monitor), ask: ask(s) };
    }
    Placement::Docked { monitor: monitor_sel(&s.monitor), dock: s.dock, align: s.align, offset_pct: s.offset as f32, inside_work: s.inside_work, margin: 0.0 }
}

/// The window's size, logical px (the taskbar dock's is taller: the open card floats above the bar).
fn win_size(s: &Sig) -> (f32, f32) {
    logic::Stand::from(s.dock).real_win()
}

fn spec(s: &Sig) -> NativeWindow {
    NativeWindow::overlay("music-widget")
        .title("pill-music")
        .size(win_size(s).0, win_size(s).1)
        .placement(placement(s))
        .hidden() // the view calls it up when something plays
        .exclude_from_capture(!s.in_capture)
        .on_event(|id, e| {
            if e == WinEvent::Closed {
                let _ = ID.compare_exchange(id.0, 0, Ordering::SeqCst, Ordering::SeqCst);
            }
        })
}

// ------------------------------------------------------------------------------------------------------------------------------ the window

static ID: AtomicU64 = AtomicU64::new(0);
/// the settings the live window was made from, and the geometry of its monitor it was placed for
static LIVE: Mutex<Option<(Sig, Probe)>> = Mutex::new(None);
/// the app handle, for the island's width news (`island_width_changed`, which comes from the island's thread)
static APP: Mutex<Option<tauri::AppHandle>> = Mutex::new(None);

fn live() -> std::sync::MutexGuard<'static, Option<(Sig, Probe)>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The settings as they are now (defaults filled in).
fn current() -> Option<Sig> {
    let m = crate::settings::merged();
    sig_from(&|k| m.get(k).cloned().unwrap_or(Value::Null))
}

/// Creates, re-creates or closes the window so that it matches the settings. Idempotent; safe from any thread.
pub fn sync(app: &tauri::AppHandle) {
    read_size();
    *APP.lock().unwrap_or_else(|e| e.into_inner()) = Some(app.clone());
    let want = current();
    let mut cur = live();
    if cur.as_ref().map(|c| &c.0) == want.as_ref() {
        return;
    }
    let host = crate::ui::native::ensure_host(app);
    let old = ID.swap(0, Ordering::SeqCst);
    if old != 0 {
        host.close(WindowId(old));
    }
    *cur = None;
    let Some(sig) = want else { return };
    let dock = sig.dock;
    let look = probe(&host, &sig);
    let (bar, align) = (look.bar, look.align);
    let id = spec(&sig).spawn(&host, move |cx| Box::new(view::MusicView::new(cx, dock).with_slot(bar, align)));
    ID.store(id.0, Ordering::SeqCst);
    FOLLOWING.store(false, Ordering::Relaxed); // a new window: the next poll points the foreground hook at it
    crate::taskbar::follow_widget(windows::Win32::Foundation::HWND::default(), false);
    *cur = Some((sig, look));
}

/// The island starts first, the widget right after it (`main.rs`).
pub fn start(app: &tauri::AppHandle) {
    sync(app);
}

pub fn on_settings(app: &tauri::AppHandle, patch: &Map<String, Value>) {
    if patch.contains_key("size") {
        read_size(); // the view reads `size` from the settings event itself; the placement reads it here
    }
    if KEYS.iter().any(|k| patch.contains_key(*k)) {
        sync(app);
    } else if ISLAND_KEYS.iter().any(|k| patch.contains_key(*k)) {
        relayout(app); // the island moved in the bar: the widget may have to step aside (or may come back)
    }
}

/// The monitor the widget stands on (the island's `monitor` setting).
fn monitor_of(host: &host::HostHandle, s: &Sig) -> Option<host::MonitorInfo> {
    let mons = host.monitors();
    match monitor_sel(&s.monitor) {
        MonitorSel::Index(i) => mons.get(i).cloned(),
        MonitorSel::Island => crate::pill::island_monitor().and_then(|r| mons.iter().find(|m| m.rect == r).cloned()),
        _ => None,
    }
    .or_else(|| mons.iter().find(|m| m.primary).cloned())
    .or_else(|| mons.first().cloned())
}

/// What one look at the monitor says about the window: a number that changes when the monitor the widget stands on moves, resizes, changes DPI or
/// loses / gains a taskbar (in the taskbar also when the tray or the island changes the place the widget gets), and where the window stands in the
/// bar: the bar's height (logical px, 0 = it is not in a bar: any other dock, or no bar it can live in) and the side it is anchored to.
#[derive(Clone, Copy, Debug)]
struct Probe {
    key: u64,
    /// `key` without the window's x: equal while only the place along the bar changed (the island beside the widget got wider or narrower)
    rest: u64,
    /// the window's x (physical px) and the monitor's DPI scale
    x: i32,
    scale: f32,
    bar: f32,
    align: Align,
}

/// One look (one monitor enumeration, one tray lookup).
fn probe(host: &host::HostHandle, s: &Sig) -> Probe {
    let mut out = Probe { key: 0, rest: 0, x: 0, scale: 1.0, bar: 0.0, align: s.align };
    let Some(m) = monitor_of(host, s) else { return out };
    let mut vals = vec![m.rect.l, m.rect.t, m.rect.r, m.rect.b, m.work.l, m.work.t, m.work.r, m.work.b, (m.scale * 100.0) as i32, host.monitors().len() as i32];
    if s.dock == Dock::Taskbar {
        let size = geom::size_to_phys(logic::WIN_BAR_REAL, m.scale);
        let ((x, y), slot) = crate::taskbar::widget_place(&m, &ask(s), size);
        vals.push(y);
        out.x = x;
        if let Some((bar, slot)) = slot {
            // not the slot's own x / width: they follow the size, which the view animates in place (the window's position is what counts)
            vals.extend([bar.h, slot.align.code() as i32]);
            (out.bar, out.align) = (bar.h as f32 / m.scale.max(0.25), slot.align);
        }
    }
    let fnv = |vals: &[i32]| vals.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ *v as u32 as u64).wrapping_mul(0x100_0000_01b3));
    out.rest = fnv(&vals);
    vals.push(out.x);
    out.key = fnv(&vals);
    out.scale = m.scale;
    out
}

/// The island's folded width changed (`taskbar::publish_island_w`, called from the island's thread when its capsule grows or shrinks in the
/// bar): the widget re-checks its place now instead of at the next guard poll. Off the island's thread, and only while a widget in the bar exists.
pub fn island_width_changed() {
    if ID.load(Ordering::SeqCst) == 0 {
        return;
    }
    let Some(app) = APP.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
    std::thread::spawn(move || relayout(&app));
}

/// How far (logical px, signed) the window moves under a body that glides to its new place, when ONLY the place along the bar changed and the widget
/// stays centred in its stretch (the island beside it got wider / narrower). `None` = anything else changed (monitor, DPI, bar, the side): the
/// body slides out, the window is re-placed and it slides in again. Pure.
fn glide_dx(prev: &Probe, now: &Probe) -> Option<f32> {
    let same = prev.rest == now.rest && prev.align == Align::Center && now.align == Align::Center && prev.bar == now.bar && prev.x != now.x;
    same.then(|| (prev.x - now.x) as f32 / now.scale.max(0.25))
}

/// `SLOT_EVENT` text for an `Align`.
fn align_name(a: Align) -> &'static str {
    match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
    }
}

/// The foreground hook points at the live window while it stands in the bar (set from the main thread; reset when the window is re-created).
static FOLLOWING: AtomicBool = AtomicBool::new(false);

/// The guard poll (every 500 ms, `pill::relayout`): the monitor, its DPI, its taskbar, the tray or the island's place changed -> put the window
/// back in its corner (in the taskbar the view slides out, is re-placed and slides in: `SLOT_EVENT`); and keep a window that stands in the bar
/// above it (the bar puts itself in front of the other topmost windows when it is clicked). Nothing here runs a timer of its own.
pub fn relayout(app: &tauri::AppHandle) {
    let id = ID.load(Ordering::SeqCst);
    let Some(host) = host::handle() else { return };
    if id == 0 {
        return;
    }
    let mut cur = live();
    let Some((sig, geo)) = cur.as_mut() else { return };
    let look = probe(&host, sig);
    if look.key != geo.key {
        let prev = std::mem::replace(geo, look);
        if sig.dock == Dock::Taskbar {
            if let Some(dx) = glide_dx(&prev, &look) {
                // only the centred widget's place along the bar moved: the window goes there at once and the body glides across (`dx` = how far
                // the window moved under it), no slide out and in
                let _ = crate::ui::bus::emit(SLOT_EVENT, json!({ "bar": look.bar, "align": align_name(look.align), "dx": dx }));
                host.relayout(WindowId(id));
            } else {
                let _ = crate::ui::bus::emit(SLOT_EVENT, json!({ "bar": look.bar, "align": align_name(look.align) }));
            }
        } else {
            host.relayout(WindowId(id));
        }
    }
    if sig.dock == Dock::Taskbar {
        let in_bar = look.bar > 0.0;
        let hwnd = host.hwnd(WindowId(id)).unwrap_or(0);
        if hwnd == 0 {
            return;
        }
        if in_bar {
            crate::taskbar::keep_widget_on_top(windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void));
        }
        if FOLLOWING.swap(in_bar, Ordering::Relaxed) != in_bar {
            // the hook is installed from the main thread's message loop, like the island's
            let _ = app.run_on_main_thread(move || crate::taskbar::follow_widget(windows::Win32::Foundation::HWND(hwnd as *mut std::ffi::c_void), in_bar));
        }
    }
}

#[cfg(test)]
mod glide_tests {
    use super::*;

    fn probe(rest: u64, x: i32, align: Align) -> Probe {
        Probe { key: rest ^ x as u64, rest, x, scale: 2.0, bar: 48.0, align }
    }

    #[test]
    fn only_a_centred_widget_that_moved_along_the_bar_glides_and_by_how_far_the_window_moved() {
        let (a, b) = (probe(7, 1000, Align::Center), probe(7, 1100, Align::Center));
        assert_eq!(glide_dx(&a, &b), Some(-50.0), "the window moved 100 device px (50 logical) right: the body starts 50 px left of its place");
        assert_eq!(glide_dx(&b, &a), Some(50.0));
        assert_eq!(glide_dx(&a, &a), None, "nothing moved");
        assert_eq!(glide_dx(&a, &probe(8, 1100, Align::Center)), None, "the monitor, DPI or bar changed: slide out and in");
        assert_eq!(glide_dx(&a, &probe(7, 1100, Align::Right)), None, "it changed side: slide out and in");
        assert_eq!(glide_dx(&probe(7, 1000, Align::Left), &probe(7, 1100, Align::Left)), None, "a corner widget never moves with the island's width");
        assert_eq!(glide_dx(&a, &Probe { bar: 40.0, ..probe(7, 1100, Align::Center) }), None);
    }
}
