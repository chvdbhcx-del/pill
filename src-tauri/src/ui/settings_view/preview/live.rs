//! The sandbox: a second `IslandView`, the real island code, running inside the settings window on a context of its own. Its `Cx` has a bus
//! nobody else publishes on, a queue nobody else posts to and the [`SampleApi`] as its backend, so it sees only what the preview feeds it:
//! the window's settings (filtered, see `scene::shareable`) and invented events. It gets no mouse or key events and its requests to the host
//! (focus, hit region, cursor, frame rate) are dropped.
//!
//! The standalone music widget (`media.widget`) is a window of its own in the real program, so it is a second view here too: a real `MusicView`
//! on a context of its own, fed the same way (the settings and one invented track), drawn by the camera on the same stage.
use super::sample::{self, SampleApi, Shared};
use crate::ui::host::geom::{Align, Dock};
use crate::ui::island::sys::SysClock;
use crate::ui::island::view::IslandView;
use crate::ui::musicwidget::{logic as widget_logic, MusicView, SLOT_EVENT};
use crate::ui::tabs::a::core::Tick;
use crate::ui::view::{Cx, FakeBus, Msg, MsgQueue, Services, View, Waker, WindowId};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;
use pill_ui_core::shell::types::Event;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The size of the island's imaginary window (css px): wide and tall enough for the biggest island; the preview looks at the part that has
/// the island in it.
pub const VW: f32 = 1500.0;
pub const VH: f32 = 900.0;
/// The height (css px) of the imaginary taskbar the island stands in when the edge is the taskbar.
pub const BAR_H: f32 = 48.0;

/// The standalone music widget of the sandbox: the real `MusicView` on a context of its own (own bus and queue, the same sample backend), so it
/// hears only the invented track and the filtered settings.
struct Player {
    view: MusicView,
    cx: Cx,
    q: Arc<MsgQueue>,
    bus: Arc<FakeBus>,
    dock: Dock,
    /// the side the window stands at (the setting, or the other side while the island is in the bar too)
    align: Align,
    /// told to go (the preview left its scene): it slides out and is dropped
    leaving: bool,
}

/// Where the widget's window stands, for the camera: its size (logical px), its edge, its side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerGeom {
    pub win: (f32, f32),
    pub dock: Dock,
    pub align: Align,
}

pub struct Live {
    pub view: IslandView,
    pub cx: Cx,
    q: Arc<MsgQueue>,
    bus: Arc<FakeBus>,
    st: Shared,
    awake: Arc<AtomicBool>,
    host_waker: Waker,
    player: Option<Player>,
    /// the sandbox clock (ms): runs only while it is awake, so a parked card keeps its remaining time
    pub vnow: f64,
    last_scale: f32,
}

impl Live {
    pub fn new(host: &Cx, settings: Map<String, Value>, os_lang: &str) -> Live {
        let st: Shared = Arc::new(Mutex::new(settings));
        let awake = Arc::new(AtomicBool::new(true));
        let (mut cx, q, bus) = side_cx(&host.waker(), host, &st, &awake);
        let view = IslandView::with_parts(&mut cx, SysClock::real(), os_lang);
        let _ = cx.take_effects();
        let live = Live { view, cx, q, bus, st, awake, host_waker: host.waker(), player: None, vnow: 0.0, last_scale: host.scale };
        let taskbar = live.st.lock().unwrap_or_else(|e| e.into_inner()).get("edge.position").and_then(|v| v.as_str()).is_some_and(|p| p == "taskbar");
        live.bar(taskbar);
        live
    }

    /// The imaginary taskbar exists (the edge is the taskbar) or not: the island hears it as `pill.rs` would tell it.
    fn bar(&self, on: bool) {
        self.bus.emit("pill:taskbar", json!({ "h": if on { BAR_H } else { 0.0 } }));
    }

    /// New settings values (already filtered): the island hears them as the backend's `settings:changed`.
    pub fn patch(&mut self, patch: Map<String, Value>) {
        {
            let mut g = self.st.lock().unwrap_or_else(|e| e.into_inner());
            for (k, v) in &patch {
                g.insert(k.clone(), v.clone());
            }
        }
        let edge = patch.get("edge.position").and_then(|v| v.as_str()).map(|p| p == "taskbar");
        let island_moved = patch.contains_key("edge.position") || patch.contains_key("edge.align");
        if let Some(p) = self.player.as_ref().filter(|p| !p.leaving) {
            p.bus.emit("settings:changed", Value::Object(patch.clone()));
        }
        self.bus.emit("settings:changed", Value::Object(patch));
        if let Some(on) = edge {
            self.bar(on);
        }
        // the island took the bar or left it: a widget in the bar steps to the other side of it, or back (the view slides out and in)
        if island_moved {
            let (_, want) = widget_wish(&self.st);
            let align = slot_align(&self.st, want);
            if let Some(p) = self.player.as_mut().filter(|p| p.dock == Dock::Taskbar && !p.leaving && p.align != align) {
                p.align = align;
                p.bus.emit(SLOT_EVENT, json!({ "bar": BAR_H, "align": align_name(align) }));
            }
        }
    }

