//! Clicks of the custom widgets, the answers of their commands, and the download job events.
use super::custom::City;
use super::ui::{key, Act};
use super::{Pend, SettingsView};
use crate::ui::view::Cx;
use pill_ui_core::settings::aisetup::{self as ais, OllamaStep, WhisperAction};
use pill_ui_core::settings::calls::BackendCall as C;
use pill_ui_core::settings::flows::{self, Msg as FMsg, Progress};
use pill_ui_core::settings::hotkey;
use serde_json::{json, Value};

impl SettingsView {
    pub(super) fn custom_act(&mut self, a: Act, cx: &mut Cx) {
        let lang = self.lang();
        let wj = self.data.status.get("whisper").cloned().unwrap_or(Value::Null);
        match a {
            Act::Provider(kind, id) => {
                let k = if kind == "ai" { "ai.provider" } else { "voice.provider" };
                self.start_pop(key(&format!("chip:{kind}:{}", id)));
                if let Ok(calls) = self.model.try_set(k, json!(id)) {
                    self.run(cx, calls);
                }
                self.sync_page();
            }
            Act::OpenLink(url) => {
                self.call(cx, C::OpenLink { url }, "");
            }
            Act::Reaction(id) => {
                if let Some(st) = self.model.reaction.set(id) {
                    self.pets_set_state(st.pet);
                }
            }
            Act::PetCard(id) => {
                if self.model.pets.select(&id).is_some() {
                    if let Ok(calls) = self.model.try_set("pet", json!(id)) {
                        self.run(cx, calls);
                    }
                    self.sync_page();
                }
            }
            Act::StyleCard(id) => {
                if pill_ui_core::settings::pickers::select_style(&id).is_some() {
                    if let Ok(calls) = self.model.try_set("character", json!(id)) {
                        self.run(cx, calls);
                    }
                    self.sync_page();
                }
            }
            Act::CitySearch => {
                let q = self.data.fields.get("city").map(|f| f.text()).unwrap_or_default();
                if flows::weather_query_ok(&q) {
                    self.call(cx, C::WeatherSearchCity { query: q.trim().to_string(), lang: lang.code().to_string() }, "weather.city");
                }
            }
            Act::CityPick(i) => {
                if let Some(c) = self.data.cities.get(i).cloned() {
                    self.call(cx, C::WeatherSetCity { name: c.name, lat: c.lat, lon: c.lon }, "weather.city");
                }
            }
            Act::ProfileSave => {
                let raw = self.data.fields.get("profile").map(|f| f.text()).unwrap_or_default();
                if let Some(name) = flows::profile_name(&raw) {
                    if self.model.profiles().len() >= flows::MAX_PROFILES && !self.model.profiles().iter().any(|p| p.0 == name) {
                        self.row_message("modules.profiles", FMsg::bad(pill_ui_core::settings::tr::pick(lang, "Не больше 8 профилей: удалите ненужный", "At most 8 profiles: delete one first")));
                    } else {
                        self.call(cx, C::ProfileSave { name }, "modules.profiles");
                        if let Some(f) = self.data.fields.get_mut("profile") {
                            f.set_text("");
                        }
                    }
                }
            }
            Act::ProfileApply(name) => {
                self.call(cx, C::ProfileApply { name: name.clone() }, &format!("modules.profiles|{name}"));
            }
            Act::ProfileDelete(name) => {
                self.call(cx, C::ProfileDelete { name }, "modules.profiles");
            }
            Act::EvDownload => {
                self.data.job_result = None;
                self.call(cx, C::EverythingDownload, "search.everything");
            }
            Act::EvRun => {
                self.call(cx, C::EverythingRunInstaller, "search.everything");
            }
            Act::JobCancel => {
                self.call(cx, C::AisetupCancel, "");
            }
            Act::WhisperModel(id) => self.data.sel_model = id,
            Act::WhisperPrimary => {
                let plan = ais::whisper_plan(lang, &wj, &self.data.sel_model);
                let call = match plan.action {
                    WhisperAction::Download { call, .. } | WhisperAction::Enable { call, .. } => Some(call),
                    WhisperAction::Running => None,
                };
                if let Some(c) = call {
                    self.data.job_result = None;
                    self.call(cx, c, "whisper");
                }
            }
            Act::WhisperSwitch => {
                let on = !wj.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                let c = ais::whisper_switch_call(&wj, &self.data.sel_model, on);
                self.data.wtoggle.set_on(on, true);
                self.call(cx, c, "whisper");
            }
            Act::WhisperDelete => {
                if self.data.del.click(self.now_ms as u64) {
                    self.call(cx, C::AisetupWhisperRemove, "whisper");
                }
            }
            Act::ServerUse(server, model) => {
                self.call(cx, ais::use_call(&server, &model), "chat");
            }
            Act::OllamaPrimary => {
                let ol = self.data.status.get("ollama").cloned().unwrap_or(Value::Null);
                let c = match ais::ollama_step(lang, &ol) {
                    OllamaStep::RunInstaller { call, .. } | OllamaStep::DownloadInstaller { call, .. } => Some(call),
                    _ => None,
                };
                if let Some(c) = c {
                    self.data.job_result = None;
                    self.call(cx, c, "ollama");
                }
            }
            Act::ModelPull(name) => {
                self.data.job_result = None;
                self.call(cx, ais::pull_call(&name), "ollama");
            }
            Act::Export => {
                self.call(cx, C::SettingsExport, "general.backup");
            }
            Act::Import => {
                self.call(cx, C::UiPickFile, "general.backup");
            }
            Act::Disclosure => {
                self.data.whatsnew_open = !self.data.whatsnew_open;
                let to = self.data.whatsnew_open as u8 as f32;
                let dur = if self.reduce() { 0.0 } else { 0.34 };
                self.data.whatsnew = glasskit::ease::Tween::new(self.data.whatsnew.value(), to, dur, glasskit::ease::curves::EASE_OUT);
            }
            _ => {}
        }
    }
}

