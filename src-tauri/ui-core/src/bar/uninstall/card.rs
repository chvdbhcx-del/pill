//! The island card for `uninstall:ready` (live-ext.js): a finished step while the user is elsewhere. Rust sends the event to every window only when
//! the command bar is closed (the bar's own strip covers the open bar), so the island shows a toast with the app's name, a one-line result and
//! two buttons: «Открыть» (white; `uninstall_open_panel` brings the bar and the panel back where they were) and «Закрыть». Quiet hours apply:
//! this is not urgent. The card lives 12 s and replaces an earlier card of the same app (same `id`).

use super::payload::Ready;
use crate::bar::lang::{Lang, Tr};
use crate::bar::protocol::BackendCall;
use serde_json::Value;

/// Card lifetime.
pub const TTL_MS: u32 = 12_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// the default (neutral) look: still installed / error
    Neutral,
    Green,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAction {
    /// `uninstall_open_panel`
    Open,
    /// dismiss the card
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardButton {
    pub action: CardAction,
    pub label: &'static str,
    /// the white (primary) look
    pub white: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IslandCard {
    pub id: String,
    pub icon: &'static str,
    pub tone: Tone,
    pub ttl_ms: u32,
    pub title: String,
    pub sub: String,
    pub buttons: Vec<CardButton>,
    /// quiet hours may hold the card back
    pub urgent: bool,
}

impl IslandCard {
    /// Builds the card from the event payload `{ name, kind, count }`.
    pub fn from_event(payload: &Value, lang: Lang) -> IslandCard {
        IslandCard::new(&Ready::from_value(payload), lang)
    }

    pub fn new(r: &Ready, lang: Lang) -> IslandCard {
        let n = r.count;
        let sub = match r.kind.as_str() {
            "review" => format!("{}{}", ("Остатков к проверке: ", "Leftovers to review: ").tr(lang), n),
            "done" => {
                if n > 0 {
                    format!("{}{}", ("Готово, удалено: ", "Done, removed: ").tr(lang), n)
                } else {
                    ("Остатков не найдено", "No leftovers found").tr(lang).to_string()
                }
            }
            "still" => ("Приложение всё ещё установлено", "The app is still installed").tr(lang).to_string(),
            _ => ("Не получилось", "It did not work").tr(lang).to_string(),
        };
        let bad = r.kind == "error" || r.kind == "still";
        IslandCard {
            id: format!("uninstall:{}", r.name),
            icon: if bad { "trash" } else { "check" },
            tone: if bad { Tone::Neutral } else { Tone::Green },
            ttl_ms: TTL_MS,
            title: format!("{}{}", ("Удаление: ", "Uninstall of ").tr(lang), r.name),
            sub,
            buttons: vec![
                CardButton { action: CardAction::Open, label: ("Открыть", "Open").tr(lang), white: true },
                CardButton { action: CardAction::Close, label: ("Закрыть", "Close").tr(lang), white: false },
            ],
            urgent: false,
        }
    }

    /// The backend command a button runs (`Close` runs nothing).
    pub fn call_for(action: CardAction) -> Option<BackendCall> {
        match action {
            CardAction::Open => Some(BackendCall::bare("uninstall_open_panel")),
            CardAction::Close => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn card_texts_per_kind() {
        let c = |k: &str, n: u64| IslandCard::from_event(&json!({ "name": "qBittorrent", "kind": k, "count": n }), Lang::En);
        assert_eq!(c("review", 7).sub, "Leftovers to review: 7");
        assert_eq!(c("done", 3).sub, "Done, removed: 3");
        assert_eq!(c("done", 0).sub, "No leftovers found");
        assert_eq!(c("still", 0).sub, "The app is still installed");
        assert_eq!(c("error", 0).sub, "It did not work");
        assert_eq!(c("review", 7).title, "Uninstall of qBittorrent");
        let ru = IslandCard::from_event(&json!({ "name": "qBittorrent", "kind": "review", "count": 7 }), Lang::Ru);
        assert_eq!((ru.title.as_str(), ru.sub.as_str()), ("Удаление: qBittorrent", "Остатков к проверке: 7"));
        assert_eq!(ru.buttons[0].label, "Открыть");
    }

    #[test]
    fn tone_icon_ttl_and_buttons() {
        let good = IslandCard::from_event(&json!({ "name": "A", "kind": "done", "count": 1 }), Lang::En);
        assert_eq!((good.icon, good.tone, good.ttl_ms), ("check", Tone::Green, 12_000));
        let bad = IslandCard::from_event(&json!({ "name": "A", "kind": "error" }), Lang::En);
        assert_eq!((bad.icon, bad.tone), ("trash", Tone::Neutral));
        assert_eq!(IslandCard::from_event(&json!({ "name": "A", "kind": "still" }), Lang::En).icon, "trash");
        assert_eq!(good.id, "uninstall:A");
        assert!(good.buttons[0].white && !good.buttons[1].white);
        assert!(!good.urgent, "quiet hours apply");
        assert_eq!(IslandCard::call_for(CardAction::Open).unwrap().cmd, "uninstall_open_panel");
        assert!(IslandCard::call_for(CardAction::Close).is_none());
    }

    #[test]
    fn missing_fields_do_not_panic() {
        let c = IslandCard::from_event(&Value::Null, Lang::En);
        assert_eq!(c.sub, "It did not work");
        assert_eq!(c.title, "Uninstall of ");
    }
}
