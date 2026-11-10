//! The armed "shut down / restart in N minutes" countdown in the collapsed pill, with a cancel button (Rust side:
//! `command/power_timer.rs`).
//!
//! * `power:timer {armed, kind: shutdown | restart, due_at}` (also the `power_timer_state` boot answer) -> row `powertimer`
//!   (priority 65, above the plain timer): an amber outline, the action, the time left, and an X button that calls `power_timer_cancel`.
//! * The row is built once per clock width and edited in place once per displayed second (the same key), like the timer row.
//! * Nothing ticks while it is not armed. It is a safety countdown, so quiet hours do not mute it.

use std::any::Any;

use serde_json::Value;

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const PRIORITY: i32 = 65;

#[derive(Debug)]
pub struct PowerTimerLive {
    armed: bool,
    restart: bool,
    due_at: i64,
    live_sec: i64,
    hidden: bool,
}

impl Default for PowerTimerLive {
    fn default() -> Self {
        PowerTimerLive { armed: false, restart: false, due_at: 0, live_sec: -1, hidden: false }
    }
}

impl PowerTimerLive {
    fn remaining_s(&self, now: i64) -> f64 {
        if self.armed { (self.due_at.saturating_sub(now) as f64 / 1000.0).max(0.0) } else { 0.0 }
    }

    fn paint(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        let sec = if self.armed { self.remaining_s(now).ceil() as i64 } else { -1 };
        if sec == self.live_sec {
            return Vec::new();
        }
        self.live_sec = sec;
        if !self.armed {
            return vec![LiveCmd::clear("powertimer")];
        }
        let clock = clock_ceil(self.remaining_s(now));
        let what = if self.restart { env.tr("Перезагрузка", "Restart") } else { env.tr("Выключение", "Shutdown") };
        let row = RowSpec::new("powertimer", PRIORITY)
            .key(format!("pt{}{}", clock.len(), self.restart as u8))
            .accent(Accent::AMBER)
            .tip(env.tr("Запланировано командой в строке", "Scheduled from the command bar"))
            .anim(Anim::Fade, Anim::None, Anim::Fade)
            .with(Elem::icon_c(IconName::Bolt, 16.0, ColorRole::Text))
            .with(Elem::text(what, TextStyle::Dim))
            .with(Elem::text(clock, TextStyle::Metric))
            .with(Elem::Button {
                action: Action::invoke0("power_timer_cancel"),
                label: None,
                icon: Some(IconName::X),
                style: ButtonStyle::IconRound,
                tip: env.tr("Отменить", "Cancel"),
            });
        vec![LiveCmd::set(row)]
    }

    fn apply(&mut self, env: &dyn Env, p: &Value, now: i64) -> Vec<LiveCmd> {
        if p.is_null() {
            return Vec::new(); // a failed call keeps what is shown
        }
        self.armed = truthy(p.get("armed")) && num_or0(p.get("due_at")) > 0.0;
        self.restart = text_or_empty(p.get("kind")) == "restart";
        self.due_at = if self.armed { num_or0(p.get("due_at")).trunc() as i64 } else { 0 };
        self.paint(env, now)
    }
}

