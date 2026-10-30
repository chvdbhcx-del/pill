//! The running timer in the collapsed pill and the "timer finished" / "reminder" cards (port of the live parts of tab-note.js;
//! Rust side timers.rs).
//!
//! * `timer:tick {id, remaining_ms, total_s}` (and the `timer_state` answer `{active, id, due_at, total_s}`) -> row `timer`
//!   (priority 60) with the remaining time, built once per run (key = `run` + clock length) and edited in place every second.
//! * `remind:due {id, text, kind, late, repeat, due_at}` -> edge `attention` for 5 s and a card ("Timer finished" 20 s with Again / OK,
//!   a reminder 30 s with Snooze 10 min / Done). Switch: Settings > Live events > Reminders and timers (`live.reminders`).
//!   Timer and reminder cards are exempt from quiet hours (ids `timer-done`, `remind-<id>`).
//! * `remind:changed` -> re-read the timer state.
//!
//! The Note tab owns the ring and the lists; it tells this provider when it is on screen ([`TimerLive::set_visible`]).

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const TIMER_PRIORITY: i32 = 60;
pub const SYNC_AFTER_ZERO_MS: u32 = 1500;
pub const SNOOZE_MIN: i32 = 10;

#[derive(Debug)]
pub struct TimerLive {
    active: bool,
    id: i64,
    due_at: i64,
    total_s: f64,
    live_sec: i64,
    zero_synced: bool,
    visible: bool,
    hidden: bool,
}

impl Default for TimerLive {
    fn default() -> Self {
        TimerLive { active: false, id: 0, due_at: 0, total_s: 0.0, live_sec: -1, zero_synced: false, visible: false, hidden: false }
    }
}

/// "каждый день", "каждые 3 ч", ... for a repeat rule (`daily`, `weekly`, `monthly`, `every:<minutes>`); empty for none.
pub fn fmt_repeat(env: &dyn Env, rule: &str) -> String {
    if rule.is_empty() || rule == "none" {
        return String::new();
    }
    match rule {
        "daily" => return env.tr("каждый день", "daily"),
        "weekly" => return env.tr("каждую неделю", "weekly"),
        "monthly" => return env.tr("каждый месяц", "monthly"),
        _ => {}
    }
    let m = rule.split(':').nth(1).map_or(0.0, |s| num_or0(Some(&Value::String(s.to_string()))).trunc()) as i64;
    if m != 0 && m % 1440 == 0 {
        env.tr(&format!("каждые {} дн.", m / 1440), &format!("every {} d", m / 1440))
    } else if m != 0 && m % 60 == 0 {
        env.tr(&format!("каждые {} ч", m / 60), &format!("every {} h", m / 60))
    } else {
        env.tr(&format!("каждые {m} мин"), &format!("every {m} min"))
    }
}

impl TimerLive {
    pub fn running(&self) -> bool {
        self.active
    }
    pub fn total_s(&self) -> f64 {
        self.total_s
    }
    pub fn timer_id(&self) -> i64 {
        self.id
    }

    /// The Note tab is on screen: tick four times a second (its ring eases over .26 s).
    pub fn set_visible(&mut self, v: bool) {
        self.visible = v;
    }

    fn remaining_s(&self, now: i64) -> f64 {
        if self.active { (self.due_at.saturating_sub(now) as f64 / 1000.0).max(0.0) } else { 0.0 }
    }

    fn paint(&mut self, now: i64) -> Vec<LiveCmd> {
        let run = self.active;
        let rem = self.remaining_s(now);
        let clock = clock_ceil(rem);
        let sec = if run { rem.ceil() as i64 } else { -1 };
        if sec == self.live_sec {
            return Vec::new();
        }
        self.live_sec = sec;
        if !run {
            return vec![LiveCmd::clear("timer")];
        }
        // key = clock length: the chip is built once per run (and when the clock gains / loses a digit), the seconds are edited in place
        let row = RowSpec::new("timer", TIMER_PRIORITY)
            .key(format!("run{}", clock.len()))
            .anim(Anim::Fade, Anim::None, Anim::Fade)
            .with(Elem::icon(IconName::Timer, 16.0))
            .with(Elem::text(clock, TextStyle::Metric));
        vec![LiveCmd::set(row)]
    }

