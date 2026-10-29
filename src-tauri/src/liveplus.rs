// Extra live-event producers: finished downloads, removable drives, battery, next calendar event. New file of the Pill app, used by main.rs +
// src/live-plus.js; user request: island events "Downloads / USB / Battery / Calendar", each with its own switch. Owner: live-events lane.
//  - live:file-done {name, path, more}        a file in the Downloads folder stopped growing (size stable 2 s). Files that existed at start, temp names
//                                              (.crdownload .part .tmp ...) and sub-folders are ignored. Switch live.dlDone.
//  - live:usb       {letter, added}           a removable drive letter appeared / vanished (GetLogicalDrives + GetDriveType, 2 s). Switch live.usb.
//  - live:battery   {kind: low|charging|full, percent}   GetSystemPowerStatus, only on devices with a battery. Switch live.battery.
//  - live:calendar  {title, minutes}          10 min before the next VEVENT of a local .ics file or of an https:// / webcal:// subscription (live.calendar.path).
//                                              Switch live.calendar (off by default).
// One supervisor thread (2 s tick); a switch that is off costs one boolean read. Everything is local and read-only; the only network use is the
// optional calendar subscription (net.rs, Feature::Calendar). Mail (IMAP) has its own thread in mail.rs.
// The decision logic (DlWatch, UsbWatch, Bat, ics parsing and recurrence, due()) is pure and unit-tested.
use crate::ui::bus::{self, Target};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
use windows::Win32::Foundation::SYSTEMTIME;

const TICK: Duration = Duration::from_secs(2);
const STABLE_MS: u64 = 2000; // a download counts as finished when its size has not changed for this long
const DRIVE_REMOVABLE: u32 = 2;
const USB_GAP_MS: u64 = 5000; // the same letter + direction is reported at most this often (a loose connector)
const AC_GAP_MS: u64 = 30_000; // plug / unplug flapping inside this window does not produce "Charging" again
const LEAD_SECS: i64 = 600; // calendar: announce 10 minutes before
const CAL_EVERY: u64 = 5; // ticks between calendar checks (10 s)
const ICS_MAX_BYTES: u64 = 8 << 20;
const MAX_EVENTS: usize = 5000;

pub(crate) fn mono_ms() -> u64 {
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64
}

// ================= downloads =================

const TEMP_EXT: &[&str] = &[".crdownload", ".part", ".partial", ".download", ".tmp"];

fn ignorable(name: &str) -> bool {
    let low = name.to_lowercase();
    name.starts_with('.') || low == "desktop.ini" || low == "thumbs.db" || TEMP_EXT.iter().any(|e| low.ends_with(e))
}

/// Finished-file detector. `step` gets the plain files of the Downloads folder (name, size) and returns the names that became stable.
#[derive(Default)]
struct DlWatch {
    primed: bool,
    seen: HashSet<String>,
    pending: HashMap<String, (u64, u64)>, // name -> (size, since_ms the size has been the same)
}

impl DlWatch {
    fn step(&mut self, now: u64, listing: &[(String, u64)]) -> Vec<String> {
        let here: HashSet<&str> = listing.iter().filter(|(n, _)| !ignorable(n)).map(|(n, _)| n.as_str()).collect();
        if !self.primed {
            self.primed = true;
            self.seen = here.iter().map(|s| s.to_string()).collect(); // everything that was there at start is old news
            return Vec::new();
        }
        self.seen.retain(|n| here.contains(n.as_str())); // a deleted file downloaded again later is news again
        self.pending.retain(|n, _| here.contains(n.as_str()));
        let mut ready = Vec::new();
        for (name, size) in listing.iter().filter(|(n, _)| !ignorable(n)) {
            if self.seen.contains(name) {
                continue;
            }
            let e = self.pending.entry(name.clone()).or_insert((*size, now));
            if e.0 != *size {
                *e = (*size, now);
            }
            if *size > 0 && now.saturating_sub(e.1) >= STABLE_MS {
                ready.push(name.clone());
            }
        }
        for n in &ready {
            self.pending.remove(n);
            self.seen.insert(n.clone());
        }
        ready
    }
}

fn list_files(dir: &std::path::Path) -> Option<Vec<(String, u64)>> {
    let rd = std::fs::read_dir(dir).ok()?;
    Some(
        rd.flatten()
            .filter_map(|e| {
                let m = e.metadata().ok().filter(|m| m.is_file())?;
                Some((e.file_name().to_string_lossy().into_owned(), m.len()))
            })
            .collect(),
    )
}

// ================= removable drives =================

/// Letter bits (A = bit 0) -> letters, floppy A/B skipped by the caller.
fn letters(mask: u32) -> Vec<char> {
    (0..26u8).filter(|i| mask & (1 << i) != 0).map(|i| (b'A' + i) as char).collect()
}

#[derive(Default)]
struct UsbWatch {
    primed: bool,
    mask: u32,
    last: HashMap<(char, bool), u64>,
}

impl UsbWatch {
    /// `mask` = removable drives now. Returns (letter, added) events, each (letter, direction) at most every USB_GAP_MS.
    fn step(&mut self, now: u64, mask: u32) -> Vec<(char, bool)> {
        let prev = self.mask;
        self.mask = mask;
        if !self.primed {
            self.primed = true;
            return Vec::new();
        }
        let mut out = Vec::new();
        for (set, added) in [(mask & !prev, true), (prev & !mask, false)] {
            for c in letters(set) {
                let fresh = self.last.get(&(c, added)).is_none_or(|t| now.saturating_sub(*t) >= USB_GAP_MS);
                if fresh {
                    self.last.insert((c, added), now);
                    out.push((c, added));
                }
            }
        }
        out
    }
}

