// Reminders + timer. Owner: text/note agent. Tables: `reminders` (store.rs V1 + the migration at `// @migrations`).
// Parts: (1) a pure natural-language parser (RU + EN, unit-tested: `cargo test timers`),
//        (2) one scheduler thread: sleeps until the next due time in slices of at most 30 s (so a resume from sleep is noticed
//            within seconds), emits `remind:due` once per occurrence, advances repeating rules, re-arms from the table at start,
//        (3) the commands remind_add / remind_list / remind_done / remind_delete / remind_snooze and timer_start / timer_cancel / timer_state.
// Events to the page (window "pill"): remind:due {id,text,kind,late,repeat,due_at}, remind:changed, timer:tick {id,remaining_ms,total_s}, timer:done {id}.
// Timer = a reminder row of kind 'timer' (`total_s` = length, `done` set when it ends).
use crate::ui::bus::{self, Target};
use crate::store;
use serde::Serialize;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle};

const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;
const DAY: i64 = 24 * HOUR;
const LATE_MS: i64 = 90_000; // fired this much after its due time = "late" (missed while the app was off or the PC slept)

// ---------------- civil time helpers (wall clock = epoch ms as if the local clock were UTC) ----------------
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
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
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 => if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 29 } else { 28 },
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
fn wall(y: i64, m: i64, d: i64, h: i64, mi: i64) -> i64 { days_from_civil(y, m, d) * DAY + h * HOUR + mi * MIN }
/// (year, month, day, hour, minute, weekday Mon=0)
fn split(ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = ms.div_euclid(DAY);
    let rem = ms.rem_euclid(DAY);
    let (y, m, d) = civil_from_days(days);
    (y, m, d, rem / HOUR, rem % HOUR / MIN, (days + 3).rem_euclid(7))
}
fn add_month(w: i64) -> i64 {
    let (y, m, d, h, mi, _) = split(w);
    let (y2, m2) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    wall(y2, m2, d.min(days_in_month(y2, m2)), h, mi) // ponytail: day 31 clamps to 30/28 and then stays there
}

// ---------------- local time zone <-> UTC (Windows API; identity elsewhere) ----------------
#[cfg(windows)]
mod tz {
    use super::{split, wall};
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::System::Time::{SystemTimeToTzSpecificLocalTime, TzSpecificLocalTimeToSystemTime};

    fn to_st(ms: i64) -> SYSTEMTIME {
        let (y, m, d, h, mi, _) = split(ms);
        SYSTEMTIME { wYear: y as u16, wMonth: m as u16, wDayOfWeek: 0, wDay: d as u16, wHour: h as u16, wMinute: mi as u16, wSecond: 0, wMilliseconds: 0 }
    }
    fn from_st(s: &SYSTEMTIME, sub_minute_ms: i64) -> i64 {
        wall(s.wYear as i64, s.wMonth as i64, s.wDay as i64, s.wHour as i64, s.wMinute as i64) + sub_minute_ms
    }
    pub fn utc_to_wall(utc: i64) -> i64 {
        let sub = utc.rem_euclid(super::MIN);
        let st = to_st(utc);
        let mut out = SYSTEMTIME::default();
        match unsafe { SystemTimeToTzSpecificLocalTime(None, &st, &mut out) } { Ok(()) => from_st(&out, sub), Err(_) => utc }
    }
    pub fn wall_to_utc(w: i64) -> i64 {
        let sub = w.rem_euclid(super::MIN);
        let st = to_st(w);
        let mut out = SYSTEMTIME::default();
        match unsafe { TzSpecificLocalTimeToSystemTime(None, &st, &mut out) } { Ok(()) => from_st(&out, sub), Err(_) => w }
    }
}
#[cfg(not(windows))]
mod tz {
    pub fn utc_to_wall(utc: i64) -> i64 { utc }
    pub fn wall_to_utc(w: i64) -> i64 { w }
}
use tz::{utc_to_wall, wall_to_utc};

// ---------------- parser ----------------
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub text: String,
    pub due_wall: i64,
    /// "in 20 minutes": the due time is `now + delta`, not a wall-clock time (matters only around a DST change)
    pub relative: bool,
    /// none | daily | weekly | monthly | every:<minutes>
    pub repeat: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Day { Today, Tomorrow, After, Wd(i64), Date(i64, i64, Option<i64>) }
#[derive(Debug, Clone, Copy, PartialEq)]
enum Rep { Daily, Weekly, Monthly, Every(i64) }
#[derive(Default)]
struct Acc { rel: Option<i64>, day: Option<Day>, time: Option<(i64, i64)>, period: Option<i64>, rep: Option<Rep> }

fn norm(t: &str) -> String {
    t.trim_matches(|c: char| ",;!?()\"'«».".contains(c)).to_lowercase().replace('ё', "е")
}
fn is_prep(t: &str) -> bool { matches!(t, "at" | "on" | "by" | "в" | "во" | "на" | "к") }
fn is_filler(t: &str) -> bool { is_prep(t) || matches!(t, "next" | "the" | "следующий" | "следующую" | "следующее" | "следующем") }

