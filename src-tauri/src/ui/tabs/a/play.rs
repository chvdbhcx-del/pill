//! `Player`: plays the model's `Anim` cues (`pill_ui_core::panel::a::Anim`, delivered as `Fx::Anim { target, anim }`) and answers "what does the
//! element `target` look like right now" as a [`Sample`]. The view multiplies its drawing by the sample (opacity, offset, scale, rotation) or reads
//! `p` for size-like cues (collapse / expand / flip / crossfade). Targets are the model's names: `title`, `art`, `row:<id>`, `chip:<id>`, `list`...
//!
//! * Every cue is a pair of springs ([`Flow`]s, see `ui::liquid`): `m` moves the transform / size, `f` the opacity. The model's durations only
//!   cap how slow a cue may be; the feel (inertia, soft landing) comes from the presets. Replacing an enter / exit cue by another one (enter
//!   <-> exit, slide in <-> away) continues from where the element is, with its momentum, instead of restarting from the rest state.
//! * Under Reduce motion [`Player::play`] ignores the cue (the models already do not push cues then; this is the second guard) so the element
//!   shows its end state at once.
//! * Exit-like cues (`Exit`, `Collapse`, `SlideAway`, `FadeTo`) HOLD their end state after finishing, so a removed element does not flash back
//!   for a frame; call [`Player::release`] when you drop the element (or [`Player::clear`]).
//! * `step` returns true while anything moves (ask the host for frames only then).
use crate::ui::liquid::{Feel, Flow};
use pill_ui_core::panel::a::anim::{Anim, AnimKind};

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
    /// Progress 0..1 of the cue (`Collapse`: height factor is `1 - p`; `Expand`: `p`; `Flip` / `Crossfade` / `Ring`: the blend). May overshoot on springy cues.
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
    /// ms of the delay still to wait (the element sits in its start state meanwhile)
    wait: f32,
    /// ms since the delay ended (the clock of the breathing `Pulse`)
    t: f32,
    /// transform / size progress 0 -> 1 (`Shake`: the sideways offset in px, ringing out to 0)
    m: Flow,
    /// opacity progress 0 -> 1
    f: Flow,
    done: bool,
}

#[derive(Default)]
pub struct Player {
    items: Vec<Item>,
}

fn holds(k: &AnimKind) -> bool {
    matches!(k, AnimKind::Exit { .. } | AnimKind::Collapse | AnimKind::SlideAway { .. } | AnimKind::FadeTo { .. })
}

fn looped(k: &AnimKind) -> bool {
    matches!(k, AnimKind::Pulse { looped: true, .. })
}

/// The breathing pulse is a periodic loop, not a movement towards a target: it keeps its own clock.
fn clocked(k: &AnimKind) -> bool {
    matches!(k, AnimKind::Pulse { .. })
}

/// The same feel without overshoot (a height or an opacity must not leave 0..1).
fn flat(f: Feel) -> Feel {
    Feel { damping: 1.0, ..f }
}

/// The springs of a cue: (motion, fade). The preset time is the ceiling, a shorter model duration makes the cue quicker.
fn flows(a: &Anim) -> (Flow, Flow) {
    let d = a.dur_ms.max(1) as f32;
    let cap = |f: Feel| f.within(f.settle_ms.min(d));
    let slow = |f: Feel| f.within(d);
    let (m, f) = match a.kind {
        AnimKind::Enter { .. } | AnimKind::SlideIn { .. } | AnimKind::Flip { .. } | AnimKind::Exit { .. } | AnimKind::SlideAway { .. } => (cap(Feel::FLOW), cap(Feel::FADE)),
        AnimKind::Collapse | AnimKind::Expand => (cap(flat(Feel::FLOW)), cap(Feel::FADE)),
        AnimKind::Pop { .. } => (cap(Feel::POP), cap(Feel::FADE)),
        AnimKind::FlyIn { .. } => (cap(Feel::POP), Feel::FADE.within(150.0)),
        AnimKind::Crossfade => (slow(Feel::FADE), slow(Feel::FADE)),
        AnimKind::Spin => (slow(Feel::FADE), Feel::FADE),
        AnimKind::Ring | AnimKind::FadeTo { .. } => (slow(Feel::FADE), slow(Feel::FADE)),
        // a hit that rings out: a lightly damped spring around 0, kicked below
        AnimKind::Shake => (Feel { settle_ms: d, damping: 0.25 }, Feel::FADE),
        AnimKind::Pulse { .. } => (Feel::FADE, Feel::FADE),
    };
    let shake = matches!(a.kind, AnimKind::Shake);
    let mut mf = if shake { Flow::px(0.0, m) } else { Flow::new(0.0, m) };
    let mut ff = Flow::new(0.0, f);
    if shake {
        mf.nudge(160.0);
        ff.jump(1.0);
    } else if clocked(&a.kind) {
        mf.jump(1.0);
        ff.jump(1.0);
    } else {
        mf.to(1.0);
        ff.to(1.0);
    }
    (mf, ff)
}

