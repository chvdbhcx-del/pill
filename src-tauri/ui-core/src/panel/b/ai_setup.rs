//! Settings > AI setup (src/ai-setup.js): first-run wizard and reopenable page for voice-to-text (local whisper.cpp or cloud) and the AI
//! chat (servers already running, Ollama install + model pulls, cloud). Backend: aisetup.rs.
//! Commands: `aisetup_status`, `aisetup_whisper_install {model}`, `aisetup_whisper_set {on, model}`, `aisetup_whisper_remove`,
//! `aisetup_ollama_download`, `aisetup_ollama_run_installer`, `aisetup_ollama_pull {model}`, `aisetup_ai_use {server, model}`,
//! `aisetup_cancel`, `voice_has_key`, `settings_set {patch:{aiSetupDone}}`, `settings_close`. Event `aisetup:progress
//! {id, item, done, total, phase, message}` (id = whisper | ollama-setup | ollama-pull; phase = download | verify | unpack | start |
//! pull | done | error | cancelled).
//!
//! The step machine: `busy` holds the id of the one running job; while it is set every download/switch button is disabled. A job starts
//! only from a button (its source host and size are shown next to it), reports through `aisetup:progress`, and ends with
//! done / error / cancelled, which sets that job's result line and re-reads the status. Nothing here touches the network; no key is
//! ever handled (cloud keys are entered on the voice / AI pages this page links to). Deleting from disk needs a second press
//! within 3 s.
//!
//! ponytail: the page is one flat `View`; the renderer lays the sections out (cards with rows).

