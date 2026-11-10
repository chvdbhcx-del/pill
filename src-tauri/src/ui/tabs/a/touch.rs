//! `Touch`: glasskit's pointer / press / focus state machine ([`Interact`]) with liquid motion. The logic (hover, capture, click on release inside,
//! keyboard activation) is `Interact`'s; the three amounts the views draw with (hover, press, focus ring) are [`Flow`]s instead of fixed-time
//! tweens, so a hover that is cancelled half-way turns around with its momentum and a release springs back past rest.
//! The API mirrors the part of `Interact` the tabs use; read-only getters (`hovered`, `pressed`, `focused`, `enabled`) come through `Deref`.
use crate::ui::liquid::{Feel, Flow};
use glasskit::controls::{Interact, Pointer};
use glasskit::geom::Pt;
use glasskit::input::{InputEvent, KeyCode, Response};
use std::ops::Deref;

pub struct Touch {
    it: Interact,
    hov: Flow,
    prs: Flow,
    rng: Flow,
    ring_on: bool,
}

impl Deref for Touch {
    type Target = Interact;
    fn deref(&self) -> &Interact {
        &self.it
    }
}

impl Default for Touch {
    fn default() -> Touch {
        Touch::new()
    }
}

impl Touch {
    pub fn new() -> Touch {
        Touch { it: Interact::new(), hov: Flow::new(0.0, Feel::FADE), prs: Flow::new(0.0, Feel::SNAP), rng: Flow::new(0.0, Feel::FADE), ring_on: false }
    }

    fn sync(&mut self) {
        self.hov.to(self.it.hovered() as u8 as f32);
        self.prs.to(self.it.pressed() as u8 as f32);
        self.rng.to(self.ring_on as u8 as f32);
    }

    /// Hover amount 0..1.
    pub fn hover(&self) -> f32 {
        self.hov.at01()
    }

    /// Press amount: 1 held, a little below 0 on the release bounce.
    pub fn press(&self) -> f32 {
        self.prs.at()
    }

    /// Focus ring opacity 0..1.
    pub fn ring(&self) -> f32 {
        self.rng.at01()
    }

    /// 1 at rest, `depth` pressed, a springy overshoot on release.
    pub fn press_scale(&self, depth: f32) -> f32 {
        1.0 - (1.0 - depth) * self.press()
    }

    pub fn pointer(&mut self, ev: &InputEvent, hit: impl Fn(Pt) -> bool) -> (Response, Pointer) {
        let r = self.it.pointer(ev, hit);
        self.sync();
        r
    }

    pub fn activation_key(&mut self, ev: &InputEvent, keys: &[KeyCode]) -> (Response, bool) {
        let r = self.it.activation_key(ev, keys);
        self.sync();
        r
    }

    pub fn set_hover(&mut self, on: bool) -> bool {
        let r = self.it.set_hover(on);
        self.sync();
        r
    }

    pub fn set_press(&mut self, on: bool) {
        self.it.set_press(on);
        self.sync();
    }

    pub fn set_focus(&mut self, focused: bool, ring: bool) {
        self.it.set_focus(focused, ring);
        self.ring_on = focused && ring;
        self.sync();
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.it.set_enabled(on);
        self.sync();
    }

    /// Advance by `dt_ms`; true while any amount is still moving (ask for another frame).
    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.sync();
        self.hov.step(dt_ms);
        self.prs.step(dt_ms);
        self.rng.step(dt_ms);
        self.moving()
    }

    pub fn moving(&self) -> bool {
        self.hov.moving() || self.prs.moving() || self.rng.moving()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use glasskit::input::{Buttons, Modifiers, MouseButton};

    fn mv(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
    }
    fn down(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseDown { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }
    }
    fn up(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseUp { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE }
    }

    #[test]
    fn hover_fades_in_without_overshoot_and_idles() {
        let _m = motion_on();
        let mut t = Touch::new();
        let hit = |p: Pt| p.0 < 10.0;
        t.pointer(&mv(5.0, 5.0), hit);
        assert!(t.moving() || t.step(8.0));
        let mut last = 0.0;
        for _ in 0..30 {
            t.step(8.0);
            assert!(t.hover() >= last && t.hover() <= 1.0);
            last = t.hover();
        }
        assert!(last > 0.5);
        for _ in 0..100 {
            t.step(8.0);
        }
        assert!(!t.moving() && t.hover() == 1.0);
        // leaving turns around smoothly from where it is
        t.pointer(&mv(50.0, 5.0), hit);
        let a = t.hover();
        t.step(8.0);
        assert!((t.hover() - a).abs() < 0.2 && t.moving());
    }

    #[test]
    fn a_release_springs_back_past_rest_a_little() {
        let _m = motion_on();
        let mut t = Touch::new();
        let hit = |_: Pt| true;
        t.pointer(&down(1.0, 1.0), hit);
        for _ in 0..60 {
            t.step(8.0);
        }
        assert!(t.press_scale(0.9) < 0.91);
        t.pointer(&up(1.0, 1.0), hit);
        let mut low = 1.0f32;
        let mut peak = 0.0f32;
        for _ in 0..120 {
            t.step(8.0);
            low = low.min(t.press());
            peak = peak.max(t.press_scale(0.9));
        }
        assert!(low < 0.0 && low > -0.4, "overshoot {low}");
        assert!(peak > 1.0 && peak < 1.05, "{peak}");
        assert!(!t.moving() && t.press_scale(0.9) == 1.0);
    }
}