fn removable_mask() -> u32 {
    let all = unsafe { GetLogicalDrives() };
    let mut m = 0u32;
    for i in 2..26u32 {
        // from C: on (A:/B: are floppies)
        if all & (1 << i) == 0 {
            continue;
        }
        let root: Vec<u16> = format!("{}:\\", (b'A' + i as u8) as char).encode_utf16().chain(Some(0)).collect();
        if unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == DRIVE_REMOVABLE {
            m |= 1 << i;
        }
    }
    m
}

// ================= battery =================

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    ac: bool,
    percent: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BatEv {
    Low(u8),
    Charging(u8),
    Full,
}

#[derive(Default)]
struct Bat {
    primed: bool,
    ac: bool,
    warned20: bool,
    warned10: bool,
    full_sent: bool,
    last_change: Option<u64>,
}

impl Bat {
    /// One event at most per call. Low: once per threshold (20 %, 10 %) per discharge, re-armed above 25 % / 15 % or when plugged in.
    /// Charging: on the plug-in edge, not more often than AC_GAP_MS. Full: once per charge at 100 %.
    fn step(&mut self, now: u64, s: Sample) -> Option<BatEv> {
        if !self.primed {
            self.primed = true;
            self.ac = s.ac;
            self.warned20 = s.percent <= 20; // started low: no warning for a level that was already passed
            self.warned10 = s.percent <= 10;
            self.full_sent = s.ac && s.percent >= 100;
            return None;
        }
        if s.ac || s.percent > 25 {
            self.warned20 = false;
        }
        if s.ac || s.percent > 15 {
            self.warned10 = false;
        }
        if !s.ac || s.percent <= 95 {
            self.full_sent = false;
        }
        if s.ac != self.ac {
            self.ac = s.ac;
            let quiet = self.last_change.is_some_and(|t| now.saturating_sub(t) < AC_GAP_MS);
            self.last_change = Some(now);
            if s.ac {
                if s.percent >= 100 {
                    self.full_sent = true;
                    if !quiet {
                        return Some(BatEv::Full);
                    }
                } else if !quiet {
                    return Some(BatEv::Charging(s.percent));
                }
            }
        }
        if !s.ac {
            if s.percent <= 10 && !self.warned10 {
                self.warned10 = true;
                self.warned20 = true;
                return Some(BatEv::Low(s.percent));
            }
            if s.percent <= 20 && !self.warned20 {
                self.warned20 = true;
                return Some(BatEv::Low(s.percent));
            }
        } else if s.percent >= 100 && !self.full_sent {
            self.full_sent = true;
            return Some(BatEv::Full);
        }
        None
    }
}

/// None on a desktop PC (no battery) or when Windows does not know the state.
fn battery_sample() -> Option<Sample> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s) }.ok()?;
    // BatteryFlag: 128 = no system battery, 255 = unknown; ACLineStatus: 255 = unknown
    if s.BatteryFlag & 128 != 0 || s.BatteryFlag == 255 || s.ACLineStatus == 255 || s.BatteryLifePercent > 100 {
        return None;
    }
    Some(Sample { ac: s.ACLineStatus == 1, percent: s.BatteryLifePercent })
}

// ================= calendar (.ics) =================

// Calendar arithmetic on "naive" seconds: seconds since 1970-01-01 00:00 of the wall clock, no zone. Event times and "now" are both local wall
// clock, so recurrence ("every day at 09:00") stays 09:00 across DST. ponytail: the lead time is off by an hour on the two DST switch nights.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    days_from_civil(if m == 12 { y + 1 } else { y }, if m == 12 { 1 } else { m + 1 }, 1) - days_from_civil(y, m, 1)
}

fn naive(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq)]
struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<i64>,
    until: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
struct Ev {
    summary: String,
    start: i64, // naive local seconds
    all_day: bool,
    rule: Option<Rule>,
}

/// DTSTART / UNTIL value -> (naive local seconds, all_day). `utc_to_local` converts a UTC wall clock (value ending in Z) to local.
/// A TZID parameter is ignored: the time is taken as local. ponytail: events in a foreign zone are off by the zone difference; add a tz table if needed.
fn parse_dt(value: &str, date_only: bool, utc_to_local: &dyn Fn(i64) -> i64) -> Option<(i64, bool)> {
    let v = value.trim();
    let num = |a: usize, b: usize| v.get(a..b).filter(|s| s.bytes().all(|c| c.is_ascii_digit())).and_then(|s| s.parse::<i64>().ok());
    let (y, mo, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) {
        return None;
    }
    if v.len() == 8 || date_only {
        return Some((naive(y, mo, d, 0, 0, 0), true));
    }
    if v.as_bytes().get(8) != Some(&b'T') || !(v.len() == 15 || (v.len() == 16 && v.ends_with('Z'))) {
        return None;
    }
    let (h, mi, s) = (num(9, 11)?, num(11, 13)?, num(13, 15)?);
    if h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let t = naive(y, mo, d, h, mi, s.min(59));
    Some((if v.ends_with('Z') { utc_to_local(t) } else { t }, false))
}

