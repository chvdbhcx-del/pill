//! The panel region: hosts the active tab view (`ui::tabs::create(id)`) under the header with the slide / fade of `pill.css`
//! (`view-in`, `view-slide-*`, `view-out`, `view-away-*`). A tab that is not ported yet (`create` answers `None`) gets a neat placeholder.
//!
//! Tab views share the island's `Cx`. What they may ask of the window is filtered (`guarded`): the island owns the hit region, the gaze
//! feed, pass-through and the window commands; a tab may request frames, the cursor, keyboard focus, other windows, a drop zone and the IME rect.
use crate::ui::liquid::{Feel, Flow};
use crate::ui::view::{Cx, DropEvent, Effects, Msg, View};
use glasskit::canvas::{translation, Canvas};
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, Response};
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::shell::types::TabDir;
use serde_json::Value;
use std::collections::HashMap;

/// Re-applies `fx` to `cx`. `tab` = filter to what a tab view may ask.
pub fn reapply(cx: &mut Cx, fx: Effects, tab: bool) {
    if fx.frame {
        cx.request_frame();
    }
    if fx.ambient {
        cx.ambient_frame();
    }
    if let Some(c) = fx.cursor {
        cx.set_cursor(c);
    }
    if let Some(z) = fx.drop_zone {
        cx.set_drop_zone(z);
    }
    if let Some(f) = fx.focus {
        cx.set_focus(f);
    }
    for o in fx.opens {
        cx.open_window(&o.name, o.args);
    }
    if let Some(r) = fx.ime_rect {
        cx.set_ime_rect(r);
    }
    if tab {
        return;
    }
    if let Some(h) = fx.hit_region {
        cx.set_hit_region(h);
    } else if fx.hit_all {
        cx.clear_hit_region();
    }
    if let Some(c) = fx.caption_region {
        cx.set_caption_region(c);
    }
    if let Some(p) = fx.pass_through {
        cx.set_pass_through(p);
    }
    if let Some(g) = fx.gaze {
        cx.set_gaze(g);
    }
    if fx.show {
        cx.show_self();
    }
    if fx.hide {
        cx.hide_self();
    }
    if fx.close {
        cx.close_self();
    }
    if let Some(t) = fx.title {
        cx.set_title(&t);
    }
    if let Some(w) = fx.window_cmd {
        cx.window_cmd(w);
    }
    if let Some(f) = fx.fps_cap {
        cx.set_fps_cap(f);
    }
}

/// Runs `f` (a call into a tab view) so that only the effects a tab may have reach the window.
pub fn guarded<R>(cx: &mut Cx, f: impl FnOnce(&mut Cx) -> R) -> R {
    let before = cx.take_effects();
    let r = f(cx);
    let fx = cx.take_effects();
    reapply(cx, before, false);
    reapply(cx, fx, true);
    r
}

/// Stand-in for a tab whose native view does not exist yet: its icon, its name and a calm shimmer.
pub struct Placeholder {
    pub title: String,
    pub icon: &'static str,
    t: f64,
    gpu: super::gpu::Gpu,
}

impl Placeholder {
    pub fn new(id: &str, ru: bool) -> Placeholder {
        Placeholder { title: super::chrome::tab_title(id, ru), icon: super::chrome::tab_icon(id), t: 0.0, gpu: super::gpu::Gpu::new() }
    }
}

impl View for Placeholder {
    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.t += dt_ms;
        !cx.reduced_motion
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        let c = rect.center();
        self.gpu.icon(cv, self.icon, Rect::new(c.0 - 14.0, c.1 - 48.0, 28.0, 28.0), th.icon, 1.6);
        cv.text(Rect::new(rect.x, c.1 - 14.0, rect.w, 20.0), &self.title, Style::new(14.0, 600).center(), th.text2);
        let sh = |i: f32| -> f32 { if cx.reduced_motion { 0.5 } else { 0.5 + 0.5 * ((self.t / 700.0) as f32 - i * 0.6).sin() } };
        for (i, w) in [160.0f32, 120.0, 90.0].iter().enumerate() {
            cv.fill_rr(Rect::new(c.0 - w / 2.0, c.1 + 14.0 + i as f32 * 14.0, *w, 8.0), 4.0, th.raised.mix(th.raised2, sh(i as f32)));
        }
    }
}

