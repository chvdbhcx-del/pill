//! `ShellSettings`: the typed view of the settings.json keys pill.js reads (same key names, same defaults as its `S` object).
//!
//! `apply_json` mirrors `Object.assign(S, payload)`: only keys present in the payload change, everything else keeps its value.
//! JS truthiness (`S.showTime ? ..`), `!== false` tests (`edgeIdle`, `module.*`) and `+value || default` number coercion are reproduced
//! per key so a string `"60"` or a missing key behaves exactly like in the page. Unknown keys are kept in `raw` (used for
//! `module.*`, `live.*`, `quiet.*`).

use serde_json::{Map, Value};

use crate::shell::color::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeSetting {
    #[default]
    Dark,
    Light,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LangSetting {
    #[default]
    Auto,
    Ru,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    Ru,
    #[default]
    En,
}

/// `shape`: only the literal "capsule" is a capsule; anything else is the notch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Capsule,
    Notch,
}

/// Setting `character`: the bottom-edge style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeStyle {
    #[default]
    Liquid,
    Tail,
    Legs,
    Glow,
    None,
}

impl EdgeStyle {
    pub fn parse(s: &str) -> EdgeStyle {
        match s {
            "tail" => EdgeStyle::Tail,
            "legs" => EdgeStyle::Legs,
            "glow" => EdgeStyle::Glow,
            "none" => EdgeStyle::None,
            _ => EdgeStyle::Liquid,
        }
    }
}

/// quiet.* keys (see quiet.rs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuietSettings {
    /// `quiet.on === true` (strict).
    pub on: bool,
    pub from: String,
    pub to: String,
    /// `None` = key missing / not an array = every day; entries are `String(x)`.
    pub days: Option<Vec<String>>,
}

