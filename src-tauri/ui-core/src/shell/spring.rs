//! The four body springs (width, height, bottom radius, top radius) of pill.js `sp` / `MODES` / `tick` / `go` / `snap`.
//!
//! Semi-implicit Euler with `ceil(dt * 240)` sub-steps, exactly as the JS loop. `mode_params` is the parameter table the renderer can
//! feed into its own spring solver (glasskit): `(stiffness k, damping ratio z)` per channel; stiffness is in 1/s^2 (`c = 2*sqrt(k)*z`).
//! Notes carried over from the JS: expand ~230 ms with +8 % width overshoot; collapse ~270 ms and a squash BELOW the final size; tab switch:
//! the panel height reaches the target in ~170 ms and settles in ~470 ms.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub x: f64,
    pub v: f64,
    pub t: f64,
    pub k: f64,
    pub z: f64,
}

impl Spring {
    pub const fn new(k: f64, z: f64) -> Spring {
        Spring { x: 0.0, v: 0.0, t: 0.0, k, z }
    }
}

/// Animation mode of a retarget (`MODES` keys).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Expand,
    Collapse,
    /// after a click that closes the island (paste)
    Squash,
    /// tab switch: only the height is retuned
    Tab,
    /// capsule content change
    Resize,
}

/// `(k, z)` per channel; `None` = the channel keeps its current tuning in this mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModeParams {
    pub w: Option<(f64, f64)>,
    pub h: Option<(f64, f64)>,
    pub r: Option<(f64, f64)>,
    pub t: Option<(f64, f64)>,
}

pub fn mode_params(m: Mode) -> ModeParams {
    match m {
        Mode::Expand => ModeParams { w: Some((420.0, 0.6)), h: Some((420.0, 0.74)), r: Some((420.0, 0.6)), t: Some((420.0, 0.6)) },
        // damping is set so the undershoot stays inside the content (h near critical, w keeps a soft squash)
        Mode::Collapse => ModeParams { w: Some((166.0, 0.74)), h: Some((166.0, 0.88)), r: Some((166.0, 0.7)), t: Some((166.0, 0.7)) },
        Mode::Squash => ModeParams { w: Some((150.0, 0.7)), h: Some((150.0, 0.86)), r: Some((150.0, 0.7)), t: Some((150.0, 0.7)) },
        Mode::Tab => ModeParams { w: None, h: Some((250.0, 0.55)), r: None, t: None },
        Mode::Resize => ModeParams { w: Some((380.0, 0.57)), h: Some((380.0, 0.7)), r: Some((380.0, 0.6)), t: Some((380.0, 0.6)) },
    }
}

/// The whole `MODES` table, for a renderer that wants to build its own spring presets.
pub fn all_modes() -> [(Mode, ModeParams); 5] {
    [Mode::Expand, Mode::Collapse, Mode::Squash, Mode::Tab, Mode::Resize].map(|m| (m, mode_params(m)))
}

/// Target of the four channels (css px at the current scale).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Target {
    pub w: f64,
    pub h: f64,
    pub r: f64,
    pub t: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodySprings {
    pub w: Spring,
    pub h: Spring,
    pub r: Spring,
    pub t: Spring,
    /// `springBusy`
    pub busy: bool,
}

impl Default for BodySprings {
    fn default() -> Self {
        BodySprings { w: Spring::new(380.0, 0.62), h: Spring::new(380.0, 0.7), r: Spring::new(380.0, 0.62), t: Spring::new(380.0, 0.62), busy: false }
    }
}

impl BodySprings {
    /// `snap(tg)`: jump to the target, nothing animates.
    pub fn snap(&mut self, tg: &Target) {
        for (s, t) in [(&mut self.w, tg.w), (&mut self.h, tg.h), (&mut self.r, tg.r), (&mut self.t, tg.t)] {
            s.x = t;
            s.t = t;
            s.v = 0.0;
        }
        self.busy = false;
    }

