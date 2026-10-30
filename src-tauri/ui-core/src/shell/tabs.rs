//! Tab registry bookkeeping of pill.js (`register`, `visibleTabs`, `tabOff`, `activate` order/direction, `tabH`).
//!
//! The tab CONTENT belongs to the panel area; the shell only needs the order, visibility (Settings > Modules switches), the current tab,
//! its body height (clamped to the OS frame) and the slide direction of a switch.

use crate::shell::settings::ShellSettings;
use crate::shell::types::TabDir;

/// Default expanded body height (`CFG.defH`).
pub const DEF_H_FALLBACK: f64 = 331.0;
/// `DEF_H[id]`: body height of a tab that did not register its own.
pub fn default_height(id: &str) -> f64 {
    match id {
        "note" | "chat" => 417.0,
        "weather" => 413.0,
        _ => DEF_H_FALLBACK,
    }
}

/// `MODTAB`: the module switch that hides each tab.
pub fn module_key(id: &str) -> Option<&'static str> {
    match id {
        "clipboard" => Some("module.clipboard"),
        "shelf" => Some("module.shelf"),
        "text" => Some("module.text"),
        "chat" => Some("module.chat"),
        "note" => Some("module.note"),
        "weather" => Some("module.weather"),
        "controls" => Some("module.controls"),
        "notifs" => Some("module.notifs"),
        "agents" => Some("live.agents"),
        _ => None,
    }
}

/// `tabOff(id)`: Home stays while the player OR the launcher is on.
pub fn tab_off(id: &str, s: &ShellSettings) -> bool {
    if id == "home" {
        return !s.module_on("module.media") && !s.module_on("module.launcher");
    }
    match module_key(id) {
        Some(k) => !s.module_on(k),
        None => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabSpec {
    pub id: String,
    pub title: String,
    pub order: i32,
    pub hidden: bool,
    /// own body height (css px at 100 %)
    pub height: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Tab {
    pub spec: TabSpec,
    pub off: bool,
    pub badge: u32,
    pub warned: bool,
}

#[derive(Debug, Clone)]
pub struct Tabs {
    pub list: Vec<Tab>,
    pub cur: Option<String>,
    pub prev: Option<String>,
    /// the user (or code) picked a tab: stop auto-selecting the first one
    pub chosen: bool,
    pub dir: TabDir,
}

impl Default for Tabs {
    fn default() -> Self {
        Tabs { list: Vec::new(), cur: None, prev: None, chosen: false, dir: TabDir::Right }
    }
}

impl Tabs {
    pub fn get(&self, id: &str) -> Option<&Tab> {
        self.list.iter().find(|t| t.spec.id == id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Tab> {
        self.list.iter_mut().find(|t| t.spec.id == id)
    }
    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }
    /// `visibleTabs()`: not hidden, not off, sorted by `order` (stable).
    pub fn visible(&self) -> Vec<&Tab> {
        let mut v: Vec<&Tab> = self.list.iter().filter(|t| !t.spec.hidden && !t.off).collect();
        v.sort_by_key(|t| t.spec.order);
        v
    }
    pub fn first_visible(&self) -> Option<String> {
        self.visible().first().map(|t| t.spec.id.clone())
    }
    /// Slide direction when switching from `old` to `new`: order = visible tabs, then the settings tab.
    pub fn direction(&self, old: &str, new: &str) -> TabDir {
        let mut order: Vec<&str> = self.visible().iter().map(|t| t.spec.id.as_str()).collect();
        order.push("settings");
        let pos = |id: &str| order.iter().position(|x| *x == id);
        // indexOf of a missing tab is -1
        let (o, n) = (pos(old).map_or(-1, |i| i as i32), pos(new).map_or(-1, |i| i as i32));
        if n > o {
            TabDir::Right
        } else {
            TabDir::Left
        }
    }
    /// `tabH()`: the body height of the current tab, clamped to the OS frame; the bool says the clamp kicked in (warn once).
    pub fn body_height(&mut self, exp_frame_h: f64, exp_top: f64, wave_pad: f64) -> (f64, bool) {
        let cur = self.cur.clone();
        let want = match cur.as_deref().and_then(|id| self.get(id)) {
            Some(t) => t.spec.height.filter(|h| *h != 0.0).unwrap_or_else(|| default_height(&t.spec.id)),
            None => DEF_H_FALLBACK,
        };
        let max = exp_frame_h - exp_top - wave_pad;
        let mut warned_now = false;
        if want > max {
            if let Some(t) = cur.as_deref().and_then(|id| self.get_mut(id)) {
                if !t.warned {
                    t.warned = true;
                    warned_now = true;
                }
            }
        }
        (want.min(max), warned_now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tab(id: &str, order: i32, hidden: bool) -> Tab {
        Tab { spec: TabSpec { id: id.into(), title: id.into(), order, hidden, height: None }, off: false, badge: 0, warned: false }
    }

    #[test]
    fn visible_sorts_by_order_and_skips_hidden_and_off() {
        let mut t = Tabs { list: vec![tab("note", 30, false), tab("home", 10, false), tab("settings", 900, true), tab("chat", 20, false)], ..Default::default() };
        t.list[3].off = true;
        let ids: Vec<_> = t.visible().iter().map(|x| x.spec.id.clone()).collect();
        assert_eq!(ids, vec!["home", "note"]);
        assert_eq!(t.first_visible().as_deref(), Some("home"));
    }

    #[test]
    fn slide_direction() {
        let t = Tabs { list: vec![tab("home", 10, false), tab("note", 20, false), tab("settings", 900, true)], ..Default::default() };
        assert_eq!(t.direction("home", "note"), TabDir::Right);
        assert_eq!(t.direction("note", "home"), TabDir::Left);
        assert_eq!(t.direction("note", "settings"), TabDir::Right); // settings is after every visible tab
        assert_eq!(t.direction("settings", "home"), TabDir::Left);
    }

    #[test]
    fn default_and_own_heights_clamp_to_the_frame() {
        let mut t = Tabs::default();
        let mut c = tab("note", 1, false);
        c.spec.height = None;
        t.list = vec![c, tab("home", 2, false)];
        t.cur = Some("note".into());
        assert_eq!(t.body_height(480.0, 8.0, 28.0), (417.0, false));
        t.cur = Some("home".into());
        assert_eq!(t.body_height(480.0, 8.0, 28.0), (331.0, false));
        t.cur = None;
        assert_eq!(t.body_height(480.0, 8.0, 28.0).0, 331.0);
        // own height beyond the frame (480 - 8 - 28 = 444): clamped, warns once
        t.get_mut("home").unwrap().spec.height = Some(600.0);
        t.cur = Some("home".into());
        assert_eq!(t.body_height(480.0, 8.0, 28.0), (444.0, true));
        assert_eq!(t.body_height(480.0, 8.0, 28.0), (444.0, false));
        assert_eq!(default_height("weather"), 413.0);
        assert_eq!(default_height("clipboard"), 331.0);
    }

    #[test]
    fn module_switches_hide_tabs() {
        let s = ShellSettings::from_json(&json!({"module.chat": false, "live.agents": false, "module.media": false}));
        assert!(tab_off("chat", &s));
        assert!(tab_off("agents", &s));
        assert!(!tab_off("shelf", &s));
        assert!(!tab_off("home", &s)); // the launcher is still on
        let s = ShellSettings::from_json(&json!({"module.media": false, "module.launcher": false}));
        assert!(tab_off("home", &s));
        assert!(!tab_off("unknown", &s));
    }
}