fn split_num(t: &str) -> Option<(i64, &str)> {
    let end = t.bytes().take_while(|b| b.is_ascii_digit()).count();
    if end == 0 || end > 6 { return None; }
    Some((t[..end].parse().ok()?, &t[end..]))
}
fn weekday(t: &str) -> Option<i64> {
    const EN: [&[&str]; 7] = [&["monday", "mon"], &["tuesday", "tue", "tues"], &["wednesday", "wed"], &["thursday", "thu", "thur", "thurs"], &["friday", "fri"], &["saturday", "sat"], &["sunday", "sun"]];
    const RU: [&[&str]; 7] = [&["понедельник"], &["вторник"], &["сред"], &["четверг"], &["пятниц"], &["суббот"], &["воскресень"]];
    for i in 0..7 {
        if EN[i].contains(&t) { return Some(i as i64); }
        if i == 2 { if ["среда", "среду", "среды", "средам"].contains(&t) { return Some(2); } }
        else if t.starts_with(RU[i][0]) { return Some(i as i64); }
    }
    None
}
fn month_of(t: &str) -> Option<i64> {
    const EN: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    const EN_FULL: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    const RU: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
    (0..12).find(|&i| t == EN[i] || t == EN_FULL[i] || t == RU[i] || (i == 8 && t == "sept")).map(|i| i as i64 + 1)
}
/// unit word -> milliseconds (month = 30 days)
fn unit_ms(t: &str) -> Option<i64> {
    let en = |p: &str| t.starts_with(p);
    if t == "s" || en("sec") || t.starts_with("сек") { Some(1000) }
    else if t == "m" || en("min") || t.starts_with("мин") { Some(MIN) }
    else if t == "h" || t == "hr" || t == "hrs" || en("hour") || t.starts_with("час") || t == "ч" { Some(HOUR) }
    else if t == "d" || en("day") || t.starts_with("ден") || t.starts_with("дне") || t.starts_with("дня") || t == "дн" { Some(DAY) }
    else if t == "w" || en("week") || t.starts_with("недел") { Some(7 * DAY) }
    else if en("month") || t.starts_with("месяц") { Some(30 * DAY) }
    else { None }
}

/// "9", "9:30", "9am", "9:30pm" -> (hour, minute, suffix, had_colon)
fn hm_token(t: &str) -> Option<(i64, i64, &str, bool)> {
    let (a, rest) = split_num(t)?;
    if a > 99 { return None; }
    if let Some(r) = rest.strip_prefix(':') {
        let (b, r2) = split_num(r)?;
        if r.len() - r2.len() != 2 || b > 59 { return None; }
        Some((a, b, r2, true))
    } else { Some((a, 0, rest, false)) }
}
fn ampm(t: &str) -> Option<&'static str> {
    match t { "am" | "a.m" | "утра" => Some("am"), "pm" | "p.m" | "вечера" | "дня" => Some("pm"), "ночи" => Some("night"), _ => None }
}
fn apply_ampm(h: i64, which: &str) -> i64 {
    match which {
        "am" => if h == 12 { 0 } else { h },
        "pm" => if h < 12 { h + 12 } else { h },
        _ => if h == 12 { 0 } else if h >= 6 && h < 12 { h + 12 } else { h }, // "в 11 ночи" = 23:00, "в 3 ночи" = 3:00
    }
}
/// A time of day starting at token i (with or without a leading "at"/"в"): (tokens used, hour, minute)
fn time_at(n: &[String], i: usize) -> Option<(usize, i64, i64)> {
    let prep = matches!(n[i].as_str(), "at" | "в" | "во" | "к" | "by");
    let j = if prep { i + 1 } else { i };
    let tok = n.get(j)?.as_str();
    if prep && matches!(tok, "noon" | "полдень") { return Some((j + 1 - i, 12, 0)); }
    if prep && matches!(tok, "midnight" | "полночь") { return Some((j + 1 - i, 0, 0)); }
    if matches!(tok, "noon" | "midnight") { return Some((1, if tok == "noon" { 12 } else { 0 }, 0)); }
    let (mut h, m, suf, colon) = hm_token(tok)?;
    let mut which = ampm(suf.trim_end_matches('.'));
    let ok_suffix = suf.is_empty() || which.is_some() || suf == "h" || suf == "ч";
    if !ok_suffix || (!prep && !colon && which.is_none()) { return None; }
    let mut used = j + 1 - i;
    for _ in 0..2 { // optional "o'clock" / "часов", then am/pm word
        match n.get(i + used).map(String::as_str) {
            Some("часов" | "часа" | "час" | "o'clock" | "oclock") => used += 1,
            Some(w) if which.is_none() && ampm(w).is_some() => { which = ampm(w); used += 1; }
            _ => {}
        }
    }
    if let Some(w) = which { h = apply_ampm(h, w); }
    if h > 23 && !(h == 24 && m == 0) { return None; }
    Some((used, h % 24, m))
}