    /// The widget stands in the sandbox: created for the edge and the side the settings ask for. It is kept while only its size or its look
    /// changes; it is made again, so that it slides in at its new place, when the edge or the side changed (the real window is re-created too).
    pub fn widget_show(&mut self) {
        let (dock, want) = widget_wish(&self.st);
        let align = slot_align(&self.st, want);
        if self.player.as_ref().is_some_and(|p| !p.leaving && p.dock == dock && p.align == align) {
            return;
        }
        let (mut cx, q, bus) = side_cx(&self.host_waker, &self.cx, &self.st, &self.awake);
        cx.now_ms = self.vnow;
        let bar = if dock == Dock::Taskbar { BAR_H } else { 0.0 };
        let view = MusicView::new(&mut cx, dock).with_slot(bar, align);
        let _ = cx.take_effects();
        bus.emit("media:changed", sample::widget_track());
        self.player = Some(Player { view, cx, q, bus, dock, align, leaving: false });
    }

    /// Replay: the widget is made again and slides in again.
    pub fn widget_restart(&mut self) {
        self.player = None;
        self.widget_show();
    }

    /// The preview left the widget scene: the widget slides out (its own leave motion) and is dropped when it has gone.
    pub fn widget_leave(&mut self) {
        if let Some(p) = self.player.as_mut().filter(|p| !p.leaving) {
            p.leaving = true;
            p.bus.emit("settings:changed", json!({ "media.widget": false }));
        }
    }

    /// The widget stands in the imaginary taskbar.
    pub fn player_in_bar(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.dock == Dock::Taskbar)
    }

    /// Where the widget's window stands (`None` = no widget).
    pub fn player(&self) -> Option<PlayerGeom> {
        self.player.as_ref().map(|p| PlayerGeom { win: p.view.size_hint().unwrap_or(widget_logic::WIN_REAL), dock: p.dock, align: p.align })
    }

    /// An invented backend event.
    pub fn emit(&self, name: &str, payload: Value) {
        self.bus.emit(name, payload);
    }

    /// Takes down what a reaction left on the folded island: the card on show, the music row, the recording capsule, the privacy dots, the rows
    /// of a download, the agents, OBS, the shelf and the load.
    pub fn clear_reactions(&mut self) {
        self.view.core.shell_event(Event::CardTtl(0.0), self.vnow);
        for (name, payload) in sample::clear_events() {
            self.bus.emit(name, payload);
        }
    }

    pub fn is_awake(&self) -> bool {
        self.awake.load(Ordering::Relaxed)
    }

    /// Messages are waiting although nobody woke us (a draw posted a re-measure while parked).
    pub fn has_mail(&self) -> bool {
        !self.q.is_empty() || self.player.as_ref().is_some_and(|p| !p.q.is_empty())
    }

    pub fn wake(&mut self) {
        if !self.awake.swap(true, Ordering::Relaxed) {
            self.view.preview_wake(&mut self.cx);
            let _ = self.cx.take_effects();
            if let Some(p) = &mut self.player {
                p.cx.post(Msg::Custom(Box::new(Tick))); // the tick that the park stopped starts again
            }
        }
    }

    pub fn sleep(&mut self) {
        if self.awake.swap(false, Ordering::Relaxed) {
            self.view.preview_sleep(&mut self.cx);
            let _ = self.cx.take_effects();
            if let Some(p) = &mut self.player {
                p.view.preview_sleep();
            }
        }
    }

    /// Delivers what is queued. True when there was something.
    fn pump(&mut self) -> bool {
        let msgs = self.q.drain();
        let any = !msgs.is_empty();
        for (_, m) in msgs {
            self.view.message(m, &mut self.cx);
        }
        any
    }

    /// One frame of the sandbox. `(busy, ambient_only)`: busy = something real moves (not just a looping idle animation).
    pub fn step(&mut self, host: &Cx, dt_ms: f64, reduced: bool) -> bool {
        self.vnow += dt_ms;
        self.cx.now_ms = self.vnow;
        self.cx.reduced_motion = reduced;
        self.cx.lang = host.lang;
        self.cx.refresh_hz = host.refresh_hz;
        if (host.scale - self.last_scale).abs() > 1e-3 {
            self.last_scale = host.scale;
            self.cx.scale = host.scale;
            self.view.on_dpi(host.scale, &mut self.cx);
        }
        let mail = self.pump();
        let more = self.view.step(dt_ms, &mut self.cx);
        let ambient = self.cx.take_ambient();
        let fx = self.cx.take_effects();
        let player = self.step_player(host, dt_ms, reduced);
        mail || fx.frame || (more && !ambient) || player
    }

    /// One frame of the widget; true while it moves or has something to say.
    fn step_player(&mut self, host: &Cx, dt_ms: f64, reduced: bool) -> bool {
        let Some(p) = &mut self.player else { return false };
        p.cx.now_ms = self.vnow;
        p.cx.reduced_motion = reduced;
        p.cx.lang = host.lang;
        p.cx.refresh_hz = host.refresh_hz;
        p.cx.scale = host.scale;
        let mut busy = false;
        for (_, m) in p.q.drain() {
            busy = true;
            let _ = p.view.message(m, &mut p.cx);
        }
        let more = p.view.step(dt_ms, &mut p.cx);
        let _ = p.cx.take_ambient();
        let fx = p.cx.take_effects();
        // the real window is shown and hidden by the host; here the view is told so
        if fx.show {
            p.view.on_show(&mut p.cx);
            let _ = p.cx.take_effects();
            return true;
        }
        if fx.hide {
            p.view.on_hide(&mut p.cx);
            let _ = p.cx.take_effects();
        }
        if p.leaving && !more && !fx.frame {
            self.player = None;
            return false;
        }
        busy || fx.frame || more
    }

    pub fn draw(&mut self, cv: &Canvas, th: &Theme, host: &Cx) {
        self.cx.scale = host.scale;
        // the imaginary window clips what slides out through its edge (the edge change), as the real window does
        let win = Rect::new(0.0, 0.0, VW, VH);
        cv.with_clip(win, |cv| self.view.draw(cv, th, win, &self.cx));
    }

    /// The widget, in the coordinates of its own window (the camera puts that window on the stage); the window clips what slides out through its edge.
    pub fn draw_player(&mut self, cv: &Canvas, th: &Theme, host: &Cx) {
        let Some(p) = &mut self.player else { return };
        p.cx.scale = host.scale;
        let (w, h) = p.view.size_hint().unwrap_or(widget_logic::WIN_REAL);
        let win = Rect::new(0.0, 0.0, w, h);
        cv.with_clip(win, |cv| p.view.draw(cv, th, win, &p.cx));
    }

    /// The island hangs from the bottom edge of its window (the `edge.position` setting, once the island has slid over).
    pub fn at_bottom(&self) -> bool {
        self.view.at_bottom()
    }

    /// The island stands inside the imaginary taskbar.
    pub fn in_bar(&self) -> bool {
        self.view.in_bar()
    }

    /// The side of its edge the island stands at (`edge.align`).
    pub fn aligned(&self) -> crate::ui::host::geom::Align {
        self.view.aligned()
    }

    /// The folded or open island's body in the imaginary window: `(left, width, height)`.
    pub fn body(&self) -> (f32, f32, f32) {
        self.view.body_in_window()
    }
}

