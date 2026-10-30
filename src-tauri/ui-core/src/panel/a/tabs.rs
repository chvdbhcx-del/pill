//! Tab registry and header model of the expanded panel: tab order, which tabs are enabled by `module.*` settings, the last
//! used tab, tab switching with its slide direction and animation parameters, the badge on the active tab, the header
//! buttons (dictation, pin, settings, VPN country chip) and the `pill:expand` / `pill:open-tab` events.
//! Port of the "tabs + header cluster" part of `src/pill.js` (`register`, `activate`, `applyModules`, `doExpand` ...).
//!
//! The tab bodies are other models; this one tells the shell which of them to show / hide through [`TabEvent`]s
//! (`Show(id)` = `onShow`, `Hide(id)` = `onHide`, `Retarget` = the panel height follows the new tab's spring).
//!
//! Backend: `settings_set {patch: {lastTab}}` on user tab clicks (the island reopens on the last used tab),
//! `settings_open {page}`, `pill_set_state {state, tab, via}`.
//! Settings: `module.clipboard|shelf|text|chat|note|weather|controls|notifs|media|launcher`, `live.agents`, `lastTab`,
//! `pinned`, `vpnCountry`, `reduceMotion`.

use super::anim::Anim;
use super::{str_of, BackendCall, Cfg, Fx, FxQueue, Tr};
use serde_json::{json, Value};

/// Settings pages `settings_open` accepts; anything else becomes `general`.
pub const SETTINGS_PAGES: [&str; 14] = ["general", "appearance", "behavior", "search", "character", "hotkeys", "ai", "voice", "weather", "sound", "live", "clipboard", "modules", "updates"];

/// Key of the object a renderer passes to `on_result` when a command was rejected (the JS pages see a rejected promise).
pub const ERR_KEY: &str = "__error";

/// Content height by tab when the tab did not register one (`DEF_H`); other tabs use the shell default.
const DEF_H: [(&str, u32); 9] = [("home", 331), ("clipboard", 331), ("shelf", 331), ("text", 331), ("controls", 331), ("settings", 331), ("note", 417), ("weather", 413), ("chat", 417)];

#[derive(Clone, Debug, PartialEq)]
pub struct TabSpec {
    pub id: &'static str,
    pub ru: &'static str,
    pub en: &'static str,
    pub order: i32,
    /// Icon name the renderer maps to a glyph.
    pub icon: &'static str,
    /// Not in the header (the in-pill Settings fallback).
    pub hidden: bool,
    /// Own content height (`register({height})`).
    pub height: Option<u32>,
}

/// Every tab the pill registers, in the order of their `register()` calls.
pub fn standard_tabs() -> Vec<TabSpec> {
    let t = |id, ru, en, order, icon, height| TabSpec { id, ru, en, order, icon, hidden: false, height };
    vec![
        t("home", "Главная", "Home", 0, "home", None),
        t("clipboard", "Буфер", "Clipboard", 10, "clipboard", None),
        t("shelf", "Полка", "Shelf", 20, "shelf", None),
        t("text", "Текст", "Text", 30, "text", None),
        t("chat", "Чат", "Chat", 40, "chat", Some(417)),
        t("note", "Заметка", "Note", 50, "note", None),
        t("weather", "Погода", "Weather", 60, "weather", Some(417)),
        t("notifs", "Уведомления", "Notifications", 65, "bell", None),
        t("agents", "Агенты", "Agents", 66, "terminal", None),
        t("controls", "Управление", "Controls", 70, "controls", None),
        TabSpec { id: "settings", ru: "Настройки", en: "Settings", order: 900, icon: "settings", hidden: true, height: None },
    ]
}

/// `MODTAB`: the setting that switches a tab on or off.
fn module_key(id: &str) -> Option<&'static str> {
    Some(match id {
        "clipboard" => "module.clipboard",
        "shelf" => "module.shelf",
        "text" => "module.text",
        "chat" => "module.chat",
        "note" => "module.note",
        "weather" => "module.weather",
        "controls" => "module.controls",
        "notifs" => "module.notifs",
        "agents" => "live.agents",
        _ => return None,
    })
}

