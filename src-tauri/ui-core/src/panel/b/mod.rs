//! Panel set B: view-models of the Note (notes + timer + reminders), Text, Chat (+ AI settings popover), Agents tabs,
//! the clipboard AI-actions panel and the Settings > AI setup page.
//!
//! Ported from src/tab-note.js, tab-text.js (+ tab-text.test.mjs), tab-chat.js, agents-live.js, ai-actions.js, ai-setup.js.
//! Not ported: md-lite.js / the markdown renderer inside tab-chat.js (the kit's RichText parser draws raw message text).
//!
//! Shared conventions (every model follows them):
//! * `on_action(Action) -> Vec<BackendCall>`: the renderer reports a user gesture, the model answers with backend calls
//!   (`cmd` + `args` are exactly the Tauri invoke name / argument names the JS used).
//! * Every call carries a `tag`; the host runs it and feeds the result back with `on_reply(tag, Reply)`, which may return
//!   more calls (the JS `await` chains are flattened into small state machines).
//! * `on_event(name, payload)` takes the backend events (`chat:delta`, `remind:due`, ...), `tick(dt_ms)` advances the clock.
//! * Side effects that are not backend commands (copy to clipboard, toast cards, live-row specs, focus) are `Effect`s,
//!   drained with `take_effects()`; animations are data (`AnimCue`), drained with `take_anims()`. With `reduce_motion`
//!   set no cue is emitted and streaming text appears at once.
//! * Language: `Lang` implements `Tr` (`t(ru, en)`); models keep their `Lang` and compute labels in `view()`.
//! * Secrets: `BackendCall`'s `Debug` redacts `key`-like arguments; nothing here ever stores an API key.

use serde_json::Value;
use std::collections::HashMap;
use std::fmt;

pub mod agents;
pub mod ai_actions;
pub mod ai_setup;
pub mod chat;
pub mod chat_settings;
pub mod jsonlite;
pub mod note;
pub mod note_html;
pub mod qr;
pub mod text;
pub mod timer;

/// Translation by (ru, en) pair, like `Pill.t({ru, en})`.
pub trait Tr {
    fn is_ru(&self) -> bool;
    fn t<'a>(&self, ru: &'a str, en: &'a str) -> &'a str {
        if self.is_ru() { ru } else { en }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lang {
    Ru,
    #[default]
    En,
}
impl Tr for Lang {
    fn is_ru(&self) -> bool {
        *self == Lang::Ru
    }
}

/// A backend command to run (Tauri `invoke` today, a direct call in the native app). `tag` correlates the reply.
#[derive(Clone, PartialEq)]
pub struct BackendCall {
    pub cmd: &'static str,
    pub args: Value,
    pub tag: u64,
}
impl BackendCall {
    pub fn new(cmd: &'static str, args: Value, tag: u64) -> Self {
        Self { cmd, args, tag }
    }
}
impl fmt::Debug for BackendCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BackendCall {{ cmd: {:?}, tag: {}, args: {} }}", self.cmd, self.tag, redact(&self.args))
    }
}
/// Debug text of call arguments with every secret-looking field masked.
pub fn redact(v: &Value) -> String {
    fn go(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, x)| {
                        let l = k.to_lowercase();
                        let secret = l.contains("key") || l.contains("token") || l.contains("secret") || l.contains("password");
                        (k.clone(), if secret && !x.is_null() { Value::String("<redacted>".into()) } else { go(x) })
                    })
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(go).collect()),
            _ => v.clone(),
        }
    }
    go(v).to_string()
}

/// Result of a backend call: like the JS `call()` wrapper (`ok` + value, or the error text).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reply {
    pub ok: bool,
    pub v: Value,
    pub err: String,
}
impl Reply {
    pub fn ok(v: Value) -> Self {
        Self { ok: true, v, err: String::new() }
    }
    pub fn err(e: &str) -> Self {
        Self { ok: false, v: Value::Null, err: e.to_string() }
    }
}

/// Hands out tags: the high byte is the owner (so a composite tab can route replies), the rest a counter.
#[derive(Clone, Debug, Default)]
pub struct Tags {
    owner: u64,
    n: u64,
}
impl Tags {
    pub fn new(owner: u8) -> Self {
        Self { owner: (owner as u64) << 56, n: 0 }
    }
    pub fn next(&mut self) -> u64 {
        self.n += 1;
        self.owner | self.n
    }
    pub fn owner_of(tag: u64) -> u8 {
        (tag >> 56) as u8
    }
}