use super::{Anim, BackendCall, Effect, Fx, Lang, Reply, Tags, Timer, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const DELETE_ARM_MS: u32 = 3000;
const OWNER: u8 = 8;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct WhisperModel {
    pub id: String,
    pub size: u64,
    pub installed: bool,
    pub host: String,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Engine {
    pub installed: bool,
    pub tag: String,
    pub host: String,
    pub size: u64,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Whisper {
    pub engine: Engine,
    pub models: Vec<WhisperModel>,
    pub enabled: bool,
    /// local port while the server runs
    pub running: Option<u32>,
    pub model: String,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub url: String,
    pub up: bool,
    pub models: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Ollama {
    pub installed: bool,
    pub up: bool,
    pub installer_ready: bool,
    pub installer_size: u64,
    pub catalog: Vec<(String, u64)>,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Status {
    pub done: bool,
    pub dir: String,
    pub busy: Option<String>,
    pub whisper: Whisper,
    pub ollama: Ollama,
    pub servers: Vec<Server>,
    pub ai_on: bool,
    pub ai_base_url: String,
    pub ai_model: String,
}
impl Status {
    pub fn from_json(v: &Value) -> Option<Status> {
        if !v.is_object() {
            return None;
        }
        let s = |x: &Value, k: &str| x[k].as_str().unwrap_or("").to_string();
        let b = |x: &Value, k: &str| x[k].as_bool().unwrap_or(false);
        let u = |x: &Value, k: &str| x[k].as_u64().unwrap_or(0);
        let w = &v["whisper"];
        let o = &v["ollama"];
        Some(Status {
            done: b(v, "done"),
            dir: s(v, "dir"),
            busy: v["busy"].as_str().map(str::to_string),
            whisper: Whisper {
                engine: Engine { installed: b(&w["engine"], "installed"), tag: s(&w["engine"], "tag"), host: s(&w["engine"], "host"), size: u(&w["engine"], "size") },
                models: w["models"].as_array().into_iter().flatten().map(|m| WhisperModel { id: s(m, "id"), size: u(m, "size"), installed: b(m, "installed"), host: s(m, "host") }).collect(),
                enabled: b(w, "enabled"),
                running: w["running"].as_u64().filter(|p| *p > 0).map(|p| p as u32),
                model: s(w, "model"),
            },
            ollama: Ollama {
                installed: b(o, "installed"),
                up: b(o, "up"),
                installer_ready: b(&o["installer"], "ready"),
                installer_size: u(&o["installer"], "size"),
                catalog: o["catalog"].as_array().into_iter().flatten().map(|m| (s(m, "name"), u(m, "size"))).collect(),
            },
            servers: v["servers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| Server { id: s(x, "id"), name: s(x, "name"), url: s(x, "url"), up: b(x, "up"), models: x["models"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() })
                .collect(),
            ai_on: b(&v["ai"], "on"),
            ai_base_url: s(&v["ai"], "baseUrl"),
            ai_model: s(&v["ai"], "model"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Progress {
    pub id: String,
    pub item: String,
    pub phase: String,
    pub done: f64,
    pub total: f64,
    pub message: String,
}
impl Progress {
    fn from_json(v: &Value) -> Progress {
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        Progress { id: s("id"), item: s("item"), phase: s("phase"), done: v["done"].as_f64().unwrap_or(0.0), total: v["total"].as_f64().unwrap_or(0.0), message: s("message") }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
    /// the model a finished pull produced (for the "Use <model>" button)
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    SelectWhisperModel(String),
    /// the on/off switch of the local server
    WhisperSwitch(bool),
    /// "Download X and turn on"
    WhisperInstall,
    /// "Turn on" / "Switch to this model"
    WhisperEnable,
    /// "Delete from disk" (press twice)
    WhisperRemove,
    /// Cancel of the running job
    CancelJob,
    /// choose a model in a running server's drop-down
    PickServerModel { server: String, model: String },
    UseServer { server: String },
    OllamaDownload,
    OllamaRunInstaller,
    OllamaPull(String),
    /// "Use <model>" after a finished pull
    UseOllamaModel,
    CheckAgain,
    /// "Choose service and key" / "Set up a cloud AI"
    GoVoice,
    GoAi,
    /// footer button: "Later" (done = false) or "Show at start again" (done = true)
    ToggleLater,
}

// ------------------------------------------------------------------ view

#[derive(Clone, Debug, PartialEq)]
pub struct ProgressView {
    pub visible: bool,
    pub running: bool,
    pub line: String,
    pub indeterminate: bool,
    /// 0..=100
    pub fill_pct: u32,
    pub cancel_label: String,
    pub result: Option<(bool, String)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ModelChip {
    pub id: String,
    pub name: String,
    pub size_text: String,
    pub on: bool,
    pub on_disk_badge: Option<String>,
    pub in_use_badge: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Download { heading: String, items: Vec<String>, button: String, hint: String, enabled: bool },
    Enable { label: String, enabled: bool },
    AllSet { text: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceView {
    pub caption: String,
    pub local_title: String,
    pub local_sub: String,
    pub local_running: bool,
    /// the switch exists only when the engine and one model are installed: (on, enabled)
    pub switch: Option<(bool, bool)>,
    pub chips: Vec<ModelChip>,
    pub model_note: String,
    pub plan: Plan,
    pub progress: ProgressView,
    pub folder_note: String,
    pub delete: Option<(String, bool)>,
    pub cloud_title: String,
    pub cloud_sub: String,
    pub cloud_button: String,
    pub key_note: Option<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ServerRow {
    pub id: String,
    pub name: String,
    pub sub: String,
    pub models: Vec<String>,
    pub selected: String,
    pub button: String,
    pub button_enabled: bool,
    pub model_picker_enabled: bool,
    pub result: Option<(bool, String)>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum OllamaControl {
    Installed { note: String },
    RunInstaller { label: String, enabled: bool },
    Download { label: String, enabled: bool },
}
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogRow {
    pub name: String,
    pub size_text: String,
    pub pulled: Option<String>,
    pub download: Option<(String, bool)>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum OllamaBlock {
    Setup { title: String, sub: String, control: OllamaControl, progress: ProgressView },
    Models { note: String, rows: Vec<CatalogRow>, progress: ProgressView, use_pulled: Option<(String, bool)> },
}
#[derive(Clone, Debug, PartialEq)]
pub struct ChatSection {
    pub caption: String,
    pub local_title: String,
    pub local_sub: String,
    pub local_found: bool,
    pub check_again: String,
    pub servers: Vec<ServerRow>,
    pub ollama: OllamaBlock,
    pub cloud_title: String,
    pub cloud_sub: String,
    pub cloud_button: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FooterView {
    pub title: String,
    pub sub: String,
    pub button: String,
    pub button_enabled: bool,
    pub note: String,
}
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    /// "Could not read the state."
    Unavailable(String),
    Page { intro: String, voice: VoiceView, chat: ChatSection, footer: FooterView },
}

enum Pend {
    Status,
    WhisperInstall { first: String },
    WhisperSwitch,
    WhisperEnable,
    WhisperRemove,
    OllamaDownload { size: u64 },
    OllamaInstaller,
    OllamaPull { model: String, size: u64 },
    Use { key: String, name: String, model: String, pulled: bool },
    VoiceKey,
    Put,
    Other,
}

/// `size(b)`: "1.5 ГБ" / "466 МБ"
pub fn size_text(b: f64, lang: Lang) -> String {
    if b >= 1e9 { format!("{:.1}{}", b / 1e9, lang.t(" ГБ", " GB")) } else { format!("{}{}", (b / 1e6).round() as i64, lang.t(" МБ", " MB")) }
}

pub struct AiSetup {
    pub fx: Fx,
    lang: Lang,
    tags: Tags,
    pend: HashMap<u64, Pend>,
    status: Option<Status>,
    sel_model: String,
    busy: Option<String>,
    prog: HashMap<String, Progress>,
    result: HashMap<String, Outcome>,
    server_sel: HashMap<String, String>,
    cancel_pending: bool,
    armed: bool,
    arm: Timer,
    key_saved: bool,
    alive: bool,
}

impl AiSetup {
    pub fn new(lang: Lang) -> Self {
        Self {
            fx: Fx::default(), lang, tags: Tags::new(OWNER), pend: HashMap::new(), status: None, sel_model: "small".into(), busy: None, prog: HashMap::new(), result: HashMap::new(),
            server_sel: HashMap::new(), cancel_pending: false, armed: false, arm: Timer::default(), key_saved: false, alive: true,
        }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
    }
    pub fn busy(&self) -> Option<&str> {
        self.busy.as_deref()
    }
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
    pub fn selected_model(&self) -> &str {
        &self.sel_model
    }
    /// The page is left: ignore everything that arrives later.
    pub fn leave(&mut self) {
        self.alive = false;
    }
    fn call(&mut self, cmd: &'static str, args: Value, p: Pend) -> BackendCall {
        let tag = self.tags.next();
        self.pend.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }
    fn lock(&self) -> bool {
        self.busy.is_some()
    }
    fn set_result(&mut self, key: &str, ok: bool, text: String) {
        self.result.insert(key.to_string(), Outcome { ok, text, model: None });
    }

    /// Open the page: read the status.
    pub fn mount(&mut self) -> Vec<BackendCall> {
        self.alive = true;
        self.load()
    }
    fn load(&mut self) -> Vec<BackendCall> {
        vec![self.call("aisetup_status", json!({}), Pend::Status), self.call("voice_has_key", json!({}), Pend::VoiceKey)]
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        if !self.alive {
            return vec![];
        }
        let Some(st) = self.status.clone() else {
            return if matches!(a, Action::CheckAgain) { self.load() } else { vec![] };
        };
        match a {
            Action::SelectWhisperModel(id) => {
                self.sel_model = id;
                vec![]
            }
            Action::WhisperSwitch(on) => {
                let model = if on && st.whisper.models.iter().any(|m| m.id == self.sel_model && m.installed) { json!(self.sel_model) } else { Value::Null };
                vec![self.call("aisetup_whisper_set", json!({ "on": on, "model": model }), Pend::WhisperSwitch)]
            }
            Action::WhisperInstall => {
                self.result.remove("whisper");
                let first = if !st.whisper.engine.installed { "engine".to_string() } else { self.sel_model.clone() };
                vec![self.call("aisetup_whisper_install", json!({ "model": self.sel_model }), Pend::WhisperInstall { first })]
            }
            Action::WhisperEnable => vec![self.call("aisetup_whisper_set", json!({ "on": true, "model": self.sel_model }), Pend::WhisperEnable)],
            Action::WhisperRemove => {
                if !self.armed {
                    self.armed = true;
                    self.arm.arm(DELETE_ARM_MS);
                    self.fx.anim("whisper-remove", Anim::Press);
                    return vec![];
                }
                self.armed = false;
                self.arm.cancel();
                vec![self.call("aisetup_whisper_remove", json!({}), Pend::WhisperRemove)]
            }
            Action::CancelJob => {
                self.cancel_pending = true;
                vec![self.call("aisetup_cancel", json!({}), Pend::Other)]
            }
            Action::PickServerModel { server, model } => {
                self.server_sel.insert(server, model);
                vec![]
            }
            Action::UseServer { server } => {
                let Some(s) = st.servers.iter().find(|s| s.id == server) else { return vec![] };
                let model = self.server_sel.get(&server).cloned().or_else(|| if st.ai_model.is_empty() { None } else { Some(st.ai_model.clone()) }).filter(|m| s.models.contains(m)).or_else(|| s.models.first().cloned()).unwrap_or_default();
                let name = s.name.clone();
                vec![self.call("aisetup_ai_use", json!({ "server": server, "model": model }), Pend::Use { key: format!("use:{server}"), name, model, pulled: false })]
            }
            Action::OllamaDownload => {
                self.result.remove("ollama-setup");
                vec![self.call("aisetup_ollama_download", json!({}), Pend::OllamaDownload { size: st.ollama.installer_size })]
            }
            Action::OllamaRunInstaller => vec![self.call("aisetup_ollama_run_installer", json!({}), Pend::OllamaInstaller)],
            Action::OllamaPull(model) => {
                self.result.remove("ollama-pull");
                let size = st.ollama.catalog.iter().find(|(n, _)| *n == model).map_or(0, |(_, s)| *s);
                vec![self.call("aisetup_ollama_pull", json!({ "model": model }), Pend::OllamaPull { model, size })]
            }
            Action::UseOllamaModel => {
                let Some(model) = self.result.get("ollama-pull").filter(|r| r.ok).and_then(|r| r.model.clone()) else { return vec![] };
                vec![self.call("aisetup_ai_use", json!({ "server": "ollama", "model": model }), Pend::Use { key: "ollama-pull".into(), name: "Ollama".into(), model, pulled: true })]
            }
            Action::CheckAgain => self.load(),
            Action::GoVoice => {
                self.fx.eff(Effect::GoPage("voice"));
                vec![]
            }
            Action::GoAi => {
                self.fx.eff(Effect::GoPage("ai"));
                vec![]
            }
            Action::ToggleLater => {
                if st.done {
                    vec![self.call("settings_set", json!({ "patch": { "aiSetupDone": false } }), Pend::Put)]
                } else if self.lock() {
                    vec![]
                } else {
                    // "Later": never open by itself again, then close the Settings window
                    vec![self.call("settings_set", json!({ "patch": { "aiSetupDone": true } }), Pend::Put), self.call("settings_close", json!({}), Pend::Other)]
                }
            }
        }
    }

    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        let Some(p) = self.pend.remove(&tag) else { return vec![] };
        if !self.alive {
            return vec![];
        }
        let v = if r.ok { r.v.clone() } else { Value::Null };
        let failed = |v: &Value| v.is_null() || v["ok"].as_bool() == Some(false);
        let err = |v: &Value| v["error"].as_str().filter(|e| !e.is_empty()).unwrap_or("error").to_string();
        let ru = self.lang.is_ru();
        match p {
            Pend::Status => {
                if let Some(s) = Status::from_json(&v) {
                    self.busy = s.busy.clone();
                    if !s.whisper.models.iter().any(|m| m.id == self.sel_model) {
                        self.sel_model = "small".into();
                    }
                    self.status = Some(s);
                }
                self.cancel_pending = false;
                vec![]
            }
            Pend::VoiceKey => {
                self.key_saved = v["has"].as_bool().unwrap_or(false);
                vec![]
            }
            Pend::WhisperInstall { first } => {
                if failed(&v) {
                    self.set_result("whisper", false, err(&v));
                    return self.load();
                }
                self.busy = Some("whisper".into());
                self.prog.insert("whisper".into(), Progress { id: "whisper".into(), phase: "download".into(), item: first, ..Default::default() });
                vec![]
            }
            Pend::WhisperSwitch => {
                if v["ok"].as_bool() == Some(false) {
                    self.set_result("whisper", false, err(&v));
                }
                self.load()
            }
            Pend::WhisperEnable => {
                if v["ok"].as_bool() == Some(true) {
                    self.set_result("whisper", true, self.lang.t("Готово: голос в текст использует локальный сервер.", "Done: voice-to-text uses the local server.").into());
                } else {
                    self.set_result("whisper", false, err(&v));
                }
                self.load()
            }
            Pend::WhisperRemove => {
                if v["ok"].as_bool() == Some(true) {
                    self.set_result("whisper", true, self.lang.t("Удалено. Голос в текст выключен.", "Deleted. Voice-to-text is off.").into());
                } else {
                    self.set_result("whisper", false, err(&v));
                }
                self.load()
            }
            Pend::OllamaDownload { size } => {
                if failed(&v) {
                    self.set_result("ollama-setup", false, err(&v));
                    return self.load();
                }
                self.busy = Some("ollama-setup".into());
                self.prog.insert("ollama-setup".into(), Progress { id: "ollama-setup".into(), item: "installer".into(), phase: "download".into(), total: size as f64, ..Default::default() });
                vec![]
            }
            Pend::OllamaInstaller => {
                if v["ok"].as_bool() == Some(true) {
                    self.set_result("ollama-setup", true, self.lang.t("Установщик запущен. Когда он закончит, нажмите «Проверить снова».", "The installer is running. When it finishes, press “Check again”.").into());
                } else {
                    self.set_result("ollama-setup", false, err(&v));
                }
                vec![]
            }
            Pend::OllamaPull { model, size } => {
                if failed(&v) {
                    self.set_result("ollama-pull", false, err(&v));
                    return self.load();
                }
                self.busy = Some("ollama-pull".into());
                self.prog.insert("ollama-pull".into(), Progress { id: "ollama-pull".into(), item: model, phase: "pull".into(), total: size as f64, ..Default::default() });
                vec![]
            }
            Pend::Use { key, name, model, pulled } => {
                if v["ok"].as_bool() == Some(true) {
                    let text = format!("{}{} · {}", self.lang.t("Чат переключён на ", "The chat now uses "), name, model);
                    self.result.insert(key, Outcome { ok: true, text, model: None });
                } else {
                    self.result.insert(key, Outcome { ok: false, text: err(&v), model: None });
                }
                let _ = (ru, pulled);
                self.load()
            }
            Pend::Put => {
                if let Some(s) = self.status.as_mut() {
                    s.done = !s.done;
                }
                vec![]
            }
            Pend::Other => vec![],
        }
    }

    /// `aisetup:progress`
    pub fn on_event(&mut self, name: &str, p: &Value) -> Vec<BackendCall> {
        if name != "aisetup:progress" || !self.alive || p.is_null() {
            return vec![];
        }
        let q = Progress::from_json(p);
        match q.phase.as_str() {
            "done" | "error" | "cancelled" => {
                self.busy = None;
                let l = self.lang;
                match q.phase.as_str() {
                    "done" => match q.id.as_str() {
                        "whisper" => self.set_result("whisper", true, format!("{}{}", l.t("Готово: голос в текст теперь использует локальный сервер ", "Done: voice-to-text now uses the local server "), q.message)),
                        "ollama-setup" => self.set_result("ollama-setup", true, l.t("Установщик скачан. Нажмите «Запустить установщик».", "The installer is downloaded. Press “Run the installer”.").into()),
                        "ollama-pull" => {
                            self.result.insert("ollama-pull".into(), Outcome { ok: true, text: format!("{}{}", l.t("Модель скачана: ", "Model downloaded: "), q.message), model: Some(q.message.clone()) });
                        }
                        _ => {}
                    },
                    "cancelled" => self.set_result(&q.id, false, l.t("Отменено. Недокачанный файл удалён.", "Cancelled. The partial file was deleted.").into()),
                    _ => {
                        let m = if q.message.is_empty() { "error".to_string() } else { q.message.clone() };
                        self.set_result(&q.id, false, m);
                    }
                }
                self.prog.remove(&q.id);
                self.load()
            }
            _ => {
                self.busy = Some(q.id.clone());
                if q.phase == "download" || q.phase == "pull" {
                    self.fx.anim(&format!("progress:{}", q.id), Anim::Press);
                } else {
                    self.fx.anim(&format!("progress:{}", q.id), Anim::ProgressSweep);
                }
                self.prog.insert(q.id.clone(), q);
                vec![]
            }
        }
    }

    pub fn tick(&mut self, dt_ms: u32) {
        if self.arm.tick(dt_ms) {
            self.armed = false;
        }
    }

    // ---- view ----
    fn progress_text(&self, q: &Progress) -> (String, bool, u32) {
        let l = self.lang;
        let total = q.total.max(0.0);
        let pc = if total > 0.0 { ((q.done / total) * 100.0).floor().min(100.0) as u32 } else { 0 };
        let known = total > 0.0 && q.done <= total * 1.02;
        let item = match q.item.as_str() {
            "engine" => l.t("движок whisper.cpp", "whisper.cpp engine").to_string(),
            "base" => l.t("модель Base", "Base model").to_string(),
            "small" => l.t("модель Small", "Small model").to_string(),
            "turbo" => l.t("модель Large-v3-turbo", "Large-v3-turbo model").to_string(),
            "installer" => l.t("установщик Ollama", "Ollama installer").to_string(),
            o => o.to_string(),
        };
        let line = match q.phase.as_str() {
            "verify" => l.t("Проверяю SHA-256…", "Verifying SHA-256…").to_string(),
            "unpack" => l.t("Распаковываю…", "Unpacking…").to_string(),
            "start" => l.t("Запускаю сервер…", "Starting the server…").to_string(),
            "pull" => format!("{} · {}{}", q.item, q.message, if known && q.done > 0.0 { format!(" · {pc} %") } else { String::new() }),
            _ => format!("{item} · {}{}", size_text(q.done, l), if known { format!(" / {} · {pc} %", size_text(total, l)) } else { String::new() }),
        };
        let indeterminate = (!known && q.phase != "pull") || ["verify", "unpack", "start"].contains(&q.phase.as_str());
        let fill = if known && (q.phase == "download" || q.phase == "pull") { pc } else { 0 };
        (line, indeterminate, fill)
    }
    fn progress_view(&self, id: &str) -> ProgressView {
        let running = self.busy.as_deref() == Some(id);
        let res = self.result.get(id).map(|r| (r.ok, r.text.clone()));
        let (line, indeterminate, fill_pct) = match self.prog.get(id).filter(|_| running) {
            Some(q) => self.progress_text(q),
            None => (String::new(), false, 0),
        };
        ProgressView { visible: running || res.is_some(), running, line, indeterminate, fill_pct, cancel_label: self.lang.t("Отмена", "Cancel").into(), result: if running { None } else { res } }
    }

    pub fn view(&self) -> View {
        let l = self.lang;
        let Some(s) = &self.status else { return View::Unavailable(l.t("Не удалось прочитать состояние.", "Could not read the state.").into()) };
        let lock = self.lock();
        let w = &s.whisper;
        let model_name = |id: &str| match id {
            "base" => "Base".to_string(),
            "small" => "Small".to_string(),
            "turbo" => "Large-v3-turbo (q5)".to_string(),
            o => o.to_string(),
        };
        let model_note = |id: &str| match id {
            "base" => l.t("Самая быстрая и лёгкая. Русский и редкие слова распознаёт заметно хуже.", "The fastest and lightest. Clearly weaker on Russian and rare words."),
            "small" => l.t("Хороший баланс: русский и английский, работает быстро даже на процессоре. Рекомендуется.", "A good balance: Russian and English, fast even on a CPU. Recommended."),
            "turbo" => l.t("Лучшее качество из трёх. Версия для процессора: заметно медленнее Small, фраза в 10 секунд может обрабатываться несколько секунд.", "The best quality of the three. A CPU build: clearly slower than Small, a 10-second phrase can take several seconds."),
            _ => "",
        };
        let cur_name = if w.model.is_empty() { w.model.clone() } else { model_name(&w.model) };
        let state = match w.running {
            Some(p) => format!("{}127.0.0.1:{p} · {}{cur_name}", l.t("работает на ", "running on "), l.t("модель ", "model ")),
            None if w.enabled => l.t("включён, но сервер не запущен (см. server.log в папке ai/whisper)", "enabled, but the server is not running (see server.log in the ai/whisper folder)").to_string(),
            None => l.t("не установлен или выключен", "not installed or off").to_string(),
        };
        let local_sub = format!(
            "{}{}{}",
            l.t("Pill запускает whisper.cpp скрыто, только на 127.0.0.1, и закрывает его вместе с собой. Запись не покидает компьютер, ключ не нужен. ", "Pill runs whisper.cpp hidden, on 127.0.0.1 only, and closes it with itself. The recording stays on this computer and no key is needed. "),
            l.t("Сейчас: ", "Now: "),
            state
        );
        let sel = w.models.iter().find(|m| m.id == self.sel_model);
        let chips = w
            .models
            .iter()
            .map(|m| ModelChip {
                id: m.id.clone(),
                name: model_name(&m.id),
                size_text: size_text(m.size as f64, l),
                on: m.id == self.sel_model,
                on_disk_badge: m.installed.then(|| l.t("на диске", "on disk").to_string()),
                in_use_badge: (m.id == w.model && w.running.is_some()).then(|| l.t("используется", "in use").to_string()),
            })
            .collect();
        let mut need: Vec<(String, f64, String)> = vec![];
        if !w.engine.installed {
            need.push((l.t(&format!("Движок whisper.cpp {} (Windows, процессор)", w.engine.tag), &format!("whisper.cpp {} engine (Windows, CPU)", w.engine.tag)).into(), w.engine.size as f64, w.engine.host.clone()));
        }
        if let Some(m) = sel.filter(|m| !m.installed) {
            need.push((l.t(&format!("Модель {}", model_name(&m.id)), &format!("{} model", model_name(&m.id))).into(), m.size as f64, m.host.clone()));
        }
        let plan = if !need.is_empty() {
            let total: f64 = need.iter().map(|n| n.1).sum();
            Plan::Download {
                heading: l.t("Будет скачано:", "Will be downloaded:").into(),
                items: need.iter().map(|n| format!("{} · {} · {}", n.0, size_text(n.1, l), n.2)).collect(),
                button: l.t(&format!("Скачать {} и включить", size_text(total, l)), &format!("Download {} and turn on", size_text(total, l))).into(),
                hint: l.t("Файлы проверяются по SHA-256, который зашит в Pill.", "Files are verified against a SHA-256 built into Pill.").into(),
                enabled: !lock,
            }
        } else if !(w.running.is_some() && w.model == self.sel_model) {
            Plan::Enable { label: if w.running.is_some() { l.t("Переключить на эту модель", "Switch to this model") } else { l.t("Включить", "Turn on") }.into(), enabled: !lock }
        } else {
            Plan::AllSet { text: l.t("Всё установлено и работает. Диктовка: Ctrl+Alt+D (клавишу можно сменить во вкладке «Клавиши»).", "Everything is installed and running. Dictate with Ctrl+Alt+D (the key can be changed under Hotkeys).").into() }
        };
        let any_installed = w.engine.installed || w.models.iter().any(|m| m.installed);
        let voice = VoiceView {
            caption: l.t("Голос в текст (Whisper)", "Voice to text (Whisper)").into(),
            local_title: l.t("Локально, на этом компьютере", "Locally, on this computer").into(),
            local_sub,
            local_running: w.running.is_some(),
            switch: (w.engine.installed && w.models.iter().any(|m| m.installed)).then_some((w.running.is_some(), !lock)),
            chips,
            model_note: model_note(&self.sel_model).into(),
            plan,
            progress: self.progress_view("whisper"),
            folder_note: format!("{}{}\\whisper · {}", l.t("Папка: ", "Folder: "), s.dir, l.t("порт 8179 (если занят, берётся следующий свободный; ваш собственный сервер на 8178 не затрагивается)", "port 8179 (the next free one if busy; your own server on 8178 is left alone)")),
            delete: any_installed.then(|| (if self.armed { l.t("Точно удалить?", "Really delete?") } else { l.t("Удалить с диска", "Delete from disk") }.to_string(), !lock)),
            cloud_title: l.t("Через облако", "Through the cloud").into(),
            cloud_sub: format!(
                "{}{}",
                l.t("Groq и Gemini бесплатны, OpenAI платный. Нужен ключ API; запись уходит на сервер выбранного сервиса только когда вы закончили диктовку.", "Groq and Gemini are free, OpenAI is paid. An API key is needed; the recording goes to the chosen service only when you finish dictating."),
                if self.key_saved { format!(" {}", l.t("Ключ уже сохранён.", "A key is already saved.")) } else { String::new() }
            ),
            cloud_button: l.t("Выбрать сервис и ключ", "Choose service and key").into(),
            key_note: self.key_saved.then(|| l.t("Ключ уже сохранён.", "A key is already saved.").to_string()),
        };

        // chat
        let up: Vec<&Server> = s.servers.iter().filter(|x| x.up).collect();
        let servers = up
            .iter()
            .map(|sv| {
                let used_here = s.ai_on && s.ai_base_url.trim_end_matches('/') == sv.url;
                let selected = self.server_sel.get(&sv.id).cloned().filter(|m| sv.models.contains(m)).or_else(|| if used_here && sv.models.contains(&s.ai_model) { Some(s.ai_model.clone()) } else { None }).or_else(|| sv.models.first().cloned()).unwrap_or_default();
                let in_use = used_here && selected == s.ai_model;
                ServerRow {
                    id: sv.id.clone(),
                    name: sv.name.clone(),
                    sub: if sv.models.is_empty() { sv.url.clone() } else { format!("{}{}{}", sv.models.len(), l.t(" моделей · ", " models · "), sv.url) },
                    models: sv.models.clone(),
                    selected,
                    button: if in_use { l.t("Используется", "In use") } else { l.t("Использовать", "Use") }.into(),
                    button_enabled: !sv.models.is_empty() && !in_use,
                    model_picker_enabled: !sv.models.is_empty(),
                    result: self.result.get(&format!("use:{}", sv.id)).map(|r| (r.ok, r.text.clone())),
                }
            })
            .collect();
        let o = &s.ollama;
        let ollama = if !o.up {
            let control = if o.installed {
                OllamaControl::Installed { note: l.t("Ollama установлена, но не запущена: откройте её из меню Пуск.", "Ollama is installed but not running: open it from the Start menu.").into() }
            } else if o.installer_ready {
                OllamaControl::RunInstaller { label: l.t("Запустить установщик", "Run the installer").into(), enabled: !lock }
            } else {
                OllamaControl::Download { label: l.t(&format!("Скачать установщик (≈ {})", size_text(o.installer_size as f64, l)), &format!("Download the installer (≈ {})", size_text(o.installer_size as f64, l))).into(), enabled: !lock }
            };
            OllamaBlock::Setup {
                title: l.t("Ollama: программа для локальных моделей", "Ollama: the app for local models").into(),
                sub: if o.installed {
                    l.t("Официальный установщик уже не нужен.", "The official installer is not needed any more.").to_string()
                } else {
                    let sz = size_text(o.installer_size as f64, l);
                    l.t(
                        &format!("Официальный установщик с ollama.com (перенаправляет на GitHub), около {sz}. Скачивание его не запускает: запуск отдельной кнопкой, дальше запрос Windows. Хеша для проверки нет: сверьте издателя в окне Windows."),
                        &format!("The official installer from ollama.com (redirects to GitHub), about {sz}. Downloading does not start it: running is a separate button, then the Windows prompt. There is no published hash: check the publisher in the prompt."),
                    )
                    .to_string()
                },
                control,
                progress: self.progress_view("ollama-setup"),
            }
        } else {
            let have = |n: &str| s.servers.iter().find(|x| x.id == "ollama").map_or(false, |x| x.models.iter().any(|m| m == n || *m == format!("{n}:latest")));
            let pr = self.result.get("ollama-pull");
            OllamaBlock::Models {
                note: l.t("Рекомендуемые небольшие модели. Скачивает сама Ollama (источник ollama.com), Pill только передаёт ей команду и показывает ход.", "Recommended small models. Ollama itself downloads them (source: ollama.com); Pill only sends the command and shows the progress.").into(),
                rows: o
                    .catalog
                    .iter()
                    .map(|(n, sz)| CatalogRow {
                        name: n.clone(),
                        size_text: format!("≈ {}", size_text(*sz as f64, l)),
                        pulled: have(n).then(|| l.t("уже скачана", "already pulled").to_string()),
                        download: (!have(n)).then(|| (l.t("Скачать", "Download").to_string(), !lock)),
                    })
                    .collect(),
                progress: self.progress_view("ollama-pull"),
                use_pulled: pr.filter(|r| r.ok).and_then(|r| r.model.as_ref()).map(|m| (format!("{}{m}", l.t("Использовать ", "Use ")), true)),
            }
        };
        let chat = ChatSection {
            caption: l.t("ИИ-чат", "AI chat").into(),
            local_title: l.t("Локальная модель", "Local model").into(),
            local_sub: if up.is_empty() {
                l.t("Запущенных локальных серверов (Ollama, LM Studio, llama.cpp, FreeLLMAPI) не найдено. Запросы к ним идут только на 127.0.0.1.", "No local servers (Ollama, LM Studio, llama.cpp, FreeLLMAPI) found. Requests to them go to 127.0.0.1 only.").to_string()
            } else {
                format!("{}{}.", l.t("Найдено на этом компьютере: ", "Found on this computer: "), up.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", "))
            },
            local_found: !up.is_empty(),
            check_again: l.t("Проверить снова", "Check again").into(),
            servers,
            ollama,
            cloud_title: l.t("Через облако", "Through the cloud").into(),
            cloud_sub: l.t("Любой OpenAI-совместимый сервис (OpenAI, OpenRouter, Groq, свой сервер). Нужны адрес API и ключ; ключ шифруется Windows и хранится только здесь.", "Any OpenAI-compatible service (OpenAI, OpenRouter, Groq, your own server). An API address and a key are needed; the key is encrypted by Windows and stays here.").into(),
            cloud_button: l.t("Настроить облачный ИИ", "Set up a cloud AI").into(),
        };
        let footer = FooterView {
            title: if s.done { l.t("Это окно больше не открывается само при запуске", "This window no longer opens by itself at start") } else { l.t("Настрою позже", "I will set this up later") }.into(),
            sub: l.t("Страница остаётся в настройках, в разделе «Установка ИИ»: вернуться можно в любой момент.", "The page stays in Settings under “AI setup”: you can come back at any time.").into(),
            button: if s.done { l.t("Показывать при запуске", "Show at start again") } else { l.t("Позже", "Later") }.into(),
            button_enabled: s.done || !lock,
            note: format!(
                "{}{}. {}",
                l.t("Скачанное лежит в ", "Downloads live in "),
                s.dir,
                l.t("Источники: github.com (движок), huggingface.co (модели Whisper), ollama.com (установщик Ollama и её модели).", "Sources: github.com (engine), huggingface.co (Whisper models), ollama.com (Ollama installer and its models).")
            ),
        };
        View::Page {
            intro: l.t(
                "Голос в текст и ИИ-чат можно скачать на этот компьютер или подключить облачный сервис. Ничего не скачивается и не запускается без вашего нажатия; у кнопки указано, откуда и сколько.",
                "Voice to text and the AI chat can run on this computer or through a cloud service. Nothing is downloaded or started without your click; the button shows the source and size.",
            )
            .into(),
            voice,
            chat,
            footer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Value {
        json!({
            "done": false, "dir": "C:\\Users\\me\\AppData\\Roaming\\pill\\ai", "busy": null,
            "whisper": {
                "engine": { "tag": "v1.9.2", "size": 8_194_445u64, "host": "github.com", "installed": false },
                "models": [
                    { "id": "base", "size": 147_951_465u64, "host": "huggingface.co", "installed": false },
                    { "id": "small", "size": 487_601_967u64, "host": "huggingface.co", "installed": false },
                    { "id": "turbo", "size": 574_041_195u64, "host": "huggingface.co", "installed": false }
                ],
                "enabled": false, "running": null, "model": "small"
            },
            "ollama": { "installed": false, "up": false, "installer": { "host": "ollama.com", "size": 1_200_000_000u64, "ready": false }, "catalog": [{ "name": "qwen3:4b", "size": 2_600_000_000u64 }, { "name": "llama3.2:3b", "size": 2_000_000_000u64 }] },
            "servers": [
                { "id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": false, "models": [] },
                { "id": "freellm", "name": "FreeLLMAPI", "url": "http://127.0.0.1:3001/v1", "up": false, "models": [] }
            ],
            "ai": { "on": false, "baseUrl": "", "model": "" }
        })
    }
    /// Backend that answers from a mutable status document and records calls.
    struct Be {
        st: Value,
        log: Vec<(String, Value)>,
        reply: HashMap<&'static str, Value>,
        key: bool,
    }
    impl Be {
        fn new() -> Self {
            Be { st: status(), log: vec![], reply: HashMap::new(), key: false }
        }
        fn answer(&mut self, c: &BackendCall) -> Reply {
            self.log.push((c.cmd.to_string(), c.args.clone()));
            Reply::ok(match c.cmd {
                "aisetup_status" => self.st.clone(),
                "voice_has_key" => json!({ "has": self.key }),
                cmd => self.reply.get(cmd).cloned().unwrap_or_else(|| json!({ "ok": true })),
            })
        }
        fn cmds(&self) -> Vec<&str> {
            self.log.iter().map(|(c, _)| c.as_str()).collect()
        }
    }
    fn drive(m: &mut AiSetup, be: &mut Be, calls: Vec<BackendCall>) {
        let mut q = calls;
        let mut n = 0;
        while !q.is_empty() {
            n += 1;
            assert!(n < 40);
            let c = q.remove(0);
            let r = be.answer(&c);
            q.extend(m.on_reply(c.tag, r));
        }
    }
    fn page() -> (AiSetup, Be) {
        let mut m = AiSetup::new(Lang::En);
        let mut be = Be::new();
        let c = m.mount();
        drive(&mut m, &mut be, c);
        be.log.clear();
        (m, be)
    }
    fn voice(m: &AiSetup) -> VoiceView {
        let View::Page { voice, .. } = m.view() else { panic!() };
        voice
    }
    fn chat(m: &AiSetup) -> ChatSection {
        let View::Page { chat, .. } = m.view() else { panic!() };
        chat
    }
    fn progress(id: &str, phase: &str, item: &str, done: f64, total: f64, msg: &str) -> Value {
        json!({ "id": id, "item": item, "phase": phase, "done": done, "total": total, "message": msg })
    }

    #[test]
    fn unreadable_state_shows_a_message() {
        let mut m = AiSetup::new(Lang::En);
        assert_eq!(m.view(), View::Unavailable("Could not read the state.".into()));
        let c = m.mount();
        m.on_reply(c[0].tag, Reply::err("x"));
        assert!(matches!(m.view(), View::Unavailable(_)));
        m.set_lang(Lang::Ru);
        assert_eq!(m.view(), View::Unavailable("Не удалось прочитать состояние.".into()));
        assert_eq!(m.on_action(Action::CheckAgain).len(), 2, "check again re-reads even without a state");
        assert!(m.on_action(Action::WhisperInstall).is_empty());
    }
    #[test]
    fn fresh_machine_plan_lists_sources_sizes_and_the_total() {
        let (m, _) = page();
        let v = voice(&m);
        assert!(v.local_sub.ends_with("Now: not installed or off"));
        assert!(v.switch.is_none() && v.delete.is_none());
        assert_eq!(v.chips.len(), 3);
        assert_eq!((v.chips[1].name.as_str(), v.chips[1].size_text.as_str(), v.chips[1].on), ("Small", "488 MB", true));
        assert!(v.chips[1].on_disk_badge.is_none());
        let Plan::Download { items, button, enabled, .. } = v.plan else { panic!() };
        assert_eq!(items, vec!["whisper.cpp v1.9.2 engine (Windows, CPU) · 8 MB · github.com", "Small model · 488 MB · huggingface.co"]);
        assert_eq!(button, "Download 496 MB and turn on");
        assert!(enabled);
        assert_eq!(v.model_note, "A good balance: Russian and English, fast even on a CPU. Recommended.");
        assert!(v.folder_note.contains("port 8179"));
        // sizes over 1 GB use one decimal
        assert_eq!(size_text(1_500_000_000.0, Lang::En), "1.5 GB");
        assert_eq!(size_text(1_500_000_000.0, Lang::Ru), "1.5 ГБ");
    }
    #[test]
    fn selecting_a_model_changes_the_plan_and_unknown_models_fall_back() {
        let (mut m, mut be) = page();
        m.on_action(Action::SelectWhisperModel("turbo".into()));
        let Plan::Download { items, .. } = voice(&m).plan else { panic!() };
        assert_eq!(items[1], "Large-v3-turbo (q5) model · 574 MB · huggingface.co");
        assert!(voice(&m).model_note.starts_with("The best quality of the three."));
        m.on_action(Action::SelectWhisperModel("gone".into()));
        let c = m.on_action(Action::CheckAgain);
        drive(&mut m, &mut be, c);
        assert_eq!(m.selected_model(), "small", "a model that is no longer in the catalog resets to small");
    }
    #[test]
    fn install_flow_download_verify_unpack_start_done() {
        let (mut m, mut be) = page();
        let c = m.on_action(Action::WhisperInstall);
        assert_eq!((c[0].cmd, c[0].args.clone()), ("aisetup_whisper_install", json!({ "model": "small" })));
        drive(&mut m, &mut be, c);
        assert_eq!(m.busy(), Some("whisper"));
        let v = voice(&m);
        assert!(v.progress.running && v.progress.visible);
        assert_eq!(v.progress.line, "whisper.cpp engine · 0 MB");
        assert!(v.progress.indeterminate, "size unknown yet: indeterminate bar");
        assert!(matches!(v.plan, Plan::Download { enabled: false, .. }), "every download button is locked while a job runs");
        // known size
        m.on_event("aisetup:progress", &progress("whisper", "download", "engine", 4_097_223.0, 8_194_445.0, ""));
        let p = voice(&m).progress;
        assert_eq!((p.line.as_str(), p.fill_pct, p.indeterminate), ("whisper.cpp engine · 4 MB / 8 MB · 50 %", 50, false));
        // a few percent over the announced size still counts as known; far over does not
        m.on_event("aisetup:progress", &progress("whisper", "download", "small", 495_000_000.0, 487_601_967.0, ""));
        assert!(!voice(&m).progress.indeterminate);
        m.on_event("aisetup:progress", &progress("whisper", "download", "small", 600_000_000.0, 487_601_967.0, ""));
        let p = voice(&m).progress;
        assert!(p.indeterminate && p.fill_pct == 0 && p.line == "Small model · 600 MB");
        for (phase, text) in [("verify", "Verifying SHA-256…"), ("unpack", "Unpacking…"), ("start", "Starting the server…")] {
            m.on_event("aisetup:progress", &progress("whisper", phase, "engine", 1.0, 1.0, ""));
            let p = voice(&m).progress;
            assert_eq!((p.line.as_str(), p.indeterminate, p.fill_pct), (text, true, 0));
        }
        // done: the job ends, the result line says where it runs, the status is re-read
        be.st["whisper"]["engine"]["installed"] = json!(true);
        be.st["whisper"]["models"][1]["installed"] = json!(true);
        be.st["whisper"]["enabled"] = json!(true);
        be.st["whisper"]["running"] = json!(8179);
        let calls = m.on_event("aisetup:progress", &progress("whisper", "done", "", 0.0, 0.0, "127.0.0.1:8179"));
        assert_eq!(m.busy(), None);
        drive(&mut m, &mut be, calls);
        let v = voice(&m);
        assert_eq!(v.progress.result, Some((true, "Done: voice-to-text now uses the local server 127.0.0.1:8179".into())));
        assert!(!v.progress.running);
        assert!(v.local_running && v.local_sub.ends_with("Now: running on 127.0.0.1:8179 · model Small"));
        assert_eq!(v.switch, Some((true, true)));
        assert_eq!(v.chips[1].in_use_badge.as_deref(), Some("in use"));
        assert_eq!(v.chips[1].on_disk_badge.as_deref(), Some("on disk"));
        assert!(matches!(v.plan, Plan::AllSet { .. }));
    }
    #[test]
    fn install_rejected_by_the_backend_shows_the_error_and_stays_unlocked() {
        let (mut m, mut be) = page();
        be.reply.insert("aisetup_whisper_install", json!({ "ok": false, "error": "Another setup job is running." }));
        let c = m.on_action(Action::WhisperInstall);
        drive(&mut m, &mut be, c);
        assert_eq!(m.busy(), None);
        assert_eq!(voice(&m).progress.result, Some((false, "Another setup job is running.".into())));
        assert_eq!(be.cmds(), vec!["aisetup_whisper_install", "aisetup_status", "voice_has_key"]);
        // a retry clears the old line first
        be.reply.insert("aisetup_whisper_install", json!({ "ok": true }));
        let c = m.on_action(Action::WhisperInstall);
        assert!(voice(&m).progress.result.is_none());
        drive(&mut m, &mut be, c);
        assert_eq!(m.busy(), Some("whisper"));
    }
    #[test]
    fn cancel_error_and_cancelled_phases() {
        let (mut m, mut be) = page();
        let c = m.on_action(Action::WhisperInstall);
        drive(&mut m, &mut be, c);
        let c = m.on_action(Action::CancelJob);
        assert_eq!(c[0].cmd, "aisetup_cancel");
        let calls = m.on_event("aisetup:progress", &progress("whisper", "cancelled", "", 0.0, 0.0, ""));
        drive(&mut m, &mut be, calls);
        assert_eq!(voice(&m).progress.result, Some((false, "Cancelled. The partial file was deleted.".into())));
        assert_eq!(m.busy(), None);
        let c = m.on_action(Action::WhisperInstall);
        drive(&mut m, &mut be, c);
        let calls = m.on_event("aisetup:progress", &progress("whisper", "error", "", 0.0, 0.0, "SHA-256 mismatch"));
        drive(&mut m, &mut be, calls);
        assert_eq!(voice(&m).progress.result, Some((false, "SHA-256 mismatch".into())));
        let c = m.on_action(Action::WhisperInstall);
        drive(&mut m, &mut be, c);
        let calls = m.on_event("aisetup:progress", &progress("whisper", "error", "", 0.0, 0.0, ""));
        drive(&mut m, &mut be, calls);
        assert_eq!(voice(&m).progress.result, Some((false, "error".into())));
    }
    #[test]
    fn a_job_already_running_when_the_page_opens_locks_the_buttons() {
        let mut m = AiSetup::new(Lang::En);
        let mut be = Be::new();
        be.st["busy"] = json!("whisper");
        let c = m.mount();
        drive(&mut m, &mut be, c);
        assert_eq!(m.busy(), Some("whisper"));
        assert!(matches!(voice(&m).plan, Plan::Download { enabled: false, .. }));
        let View::Page { footer, .. } = m.view() else { panic!() };
        assert!(!footer.button_enabled, "\"Later\" is locked while a job runs");
        // an event for the job paints the progress box
        m.on_event("aisetup:progress", &progress("whisper", "download", "small", 100_000_000.0, 487_601_967.0, ""));
        assert_eq!(voice(&m).progress.line, "Small model · 100 MB / 488 MB · 20 %");
    }
    #[test]
    fn switch_enable_and_remove() {
        let (mut m, mut be) = page();
        be.st["whisper"]["engine"]["installed"] = json!(true);
        be.st["whisper"]["models"][1]["installed"] = json!(true);
        let c = m.on_action(Action::CheckAgain);
        drive(&mut m, &mut be, c);
        let v = voice(&m);
        assert_eq!(v.switch, Some((false, true)));
        assert!(matches!(v.plan, Plan::Enable { ref label, .. } if label == "Turn on"));
        be.log.clear();
        let c = m.on_action(Action::WhisperSwitch(true));
        assert_eq!(c[0].args, json!({ "on": true, "model": "small" }));
        let c2 = m.on_action(Action::WhisperSwitch(false));
        assert_eq!(c2[0].args, json!({ "on": false, "model": null }));
        m.on_action(Action::SelectWhisperModel("turbo".into()));
        let c3 = m.on_action(Action::WhisperSwitch(true));
        assert_eq!(c3[0].args["model"], Value::Null, "a model that is not on disk is never passed");
        // failing switch shows the error
        be.reply.insert("aisetup_whisper_set", json!({ "ok": false, "error": "port busy" }));
        drive(&mut m, &mut be, c);
        assert_eq!(voice(&m).progress.result, Some((false, "port busy".into())));
        // enable
        m.on_action(Action::SelectWhisperModel("small".into()));
        be.reply.insert("aisetup_whisper_set", json!({ "ok": true }));
        let c = m.on_action(Action::WhisperEnable);
        assert_eq!(c[0].args, json!({ "on": true, "model": "small" }));
        drive(&mut m, &mut be, c);
        assert_eq!(voice(&m).progress.result, Some((true, "Done: voice-to-text uses the local server.".into())));
        // remove needs two presses within 3 s
        assert_eq!(voice(&m).delete, Some(("Delete from disk".into(), true)));
        assert!(m.on_action(Action::WhisperRemove).is_empty());
        assert_eq!(voice(&m).delete.unwrap().0, "Really delete?");
        m.tick(2999);
        assert_eq!(voice(&m).delete.unwrap().0, "Really delete?");
        m.tick(1);
        assert_eq!(voice(&m).delete.unwrap().0, "Delete from disk");
        m.on_action(Action::WhisperRemove);
        let c = m.on_action(Action::WhisperRemove);
        assert_eq!(c[0].cmd, "aisetup_whisper_remove");
        drive(&mut m, &mut be, c);
        assert_eq!(voice(&m).progress.result, Some((true, "Deleted. Voice-to-text is off.".into())));
    }
    #[test]
    fn running_servers_pick_a_model_and_switch_the_chat() {
        let (mut m, mut be) = page();
        assert!(!chat(&m).local_found);
        assert!(chat(&m).local_sub.starts_with("No local servers"));
        be.st["servers"][0] = json!({ "id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": true, "models": ["llama3", "qwen3"] });
        be.st["ollama"]["up"] = json!(true);
        be.st["ai"] = json!({ "on": true, "baseUrl": "http://127.0.0.1:11434/v1/", "model": "qwen3" });
        let c = m.on_action(Action::CheckAgain);
        drive(&mut m, &mut be, c);
        let ch = chat(&m);
        assert_eq!(ch.local_sub, "Found on this computer: Ollama.");
        let row = &ch.servers[0];
        assert_eq!((row.sub.as_str(), row.selected.as_str(), row.button.as_str(), row.button_enabled), ("2 models · http://127.0.0.1:11434/v1", "qwen3", "In use", false));
        m.on_action(Action::PickServerModel { server: "ollama".into(), model: "llama3".into() });
        let row = chat(&m).servers.remove(0);
        assert_eq!((row.selected.as_str(), row.button.as_str(), row.button_enabled), ("llama3", "Use", true));
        let c = m.on_action(Action::UseServer { server: "ollama".into() });
        assert_eq!((c[0].cmd, c[0].args.clone()), ("aisetup_ai_use", json!({ "server": "ollama", "model": "llama3" })));
        drive(&mut m, &mut be, c);
        assert_eq!(chat(&m).servers[0].result, Some((true, "The chat now uses Ollama · llama3".into())));
        be.reply.insert("aisetup_ai_use", json!({ "ok": false, "error": "no such model" }));
        let c = m.on_action(Action::UseServer { server: "ollama".into() });
        drive(&mut m, &mut be, c);
        assert_eq!(chat(&m).servers[0].result, Some((false, "no such model".into())));
        assert!(m.on_action(Action::UseServer { server: "nope".into() }).is_empty());
    }
    #[test]
    fn server_without_models_has_no_use_button() {
        let (mut m, mut be) = page();
        be.st["servers"][1] = json!({ "id": "freellm", "name": "FreeLLMAPI", "url": "http://127.0.0.1:3001/v1", "up": true, "models": [] });
        let c = m.on_action(Action::CheckAgain);
        drive(&mut m, &mut be, c);
        let row = &chat(&m).servers[0];
        assert_eq!((row.sub.as_str(), row.button_enabled, row.model_picker_enabled), ("http://127.0.0.1:3001/v1", false, false));
    }
    #[test]
    fn ollama_installer_two_clicks_then_models() {
        let (mut m, mut be) = page();
        let OllamaBlock::Setup { control, sub, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(control, OllamaControl::Download { label: "Download the installer (≈ 1.2 GB)".into(), enabled: true });
        assert!(sub.contains("about 1.2 GB") && sub.contains("no published hash"));
        let c = m.on_action(Action::OllamaDownload);
        drive(&mut m, &mut be, c);
        assert_eq!(m.busy(), Some("ollama-setup"));
        m.on_event("aisetup:progress", &progress("ollama-setup", "download", "installer", 600_000_000.0, 1_200_000_000.0, ""));
        let OllamaBlock::Setup { progress: p, control, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.line, "Ollama installer · 600 MB / 1.2 GB · 50 %");
        assert!(matches!(control, OllamaControl::Download { enabled: false, .. }));
        be.st["ollama"]["installer"]["ready"] = json!(true);
        let calls = m.on_event("aisetup:progress", &progress("ollama-setup", "done", "", 0.0, 0.0, ""));
        drive(&mut m, &mut be, calls);
        let OllamaBlock::Setup { progress: p, control, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.result, Some((true, "The installer is downloaded. Press “Run the installer”.".into())));
        assert_eq!(control, OllamaControl::RunInstaller { label: "Run the installer".into(), enabled: true });
        let c = m.on_action(Action::OllamaRunInstaller);
        drive(&mut m, &mut be, c);
        let OllamaBlock::Setup { progress: p, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.result.unwrap().1, "The installer is running. When it finishes, press “Check again”.");
        // installed but not running
        be.st["ollama"]["installed"] = json!(true);
        let c = m.on_action(Action::CheckAgain);
        drive(&mut m, &mut be, c);
        let OllamaBlock::Setup { control, sub, .. } = chat(&m).ollama else { panic!() };
        assert!(matches!(control, OllamaControl::Installed { .. }));
        assert_eq!(sub, "The official installer is not needed any more.");
    }
    #[test]
    fn model_pull_flow_and_use_pulled() {
        let (mut m, mut be) = page();
        be.st["ollama"]["up"] = json!(true);
        be.st["servers"][0] = json!({ "id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": true, "models": ["llama3.2:3b:latest"] });
        let c = m.on_action(Action::CheckAgain);
        drive(&mut m, &mut be, c);
        let OllamaBlock::Models { rows, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(rows[0].download, Some(("Download".into(), true)));
        assert_eq!((rows[0].size_text.as_str(), rows[1].pulled.as_deref()), ("≈ 2.6 GB", Some("already pulled")), "name:latest counts as pulled");
        let c = m.on_action(Action::OllamaPull("qwen3:4b".into()));
        assert_eq!(c[0].args, json!({ "model": "qwen3:4b" }));
        drive(&mut m, &mut be, c);
        m.on_event("aisetup:progress", &progress("ollama-pull", "pull", "qwen3:4b", 1_300_000_000.0, 2_600_000_000.0, "pulling manifest"));
        let OllamaBlock::Models { progress: p, rows, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.line, "qwen3:4b · pulling manifest · 50 %");
        assert_eq!((p.fill_pct, p.indeterminate), (50, false));
        assert_eq!(rows[0].download, Some(("Download".into(), false)));
        m.on_event("aisetup:progress", &progress("ollama-pull", "pull", "qwen3:4b", 0.0, 0.0, "verifying sha256"));
        let OllamaBlock::Models { progress: p, .. } = chat(&m).ollama else { panic!() };
        assert_eq!((p.line.as_str(), p.indeterminate), ("qwen3:4b · verifying sha256", false), "pull never shows the sweep for unknown size");
        be.st["servers"][0]["models"] = json!(["llama3.2:3b:latest", "qwen3:4b"]);
        let calls = m.on_event("aisetup:progress", &progress("ollama-pull", "done", "", 0.0, 0.0, "qwen3:4b"));
        drive(&mut m, &mut be, calls);
        let OllamaBlock::Models { progress: p, use_pulled, rows, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.result, Some((true, "Model downloaded: qwen3:4b".into())));
        assert_eq!(use_pulled, Some(("Use qwen3:4b".into(), true)));
        assert!(rows[0].pulled.is_some());
        let c = m.on_action(Action::UseOllamaModel);
        assert_eq!(c[0].args, json!({ "server": "ollama", "model": "qwen3:4b" }));
        drive(&mut m, &mut be, c);
        let OllamaBlock::Models { progress: p, use_pulled, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.result, Some((true, "The chat now uses Ollama · qwen3:4b".into())));
        assert!(use_pulled.is_none(), "the use button is gone once the chat was switched");
        assert!(m.on_action(Action::UseOllamaModel).is_empty());
    }
    #[test]
    fn pull_rejected_and_cancelled() {
        let (mut m, mut be) = page();
        be.st["ollama"]["up"] = json!(true);
        let c = m.on_action(Action::CheckAgain);
        drive(&mut m, &mut be, c);
        be.reply.insert("aisetup_ollama_pull", json!({ "ok": false, "error": "That model is not in the list." }));
        let c = m.on_action(Action::OllamaPull("zzz".into()));
        drive(&mut m, &mut be, c);
        let OllamaBlock::Models { progress: p, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.result, Some((false, "That model is not in the list.".into())));
        be.reply.insert("aisetup_ollama_pull", json!({ "ok": true }));
        let c = m.on_action(Action::OllamaPull("qwen3:4b".into()));
        drive(&mut m, &mut be, c);
        let calls = m.on_event("aisetup:progress", &progress("ollama-pull", "cancelled", "", 0.0, 0.0, ""));
        drive(&mut m, &mut be, calls);
        let OllamaBlock::Models { progress: p, .. } = chat(&m).ollama else { panic!() };
        assert_eq!(p.result.unwrap().1, "Cancelled. The partial file was deleted.");
    }
    #[test]
    fn footer_later_marks_done_and_closes_settings() {
        let (mut m, mut be) = page();
        let View::Page { footer, .. } = m.view() else { panic!() };
        assert_eq!((footer.title.as_str(), footer.button.as_str(), footer.button_enabled), ("I will set this up later", "Later", true));
        let c = m.on_action(Action::ToggleLater);
        assert_eq!((c[0].cmd, c[0].args.clone(), c[1].cmd), ("settings_set", json!({ "patch": { "aiSetupDone": true } }), "settings_close"));
        drive(&mut m, &mut be, c);
        let View::Page { footer, .. } = m.view() else { panic!() };
        assert_eq!((footer.title.as_str(), footer.button.as_str()), ("This window no longer opens by itself at start", "Show at start again"));
        let c = m.on_action(Action::ToggleLater);
        assert_eq!(c[0].args, json!({ "patch": { "aiSetupDone": false } }));
        assert_eq!(c.len(), 1, "re-enabling does not close the window");
        drive(&mut m, &mut be, c);
        let View::Page { footer, .. } = m.view() else { panic!() };
        assert_eq!(footer.button, "Later");
        assert!(footer.note.starts_with("Downloads live in C:\\Users\\me\\AppData\\Roaming\\pill\\ai. Sources: github.com"));
    }
    #[test]
    fn cloud_links_and_the_saved_voice_key_note() {
        let mut m = AiSetup::new(Lang::En);
        let mut be = Be::new();
        be.key = true;
        let c = m.mount();
        drive(&mut m, &mut be, c);
        assert_eq!(voice(&m).key_note.as_deref(), Some("A key is already saved."));
        assert!(voice(&m).cloud_sub.ends_with("A key is already saved."));
        m.on_action(Action::GoVoice);
        m.on_action(Action::GoAi);
        assert_eq!(m.fx.take_effects(), vec![Effect::GoPage("voice"), Effect::GoPage("ai")]);
        assert_eq!(chat(&m).cloud_button, "Set up a cloud AI");
    }
    #[test]
    fn leaving_the_page_ignores_late_events_and_replies() {
        let (mut m, mut be) = page();
        let c = m.on_action(Action::WhisperInstall);
        m.leave();
        drive(&mut m, &mut be, c);
        assert_eq!(m.busy(), None);
        assert!(m.on_event("aisetup:progress", &progress("whisper", "download", "engine", 1.0, 2.0, "")).is_empty());
        assert_eq!(m.busy(), None);
        assert!(m.on_action(Action::CheckAgain).is_empty());
        assert_eq!(m.mount().len(), 2, "mount revives it");
    }
    #[test]
    fn russian_page_texts() {
        let (mut m, _) = page();
        m.set_lang(Lang::Ru);
        let v = voice(&m);
        assert_eq!(v.caption, "Голос в текст (Whisper)");
        let Plan::Download { button, items, .. } = v.plan else { panic!() };
        assert_eq!(button, "Скачать 496 МБ и включить");
        assert_eq!(items[0], "Движок whisper.cpp v1.9.2 (Windows, процессор) · 8 МБ · github.com");
        let View::Page { intro, .. } = m.view() else { panic!() };
        assert!(intro.starts_with("Голос в текст и ИИ-чат"));
    }
    #[test]
    fn progress_events_ignore_garbage() {
        let (mut m, _) = page();
        assert!(m.on_event("aisetup:progress", &Value::Null).is_empty());
        assert!(m.on_event("other", &json!({})).is_empty());
        m.on_event("aisetup:progress", &json!({ "id": "whisper", "phase": "download" }));
        assert_eq!(m.busy(), Some("whisper"));
        assert_eq!(voice(&m).progress.line, " · 0 MB", "same as the JS with an empty item name");
    }
}
