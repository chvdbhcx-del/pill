// Command bar: "shut down in 30 minutes", "restart at 23:00", "cancel shutdown" (RU + EN). Split out of command.rs.
// The text is parsed into a `Cmd`; the bar shows it as a system row that needs the usual second Enter (the confirm step), and only then
// `shutdown.exe /s|/r /t N` (or `/a`) runs through a `Runner`. Tests use a recording runner: nothing here ever shuts a test machine down.
// While armed, the island shows a countdown chip with a cancel button: event `power:timer {armed, kind, due_at}` (ui-core live/powertimer.rs).
use super::{res, Res};
use crate::ui::bus;
use serde_json::json;
use std::os::windows::process::CommandExt;
use std::sync::Mutex;

const NO_WINDOW: u32 = 0x0800_0000;
/// Longest wait accepted from a typed duration.
const MAX_SECS: u32 = 7 * 24 * 3600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Shutdown,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    Arm { kind: Kind, secs: u32 },
    Cancel,
}

// ---- parser ----

/// Lower-case words and numbers; a number glued to letters ("30min", "11pm", "1h30m") is split, "23:00" / "23.00" stays one token.
fn tokens(q: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_digit = false;
    for c in q.to_lowercase().chars().map(|c| if c == 'ё' { 'е' } else { c }) {
        let digit = c.is_ascii_digit() || ((c == ':' || c == '.') && prev_digit && !cur.is_empty());
        if c.is_whitespace() || c == ',' || c == ';' || c == '!' || c == '?' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_digit = false;
            continue;
        }
        if !cur.is_empty() && digit != prev_digit && !(c == ':' || c == '.') {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(c);
        prev_digit = digit;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|t| t.trim_end_matches(['.', ':']).to_string()).filter(|t| !t.is_empty()).collect()
}

fn has(toks: &[String], stems: &[&str]) -> bool {
    toks.iter().any(|t| stems.iter().any(|s| t.starts_with(s)))
}

fn unit_secs(t: &str) -> Option<f64> {
    Some(match t {
        "с" | "сек" | "секунд" | "секунду" | "секунды" | "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
        "м" | "мин" | "минут" | "минуту" | "минуты" | "минутки" | "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
        "ч" | "час" | "часа" | "часов" | "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
        _ => return None,
    })
}

fn number_word(t: &str) -> Option<f64> {
    Some(match t {
        "a" | "an" | "one" | "один" | "одну" | "одна" => 1.0,
        "two" | "два" | "две" => 2.0,
        "three" | "три" => 3.0,
        "five" | "пять" => 5.0,
        "ten" | "десять" => 10.0,
        "полтора" | "полторы" => 1.5,
        "half" | "пол" => 0.5,
        _ => return t.replace(',', ".").parse().ok().filter(|_| t.chars().all(|c| c.is_ascii_digit() || c == '.')),
    })
}

/// "in 30 minutes", "через час", "in 1h 30m", "через полтора часа", "in half an hour", "через полчаса".
fn relative(toks: &[String]) -> Option<u32> {
    let (mut total, mut num, mut any) = (0.0f64, None::<f64>, false);
    for t in toks {
        if matches!(t.as_str(), "and" | "и" | "the" | "an" | "a") && num.is_none() && !any {
            if matches!(t.as_str(), "an" | "a") {
                num = Some(1.0);
            }
            continue;
        }
        if matches!(t.as_str(), "and" | "и") {
            continue;
        }
        if t == "полчаса" {
            total += 1800.0;
            any = true;
            num = None;
        } else if let Some(u) = unit_secs(t) {
            let n = num.take().unwrap_or(1.0);
            // "half an hour" arrives as half, an, hour: the "an" must not overwrite the half
            total += n * u;
            any = true;
        } else if let Some(n) = number_word(t) {
            num = Some(match (num, n) {
                (Some(h), 1.0) if h == 0.5 => 0.5, // half + an
                _ => n,
            });
        } else if any {
            break; // trailing words after the duration ("please", the computer, ...)
        }
    }
    let secs = total.round();
    (any && secs >= 1.0 && secs <= MAX_SECS as f64).then_some(secs as u32)
}

/// "at 23:00", "в 23:30", "at 11pm", "в 9 вечера": seconds from `now` (seconds since local midnight) to the next such time, 1..=86400.
fn absolute(toks: &[String], now: u32) -> Option<u32> {
    let first = toks.first()?;
    let (h, m) = match first.split_once([':', '.']) {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (first.parse::<u32>().ok()?, 0),
    };
    let rest = &toks[1..];
    let pm = rest.iter().take(2).any(|t| matches!(t.as_str(), "pm" | "вечера" | "дня"));
    let am = rest.iter().take(2).any(|t| matches!(t.as_str(), "am" | "утра" | "ночи"));
    let h = match (h, pm, am) {
        (12, false, true) => 0,
        (1..=11, true, _) => h + 12,
        (13..=23, true, _) | (13..=23, _, true) | (0, true, _) => return None, // "13 pm"
        _ => h,
    };
    if h > 23 || m > 59 {
        return None;
    }
    let target = h * 3600 + m * 60;
    let d = if target > now { target - now } else { target + 86_400 - now };
    Some(d.clamp(1, 86_400))
}

