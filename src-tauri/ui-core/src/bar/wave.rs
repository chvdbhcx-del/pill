//! The liquid outline along the bar's bottom edge (the canvas of bar.js): two waves drift along it, a sag follows the cursor on a spring and every
//! keystroke kicks it. The curve is the body's own bottom edge: it only ever bulges DOWN from the straight edge (a soft `max(y, 0)`), tapers to
//! zero slope where the corner arcs take over, and the outline dissolves over the last 15 % at both ends (`GRADIENT_STOPS`).
//!
//! Runs only while the bar is shown; under reduced motion it is drawn once and never stepped. The renderer calls `step(now_ms)` every frame at
//! its own frame rate (never throttled) and draws `points()`.

/// The canvas starts this many px above the body's bottom edge.
pub const UP: f32 = 4.0;
/// Canvas height.
pub const STRIP: f32 = 16.0;
/// The body's bottom corner radius: the straight part between the corners is the wave.
pub const INSET: f32 = 24.0;
/// Wave step along x.
pub const DX: f32 = 3.0;
/// Alpha stops of the horizontal outline gradient: (position 0..1, alpha).
pub const GRADIENT_STOPS: [(f32, f32); 6] = [(0.0, 0.0), (0.075, 0.3), (0.15, 1.0), (0.85, 1.0), (0.925, 0.3), (1.0, 0.0)];
/// Outline width in px.
pub const LINE_WIDTH: f32 = 2.0;

fn smooth(e: f32) -> f32 {
    let e = e.clamp(0.0, 1.0);
    e * e * (3.0 - 2.0 * e)
}

#[derive(Clone, Debug)]
pub struct Wave {
    w: f32,
    ph: f32,
    run: bool,
    last: f64,
    mx: f32,
    mv: f32,
    tx: f32,
    near: f32,
    inside: f32,
    kick: f32,
    kv: f32,
}

impl Default for Wave {
    fn default() -> Wave {
        Wave { w: 40.0, ph: 0.0, run: false, last: 0.0, mx: 0.0, mv: 0.0, tx: 0.0, near: 0.0, inside: 0.0, kick: 0.0, kv: 0.0 }
    }
}

impl Wave {
    pub fn new() -> Wave {
        Wave::default()
    }

    /// Width of the curve for a body `body_width` px wide (`max(40, round(width - 2 * INSET))`). Returns true when it changed (re-cut the gradient).
    pub fn set_body_width(&mut self, body_width: f32) -> bool {
        let w = (body_width - 2.0 * INSET).round().max(40.0);
        let changed = (w - self.w).abs() > f32::EPSILON;
        self.w = w;
        changed
    }
    pub fn width(&self) -> f32 {
        self.w
    }

    /// Start stepping (the bar is shown and motion is allowed). Under reduced motion or a hidden window the curve is only drawn once.
    pub fn start(&mut self, now_ms: f64, reduce_motion: bool, window_hidden: bool) {
        if self.run || reduce_motion || window_hidden {
            return;
        }
        self.run = true;
        self.last = now_ms;
    }
    pub fn stop(&mut self) {
        self.run = false;
    }
    pub fn running(&self) -> bool {
        self.run
    }

    /// Every keystroke kicks the sag (`LW.kv += 45`).
    pub fn kick(&mut self) {
        self.kv += 45.0;
    }
    /// The cursor moved over the body; `x` is relative to the wave's left edge.
    pub fn mouse(&mut self, x: f32) {
        self.tx = x;
        self.inside = 1.0;
    }
    pub fn mouse_leave(&mut self) {
        self.inside = 0.0;
    }

    /// One frame. No-op while stopped. `dt` is clamped to 50 ms so a stalled frame does not blow the spring up.
    pub fn step(&mut self, now_ms: f64) {
        if !self.run {
            return;
        }
        let dt = (((now_ms - self.last) / 1000.0) as f32).clamp(0.0, 0.05);
        let dt = if dt == 0.0 { 0.016 } else { dt };
        self.last = now_ms;
        self.ph += dt * 3.6; // ~ 25 px/s at the 56 px wavelength
        self.mv += ((self.tx - self.mx) * 90.0 - self.mv * 11.0) * dt; // the sag has mass: it follows the cursor late and overshoots a little
        self.mx += self.mv * dt;
        self.near += (self.inside - self.near) * (1.0 - (-dt * 7.0).exp());
        self.kv += (-self.kick * 160.0 - self.kv * 9.0) * dt;
        self.kick += self.kv * dt;
    }