impl Default for QuietSettings {
    fn default() -> Self {
        QuietSettings { on: false, from: "22:00".into(), to: "07:00".into(), days: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShellSettings {
    /// Raw `size` (percent). Use [`ShellSettings::scale`].
    pub size: f64,
    pub theme: ThemeSetting,
    pub clock24: bool,
    pub auto_hide: bool,
    pub auto_hide_keep_desktop: bool,
    pub reduce_motion: bool,
    pub language: LangSetting,
    pub shape: Shape,
    /// `S.edgeIdle !== false`.
    pub edge_idle: bool,
    pub edge_cones: bool,
    pub last_tab: String,
    pub show_time: bool,
    pub show_date: bool,
    pub show_temp: bool,
    pub show_wx_icon: bool,
    pub show_seconds: bool,
    pub show_battery: bool,
    /// Raw percent values (clamped when used, see `op_col` / `op_exp`).
    pub opacity_col: f64,
    pub opacity_exp: f64,
    pub vpn_country: String,
    pub pinned: bool,
    /// Setting `fps` resolved: 0 = auto (no cap) or one of 30 | 60 | 120 | 144 | 240.
    pub fps_cap: u32,
    /// Setting `edgeIdleFps` resolved: 15 | 30 | 60.
    pub idle_fps: u32,
    pub pass_through: bool,
    pub pass_through_opacity: f64,
    pub character: EdgeStyle,
    pub net_weather: bool,
    pub pet: String,
    pub monitor: String,
    pub quiet: QuietSettings,
    /// Every key ever received (also the typed ones), for `module.*` / `live.*` lookups.
    pub raw: Map<String, Value>,
}

impl Default for ShellSettings {
    fn default() -> Self {
        ShellSettings {
            size: 100.0,
            theme: ThemeSetting::Dark,
            clock24: true,
            auto_hide: false,
            auto_hide_keep_desktop: false,
            reduce_motion: false,
            language: LangSetting::Auto,
            shape: Shape::Capsule,
            edge_idle: true,
            edge_cones: false,
            last_tab: String::new(),
            show_time: true,
            show_date: false,
            show_temp: true,
            show_wx_icon: true,
            show_seconds: false,
            show_battery: true,
            opacity_col: 0.0,
            opacity_exp: 0.0,
            vpn_country: String::new(),
            pinned: false,
            fps_cap: 0,
            idle_fps: 30,
            pass_through: false,
            pass_through_opacity: 0.0,
            character: EdgeStyle::Liquid,
            net_weather: false,
            pet: String::new(),
            monitor: "primary".into(),
            quiet: QuietSettings::default(),
            raw: Map::new(),
        }
    }
}

/// JS truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|x| x != 0.0 && !x.is_nan()).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// JS unary plus: `+value` (NaN when not numeric).
pub fn plus(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => *b as u8 as f64,
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        _ => f64::NAN,
    }
}

/// `+v || 0`.
fn num_or0(v: &Value) -> f64 {
    let x = plus(v);
    if x.is_nan() {
        0.0
    } else {
        x
    }
}

/// JS `Math.round`.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

impl ShellSettings {
    pub fn from_json(v: &Value) -> ShellSettings {
        let mut s = ShellSettings::default();
        s.apply_json(v);
        s
    }

    /// `Object.assign(S, payload)` + the derived values.
    pub fn apply_json(&mut self, v: &Value) {
        let Some(o) = v.as_object() else { return };
        for (k, val) in o {
            self.raw.insert(k.clone(), val.clone());
            match k.as_str() {
                "size" => self.size = plus(val),
                "theme" => {
                    self.theme = match val.as_str() {
                        Some("light") => ThemeSetting::Light,
                        Some("auto") => ThemeSetting::Auto,
                        _ => ThemeSetting::Dark,
                    }
                }
                "clock24" => self.clock24 = truthy(val),
                "autoHide" => self.auto_hide = truthy(val),
                "autoHideKeepDesktop" => self.auto_hide_keep_desktop = truthy(val),
                "reduceMotion" => self.reduce_motion = truthy(val),
                "language" => {
                    self.language = match val.as_str() {
                        Some("ru") => LangSetting::Ru,
                        Some("en") => LangSetting::En,
                        _ => LangSetting::Auto,
                    }
                }
                "shape" => self.shape = if val.as_str() == Some("capsule") { Shape::Capsule } else { Shape::Notch },
                "edgeIdle" => self.edge_idle = *val != Value::Bool(false),
                "edgeCones" => self.edge_cones = truthy(val),
                "lastTab" => self.last_tab = text(val),
                "showTime" => self.show_time = truthy(val),
                "showDate" => self.show_date = truthy(val),
                "showTemp" => self.show_temp = truthy(val),
                "showWxIcon" => self.show_wx_icon = truthy(val),
                "showSeconds" => self.show_seconds = truthy(val),
                "showBattery" => self.show_battery = truthy(val),
                "opacityCol" => self.opacity_col = num_or0(val),
                "opacityExp" => self.opacity_exp = num_or0(val),
                "vpnCountry" => self.vpn_country = text(val),
                "pinned" => self.pinned = truthy(val),
                "fps" => {
                    let f = js_round(plus(val));
                    self.fps_cap = if [30.0, 60.0, 120.0, 144.0, 240.0].contains(&f) { f as u32 } else { 0 };
                }
                "edgeIdleFps" => {
                    let e = js_round(plus(val));
                    self.idle_fps = if e == 15.0 || e == 60.0 { e as u32 } else { 30 };
                }
                "passThrough" => self.pass_through = truthy(val),
                "passThroughOpacity" => self.pass_through_opacity = num_or0(val),
                "character" => self.character = EdgeStyle::parse(val.as_str().unwrap_or("")),
                "net.weather" => self.net_weather = truthy(val),
                "pet" => self.pet = text(val),
                "monitor" => self.monitor = text(val),
                "quiet.on" => self.quiet.on = *val == Value::Bool(true),
                "quiet.from" => self.quiet.from = if val.is_null() { "22:00".into() } else { js_string(val) },
                "quiet.to" => self.quiet.to = if val.is_null() { "07:00".into() } else { js_string(val) },
                "quiet.days" => self.quiet.days = val.as_array().map(|a| a.iter().map(js_string).collect()),
                _ => {}
            }
        }
    }

    /// `scale()`: user size 80..200 % as a factor.
    pub fn scale(&self) -> f64 {
        let p = if self.size.is_nan() || self.size == 0.0 { 100.0 } else { self.size };
        (p / 100.0).clamp(0.8, 2.0)
    }

    pub fn is_capsule(&self) -> bool {
        self.shape == Shape::Capsule
    }

    /// `--op-col`: folded opacity (the pass-through mode has its own transparency, 0..80 %).
    pub fn op_col(&self) -> f64 {
        if self.pass_through {
            1.0 - self.pass_through_opacity.clamp(0.0, 80.0) / 100.0
        } else {
            1.0 - self.opacity_col.clamp(0.0, 60.0) / 100.0
        }
    }

    /// `--op-exp`: expanded opacity.
    pub fn op_exp(&self) -> f64 {
        1.0 - self.opacity_exp.clamp(0.0, 60.0) / 100.0
    }

    /// `data-theme` after the `auto` setting is resolved against the OS preference.
    pub fn resolved_theme(&self, os_prefers_light: bool) -> Theme {
        match self.theme {
            ThemeSetting::Light => Theme::Light,
            ThemeSetting::Dark => Theme::Dark,
            ThemeSetting::Auto => {
                if os_prefers_light {
                    Theme::Light
                } else {
                    Theme::Dark
                }
            }
        }
    }

    /// `resolveLang(S.language)` against the OS language ("ru-RU" etc).
    pub fn resolved_lang(&self, os_lang: &str) -> Lang {
        match self.language {
            LangSetting::Ru => Lang::Ru,
            LangSetting::En => Lang::En,
            LangSetting::Auto => lang_from_os(os_lang),
        }
    }

    /// `S[k] !== false` (Settings > Modules).
    pub fn module_on(&self, key: &str) -> bool {
        self.raw.get(key) != Some(&Value::Bool(false))
    }

    /// `Pill.liveOn(k)`: Settings > Live events; OBS is the only one off by default.
    pub fn live_on(&self, k: &str) -> bool {
        match self.raw.get(&format!("live.{k}")) {
            None => k != "obs",
            Some(v) => truthy(v),
        }
    }

    /// `wxWanted()`: Settings > Weather switch AND Settings > Modules > Weather.
    pub fn wx_wanted(&self) -> bool {
        self.net_weather && self.module_on("module.weather")
    }

    /// `S.pet && S.pet !== 'none'`.
    pub fn has_pet(&self) -> bool {
        !self.pet.is_empty() && self.pet != "none"
    }
}

/// `nav()`: Russian when the OS language starts with "ru", else English.
pub fn lang_from_os(os_lang: &str) -> Lang {
    if os_lang.len() >= 2 && os_lang[..2].eq_ignore_ascii_case("ru") {
        Lang::Ru
    } else {
        Lang::En
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_match_the_js_s_object() {
        let s = ShellSettings::default();
        assert_eq!(s.scale(), 1.0);
        assert!(s.clock24 && s.show_time && s.show_temp && s.show_wx_icon && s.show_battery && s.edge_idle);
        assert!(!s.show_date && !s.show_seconds && !s.auto_hide && !s.pinned && !s.reduce_motion && !s.edge_cones);
        assert_eq!(s.shape, Shape::Capsule);
        assert_eq!((s.fps_cap, s.idle_fps), (0, 30));
        assert_eq!(s.op_col(), 1.0);
        assert_eq!(s.op_exp(), 1.0);
        assert_eq!(s.character, EdgeStyle::Liquid);
    }

    #[test]
    fn apply_json_is_object_assign() {
        let mut s = ShellSettings::from_json(&json!({"showDate": true, "clock24": false}));
        assert!(s.show_date && !s.clock24);
        s.apply_json(&json!({"showTime": false}));
        assert!(s.show_date && !s.clock24 && !s.show_time); // earlier keys survive
        s.apply_json(&json!(null));
        assert!(!s.show_time);
    }

    #[test]
    fn scale_coercion() {
        assert_eq!(ShellSettings::from_json(&json!({"size": 50})).scale(), 0.8);
        assert_eq!(ShellSettings::from_json(&json!({"size": 500})).scale(), 2.0);
        assert_eq!(ShellSettings::from_json(&json!({"size": "150"})).scale(), 1.5);
        assert_eq!(ShellSettings::from_json(&json!({"size": "abc"})).scale(), 1.0); // NaN -> 100
        assert_eq!(ShellSettings::from_json(&json!({"size": 0})).scale(), 1.0);
        assert_eq!(ShellSettings::from_json(&json!({"size": null})).scale(), 1.0);
    }

    #[test]
    fn fps_and_idle_fps_resolution() {
        for (v, want) in [(json!(30), 30), (json!("60"), 60), (json!(144.4), 144), (json!("auto"), 0), (json!(75), 0), (json!(240), 240)] {
            assert_eq!(ShellSettings::from_json(&json!({"fps": v})).fps_cap, want);
        }
        for (v, want) in [(json!(15), 15), (json!(60), 60), (json!(30), 30), (json!(45), 30), (json!("x"), 30)] {
            assert_eq!(ShellSettings::from_json(&json!({"edgeIdleFps": v})).idle_fps, want);
        }
    }

    #[test]
    fn opacity_clamps_and_pass_through() {
        let s = ShellSettings::from_json(&json!({"opacityCol": 100, "opacityExp": 30}));
        assert!((s.op_col() - 0.4).abs() < 1e-12);
        assert!((s.op_exp() - 0.7).abs() < 1e-12);
        let s = ShellSettings::from_json(&json!({"opacityCol": 20, "passThrough": true, "passThroughOpacity": 95}));
        assert!((s.op_col() - 0.2).abs() < 1e-12); // pass-through reaches 80 % transparency
        let s = ShellSettings::from_json(&json!({"opacityCol": "x"}));
        assert_eq!(s.op_col(), 1.0);
    }

    #[test]
    fn edge_idle_is_not_equal_false() {
        assert!(ShellSettings::from_json(&json!({"edgeIdle": null})).edge_idle);
        assert!(ShellSettings::from_json(&json!({"edgeIdle": 0})).edge_idle); // only a literal false switches it off
        assert!(!ShellSettings::from_json(&json!({"edgeIdle": false})).edge_idle);
    }

    #[test]
    fn modules_and_live_switches() {
        let s = ShellSettings::from_json(&json!({"module.weather": false, "net.weather": true, "live.music": false, "live.obs": true}));
        assert!(!s.module_on("module.weather"));
        assert!(s.module_on("module.chat"));
        assert!(!s.wx_wanted()); // both switches are required
        assert!(!s.live_on("music"));
        assert!(s.live_on("obs"));
        assert!(s.live_on("mail"));
        assert!(!ShellSettings::default().live_on("obs"));
        let s = ShellSettings::from_json(&json!({"net.weather": true}));
        assert!(s.wx_wanted());
    }

    #[test]
    fn theme_shape_language() {
        let s = ShellSettings::from_json(&json!({"theme": "auto", "shape": "notch", "language": "ru"}));
        assert_eq!(s.resolved_theme(true), Theme::Light);
        assert_eq!(s.resolved_theme(false), Theme::Dark);
        assert!(!s.is_capsule());
        assert_eq!(s.resolved_lang("en-US"), Lang::Ru);
        let s = ShellSettings::from_json(&json!({"theme": "weird"}));
        assert_eq!(s.resolved_theme(true), Theme::Dark);
        assert_eq!(ShellSettings::default().resolved_lang("ru-RU"), Lang::Ru);
        assert_eq!(ShellSettings::default().resolved_lang("de"), Lang::En);
        assert_eq!(ShellSettings::default().resolved_lang(""), Lang::En);
    }

    #[test]
    fn character_and_pet() {
        assert_eq!(ShellSettings::from_json(&json!({"character": "glow"})).character, EdgeStyle::Glow);
        assert_eq!(ShellSettings::from_json(&json!({"character": "bogus"})).character, EdgeStyle::Liquid);
        assert!(ShellSettings::from_json(&json!({"pet": "cat"})).has_pet());
        assert!(!ShellSettings::from_json(&json!({"pet": "none"})).has_pet());
    }

    #[test]
    fn quiet_keys() {
        let s = ShellSettings::from_json(&json!({"quiet.on": true, "quiet.from": "23:00", "quiet.days": [1, "2"]}));
        assert!(s.quiet.on);
        assert_eq!(s.quiet.from, "23:00");
        assert_eq!(s.quiet.to, "07:00");
        assert_eq!(s.quiet.days, Some(vec!["1".to_string(), "2".to_string()]));
        assert!(!ShellSettings::from_json(&json!({"quiet.on": "yes"})).quiet.on); // strict === true
    }
}
