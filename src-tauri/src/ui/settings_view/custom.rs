//! The 12 `Control::Custom` widgets and the state they share. A custom row paints its whole body itself (label included when it wants);
//! `custom_h` measures it, `custom_paint` draws it and registers its hits, `custom_act` / `custom_reply` / `custom_key` react.
use super::page::*;
use super::rows::*;
use super::txt;
use super::ui::Act;
use super::{Pend, SettingsView};
use crate::ui::view::Cx;
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use pill_ui_core::settings::flows::{Confirm, EverythingView, Progress};
use pill_ui_core::settings::hotkey::LauncherShortcut;
use serde_json::Value;

pub struct Data {
    pub open_page: Option<String>,
    pub wipe: Confirm,
    pub ai_done: bool,
    /// id of the running download job (whisper | ollama-setup | ollama-pull | everything-*)
    pub job: Option<String>,
    pub progress: Option<Progress>,
    pub job_result: Option<(String, pill_ui_core::settings::flows::Msg)>,
    pub voice_source: String,
    pub ai_has_key: bool,
    pub launcher: Vec<LauncherShortcut>,
    pub launcher_loaded: bool,
    pub clip_count: i64,
    pub clip_private: bool,
    pub ev: Option<EverythingView>,
    pub status: Value,
    pub ai_cfg: Value,
    pub net: Value,
    pub tick: u64,
}

impl Default for Data {
    fn default() -> Self {
        Data {
            open_page: None,
            wipe: Confirm::wipe(),
            ai_done: false,
            job: None,
            progress: None,
            job_result: None,
            voice_source: String::new(),
            ai_has_key: false,
            launcher: Vec::new(),
            launcher_loaded: false,
            clip_count: 0,
            clip_private: false,
            ev: None,
            status: Value::Null,
            ai_cfg: Value::Null,
            net: Value::Null,
            tick: 0,
        }
    }
}

impl Data {
    pub fn has_secret(&self, rid: &str) -> bool {
        match rid {
            "ai.key" => self.ai_has_key,
            "voice.key" => self.voice_source == "voice",
            _ => false,
        }
    }
}

impl SettingsView {
    /// Height of a label-only body: title, hint and the row's message lines on the 56 px grid.
    pub(super) fn plain_h(&mut self, d: &Dc, r: &RowRt, w: f32) -> f32 {
        let m = self.row_metrics_plain(d, r, w - 2.0 * PAD_X);
        m.nh
    }

    pub(super) fn row_metrics_plain(&mut self, d: &Dc, r: &RowRt, inner_w: f32) -> RowM {
        let title = r.row.label.get(d.lang);
        let hint = r.row.hint.get(d.lang);
        let title_h = txt::wrapped_h(d.cv, title, st_title(), inner_w, 18.0).max(18.0);
        let hint_h = if hint.is_empty() { 0.0 } else { 2.0 + txt::wrapped_h(d.cv, hint, st_hint(), inner_w, 16.0) };
        let mut msg_h = 0.0;
        if let Some((m, _)) = &r.msg {
            if !m.text.is_empty() {
                msg_h += 2.0 + txt::wrapped_h(d.cv, &m.text, st_hint(), inner_w, 16.0);
            }
        }
        RowM { nh: (11.0 + title_h + hint_h + msg_h + 11.0).max(56.0), cw: 0.0, ch: 0.0, block: false, title_h, hint_h, msg_h, label_w: inner_w, top: 11.0 }
    }

    pub(super) fn pump_custom_fields(&mut self, _cv: &Canvas) {}
    pub(super) fn step_custom(&mut self, _dt: f32) -> bool {
        false
    }
    pub(super) fn custom_key(&mut self, _n: &'static str, _ev: &glasskit::input::InputEvent, _activate: bool, _cx: &mut Cx) -> glasskit::input::Response {
        glasskit::input::Response::NONE
    }
    pub(super) fn custom_act(&mut self, _a: Act, _cx: &mut Cx) {}
    pub(super) fn custom_reply(&mut self, _p: &Pend, _res: &Result<Value, String>, _cx: &mut Cx) {}
    pub(super) fn job_event(&mut self, _v: &Value, _cx: &mut Cx) {}
    pub(super) fn card_rect_by_title(&self, _t: &str) -> Option<Rect> {
        None
    }

    pub(super) fn custom_h(&mut self, d: &Dc, r: &RowRt, name: &str, w: f32) -> f32 {
        let _ = name;
        self.plain_h(d, r, w)
    }

    pub(super) fn custom_paint(&mut self, d: &Dc, r: &mut RowRt, name: &str, full: Rect) {
        let _ = name;
        let m = self.row_metrics_plain(d, r, full.w - 2.0 * PAD_X);
        self.paint_label(d, r, &m, full.x + PAD_X, full.y);
    }
}

impl SettingsView {
    pub(super) fn on_picked(&mut self, r: Result<String, String>, cx: &mut Cx) {
        use pill_ui_core::settings::flows;
        let lang = self.lang();
        match r {
            Ok(text) => match flows::import_call(lang, text.len() as u64, &text) {
                Ok(call) => {
                    self.call(cx, call, "general.backup");
                }
                Err(m) => self.row_message("general.backup", m),
            },
            Err(e) if e == "cancelled" => {}
            Err(e) if e == "big" => self.row_message("general.backup", flows::import_call(lang, u64::MAX, "").err().unwrap_or_else(|| flows::read_error(lang))),
            Err(_) => self.row_message("general.backup", flows::read_error(lang)),
        }
    }
}