/// Persistent key/value of the page (`localStorage` in the web UI); a fake in tests.
pub trait Kv {
    fn get(&self, k: &str) -> Option<String>;
    fn set(&mut self, k: &str, v: &str);
}
#[derive(Default, Clone, Debug)]
pub struct MemKv(pub HashMap<String, String>);
impl Kv for MemKv {
    fn get(&self, k: &str) -> Option<String> {
        self.0.get(k).cloned()
    }
    fn set(&mut self, k: &str, v: &str) {
        self.0.insert(k.to_string(), v.to_string());
    }
}

/// A `Kv` shared by several models of one tab (single-threaded: UI thread only).
#[derive(Clone)]
pub struct SharedKv(pub std::rc::Rc<std::cell::RefCell<dyn Kv>>);
impl SharedKv {
    pub fn new<K: Kv + 'static>(k: K) -> Self {
        SharedKv(std::rc::Rc::new(std::cell::RefCell::new(k)))
    }
}
impl Kv for SharedKv {
    fn get(&self, k: &str) -> Option<String> {
        self.0.borrow().get(k)
    }
    fn set(&mut self, k: &str, v: &str) {
        self.0.borrow_mut().set(k, v)
    }
}

/// Animation kinds the renderer plays (durations/easing are the JS ones). `target` of a cue names the element/row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anim {
    /// fade + translate from `y` px to 0 (PillMotion.enter / the chat bubble enter: 180 ms ease-out-expo, y = 6)
    Enter { y: f32, ms: u32 },
    /// fade + translate out (PillMotion.exit)
    Exit { y: f32, scale: f32, ms: u32 },
    /// height collapse / expand (PillMotion.collapse / expand)
    Collapse,
    Expand,
    /// dot/badge pop from a smaller scale (PillMotion.pop)
    Pop { from_scale: f32, ms: u32 },
    /// rows slide to their new place, new rows drop in, gone ones fade (PillMotion.flip)
    Flip,
    /// list rows enter one after another (the `stag` class + `--i`)
    Stagger,
    /// button press feedback (scale dip)
    Press,
    /// three-dot typing indicator while the model has not produced text yet
    TypingDots,
    /// streaming caret at the end of the growing answer
    Caret,
    /// spinner (wait states)
    Spin,
    /// indeterminate progress bar sweep
    ProgressSweep,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AnimCue {
    pub target: String,
    pub kind: Anim,
}

