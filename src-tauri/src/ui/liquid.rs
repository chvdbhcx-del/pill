//! Liquid motion: every moving value in Pill is a mass on a spring. It has inertia (it starts softly and keeps its velocity when the target
//! changes half-way, so a reversed movement turns around instead of kinking), it settles by itself (no end-of-animation snap), and it lets
//! the window idle once it has settled (`moving()` is false). Nothing in the UI should use a linear step, a fixed-duration tween or a
//! cubic-bezier curve for a visible movement: use a [`Flow`] with one of the [`Feel`]s below.
//!
//! The feel of the whole app is tuned HERE (the five [`Feel`] constants): change a number and every movement changes with it.
//!
//! ```ignore
//! let mut a = Flow::new(0.0, Feel::FADE);       // an opacity
//! a.to(1.0);                                     // retarget at any moment
//! let alpha = a.step(dt_ms);                     // per frame; `a.moving()` says whether the window still needs frames
//! ```
//!
//! Reduced motion: a [`Flow`] lands on its target at once (the underlying `glasskit::ease::Spring` does that itself).
use glasskit::ease::Spring;

/// How a value moves: `settle_ms` = about when it has arrived (95 %), `damping` = 1.0 critically damped (no overshoot), lower = softer
/// landing with a bounce (0.6 = a small overshoot, 0.4 = lively).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feel {
    pub settle_ms: f32,
    pub damping: f32,
}

impl Feel {
    /// Opacity, colour mixes, glows, hover highlights: smooth inertia, never overshoots (an alpha above 1 would be a bug).
    pub const FADE: Feel = Feel { settle_ms: 200.0, damping: 1.0 };
    /// Positions, sizes, scale, slides, panels: lands like a drop of water, a soft overshoot.
    pub const FLOW: Feel = Feel { settle_ms: 360.0, damping: 0.62 };
    /// Pops and toggles: switch knobs, check marks, badges, things that appear: lively.
    pub const POP: Feel = Feel { settle_ms: 420.0, damping: 0.45 };
    /// Follows a pointer, a scroll or a drag: heavy and slow, no bounce.
    pub const DRAG: Feel = Feel { settle_ms: 260.0, damping: 0.9 };
    /// Immediate feedback: press, hover scale, small nudges.
    pub const SNAP: Feel = Feel { settle_ms: 140.0, damping: 0.7 };

    /// The same feel, arriving in `ms`.
    pub const fn within(self, ms: f32) -> Feel {
        Feel { settle_ms: ms, damping: self.damping }
    }
}

fn spring(x: f32, f: Feel, eps: f32) -> Spring {
    let t = (f.settle_ms / 1000.0).max(0.02);
    // 5 % settling time: critical ~ 4.74 / omega; under-damped ~ 3 / (zeta omega)
    let w = if f.damping >= 1.0 { 4.74 / t } else { 3.0 / (f.damping.max(0.05) * t) };
    Spring::new(x, w * w, f.damping).with_eps(eps)
}

/// One animated number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flow {
    sp: Spring,
    feel: Feel,
    eps: f32,
}

impl Flow {
    /// A value in 0..1 units (opacity, progress, a mix): settles within 0.002.
    pub fn new(x: f32, feel: Feel) -> Flow {
        Flow { sp: spring(x, feel, 0.002), feel, eps: 0.002 }
    }

    /// A value in pixels: settles within 0.05 px.
    pub fn px(x: f32, feel: Feel) -> Flow {
        Flow { sp: spring(x, feel, 0.05), feel, eps: 0.05 }
    }

    /// Where it is heading.
    pub fn to(&mut self, target: f32) {
        self.sp.set_target(target);
    }

    pub fn target(&self) -> f32 {
        self.sp.target
    }

    /// The current value.
    pub fn at(&self) -> f32 {
        self.sp.x
    }

    /// The current value held inside 0..1 (for an opacity driven by an overshooting feel).
    pub fn at01(&self) -> f32 {
        self.sp.x.clamp(0.0, 1.0)
    }

    pub fn velocity(&self) -> f32 {
        self.sp.v
    }

    /// True while it has not arrived: the window must keep asking for frames.
    pub fn moving(&self) -> bool {
        !self.sp.at_rest()
    }

    /// Advance by `dt_ms`; the new value.
    pub fn step(&mut self, dt_ms: f64) -> f32 {
        self.sp.step((dt_ms / 1000.0) as f32);
        self.sp.x
    }

    /// Retarget and advance in one go (the common per-frame call).
    pub fn chase(&mut self, target: f32, dt_ms: f64) -> f32 {
        self.to(target);
        self.step(dt_ms)
    }

    /// Land on `x` at once, with no velocity (first layout, reduced previews).
    pub fn jump(&mut self, x: f32) {
        self.sp.jump(x);
    }

    /// A push: adds velocity (value units per second), e.g. a small kick when something appears so it settles with a wobble.
    pub fn nudge(&mut self, dv: f32) {
        self.sp.v += dv;
    }

