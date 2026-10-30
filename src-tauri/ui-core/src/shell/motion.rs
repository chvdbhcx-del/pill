//! Easing curves and the CSS animation tables of the shell (pill.css / theme.css / motion.js tokens) as data.
//!
//! The renderer plays [`AnimSpec`]s with its own animation system (glasskit); this file only holds the numbers and a reference
//! evaluator so tests (and a software renderer) can sample them. Reduced motion: every spec is skipped and the `to` state applied at once
//! (`AnimSpec::at` returns `to` when `reduced`).

/// CSS `cubic-bezier(x1, y1, x2, y2)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier(pub f64, pub f64, pub f64, pub f64);

/// theme.css `--ease-out` (also the transition of `#body` transform, `--dur-slow` 0.38 s).
pub const EASE_OUT: CubicBezier = CubicBezier(0.22, 1.0, 0.36, 1.0);
/// `--ease-in`.
pub const EASE_IN: CubicBezier = CubicBezier(0.4, 0.0, 1.0, 1.0);
/// `--ease-spring` (small overshoot; also the `--spring-pop` fallback).
pub const EASE_SPRING: CubicBezier = CubicBezier(0.34, 1.56, 0.64, 1.0);
/// `--ease-out-expo` (motion.js OUT).
pub const EASE_OUT_EXPO: CubicBezier = CubicBezier(0.16, 1.0, 0.3, 1.0);
/// `--ease`.
pub const EASE: CubicBezier = CubicBezier(0.2, 0.9, 0.3, 1.0);
/// `--ease-in-out`.
pub const EASE_IN_OUT: CubicBezier = CubicBezier(0.65, 0.0, 0.35, 1.0);
/// `--spring-settle` as a `linear()` easing (22 intervals), used for transforms only, never opacity.
pub const SPRING_SETTLE: [f64; 23] = [
    0.0, 0.034, 0.116, 0.223, 0.338, 0.451, 0.556, 0.648, 0.728, 0.794, 0.848, 0.891, 0.925, 0.95, 0.969, 0.983, 0.992, 0.998, 1.002,
    1.005, 1.006, 1.006, 1.0,
];
/// `--spring-pop` as a `linear()` easing (28 intervals, overshoot 1.094).
pub const SPRING_POP: [f64; 29] = [
    0.0, 0.044, 0.152, 0.296, 0.452, 0.604, 0.741, 0.856, 0.946, 1.013, 1.057, 1.083, 1.094, 1.093, 1.086, 1.073, 1.059, 1.044, 1.03,
    1.018, 1.009, 1.001, 0.996, 0.993, 0.991, 0.991, 0.991, 0.992, 1.0,
];

impl CubicBezier {
    /// Progress -> value. Newton with a bisection fallback (same approach browsers use).
    pub fn eval(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 {
            return t;
        }
        let CubicBezier(x1, y1, x2, y2) = *self;
        let cx = 3.0 * x1;
        let bx = 3.0 * (x2 - x1) - cx;
        let ax = 1.0 - cx - bx;
        let cy = 3.0 * y1;
        let by = 3.0 * (y2 - y1) - cy;
        let ay = 1.0 - cy - by;
        let sx = |s: f64| ((ax * s + bx) * s + cx) * s;
        let dsx = |s: f64| (3.0 * ax * s + 2.0 * bx) * s + cx;
        let mut s = t;
        for _ in 0..8 {
            let err = sx(s) - t;
            if err.abs() < 1e-7 {
                return ((ay * s + by) * s + cy) * s;
            }
            let d = dsx(s);
            if d.abs() < 1e-6 {
                break;
            }
            s -= err / d;
        }
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        s = t;
        for _ in 0..60 {
            let x = sx(s);
            if (x - t).abs() < 1e-7 {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) / 2.0;
        }
        ((ay * s + by) * s + cy) * s
    }
}

/// CSS `linear(...)` easing: piecewise linear through equally spaced points.
pub fn eval_linear(points: &[f64], t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let n = points.len() - 1;
    let f = t * n as f64;
    let i = (f.floor() as usize).min(n - 1);
    points[i] + (points[i + 1] - points[i]) * (f - i as f64)
}

