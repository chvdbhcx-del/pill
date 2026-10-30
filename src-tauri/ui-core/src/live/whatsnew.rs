//! The "What's new" card after an update (port of whatsnew-live.js; Rust side whatsnew.rs).
//!
//! `whatsnew_check` stores the running version and answers `{ version, notes }` only once, right after an update (null on a fresh
//! install, an unchanged version or with the switch off), so a reload cannot show the card twice. The host asks 4 s after start
//! (the [`Provider::boot`] query carries `delay_ms: 4000`, "let the island settle first") and feeds the answer back as `whatsnew:result`.

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const CHECK_DELAY_MS: u32 = 4000;

#[derive(Debug, Default)]
pub struct WhatsNewLive;

impl Provider for WhatsNewLive {
    fn name(&self) -> &'static str {
        "whatsnew"
    }
    fn events(&self) -> &'static [&'static str] {
        &["whatsnew:result"]
    }
    fn boot(&self) -> Vec<Query> {
        let mut q = Query::new("whatsnew_check", "whatsnew:result");
        q.delay_ms = CHECK_DELAY_MS;
        vec![q]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, r: &Value, _now: i64) -> Vec<LiveCmd> {
        if r.is_null() || !truthy(r.get("version")) {
            return Vec::new();
        }
        let notes: Vec<String> = r.get("notes").and_then(Value::as_array).map(|a| a.iter().map(|n| if n.is_null() { String::new() } else { display(Some(n)) }).collect()).unwrap_or_default();
        let notes = notes.join(" · ");
        let card = CardSpec::text(
            "whatsnew",
            14000,
            format!("Pill {}", text(r.get("version"))),
            Some(if notes.is_empty() { env.tr("Приложение обновлено", "Pill was updated") } else { notes }),
        )
        .icon(IconName::Sparkles)
        .actions(vec![
            CardAction::close(env.tr("Закрыть", "Close")),
            CardAction::new(env.tr("Все изменения", "All changes"), ActionKind::White, Action::invoke("settings_open", json!({ "page": "updates" }))),
        ]);
        vec![LiveCmd::Toast(card)]
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

    fn card(cmds: &[LiveCmd]) -> &CardSpec {
        match cmds {
            [LiveCmd::Toast(c)] => c,
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn boot_waits_four_seconds() {
        let b = WhatsNewLive.boot();
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].cmd, b[0].as_event, b[0].delay_ms), ("whatsnew_check", "whatsnew:result", 4000));
    }

    #[test]
    fn card_with_notes() {
        let mut w = WhatsNewLive;
        let cmds = w.on_event(&MemEnv::default(), "whatsnew:result", &json!({"version": "0.5.2", "notes": ["Новые виджеты", "Исправления"]}), T0);
        let c = card(&cmds);
        assert_eq!((c.id.as_str(), c.ttl_ms, c.tone), ("whatsnew", 14000, Tone::Neutral));
        assert_eq!(c.icon, Some(CardIcon::Name(IconName::Sparkles)));
        assert_eq!(c.body, CardBody::Text { title: "Pill 0.5.2".into(), sub: Some("Новые виджеты · Исправления".into()) });
        assert_eq!((c.actions[0].label.as_str(), c.actions[0].kind), ("Закрыть", ActionKind::Normal));
        assert_eq!((c.actions[1].label.as_str(), c.actions[1].kind), ("Все изменения", ActionKind::White));
        assert_eq!(c.actions[1].action, Action::invoke("settings_open", json!({"page": "updates"})));
    }

    #[test]
    fn card_without_notes_en() {
        let mut w = WhatsNewLive;
        let en = MemEnv::en();
        for payload in [json!({"version": "1.0"}), json!({"version": "1.0", "notes": []}), json!({"version": "1.0", "notes": null})] {
            let cmds = w.on_event(&en, "whatsnew:result", &payload, T0);
            assert_eq!(card(&cmds).body, CardBody::Text { title: "Pill 1.0".into(), sub: Some("Pill was updated".into()) });
        }
        let cmds = w.on_event(&en, "whatsnew:result", &json!({"version": "1.0"}), T0);
        assert_eq!((card(&cmds).actions[0].label.as_str(), card(&cmds).actions[1].label.as_str()), ("Close", "All changes"));
    }

    #[test]
    fn nothing_to_show() {
        let mut w = WhatsNewLive;
        let env = MemEnv::default();
        assert!(w.on_event(&env, "whatsnew:result", &Value::Null, T0).is_empty());
        assert!(w.on_event(&env, "whatsnew:result", &json!({}), T0).is_empty());
        assert!(w.on_event(&env, "whatsnew:result", &json!({"version": ""}), T0).is_empty());
        // a numeric version still prints
        let cmds = w.on_event(&env, "whatsnew:result", &json!({"version": 2}), T0);
        assert_eq!(card(&cmds).body, CardBody::Text { title: "Pill 2".into(), sub: Some("Приложение обновлено".into()) });
    }
}
