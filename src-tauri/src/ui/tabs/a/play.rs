//! `Player`: plays the model's `Anim` cues (`pill_ui_core::panel::a::Anim`, delivered as `Fx::Anim { target, anim }`) and answers "what does the
//! element `target` look like right now" as a [`Sample`]. The view multiplies its drawing by the sample (opacity, offset, scale, rotation) or reads
//! `p` for size-like cues (collapse / expand / flip / crossfade). Targets are the model's names: `title`, `art`, `row:<id>`, `chip:<id>`, `list`...
//!
//! * Under Reduce motion [`Player::play`] ignores the cue (the models already do not push cues then; this is the second guard) so the element
//!   shows its end state at once.
//! * Exit-like cues (`Exit`, `Collapse`, `SlideAway`, `FadeTo`) HOLD their end state after finishing, so a removed element does not flash back
//!   for a frame; call [`Player::release`] when you drop the element (or [`Player::clear`]).
//! * `step` returns true while anything moves (ask the host for frames only then).
use glasskit::ease::{curves, CubicBezier, Ease};
use pill_ui_core::panel::a::anim::{Anim, AnimKind, Ease as ModelEase};

/// How an element looks at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Opacity multiplier 0..1 (for `FadeTo` and `Ring`: the wash / ring opacity).
    pub alpha: f32,
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    /// Rotation in degrees (`FlyIn`, `Spin`).
    pub rot: f32,
    /// Eased progress 0..1 of the cue (`Collapse`: height factor is `1 - p`; `Expand`: `p`; `Flip` / `Crossfade` / `Ring`: the blend). May overshoot on pop curves.
    pub p: f32,
    /// A cue is running or held on this target.
    pub active: bool,
}

impl Sample {
    pub const REST: Sample = Sample { alpha: 1.0, dx: 0.0, dy: 0.0, scale: 1.0, rot: 0.0, p: 1.0, active: false };
}

struct Item {
    target: String,
    anim: Anim,
    /// ms since the cue was started; negative while its delay runs
    t: f32,
    done: bool,
}

#[derive(Default)]
pub struct Player {
    items: Vec<Item>,
}

fn ease_of(e: ModelEase) -> Ease {
    match e {
        ModelEase::Out => curves::EASE_OUT,
        ModelEase::OutExpo => curves::EASE_OUT_EXPO,
        ModelEase::SpringSettle => curves::SPRING_SETTLE,
        ModelEase::SpringPop => curves::SPRING_POP,
        ModelEase::In => curves::EASE_IN,
        ModelEase::CssOut => Ease::Bezier(CubicBezier::new(0.0, 0.0, 0.58, 1.0)),
        ModelEase::CssInOut => curves::CSS_EASE_IN_OUT,
        ModelEase::Linear => Ease::Linear,
    }
}

fn holds(k: &AnimKind) -> bool {
    matches!(k, AnimKind::Exit { .. } | AnimKind::Collapse | AnimKind::SlideAway { .. } | AnimKind::FadeTo { .. })
}

fn looped(k: &AnimKind) -> bool {
    matches!(k, AnimKind::Pulse { looped: true, .. })
}

impl Player {
    pub fn new() -> Player {
        Player::default()
    }

    /// Start `anim` on `target` (replaces a cue already playing there). Ignored when `reduced`.
    pub fn play(&mut self, target: &str, anim: Anim, reduced: bool) {
        if reduced || glasskit::ease::reduced() {
            self.items.retain(|i| i.target != target);
            return;
        }
        let t = -(anim.delay_ms as f32);
        self.items.retain(|i| i.target != target);
        self.items.push(Item { target: target.to_string(), anim, t, done: false });
    }

