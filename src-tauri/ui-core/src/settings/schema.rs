//! The declarative settings schema: `Page > Group > Row`, each row owning one `Control`.
//!
//! A renderer draws pages from this data with its own controls (toggle, slider, segmented, select, button, text input,
//! hotkey capture, pickers); nothing here knows about pixels. The data lives in `pages/*`; this file only has the types,
//! the tiny constructors the data uses, and the lookups (`Schema`).
//!
//! Vocabulary
//! - `Row.key` is the exact `settings.json` key (never renamed). Rows that write nothing to settings.json (buttons,
//!   backend toggles such as autostart, secret fields that go to DPAPI storage) have an empty key and a unique `id`.
//! - `Row.default` is the value the page shows while the key is absent (the JS `DEF` table, equal to the Rust default
//!   where `settings.rs` has one; `tests.rs` checks that).
//! - `visible_if` hides a row (native refinement, used where a row is meaningless otherwise), `enabled_if` greys it.
//! - `restart_needed`: no key of the JS settings window needs a restart (the backend applies every patch at once), the
//!   field exists so the native shell can flag one later.
//! - `search_keywords`: extra ru+en words the Ctrl+F search matches besides label and hint.
use crate::settings::tr::{Lang, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

/// One option of a segmented control or select. `value` is what lands in settings.json (string, number or bool).
#[derive(Clone, Debug, PartialEq)]
pub struct Opt {
    pub value: Value,
    pub label: Tr,
}

/// Options that the backend fills in at runtime (the static `options` then hold only the leading fixed entries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dynamic {
    None,
    /// `pill_monitors`: one segment per monitor, value "1".."n", after the fixed "primary".
    Monitors,
    /// `media_devices`: output devices (id, name), after the fixed "same as Windows".
    OutputDevices,
    /// `media_capture_devices`: microphones (id, name), after the fixed "same as Windows".
    CaptureDevices,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    None,
    /// The slider value is shown as "{v} %".
    Percent,
}

impl Unit {
    pub fn format(self, v: f64) -> String {
        match self {
            Unit::None => format!("{}", v),
            Unit::Percent => format!("{} %", v),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Plain,
    Url,
    Path,
    /// IMAP host: lower-cased, scheme and path dropped, ":port" moves to `mail.port`.
    Host,
    Port,
    Secret,
}

/// Where a text field's value goes on commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    /// `settings_set { key: value }`.
    Settings,
    /// A backend command (secrets never touch settings.json in plain text); the field is cleared after the call.
    Backend(Action),
}

/// Toggles whose state lives outside settings.json.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendToggle {
    /// HKCU Run value "Pill": `autostart_get` / `autostart_set`, the registry is the truth.
    Autostart,
    /// Clipboard private mode: `clip_status` / `clip_pause`.
    ClipboardPrivate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    Normal,
    Small,
    Primary,
    Danger,
}