    fn apply(&mut self, state: Option<(i64, i64, f64)>, now: i64) -> Vec<LiveCmd> {
        match state {
            Some((id, due_at, total_s)) => {
                self.active = true;
                self.id = id;
                self.due_at = due_at;
                self.total_s = total_s;
            }
            None => {
                self.active = false;
                self.id = 0;
                self.due_at = 0;
                self.total_s = 0.0;
            }
        }
        self.zero_synced = false;
        self.paint(now)
    }

    fn on_due(&mut self, env: &dyn Env, p: &Value, now: i64) -> Vec<LiveCmd> {
        if p.is_null() {
            return Vec::new();
        }
        let cards = env.live_on("reminders"); // off = no card, no edge reaction
        let mut out = Vec::new();
        if cards {
            out.push(LiveCmd::react("remind", Preset::Attention, Some(5000), None)); // timer or reminder fired
        }
        let kind = text_or_empty(p.get("kind"));
        if kind == "timer" {
            out.extend(self.apply(None, now));
            if cards {
                let card = CardSpec::text("timer-done", 20000, env.tr("Таймер закончился", "Timer finished"), None)
                    .icon(IconName::Bell)
                    .actions(vec![
                        CardAction::new(env.tr("Ещё раз", "Again"), ActionKind::Normal, Action::TimerAgain),
                        CardAction::close(env.tr("Ок", "OK")),
                    ]);
                out.push(LiveCmd::Toast(card));
            }
        } else if cards {
            let rule = text_or_empty(p.get("repeat"));
            let repeating = !rule.is_empty() && rule != "none";
            let late = truthy(p.get("late"));
            let sub = if late { env.tr("пропущено", "missed") } else if repeating { fmt_repeat(env, &rule) } else { String::new() };
            let id = p.get("id").cloned().unwrap_or(Value::Null);
            let card = CardSpec::text(
                format!("remind-{}", display(p.get("id"))),
                30000,
                text_or_empty(p.get("text")),
                if sub.is_empty() { None } else { Some(sub) },
            )
            .icon(IconName::Bell)
            .actions(vec![
                CardAction::new(
                    env.tr(&format!("Отложить {SNOOZE_MIN} мин"), &format!("Snooze {SNOOZE_MIN} min")),
                    ActionKind::Normal,
                    Action::invoke("remind_snooze", json!({ "id": id, "minutes": SNOOZE_MIN })),
                ),
                CardAction::new(env.tr("Готово", "Done"), ActionKind::Normal, if repeating { Action::Noop } else { Action::invoke("remind_done", json!({ "id": id })) }),
            ]);
            out.push(LiveCmd::Toast(card));
        }
        out
    }
}

