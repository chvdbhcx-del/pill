//! The "VPN is on / off" card (Rust side: `vpn.rs`).
//!
//! * `vpn:changed {country, on}` -> a short card when the state really flips (the first answer after start only sets the baseline, so
//!   expanding the island never announces anything).
//! * With the VPN module switched off in Settings (`module.vpn`) there is no card: the backend sends an empty `country` and the baseline
//!   is forgotten. The chip in the open island is not a live row; it is part of the header (`ui/island/chrome.rs`).

use std::any::Any;

use serde_json::Value;

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

#[derive(Debug, Default)]
pub struct VpnLive {
    prev: Option<bool>,
}

impl Provider for VpnLive {
    fn name(&self) -> &'static str {
        "vpn"
    }
    fn events(&self) -> &'static [&'static str] {
        &["vpn:changed"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if p.is_null() {
            return Vec::new();
        }
        if text_or_empty(p.get("country")).is_empty() || !env.module_on("module.vpn") {
            self.prev = None; // feature off, or nothing to switch: forget the baseline
            return Vec::new();
        }
        let on = truthy(p.get("on"));
        let before = self.prev.replace(on);
        if before.is_none() || before == Some(on) {
            return Vec::new();
        }
        let title = if on { env.tr("VPN включён", "VPN is on") } else { env.tr("VPN выключен", "VPN is off") };
        let card = CardSpec::text("vpn", 4000, title, None)
            .icon(if on { IconName::Check } else { IconName::X })
            .tone(if on { Tone::Accent } else { Tone::Neutral })
            .actions(vec![CardAction::close(env.tr("Ок", "OK"))]);
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
    use serde_json::json;

    fn ev(p: &mut VpnLive, env: &MemEnv, country: &str, on: bool) -> Vec<LiveCmd> {
        p.on_event(env, "vpn:changed", &json!({ "country": country, "on": on }), 0)
    }

    #[test]
    fn only_a_real_flip_makes_a_card() {
        let mut p = VpnLive::default();
        let env = MemEnv::en();
        assert!(ev(&mut p, &env, "VPN", false).is_empty(), "the first answer is the baseline");
        assert!(ev(&mut p, &env, "VPN", false).is_empty(), "expanding again says nothing");
        let on = ev(&mut p, &env, "VPN", true);
        assert!(matches!(&on[..], [LiveCmd::Toast(c)] if c.id == "vpn" && matches!(&c.body, CardBody::Text { title, .. } if title == "VPN is on")));
        assert!(ev(&mut p, &env, "VPN", true).is_empty());
        let off = ev(&mut p, &env, "VPN", false);
        assert!(matches!(&off[..], [LiveCmd::Toast(c)] if matches!(&c.body, CardBody::Text { title, .. } if title == "VPN is off")));
    }

    #[test]
    fn no_card_when_the_module_is_off_or_there_is_nothing_to_show() {
        let mut p = VpnLive::default();
        let mut env = MemEnv::en();
        ev(&mut p, &env, "VPN", false);
        env.set("module.vpn", json!(false));
        assert!(ev(&mut p, &env, "VPN", true).is_empty(), "module.vpn off: silent");
        env.unset("module.vpn");
        assert!(ev(&mut p, &env, "", true).is_empty(), "no chip: silent");
        assert!(ev(&mut p, &env, "VPN", true).is_empty(), "the baseline was forgotten, this is a new first answer");
        assert!(p.on_event(&env, "vpn:changed", &Value::Null, 0).is_empty());
    }
}