fn date_token(t: &str) -> Option<Day> {
    let parts: Vec<&str> = t.split(|c| c == '.' || c == '/').collect();
    if parts.len() < 2 || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) { return None; }
    let (d, m): (i64, i64) = (parts[0].parse().ok()?, parts[1].parse().ok()?);
    if !(1..=31).contains(&d) || !(1..=12).contains(&m) { return None; }
    let y = match parts.get(2) { None => None, Some(p) if p.len() == 4 => p.parse().ok(), Some(p) if p.len() == 2 => p.parse::<i64>().ok().map(|v| v + 2000), _ => return None };
    Some(Day::Date(d, m, y))
}

/// Try every pattern at token i. Returns (tokens used, whether a preceding "on"/"в"/"next" belongs to the match).
fn try_at(n: &[String], i: usize, a: &mut Acc) -> Option<(usize, bool)> {
    let t = n[i].as_str();
    let next = |k: usize| n.get(i + k).map(String::as_str);
    // repeat: every/each/кажд*, daily, ...
    if t == "every" || t == "each" || t.starts_with("кажд") {
        let nx = next(1)?;
        if let Some(w) = weekday(nx) { a.rep = Some(Rep::Weekly); a.day = Some(Day::Wd(w)); return Some((2, false)); }
        let (num, unit_tok, used) = match split_num(nx) {
            Some((k, r)) if !r.is_empty() => (k, r, 2),
            Some((k, _)) => (k, next(2)?, 3),
            None => (1, nx, 2),
        };
        let ms = unit_ms(unit_tok)?;
        a.rep = Some(match (ms, num) {
            (DAY, 1) => Rep::Daily,
            (x, 1) if x == 7 * DAY => Rep::Weekly,
            (x, 1) if x == 30 * DAY => Rep::Monthly,
            (x, k) => Rep::Every((x * k / MIN).max(1)),
        });
        return Some((used, false));
    }
    if let Some(r) = match t { "daily" | "ежедневно" => Some(Rep::Daily), "weekly" | "еженедельно" => Some(Rep::Weekly), "monthly" | "ежемесячно" => Some(Rep::Monthly),
        "hourly" | "ежечасно" => Some(Rep::Every(60)), _ => None } { a.rep = Some(r); return Some((1, false)); }
    if t == "по" { // "по понедельникам" = every Monday
        let w = next(1).and_then(|x| { let s = x.strip_suffix("ам").or_else(|| x.strip_suffix("ям"))?; weekday(s).or_else(|| weekday(&format!("{s}а"))) });
        if let Some(w) = w { a.rep = Some(Rep::Weekly); a.day = Some(Day::Wd(w)); return Some((2, false)); }
    }
    // relative: in 20 minutes / через 20 минут / in half an hour / через полчаса / in an hour 30 min
    if t == "in" || t == "через" {
        let mut j = i + 1;
        let mut total = 0i64;
        if matches!(n.get(j).map(String::as_str), Some("полчаса")) { a.rel = Some(30 * MIN); return Some((2, false)); }
        if t == "in" && n.get(j).map(String::as_str) == Some("half") { a.rel = Some(30 * MIN); return Some((if n.get(j + 1).map(String::as_str) == Some("an") { 4 } else { 3 }, false)); }
        loop {
            let (k, unit, step) = match n.get(j).map(|s| s.as_str()) {
                Some("a" | "an") => (1, n.get(j + 1)?.as_str(), 2),
                Some(x) => match split_num(x) {
                    Some((k, r)) if !r.is_empty() => (k, r, 1),
                    Some((k, _)) => (k, match n.get(j + 1) { Some(u) => u.as_str(), None => break }, 2),
                    None => (1, x, 1),
                },
                None => break,
            };
            match unit_ms(unit) { Some(ms) => { total += k * ms; j += step; } None => break }
            if n.get(j).map(String::as_str) == Some("and") || n.get(j).map(String::as_str) == Some("и") { j += 1; }
        }
        if total == 0 { return None; }
        a.rel = Some(total);
        return Some((j - i, false));
    }
    // day words
    let day = match t { "today" | "сегодня" => Some(Day::Today), "tomorrow" | "завтра" => Some(Day::Tomorrow), "послезавтра" => Some(Day::After), "tonight" => { a.period = Some(21); Some(Day::Today) } _ => None };
    if let Some(d) = day { a.day = Some(d); return Some((1, true)); }
    if t == "day" && next(1) == Some("after") && next(2) == Some("tomorrow") { a.day = Some(Day::After); return Some((3, true)); }
    let period = match t { "утром" | "morning" => Some(9), "днем" | "afternoon" => Some(14), "вечером" | "evening" => Some(19), _ => None };
    if let Some(h) = period { a.period = Some(h); return Some((1, true)); }
    if let Some(w) = weekday(t) { a.day = Some(Day::Wd(w)); return Some((1, true)); }
    // dates: 25.10, 25.10.2026, 25 october / 25 октября, october 25
    if let Some(d) = date_token(t) {
        let prev_prep = i > 0 && is_prep(&n[i - 1]);
        let next_time = next(1).map_or(false, |x| is_prep(x) || hm_token(x).map_or(false, |h| h.3));
        if matches!(d, Day::Date(_, _, Some(_))) || prev_prep || next_time { a.day = Some(d); return Some((1, true)); }
    }
    if let Some((dd, rest)) = split_num(t) {
        if rest.is_empty() && (1..=31).contains(&dd) { if let Some(m) = next(1).and_then(month_of) { a.day = Some(Day::Date(dd, m, None)); return Some((2, true)); } }
    }
    if let Some(m) = month_of(t) {
        if let Some((dd, "")) = next(1).and_then(split_num) { if (1..=31).contains(&dd) { a.day = Some(Day::Date(dd, m, None)); return Some((2, true)); } }
    }
    // time of day
    if let Some((used, h, m)) = time_at(n, i) {
        a.time = Some((h, m));
        return Some((used, !is_prep(t)));
    }
    None
}

