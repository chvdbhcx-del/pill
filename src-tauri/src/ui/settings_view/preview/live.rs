//! The sandbox: a second `IslandView`, the real island code, running inside the settings window on a context of its own. Its `Cx` has a bus
//! nobody else publishes on, a queue nobody else posts to and the [`SampleApi`] as its backend, so it sees only what the preview feeds it:
//! the window's settings (filtered, see `scene::shareable`) and invented events. It gets no mouse or key events and its requests to the host
//! (focus, hit region, cursor, frame rate) are dropped.
use super::sample::{SampleApi, Shared};
use crate::ui::island::sys::SysClock;
use crate::ui::island::view::IslandView;
use crate::ui::view::{Cx, Fakes, FakeBus, MsgQueue, Services, View, Waker, WindowId};
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

pub struct Live {
    pub view: IslandView,
    pub cx: Cx,
    q: Arc<MsgQueue>,
    bus: Arc<FakeBus>,
    st: Shared,
    awake: Arc<AtomicBool>,
    /// the sandbox clock (ms): runs only while it is awake, so a parked card keeps its remaining time
    pub vnow: f64,
    last_scale: f32,
}

impl Live {
    pub fn new(host: &Cx, settings: Map<String, Value>, os_lang: &str) -> Live {
        let (mut svc, fakes): (Services, Fakes) = Services::fake();
        let st: Shared = Arc::new(Mutex::new(settings));
        svc.api = Arc::new(SampleApi { settings: st.clone() });
        let awake = Arc::new(AtomicBool::new(true));
        let (hw, aw) = (host.waker(), awake.clone());
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
        let view = IslandView::with_parts(&mut cx, SysClock::real(), os_lang);
        let _ = cx.take_effects();
        Live { view, cx, q, bus: fakes.bus, st, awake, vnow: 0.0, last_scale: host.scale }
    }

    /// New settings values (already filtered): the island hears them as the backend's `settings:changed`.
    pub fn patch(&mut self, patch: Map<String, Value>) {
        {
            let mut g = self.st.lock().unwrap_or_else(|e| e.into_inner());
            for (k, v) in &patch {
                g.insert(k.clone(), v.clone());
            }
        }
        self.bus.emit("settings:changed", Value::Object(patch));
    }

    /// An invented backend event.
    pub fn emit(&self, name: &str, payload: Value) {
        self.bus.emit(name, payload);
    }

    /// Takes down what a reaction left on the folded island: the card on show, the music row, the recording capsule, the privacy dots.
    pub fn clear_reactions(&mut self) {
        self.view.core.shell_event(Event::CardTtl(0.0), self.vnow);
        self.bus.emit("media:changed", json!({ "has_session": false, "playing": false }));
        self.bus.emit("rec:state", json!({ "recording": false }));
        self.bus.emit("live:privacy", json!({ "mic": false, "cam": false, "apps": [] }));
    }

    pub fn is_awake(&self) -> bool {
        self.awake.load(Ordering::Relaxed)
    }

    /// Messages are waiting although nobody woke us (a draw posted a re-measure while parked).
    pub fn has_mail(&self) -> bool {
        !self.q.is_empty()
    }

    pub fn wake(&mut self) {
        if !self.awake.swap(true, Ordering::Relaxed) {
            self.view.preview_wake(&mut self.cx);
            let _ = self.cx.take_effects();
        }
    }

    pub fn sleep(&mut self) {
        if self.awake.swap(false, Ordering::Relaxed) {
            self.view.preview_sleep(&mut self.cx);
            let _ = self.cx.take_effects();
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
        mail || fx.frame || (more && !ambient)
    }

    pub fn draw(&mut self, cv: &Canvas, th: &Theme, host: &Cx) {
        self.cx.scale = host.scale;
        self.view.draw(cv, th, Rect::new(0.0, 0.0, VW, VH), &self.cx);
    }

    /// The folded or open island's body in the imaginary window: `(left, width, height)`.
    pub fn body(&self) -> (f32, f32, f32) {
        let b = self.view.core.shell.view().body;
        (b.left as f32, b.width as f32, b.height as f32)
    }
}
