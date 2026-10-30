//! The bar's contract with the backend: `BackendCall { cmd, args }` mirrors a Tauri `invoke(cmd, args)` of the JS pages (argument names are the
//! camelCase ones the JS sends), and the structs here are the result shapes of `cmd_query` / `cmd_run` / `cmd_ask` / `settings_get` / `voice_*`.
//! The renderer performs the call and answers with `BarEvent::Reply { ticket, value }`; a failed invoke is `None` (the JS wrappers turn a
//! rejected promise into `null`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One backend command to run: `invoke(cmd, args)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackendCall {
    pub cmd: &'static str,
    pub args: Value,
}

impl BackendCall {
    pub fn new(cmd: &'static str, args: Value) -> BackendCall {
        BackendCall { cmd, args }
    }
    /// A command without arguments (`Value::Null`, like `invoke('cmd_toggle')`).
    pub fn bare(cmd: &'static str) -> BackendCall {
        BackendCall { cmd, args: Value::Null }
    }

    pub fn ready() -> BackendCall {
        BackendCall::bare("cmd_ready")
    }
    pub fn toggle() -> BackendCall {
        BackendCall::bare("cmd_toggle")
    }
    pub fn query(text: &str) -> BackendCall {
        BackendCall::new("cmd_query", json!({ "text": text }))
    }
    pub fn icon(id: &str) -> BackendCall {
        BackendCall::new("cmd_icon", json!({ "id": id }))
    }
    pub fn run(id: &str, mode: &str) -> BackendCall {
        BackendCall::new("cmd_run", json!({ "id": id, "mode": mode }))
    }
    /// `cmd_ask { question }` or, in a dialogue, `{ question, history }`.
    pub fn ask(question: &str, history: &[Turn]) -> BackendCall {
        if history.is_empty() {
            BackendCall::new("cmd_ask", json!({ "question": question }))
        } else {
            BackendCall::new("cmd_ask", json!({ "question": question, "history": history }))
        }
    }
    pub fn ask_cancel() -> BackendCall {
        BackendCall::bare("ai_transform_cancel")
    }
    pub fn settings_get() -> BackendCall {
        BackendCall::bare("settings_get")
    }
    pub fn settings_open(page: &str) -> BackendCall {
        BackendCall::new("settings_open", json!({ "page": page }))
    }
    pub fn voice_state() -> BackendCall {
        BackendCall::bare("voice_state")
    }
    pub fn voice_start() -> BackendCall {
        BackendCall::bare("voice_start")
    }
    pub fn voice_stop() -> BackendCall {
        BackendCall::bare("voice_stop")
    }
}

/// A chat turn of the dialogue memory (`{ role, content }`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

/// Result kinds of `cmd_query` plus the three rows the bar makes up itself while an AI dialogue is open.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    App,
    Uninstall,
    File,
    Folder,
    Calc,
    Convert,
    Web,
    System,
    Shell,
    Hint,
    Ask,
    AskCopy,
    NewConv,
    Clip,
    Note,
    Shelf,
    Reminder,
    Tab,
    Setting,
    Action,
    /// a kind this build does not know: drawn as a plain file row
    Other(String),
}

impl Kind {
    pub fn parse(s: &str) -> Kind {
        match s {
            "app" => Kind::App,
            "uninstall" => Kind::Uninstall,
            "file" => Kind::File,
            "folder" => Kind::Folder,
            "calc" => Kind::Calc,
            "convert" => Kind::Convert,
            "web" => Kind::Web,
            "system" => Kind::System,
            "shell" => Kind::Shell,
            "hint" => Kind::Hint,
            "ask" => Kind::Ask,
            "askcopy" => Kind::AskCopy,
            "newconv" => Kind::NewConv,
            "clip" => Kind::Clip,
            "note" => Kind::Note,
            "shelf" => Kind::Shelf,
            "reminder" => Kind::Reminder,
            "tab" => Kind::Tab,
            "setting" => Kind::Setting,
            "action" => Kind::Action,
            o => Kind::Other(o.to_string()),
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            Kind::App => "app",
            Kind::Uninstall => "uninstall",
            Kind::File => "file",
            Kind::Folder => "folder",
            Kind::Calc => "calc",
            Kind::Convert => "convert",
            Kind::Web => "web",
            Kind::System => "system",
            Kind::Shell => "shell",
            Kind::Hint => "hint",
            Kind::Ask => "ask",
            Kind::AskCopy => "askcopy",
            Kind::NewConv => "newconv",
            Kind::Clip => "clip",
            Kind::Note => "note",
            Kind::Shelf => "shelf",
            Kind::Reminder => "reminder",
            Kind::Tab => "tab",
            Kind::Setting => "setting",
            Kind::Action => "action",
            Kind::Other(s) => s,
        }
    }
    /// app / file / folder rows: real shell icons, hover buttons, a context menu
    pub fn is_pathy(&self) -> bool {
        matches!(self, Kind::App | Kind::File | Kind::Folder)
    }
    /// app / file / folder can be pinned to the island's launcher strip (`pinnable()` in bar.js)
    pub fn pinnable(&self) -> bool {
        self.is_pathy()
    }
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Kind, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Kind::parse(&s))
    }
}