/// `bodyTy()` mirror: the CSS translateY transition of `#body` approximated with an ease-out quartic.
pub fn ease_out_quart(p: f64) -> f64 {
    1.0 - (1.0 - p.clamp(0.0, 1.0)).powi(4)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ease {
    Linear,
    Bezier(CubicBezier),
    /// `linear()` table (springs).
    Table(&'static [f64]),
}

impl Ease {
    pub fn eval(&self, t: f64) -> f64 {
        match self {
            Ease::Linear => t.clamp(0.0, 1.0),
            Ease::Bezier(b) => b.eval(t),
            Ease::Table(p) => eval_linear(p, t),
        }
    }
}

/// The animated properties the shell animations touch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimProps {
    pub opacity: f64,
    pub blur_px: f64,
    pub scale: f64,
    pub translate_y: f64,
}
impl AnimProps {
    pub const REST: AnimProps = AnimProps { opacity: 1.0, blur_px: 0.0, scale: 1.0, translate_y: 0.0 };
    fn lerp(a: &AnimProps, b: &AnimProps, p: f64) -> AnimProps {
        let l = |x: f64, y: f64| x + (y - x) * p;
        AnimProps { opacity: l(a.opacity, b.opacity), blur_px: l(a.blur_px, b.blur_px), scale: l(a.scale, b.scale), translate_y: l(a.translate_y, b.translate_y) }
    }
}

/// One keyframe animation: `from` -> `to` over `dur_ms` after `delay_ms`, eased.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimSpec {
    pub name: &'static str,
    pub dur_ms: f64,
    pub delay_ms: f64,
    pub ease: Ease,
    pub from: AnimProps,
    pub to: AnimProps,
}

impl AnimSpec {
    /// Sample after `t_ms` since the animation started. `reduced`: end state at once. Before the delay the `from` state is held
    /// (`animation-fill-mode: backwards`).
    pub fn at(&self, t_ms: f64, reduced: bool) -> AnimProps {
        if reduced {
            return self.to;
        }
        let p = ((t_ms - self.delay_ms) / self.dur_ms).clamp(0.0, 1.0);
        AnimProps::lerp(&self.from, &self.to, self.ease.eval(p))
    }
    pub fn end_ms(&self) -> f64 {
        self.delay_ms + self.dur_ms
    }
}

const fn props(opacity: f64, blur_px: f64, scale: f64, translate_y: f64) -> AnimProps {
    AnimProps { opacity, blur_px, scale, translate_y }
}

/// `.tc.inpill { animation: inpill-in .36s var(--ease-out) both }`: a card appearing inside the island.
pub const CARD_IN: AnimSpec =
    AnimSpec { name: "inpill-in", dur_ms: 360.0, delay_ms: 0.0, ease: Ease::Bezier(EASE_OUT), from: props(0.0, 4.0, 0.96, 0.0), to: AnimProps::REST };
/// `.tc.inpill.out { animation: inpill-out .18s var(--ease-in) both }` (JS removes the card after 180 ms, 0 when reduced).
pub const CARD_OUT: AnimSpec =
    AnimSpec { name: "inpill-out", dur_ms: 180.0, delay_ms: 0.0, ease: Ease::Bezier(EASE_IN), from: AnimProps::REST, to: props(0.0, 4.0, 0.97, 0.0) };
/// `#live-in.swap { animation: lp-swap .13s var(--ease-out) backwards }`: incoming live-row content.
pub const LIVE_SWAP: AnimSpec =
    AnimSpec { name: "lp-swap", dur_ms: 130.0, delay_ms: 0.0, ease: Ease::Bezier(EASE_OUT), from: props(0.0, 4.0, 1.0, 0.0), to: AnimProps::REST };
/// `.lp-ghost { animation: lp-ghost-out .09s var(--ease-in) both }`: outgoing live-row content (the copy is removed after 120 ms).
pub const LIVE_GHOST_OUT: AnimSpec =
    AnimSpec { name: "lp-ghost-out", dur_ms: 90.0, delay_ms: 0.0, ease: Ease::Bezier(EASE_IN), from: AnimProps::REST, to: props(0.0, 4.0, 1.0, 0.0) };
pub const GHOST_LIFETIME_MS: f64 = 120.0;
/// `.expanded #panel { animation: panel-in .14s var(--ease-out) .06s backwards }`: panel content sharpening while the body grows.
pub const PANEL_IN: AnimSpec =
    AnimSpec { name: "panel-in", dur_ms: 140.0, delay_ms: 60.0, ease: Ease::Bezier(EASE_OUT), from: props(0.0, 6.0, 1.0, 0.0), to: AnimProps::REST };
