//! Quiet hours: port of src/quiet.js `quietNow` plus the exempt-toast and muted-source rules from pill.js.
//!
//! Settings: `quiet.on` (strict true), `quiet.from` / `quiet.to` ("HH:MM", the window may cross midnight), `quiet.days` (the days a window
//! STARTS on, "0" = Sunday .. "6" = Saturday; missing = every day). `from == to` is an empty window. All cases of quiet.test.mjs are
//! ported below.

use crate::shell::io::Local;
use crate::shell::settings::QuietSettings;

/// Sources whose cards / pulses / chips are muted during quiet hours (pill.js `QUIET_SRC`).
pub const QUIET_SRC: [&str; 7] = ["agents", "agent-done", "copy", "download", "steam", "sysmon", "kbd"];

pub fn is_quiet_source(id: &str) -> bool {
    QUIET_SRC.contains(&id)
}

/// quiet.js `mins`: "H:MM" / "HH:MM" -> minutes since midnight, -1 when invalid.
pub fn mins(s: &str) -> i32 {
    let Some((h, m)) = s.split_once(':') else { return -1 };
    let ok = |p: &str, lo: usize, hi: usize| p.len() >= lo && p.len() <= hi && p.bytes().all(|b| b.is_ascii_digit());
    if !ok(h, 1, 2) || !ok(m, 2, 2) {
        return -1;
    }
    let (h, m): (i32, i32) = (h.parse().unwrap(), m.parse().unwrap());
    if h < 24 && m < 60 {
        h * 60 + m
    } else {
        -1
    }
}

/// quiet.js `quietNow(s, d)`.
pub fn quiet_now(q: &QuietSettings, d: &Local) -> bool {
    if !q.on {
        return false;
    }
    let (f, t) = (mins(&q.from), mins(&q.to));
    if f < 0 || t < 0 || f == t {
        return false;
    }
    let has = |x: u32| match &q.days {
        None => true,
        Some(days) => days.iter().any(|s| *s == x.to_string()),
    };
    let (day, m) = (d.weekday, d.minutes_of_day() as i32);
    if f < t {
        has(day) && m >= f && m < t
    } else {
        // after midnight the window still belongs to the day it started on
        (m >= f && has(day)) || (m < t && has((day + 6) % 7))
    }
}

/// pill.js `exemptToast`: timers / reminders the user set (id starts with "timer" or "remind") and anything marked urgent always show.
pub fn exempt_toast(id: Option<&str>, urgent: bool) -> bool {
    urgent || id.map(|i| i.starts_with("timer") || i.starts_with("remind")).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// 2026-10-25 is a Sunday: `at(day, h, m)` like the mjs helper (day 0 = Sunday 25 Oct).
    fn at(day: u32, h: u32, m: u32) -> Local {
        Local::ymd_hm(2026, 10, 25 + day, h, m)
    }

    /// Build settings the way the page would receive them (through the JSON keys).
    fn s(extra: Value) -> QuietSettings {
        let mut base = json!({"quiet.on": true, "quiet.from": "22:00", "quiet.to": "07:00"});
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        crate::shell::settings::ShellSettings::from_json(&base).quiet
    }

    #[test]
    fn disabled_missing_or_broken_settings_are_never_quiet() {
        assert!(!quiet_now(&QuietSettings::default(), &at(1, 23, 0)));
        assert!(!quiet_now(&s(json!({"quiet.on": false})), &at(1, 23, 0)));
        assert!(!quiet_now(&s(json!({"quiet.from": "25:00"})), &at(1, 23, 0)));
        assert!(!quiet_now(&s(json!({"quiet.to": "x"})), &at(1, 23, 0)));
        assert!(!quiet_now(&s(json!({"quiet.from": "08:00", "quiet.to": "08:00"})), &at(1, 8, 0))); // empty window
    }

    #[test]
    fn crossing_midnight_every_day() {
        let q = s(json!({}));
        assert!(!quiet_now(&q, &at(1, 21, 59)));
        assert!(quiet_now(&q, &at(1, 22, 0)));
        assert!(quiet_now(&q, &at(1, 23, 30)));
        assert!(quiet_now(&q, &at(2, 0, 0)));
        assert!(quiet_now(&q, &at(2, 6, 59)));
        assert!(!quiet_now(&q, &at(2, 7, 0)));
        assert!(!quiet_now(&q, &at(2, 12, 0)));
    }

    #[test]
    fn same_day_window() {
        let q = s(json!({"quiet.from": "13:00", "quiet.to": "14:30"}));
        assert!(quiet_now(&q, &at(3, 13, 0)));
        assert!(quiet_now(&q, &at(3, 14, 29)));
        assert!(!quiet_now(&q, &at(3, 14, 30)));
        assert!(!quiet_now(&q, &at(3, 12, 59)));
    }

    #[test]
    fn weekdays_window_belongs_to_the_day_it_starts_on() {
        let wk = s(json!({"quiet.days": ["1", "2", "3", "4", "5"]}));
        assert!(quiet_now(&wk, &at(5, 23, 0))); // Friday 23:00
        assert!(quiet_now(&wk, &at(6, 3, 0))); // Saturday 03:00 is still Friday's window
        assert!(!quiet_now(&wk, &at(6, 23, 0))); // Saturday 23:00 starts a Saturday window
        assert!(!quiet_now(&wk, &at(0, 3, 0))); // Sunday 03:00 belongs to Saturday
        assert!(!quiet_now(&wk, &at(1, 3, 0))); // Monday 03:00 belongs to Sunday
        assert!(quiet_now(&wk, &at(1, 22, 30)));
        assert!(quiet_now(&s(json!({"quiet.days": [1, 2]})), &at(1, 23, 0))); // numbers work too
        assert!(!quiet_now(&s(json!({"quiet.days": []})), &at(1, 23, 0)));
    }

    #[test]
    fn mins_parser() {
        assert_eq!(mins("22:00"), 1320);
        assert_eq!(mins("7:05"), 425);
        assert_eq!(mins("24:00"), -1);
        assert_eq!(mins("12:60"), -1);
        assert_eq!(mins("12:5"), -1);
        assert_eq!(mins("1230"), -1);
        assert_eq!(mins(""), -1);
        assert_eq!(mins("ab:cd"), -1);
        assert_eq!(mins("123:00"), -1);
    }

    #[test]
    fn exempt_rules() {
        assert!(exempt_toast(Some("timer-3"), false));
        assert!(exempt_toast(Some("remind:9"), false));
        assert!(exempt_toast(Some("x"), true));
        assert!(!exempt_toast(Some("copy"), false));
        assert!(!exempt_toast(None, false));
        assert!(is_quiet_source("copy") && !is_quiet_source("music"));
    }
}
