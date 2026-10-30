//! The plumbing every tab view shares: backend calls with their results, `Fx` routing, the periodic tick, and the `Base` that bundles it.
//!
//! The wiring of a tab view (see `a.rs` for the host contract):
//! ```ignore
//! fn on_show(&mut self, cx) { self.base.show(cx, &["media:changed", ..], 1000); self.run(cx, self.model.on_show()); }
//! fn message(&mut self, msg, cx) -> Response {
//!     match self.base.classify(msg, cx) {
//!         Incoming::Event { name, payload } => { let calls = self.model.on_event(&name, &payload); self.run(cx, calls); }
//!         Incoming::Result { cmd, args, result } => { /* feed model.on_result(cmd, &args, &value) */ }
//!         Incoming::Tick(dt_ms) => { let calls = self.model.tick(dt_ms); self.run(cx, calls); }
//!         Incoming::Image(_) | Incoming::Other(_) => {}
//!     }
//!     let local = self.base.route_fx(cx, self.model.take_fx());
//!     Response::redraw()
//! }
//! ```
use super::draw::{ImageCache, ImageReady};
use super::play::Player;
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{Cx, Msg, Ticket, Waker};
use glasskit::geom::Rect;
use pill_ui_core::panel::a::{BackendCall, Fx};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The tick message a [`Ticker`] posts to the view (`Msg::Custom(Tick)`). It carries nothing: the base measures the real elapsed time itself.
pub struct Tick;

/// A background thread that posts [`Tick`] every `period` until dropped. This is how a model's clock (seek bar, polling intervals, "press
/// again" arms) advances while no frame is running; it costs one wake per period, never a frame by itself.
pub struct Ticker {
    stop: Arc<AtomicBool>,
}

impl Ticker {
    pub fn start(w: Waker, period: Duration) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        std::thread::Builder::new()
            .name("pill-tab-tick".into())
            .spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    std::thread::sleep(period);
                    if s.load(Ordering::Relaxed) {
                        break;
                    }
                    w.post_custom(Tick);
                }
            })
            .ok();
        Ticker { stop }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Commands in flight: `Cx::call_async` gives a ticket, the model wants `(cmd, args)` back with the answer.
#[derive(Default)]
pub struct Calls {
    map: HashMap<u64, (&'static str, Value)>,
}

impl Calls {
    /// Start every call on a worker.
    pub fn run(&mut self, cx: &mut Cx, calls: Vec<BackendCall>) {
        for c in calls {
            let t = cx.call_async(c.cmd, c.args.clone());
            self.map.insert(t.0, (c.cmd, c.args));
        }
    }

    /// The call a reply belongs to (once).
    pub fn claim(&mut self, t: Ticket) -> Option<(&'static str, Value)> {
        self.map.remove(&t.0)
    }

