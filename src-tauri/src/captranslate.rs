// "Translate" button of the screen-text card (capture.rs `finish_text`). It runs only when the user presses it: the recognised text goes
// through the same AI action as the Text tab (`aiactions::ai_transform`, "translate": Cyrillic -> English, anything else -> Russian) to
// the endpoint from Settings > AI, and the answer comes back as a `capture:done {kind: "translate"}` card with a Copy button.
// No AI endpoint (switch off, no address, no model) -> a card that leads to the AI setup (`open_ai_setup`). Nothing is stored or logged.
use crate::ui::bus;
use crate::{aiactions, net, settings};
use serde_json::{json, Value};

/// Why the AI cannot be used right now, as the error code of the card; `None` = configured.
pub fn ai_state(switch_on: bool, base_url: &str, model: &str) -> Option<&'static str> {
    if !switch_on {
        Some("net_off")
    } else if base_url.trim().is_empty() {
        Some("no_endpoint")
    } else if model.trim().is_empty() {
        Some("no_model")
    } else {
        None
    }
}

/// The card payload for one `ai_transform` answer (`None` = a newer request replaced this one: show nothing).
pub fn result_event(r: &Value) -> Option<Value> {
    if r.get("ok").and_then(Value::as_bool) == Some(true) {
        return Some(json!({ "kind": "translate", "text": r.get("text").and_then(Value::as_str).unwrap_or("") }));
    }
    let e = r.get("error").and_then(Value::as_str).unwrap_or("");
    match e {
        "cancelled" => None,
        "net_off" | "no_endpoint" | "no_model" => Some(json!({ "kind": "translate", "error": e })),
        _ => Some(json!({ "kind": "translate", "error": "failed", "message": e })),
    }
}

/// Starts the translation of `text` on its own thread (a model can take a while); the cards are events, this returns at once.
pub fn run(text: String) {
    if text.trim().is_empty() {
        return;
    }
    if let Some(code) = ai_state(net::enabled(net::Feature::Ai), &settings::get_str("ai.baseUrl", ""), &settings::get_str("ai.model", "")) {
        let _ = bus::emit("capture:done", json!({ "kind": "translate", "error": code }));
        return;
    }
    let _ = bus::emit("capture:done", json!({ "kind": "translate", "pending": true }));
    std::thread::spawn(move || {
        let r = aiactions::ai_transform(text, "translate".into(), None);
        if let Some(ev) = result_event(&r) {
            let _ = bus::emit("capture:done", ev);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ai_must_be_switched_on_addressed_and_have_a_model() {
        assert_eq!(ai_state(false, "http://127.0.0.1:11434/v1", "m"), Some("net_off"));
        assert_eq!(ai_state(true, "  ", "m"), Some("no_endpoint"));
        assert_eq!(ai_state(true, "http://127.0.0.1:11434/v1", ""), Some("no_model"));
        assert_eq!(ai_state(true, "http://127.0.0.1:11434/v1", "qwen"), None);
    }

    #[test]
    fn answers_become_cards() {
        assert_eq!(result_event(&json!({ "ok": true, "text": "Привет" })), Some(json!({ "kind": "translate", "text": "Привет" })));
        assert_eq!(result_event(&json!({ "ok": false, "error": "cancelled" })), None);
        assert_eq!(result_event(&json!({ "ok": false, "error": "no_endpoint" })), Some(json!({ "kind": "translate", "error": "no_endpoint" })));
        assert_eq!(result_event(&json!({ "ok": false, "error": "timeout" })), Some(json!({ "kind": "translate", "error": "failed", "message": "timeout" })));
    }
}
