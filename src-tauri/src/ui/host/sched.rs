//! The frame scheduler: pure logic deciding WHEN a window renders and what `dt` its view's `step` gets. No Win32, no clock of its own (the
//! host passes `now_ms`), so every rule here is unit-tested.
//!
//! Rules:
//! * A window renders only while it `wants_frame()`: something invalidated it (`request`: input, a message, `Cx::request_frame`) or the last
//!   `step` returned `true` (`animating`). Otherwise it costs nothing: the host does not even wait on its frame-latency handle.
//! * Frames are paced by vsync (the swap chain's waitable object) at the display's refresh rate. This scheduler never slows that down; the
//!   only throttles are an optional per-window `cap` that a caller chose explicitly (`set_cap`) and a runaway guard that engages only when
//!   frames come faster than half a refresh period (a swap chain that stopped pacing).
//! * `dt` after an idle gap is ONE nominal frame, never the gap: a spring that slept for ten seconds must not leap. During continuous
//!   animation `dt` is the real time since the previous frame, clamped to `MAX_DT_MS` (a stall, an occluded window, a debugger).

/// Longest `dt` a view is ever given.
pub const MAX_DT_MS: f64 = 100.0;
/// Frames closer than this fraction of a refresh period are held back (runaway guard, see `wait_ms`). Normal vsync frames are a full
/// period apart, so the guard never engages on a healthy window and never lowers the frame rate.
pub const GUARD: f64 = 0.5;

#[derive(Clone, Debug)]
pub struct FrameState {
    dirty: bool,
    animating: bool,
    /// the previous frame asked for another one, so `last_ms` is meaningful for `dt`
    active: bool,
    last_ms: f64,
    nominal_ms: f64,
    cap_ms: Option<f64>,
    frames: u64,
}

impl FrameState {
    /// A new window wants its first frame.
    pub fn new(refresh_hz: f32) -> FrameState {
        FrameState { dirty: true, animating: false, active: false, last_ms: f64::NEG_INFINITY, nominal_ms: nominal(refresh_hz), cap_ms: None, frames: 0 }
    }

    pub fn set_refresh_hz(&mut self, hz: f32) {
        self.nominal_ms = nominal(hz);
    }

    /// `Some(n)`: at most `n` frames per second (a caller's explicit choice); `None`: vsync pace.
    pub fn set_cap(&mut self, fps: Option<u32>) {
        self.cap_ms = fps.filter(|&n| n > 0).map(|n| 1000.0 / n as f64);
    }

    /// Something visible changed.
    pub fn request(&mut self) {
        self.dirty = true;
    }

    pub fn wants_frame(&self) -> bool {
        self.dirty || self.animating
    }