/// How far along the cue `new` the element is when it looks like `s` (the transform axis that moves most decides).
fn progress_of(new: &AnimKind, s: &Sample) -> Option<f32> {
    let by = |d: f32, cur: f32, inv: bool| (d.abs() > 1e-3).then(|| if inv { 1.0 - cur / d } else { cur / d });
    let p = match *new {
        AnimKind::Enter { dx, dy, scale } => by(dy, s.dy, true).or(by(dx, s.dx, true)).or_else(|| ((1.0 - scale).abs() > 1e-3).then(|| (s.scale - scale) / (1.0 - scale))),
        AnimKind::Exit { dx, dy, scale } => by(dy, s.dy, false).or(by(dx, s.dx, false)).or_else(|| ((scale - 1.0).abs() > 1e-3).then(|| (s.scale - 1.0) / (scale - 1.0))),
        AnimKind::SlideIn { dx } => by(dx, s.dx, true),
        AnimKind::SlideAway { dx } => by(dx, s.dx, false),
        _ => None,
    };
    p.map(|p| p.clamp(0.0, 1.5))
}

/// `Some(true)` for the cues that bring an element in, `Some(false)` for the ones that take it out.
fn entering(k: &AnimKind) -> Option<bool> {
    match k {
        AnimKind::Enter { .. } | AnimKind::SlideIn { .. } => Some(true),
        AnimKind::Exit { .. } | AnimKind::SlideAway { .. } => Some(false),
        _ => None,
    }
}

impl Player {
    pub fn new() -> Player {
        Player::default()
    }

    /// Start `anim` on `target` (replaces a cue already playing there; enter / exit cues continue from where the old one stood).
    /// Ignored when `reduced`.
    pub fn play(&mut self, target: &str, anim: Anim, reduced: bool) {
        let old = self.items.iter().position(|i| i.target == target).map(|k| self.items.remove(k));
        if reduced || glasskit::ease::reduced() {
            return;
        }
        let (mut m, mut f) = flows(&anim);
        if let (Some(old), Some(now)) = (&old, entering(&anim.kind)) {
            let s = sample_of(old);
            if let (Some(was), Some(m0)) = (entering(&old.anim.kind), progress_of(&anim.kind, &s)) {
                let sign = if was == now { 1.0 } else { -1.0 };
                m.jump(m0);
                f.jump(if now { s.alpha } else { 1.0 - s.alpha });
                m.to(1.0);
                f.to(1.0);
                m.nudge(old.m.velocity() * sign);
                f.nudge(old.f.velocity() * sign);
            }
        }
        let wait = anim.delay_ms as f32;
        self.items.push(Item { target: target.to_string(), anim, wait, t: 0.0, m, f, done: false });
    }

