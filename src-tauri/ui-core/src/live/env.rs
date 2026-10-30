//! What providers may ask the host: language, settings, quiet hours, capsule state.
//!
//! Providers never read the clock or settings themselves; `now` (epoch ms, `Date.now()`) comes in as
//! an argument and everything else goes through [`Env`]. [`MemEnv`] is the in-memory implementation
//! the tests (and any renderer harness) use.

use std::collections::HashMap;

use serde_json::Value;

use super::util::truthy;

/// `Pill.t({ ru, en })` as a trait: pick by language.
pub trait Tr {
    fn is_ru(&self) -> bool;
    fn tr(&self, ru: &str, en: &str) -> String {
        if self.is_ru() { ru } else { en }.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillState {
    Collapsed,
    Expanded,
}

pub trait Env: Tr {
    /// A key of settings.json (`live.volume`, `module.media`, `capture.format`, ...); `None` = never written.
    fn setting(&self, key: &str) -> Option<Value>;
    /// Quiet hours are on right now (`PillQuiet.quietNow`; the window logic lives in the shell module).
    fn quiet(&self) -> bool;
    fn state(&self) -> PillState;

    fn collapsed(&self) -> bool {
        self.state() == PillState::Collapsed
    }
    fn expanded(&self) -> bool {
        self.state() == PillState::Expanded
    }
    /// `Pill.liveOn(k)`: Settings > Live events. A missing key is ON, except `obs` (off by default).
    fn live_on(&self, k: &str) -> bool {
        match self.setting(&format!("live.{k}")) {
            None => k != "obs",
            Some(v) => truthy(Some(&v)),
        }
    }
    /// `Pill.moduleOn(k)`: `S[k] !== false`.
    fn module_on(&self, k: &str) -> bool {
        self.setting(k) != Some(Value::Bool(false))
    }
    /// `settings[k] === true` (the opt-in producers: calendar, mail).
    fn setting_true(&self, k: &str) -> bool {
        self.setting(k) == Some(Value::Bool(true))
    }
    fn setting_str(&self, k: &str) -> Option<String> {
        self.setting(k).and_then(|v| v.as_str().map(str::to_string))
    }
}

/// In-memory [`Env`]. Public fields so a test flips them directly.
#[derive(Clone, Debug)]
pub struct MemEnv {
    pub ru: bool,
    pub quiet: bool,
    pub state: PillState,
    pub settings: HashMap<String, Value>,
}

impl Default for MemEnv {
    /// Russian, not quiet, collapsed, no settings.
    fn default() -> Self {
        MemEnv { ru: true, quiet: false, state: PillState::Collapsed, settings: HashMap::new() }
    }
}

impl MemEnv {
    pub fn en() -> MemEnv {
        MemEnv { ru: false, ..MemEnv::default() }
    }
    pub fn set(&mut self, k: &str, v: Value) -> &mut Self {
        self.settings.insert(k.to_string(), v);
        self
    }
    pub fn unset(&mut self, k: &str) -> &mut Self {
        self.settings.remove(k);
        self
    }
}

impl Tr for MemEnv {
    fn is_ru(&self) -> bool {
        self.ru
    }
}

impl Env for MemEnv {
    fn setting(&self, key: &str) -> Option<Value> {
        self.settings.get(key).cloned()
    }
    fn quiet(&self) -> bool {
        self.quiet
    }
    fn state(&self) -> PillState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tr_picks_language() {
        let mut e = MemEnv::default();
        assert_eq!(e.tr("да", "yes"), "да");
        e.ru = false;
        assert_eq!(e.tr("да", "yes"), "yes");
    }

    #[test]
    fn live_on_defaults() {
        let mut e = MemEnv::default();
        assert!(e.live_on("volume")); // missing = on
        assert!(!e.live_on("obs")); // the one opt-in
        e.set("live.volume", json!(false));
        assert!(!e.live_on("volume"));
        e.set("live.obs", json!(true));
        assert!(e.live_on("obs"));
        e.set("live.copy", json!(null));
        assert!(!e.live_on("copy")); // !!null
        e.set("live.usb", json!(0));
        assert!(!e.live_on("usb"));
        e.set("live.battery", json!("yes"));
        assert!(e.live_on("battery"));
    }

    #[test]
    fn module_and_opt_in_semantics() {
        let mut e = MemEnv::default();
        assert!(e.module_on("module.media"));
        e.set("module.media", json!(false));
        assert!(!e.module_on("module.media"));
        e.set("module.media", json!(null));
        assert!(e.module_on("module.media")); // only an explicit false turns it off
        assert!(!e.setting_true("live.mail"));
        e.set("live.mail", json!(1));
        assert!(!e.setting_true("live.mail")); // === true, not truthy
        e.set("live.mail", json!(true));
        assert!(e.setting_true("live.mail"));
    }

    #[test]
    fn state_helpers() {
        let mut e = MemEnv::default();
        assert!(e.collapsed() && !e.expanded());
        e.state = PillState::Expanded;
        assert!(e.expanded() && !e.collapsed());
        assert_eq!(e.setting_str("x"), None);
        e.set("x", json!("v"));
        assert_eq!(e.setting_str("x").as_deref(), Some("v"));
    }
}
