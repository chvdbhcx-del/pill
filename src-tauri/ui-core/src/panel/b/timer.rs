//! Timer ring + reminders: the right-hand card of the Note tab (src/tab-note.js, "timer" and "reminders" sections).
//!
//! Backend: timers.rs (`timer_state`, `timer_start`, `timer_cancel`, `remind_list`, `remind_add`, `remind_done`,
//! `remind_snooze`, `remind_delete`; events `remind:due`, `remind:changed`, `timer:tick`, `timer:done`).
//! The running timer also feeds the collapsed pill's live row (`Effect::SetLive{key:"timer"}`, priority 60, key = "run" + clock
//! length so the chip is rebuilt only when the clock gains a digit and the seconds are written in place).
//! Wall time comes from `set_now(ms)` (kept in step by `tick`), local time from `set_tz_offset_min`, 12/24 h from `set_clock24`.
//! `next_tick_hint_ms()` tells the host how soon the ring needs repainting (4 Hz while the Note tab shows it, once per
//! displayed second otherwise, never while the pill is hidden); the host may always tick faster, never slower.

use super::{plural, Anim, BackendCall, Effect, Fx, Kv, Lang, LiveRow, Reply, Tags, Timer, ToastAction, ToastRun, ToastSpec, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const RING_CIRC: f64 = 2.0 * std::f64::consts::PI * 34.0;
pub const PRESETS_MIN: [u32; 5] = [5, 10, 15, 25, 45];
pub const KV_SEL: &str = "pill.timer.sel";
const MSG_MS: u32 = 3500;
const REM_REFRESH_MS: u32 = 30_000;

/// "12" = minutes, "1:30" = 1 min 30 s, "1:02:03" = h:mm:ss, "90s", "1h30m", "2 мин". Returns seconds or 0.
pub fn parse_dur(s: &str) -> u64 {
    let s: String = s.trim_matches(super::text::js_ws).to_lowercase();
    let all_digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    let num = |x: &str| x.parse::<u64>().unwrap_or(0);
    if all_digits(&s) {
        return num(&s) * 60;
    }
    let parts: Vec<&str> = s.split(':').collect();
    let ok2 = |x: &str| all_digits(x) && x.len() <= 2;
    if parts.len() == 2 && all_digits(parts[0]) && ok2(parts[1]) {
        return num(parts[0]) * 60 + num(parts[1]);
    }
    if parts.len() == 3 && all_digits(parts[0]) && ok2(parts[1]) && ok2(parts[2]) {
        return num(parts[0]) * 3600 + num(parts[1]) * 60 + num(parts[2]);
    }
    // ^(?:(\d+)\s*(?:h|ч)[a-zа-я]*)?\s*(?:(\d+)\s*(?:m|м)[a-zа-я]*)?\s*(?:(\d+)\s*(?:s|с)[a-zа-я]*)?$
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let ws = |i: &mut usize| {
        while *i < c.len() && super::text::js_ws(c[*i]) {
            *i += 1;
        }
    };
    let group = |i: &mut usize, units: [char; 2]| -> Option<u64> {
        let st = *i;
        let mut j = *i;
        while j < c.len() && c[j].is_ascii_digit() {
            j += 1;
        }
        if j == st {
            return None;
        }
        let n: u64 = c[st..j].iter().collect::<String>().parse().unwrap_or(0);
        while j < c.len() && super::text::js_ws(c[j]) {
            j += 1;
        }
        if j < c.len() && units.contains(&c[j]) {
            j += 1;
            while j < c.len() && (c[j].is_ascii_lowercase() || ('а'..='я').contains(&c[j])) {
                j += 1;
            }
            *i = j;
            Some(n)
        } else {
            None
        }
    };
    let h = group(&mut i, ['h', 'ч']);
    ws(&mut i);
    let m = group(&mut i, ['m', 'м']);
    ws(&mut i);
    let sec = group(&mut i, ['s', 'с']);
    if i != c.len() || (h.is_none() && m.is_none() && sec.is_none()) {
        return 0;
    }
    h.unwrap_or(0) * 3600 + m.unwrap_or(0) * 60 + sec.unwrap_or(0)
}

/// m:ss or h:mm:ss, seconds rounded up.
pub fn fmt_clock(sec: f64) -> String {
    let sec = sec.max(0.0).ceil() as u64;
    let (h, m, s) = (sec / 3600, (sec % 3600) / 60, sec % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

// ---- local date arithmetic (no chrono) ----
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub struct Wall {
    pub days: i64,
    pub hour: u32,
    pub min: u32,
    pub weekday: usize, // 0 = Sunday
    pub month: u32,
    pub day: u32,
}
pub fn wall(ms: i64, tz_min: i32) -> Wall {
    let local = ms + tz_min as i64 * 60_000;
    let days = local.div_euclid(86_400_000);
    let secs = local.rem_euclid(86_400_000) / 1000;
    let (_, month, day) = civil(days);
    Wall { days, hour: (secs / 3600) as u32, min: ((secs % 3600) / 60) as u32, weekday: (days + 4).rem_euclid(7) as usize, month, day }
}

const WD_RU: [&str; 7] = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];
const WD_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MO_RU: [&str; 12] = ["янв.", "февр.", "мар.", "апр.", "мая", "июн.", "июл.", "авг.", "сент.", "окт.", "нояб.", "дек."];
const MO_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Display settings for dates/times.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub tz_min: i32,
    pub clock24: bool,
}
pub fn hm(w: &Wall, clock24: bool) -> String {
    if clock24 {
        format!("{:02}:{:02}", w.hour, w.min)
    } else {
        let h12 = if w.hour % 12 == 0 { 12 } else { w.hour % 12 };
        format!("{}:{:02} {}", h12, w.min, if w.hour < 12 { "AM" } else { "PM" })
    }
}

/// "overdue · 14:05", "in 12 min", "14:05", "tomorrow 09:00", "Mon 09:00", "Nov 5 09:00".
pub fn fmt_due(ms: i64, now: i64, lang: Lang, clk: Clock) -> String {
    let (d, n) = (wall(ms, clk.tz_min), wall(now, clk.tz_min));
    let diff = ms - now;
    let ru = lang.is_ru();
    if diff <= 0 {
        return format!("{} · {}", lang.t("просрочено", "overdue"), hm(&d, clk.clock24));
    }
    if diff < 3_600_000 {
        let m = (((diff as f64) / 60_000.0).round() as i64).max(1);
        return if ru { format!("через {m} мин") } else { format!("in {m} min") };
    }
    let k = d.days - n.days;
    if k == 0 {
        hm(&d, clk.clock24)
    } else if k == 1 {
        format!("{} {}", lang.t("завтра", "tomorrow"), hm(&d, clk.clock24))
    } else if k < 7 {
        format!("{} {}", if ru { WD_RU[d.weekday] } else { WD_EN[d.weekday] }, hm(&d, clk.clock24))
    } else if ru {
        format!("{} {} {}", d.day, MO_RU[d.month as usize - 1], hm(&d, clk.clock24))
    } else {
        format!("{} {} {}", MO_EN[d.month as usize - 1], d.day, hm(&d, clk.clock24))
    }
}

/// "daily", "weekly", "monthly", "every:90" (minutes) -> readable text; "" for none.
pub fn fmt_repeat(rule: &str, lang: Lang) -> String {
    if rule.is_empty() || rule == "none" {
        return String::new();
    }
    match rule {
        "daily" => return lang.t("каждый день", "daily").into(),
        "weekly" => return lang.t("каждую неделю", "weekly").into(),
        "monthly" => return lang.t("каждый месяц", "monthly").into(),
        _ => {}
    }
    let m: i64 = rule.split(':').nth(1).and_then(|x| x.parse().ok()).unwrap_or(0);
    let ru = lang.is_ru();
    if m != 0 && m % 1440 == 0 {
        if ru { format!("каждые {} дн.", m / 1440) } else { format!("every {} d", m / 1440) }
    } else if m != 0 && m % 60 == 0 {
        if ru { format!("каждые {} ч", m / 60) } else { format!("every {} h", m / 60) }
    } else if ru {
        format!("каждые {m} мин")
    } else {
        format!("every {m} min")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    StartStop,
    /// select the duration (seconds, clamped to 1 s .. 99 h)
    SetSel(u64),
    /// one of the preset chips (minutes)
    Preset(u32),
    /// "Custom time" pressed: the button turns into a text field
    CustomOpen,
    /// the field lost focus / Enter (apply = true) / Escape (apply = false)
    CustomCommit { text: String, apply: bool },
    ReminderInput(String),
    AddReminder,
    ReminderDone(i64),
    ReminderSnooze(i64),
    ReminderDelete(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct TimerState {
    active: bool,
    id: i64,
    due_at: i64,
    total_s: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReminderRow {
    pub id: i64,
    pub text: String,
    pub when: String,
    /// "↻ daily" badge text or empty
    pub repeat: String,
    pub overdue: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TimerView {
    pub clock: String,
    /// font size steps with the clock length, the box never reflows
    pub clock_len: usize,
    pub running: bool,
    /// fraction of the ring that is still filled (1 = full); the JS dashoffset is `CIRC * (1 - left)`
    pub left: f64,
    pub start_label: String,
    pub presets: Vec<(u32, bool)>,
    pub unit: String,
    pub custom_label: String,
    pub custom_editing: bool,
    pub custom_placeholder: String,
    pub reminders_head: String,
    pub reminders_count: usize,
    pub empty_hint: Vec<String>,
    pub reminders: Vec<ReminderRow>,
    pub msg: Option<(String, bool)>,
    pub input: String,
    pub input_placeholder: String,
}

#[derive(Debug)]
enum Pend {
    State,
    Start,
    Cancel,
    List,
    Add,
    Other,
}

pub struct TimerCard {
    pub fx: Fx,
    lang: Lang,
    clk: Clock,
    kv: Box<dyn Kv>,
    now: i64,
    timer: TimerState,
    sel_s: u64,
    live_sec: i64,
    zero_synced: bool,
    resync: Timer,
    visible: bool,
    pill_hidden: bool,
    custom_editing: bool,
    live_on: bool,
    tags: Tags,
    pend: HashMap<u64, Pend>,
    // reminders
    list: Vec<Value>,
    rem_known: bool,
    input: String,
    msg: Option<(String, bool)>,
    msg_left: Timer,
    refresh: Timer,
}

impl TimerCard {
    pub fn new(lang: Lang, kv: Box<dyn Kv>, owner: u8) -> Self {
        let sel = kv.get(KV_SEL).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| *v >= 1.0).map(|v| v as u64).unwrap_or(300).max(1);
        Self {
            fx: Fx::default(), lang, clk: Clock { tz_min: 0, clock24: true }, kv, now: 0, timer: TimerState::default(), sel_s: sel, live_sec: -1, zero_synced: false,
            resync: Timer::default(), visible: false, pill_hidden: false, custom_editing: false, live_on: true, tags: Tags::new(owner), pend: HashMap::new(),
            list: vec![], rem_known: false, input: String::new(), msg: None, msg_left: Timer::default(), refresh: Timer::default(),
        }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
    }
    pub fn set_now(&mut self, ms: i64) {
        self.now = ms;
    }
    pub fn set_tz_offset_min(&mut self, m: i32) {
        self.clk.tz_min = m;
    }
    pub fn set_clock24(&mut self, v: bool) {
        self.clk.clock24 = v;
    }
    /// Settings > Live events > Reminders and timers (off = no card, no edge reaction)
    pub fn set_live_on(&mut self, v: bool) {
        self.live_on = v;
    }
    pub fn selected_secs(&self) -> u64 {
        self.sel_s
    }
    pub fn is_running(&self) -> bool {
        self.timer.active
    }
    pub fn remaining_s(&self) -> f64 {
        if self.timer.active { ((self.timer.due_at - self.now) as f64 / 1000.0).max(0.0) } else { 0.0 }
    }
    fn call(&mut self, cmd: &'static str, args: Value, p: Pend) -> BackendCall {
        let tag = self.tags.next();
        self.pend.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }

    // ---- lifecycle ----
    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.visible = true;
        self.rem_known = false;
        self.refresh.arm(REM_REFRESH_MS);
        vec![self.call("timer_state", json!({}), Pend::State), self.call("remind_list", json!({}), Pend::List)]
    }
    pub fn on_hide(&mut self) {
        self.visible = false;
        self.refresh.cancel();
    }
    pub fn set_pill_hidden(&mut self, hidden: bool) {
        self.pill_hidden = hidden;
        if !hidden {
            self.paint_live();
        }
    }
    /// The host is told when to tick next (None = nothing to repaint).
    pub fn next_tick_hint_ms(&self) -> Option<u32> {
        if !self.timer.active || self.pill_hidden {
            return None;
        }
        let left = self.timer.due_at - self.now;
        let r = left % 1000;
        Some(if self.visible || left <= 0 { 250 } else { (if r == 0 { 1000 } else { r }) as u32 + 20 })
    }
    /// Initial load of the timer (the JS mount did `syncTimer()` before the tab was shown).
    pub fn sync(&mut self) -> BackendCall {
        self.call("timer_state", json!({}), Pend::State)
    }

    // ---- timer ----
    fn apply_timer(&mut self, v: &Value) {
        self.timer = if v.get("active").and_then(Value::as_bool).unwrap_or(false) {
            TimerState { active: true, id: v["id"].as_i64().unwrap_or(0), due_at: v["due_at"].as_i64().unwrap_or(0), total_s: v["total_s"].as_i64().unwrap_or(0) }
        } else {
            TimerState::default()
        };
        self.zero_synced = false;
        self.resync.cancel();
        self.paint_live();
    }
    fn clock_text(&self) -> String {
        fmt_clock(if self.timer.active { self.remaining_s() } else { self.sel_s as f64 })
    }
    /// The collapsed pill shows the running timer; written when the displayed second changes.
    fn paint_live(&mut self) {
        if self.pill_hidden {
            return;
        }
        let run = self.timer.active;
        let sec = if run { self.remaining_s().ceil() as i64 } else { -1 };
        if sec == self.live_sec {
            return;
        }
        self.live_sec = sec;
        let clock = self.clock_text();
        let row = run.then(|| LiveRow { priority: 60, key: format!("run{}", clock.chars().count()), text: clock, accent: None, ttl_ms: None, style: "timer" });
        self.fx.eff(Effect::SetLive { key: "timer".into(), row });
    }
    fn start_stop(&mut self) -> Vec<BackendCall> {
        if self.timer.active {
            vec![self.call("timer_cancel", json!({}), Pend::Cancel)]
        } else {
            vec![self.call("timer_start", json!({ "seconds": self.sel_s }), Pend::Start)]
        }
    }
    fn set_sel(&mut self, sec: u64) {
        self.sel_s = sec.clamp(1, 99 * 3600);
        self.kv.set(KV_SEL, &self.sel_s.to_string());
        self.live_sec = -2;
        self.paint_live();
    }

    // ---- reminders ----
    fn show_msg(&mut self, text: String, bad: bool) {
        self.msg = Some((text, bad));
        self.msg_left.arm(MSG_MS);
    }
    pub fn prefill_reminder(&mut self, text: &str, hint: bool) {
        self.input = text.to_string();
        self.fx.eff(Effect::Focus("reminder-input"));
        if hint {
            let m = self.lang.t("Допиши, когда напомнить: «завтра в 9»", "Add when to remind: \"tomorrow at 9\"").to_string();
            self.show_msg(m, false);
        }
    }
    /// pill:open-tab {tab:'note', section:'reminders'}: scroll to the block and focus its input
    pub fn reveal(&mut self) {
        self.fx.eff(Effect::ScrollIntoView("reminders"));
        self.fx.eff(Effect::Focus("reminder-input"));
    }
    fn row_view(&self, v: &Value) -> ReminderRow {
        let rule = v["repeat_rule"].as_str().unwrap_or("");
        let rep = fmt_repeat(rule, self.lang);
        ReminderRow {
            id: v["id"].as_i64().unwrap_or(0),
            text: v["text"].as_str().unwrap_or("").to_string(),
            when: fmt_due(v["due_at"].as_i64().unwrap_or(0), self.now, self.lang, self.clk),
            repeat: if rep.is_empty() { rep } else { format!("↻ {rep}") },
            overdue: v["fired"].as_bool().unwrap_or(false),
        }
    }
    fn draw(&mut self) -> BackendCall {
        self.call("remind_list", json!({}), Pend::List)
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::StartStop => {
                self.fx.anim("timer-start", Anim::Press);
                self.start_stop()
            }
            Action::SetSel(s) => {
                self.set_sel(s);
                vec![]
            }
            Action::Preset(m) => {
                self.fx.anim(&format!("preset-{m}"), Anim::Press);
                self.set_sel(m as u64 * 60);
                vec![]
            }
            Action::CustomOpen => {
                self.custom_editing = true;
                self.fx.eff(Effect::Focus("timer-custom"));
                vec![]
            }
            Action::CustomCommit { text, apply } => {
                if !self.custom_editing {
                    return vec![];
                }
                self.custom_editing = false;
                if apply {
                    let s = parse_dur(&text);
                    if s > 0 {
                        self.set_sel(s);
                    } else if !text.trim().is_empty() {
                        let m = self.lang.t("Не понял время", "Could not read that time").to_string();
                        self.show_msg(m, true);
                    }
                }
                vec![]
            }
            Action::ReminderInput(t) => {
                self.input = t;
                vec![]
            }
            Action::AddReminder => {
                let text = self.input.trim().to_string();
                if text.is_empty() {
                    return vec![];
                }
                vec![self.call("remind_add", json!({ "text": text }), Pend::Add)]
            }
            Action::ReminderDone(id) => {
                let repeating = self.list.iter().find(|v| v["id"].as_i64() == Some(id)).map_or(false, |v| v["repeat_rule"].as_str().map_or(false, |r| !r.is_empty() && r != "none"));
                if !repeating {
                    self.fx.anim(&format!("rem:{id}"), Anim::Collapse);
                }
                vec![self.call("remind_done", json!({ "id": id }), Pend::Other), self.draw()]
            }
            Action::ReminderSnooze(id) => vec![self.call("remind_snooze", json!({ "id": id, "minutes": 10 }), Pend::Other), self.draw()],
            Action::ReminderDelete(id) => {
                self.fx.anim(&format!("rem:{id}"), Anim::Collapse);
                vec![self.call("remind_delete", json!({ "id": id }), Pend::Other), self.draw()]
            }
        }
    }

    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        let Some(p) = self.pend.remove(&tag) else { return vec![] };
        match p {
            Pend::State => {
                if r.ok {
                    self.apply_timer(&r.v);
                }
            }
            Pend::Start => {
                if r.ok {
                    self.apply_timer(&r.v);
                } else {
                    let m = self.lang.t("Таймер недоступен", "Timer unavailable").to_string();
                    self.show_msg(m, true);
                }
            }
            Pend::Cancel => {
                let v = if r.ok { r.v } else { Value::Null };
                self.apply_timer(&v);
            }
            Pend::List => {
                let list = if r.ok { r.v.as_array().cloned().unwrap_or_default() } else { vec![] };
                let was_empty = self.list.is_empty();
                if self.rem_known {
                    if !list.is_empty() && was_empty {
                        self.fx.anim("reminders-list", Anim::Enter { y: 6.0, ms: 220 });
                    } else if list.is_empty() && !was_empty {
                        self.fx.anim("reminders-empty", Anim::Enter { y: 6.0, ms: 220 });
                    }
                    if !list.is_empty() {
                        // rows that stay slide to their new place, a reminder that was not there drops in
                        self.fx.anim("reminders-list", Anim::Flip);
                    }
                }
                self.list = list;
                self.rem_known = true;
            }
            Pend::Add => {
                if r.ok {
                    self.input.clear();
                    let due = fmt_due(r.v["due_at"].as_i64().unwrap_or(0), self.now, self.lang, self.clk);
                    let m = format!("{}{}", self.lang.t("Напомню: ", "Reminder set: "), due);
                    self.show_msg(m, false);
                    return vec![self.draw()];
                }
                let (ru, en) = if r.err.contains("past") {
                    ("Это время уже прошло", "That moment has already passed")
                } else if r.err.contains("no_time") {
                    ("Не нашёл время. Например: «завтра в 9» или «через 20 минут»", "No time found. Try \"tomorrow at 9\" or \"in 20 minutes\"")
                } else {
                    ("Не удалось сохранить напоминание", "Could not save the reminder")
                };
                let m = self.lang.t(ru, en).to_string();
                self.show_msg(m, true);
            }
            Pend::Other => {}
        }
        vec![]
    }

    pub fn on_event(&mut self, name: &str, p: &Value) -> Vec<BackendCall> {
        match name {
            "remind:due" => self.on_due(p),
            "remind:changed" => vec![self.draw(), self.sync()],
            "timer:tick" => {
                if let Some(id) = p["id"].as_i64().filter(|i| *i != 0) {
                    let v = json!({ "active": true, "id": id, "due_at": self.now + p["remaining_ms"].as_i64().unwrap_or(0), "total_s": p["total_s"].as_i64().unwrap_or(0) });
                    self.apply_timer(&v);
                }
                vec![]
            }
            _ => vec![],
        }
    }

    /// card in the collapsed pill when something fires
    fn on_due(&mut self, p: &Value) -> Vec<BackendCall> {
        if p.is_null() {
            return vec![];
        }
        let cards = self.live_on;
        if cards {
            self.fx.eff(Effect::React { key: "remind".into(), kind: "attention", ttl_ms: Some(5000) });
        }
        let id = p["id"].as_i64().unwrap_or(0);
        let late = p["late"].as_bool().unwrap_or(false);
        let ru = self.lang.is_ru();
        if p["kind"].as_str() == Some("timer") {
            self.apply_timer(&Value::Null);
            if cards {
                let spec = ToastSpec {
                    id: "timer-done".into(),
                    icon: "bell".into(),
                    title: self.lang.t("Таймер закончился", "Timer finished").into(),
                    ttl_ms: 20_000,
                    actions: vec![
                        ToastAction { label: self.lang.t("Ещё раз", "Again").into(), kind: None, run: ToastRun::Model("timer:again".into()) },
                        ToastAction { label: self.lang.t("Ок", "OK").into(), kind: None, run: ToastRun::Nothing },
                    ],
                    ..Default::default()
                };
                self.fx.eff(Effect::Card(spec));
            }
        } else if cards {
            let rule = p["repeat"].as_str().unwrap_or("");
            let repeating = !rule.is_empty() && rule != "none";
            let sub = if late { (if ru { "пропущено" } else { "missed" }).to_string() } else if repeating { fmt_repeat(rule, self.lang) } else { String::new() };
            let mut done = ToastAction { label: self.lang.t("Готово", "Done").into(), kind: None, run: ToastRun::Call("remind_done", json!({ "id": id })) };
            if repeating {
                done.run = ToastRun::Nothing;
            }
            let spec = ToastSpec {
                id: format!("remind-{id}"),
                icon: "bell".into(),
                title: p["text"].as_str().unwrap_or("").to_string(),
                sub,
                ttl_ms: 30_000,
                actions: vec![ToastAction { label: self.lang.t("Отложить 10 мин", "Snooze 10 min").into(), kind: None, run: ToastRun::Call("remind_snooze", json!({ "id": id, "minutes": 10 })) }, done],
                ..Default::default()
            };
            self.fx.eff(Effect::Card(spec));
        }
        vec![self.draw()]
    }

    /// A `ToastRun::Model(id)` button was pressed.
    pub fn on_toast_action(&mut self, id: &str) -> Vec<BackendCall> {
        if id == "timer:again" { self.start_stop() } else { vec![] }
    }

    pub fn tick(&mut self, dt_ms: u32) -> Vec<BackendCall> {
        self.now += dt_ms as i64;
        let mut calls = vec![];
        if self.msg_left.tick(dt_ms) {
            self.msg = None;
            self.fx.anim("reminder-msg", Anim::Exit { y: -4.0, scale: 1.0, ms: 180 });
        }
        if self.refresh.tick(dt_ms) {
            self.refresh.arm(REM_REFRESH_MS);
            if self.visible {
                calls.push(self.draw());
            }
        }
        if self.resync.tick(dt_ms) {
            calls.push(self.sync());
        }
        if self.timer.active && !self.pill_hidden {
            self.paint_live();
            if self.remaining_s() <= 0.0 && !self.zero_synced {
                self.zero_synced = true; // Rust fires timer:done; resync if the event was missed
                self.resync.arm(1500);
            }
        }
        calls
    }

    pub fn view(&self) -> TimerView {
        let l = self.lang;
        let run = self.timer.active;
        let rem = self.remaining_s();
        let clock = self.clock_text();
        TimerView {
            clock_len: clock.chars().count(),
            clock,
            running: run,
            left: if run { (rem / (self.timer.total_s.max(1) as f64)).clamp(0.0, 1.0) } else { 1.0 },
            start_label: if run { l.t("Стоп", "Stop") } else { l.t("Старт", "Start") }.into(),
            presets: PRESETS_MIN.iter().map(|m| (*m, !run && *m as u64 * 60 == self.sel_s)).collect(),
            unit: l.t("мин", "min").into(),
            custom_label: l.t("Своё время", "Custom time").into(),
            custom_editing: self.custom_editing,
            custom_placeholder: l.t("12, 1:30, 90с", "12, 1:30, 90s").into(),
            reminders_head: l.t("Напоминания", "Reminders").into(),
            reminders_count: self.list.len(),
            empty_hint: [
                l.t("Напиши ниже, о чём и когда:", "Write below what and when:"),
                l.t("«позвонить маме завтра в 9»,", "\"call mum tomorrow at 9\","),
                l.t("«таблетки каждый день в 23:00»,", "\"pills every day at 23:00\","),
                l.t("«через 20 минут выключить плиту».", "\"turn off the stove in 20 minutes\"."),
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            reminders: self.list.iter().map(|v| self.row_view(v)).collect(),
            msg: self.msg.clone(),
            input: self.input.clone(),
            input_placeholder: l.t("Позвонить маме завтра в 9", "Call mum tomorrow at 9").into(),
        }
    }
    /// "3 reminders" style summary for accessibility labels
    pub fn count_label(&self) -> String {
        let n = self.list.len() as i64;
        if self.lang.is_ru() { format!("{n} {}", plural(n, "напоминание", "напоминания", "напоминаний")) } else { format!("{n} reminder{}", if n == 1 { "" } else { "s" }) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::b::MemKv;

    const T0: i64 = 1_761_800_000_000; // 2025-10-30 04:53:20 UTC

    fn card() -> TimerCard {
        let mut c = TimerCard::new(Lang::En, Box::new(MemKv::default()), 2);
        c.set_now(T0);
        c
    }
    fn run(c: &mut TimerCard, calls: Vec<BackendCall>, replies: Vec<(&str, Reply)>) -> Vec<BackendCall> {
        let mut out = vec![];
        for call in calls {
            if let Some((_, r)) = replies.iter().find(|(cmd, _)| *cmd == call.cmd) {
                out.extend(c.on_reply(call.tag, r.clone()));
            }
        }
        out
    }
    fn state(active: bool, due: i64, total: i64) -> Reply {
        Reply::ok(json!({ "active": active, "id": 7, "due_at": due, "total_s": total, "remaining_ms": due - T0 }))
    }

    #[test]
    fn parse_dur_matches_js_cases() {
        for (s, want) in [
            ("12", 720), ("1:30", 90), ("1:02:03", 3723), ("90s", 90), ("1h30m", 5400), ("2 мин", 120), ("90с", 90), ("1 час 30 мин", 5400), ("5 min 30 sec", 330),
            ("2ч", 7200), ("", 0), ("abc", 0), ("1:5:", 0), ("1:123", 0), ("мин", 0), ("1m 30", 0), ("  45 S ", 45), ("0", 0), ("1h", 3600), ("30 sec", 30), ("1:30:15", 5415),
        ] {
            assert_eq!(parse_dur(s), want, "{s:?}");
        }
    }
    #[test]
    fn clock_text() {
        assert_eq!(fmt_clock(300.0), "5:00");
        assert_eq!(fmt_clock(0.2), "0:01");
        assert_eq!(fmt_clock(3723.0), "1:02:03");
        assert_eq!(fmt_clock(-5.0), "0:00");
        assert_eq!(fmt_clock(59.01), "1:00");
    }
    #[test]
    fn due_formatting() {
        let clk = Clock { tz_min: 180, clock24: true }; // UTC+3: now = 07:53
        assert_eq!(fmt_due(T0 - 60_000, T0, Lang::En, clk), "overdue · 07:52");
        assert_eq!(fmt_due(T0 + 12 * 60_000, T0, Lang::En, clk), "in 12 min");
        assert_eq!(fmt_due(T0 + 12 * 60_000, T0, Lang::Ru, clk), "через 12 мин");
        assert_eq!(fmt_due(T0 + 3 * 3_600_000, T0, Lang::En, clk), "10:53");
        assert_eq!(fmt_due(T0 + 24 * 3_600_000, T0, Lang::En, clk), "tomorrow 07:53");
        assert_eq!(fmt_due(T0 + 24 * 3_600_000, T0, Lang::Ru, clk), "завтра 07:53");
        assert_eq!(fmt_due(T0 + 3 * 86_400_000, T0, Lang::En, clk), "Sun 07:53");
        assert_eq!(fmt_due(T0 + 10 * 86_400_000, T0, Lang::En, clk), "Nov 9 07:53");
        assert_eq!(fmt_due(T0 + 10 * 86_400_000, T0, Lang::Ru, clk), "9 нояб. 07:53");
        let c12 = Clock { tz_min: 180, clock24: false };
        assert_eq!(fmt_due(T0 + 3 * 3_600_000, T0, Lang::En, c12), "10:53 AM");
        assert_eq!(fmt_due(T0 - 3_600_000, T0, Lang::En, c12), "overdue · 6:53 AM");
        // local midnight boundary: 23:30 local is "today", 00:30 local the next day is "tomorrow"
        let clk = Clock { tz_min: 0, clock24: true };
        let now = T0 - (4 * 3600 + 53 * 60 + 20) * 1000 + 22 * 3_600_000; // 22:00 UTC
        assert_eq!(fmt_due(now + 90 * 60_000, now, Lang::En, clk), "23:30");
        assert_eq!(fmt_due(now + 150 * 60_000, now, Lang::En, clk), "tomorrow 00:30");
    }
    #[test]
    fn repeat_text() {
        assert_eq!(fmt_repeat("none", Lang::En), "");
        assert_eq!(fmt_repeat("daily", Lang::Ru), "каждый день");
        assert_eq!(fmt_repeat("every:2880", Lang::En), "every 2 d");
        assert_eq!(fmt_repeat("every:120", Lang::Ru), "каждые 2 ч");
        assert_eq!(fmt_repeat("every:45", Lang::En), "every 45 min");
    }

    #[test]
    fn start_stop_and_live_row() {
        let mut c = card();
        let on = c.on_action(Action::StartStop);
        assert_eq!((on[0].cmd, on[0].args.clone()), ("timer_start", json!({ "seconds": 300 })));
        run(&mut c, on, vec![("timer_start", state(true, T0 + 300_000, 300))]);
        let v = c.view();
        assert!(v.running && v.clock == "5:00" && v.start_label == "Stop" && v.left > 0.99);
        let fx = c.fx.take_effects();
        assert_eq!(fx.len(), 1);
        match &fx[0] {
            Effect::SetLive { key, row: Some(r) } => assert_eq!((key.as_str(), r.priority, r.key.as_str(), r.text.as_str()), ("timer", 60, "run4", "5:00")),
            e => panic!("{e:?}"),
        }
        // the same displayed second does not write again; a new second does
        c.tick(100);
        assert!(c.fx.take_effects().is_empty());
        c.tick(1000);
        match &c.fx.take_effects()[..] {
            [Effect::SetLive { row: Some(r), .. }] => assert_eq!(r.text, "4:59"),
            e => panic!("{e:?}"),
        }
        // stop
        let off = c.on_action(Action::StartStop);
        assert_eq!(off[0].cmd, "timer_cancel");
        run(&mut c, off, vec![("timer_cancel", state(false, 0, 0))]);
        assert!(!c.view().running);
        assert_eq!(c.fx.take_effects(), vec![Effect::SetLive { key: "timer".into(), row: None }]);
    }
    #[test]
    fn clock_gaining_a_digit_changes_the_live_key() {
        let mut c = card();
        c.apply_timer(&json!({ "active": true, "id": 1, "due_at": T0 + 601_000, "total_s": 601 }));
        c.fx.take_effects();
        assert_eq!(c.view().clock, "10:01");
        c.tick(2000); // 9:59: shorter clock -> new key
        match &c.fx.take_effects()[..] {
            [Effect::SetLive { row: Some(r), .. }] => assert_eq!((r.key.as_str(), r.text.as_str()), ("run4", "9:59")),
            e => panic!("{e:?}"),
        }
    }
    #[test]
    fn unavailable_timer_says_so() {
        let mut c = card();
        let on = c.on_action(Action::StartStop);
        run(&mut c, on, vec![("timer_start", Reply::err("boom"))]);
        assert_eq!(c.view().msg, Some(("Timer unavailable".into(), true)));
        c.tick(3499);
        assert!(c.view().msg.is_some());
        c.tick(1);
        assert!(c.view().msg.is_none());
    }
    #[test]
    fn zero_resync_after_1500ms() {
        let mut c = card();
        c.apply_timer(&json!({ "active": true, "id": 1, "due_at": T0 + 1000, "total_s": 1 }));
        let a = c.tick(1000);
        assert!(a.is_empty());
        let b = c.tick(1499);
        assert!(b.is_empty());
        let d = c.tick(1);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].cmd, "timer_state");
    }
    #[test]
    fn selection_is_clamped_and_remembered() {
        let kv = MemKv::default();
        let mut c = TimerCard::new(Lang::En, Box::new(kv), 2);
        c.on_action(Action::Preset(25));
        assert_eq!(c.selected_secs(), 1500);
        assert_eq!(c.view().clock, "25:00");
        assert!(c.view().presets.contains(&(25, true)) && c.view().presets.contains(&(5, false)));
        c.on_action(Action::SetSel(0));
        assert_eq!(c.selected_secs(), 1);
        c.on_action(Action::SetSel(1_000_000));
        assert_eq!(c.selected_secs(), 99 * 3600);
        let mut kv = MemKv::default();
        kv.set(KV_SEL, "600");
        assert_eq!(TimerCard::new(Lang::En, Box::new(kv), 2).selected_secs(), 600);
        let mut kv = MemKv::default();
        kv.set(KV_SEL, "nonsense");
        assert_eq!(TimerCard::new(Lang::En, Box::new(kv), 2).selected_secs(), 300);
    }
    #[test]
    fn custom_time_field() {
        let mut c = card();
        c.on_action(Action::CustomOpen);
        assert!(c.view().custom_editing);
        c.on_action(Action::CustomCommit { text: "1:30".into(), apply: true });
        assert!(!c.view().custom_editing);
        assert_eq!(c.selected_secs(), 90);
        c.on_action(Action::CustomOpen);
        c.on_action(Action::CustomCommit { text: "zzz".into(), apply: true });
        assert_eq!(c.view().msg, Some(("Could not read that time".into(), true)));
        c.on_action(Action::CustomOpen);
        c.on_action(Action::CustomCommit { text: "5".into(), apply: false });
        assert_eq!(c.selected_secs(), 90);
        // a commit without an open field (blur after Enter) is ignored
        assert!(c.on_action(Action::CustomCommit { text: "7".into(), apply: true }).is_empty());
        assert_eq!(c.selected_secs(), 90);
    }
    #[test]
    fn timer_tick_event_rebuilds_the_state() {
        let mut c = card();
        c.on_event("timer:tick", &json!({ "id": 3, "remaining_ms": 61_000, "total_s": 120 }));
        let v = c.view();
        assert!(v.running && v.clock == "1:01");
        assert!((v.left - 61.0 / 120.0).abs() < 1e-9);
    }

    #[test]
    fn reminders_list_add_and_errors() {
        let mut c = card();
        let calls = c.on_show();
        assert_eq!(calls.iter().map(|x| x.cmd).collect::<Vec<_>>(), vec!["timer_state", "remind_list"]);
        let list = json!([{ "id": 1, "kind": "reminder", "text": "call mum", "due_at": T0 + 600_000, "repeat_rule": "daily", "fired": false, "total_s": 0, "created_at": 0 },
                          { "id": 2, "kind": "reminder", "text": "late", "due_at": T0 - 1000, "repeat_rule": "none", "fired": true, "total_s": 0, "created_at": 0 }]);
        run(&mut c, calls, vec![("remind_list", Reply::ok(list)), ("timer_state", state(false, 0, 0))]);
        let v = c.view();
        assert_eq!(v.reminders_count, 2);
        assert_eq!(v.reminders[0].when, "in 10 min");
        assert_eq!(v.reminders[0].repeat, "↻ daily");
        assert!(v.reminders[1].overdue && v.reminders[1].when.starts_with("overdue"));
        assert_eq!(c.count_label(), "2 reminders");
        // add
        c.on_action(Action::ReminderInput("  call dad tomorrow at 9 ".into()));
        let add = c.on_action(Action::AddReminder);
        assert_eq!(add[0].args, json!({ "text": "call dad tomorrow at 9" }));
        let more = run(&mut c, add, vec![("remind_add", Reply::ok(json!({ "due_at": T0 + 86_400_000 })))]);
        assert_eq!(more[0].cmd, "remind_list");
        assert_eq!(c.view().input, "");
        assert!(c.view().msg.unwrap().0.starts_with("Reminder set: "));
        // errors
        for (err, msg) in [("past", "That moment has already passed"), ("no_time", "No time found. Try \"tomorrow at 9\" or \"in 20 minutes\""), ("store", "Could not save the reminder")] {
            c.on_action(Action::ReminderInput("x".into()));
            let add = c.on_action(Action::AddReminder);
            run(&mut c, add, vec![("remind_add", Reply::err(err))]);
            assert_eq!(c.view().msg, Some((msg.into(), true)));
            assert_eq!(c.view().input, "x");
        }
        c.on_action(Action::ReminderInput("   ".into()));
        assert!(c.on_action(Action::AddReminder).is_empty());
    }
    #[test]
    fn reminder_row_actions_and_animations() {
        let mut c = card();
        let list = json!([{ "id": 1, "text": "a", "due_at": T0 + 600_000, "repeat_rule": "daily", "fired": false },
                          { "id": 2, "text": "b", "due_at": T0 + 700_000, "repeat_rule": "none", "fired": false }]);
        let l = c.on_show();
        run(&mut c, l, vec![("remind_list", Reply::ok(list))]);
        c.fx.take_anims();
        // repeating reminder: done does not collapse the row; one-shot does
        let d1 = c.on_action(Action::ReminderDone(1));
        assert_eq!((d1[0].cmd, d1[1].cmd), ("remind_done", "remind_list"));
        assert!(c.fx.take_anims().is_empty());
        c.on_action(Action::ReminderDone(2));
        assert_eq!(c.fx.take_anims()[0].target, "rem:2");
        let s = c.on_action(Action::ReminderSnooze(2));
        assert_eq!(s[0].args, json!({ "id": 2, "minutes": 10 }));
        let x = c.on_action(Action::ReminderDelete(2));
        assert_eq!(x[0].cmd, "remind_delete");
        assert_eq!(c.fx.take_anims()[0].kind, Anim::Collapse);
    }
    #[test]
    fn list_animates_only_after_the_first_draw_of_a_visit() {
        let mut c = card();
        let l = c.on_show();
        run(&mut c, l, vec![("remind_list", Reply::ok(json!([])))]);
        assert!(c.fx.take_anims().is_empty());
        let l = vec![c.draw()];
        run(&mut c, l, vec![("remind_list", Reply::ok(json!([{ "id": 1, "text": "a", "due_at": T0 + 600_000, "repeat_rule": "none", "fired": false }])))]);
        let a = c.fx.take_anims();
        assert!(a.iter().any(|x| x.target == "reminders-list" && matches!(x.kind, Anim::Enter { .. })));
    }
    #[test]
    fn due_events_make_cards_and_react() {
        let mut c = card();
        let more = c.on_event("remind:due", &json!({ "id": 5, "text": "pills", "kind": "reminder", "late": false, "repeat": "daily", "due_at": T0 }));
        assert_eq!(more[0].cmd, "remind_list");
        let fx = c.fx.take_effects();
        assert_eq!(fx[0], Effect::React { key: "remind".into(), kind: "attention", ttl_ms: Some(5000) });
        let Effect::Card(card) = &fx[1] else { panic!() };
        assert_eq!((card.id.as_str(), card.title.as_str(), card.sub.as_str(), card.ttl_ms), ("remind-5", "pills", "daily", 30_000));
        assert_eq!(card.actions[0].run, ToastRun::Call("remind_snooze", json!({ "id": 5, "minutes": 10 })));
        assert_eq!(card.actions[1].run, ToastRun::Nothing, "repeating reminders are not marked done by the card");
        // one-shot, late
        c.on_event("remind:due", &json!({ "id": 6, "text": "x", "kind": "reminder", "late": true, "repeat": "none" }));
        let fx = c.fx.take_effects();
        let Effect::Card(card) = &fx[1] else { panic!() };
        assert_eq!(card.sub, "missed");
        assert_eq!(card.actions[1].run, ToastRun::Call("remind_done", json!({ "id": 6 })));
        // quiet: the live-events switch is off
        c.set_live_on(false);
        c.on_event("remind:due", &json!({ "id": 7, "text": "y", "kind": "reminder", "late": false, "repeat": "none" }));
        assert!(c.fx.take_effects().is_empty());
    }
    #[test]
    fn timer_done_clears_state_and_offers_again() {
        let mut c = card();
        c.apply_timer(&json!({ "active": true, "id": 9, "due_at": T0, "total_s": 60 }));
        c.fx.take_effects();
        c.on_event("remind:due", &json!({ "id": 9, "text": "", "kind": "timer", "late": false, "repeat": "none" }));
        assert!(!c.view().running);
        let fx = c.fx.take_effects();
        let Effect::Card(card) = fx.iter().find(|e| matches!(e, Effect::Card(_))).unwrap() else { panic!() };
        assert_eq!((card.id.as_str(), card.ttl_ms), ("timer-done", 20_000));
        assert_eq!(card.actions[0].run, ToastRun::Model("timer:again".into()));
        let again = c.on_toast_action("timer:again");
        assert_eq!(again[0].cmd, "timer_start");
    }
    #[test]
    fn hidden_pill_means_no_ticks_and_no_live_writes() {
        let mut c = card();
        c.apply_timer(&json!({ "active": true, "id": 1, "due_at": T0 + 100_000, "total_s": 100 }));
        c.fx.take_effects();
        c.set_pill_hidden(true);
        assert_eq!(c.next_tick_hint_ms(), None);
        c.tick(5000);
        assert!(c.fx.take_effects().is_empty());
        c.set_pill_hidden(false);
        assert!(matches!(&c.fx.take_effects()[..], [Effect::SetLive { row: Some(_), .. }]));
        assert_eq!(c.next_tick_hint_ms(), Some(1020)); // 100 s left: the next displayed second
        c.on_show();
        assert_eq!(c.next_tick_hint_ms(), Some(250)); // 4 Hz while the Note tab shows the ring
        c.on_hide();
        assert_eq!(c.next_tick_hint_ms(), Some(1020));
    }
    #[test]
    fn periodic_refresh_only_while_visible() {
        let mut c = card();
        c.on_show();
        let calls = c.tick(30_000);
        assert_eq!(calls.iter().filter(|x| x.cmd == "remind_list").count(), 1);
        c.on_hide();
        assert!(c.tick(60_000).is_empty());
    }
    #[test]
    fn prefill_sets_input_and_hint() {
        let mut c = card();
        c.prefill_reminder("buy milk ", true);
        assert_eq!(c.view().input, "buy milk ");
        assert!(c.view().msg.unwrap().0.contains("tomorrow at 9"));
        assert!(c.fx.take_effects().contains(&Effect::Focus("reminder-input")));
    }
}