    /// Advance by `dt_ms`; true while any cue is still running (a held end state does not count).
    pub fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = false;
        for i in &mut self.items {
            if i.done {
                continue;
            }
            let mut dt = dt_ms;
            if i.wait > 0.0 {
                i.wait -= dt as f32;
                if i.wait > 0.0 {
                    busy = true;
                    continue;
                }
                dt = f64::from(-i.wait);
                i.wait = 0.0;
            }
            i.t += dt as f32;
            i.m.step(dt);
            i.f.step(dt);
            if looped(&i.anim.kind) {
                busy = true;
            } else if clocked(&i.anim.kind) {
                i.done = i.t >= i.anim.dur_ms as f32;
                busy |= !i.done;
            } else {
                i.done = !i.m.moving() && !i.f.moving();
                busy |= !i.done;
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
        self.items.iter().find(|i| i.target == target).map_or(Sample::REST, sample_of)
    }
}

fn sample_of(i: &Item) -> Sample {
    let a = &i.anim;
    let dur = a.dur_ms.max(1) as f32;
    let (p, f) = (i.m.at(), i.f.at01());
    let mut s = Sample { active: true, p, ..Sample::REST };
    match a.kind {
        AnimKind::Enter { dx, dy, scale } => {
            s.alpha = f;
            s.dx = dx * (1.0 - p);
            s.dy = dy * (1.0 - p);
            s.scale = scale + (1.0 - scale) * p;
        }
        AnimKind::Exit { dx, dy, scale } => {
            s.alpha = 1.0 - f;
            s.dx = dx * p;
            s.dy = dy * p;
            s.scale = 1.0 + (scale - 1.0) * p;
        }
        AnimKind::Collapse => s.alpha = 1.0 - f,
        AnimKind::Expand => s.alpha = f,
        AnimKind::Pop { from_scale } => s.scale = from_scale + (1.0 - from_scale) * p,
        AnimKind::Flip { .. } => {}
        AnimKind::FlyIn { from_dx, from_dy, rot_deg } => {
            s.alpha = f;
            s.dx = from_dx * (1.0 - p);
            s.dy = from_dy * (1.0 - p);
            s.rot = rot_deg * (1.0 - p);
        }
        AnimKind::Crossfade => s.alpha = p.clamp(0.0, 1.0),
        AnimKind::SlideIn { dx } => {
            s.alpha = f;
            s.dx = dx * (1.0 - p);
        }
        AnimKind::SlideAway { dx } => {
            s.alpha = 1.0 - f;
            s.dx = dx * p;
        }
        AnimKind::Spin => s.rot = 360.0 * p,
        AnimKind::Pulse { min_opacity, looped } => {
            // a periodic breath (a smooth cosine), not a movement towards a target: it keeps its own clock
            let ph = if looped { (i.t / dur).fract() } else { (i.t / dur).clamp(0.0, 1.0) };
            s.alpha = 1.0 - (1.0 - min_opacity) * (0.5 - 0.5 * (ph * std::f32::consts::TAU).cos());
            s.p = 1.0;
        }
        AnimKind::Shake => {
            s.dx = p;
            s.p = 1.0;
        }
        AnimKind::Ring => s.alpha = 0.6 * (1.0 - f),
        AnimKind::FadeTo { from } => s.alpha = from * (1.0 - f),
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    fn step_all(p: &mut Player, ms: f64) {
        let mut left = ms;
        while left > 0.0 {
            p.step(4.0);
            left -= 4.0;
        }
    }

    #[test]
    fn enter_fades_in_monotonically_and_settles_at_rest() {
        let _m = motion_on();
        let mut p = Player::new();
        p.play("title", Anim::text_swap(), false);
        let s0 = p.sample("title");
        assert!(s0.alpha < 0.05 && s0.dy > 4.0, "{s0:?}");
        assert!(p.busy());
        let mut last = 0.0;
        for _ in 0..40 {
            p.step(4.0);
            let a = p.sample("title").alpha;
            assert!(a >= last - 1e-5 && a <= 1.0, "alpha {a} after {last}");
            last = a;
        }
        step_all(&mut p, 400.0);
        assert!(!p.busy(), "idle once settled");
        assert_eq!(p.sample("title"), Sample::REST);
    }

    #[test]
    fn delay_keeps_the_start_state() {
        let _m = motion_on();
        let mut p = Player::new();
        p.play("row:1", Anim::row_stagger(3), false); // 84 ms delay
        step_all(&mut p, 40.0);
        let s = p.sample("row:1");
        assert_eq!(s.alpha, 0.0);
        assert!(s.dy > 5.0);
        step_all(&mut p, 1500.0);
        assert!(!p.busy());
    }

    #[test]
    fn exit_holds_until_released() {
        let _m = motion_on();
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
    fn a_leave_cancelling_an_enter_turns_around_without_a_jump() {
        let _m = motion_on();
        let mut p = Player::new();
        p.play("pop", Anim::popover_in(), false);
        step_all(&mut p, 80.0);
        let a = p.sample("pop");
        assert!(a.alpha > 0.1 && a.alpha < 0.95, "{a:?}");
        p.play("pop", Anim::popover_out(), false);
        let b = p.sample("pop");
        assert!((b.alpha - a.alpha).abs() < 0.02 && (b.dy - a.dy).abs() < 0.3, "no jump: {a:?} -> {b:?}");
        step_all(&mut p, 400.0);
        assert!(!p.busy() && p.sample("pop").alpha < 0.01);
    }

    #[test]
    fn reduced_motion_plays_nothing_and_a_looped_pulse_never_ends() {
        let _m = motion_on();
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
    fn pop_overshoots_a_little_and_lands_on_one() {
        let _m = motion_on();
        let mut p = Player::new();
        p.play("x", Anim::pop(0.8, 400), false);
        let mut peak = 0.0f32;
        for _ in 0..400 {
            p.step(4.0);
            peak = peak.max(p.sample("x").scale);
        }
        assert!(peak > 1.0 && peak < 1.2, "peak {peak}");
        assert!(!p.busy() && p.sample("x") == Sample::REST);
    }

    #[test]
    fn spin_makes_a_smooth_full_turn_and_shake_rings_out_to_zero() {
        let _m = motion_on();
        let mut p = Player::new();
        p.play("refresh", Anim::spin(), false);
        let mut last = 0.0;
        for _ in 0..100 {
            p.step(4.0);
            let r = p.sample("refresh").rot;
            assert!(r >= last - 1e-3 && r <= 361.0, "{r}");
            last = r;
        }
        assert!(last > 90.0 && last < 360.0, "{last}");
        step_all(&mut p, 1500.0);
        assert!(!p.busy());
        p.play("tile", Anim::shake(), false);
        let mut seen = 0.0f32;
        for _ in 0..40 {
            p.step(4.0);
            seen = seen.max(p.sample("tile").dx.abs());
        }
        assert!(seen > 1.0 && seen < 12.0, "wobbles: {seen}");
        step_all(&mut p, 800.0);
        assert!(!p.busy());
        assert_eq!(p.sample("tile").dx, 0.0);
    }
}