impl Provider for TimerLive {
    fn name(&self) -> &'static str {
        "timer"
    }
    fn events(&self) -> &'static [&'static str] {
        &["timer:tick", "timer:state", "remind:due", "remind:changed", "pill:hide", "pill:show"]
    }
    fn boot(&self) -> Vec<Query> {
        vec![Query::new("timer_state", "timer:state")]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "timer:tick" => {
                if p.is_null() || !truthy(p.get("id")) {
                    return Vec::new();
                }
                let rem = num_or0(p.get("remaining_ms")).trunc() as i64;
                self.apply(Some((num_or0(p.get("id")).trunc() as i64, now.saturating_add(rem), num_or0(p.get("total_s")))), now)
            }
            // the answer to `timer_state`; a failed call (null) leaves the state alone
            "timer:state" => {
                if p.is_null() {
                    return Vec::new();
                }
                if truthy(p.get("active")) {
                    self.apply(Some((num_or0(p.get("id")).trunc() as i64, num_or0(p.get("due_at")).trunc() as i64, num_or0(p.get("total_s")))), now)
                } else {
                    self.apply(None, now)
                }
            }
            "remind:changed" => vec![LiveCmd::Query(Query::new("timer_state", "timer:state"))],
            "remind:due" => self.on_due(env, p, now),
            "pill:hide" => {
                self.hidden = true; // nothing while the pill is hidden
                Vec::new()
            }
            "pill:show" => {
                self.hidden = false;
                self.paint(now)
            }
            _ => Vec::new(),
        }
    }
    fn on_tick(&mut self, _env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if !self.active || self.hidden {
            return Vec::new();
        }
        let mut out = self.paint(now);
        // Rust fires timer:done; resync if the event was missed
        if self.remaining_s(now) <= 0.0 && !self.zero_synced {
            self.zero_synced = true;
            let mut q = Query::new("timer_state", "timer:state");
            q.delay_ms = SYNC_AFTER_ZERO_MS;
            out.push(LiveCmd::Query(q));
        }
        out
    }
    /// 4 Hz while the Note tab shows the ring, else once per displayed second (just after the boundary); nothing while hidden.
    fn tick_ms(&self, now: i64) -> Option<u32> {
        if !self.active || self.hidden {
            return None;
        }
        let left = self.due_at.saturating_sub(now);
        Some(if self.visible || left <= 0 {
            250
        } else {
            let r = left % 1000;
            ((if r == 0 { 1000 } else { r }) + 20) as u32
        })
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    const T0: i64 = 1_700_000_000_000;

    fn tick(t: &mut TimerLive, env: &MemEnv, remaining_ms: i64, now: i64) -> Vec<LiveCmd> {
        t.on_event(env, "timer:tick", &json!({"id": 4, "remaining_ms": remaining_ms, "total_s": 300}), now)
    }
    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no row in {cmds:?}"))
    }
    fn clock(r: &RowSpec) -> String {
        match &r.elements[1] {
            Elem::Text { text, .. } => text.clone(),
            o => panic!("{o:?}"),
        }
    }
    fn toast(cmds: &[LiveCmd]) -> &CardSpec {
        cmds.iter().find_map(|c| if let LiveCmd::Toast(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no toast in {cmds:?}"))
    }
    fn ts(c: &CardSpec) -> (String, Option<String>) {
        match &c.body {
            CardBody::Text { title, sub } => (title.clone(), sub.clone()),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn running_timer_row() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        let cmds = tick(&mut t, &env, 299_400, T0);
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms, r.key.as_str()), ("timer", 60, None, "run4"));
        assert_eq!(clock(r), "5:00", "rounded up like the Note tab ring");
        assert!(matches!(r.elements[0], Elem::Icon { name: IconName::Timer, .. }));
        assert!(t.running());
        assert_eq!((t.total_s(), t.timer_id()), (300.0, 4));
    }

    #[test]
    fn row_is_edited_once_per_second_with_the_same_key() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        let k1 = row(&tick(&mut t, &env, 12_000, T0)).key.clone();
        assert!(t.on_tick(&env, T0 + 300).is_empty(), "same displayed second");
        let c = t.on_tick(&env, T0 + 1100);
        assert_eq!(clock(row(&c)), "0:11");
        assert_eq!(row(&c).key, k1);
        // the clock gains no digit here, but loses one at 10 s: a new key (the chip is re-measured)
        let c = t.on_tick(&env, T0 + 2100);
        assert_eq!(clock(row(&c)), "0:10");
        let c = t.on_tick(&env, T0 + 3100);
        assert_eq!(clock(row(&c)), "0:09");
        assert_eq!(row(&c).key, "run4");
    }

    #[test]
    fn long_timers_use_hours_and_a_longer_key() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        let r = row(&tick(&mut t, &env, 3_725_000, T0)).clone();
        assert_eq!((clock(&r).as_str(), r.key.as_str()), ("1:02:05", "run7"));
    }

    #[test]
    fn state_answer_and_inactive() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        let cmds = t.on_event(&env, "timer:state", &json!({"active": true, "id": 9, "due_at": T0 + 90_000, "total_s": 120}), T0);
        assert_eq!(clock(row(&cmds)), "1:30");
        let cmds = t.on_event(&env, "timer:state", &json!({"active": false}), T0 + 10);
        assert_eq!(cmds, vec![LiveCmd::clear("timer")]);
        assert!(!t.running());
        assert!(t.on_event(&env, "timer:state", &json!({"active": false}), T0 + 20).is_empty(), "already clear");
        // a failed call keeps the timer
        tick(&mut t, &env, 5000, T0);
        assert!(t.on_event(&env, "timer:state", &Value::Null, T0 + 30).is_empty());
        assert!(t.running());
        assert_eq!(t.boot(), vec![Query::new("timer_state", "timer:state")]);
    }

    #[test]
    fn tick_without_an_id_is_ignored() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        assert!(t.on_event(&env, "timer:tick", &json!({"remaining_ms": 1000}), T0).is_empty());
        assert!(t.on_event(&env, "timer:tick", &Value::Null, T0).is_empty());
        assert!(!t.running());
    }

    #[test]
    fn tick_cadence() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        assert_eq!(t.tick_ms(T0), None);
        tick(&mut t, &env, 10_300, T0);
        assert_eq!(t.tick_ms(T0), Some(320), "300 ms to the next second boundary + 20");
        assert_eq!(t.tick_ms(T0 + 300), Some(1020), "on the boundary: a full second");
        t.set_visible(true);
        assert_eq!(t.tick_ms(T0), Some(250), "the Note tab animates a ring");
        t.set_visible(false);
        assert_eq!(t.tick_ms(T0 + 20_000), Some(250), "overdue: 4 Hz until Rust answers");
        t.on_event(&env, "pill:hide", &Value::Null, T0);
        assert_eq!(t.tick_ms(T0), None);
        assert!(t.on_tick(&env, T0 + 500).is_empty());
        let c = t.on_event(&env, "pill:show", &Value::Null, T0 + 5000);
        assert_eq!(clock(row(&c)), "0:06");
    }

    #[test]
    fn overdue_timer_resyncs_once() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        tick(&mut t, &env, 1000, T0);
        let c = t.on_tick(&env, T0 + 1500);
        let q = c.iter().find_map(|c| if let LiveCmd::Query(q) = c { Some(q.clone()) } else { None }).expect("resync query");
        assert_eq!((q.cmd, q.as_event, q.delay_ms), ("timer_state", "timer:state", 1500));
        assert_eq!(clock(row(&c)), "0:00");
        assert!(!t.on_tick(&env, T0 + 1750).iter().any(|c| matches!(c, LiveCmd::Query(_))), "only once");
        // a fresh tick from Rust re-arms the check
        tick(&mut t, &env, 500, T0 + 2000);
        assert!(t.on_tick(&env, T0 + 2600).iter().any(|c| matches!(c, LiveCmd::Query(_))));
    }

    #[test]
    fn remind_changed_rereads_the_timer() {
        let mut t = TimerLive::default();
        assert_eq!(t.on_event(&MemEnv::default(), "remind:changed", &Value::Null, T0), vec![LiveCmd::Query(Query::new("timer_state", "timer:state"))]);
    }

    #[test]
    fn timer_finished_card() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        tick(&mut t, &env, 1000, T0);
        let cmds = t.on_event(&env, "remind:due", &json!({"id": 4, "text": "", "kind": "timer", "late": false, "repeat": "none", "due_at": T0}), T0 + 1000);
        assert!(cmds.contains(&LiveCmd::react("remind", Preset::Attention, Some(5000), None)));
        assert!(cmds.contains(&LiveCmd::clear("timer")), "the chip goes");
        assert!(!t.running());
        let c = toast(&cmds);
        assert_eq!((c.id.as_str(), c.ttl_ms, c.icon.clone()), ("timer-done", 20000, Some(CardIcon::Name(IconName::Bell))));
        assert!(c.quiet_exempt());
        assert_eq!(ts(c), ("Таймер закончился".to_string(), None));
        assert_eq!(c.actions[0].action, Action::TimerAgain);
        assert_eq!((c.actions[0].label.as_str(), c.actions[1].label.as_str()), ("Ещё раз", "Ок"));
        let cmds = t.on_event(&MemEnv::en(), "remind:due", &json!({"id": 4, "kind": "timer"}), T0);
        assert_eq!(ts(toast(&cmds)).0, "Timer finished");
        assert_eq!((toast(&cmds).actions[0].label.as_str(), toast(&cmds).actions[1].label.as_str()), ("Again", "OK"));
    }

    #[test]
    fn reminder_card_plain_late_and_repeating() {
        let mut t = TimerLive::default();
        let env = MemEnv::default();
        let cmds = t.on_event(&env, "remind:due", &json!({"id": 12, "text": "Позвонить маме", "kind": "reminder", "late": false, "repeat": "none", "due_at": T0}), T0);
        let c = toast(&cmds);
        assert_eq!((c.id.as_str(), c.ttl_ms), ("remind-12", 30000));
        assert!(c.quiet_exempt());
        assert_eq!(ts(c), ("Позвонить маме".to_string(), None));
        assert_eq!(c.actions[0].label, "Отложить 10 мин");
        assert_eq!(c.actions[0].action, Action::invoke("remind_snooze", json!({"id": 12, "minutes": 10})));
        assert_eq!(c.actions[1].action, Action::invoke("remind_done", json!({"id": 12})));
        // late
        let cmds = t.on_event(&MemEnv::en(), "remind:due", &json!({"id": 12, "text": "Call", "kind": "reminder", "late": true, "repeat": "daily"}), T0);
        assert_eq!(ts(toast(&cmds)), ("Call".to_string(), Some("missed".to_string())), "late beats the repeat hint");
        assert_eq!(toast(&cmds).actions[0].label, "Snooze 10 min");
        // repeating: Done does nothing (the rule re-arms the reminder)
        let cmds = t.on_event(&env, "remind:due", &json!({"id": 13, "text": "Зарядка", "kind": "reminder", "late": false, "repeat": "daily"}), T0);
        assert_eq!(ts(toast(&cmds)).1, Some("каждый день".to_string()));
        assert_eq!(toast(&cmds).actions[1].action, Action::Noop);
        let cmds = t.on_event(&env, "remind:due", &json!({"id": 13, "text": "x", "kind": "reminder", "late": true}), T0);
        assert_eq!(ts(toast(&cmds)).1, Some("пропущено".to_string()));
    }

    #[test]
    fn reminders_switch_silences_cards_but_the_timer_still_resets() {
        let mut t = TimerLive::default();
        let mut env = MemEnv::default();
        env.set("live.reminders", json!(false));
        tick(&mut t, &env, 1000, T0);
        let cmds = t.on_event(&env, "remind:due", &json!({"id": 4, "kind": "timer"}), T0 + 1000);
        assert_eq!(cmds, vec![LiveCmd::clear("timer")]);
        assert!(t.on_event(&env, "remind:due", &json!({"id": 5, "text": "x", "kind": "reminder"}), T0).is_empty());
        assert!(t.on_event(&env, "remind:due", &Value::Null, T0).is_empty());
    }

    #[test]
    fn repeat_rules() {
        let env = MemEnv::default();
        let en = MemEnv::en();
        assert_eq!(fmt_repeat(&env, ""), "");
        assert_eq!(fmt_repeat(&env, "none"), "");
        assert_eq!(fmt_repeat(&env, "weekly"), "каждую неделю");
        assert_eq!(fmt_repeat(&en, "monthly"), "monthly");
        assert_eq!(fmt_repeat(&env, "every:2880"), "каждые 2 дн.");
        assert_eq!(fmt_repeat(&en, "every:2880"), "every 2 d");
        assert_eq!(fmt_repeat(&env, "every:180"), "каждые 3 ч");
        assert_eq!(fmt_repeat(&en, "every:180"), "every 3 h");
        assert_eq!(fmt_repeat(&env, "every:45"), "каждые 45 мин");
        assert_eq!(fmt_repeat(&en, "every:45"), "every 45 min");
        assert_eq!(fmt_repeat(&en, "mystery"), "every 0 min", "an unparsable rule falls through like the JS");
    }
}
