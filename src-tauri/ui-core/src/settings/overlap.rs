//! Switches that said the same thing twice are ONE switch now; this folds the keys of a settings file from 0.9.0 and before into it.
//!
//! | was | now |
//! |---|---|
//! | `module.weather` (tab + the same weather traffic) and `net.weather` (the internet, opt-in) | `net.weather` alone; `module.weather` off turns it off and hides the tab (`tabs.hidden`) |
//! | `module.phone` (the Phone tab, nothing else) | the Tabs page: `module.phone` off puts `phone` into `tabs.hidden` |
//! | `module.notifs` (tab) and `history.keep` (record the same history) | `module.notifs` alone; `history.keep` off turns it off |
//!
//! The rule is always the same: if EITHER old key was off, the one switch is off, so behaviour is what it was and nothing that was off
//! comes on (`net.weather` stays opt-in). The old key is removed from the file (it reads as its default, on, afterwards). The backend keeps
//! reading the old keys where it did (a default of "on" changes nothing); the Settings window has no row for them any more.
use serde_json::{json, Map, Value};

/// (old switch that is gone, the switch it folds into when it was off, the tab to hide with it)
const FOLDS: [(&str, Option<&str>, Option<&str>); 3] = [
    ("module.weather", Some("net.weather"), Some("weather")),
    ("module.phone", None, Some("phone")),
    ("history.keep", Some("module.notifs"), None),
];

fn is_off(cfg: &Map<String, Value>, key: &str) -> bool {
    cfg.get(key) == Some(&Value::Bool(false))
}

/// Fold the old keys of `cfg` into their one switch. Returns true when something changed (the caller saves the file). Idempotent.
pub fn fold(cfg: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    for (old, into, tab) in FOLDS {
        if !is_off(cfg, old) {
            continue;
        }
        cfg.remove(old);
        if let Some(k) = into {
            cfg.insert(k.to_string(), json!(false));
        }
        if let Some(t) = tab {
            let mut hidden = cfg.get("tabs.hidden").and_then(Value::as_array).cloned().unwrap_or_default();
            if !hidden.iter().any(|v| v.as_str() == Some(t)) {
                hidden.push(json!(t));
            }
            cfg.insert("tabs.hidden".to_string(), Value::Array(hidden));
        }
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn either_old_key_off_turns_the_one_switch_off() {
        // module off, forecast on: it did not work before, it is off now (and the tab stays hidden)
        let mut c = map(json!({ "module.weather": false, "net.weather": true }));
        assert!(fold(&mut c));
        assert_eq!((&c["net.weather"], &c["tabs.hidden"]), (&json!(false), &json!(["weather"])));
        assert!(!c.contains_key("module.weather"));
        // forecast off, module on: nothing to do
        let mut c = map(json!({ "module.weather": true, "net.weather": false }));
        assert!(!fold(&mut c));
        assert_eq!(c, map(json!({ "module.weather": true, "net.weather": false })));
        // both off: stays off
        let mut c = map(json!({ "module.weather": false, "net.weather": false }));
        fold(&mut c);
        assert_eq!(c["net.weather"], json!(false));
        // history: the same history switch, either one
        let mut c = map(json!({ "history.keep": false, "module.notifs": true, "history.mail": true }));
        assert!(fold(&mut c));
        assert_eq!((&c["module.notifs"], &c["history.mail"]), (&json!(false), &json!(true)));
        assert!(!c.contains_key("history.keep"));
        let mut c = map(json!({ "history.keep": true, "module.notifs": false }));
        assert!(!fold(&mut c));
        assert_eq!(c["module.notifs"], json!(false));
    }

    #[test]
    fn the_phone_tab_goes_to_the_tab_strip_and_nothing_else_changes() {
        let mut c = map(json!({ "module.phone": false, "tabs.hidden": ["chat", "weather"], "phone.enabled": true, "live.phone": true }));
        assert!(fold(&mut c));
        assert_eq!(c["tabs.hidden"], json!(["chat", "weather", "phone"]));
        assert_eq!((&c["phone.enabled"], &c["live.phone"]), (&json!(true), &json!(true)));
        assert!(!c.contains_key("module.phone"));
    }

    #[test]
    fn nothing_that_was_off_comes_on_and_a_second_fold_is_a_no_op() {
        // an untouched file (all defaults, the old keys absent or on) is left alone: the opt-ins stay opt-in
        let mut c = map(json!({ "module.weather": true, "module.phone": true, "history.keep": true, "theme": "dark" }));
        assert!(!fold(&mut c));
        let mut c = map(json!({}));
        assert!(!fold(&mut c));
        assert!(c.is_empty());
        // a mixed file: every rule fires once, the second pass changes nothing, no key turns true
        let mut c = map(json!({ "module.weather": false, "module.phone": false, "history.keep": false, "net.ai": false }));
        assert!(fold(&mut c));
        let once = c.clone();
        assert!(!fold(&mut c));
        assert_eq!(c, once);
        assert!(c.values().all(|v| v != &json!(true)), "{c:?}");
        // a stored non-list tabs.hidden is replaced by a list
        let mut c = map(json!({ "module.phone": false, "tabs.hidden": "junk" }));
        fold(&mut c);
        assert_eq!(c["tabs.hidden"], json!(["phone"]));
    }
}