    /// Advance by `dt_ms`; true while any cue is still running (a held end state does not count).
    pub fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = false;
        for i in &mut self.items {
            if i.done {
                continue;
            }
            i.t += dt_ms as f32;
            if looped(&i.anim.kind) {
                busy = true;
            } else if i.t >= i.anim.dur_ms as f32 {
                i.done = true;
            } else {
                busy = true;
            }
        }
        self.items.retain(|i| !i.done || holds(&i.anim.kind));
        busy
    }

    /// Anything running (frames needed)?
    pub fn busy(&self) -> bool {
        self.items.iter().any(|i| !i.done)
    }

    pub fn is_active(&self, target: &str) -> bool {
        self.items.iter().any(|i| i.target == target && !i.done)
    }

    /// Forget the cue of `target` (an exit finished and the element is gone).
    pub fn release(&mut self, target: &str) {
        self.items.retain(|i| i.target != target);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Targets whose held end state (`Exit` / `Collapse`...) is on screen: the view may remove those elements now.
    pub fn finished(&self) -> Vec<String> {
        self.items.iter().filter(|i| i.done).map(|i| i.target.clone()).collect()
    }

    pub fn sample(&self, target: &str) -> Sample {
        self.items.iter().find(|i| i.target == target).map_or(Sample::REST, |i| sample_of(i))
    }
}

fn sample_of(i: &Item) -> Sample {
    let a = &i.anim;
    let dur = a.dur_ms.max(1) as f32;
    let raw = if i.done { 1.0 } else { (i.t / dur).clamp(0.0, 1.0) };
    let waiting = i.t < 0.0;
    let p = ease_of(a.ease).at(raw);
    // fades never ride a spring: CSS opacity is its own short ease-out
    let fade = |ms: f32| curves::EASE_OUT.at(if i.done { 1.0 } else { (i.t / ms.min(dur).max(1.0)).clamp(0.0, 1.0) });
    let mut s = Sample { active: true, p, ..Sample::REST };
    match a.kind {
        AnimKind::Enter { dx, dy, scale } => {
            s.alpha = fade(220.0);
            s.dx = dx * (1.0 - p);
            s.dy = dy * (1.0 - p);
            s.scale = scale + (1.0 - scale) * p;
        }
        AnimKind::Exit { dx, dy, scale } => {
            s.alpha = 1.0 - raw_in(raw);
            s.dx = dx * p;
            s.dy = dy * p;
            s.scale = 1.0 + (scale - 1.0) * p;
        }
        AnimKind::Collapse => s.alpha = 1.0 - raw_in(raw),
        AnimKind::Expand => s.alpha = fade(220.0),
        AnimKind::Pop { from_scale } => s.scale = from_scale + (1.0 - from_scale) * p,
        AnimKind::Flip { .. } => {}
        AnimKind::FlyIn { from_dx, from_dy, rot_deg } => {
            s.alpha = fade(150.0);
            s.dx = from_dx * (1.0 - p);
            s.dy = from_dy * (1.0 - p);
            s.rot = rot_deg * (1.0 - p);
        }
        AnimKind::Crossfade => s.alpha = p.clamp(0.0, 1.0),
        AnimKind::SlideIn { dx } => {
            s.alpha = fade(140.0);
            s.dx = dx * (1.0 - p);
        }
        AnimKind::SlideAway { dx } => {
            s.alpha = 1.0 - raw_in(raw);
            s.dx = dx * p;
        }
        AnimKind::Spin => s.rot = 360.0 * p,
        AnimKind::Pulse { min_opacity, looped } => {
            let ph = if looped { (i.t.max(0.0) / dur).fract() } else { raw };
            s.alpha = 1.0 - (1.0 - min_opacity) * (0.5 - 0.5 * (ph * std::f32::consts::TAU).cos());
        }
        AnimKind::Shake => s.dx = 6.0 * (1.0 - raw) * (raw * 18.0).sin(),
        AnimKind::Ring => s.alpha = 0.6 * (1.0 - raw),
        AnimKind::FadeTo { from } => s.alpha = from * (1.0 - raw),
    }
    if waiting {
        // before the delay is over the element sits in its start state
        s.p = 0.0;
        match a.kind {
            AnimKind::Enter { dx, dy, scale } => {
                s.alpha = 0.0;
                s.dx = dx;
                s.dy = dy;
                s.scale = scale;
            }
            AnimKind::FlyIn { from_dx, from_dy, rot_deg } => {
                s.alpha = 0.0;
                s.dx = from_dx;
                s.dy = from_dy;
                s.rot = rot_deg;
            }
            AnimKind::SlideIn { dx } => {
                s.alpha = 0.0;
                s.dx = dx;
            }
            AnimKind::Crossfade => s.alpha = 0.0,
            AnimKind::Expand => s.alpha = 0.0,
            _ => {}
        }
    }
    s
}