    /// Change how it moves from now on; position and velocity carry over.
    pub fn set_feel(&mut self, feel: Feel) {
        let (x, v, t) = (self.sp.x, self.sp.v, self.sp.target);
        self.sp = spring(x, feel, self.eps);
        self.sp.v = v;
        self.sp.target = t;
        self.feel = feel;
    }

    pub fn feel(&self) -> Feel {
        self.feel
    }
}

/// A point that flows on both axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flow2 {
    pub x: Flow,
    pub y: Flow,
}

impl Flow2 {
    pub fn px(p: (f32, f32), feel: Feel) -> Flow2 {
        Flow2 { x: Flow::px(p.0, feel), y: Flow::px(p.1, feel) }
    }
    pub fn to(&mut self, p: (f32, f32)) {
        self.x.to(p.0);
        self.y.to(p.1);
    }
    pub fn step(&mut self, dt_ms: f64) -> (f32, f32) {
        (self.x.step(dt_ms), self.y.step(dt_ms))
    }
    pub fn chase(&mut self, p: (f32, f32), dt_ms: f64) -> (f32, f32) {
        self.to(p);
        self.step(dt_ms)
    }
    pub fn at(&self) -> (f32, f32) {
        (self.x.at(), self.y.at())
    }
    pub fn moving(&self) -> bool {
        self.x.moving() || self.y.moving()
    }
    pub fn jump(&mut self, p: (f32, f32)) {
        self.x.jump(p.0);
        self.y.jump(p.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    /// ms until a flow stepping at 8 ms has stayed within 5 % of its target for good.
    fn settle_ms(feel: Feel) -> f32 {
        let mut f = Flow::new(0.0, feel);
        f.to(1.0);
        let (mut t, mut last_out) = (0.0f32, 0.0f32);
        while t < 3000.0 {
            t += 8.0;
            let x = f.step(8.0);
            if (x - 1.0).abs() > 0.05 {
                last_out = t;
            }
        }
        last_out
    }

    #[test]
    fn every_feel_settles_near_its_time() {
        let _m = motion_on();
        for f in [Feel::FADE, Feel::FLOW, Feel::POP, Feel::DRAG, Feel::SNAP] {
            let t = settle_ms(f);
            assert!(t > f.settle_ms * 0.4 && t < f.settle_ms * 1.5, "{f:?} settled at {t} ms");
        }
    }

    #[test]
    fn fade_never_overshoots_and_flow_bounces_a_little() {
        let _m = motion_on();
        let peak = |feel| {
            let mut f = Flow::new(0.0, feel);
            f.to(1.0);
            (0..400).map(|_| f.step(8.0)).fold(0.0f32, f32::max)
        };
        assert!(peak(Feel::FADE) <= 1.0001, "fade peak {}", peak(Feel::FADE));
        let p = peak(Feel::FLOW);
        assert!(p > 1.02 && p < 1.25, "flow overshoot {p}");
        assert!(peak(Feel::POP) > p, "pop is livelier than flow");
    }

    #[test]
    fn a_reversed_movement_keeps_its_momentum_and_the_value_never_jumps() {
        let _m = motion_on();
        let mut f = Flow::new(0.0, Feel::FADE);
        f.to(1.0);
        let mut x = 0.0;
        for _ in 0..6 {
            x = f.step(16.0);
        }
        assert!(x > 0.1 && x < 0.9 && f.velocity() > 0.0);
        f.to(0.0);
        let after = f.step(16.0);
        assert!(after >= x - 1e-4, "it carries on a little before turning: {after} vs {x}");
        assert!((after - x).abs() < 0.2, "no jump");
    }

    #[test]
    fn it_idles_when_settled_and_lands_at_once_under_reduced_motion() {
        let m = motion_on();
        let mut f = Flow::px(0.0, Feel::FLOW);
        assert!(!f.moving());
        f.to(40.0);
        assert!(f.moving());
        for _ in 0..400 {
            f.step(8.0);
        }
        assert!(!f.moving() && f.at() == 40.0, "settled exactly on the target");
        drop(m);
        let _lock = crate::ui::host::testing::motion_lock();
        glasskit::ease::force_reduced(Some(true));
        let mut g = Flow::px(0.0, Feel::FLOW);
        g.to(40.0);
        assert_eq!(g.step(16.0), 40.0);
        glasskit::ease::force_reduced(None);
    }

    #[test]
    fn a_nudge_makes_it_wobble_and_a_new_feel_keeps_the_position() {
        let _m = motion_on();
        let mut f = Flow::new(1.0, Feel::POP);
        f.nudge(3.0);
        assert!(f.moving());
        assert!(f.step(16.0) > 1.0);
        let (x, v) = (f.at(), f.velocity());
        f.set_feel(Feel::FADE);
        assert_eq!((f.at(), f.velocity(), f.target()), (x, v, 1.0));
    }
}
