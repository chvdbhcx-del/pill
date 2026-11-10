//! Small stateful flows shared by the set-B views, all on `ui::liquid`: [`Lit`] (a yes / no as a glide), [`Swap`] (a value that cross-fades into
//! the next), [`LiquidBar`] / [`LiquidSpinner`] (progress with a flowing fill and fade) and [`FlowSwitch`] (the switch, knob on a pop spring).
use crate::ui::liquid::{Feel, Flow};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, MouseButton, Response};
use glasskit::theme::Theme;


/// Heights that grow from / shrink to nothing: the [`Feel::FLOW`] time without its bounce (a height cannot dip below zero).
pub const GROW: Feel = Feel::FADE.within(Feel::FLOW.settle_ms);

/// A yes / no state shown as a flow 0..1 (a highlight, a selected look, a mix of two colours). The first `set` lands at once, so a freshly
/// built view never animates from a wrong start; later changes glide and can turn around mid-way.
pub struct Lit {
    f: Flow,
    seen: bool,
}

impl Lit {
    pub fn new(feel: Feel) -> Lit {
        Lit { f: Flow::new(0.0, feel), seen: false }
    }

    pub fn set(&mut self, on: bool) {
        self.set_to(on as u8 as f32);
    }

    /// Like [`Lit::set`] for a target other than 0 / 1.
    pub fn set_to(&mut self, t: f32) {
        if !self.seen {
            self.seen = true;
            self.f.jump(t);
        }
        self.f.to(t);
    }

    /// Held inside 0..1 (for colour mixes and opacities).
    pub fn at(&self) -> f32 {
        self.f.at01()
    }

    /// As it is, bounce included (for scales and offsets).
    pub fn raw(&self) -> f32 {
        self.f.at()
    }

    pub fn moving(&self) -> bool {
        self.f.moving()
    }

    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.f.step(dt_ms);
        self.f.moving()
    }
}

/// A value that is swapped (a label, an icon name): the old one fades out while the new one fades in. Paint both with [`Swap::paint`].
pub struct Swap<T> {
    cur: Option<T>,
    prev: Option<T>,
    k: Flow,
}

impl<T: Clone + PartialEq> Default for Swap<T> {
    fn default() -> Self {
        Swap { cur: None, prev: None, k: Flow::new(1.0, Feel::FADE) }
    }
}

impl<T: Clone + PartialEq> Swap<T> {
    pub fn new() -> Swap<T> {
        Swap::default()
    }