/// A toast/card in the collapsed pill (Pill.toast).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ToastSpec {
    pub id: String,
    pub icon: String,
    pub title: String,
    pub sub: String,
    pub tone: Option<&'static str>,
    pub ttl_ms: u32,
    pub actions: Vec<ToastAction>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ToastAction {
    pub label: String,
    /// "white" for secondary buttons
    pub kind: Option<&'static str>,
    pub run: ToastRun,
}
/// What a toast button does; the host maps it back with `Model::on_toast_action`-like glue or runs the calls directly.
#[derive(Clone, Debug, PartialEq)]
pub enum ToastRun {
    Nothing,
    Call(&'static str, Value),
    CopyText(String),
    /// route back into the owning model as an action id
    Model(String),
}

/// Non-command side effects.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    CopyText(String),
    /// Pill.toast card
    Card(ToastSpec),
    /// Pill.setLive(key, spec) / None clears it
    SetLive { key: String, row: Option<LiveRow> },
    /// Pill.react(key, kind, ttl) / unreact
    React { key: String, kind: &'static str, ttl_ms: Option<u32> },
    Unreact(String),
    /// liquid edge ripple of this strength
    EdgePulse(f32),
    SetBadge { tab: &'static str, count: u32 },
    ScrollToBottom,
    ScrollIntoView(&'static str),
    Focus(&'static str),
    /// replace chars [start, end) of the focused text input through its edit pipeline (so its undo restores the original)
    ReplaceRange { start: usize, end: usize, text: String },
    /// set the whole contents of the note editor (sanitized html)
    SetEditorHtml(String),
    /// execute a rich-text command in the note editor
    EditorExec(&'static str),
    /// collapse the pill (after a paste)
    CollapsePill,
    /// start/stop dictation (PillVoice.toggle)
    VoiceToggle,
    /// jump to another Settings page (`go('voice')`, `go('ai')`)
    GoPage(&'static str),
}
#[derive(Clone, Debug, PartialEq)]
pub struct LiveRow {
    pub priority: u32,
    /// stable key: a changed key replays the swap animation, the same key only rewrites `text`
    pub key: String,
    pub text: String,
    pub accent: Option<&'static str>,
    pub ttl_ms: Option<u32>,
    pub style: &'static str,
}

/// Effects + animation cues + the motion preference, embedded in every model as `fx`.
#[derive(Clone, Debug, Default)]
pub struct Fx {
    pub effects: Vec<Effect>,
    pub anims: Vec<AnimCue>,
    pub reduce_motion: bool,
}
impl Fx {
    pub fn eff(&mut self, e: Effect) {
        self.effects.push(e);
    }
    pub fn anim(&mut self, target: &str, kind: Anim) {
        if !self.reduce_motion {
            self.anims.push(AnimCue { target: target.to_string(), kind });
        }
    }
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }
    pub fn take_anims(&mut self) -> Vec<AnimCue> {
        std::mem::take(&mut self.anims)
    }
}

/// A one-shot countdown driven by `tick` (setTimeout replacement).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timer(Option<u32>);
impl Timer {
    pub fn arm(&mut self, ms: u32) {
        self.0 = Some(ms);
    }
    pub fn cancel(&mut self) {
        self.0 = None;
    }
    pub fn armed(&self) -> bool {
        self.0.is_some()
    }
    /// Advance; true exactly once when it expires.
    pub fn tick(&mut self, dt: u32) -> bool {
        match self.0 {
            Some(left) if left <= dt => {
                self.0 = None;
                true
            }
            Some(left) => {
                self.0 = Some(left - dt);
                false
            }
            None => false,
        }
    }
}

/// A transient one-line message (toast inside a tab): text, tone and remaining time.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Flash {
    pub text: String,
    pub bad: bool,
    left: Option<u32>,
    shown: bool,
}
impl Flash {
    pub fn say(&mut self, text: &str, bad: bool, ttl: Option<u32>) {
        self.text = text.to_string();
        self.bad = bad;
        self.left = ttl;
        self.shown = true;
    }
    pub fn hide(&mut self) {
        self.shown = false;
        self.left = None;
    }
    pub fn visible(&self) -> bool {
        self.shown
    }
    pub fn tick(&mut self, dt: u32) {
        if let Some(l) = self.left {
            if l <= dt {
                self.hide();
            } else {
                self.left = Some(l - dt);
            }
        }
    }
}

/// `s.chars().take(n)` as a String (JS `slice(0, n)` on code points).
pub fn clip_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Russian plural form; English passes (one, many, many).
pub fn plural<'a>(n: i64, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    let a = n.abs() % 100;
    let b = a % 10;
    if a > 10 && a < 20 {
        many
    } else if b == 1 {
        one
    } else if (2..=4).contains(&b) {
        few
    } else {
        many
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn debug_redacts_keys() {
        let c = BackendCall::new("ai_set_key", json!({ "key": "sk-live-123" }), 1);
        let s = format!("{c:?}");
        assert!(!s.contains("sk-live-123") && s.contains("<redacted>"));
        let c = BackendCall::new("settings_set", json!({ "patch": { "ai.apiKeyEnc": "zzz", "ai.model": "m" } }), 2);
        let s = format!("{c:?}");
        assert!(!s.contains("zzz") && s.contains("\"m\""));
    }
    #[test]
    fn timer_and_flash() {
        let mut t = Timer::default();
        t.arm(400);
        assert!(!t.tick(399) && t.armed() && t.tick(1) && !t.armed() && !t.tick(10));
        let mut f = Flash::default();
        f.say("x", true, Some(100));
        f.tick(99);
        assert!(f.visible());
        f.tick(1);
        assert!(!f.visible());
    }
    #[test]
    fn plural_forms() {
        assert_eq!(plural(1, "слово", "слова", "слов"), "слово");
        assert_eq!(plural(3, "слово", "слова", "слов"), "слова");
        assert_eq!(plural(11, "слово", "слова", "слов"), "слов");
        assert_eq!(plural(21, "слово", "слова", "слов"), "слово");
        assert_eq!(plural(0, "слово", "слова", "слов"), "слов");
    }
    #[test]
    fn tags_route_by_owner() {
        let mut a = Tags::new(2);
        let t = a.next();
        assert_eq!(Tags::owner_of(t), 2);
        assert_ne!(t, a.next());
    }
    #[test]
    fn reduce_motion_drops_cues() {
        let mut fx = Fx::default();
        fx.anim("a", Anim::Press);
        assert_eq!(fx.take_anims().len(), 1);
        fx.reduce_motion = true;
        fx.anim("a", Anim::Press);
        assert!(fx.take_anims().is_empty());
    }
}