/// A context for one sandbox view: its own bus (nobody else publishes on it), its own queue (nobody else posts to it; it wakes the host while the
/// sandbox is awake) and the sample backend over the shared settings.
fn side_cx(host_waker: &Waker, host: &Cx, st: &Shared, awake: &Arc<AtomicBool>) -> (Cx, Arc<MsgQueue>, Arc<FakeBus>) {
    let (mut svc, fakes) = Services::fake();
    svc.api = Arc::new(SampleApi { settings: st.clone() });
    let (hw, aw) = (host_waker.clone(), awake.clone());
    let q = Arc::new(MsgQueue::new(move || {
        if aw.load(Ordering::Relaxed) {
            hw.post_custom(super::super::Local::Pv);
        }
    }));
    let mut cx = Cx::new(WindowId(0), "preview", svc, Waker::new(WindowId(0), q.clone()));
    cx.lang = host.lang;
    cx.scale = host.scale;
    cx.reduced_motion = host.reduced_motion;
    cx.refresh_hz = host.refresh_hz;
    (cx, q, fakes.bus)
}

/// The edge and the side the settings ask the widget for (`media.widget.edge` / `.align`, with the real defaults: the bottom, the left).
fn widget_wish(st: &Shared) -> (Dock, Align) {
    let g = st.lock().unwrap_or_else(|e| e.into_inner());
    let s = |k: &str, d: &'static str| g.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
    let dock = Dock::from_setting(&s("media.widget.edge", "bottom"));
    let align = match s("media.widget.align", "left").as_str() {
        "center" => Align::Center,
        "right" => Align::Right,
        _ => Align::Left,
    };
    (dock, align)
}

/// Where the widget stands along the bar: its own side, unless the island stands in the bar too (`geom::widget_bar_slot`): then the widget keeps
/// clear of it, on the island's opposite side (a simplified copy of the rule: the sandbox has no tray to measure).
fn slot_align(st: &Shared, want: Align) -> Align {
    let g = st.lock().unwrap_or_else(|e| e.into_inner());
    if g.get("edge.position").and_then(Value::as_str) != Some("taskbar") {
        return want;
    }
    match (Align::from_setting(g.get("edge.align").and_then(Value::as_str).unwrap_or("center")), want) {
        (Align::Left, Align::Left | Align::Center) => Align::Right,
        (Align::Right, Align::Right | Align::Center) => Align::Left,
        _ => want,
    }
}

fn align_name(a: Align) -> &'static str {
    match a {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
    }
}
