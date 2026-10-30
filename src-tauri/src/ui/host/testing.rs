//! Headless harness for views: no window, no GPU. Rendering runs on WARP through `glasskit::testkit::Harness`; the "host loop" is
//! simulated by [`Sim`], which uses the SAME frame scheduler (`sched::FrameState`) and the SAME `Cx` as the real host, so a test sees exactly
//! when a view would be stepped, redrawn, or left idle.
//!
//! ```ignore
//! // pixels
//! let img = testing::render_view(&mut MyView::new(), (200.0, 60.0), 2.0, &Theme::pill_dark());
//! assert_eq!(img.at(20.0, 10.0)[3], 255);
//! // behaviour
//! let mut sim = Sim::new(Box::new(MyView::new()));
//! sim.send(&InputEvent::MouseMove { pos: (10.0, 10.0), mods: Modifiers::NONE, buttons: Buttons::NONE });
//! assert!(sim.run(600) > 0, "the hover animation ran");      // frames until the view says it is settled
//! assert!(!sim.frame.wants_frame(), "and then it idles");
//! ```
use super::sched::FrameState;
use crate::ui::view::{Cx, DropEvent, Effects, Fakes, Msg, MsgQueue, ProximityEvent, Services, View};
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, Response};
use glasskit::testkit::{Harness, Image};
use glasskit::theme::Theme;
use std::sync::{Arc, Mutex, MutexGuard};

thread_local! {
    static HARNESS: Harness = Harness::new();
}

/// Serialises tests that flip the process-wide reduced-motion override (`glasskit::ease::force_reduced`).
pub fn motion_lock() -> MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

/// Draws `view` once into a transparent `size` (logical px) canvas at `scale` and returns the pixels (premultiplied BGRA, device px).
pub fn render_view(view: &mut dyn View, size: (f32, f32), scale: f32, theme: &Theme) -> Image {
    let mut cx = Cx::headless();
    cx.scale = scale;
    render_view_cx(view, &cx, size, scale, theme)
}

/// Like `render_view`, with a context of your choice (language, reduced motion, refresh rate...).
pub fn render_view_cx(view: &mut dyn View, cx: &Cx, size: (f32, f32), scale: f32, theme: &Theme) -> Image {
    let rect = Rect::new(0.0, 0.0, size.0, size.1);
    HARNESS.with(|h| h.render(size.0.ceil() as u32, size.1.ceil() as u32, scale, Color::rgba(0.0, 0.0, 0.0, 0.0), |cv| view.draw(cv, theme, rect, cx)))
}

fn merge(into: &mut Effects, fx: Effects) {
    into.frame |= fx.frame;
    into.cursor = fx.cursor.or(into.cursor);
    if fx.hit_all {
        into.hit_all = true;
        into.hit_region = None;
    }
    if let Some(r) = fx.hit_region {
        into.hit_region = Some(r);
        into.hit_all = false;
    }
    into.caption_region = fx.caption_region.or(into.caption_region.take());
    into.drop_zone = fx.drop_zone.or(into.drop_zone.take());
    into.pass_through = fx.pass_through.or(into.pass_through);
    into.gaze = fx.gaze.or(into.gaze);
    into.focus = fx.focus.or(into.focus);
    into.show |= fx.show;
    into.hide |= fx.hide;
    into.close |= fx.close;
    into.title = fx.title.or(into.title.take());
    into.window_cmd = fx.window_cmd.or(into.window_cmd);
    into.opens.extend(fx.opens);
    into.ime_rect = fx.ime_rect.or(into.ime_rect);
    into.fps_cap = fx.fps_cap.or(into.fps_cap);
}

/// A simulated host window around one view: delivers input / messages like the real loop and runs frames on the real scheduler.
pub struct Sim {
    pub view: Box<dyn View>,
    pub cx: Cx,
    pub queue: Arc<MsgQueue>,
    pub frame: FrameState,
    /// The simulated clock (ms); advances one display period per frame.
    pub now_ms: f64,
    pub hz: f32,
    pub frames_run: u64,
    fx: Effects,
}

impl Sim {
    /// Fake services (recording backend, in-memory clipboard, inline jobs), 240 Hz display, scale 1.
    pub fn new(view: Box<dyn View>) -> Sim {
        Sim::with_services(view, Services::null())
    }

