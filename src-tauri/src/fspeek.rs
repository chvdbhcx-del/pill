// The rules behind guard.rs, kept pure so they can be tested: who may hide the island, who may bring it over a fullscreen app
// ("peek"), when the cursor at the top edge counts as a request, and how the app lists grow ("add the active program").
use std::sync::atomic::{AtomicU8, Ordering};

/// Why the guard wants the island hidden (bits).
pub const WHY_FULLSCREEN: u8 = 1;
pub const WHY_LIST: u8 = 2;

/// How far a peek overrides the guard: nothing, a fullscreen app only, or everything (the tray's "Open island").
pub const PEEK_NONE: u8 = 0;
pub const PEEK_FULLSCREEN: u8 = 1;
pub const PEEK_ALL: u8 = 2;

static PEEK: AtomicU8 = AtomicU8::new(PEEK_NONE);
static WHY: AtomicU8 = AtomicU8::new(0);

pub fn peek() -> u8 {
    PEEK.load(Ordering::Relaxed)
}

pub fn start_peek(level: u8) {
    PEEK.store(level, Ordering::Relaxed);
}

/// Ends a peek; true when one was running.
pub fn end_peek() -> bool {
    PEEK.swap(PEEK_NONE, Ordering::Relaxed) != PEEK_NONE
}

/// The guard publishes its verdict (ignoring peeks) on every poll.
pub fn publish_why(why: u8) {
    WHY.store(why, Ordering::Relaxed);
}

pub fn why() -> u8 {
    WHY.load(Ordering::Relaxed)
}

/// What the foreground window looks like to the guard.
#[derive(Clone, Copy, Default, Debug)]
pub struct Facts {
    /// setting `hideInFullscreen`
    pub fs_setting: bool,
    /// the foreground window covers its monitor, or Windows reports exclusive fullscreen / presentation / busy
    pub fullscreen: bool,
    /// the foreground program is in `hideInApps`
    pub in_hide_list: bool,
    /// the foreground program is in `showInApps`: the island stays whatever else is true
    pub in_show_list: bool,
}

/// `WHY_*` bits for "the island should be hidden now" (0 = it may stay), before any peek.
pub fn why_hide(f: Facts) -> u8 {
    if f.in_show_list {
        return 0;
    }
    (if f.fs_setting && f.fullscreen { WHY_FULLSCREEN } else { 0 }) | (if f.in_hide_list { WHY_LIST } else { 0 })
}

/// Does the guard hide the island, given a running peek?
pub fn hides(why: u8, peek: u8) -> bool {
    (why & WHY_LIST != 0 && peek < PEEK_ALL) || (why & WHY_FULLSCREEN != 0 && peek == PEEK_NONE)
}

/// May `via` ("hotkey", "edge", "tray") bring the island over what hides it now? `setting` = `fullscreenPeek`.
/// The hotkey and the top edge only beat a fullscreen app (never the user's hide list); the tray entry beats both.
pub fn peek_level(via: &str, why: u8, setting: bool) -> Option<u8> {
    match via {
        "tray" => Some(PEEK_ALL),
        "hotkey" | "edge" if setting && why & WHY_LIST == 0 => Some(PEEK_FULLSCREEN),
        _ => None,
    }
}

/// The cursor rests on the top row of the island's monitor, inside the island's x range, no button held.
pub fn on_top_edge(cursor: (i32, i32), monitor: [i32; 4], x_range: (i32, i32), held: bool) -> bool {
    !held && cursor.1 <= monitor[1] + 1 && cursor.1 >= monitor[1] - 1 && cursor.0 >= monitor[0].max(x_range.0) && cursor.0 < monitor[2].min(x_range.1)
}

/// Top-edge dwell: true once the cursor has stayed on the edge for `DWELL_MS`.
#[derive(Default)]
pub struct EdgeDwell(Option<u64>);

pub const DWELL_MS: u64 = 150;

impl EdgeDwell {
    pub fn step(&mut self, now_ms: u64, on_edge: bool) -> bool {
        if !on_edge {
            self.0 = None;
            return false;
        }
        let since = *self.0.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) >= DWELL_MS {
            self.0 = None;
            return true;
        }
        false
    }
}

