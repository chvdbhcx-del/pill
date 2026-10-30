//! Panel set A: the logic of the Pill tabs Home, Weather, Controls, Notifications, Clipboard, Shelf, the Home
//! launcher strip, and the tab registry / header model (`tabs`). Ported from `src/tab-*.js` and the tab part of `src/pill.js`.
//!
//! Conventions shared by every model here (so a renderer can drive them all the same way):
//! * **View-model**: `view(...)` returns plain structs of what to show. No drawing, no strings in a fixed language:
//!   text comes through the [`Tr`] trait (ru, en).
//! * **Actions**: user interactions are an `Action` enum per tab. `on_action(&mut self, a) -> Vec<BackendCall>` returns the
//!   backend commands (the exact `invoke` names and argument keys the JS passes, see `docs/native-ui-contract.md`) to run.
//! * **Results**: the renderer runs each [`BackendCall`] and feeds the answer back with `on_result(cmd, args, result)`;
//!   backend events go in through `on_event(name, payload)`; both may return follow-up calls.
//! * **Time**: `tick(dt_ms)` advances the model's own monotonic clock (seek bar, volume glide, 3 s "press again" arms,
//!   polling intervals, debounce timers) and returns calls that became due. Models never read a clock themselves.
//! * **Effects**: things that are not backend calls (toast, collapse the island, live-row widget, badge, animation cue) are
//!   queued as [`Fx`] and drained with `take_fx()`.
//! * **Animation**: every animated change pushes an `Fx::Anim` with data from [`anim`]; under Reduce motion no cue is
//!   pushed at all (the renderer shows the end state). Reduce motion = setting `reduceMotion` or the OS flag (`Cfg::os_reduce`).

pub mod anim;
pub mod clipboard;
pub mod controls;
pub mod cover;
pub mod home;
pub mod launcher;
pub mod notifs;
pub mod shelf;
pub mod tabs;
pub mod util;
pub mod weather;

use serde_json::{Map, Value};

pub use anim::Anim;
pub use util::Wall;

/// Two-language text lookup: `t("Главная", "Home")`. The i18n agent's string table plugs in behind this.
pub trait Tr {
    fn is_ru(&self) -> bool;
    fn t(&self, ru: &str, en: &str) -> String {
        if self.is_ru() { ru } else { en }.to_string()
    }
}

/// Russian.
pub struct RuTr;
/// English.
pub struct EnTr;
impl Tr for RuTr {
    fn is_ru(&self) -> bool {
        true
    }
}
impl Tr for EnTr {
    fn is_ru(&self) -> bool {
        false
    }
}

/// One backend command: the Tauri command name and its argument object (JS-side camelCase names, as `invoke` takes them).
#[derive(Clone, Debug, PartialEq)]
pub struct BackendCall {
    pub cmd: &'static str,
    pub args: Value,
}

impl BackendCall {
    pub fn new(cmd: &'static str, args: Value) -> BackendCall {
        BackendCall { cmd, args }
    }
    /// A command without arguments.
    pub fn bare(cmd: &'static str) -> BackendCall {
        BackendCall { cmd, args: Value::Null }
    }
}

/// Settings snapshot (`settings_get` / `settings:changed` payload) with the JS pages' truthiness rules.
#[derive(Clone, Debug, Default)]
pub struct Cfg {
    map: Map<String, Value>,
    /// `prefers-reduced-motion` of the OS, set by the renderer.
    pub os_reduce: bool,
}

impl Cfg {
    pub fn new() -> Cfg {
        Cfg::default()
    }
    /// Replace everything (the payload of `settings_get` / `settings:changed` is the whole object). Non-objects are ignored.
    pub fn apply(&mut self, v: &Value) -> bool {
        match v.as_object() {
            Some(m) => {
                self.map = m.clone();
                true
            }
            None => false,
        }
    }
    pub fn set(&mut self, key: &str, v: Value) {
        self.map.insert(key.to_string(), v);
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key)
    }
    /// `cfg[key] !== false` (modules and most switches default to on).
    pub fn not_false(&self, key: &str) -> bool {
        self.map.get(key) != Some(&Value::Bool(false))
    }
    /// `!!cfg[key]`
    pub fn truthy(&self, key: &str) -> bool {
        match self.map.get(key) {
            None | Some(Value::Null) | Some(Value::Bool(false)) => false,
            Some(Value::Number(n)) => n.as_f64().map_or(true, |f| f != 0.0),
            Some(Value::String(s)) => !s.is_empty(),
            Some(_) => true,
        }
    }
    pub fn str(&self, key: &str) -> &str {
        self.map.get(key).and_then(Value::as_str).unwrap_or("")
    }
    pub fn module_on(&self, key: &str) -> bool {
        self.not_false(key)
    }
    /// `Pill.liveOn(k)`: `live.<k>` when set, else on for everything except OBS.
    pub fn live_on(&self, k: &str) -> bool {
        match self.map.get(&format!("live.{k}")) {
            None => k != "obs",
            Some(_) => self.truthy(&format!("live.{k}")),
        }
    }
    pub fn reduce_motion(&self) -> bool {
        self.truthy("reduceMotion") || self.os_reduce
    }
    pub fn clock24(&self) -> bool {
        self.not_false("clock24")
    }
}

