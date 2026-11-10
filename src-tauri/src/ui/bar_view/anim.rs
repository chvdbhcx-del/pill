//! Plays the model's animation cues. `pill_ui_core::bar::Anim` is "start this move on that target"; the player turns each cue into two small
//! springs (a move / scale / height one and an opacity one, `crate::ui::liquid::Flow`) that the drawing code reads per target (`look`).
//! Nothing here is a time curve: a cue that is replaced half-way hands its position and velocity to the next one, and a cue is over when its
//! springs have settled. Under reduced motion `play` drops every cue and nothing is ever in flight.
//! Frame rate is never touched: the owner calls `step(dt)` once per frame and keeps frames coming while `busy()`.
use crate::ui::liquid::{Feel, Flow};
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

/// A running cue: two springs from 0 to 1 (`mv` = the move / scale / height progress, `fade` = opacity), aimed once the stagger delay is over.
#[derive(Clone, Debug)]
struct Active {
    cue: Anim,
    /// seconds since the cue was played (the delay is part of it)
    t: f32,
    mv: Flow,
    fade: Flow,
    /// the delay is over and the springs have been aimed
    started: bool,
    /// Flash: it has peaked and is on its way back
    back: bool,
}

/// How a cue moves: moves, scales and heights flow (pops are livelier), opacity fades, an exit is as quick as its cue says.
fn feels(c: &Anim) -> (Feel, Feel) {
    let dur = c.dur_ms as f32;
    match c.kind {
        Kind::Enter | Kind::Expand | Kind::Slide | Kind::Glide => (if c.ease == Ease::Pop { Feel::POP } else { Feel::FLOW }, Feel::FADE),
        Kind::Exit | Kind::Collapse => (Feel::FLOW.within(dur.max(120.0)), Feel::FADE.within(dur.max(120.0))),
        Kind::Pop => (Feel::POP, Feel::FADE),
        Kind::Flash => (Feel::FADE.within(120.0), Feel::FADE),
        Kind::Shake => (Feel::POP.within(dur.max(240.0)), Feel::FADE),
        Kind::DrawTick => (Feel::POP.within(dur.max(240.0)), Feel::FADE),
    }
}

impl Active {
    fn new(cue: &Anim) -> Active {
        let (mv, fade) = feels(cue);
        // a shake is a kick on a spring that rests at 0 (it swings out and back); everything else runs 0 -> 1
        let mv = if cue.kind == Kind::Shake { Flow::px(0.0, mv) } else { Flow::new(0.0, mv) };
        Active { cue: cue.clone(), t: 0.0, mv, fade: Flow::new(0.0, fade), started: false, back: false }
    }

    fn start(&mut self) {
        self.started = true;
        if self.cue.kind == Kind::Shake {
            self.mv.nudge(-self.cue.dx * 22.0);
        } else {
            self.mv.to(1.0);
            self.fade.to(1.0);
        }
    }

    fn advance(&mut self, dt_ms: f64) {
        self.t += (dt_ms / 1000.0) as f32;
        if !self.started && self.t >= self.cue.delay_ms as f32 / 1000.0 {
            self.start();
        }
        if !self.started {
            return;
        }
        self.mv.step(dt_ms);
        self.fade.step(dt_ms);
        if self.cue.kind == Kind::Flash && !self.back && self.mv.at() >= 0.9 {
            self.back = true;
            self.mv.set_feel(Feel::FADE.within(480.0));
            self.mv.to(0.0);
        }
    }

    fn done(&self) -> bool {
        self.started && !self.mv.moving() && !self.fade.moving() && (self.cue.kind != Kind::Flash || self.back)
    }

    /// Takes over the progress of the cue this one replaces, so an interrupted move turns around (an Enter and an Exit, an Expand and a
    /// Collapse are each other's mirror image) instead of jumping back to its start.
    fn inherit(&mut self, old: &Active) {
        if !old.started || self.cue.delay_ms > 0 || self.cue.kind == Kind::Flash || self.cue.kind == Kind::Shake {
            return;
        }
        let mirror = |a: Kind, b: Kind| matches!((a, b), (Kind::Enter, Kind::Exit) | (Kind::Exit, Kind::Enter) | (Kind::Expand, Kind::Collapse) | (Kind::Collapse, Kind::Expand));
        let sign = if old.cue.kind == self.cue.kind {
            1.0
        } else if mirror(old.cue.kind, self.cue.kind) {
            -1.0
        } else {
            return;
        };
        let take = |from: &Flow, to: &mut Flow| {
            let mut f = Flow::new(if sign > 0.0 { from.at() } else { 1.0 - from.at() }, to.feel());
            f.nudge(sign * from.velocity());
            f.to(1.0);
            *to = f;
        };
        take(&old.mv, &mut self.mv);
        take(&old.fade, &mut self.fade);
        self.started = true;
        self.t = self.cue.delay_ms as f32 / 1000.0;
    }
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