const LEAD: &[&[&str]] = &[
    &["remind", "me", "to"], &["remind", "me", "that"], &["remind", "me", "about"], &["remind", "me"], &["remind"],
    &["напомни", "мне", "про"], &["напомни", "мне", "о"], &["напомни", "мне", "что"], &["напомни", "мне"], &["напомни", "про"], &["напомни", "о"], &["напомни", "что"], &["напомни"], &["напомнить"],
];

/// Parse a reminder phrase. `now_wall` = current local wall clock as epoch ms (see `utc_to_wall`).
/// Err: "no_time" (nothing time-like found) or "past" (the explicit moment is already gone).
pub fn parse(input: &str, now_wall: i64) -> Result<Parsed, &'static str> {
    let toks: Vec<&str> = input.split_whitespace().collect();
    let n: Vec<String> = toks.iter().map(|t| norm(t)).collect();
    let mut used = vec![false; n.len()];
    let mut a = Acc::default();
    let mut i = 0;
    while i < n.len() {
        match try_at(&n, i, &mut a) {
            None => i += 1,
            Some((len, eat)) => {
                let end = (i + len).min(n.len());
                used[i..end].iter_mut().for_each(|u| *u = true);
                if eat { let mut k = i; while k > 0 && !used[k - 1] && is_filler(&n[k - 1]) { used[k - 1] = true; k -= 1; } }
                i = end;
            }
        }
    }
    if a.rel.is_none() && a.day.is_none() && a.time.is_none() && a.rep.is_none() && a.period.is_none() { return Err("no_time"); }

    // ---- when ----
    let (ny, _, _, nh, nmi, ndow) = split(now_wall);
    let today = now_wall.div_euclid(DAY);
    let repeat = match a.rep { None => "none".to_string(), Some(Rep::Daily) => "daily".into(), Some(Rep::Weekly) => "weekly".into(), Some(Rep::Monthly) => "monthly".into(), Some(Rep::Every(m)) => format!("every:{m}") };
    let (due_wall, relative) = if let Some(rel) = a.rel {
        match a.time {
            Some((h, m)) if rel >= DAY => ((now_wall + rel).div_euclid(DAY) * DAY + h * HOUR + m * MIN, false),
            _ => (now_wall + rel, true),
        }
    } else if let (Some(Rep::Every(m)), None, None) = (a.rep, a.day, a.time) {
        (now_wall + m * MIN, true)
    } else if a.day.is_none() && a.time.is_none() && a.period.is_none() {
        // bare "daily" / "weekly" / "monthly": first run one period from now
        (match a.rep { Some(Rep::Weekly) => now_wall + 7 * DAY, Some(Rep::Monthly) => add_month(now_wall), _ => now_wall + DAY }, false)
    } else {
        let (h, m) = a.time.or(a.period.map(|h| (h, 0))).unwrap_or(if a.day.is_some() { (9, 0) } else { (nh, nmi) });
        let at = |day: i64| day * DAY + h * HOUR + m * MIN;
        match a.day {
            None => { let t = at(today); (if t <= now_wall { t + DAY } else { t }, false) }
            Some(Day::Today) => { let t = at(today); if t <= now_wall { return Err("past"); } (t, false) }
            Some(Day::Tomorrow) => (at(today + 1), false),
            Some(Day::After) => (at(today + 2), false),
            Some(Day::Wd(w)) => { let t = at(today + (w - ndow).rem_euclid(7)); (if t <= now_wall { t + 7 * DAY } else { t }, false) }
            Some(Day::Date(d, mo, y)) => {
                let year = y.unwrap_or(ny);
                if d > days_in_month(year, mo) { return Err("no_time"); }
                let t = wall(year, mo, d, h, m);
                if t > now_wall { (t, false) }
                else if y.is_none() && d <= days_in_month(year + 1, mo) { (wall(year + 1, mo, d, h, m), false) }
                else { return Err("past"); }
            }
        }
    };

    // ---- what: the unused words, minus a leading "remind me to" and dangling prepositions ----
    let mut rest: Vec<(&str, &str)> = toks.iter().zip(n.iter()).enumerate().filter(|(k, _)| !used[*k]).map(|(_, (o, nn))| (*o, nn.as_str())).collect();
    for pat in LEAD {
        if rest.len() >= pat.len() && rest.iter().zip(pat.iter()).all(|((_, nn), p)| nn == p) { rest.drain(..pat.len()); break; }
    }
    let dangling = |nn: &str| nn.is_empty() || is_prep(nn) || matches!(nn, "in" | "через" | "and" | "и" | "-" | "to" | "that");
    while rest.last().map_or(false, |(_, nn)| dangling(nn)) { rest.pop(); }
    while rest.first().map_or(false, |(_, nn)| dangling(nn)) { rest.remove(0); }
    let mut text = rest.iter().map(|(o, _)| *o).collect::<Vec<_>>().join(" ");
    text = text.trim_end_matches(|c: char| c == ',' || c == ';' || c == '-').trim().to_string();
    if text.is_empty() { text = if input.chars().any(|c| ('а'..='я').contains(&c.to_lowercase().next().unwrap_or(' '))) { "Напоминание".into() } else { "Reminder".into() }; }
    Ok(Parsed { text, due_wall, relative, repeat })
}

