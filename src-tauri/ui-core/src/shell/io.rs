//! World interface of the shell model: the backend command sink, the wall clock, a seeded RNG and civil-time helpers.
//!
//! The model never talks to the OS. Commands to the Rust backend (`pill_rect`, `pill_set_state`, `pill_cards`, `pill_auto_hide`,
//! `pill_activate`, `settings_set`, `settings_open`, `battery_state`, `weather_get`, `pill_desktop_fg`, `pill_toggle`) go out through
//! [`Backend::invoke`] and are fire-and-forget; the ones whose JS promise result matters come back as events
//! (`Event::BatteryState`, `Event::WeatherGet`, `Event::SettingsOpened`, `Event::DesktopFg`). Time of day comes from [`Clock`].
//! `Math.random()` of the JS (liquid ball, drip) is replaced by [`Rng`] so the simulation is deterministic and testable.

use serde_json::Value;

/// Sink for UI -> backend commands (names as in docs/native-ui-contract.md).
pub trait Backend {
    fn invoke(&mut self, cmd: &str, args: Value);
}

/// Wall clock (`Date.now()` + local UTC offset). Monotonic time (`performance.now()`) is the `now_ms` the host passes to `tick`.
pub trait Clock {
    /// Milliseconds since the Unix epoch (UTC).
    fn wall_ms(&self) -> i64;
    /// Local offset from UTC in minutes (east positive), i.e. `-Date.getTimezoneOffset()`.
    fn utc_offset_min(&self) -> i32;
}

/// Test backend: records every call.
#[derive(Default, Debug, Clone)]
pub struct RecBackend {
    pub calls: Vec<(String, Value)>,
}
impl Backend for RecBackend {
    fn invoke(&mut self, cmd: &str, args: Value) {
        self.calls.push((cmd.to_string(), args));
    }
}
impl RecBackend {
    pub fn count(&self, cmd: &str) -> usize {
        self.calls.iter().filter(|(c, _)| c == cmd).count()
    }
    pub fn last(&self, cmd: &str) -> Option<&Value> {
        self.calls.iter().rev().find(|(c, _)| c == cmd).map(|(_, v)| v)
    }
    pub fn all(&self, cmd: &str) -> Vec<&Value> {
        self.calls.iter().filter(|(c, _)| c == cmd).map(|(_, v)| v).collect()
    }
    pub fn clear(&mut self) {
        self.calls.clear();
    }
}

/// Test clock the caller advances by hand.
#[derive(Debug, Clone, Copy)]
pub struct ManualClock {
    pub wall_ms: i64,
    pub offset_min: i32,
}
impl Clock for ManualClock {
    fn wall_ms(&self) -> i64 {
        self.wall_ms
    }
    fn utc_offset_min(&self) -> i32 {
        self.offset_min
    }
}
impl Default for ManualClock {
    fn default() -> Self {
        // 2026-10-30 12:00:00 UTC
        ManualClock { wall_ms: 1_793_361_600_000, offset_min: 0 }
    }
}

/// xorshift64* generator, seeded: the liquid ball / drip use it where the JS used `Math.random()`.
#[derive(Debug, Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Broken-down local time (what the JS read from `new Date()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    pub month: u32,   // 1..=12
    pub day: u32,     // 1..=31
    pub weekday: u32, // 0 = Sunday .. 6 = Saturday (JS getDay)
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

/// Days since 1970-01-01 of a civil date (Hinnant's days_from_civil).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + if m <= 2 { 1 } else { 0 }) as i32, m, d)
}

impl Local {
    pub fn from_wall(wall_ms: i64, offset_min: i32) -> Local {
        let t = wall_ms + offset_min as i64 * 60_000;
        let days = t.div_euclid(86_400_000);
        let rem = t.rem_euclid(86_400_000);
        let (year, month, day) = civil_from_days(days);
        Local {
            year,
            month,
            day,
            weekday: (days + 4).rem_euclid(7) as u32, // 1970-01-01 was a Thursday
            hour: (rem / 3_600_000) as u32,
            minute: (rem / 60_000 % 60) as u32,
            second: (rem / 1000 % 60) as u32,
        }
    }
    /// Build from civil fields (seconds 0).
    pub fn ymd_hm(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Local {
        let days = days_from_civil(year, month, day);
        Local { year, month, day, weekday: (days + 4).rem_euclid(7) as u32, hour, minute, second: 0 }
    }
    pub fn minutes_of_day(&self) -> u32 {
        self.hour * 60 + self.minute
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip_and_weekday() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 25)), (2026, 10, 25));
        // 2026-10-25 is a Sunday, 2026-10-26 a Monday (quiet.test.mjs relies on it)
        assert_eq!(Local::ymd_hm(2026, 10, 25, 0, 0).weekday, 0);
        assert_eq!(Local::ymd_hm(2026, 10, 26, 0, 0).weekday, 1);
        assert_eq!(Local::ymd_hm(2024, 2, 29, 0, 0).weekday, 4);
    }

    #[test]
    fn local_from_wall_applies_offset() {
        let c = ManualClock::default();
        let l = Local::from_wall(c.wall_ms, 0);
        assert_eq!((l.year, l.month, l.day, l.hour, l.minute), (2026, 10, 30, 12, 0));
        let l = Local::from_wall(c.wall_ms, 180);
        assert_eq!((l.day, l.hour), (30, 15));
        let l = Local::from_wall(c.wall_ms, 720);
        assert_eq!((l.day, l.hour, l.weekday), (31, 0, 6));
        let l = Local::from_wall(-1, 0); // before the epoch
        assert_eq!((l.year, l.month, l.day, l.hour, l.minute, l.second), (1969, 12, 31, 23, 59, 59));
    }

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..1000 {
            let x = a.next_f64();
            assert_eq!(x, b.next_f64());
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
        assert_eq!(Rng::new(0).next_u64(), Rng::new(0).next_u64());
    }
}