fn parse_rule(value: &str, utc_to_local: &dyn Fn(i64) -> i64) -> Option<Rule> {
    let (mut freq, mut interval, mut count, mut until) = (None, 1i64, None, None);
    for part in value.split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = match v.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => Some(Freq::Daily),
                    "WEEKLY" => Some(Freq::Weekly),
                    "MONTHLY" => Some(Freq::Monthly),
                    "YEARLY" => Some(Freq::Yearly),
                    _ => None, // HOURLY, MINUTELY...: the event is treated as a one-off
                }
            }
            "INTERVAL" => interval = v.trim().parse::<i64>().ok().filter(|n| (1..=1000).contains(n))?,
            "COUNT" => count = v.trim().parse::<i64>().ok().filter(|n| *n >= 1),
            "UNTIL" => until = parse_dt(v, false, utc_to_local).map(|(t, all_day)| if all_day { t + 86_399 } else { t }),
            _ => {} // BYDAY, BYMONTHDAY...: not supported, the DTSTART weekday / day of month is used
        }
    }
    Some(Rule { freq: freq?, interval, count, until })
}

/// VEVENTs of an iCalendar text. Cancelled events and events without a valid DTSTART are dropped; the result is capped at MAX_EVENTS.
fn parse_ics(text: &str, utc_to_local: &dyn Fn(i64) -> i64) -> Vec<Ev> {
    // unfold: a line that starts with a space or tab continues the previous one
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        match l.strip_prefix([' ', '\t']) {
            Some(rest) if !lines.is_empty() => lines.last_mut().unwrap().push_str(rest),
            _ => lines.push(l.to_string()),
        }
    }
    let mut out = Vec::new();
    let (mut cur, mut alarm): (Option<(Option<String>, Option<(i64, bool)>, Option<Rule>, bool)>, bool) = (None, false);
    for l in &lines {
        let Some((head, value)) = l.split_once(':') else { continue };
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or("").trim().to_ascii_uppercase();
        let date_only = parts.any(|p| p.trim().eq_ignore_ascii_case("VALUE=DATE"));
        match (name.as_str(), value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") => cur = Some((None, None, None, false)),
            ("BEGIN", "VALARM") => alarm = true,
            ("END", "VALARM") => alarm = false,
            ("END", "VEVENT") => {
                if let Some((sum, start, rule, cancelled)) = cur.take() {
                    if let (Some((start, all_day)), false) = (start, cancelled) {
                        out.push(Ev { summary: sum.unwrap_or_default(), start, all_day, rule });
                        if out.len() >= MAX_EVENTS {
                            break;
                        }
                    }
                }
            }
            _ if alarm => {}
            ("SUMMARY", _) => {
                if let Some(c) = cur.as_mut() {
                    c.0 = Some(unescape(value));
                }
            }
            ("DTSTART", _) => {
                if let Some(c) = cur.as_mut() {
                    c.1 = parse_dt(value, date_only, utc_to_local);
                }
            }
            ("RRULE", _) => {
                if let Some(c) = cur.as_mut() {
                    c.2 = parse_rule(value, utc_to_local);
                }
            }
            ("STATUS", "CANCELLED") => {
                if let Some(c) = cur.as_mut() {
                    c.3 = true;
                }
            }
            _ => {}
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.trim().chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n' | 'N') => o.push(' '),
                Some(x) => o.push(x),
                None => {}
            }
        } else {
            o.push(c);
        }
    }
    o
}

/// The n-th occurrence (0 = DTSTART) in naive seconds; None when that month has no such day (31st) or n > 0 without a rule.
fn occurrence(ev: &Ev, n: i64) -> Option<i64> {
    let Some(r) = &ev.rule else { return (n == 0).then_some(ev.start) };
    match r.freq {
        Freq::Daily => Some(ev.start + n * r.interval * 86_400),
        Freq::Weekly => Some(ev.start + n * r.interval * 7 * 86_400),
        Freq::Monthly | Freq::Yearly => {
            let months = n * r.interval * if r.freq == Freq::Yearly { 12 } else { 1 };
            let (day, tod) = (ev.start.div_euclid(86_400), ev.start.rem_euclid(86_400));
            let (y, m, d) = civil_from_days(day);
            let total = y * 12 + (m - 1) + months;
            let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
            (d <= days_in_month(ny, nm)).then(|| days_from_civil(ny, nm, d) * 86_400 + tod)
        }
    }
}

/// First occurrence starting at or after `t` (naive seconds), honouring COUNT and UNTIL.
fn next_start(ev: &Ev, t: i64) -> Option<i64> {
    let n0 = match &ev.rule {
        None => 0,
        _ if t <= ev.start => 0,
        Some(r) => match r.freq {
            Freq::Daily | Freq::Weekly => {
                let step = r.interval * 86_400 * if r.freq == Freq::Weekly { 7 } else { 1 };
                (t - ev.start + step - 1) / step
            }
            Freq::Monthly | Freq::Yearly => {
                let (ys, ms, _) = civil_from_days(ev.start.div_euclid(86_400));
                let (yt, mt, _) = civil_from_days(t.div_euclid(86_400));
                let step = r.interval * if r.freq == Freq::Yearly { 12 } else { 1 };
                (((yt * 12 + mt) - (ys * 12 + ms)) / step - 1).max(0)
            }
        },
    };
    for n in n0..n0 + 100 {
        if ev.rule.as_ref().and_then(|r| r.count).is_some_and(|c| n >= c) {
            return None;
        }
        if let Some(o) = occurrence(ev, n) {
            if ev.rule.as_ref().and_then(|r| r.until).is_some_and(|u| o > u) {
                return None;
            }
            if o >= t {
                return Some(o);
            }
        }
    }
    None
}