impl SettingsView {
    pub(super) fn custom_reply(&mut self, p: &Pend, res: &Result<Value, String>, cx: &mut Cx) {
        let lang = self.lang();
        let v = res.clone().unwrap_or(Value::Null);
        let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let tag = p.tag.clone();
        match &p.call {
            C::LauncherList => self.data.launcher = hotkey::launcher_shortcuts(&v),
            C::ClipStatus => self.data.clip_count = v.get("count").and_then(Value::as_i64).unwrap_or(0),
            C::ClipWipe => self.row_message(&tag, FMsg::ok(flows::WIPE_DONE.get(lang))),
            C::AiConfig => self.data.ai_has_key = v.get("hasKey").and_then(Value::as_bool).unwrap_or(false),
            C::VoiceHasKey { .. } => self.data.voice_source = v.get("source").and_then(Value::as_str).unwrap_or("").to_string(),
            C::AisetupStatus => {
                self.data.ai_done = v.get("done").and_then(Value::as_bool).unwrap_or(false);
                let on = v.get("whisper").and_then(|w| w.get("enabled")).and_then(Value::as_bool).unwrap_or(false);
                self.data.wtoggle.set_on(on, false);
                if let Some(j) = v.get("busy").and_then(Value::as_str) {
                    self.data.job = Some(j.to_string());
                }
                self.data.status = v;
            }
            C::AisetupWhisperInstall { .. } | C::AisetupWhisperSet { .. } | C::AisetupWhisperRemove | C::AisetupOllamaDownload | C::AisetupOllamaRunInstaller | C::AisetupOllamaPull { .. } | C::AisetupAiUse { .. } => {
                if !ok && v.get("error").is_some() {
                    self.data.job = None;
                    self.data.job_result = Some((tag.clone(), FMsg::bad(v.get("error").and_then(Value::as_str).unwrap_or("error"))));
                } else if matches!(p.call, C::AisetupWhisperInstall { .. } | C::AisetupOllamaDownload | C::AisetupOllamaPull { .. }) && ok {
                    self.data.job = Some(match p.call {
                        C::AisetupWhisperInstall { .. } => "whisper".into(),
                        C::AisetupOllamaDownload => "ollama-setup".into(),
                        _ => "ollama-pull".into(),
                    });
                }
                self.call(cx, C::AisetupStatus, "");
            }
            C::EverythingStatus => {
                let ev = flows::everything_view(lang, &v);
                if v.get("installed").and_then(Value::as_bool).unwrap_or(false) {
                    self.data.ev_poll = 0;
                }
                self.data.ev = Some(ev);
            }
            C::EverythingDownload => {
                if ok {
                    self.data.job = Some("everything-es".into());
                } else {
                    self.data.job_result = Some(("everything-".into(), flows::everything_start_error(lang, &v)));
                }
            }
            C::EverythingRunInstaller => {
                let m = flows::everything_installer_result(lang, &v);
                if ok {
                    self.data.ev_poll = flows::EVERYTHING_POLL_TRIES;
                    self.ensure_ticker(cx, 500);
                }
                self.data.job_result = Some(("everything-".into(), m));
            }
            C::SettingsExport => self.row_message(&tag, flows::export_result(lang, &v)),
            C::SettingsImport { .. } => {
                let (m, good) = flows::import_result(lang, &v);
                self.row_message(&tag, m);
                if good {
                    self.call(cx, C::SettingsGet, "");
                }
            }
            C::ProfileSave { .. } | C::ProfileApply { .. } | C::ProfileDelete { .. } => {
                let (row, name) = tag.split_once('|').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((tag.clone(), String::new()));
                let text = match &p.call {
                    C::ProfileSave { .. } => flows::profile_saved_text(lang).to_string(),
                    C::ProfileApply { .. } => flows::profile_applied_text(lang, &name),
                    _ => String::new(),
                };
                self.row_message(&row, flows::profile_result(lang, &v, &text));
                self.call(cx, C::SettingsGet, "");
            }
            C::WeatherSearchCity { .. } => {
                self.data.cities = v
                    .get("results")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| {
                                let name = c.get("name").and_then(Value::as_str)?.to_string();
                                let mut label = name.clone();
                                for k in ["admin1", "country"] {
                                    if let Some(x) = c.get(k).and_then(Value::as_str) {
                                        label = format!("{label}, {x}");
                                    }
                                }
                                Some(City { label, name, lat: c.get("lat").and_then(Value::as_f64)?, lon: c.get("lon").and_then(Value::as_f64)? })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.row_message(&tag, flows::weather_search_result(lang, &v));
            }
            C::WeatherSetCity { name, .. } => {
                if v.get("ok").and_then(Value::as_bool).unwrap_or(true) && !v.is_null() {
                    self.model.on_city_saved(name);
                    self.data.cities.clear();
                    self.row_message(&tag, FMsg::ok(flows::CITY_SAVED.get(lang)));
                }
            }
            C::WeatherGet { .. } => self.row_message(&tag, flows::weather_refresh_result(lang, &v)),
            _ => {}
        }
    }

    pub(super) fn job_event(&mut self, v: &Value, cx: &mut Cx) {
        let lang = self.lang();
        let Some(q) = Progress::from_event(v) else { return };
        if q.is_final() {
            let id = if q.id.starts_with("everything-") { "everything-".to_string() } else { q.id.clone() };
            self.data.job = None;
            self.data.progress = None;
            self.data.job_result = Some((id.clone(), flows::progress_final(lang, &q)));
            if id == "everything-" {
                self.call(cx, C::EverythingStatus, "");
            } else {
                self.call(cx, C::AisetupStatus, "");
            }
        } else {
            self.data.job = Some(q.id.clone());
            self.data.progress = Some(q);
        }
    }
}

impl SettingsView {
    /// The file chosen for the settings import (or why there is none): the import call, or a message under the backup row.
    pub(super) fn on_picked(&mut self, r: Result<String, String>, cx: &mut Cx) {
        let lang = self.lang();
        match r {
            Ok(text) => match flows::import_call(lang, text.len() as u64, &text) {
                Ok(call) => {
                    self.call(cx, call, "general.backup");
                }
                Err(m) => self.row_message("general.backup", m),
            },
            Err(e) if e == "cancelled" => {}
            Err(e) if e == "big" => {
                let m = flows::import_call(lang, flows::MAX_IMPORT_BYTES + 1, "").err();
                self.row_message("general.backup", m.unwrap_or_else(|| flows::read_error(lang)));
            }
            Err(_) => self.row_message("general.backup", flows::read_error(lang)),
        }
    }
}
