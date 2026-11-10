//! Liquid helpers local to the settings window. The motion engine itself is `crate::ui::liquid` (`Flow`, `Feel`); the window has its own
//! "Reduce motion" setting on top of the OS one (which a `Flow` honours by itself), so every retarget goes through [`go`].
use crate::ui::liquid::{Feel, Flow};

/// Advances `f` by `dt` seconds (the unit of this window's `step`); true while it still moves.
pub fn tick(f: &mut Flow, dt: f32) -> bool {
    f.step(dt as f64 * 1000.0);
    f.moving()
}

/// Heads for `to`, or lands there at once when the window's Reduce motion is on.
pub fn go(f: &mut Flow, to: f32, reduce: bool) {
    if reduce {
        f.jump(to);
    } else {
        f.to(to);
    }
}

/// Starts at `from` (no velocity) and flows to `to`.
pub fn from_to(f: &mut Flow, from: f32, to: f32, reduce: bool) {
    f.jump(from);
    go(f, to, reduce);
}

/// The wash of a found row / card: it swells, dips, swells again and drains (the old `hl-pulse` keyframes), but as one soft value that
/// chases a staged target instead of a keyframe line, so there are no corners.
pub struct Pulse {
    t: f32,
    f: Flow,
}

impl Pulse {
    pub fn new() -> Pulse {
        Pulse { t: 0.0, f: Flow::new(0.0, Feel::FADE) }
    }

    fn want(t: f32) -> f32 {
        match t {
            t if t < 0.35 => 0.34,
            t if t < 0.8 => 0.1,
            t if t < 1.2 => 0.28,
            _ => 0.0,
        }
    }

    /// Advances; false once the wash is gone.
    pub fn step(&mut self, dt: f32, reduced: bool) -> bool {
        self.t += dt;
        if reduced {
            return self.t < 1.7;
        }
        self.f.to(Pulse::want(self.t));
        self.f.step(dt as f64 * 1000.0);
        self.t < 1.2 || self.f.moving()
    }

    pub fn alpha(&self, reduced: bool) -> f32 {
        if reduced {
            return if self.t < 1.7 { 0.2 } else { 0.0 };
        }
        self.f.at01()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    #[test]
    fn go_flows_or_lands_and_from_to_starts_where_told() {
        let _m = motion_on();
        let mut f = Flow::new(0.0, Feel::FLOW);
        go(&mut f, 1.0, false);
        assert!(f.moving() && f.at() == 0.0, "it has not moved yet, it will on the next tick");
        assert!(tick(&mut f, 0.016) && f.at() > 0.0 && f.at() < 1.0);
        go(&mut f, 0.0, true);
        assert!(!f.moving() && f.at() == 0.0, "reduce: lands at once");
        from_to(&mut f, 0.3, 1.0, false);
        assert_eq!(f.at(), 0.3);
        assert!(f.moving());
    }

    #[test]
    fn the_pulse_swells_never_exceeds_its_peak_and_ends() {
        let _m = motion_on();
        let mut p = Pulse::new();
        let (mut peak, mut frames) = (0.0f32, 0);
        while p.step(0.008, false) {
            peak = peak.max(p.alpha(false));
            frames += 1;
            assert!(frames < 400, "it must end");
        }
        assert!(peak > 0.25 && peak <= 0.34 + 1e-3, "peak {peak}");
        assert!(p.alpha(false) < 0.01, "drained at the end");
        let mut r = Pulse::new();
        assert!(r.step(0.5, true) && (r.alpha(true) - 0.2).abs() < 1e-6, "reduced: a steady wash");
        assert!(!r.step(1.5, true));
    }
}
