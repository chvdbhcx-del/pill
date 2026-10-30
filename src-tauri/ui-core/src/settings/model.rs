//! `SettingsModel`: the live state of the settings window. Holds the merged settings (what `settings_get` / the
//! `settings:changed` event deliver), answers "what does this row show / is it visible / enabled", validates and normalises
//! a change (`try_set`) and says which backend calls it takes (`BackendCall`: exactly the `settings_set { patch }` and side
//! commands the JS page issues), and reacts to the window's events (`on_event`).
//!
//! Rules carried over from `settings.js`
//! - A change is applied locally at once (optimistic, `Object.assign(S, patch)`), then `settings_set` runs; its answer (the
//!   full merged map) or the `settings:changed` event replaces the local copy.
//! - Hotkeys never go through `settings_set`: the recorder calls `hotkey_set`, the stored combo changes only on `ok`.
//! - `language` also switches the window language and redraws (the wizard redraws itself and the sidebar, nothing else).
//! - No key is invented: only schema keys (plus `ai.preset`, `firstRunDone`) can be written; anything else is refused.
//! - Stricter than the JS where the JS would store garbage: HH:MM must be a real time, ports 1..65535, options must be one of
//!   the row's options, sliders are clamped and snapped to their step.
use crate::settings::anim::{self, AnimSpec};
use crate::settings::calls::BackendCall;
use crate::settings::flows::UpdateFlow;
use crate::settings::hotkey::{self, HotkeyError, HotkeyOutcome, Recorder};
use crate::settings::nav::{self, OpenTarget, ShowPlan};
use crate::settings::pickers::{PetPicker, ReactionPreview};
use crate::settings::providers;
use crate::settings::schema::{schema, BackendToggle, Control, Dynamic, Row, TextKind};
use crate::settings::search::{self, Entry, SearchBox};
use crate::settings::tr::{tr, Lang, Tr};
use crate::settings::wizard::Wizard;
use serde_json::{json, Map, Value};

/// Keys the UI may write although no row owns them.
pub const EXTRA_WRITABLE: [&str; 2] = ["ai.preset", "firstRunDone"];