/// One row of `cmd_query` (`command.rs: Res`). The backend sends English titles; the bar localises the known ones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Res {
    pub id: String,
    pub kind: Kind,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default)]
    pub payload: String,
}

impl Res {
    pub fn new(kind: Kind, id: &str, title: &str, subtitle: &str, payload: &str) -> Res {
        Res { id: id.into(), kind, title: title.into(), subtitle: subtitle.into(), payload: payload.into() }
    }
    /// Parses the `cmd_query` answer (an array); rows that do not parse are skipped, a non-array is an empty list.
    pub fn list_from(v: &Value) -> Vec<Res> {
        v.as_array().map(|a| a.iter().filter_map(|x| serde_json::from_value(x.clone()).ok()).collect()).unwrap_or_default()
    }
}

/// `cmd_run` outcome status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// press Enter again within 3 s
    Confirm,
    Error,
    /// copy `message` to the clipboard, then close the bar
    Copy,
    /// a setting / action switched, `message` is the new state (or a key for the pin result)
    Updated,
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub status: Status,
    pub message: String,
}

impl Outcome {
    pub fn from_value(v: &Value) -> Outcome {
        let status = match v.get("status").and_then(Value::as_str).unwrap_or("") {
            "ok" => Status::Ok,
            "confirm" => Status::Confirm,
            "error" => Status::Error,
            "copy" => Status::Copy,
            "updated" => Status::Updated,
            o => Status::Other(o.to_string()),
        };
        Outcome { status, message: v.get("message").and_then(Value::as_str).unwrap_or("").to_string() }
    }
}

/// The settings the bar reads out of `settings_get()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub language: Option<String>,
    pub reduce_motion: bool,
    /// `light`, `dark` or `auto`
    pub theme: String,
    pub hotkey: String,
    pub net_ai: bool,
    pub ai_base_url: String,
    pub ai_model: String,
}

impl Default for Config {
    fn default() -> Config {
        Config { language: None, reduce_motion: false, theme: "dark".into(), hotkey: DEFAULT_HOTKEY.into(), net_ai: false, ai_base_url: String::new(), ai_model: String::new() }
    }
}

pub const DEFAULT_HOTKEY: &str = "Alt+Shift+Space";

impl Config {
    pub fn from_value(v: &Value) -> Config {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let hotkey = s("hotkey.command");
        Config {
            language: v.get("language").and_then(Value::as_str).map(str::to_string),
            reduce_motion: v.get("reduceMotion").and_then(Value::as_bool).unwrap_or(false),
            theme: v.get("theme").and_then(Value::as_str).unwrap_or("dark").to_string(),
            hotkey: if hotkey.is_empty() { DEFAULT_HOTKEY.into() } else { hotkey },
            net_ai: v.get("net.ai").and_then(Value::as_bool) == Some(true), // opt-in: only a literal `true` counts
            ai_base_url: s("ai.baseUrl"),
            ai_model: s("ai.model"),
        }
    }
    /// `askReady()`: the network switch is on and both an address and a model are set.
    pub fn ask_ready(&self) -> bool {
        self.net_ai && !self.ai_base_url.trim().is_empty() && !self.ai_model.trim().is_empty()
    }
}

/// `cmd_ask` result: `{ ok: true, text }` or `{ ok: false, error }`.
#[derive(Clone, Debug, PartialEq)]
pub enum AskResult {
    Answer(String),
    Failed(Option<String>),
}