/// A row for the collapsed island (`Pill.setLive`), described as data; the live-row agent's renderer draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveSpec {
    pub priority: u8,
    pub ttl_ms: Option<u32>,
    /// The island grows to the card's own height (`tall`).
    pub tall: bool,
    pub accent: String,
    pub body: LiveBody,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LiveBody {
    /// Music widget. `mode` is `title` (equalizer + name) or `mini` (cover, name, prev / play / next buttons).
    Music { mode: MusicMode, title: String, playing: bool, cover: Option<String>, eq_color: String },
    /// "N files" chip after the shelf count changed.
    ShelfCount { count: usize },
    /// Compact drop card while a file drag hovers the folded island.
    ShelfDrop(DropCard),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicMode {
    Title,
    Mini,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropCard {
    pub title: String,
    pub sub: String,
    /// `idle` | `go` (files are flying in) | `ok` | `dup`
    pub phase: &'static str,
}

/// Everything a tab asks the shell / renderer to do that is not a backend command.
#[derive(Clone, Debug, PartialEq)]
pub enum Fx {
    Toast { id: &'static str, icon: &'static str, title: String, ttl_ms: u32 },
    /// `Pill.collapse(via)`; `via` is `Some("click")` for a click that closes the island.
    Collapse { via: Option<&'static str> },
    /// `Pill.expand(tab)`
    Expand { tab: &'static str },
    Live { key: &'static str, spec: Option<LiveSpec> },
    /// `Pill.react(key, kind, {color, ttlMs})`: the pet / edge reaction
    React { key: &'static str, kind: &'static str, color: Option<String>, ttl_ms: Option<u32> },
    Unreact { key: &'static str },
    /// `Pill.setBadge(tab, text)`; empty text hides the badge.
    Badge { tab: &'static str, text: String },
    SetPinned(bool),
    OpenCommandBar,
    /// `PillCapture.start(mode)` (collapses the pill, then `capture_start`)
    StartCapture { mode: &'static str },
    /// AI-actions panel over the clipboard row (set B draws it)
    OpenClipAi { id: i64 },
    CloseClipAi,
    DragPull(bool),
    EdgeColor(Option<String>),
    /// Put the keyboard caret in the tab's search box.
    FocusSearch,
    BlurSearch,
    /// Scroll the named element of the tab into view (keyboard selection).
    ScrollTo { target: String },
    /// Play an animation on a named element of the tab (`title`, `art`, `row:<id>`, `chip:<id>`, `list`, ...).
    Anim { target: String, anim: Anim },
}

/// Queue of [`Fx`] with the reduce-motion gate built in.
#[derive(Clone, Debug, Default)]
pub struct FxQueue(Vec<Fx>);

impl FxQueue {
    pub fn push(&mut self, fx: Fx) {
        self.0.push(fx);
    }
    /// Queue an animation unless motion is reduced.
    pub fn anim(&mut self, reduce: bool, target: impl Into<String>, anim: Anim) {
        if let Some(a) = anim.gate(reduce) {
            self.0.push(Fx::Anim { target: target.into(), anim: a });
        }
    }
    pub fn toast(&mut self, id: &'static str, icon: &'static str, title: String, ttl_ms: u32) {
        self.0.push(Fx::Toast { id, icon, title, ttl_ms });
    }
    pub fn take(&mut self) -> Vec<Fx> {
        std::mem::take(&mut self.0)
    }
    pub fn peek(&self) -> &[Fx] {
        &self.0
    }
}

/// Shared tiny helpers.
pub(crate) fn i64_of(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}
pub(crate) fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
pub(crate) fn bool_of(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cfg_truthiness_matches_the_js_pages() {
        let mut c = Cfg::new();
        c.apply(&json!({ "module.media": false, "net.weather": true, "name": "", "n": 0, "clock24": false }));
        assert!(!c.not_false("module.media"));
        assert!(c.not_false("module.shelf")); // unset = on
        assert!(c.truthy("net.weather") && !c.truthy("name") && !c.truthy("n") && !c.truthy("missing"));
        assert!(!c.clock24());
    }

    #[test]
    fn live_on_defaults_everything_but_obs() {
        let mut c = Cfg::new();
        assert!(c.live_on("music") && !c.live_on("obs"));
        c.apply(&json!({ "live.music": false, "live.obs": true }));
        assert!(!c.live_on("music") && c.live_on("obs"));
    }

    #[test]
    fn reduce_motion_from_setting_or_os() {
        let mut c = Cfg::new();
        assert!(!c.reduce_motion());
        c.os_reduce = true;
        assert!(c.reduce_motion());
        c.os_reduce = false;
        c.apply(&json!({ "reduceMotion": true }));
        assert!(c.reduce_motion());
    }

    #[test]
    fn fx_queue_gates_animation() {
        let mut q = FxQueue::default();
        q.anim(true, "x", Anim::text_swap());
        assert!(q.peek().is_empty());
        q.anim(false, "x", Anim::text_swap());
        assert_eq!(q.take().len(), 1);
        assert!(q.take().is_empty());
    }

    #[test]
    fn tr_picks_language() {
        assert_eq!(RuTr.t("Главная", "Home"), "Главная");
        assert_eq!(EnTr.t("Главная", "Home"), "Home");
    }
}