    /// Fake services whose recorders you keep: `let (svc, fakes) = Services::fake(); let sim = Sim::with_services(view, svc);`.
    pub fn with_services(view: Box<dyn View>, svc: Services) -> Sim {
        let (cx, queue) = Cx::headless_queue(svc);
        Sim { view, cx, queue, frame: FrameState::new(240.0), now_ms: 0.0, hz: 240.0, frames_run: 0, fx: Effects::default() }
    }

    pub fn fakes() -> (Services, Fakes) {
        Services::fake()
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.cx.scale = scale;
        self.view.on_dpi(scale, &mut self.cx);
        self.absorb(Response::redraw());
    }

    pub fn set_refresh_hz(&mut self, hz: f32) {
        self.hz = hz;
        self.cx.refresh_hz = hz;
        self.frame.set_refresh_hz(hz);
    }

    fn absorb(&mut self, resp: Response) -> Response {
        let fx = self.cx.take_effects();
        if resp.redraw || fx.frame {
            self.frame.request();
        }
        merge(&mut self.fx, fx);
        resp
    }

    /// An input event, as the host would deliver it.
    pub fn send(&mut self, ev: &InputEvent) -> Response {
        self.cx.now_ms = self.now_ms;
        let r = self.view.event(ev, &mut self.cx);
        self.absorb(r)
    }

    pub fn send_drop(&mut self, ev: &DropEvent) -> Response {
        self.cx.now_ms = self.now_ms;
        let r = self.view.on_drop(ev, &mut self.cx);
        self.absorb(r)
    }

    pub fn send_proximity(&mut self, ev: &ProximityEvent) -> Response {
        self.cx.now_ms = self.now_ms;
        let r = self.view.on_proximity(ev, &mut self.cx);
        self.absorb(r)
    }

    pub fn show(&mut self) {
        self.view.on_show(&mut self.cx);
        self.absorb(Response::redraw());
    }

    pub fn hide(&mut self) {
        self.view.on_hide(&mut self.cx);
        self.absorb(Response::NONE);
    }

    /// Delivers a message directly.
    pub fn deliver(&mut self, msg: Msg) -> Response {
        self.cx.now_ms = self.now_ms;
        let r = self.view.message(msg, &mut self.cx);
        self.absorb(r)
    }

    /// Delivers everything the view's subscriptions and `call_async` posted to its queue; returns how many messages.
    pub fn pump(&mut self) -> usize {
        let msgs = self.queue.drain();
        let n = msgs.len();
        for (_, m) in msgs {
            self.deliver(m);
        }
        n
    }

    /// One frame if the view wants one (advances the clock by one display period). False when the window is idle.
    pub fn step_frame(&mut self) -> bool {
        if !self.frame.wants_frame() {
            return false;
        }
        self.now_ms += 1000.0 / self.hz as f64;
        self.cx.now_ms = self.now_ms;
        let dt = self.frame.begin(self.now_ms);
        let keep = self.view.step(dt, &mut self.cx);
        self.absorb(Response::NONE);
        self.frame.end(keep);
        self.frames_run += 1;
        true
    }

    /// Runs frames until the view settles (or `max` frames); returns how many ran. 0 = it was idle.
    pub fn run(&mut self, max: usize) -> usize {
        let mut n = 0;
        while n < max && self.step_frame() {
            n += 1;
        }
        n
    }

    /// What the view asked of the host since the last call (merged), and clears it.
    pub fn take_fx(&mut self) -> Effects {
        let mut fx = std::mem::take(&mut self.fx);
        merge(&mut fx, self.cx.take_effects());
        fx
    }

    pub fn render(&mut self, size: (f32, f32), theme: &Theme) -> Image {
        render_view_cx(&mut *self.view, &self.cx, size, self.cx.scale, theme)
    }
}

/// Feeds `events` to a fresh simulated window, running up to `max_frames` frames after each, and returns the window for inspection.
pub fn drive(view: Box<dyn View>, events: &[InputEvent], max_frames: usize) -> Sim {
    let mut s = Sim::new(view);
    s.show();
    s.run(max_frames);
    for e in events {
        s.send(e);
        s.run(max_frames);
    }
    s
}

/// Writes `img` as a PNG (straight alpha) for a look with your own eyes; creates the directory.
pub fn save_png(img: &Image, path: impl AsRef<std::path::Path>) {
    HARNESS.with(|h| {
        if let Err(e) = img.save_png(h, path) {
            eprintln!("save_png: {e}");
        }
    });
}
