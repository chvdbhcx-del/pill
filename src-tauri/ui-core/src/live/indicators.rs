//! Small system indicators: a short chip on the folded island when Caps Lock / Num Lock is toggled, when a paired Bluetooth device connects
//! (with its battery when Windows knows it) and a card when that battery is low. Each has its own opt-in switch (all off by default).
//!
//! * `ind:lock {key: "caps" | "num", on}` (src/indicators.rs) -> "Caps Lock  on" for 1.6 s. Switches `live.capsLock`, `live.numLock`.
//! * `ind:bt {kind: "connect" | "low", name, percent?}` -> a chip "<name> [bar] 80 %" for 2.6 s, or (low, 15 % and less) an amber / red card.
//!   Switch `live.btBattery`.
//!
//! The chips are transient and are NOT queued: they stay out of the way in quiet hours (`env.quiet()`), while Windows Focus Assist is respected
//! and active (`focus::HOLD_KEY`), and while the island is open. The low-battery CARD is not critical, so it follows the card rules: dropped in
//! quiet hours, held by Focus Assist (`focus::verdict`) and shown when that ends.
//! PRIVACY: the producers read the lock state of the two keys and the names / battery levels of devices the user has paired; no key text, no scan.

use std::any::Any;

use serde_json::Value;

use super::env::Env;
use super::focus::HOLD_KEY;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const LOCK_MS: u32 = 1600;
pub const BT_MS: u32 = 2600;
/// At or below this level a device gets the low-battery card.
pub const LOW_PCT: i32 = 15;

#[derive(Debug, Default)]
pub struct IndicatorsLive;

/// May a transient chip appear now: collapsed island, no quiet hours, no Focus Assist hold.
fn chip_ok(env: &dyn Env) -> bool {
    env.collapsed() && !env.quiet() && !env.setting_true(HOLD_KEY)
}

fn lock_row(env: &dyn Env, key: &str, on: bool) -> RowSpec {
    let (id, name) = if key == "caps" { ("ind-caps", "Caps Lock") } else { ("ind-num", "Num Lock") };
    let state = if on { env.tr("вкл", "on") } else { env.tr("выкл", "off") };
    RowSpec::new(id, 91)
        .ttl(LOCK_MS)
        .key(format!("{id}:{on}"))
        .tip(name)
        .anim(Anim::Pop, Anim::None, Anim::Fade)
        .with(Elem::icon_c(IconName::Keyboard, 17.0, if on { ColorRole::Accent } else { ColorRole::Dim }))
        .with(Elem::text(name, TextStyle::Label))
        .with(Elem::text(state, if on { TextStyle::Code } else { TextStyle::Dim }))
}

fn bt_row(env: &dyn Env, name: &str, pct: Option<i32>) -> RowSpec {
    let r = RowSpec::new("ind-bt", 91)
        .ttl(BT_MS)
        .key(format!("ind-bt:{name}:{pct:?}"))
        .anim(Anim::Pop, Anim::None, Anim::Fade)
        .with(Elem::icon_c(IconName::Battery, 17.0, ColorRole::Dim))
        .with(Elem::text_max(name, TextStyle::Label, 150.0));
    match pct {
        Some(p) => r
            .with(Elem::Bar { value: p as f32 / 100.0, width: 34.0, height: 5.0, glide_ms: 420, muted: false })
            .with(Elem::text(format!("{p} %"), TextStyle::Dim)),
        None => r.with(Elem::text(env.tr("подключено", "connected"), TextStyle::Dim)),
    }
}

fn low_card(env: &dyn Env, name: &str, pct: i32) -> CardSpec {
    let tone = if pct <= 10 { Tone::Red } else { Tone::Amber };
    CardSpec::text(
        format!("ind-bt-low:{name}"),
        7000,
        env.tr(&format!("{name}: заряд {pct} %"), &format!("{name}: battery {pct}%")),
        Some(env.tr("Пора зарядить устройство", "Time to charge the device")),
    )
    .icon(IconName::Battery)
    .tone(tone)
    .actions(vec![CardAction::close(env.tr("Закрыть", "Close"))])
}

