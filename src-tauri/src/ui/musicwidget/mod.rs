//! The music widget: the island's player as its OWN small window, so it can be parked apart from the island (the bottom left corner next to the
//! taskbar while the pill stays in the middle, or the top, or any side of either edge). Settings (Media and sound page):
//!
//! | key | meaning |
//! |---|---|
//! | `media.widget` | off (default: nothing changes, no window exists) / on |
//! | `media.widget.edge` | `top` / `bottom` (default `bottom`) |
//! | `media.widget.align` | `left` / `center` / `right` along that edge (default `left`) |
//! | `media.widget.offset` | -50..50 % of the monitor width, only for `center` (like `offsetX`) |
//!
//! * One Overlay window (`ui::host`, `Placement::Docked`), 344 x 208 logical px, never resized: topmost, tool window, no-activate, per-pixel
//!   alpha, click-through outside the body. It uses the island's monitor (`monitor`), the work area (`underTaskbar`) and `showInCapture`.
//! * `view.rs` draws it and `logic.rs` decides everything (when it shows, the geometry, what a click hits); both are fed by the SAME backend
//!   the island's player uses (`media:changed`, `media_state`, `media_control`, `media_seek`): there is no second media backend.
//! * De-duplication: while `media.widget` is on, the island does not show the player (`ui-core` `live::media` gives no row and no edge
//!   reaction, `panel::a::home` has no player part and `command::tab_on` ignores it). Turning it off gives all of it back at once.
//! * The window exists only while the setting is on. Its placement is part of its spec, so a change of the placement settings re-creates it
//!   (`sync`); a change of the monitors, DPI or the taskbar re-places it (`relayout`, from the guard poll like the island's).
//! * The fullscreen guard hides it with the island (`pill.rs` emits `pill:guard`); nothing plays, nothing shows: no window, no timer, no frames.
pub mod logic;
mod view;

#[cfg(test)]
mod tests;

pub use view::GUARD_EVENT;

use crate::ui::host::geom::{Align, Dock};
use crate::ui::host::{self, MonitorSel, NativeWindow, Placement, WinEvent};
use crate::ui::view::WindowId;
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// The settings that decide where the window stands and how it is made. A change of any of them re-creates the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Sig {
    pub dock: Dock,
    pub align: Align,
    /// `media.widget.offset`, whole percent (only for the centre)
    pub offset: i32,
    /// `monitor` of the island: "primary" or "1".."n"
    pub monitor: String,
    /// `underTaskbar`
    pub inside_work: bool,
    /// `showInCapture` (off = the window is hidden from screenshots and recordings)
    pub in_capture: bool,
}

/// Every key that matters to the window (what `on_settings` watches).
pub const KEYS: &[&str] = &["media.widget", "media.widget.edge", "media.widget.align", "media.widget.offset", "monitor", "underTaskbar", "showInCapture"];

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
        monitor: s("monitor", "primary"),
        inside_work: get("underTaskbar") != Value::Bool(false),
        in_capture: get("showInCapture") != Value::Bool(false),
    })
}

/// The monitor of the island's `monitor` setting ("1".. = list order, else the primary): the same rule as `pill::layout_native`.
pub fn monitor_sel(setting: &str) -> MonitorSel {
    match setting.parse::<usize>() {
        Ok(i) if i >= 1 => MonitorSel::Index(i - 1),
        _ => MonitorSel::Primary,
    }
}

pub fn placement(s: &Sig) -> Placement {
    Placement::Docked { monitor: monitor_sel(&s.monitor), dock: s.dock, align: s.align, offset_pct: s.offset as f32, inside_work: s.inside_work, margin: 0.0 }
}

fn spec(s: &Sig) -> NativeWindow {
    NativeWindow::overlay("music-widget")
        .title("pill-music")
        .size(logic::WIN.0, logic::WIN.1)
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
static LIVE: Mutex<Option<(Sig, u64)>> = Mutex::new(None);

fn live() -> std::sync::MutexGuard<'static, Option<(Sig, u64)>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The settings as they are now (defaults filled in).
fn current() -> Option<Sig> {
    let m = crate::settings::merged();
    sig_from(&|k| m.get(k).cloned().unwrap_or(Value::Null))
}

/// Creates, re-creates or closes the window so that it matches the settings. Idempotent; safe from any thread.
pub fn sync(app: &tauri::AppHandle) {
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
    let id = spec(&sig).spawn(&host, move |cx| Box::new(view::MusicView::new(cx, dock)));
    ID.store(id.0, Ordering::SeqCst);
    let geo = geometry_key(&host, &sig);
    *cur = Some((sig, geo));
}

/// The island starts first, the widget right after it (`main.rs`).
pub fn start(app: &tauri::AppHandle) {
    sync(app);
}

pub fn on_settings(app: &tauri::AppHandle, patch: &Map<String, Value>) {
    if KEYS.iter().any(|k| patch.contains_key(*k)) {
        sync(app);
    }
}

/// A number that changes when the monitor the widget stands on moves, resizes, changes DPI or loses / gains a taskbar.
fn geometry_key(host: &host::HostHandle, s: &Sig) -> u64 {
    let mons = host.monitors();
    let m = match monitor_sel(&s.monitor) {
        MonitorSel::Index(i) => mons.get(i),
        _ => None,
    }
    .or_else(|| mons.iter().find(|m| m.primary))
    .or_else(|| mons.first());
    let Some(m) = m else { return 0 };
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [m.rect.l, m.rect.t, m.rect.r, m.rect.b, m.work.l, m.work.t, m.work.r, m.work.b, (m.scale * 100.0) as i32, mons.len() as i32] {
        h = (h ^ v as u32 as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// The guard poll (every 500 ms, `pill::relayout`): the monitor, its DPI or its taskbar changed -> put the window back in its corner.
pub fn relayout() {
    let id = ID.load(Ordering::SeqCst);
    let Some(host) = host::handle() else { return };
    if id == 0 {
        return;
    }
    let mut cur = live();
    let Some((sig, geo)) = cur.as_mut() else { return };
    let now = geometry_key(&host, sig);
    if now != *geo {
        *geo = now;
        host.relayout(WindowId(id));
    }
}