/// Events starting within LEAD_SECS after `now` that were not announced yet: (title, minutes). All-day events are never announced.
/// `fired` is keyed by (title, start) so a re-read file does not announce the same occurrence twice.
fn due(events: &[Ev], now: i64, fired: &mut HashSet<(String, i64)>) -> Vec<(String, i64)> {
    fired.retain(|(_, o)| *o > now - 3600);
    let mut out = Vec::new();
    for ev in events.iter().filter(|e| !e.all_day) {
        if let Some(o) = next_start(ev, now + 1) {
            if o - now <= LEAD_SECS && fired.insert((ev.summary.clone(), o)) {
                out.push((ev.summary.clone(), (o - now + 59) / 60));
            }
        }
    }
    out
}

// ---- Windows glue for the calendar
fn st_to_naive(t: &SYSTEMTIME) -> i64 {
    naive(t.wYear as i64, t.wMonth as i64, t.wDay as i64, t.wHour as i64, t.wMinute as i64, t.wSecond as i64)
}

fn local_now() -> i64 {
    st_to_naive(&unsafe { GetLocalTime() })
}

fn utc_to_local(utc: i64) -> i64 {
    let (day, tod) = (utc.div_euclid(86_400), utc.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(day);
    let u = SYSTEMTIME { wYear: y as u16, wMonth: m as u16, wDay: d as u16, wHour: (tod / 3600) as u16, wMinute: (tod % 3600 / 60) as u16, wSecond: (tod % 60) as u16, ..Default::default() };
    let mut l = SYSTEMTIME::default();
    if unsafe { SystemTimeToTzSpecificLocalTime(None, &u, &mut l) }.is_ok() {
        st_to_naive(&l)
    } else {
        utc
    }
}

// ---- calendar subscription: live.calendar.path may also be an https:// or webcal:// URL. It is fetched by net.rs (Feature::Calendar: gated by the
// live.calendar switch, only that host, 2 MB, 15 s) every 30 minutes in a short-lived thread (the 2 s tick never waits for the network); the last good
// body is cached next to the settings, calendar-<hash of the URL>.ics, and the normal file path below reads that cache, so it works offline.
// The URL can carry a secret token (Google "secret address"): it is never logged or sent to the page, only its host is.
const ICS_URL_EVERY_MS: u64 = 30 * 60_000;
const ICS_URL_RETRY_MS: u64 = 5 * 60_000; // after a failed fetch
const ICS_URL_MAX: usize = 2 << 20;
const ICS_URL_TIMEOUT_MS: u32 = 15_000;

#[derive(Debug, PartialEq)]
enum IcsSrc {
    None,
    File(PathBuf),
    Url(String),            // https:// URL to fetch (webcal:// already rewritten)
    Bad(&'static str),      // "scheme" (not http/https/webcal), "http" (plain http to a non-local host), "url" (unparsable)
}

/// `scheme://` with a scheme of 2+ letters (so `C:\x` and `\\server\share` stay file paths).
fn has_scheme(low: &str) -> bool {
    low.find("://").is_some_and(|i| i >= 2 && low.as_bytes()[0].is_ascii_alphabetic() && low[..i].bytes().all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b)))
}

fn ics_src(raw: &str) -> IcsSrc {
    let s = raw.trim().trim_matches('"').trim();
    if s.is_empty() {
        return IcsSrc::None;
    }
    let low = s.to_ascii_lowercase();
    let url = if low.starts_with("webcal://") {
        format!("https://{}", &s[9..])
    } else if low.starts_with("webcals://") {
        format!("https://{}", &s[10..])
    } else if low.starts_with("https://") || low.starts_with("http://") {
        s.to_string()
    } else if has_scheme(&low) {
        return IcsSrc::Bad("scheme");
    } else {
        return IcsSrc::File(PathBuf::from(s));
    };
    match crate::net::parse_url(&url) {
        Ok(u) if !u.https && !crate::net::is_loopback(&u.host) => IcsSrc::Bad("http"),
        Ok(_) => IcsSrc::Url(url),
        Err(_) => IcsSrc::Bad("url"),
    }
}

/// File name of the cache of a subscription: a hash of the URL, so neither the host nor a token appears in it.
fn cache_name(url: &str) -> String {
    let h = Sha256::digest(url.as_bytes());
    format!("calendar-{}.ics", h[..6].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn ics_setting() -> IcsSrc {
    ics_src(&crate::settings::get_str("live.calendar.path", ""))
}

fn ics_path() -> Option<PathBuf> {
    match ics_setting() {
        IcsSrc::File(p) => Some(p),
        IcsSrc::Url(u) => Some(crate::store::dir().join(cache_name(&u))),
        _ => None,
    }
}

/// Host of the subscription URL (for net.rs: the only host Feature::Calendar may call).
pub fn ics_url_host() -> Option<String> {
    match ics_setting() {
        IcsSrc::Url(u) => crate::net::parse_url(&u).ok().map(|u| u.host),
        _ => None,
    }
}

#[derive(Default)]
struct Remote {
    url: String,
    next_ms: u64,
    busy: bool,
    err: Option<String>, // Russian text for the settings page, never contains the URL
}

static REMOTE: Mutex<Remote> = Mutex::new(Remote { url: String::new(), next_ms: 0, busy: false, err: None });

fn remote() -> std::sync::MutexGuard<'static, Remote> {
    REMOTE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Readable category of a net.rs error (those texts can be in the system language and carry the host, so they are not shown as they are).
fn fetch_err_text(e: &str) -> &'static str {
    if e.contains("larger than") {
        "ответ больше 2 МБ"
    } else if e.contains("allow-list") || e.contains("redirect") {
        "сервер перенаправляет на другой адрес"
    } else {
        "нет соединения с сервером или он не ответил за 15 секунд"
    }
}

fn fetch_ics(url: &str) -> Result<(), &'static str> {
    let o = crate::net::Opts {
        feature: crate::net::Feature::Calendar, method: "GET", url, headers: &[("Accept", "text/calendar, text/plain, */*")],
        body: None, max_body: ICS_URL_MAX, recv_ms: ICS_URL_TIMEOUT_MS, decompress: true, abort: None,
    };
    let r = crate::net::request(&o, None).map_err(|e| fetch_err_text(&e))?;
    if r.status / 100 != 2 {
        return Err("сервер не отдал календарь (ответ не 200)");
    }
    let text = r.text();
    if !text.to_ascii_uppercase().contains("BEGIN:VCALENDAR") {
        return Err("по этому адресу не календарь (.ics)"); // an HTML login page must not replace the good cache
    }
    let p = crate::store::dir().join(cache_name(url));
    let tmp = p.with_extension("ics.tmp");
    if std::fs::write(&tmp, text.as_bytes()).is_err() || std::fs::rename(&tmp, &p).is_err() {
        return Err("не удалось сохранить копию календаря");
    }
    Ok(())
}