    /// Starts a cue. A new cue on a target replaces the running one (taking over its progress and velocity) and forgets a held end state.
    pub fn play(&mut self, a: &Anim) {
        if self.reduced {
            return;
        }
        self.held.retain(|(t, _)| *t != a.target);
        // an Enter / Pop / Expand supersedes an Exit of the same element and vice versa; effects of other kinds (Flash, Shake) stack
        let group = |k: Kind| matches!(k, Kind::Enter | Kind::Exit | Kind::Collapse | Kind::Expand | Kind::Pop | Kind::Glide);
        let same = |x: &Active| x.cue.target == a.target && group(x.cue.kind) == group(a.kind) && !(x.cue.kind != a.kind && matches!(a.kind, Kind::Flash | Kind::Shake));
        let mut next = Active::new(a);
        if let Some(old) = self.active.iter().find(|x| same(x)) {
            next.inherit(old);
        }
        self.active.retain(|x| !same(x));
        self.active.push(next);
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
        let dt_ms = (dt.max(0.0) * 1000.0) as f64;
        for a in &mut self.active {
            a.advance(dt_ms);
        }
        let mut done = Vec::new();
        self.active.retain(|a| {
            if a.done() {
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
                l = Self::apply(l, c, 1.0, 1.0);
            }
        }
        for a in self.active.iter().filter(|a| a.cue.target == *t) {
            l = Self::apply(l, &a.cue, a.mv.at(), a.fade.at01());
        }
        l
    }

    /// One cue at move progress `e` (it may overshoot: that is the bounce) and opacity progress `f` (0..1).
    fn apply(mut l: Look, c: &Anim, e: f32, f: f32) -> Look {
        match c.kind {
            Kind::Enter => {
                l.alpha *= f;
                l.dx += (1.0 - e) * c.dx;
                l.dy += (1.0 - e) * c.dy;
                let (sx, sy) = Self::scale_pair(&c.target, c.from_scale, e);
                l.sx *= sx;
                l.sy *= sy;
            }
            Kind::Exit => {
                l.alpha *= 1.0 - f;
                l.dx += e * c.dx;
                l.dy += e * c.dy;
            }
            // a height never goes below 0 or above its natural size: the bounce of a fold would push the rows around
            Kind::Collapse => l.height *= (1.0 - e).clamp(0.0, 1.0),
            Kind::Expand => l.height *= e.clamp(0.0, 1.0),
            Kind::Pop => {
                l.alpha *= f;
                let s = lerp(c.from_scale, 1.0, e);
                l.sx *= s;
                l.sy *= s;
                l.rot_deg += lerp(c.from_rotate_deg, 0.0, e);
            }
            Kind::Flash => l.flash = e.clamp(0.0, 1.0),
            Kind::Shake => l.dx += e,
            Kind::DrawTick => l.draw = e.clamp(0.0, 1.0),
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
    use crate::ui::host::testing::motion_on;
    use pill_ui_core::bar::anim as cue;

    /// Steps at 8 ms until idle; the ms it took (capped at 3 s).
    fn settle(p: &mut AnimPlayer) -> f32 {
        let mut t = 0.0;
        while p.step(0.008) && t < 3000.0 {
            t += 8.0;
        }
        t
    }

    #[test]
    fn enter_starts_softly_fades_up_and_settles() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::message_enter());
        let l0 = p.look(&Target::Message);
        assert_eq!(l0.alpha, 0.0);
        assert!((l0.dy - 8.0).abs() < 1e-3);
        p.step(0.016);
        let first = p.look(&Target::Message);
        assert!(first.alpha < 0.3 && first.alpha > 0.0, "inertia: it does not start at speed: {}", first.alpha);
        let mut prev = first.alpha;
        for _ in 0..40 {
            p.step(0.008);
            let l = p.look(&Target::Message);
            assert!(l.alpha >= prev - 1e-6 && l.alpha <= 1.0, "opacity rises and never overshoots");
            prev = l.alpha;
        }
        assert!(prev > 0.9);
        let ms = settle(&mut p);
        assert!(ms < 600.0 * 1.5, "settled after {ms} ms");
        assert!(!p.busy() && p.look(&Target::Message).is_rest(), "idle and exactly at rest afterwards");
    }

    #[test]
    fn body_pop_squashes_vertically_more_and_overshoots_a_little() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::body_pop());
        let l = p.look(&Target::Body);
        assert!(l.sy < l.sx && (l.sx - 0.96).abs() < 1e-3 && (l.sy - 0.9).abs() < 1e-3);
        assert!(l.dy < -17.0);
        let (mut max_sy, mut max_alpha) = (0.0f32, 0.0f32);
        while p.step(0.008) {
            let l = p.look(&Target::Body);
            max_sy = max_sy.max(l.sy);
            max_alpha = max_alpha.max(l.alpha);
        }
        assert!(max_sy > 1.0 && max_sy < 1.2, "a soft bounce: {max_sy}");
        assert!(max_alpha <= 1.0);
    }

    #[test]
    fn exit_holds_for_lists_and_panels_but_not_for_rows() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::list_step_aside());
        p.play(&cue::row_remove(2));
        settle(&mut p);
        assert!(p.is_held(&Target::List));
        assert_eq!(p.look(&Target::List).alpha, 0.0);
        assert!(!p.is_held(&Target::Row(2)), "a row index is reused by the next render");
        p.play(&cue::list_back());
        assert!(!p.is_held(&Target::List), "the list coming back clears the held exit");
    }

    #[test]
    fn collapse_ends_folded_and_expand_unfolds_without_leaving_0_to_1() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::panel_collapse());
        for _ in 0..16 {
            p.step(0.008);
        }
        let h = p.look(&Target::PanelBox).height;
        assert!(h > 0.0 && h < 1.0, "folding: {h}");
        while p.step(0.008) {
            let h = p.look(&Target::PanelBox).height;
            assert!((0.0..=1.0).contains(&h));
        }
        assert_eq!(p.look(&Target::PanelBox).height, 0.0);
        p.play(&cue::group_expand(0));
        assert_eq!(p.look(&Target::Group(0)).height, 0.0);
        settle(&mut p);
        assert!(p.look(&Target::Group(0)).is_rest());
    }

    #[test]
    fn stagger_delays_the_start() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::part_enter(3));
        p.step(0.05);
        assert_eq!(p.look(&Target::PanelPart(3)).alpha, 0.0, "84 ms stagger: still waiting");
        for _ in 0..20 {
            p.step(0.008);
        }
        assert!(p.look(&Target::PanelPart(3)).alpha > 0.0);
    }

    #[test]
    fn reduced_motion_plays_nothing() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::list_step_aside());
        p.set_reduced(true);
        assert!(!p.busy() && p.look(&Target::List).is_rest());
        p.play(&cue::body_pop());
        assert!(!p.busy());
    }

    #[test]
    fn shake_swings_both_ways_and_returns() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::shake(Target::PanelStep));
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        let (mut t, mut last_off) = (0.0, 0.0);
        while p.step(0.008) && t < 3000.0 {
            let dx = p.look(&Target::PanelStep).dx;
            lo = lo.min(dx);
            hi = hi.max(dx);
            if dx.abs() > 0.3 {
                last_off = t;
            }
            t += 8.0;
        }
        assert!(lo < -3.0 && lo > -9.0 && hi > 0.5, "swings out and back: {lo} .. {hi}");
        assert!(last_off < 120.0 + 420.0 * 1.5, "calmed down by {last_off} ms");
        assert!(t < 2000.0, "idle after {t} ms");
        assert_eq!(p.look(&Target::PanelStep).dx, 0.0);
    }

    #[test]
    fn flash_rises_falls_and_idles() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::row_flash(1));
        let mut peak = 0.0f32;
        let mut t = 0.0;
        while p.step(0.008) && t < 3000.0 {
            peak = peak.max(p.look(&Target::Row(1)).flash);
            t += 8.0;
        }
        assert!(peak > 0.85 && peak <= 1.0, "peak {peak}");
        assert!(t < 700.0 * 1.5);
        assert_eq!(p.look(&Target::Row(1)).flash, 0.0, "no cue left: the neutral value");
    }

    #[test]
    fn an_interrupted_exit_turns_around_without_a_jump() {
        let _m = motion_on();
        let mut p = AnimPlayer::new();
        p.play(&cue::message_enter());
        for _ in 0..30 {
            p.step(0.008);
        }
        p.play(&cue::message_exit());
        for _ in 0..6 {
            p.step(0.008);
        }
        let a = p.look(&Target::Message).alpha;
        assert!(a > 0.05 && a < 1.0, "part-way: {a}");
        p.play(&cue::message_enter());
        let b = p.look(&Target::Message).alpha;
        assert!((a - b).abs() < 0.02, "the enter carries on from where the exit was: {a} vs {b}");
        settle(&mut p);
        assert!(p.look(&Target::Message).is_rest());
    }
}
