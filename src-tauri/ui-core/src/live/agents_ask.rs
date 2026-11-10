//! The "answer Claude Code from the island" cards (backend: `src/agent_bridge.rs`, setting `agents.answerFromIsland`, default off).
//!
//! * `agents:ask {id, kind, toolName, summary, question, options, project, ...}` -> an amber card: a permission request has Deny / Allow,
//!   a question has one button per option (2-3). A button calls `agents_answer {id, choice}` (`allow`, `deny`, `opt:N`) and dismisses the
//!   card (the island plays the leave; the pressed button stays lit while it goes). Quiet hours do not hold it back: the agent is blocked.
//! * `agents:ask-gone {id}` (the user answered in the terminal) -> the same card id is replaced by a short "answered in the terminal" note.
//!
//! The card lives a little less than the backend waits ([`ASK_CARD_MS`] < `agent_bridge::ANSWER_MS`), so a card that nobody answered is
//! gone before the hook falls through to the terminal. PRIVACY: everything shown is plain text from the hook payload (cleaned by the backend).

use serde_json::{json, Value};

use super::env::Env;
use super::events::edge;
use super::model::*;
use super::util::*;

pub const ASK_CARD_MS: u32 = 118_000;
const NOTE_MS: u32 = 1800;

pub fn card_id(id: &str) -> String {
    format!("agent-ask:{id}")
}

fn answer(id: &str, choice: &str) -> Action {
    Action::invoke("agents_answer", json!({ "id": id, "choice": choice }))
}

pub fn ask(env: &dyn Env, p: &Value) -> Vec<LiveCmd> {
    let id = text_or_empty(p.get("id"));
    // the backend only runs the bridge while the switch is on; a stray event with it off is ignored all the same
    if id.is_empty() || !env.live_on("agents") || !env.setting_true("agents.answerFromIsland") {
        return Vec::new();
    }
    let proj = text_or_empty(p.get("project"));
    let at = if proj.is_empty() { String::new() } else { format!(" · {proj}") };
    let tool = cut(&text_or_empty(p.get("toolName")), 24);
    let card = if text_or_empty(p.get("kind")) == "question" {
        let q = cut(&text_or_empty(p.get("question")), 70);
        let actions: Vec<CardAction> = p
            .get("options")
            .and_then(Value::as_array)
            .map(|o| o.iter().take(3).enumerate().map(|(i, l)| CardAction::new(cut(&text(Some(l)), 16), ActionKind::White, answer(&id, &format!("opt:{i}")))).collect())
            .unwrap_or_default();
        if actions.len() < 2 {
            return Vec::new();
        }
        CardSpec::text(card_id(&id), ASK_CARD_MS, format!("{q}{at}"), Some(env.tr("Claude Code спрашивает", "Claude Code asks")))
            .icon(IconName::Sparkles)
            .actions(actions)
    } else {
        let sub = cut(&text_or_empty(p.get("summary")), 90);
        let icon = match tool.as_str() {
            "Bash" | "PowerShell" => IconName::Terminal,
            "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "Read" => IconName::File,
            _ => IconName::Warn,
        };
        let title = if tool.is_empty() { env.tr("Разрешить действие?", "Allow this action?") } else { env.tr(&format!("Разрешить {tool}?"), &format!("Allow {tool}?")) };
        CardSpec::text(card_id(&id), ASK_CARD_MS, format!("{title}{at}"), Some(if sub.is_empty() { env.tr("Claude Code ждёт ответа", "Claude Code is waiting") } else { sub }))
            .icon(icon)
            .actions(vec![
                CardAction::new(env.tr("Отклонить", "Deny"), ActionKind::Normal, answer(&id, "deny")),
                CardAction::new(env.tr("Разрешить", "Allow"), ActionKind::White, answer(&id, "allow")),
            ])
    };
    let mut card = card.tone(Tone::Amber);
    card.urgent = true;
    let mut out = edge(env, 0.9);
    out.push(LiveCmd::Toast(card));
    out
}