/// `#panel` collapse: `transition: opacity .06s linear`.
pub const PANEL_OUT_MS: f64 = 60.0;
/// `#live { transition: opacity .15s }` (hidden while expanded).
pub const LIVE_FADE_MS: f64 = 150.0;
/// PillMotion.enter(n, { y: 0, scale: .6, dur: .45 }) for side-column items that appear in a reshaped clock row
/// (opacity 220 ms ease-out-expo, transform 450 ms on the settle spring).
pub const RESHAPE_POP: AnimSpec =
    AnimSpec { name: "reshape-pop", dur_ms: 450.0, delay_ms: 0.0, ease: Ease::Table(&SPRING_SETTLE), from: props(0.0, 0.0, 0.6, 0.0), to: AnimProps::REST };
pub const RESHAPE_POP_FADE_MS: f64 = 220.0;
/// `.lp-time .d.roll`: the changed clock digit rolls up (out/in `--dur-med` .22 s ease-out; the JS cleans up after 280 ms).
pub const DIGIT_ROLL_MS: f64 = 220.0;
pub const DIGIT_ROLL_CLEANUP_MS: f64 = 280.0;
/// `#body { transition: transform var(--dur-slow) var(--ease-out) }`: offsets and the auto-hide slide (`.away` -> translateY(-130 %)).
pub const BODY_TRANSLATE_MS: f64 = 380.0;
/// `--dur-med` / `--dur-fast` / `--dur-slow` tokens.
pub const DUR_FAST_MS: f64 = 120.0;
pub const DUR_MED_MS: f64 = 220.0;
pub const DUR_SLOW_MS: f64 = 380.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_endpoints_and_monotonic_ease_out() {
        for b in [EASE_OUT, EASE_IN, EASE_SPRING, EASE_OUT_EXPO, EASE, EASE_IN_OUT] {
            assert_eq!(b.eval(0.0), 0.0);
            assert_eq!(b.eval(1.0), 1.0);
        }
        let mut prev = 0.0;
        for i in 1..=100 {
            let v = EASE_OUT.eval(i as f64 / 100.0);
            assert!(v >= prev - 1e-9);
            prev = v;
        }
        assert!(EASE_OUT.eval(0.5) > 0.5); // front-loaded
        assert!(EASE_IN.eval(0.5) < 0.5);
    }

    #[test]
    fn spring_ease_overshoots_but_ends_at_one() {
        assert!((0..=100).any(|i| EASE_SPRING.eval(i as f64 / 100.0) > 1.0));
        assert_eq!(EASE_SPRING.eval(1.0), 1.0);
        assert!(SPRING_POP.iter().cloned().fold(0.0, f64::max) > 1.09);
        assert_eq!(eval_linear(&SPRING_SETTLE, 0.0), 0.0);
        assert_eq!(eval_linear(&SPRING_SETTLE, 1.0), 1.0);
        // halfway between two table points
        let mid = eval_linear(&SPRING_SETTLE, 0.5 / 22.0);
        assert!((mid - 0.017).abs() < 1e-9);
    }

    #[test]
    fn quartic_mirror_of_body_ty() {
        assert_eq!(ease_out_quart(0.0), 0.0);
        assert_eq!(ease_out_quart(1.0), 1.0);
        assert!((ease_out_quart(0.5) - 0.9375).abs() < 1e-12);
        assert_eq!(ease_out_quart(3.0), 1.0);
    }

    #[test]
    fn anim_specs_sample_and_respect_reduced_motion() {
        let a = CARD_IN.at(0.0, false);
        assert_eq!((a.opacity, a.blur_px, a.scale), (0.0, 4.0, 0.96));
        assert_eq!(CARD_IN.at(360.0, false), AnimProps::REST);
        assert_eq!(CARD_IN.at(10.0, true), AnimProps::REST); // reduced: end state at once
        let mid = CARD_IN.at(100.0, false);
        assert!(mid.opacity > 0.0 && mid.opacity < 1.0);
        // delay is held at the `from` state
        assert_eq!(PANEL_IN.at(30.0, false).opacity, 0.0);
        assert_eq!(PANEL_IN.end_ms(), 200.0);
        assert_eq!(CARD_OUT.at(180.0, false).opacity, 0.0);
        assert_eq!(LIVE_SWAP.dur_ms, 130.0);
        assert_eq!(LIVE_GHOST_OUT.dur_ms, 90.0);
    }
}
