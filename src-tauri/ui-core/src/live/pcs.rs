//! Live cards of the PC link (docs/pc-link.md): "<PC> wants to pair" with Allow / Deny while this PC shows a token and the other one entered it, and
//! the short chip "Clipboard from <PC>" when a text of a paired PC was put on the clipboard. Neither card ever carries clipboard text or the token.
//!
//! * `pc:state {pair: {mode: show, state: request, peer: {name}}}` -> card `pc-pair` (45 s, shown even in quiet hours: the user just started it
//!   on the other PC); the same request does not pop up again on the next state event.
//! * `pc:clip {device, name}` -> card `pc-clip` for 2 s (one id: a second text replaces it).

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::{Action, ActionKind, CardAction, CardSpec, IconName, LiveCmd, Tone};
use super::util::cut;

#[derive(Debug, Default)]
pub struct PcLive {
    /// the PC whose request is on screen
    asked: Option<String>,
}

fn s(v: &Value) -> String {
    cut(v.as_str().unwrap_or("").trim(), 40)
}

impl Provider for PcLive {
    fn name(&self) -> &'static str {
        "pcs"
    }
    fn events(&self) -> &'static [&'static str] {
        &["pc:state", "pc:clip"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if name == "pc:clip" {
            let who = s(&p["name"]);
            let who = if who.is_empty() { env.tr("компьютера", "PC") } else { who };
            return vec![LiveCmd::Toast(CardSpec::text("pc-clip", 2000, format!("{} {}", env.tr("Буфер от", "Clipboard from"), who), None).icon(IconName::Clipboard))];
        }
        let pair = &p["pair"];
        let who = (pair["mode"] == "show" && pair["state"] == "request").then(|| s(&pair["peer"]["name"]));
        if who == self.asked {
            return Vec::new();
        }
        self.asked = who.clone();
        let Some(who) = who else { return Vec::new() };
        let mut c = CardSpec::text("pc-pair", 45_000, format!("{who} {}", env.tr("хочет подключиться", "wants to pair")), Some(env.tr("Общий буфер обмена с этим компьютером?", "Share the clipboard with this PC?"))).icon(IconName::Clipboard).tone(Tone::Accent);
        c.urgent = true;
        c.actions = vec![
            CardAction::new(env.tr("Отклонить", "Deny"), ActionKind::Normal, Action::invoke("pc_pair", json!({ "op": "deny" }))),
            CardAction::new(env.tr("Разрешить", "Allow"), ActionKind::White, Action::invoke("pc_pair", json!({ "op": "allow" }))),
        ];
        vec![LiveCmd::Toast(c)]
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;
    use crate::live::model::CardBody;

    fn toast(cmds: &[LiveCmd]) -> Option<&CardSpec> {
        cmds.iter().find_map(|c| if let LiveCmd::Toast(c) = c { Some(c) } else { None })
    }

    fn asking(name: &str) -> Value {
        json!({ "pair": { "mode": "show", "state": "request", "token": "PILL-X", "peer": { "name": name, "os": "Windows" } }, "peers": [] })
    }

    #[test]
    fn a_request_becomes_an_allow_deny_card_once() {
        let mut p = PcLive::default();
        let cmds = p.on_event(&MemEnv::en(), "pc:state", &asking("LAPTOP-X"), 0);
        let c = toast(&cmds).expect("a card");
        assert_eq!((c.id.as_str(), c.ttl_ms, c.urgent), ("pc-pair", 45_000, true));
        assert!(matches!(&c.body, CardBody::Text { title, sub: Some(_) } if title == "LAPTOP-X wants to pair"));
        let labels: Vec<_> = c.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Deny", "Allow"]);
        assert_eq!(c.actions[1].action, Action::invoke("pc_pair", json!({ "op": "allow" })));
        assert_eq!(c.actions[0].action, Action::invoke("pc_pair", json!({ "op": "deny" })));
        assert!(p.on_event(&MemEnv::en(), "pc:state", &asking("LAPTOP-X"), 1).is_empty(), "the same request does not pop up again");
        let ru = PcLive::default().on_event(&MemEnv::default(), "pc:state", &asking("LAPTOP-X"), 0);
        assert!(matches!(&toast(&ru).unwrap().body, CardBody::Text { title, .. } if title == "LAPTOP-X хочет подключиться"));
    }

    #[test]
    fn other_states_are_silent_and_a_new_request_pops_again() {
        let mut p = PcLive::default();
        let env = MemEnv::en();
        for st in [json!({ "pair": null }), json!({ "pair": { "mode": "enter", "state": "request", "peer": { "name": "X" } } }), json!({ "pair": { "mode": "show", "state": "waiting" } })] {
            assert!(p.on_event(&env, "pc:state", &st, 0).is_empty(), "{st}");
        }
        assert!(toast(&p.on_event(&env, "pc:state", &asking("A"), 0)).is_some());
        assert!(p.on_event(&env, "pc:state", &json!({ "pair": { "mode": "show", "state": "done" } }), 0).is_empty());
        assert!(toast(&p.on_event(&env, "pc:state", &asking("A"), 0)).is_some(), "after it ended, a new request is news");
    }

    #[test]
    fn a_text_from_a_pc_shows_a_two_second_chip_without_the_text() {
        let mut p = PcLive::default();
        let cmds = p.on_event(&MemEnv::en(), "pc:clip", &json!({ "device": "d1", "name": "LAPTOP-X", "text": "must never be read" }), 0);
        let c = toast(&cmds).expect("a chip");
        assert_eq!((c.id.as_str(), c.ttl_ms), ("pc-clip", 2000));
        assert!(matches!(&c.body, CardBody::Text { title, sub: None } if title == "Clipboard from LAPTOP-X"));
        let ru = p.on_event(&MemEnv::default(), "pc:clip", &json!({}), 0);
        assert!(matches!(&toast(&ru).unwrap().body, CardBody::Text { title, .. } if title == "Буфер от компьютера"));
    }
}
