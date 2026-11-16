//! Windows Focus Assist / "do not disturb" as a second, opt-in reason to hold back event cards (`notify.respectFocus`, off by default).
//!
//! Two independent things can keep the island quiet:
//! * **Quiet hours** (`quiet.*`, the island's own schedule): non-critical cards are DROPPED, never shown later (`shell::quiet`).
//! * **Windows quiet time** (`SHQueryUserNotificationState`, only when `notify.respectFocus` is on): non-critical cards are HELD, counted on a
//!   small chip beside the time, and shown when Windows is quiet no more.
//!
//! [`verdict`] is the one place that decides, as a pure function of those inputs. Critical = what the user set or what cannot wait: timers,
//! reminders, urgent cards (`shell::quiet::exempt_toast`). Own quiet hours win over Focus Assist (a card that would be dropped is not worth
//! holding). Nothing here reads the system: the backend (`src/focus.rs`) asks Windows and sends `focus:changed {quiet}`.

/// What to do with a card that is about to appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Show it now.
    Show,
    /// Keep it silently and show it when Windows is quiet no more.
    Hold,
    /// Never show it (quiet hours).
    Drop,
}

/// The decision. `critical`: timer, reminder or urgent card. `own_quiet`: the island's quiet hours are on right now.
/// `respect_focus`: `notify.respectFocus`. `sys_quiet`: Windows says "do not disturb" ([`qun_is_quiet`]).
pub fn verdict(critical: bool, own_quiet: bool, respect_focus: bool, sys_quiet: bool) -> Verdict {
    if critical {
        Verdict::Show
    } else if own_quiet {
        Verdict::Drop
    } else if respect_focus && sys_quiet {
        Verdict::Hold
    } else {
        Verdict::Show
    }
}

/// `QUERY_USER_NOTIFICATION_STATE` of `SHQueryUserNotificationState`: does Windows ask for no interruptions?
/// 2 busy (a fullscreen app), 3 exclusive Direct3D fullscreen, 4 presentation mode, 6 quiet time (Focus Assist / Do Not Disturb),
/// 7 a fullscreen Store app. 1 (screen saver or locked: nobody to interrupt) and 5 (accepts notifications) are not quiet; unknown = not quiet.
pub fn qun_is_quiet(qun: i32) -> bool {
    matches!(qun, 2 | 3 | 4 | 6 | 7)
}

/// A short name of the state for the log and the event payload.
pub fn qun_name(qun: i32) -> &'static str {
    match qun {
        1 => "not-present",
        2 => "busy",
        3 => "d3d-fullscreen",
        4 => "presentation",
        5 => "open",
        6 => "quiet-time",
        7 => "app-fullscreen",
        _ => "unknown",
    }
}

/// The key under which the island's core hands "Windows is quiet and the user asked to respect it" to the providers (they only see settings).
/// Never stored: it is set in the core's own copy of the settings.
pub const HOLD_KEY: &str = "_focusHold";

/// How many cards are kept while holding; the oldest go first.
pub const HOLD_MAX: usize = 20;

/// The chip beside the time while cards are held: "Focus · 3" / "Фокус · 3". Empty = nothing held.
pub fn chip_text(held: usize, ru: bool) -> Option<String> {
    (held > 0).then(|| format!("{} · {held}", if ru { "Фокус" } else { "Focus" }))
}

/// Puts `item` into a hold list: an item with the same key replaces the old one in place (a card that updates), and a full list drops the oldest.
pub fn hold_push<T>(list: &mut Vec<(String, T)>, key: String, item: T) {
    if let Some(slot) = list.iter_mut().find(|(k, _)| *k == key) {
        slot.1 = item;
        return;
    }
    if list.len() >= HOLD_MAX {
        list.remove(0);
    }
    list.push((key, item));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_decision_table() {
        // (critical, own quiet, respect focus, windows quiet) -> verdict
        let t = [
            (false, false, false, false, Verdict::Show),
            (false, false, false, true, Verdict::Show),   // the switch is off: Windows is not asked
            (false, false, true, false, Verdict::Show),
            (false, false, true, true, Verdict::Hold),    // the one case that holds
            (false, true, false, false, Verdict::Drop),   // quiet hours drop as before
            (false, true, true, true, Verdict::Drop),     // own quiet wins over Focus Assist
            (false, true, true, false, Verdict::Drop),
            (true, false, true, true, Verdict::Show),     // timers, reminders, urgent: always
            (true, true, true, true, Verdict::Show),
            (true, true, false, false, Verdict::Show),
        ];
        for (crit, own, respect, sys, want) in t {
            assert_eq!(verdict(crit, own, respect, sys), want, "critical={crit} own={own} respect={respect} sys={sys}");
        }
    }

    #[test]
    fn windows_states_that_count_as_quiet() {
        for q in [2, 3, 4, 6, 7] {
            assert!(qun_is_quiet(q), "{q} {}", qun_name(q));
        }
        for q in [0, 1, 5, 8, -1] {
            assert!(!qun_is_quiet(q), "{q} {}", qun_name(q));
        }
        assert_eq!((qun_name(6), qun_name(5), qun_name(99)), ("quiet-time", "open", "unknown"));
    }

    #[test]
    fn the_chip_counts_what_is_held() {
        assert_eq!(chip_text(0, true), None);
        assert_eq!(chip_text(3, true).as_deref(), Some("Фокус · 3"));
        assert_eq!(chip_text(12, false).as_deref(), Some("Focus · 12"));
    }

    #[test]
    fn the_hold_list_replaces_by_key_and_forgets_the_oldest() {
        let mut l: Vec<(String, u32)> = Vec::new();
        hold_push(&mut l, "a".into(), 1);
        hold_push(&mut l, "b".into(), 2);
        hold_push(&mut l, "a".into(), 3);
        assert_eq!(l, vec![("a".to_string(), 3), ("b".to_string(), 2)], "same key: updated in place, order kept");
        for i in 0..HOLD_MAX as u32 + 5 {
            hold_push(&mut l, format!("k{i}"), i);
        }
        assert_eq!(l.len(), HOLD_MAX);
        assert_eq!(l.last().map(|x| x.1), Some(HOLD_MAX as u32 + 4), "the newest is kept");
        assert!(l.iter().all(|(k, _)| k != "a" && k != "b"), "the oldest went first");
    }
}