    pub fn in_flight(&self) -> usize {
        self.map.len()
    }
}

/// A rejected command, as the models that care expect it (`tabs::ERR_KEY`).
pub fn err_value(e: &str) -> Value {
    json!({ pill_ui_core::panel::a::tabs::ERR_KEY: e })
}

/// What a `Msg` means to a tab.
pub enum Incoming {
    /// A backend event the tab subscribed to.
    Event { name: String, payload: Value },
    /// The answer of a command this tab started (`result` is the `Err` text when it was rejected).
    Result { cmd: &'static str, args: Value, result: Result<Value, String> },
    /// The periodic tick; the value is the real milliseconds since the previous one.
    Tick(f64),
    /// A decoded image arrived; `Some(key)` when it was stored in `base.images` (repaint), `None` when decoding failed.
    Image(Option<String>),
    /// Anything else (an `Open`, a reply of another view, a foreign `Custom`).
    Other(Msg),
}

/// View-local effects the model asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Local {
    FocusSearch,
    BlurSearch,
    ScrollTo(String),
}

/// What every tab view holds.
pub struct Base {
    pub calls: Calls,
    pub player: Player,
    pub images: ImageCache,
    ticker: Option<Ticker>,
    last_tick_ms: Option<f64>,
    /// The rect given to the last `draw`; events arrive in the same coordinate space.
    pub rect: Rect,
    /// Reduce motion as the view last saw it (`Cx::reduced_motion`).
    pub reduced: bool,
    shown: bool,
}

impl Default for Base {
    fn default() -> Base {
        Base { calls: Calls::default(), player: Player::new(), images: ImageCache::default(), ticker: None, last_tick_ms: None, rect: Rect::default(), reduced: false, shown: false }
    }
}

impl Base {
    pub fn new() -> Base {
        Base::default()
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    /// The tab became visible: subscribe to `events`, start the ticker (`tick_ms` period, 0 = none).
    pub fn show(&mut self, cx: &mut Cx, events: &[&str], tick_ms: u64) {
        self.shown = true;
        self.reduced = cx.reduced_motion;
        for e in events {
            cx.subscribe(e);
        }
        if tick_ms > 0 && self.ticker.is_none() {
            self.ticker = Some(Ticker::start(cx.waker(), Duration::from_millis(tick_ms)));
            self.last_tick_ms = Some(cx.now_ms);
        }
    }

    /// The tab is hidden: stop the ticker (the subscriptions stay, they are cheap and die with the window).
    pub fn hide(&mut self) {
        self.shown = false;
        self.ticker = None;
        self.last_tick_ms = None;
    }

    /// Sort an incoming message. Replies are matched to their command; images are stored.
    pub fn classify(&mut self, msg: Msg, cx: &Cx) -> Incoming {
        self.reduced = cx.reduced_motion;
        match msg {
            Msg::Event { name, payload, .. } => Incoming::Event { name, payload },
            Msg::Reply { ticket, result } => match self.calls.claim(ticket) {
                Some((cmd, args)) => Incoming::Result { cmd, args, result },
                None => Incoming::Other(Msg::Reply { ticket, result }),
            },
            Msg::Custom(b) => {
                if b.is::<Tick>() {
                    let now = cx.now_ms;
                    let dt = self.last_tick_ms.map_or(0.0, |l| (now - l).max(0.0));
                    self.last_tick_ms = Some(now);
                    return Incoming::Tick(dt);
                }
                match b.downcast::<ImageReady>() {
                    Ok(r) => Incoming::Image(self.images.ready(*r)),
                    Err(b) => Incoming::Other(Msg::Custom(b)),
                }
            }
            other => Incoming::Other(other),
        }
    }

    /// Route the model's `Fx`: animation cues start on the player, island-bound ones go to the host as `TabFx`, the rest comes back as `Local`.
    pub fn route_fx(&mut self, cx: &mut Cx, fxs: Vec<Fx>) -> Vec<Local> {
        let mut local = Vec::new();
        for fx in fxs {
            match fx {
                Fx::Anim { target, anim } => self.player.play(&target, anim, self.reduced || cx.reduced_motion),
                Fx::FocusSearch => local.push(Local::FocusSearch),
                Fx::BlurSearch => local.push(Local::BlurSearch),
                Fx::ScrollTo { target } => local.push(Local::ScrollTo(target)),
                other => {
                    if let Some(t) = TabFx::from_fx(&other) {
                        cx.post(Msg::Custom(Box::new(t)));
                    }
                }
            }
        }
        if self.player.busy() {
            cx.request_frame();
        }
        local
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_lock;
    use crate::ui::view::Services;
    use pill_ui_core::panel::a::Anim;

    #[test]
    fn calls_map_replies_back_to_their_command_and_arguments() {
        let (svc, fakes) = Services::fake();
        fakes.api.reply("media_get", Ok(json!({ "title": "x" })));
        let (mut cx, q) = Cx::headless_queue(svc);
        let mut b = Base::new();
        b.calls.run(&mut cx, vec![BackendCall::new("media_get", json!({ "a": 1 }))]);
        assert_eq!(b.calls.in_flight(), 1);
        let (_, m) = q.drain().pop().unwrap();
        match b.classify(m, &cx) {
            Incoming::Result { cmd, args, result } => {
                assert_eq!(cmd, "media_get");
                assert_eq!(args, json!({ "a": 1 }));
                assert_eq!(result.unwrap(), json!({ "title": "x" }));
            }
            _ => panic!("not a result"),
        }
        assert_eq!(b.calls.in_flight(), 0);
    }

    #[test]
    fn fx_split_island_bound_local_and_animation() {
        let _g = motion_lock();
        glasskit::ease::force_reduced(Some(false));
        let (mut cx, q) = Cx::headless_queue(Services::null());
        let mut b = Base::new();
        let local = b.route_fx(
            &mut cx,
            vec![Fx::Collapse { via: None }, Fx::FocusSearch, Fx::ScrollTo { target: "row:2".into() }, Fx::Anim { target: "title".into(), anim: Anim::text_swap() }],
        );
        glasskit::ease::force_reduced(None);
        assert_eq!(local, vec![Local::FocusSearch, Local::ScrollTo("row:2".into())]);
        let got = q.drain();
        assert_eq!(got.len(), 1);
        match &got[0].1 {
            Msg::Custom(c) => assert_eq!(c.downcast_ref::<TabFx>(), Some(&TabFx::Collapse { via: None })),
            _ => panic!(),
        }
        assert!(cx.effects().frame, "a running cue asks for frames");
    }

    #[test]
    fn the_ticker_posts_ticks_until_dropped() {
        let (cx, q) = Cx::headless_queue(Services::null());
        let t = Ticker::start(cx.waker(), Duration::from_millis(5));
        std::thread::sleep(Duration::from_millis(60));
        drop(t);
        let n = q.drain().len();
        assert!(n >= 2, "{n}");
        std::thread::sleep(Duration::from_millis(30));
        assert!(q.drain().len() <= 1, "stopped");
    }
}