/// Starts a background fetch when the subscription is due (new URL: at once; then every 30 min, 5 min after a failure).
fn fetch_if_due() {
    let IcsSrc::Url(url) = ics_setting() else { return };
    let now = mono_ms();
    {
        let mut r = remote();
        if r.url != url {
            *r = Remote { url: url.clone(), ..Remote::default() };
        }
        if r.busy || now < r.next_ms {
            return;
        }
        r.busy = true;
    }
    std::thread::spawn(move || {
        let res = std::panic::catch_unwind(|| fetch_ics(&url)).unwrap_or(Err("внутренняя ошибка"));
        let mut r = remote();
        if r.url == url {
            r.busy = false;
            r.next_ms = mono_ms() + if res.is_ok() { ICS_URL_EVERY_MS } else { ICS_URL_RETRY_MS };
            r.err = res.err().map(str::to_string);
        }
        if let Err(w) = res {
            eprintln!("calendar: fetch from {} failed: {w}", crate::net::parse_url(&url).map(|u| u.host).unwrap_or_default());
        }
    });
}

/// Ok(events) or a short state word for the settings page: "missing" (cannot be read), "big" (over 8 MB).
fn load_ics(p: &std::path::Path) -> Result<Vec<Ev>, &'static str> {
    let m = std::fs::metadata(p).map_err(|_| "missing")?;
    if !m.is_file() {
        return Err("missing");
    }
    if m.len() > ICS_MAX_BYTES {
        return Err("big");
    }
    let bytes = std::fs::read(p).map_err(|_| "missing")?;
    Ok(parse_ics(&String::from_utf8_lossy(&bytes), &utc_to_local))
}

#[derive(Default)]
struct Cal {
    path: Option<PathBuf>,
    mtime: Option<SystemTime>,
    events: Vec<Ev>,
    fired: HashSet<(String, i64)>,
}

impl Cal {
    fn reset(&mut self) {
        *self = Cal::default();
    }

    fn poll(&mut self, app: &AppHandle) {
        fetch_if_due();
        let Some(p) = ics_path() else { return self.reset() };
        let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        if self.path.as_ref() != Some(&p) || self.mtime != mtime {
            self.events = load_ics(&p).unwrap_or_default();
            self.path = Some(p);
            self.mtime = mtime;
        }
        for (title, minutes) in due(&self.events, local_now(), &mut self.fired).into_iter().take(3) {
            let title: String = title.chars().take(80).collect();
            let _ = bus::emit(&app, Target::Label("pill"), "live:calendar", json!({ "title": title, "minutes": minutes }));
        }
    }
}

// ================= supervisor =================

fn on(key: &str, default: bool) -> bool {
    crate::settings::get_bool(key, default)
}

fn tick(app: &AppHandle, n: u64, dl: &mut DlWatch, usb: &mut UsbWatch, bat: &mut Bat, cal: &mut Cal) {
    let now = mono_ms();
    if on("live.dlDone", true) {
        let dir = crate::livesys::downloads_dir();
        if let Some(files) = list_files(&dir) {
            let ready = dl.step(now, &files);
            if let Some(first) = ready.first() {
                let path = dir.join(first);
                let _ = bus::emit(&app, Target::Label("pill"), "live:file-done", json!({ "name": first, "path": path.to_string_lossy(), "more": ready.len() - 1 }));
            }
        }
    } else {
        *dl = DlWatch::default();
    }
    if on("live.usb", true) {
        for (c, added) in usb.step(now, removable_mask()) {
            let _ = bus::emit(&app, Target::Label("pill"), "live:usb", json!({ "letter": c.to_string(), "added": added }));
        }
    } else {
        *usb = UsbWatch::default();
    }
    if on("live.battery", true) {
        if let Some(s) = battery_sample() {
            let ev = match bat.step(now, s) {
                Some(BatEv::Low(p)) => Some(("low", p)),
                Some(BatEv::Charging(p)) => Some(("charging", p)),
                Some(BatEv::Full) => Some(("full", 100)),
                None => None,
            };
            if let Some((kind, percent)) = ev {
                let _ = bus::emit(&app, Target::Label("pill"), "live:battery", json!({ "kind": kind, "percent": percent }));
            }
        }
    } else {
        *bat = Bat::default();
    }
    if on("live.calendar", false) {
        if n % CAL_EVERY == 0 {
            cal.poll(app);
        }
    } else {
        cal.reset();
    }
}