/// The bar text as a power command. `now` = seconds since local midnight (clock times need it).
pub fn parse(q: &str, now: u32) -> Option<Cmd> {
    let toks = tokens(q);
    if toks.len() < 2 {
        return None;
    }
    let restart = has(&toks, &["перезагруз", "рестарт", "ребут", "restart", "reboot"]);
    let shut = has(&toks, &["выключ", "выруб", "заверши", "shutdown", "poweroff"])
        || toks.windows(2).any(|w| (w[0] == "shut" && w[1] == "down") || (matches!(w[0].as_str(), "power" | "turn" | "switch") && w[1] == "off"));
    if !restart && !shut {
        return None;
    }
    if has(&toks, &["отмен", "cancel", "abort", "stop", "прекрати"]) {
        return Some(Cmd::Cancel);
    }
    // the time phrase starts after the first marker word
    let (i, rel) = toks.iter().position(|t| matches!(t.as_str(), "через" | "in" | "after")).map(|i| (i, true)).or_else(|| toks.iter().position(|t| matches!(t.as_str(), "в" | "at" | "к" | "by")).map(|i| (i, false)))?;
    let secs = if rel { relative(&toks[i + 1..])? } else { absolute(&toks[i + 1..], now)? };
    Some(Cmd::Arm { kind: if restart { Kind::Restart } else { Kind::Shutdown }, secs })
}

/// `shutdown.exe` arguments of a command.
pub fn args(c: &Cmd) -> Vec<String> {
    match c {
        Cmd::Arm { kind, secs } => vec![if *kind == Kind::Restart { "/r" } else { "/s" }.into(), "/t".into(), secs.to_string()],
        Cmd::Cancel => vec!["/a".into()],
    }
}

// ---- running ----

pub trait Runner {
    fn run(&self, args: &[String]) -> Result<(), String>;
}

pub struct Real;
impl Runner for Real {
    fn run(&self, args: &[String]) -> Result<(), String> {
        let out = std::process::Command::new("shutdown.exe").args(args).creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("shutdown.exe exit code {}", out.status.code().unwrap_or(-1)))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Armed {
    pub kind: Kind,
    pub due_ms: i64,
}

static ARMED: Mutex<Option<Armed>> = Mutex::new(None);

/// Runs `c` and updates `slot`. Arming first aborts whatever was scheduled (Windows refuses a second schedule with error 1190; the abort of
/// nothing scheduled fails with 1116, which is fine), so the new time always wins.
pub fn apply(r: &dyn Runner, c: &Cmd, now_ms: i64, slot: &mut Option<Armed>) -> Result<(), String> {
    match c {
        Cmd::Arm { kind, secs } => {
            let _ = r.run(&args(&Cmd::Cancel));
            r.run(&args(c))?;
            *slot = Some(Armed { kind: *kind, due_ms: now_ms + *secs as i64 * 1000 });
            Ok(())
        }
        Cmd::Cancel => {
            let _ = r.run(&args(c)); // cancelling "nothing" (error 1116) is not an error for the user
            *slot = None;
            Ok(())
        }
    }
}

fn payload(a: &Option<Armed>) -> serde_json::Value {
    match a {
        Some(a) => json!({ "armed": true, "kind": if a.kind == Kind::Restart { "restart" } else { "shutdown" }, "due_at": a.due_ms }),
        None => json!({ "armed": false }),
    }
}

/// Command `power_timer_state` (also the island's boot query): what is armed right now.
pub fn power_timer_state() -> serde_json::Value {
    let mut g = super::lk(&ARMED);
    if g.is_some_and(|a| a.due_ms <= crate::store::now_ms() + 1000) {
        *g = None; // the PC should be gone by now; a stale chip must not stay if the shutdown was cancelled elsewhere
    }
    payload(&g)
}

/// Command `power_timer_cancel`: the chip's cancel button.
pub fn power_timer_cancel() -> serde_json::Value {
    run_cmd(&Real, &Cmd::Cancel)
}

fn run_cmd(r: &dyn Runner, c: &Cmd) -> serde_json::Value {
    let mut g = super::lk(&ARMED);
    let out = apply(r, c, crate::store::now_ms(), &mut g);
    let p = payload(&g);
    drop(g);
    let _ = bus::emit("power:timer", p.clone());
    match out {
        Ok(()) => p,
        Err(e) => json!({ "armed": p["armed"], "error": e }),
    }
}

// ---- the bar rows ----

const PREFIX: &str = "pt:";

fn local_secs() -> u32 {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe { GetLocalTime() };
    t.wHour as u32 * 3600 + t.wMinute as u32 * 60 + t.wSecond as u32
}

fn ru() -> bool {
    pill_ui_core::i18n::lang() == pill_ui_core::i18n::Lang::Ru
}

fn human(secs: u32, ru: bool) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    let (hu, mu) = if ru { ("ч", "мин") } else { ("h", "min") };
    match (h, m) {
        (0, 0) => format!("{secs} {}", if ru { "с" } else { "s" }),
        (0, m) => format!("{m} {mu}"),
        (h, 0) => format!("{h} {hu}"),
        (h, m) => format!("{h} {hu} {m} {mu}"),
    }
}