/// A tab view and where it stands in the switch: opacity and sideways offset are flows, so a switch that is cancelled half-way turns around
/// from where the views are instead of restarting.
struct Slot {
    view: Box<dyn View>,
    placeholder: bool,
    a: Flow,
    dx: Flow,
}

const ENTER_FADE: Feel = Feel::FADE.within(190.0);
const LEAVE_FADE: Feel = Feel::FADE.within(110.0);
const ENTER_SLIDE: Feel = Feel::FLOW.within(300.0);
const LEAVE_SLIDE: Feel = Feel::FLOW.within(220.0);
/// How far a view travels sideways when it enters or leaves (css px).
const SHIFT: f32 = 10.0;

impl Slot {
    fn new(view: Box<dyn View>, placeholder: bool) -> Slot {
        Slot { view, placeholder, a: Flow::new(0.0, ENTER_FADE), dx: Flow::px(0.0, ENTER_SLIDE) }
    }

    fn visible(&self) -> bool {
        self.a.at() > 0.003 || self.a.moving()
    }

    fn set_feel(&mut self, fade: Feel, slide: Feel) {
        if self.a.feel() != fade {
            self.a.set_feel(fade);
        }
        if self.dx.feel() != slide {
            self.dx.set_feel(slide);
        }
    }
}

#[derive(Default)]
pub struct TabHost {
    slots: HashMap<String, Slot>,
    pub cur: Option<String>,
    /// the `ai_actions` overlay over the clipboard tab (created on the first open, then kept: it paints nothing while closed)
    overlay: Option<Box<dyn View>>,
    /// true from `OpenClipAi` until the overlay says `OverlayClosed`: Esc and clicks go to it first
    pub overlay_open: bool,
    pub dir: Option<TabDir>,
    /// the last `step` kept the frames coming only for an ambient animation of a tab (see `Cx::ambient_frame`)
    ambient: bool,
}

/// The sideways offset a view comes from when the switch goes in direction `dir` (it leaves towards the opposite side).
fn from_dx(dir: TabDir) -> f32 {
    if dir == TabDir::Right {
        SHIFT
    } else {
        -SHIFT
    }
}

impl TabHost {
    pub fn new() -> TabHost {
        TabHost::default()
    }

    pub fn has(&self, id: &str) -> bool {
        self.slots.contains_key(id)
    }

    /// Creates the views of the tabs with background duties (docs/native-tabs-a.md section 3) and lets them subscribe: `on_show` then `on_hide`,
    /// so their events and replies keep flowing while they are not the visible tab, and nothing ticks or draws.
    pub fn warm(&mut self, ids: &[&str], cx: &mut Cx) {
        for id in ids {
            if self.slots.contains_key(*id) {
                continue;
            }
            if let Some(mut view) = crate::ui::tabs::create(id) {
                guarded(cx, |cx| {
                    view.on_show(cx);
                    view.on_hide(cx);
                });
                self.slots.insert(id.to_string(), Slot::new(view, false));
            }
        }
    }

    /// True when the tab shows the stand-in (no native view for that id).
    pub fn is_placeholder(&self, id: &str) -> bool {
        self.slots.get(id).is_some_and(|s| s.placeholder)
    }

    /// Every view that exists: the tabs and the overlay.
    fn all_views(&mut self) -> impl Iterator<Item = &mut Box<dyn View>> {
        self.slots.values_mut().map(|s| &mut s.view).chain(self.overlay.iter_mut())
    }

    /// Opens the overlay `name` (`ai_actions`) with `Msg::Open(args)`.
    pub fn open_overlay(&mut self, name: &str, args: Value, cx: &mut Cx) {
        if self.overlay.is_none() {
            self.overlay = crate::ui::tabs::b::create_overlay(name);
            if let Some(o) = self.overlay.as_mut() {
                guarded(cx, |cx| o.on_show(cx));
            }
        }
        if let Some(o) = self.overlay.as_mut() {
            guarded(cx, |cx| o.message(Msg::Open(args), cx));
            self.overlay_open = true;
        }
    }