fn run(app: AppHandle) {
    let (mut dl, mut usb, mut bat, mut cal) = (DlWatch::default(), UsbWatch::default(), Bat::default(), Cal::default());
    let mut n = 0u64;
    loop {
        std::thread::sleep(TICK);
        n += 1;
        // a panic in one tick (it should not happen) must not end the producers
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(&app, n, &mut dl, &mut usb, &mut bat, &mut cal)));
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    std::thread::spawn(move || run(h));
}

// ================= commands =================

/// For the settings page: {state: nopath | missing | big | ok, events, next: {title, inMin} | null}. Reads the file, changes nothing.
#[tauri::command(async)]
pub fn liveplus_ics_status() -> Value {
    let src = ics_setting();
    let url = matches!(src, IcsSrc::Url(_));
    let Some(p) = ics_path() else {
        return match src {
            IcsSrc::Bad(why) => json!({ "state": "bad", "why": why, "events": 0, "next": Value::Null }),
            _ => json!({ "state": "nopath", "events": 0, "next": Value::Null }),
        };
    };
    let mut v = match load_ics(&p) {
        Err(s) => json!({ "state": s, "events": 0, "next": Value::Null }),
        Ok(evs) => {
            let now = local_now();
            let next = evs.iter().filter(|e| !e.all_day).filter_map(|e| next_start(e, now).map(|o| (o, e))).min_by_key(|(o, _)| *o);
            json!({
                "state": "ok", "events": evs.len(),
                "next": next.map(|(o, e)| json!({ "title": e.summary.chars().take(80).collect::<String>(), "inMin": (o - now + 59) / 60 })),
            })
        }
    };
    if url {
        // subscription: only the host, the fetch state and the last error text (never the URL)
        let r = remote();
        v["host"] = json!(ics_url_host());
        v["busy"] = json!(r.busy);
        v["fetchErr"] = json!(r.err);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dl_files(v: &[(&str, u64)]) -> Vec<(String, u64)> {
        v.iter().map(|(n, s)| (n.to_string(), *s)).collect()
    }

    #[test]
    fn download_old_files_ignored_new_one_needs_stable_size() {
        let mut w = DlWatch::default();
        assert!(w.step(0, &dl_files(&[("old.zip", 5)])).is_empty()); // existed at start
        assert!(w.step(2000, &dl_files(&[("old.zip", 5), ("a.pdf", 10)])).is_empty()); // just appeared
        assert!(w.step(3000, &dl_files(&[("old.zip", 5), ("a.pdf", 10)])).is_empty()); // stable for 1 s only
        assert!(w.step(3500, &dl_files(&[("old.zip", 5), ("a.pdf", 20)])).is_empty()); // grew: timer restarts
        assert_eq!(w.step(5500, &dl_files(&[("old.zip", 5), ("a.pdf", 20)])), vec!["a.pdf"]); // stable 2 s
        assert!(w.step(7500, &dl_files(&[("old.zip", 5), ("a.pdf", 20)])).is_empty()); // announced once
        assert!(w.step(9500, &dl_files(&[("old.zip", 5), ("a.pdf", 99)])).is_empty()); // later growth is not news
    }

    #[test]
    fn download_temp_names_zero_size_and_redownload() {
        let mut w = DlWatch::default();
        w.step(0, &[]);
        assert!(w.step(100, &dl_files(&[("x.pdf.crdownload", 5), ("y.part", 1), ("z.TMP", 1), ("w.partial", 1), (".hidden", 1)])).is_empty());
        assert!(w.step(9000, &dl_files(&[("x.pdf.crdownload", 5), ("empty.bin", 0)])).is_empty()); // temp and empty files never fire
        assert!(w.step(9100, &dl_files(&[("x.pdf", 7)])).is_empty()); // renamed: the final name starts its stable timer
        assert_eq!(w.step(11_100, &dl_files(&[("x.pdf", 7)])), vec!["x.pdf"]);
        w.step(12_000, &[]); // deleted
        assert!(w.step(13_000, &dl_files(&[("x.pdf", 7)])).is_empty());
        assert_eq!(w.step(15_000, &dl_files(&[("x.pdf", 7)])), vec!["x.pdf"]); // downloaded again: news again
    }

    #[test]
    fn usb_diff_and_rate_limit() {
        let mut u = UsbWatch::default();
        assert!(u.step(0, 1 << 4).is_empty()); // E: present at start
        assert_eq!(u.step(2000, (1 << 4) | (1 << 5)), vec![('F', true)]);
        assert_eq!(u.step(4000, 1 << 4), vec![('F', false)]);
        assert!(u.step(6000, (1 << 4) | (1 << 5)).is_empty()); // F: added again after 4 s: suppressed
        assert_eq!(u.step(9000, 1 << 4), vec![('F', false)]); // removal 5 s after the previous one
        assert_eq!(u.step(12_000, (1 << 4) | (1 << 5)), vec![('F', true)]);
        assert_eq!(letters(0b101), vec!['A', 'C']);
    }

    fn s(ac: bool, percent: u8) -> Sample {
        Sample { ac, percent }
    }

    #[test]
    fn battery_low_thresholds_once_and_rearm() {
        let mut b = Bat::default();
        assert_eq!(b.step(0, s(false, 50)), None); // baseline
        assert_eq!(b.step(1, s(false, 21)), None);
        assert_eq!(b.step(2, s(false, 20)), Some(BatEv::Low(20)));
        assert_eq!(b.step(3, s(false, 19)), None); // no spam
        assert_eq!(b.step(4, s(false, 10)), Some(BatEv::Low(10)));
        assert_eq!(b.step(5, s(false, 9)), None);
        assert_eq!(b.step(6, s(true, 9)), Some(BatEv::Charging(9)));
        assert_eq!(b.step(100_000, s(false, 9)), Some(BatEv::Low(9))); // unplugged again below 10 %: the charge re-armed the warning
        assert_eq!(b.step(100_001, s(false, 8)), None);
    }

    #[test]
    fn battery_start_low_is_silent_and_charge_cycle() {
        let mut b = Bat::default();
        assert_eq!(b.step(0, s(false, 15)), None); // started at 15 %: 20 % already passed
        assert_eq!(b.step(1, s(false, 14)), None);
        assert_eq!(b.step(2, s(false, 10)), Some(BatEv::Low(10)));
        assert_eq!(b.step(3, s(true, 10)), Some(BatEv::Charging(10)));
        assert_eq!(b.step(4, s(true, 60)), None);
        assert_eq!(b.step(5, s(true, 99)), None);
        assert_eq!(b.step(6, s(true, 100)), Some(BatEv::Full));
        assert_eq!(b.step(7, s(true, 100)), None);
        assert_eq!(b.step(8, s(false, 100)), None); // unplugged
        assert_eq!(b.step(9, s(true, 100)), None); // plugged back within 30 s: flapping stays quiet
    }

    #[test]
    fn battery_plug_flapping_is_rate_limited() {
        let mut b = Bat::default();
        b.step(0, s(false, 50));
        assert_eq!(b.step(1000, s(true, 50)), Some(BatEv::Charging(50)));
        assert_eq!(b.step(2000, s(false, 50)), None);
        assert_eq!(b.step(3000, s(true, 50)), None); // within 30 s of the previous change
        assert_eq!(b.step(4000, s(false, 50)), None);
        assert_eq!(b.step(40_000, s(true, 50)), Some(BatEv::Charging(50)));
        // plugged in already full
        let mut f = Bat::default();
        f.step(0, s(false, 100));
        assert_eq!(f.step(1000, s(true, 100)), Some(BatEv::Full));
    }

    fn utc_plus3(t: i64) -> i64 {
        t + 3 * 3600
    }

    #[test]
    fn civil_roundtrip() {
        for (y, m, d) in [(1970, 1, 1), (2000, 2, 29), (2026, 10, 25), (2100, 3, 1), (1969, 12, 31)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 12), 31);
    }

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nUID:1\r\nDTSTART:20261026T090000Z\r\nSUMMARY:Planning\\, weekly\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART;TZID=Europe/Moscow:20261026T100000\r\nSUMMARY:Long title that\r\n  continues here\r\nBEGIN:VALARM\r\nSUMMARY:alarm text\r\nEND:VALARM\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20261027\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART:20261028T120000\r\nSUMMARY:Cancelled\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART:garbage\r\nSUMMARY:Bad\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nDTSTART:20261026T080000\r\nSUMMARY:Standup\r\nRRULE:FREQ=DAILY;INTERVAL=2\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn ics_parse_forms() {
        let v = parse_ics(ICS, &utc_plus3);
        assert_eq!(v.len(), 4);
        assert_eq!(v[0].summary, "Planning, weekly");
        assert_eq!(v[0].start, naive(2026, 10, 26, 12, 0, 0)); // 09:00Z -> 12:00 local
        assert!(!v[0].all_day);
        assert_eq!(v[1].summary, "Long title that continues here"); // unfolded: the first space of the fold is dropped, the second kept
        assert_eq!(v[1].start, naive(2026, 10, 26, 10, 0, 0)); // floating / TZID: as is
        assert!(v[2].all_day);
        assert_eq!(v[2].start, naive(2026, 10, 27, 0, 0, 0));
        assert_eq!(v[3].summary, "Standup");
        assert_eq!(v[3].rule, Some(Rule { freq: Freq::Daily, interval: 2, count: None, until: None }));
    }

    #[test]
    fn ics_garbage_never_panics() {
        for t in ["", "BEGIN:VEVENT", "END:VEVENT\nEND:VEVENT", "DTSTART:2026\nBEGIN:VEVENT\nDTSTART:20261301T000000\nEND:VEVENT", "\u{0}\u{ffff}:::;;;", "BEGIN:VEVENT\nDTSTART:20260230T010101\nEND:VEVENT"] {
            assert!(parse_ics(t, &|x| x).is_empty());
        }
        assert_eq!(parse_rule("FREQ=HOURLY", &|x| x), None);
        assert_eq!(parse_rule("FREQ=DAILY;INTERVAL=0", &|x| x), None);
    }

    #[test]
    fn recurrence_daily_weekly_monthly() {
        let mk = |rule: Option<Rule>| Ev { summary: "e".into(), start: naive(2026, 1, 31, 9, 0, 0), all_day: false, rule };
        let r = |freq, interval, count, until| Some(Rule { freq, interval, count, until });
        let t = naive(2026, 2, 3, 0, 0, 0);
        assert_eq!(next_start(&mk(None), t), None); // one-off in the past
        assert_eq!(next_start(&mk(None), naive(2026, 1, 1, 0, 0, 0)), Some(naive(2026, 1, 31, 9, 0, 0)));
        assert_eq!(next_start(&mk(r(Freq::Daily, 1, None, None)), t), Some(naive(2026, 2, 3, 9, 0, 0)));
        assert_eq!(next_start(&mk(r(Freq::Daily, 1, None, None)), naive(2026, 2, 3, 9, 0, 0)), Some(naive(2026, 2, 3, 9, 0, 0))); // at or after
        assert_eq!(next_start(&mk(r(Freq::Daily, 1, None, None)), naive(2026, 2, 3, 9, 0, 1)), Some(naive(2026, 2, 4, 9, 0, 0)));
        assert_eq!(next_start(&mk(r(Freq::Weekly, 2, None, None)), t), Some(naive(2026, 2, 14, 9, 0, 0))); // 31 Jan + 14 d
        assert_eq!(next_start(&mk(r(Freq::Monthly, 1, None, None)), t), Some(naive(2026, 3, 31, 9, 0, 0))); // February has no 31st
        assert_eq!(next_start(&mk(r(Freq::Monthly, 1, None, None)), naive(2026, 4, 1, 0, 0, 0)), Some(naive(2026, 5, 31, 9, 0, 0))); // April too
        assert_eq!(next_start(&mk(r(Freq::Yearly, 1, None, None)), t), Some(naive(2027, 1, 31, 9, 0, 0)));
        assert_eq!(next_start(&mk(r(Freq::Daily, 1, Some(3), None)), t), None); // COUNT=3: 31 Jan, 1 Feb, 2 Feb
        assert_eq!(next_start(&mk(r(Freq::Daily, 1, Some(4), None)), t), Some(naive(2026, 2, 3, 9, 0, 0)));
        assert_eq!(next_start(&mk(r(Freq::Daily, 1, None, Some(naive(2026, 2, 2, 23, 59, 59)))), t), None); // UNTIL
    }

    #[test]
    fn due_once_within_ten_minutes() {
        let ev = |s: &str, start, all_day, rule| Ev { summary: s.into(), start, all_day, rule };
        let day = naive(2026, 10, 26, 9, 0, 0);
        let evs = vec![
            ev("soon", day, false, None),
            ev("later", day + 3600, false, None),
            ev("allday", naive(2026, 10, 26, 0, 0, 0) + 86_400, true, None),
            ev("daily", day - 86_400 * 5, false, Some(Rule { freq: Freq::Daily, interval: 1, count: None, until: None })),
        ];
        let mut fired = HashSet::new();
        assert!(due(&evs, day - 700, &mut fired).is_empty());
        let d = due(&evs, day - 600, &mut fired);
        assert_eq!(d, vec![("soon".to_string(), 10), ("daily".to_string(), 10)]);
        assert!(due(&evs, day - 590, &mut fired).is_empty()); // once per event
        assert!(due(&evs, day + 5, &mut fired).is_empty()); // started: nothing
        let d = due(&evs, day + 3600 - 61, &mut fired); // "later" in 61 s -> 2 min; the daily one is 23 h away
        assert_eq!(d, vec![("later".to_string(), 2)]);
        let d = due(&evs, day + 86_400 - 300, &mut fired); // next day's occurrence of "daily"
        assert_eq!(d, vec![("daily".to_string(), 5)]);
    }

    #[test]
    fn ics_source_file_url_and_rejected_schemes() {
        assert_eq!(ics_src("  "), IcsSrc::None);
        assert_eq!(ics_src(r#""C:\Users\Me\cal.ics""#), IcsSrc::File(PathBuf::from(r"C:\Users\Me\cal.ics"))); // a drive letter is not a scheme
        assert_eq!(ics_src(r"\\nas\share\cal.ics"), IcsSrc::File(PathBuf::from(r"\\nas\share\cal.ics")));
        assert_eq!(ics_src("https://cal.example.com/a/b.ics?token=x"), IcsSrc::Url("https://cal.example.com/a/b.ics?token=x".into()));
        assert_eq!(ics_src("WEBCAL://cal.example.com/a.ics"), IcsSrc::Url("https://cal.example.com/a.ics".into())); // webcal is https
        assert_eq!(ics_src("webcals://cal.example.com/a.ics"), IcsSrc::Url("https://cal.example.com/a.ics".into()));
        assert_eq!(ics_src("http://127.0.0.1:8080/a.ics"), IcsSrc::Url("http://127.0.0.1:8080/a.ics".into())); // loopback only
        assert_eq!(ics_src("http://cal.example.com/a.ics"), IcsSrc::Bad("http"));
        for bad in ["file:///C:/cal.ics", "ftp://x/a.ics", "javascript://x", "imap://x"] {
            assert_eq!(ics_src(bad), IcsSrc::Bad("scheme"), "{bad}");
        }
        assert_eq!(ics_src("https://user:pw@host/a.ics"), IcsSrc::Bad("url")); // no credentials in the URL
        assert_eq!(ics_src("https://"), IcsSrc::Bad("url"));
        assert_eq!(ics_src("https://a b/c"), IcsSrc::Bad("url"));
    }

    #[test]
    fn ics_cache_name_hides_the_url_and_is_stable() {
        let a = cache_name("https://cal.example.com/private/SECRETTOKEN/basic.ics");
        assert_eq!(a, cache_name("https://cal.example.com/private/SECRETTOKEN/basic.ics"));
        assert_ne!(a, cache_name("https://cal.example.com/private/OTHER/basic.ics"));
        assert!(a.starts_with("calendar-") && a.ends_with(".ics") && a.len() == "calendar-".len() + 12 + 4, "{a}");
        assert!(!a.contains("example") && !a.contains("SECRET"));
    }

    #[test]
    fn ics_fetch_errors_are_readable_and_carry_no_host() {
        assert_eq!(fetch_err_text("response from h is larger than the 2048 KB cap"), "ответ больше 2 МБ");
        assert!(fetch_err_text("request to h: Время ожидания истекло").contains("15"));
    }
}
