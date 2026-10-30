//! Plays the model's animation cues. `pill_ui_core::bar::Anim` is "start this move on that target"; the player turns each cue into a small
//! time-driven state (opacity, offset, scale, rotation, height factor...) that the drawing code asks for per target (`look`). Easing is glasskit's
//! CSS curves, so the moves match the web's `PillMotion`. Under reduced motion `play` drops every cue and nothing is ever in flight.
//! Frame rate is never touched: the owner calls `step(dt)` once per frame and keeps frames coming while `busy()`.
use glasskit::ease::{curves, Ease as Curve};
use pill_ui_core::bar::anim::{Anim, Ease, Kind, Target};

/// What the drawing code applies to an element: opacity, move, scale (about the element's own origin), rotation, height factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub alpha: f32,
    pub dx: f32,
    pub dy: f32,
    pub sx: f32,
    pub sy: f32,
    pub rot_deg: f32,
    /// 0..1 share of the natural height (Collapse / Expand)
    pub height: f32,
    /// 0..1 how much of a tick mark is drawn (DrawTick)
    pub draw: f32,
    /// 0..1 strength of the soft colour flash over a row (Flash)
    pub flash: f32,
}

impl Look {
    pub const REST: Look = Look { alpha: 1.0, dx: 0.0, dy: 0.0, sx: 1.0, sy: 1.0, rot_deg: 0.0, height: 1.0, draw: 1.0, flash: 0.0 };

    pub fn is_rest(&self) -> bool {
        *self == Look::REST
    }
}

#[derive(Clone, Debug)]
struct Active {
    cue: Anim,
    /// seconds since the cue started (the delay is part of it)
    t: f32,
}

fn curve(e: Ease) -> Curve {
    match e {
        Ease::Out => curves::EASE_OUT,
        Ease::OutExpo => curves::EASE_OUT_EXPO,
        Ease::In => curves::EASE_IN,
        Ease::Settle => curves::SPRING_SETTLE,
        Ease::Pop => curves::SPRING_POP,
        Ease::CubicOut => Curve::bezier(0.215, 0.61, 0.355, 1.0),
    }
}