    /// The outline as `(x, y)` pairs in canvas px (y measured from the top of the canvas, which sits `UP` above the body's bottom edge).
    pub fn points(&self) -> Vec<(f32, f32)> {
        let w = self.w;
        let a = 2.2 * (1.0 + (self.kick * 0.1).clamp(-0.4, 1.2));
        let tau = std::f32::consts::TAU;
        let mut pts = Vec::with_capacity((w / DX) as usize + 2);
        let mut i = 0usize;
        loop {
            let x = (i as f32 * DX).min(w);
            let u = (x - self.mx) / 50.0;
            let y = 2.0 + a * ((x / 56.0) * tau - self.ph).sin() + 1.1 * ((x / 170.0) * tau + self.ph * 0.4).sin() + self.near * 3.4 * (-u * u).exp();
            pts.push((x, UP + smooth(x.min(w - x) / 40.0) * 0.5 * (y + (y * y + 1.5).sqrt())));
            if x >= w {
                break;
            }
            i += 1;
        }
        pts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_follows_the_body_with_a_floor() {
        let mut w = Wave::new();
        assert!(w.set_body_width(560.0));
        assert_eq!(w.width(), 512.0);
        assert!(!w.set_body_width(560.0));
        w.set_body_width(60.0);
        assert_eq!(w.width(), 40.0, "never narrower than 40");
    }

    #[test]
    fn the_curve_covers_the_width_and_tapers_to_the_corners() {
        let mut w = Wave::new();
        w.set_body_width(560.0);
        let p = w.points();
        assert_eq!(p.first().unwrap().0, 0.0);
        assert_eq!(p.last().unwrap().0, 512.0);
        // at the very ends smooth() is 0: the curve sits on the straight edge
        assert!((p[0].1 - UP).abs() < 1e-5 && (p.last().unwrap().1 - UP).abs() < 1e-5);
        // it only bulges down from the straight edge
        assert!(p.iter().all(|&(_, y)| y >= UP - 1e-4));
        assert_eq!(p.len(), 512 / 3 + 2);
    }

    #[test]
    fn it_does_not_move_until_started_and_reduced_motion_never_starts() {
        let mut w = Wave::new();
        w.set_body_width(560.0);
        let before = w.points();
        w.step(1000.0);
        assert_eq!(w.points(), before);
        w.start(0.0, true, false);
        assert!(!w.running(), "reduced motion draws once");
        w.start(0.0, false, true);
        assert!(!w.running(), "a hidden window does not animate");
        w.start(0.0, false, false);
        assert!(w.running());
        w.step(16.0);
        w.step(32.0);
        assert_ne!(w.points(), before, "the waves drift");
        w.stop();
        let frozen = w.points();
        w.step(5000.0);
        assert_eq!(w.points(), frozen);
    }

    #[test]
    fn keystrokes_kick_and_the_kick_decays() {
        let mut w = Wave::new();
        w.set_body_width(560.0);
        w.start(0.0, false, false);
        w.kick();
        let mut peak: f32 = 0.0;
        let mut t = 0.0;
        for _ in 0..30 {
            t += 16.0;
            w.step(t);
            peak = peak.max(w.kick.abs());
        }
        assert!(peak > 0.1, "the sag swells: {peak}");
        for _ in 0..600 {
            t += 16.0;
            w.step(t);
        }
        assert!(w.kick.abs() < 0.01 && w.kv.abs() < 0.05, "and settles");
    }

    #[test]
    fn the_cursor_pulls_the_sag_with_a_spring() {
        let mut w = Wave::new();
        w.set_body_width(560.0);
        w.start(0.0, false, false);
        w.mouse(300.0);
        let mut t = 0.0;
        for _ in 0..120 {
            t += 16.0;
            w.step(t);
        }
        assert!((w.mx - 300.0).abs() < 5.0, "the sag arrived: {}", w.mx);
        assert!(w.near > 0.9);
        w.mouse_leave();
        for _ in 0..120 {
            t += 16.0;
            w.step(t);
        }
        assert!(w.near < 0.05);
    }

    #[test]
    fn a_stalled_frame_is_clamped() {
        let mut w = Wave::new();
        w.start(0.0, false, false);
        w.kick();
        w.step(10_000.0);
        assert!(w.kick.is_finite() && w.kick.abs() < 5.0);
        let g = GRADIENT_STOPS;
        assert_eq!((g[0].1, g[2].1, g[3].1, g[5].1), (0.0, 1.0, 1.0, 0.0));
    }
}