pub fn gone(env: &dyn Env, p: &Value) -> Vec<LiveCmd> {
    let id = text_or_empty(p.get("id"));
    if id.is_empty() {
        return Vec::new();
    }
    let mut card = CardSpec::text(card_id(&id), NOTE_MS, env.tr("Отвечено в терминале", "Answered in the terminal"), None).icon(IconName::Check);
    card.urgent = true;
    vec![LiveCmd::Toast(card)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    fn on() -> MemEnv {
        let mut e = MemEnv::en();
        e.set("agents.answerFromIsland", json!(true));
        e
    }
    fn toast(c: &[LiveCmd]) -> &CardSpec {
        c.iter().find_map(|x| if let LiveCmd::Toast(t) = x { Some(t) } else { None }).expect("a card")
    }

    #[test]
    fn permission_card_has_deny_and_allow_that_answer_by_id() {
        let c = ask(&on(), &json!({ "id": "7", "kind": "permission", "toolName": "Bash", "summary": "git push --force", "project": "app" }));
        assert_eq!(c[0], LiveCmd::EdgePulse(0.9));
        let t = toast(&c);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone, t.urgent, t.icon.clone()), ("agent-ask:7", ASK_CARD_MS, Tone::Amber, true, Some(CardIcon::Name(IconName::Terminal))));
        assert_eq!(t.body, CardBody::Text { title: "Allow Bash? · app".into(), sub: Some("git push --force".into()) });
        let b: Vec<(&str, ActionKind)> = t.actions.iter().map(|a| (a.label.as_str(), a.kind)).collect();
        assert_eq!(b, [("Deny", ActionKind::Normal), ("Allow", ActionKind::White)]);
        assert_eq!(t.actions[0].action, Action::invoke("agents_answer", json!({ "id": "7", "choice": "deny" })));
        assert_eq!(t.actions[1].action, Action::invoke("agents_answer", json!({ "id": "7", "choice": "allow" })));
        let mut ru = on();
        ru.ru = true;
        let c = ask(&ru, &json!({ "id": "7", "toolName": "Edit", "summary": "" }));
        let t = toast(&c);
        assert_eq!(t.body, CardBody::Text { title: "Разрешить Edit?".into(), sub: Some("Claude Code ждёт ответа".into()) });
        assert_eq!(t.actions[0].label, "Отклонить");
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::File)));
    }

    #[test]
    fn question_card_has_one_button_per_option() {
        let c = ask(&on(), &json!({ "id": "9", "kind": "question", "question": "Which database should we use?", "options": ["SQLite", "Postgres", "A very long option label"] }));
        let t = toast(&c);
        assert_eq!(t.body, CardBody::Text { title: "Which database should we use?".into(), sub: Some("Claude Code asks".into()) });
        assert_eq!(t.actions.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["SQLite", "Postgres", "A very long opt…"]);
        assert_eq!(t.actions[2].action, Action::invoke("agents_answer", json!({ "id": "9", "choice": "opt:2" })));
        assert!(ask(&on(), &json!({ "id": "9", "kind": "question", "question": "q", "options": ["only"] })).is_empty());
    }

    #[test]
    fn nothing_shows_when_the_switch_is_off_or_the_id_is_missing() {
        let p = json!({ "id": "1", "kind": "permission", "toolName": "Bash" });
        assert!(ask(&MemEnv::en(), &p).is_empty());
        let mut off = on();
        off.set("live.agents", json!(false));
        assert!(ask(&off, &p).is_empty());
        assert!(ask(&on(), &json!({ "kind": "permission" })).is_empty());
        let mut quiet = on();
        quiet.quiet = true;
        let c = ask(&quiet, &p);
        assert!(!c.contains(&LiveCmd::EdgePulse(0.9)) && toast(&c).urgent, "quiet hours: no ripple, but the card still comes");
    }

    #[test]
    fn a_terminal_answer_replaces_the_card_by_id() {
        let c = gone(&on(), &json!({ "id": "7" }));
        let LiveCmd::Toast(t) = &c[0] else { panic!() };
        assert_eq!((t.id.as_str(), t.ttl_ms, t.actions.len()), ("agent-ask:7", 1800, 0));
        assert!(gone(&on(), &json!({})).is_empty());
    }
}