    /// The tab that owns the overlay asks to close it (or the island collapsed): the overlay closes itself (it cancels a running request) and
    /// reports `OverlayClosed`.
    pub fn close_overlay(&mut self, cx: &mut Cx) {
        if let Some(o) = self.overlay.as_mut() {
            guarded(cx, |cx| o.on_hide(cx));
        }
    }

    pub fn overlay_closed(&mut self) {
        self.overlay_open = false;
    }

    /// Shows `id` (creating its view the first time); `dir` is the slide direction of this switch. The view that was current flows out the
    /// other way; one that is still on its way out (a quick switch back) turns around from where it is.
    pub fn show(&mut self, id: &str, dir: TabDir, ru: bool, cx: &mut Cx, _now: f64) {
        if !self.slots.contains_key(id) {
            let real = crate::ui::tabs::create(id);
            let placeholder = real.is_none();
            let view = real.unwrap_or_else(|| Box::new(Placeholder::new(id, ru)));
            self.slots.insert(id.to_string(), Slot::new(view, placeholder));
        }
        let from = from_dx(dir);
        if let Some(old) = self.cur.take() {
            if old != id {
                if let Some(o) = self.slots.get_mut(&old) {
                    o.set_feel(LEAVE_FADE, LEAVE_SLIDE);
                    o.a.to(0.0);
                    o.dx.to(-from);
                }
            }
        }
        self.cur = Some(id.to_string());
        self.dir = Some(dir);
        let reduced = cx.reduced_motion;
        if let Some(s) = self.slots.get_mut(id) {
            if !s.visible() {
                s.dx.jump(from); // a fresh entry starts beside its place
            }
            s.set_feel(ENTER_FADE, ENTER_SLIDE);
            s.a.to(1.0);
            s.dx.to(0.0);
            guarded(cx, |cx| s.view.on_show(cx));
        }
        if reduced {
            for (k, s) in self.slots.iter_mut() {
                let here = self.cur.as_deref() == Some(k.as_str());
                s.a.jump(if here { 1.0 } else { 0.0 });
                s.dx.jump(0.0);
            }
        }
    }

    pub fn hide(&mut self, id: &str, cx: &mut Cx) {
        if let Some(s) = self.slots.get_mut(id) {
            guarded(cx, |cx| s.view.on_hide(cx));
        }
        if self.cur.as_deref() == Some(id) && false {
            self.cur = None;
        }
    }

    fn cur_view(&mut self) -> Option<&mut Box<dyn View>> {
        let id = self.cur.clone()?;
        self.slots.get_mut(&id).map(|s| &mut s.view)
    }