/// What the shell must do with the tab bodies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabEvent {
    /// `onShow` of the tab.
    Show(String),
    /// `onHide` of the tab.
    Hide(String),
    /// The panel height follows the new tab's spring (`retarget(false, null, 'tab')`).
    Retarget,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A click on a tab button.
    SelectTab(String),
    /// Dictation button: `dictate_available` = the shell has a dictation handler.
    Dictate { available: bool },
    Pin,
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabButton {
    pub id: &'static str,
    pub icon: &'static str,
    pub title: String,
    pub active: bool,
    /// Counter pill shown on the active tab only (empty = none).
    pub badge: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeaderView {
    pub tabs: Vec<TabButton>,
    pub dictate_title: String,
    pub pin_title: String,
    pub pin_on: bool,
    pub settings_title: String,
    /// The settings button is lit while the in-pill Settings tab is open.
    pub settings_current: bool,
    /// VPN country chip, hidden when empty.
    pub vpn: String,
    /// Slide direction of the last switch: `r` or `l`.
    pub dir: char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsFrom {
    Button,
    Expand,
}

pub struct Tabs {
    specs: Vec<TabSpec>,
    off: Vec<&'static str>,
    cfg: Cfg,
    cur: Option<&'static str>,
    prev_tab: Option<&'static str>,
    chosen: bool,
    expanded: bool,
    frame_exp: bool,
    dir_right: bool,
    badges: Vec<(String, String)>,
    pinned: bool,
    settings_from: Option<SettingsFrom>,
    events: Vec<TabEvent>,
    fx: FxQueue,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs::new()
    }
}

impl Tabs {
    pub fn new() -> Tabs {
        let mut t = Tabs {
            specs: standard_tabs(),
            off: vec![],
            cfg: Cfg::new(),
            cur: None,
            prev_tab: None,
            chosen: false,
            expanded: false,
            frame_exp: false,
            dir_right: true,
            badges: vec![],
            pinned: false,
            settings_from: None,
            events: vec![],
            fx: FxQueue::default(),
        };
        // `register`: until the user (or code) picks a tab, the first visible one is the current one
        t.cur = t.visible().first().map(|s| s.id);
        t
    }

    pub fn take_fx(&mut self) -> Vec<Fx> {
        self.fx.take()
    }
    pub fn take_events(&mut self) -> Vec<TabEvent> {
        std::mem::take(&mut self.events)
    }
    pub fn current(&self) -> Option<&'static str> {
        self.cur
    }
    /// `P.lastTabId()`: the persisted last tab (`settings.lastTab`), `""` if none; the shelf uses it to decide where a file drag opens.
    pub fn last_tab_id(&self) -> &str {
        self.cfg.str("lastTab")
    }
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }
    pub fn pinned(&self) -> bool {
        self.pinned
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.cfg.os_reduce = on;
    }
    /// `Pill.setPinned` (the shelf pins the island during a native drag).
    pub fn set_pinned(&mut self, v: bool) {
        self.pinned = v;
    }
    /// The OS frame is expanded (`frameExp`); only the settings-window hand-off needs it.
    pub fn set_frame_expanded(&mut self, v: bool) {
        self.frame_exp = v;
    }
    /// Keep the shell's collapse in sync: `doCollapse` hides the current tab.
    pub fn on_collapsed(&mut self) {
        if self.expanded {
            self.expanded = false;
            if let Some(c) = self.cur {
                self.events.push(TabEvent::Hide(c.to_string()));
            }
        }
    }
    /// `forceCollapse` (hidden by Rust): same bookkeeping.
    pub fn force_collapse(&mut self) {
        self.on_collapsed();
    }
    /// `Pill.setBadge(tab, text)`; the header shows it on the active tab only.
    pub fn set_badge(&mut self, id: &str, text: &str) {
        self.badges.retain(|(i, _)| i != id);
        if !text.is_empty() {
            self.badges.push((id.to_string(), text.to_string()));
        }
    }

    fn spec(&self, id: &str) -> Option<&TabSpec> {
        self.specs.iter().find(|s| s.id == id)
    }

    fn is_off(&self, id: &str) -> bool {
        self.off.contains(&id)
    }

    /// `visibleTabs`: not hidden, not switched off, sorted by order (registration order breaks ties).
    pub fn visible(&self) -> Vec<&TabSpec> {
        let mut v: Vec<&TabSpec> = self.specs.iter().filter(|s| !s.hidden && !self.is_off(s.id)).collect();
        v.sort_by_key(|s| s.order);
        v
    }

    /// `tabOff`: Home stays while the player OR the launcher is on.
    pub fn tab_off(cfg: &Cfg, id: &str) -> bool {
        if id == "home" {
            return !cfg.module_on("module.media") && !cfg.module_on("module.launcher");
        }
        module_key(id).map_or(false, |k| !cfg.module_on(k))
    }

    /// Content height of the current tab in CSS px; `None` = the shell's default. (`tabH` before clamping to the OS frame.)
    pub fn body_height(&self) -> Option<u32> {
        let c = self.cur?;
        self.spec(c)?.height.or_else(|| DEF_H.iter().find(|(i, _)| *i == c).map(|(_, h)| *h))
    }

    fn reduce(&self) -> bool {
        self.cfg.reduce_motion()
    }

    // ---------------------------------------------------------------- settings
    /// `settings_get` answer or `settings:changed` payload: modules, `lastTab`, pin, VPN chip.
    pub fn on_settings(&mut self, payload: &Value, startup: bool) {
        if !self.cfg.apply(payload) {
            return;
        }
        self.pinned = self.cfg.truthy("pinned");
        self.apply_modules();
        if startup {
            // on start: reopen on the last used tab, unless something already picked one
            let last = self.cfg.str("lastTab").to_string();
            if !self.chosen && !last.is_empty() {
                if let Some(s) = self.spec(&last) {
                    if !s.hidden && !self.is_off(s.id) {
                        let id = s.id;
                        self.activate(id, false);
                    }
                }
            }
        }
    }

    /// `applyModules`: a module that is off hides its tab (live, from `settings:changed`).
    fn apply_modules(&mut self) {
        let mut changed = false;
        let ids: Vec<&'static str> = self.specs.iter().map(|s| s.id).collect();
        for id in ids {
            let off = Tabs::tab_off(&self.cfg, id);
            if off == self.is_off(id) {
                continue;
            }
            if off {
                self.off.push(id);
            } else {
                self.off.retain(|o| *o != id);
            }
            changed = true;
        }
        if !changed {
            return;
        }
        if self.cur.map_or(false, |c| self.is_off(c)) {
            // the open tab was switched off: go to the first one that is left
            if let Some(first) = self.visible().first().map(|s| s.id) {
                self.activate(first, false);
            }
        }
        if self.expanded {
            self.events.push(TabEvent::Retarget);
        }
    }

    // ---------------------------------------------------------------- switching
    fn activate(&mut self, id: &str, persist: bool) -> Vec<BackendCall> {
        let Some(tb) = self.spec(id).cloned() else { return vec![] };
        if self.cur == Some(tb.id) {
            return vec![];
        }
        let mut calls = vec![];
        if persist && !tb.hidden && self.cfg.str("lastTab") != tb.id {
            self.cfg.set("lastTab", json!(tb.id)); // the island reopens on the last used tab
            calls.push(BackendCall::new("settings_set", json!({ "patch": { "lastTab": tb.id } })));
        }
        if let Some(old_id) = self.cur {
            let old = self.spec(old_id).cloned().unwrap();
            let mut order: Vec<&str> = self.visible().iter().map(|s| s.id).collect();
            order.push("settings");
            let idx = |i: &str| order.iter().position(|o| *o == i).map_or(-1, |p| p as isize);
            self.dir_right = idx(tb.id) > idx(old.id);
            if self.expanded {
                self.fx.anim(self.reduce(), format!("view:{}", old.id), Anim::view_away(self.dir_right));
            }
            if !old.hidden {
                self.prev_tab = Some(old.id);
            }
            if self.expanded {
                self.events.push(TabEvent::Hide(old.id.to_string()));
            }
        }
        self.cur = Some(tb.id);
        if self.expanded {
            self.fx.anim(self.reduce(), format!("view:{}", tb.id), Anim::view_in(self.dir_right));
            self.events.push(TabEvent::Show(tb.id.to_string())); // tabs differ in height: the panel follows its own spring
            self.events.push(TabEvent::Retarget);
        }
        calls
    }

    /// `doExpand(tab)`: the island opens (the shell has already moved the OS frame). `section` goes to the settings window.
    pub fn expand(&mut self, tab: Option<&str>, section: Option<&str>) -> Vec<BackendCall> {
        if tab == Some("settings") {
            // the command bar asks for the settings tab: open the window instead and give the frame back
            self.settings_from = Some(SettingsFrom::Expand);
            let page = section.filter(|s| SETTINGS_PAGES.contains(s)).unwrap_or("general");
            return vec![BackendCall::new("settings_open", json!({ "page": page }))];
        }
        self.do_expand(tab)
    }

    fn do_expand(&mut self, tab: Option<&str>) -> Vec<BackendCall> {
        let was = self.expanded;
        self.expanded = true;
        self.frame_exp = true;
        let mut calls = vec![];
        let known = tab.and_then(|t| self.spec(t)).filter(|s| !self.is_off(s.id)).map(|s| s.id);
        match known {
            Some(id) => {
                self.chosen = true;
                if self.cur != Some(id) {
                    calls = self.activate(id, true);
                } else if !was {
                    self.events.push(TabEvent::Show(id.to_string()));
                }
            }
            None => {
                if !was {
                    if let Some(c) = self.cur {
                        self.events.push(TabEvent::Show(c.to_string()));
                    }
                }
            }
        }
        if !was {
            self.events.push(TabEvent::Retarget);
        }
        calls
    }

    /// `Pill.expand(tab, via)`: ask Rust to expand; it answers with `pill:expand`, which calls [`Tabs::expand`].
    pub fn request_expand(tab: Option<&str>, via: Option<&str>) -> BackendCall {
        BackendCall::new("pill_set_state", json!({ "state": "expanded", "tab": tab, "via": via.unwrap_or("page") }))
    }

    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        match name {
            "pill:expand" => {
                let tab = payload.get("tab").and_then(Value::as_str);
                let section = payload.get("section").and_then(Value::as_str);
                self.expand(tab, section)
            }
            // command bar payload { tab, noteId?, section? }: only the settings tab is ours, the note tab handles its own
            "pill:open-tab" => {
                if payload.get("tab").and_then(Value::as_str) == Some("settings") {
                    return self.expand(Some("settings"), payload.get("section").and_then(Value::as_str));
                }
                vec![]
            }
            "pill:collapse" => {
                if !self.pinned {
                    self.on_collapsed();
                }
                vec![]
            }
            "pill:hide" => {
                self.force_collapse();
                vec![]
            }
            "settings:changed" => {
                self.on_settings(payload, false);
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn on_action(&mut self, a: Action, tr: &dyn Tr) -> Vec<BackendCall> {
        match a {
            Action::SelectTab(id) => {
                self.chosen = true;
                self.activate(&id, true)
            }
            Action::Dictate { available } => {
                if !available {
                    self.fx.toast("dict-soon", "wave", tr.t("Диктовка скоро", "Dictation is coming soon"), 1800);
                }
                vec![]
            }
            Action::Pin => {
                self.pinned = !self.pinned;
                self.fx.push(Fx::SetPinned(self.pinned));
                vec![]
            }
            Action::Settings => {
                if self.cur == Some("settings") {
                    // back to where we came from
                    let back = self.prev_tab.filter(|p| self.spec(p).is_some()).or_else(|| self.visible().first().map(|s| s.id));
                    return back.map(|b| self.activate(b, false)).unwrap_or_default();
                }
                self.settings_from = Some(SettingsFrom::Button);
                vec![BackendCall::new("settings_open", json!({ "page": "general" }))]
            }
        }
    }

    /// Answer of `settings_open`. A rejected command arrives as `{ "__error": ".." }` (the in-pill Settings tab is the fallback).
    pub fn on_result(&mut self, cmd: &str, _args: &Value, res: &Value) -> Vec<BackendCall> {
        if cmd != "settings_open" {
            return vec![];
        }
        let ok = res.get(ERR_KEY).is_none();
        match (self.settings_from.take(), ok) {
            (Some(SettingsFrom::Button), true) => {
                self.fx.push(Fx::Collapse { via: None }); // doCollapse(true)
                vec![]
            }
            (Some(SettingsFrom::Button), false) => {
                self.chosen = true;
                self.activate("settings", true)
            }
            (Some(SettingsFrom::Expand), true) => {
                if self.frame_exp || self.expanded {
                    if self.expanded {
                        self.fx.push(Fx::Collapse { via: None });
                    } else {
                        return vec![BackendCall::new("pill_set_state", json!({ "state": "collapsed" }))];
                    }
                }
                vec![]
            }
            (Some(SettingsFrom::Expand), false) => self.do_expand(Some("settings")),
            (None, _) => vec![],
        }
    }

    // ---------------------------------------------------------------- view
    pub fn view(&self, tr: &dyn Tr) -> HeaderView {
        HeaderView {
            tabs: self
                .visible()
                .into_iter()
                .map(|s| {
                    let active = self.cur == Some(s.id);
                    TabButton {
                        id: s.id,
                        icon: s.icon,
                        title: tr.t(s.ru, s.en),
                        active,
                        badge: if active { self.badges.iter().find(|(i, _)| i == s.id).map(|(_, t)| t.clone()).unwrap_or_default() } else { String::new() },
                    }
                })
                .collect(),
            dictate_title: tr.t("Диктовка", "Dictation"),
            pin_title: tr.t("Закрепить", "Pin open"),
            pin_on: self.pinned,
            settings_title: tr.t("Настройки", "Settings"),
            settings_current: self.cur == Some("settings"),
            vpn: self.cfg.str("vpnCountry").to_uppercase().chars().take(3).collect(),
            dir: if self.dir_right { 'r' } else { 'l' },
        }
    }

    /// Title of a registered tab (for the shell's own use, e.g. window titles).
    pub fn title(&self, id: &str, tr: &dyn Tr) -> String {
        self.spec(id).map(|s| tr.t(s.ru, s.en)).unwrap_or_else(|| id.to_string())
    }
}

/// Convenience for tests and callers: the string field of a payload.
pub fn tab_of(payload: &Value) -> &str {
    str_of(payload, "tab")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::anim::AnimKind;
    use crate::panel::a::{EnTr, RuTr};

    fn ids(t: &Tabs) -> Vec<&'static str> {
        t.visible().iter().map(|s| s.id).collect()
    }

    fn open() -> Tabs {
        let mut t = Tabs::new();
        t.expand(None, None);
        t.take_events();
        t.take_fx();
        t
    }

    #[test]
    fn registry_order_and_default_tab() {
        let t = Tabs::new();
        assert_eq!(ids(&t), ["home", "clipboard", "shelf", "text", "chat", "note", "weather", "notifs", "agents", "controls"]);
        assert_eq!(t.current(), Some("home"));
        assert!(!ids(&t).contains(&"settings")); // hidden fallback
        let v = t.view(&RuTr);
        assert_eq!(v.tabs[0].title, "Главная");
        assert_eq!(v.tabs.iter().find(|b| b.id == "notifs").unwrap().icon, "bell");
        assert_eq!(t.view(&EnTr).tabs[7].title, "Notifications");
    }

    #[test]
    fn modules_hide_tabs_live_and_home_needs_media_or_launcher() {
        let mut t = Tabs::new();
        t.on_event("settings:changed", &json!({ "module.clipboard": false, "module.weather": false, "live.agents": false }));
        assert!(!ids(&t).contains(&"clipboard") && !ids(&t).contains(&"weather") && !ids(&t).contains(&"agents"));
        t.on_event("settings:changed", &json!({ "module.media": false }));
        assert!(ids(&t).contains(&"home")); // launcher still on
        t.on_event("settings:changed", &json!({ "module.media": false, "module.launcher": false }));
        assert!(!ids(&t).contains(&"home"));
        // (settings:changed carries the whole object, so the earlier switches are back on) the open tab went off: first remaining
        assert_eq!(t.current(), Some("clipboard"));
        t.on_event("settings:changed", &json!({}));
        assert_eq!(ids(&t).len(), 10);
        assert_eq!(t.current(), Some("clipboard")); // coming back does not steal the tab
    }

    #[test]
    fn unknown_module_keys_are_ignored_and_home_key_pair_matters() {
        let cfg = {
            let mut c = Cfg::new();
            c.apply(&json!({ "module.fileSearch": false, "module.uninstall": false }));
            c
        };
        for s in standard_tabs() {
            assert!(!Tabs::tab_off(&cfg, s.id), "{}", s.id);
        }
    }

    #[test]
    fn clicking_a_tab_persists_it_and_slides_in_the_right_direction() {
        let mut t = open();
        let c = t.on_action(Action::SelectTab("shelf".into()), &EnTr);
        assert_eq!(c, vec![BackendCall::new("settings_set", json!({ "patch": { "lastTab": "shelf" } }))]);
        assert_eq!(t.take_events(), [TabEvent::Hide("home".into()), TabEvent::Show("shelf".into()), TabEvent::Retarget]);
        assert_eq!(t.view(&EnTr).dir, 'r');
        let fx = t.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "view:home" && anim.kind == AnimKind::SlideAway { dx: -10.0 })));
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "view:shelf" && anim.kind == AnimKind::SlideIn { dx: 10.0 } && anim.delay_ms == 30)));
        // same tab again: nothing; a tab to the left slides the other way
        assert!(t.on_action(Action::SelectTab("shelf".into()), &EnTr).is_empty());
        t.on_action(Action::SelectTab("clipboard".into()), &EnTr);
        assert_eq!(t.view(&EnTr).dir, 'l');
        // lastTab is only written when it changes
        t.on_action(Action::SelectTab("shelf".into()), &EnTr);
        let c = t.on_action(Action::SelectTab("clipboard".into()), &EnTr);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn reduced_motion_keeps_the_switch_but_drops_the_cues() {
        let mut t = open();
        t.on_event("settings:changed", &json!({ "reduceMotion": true }));
        t.take_events();
        t.on_action(Action::SelectTab("text".into()), &EnTr);
        assert!(t.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
        assert_eq!(t.current(), Some("text"));
    }

    #[test]
    fn switching_while_collapsed_changes_nothing_visible() {
        let mut t = Tabs::new();
        t.on_action(Action::SelectTab("chat".into()), &EnTr);
        assert_eq!(t.current(), Some("chat"));
        assert!(t.take_events().is_empty());
        assert!(t.take_fx().is_empty());
        // opening shows the current tab once
        t.expand(None, None);
        assert_eq!(t.take_events(), [TabEvent::Show("chat".into()), TabEvent::Retarget]);
    }

    #[test]
    fn expand_with_a_tab_selects_it_and_remembers_it() {
        let mut t = Tabs::new();
        let c = t.on_event("pill:expand", &json!({ "tab": "clipboard", "via": "hotkey" }));
        assert_eq!(c[0].args, json!({ "patch": { "lastTab": "clipboard" } }));
        assert!(t.is_expanded());
        assert_eq!(t.current(), Some("clipboard"));
        // already expanded on that tab: just stays
        t.take_events();
        assert!(t.on_event("pill:expand", &json!({ "tab": "clipboard" })).is_empty());
        assert!(t.take_events().is_empty());
        // a switched-off tab is ignored
        t.on_event("settings:changed", &json!({ "module.shelf": false }));
        t.on_event("pill:expand", &json!({ "tab": "shelf" }));
        assert_eq!(t.current(), Some("clipboard"));
    }

    #[test]
    fn expanding_on_the_current_tab_fires_show_once() {
        let mut t = Tabs::new();
        t.on_event("pill:expand", &json!({ "tab": "home" }));
        assert_eq!(t.take_events(), [TabEvent::Show("home".into()), TabEvent::Retarget]);
        t.on_event("pill:expand", &json!({ "tab": "home" }));
        assert!(t.take_events().is_empty());
    }

    #[test]
    fn collapse_hides_the_current_tab_unless_pinned() {
        let mut t = open();
        t.on_action(Action::Pin, &EnTr);
        assert!(t.pinned());
        t.on_event("pill:collapse", &Value::Null);
        assert!(t.is_expanded());
        t.on_action(Action::Pin, &EnTr);
        t.take_fx();
        t.on_event("pill:collapse", &Value::Null);
        assert!(!t.is_expanded());
        assert_eq!(t.take_events(), [TabEvent::Hide("home".into())]);
        // hidden by Rust: no animation, same bookkeeping, idempotent
        t.on_event("pill:hide", &Value::Null);
        assert!(t.take_events().is_empty());
    }

    #[test]
    fn pin_button_and_shelf_pinning() {
        let mut t = open();
        t.on_action(Action::Pin, &EnTr);
        assert_eq!(t.take_fx(), vec![Fx::SetPinned(true)]);
        assert!(t.view(&EnTr).pin_on);
        t.set_pinned(false);
        assert!(!t.view(&EnTr).pin_on);
        assert_eq!(t.view(&RuTr).pin_title, "Закрепить");
        // the setting seeds it at startup
        let mut t2 = Tabs::new();
        t2.on_settings(&json!({ "pinned": true }), true);
        assert!(t2.pinned());
    }

    #[test]
    fn startup_reopens_on_the_last_tab_unless_one_was_chosen() {
        let mut t = Tabs::new();
        t.on_settings(&json!({ "lastTab": "weather" }), true);
        assert_eq!(t.current(), Some("weather"));
        // a switched-off or unknown last tab is ignored
        let mut t = Tabs::new();
        t.on_settings(&json!({ "lastTab": "weather", "module.weather": false }), true);
        assert_eq!(t.current(), Some("home"));
        let mut t = Tabs::new();
        t.on_settings(&json!({ "lastTab": "nope" }), true);
        assert_eq!(t.current(), Some("home"));
        let mut t = Tabs::new();
        t.on_settings(&json!({ "lastTab": "settings" }), true); // the hidden fallback is never restored
        assert_eq!(t.current(), Some("home"));
        let mut t = Tabs::new();
        t.on_action(Action::SelectTab("chat".into()), &EnTr);
        t.on_settings(&json!({ "lastTab": "weather" }), true);
        assert_eq!(t.current(), Some("chat"));
        // startup restore does not write the setting back
        let mut t = Tabs::new();
        t.on_settings(&json!({ "lastTab": "note" }), true);
        assert!(t.take_events().is_empty());
    }

    #[test]
    fn badge_shows_on_the_active_tab_only() {
        let mut t = Tabs::new();
        t.set_badge("notifs", "3");
        assert!(t.view(&EnTr).tabs.iter().all(|b| b.badge.is_empty()));
        t.on_action(Action::SelectTab("notifs".into()), &EnTr);
        assert_eq!(t.view(&EnTr).tabs.iter().find(|b| b.id == "notifs").unwrap().badge, "3");
        t.set_badge("notifs", "99+");
        assert_eq!(t.view(&EnTr).tabs.iter().find(|b| b.id == "notifs").unwrap().badge, "99+");
        t.set_badge("notifs", "");
        assert!(t.view(&EnTr).tabs.iter().all(|b| b.badge.is_empty()));
        t.set_badge("ghost", "1"); // unknown tab: harmless
    }

    #[test]
    fn dictation_button_is_a_stub_until_a_handler_exists() {
        let mut t = Tabs::new();
        t.on_action(Action::Dictate { available: true }, &EnTr);
        assert!(t.take_fx().is_empty());
        t.on_action(Action::Dictate { available: false }, &RuTr);
        assert_eq!(t.take_fx(), vec![Fx::Toast { id: "dict-soon", icon: "wave", title: "Диктовка скоро".into(), ttl_ms: 1800 }]);
        assert_eq!(t.view(&EnTr).dictate_title, "Dictation");
    }

    #[test]
    fn vpn_chip_shows_a_three_letter_country() {
        let mut t = Tabs::new();
        assert_eq!(t.view(&EnTr).vpn, "");
        t.on_event("settings:changed", &json!({ "vpnCountry": "de" }));
        assert_eq!(t.view(&EnTr).vpn, "DE");
        t.on_event("settings:changed", &json!({ "vpnCountry": "germany" }));
        assert_eq!(t.view(&EnTr).vpn, "GER");
    }

    #[test]
    fn settings_button_opens_the_window_and_collapses() {
        let mut t = open();
        let c = t.on_action(Action::Settings, &EnTr);
        assert_eq!(c, vec![BackendCall::new("settings_open", json!({ "page": "general" }))]);
        assert!(t.on_result("settings_open", &Value::Null, &Value::Null).is_empty());
        assert_eq!(t.take_fx(), vec![Fx::Collapse { via: None }]);
    }

    #[test]
    fn settings_window_unavailable_falls_back_to_the_in_pill_tab_and_back() {
        let mut t = open();
        t.on_action(Action::Settings, &EnTr);
        let c = t.on_result("settings_open", &Value::Null, &json!({ ERR_KEY: "no tauri" }));
        assert!(c.is_empty()); // settings is a hidden tab: lastTab is not written
        assert_eq!(t.current(), Some("settings"));
        assert!(t.view(&EnTr).settings_current);
        assert!(t.view(&EnTr).tabs.iter().all(|b| !b.active));
        // pressing the button again returns to the previous tab
        t.on_action(Action::Settings, &EnTr);
        assert_eq!(t.current(), Some("home"));
        assert!(!t.view(&EnTr).settings_current);
        // settings sorts last, so going there slides right and coming back slides left
        assert_eq!(t.view(&EnTr).dir, 'l');
    }

    #[test]
    fn command_bar_settings_request_opens_a_window_page() {
        let mut t = Tabs::new();
        let c = t.on_event("pill:open-tab", &json!({ "tab": "settings", "section": "voice" }));
        assert_eq!(c, vec![BackendCall::new("settings_open", json!({ "page": "voice" }))]);
        // an unknown section becomes general
        let c = t.on_event("pill:expand", &json!({ "tab": "settings", "section": "bogus" }));
        assert_eq!(c[0].args, json!({ "page": "general" }));
        // other tabs in open-tab are not ours
        assert!(t.on_event("pill:open-tab", &json!({ "tab": "note", "noteId": 4 })).is_empty());
        assert!(!t.is_expanded());
    }

    #[test]
    fn settings_hand_off_collapses_an_open_island_or_returns_the_frame() {
        let mut t = open();
        t.on_event("pill:open-tab", &json!({ "tab": "settings" }));
        t.on_result("settings_open", &Value::Null, &Value::Null);
        assert_eq!(t.take_fx(), vec![Fx::Collapse { via: None }]);
        // frame still expanded after a collapse: tell Rust to give it back
        let mut t = Tabs::new();
        t.set_frame_expanded(true);
        t.on_event("pill:open-tab", &json!({ "tab": "settings" }));
        let c = t.on_result("settings_open", &Value::Null, &Value::Null);
        assert_eq!(c, vec![BackendCall::new("pill_set_state", json!({ "state": "collapsed" }))]);
        // nothing expanded at all: nothing to do
        let mut t = Tabs::new();
        t.on_event("pill:open-tab", &json!({ "tab": "settings" }));
        assert!(t.on_result("settings_open", &Value::Null, &Value::Null).is_empty());
        // window unavailable: expand into the in-pill Settings tab
        let mut t = Tabs::new();
        t.on_event("pill:open-tab", &json!({ "tab": "settings" }));
        t.on_result("settings_open", &Value::Null, &json!({ ERR_KEY: "x" }));
        assert!(t.is_expanded());
        assert_eq!(t.current(), Some("settings"));
    }

    #[test]
    fn request_expand_is_a_pill_set_state_call() {
        assert_eq!(Tabs::request_expand(Some("shelf"), Some("hover")), BackendCall::new("pill_set_state", json!({ "state": "expanded", "tab": "shelf", "via": "hover" })));
        assert_eq!(Tabs::request_expand(None, None).args, json!({ "state": "expanded", "tab": null, "via": "page" }));
    }

    #[test]
    fn body_heights() {
        let mut t = Tabs::new();
        assert_eq!(t.body_height(), Some(331));
        t.on_action(Action::SelectTab("chat".into()), &EnTr);
        assert_eq!(t.body_height(), Some(417));
        t.on_action(Action::SelectTab("weather".into()), &EnTr);
        assert_eq!(t.body_height(), Some(417)); // register({height}) wins over the 413 table value
        t.on_action(Action::SelectTab("note".into()), &EnTr);
        assert_eq!(t.body_height(), Some(417));
        t.on_action(Action::SelectTab("agents".into()), &EnTr);
        assert_eq!(t.body_height(), None); // shell default
    }

    #[test]
    fn module_toggle_while_expanded_retargets() {
        let mut t = open();
        t.on_event("settings:changed", &json!({ "module.chat": false }));
        assert_eq!(t.take_events(), [TabEvent::Retarget]);
        t.on_event("settings:changed", &json!({ "module.chat": false }));
        assert!(t.take_events().is_empty());
        // switching off the open tab: it hides, the first remaining shows, then one retarget
        t.on_action(Action::SelectTab("text".into()), &EnTr);
        t.take_events();
        t.on_event("settings:changed", &json!({ "module.chat": false, "module.text": false }));
        assert_eq!(t.take_events(), [TabEvent::Hide("text".into()), TabEvent::Show("home".into()), TabEvent::Retarget, TabEvent::Retarget]);
    }

    #[test]
    fn last_tab_id_follows_clicks() {
        let mut t = Tabs::new();
        assert_eq!(t.last_tab_id(), "");
        t.on_action(Action::SelectTab("shelf".into()), &EnTr);
        assert_eq!(t.last_tab_id(), "shelf");
    }

    #[test]
    fn titles_by_id() {
        let t = Tabs::new();
        assert_eq!(t.title("controls", &RuTr), "Управление");
        assert_eq!(t.title("nope", &RuTr), "nope");
        assert_eq!(tab_of(&json!({ "tab": "x" })), "x");
    }
}