// ---------------- repeat rules ----------------
/// Next occurrence of a repeating rule strictly after `now` (UTC ms), keeping the local wall-clock time across DST.
fn advance(rule: &str, due: i64, now: i64) -> Option<i64> {
    if let Some(m) = rule.strip_prefix("every:").and_then(|x| x.parse::<i64>().ok()) {
        let step = m.max(1) * MIN;
        return Some(due + ((now - due).max(0) / step + 1) * step);
    }
    let mut d = due;
    for _ in 0..50_000 {
        let w = utc_to_wall(d);
        d = wall_to_utc(match rule { "daily" => w + DAY, "weekly" => w + 7 * DAY, "monthly" => add_month(w), _ => return None });
        if d > now { return Some(d); }
    }
    None
}

// ---------------- scheduler ----------------
static APP: OnceLock<AppHandle> = OnceLock::new();
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
fn wake() { if let Ok(mut g) = WAKE.0.lock() { *g = true; WAKE.1.notify_all(); } }
fn emit(name: &str, payload: serde_json::Value) { if let Some(app) = APP.get() { let _ = bus::emit(&app, Target::Label("pill"), name, payload); } }

pub fn init(app: &tauri::App) {
    let _ = APP.set(app.handle().clone());
    let _ = std::thread::Builder::new().name("reminders".into()).spawn(|| {
        std::thread::sleep(Duration::from_secs(4)); // let the page load its listeners, so a reminder missed while the app was off is not lost
        store::with(|c| c.execute("delete from reminders where (done = 1 or fired = 1) and due_at < ?1", [store::now_ms() - 30 * DAY]));
        loop {
            let now = store::now_ms();
            fire_due(now);
            let wait = next_wait(now);
            let g = WAKE.0.lock().unwrap_or_else(|e| e.into_inner());
            let mut g = if *g { g } else { WAKE.1.wait_timeout(g, wait).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0) };
            *g = false;
        }
    });
}

struct Due { id: i64, kind: String, text: String, due_at: i64, rule: String }

// ---------------- reminder signal: settings `sound.reminders` (on/off) and `sound.reminderTone` (chime | bell | beep) ----------------
// Three short sine signals synthesised in memory (no files), played through winmm PlaySound; same notes as the Settings preview.
const RATE: u32 = 22_050;

fn synth(tone: &str) -> Vec<u8> {
    let seq: &[(f32, f32)] = match tone {
        "bell" => &[(1046.0, 0.0), (1046.0, 0.32), (1046.0, 0.64)],
        "beep" => &[(740.0, 0.0)],
        _ => &[(880.0, 0.0), (1318.0, 0.18)],
    };
    let rate = RATE as f32;
    let n = ((seq[seq.len() - 1].1 + 0.55) * rate) as usize;
    let mut pcm = vec![0f32; n];
    for &(f, at) in seq {
        let s0 = (at * rate) as usize;
        for i in 0..(0.55 * rate) as usize {
            let t = i as f32 / rate;
            let env = if t < 0.02 { t / 0.02 } else { (-(t - 0.02) * 16.0).exp() }; // 20 ms attack, then exponential decay
            if let Some(v) = pcm.get_mut(s0 + i) {
                *v += 0.25 * env * (2.0 * std::f32::consts::PI * f * t).sin();
            }
        }
    }
    let data = (n * 2) as u32;
    let mut w = Vec::with_capacity(44 + n * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for v in pcm {
        w.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}

fn play_tone(tone: &str) {
    let wav = synth(tone);
    // SND_SYNC on its own thread: the buffer must outlive the playback; a newer signal replaces an older one
    std::thread::spawn(move || unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_NODEFAULT, SND_SYNC};
        let _ = PlaySoundW(PCWSTR(wav.as_ptr() as *const u16), HMODULE::default(), SND_MEMORY | SND_SYNC | SND_NODEFAULT);
    });
}