    /// Show `v`: the first value is there at once, a different one cross-fades in.
    pub fn set(&mut self, v: &T) {
        match &self.cur {
            None => self.cur = Some(v.clone()),
            Some(c) if c == v => {}
            Some(c) => {
                self.prev = Some(c.clone());
                self.cur = Some(v.clone());
                self.k.jump(0.0);
                self.k.to(1.0);
            }
        }
    }

    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.k.step(dt_ms);
        if !self.k.moving() {
            self.prev = None;
        }
        self.k.moving()
    }

    pub fn moving(&self) -> bool {
        self.k.moving()
    }

    /// Calls `f(value, opacity)` for the outgoing value (if any) and the current one.
    pub fn paint(&self, mut f: impl FnMut(&T, f32)) {
        let k = self.k.at01();
        if let Some(p) = &self.prev {
            f(p, 1.0 - k);
        }
        if let Some(c) = &self.cur {
            f(c, if self.prev.is_some() { k } else { 1.0 });
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------------------------------------------------------------------

/// 0.5 - 0.5 cos: a smooth 0 -> 1 -> 0 over one unit of phase.
fn breathe(phase: f32) -> f32 {
    0.5 - 0.5 * (phase * std::f32::consts::TAU).cos()
}

/// A progress bar: determinate (the fill follows the reported value with inertia, so chunked download reports glide) or indeterminate (a
/// segment travels across, a periodic loop), fading in and out. The look of `glasskit::controls::ProgressBar`.
pub struct LiquidBar {
    value: Option<f32>,
    fill: Flow,
    vis: Flow,
    phase: f32,
    want: bool,
}

impl LiquidBar {
    /// Starts hidden and indeterminate; `set_visible(true)` fades it in.
    pub fn hidden() -> LiquidBar {
        LiquidBar { value: None, fill: Flow::new(0.0, Feel::DRAG), vis: Flow::new(0.0, Feel::FADE), phase: 0.0, want: false }
    }

    pub fn set_value(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if self.value.is_none() {
            self.fill.jump(0.0);
        }
        self.value = Some(v);
        self.fill.to(v);
    }

    pub fn set_indeterminate(&mut self) {
        self.value = None;
    }

    /// The reported value (None while indeterminate).
    pub fn value(&self) -> Option<f32> {
        self.value
    }

    pub fn set_visible(&mut self, on: bool) {
        self.want = on;
        self.vis.to(on as u8 as f32);
    }

    /// True while something moves (an indeterminate bar always does, while it is shown).
    pub fn step(&mut self, dt: f32) -> bool {
        let ms = dt as f64 * 1000.0;
        self.fill.step(ms);
        self.vis.step(ms);
        let mut busy = self.fill.moving() || self.vis.moving();
        if self.value.is_none() && (self.want || self.vis.at() > 0.0) {
            self.phase = (self.phase + dt / 1.4).fract();
            busy = true;
        }
        busy
    }

    pub fn draw(&self, cv: &Canvas, th: &Theme, rect: Rect) {
        let k = self.vis.at01();
        if k <= 0.0 {
            return;
        }
        let rad = rect.h / 2.0;
        cv.with_opacity(k, |cv| {
            cv.fill_rr(rect, rad, th.track);
            match self.value {
                Some(_) => {
                    let f = self.fill.at01();
                    if f > 0.0 {
                        cv.fill_rr(Rect::new(rect.x, rect.y, (rect.w * f).max(rect.h), rect.h), rad, th.accent);
                    }
                }
                None if glasskit::ease::reduced() => {
                    let a = 0.35 + 0.65 * breathe(self.phase);
                    cv.fill_rr(Rect::new(rect.x + rect.w * 0.3, rect.y, rect.w * 0.4, rect.h), rad, th.accent.fade(a));
                }
                None => {
                    // the segment enters from the left edge and leaves through the right one, slowest at both ends
                    let e = 0.5 - 0.5 * (self.phase * std::f32::consts::PI).cos();
                    let w = rect.w * 0.4;
                    let x = rect.x - w + (rect.w + w) * e;
                    cv.with_clip_rr(rect, rad, |cv| cv.fill_rr(Rect::new(x, rect.y, w, rect.h), rad, th.accent));
                }
            }
        });
    }
}

/// The wait spinner: a turning arc whose sweep breathes (periodic), fading in / out when it starts / stops. Look of `glasskit::controls::Spinner`.
pub struct LiquidSpinner {
    phase: f32,
    active: bool,
    vis: Flow,
}

impl LiquidSpinner {
    pub fn new() -> LiquidSpinner {
        LiquidSpinner { phase: 0.0, active: false, vis: Flow::new(0.0, Feel::FADE) }
    }

    pub fn set_active(&mut self, on: bool) {
        self.active = on;
        self.vis.to(on as u8 as f32);
    }

    pub fn step(&mut self, dt: f32) -> bool {
        self.vis.step(dt as f64 * 1000.0);
        let mut busy = self.vis.moving();
        if self.active || self.vis.at() > 0.0 {
            self.phase += dt;
            if self.phase > 12.6 {
                self.phase -= 12.6; // both periods divide 12.6 s: no visible jump
            }
            busy = true;
        }
        busy
    }

    pub fn draw(&self, cv: &Canvas, th: &Theme, rect: Rect) {
        let k = self.vis.at01();
        if k <= 0.0 {
            return;
        }
        let d = rect.w.min(rect.h);
        let w = (d / 8.0).clamp(1.5, 4.0);
        let r = d / 2.0 - w / 2.0;
        let c = rect.center();
        cv.with_opacity(k, |cv| {
            cv.stroke_circle(c, r, w, th.track);
            if glasskit::ease::reduced() {
                cv.arc(c, r, w, 0.0, 270.0, th.accent.fade(0.4 + 0.6 * breathe((self.phase / 1.6).fract())));
            } else {
                let rot = (self.phase / 0.9).fract() * 360.0;
                let sweep = 90.0 + 180.0 * breathe((self.phase / 1.4).fract());
                cv.arc(c, r, w, rot, sweep, th.accent);
            }
        });
    }
}

impl Default for LiquidSpinner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------------------------------------------------------------------

/// The on / off switch: the knob rides a pop spring (a lively settle), the track colour fades, the knob stretches while held. The look of
/// `glasskit::controls::Toggle`; a click needs a press and a release on the switch.
pub struct FlowSwitch {
    on: bool,
    knob: Flow,
    tint: Flow,
    press: Flow,
    armed: bool,
    pub enabled: bool,
}

impl FlowSwitch {
    pub fn new(on: bool) -> FlowSwitch {
        let v = on as u8 as f32;
        FlowSwitch { on, knob: Flow::new(v, Feel::POP), tint: Flow::new(v, Feel::FADE), press: Flow::new(0.0, Feel::SNAP), armed: false, enabled: true }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// `animated` false lands at once (the first state from settings).
    pub fn set_on(&mut self, on: bool, animated: bool) {
        self.on = on;
        let v = on as u8 as f32;
        self.knob.to(v);
        self.tint.to(v);
        if !animated {
            self.knob.jump(v);
            self.tint.jump(v);
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.armed = false;
            self.press.to(0.0);
        }
    }

    pub fn event(&mut self, ev: &InputEvent, rect: Rect) -> Response {
        let mut r = Response::NONE;
        if !self.enabled {
            return r;
        }
        match ev {
            InputEvent::MouseMove { pos, .. } => {
                let over = rect.contains(pos.0, pos.1);
                if over || self.armed {
                    r.cursor = Some(Cursor::Hand);
                }
                if self.armed {
                    r.capture = true;
                    r.redraw = true;
                    self.press.to(over as u8 as f32);
                }
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } if rect.contains(pos.0, pos.1) => {
                self.armed = true;
                self.press.to(1.0);
                r = Response { consumed: true, redraw: true, capture: true, focus: true, cursor: Some(Cursor::Hand), ..Response::NONE };
            }
            InputEvent::MouseUp { pos, button: MouseButton::Left, .. } if self.armed => {
                self.armed = false;
                self.press.to(0.0);
                r = Response { consumed: true, redraw: true, ..Response::NONE };
                if rect.contains(pos.0, pos.1) {
                    self.set_on(!self.on, true);
                    r.changed = true;
                }
            }
            _ => {}
        }
        r
    }

    pub fn step(&mut self, dt: f32) -> bool {
        let ms = dt as f64 * 1000.0;
        self.knob.step(ms);
        self.tint.step(ms);
        self.press.step(ms);
        self.knob.moving() || self.tint.moving() || self.press.moving()
    }

    pub fn draw(&self, cv: &Canvas, th: &Theme, rect: Rect) {
        let m = &th.m;
        let b = rect.centered(m.sw_w, m.sw_h);
        let travel = m.sw_w - m.sw_knob - 2.0 * m.sw_inset;
        let grow = (m.sw_press_grow * self.press.at()).max(0.0);
        let p = self.knob.at();
        let w = m.sw_knob + grow;
        // the far edge stays flush while the knob stretches: on the right the knob grows to the left
        let x = b.x + m.sw_inset + travel * p - grow * p.clamp(0.0, 1.0);
        let track = th.switch_off.mix(th.accent, self.tint.at01());
        cv.with_opacity(if self.enabled { 1.0 } else { 0.45 }, |cv| {
            cv.fill_rr(b, b.h / 2.0, track);
            cv.fill_rr(Rect::new(x, b.cy() - m.sw_knob / 2.0, w, m.sw_knob), m.sw_knob / 2.0, th.thumb);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use glasskit::input::{Buttons, Modifiers};

    fn at(x: f32) -> (f32, f32) {
        (x, 10.0)
    }

    #[test]
    fn lit_lands_on_its_first_value_then_glides_and_idles() {
        let _m = motion_on();
        let mut l = Lit::new(Feel::FADE);
        l.set(true);
        assert_eq!(l.at(), 1.0, "the first value is there at once");
        assert!(!l.moving());
        l.set(false);
        assert_eq!(l.at(), 1.0, "no jump");
        assert!(l.step(16.0) && l.at() < 1.0 && l.at() > 0.9);
        let mut t = 16.0;
        while l.step(8.0) {
            t += 8.0;
            assert!(t < 500.0, "settles");
        }
        assert_eq!(l.at(), 0.0);
    }

    #[test]
    fn the_switch_knob_springs_over_and_settles_and_a_click_needs_both_halves() {
        let _m = motion_on();
        let mut s = FlowSwitch::new(false);
        let rect = Rect::new(0.0, 0.0, 40.0, 24.0);
        let mv = InputEvent::MouseMove { pos: at(20.0), mods: Modifiers::NONE, buttons: Buttons::NONE };
        let down = InputEvent::MouseDown { pos: at(20.0), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 };
        let up = InputEvent::MouseUp { pos: at(20.0), button: MouseButton::Left, mods: Modifiers::NONE };
        assert_eq!(s.event(&mv, rect).cursor, Some(Cursor::Hand));
        assert!(!s.event(&down, rect).changed && !s.on(), "the click completes on release");
        assert!(s.event(&up, rect).changed && s.on());
        let (mut peak, mut ms) = (0.0f32, 0.0f32);
        while s.step(1.0 / 120.0) {
            peak = peak.max(s.knob.at());
            ms += 1000.0 / 120.0;
            assert!(ms < 1000.0, "settles");
        }
        assert!(peak > 1.0 && peak < 1.3, "a lively overshoot: {peak}");
        assert_eq!(s.knob.at(), 1.0);
        s.set_enabled(false);
        assert!(!s.event(&down, rect).consumed);
    }

    #[test]
    fn the_bar_fill_glides_without_overshoot_and_a_hidden_bar_costs_nothing() {
        let _m = motion_on();
        let mut b = LiquidBar::hidden();
        assert!(!b.step(0.016), "hidden: idle");
        b.set_visible(true);
        b.set_value(0.0);
        b.step(0.5);
        b.set_value(0.6);
        let mut prev = 0.0;
        let mut frames = 0;
        while b.step(1.0 / 60.0) {
            let f = b.fill.at();
            assert!(f >= prev - 1e-4 && f <= 0.6 + 0.02, "follows without a jump: {f}");
            prev = f;
            frames += 1;
            assert!(frames < 120, "settles");
        }
        b.set_visible(false);
        for _ in 0..40 {
            b.step(0.016);
        }
        assert!(!b.step(0.016));
    }
}
