//! Windows Focus Assist on the island: the cards held back while Windows says "do not disturb" and the user asked to respect it
//! (`notify.respectFocus`, off by default). The decision is `pill_ui_core::live::focus::verdict`; this keeps the state: what Windows said last
//! (`focus:changed {quiet}` from `src/focus.rs`), and the held cards. No window, no clock, no polling: the core calls it when a card arrives,
//! when Windows' state or the setting changes and when the island shows again.
use pill_ui_core::live::focus::{hold_push, HOLD_KEY};
use pill_ui_core::live::model::CardSpec;
use serde_json::{json, Value};

#[derive(Default)]
pub struct FocusHold {
    /// Windows' last answer (`SHQueryUserNotificationState` is one of the quiet states).
    pub sys_quiet: bool,
    held: Vec<(String, CardSpec)>,
}

/// `notify.respectFocus` (strict `true`; a missing key is off).
pub fn respect(settings: &Value) -> bool {
    settings.get("notify.respectFocus") == Some(&Value::Bool(true))
}

impl FocusHold {
    /// Windows' state as the backend reports it: `{quiet: bool}`.
    pub fn set_state(&mut self, payload: &Value) {
        self.sys_quiet = payload.get("quiet").and_then(Value::as_bool).unwrap_or(false);
    }

    /// Cards are being held right now: the user asked for it and Windows is quiet.
    pub fn active(&self, settings: &Value) -> bool {
        respect(settings) && self.sys_quiet
    }

    pub fn hold(&mut self, card: CardSpec) {
        hold_push(&mut self.held, card.id.clone(), card);
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Everything held, oldest first; the list is empty afterwards.
    pub fn take_all(&mut self) -> Vec<CardSpec> {
        std::mem::take(&mut self.held).into_iter().map(|(_, c)| c).collect()
    }

    /// Hands the "holding" flag to the providers (they only see settings); a no-op on a settings value that is not an object.
    pub fn publish(&self, settings: &mut Value, active: bool) {
        if let Some(o) = settings.as_object_mut() {
            o.insert(HOLD_KEY.to_string(), json!(active));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str) -> CardSpec {
        CardSpec::text(id, 4000, id, None)
    }

    #[test]
    fn holding_needs_the_switch_and_a_quiet_windows() {
        let mut f = FocusHold::default();
        f.set_state(&json!({"quiet": true, "state": "quiet-time"}));
        assert!(!f.active(&json!({})), "the switch is off by default");
        assert!(!f.active(&json!({"notify.respectFocus": false})));
        assert!(!f.active(&json!({"notify.respectFocus": "yes"})), "strict true");
        assert!(f.active(&json!({"notify.respectFocus": true})));
        f.set_state(&json!({"quiet": false}));
        assert!(!f.active(&json!({"notify.respectFocus": true})), "Windows is open again");
        f.set_state(&json!(null));
        assert!(!f.sys_quiet, "an unreadable answer is not quiet");
    }

    #[test]
    fn held_cards_come_back_oldest_first_and_once() {
        let mut f = FocusHold::default();
        for id in ["a", "b", "c"] {
            f.hold(card(id));
        }
        f.hold(card("a")); // the same card again: replaced, not doubled
        assert_eq!(f.len(), 3);
        let ids: Vec<String> = f.take_all().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert!(f.is_empty() && f.take_all().is_empty());
    }

    #[test]
    fn the_flag_for_providers_lives_in_the_core_copy_of_the_settings() {
        let f = FocusHold::default();
        let mut s = json!({"a": 1});
        f.publish(&mut s, true);
        assert_eq!(s.get(HOLD_KEY), Some(&json!(true)));
        f.publish(&mut s, false);
        assert_eq!(s.get(HOLD_KEY), Some(&json!(false)));
        let mut n = json!(null);
        f.publish(&mut n, true);
        assert!(n.is_null());
    }
}