/// The payload the row carries: `pt:s:1800`, `pt:r:1800`, `pt:a`.
fn encode(c: &Cmd) -> String {
    match c {
        Cmd::Arm { kind: Kind::Shutdown, secs } => format!("{PREFIX}s:{secs}"),
        Cmd::Arm { kind: Kind::Restart, secs } => format!("{PREFIX}r:{secs}"),
        Cmd::Cancel => format!("{PREFIX}a"),
    }
}

fn decode(p: &str) -> Option<Cmd> {
    let p = p.strip_prefix(PREFIX)?;
    if p == "a" {
        return Some(Cmd::Cancel);
    }
    let (k, s) = p.split_once(':')?;
    let secs: u32 = s.parse().ok().filter(|s| (1..=MAX_SECS).contains(s))?;
    Some(Cmd::Arm { kind: if k == "r" { Kind::Restart } else if k == "s" { Kind::Shutdown } else { return None }, secs })
}

/// The row for what was typed, if it is a timed power command. Needs the second Enter like every system row.
pub(super) fn rows(q: &str) -> Vec<Res> {
    let Some(c) = parse(q, local_secs()) else { return vec![] };
    let ru = ru();
    let title = match c {
        Cmd::Arm { kind, secs } => {
            let at = (local_secs() + secs) % 86_400;
            let (verb_ru, verb_en) = if kind == Kind::Restart { ("Перезагрузка", "Restart") } else { ("Выключение", "Shut down") };
            let clock = format!("{:02}:{:02}", at / 3600, at % 3600 / 60);
            if ru { format!("{verb_ru} через {} (в {clock})", human(secs, true)) } else { format!("{verb_en} in {} (at {clock})", human(secs, false)) }
        }
        Cmd::Cancel => if ru { "Отменить запланированное выключение".to_string() } else { "Cancel the scheduled shutdown".to_string() },
    };
    vec![res("system", format!("sys:{}", encode(&c)), title, "Press Enter twice to confirm".into(), encode(&c))]
}