impl AskResult {
    pub fn from_value(v: Option<&Value>) -> AskResult {
        match v {
            Some(r) if r.get("ok").and_then(Value::as_bool) == Some(true) => AskResult::Answer(r.get("text").and_then(Value::as_str).unwrap_or("").to_string()),
            Some(r) => AskResult::Failed(r.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_string)),
            None => AskResult::Failed(None),
        }
    }
}

/// `voice:state` event payload and `voice_state` result.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct VoicePayload {
    pub state: String,
    pub text: String,
    pub code: String,
    pub message: String,
    /// `voice_state().enabled === false`
    pub disabled: bool,
}

impl VoicePayload {
    pub fn from_value(v: &Value) -> VoicePayload {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        VoicePayload { state: s("state"), text: s("text"), code: s("code"), message: s("message"), disabled: v.get("enabled").and_then(Value::as_bool) == Some(false) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_mirror_the_js_invokes() {
        assert_eq!(BackendCall::query("5*8"), BackendCall::new("cmd_query", json!({ "text": "5*8" })));
        assert_eq!(BackendCall::run("app:x", "folder").args, json!({ "id": "app:x", "mode": "folder" }));
        assert_eq!(BackendCall::ask("why", &[]).args, json!({ "question": "why" }));
        let h = vec![Turn { role: "user".into(), content: "a".into() }];
        assert_eq!(BackendCall::ask("b", &h).args, json!({ "question": "b", "history": [{ "role": "user", "content": "a" }] }));
        assert_eq!(BackendCall::settings_open("ai").args, json!({ "page": "ai" }));
        assert_eq!(BackendCall::toggle().args, Value::Null);
    }

    #[test]
    fn rows_parse_and_unknown_kinds_survive() {
        let v = json!([
            { "id": "calc:40", "kind": "calc", "title": "= 40", "subtitle": "5*8  -  Enter copies", "payload": "40" },
            { "id": "z", "kind": "zzz", "title": "?", "subtitle": "", "payload": "" },
            { "broken": true }
        ]);
        let r = Res::list_from(&v);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].kind, Kind::Calc);
        assert_eq!(r[1].kind, Kind::Other("zzz".into()));
        assert_eq!(Kind::parse("askcopy"), Kind::AskCopy);
        assert_eq!(Res::list_from(&json!(null)).len(), 0);
        assert_eq!(serde_json::to_value(&r[0]).unwrap()["kind"], "calc");
    }

    #[test]
    fn outcome_and_config() {
        let o = Outcome::from_value(&json!({ "status": "updated", "message": "pinned" }));
        assert_eq!((o.status, o.message.as_str()), (Status::Updated, "pinned"));
        assert_eq!(Outcome::from_value(&json!({ "status": "wat" })).status, Status::Other("wat".into()));
        let c = Config::from_value(&json!({ "language": "ru", "reduceMotion": true, "net.ai": true, "ai.baseUrl": " http://x ", "ai.model": "m", "hotkey.command": "" }));
        assert!(c.ask_ready() && c.reduce_motion);
        assert_eq!(c.hotkey, DEFAULT_HOTKEY);
        assert!(!Config::from_value(&json!({ "net.ai": "yes", "ai.baseUrl": "x", "ai.model": "m" })).ask_ready(), "only a literal true opts in");
        assert!(!Config::from_value(&json!({ "net.ai": true, "ai.baseUrl": "  ", "ai.model": "m" })).ask_ready());
    }

    #[test]
    fn ask_results() {
        assert_eq!(AskResult::from_value(Some(&json!({ "ok": true, "text": "hi" }))), AskResult::Answer("hi".into()));
        assert_eq!(AskResult::from_value(Some(&json!({ "ok": false, "error": "net_off" }))), AskResult::Failed(Some("net_off".into())));
        assert_eq!(AskResult::from_value(Some(&json!({ "ok": false }))), AskResult::Failed(None));
        assert_eq!(AskResult::from_value(None), AskResult::Failed(None));
    }

    #[test]
    fn voice_payload() {
        let v = VoicePayload::from_value(&json!({ "state": "idle", "enabled": false }));
        assert!(v.disabled && v.state == "idle");
        assert!(!VoicePayload::from_value(&json!({ "state": "idle" })).disabled);
    }
}