/// The exit fades use an ease-in on the raw progress.
fn raw_in(raw: f32) -> f32 {
    curves::EASE_IN.at(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Calm(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
    impl Drop for Calm {
        fn drop(&mut self) {
            glasskit::ease::force_reduced(None);
        }
    }
    fn calm() -> Calm {
        let g = crate::ui::host::testing::motion_lock();
        glasskit::ease::force_reduced(Some(false));
        Calm(g)
    }

    fn step_all(p: &mut Player, ms: f64) {
        let mut left = ms;
        while left > 0.0 {
            p.step(4.0);
            left -= 4.0;
        }
    }

    #[test]
    fn enter_fades_in_and_settles_at_rest() {
        let _c = calm();
        let mut p = Player::new();
        p.play("title", Anim::text_swap(), false);
        let s0 = p.sample("title");
        assert!(s0.alpha < 0.05 && s0.dy > 4.0, "{s0:?}");
        assert!(p.busy());
        step_all(&mut p, 400.0);
        assert!(!p.busy());
        assert_eq!(p.sample("title"), Sample::REST);
    }

    #[test]
    fn delay_keeps_the_start_state() {
        let _c = calm();
        let mut p = Player::new();
        p.play("row:1", Anim::row_stagger(3), false); // 84 ms delay
        step_all(&mut p, 40.0);
        let s = p.sample("row:1");
        assert_eq!(s.alpha, 0.0);
        assert!(s.dy > 5.0);
        step_all(&mut p, 600.0);
        assert!(!p.busy());
    }

    #[test]
    fn exit_holds_until_released() {
        let _c = calm();
        let mut p = Player::new();
        p.play("chip:1", Anim::exit(8.0, 0.9), false);
        step_all(&mut p, 400.0);
        assert!(!p.busy());
        let s = p.sample("chip:1");
        assert!(s.alpha < 0.01 && s.active, "{s:?}");
        assert_eq!(p.finished(), vec!["chip:1".to_string()]);
        p.release("chip:1");
        assert_eq!(p.sample("chip:1"), Sample::REST);
    }

    #[test]
    fn reduced_motion_plays_nothing_and_a_looped_pulse_never_ends() {
        let _c = calm();
        let mut p = Player::new();
        p.play("x", Anim::pop(0.8, 300), true);
        assert!(!p.busy() && !p.is_active("x"));
        p.play("tile", Anim::busy_pulse(), false);
        step_all(&mut p, 5000.0);
        assert!(p.busy());
        let a = p.sample("tile").alpha;
        assert!((0.44..=1.0).contains(&a));
    }

    #[test]
    fn spin_makes_a_full_turn_and_shake_returns_to_zero() {
        let _c = calm();
        let mut p = Player::new();
        p.play("refresh", Anim::spin(), false);
        step_all(&mut p, 400.0);
        let mid = p.sample("refresh").rot;
        assert!(mid > 90.0 && mid < 270.0, "{mid}");
        p.play("tile", Anim::shake(), false);
        step_all(&mut p, 500.0);
        assert_eq!(p.sample("tile").dx, 0.0);
    }
}