    pub fn animating(&self) -> bool {
        self.animating
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Milliseconds until a frame may start (0 = now), `None` when no frame is wanted. A vsync-paced frame is a whole refresh period after
    /// the last one, so only two things ever make this positive: the caller's explicit cap, and the runaway guard (`GUARD` of a period): a
    /// swap chain that stopped pacing (window off-screen, covered, display asleep) returns from `Present` at once, and without the guard
    /// the loop would spin at thousands of frames per second.
    pub fn wait_ms(&self, now_ms: f64) -> Option<f64> {
        if !self.wants_frame() {
            return None;
        }
        let gap = self.cap_ms.unwrap_or(0.0).max(self.nominal_ms * GUARD);
        Some((self.last_ms + gap - now_ms).max(0.0))
    }

    /// Starts a frame at `now_ms`: clears the invalidation and returns the `dt` for `View::step`.
    pub fn begin(&mut self, now_ms: f64) -> f64 {
        self.dirty = false;
        let dt = if self.active { (now_ms - self.last_ms).clamp(0.0, MAX_DT_MS) } else { self.nominal_ms };
        self.last_ms = now_ms;
        self.frames += 1;
        dt
    }

    /// Ends the frame: `keep` is what `View::step` returned (true = animate on).
    pub fn end(&mut self, keep: bool) {
        self.animating = keep;
        self.active = keep || self.dirty;
    }
}

fn nominal(hz: f32) -> f64 {
    1000.0 / if hz.is_finite() && hz >= 1.0 { hz as f64 } else { 60.0 }
}

/// One refresh period in whole microseconds of a display running at `hz` (60 when unknown). Used by the cursor poll, never to throttle drawing.
pub fn period_us(hz: u32) -> u64 {
    1_000_000 / if hz == 0 { 60 } else { hz as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_window_wants_one_frame_then_idles() {
        let mut f = FrameState::new(240.0);
        assert!(f.wants_frame());
        assert_eq!(f.begin(1000.0), 1000.0 / 240.0, "first frame: one nominal frame");
        f.end(false);
        assert!(!f.wants_frame(), "settled = zero work");
        assert_eq!(f.wait_ms(1010.0), None);
        assert_eq!(f.frames(), 1);
    }

    #[test]
    fn animation_runs_until_step_says_stop_with_real_dt() {
        let mut f = FrameState::new(144.0);
        f.begin(0.0);
        f.end(true);
        assert!(f.wants_frame() && f.animating());
        assert_eq!(f.begin(7.0), 7.0);
        f.end(true);
        assert_eq!(f.begin(13.5), 6.5);
        f.end(false);
        assert!(!f.wants_frame());
    }

    #[test]
    fn dt_after_an_idle_gap_is_one_nominal_frame_not_the_gap() {
        let mut f = FrameState::new(60.0);
        f.begin(0.0);
        f.end(false);
        f.request();
        let dt = f.begin(10_000.0);
        assert!((dt - 1000.0 / 60.0).abs() < 1e-9, "{dt}");
        f.end(true);
        // a stall while animating is clamped
        assert_eq!(f.begin(10_000.0 + 5_000.0), MAX_DT_MS);
    }

    #[test]
    fn an_invalidation_during_the_frame_keeps_the_clock_running() {
        let mut f = FrameState::new(60.0);
        f.begin(0.0);
        f.request(); // an event arrived while drawing
        f.end(false);
        assert!(f.wants_frame());
        assert_eq!(f.begin(5.0), 5.0, "the next frame continues the clock");
        f.end(false);
        assert!(!f.wants_frame());
    }

    #[test]
    fn the_default_never_throttles_and_a_cap_is_opt_in() {
        let mut f = FrameState::new(240.0);
        f.begin(0.0);
        f.end(true);
        // a healthy vsync frame is a whole period (4.17 ms) later: always ready
        assert_eq!(f.wait_ms(1000.0 / 240.0), Some(0.0), "vsync pace: always ready");
        // ...a frame 0.1 ms after the last one means the swap chain stopped pacing: held to half a period
        let w = f.wait_ms(0.1).unwrap();
        assert!((w - (1000.0 / 240.0 * GUARD - 0.1)).abs() < 1e-9, "{w}");
        f.set_cap(Some(30));
        assert!((f.wait_ms(10.0).unwrap() - (1000.0 / 30.0 - 10.0)).abs() < 1e-9);
        assert_eq!(f.wait_ms(100.0), Some(0.0));
        f.set_cap(None);
        assert_eq!(f.wait_ms(1000.0 / 240.0), Some(0.0));
        f.set_cap(Some(0));
        assert_eq!(f.wait_ms(1000.0 / 240.0), Some(0.0), "0 means no cap");
    }

    #[test]
    fn bad_refresh_rates_fall_back_to_60() {
        for hz in [0.0, f32::NAN, -5.0] {
            let mut f = FrameState::new(hz);
            assert!((f.begin(0.0) - 1000.0 / 60.0).abs() < 1e-9);
        }
        assert_eq!(period_us(0), 16_666);
        assert_eq!(period_us(240), 4_166);
        assert_eq!(period_us(144), 6_944);
    }
}
