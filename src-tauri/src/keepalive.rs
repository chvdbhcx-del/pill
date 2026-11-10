// "Another program hid the island": the guard poll (500 ms, no extra thread) looks at the island's window and, when it is hidden, minimised
// ("show desktop"), no longer topmost or has ended up off every monitor, puts it back. The decision is pure; `repair` is the Win32 part.
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GetWindowRect, IsIconic, IsWindowVisible, ShowWindow, GWL_EXSTYLE, SW_SHOWNOACTIVATE, WS_EX_TOPMOST};

/// What the OS says about the island window.
#[derive(Clone, Copy, Debug)]
pub struct Seen {
    pub visible: bool,
    pub iconic: bool,
    pub topmost: bool,
    /// l, t, r, b on screen
    pub rect: [i32; 4],
}

/// What has to be done.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Fix {
    /// `ShowWindow` (it was hidden or minimised)
    pub show: bool,
    /// re-place it: bounds + topmost again (it lost topmost, or it is off every monitor)
    pub place: bool,
}

impl Fix {
    pub fn any(self) -> bool {
        self.show || self.place
    }
}

/// At least this many physical px of the window must lie on a monitor in both directions.
const MIN_ON_SCREEN: i32 = 24;

fn on_a_monitor(r: [i32; 4], monitors: &[[i32; 4]]) -> bool {
    monitors.iter().any(|m| r[2].min(m[2]) - r[0].max(m[0]) >= MIN_ON_SCREEN && r[3].min(m[3]) - r[1].max(m[1]) >= MIN_ON_SCREEN)
}

pub fn decide(s: Seen, monitors: &[[i32; 4]]) -> Fix {
    let show = !s.visible || s.iconic;
    // no monitor list (host not up yet) = nothing to compare with: do not move a window on a guess
    let off = !monitors.is_empty() && !on_a_monitor(s.rect, monitors);
    Fix { show, place: show || !s.topmost || off }
}

pub fn look(hwnd: HWND) -> Option<Seen> {
    if hwnd.0.is_null() {
        return None;
    }
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(hwnd, &mut r).ok()?;
        Some(Seen {
            visible: IsWindowVisible(hwnd).as_bool(),
            iconic: IsIconic(hwnd).as_bool(),
            topmost: GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0,
            rect: [r.left, r.top, r.right, r.bottom],
        })
    }
}

/// Shows a hidden / minimised window again (no activation). Returns true when the caller must `place()` the island afterwards.
/// UI thread only.
pub fn repair(hwnd: HWND, monitors: &[[i32; 4]]) -> bool {
    let Some(seen) = look(hwnd) else { return false };
    let fix = decide(seen, monitors);
    if fix.show {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    }
    fix.any()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: [[i32; 4]; 2] = [[0, 0, 2560, 1440], [2560, 0, 4480, 1080]];
    fn seen(visible: bool, iconic: bool, topmost: bool, rect: [i32; 4]) -> Seen {
        Seen { visible, iconic, topmost, rect }
    }

    #[test]
    fn a_healthy_island_is_left_alone() {
        assert_eq!(decide(seen(true, false, true, [920, 48, 1640, 168]), &MON), Fix::default());
        assert!(!decide(seen(true, false, true, [920, 48, 1640, 168]), &MON).any());
    }

    #[test]
    fn hidden_or_minimised_is_shown_and_placed() {
        assert_eq!(decide(seen(false, false, true, [920, 48, 1640, 168]), &MON), Fix { show: true, place: true });
        assert_eq!(decide(seen(true, true, true, [920, 48, 1640, 168]), &MON), Fix { show: true, place: true });
    }

    #[test]
    fn losing_topmost_only_needs_a_replace() {
        assert_eq!(decide(seen(true, false, false, [920, 48, 1640, 168]), &MON), Fix { show: false, place: true });
    }

    #[test]
    fn off_screen_is_brought_back_but_a_second_monitor_is_fine() {
        assert!(decide(seen(true, false, true, [-32000, -32000, -31280, -31880]), &MON).place, "minimised parking spot");
        assert!(decide(seen(true, false, true, [5000, 10, 5720, 130]), &MON).place);
        assert!(!decide(seen(true, false, true, [3000, 10, 3720, 130]), &MON).any(), "on the second monitor");
        assert!(!decide(seen(true, false, true, [100, -90, 820, 30]), &MON).place, "30 px still on screen is enough");
        assert!(!decide(seen(true, false, true, [-5000, 0, -4280, 120]), &[]).place, "no monitor list: do not guess");
    }
}
