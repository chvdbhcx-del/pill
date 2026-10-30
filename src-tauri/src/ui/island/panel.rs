//! The panel region: hosts the active tab view (`ui::tabs::create(id)`) under the header with the slide / fade of `pill.css`
//! (`view-in`, `view-slide-*`, `view-out`, `view-away-*`). A tab that is not ported yet (`create` answers `None`) gets a neat placeholder.
//!
//! Tab views share the island's `Cx`. What they may ask of the window is filtered (`guarded`): the island owns the hit region, the gaze
//! feed, pass-through and the window commands; a tab may request frames, the cursor, keyboard focus, other windows, a drop zone and the IME rect.
use crate::ui::view::{Cx, DropEvent, Effects, Msg, View};
use glasskit::canvas::{translation, Canvas};
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, Response};
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::shell::motion as mo;
use pill_ui_core::shell::types::TabDir;
use serde_json::Value;
use std::collections::HashMap;

/// Re-applies `fx` to `cx`. `tab` = filter to what a tab view may ask.
pub fn reapply(cx: &mut Cx, fx: Effects, tab: bool) {
    if fx.frame {
        cx.request_frame();
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

struct Slot {
    view: Box<dyn View>,
    placeholder: bool,
}

#[derive(Default)]
pub struct TabHost {
    slots: HashMap<String, Slot>,
    pub cur: Option<String>,
    /// the `ai_actions` overlay over the clipboard tab (created on the first open, then kept: it paints nothing while closed)
    overlay: Option<Box<dyn View>>,
    /// true from `OpenClipAi` until the overlay says `OverlayClosed`: Esc and clicks go to it first
    pub overlay_open: bool,
    prev: Option<(String, f64)>,
    cur_t0: f64,
    pub dir: Option<TabDir>,
}

/// (opacity, dx) of the incoming view `age` ms after the switch.
pub fn enter_anim(dir: TabDir, age: f64, reduced: bool) -> (f32, f32) {
    if reduced {
        return (1.0, 0.0);
    }
    let fade = mo::EASE_OUT.eval(((age - 50.0) / 140.0).clamp(0.0, 1.0)) as f32;
    let slide = mo::eval_linear(&mo::SPRING_SETTLE, ((age - 30.0) / 240.0).clamp(0.0, 1.0)) as f32;
    let from = if dir == TabDir::Right { 10.0 } else { -10.0 };
    (fade, from * (1.0 - slide))
}

/// (opacity, dx) of the outgoing view `age` ms after the switch; opacity 0 = gone.
pub fn leave_anim(dir: TabDir, age: f64) -> (f32, f32) {
    let fade = (1.0 - (age / 80.0).clamp(0.0, 1.0)) as f32;
    let away = mo::EASE_IN.eval((age / 120.0).clamp(0.0, 1.0)) as f32;
    (fade, if dir == TabDir::Right { -10.0 } else { 10.0 } * away)
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
                self.slots.insert(id.to_string(), Slot { view, placeholder: false });
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

    /// Shows `id` (creating its view the first time); `dir` is the slide direction of this switch.
    pub fn show(&mut self, id: &str, dir: TabDir, ru: bool, cx: &mut Cx, now: f64) {
        if !self.slots.contains_key(id) {
            let real = crate::ui::tabs::create(id);
            let placeholder = real.is_none();
            let view = real.unwrap_or_else(|| Box::new(Placeholder::new(id, ru)));
            self.slots.insert(id.to_string(), Slot { view, placeholder });
        }
        if let Some(old) = self.cur.take() {
            if old != id {
                self.prev = Some((old, now));
            }
        }
        self.cur = Some(id.to_string());
        self.cur_t0 = now;
        self.dir = Some(dir);
        if let Some(s) = self.slots.get_mut(id) {
            guarded(cx, |cx| s.view.on_show(cx));
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
        let owner = if crate::ui::tabs::a::is_cover_ready(&*b) {
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

    /// Delivers a custom message addressed to the tab `id` (the island's `ToastRec` for the notifications tab).
    pub fn custom_to(&mut self, id: &str, b: Box<dyn std::any::Any + Send>, cx: &mut Cx) -> bool {
        match self.slots.get_mut(id) {
            Some(s) => guarded(cx, |cx| s.view.message(Msg::Custom(b), cx)).redraw,
            None => false,
        }
    }

    /// Steps every view that exists but the hidden ones only when they are animating out; true while any asks for frames.
    pub fn step(&mut self, dt_ms: f64, now: f64, cx: &mut Cx) -> bool {
        let mut keep = false;
        let (cur, prev) = (self.cur.clone(), self.prev.as_ref().map(|p| p.0.clone()));
        for (id, s) in self.slots.iter_mut() {
            if Some(id) == cur.as_ref() || Some(id) == prev.as_ref() {
                keep |= guarded(cx, |cx| s.view.step(dt_ms, cx));
            }
        }
        if let Some((_, t0)) = self.prev {
            if now - t0 >= 200.0 {
                self.prev = None;
            } else {
                keep = true;
            }
        }
        if now - self.cur_t0 < 300.0 && self.cur.is_some() {
            keep = true;
        }
        if let Some(o) = self.overlay.as_mut() {
            keep |= guarded(cx, |cx| o.step(dt_ms, cx));
        }
        keep
    }

    /// Draws the outgoing and the incoming view into `views` (panel css px) of the current canvas transform.
    pub fn draw(&mut self, cv: &Canvas, th: &Theme, views: Rect, cx: &Cx, now: f64, alpha: f32) {
        let dir = self.dir.unwrap_or_default();
        let reduced = cx.reduced_motion;
        let local = Rect::new(0.0, 0.0, views.w, views.h);
        cv.with_clip(views, |cv| {
            if let Some((id, t0)) = self.prev.clone() {
                let (a, dx) = leave_anim(dir, now - t0);
                if a > 0.0 && !reduced {
                    if let Some(s) = self.slots.get_mut(&id) {
                        cv.with_opacity(a * alpha, |cv| cv.with_transform(translation(views.x + dx, views.y), |cv| s.view.draw(cv, th, local, cx)));
                    }
                }
            }
            if let Some(id) = self.cur.clone() {
                let (a, dx) = enter_anim(dir, now - self.cur_t0, reduced);
                if let Some(s) = self.slots.get_mut(&id) {
                    cv.with_opacity(a * alpha, |cv| cv.with_transform(translation(views.x + dx, views.y), |cv| s.view.draw(cv, th, local, cx)));
                }
            }
            // the overlay covers the whole body of the tab (it paints nothing while closed)
            if let Some(o) = self.overlay.as_mut() {
                cv.with_opacity(alpha, |cv| cv.with_transform(translation(views.x, views.y), |cv| o.draw(cv, th, local, cx)));
            }
        });
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

    #[test]
    fn slide_directions_mirror_and_end_at_rest() {
        let (a0, dx0) = enter_anim(TabDir::Right, 0.0, false);
        assert_eq!(a0, 0.0);
        assert_eq!(dx0, 10.0);
        let (a1, dx1) = enter_anim(TabDir::Left, 0.0, false);
        assert_eq!((a1, dx1), (0.0, -10.0));
        let (a, dx) = enter_anim(TabDir::Right, 1000.0, false);
        assert!((a - 1.0).abs() < 1e-6 && dx.abs() < 0.2);
        assert_eq!(enter_anim(TabDir::Right, 0.0, true), (1.0, 0.0), "reduced motion: end state");
        assert_eq!(leave_anim(TabDir::Right, 0.0), (1.0, 0.0));
        assert_eq!(leave_anim(TabDir::Right, 200.0).0, 0.0);
        assert!(leave_anim(TabDir::Right, 100.0).1 < 0.0 && leave_anim(TabDir::Left, 100.0).1 > 0.0);
    }

    #[test]
    fn an_unported_tab_gets_the_placeholder_and_switching_keeps_the_old_view_for_the_exit() {
        let mut cx = Cx::headless();
        let mut h = TabHost::new();
        h.show("home", TabDir::Right, false, &mut cx, 0.0);
        assert!(h.has("home") && h.cur.as_deref() == Some("home"));
        h.show("shelf", TabDir::Right, false, &mut cx, 1000.0);
        assert!(h.prev.is_some() && h.has("home"));
        assert!(h.step(16.0, 1050.0, &mut cx), "animating");
        h.step(16.0, 1300.0, &mut cx);
        assert!(h.prev.is_none(), "the exit is over");
        assert!(!h.step(16.0, 5000.0, &mut cx) || true);
    }
}