#[derive(Clone, Debug, PartialEq)]
pub enum SetError {
    UnknownKey(String),
    WrongType(&'static str),
    NotAnOption,
    OutOfRange { min: f64, max: f64 },
    BadTime,
    BadDay,
    Hotkey(HotkeyError),
}

impl SetError {
    /// A short machine reason (for logs and tests).
    pub fn reason(&self) -> &'static str {
        match self {
            SetError::UnknownKey(_) => "unknown-key",
            SetError::WrongType(_) => "wrong-type",
            SetError::NotAnOption => "not-an-option",
            SetError::OutOfRange { .. } => "out-of-range",
            SetError::BadTime => "bad-time",
            SetError::BadDay => "bad-day",
            SetError::Hotkey(h) => h.reason(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowState {
    pub visible: bool,
    pub enabled: bool,
}

/// Monday..Sunday chips of Quiet hours: (day id as stored, label). `0` is Sunday.
pub const WEEKDAYS: [(&str, Tr); 7] = [
    ("1", tr("Пн", "Mon")),
    ("2", tr("Вт", "Tue")),
    ("3", tr("Ср", "Wed")),
    ("4", tr("Чт", "Thu")),
    ("5", tr("Пт", "Fri")),
    ("6", tr("Сб", "Sat")),
    ("0", tr("Вс", "Sun")),
];

/// `^\d{2}:\d{2}$` and a real time of day.
pub fn valid_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 5 && b[2] == b':' && b[..2].iter().chain(&b[3..]).all(u8::is_ascii_digit) && s[..2].parse::<u8>().is_ok_and(|h| h < 24) && s[3..].parse::<u8>().is_ok_and(|m| m < 60)
}

/// A textarea value as a list: split on newline, comma and semicolon, trim, drop empties.
pub fn parse_lines(text: &str) -> Vec<String> {
    text.split(['\n', ',', ';']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

/// The text a list textarea shows; `exe_suffix` adds ".exe" to each name (clipboard.denyList).
pub fn lines_text(list: &[String], exe_suffix: bool) -> String {
    list.iter().map(|x| if exe_suffix { format!("{x}.exe") } else { x.clone() }).collect::<Vec<_>>().join("\n")
}

fn strip_exe(x: &str) -> String {
    let l = x.to_ascii_lowercase();
    l.strip_suffix(".exe").unwrap_or(&l).to_string()
}

fn repr(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// 100.0 -> 100 (an integer in the file), 12.5 stays a float.
fn num(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

fn one(key: &str, v: Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert(key.to_string(), v);
    m
}

pub struct SettingsModel {
    values: Map<String, Value>,
    system_lang: String,
    lang: Lang,
    os_reduce_motion: bool,
    pub current: &'static str,
    pub wizard: Wizard,
    pub recorder: Recorder,
    pub search: SearchBox,
    pub pets: PetPicker,
    pub reaction: ReactionPreview,
    pub update: UpdateFlow,
    index: Option<Vec<Entry>>,
    autostart: bool,
}

impl SettingsModel {
    /// `system_lang`: the system UI language tag ("ru-RU"), used while `language` is "auto".
    pub fn new(system_lang: &str) -> SettingsModel {
        let mut m = SettingsModel {
            values: Map::new(),
            system_lang: system_lang.to_string(),
            lang: Lang::default(),
            os_reduce_motion: false,
            current: nav::DEFAULT_PAGE,
            wizard: Wizard::default(),
            recorder: Recorder::default(),
            search: SearchBox::default(),
            pets: PetPicker::default(),
            reaction: ReactionPreview::default(),
            update: UpdateFlow::default(),
            index: None,
            autostart: false,
        };
        m.lang = m.resolve_lang();
        m
    }

    fn resolve_lang(&self) -> Lang {
        Lang::resolve(self.get("language").as_str().unwrap_or("auto"), &self.system_lang)
    }

    /// First load: the merged settings and the page the window was asked to open (`?page=`). Returns where to start.
    pub fn begin(&mut self, merged: Map<String, Value>, page: Option<&str>) -> OpenTarget {
        self.values = merged;
        self.lang = self.resolve_lang();
        self.index = None;
        let target = nav::initial(page);
        match &target {
            OpenTarget::Wizard => self.start_wizard(),
            OpenTarget::Page(p) => self.current = p,
        }
        target
    }

    // ---------------------------------------------------------------------------------------------------- reading

    pub fn lang(&self) -> Lang {
        self.lang
    }
    pub fn values(&self) -> &Map<String, Value> {
        &self.values
    }
    /// The stored value, else the schema default, else null (the JS `get`).
    pub fn get(&self, key: &str) -> Value {
        match self.values.get(key) {
            Some(v) => v.clone(),
            None => schema().default_of(key).cloned().unwrap_or(Value::Null),
        }
    }
    pub fn get_bool(&self, key: &str) -> bool {
        self.get(key) == json!(true)
    }
    pub fn get_str(&self, key: &str) -> String {
        match self.get(key) {
            Value::String(s) => s,
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        }
    }
    /// The `reduceMotion` setting or the OS preference.
    pub fn reduce_motion(&self) -> bool {
        self.get_bool("reduceMotion") || self.os_reduce_motion
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.os_reduce_motion = on;
    }
    pub fn anim(&self, kind: anim::AnimKind) -> AnimSpec {
        anim::spec(kind, self.reduce_motion())
    }

    pub fn row_state(&self, row: &Row) -> RowState {
        let get = |k: &str| self.get(k);
        RowState { visible: row.visible_if.eval(&get), enabled: row.enabled_if.eval(&get) }
    }

    /// Rows of a page that are visible now (in screen order).
    pub fn visible_rows(&self, page: &str) -> Vec<&'static Row> {
        schema().page(page).map(|p| p.rows().filter(|r| self.row_state(r).visible).collect()).unwrap_or_default()
    }

    /// Ids of the rows whose visibility or enabled state depends on `key` (the ones to animate when it changes).
    pub fn dependents_of(key: &str) -> Vec<&'static str> {
        let mut out = Vec::new();
        for (_, r) in schema().all_rows() {
            let mut keys = Vec::new();
            r.visible_if.keys(&mut keys);
            r.enabled_if.keys(&mut keys);
            if keys.contains(&key) {
                out.push(r.id);
            }
        }
        out
    }

    /// Option the segmented control / select currently shows (the typed option value matching the stored one by its text).
    pub fn selected_option<'a>(&self, row: &'a Row) -> Option<&'a Value> {
        let cur = repr(&self.get(row.key));
        match &row.control {
            Control::Segmented { options, .. } | Control::Select { options, .. } => options.iter().find(|o| repr(&o.value) == cur).map(|o| &o.value),
            _ => None,
        }
    }

    // ---------------------------------------------------------------------------------------------------- writing

    /// Validate / normalise one change into the patch it stands for (may hold more than one key).
    pub fn normalize(&self, key: &str, value: &Value) -> Result<Map<String, Value>, SetError> {
        let sc = schema();
        let Some(row) = sc.row_for_key(key) else {
            return if EXTRA_WRITABLE.contains(&key) { Ok(one(key, value.clone())) } else { Err(SetError::UnknownKey(key.to_string())) };
        };
        // composite keys first: a provider click / address edit moves several keys together
        match key {
            "ai.provider" => return value.as_str().and_then(providers::ai_pick).ok_or(SetError::NotAnOption),
            "ai.baseUrl" => return Ok(providers::ai_url_changed(value.as_str().ok_or(SetError::WrongType("string"))?)),
            "voice.provider" => {
                let url = providers::voice_url(&self.get_str("voice.endpoint")).to_string();
                return value.as_str().and_then(|id| providers::voice_pick(id, &url, &self.get_str("voice.model"))).ok_or(SetError::NotAnOption);
            }
            "voice.endpoint" => return Ok(providers::voice_url_changed(value.as_str().ok_or(SetError::WrongType("string"))?)),
            "mail.host" => {
                let raw = value.as_str().ok_or(SetError::WrongType("string"))?;
                let (host, port) = split_host(raw);
                let mut m = one("mail.host", json!(host));
                if let Some(p) = port {
                    m.insert("mail.port".into(), json!(p));
                }
                return Ok(m);
            }
            "mail.mailbox" => {
                let t = value.as_str().ok_or(SetError::WrongType("string"))?.trim().to_string();
                return Ok(one(key, json!(if t.is_empty() { "INBOX".to_string() } else { t })));
            }
            "live.calendar.path" => {
                let t = value.as_str().ok_or(SetError::WrongType("string"))?.trim();
                let t = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(t);
                return Ok(one(key, json!(t)));
            }
            "character" => {
                let id = value.as_str().ok_or(SetError::WrongType("string"))?;
                return if crate::settings::pickers::STYLES.iter().any(|s| s.id == id) { Ok(one(key, json!(id))) } else { Err(SetError::NotAnOption) };
            }
            "pet" => {
                let id = value.as_str().ok_or(SetError::WrongType("string"))?;
                return if self.pets.knows(id) { Ok(one(key, json!(id))) } else { Err(SetError::NotAnOption) };
            }
            _ => {}
        }
        let v = match &row.control {
            Control::Toggle | Control::BackendToggle(_) => json!(value.as_bool().ok_or(SetError::WrongType("bool"))?),
            Control::Slider { min, max, step, .. } => {
                let x = as_f64(value).ok_or(SetError::WrongType("number"))?;
                let snapped = if *step > 0.0 { min + ((x - min) / step).round() * step } else { x };
                num(snapped.clamp(*min, *max))
            }
            Control::Segmented { options, dynamic: Dynamic::None } | Control::Select { options, dynamic: Dynamic::None } => {
                let want = repr(value);
                options.iter().find(|o| repr(&o.value) == want).map(|o| o.value.clone()).ok_or(SetError::NotAnOption)?
            }
            // device lists and monitors come from the backend at runtime: any string is accepted
            Control::Segmented { .. } | Control::Select { .. } => json!(value.as_str().ok_or(SetError::WrongType("string"))?),
            Control::Number { min, max } => {
                let x = as_f64(value).ok_or(SetError::WrongType("number"))?.round();
                if x < *min as f64 || x > *max as f64 {
                    return Err(SetError::OutOfRange { min: *min as f64, max: *max as f64 });
                }
                num(x)
            }
            Control::Time => {
                let t = value.as_str().ok_or(SetError::WrongType("string"))?;
                if !valid_time(t) {
                    return Err(SetError::BadTime);
                }
                json!(t)
            }
            Control::Weekdays => {
                let list: Vec<&str> = value.as_array().ok_or(SetError::WrongType("array"))?.iter().map(|x| x.as_str().unwrap_or("")).collect();
                let mut out: Vec<String> = Vec::new();
                for d in list {
                    if !WEEKDAYS.iter().any(|(id, _)| *id == d) {
                        return Err(SetError::BadDay);
                    }
                    if !out.iter().any(|x| x == d) {
                        out.push(d.to_string());
                    }
                }
                json!(out)
            }
            Control::Lines { exe_suffix, .. } => {
                let items: Vec<String> = match value {
                    Value::String(s) => parse_lines(s),
                    Value::Array(a) => a.iter().filter_map(Value::as_str).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
                    _ => return Err(SetError::WrongType("string or array")),
                };
                json!(if *exe_suffix { items.iter().map(|x| strip_exe(x)).filter(|x| !x.is_empty()).collect::<Vec<_>>() } else { items })
            }
            Control::Text { kind, commit, .. } => {
                if matches!(commit, crate::settings::schema::Commit::Backend(_)) {
                    return Err(SetError::UnknownKey(key.to_string()));
                }
                let t = value.as_str().ok_or(SetError::WrongType("string"))?;
                match kind {
                    TextKind::Secret => return Err(SetError::UnknownKey(key.to_string())),
                    _ => json!(t.trim()),
                }
            }
            // custom widgets and buttons own their keys through other paths (weather city, profiles, aiSetupDone)
            Control::Custom(_) | Control::Button { .. } => match value {
                Value::Bool(_) | Value::String(_) | Value::Number(_) => value.clone(),
                _ => return Err(SetError::WrongType("scalar")),
            },
            Control::Hotkey { .. } | Control::PetsPicker | Control::CharacterPicker | Control::Color => value.clone(),
        };
        Ok(one(key, v))
    }

    /// One change, as the page would make it. `Ok(calls)`: apply it locally (already done) and run these calls in order.
    pub fn try_set(&mut self, key: &str, value: Value) -> Result<Vec<BackendCall>, SetError> {
        if let Some(action) = key.strip_prefix("hotkey.") {
            let combo = value.as_str().ok_or(SetError::WrongType("string"))?;
            let combo = hotkey::check_local(combo, action, &self.bound_hotkeys()).map_err(SetError::Hotkey)?;
            return Ok(vec![BackendCall::HotkeySet { action: action.to_string(), combo }]);
        }
        let patch = self.normalize(key, &value)?;
        let mut calls = vec![BackendCall::SettingsSet { patch: patch.clone() }];
        for (k, v) in &patch {
            self.values.insert(k.clone(), v.clone());
        }
        match key {
            "language" => {
                let setting = patch["language"].as_str().unwrap_or("auto").to_string();
                self.lang = self.resolve_lang();
                self.index = None;
                if self.wizard.is_active() {
                    calls = Wizard::language_changed_calls(&setting);
                    calls[0] = BackendCall::SettingsSet { patch };
                } else {
                    calls.push(BackendCall::UiSetLanguage { lang: setting });
                    calls.push(BackendCall::UiRerender);
                }
            }
            "ai.provider" | "ai.baseUrl" => calls.push(BackendCall::AiConfig),
            "voice.provider" | "voice.endpoint" => {
                calls.push(BackendCall::VoiceHasKey { provider: Some(self.get_str("voice.provider")) });
                calls.push(BackendCall::NetFeaturesStatus);
            }
            "live.calendar.path" => calls.push(BackendCall::LiveplusIcsStatus),
            _ => {}
        }
        Ok(calls)
    }

    /// `try_set` that gives no calls for a refused change.
    pub fn set(&mut self, key: &str, value: Value) -> Vec<BackendCall> {
        self.try_set(key, value).unwrap_or_default()
    }

    /// Backend-backed switches (autostart, clipboard private mode).
    pub fn set_backend_toggle(&mut self, which: BackendToggle, on: bool) -> Vec<BackendCall> {
        match which {
            BackendToggle::Autostart => vec![BackendCall::AutostartSet { on }],
            BackendToggle::ClipboardPrivate => vec![BackendCall::ClipPause { paused: on }],
        }
    }

    /// `autostart_get` / `autostart_set` answered (the registry is the truth). Err text = leave the switch off and show it.
    pub fn on_autostart(&mut self, answer: Result<bool, String>) -> Option<String> {
        match answer {
            Ok(v) => {
                self.autostart = v;
                None
            }
            Err(e) => Some(e),
        }
    }
    pub fn autostart(&self) -> bool {
        self.autostart
    }

    /// The answer of `settings_set` (the full merged map) replaces the local copy.
    pub fn on_set_result(&mut self, merged: &Value) {
        if let Value::Object(m) = merged {
            self.values = m.clone();
        }
    }

    /// Chip of a quiet-hours day clicked: the new `quiet.days` list (existing order, a new day goes last).
    pub fn toggle_day(&self, day: &str) -> Vec<String> {
        let mut cur = self.quiet_days();
        if let Some(i) = cur.iter().position(|x| x == day) {
            cur.remove(i);
        } else {
            cur.push(day.to_string());
        }
        cur
    }

    pub fn quiet_days(&self) -> Vec<String> {
        match self.get("quiet.days") {
            Value::Array(a) => a.iter().map(|x| match x {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }).collect(),
            _ => WEEKDAYS.iter().map(|(d, _)| d.to_string()).collect(),
        }
    }

    // ---------------------------------------------------------------------------------------------------- hotkeys

    /// (action, combo) of every action (stored value or default) plus the `hotkey.launcher.*` shortcuts.
    pub fn bound_hotkeys(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = hotkey::ACTIONS.iter().filter(|a| a.id != "uninstall").map(|a| (a.id.to_string(), self.values.get(&hotkey::key_of(a.id)).and_then(Value::as_str).unwrap_or(a.default).to_string())).collect();
        for (k, v) in &self.values {
            if let (Some(a), Some(s)) = (k.strip_prefix("hotkey.launcher."), v.as_str()) {
                out.push((format!("launcher.{a}"), s.to_string()));
            }
        }
        out
    }

    /// The combo a row shows: stored, else the default ("" = not assigned).
    pub fn combo_of(&self, action: &str) -> String {
        let key = hotkey::key_of(action);
        match self.values.get(&key) {
            Some(Value::String(s)) => s.clone(),
            _ => hotkey::action(action).map(|a| a.default.to_string()).unwrap_or_default(),
        }
    }

    /// The answer of `hotkey_set`. Saved: remember the combo; Refused: keep the old one (the row shows the reason).
    pub fn on_hotkey_result(&mut self, action: &str, answer: &Value) -> HotkeyOutcome {
        let out = hotkey::outcome(answer);
        if let HotkeyOutcome::Saved(c) = &out {
            self.values.insert(hotkey::key_of(action), json!(c));
        }
        out
    }

    /// The city picked in the weather search (`weather_set_city` said ok): the title follows at once.
    pub fn on_city_saved(&mut self, name: &str) {
        self.values.insert("weather.city".into(), json!(name));
    }

    // ---------------------------------------------------------------------------------------------------- events

    /// `settings:changed` (payload = the full merged map) and `settings:page` (payload = a page id or "welcome").
    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        match name {
            "settings:changed" => {
                if let Value::Object(m) = payload {
                    self.values = m.clone();
                }
                let before = self.lang;
                self.lang = self.resolve_lang();
                let mut out = vec![BackendCall::UiSetReduceMotion { on: self.get_bool("reduceMotion") }];
                if before != self.lang {
                    self.index = None;
                    if self.wizard.is_active() {
                        out.push(BackendCall::UiRebuildSidebar);
                        out.push(BackendCall::UiRedrawWizard);
                    } else {
                        out.push(BackendCall::UiRerender);
                    }
                } else if !self.wizard.is_active() {
                    out.push(BackendCall::UiSyncControls);
                }
                out
            }
            "settings:page" => match payload.as_str().and_then(|p| nav::resolve_open(Some(p))) {
                Some(OpenTarget::Wizard) => {
                    self.start_wizard();
                    vec![BackendCall::UiStartWizard]
                }
                Some(OpenTarget::Page(p)) => {
                    let mut out = Vec::new();
                    if self.wizard.is_active() {
                        self.wizard.abandon();
                        out.push(BackendCall::UiEndWizardChrome);
                    }
                    out.push(BackendCall::UiShowPage { page: p.to_string() });
                    out
                }
                None => vec![],
            },
            _ => vec![],
        }
    }

    // ---------------------------------------------------------------------------------------------------- navigation

    /// Show a page: stops the key recorder (and the previous page's cleanups, the caller's job) and returns the plan.
    pub fn show(&mut self, id: &str, has_old: bool) -> ShowPlan {
        self.recorder.stop();
        let plan = nav::show_plan(id, self.lang, self.reduce_motion(), self.wizard.is_active(), has_old);
        self.current = plan.page;
        plan
    }

    /// "Run again" / the `welcome` request.
    pub fn start_wizard(&mut self) {
        self.recorder.stop();
        self.wizard.start();
    }

    pub fn search_index(&mut self) -> &[Entry] {
        let lang = self.lang;
        self.index.get_or_insert_with(|| search::build_index(lang))
    }

    /// Run the Ctrl+F field: needs the index and the box at once.
    pub fn search_input(&mut self, text: &str) -> search::SearchStep {
        let lang = self.lang;
        let idx = self.index.get_or_insert_with(|| search::build_index(lang));
        self.search.input(idx, text)
    }

    pub fn search_key(&mut self, key: search::SearchKey) -> search::SearchStep {
        let lang = self.lang;
        let idx = self.index.get_or_insert_with(|| search::build_index(lang));
        self.search.key(idx, key)
    }

    pub fn search_pick(&mut self, i: usize) -> search::SearchStep {
        let lang = self.lang;
        let idx = self.index.get_or_insert_with(|| search::build_index(lang));
        self.search.pick(idx, i)
    }

    /// The first answer: `profiles` is {name: {key: bool}}; the list shows each name with how many switches are on.
    pub fn profiles(&self) -> Vec<(String, usize)> {
        match self.values.get("profiles") {
            Some(Value::Object(o)) => o.iter().map(|(n, p)| (n.clone(), p.as_object().map(|m| m.values().filter(|v| **v == json!(true)).count()).unwrap_or(0))).collect(),
            _ => vec![],
        }
    }

    pub fn quiet_view(&self) -> QuietView {
        let days = self.quiet_days();
        QuietView {
            on: self.get_bool("quiet.on"),
            from: self.get_str("quiet.from"),
            to: self.get_str("quiet.to"),
            days: WEEKDAYS.iter().map(|(d, l)| (*d, *l, days.iter().any(|x| x == d))).collect(),
        }
    }
}

/// What the Quiet hours card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct QuietView {
    pub on: bool,
    pub from: String,
    pub to: String,
    /// (day id, label, selected) in Mon..Sun order.
    pub days: Vec<(&'static str, Tr, bool)>,
}

impl QuietView {
    /// A window that runs past midnight ("22:00" -> "07:00").
    pub fn crosses_midnight(&self) -> bool {
        valid_time(&self.from) && valid_time(&self.to) && self.from > self.to
    }
    /// `from == to` is an empty window (never quiet).
    pub fn is_empty_window(&self) -> bool {
        self.from == self.to
    }
}

/// "Imap.Gmail.com:993/inbox" -> ("imap.gmail.com", Some(993)): lower-cased, scheme and path dropped, ":port" split off.
pub fn split_host(raw: &str) -> (String, Option<u16>) {
    let mut v = raw.trim().to_ascii_lowercase();
    if let Some(i) = v.find("://") {
        if v[..i].chars().all(|c| c.is_ascii_lowercase()) && i > 0 {
            v = v[i + 3..].to_string();
        }
    }
    if let Some(i) = v.find('/') {
        v.truncate(i);
    }
    if let Some(i) = v.rfind(':') {
        let digits = &v[i + 1..];
        if (1..=5).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()) {
            let port = digits.parse::<u32>().ok().filter(|p| (1..=65535).contains(p)).map(|p| p as u16);
            v.truncate(i);
            return (v, port);
        }
    }
    (v, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> SettingsModel {
        let mut m = SettingsModel::new("en-US");
        m.begin(Map::new(), None);
        m
    }

    fn patch_of(c: &BackendCall) -> Map<String, Value> {
        match c {
            BackendCall::SettingsSet { patch } => patch.clone(),
            other => panic!("not a settings_set: {other:?}"),
        }
    }

    #[test]
    fn get_falls_back_to_the_schema_default() {
        let mut m = model();
        assert_eq!((m.get("theme"), m.get("size"), m.get("showTime"), m.get("fps")), (json!("dark"), json!(100), json!(true), json!("auto")));
        assert_eq!(m.get("no.such.key"), Value::Null);
        m.begin(json!({"theme": "light"}).as_object().unwrap().clone(), None);
        assert_eq!(m.get("theme"), json!("light"));
        assert!(!m.get_bool("quiet.on") && m.get_bool("underTaskbar"));
        assert_eq!(m.get_str("clipboard.maxItems"), "1000");
    }

    #[test]
    fn toggle_and_slider_changes() {
        let mut m = model();
        let c = m.set("autoHide", json!(true));
        assert_eq!(c, vec![BackendCall::put("autoHide", json!(true))]);
        assert!(m.get_bool("autoHide"), "applied locally at once");
        assert_eq!(m.try_set("autoHide", json!("yes")), Err(SetError::WrongType("bool")));
        // size: clamped to 80..200 and snapped to 5
        assert_eq!(patch_of(&m.set("size", json!(250))[0])["size"], 200);
        assert_eq!(patch_of(&m.set("size", json!(40))[0])["size"], 80);
        assert_eq!(patch_of(&m.set("size", json!(123))[0])["size"], 125);
        assert_eq!(patch_of(&m.set("size", json!("110"))[0])["size"], 110);
        assert_eq!(patch_of(&m.set("offsetX", json!(-7))[0])["offsetX"], -7);
        assert!(m.set("offsetX", json!(-70)).len() == 1 && m.get("offsetX") == json!(-50));
    }

    #[test]
    fn segmented_values_keep_their_type() {
        let mut m = model();
        assert_eq!(patch_of(&m.set("fps", json!("60"))[0])["fps"], json!(60));
        assert_eq!(patch_of(&m.set("fps", json!("auto"))[0])["fps"], json!("auto"));
        assert_eq!(patch_of(&m.set("clock24", json!("false"))[0])["clock24"], json!(false));
        assert_eq!(patch_of(&m.set("clipboard.maxItems", json!("500"))[0])["clipboard.maxItems"], json!(500));
        assert_eq!(patch_of(&m.set("clipboard.maxImageMb", json!(64))[0])["clipboard.maxImageMb"], json!(64));
        assert_eq!(m.try_set("theme", json!("neon")), Err(SetError::NotAnOption));
        assert_eq!(m.try_set("fps", json!(61)), Err(SetError::NotAnOption));
        assert!(m.set("theme", json!("neon")).is_empty());
        m.set("fps", json!(60));
        assert_eq!(m.selected_option(schema().row("fps").unwrap()), Some(&json!(60)));
    }

    #[test]
    fn unknown_keys_are_refused_and_extras_allowed() {
        let mut m = model();
        assert_eq!(m.try_set("brand.new.key", json!(1)), Err(SetError::UnknownKey("brand.new.key".into())));
        assert!(m.try_set("firstRunDone", json!(true)).is_ok() && m.try_set("ai.preset", json!("ollama")).is_ok());
        assert!(m.try_set("ai.key", json!("secret")).is_err(), "secrets never go through settings_set");
        assert!(m.try_set("live.obs.password", json!("x")).is_err());
    }

    #[test]
    fn language_switch_also_redraws() {
        let mut m = SettingsModel::new("en-US");
        m.begin(Map::new(), None);
        assert_eq!(m.lang(), Lang::En);
        let c = m.set("language", json!("ru"));
        assert_eq!(c, vec![BackendCall::put("language", json!("ru")), BackendCall::UiSetLanguage { lang: "ru".into() }, BackendCall::UiRerender]);
        assert_eq!(m.lang(), Lang::Ru);
        let c = m.set("language", json!("auto"));
        assert_eq!(m.lang(), Lang::En, "auto follows the system language");
        assert_eq!(c.len(), 3);
        m.start_wizard();
        let c = m.set("language", json!("ru"));
        assert_eq!((c.len(), &c[2], &c[3]), (4, &BackendCall::UiRebuildSidebar, &BackendCall::UiRedrawWizard), "inside the wizard only the wizard redraws");
        assert_eq!(m.lang(), Lang::Ru);
    }

    #[test]
    fn hotkeys_go_through_hotkey_set_only() {
        let mut m = model();
        assert_eq!(m.set("hotkey.toggle", json!("ctrl+alt+k")), vec![BackendCall::HotkeySet { action: "toggle".into(), combo: "Ctrl+Alt+K".into() }]);
        assert_eq!(m.combo_of("toggle"), "Ctrl+Alt+I", "not stored until the backend says ok");
        assert_eq!(m.try_set("hotkey.chat", json!("Ctrl+Alt+I")), Err(SetError::Hotkey(HotkeyError::Used)));
        assert_eq!(m.try_set("hotkey.chat", json!("Ctrl+Alt+Q")), Err(SetError::Hotkey(HotkeyError::Reserved)));
        assert_eq!(m.try_set("hotkey.chat", json!("A")), Err(SetError::Hotkey(HotkeyError::Invalid)));
        assert_eq!(m.set("hotkey.chat", json!("")), vec![BackendCall::HotkeySet { action: "chat".into(), combo: String::new() }], "clearing is allowed");
        assert_eq!(m.on_hotkey_result("toggle", &json!({"ok": true, "combo": "Ctrl+Alt+K"})), HotkeyOutcome::Saved("Ctrl+Alt+K".into()));
        assert_eq!(m.combo_of("toggle"), "Ctrl+Alt+K");
        assert_eq!(m.on_hotkey_result("toggle", &json!({"ok": false, "reason": "taken"})), HotkeyOutcome::Refused(HotkeyError::Taken));
        assert_eq!(m.combo_of("toggle"), "Ctrl+Alt+K", "a refusal keeps the old binding");
        // the freed default can now be taken by another action
        assert!(m.try_set("hotkey.chat", json!("Ctrl+Alt+I")).is_ok());
        assert!(m.set("hotkey.launcher.abc", json!("Ctrl+Alt+9")).len() == 1);
        assert!(m.bound_hotkeys().iter().all(|(a, _)| a != "uninstall"));
    }

    #[test]
    fn quiet_hours_validation_and_days() {
        let mut m = model();
        assert_eq!(m.set("quiet.on", json!(true)), vec![BackendCall::put("quiet.on", json!(true))]);
        assert_eq!(patch_of(&m.set("quiet.from", json!("21:30"))[0])["quiet.from"], "21:30");
        for bad in ["2130", "9:30", "24:00", "12:60", "ab:cd", ""] {
            assert_eq!(m.try_set("quiet.to", json!(bad)), Err(SetError::BadTime), "{bad}");
        }
        assert_eq!(m.quiet_days().len(), 7);
        assert_eq!(m.toggle_day("3"), ["0", "1", "2", "4", "5", "6"]);
        let c = m.set("quiet.days", json!(["1", "2", "3", "4", "5"]));
        assert_eq!(patch_of(&c[0])["quiet.days"], json!(["1", "2", "3", "4", "5"]));
        assert_eq!(m.toggle_day("6"), ["1", "2", "3", "4", "5", "6"], "a new day goes last");
        assert_eq!(m.try_set("quiet.days", json!(["1", "9"])), Err(SetError::BadDay));
        assert_eq!(patch_of(&m.set("quiet.days", json!(["1", "1", "2"]))[0])["quiet.days"], json!(["1", "2"]), "duplicates collapse");
        let v = m.quiet_view();
        assert!(v.on && v.crosses_midnight() && !v.is_empty_window());
        assert_eq!(v.days.iter().map(|d| d.0).collect::<Vec<_>>(), ["1", "2", "3", "4", "5", "6", "0"]);
        assert_eq!(v.days.iter().filter(|d| d.2).count(), 2);
        assert!(valid_time("00:00") && valid_time("23:59") && !valid_time("7:00"));
    }

    #[test]
    fn list_fields_split_and_clean() {
        let mut m = model();
        let c = m.set("hideInApps", json!("photoshop.exe, premiere.exe;\n  \nblender.exe"));
        assert_eq!(patch_of(&c[0])["hideInApps"], json!(["photoshop.exe", "premiere.exe", "blender.exe"]));
        let c = m.set("clipboard.denyList", json!("KeePass.exe\n1Password.EXE\n.exe\n"));
        assert_eq!(patch_of(&c[0])["clipboard.denyList"], json!(["keepass", "1password"]));
        assert_eq!(lines_text(&["keepass".into(), "mstsc".into()], true), "keepass.exe\nmstsc.exe");
        assert_eq!(parse_lines(" a , b\n\n;c "), ["a", "b", "c"]);
        let d = model().get("clipboard.denyList");
        assert_eq!(d.as_array().unwrap().len(), 8);
    }

    #[test]
    fn text_fields_normalise() {
        let mut m = model();
        assert_eq!(patch_of(&m.set("live.calendar.path", json!("  \"C:\\Users\\Me\\cal.ics\" "))[0])["live.calendar.path"], "C:\\Users\\Me\\cal.ics");
        assert_eq!(m.set("live.calendar.path", json!("x")).last(), Some(&BackendCall::LiveplusIcsStatus));
        let c = m.set("mail.host", json!("IMAP://Imap.Gmail.com:995/inbox"));
        let p = patch_of(&c[0]);
        assert_eq!((p["mail.host"].clone(), p["mail.port"].clone()), (json!("imap.gmail.com"), json!(995)));
        assert_eq!(m.get("mail.port"), json!(995));
        assert_eq!(patch_of(&m.set("mail.host", json!("mail.example.org"))[0]).len(), 1);
        assert_eq!(patch_of(&m.set("mail.mailbox", json!("  "))[0])["mail.mailbox"], "INBOX");
        assert_eq!(patch_of(&m.set("capture.folder", json!("  D:\\Shots "))[0])["capture.folder"], "D:\\Shots");
        assert_eq!(patch_of(&m.set("live.obs.port", json!(4456.4))[0])["live.obs.port"], 4456);
        assert_eq!(m.try_set("live.obs.port", json!(70000)), Err(SetError::OutOfRange { min: 1.0, max: 65535.0 }));
        assert_eq!(m.try_set("mail.port", json!(0)), Err(SetError::OutOfRange { min: 1.0, max: 65535.0 }));
        assert_eq!(split_host("imap.x.org:993"), ("imap.x.org".to_string(), Some(993)));
        assert_eq!(split_host("host:999999"), ("host:999999".to_string(), None), "six digits are not a port");
        assert_eq!(split_host("host:70000"), ("host".to_string(), None), "out of range: the port is dropped");
        assert_eq!(split_host("host:abc"), ("host:abc".to_string(), None));
    }

    #[test]
    fn provider_clicks_move_several_keys() {
        let mut m = model();
        let c = m.set("ai.provider", json!("ollama"));
        let p = patch_of(&c[0]);
        assert_eq!((p["ai.provider"].clone(), p["ai.preset"].clone(), p["ai.baseUrl"].clone()), (json!("ollama"), json!("ollama"), json!("http://127.0.0.1:11434/v1")));
        assert_eq!(c[1], BackendCall::AiConfig, "the config is read again afterwards");
        let c = m.set("ai.baseUrl", json!("https://api.deepseek.com/v1"));
        assert_eq!(patch_of(&c[0])["ai.provider"], "deepseek");
        assert_eq!(m.get("ai.provider"), json!("deepseek"));
        let c = m.set("voice.provider", json!("gemini"));
        assert_eq!(patch_of(&c[0])["voice.model"], "gemini-2.5-flash");
        assert!(c.contains(&BackendCall::NetFeaturesStatus) && c.contains(&BackendCall::VoiceHasKey { provider: Some("gemini".into()) }));
        assert!(m.try_set("ai.provider", json!("nope")).is_err());
    }

    #[test]
    fn pickers_write_character_and_pet() {
        let mut m = model();
        assert_eq!(m.set("character", json!("glow")), vec![BackendCall::put("character", json!("glow"))]);
        assert!(m.try_set("character", json!("rainbow")).is_err());
        assert_eq!(m.set("pet", json!("none")), vec![BackendCall::put("pet", json!("none"))]);
        assert!(m.try_set("pet", json!("dragon")).is_err());
    }

    #[test]
    fn dependent_rows_follow_their_switch() {
        let mut m = model();
        let st = |m: &SettingsModel, id: &str| m.row_state(schema().row(id).unwrap());
        // pass-through opacity is greyed until pass-through is on
        assert!(!st(&m, "passThroughOpacity").enabled && st(&m, "passThroughOpacity").visible);
        m.set("passThrough", json!(true));
        assert!(st(&m, "passThroughOpacity").enabled);
        // quiet hours fields follow quiet.on
        assert!(!st(&m, "quiet.from").enabled);
        m.set("quiet.on", json!(true));
        assert!(st(&m, "quiet.from").enabled && st(&m, "quiet.days").enabled);
        // pet side / size disappear without a pet
        assert!(st(&m, "pet.side").visible);
        m.set("pet", json!("none"));
        assert!(!st(&m, "pet.side").visible && !st(&m, "pet.size").visible);
        // MP4-only recording rows
        assert!(st(&m, "capture.fps").visible);
        m.set("capture.format", json!("gif"));
        assert!(!st(&m, "capture.fps").visible && !st(&m, "capture.encoder").visible);
        assert!(!m.visible_rows("clipboard").iter().any(|r| r.id == "capture.fps"));
        // network opt-ins gate their fields
        assert!(!st(&m, "weather.city").enabled && !st(&m, "showTemp").enabled);
        m.set("net.weather", json!(true));
        assert!(st(&m, "weather.city").enabled && st(&m, "showTemp").enabled);
        assert!(!st(&m, "mail.host").enabled);
        m.set("live.mail", json!(true));
        assert!(st(&m, "mail.host").enabled);
        let d = SettingsModel::dependents_of("quiet.on");
        assert!(d.contains(&"quiet.from") && d.contains(&"quiet.to") && d.contains(&"quiet.days") && d.len() == 3);
        assert!(SettingsModel::dependents_of("theme").is_empty());
        assert!(SettingsModel::dependents_of("autoHide").contains(&"autoHideKeepDesktop"));
    }

    #[test]
    fn settings_changed_event_resyncs_or_redraws() {
        let mut m = model();
        let c = m.on_event("settings:changed", &json!({"theme": "light", "reduceMotion": true}));
        assert_eq!(c, vec![BackendCall::UiSetReduceMotion { on: true }, BackendCall::UiSyncControls]);
        assert_eq!(m.get("theme"), json!("light"));
        assert!(m.reduce_motion());
        let c = m.on_event("settings:changed", &json!({"language": "ru"}));
        assert_eq!(c, vec![BackendCall::UiSetReduceMotion { on: false }, BackendCall::UiRerender], "a language change redraws everything");
        m.start_wizard();
        let c = m.on_event("settings:changed", &json!({"language": "en"}));
        assert_eq!(c[1..], [BackendCall::UiRebuildSidebar, BackendCall::UiRedrawWizard]);
        let c = m.on_event("settings:changed", &json!({"language": "en"}));
        assert_eq!(c, vec![BackendCall::UiSetReduceMotion { on: false }], "in the wizard an unchanged language does not sync the pages");
        let _ = m.on_event("settings:changed", &json!("not an object"));
        assert_eq!(m.get("language"), json!("en"), "a non-object payload is ignored");
    }

    #[test]
    fn page_event_opens_pages_and_the_wizard() {
        let mut m = model();
        assert_eq!(m.on_event("settings:page", &json!("hotkeys")), vec![BackendCall::UiShowPage { page: "hotkeys".into() }]);
        assert_eq!(m.on_event("settings:page", &json!("welcome")), vec![BackendCall::UiStartWizard]);
        assert!(m.wizard.is_active());
        assert_eq!(m.on_event("settings:page", &json!("updates")), vec![BackendCall::UiEndWizardChrome, BackendCall::UiShowPage { page: "updates".into() }]);
        assert!(!m.wizard.is_active());
        assert!(m.on_event("settings:page", &json!("calc.exe")).is_empty());
        assert!(m.on_event("whatever", &json!(1)).is_empty());
    }

    #[test]
    fn begin_and_show() {
        let mut m = SettingsModel::new("ru-RU");
        assert_eq!(m.begin(Map::new(), Some("welcome")), OpenTarget::Wizard);
        assert!(m.wizard.is_active() && m.lang() == Lang::Ru);
        let mut m = SettingsModel::new("ru-RU");
        assert_eq!(m.begin(json!({"reduceMotion": true}).as_object().unwrap().clone(), Some("Weather")), OpenTarget::Page("weather"));
        assert_eq!(m.current, "weather");
        m.recorder.start("toggle");
        let plan = m.show("sound", true);
        assert_eq!((plan.page, m.current, m.recorder.active()), ("sound", "sound", None), "leaving a page stops the key recorder");
        assert!(plan.exit.is_none() && plan.enter.is_instant(), "Reduce motion");
        assert_eq!(m.anim(anim::AnimKind::ToggleFlip).dur_ms, 0);
    }

    #[test]
    fn profiles_and_backend_toggles() {
        let mut m = model();
        m.on_set_result(&json!({"profiles": {"Work": {"module.chat": true, "module.media": false, "live.usb": true}, "Empty": {}}}));
        let mut p = m.profiles();
        p.sort();
        assert_eq!(p, [("Empty".to_string(), 0), ("Work".to_string(), 2)]);
        assert_eq!(m.set_backend_toggle(BackendToggle::Autostart, true), vec![BackendCall::AutostartSet { on: true }]);
        assert_eq!(m.set_backend_toggle(BackendToggle::ClipboardPrivate, true), vec![BackendCall::ClipPause { paused: true }]);
        assert_eq!(m.on_autostart(Ok(true)), None);
        assert!(m.autostart());
        assert_eq!(m.on_autostart(Err("registry error 5".into())).as_deref(), Some("registry error 5"));
        m.on_city_saved("Kazan");
        assert_eq!(m.get_str("weather.city"), "Kazan");
        assert_eq!(patch_of(&BackendCall::put("x", json!(1)))["x"], 1);
    }

    #[test]
    fn search_uses_the_cached_index_and_follows_the_language() {
        let mut m = model();
        assert_eq!(m.search_input("quiet"), search::SearchStep::Paint { fresh: true });
        let hits = m.search.hits.clone();
        assert!(hits.iter().any(|h| m.search_index()[h.index].row_id == "quiet.on"));
        m.set("language", json!("ru"));
        assert!(m.search_index().iter().any(|e| e.title == "Тихие часы"), "the index is rebuilt in the new language");
        assert!(matches!(m.search_pick(0), search::SearchStep::Go(_)) || m.search.hits.is_empty());
    }
}
