//! Two slow, deliberate animations of the island that outlive their event: the soft glow while a timer or reminder waits for an answer
//! (`alert:wait`, from alert.rs) and the small ring next to the clock while a download runs (`dl:ring`, from the download provider when
//! Settings > Live events > "Download as a ring" is on). Both are fed by [`LiveCmd::Broadcast`] messages and move on [`Flow`]s only.
//!
//! Idle cost: neither is allowed to keep the window at the display's full rate. [`Ambient::step`] reports what is *transient* (a fade, a
//! glide: full rate) apart from what is *ambient* (the breathing, the turning arc): the view lets `pace_ambient` cap the latter.
use crate::ui::liquid::{Feel, Flow};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::Color;
use serde_json::Value;

const AMBER: (u8, u8, u8) = (0xe8, 0xa0, 0x40);
/// The glow fades in/out in this time; one breath (in or out) takes this long.
const GLOW_FADE: Feel = Feel::FADE.within(520.0);
const BREATH: Feel = Feel::FADE.within(1500.0);
const RING_GLIDE: Feel = Feel::FADE.within(420.0);
/// The turning arc of an unknown-size download: a spring chasing a target that keeps moving a turn ahead.
const SPIN: Feel = Feel::DRAG.within(900.0);

/// The glow around the island while an alert waits.
pub struct WaitGlow {
    on: bool,
    fade: Flow,
    breath: Flow,
    inhale: bool,
}

impl Default for WaitGlow {
    fn default() -> Self {
        WaitGlow { on: false, fade: Flow::new(0.0, GLOW_FADE), breath: Flow::new(0.5, BREATH), inhale: true }
    }
}

impl WaitGlow {
    pub fn set(&mut self, on: bool) {
        if on && !self.on {
            self.inhale = true;
            self.breath.to(1.0);
        }
        self.on = on;
    }

    /// (transient: the fade is moving, ambient: breathing).
    pub fn step(&mut self, reduced: bool, dt_ms: f64) -> (bool, bool) {
        self.fade.to(if self.on { 1.0 } else { 0.0 });
        if reduced {
            // no breathing: a steady soft glow
            self.fade.jump(self.fade.target());
            self.breath.jump(0.5);
            return (false, false);
        }
        if self.on && (self.breath.at() - self.breath.target()).abs() < 0.12 {
            self.inhale = !self.inhale;
            self.breath.to(if self.inhale { 1.0 } else { 0.0 });
        }
        self.fade.step(dt_ms);
        self.breath.step(dt_ms);
        (self.fade.moving(), self.on)
    }

    /// 0..1: how strongly the glow shows right now.
    pub fn strength(&self) -> f32 {
        self.fade.at01() * (0.55 + 0.45 * self.breath.at01())
    }

    /// Behind the body: a soft amber halo.
    pub fn paint(&self, cv: &Canvas, body: Rect, radius: f32, s: f32) {
        let k = self.strength();
        if k < 0.01 {
            return;
        }
        let c = Color::rgb8(AMBER.0, AMBER.1, AMBER.2);
        cv.shadow_rr(body, radius, 16.0 * s, 0.0, c.alpha(0.62 * k));
        cv.shadow_rr(body, radius, 6.0 * s, 0.0, c.alpha(0.42 * k));
    }
}

/// The ring next to the clock. `value` None = the size is unknown: a short arc turns.
pub struct DlRing {
    on: bool,
    value: Option<f32>,
    glide: Flow,
    spin: Flow,
}

impl Default for DlRing {
    fn default() -> Self {
        DlRing { on: false, value: None, glide: Flow::new(0.0, RING_GLIDE), spin: Flow::px(0.0, SPIN) }
    }
}

impl DlRing {
    pub fn set(&mut self, on: bool, value: Option<f32>) {
        if on && !self.on {
            self.glide.jump(0.0);
        }
        self.on = on;
        self.value = value.filter(|_| on);
        if let Some(v) = self.value {
            self.glide.to(v.clamp(0.0, 1.0));
        }
    }

    pub fn shown(&self) -> bool {
        self.on
    }

    /// (transient: the arc is gliding, ambient: the arc is turning).
    pub fn step(&mut self, reduced: bool, dt_ms: f64) -> (bool, bool) {
        if !self.on {
            return (false, false);
        }
        if reduced {
            self.glide.jump(self.glide.target());
            self.spin.jump(0.0);
            return (false, false);
        }
        let turning = self.value.is_none();
        if turning && self.spin.target() - self.spin.at() < 150.0 {
            self.spin.to(self.spin.target() + 360.0);
        }
        self.glide.step(dt_ms);
        self.spin.step(dt_ms);
        (self.glide.moving(), turning)
    }

