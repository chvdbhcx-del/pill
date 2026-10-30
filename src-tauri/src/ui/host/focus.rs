//! Focus policies as pure state machines (clock passed in): the command bar's blur-to-hide rules and the capture overlay's "lost the
//! foreground for too long" watchdog. Lifted from `command.rs` (SHOWN_AT / LAST_BLUR_HIDE) and `capture.rs::lost_too_long`.

/// A just-shown popup moves the keyboard focus between itself and its content while it takes the foreground, which reports a transient
/// "inactive". Blurs inside this window after a show are ours, not the user clicking away.
pub const SHOW_GRACE_MS: f64 = 400.0;
/// The click that blurred (and hid) the popup must not re-open it: the hotkey / tray click that follows within this time is ignored.
pub const REOPEN_BLOCK_MS: f64 = 300.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct BlurGuard {
    shown_at: Option<f64>,
    blur_hid_at: Option<f64>,
}

impl BlurGuard {
    pub fn note_shown(&mut self, now_ms: f64) {
        self.shown_at = Some(now_ms);
    }

    /// The window lost activation at `now_ms`: should it hide? Records the hide when it should (for `reopen_blocked`).
    pub fn blur_hides(&mut self, now_ms: f64) -> bool {
        if self.shown_at.is_some_and(|t| now_ms - t < SHOW_GRACE_MS) {
            return false;
        }
        self.blur_hid_at = Some(now_ms);
        true
    }

    /// A request to show the window arrives at `now_ms`: is it the echo of the click that just hid it?
    pub fn reopen_blocked(&self, now_ms: f64) -> bool {
        self.blur_hid_at.is_some_and(|t| now_ms - t < REOPEN_BLOCK_MS)
    }
}

/// Tracks how long a window that must stay in front (the capture overlay) has been out of the foreground.
#[derive(Clone, Copy, Debug)]
pub struct FocusWatch {
    lost_since: Option<f64>,
    limit_ms: f64,
}

impl FocusWatch {
    pub fn new(limit_ms: f64) -> FocusWatch {
        FocusWatch { lost_since: None, limit_ms }
    }

    /// Feed the current state; true once the window has been out of the foreground for `limit_ms` without a break.
    pub fn lost_too_long(&mut self, in_front: bool, now_ms: f64) -> bool {
        if in_front {
            self.lost_since = None;
            return false;
        }
        now_ms - *self.lost_since.get_or_insert(now_ms) >= self.limit_ms
    }

    pub fn reset(&mut self) {
        self.lost_since = None;
    }

    pub fn limit_ms(&self) -> f64 {
        self.limit_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blur_right_after_show_is_ours() {
        let mut g = BlurGuard::default();
        g.note_shown(1000.0);
        assert!(!g.blur_hides(1399.0), "inside the 400 ms hand-over");
        assert!(!g.reopen_blocked(1400.0), "nothing was hidden");
        assert!(g.blur_hides(1400.0), "a real click-away");
    }

    #[test]
    fn the_click_that_hid_the_popup_does_not_reopen_it() {
        let mut g = BlurGuard::default();
        g.note_shown(0.0);
        assert!(g.blur_hides(5000.0));
        assert!(g.reopen_blocked(5299.0));
        assert!(!g.reopen_blocked(5300.0));
        g.note_shown(6000.0);
        assert!(g.blur_hides(6500.0));
        assert!(g.reopen_blocked(6600.0));
    }

    #[test]
    fn a_never_shown_popup_blurs_normally() {
        let mut g = BlurGuard::default();
        assert!(g.blur_hides(10.0));
    }

    #[test]
    fn the_watchdog_fires_after_a_continuous_loss_only() {
        let mut w = FocusWatch::new(1500.0);
        assert!(!w.lost_too_long(true, 0.0));
        assert!(!w.lost_too_long(false, 100.0));
        assert!(!w.lost_too_long(false, 1599.0));
        assert!(w.lost_too_long(false, 1600.0));
        // a return to the front breaks the streak
        assert!(!w.lost_too_long(true, 1700.0));
        assert!(!w.lost_too_long(false, 1800.0));
        assert!(!w.lost_too_long(false, 3200.0));
        assert!(w.lost_too_long(false, 3300.0));
        w.reset();
        assert!(!w.lost_too_long(false, 3400.0));
    }
}