    /// The retargeting half of `go()`: set targets and (re)tune the channels of `mode`; the caller decides snap vs animate.
    pub fn retarget(&mut self, tg: &Target, mode: Mode) {
        let mp = mode_params(mode);
        let was_busy = self.busy;
        for (s, t, p) in [(&mut self.w, tg.w, mp.w), (&mut self.h, tg.h, mp.h), (&mut self.r, tg.r, mp.r), (&mut self.t, tg.t, mp.t)] {
            s.t = t;
            // a capsule content resize must not retune a running expand / collapse
            if let Some((k, z)) = p {
                if !(mode == Mode::Resize && was_busy) {
                    s.k = k;
                    s.z = z;
                }
            }
        }
        self.busy = true;
    }

    /// One `tick(dt)` step of the springs (`dt` in seconds). Returns `(moving, settled)`: `moving` = `sMove`, `settled` = the frame in which
    /// `springBusy` turned off.
    pub fn step(&mut self, dt: f64) -> (bool, bool) {
        if !self.busy {
            return (false, false);
        }
        let n = ((dt * 240.0).ceil() as i64).max(1);
        let step = dt / n as f64;
        let mut s_move = false;
        for q in [&mut self.w, &mut self.h, &mut self.r, &mut self.t] {
            let c = 2.0 * q.k.sqrt() * q.z;
            for _ in 0..n {
                q.v += (-q.k * (q.x - q.t) - c * q.v) * step;
                q.x += q.v * step;
            }
            // px / px per s: below this nothing is visible
            if (q.x - q.t).abs() > 0.5 || q.v.abs() > 3.0 {
                s_move = true;
            } else {
                q.x = q.t;
                q.v = 0.0;
            }
        }
        let settled = !s_move;
        if settled {
            self.busy = false;
        }
        (s_move, settled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(sp: &mut BodySprings, ms: f64, dt_ms: f64) -> (f64, f64) {
        let (mut peak_w, mut min_h) = (0.0f64, f64::MAX);
        let mut t = 0.0;
        while t < ms {
            sp.step(dt_ms / 1000.0);
            peak_w = peak_w.max(sp.w.x);
            min_h = min_h.min(sp.h.x);
            t += dt_ms;
        }
        (peak_w, min_h)
    }

    #[test]
    fn mode_table_matches_pill_js() {
        assert_eq!(mode_params(Mode::Expand).h, Some((420.0, 0.74)));
        assert_eq!(mode_params(Mode::Collapse).w, Some((166.0, 0.74)));
        assert_eq!(mode_params(Mode::Collapse).h, Some((166.0, 0.88)));
        assert_eq!(mode_params(Mode::Squash).r, Some((150.0, 0.7)));
        assert_eq!(mode_params(Mode::Tab), ModeParams { w: None, h: Some((250.0, 0.55)), r: None, t: None });
        assert_eq!(mode_params(Mode::Resize).t, Some((380.0, 0.6)));
    }

    #[test]
    fn expand_overshoots_width_then_settles() {
        let mut sp = BodySprings::default();
        sp.snap(&Target { w: 144.0, h: 30.0, r: 15.0, t: 15.0 });
        let tg = Target { w: 680.0, h: 331.0, r: 30.0, t: 26.0 };
        sp.retarget(&tg, Mode::Expand);
        assert!(sp.busy);
        let (peak, _) = run(&mut sp, 3000.0, 1000.0 / 144.0);
        assert!(peak > 680.0 * 1.03 && peak < 680.0 * 1.2, "peak {peak}"); // soft overshoot (notes: ~8 %)
        assert!(!sp.busy);
        assert_eq!((sp.w.x, sp.h.x, sp.r.x, sp.t.x), (680.0, 331.0, 30.0, 26.0));
        assert_eq!(sp.w.v, 0.0);
    }

    #[test]
    fn collapse_height_is_near_critical_and_width_squashes() {
        let mut sp = BodySprings::default();
        sp.snap(&Target { w: 680.0, h: 331.0, r: 30.0, t: 26.0 });
        sp.retarget(&Target { w: 144.0, h: 30.0, r: 15.0, t: 15.0 }, Mode::Collapse);
        let (mut min_w, mut min_h) = (f64::MAX, f64::MAX);
        for _ in 0..180 {
            sp.step(1.0 / 60.0);
            min_w = min_w.min(sp.w.x);
            min_h = min_h.min(sp.h.x);
        }
        assert!(min_h > 20.0, "h stays inside the content: {min_h}"); // z 0.88: a small undershoot only
        assert!(min_w < 144.0, "w undershoots (soft squash): {min_w}"); // z 0.74
        assert_eq!(sp.h.x, 30.0);
        assert!(!sp.busy);
    }

    #[test]
    fn step_is_frame_rate_independent_enough() {
        let mk = || {
            let mut sp = BodySprings::default();
            sp.snap(&Target { w: 144.0, h: 30.0, r: 15.0, t: 15.0 });
            sp.retarget(&Target { w: 680.0, h: 331.0, r: 30.0, t: 26.0 }, Mode::Expand);
            sp
        };
        let (mut a, mut b) = (mk(), mk());
        for _ in 0..30 {
            a.step(1.0 / 60.0);
        }
        for _ in 0..60 {
            b.step(1.0 / 120.0);
        }
        assert!((a.w.x - b.w.x).abs() < 4.0, "{} vs {}", a.w.x, b.w.x);
    }

    #[test]
    fn resize_does_not_retune_a_running_expand() {
        let mut sp = BodySprings::default();
        sp.snap(&Target { w: 144.0, h: 30.0, r: 15.0, t: 15.0 });
        sp.retarget(&Target { w: 680.0, h: 331.0, r: 30.0, t: 26.0 }, Mode::Expand);
        assert_eq!(sp.w.k, 420.0);
        sp.retarget(&Target { w: 700.0, h: 331.0, r: 30.0, t: 26.0 }, Mode::Resize);
        assert_eq!((sp.w.k, sp.w.z), (420.0, 0.6)); // still the expand tuning
        assert_eq!(sp.w.t, 700.0); // but the target moved
        // idle springs do take the resize tuning
        let mut idle = BodySprings::default();
        idle.retarget(&Target { w: 10.0, h: 10.0, r: 1.0, t: 1.0 }, Mode::Resize);
        assert_eq!((idle.h.k, idle.h.z), (380.0, 0.7));
    }

    #[test]
    fn tab_mode_only_retunes_height() {
        let mut sp = BodySprings::default();
        sp.snap(&Target { w: 680.0, h: 331.0, r: 30.0, t: 26.0 });
        sp.retarget(&Target { w: 680.0, h: 417.0, r: 30.0, t: 26.0 }, Mode::Tab);
        assert_eq!((sp.h.k, sp.h.z), (250.0, 0.55));
        assert_eq!((sp.w.k, sp.w.z), (380.0, 0.62));
        run(&mut sp, 2000.0, 1000.0 / 60.0);
        assert_eq!(sp.h.x, 417.0);
    }

    #[test]
    fn idle_step_does_nothing_and_snap_clears_velocity() {
        let mut sp = BodySprings::default();
        assert_eq!(sp.step(0.016), (false, false));
        sp.w.v = 50.0;
        sp.snap(&Target { w: 5.0, h: 5.0, r: 1.0, t: 1.0 });
        assert_eq!((sp.w.x, sp.w.v, sp.busy), (5.0, 0.0, false));
    }

    #[test]
    fn substeps_scale_with_dt() {
        // a 50 ms frame (the dt clamp) must not blow up
        let mut sp = BodySprings::default();
        sp.snap(&Target { w: 144.0, h: 30.0, r: 15.0, t: 15.0 });
        sp.retarget(&Target { w: 680.0, h: 331.0, r: 30.0, t: 26.0 }, Mode::Expand);
        for _ in 0..200 {
            sp.step(0.05);
            assert!(sp.w.x.is_finite() && sp.w.x < 2000.0);
        }
        assert_eq!(sp.w.x, 680.0);
    }
}