    /// An input event in the tab view's own coordinates.
    pub fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        if self.overlay_open {
            if let Some(o) = self.overlay.as_mut() {
                let r = guarded(cx, |cx| o.event(ev, cx));
                if r.consumed {
                    return r;
                }
            }
        }
        match self.cur_view() {
            Some(v) => guarded(cx, |cx| v.event(ev, cx)),
            None => Response::NONE,
        }
    }

    pub fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        match self.cur_view() {
            Some(v) => guarded(cx, |cx| v.on_drop(ev, cx)),
            None => Response::NONE,
        }
    }

    /// A backend event reaches every tab view that exists (they subscribed through the shared `Cx`).
    pub fn event_msg(&mut self, name: &str, payload: &Value, latest: bool, cx: &mut Cx) -> bool {
        let mut redraw = false;
        for v in self.all_views() {
            let m = Msg::Event { name: name.to_string(), payload: payload.clone(), latest };
            redraw |= guarded(cx, |cx| v.message(m, cx)).redraw;
        }
        redraw
    }

    /// Replies and custom messages: tried on the active view first, a reply goes to all (tickets are unique).
    pub fn other_msg(&mut self, msg: Msg, cx: &mut Cx) -> bool {
        match msg {
            Msg::Reply { ticket, result } => {
                let mut r = false;
                for v in self.all_views() {
                    r |= guarded(cx, |cx| v.message(Msg::Reply { ticket, result: result.clone() }, cx)).redraw;
                }
                r
            }
            Msg::Open(v) => {
                let mut r = false;
                for s in self.slots.values_mut() {
                    r |= guarded(cx, |cx| s.view.message(Msg::Open(v.clone()), cx)).redraw;
                }
                r
            }
            // ticks, image-decode answers and the notifications record are addressed by type: every view looks at it, the others ignore it
            Msg::Custom(b) => self.custom(b, cx),
            Msg::Event { .. } => false,
        }
    }

    /// A `Msg::Custom` cannot be cloned, so it goes where it belongs: decoded images to every view (a cache that never asked ignores them),
    /// a cover-colour answer to home, a file-picker answer to chat, anything else (ticks) to the visible tab.
    fn custom(&mut self, b: Box<dyn std::any::Any + Send>, cx: &mut Cx) -> bool {
        let b = match b.downcast::<crate::ui::tabs::a::draw::ImageReady>() {
            Ok(r) => {
                let mut redraw = false;
                for v in self.all_views() {
                    redraw |= guarded(cx, |cx| v.message(Msg::Custom(Box::new((*r).clone())), cx)).redraw;
                }
                return redraw;
            }
            Err(b) => b,
        };
        let owner = if crate::ui::tabs::a::is_meters(&*b) {
            Some("controls".to_string())
        } else if crate::ui::tabs::a::is_cover_ready(&*b) {
            Some("home".to_string())
        } else if crate::ui::tabs::b::is_picked(&*b) {
            Some("chat".to_string())
        } else {
            self.cur.clone()
        };
        match owner.and_then(|id| self.slots.get_mut(&id)) {
            Some(s) => guarded(cx, |cx| s.view.message(Msg::Custom(b), cx)).redraw,
            None => false,
        }
    }

    /// Has the tab `id` a view (created by `show` or `warm`)?
    pub fn exists(&self, id: &str) -> bool {
        self.slots.contains_key(id)
    }

    /// Delivers a custom message addressed to the tab `id` (the island's `ToastRec` for the notifications tab).
    pub fn custom_to(&mut self, id: &str, b: Box<dyn std::any::Any + Send>, cx: &mut Cx) -> bool {
        match self.slots.get_mut(id) {
            Some(s) => guarded(cx, |cx| s.view.message(Msg::Custom(b), cx)).redraw,
            None => false,
        }
    }

    /// Steps the flows of the switch and the views that are visible (the current one and those still flowing out); true while any asks for frames.
    pub fn step(&mut self, dt_ms: f64, _now: f64, cx: &mut Cx) -> bool {
        let mut keep = false;
        let mut amb = false;
        let cur = self.cur.clone();
        for (id, s) in self.slots.iter_mut() {
            s.a.step(dt_ms);
            s.dx.step(dt_ms);
            keep |= s.a.moving() || s.dx.moving();
            if Some(id) == cur.as_ref() || s.visible() {
                let more = guarded(cx, |cx| s.view.step(dt_ms, cx));
                if cx.take_ambient() {
                    amb |= more; // asks for frames only for an ambient animation
                } else {
                    keep |= more;
                }
            }
        }
        if let Some(o) = self.overlay.as_mut() {
            keep |= guarded(cx, |cx| o.step(dt_ms, cx));
            cx.take_ambient();
        }
        self.ambient = amb && !keep;
        keep
    }

    /// True when the frames the tabs asked for in the last `step` are all ambient (a drifting cloud): the window may pace them.
    pub fn ambient(&self) -> bool {
        self.ambient
    }

    /// Draws the outgoing and the incoming view into `views` (panel css px) of the current canvas transform.
    pub fn draw(&mut self, cv: &Canvas, th: &Theme, views: Rect, cx: &Cx, _now: f64, alpha: f32) {
        let local = Rect::new(0.0, 0.0, views.w, views.h);
        let cur = self.cur.clone();
        cv.with_clip(views, |cv| {
            // the ones flowing out first, the current one on top
            let mut order: Vec<&String> = self.slots.keys().filter(|k| Some(*k) != cur.as_ref()).collect();
            order.sort();
            let order: Vec<String> = order.into_iter().cloned().chain(cur.clone()).collect();
            for id in order {
                if let Some(s) = self.slots.get_mut(&id) {
                    let (a, dx) = (s.a.at01(), s.dx.at());
                    if a > 0.003 {
                        cv.with_opacity(a * alpha, |cv| cv.with_transform(translation(views.x + dx, views.y), |cv| s.view.draw(cv, th, local, cx)));
                    }
                }
            }
            // the overlay covers the whole body of the tab (it paints nothing while closed)
            if let Some(o) = self.overlay.as_mut() {
                cv.with_opacity(alpha, |cv| cv.with_transform(translation(views.x, views.y), |cv| o.draw(cv, th, local, cx)));
            }
        });
    }

    /// Is any flow of the switch still moving? (the views themselves may keep asking for frames)
    #[cfg(test)]
    pub fn flows_moving(&self) -> bool {
        self.slots.values().any(|s| s.a.moving() || s.dx.moving())
    }

    /// (opacity, dx) of a tab's view in the switch, for tests.
    #[cfg(test)]
    pub fn look(&self, id: &str) -> Option<(f32, f32)> {
        self.slots.get(id).map(|s| (s.a.at(), s.dx.at()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::CursorShape;

    #[test]
    fn a_tab_may_not_take_the_hit_region_or_the_window() {
        let mut cx = Cx::headless();
        cx.set_hit_region(vec![Rect::new(0.0, 0.0, 10.0, 10.0)]);
        guarded(&mut cx, |cx| {
            cx.set_hit_region(vec![]);
            cx.hide_self();
            cx.set_gaze(true);
            cx.set_cursor(CursorShape::Hand);
            cx.request_frame();
            cx.set_focus(true);
        });
        let fx = cx.take_effects();
        assert_eq!(fx.hit_region, Some(vec![Rect::new(0.0, 0.0, 10.0, 10.0)]), "the island's own hit region survives");
        assert!(!fx.hide && fx.gaze.is_none(), "{fx:?}");
        assert!(fx.frame && fx.cursor == Some(CursorShape::Hand) && fx.focus == Some(true));
    }

    /// A tab that keeps asking for frames; `ambient` says whether that is only for a breathing background.
    struct Breather(bool);
    impl View for Breather {
        fn step(&mut self, _dt_ms: f64, cx: &mut Cx) -> bool {
            if self.0 {
                cx.ambient_frame();
            }
            true
        }
        fn draw(&mut self, _cv: &Canvas, _th: &Theme, _rect: Rect, _cx: &Cx) {}
    }

    #[test]
    fn a_tab_that_only_breathes_is_reported_as_ambient_and_a_busy_one_is_not() {
        let _m = crate::ui::host::testing::motion_on();
        let mut cx = Cx::headless();
        for (ambient, keep) in [(true, false), (false, true)] {
            let mut h = TabHost::new();
            h.show("home", TabDir::Right, false, &mut cx, 0.0);
            frames(&mut h, &mut cx, 100);
            h.slots.insert("home".into(), Slot::new(Box::new(Breather(ambient)), false));
            h.slots.get_mut("home").unwrap().a.jump(1.0);
            assert_eq!(h.step(8.0, 0.0, &mut cx), keep, "ambient={ambient}");
            assert_eq!(h.ambient(), ambient);
            assert!(!cx.take_ambient(), "the host consumed the mark");
        }
        let mut h = TabHost::new();
        h.show("home", TabDir::Right, false, &mut cx, 0.0);
        frames(&mut h, &mut cx, 100);
        assert!(!h.ambient(), "a quiet host is not ambient");
    }

    fn frames(h: &mut TabHost, cx: &mut Cx, n: usize) {
        for _ in 0..n {
            h.step(8.0, 0.0, cx);
        }
    }

    #[test]
    fn a_switch_flows_in_the_direction_of_travel_and_idles_afterwards() {
        let _m = crate::ui::host::testing::motion_on();
        let mut cx = Cx::headless();
        cx.reduced_motion = false;
        let mut h = TabHost::new();
        h.show("home", TabDir::Right, false, &mut cx, 0.0);
        frames(&mut h, &mut cx, 100);
        assert_eq!(h.look("home"), Some((1.0, 0.0)), "the first view has settled");
        assert!(!h.flows_moving(), "idle");
        h.show("shelf", TabDir::Right, false, &mut cx, 1000.0);
        assert_eq!(h.look("shelf"), Some((0.0, 10.0)), "the new view starts beside its place, invisible");
        let mut last = h.look("shelf").unwrap();
        for _ in 0..8 {
            assert!(h.step(8.0, 0.0, &mut cx), "animating");
            let now = h.look("shelf").unwrap();
            assert!(now.0 >= last.0 && now.1 <= last.1 + 1e-3, "it only moves towards its place: {last:?} -> {now:?}");
            last = now;
        }
        assert!(last.0 > 0.0 && last.1 > 0.0 && last.1 < 10.0, "softly under way, not there yet: {last:?}");
        let (a, dx) = h.look("home").unwrap();
        assert!(a < 1.0 && dx < 0.0, "the old view leaves the other way: {a} {dx}");
        // ~1.5 x its feel is enough to be there and quiet
        frames(&mut h, &mut cx, 70);
        let (a, dx) = h.look("shelf").unwrap();
        assert!((a - 1.0).abs() < 0.01 && dx.abs() < 0.3, "{a} {dx}");
        assert_eq!(h.look("home").unwrap().0, 0.0);
        frames(&mut h, &mut cx, 100);
        assert!(!h.flows_moving(), "idle once everything has settled");
        // left switches the other way round
        h.show("home", TabDir::Left, false, &mut cx, 2000.0);
        assert_eq!(h.look("home").unwrap().1, -10.0);
    }

    #[test]
    fn a_cancelled_switch_turns_around_from_where_the_views_are() {
        let _m = crate::ui::host::testing::motion_on();
        let mut cx = Cx::headless();
        cx.reduced_motion = false;
        let mut h = TabHost::new();
        h.show("home", TabDir::Right, false, &mut cx, 0.0);
        frames(&mut h, &mut cx, 100);
        h.show("shelf", TabDir::Right, false, &mut cx, 1000.0);
        frames(&mut h, &mut cx, 6);
        let (home_a, home_dx) = h.look("home").unwrap();
        let (shelf_a, _) = h.look("shelf").unwrap();
        assert!(home_a > 0.0 && home_a < 1.0 && shelf_a > 0.0 && shelf_a < 1.0, "both are mid-flight");
        h.show("home", TabDir::Left, false, &mut cx, 1050.0);
        let (a, dx) = h.look("home").unwrap();
        assert_eq!((a, dx), (home_a, home_dx), "the returning view does not restart: it keeps its place");
        frames(&mut h, &mut cx, 3);
        let (a2, _) = h.look("home").unwrap();
        assert!(a2 > a - 0.2, "it turns around after at most a small carry-on: {a} -> {a2}");
        frames(&mut h, &mut cx, 150);
        assert_eq!(h.look("home").unwrap().0, 1.0);
        assert_eq!(h.look("shelf").unwrap().0, 0.0);
    }

    #[test]
    fn reduced_motion_swaps_the_views_at_once() {
        let _lock = crate::ui::host::testing::motion_lock();
        let mut cx = Cx::headless();
        cx.reduced_motion = true;
        let mut h = TabHost::new();
        h.show("home", TabDir::Right, false, &mut cx, 0.0);
        h.show("shelf", TabDir::Right, false, &mut cx, 1.0);
        assert_eq!(h.look("shelf"), Some((1.0, 0.0)));
        assert_eq!(h.look("home"), Some((0.0, 0.0)));
    }

    #[test]
    fn an_unported_tab_gets_the_placeholder_and_the_old_view_stays_alive_for_the_exit() {
        let _m = crate::ui::host::testing::motion_on();
        let mut cx = Cx::headless();
        cx.reduced_motion = false;
        let mut h = TabHost::new();
        h.show("home", TabDir::Right, false, &mut cx, 0.0);
        assert!(h.has("home") && h.cur.as_deref() == Some("home"));
        frames(&mut h, &mut cx, 100);
        h.show("shelf", TabDir::Right, false, &mut cx, 1000.0);
        assert!(h.has("home"));
        assert!(h.step(16.0, 1050.0, &mut cx), "animating");
        frames(&mut h, &mut cx, 150);
        assert!(!h.slots["home"].visible(), "the exit is over");
    }
}
