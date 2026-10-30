//! "AI settings" popover of the Chat tab (buildSettings in src/tab-chat.js): provider preset, API address, model (a picker once the
//! server's models are known), API key, test call, local-server scan, "turn AI off".
//!
//! Backend: `ai_config`, `settings_set {patch}`, `ai_probe`, `ai_models`, `ai_set_key {key}`, `ai_test`, `net_features_status`.
//! Setting keys written: `ai.preset`, `ai.baseUrl`, `ai.model`, `net.ai`. The API key only ever travels inside the one
//! `ai_set_key` call (its `Debug` is redacted, see `BackendCall`); the model never stores or echoes it, and the backend never
//! sends it back (`ai_config` has only `hasKey`).
//! The chat model owns one `AiSettings` and reads `cfg()` from it, so there is a single source of truth for "is AI ready".

use super::{BackendCall, Fx, Lang, Reply, Tags, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const PRESETS: [(&str, &str); 2] = [("ollama", "http://127.0.0.1:11434/v1"), ("freellm", "http://127.0.0.1:3001/v1")];
fn preset_url(id: &str) -> Option<&'static str> {
    PRESETS.iter().find(|(p, _)| *p == id).map(|(_, u)| *u)
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct AiCfg {
    pub on: bool,
    pub base_url: String,
    pub model: String,
    pub preset: String,
    pub has_key: bool,
    pub url_error: String,
}
impl AiCfg {
    pub fn from_json(v: &Value) -> Self {
        Self {
            on: v["on"].as_bool().unwrap_or(false),
            base_url: v["baseUrl"].as_str().unwrap_or("").into(),
            model: v["model"].as_str().unwrap_or("").into(),
            preset: v["preset"].as_str().unwrap_or("ollama").into(),
            has_key: v["hasKey"].as_bool().unwrap_or(false),
            url_error: v["urlError"].as_str().unwrap_or("").into(),
        }
    }
    /// `on && baseUrl && model`
    pub fn ready(&self) -> bool {
        self.on && !self.base_url.is_empty() && !self.model.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tone {
    #[default]
    Plain,
    Dim,
    Ok,
    Bad,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Msg {
    pub text: String,
    pub tone: Tone,
}
fn msg(text: impl Into<String>, tone: Tone) -> Msg {
    Msg { text: text.into(), tone }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SAction {
    Preset(String),
    /// the address field was committed (change event)
    UrlChanged(String),
    /// the model name was typed or picked
    ModelChanged(String),
    /// the "Scan" button
    Scan,
    /// the key field's Save button; the text is passed on and dropped
    SaveKey(String),
    RemoveKey,
    /// "Test": `url` and `model` are the current field values
    Test { url: String, model: String },
    TurnOff,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PresetBtn {
    pub id: String,
    pub label: String,
    pub on: bool,
    /// status dot after a scan: None = unknown, Some(up)
    pub up: Option<bool>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsView {
    pub open: bool,
    pub title: String,
    pub presets: Vec<PresetBtn>,
    pub scan_msg: Msg,
    pub url_label: String,
    pub url: String,
    pub url_placeholder: String,
    pub url_error: String,
    pub model_label: String,
    pub model: String,
    pub model_placeholder: String,
    /// when non-empty the model is chosen from this list (the current value is first if it is not in the server list)
    pub models: Vec<String>,
    pub scan_label: String,
    pub scan_tip: String,
    pub scan_busy: bool,
    pub key_label: String,
    pub key_placeholder: String,
    pub has_key: bool,
    pub key_msg: Msg,
    pub save_label: String,
    pub remove_label: String,
    pub test_label: String,
    pub test_busy: bool,
    pub test_msg: Msg,
    pub off_label: String,
    pub hosts: String,
    pub hosts_tip: String,
}

#[derive(Debug, Clone)]
enum After {
    Rebuild,
    UpdateSub,
    None,
    /// ai_models after saving the address (manual scan of a custom server)
    ManualModels,
    /// the test call after saving address+model
    Test,
    Close,
}
#[derive(Debug)]
enum Pend {
    Cfg(After),
    Save(After),
    Probe { manual: bool },
    Models,
    Key,
    RemoveKey,
    Test,
    Hosts,
}

pub struct AiSettings {
    pub fx: Fx,
    lang: Lang,
    tags: Tags,
    pend: HashMap<u64, Pend>,
    cfg: AiCfg,
    open: bool,
    models: Vec<String>,
    dots: HashMap<String, bool>,
    scan_msg: Msg,
    scan_busy: bool,
    key_msg: Msg,
    test_msg: Msg,
    test_busy: bool,
    hosts: String,
    /// set when a saved change should make the owner re-run `apply_mode`/`updateSub`
    changed: bool,
    model_draft: Option<String>,
}

impl AiSettings {
    pub fn new(lang: Lang, owner: u8) -> Self {
        Self {
            fx: Fx::default(), lang, tags: Tags::new(owner), pend: HashMap::new(), cfg: AiCfg::default(), open: false, models: vec![], dots: HashMap::new(),
            scan_msg: Msg::default(), scan_busy: false, key_msg: Msg::default(), test_msg: Msg::default(), test_busy: false, hosts: String::new(), changed: false, model_draft: None,
        }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
    }
    pub fn cfg(&self) -> &AiCfg {
        &self.cfg
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    /// True once after a change of the saved configuration (the chat re-applies its mode).
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
    pub fn is_mine(tag: u64, owner: u8) -> bool {
        Tags::owner_of(tag) == owner
    }
    fn call(&mut self, cmd: &'static str, args: Value, p: Pend) -> BackendCall {
        let tag = self.tags.next();
        self.pend.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }
    pub fn load_cfg(&mut self) -> BackendCall {
        self.call("ai_config", json!({}), Pend::Cfg(After::None))
    }
    /// `settings_set {patch}` followed by a config reload (the owner is told through `take_changed`).
    pub fn patch(&mut self, patch: Value) -> BackendCall {
        self.save_cfg(patch, After::None)
    }
    fn save_cfg(&mut self, patch: Value, then: After) -> BackendCall {
        self.call("settings_set", json!({ "patch": patch }), Pend::Save(then))
    }

    /// `show = true` always opens (loading the config first); otherwise toggles.
    pub fn open(&mut self, show: bool) -> Vec<BackendCall> {
        if show || !self.open {
            self.open = true;
            self.fx.anim("ai-settings", super::Anim::Enter { y: 6.0, ms: 220 });
            vec![self.call("ai_config", json!({}), Pend::Cfg(After::Rebuild))]
        } else {
            self.close();
            vec![]
        }
    }
    pub fn close(&mut self) {
        if self.open {
            self.open = false;
            self.fx.anim("ai-settings", super::Anim::Exit { y: -6.0, scale: 0.97, ms: 160 });
        }
    }
    /// buildSettings(): fresh drafts from the saved config, then a quiet scan of the usual local ports.
    fn rebuild(&mut self) -> Vec<BackendCall> {
        self.scan_msg = Msg::default();
        self.key_msg = Msg::default();
        self.test_msg = Msg::default();
        self.model_draft = None;
        self.scan_busy = true;
        self.scan_msg = msg(self.lang.t("Ищу локальные серверы…", "Looking for local servers…"), Tone::Dim);
        vec![self.call("ai_probe", json!({}), Pend::Probe { manual: false }), self.call("net_features_status", json!({}), Pend::Hosts)]
    }

    pub fn on_action(&mut self, a: SAction) -> Vec<BackendCall> {
        match a {
            SAction::Preset(id) => {
                let mut patch = json!({ "ai.preset": id });
                if let Some(u) = preset_url(&id) {
                    patch["ai.baseUrl"] = json!(u);
                }
                vec![self.save_cfg(patch, After::Rebuild)]
            }
            SAction::UrlChanged(v) => {
                let v = v.trim().to_string();
                let c = &self.cfg;
                let keep = c.preset != "custom" && preset_url(&c.preset).map_or(false, |u| u == v);
                let preset = if keep { c.preset.clone() } else { "custom".to_string() };
                vec![self.save_cfg(json!({ "ai.baseUrl": v, "ai.preset": preset }), After::Rebuild)]
            }
            SAction::ModelChanged(v) => {
                self.model_draft = Some(v.trim().to_string());
                vec![self.save_cfg(json!({ "ai.model": v.trim() }), After::UpdateSub)]
            }
            SAction::Scan => {
                self.scan_busy = true;
                self.scan_msg = msg(self.lang.t("Ищу локальные серверы…", "Looking for local servers…"), Tone::Dim);
                vec![self.call("ai_probe", json!({}), Pend::Probe { manual: true })]
            }
            SAction::SaveKey(k) => {
                let k = k.trim().to_string();
                if k.is_empty() {
                    return vec![];
                }
                vec![self.call("ai_set_key", json!({ "key": k }), Pend::Key)]
            }
            SAction::RemoveKey => vec![self.call("ai_set_key", json!({ "key": "" }), Pend::RemoveKey)],
            SAction::Test { url, model } => {
                self.test_busy = true;
                self.test_msg = msg(self.lang.t("Проверяю…", "Testing…"), Tone::Plain);
                vec![self.save_cfg(json!({ "ai.baseUrl": url.trim(), "ai.model": model.trim() }), After::Test)]
            }
            SAction::TurnOff => vec![self.save_cfg(json!({ "net.ai": false }), After::Close)],
            SAction::Close => {
                self.close();
                vec![]
            }
        }
    }

    fn after(&mut self, a: After) -> Vec<BackendCall> {
        match a {
            After::Rebuild => self.rebuild(),
            After::UpdateSub => {
                self.changed = true;
                vec![]
            }
            After::None => vec![],
            After::ManualModels => vec![self.call("ai_models", json!({}), Pend::Models)],
            After::Test => vec![self.call("ai_test", json!({}), Pend::Test)],
            After::Close => {
                self.close();
                vec![]
            }
        }
    }

    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        let Some(p) = self.pend.remove(&tag) else { return vec![] };
        let ru = self.lang.is_ru();
        match p {
            Pend::Cfg(after) => {
                if r.ok && !r.v.is_null() {
                    self.cfg = AiCfg::from_json(&r.v);
                }
                self.changed = true;
                self.after(after)
            }
            // saveCfg(): settings_set, then reload ai_config, then apply the mode
            Pend::Save(after) => {
                vec![self.call("ai_config", json!({}), Pend::Cfg(after))]
            }
            Pend::Hosts => {
                let f = r.v["features"].as_array().and_then(|a| a.iter().find(|x| x["id"] == "ai")).cloned();
                if let Some(f) = f {
                    let hosts: Vec<&str> = f["hosts"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    self.hosts = format!(
                        "{}{} · {}{}",
                        self.lang.t("Запросы уходят только на: ", "Requests go only to: "),
                        if hosts.is_empty() { "—".to_string() } else { hosts.join(", ") },
                        self.lang.t("вызовов: ", "calls: "),
                        f["calls"].as_i64().unwrap_or(0)
                    );
                }
                vec![]
            }
            Pend::Probe { manual } => {
                self.scan_busy = false;
                let found: Vec<Value> = if r.ok { r.v.as_array().cloned().unwrap_or_default() } else { vec![] };
                for f in &found {
                    if let Some(id) = f["id"].as_str() {
                        self.dots.insert(id.to_string(), f["up"].as_bool().unwrap_or(false));
                    }
                }
                let norm = |u: &str| u.trim().trim_end_matches('/').to_string();
                let cur = norm(&self.cfg.base_url);
                let here = found.iter().find(|f| norm(f["url"].as_str().unwrap_or("")) == cur).cloned();
                let name = |f: &Value| f["name"].as_str().unwrap_or("").to_string();
                let others: Vec<String> = found.iter().filter(|f| f["up"].as_bool().unwrap_or(false) && Some(*f) != here.as_ref()).map(name).collect();
                let models_of = |f: &Value| -> Vec<String> { f["models"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() };
                match here {
                    Some(h) if h["up"].as_bool().unwrap_or(false) => {
                        let ms = models_of(&h);
                        self.models = ms.clone();
                        if ms.is_empty() {
                            self.scan_msg = msg(
                                format!("{}{}", name(&h), self.lang.t(" запущен, но моделей нет. Скачайте любую, например: ollama pull qwen3", " is running but has no models. Pull one, e.g.: ollama pull qwen3")),
                                Tone::Bad,
                            );
                            return vec![];
                        }
                        let cur_model = self.model_draft.clone().unwrap_or_else(|| self.cfg.model.clone());
                        let mut out = vec![];
                        if cur_model.is_empty() || !ms.contains(&cur_model) {
                            self.model_draft = Some(ms[0].clone());
                            out.push(self.save_cfg(json!({ "ai.model": ms[0] }), After::UpdateSub));
                        }
                        self.scan_msg = msg(format!("{}{}{}", name(&h), self.lang.t(" найден · моделей: ", " found · models: "), ms.len()), Tone::Ok);
                        out
                    }
                    Some(h) => {
                        let addr = norm(h["url"].as_str().unwrap_or("")).replace("http://", "").replace("/v1", "");
                        let tail = if others.is_empty() {
                            self.lang.t(" Запустите его и нажмите «Найти».", " Start it and press Scan.").to_string()
                        } else {
                            format!("{}{}{}", self.lang.t(" Запущено: ", " Running: "), others.join(", "), self.lang.t(" — выберите его выше.", " — pick it above."))
                        };
                        self.scan_msg = msg(format!("{}{}{}.{}", name(&h), self.lang.t(" не отвечает на ", " is not answering on "), addr, tail), Tone::Bad);
                        vec![]
                    }
                    None if manual => {
                        // a custom address: ask that very server
                        vec![self.save_cfg(json!({ "ai.baseUrl": self.cfg.base_url }), After::ManualModels)]
                    }
                    None => {
                        self.scan_msg = if others.is_empty() { Msg::default() } else { msg(format!("{}{}", self.lang.t("Запущено локально: ", "Running locally: "), others.join(", ")), Tone::Plain) };
                        vec![]
                    }
                }
            }
            Pend::Models => {
                if r.ok && r.v["ok"].as_bool().unwrap_or(false) {
                    let ms: Vec<String> = r.v["models"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
                    self.scan_msg = if ms.is_empty() {
                        msg(self.lang.t("Сервер не вернул моделей", "The server returned no models"), Tone::Plain)
                    } else {
                        msg(format!("{}{}", self.lang.t("Моделей: ", "Models: "), ms.len()), Tone::Ok)
                    };
                    self.models = ms;
                } else {
                    let e = if r.ok { r.v["error"].as_str().unwrap_or("error").to_string() } else { r.err };
                    self.scan_msg = msg(e, Tone::Bad);
                }
                vec![]
            }
            Pend::Key => {
                let ok = r.ok && r.v["ok"].as_bool().unwrap_or(false);
                self.key_msg = if ok {
                    msg(self.lang.t("Ключ сохранён: зашифрован Windows, хранится только здесь.", "Key saved: encrypted by Windows, stored only on this computer."), Tone::Plain)
                } else {
                    msg(r.v["error"].as_str().unwrap_or("error"), Tone::Bad)
                };
                vec![self.call("ai_config", json!({}), Pend::Cfg(After::None))]
            }
            Pend::RemoveKey => {
                self.key_msg = Msg::default();
                vec![self.call("ai_config", json!({}), Pend::Cfg(After::Rebuild))]
            }
            Pend::Test => {
                self.test_busy = false;
                let ok = r.ok && r.v["ok"].as_bool().unwrap_or(false);
                self.test_msg = if ok {
                    msg(format!("{} · {} {}", self.lang.t("Работает", "Works"), r.v["ms"].as_i64().unwrap_or(0), self.lang.t("мс", "ms")), Tone::Ok)
                } else {
                    let e = r.v["error"].as_str().filter(|e| !e.is_empty()).unwrap_or(self.lang.t("Нет ответа", "No answer"));
                    msg(e, Tone::Bad)
                };
                self.changed = true; // applyMode()
                let _ = ru;
                vec![]
            }
        }
    }

    pub fn view(&self) -> SettingsView {
        let l = self.lang;
        let c = &self.cfg;
        let preset = |id: &str, ru: &str, en: &str| PresetBtn { id: id.into(), label: l.t(ru, en).into(), on: c.preset == id, up: self.dots.get(id).copied() };
        let mut models = self.models.clone();
        let model = self.model_draft.clone().unwrap_or_else(|| c.model.clone());
        if !models.is_empty() && !model.is_empty() && !models.contains(&model) {
            models.insert(0, model.clone());
        }
        SettingsView {
            open: self.open,
            title: l.t("Настройки ИИ", "AI settings").into(),
            presets: vec![
                preset("ollama", "Ollama (локально)", "Ollama (local)"),
                preset("freellm", "FreeLLMAPI (локально)", "FreeLLMAPI (local)"),
                PresetBtn { id: "custom".into(), label: l.t("Другой", "Custom").into(), on: c.preset == "custom", up: None },
            ],
            scan_msg: self.scan_msg.clone(),
            url_label: l.t("Адрес API (OpenAI-совместимый)", "API address (OpenAI-compatible)").into(),
            url: c.base_url.clone(),
            url_placeholder: "http://127.0.0.1:11434/v1".into(),
            url_error: c.url_error.clone(),
            model_label: l.t("Модель", "Model").into(),
            model,
            model_placeholder: l.t("имя модели", "model name").into(),
            models,
            scan_label: l.t("Найти", "Scan").into(),
            scan_tip: l.t("Найти локальные серверы и их модели", "Find local servers and their models").into(),
            scan_busy: self.scan_busy,
            key_label: l.t("Ключ API", "API key").into(),
            key_placeholder: if c.has_key { l.t("•••••••• (ключ сохранён)", "•••••••• (key saved)") } else { l.t("не нужен для локальных серверов", "not needed for local servers") }.into(),
            has_key: c.has_key,
            key_msg: self.key_msg.clone(),
            save_label: l.t("Сохранить", "Save").into(),
            remove_label: l.t("Удалить", "Remove").into(),
            test_label: l.t("Проверить", "Test").into(),
            test_busy: self.test_busy,
            test_msg: self.test_msg.clone(),
            off_label: l.t("Выключить ИИ", "Turn AI off").into(),
            hosts: self.hosts.clone(),
            hosts_tip: l.t(
                "История чатов хранится только на этом компьютере. http разрешён только для 127.0.0.1 / localhost.",
                "Chat history stays on this computer. Plain http is allowed only for 127.0.0.1 / localhost.",
            )
            .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scripted backend: settings live in a map, probe/models/test answers are configurable.
    struct Be {
        cfg: Value,
        probe: Value,
        models: Value,
        test: Value,
        key_ok: bool,
        log: Vec<(String, Value)>,
    }
    impl Be {
        fn new() -> Self {
            Be {
                cfg: json!({ "on": true, "baseUrl": "http://127.0.0.1:11434/v1", "model": "", "preset": "ollama", "hasKey": false, "urlError": null }),
                probe: json!([
                    { "id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": true, "models": ["llama3", "qwen3"] },
                    { "id": "freellm", "name": "FreeLLMAPI", "url": "http://127.0.0.1:3001/v1", "up": true, "models": ["x"] },
                    { "id": "lmstudio", "name": "LM Studio", "url": "http://127.0.0.1:1234/v1", "up": false, "models": [] }
                ]),
                models: json!({ "ok": true, "models": ["a", "b"] }),
                test: json!({ "ok": true, "ms": 42, "model": "m" }),
                key_ok: true,
                log: vec![],
            }
        }
        fn answer(&mut self, c: &BackendCall) -> Reply {
            self.log.push((c.cmd.to_string(), c.args.clone()));
            match c.cmd {
                "ai_config" => Reply::ok(self.cfg.clone()),
                "settings_set" => {
                    for (k, v) in c.args["patch"].as_object().unwrap() {
                        match k.as_str() {
                            "ai.preset" => self.cfg["preset"] = v.clone(),
                            "ai.baseUrl" => self.cfg["baseUrl"] = v.clone(),
                            "ai.model" => self.cfg["model"] = v.clone(),
                            "net.ai" => self.cfg["on"] = v.clone(),
                            _ => {}
                        }
                    }
                    Reply::ok(Value::Null)
                }
                "ai_probe" => Reply::ok(self.probe.clone()),
                "ai_models" => Reply::ok(self.models.clone()),
                "ai_test" => Reply::ok(self.test.clone()),
                "ai_set_key" => {
                    let has = !c.args["key"].as_str().unwrap().is_empty();
                    if self.key_ok {
                        self.cfg["hasKey"] = json!(has);
                        Reply::ok(json!({ "ok": true, "hasKey": has }))
                    } else {
                        Reply::ok(json!({ "ok": false, "error": "That does not look like an API key." }))
                    }
                }
                "net_features_status" => Reply::ok(json!({ "features": [{ "id": "ai", "hosts": ["127.0.0.1"], "calls": 7 }] })),
                _ => Reply::err("unknown"),
            }
        }
    }
    fn drive(s: &mut AiSettings, be: &mut Be, calls: Vec<BackendCall>) {
        let mut q = calls;
        let mut n = 0;
        while !q.is_empty() {
            n += 1;
            assert!(n < 40, "runaway");
            let c = q.remove(0);
            let r = be.answer(&c);
            q.extend(s.on_reply(c.tag, r));
        }
    }
    fn open() -> (AiSettings, Be) {
        let mut s = AiSettings::new(Lang::En, 5);
        let mut be = Be::new();
        let c = s.open(true);
        drive(&mut s, &mut be, c);
        (s, be)
    }
    fn cmds(be: &Be) -> Vec<&str> {
        be.log.iter().map(|(c, _)| c.as_str()).collect()
    }

    #[test]
    fn opening_loads_config_scans_and_picks_the_first_model() {
        let (s, be) = open();
        assert_eq!(cmds(&be), vec!["ai_config", "ai_probe", "net_features_status", "settings_set", "ai_config"]);
        assert_eq!(be.log[3].1, json!({ "patch": { "ai.model": "llama3" } }), "an empty model is filled from the running server");
        let v = s.view();
        assert!(v.open && v.models == vec!["llama3", "qwen3"] && v.model == "llama3");
        assert_eq!(v.scan_msg, Msg { text: "Ollama found · models: 2".into(), tone: Tone::Ok });
        assert_eq!(v.presets[0].up, Some(true));
        assert_eq!(v.presets[1].up, Some(true));
        assert_eq!(v.presets[2].up, None);
        assert_eq!(v.hosts, "Requests go only to: 127.0.0.1 · calls: 7");
        assert!(s.cfg().ready());
    }
    #[test]
    fn model_missing_from_the_server_list_is_replaced_by_its_first() {
        let mut s = AiSettings::new(Lang::En, 5);
        let mut be = Be::new();
        be.cfg["model"] = json!("my-finetune");
        let c = s.open(true);
        drive(&mut s, &mut be, c);
        // like the JS scan: a saved model the running server does not list is swapped for the server's first one
        assert_eq!(s.view().models, vec!["llama3", "qwen3"]);
        assert_eq!(s.view().model, "llama3");
        // ...but the picker keeps an unknown draft visible when the user typed one after the scan
        s.on_action(SAction::ModelChanged("my-finetune".into()));
        assert_eq!(s.view().models, vec!["my-finetune", "llama3", "qwen3"]);
    }
    #[test]
    fn preset_switch_writes_preset_and_address() {
        let (mut s, mut be) = open();
        be.log.clear();
        let c = s.on_action(SAction::Preset("freellm".into()));
        assert_eq!(c[0].args, json!({ "patch": { "ai.preset": "freellm", "ai.baseUrl": "http://127.0.0.1:3001/v1" } }));
        drive(&mut s, &mut be, c);
        assert!(s.view().presets[1].on && s.view().url == "http://127.0.0.1:3001/v1");
        let c = s.on_action(SAction::Preset("custom".into()));
        assert_eq!(c[0].args, json!({ "patch": { "ai.preset": "custom" } }), "custom keeps whatever address is there");
    }
    #[test]
    fn typing_a_preset_address_keeps_the_preset_anything_else_is_custom() {
        let (mut s, _) = open();
        let c = s.on_action(SAction::UrlChanged(" http://127.0.0.1:11434/v1 ".into()));
        assert_eq!(c[0].args, json!({ "patch": { "ai.baseUrl": "http://127.0.0.1:11434/v1", "ai.preset": "ollama" } }));
        let c = s.on_action(SAction::UrlChanged("https://api.example.com/v1".into()));
        assert_eq!(c[0].args["patch"]["ai.preset"], json!("custom"));
    }
    #[test]
    fn running_server_without_models_and_not_answering_messages() {
        let mut s = AiSettings::new(Lang::En, 5);
        let mut be = Be::new();
        be.probe = json!([{ "id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1/", "up": true, "models": [] }]);
        let c = s.open(true);
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().scan_msg.tone, Tone::Bad);
        assert!(s.view().scan_msg.text.contains("ollama pull qwen3"));
        be.probe = json!([{ "id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": false, "models": [] },
                          { "id": "freellm", "name": "FreeLLMAPI", "url": "http://127.0.0.1:3001/v1", "up": true, "models": ["x"] }]);
        let c = s.on_action(SAction::Scan);
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().scan_msg.text, "Ollama is not answering on 127.0.0.1:11434. Running: FreeLLMAPI — pick it above.");
        s.set_lang(Lang::Ru);
        be.probe = json!([{ "id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": false, "models": [] }]);
        let c = s.on_action(SAction::Scan);
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().scan_msg.text, "Ollama не отвечает на 127.0.0.1:11434. Запустите его и нажмите «Найти».");
    }
    #[test]
    fn custom_address_manual_scan_asks_that_server() {
        let mut s = AiSettings::new(Lang::En, 5);
        let mut be = Be::new();
        be.cfg["baseUrl"] = json!("https://llm.example.com/v1");
        be.cfg["preset"] = json!("custom");
        let c = s.open(true);
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().scan_msg.text, "Running locally: Ollama, FreeLLMAPI");
        be.log.clear();
        be.probe = json!([]);
        let c = s.on_action(SAction::Scan);
        drive(&mut s, &mut be, c);
        assert_eq!(cmds(&be), vec!["ai_probe", "settings_set", "ai_config", "ai_models"]);
        assert_eq!(s.view().scan_msg, Msg { text: "Models: 2".into(), tone: Tone::Ok });
        assert_eq!(s.view().models, vec!["a", "b"]);
        be.models = json!({ "ok": false, "error": "HTTP 401: bad key" });
        let c = s.on_action(SAction::Scan);
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().scan_msg, Msg { text: "HTTP 401: bad key".into(), tone: Tone::Bad });
    }
    #[test]
    fn quiet_scan_lists_other_running_servers() {
        let mut s = AiSettings::new(Lang::En, 5);
        let mut be = Be::new();
        be.cfg["baseUrl"] = json!("https://llm.example.com/v1");
        be.cfg["model"] = json!("m");
        let c = s.open(true);
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().scan_msg.text, "Running locally: Ollama, FreeLLMAPI");
    }
    #[test]
    fn key_is_sent_once_never_stored_or_logged() {
        let (mut s, mut be) = open();
        let c = s.on_action(SAction::SaveKey("  sk-secret-123  ".into()));
        assert_eq!((c[0].cmd, c[0].args.clone()), ("ai_set_key", json!({ "key": "sk-secret-123" })));
        assert!(!format!("{:?}", c[0]).contains("sk-secret-123"));
        drive(&mut s, &mut be, c);
        let v = s.view();
        assert!(v.has_key && v.key_placeholder == "•••••••• (key saved)");
        assert_eq!(v.key_msg.text, "Key saved: encrypted by Windows, stored only on this computer.");
        assert!(!format!("{v:?}").contains("sk-secret"), "the view never contains the key");
        assert!(s.on_action(SAction::SaveKey("   ".into())).is_empty());
        be.key_ok = false;
        let c = s.on_action(SAction::SaveKey("bad key".into()));
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().key_msg.tone, Tone::Bad);
        be.key_ok = true;
        let c = s.on_action(SAction::RemoveKey);
        assert_eq!(c[0].args, json!({ "key": "" }));
        drive(&mut s, &mut be, c);
        assert!(!s.view().has_key);
    }
    #[test]
    fn test_call_saves_fields_first_and_reports() {
        let (mut s, mut be) = open();
        be.log.clear();
        let c = s.on_action(SAction::Test { url: " http://h/v1 ".into(), model: " m1 ".into() });
        assert!(s.view().test_busy);
        drive(&mut s, &mut be, c);
        assert_eq!(cmds(&be), vec!["settings_set", "ai_config", "ai_test"]);
        assert_eq!(be.log[0].1, json!({ "patch": { "ai.baseUrl": "http://h/v1", "ai.model": "m1" } }));
        assert_eq!(s.view().test_msg, Msg { text: "Works · 42 ms".into(), tone: Tone::Ok });
        assert!(!s.view().test_busy);
        assert!(s.take_changed() && !s.take_changed());
        be.test = json!({ "ok": false, "error": "HTTP 404: nope" });
        let c = s.on_action(SAction::Test { url: "u".into(), model: "m".into() });
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().test_msg, Msg { text: "HTTP 404: nope".into(), tone: Tone::Bad });
        be.test = json!({ "ok": false });
        let c = s.on_action(SAction::Test { url: "u".into(), model: "m".into() });
        drive(&mut s, &mut be, c);
        assert_eq!(s.view().test_msg.text, "No answer");
    }
    #[test]
    fn turn_off_writes_the_switch_and_closes() {
        let (mut s, mut be) = open();
        let c = s.on_action(SAction::TurnOff);
        assert_eq!(c[0].args, json!({ "patch": { "net.ai": false } }));
        drive(&mut s, &mut be, c);
        assert!(!s.cfg().on && !s.is_open());
    }
    #[test]
    fn toggle_open_close_and_animations() {
        let mut s = AiSettings::new(Lang::En, 5);
        assert_eq!(s.open(false).len(), 1);
        assert!(s.is_open());
        assert_eq!(s.fx.take_anims().len(), 1);
        assert!(s.open(false).is_empty() && !s.is_open());
        let a = s.fx.take_anims();
        assert!(matches!(a[0].kind, super::super::Anim::Exit { .. }));
        s.fx.reduce_motion = true;
        s.open(true);
        assert!(s.fx.take_anims().is_empty());
    }
    #[test]
    fn model_pick_is_saved_and_flagged_for_the_owner() {
        let (mut s, mut be) = open();
        s.take_changed();
        let c = s.on_action(SAction::ModelChanged(" qwen3 ".into()));
        assert_eq!(c[0].args, json!({ "patch": { "ai.model": "qwen3" } }));
        drive(&mut s, &mut be, c);
        assert!(s.take_changed());
        assert_eq!(s.cfg().model, "qwen3");
    }
    #[test]
    fn labels_in_russian() {
        let (mut s, _) = open();
        s.set_lang(Lang::Ru);
        let v = s.view();
        assert_eq!((v.title.as_str(), v.presets[2].label.as_str(), v.test_label.as_str()), ("Настройки ИИ", "Другой", "Проверить"));
    }
}