/// "Проверить" in Settings > Sound: plays the given signal (or the chosen one) whatever the on/off switch says.
#[tauri::command]
pub fn sound_play(tone: Option<String>) {
    play_tone(&tone.unwrap_or_else(|| crate::settings::get_str("sound.reminderTone", "chime")));
}

fn fire_due(now: i64) {
    let rows = store::with(|c| {
        let mut st = c.prepare("select id, kind, text, due_at, repeat_rule from reminders where done = 0 and fired = 0 and due_at <= ?1 order by due_at")?;
        let v = st.query_map([now], |r| Ok(Due { id: r.get(0)?, kind: r.get(1)?, text: r.get(2)?, due_at: r.get(3)?, rule: r.get(4)? }))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }).unwrap_or_default();
    for d in rows {
        let next = if d.rule == "none" { None } else { advance(&d.rule, d.due_at, now) };
        store::with(|c| match next {
            Some(t) => c.execute("update reminders set due_at = ?2 where id = ?1", (d.id, t)),
            None if d.kind == "timer" => c.execute("update reminders set done = 1 where id = ?1", [d.id]),
            None => c.execute("update reminders set fired = 1 where id = ?1", [d.id]),
        });
        if now - d.due_at <= LATE_MS && crate::settings::get_bool("sound.reminders", true) {
            play_tone(&crate::settings::get_str("sound.reminderTone", "chime"));
        }
        emit("remind:due", serde_json::json!({ "id": d.id, "text": d.text, "kind": d.kind, "late": now - d.due_at > LATE_MS, "repeat": d.rule, "due_at": d.due_at }));
        if d.kind == "timer" { emit("timer:done", serde_json::json!({ "id": d.id })); }
        emit("remind:changed", serde_json::Value::Null);
    }
}

/// Sleep until the next due row, in slices of at most 30 s; once a second while a timer runs (timer:tick).
fn next_wait(now: i64) -> Duration {
    let next = store::with(|c| c.query_row("select min(due_at) from reminders where done = 0 and fired = 0", [], |r| r.get::<_, Option<i64>>(0))).flatten();
    let mut wait = 30_000;
    if let Some(t) = next { wait = wait.min((t - now).max(0)); }
    if let Some(t) = timer_state_inner(now).filter(|t| t.active) {
        emit("timer:tick", serde_json::json!({ "id": t.id, "remaining_ms": t.remaining_ms, "total_s": t.total_s }));
        wait = wait.min(1000);
    }
    Duration::from_millis(wait.max(20) as u64)
}

// ---------------- commands ----------------
#[derive(Serialize)]
pub struct Reminder { id: i64, kind: String, text: String, due_at: i64, repeat_rule: String, fired: bool, total_s: i64, created_at: i64 }
const COLS: &str = "id, kind, text, due_at, repeat_rule, fired, total_s, created_at";
fn row(r: &rusqlite::Row) -> rusqlite::Result<Reminder> {
    Ok(Reminder { id: r.get(0)?, kind: r.get(1)?, text: r.get(2)?, due_at: r.get(3)?, repeat_rule: r.get(4)?, fired: r.get::<_, i64>(5)? != 0, total_s: r.get(6)?, created_at: r.get(7)? })
}
fn changed() { wake(); emit("remind:changed", serde_json::Value::Null); }

/// Parse natural language and store it. Err: "no_time" | "past" | "store".
#[tauri::command]
pub fn remind_add(text: String) -> Result<Reminder, String> {
    if text.chars().count() > 500 { return Err("no_time".into()); }
    let now = store::now_ms();
    let p = parse(&text, utc_to_wall(now)).map_err(str::to_string)?;
    let due = if p.relative { now + (p.due_wall - utc_to_wall(now)) } else { wall_to_utc(p.due_wall) };
    let r = store::with(|c| {
        c.execute("insert into reminders (kind, text, due_at, repeat_rule, done, created_at) values ('reminder', ?1, ?2, ?3, 0, ?4)", (&p.text, due, &p.repeat, now))?;
        c.query_row(&format!("select {COLS} from reminders where id = ?1"), [c.last_insert_rowid()], row)
    }).ok_or("store")?;
    changed();
    Ok(r)
}

/// Open reminders (not done), soonest first; `fired` ones are overdue and still waiting for Done / Snooze.
#[tauri::command]
pub fn remind_list() -> Vec<Reminder> {
    store::with(|c| {
        let mut st = c.prepare(&format!("select {COLS} from reminders where kind = 'reminder' and done = 0 order by due_at"))?;
        let v = st.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }).unwrap_or_default()
}

/// Finish a reminder (a repeating one stops here: its rule ends).
#[tauri::command]
pub fn remind_done(id: i64) { store::with(|c| c.execute("update reminders set done = 1 where id = ?1", [id])); changed(); }

#[tauri::command]
pub fn remind_delete(id: i64) { store::with(|c| c.execute("delete from reminders where id = ?1", [id])); changed(); }