impl Provider for PowerTimerLive {
    fn name(&self) -> &'static str {
        "powertimer"
    }
    fn events(&self) -> &'static [&'static str] {
        &["power:timer", "pill:hide", "pill:show"]
    }
    fn boot(&self) -> Vec<Query> {
        vec![Query::new("power_timer_state", "power:timer")]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "power:timer" => self.apply(env, p, now),
            "pill:hide" => {
                self.hidden = true;
                Vec::new()
            }
            "pill:show" => {
                self.hidden = false;
                self.live_sec = -2; // repaint the current second
                self.paint(env, now)
            }
            _ => Vec::new(),
        }
    }
    fn on_tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if !self.armed || self.hidden {
            return Vec::new();
        }
        self.paint(env, now)
    }
    /// Once per displayed second (just after the boundary) while armed and the clock has not run out; nothing otherwise.
    fn tick_ms(&self, now: i64) -> Option<u32> {
        if !self.armed || self.hidden {
            return None;
        }
        let left = self.due_at.saturating_sub(now);
        if left <= 0 {
            return None;
        }
        let r = left % 1000;
        Some(((if r == 0 { 1000 } else { r }) + 20) as u32)
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;
    use serde_json::json;

    const T0: i64 = 1_700_000_000_000;

    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no row in {cmds:?}"))
    }
    fn clock(r: &RowSpec) -> String {
        match &r.elements[2] {
            Elem::Text { text, .. } => text.clone(),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn an_armed_shutdown_shows_a_countdown_with_a_cancel_button() {
        let mut p = PowerTimerLive::default();
        let env = MemEnv::en();
        let cmds = p.on_event(&env, "power:timer", &json!({ "armed": true, "kind": "shutdown", "due_at": T0 + 1_800_000 }), T0);
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("powertimer", 65, None));
        assert_eq!(clock(r), "30:00");
        assert!(matches!(&r.elements[1], Elem::Text { text, .. } if text == "Shutdown"));
        match &r.elements[3] {
            Elem::Button { action, icon, style, tip, .. } => {
                assert_eq!(*action, Action::invoke0("power_timer_cancel"));
                assert_eq!((*icon, *style, tip.as_str()), (Some(IconName::X), ButtonStyle::IconRound, "Cancel"));
            }
            o => panic!("{o:?}"),
        }
        assert!(p.tick_ms(T0).is_some());
    }

    #[test]
    fn restart_reads_restart_and_russian_reads_russian() {
        let mut p = PowerTimerLive::default();
        let cmds = p.on_event(&MemEnv::default(), "power:timer", &json!({ "armed": true, "kind": "restart", "due_at": T0 + 90_000 }), T0);
        assert!(matches!(&row(&cmds).elements[1], Elem::Text { text, .. } if text == "Перезагрузка"));
        assert_eq!(clock(row(&cmds)), "1:30");
    }

    #[test]
    fn the_clock_is_edited_once_per_second_and_clears_when_cancelled() {
        let mut p = PowerTimerLive::default();
        let env = MemEnv::en();
        let k = row(&p.on_event(&env, "power:timer", &json!({ "armed": true, "kind": "shutdown", "due_at": T0 + 65_000 }), T0)).key.clone();
        assert!(p.on_tick(&env, T0 + 300).is_empty(), "same displayed second");
        let c = p.on_tick(&env, T0 + 1100);
        assert_eq!((clock(row(&c)).as_str(), row(&c).key.as_str()), ("1:04", k.as_str()));
        let gone = p.on_event(&env, "power:timer", &json!({ "armed": false }), T0 + 2000);
        assert_eq!(gone, vec![LiveCmd::clear("powertimer")]);
        assert_eq!(p.tick_ms(T0 + 2000), None, "nothing ticks when nothing is armed");
        assert!(p.on_event(&env, "power:timer", &json!({ "armed": false }), T0 + 2100).is_empty());
    }

    #[test]
    fn a_failed_boot_call_keeps_the_state_and_the_boot_query_is_declared() {
        let mut p = PowerTimerLive::default();
        let env = MemEnv::en();
        p.on_event(&env, "power:timer", &json!({ "armed": true, "kind": "shutdown", "due_at": T0 + 5000 }), T0);
        assert!(p.on_event(&env, "power:timer", &Value::Null, T0 + 10).is_empty());
        assert_eq!(p.boot(), vec![Query::new("power_timer_state", "power:timer")]);
        // an armed answer with no due time is not armed
        let mut q = PowerTimerLive::default();
        assert!(q.on_event(&env, "power:timer", &json!({ "armed": true }), T0).is_empty());
    }
}