impl Provider for IndicatorsLive {
    fn name(&self) -> &'static str {
        "indicators"
    }
    fn events(&self) -> &'static [&'static str] {
        &["ind:lock", "ind:bt"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        match name {
            "ind:lock" => {
                let key = p.get("key").and_then(Value::as_str).unwrap_or("");
                let switch = match key {
                    "caps" => "live.capsLock",
                    "num" => "live.numLock",
                    _ => return Vec::new(),
                };
                if !env.setting_true(switch) || !chip_ok(env) {
                    return Vec::new();
                }
                vec![LiveCmd::set(lock_row(env, key, truthy(p.get("on"))))]
            }
            "ind:bt" => {
                let dev = cut(&text(p.get("name")), 40);
                if !env.setting_true("live.btBattery") || dev.is_empty() {
                    return Vec::new();
                }
                let pct = p.get("percent").filter(|v| !v.is_null()).map(|v| int32(Some(v)).clamp(0, 100));
                match p.get("kind").and_then(Value::as_str) {
                    Some("connect") if chip_ok(env) => vec![LiveCmd::set(bt_row(env, &dev, pct))],
                    Some("low") => match pct.filter(|p| *p <= LOW_PCT) {
                        Some(pct) => vec![LiveCmd::Toast(low_card(env, &dev, pct))],
                        None => Vec::new(),
                    },
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::{MemEnv, PillState};
    use serde_json::json;

    const T0: i64 = 1_793_363_640_000;

    fn on(keys: &[&str]) -> MemEnv {
        let mut e = MemEnv::default();
        for k in keys {
            e.set(k, json!(true));
        }
        e
    }
    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no row in {cmds:?}"))
    }
    fn texts(r: &RowSpec) -> Vec<String> {
        r.elements.iter().filter_map(|e| if let Elem::Text { text, .. } = e { Some(text.clone()) } else { None }).collect()
    }

    #[test]
    fn every_indicator_is_opt_in() {
        let mut i = IndicatorsLive;
        let off = MemEnv::default();
        assert!(i.on_event(&off, "ind:lock", &json!({"key": "caps", "on": true}), T0).is_empty(), "a missing key is OFF");
        assert!(i.on_event(&off, "ind:bt", &json!({"kind": "connect", "name": "Buds"}), T0).is_empty());
        let mut e = off.clone();
        e.set("live.capsLock", json!(false));
        assert!(i.on_event(&e, "ind:lock", &json!({"key": "caps", "on": true}), T0).is_empty());
        // the switches are separate: Caps on does not turn Num on
        let caps = on(&["live.capsLock"]);
        assert!(!i.on_event(&caps, "ind:lock", &json!({"key": "caps", "on": true}), T0).is_empty());
        assert!(i.on_event(&caps, "ind:lock", &json!({"key": "num", "on": true}), T0).is_empty());
        assert!(i.on_event(&caps, "ind:bt", &json!({"kind": "connect", "name": "Buds"}), T0).is_empty());
    }

    #[test]
    fn the_lock_chip_says_which_key_and_the_state() {
        let mut i = IndicatorsLive;
        let e = on(&["live.capsLock", "live.numLock"]);
        let c = i.on_event(&e, "ind:lock", &json!({"key": "caps", "on": true}), T0);
        let r = row(&c);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("ind-caps", 91, Some(LOCK_MS)));
        assert_eq!(texts(r), ["Caps Lock", "вкл"]);
        assert_eq!(r.anim.enter, Anim::Pop, "it pops in");
        let en = MemEnv { ru: false, ..e.clone() };
        let c = i.on_event(&en, "ind:lock", &json!({"key": "num", "on": false}), T0);
        assert_eq!(row(&c).id, "ind-num");
        assert_eq!(texts(row(&c)), ["Num Lock", "off"]);
        // on and off are different keys of the row, so the shell redraws it when the state flips inside the 1.6 s
        let a = row(&i.on_event(&e, "ind:lock", &json!({"key": "caps", "on": true}), T0)).key.clone();
        let b = row(&i.on_event(&e, "ind:lock", &json!({"key": "caps", "on": false}), T0)).key.clone();
        assert_ne!(a, b);
        assert!(i.on_event(&e, "ind:lock", &json!({"key": "scroll", "on": true}), T0).is_empty(), "only the two keys");
    }

    #[test]
    fn chips_stay_out_in_quiet_hours_focus_hold_and_the_open_island() {
        let mut i = IndicatorsLive;
        let ev = json!({"key": "caps", "on": true});
        let mut e = on(&["live.capsLock", "live.btBattery"]);
        assert!(!i.on_event(&e, "ind:lock", &ev, T0).is_empty());
        e.quiet = true;
        assert!(i.on_event(&e, "ind:lock", &ev, T0).is_empty(), "quiet hours");
        assert!(i.on_event(&e, "ind:bt", &json!({"kind": "connect", "name": "Buds"}), T0).is_empty());
        e.quiet = false;
        e.set(HOLD_KEY, json!(true));
        assert!(i.on_event(&e, "ind:lock", &ev, T0).is_empty(), "Focus Assist is respected and on");
        assert!(i.on_event(&e, "ind:bt", &json!({"kind": "connect", "name": "Buds"}), T0).is_empty());
        e.set(HOLD_KEY, json!(false));
        e.state = PillState::Expanded;
        assert!(i.on_event(&e, "ind:lock", &ev, T0).is_empty(), "the open island shows no chip");
    }

    #[test]
    fn a_connected_device_shows_its_battery_when_known() {
        let mut i = IndicatorsLive;
        let e = on(&["live.btBattery"]);
        let c = i.on_event(&e, "ind:bt", &json!({"kind": "connect", "name": "WH-1000XM5", "percent": 80}), T0);
        let r = row(&c);
        assert_eq!((r.id.as_str(), r.ttl_ms), ("ind-bt", Some(BT_MS)));
        assert_eq!(texts(r), ["WH-1000XM5", "80 %"]);
        assert!(r.elements.iter().any(|x| matches!(x, Elem::Bar { value, .. } if (*value - 0.8).abs() < 1e-6)), "a bar that glides to the level");
        let c = i.on_event(&e, "ind:bt", &json!({"kind": "connect", "name": "Mouse", "percent": null}), T0);
        assert_eq!(texts(row(&c)), ["Mouse", "подключено"], "no level known: just connected");
        assert!(i.on_event(&e, "ind:bt", &json!({"kind": "connect", "name": ""}), T0).is_empty(), "nameless: nothing");
        let long = "x".repeat(100);
        let c = i.on_event(&e, "ind:bt", &json!({"kind": "connect", "name": long}), T0);
        assert!(texts(row(&c))[0].chars().count() <= 40);
    }

    #[test]
    fn low_battery_is_a_card_that_follows_the_card_rules() {
        let mut i = IndicatorsLive;
        let e = on(&["live.btBattery"]);
        let c = i.on_event(&e, "ind:bt", &json!({"kind": "low", "name": "Buds", "percent": 12}), T0);
        let LiveCmd::Toast(t) = &c[0] else { panic!("{c:?}") };
        assert_eq!((t.id.as_str(), t.tone, t.urgent), ("ind-bt-low:Buds", Tone::Amber, false));
        assert!(!t.quiet_exempt(), "not critical: quiet hours and Focus Assist may hold it back");
        let c = i.on_event(&e, "ind:bt", &json!({"kind": "low", "name": "Buds", "percent": 9}), T0);
        let LiveCmd::Toast(t) = &c[0] else { panic!() };
        assert_eq!(t.tone, Tone::Red);
        assert!(i.on_event(&e, "ind:bt", &json!({"kind": "low", "name": "Buds", "percent": 40}), T0).is_empty(), "above 15 %: no warning");
        assert!(i.on_event(&e, "ind:bt", &json!({"kind": "low", "name": "Buds"}), T0).is_empty(), "unknown level: no warning");
        // the hub's quiet filter drops it in quiet hours
        let kept = crate::live::board::filter_quiet(i.on_event(&e, "ind:bt", &json!({"kind": "low", "name": "Buds", "percent": 12}), T0), true);
        assert!(kept.is_empty());
    }
}