/// Fire again in `minutes`. A repeating reminder keeps its rule; the snooze is a one-shot copy.
#[tauri::command]
pub fn remind_snooze(id: i64, minutes: i64) {
    let now = store::now_ms();
    let due = now + minutes.clamp(1, 7 * 24 * 60) * MIN;
    store::with(|c| {
        let (text, rule): (String, String) = c.query_row("select text, repeat_rule from reminders where id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        if rule == "none" {
            c.execute("update reminders set due_at = ?2, fired = 0, done = 0 where id = ?1", (id, due))
        } else {
            c.execute("insert into reminders (kind, text, due_at, repeat_rule, done, created_at) values ('reminder', ?1, ?2, 'none', 0, ?3)", (text, due, now))
        }
    });
    changed();
}

#[derive(Serialize)]
pub struct TimerState { active: bool, id: i64, due_at: i64, total_s: i64, remaining_ms: i64 }
fn timer_state_inner(now: i64) -> Option<TimerState> {
    let idle = TimerState { active: false, id: 0, due_at: 0, total_s: 0, remaining_ms: 0 };
    store::with(|c| {
        let mut st = c.prepare("select id, due_at, total_s from reminders where kind = 'timer' and done = 0 and fired = 0 order by due_at limit 1")?;
        let mut rows = st.query([])?;
        Ok(match rows.next()? {
            Some(r) => { let due: i64 = r.get(1)?; TimerState { active: true, id: r.get(0)?, due_at: due, total_s: r.get(2)?, remaining_ms: (due - now).max(0) } }
            None => idle,
        })
    })
}
#[tauri::command]
pub fn timer_state() -> TimerState { timer_state_inner(store::now_ms()).unwrap_or(TimerState { active: false, id: 0, due_at: 0, total_s: 0, remaining_ms: 0 }) }

/// Start (or restart) the one timer. Seconds are clamped to 1 s .. 99 h.
#[tauri::command]
pub fn timer_start(seconds: i64) -> TimerState {
    let s = seconds.clamp(1, 99 * 3600);
    let now = store::now_ms();
    store::with(|c| {
        c.execute("update reminders set done = 1 where kind = 'timer' and done = 0", [])?;
        c.execute("insert into reminders (kind, text, due_at, repeat_rule, done, created_at, total_s) values ('timer', '', ?1, 'none', 0, ?2, ?3)", (now + s * 1000, now, s))
    });
    changed();
    timer_state()
}