/// `cmd_run` for a system row whose payload is ours: `None` = not ours (the caller runs the plain power action).
pub(super) fn run_payload(p: &str) -> Option<Result<(), String>> {
    let c = decode(p)?;
    let out = run_cmd(&Real, &c);
    Some(match out.get("error").and_then(|e| e.as_str()) {
        Some(e) => Err(e.to_string()),
        None => Ok(()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const NOON: u32 = 12 * 3600;

    fn arm(kind: Kind, secs: u32) -> Option<Cmd> {
        Some(Cmd::Arm { kind, secs })
    }

    #[test]
    fn russian_and_english_durations() {
        let p = |q| parse(q, NOON);
        assert_eq!(p("выключи компьютер через 30 минут"), arm(Kind::Shutdown, 1800));
        assert_eq!(p("shut down in 30 minutes"), arm(Kind::Shutdown, 1800));
        assert_eq!(p("shutdown in 30min"), arm(Kind::Shutdown, 1800));
        assert_eq!(p("выключи пк через час"), arm(Kind::Shutdown, 3600));
        assert_eq!(p("выключи через 2 часа"), arm(Kind::Shutdown, 7200));
        assert_eq!(p("shut down the computer in 1h 30m"), arm(Kind::Shutdown, 5400));
        assert_eq!(p("power off in 1h30m"), arm(Kind::Shutdown, 5400));
        assert_eq!(p("выключи компьютер через 1 час 15 минут"), arm(Kind::Shutdown, 4500));
        assert_eq!(p("выключи через полтора часа"), arm(Kind::Shutdown, 5400));
        assert_eq!(p("выключи через полчаса"), arm(Kind::Shutdown, 1800));
        assert_eq!(p("shut down in half an hour"), arm(Kind::Shutdown, 1800));
        assert_eq!(p("shut down in an hour"), arm(Kind::Shutdown, 3600));
        assert_eq!(p("перезагрузи через 10 минут"), arm(Kind::Restart, 600));
        assert_eq!(p("restart in 5 minutes"), arm(Kind::Restart, 300));
        assert_eq!(p("reboot in 2 hours"), arm(Kind::Restart, 7200));
    }

    #[test]
    fn clock_times_roll_over_midnight() {
        assert_eq!(parse("перезагрузи в 23:00", NOON), arm(Kind::Restart, 11 * 3600));
        assert_eq!(parse("restart at 23:00", NOON), arm(Kind::Restart, 11 * 3600));
        assert_eq!(parse("shut down at 11pm", NOON), arm(Kind::Shutdown, 11 * 3600));
        assert_eq!(parse("shut down at 11:30 pm", NOON), arm(Kind::Shutdown, 11 * 3600 + 1800));
        assert_eq!(parse("выключи в 9 вечера", NOON), arm(Kind::Shutdown, 9 * 3600));
        assert_eq!(parse("выключи компьютер в 23.30", NOON), arm(Kind::Shutdown, 11 * 3600 + 1800));
        // 08:00 has passed at noon: it is tomorrow's
        assert_eq!(parse("shut down at 8:00", NOON), arm(Kind::Shutdown, 20 * 3600));
        assert_eq!(parse("shut down at 12:00", NOON), arm(Kind::Shutdown, 86_400));
        assert_eq!(parse("shut down at 12am", NOON), arm(Kind::Shutdown, 12 * 3600));
    }

    #[test]
    fn cancel_in_both_languages() {
        for q in ["отмени выключение", "отмени перезагрузку", "cancel shutdown", "cancel restart", "abort shutdown", "отменить выключение компьютера"] {
            assert_eq!(parse(q, NOON), Some(Cmd::Cancel), "{q}");
        }
    }

    #[test]
    fn ordinary_text_and_bad_times_are_not_commands() {
        for q in ["shutdown", "выключи", "restart", "shut down in", "shut down at 25:00", "shut down at 12:75", "shut down in 0 minutes", "shut down in 40 days", "chrome", "how to shut down in windows", "перезагрузи когда-нибудь"] {
            let r = parse(q, NOON);
            assert!(r.is_none() || q.starts_with("how"), "{q} -> {r:?}");
        }
        assert_eq!(parse("shut down at 13 pm", NOON), None);
    }

    #[test]
    fn shutdown_arguments() {
        assert_eq!(args(&Cmd::Arm { kind: Kind::Shutdown, secs: 1800 }), ["/s", "/t", "1800"]);
        assert_eq!(args(&Cmd::Arm { kind: Kind::Restart, secs: 60 }), ["/r", "/t", "60"]);
        assert_eq!(args(&Cmd::Cancel), ["/a"]);
    }

    struct Mock(RefCell<Vec<Vec<String>>>, bool);
    impl Runner for Mock {
        fn run(&self, a: &[String]) -> Result<(), String> {
            self.0.borrow_mut().push(a.to_vec());
            if self.1 && a == ["/a"] { Err("1116".into()) } else { Ok(()) }
        }
    }

    #[test]
    fn arming_aborts_the_old_schedule_first_and_tracks_the_due_time() {
        let m = Mock(RefCell::new(vec![]), true); // nothing scheduled: /a fails, arming goes on
        let mut slot = None;
        apply(&m, &Cmd::Arm { kind: Kind::Restart, secs: 90 }, 1_000_000, &mut slot).unwrap();
        assert_eq!(*m.0.borrow(), vec![vec!["/a".to_string()], vec!["/r".into(), "/t".into(), "90".into()]]);
        assert_eq!(slot, Some(Armed { kind: Kind::Restart, due_ms: 1_090_000 }));
        apply(&m, &Cmd::Cancel, 2_000_000, &mut slot).unwrap(); // a failing abort is not an error
        assert_eq!(slot, None);
    }

    #[test]
    fn row_payloads_round_trip_and_junk_is_refused() {
        for c in [Cmd::Cancel, Cmd::Arm { kind: Kind::Shutdown, secs: 1800 }, Cmd::Arm { kind: Kind::Restart, secs: 7 }] {
            assert_eq!(decode(&encode(&c)), Some(c));
        }
        for p in ["pt:s:0", "pt:s:99999999", "pt:x:5", "lock", "pt:s:abc"] {
            assert_eq!(decode(p), None, "{p}");
        }
        assert_eq!(human(5400, false), "1 h 30 min");
        assert_eq!(human(600, true), "10 мин");
    }
}
