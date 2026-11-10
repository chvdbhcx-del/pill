//! The mouse wheel over the timer ring: one notch = one minute (Shift: one second), wheel up = more time. The ring answers every notch
//! with a liquid tick (a small push on a spring: the ring swells a hair and the clock shifts in the wheel's direction, then both settle).
//! A running timer is restarted from its new remaining time, so its notches are collected and sent 200 ms after the last one (one write
//! per burst, not one per notch); the clock of a timer that is not running follows at once.
use crate::ui::liquid::{Feel, Flow};
use glasskit::input::WHEEL_NOTCH_PX;

/// Seconds added per notch.
pub const MINUTE_S: i64 = 60;
pub const SHIFT_S: i64 = 1;
const FLUSH_MS: f64 = 200.0;

pub struct RingWheel {
    /// fractional notches left over (touchpads send many small deltas)
    acc: f32,
    /// seconds collected for a running timer, and the time since the last notch
    pending: i64,
    idle_ms: f64,
    /// signed tick: pushed by a notch, settles back to 0
    tick: Flow,
}

impl Default for RingWheel {
    fn default() -> Self {
        RingWheel::new()
    }
}

impl RingWheel {
    pub fn new() -> RingWheel {
        RingWheel { acc: 0.0, pending: 0, idle_ms: 0.0, tick: Flow::new(0.0, Feel::SNAP) }
    }

    /// Whole notches in a wheel delta (css px, wheel up = negative); positive = more time.
    pub fn notches(&mut self, dy: f32) -> i32 {
        self.acc += -dy / WHEEL_NOTCH_PX;
        let n = self.acc.trunc();
        self.acc -= n;
        n as i32
    }

    /// Seconds for `n` notches.
    pub fn seconds(n: i32, shift: bool) -> i64 {
        n as i64 * if shift { SHIFT_S } else { MINUTE_S }
    }

    /// The tick feedback of `n` notches.
    pub fn kick(&mut self, n: i32) {
        self.tick.nudge(n.signum() as f32 * 9.0);
    }

    /// A running timer: keep the seconds until the burst ends.
    pub fn hold(&mut self, secs: i64) {
        self.pending += secs;
        self.idle_ms = 0.0;
    }

    pub fn holding(&self) -> bool {
        self.pending != 0
    }

    /// One frame. (still moving, seconds to send now)
    pub fn step(&mut self, dt_ms: f64) -> (bool, Option<i64>) {
        self.tick.step(dt_ms);
        let mut send = None;
        if self.pending != 0 {
            self.idle_ms += dt_ms;
            if self.idle_ms >= FLUSH_MS {
                send = Some(std::mem::take(&mut self.pending));
            }
        }
        (self.tick.moving() || self.pending != 0, send)
    }

    /// Ring scale and the clock's vertical shift (css px) for painting.
    pub fn paint(&self) -> (f32, f32) {
        let t = self.tick.at();
        (1.0 + 0.012 * t.abs().min(4.0), -0.35 * t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    #[test]
    fn notches_collect_small_deltas_and_wheel_up_adds_time() {
        let mut w = RingWheel::new();
        assert_eq!(w.notches(-WHEEL_NOTCH_PX), 1, "up = more");
        assert_eq!(w.notches(WHEEL_NOTCH_PX * 2.0), -2);
        assert_eq!(w.notches(-40.0), 0);
        assert_eq!(w.notches(-40.0), 0);
        assert_eq!(w.notches(-40.0), 1, "three touchpad flicks make one notch");
        assert_eq!(RingWheel::seconds(2, false), 120);
        assert_eq!(RingWheel::seconds(-3, true), -3);
    }

    #[test]
    fn the_tick_swells_and_settles_and_a_burst_is_sent_once() {
        let _m = motion_on();
        let mut w = RingWheel::new();
        assert_eq!(w.paint(), (1.0, 0.0));
        assert!(!w.step(16.0).0, "idle: no frames");
        w.kick(1);
        w.hold(60);
        w.hold(60);
        let (busy, send) = w.step(16.0);
        assert!(busy && send.is_none());
        assert!(w.paint().0 > 1.0, "the ring swells");
        let mut sent = None;
        let mut frames = 0;
        while w.step(16.0).0 && frames < 400 {
            frames += 1;
        }
        // the burst went out as one write after 200 ms of quiet
        let mut w2 = RingWheel::new();
        w2.hold(60);
        w2.hold(-1);
        for _ in 0..20 {
            if let (_, Some(s)) = w2.step(16.0) {
                assert!(sent.is_none(), "sent twice");
                sent = Some(s);
            }
        }
        assert_eq!(sent, Some(59));
        assert!(!w2.holding());
        assert_eq!(w.paint(), (1.0, 0.0), "settled back");
        assert!(frames < 400, "it settles by itself");
    }
}