/// `exe` ("Chrome.exe") added to a list of process names: None when it is already there (with or without ".exe", any case).
pub fn with_app(list: &[String], exe: &str) -> Option<Vec<String>> {
    let norm = |s: &str| {
        let s = s.trim().to_lowercase();
        s.strip_suffix(".exe").map(str::to_string).unwrap_or(s)
    };
    let want = norm(exe);
    if want.is_empty() || list.iter().any(|e| norm(e) == want) {
        return None;
    }
    let mut next = list.to_vec();
    next.push(exe.trim().to_lowercase());
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(fs_setting: bool, fullscreen: bool, in_hide_list: bool, in_show_list: bool) -> Facts {
        Facts { fs_setting, fullscreen, in_hide_list, in_show_list }
    }

    #[test]
    fn the_show_list_beats_fullscreen_and_the_hide_list() {
        assert_eq!(why_hide(f(true, true, false, false)), WHY_FULLSCREEN);
        assert_eq!(why_hide(f(false, true, false, false)), 0, "hideInFullscreen off");
        assert_eq!(why_hide(f(true, true, true, false)), WHY_FULLSCREEN | WHY_LIST);
        assert_eq!(why_hide(f(true, true, true, true)), 0);
        assert_eq!(why_hide(f(false, false, true, false)), WHY_LIST);
    }

    #[test]
    fn a_peek_lifts_only_what_it_may() {
        assert!(hides(WHY_FULLSCREEN, PEEK_NONE));
        assert!(!hides(WHY_FULLSCREEN, PEEK_FULLSCREEN));
        assert!(hides(WHY_LIST, PEEK_FULLSCREEN), "the hide list survives a hotkey peek");
        assert!(!hides(WHY_LIST | WHY_FULLSCREEN, PEEK_ALL));
        assert!(!hides(0, PEEK_NONE));
    }

    #[test]
    fn who_may_peek() {
        assert_eq!(peek_level("hotkey", WHY_FULLSCREEN, true), Some(PEEK_FULLSCREEN));
        assert_eq!(peek_level("edge", WHY_FULLSCREEN, true), Some(PEEK_FULLSCREEN));
        assert_eq!(peek_level("hotkey", WHY_FULLSCREEN, false), None, "setting off");
        assert_eq!(peek_level("hotkey", WHY_LIST, true), None, "the hide list is the user's word");
        assert_eq!(peek_level("tray", WHY_LIST | WHY_FULLSCREEN, false), Some(PEEK_ALL), "the tray always works");
        assert_eq!(peek_level("page", WHY_FULLSCREEN, true), None);
    }

    #[test]
    fn the_edge_needs_the_top_row_the_island_x_range_and_no_button() {
        let mon = [0, 0, 2560, 1440];
        let x = (920, 1640);
        assert!(on_top_edge((1000, 0), mon, x, false));
        assert!(!on_top_edge((1000, 0), mon, x, true), "a drag is not a request");
        assert!(!on_top_edge((500, 0), mon, x, false), "left of the island");
        assert!(!on_top_edge((1000, 30), mon, x, false));
        assert!(!on_top_edge((1640, 0), mon, x, false), "x range is half open");
        let second = [2560, 0, 4480, 1080];
        assert!(!on_top_edge((1000, 0), second, x, false), "another monitor");
    }

    #[test]
    fn the_edge_must_dwell() {
        let mut d = EdgeDwell::default();
        assert!(!d.step(1000, true));
        assert!(!d.step(1100, true));
        assert!(d.step(1150, true), "150 ms in");
        assert!(!d.step(1160, true), "starts over after firing");
        assert!(!d.step(1200, false));
        assert!(!d.step(1300, true), "leaving the edge reset the clock");
        assert!(d.step(1450, true));
    }

    #[test]
    fn adding_the_active_program_never_duplicates() {
        let l = vec!["photoshop.exe".to_string(), "Blender".to_string()];
        assert_eq!(with_app(&l, "Chrome.exe"), Some(vec!["photoshop.exe".into(), "Blender".into(), "chrome.exe".into()]));
        assert_eq!(with_app(&l, "PHOTOSHOP.EXE"), None);
        assert_eq!(with_app(&l, "blender.exe"), None, "a stem entry matches too");
        assert_eq!(with_app(&l, "  "), None);
    }

    #[test]
    fn peek_state_round_trips() {
        start_peek(PEEK_FULLSCREEN);
        assert_eq!(peek(), PEEK_FULLSCREEN);
        assert!(end_peek());
        assert!(!end_peek());
        publish_why(WHY_LIST);
        assert_eq!(why(), WHY_LIST);
        publish_why(0);
    }
}