/// What a button does. The matching backend call is built by `calls::action_call` (flow state lives in `flows.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    RunWizard,
    OpenPage(&'static str),
    SettingsExport,
    SettingsImport,
    EverythingDownload,
    EverythingRunInstaller,
    UpdateCheck,
    ReportProblem,
    ClipWipe,
    WeatherSearch,
    WeatherRefresh,
    PlaySignal,
    AgentsSnippets,
    AiSetKey,
    AiRemoveKey,
    AiList,
    AiTest,
    VoiceSetKey,
    VoiceRemoveKey,
    VoiceTestMic,
    VoiceTestService,
    ObsSetPassword,
    MailSetPassword,
    MailTest,
    NotifAllow,
    ProfileSave,
    AiSetupLater,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Toggle,
    BackendToggle(BackendToggle),
    Slider { min: f64, max: f64, step: f64, unit: Unit },
    Segmented { options: Vec<Opt>, dynamic: Dynamic },
    Select { options: Vec<Opt>, dynamic: Dynamic },
    /// Hotkey capture for `hotkey.<action>`; `action` is "toggle", "command", ... or "launcher.<id>".
    Hotkey { action: &'static str },
    Text { secret: bool, kind: TextKind, placeholder: Tr, mono: bool, commit: Commit },
    /// A textarea holding a list, one entry per line (split on newline, comma, semicolon). `exe_suffix`: shown as
    /// "name.exe", stored as the lower-case name without ".exe" (clipboard.denyList).
    Lines { rows: u8, mono: bool, placeholder: Tr, exe_suffix: bool },
    Number { min: i64, max: i64 },
    /// "HH:MM" time field, committed only when it matches `^\d{2}:\d{2}$`.
    Time,
    /// Seven day chips ("1".."6","0" = Mon..Sun) stored as a string list.
    Weekdays,
    /// Reserved for a colour field (the JS settings window has none yet).
    Color,
    Button { label: Tr, action: Action, style: ButtonStyle },
    /// Pet cards by group (see `pickers`), stored in `pet`.
    PetsPicker,
    /// The five island edge-style cards (see `pickers`), stored in `character`.
    CharacterPicker,
    /// Page-specific widget the renderer owns (provider chips, status lines, profile list, launcher shortcuts...).
    Custom(&'static str),
}

impl Control {
    /// The JS `.seg` with long labels takes the whole row; the renderer then puts it under the text.
    pub fn label_len(&self, lang: Lang) -> usize {
        match self {
            Control::Segmented { options, .. } => options.iter().map(|o| o.label.get(lang).chars().count()).sum(),
            _ => 0,
        }
    }
}

/// Condition on current setting values (`get` returns the value or the schema default, never "missing").
#[derive(Clone, Debug, PartialEq)]
pub enum Cond {
    Always,
    /// The key is true.
    On(&'static str),
    /// The key is not true.
    Off(&'static str),
    Eq(&'static str, Value),
    Ne(&'static str, Value),
    All(Vec<Cond>),
    Any(Vec<Cond>),
}

impl Cond {
    pub fn eval(&self, get: &dyn Fn(&str) -> Value) -> bool {
        match self {
            Cond::Always => true,
            Cond::On(k) => get(k) == json!(true),
            Cond::Off(k) => get(k) != json!(true),
            Cond::Eq(k, v) => get(k) == *v,
            Cond::Ne(k, v) => get(k) != *v,
            Cond::All(c) => c.iter().all(|x| x.eval(get)),
            Cond::Any(c) => c.iter().any(|x| x.eval(get)),
        }
    }

    pub fn keys(&self, out: &mut Vec<&'static str>) {
        match self {
            Cond::Always => {}
            Cond::On(k) | Cond::Off(k) | Cond::Eq(k, _) | Cond::Ne(k, _) => out.push(k),
            Cond::All(c) | Cond::Any(c) => c.iter().for_each(|x| x.keys(out)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Unique across the schema: the key for stored rows, "<page>.<name>" for the others.
    pub id: &'static str,
    /// Exact settings.json key, "" for rows that write nothing there.
    pub key: &'static str,
    pub label: Tr,
    pub hint: Tr,
    pub control: Control,
    pub default: Value,
    pub visible_if: Cond,
    pub enabled_if: Cond,
    pub restart_needed: bool,
    pub search_keywords: &'static [&'static str],
    /// Id of a polled status line shown under the row ("" = none): "obs", "steam", "calendar", "mail", "notifications"
    /// (the text comes from `flows::status_line`).
    pub status: &'static str,
    /// Control goes under the text instead of beside it (long segmented controls, textareas, path fields).
    pub block: bool,
}

impl Row {
    pub fn new(id: &'static str, key: &'static str, label: Tr, control: Control, default: Value) -> Row {
        Row { id, key, label, hint: Tr::NONE, control, default, visible_if: Cond::Always, enabled_if: Cond::Always, restart_needed: false, search_keywords: &[], status: "", block: false }
    }
    pub fn hint(mut self, h: Tr) -> Row {
        self.hint = h;
        self
    }
    pub fn visible(mut self, c: Cond) -> Row {
        self.visible_if = c;
        self
    }
    pub fn enabled(mut self, c: Cond) -> Row {
        self.enabled_if = c;
        self
    }
    pub fn kw(mut self, k: &'static [&'static str]) -> Row {
        self.search_keywords = k;
        self
    }
    pub fn status(mut self, id: &'static str) -> Row {
        self.status = id;
        self
    }
    pub fn block(mut self) -> Row {
        self.block = true;
        self
    }
    pub fn restart(mut self) -> Row {
        self.restart_needed = true;
        self
    }
    /// Stored in settings.json (as opposed to buttons and backend-backed rows).
    pub fn is_stored(&self) -> bool {
        !self.key.is_empty()
    }
    /// Layout in the current language: explicit `block`, or a segmented control whose labels together exceed 34 characters.
    pub fn is_block(&self, lang: Lang) -> bool {
        self.block || self.control.label_len(lang) > 34
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The caption above the card ("" = none).
    pub title: Tr,
    /// A small note under the card.
    pub footer: Tr,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub id: &'static str,
    /// Sidebar icon id (see `nav::ICONS`).
    pub icon: &'static str,
    pub title: Tr,
    /// Heading when it differs from the sidebar name (Hotkeys -> "Горячие клавиши").
    pub head: Tr,
    pub footnote: Tr,
    pub groups: Vec<Group>,
}

impl Page {
    pub fn heading(&self) -> Tr {
        if self.head.is_blank() {
            self.title
        } else {
            self.head
        }
    }
    pub fn rows(&self) -> impl Iterator<Item = &Row> {
        self.groups.iter().flat_map(|g| g.rows.iter())
    }
}

// ------------------------------------------------------------------------------------------------ constructors for the data

pub fn group(title: Tr, rows: Vec<Row>) -> Group {
    Group { title, footer: Tr::NONE, rows }
}
pub fn group_note(title: Tr, footer: Tr, rows: Vec<Row>) -> Group {
    Group { title, footer, rows }
}
pub fn page(id: &'static str, icon: &'static str, title: Tr, groups: Vec<Group>) -> Page {
    Page { id, icon, title, head: Tr::NONE, footnote: Tr::NONE, groups }
}

pub fn toggle(key: &'static str, label: Tr, hint: Tr, default: bool) -> Row {
    Row::new(key, key, label, Control::Toggle, json!(default)).hint(hint)
}
pub fn seg(key: &'static str, label: Tr, hint: Tr, options: Vec<Opt>, default: Value) -> Row {
    Row::new(key, key, label, Control::Segmented { options, dynamic: Dynamic::None }, default).hint(hint)
}
pub fn select(key: &'static str, label: Tr, hint: Tr, options: Vec<Opt>, default: Value) -> Row {
    Row::new(key, key, label, Control::Select { options, dynamic: Dynamic::None }, default).hint(hint)
}
#[allow(clippy::too_many_arguments)]
pub fn slider(key: &'static str, label: Tr, hint: Tr, min: f64, max: f64, step: f64, unit: Unit, default: f64) -> Row {
    let d = if default.fract() == 0.0 { json!(default as i64) } else { json!(default) };
    Row::new(key, key, label, Control::Slider { min, max, step, unit }, d).hint(hint)
}
pub fn text(key: &'static str, label: Tr, hint: Tr, kind: TextKind, placeholder: Tr, default: &str) -> Row {
    Row::new(key, key, label, Control::Text { secret: false, kind, placeholder, mono: false, commit: Commit::Settings }, json!(default)).hint(hint)
}
/// A secret field: the value goes to a backend command, never to settings.json.
pub fn secret(id: &'static str, label: Tr, hint: Tr, placeholder: Tr, save: Action) -> Row {
    Row::new(id, "", label, Control::Text { secret: true, kind: TextKind::Secret, placeholder, mono: false, commit: Commit::Backend(save) }, Value::Null).hint(hint)
}
pub fn button(id: &'static str, label: Tr, hint: Tr, btn: Tr, action: Action, style: ButtonStyle) -> Row {
    Row::new(id, "", label, Control::Button { label: btn, action, style }, Value::Null).hint(hint)
}
pub fn custom(id: &'static str, label: Tr, hint: Tr, widget: &'static str) -> Row {
    Row::new(id, "", label, Control::Custom(widget), Value::Null).hint(hint)
}
pub fn opt(value: Value, label: Tr) -> Opt {
    Opt { value, label }
}
/// String-valued options: `so(&[("dark", tr(..)), ...])`.
pub fn so(list: &[(&str, Tr)]) -> Vec<Opt> {
    list.iter().map(|(v, l)| Opt { value: json!(v), label: *l }).collect()
}
/// Number-valued options.
pub fn no(list: &[(i64, Tr)]) -> Vec<Opt> {
    list.iter().map(|(v, l)| Opt { value: json!(v), label: *l }).collect()
}

// ------------------------------------------------------------------------------------------------ lookups

pub struct Schema {
    pub pages: Vec<Page>,
    by_id: HashMap<&'static str, (usize, usize, usize)>,
    by_key: HashMap<&'static str, (usize, usize, usize)>,
}

impl Schema {
    pub fn new(pages: Vec<Page>) -> Schema {
        let (mut by_id, mut by_key) = (HashMap::new(), HashMap::new());
        for (pi, p) in pages.iter().enumerate() {
            for (gi, g) in p.groups.iter().enumerate() {
                for (ri, r) in g.rows.iter().enumerate() {
                    by_id.insert(r.id, (pi, gi, ri));
                    if r.is_stored() {
                        by_key.entry(r.key).or_insert((pi, gi, ri));
                    }
                }
            }
        }
        Schema { pages, by_id, by_key }
    }
    pub fn page(&self, id: &str) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == id)
    }
    fn at(&self, i: (usize, usize, usize)) -> &Row {
        &self.pages[i.0].groups[i.1].rows[i.2]
    }
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.by_id.get(id).map(|i| self.at(*i))
    }
    /// The first row writing `key`.
    pub fn row_for_key(&self, key: &str) -> Option<&Row> {
        self.by_key.get(key).map(|i| self.at(*i))
    }
    /// (page id, row) for every row of every page, in screen order.
    pub fn all_rows(&self) -> impl Iterator<Item = (&Page, &Row)> {
        self.pages.iter().flat_map(|p| p.rows().map(move |r| (p, r)))
    }
    pub fn page_of(&self, row_id: &str) -> Option<&Page> {
        self.by_id.get(row_id).map(|i| &self.pages[i.0])
    }
    /// Schema default of a stored key (None when the schema has no row for it).
    pub fn default_of(&self, key: &str) -> Option<&Value> {
        self.row_for_key(key).map(|r| &r.default)
    }
}

/// The schema of the whole settings window, built once.
pub fn schema() -> &'static Schema {
    static S: OnceLock<Schema> = OnceLock::new();
    S.get_or_init(|| Schema::new(crate::settings::pages::build()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::tr::tr;

    #[test]
    fn cond_evaluates_against_values() {
        let get = |k: &str| match k {
            "a" => json!(true),
            "m" => json!("mp4"),
            _ => Value::Null,
        };
        assert!(Cond::On("a").eval(&get) && !Cond::Off("a").eval(&get));
        assert!(Cond::Off("zzz").eval(&get), "a missing key is not true");
        assert!(Cond::Eq("m", json!("mp4")).eval(&get) && Cond::Ne("m", json!("gif")).eval(&get));
        assert!(Cond::All(vec![Cond::On("a"), Cond::Eq("m", json!("mp4"))]).eval(&get));
        assert!(!Cond::All(vec![Cond::On("a"), Cond::Eq("m", json!("gif"))]).eval(&get));
        assert!(Cond::Any(vec![Cond::Off("a"), Cond::On("a")]).eval(&get));
        let mut ks = vec![];
        Cond::All(vec![Cond::On("a"), Cond::Any(vec![Cond::Ne("b", json!(1))])]).keys(&mut ks);
        assert_eq!(ks, ["a", "b"]);
    }

    #[test]
    fn long_segmented_controls_go_under_the_text() {
        let long = seg("k", tr("a", "a"), Tr::NONE, so(&[("x", tr("aaaaaaaaaaaaaaaaaaaa", "b")), ("y", tr("bbbbbbbbbbbbbbbbbbbb", "b"))]), json!("x"));
        assert!(long.is_block(Lang::Ru) && !long.is_block(Lang::En));
        assert!(toggle("t", tr("a", "a"), Tr::NONE, true).block().is_block(Lang::En));
    }

    #[test]
    fn units_format_like_the_page() {
        assert_eq!(Unit::Percent.format(80.0), "80 %");
        assert_eq!(Unit::None.format(5.0), "5");
    }
}
