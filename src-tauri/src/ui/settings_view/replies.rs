//! Messages from outside the UI thread: command replies, backend events, window hook messages, ticks.
use super::page::*;
use super::{Local, Pend, SettingsView};
use crate::ui::view::{Cx, Msg};
use pill_ui_core::settings::calls::BackendCall as C;
use pill_ui_core::settings::flows::{self, Msg as FMsg};
use pill_ui_core::settings::hotkey::HotkeyOutcome;
use pill_ui_core::settings::nav::{self, OpenTarget};
use serde_json::Value;

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect()).unwrap_or_default()
}

fn devices(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|d| match d {
                    Value::String(s) => Some((s.clone(), s.clone())),
                    Value::Object(_) => {
                        let id = d.get("id").and_then(Value::as_str)?.to_string();
                        let name = d.get("name").and_then(Value::as_str).unwrap_or(&id).to_string();
                        Some((id, name))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

impl SettingsView {
    pub(super) fn on_msg(&mut self, msg: Msg, cx: &mut Cx) {
        match msg {
            Msg::Reply { ticket, result } => {
                if let Some(p) = self.pend.remove(&ticket.0) {
                    self.on_reply(p, result, cx);
                }
            }
            Msg::Event { name, payload, .. } => self.on_event(&name, &payload, cx),
            Msg::Open(args) => {
                let page = args.get("page").and_then(Value::as_str);
                match nav::resolve_open(page) {
                    Some(OpenTarget::Wizard) => self.start_wizard(cx),
                    Some(OpenTarget::Page(p)) => self.show_page(p, cx),
                    None => {}
                }
            }
            Msg::Custom(b) => {
                if let Ok(l) = b.downcast::<Local>() {
                    match *l {
                        Local::Geometry { maximized } => {
                            self.maximized = maximized;
                            self.caption_dirty = true;
                        }
                        Local::Tick => self.on_tick(cx),
                        Local::Picked(r) => self.on_picked(r, cx),
                    }
                }
            }
        }
        cx.request_frame();
    }

    pub(super) fn on_event(&mut self, name: &str, payload: &Value, cx: &mut Cx) {
        if name == "aisetup:progress" {
            self.job_event(payload, cx);
            return;
        }
        let calls = self.model.on_event(name, payload);
        if name == "settings:changed" {
            self.sync_page();
        }
        self.run(cx, calls);
    }

    fn set_status(&mut self, id: &str, st: &Value) {
        let lang = self.lang();
        let model = &self.model;
        for r in self.page.rows_mut() {
            if r.row.status == id {
                let enabled = model.get_bool(r.row.key);
                if let Some(m) = flows::status_line(lang, id, enabled, st) {
                    r.status = Some(m);
                }
            }
        }
    }

    /// Monitors / device lists arrived: rebuild the controls that show them.
    pub(super) fn rebuild_dynamic(&mut self) {
        use pill_ui_core::settings::schema::{Control, Dynamic};
        let lang = self.lang();
        let (model, th, dy) = (&self.model, &self.th, &self.dy);
        for r in self.page.rows_mut() {
            let dynamic = matches!(&r.row.control, Control::Segmented { dynamic, .. } | Control::Select { dynamic, .. } if *dynamic != Dynamic::None);
            if dynamic {
                r.ctl = make_ctl(r.row, model, th, dy, lang);
                r.opts = options_of(r.row, dy, lang);
            }
        }
    }
}

impl SettingsView {
    pub(super) fn on_reply(&mut self, p: Pend, res: Result<Value, String>, cx: &mut Cx) {
        let lang = self.lang();
        let v = res.clone().unwrap_or(Value::Null);
        let tag = p.tag.clone();
        match &p.call {
            C::SettingsGet => {
                if let Value::Object(m) = v {
                    if self.loaded {
                        self.model.on_set_result(&Value::Object(m));
                        self.sync_page();
                    } else {
                        self.begin(m, cx);
                    }
                } else if !self.loaded {
                    self.begin(serde_json::Map::new(), cx);
                }
            }
            C::SettingsSet { .. } => {
                if v.is_object() {
                    self.model.on_set_result(&v);
                    self.sync_page();
                }
            }
            C::SettingsInfo => {
                self.version = v.get("version").and_then(Value::as_str).unwrap_or("").to_string();
                self.dir = v.get("dir").and_then(Value::as_str).unwrap_or("").to_string();
                if self.loaded && self.model.current == "updates" && !self.wiz.active() {
                    self.rebuild(cx);
                }
            }
            C::AutostartGet | C::AutostartSet { .. } => {
                let ans = match &res {
                    Ok(x) => Ok(x.as_bool().unwrap_or(false)),
                    Err(e) => Err(e.clone()),
                };
                match self.model.on_autostart(ans) {
                    Some(e) => self.row_message("general.autostart", FMsg::bad(e)),
                    None => self.row_message("general.autostart", FMsg::plain("")),
                }
                self.sync_page();
            }
            C::HotkeySet { action, .. } => match self.model.on_hotkey_result(action, &v) {
                HotkeyOutcome::Saved(_) => self.row_message(action, FMsg::plain("")),
                HotkeyOutcome::Refused(e) => self.row_message(action, FMsg::bad(e.message().get(lang))),
            },
            C::PillMonitors => {
                self.dy.monitors = strs(&v);
                self.rebuild_dynamic();
            }
            C::MediaDevices => {
                self.dy.outputs = devices(&v);
                self.rebuild_dynamic();
            }
            C::MediaCaptureDevices => {
                self.dy.captures = devices(&v);
                self.rebuild_dynamic();
            }
            C::AiModels => self.row_message(&tag, flows::ai_models_result(lang, &v)),
            C::AiTest => self.row_message(&tag, flows::ai_test_result(lang, &v)),
            C::VoiceTestMic => self.row_message(&tag, flows::voice_mic_result(lang, &v)),
            C::VoiceTestService => self.row_message(&tag, flows::voice_service_result(lang, &v)),
            C::MailTest => self.row_message(&tag, flows::mail_test_result(lang, &v)),
            C::ReportProblem => self.row_message(&tag, flows::report_result(lang, &v)),
            C::AgentsHookSnippets => self.row_message(&tag, FMsg::plain(flows::snippets_text(lang, &v))),
            C::UpdateCheck => {
                let m = self.model.update.on_check(lang, &v);
                self.row_message(&tag, m);
            }
            C::UpdateInstall => {
                if let Some(m) = self.model.update.on_install(lang, &v) {
                    self.row_message(&tag, m);
                }
            }
            C::AiSetKey { .. } => {
                self.row_message(&tag, flows::key_saved(lang, &v));
                self.call(cx, C::AiConfig, "");
            }
            C::VoiceSetKey { provider, .. } => {
                self.row_message(&tag, flows::key_saved(lang, &v));
                self.call(cx, C::VoiceHasKey { provider: Some(provider.clone()) }, "");
            }
            C::ObsSetPassword { .. } => {
                if let Some(m) = flows::obs_password_saved(lang, v.get("ok").and_then(Value::as_bool).unwrap_or(false)) {
                    self.row_message(&tag, m);
                }
            }
            C::MailSetPassword { .. } => {
                let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(false);
                self.row_message(&tag, flows::mail_password_saved(lang, ok));
            }
            C::ObsStatus => self.set_status("obs", &v),
            C::SteamStatus => self.set_status("steam", &v),
            C::NotifStatus => self.set_status("notifications", &v),
            C::LiveplusIcsStatus => self.set_status("calendar", &v),
            C::MailStatus => self.set_status("mail", &v),
            _ => self.custom_reply(&p, &res, cx),
        }
    }
}