    /// (progress 0..1 or -1 while the size is unknown, angle of the turning arc in degrees) for painting.
    pub fn paint(&self) -> (f32, f32) {
        if !self.on {
            return (0.0, 0.0);
        }
        (if self.value.is_some() { self.glide.at01() } else { -1.0 }, self.spin.at().rem_euclid(360.0))
    }
}

/// What the view did with a broadcast.
#[derive(Debug, PartialEq)]
pub enum Taken {
    /// not ours: pass it on to the tabs
    No,
    Yes,
    /// the ring appeared / left: the clock row needs (or no longer needs) its slot
    RingSlot(bool),
}

#[derive(Default)]
pub struct Ambient {
    pub glow: WaitGlow,
    pub ring: DlRing,
}

impl Ambient {
    pub fn take(&mut self, name: &str, p: &Value) -> Taken {
        let on = p.get("on").and_then(Value::as_bool).unwrap_or(false);
        match name {
            "alert:wait" => {
                self.glow.set(on);
                Taken::Yes
            }
            "dl:ring" => {
                let was = self.ring.shown();
                self.ring.set(on, p.get("value").and_then(Value::as_f64).map(|v| v as f32));
                if was != on { Taken::RingSlot(on) } else { Taken::Yes }
            }
            _ => Taken::No,
        }
    }

    /// One frame: (transient, ambient).
    pub fn step(&mut self, reduced: bool, dt_ms: f64) -> (bool, bool) {
        let (g, r) = (self.glow.step(reduced, dt_ms), self.ring.step(reduced, dt_ms));
        (g.0 || r.0, g.1 || r.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use serde_json::json;

    fn run(a: &mut Ambient, frames: usize) -> (bool, bool) {
        let mut last = (false, false);
        for _ in 0..frames {
            last = a.step(false, 16.0);
        }
        last
    }

    #[test]
    fn the_glow_fades_in_breathes_and_leaves_without_frames() {
        let _m = motion_on();
        let mut a = Ambient::default();
        assert_eq!(a.step(false, 16.0), (false, false), "idle: nothing asks for frames");
        assert_eq!(a.take("alert:wait", &json!({"on": true})), Taken::Yes);
        let (tr, amb) = a.step(false, 16.0);
        assert!(tr && amb, "fading in");
        let (tr, amb) = run(&mut a, 120);
        assert!(!tr && amb, "faded in; only the breathing is left, which is ambient");
        // it breathes: the strength rises and falls, never leaves 0.1..1
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..600 {
            a.step(false, 16.0);
            let k = a.glow.strength();
            lo = lo.min(k);
            hi = hi.max(k);
        }
        assert!(hi - lo > 0.15, "it breathes: {lo}..{hi}");
        assert!(lo > 0.3 && hi <= 1.0, "stays a soft glow: {lo}..{hi}");
        a.take("alert:wait", &json!({"on": false}));
        let (_, amb) = run(&mut a, 1);
        assert!(!amb, "answered: no more ambient frames");
        assert_eq!(run(&mut a, 200), (false, false), "and the window idles again");
        assert_eq!(a.glow.strength(), 0.0);
    }

    #[test]
    fn reduced_motion_shows_a_steady_glow_and_asks_for_no_frames() {
        let _m = motion_on();
        let mut a = Ambient::default();
        a.take("alert:wait", &json!({"on": true}));
        assert_eq!(a.step(true, 16.0), (false, false));
        let k = a.glow.strength();
        assert!(k > 0.3);
        a.step(true, 16.0);
        assert_eq!(a.glow.strength(), k, "steady");
    }

    #[test]
    fn the_ring_turns_when_the_size_is_unknown_and_glides_to_a_known_percent() {
        let _m = motion_on();
        let mut a = Ambient::default();
        assert_eq!(a.take("dl:ring", &json!({"on": true, "value": null})), Taken::RingSlot(true));
        let (_, amb) = run(&mut a, 5);
        assert!(amb, "turning is ambient");
        let a0 = a.ring.paint();
        assert_eq!(a0.0, -1.0);
        run(&mut a, 40);
        assert_ne!(a.ring.paint().1, a0.1, "the arc has moved");
        assert!(a.ring.paint().1 < 360.0);
        // a known percent: no more turning, the arc glides to it
        assert_eq!(a.take("dl:ring", &json!({"on": true, "value": 0.25})), Taken::Yes);
        let (tr, amb) = a.step(false, 16.0);
        assert!(tr && !amb);
        run(&mut a, 120);
        assert!((a.ring.paint().0 - 0.25).abs() < 0.01);
        assert_eq!(run(&mut a, 1), (false, false), "settled: idle");
        assert_eq!(a.take("dl:ring", &json!({"on": false})), Taken::RingSlot(false));
        assert_eq!(run(&mut a, 3), (false, false));
    }

    #[test]
    fn other_broadcasts_pass_through() {
        let mut a = Ambient::default();
        assert_eq!(a.take("voice:state", &json!({"on": true})), Taken::No);
    }
}