#[tauri::command]
pub fn timer_cancel() -> TimerState {
    store::with(|c| c.execute("update reminders set done = 1 where kind = 'timer' and done = 0", []));
    changed();
    timer_state()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed "now": Friday 2026-10-23 14:00 local wall clock.
    fn now() -> i64 { wall(2026, 10, 23, 14, 0) }
    enum At { Wall(i64, i64, i64, i64, i64), After(i64) }
    use At::*;

    #[test]
    fn signals() {
        for t in ["chime", "bell", "beep", "?"] {
            let w = synth(t);
            assert_eq!(&w[..4], b"RIFF");
            assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize, w.len() - 44);
            assert!(w[44..].chunks(2).any(|c| i16::from_le_bytes([c[0], c[1]]).abs() > 3000)); // audible
        }
    }

    #[test]
    fn civil_roundtrip() {
        assert_eq!(split(now()), (2026, 10, 23, 14, 0, 4)); // Friday = 4
        assert_eq!(split(wall(2024, 2, 29, 23, 59)).2, 29);
        assert_eq!(add_month(wall(2026, 1, 31, 8, 0)), wall(2026, 2, 28, 8, 0));
        assert_eq!(add_month(wall(2026, 12, 15, 8, 0)), wall(2027, 1, 15, 8, 0));
    }

    #[test]
    fn phrases() {
        // (input, when, text, repeat)
        let table: Vec<(&str, At, &str, &str)> = vec![
            ("in 20 minutes call mum", After(20 * MIN), "call mum", "none"),
            ("через 20 минут выключить плиту", After(20 * MIN), "выключить плиту", "none"),
            ("in 5 min", After(5 * MIN), "Reminder", "none"),
            ("in 1 hour 30 minutes tea", After(90 * MIN), "tea", "none"),
            ("in an hour stretch", After(HOUR), "stretch", "none"),
            ("in half an hour tea", After(30 * MIN), "tea", "none"),
            ("через полчаса чай", After(30 * MIN), "чай", "none"),
            ("через час позвонить", After(HOUR), "позвонить", "none"),
            ("через 3 дня позвонить", After(3 * DAY), "позвонить", "none"),
            ("in 2 days at 9 dentist", Wall(2026, 10, 25, 9, 0), "dentist", "none"),
            ("tomorrow at 9 call mum", Wall(2026, 10, 24, 9, 0), "call mum", "none"),
            ("позвонить маме завтра в 9", Wall(2026, 10, 24, 9, 0), "позвонить маме", "none"),
            ("remind me to call mum tomorrow at 9am", Wall(2026, 10, 24, 9, 0), "call mum", "none"),
            ("напомни мне про отчёт завтра в 9:30", Wall(2026, 10, 24, 9, 30), "отчёт", "none"),
            ("завтра в 9 вечера кино", Wall(2026, 10, 24, 21, 0), "кино", "none"),
            ("завтра в 12 ночи", Wall(2026, 10, 24, 0, 0), "Напоминание", "none"),
            ("tomorrow morning run", Wall(2026, 10, 24, 9, 0), "run", "none"),
            ("tomorrow", Wall(2026, 10, 24, 9, 0), "Reminder", "none"),
            ("day after tomorrow at 8 flight", Wall(2026, 10, 25, 8, 0), "flight", "none"),
            ("today at 18:30 meeting", Wall(2026, 10, 23, 18, 30), "meeting", "none"),
            ("сегодня в 18:30 встреча", Wall(2026, 10, 23, 18, 30), "встреча", "none"),
            ("сегодня вечером ужин", Wall(2026, 10, 23, 19, 0), "ужин", "none"),
            ("at 15:00 call", Wall(2026, 10, 23, 15, 0), "call", "none"),
            ("at 9 call", Wall(2026, 10, 24, 9, 0), "call", "none"), // 9:00 already passed today -> tomorrow
            ("в 15:00 созвон", Wall(2026, 10, 23, 15, 0), "созвон", "none"),
            ("at noon lunch", Wall(2026, 10, 24, 12, 0), "lunch", "none"),
            ("on Friday at 10 standup", Wall(2026, 10, 30, 10, 0), "standup", "none"),
            ("в пятницу в 10", Wall(2026, 10, 30, 10, 0), "Напоминание", "none"),
            ("в субботу в 12:00 рынок", Wall(2026, 10, 24, 12, 0), "рынок", "none"),
            ("on 25.10 at 9 dentist", Wall(2026, 10, 25, 9, 0), "dentist", "none"),
            ("25 октября в 9 стоматолог", Wall(2026, 10, 25, 9, 0), "стоматолог", "none"),
            ("on 1.3 at 10 taxes", Wall(2027, 3, 1, 10, 0), "taxes", "none"), // date already passed this year -> next year
            ("every day at 23:00 take pills", Wall(2026, 10, 23, 23, 0), "take pills", "daily"),
            ("таблетки каждый день в 23:00", Wall(2026, 10, 23, 23, 0), "таблетки", "daily"),
            ("every 2 hours drink water", After(2 * HOUR), "drink water", "every:120"),
            ("каждые 2 часа пить воду", After(2 * HOUR), "пить воду", "every:120"),
            ("every hour", After(HOUR), "Reminder", "every:60"),
            ("every 15 minutes", After(15 * MIN), "Reminder", "every:15"),
            ("every monday at 10 sync", Wall(2026, 10, 26, 10, 0), "sync", "weekly"),
            ("каждый вторник в 8:30 зарядка", Wall(2026, 10, 27, 8, 30), "зарядка", "weekly"),
            ("по средам в 19:00 футбол", Wall(2026, 10, 28, 19, 0), "футбол", "weekly"),
            ("monthly pay rent", Wall(2026, 11, 23, 14, 0), "pay rent", "monthly"),
            ("каждую неделю в 9 отчёт", Wall(2026, 10, 24, 9, 0), "отчёт", "weekly"),
        ];
        for (input, when, text, repeat) in table {
            let p = parse(input, now()).unwrap_or_else(|e| panic!("{input}: {e}"));
            let want = match when { Wall(y, m, d, h, mi) => wall(y, m, d, h, mi), After(ms) => now() + ms };
            assert_eq!(p.due_wall, want, "{input}: due {:?} want {:?}", split(p.due_wall), split(want));
            assert_eq!(p.text, text, "{input}: text");
            assert_eq!(p.repeat, repeat, "{input}: repeat");
            assert_eq!(p.relative, matches!(when, After(_)), "{input}: relative");
        }
    }

    #[test]
    fn errors() {
        assert_eq!(parse("buy milk", now()), Err("no_time"));
        assert_eq!(parse("", now()), Err("no_time"));
        assert_eq!(parse("today at 9 call", now()), Err("past"));
        assert_eq!(parse("30.02 at 9 x", now()), Err("no_time"));
    }

    #[test]
    fn repeat_advance() {
        let due = 1_000_000_000_000;
        assert_eq!(advance("every:10", due, due + 25 * MIN), Some(due + 30 * MIN));
        assert_eq!(advance("every:10", due, due), Some(due + 10 * MIN));
        assert!(advance("daily", due, due + 3 * DAY).unwrap() > due + 3 * DAY);
        assert_eq!(advance("none", due, due), None);
    }

    #[cfg(windows)]
    #[test]
    fn tz_roundtrip() {
        let w = wall(2026, 7, 1, 12, 0);
        assert_eq!(utc_to_wall(wall_to_utc(w)), w);
    }
}