/// The `translateX` keyframes of `u-shake` (0 / 20 / 45 / 70 / 100 %).
fn shake_x(p: f32, amp: f32) -> f32 {
    const K: [(f32, f32); 5] = [(0.0, 0.0), (0.2, -5.0), (0.45, 4.0), (0.7, -2.0), (1.0, 0.0)];
    let p = p.clamp(0.0, 1.0);
    for w in K.windows(2) {
        if p <= w[1].0 {
            let k = (p - w[0].0) / (w[1].0 - w[0].0);
            return (w[0].1 + (w[1].1 - w[0].1) * k) * amp / 5.0;
        }
    }
    0.0
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Does a finished cue of this kind leave the element in its end state (gone / folded) instead of at rest?
fn holds(a: &Anim) -> bool {
    match a.kind {
        Kind::Exit | Kind::Collapse => !matches!(a.target, Target::Row(_) | Target::ChecklistRow(_)) || a.hide,
        _ => false,
    }
}

#[derive(Default)]
pub struct AnimPlayer {
    active: Vec<Active>,
    held: Vec<(Target, Anim)>,
    reduced: bool,
}

impl AnimPlayer {
    pub fn new() -> AnimPlayer {
        AnimPlayer::default()
    }

    pub fn set_reduced(&mut self, on: bool) {
        self.reduced = on;
        if on {
            self.active.clear();
            self.held.clear();
        }
    }

    /// Starts a cue. A new cue on a target replaces the running one and forgets a held end state of that target.
    pub fn play(&mut self, a: &Anim) {
        if self.reduced {
            return;
        }
        self.held.retain(|(t, _)| *t != a.target);
        // an Enter / Pop / Expand supersedes an Exit of the same element and vice versa; effects of other kinds (Flash, Shake) stack
        let same = |x: &Active| x.cue.target == a.target && matches!(x.cue.kind, Kind::Enter | Kind::Exit | Kind::Collapse | Kind::Expand | Kind::Pop | Kind::Glide) == matches!(a.kind, Kind::Enter | Kind::Exit | Kind::Collapse | Kind::Expand | Kind::Pop | Kind::Glide);
        self.active.retain(|x| !same(x) || x.cue.kind != a.kind && matches!(a.kind, Kind::Flash | Kind::Shake));
        self.active.push(Active { cue: a.clone(), t: 0.0 });
    }

    /// Forget everything about `t` (the element is gone from the view).
    pub fn forget(&mut self, t: &Target) {
        self.active.retain(|x| x.cue.target != *t);
        self.held.retain(|(h, _)| h != t);
    }

    pub fn clear(&mut self) {
        self.active.clear();
        self.held.clear();
    }

    /// Advance by `dt` seconds. True while any cue is still in flight.
    pub fn step(&mut self, dt: f32) -> bool {
        for a in &mut self.active {
            a.t += dt.max(0.0);
        }
        let mut done = Vec::new();
        self.active.retain(|a| {
            let total = (a.cue.delay_ms + a.cue.dur_ms) as f32 / 1000.0;
            if a.t >= total {
                done.push(a.cue.clone());
                false
            } else {
                true
            }
        });
        for c in done {
            if holds(&c) {
                self.held.retain(|(t, _)| *t != c.target);
                self.held.push((c.target.clone(), c));
            }
        }
        !self.active.is_empty()
    }

    pub fn busy(&self) -> bool {
        !self.active.is_empty()
    }

    /// The end state of a finished Exit / Collapse (opacity 0 / height 0), if the element was left like that.
    pub fn is_held(&self, t: &Target) -> bool {
        self.held.iter().any(|(h, _)| h == t)
    }

    /// The combined look of `t` right now.
    pub fn look(&self, t: &Target) -> Look {
        let mut l = Look::REST;
        for (h, c) in &self.held {
            if h == t {
                l = Self::apply(l, c, 1.0e9, 1.0);
            }
        }
        for a in self.active.iter().filter(|a| a.cue.target == *t) {
            let el = a.t - a.cue.delay_ms as f32 / 1000.0;
            l = Self::apply(l, &a.cue, 0.0f32.max(el), a.cue.dur_ms as f32 / 1000.0);
        }
        l
    }

    /// One cue at `el` seconds since its delay ran out (`dur` seconds long; `el` beyond `dur` = its end state).
    fn apply(mut l: Look, c: &Anim, el: f32, dur: f32) -> Look {
        let before_start = el <= 0.0;
        let p = if dur <= 0.0 || el >= dur { 1.0 } else { (el / dur).clamp(0.0, 1.0) };
        let e = curve(c.ease).at(p);
        match c.kind {
            Kind::Enter => {
                // opacity on ease-out over the medium duration, the move on the spring (transform only)
                l.alpha *= if before_start { 0.0 } else { curves::EASE_OUT.at((el / 0.22).min(1.0)) };
                l.dx += (1.0 - e) * c.dx;
                l.dy += (1.0 - e) * c.dy;
                let (sx, sy) = Self::scale_pair(&c.target, c.from_scale, e);
                l.sx *= sx;
                l.sy *= sy;
            }
            Kind::Exit => {
                l.alpha *= 1.0 - curves::EASE_IN.at(p);
                l.dx += e * c.dx;
                l.dy += e * c.dy;
            }
            Kind::Collapse => l.height *= 1.0 - e,
            Kind::Expand => l.height *= if before_start { 0.0 } else { e },
            Kind::Pop => {
                l.alpha *= if before_start { 0.0 } else { curves::EASE_OUT.at((el / 0.22).min(1.0)) };
                let s = lerp(c.from_scale, 1.0, e);
                l.sx *= s;
                l.sy *= s;
                l.rot_deg += lerp(c.from_rotate_deg, 0.0, e);
            }
            Kind::Flash => {
                // 0 -> 1 at 18 %, back to 0 at 100 %
                l.flash = if before_start || p >= 1.0 { 0.0 } else if p < 0.18 { p / 0.18 } else { 1.0 - (p - 0.18) / 0.82 };
            }
            Kind::Shake => l.dx += if before_start { 0.0 } else { shake_x(p, c.dx) },
            Kind::DrawTick => l.draw = if before_start { 0.0 } else { curves::EASE_OUT.at(p) },
            Kind::Slide | Kind::Glide => {}
        }
        l
    }

    /// `translateY(-18px) scale(.96, .9)` of the bar's pop: the body squashes more vertically than horizontally.
    fn scale_pair(t: &Target, from: f32, e: f32) -> (f32, f32) {
        if *t == Target::Body {
            (lerp(0.96, 1.0, e), lerp(0.9, 1.0, e))
        } else {
            let s = lerp(from, 1.0, e);
            (s, s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pill_ui_core::bar::anim as cue;

    #[test]
    fn enter_fades_and_settles() {
        let mut p = AnimPlayer::new();
        p.play(&cue::message_enter());
        let l0 = p.look(&Target::Message);
        assert_eq!(l0.alpha, 0.0);
        assert!((l0.dy - 8.0).abs() < 1e-3);
        p.step(0.1);
        let l1 = p.look(&Target::Message);
        assert!(l1.alpha > 0.3 && l1.dy < 8.0);
        assert!(p.step(0.2));
        assert!(!p.step(0.5));
        assert!(p.look(&Target::Message).is_rest());
    }

    #[test]
    fn body_pop_squashes_vertically_more() {
        let mut p = AnimPlayer::new();
        p.play(&cue::body_pop());
        let l = p.look(&Target::Body);
        assert!(l.sy < l.sx && (l.sx - 0.96).abs() < 1e-3 && (l.sy - 0.9).abs() < 1e-3);
        assert!(l.dy < -17.0);
    }

    #[test]
    fn exit_holds_for_lists_and_panels_but_not_for_rows() {
        let mut p = AnimPlayer::new();
        p.play(&cue::list_step_aside());
        p.play(&cue::row_remove(2));
        p.step(1.0);
        assert!(p.is_held(&Target::List));
        assert_eq!(p.look(&Target::List).alpha, 0.0);
        assert!(!p.is_held(&Target::Row(2)), "a row index is reused by the next render");
        p.play(&cue::list_back());
        assert!(!p.is_held(&Target::List), "the list coming back clears the held exit");
    }

    #[test]
    fn collapse_ends_folded_and_expand_unfolds() {
        let mut p = AnimPlayer::new();
        p.play(&cue::panel_collapse());
        p.step(0.13);
        let h = p.look(&Target::PanelBox).height;
        assert!(h > 0.0 && h < 1.0);
        p.step(1.0);
        assert_eq!(p.look(&Target::PanelBox).height, 0.0);
        p.play(&cue::group_expand(0));
        assert_eq!(p.look(&Target::Group(0)).height, 0.0);
        p.step(1.0);
        assert!(p.look(&Target::Group(0)).is_rest());
    }

    #[test]
    fn stagger_delays_the_start() {
        let mut p = AnimPlayer::new();
        p.play(&cue::part_enter(3));
        p.step(0.05);
        assert_eq!(p.look(&Target::PanelPart(3)).alpha, 0.0, "84 ms stagger: still waiting");
        p.step(0.1);
        assert!(p.look(&Target::PanelPart(3)).alpha > 0.0);
    }

    #[test]
    fn reduced_motion_plays_nothing() {
        let mut p = AnimPlayer::new();
        p.play(&cue::list_step_aside());
        p.set_reduced(true);
        assert!(!p.busy() && p.look(&Target::List).is_rest());
        p.play(&cue::body_pop());
        assert!(!p.busy());
    }

    #[test]
    fn shake_swings_and_returns() {
        let mut p = AnimPlayer::new();
        p.play(&cue::shake(Target::PanelStep));
        p.step(0.12 + 0.42 * 0.2);
        assert!(p.look(&Target::PanelStep).dx < -4.0);
        p.step(1.0);
        assert_eq!(p.look(&Target::PanelStep).dx, 0.0);
    }

    #[test]
    fn flash_peaks_early() {
        let mut p = AnimPlayer::new();
        p.play(&cue::row_flash(1));
        p.step(0.7 * 0.18);
        assert!(p.look(&Target::Row(1)).flash > 0.9);
        p.step(1.0);
        assert_eq!(p.look(&Target::Row(1)).flash, 0.0, "no cue left: the neutral value");
    }
}
