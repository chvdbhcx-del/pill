//! The 12 `Control::Custom` widgets and the state they share. A custom row paints its whole body itself (label included when it wants);
//! `custom_h` measures it, `custom_paint` draws it and registers its hits (the widgets are in `cw1`, `cw2`, `cw3`, `cw3b`).
use super::fields::TextField;
use super::page::*;
use super::rows::*;
use super::ui::Act;
use super::SettingsView;
use crate::ui::view::Cx;
use glasskit::canvas::Canvas;
use glasskit::controls::{Toggle, Widget};
use glasskit::ease::{curves, Tween};
use glasskit::geom::Rect;
use pill_ui_core::settings::flows::{Confirm, EverythingView, Msg as FMsg, Progress};
use pill_ui_core::settings::hotkey::LauncherShortcut;
use serde_json::Value;
use std::collections::HashMap;

/// A city of the weather search.
#[derive(Clone)]
pub struct City {
    pub label: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

pub struct Data {
    pub open_page: Option<String>,
    pub wipe: Confirm,
    pub del: Confirm,
    pub ai_done: bool,
    /// id of the running download job (whisper | ollama-setup | ollama-pull | everything-*)
    pub job: Option<String>,
    pub progress: Option<Progress>,
    pub job_result: Option<(String, FMsg)>,
    pub voice_source: String,
    pub ai_has_key: bool,
    pub launcher: Vec<LauncherShortcut>,
    pub clip_count: i64,
    pub ev: Option<EverythingView>,
    pub ev_poll: u32,
    pub status: Value,
    pub sel_model: String,
    pub wtoggle: Toggle,
    pub cities: Vec<City>,
    pub fields: HashMap<&'static str, TextField>,
    pub pops: Vec<(u64, f64)>,
    pub whatsnew: Tween<f32>,
    pub whatsnew_open: bool,
    pub tick: u64,
}

impl Default for Data {
    fn default() -> Self {
        Data {
            open_page: None,
            wipe: Confirm::wipe(),
            del: Confirm::delete_from_disk(),
            ai_done: false,
            job: None,
            progress: None,
            job_result: None,
            voice_source: String::new(),
            ai_has_key: false,
            launcher: Vec::new(),
            clip_count: 0,
            ev: None,
            ev_poll: 0,
            status: Value::Null,
            sel_model: "small".into(),
            wtoggle: Toggle::new(false).spring(0.45),
            cities: Vec::new(),
            fields: HashMap::new(),
            pops: Vec::new(),
            whatsnew: Tween::at_rest(0.0),
            whatsnew_open: false,
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
    pub(super) fn row_metrics_plain(&mut self, d: &Dc, r: &RowRt, inner_w: f32) -> RowM {
        use super::txt;
        let title = r.row.label.get(d.lang);
        let hint = r.row.hint.get(d.lang);
        let title_h = txt::wrapped_h(d.cv, title, st_title(), inner_w, 18.0).max(18.0);
        let hint_h = if hint.is_empty() { 0.0 } else { 2.0 + txt::wrapped_h(d.cv, hint, st_hint(), inner_w, 16.0) };
        let mut msg_h = 0.0;
        for m in [r.msg.as_ref().map(|x| &x.0), r.status.as_ref()].into_iter().flatten() {
            if !m.text.is_empty() {
                msg_h += 2.0 + txt::wrapped_h(d.cv, &m.text, st_hint(), inner_w, 16.0);
            }
        }
        RowM { nh: (11.0 + title_h + hint_h + msg_h + 11.0).max(56.0), cw: 0.0, ch: 0.0, block: false, title_h, hint_h, msg_h, label_w: inner_w, top: 11.0 }
    }

    pub(super) fn custom_h(&mut self, d: &Dc, r: &RowRt, name: &str, w: f32) -> f32 {
        let full = Rect::new(0.0, 0.0, w, 0.0);
        self.custom_run(d, r, name, full, false)
    }

    pub(super) fn custom_paint(&mut self, d: &Dc, r: &mut RowRt, name: &str, full: Rect) {
        if name == "weather.city" {
            self.w_weather(d, r, full, true);
        } else {
            self.custom_run(d, r, name, full, true);
        }
    }

    fn custom_run(&mut self, d: &Dc, r: &RowRt, name: &str, full: Rect, paint: bool) -> f32 {
        match name {
            "ai.providers" => self.w_providers(d, true, full, paint),
            "voice.providers" => self.w_providers(d, false, full, paint),
            "character.reactions" => self.w_reactions(d, full, paint),
            "general.backup" => self.w_backup(d, r, full, paint),
            "updates.whatsnew" => self.w_whatsnew(d, full, paint),
            "hotkeys.launcher" => self.w_launcher(d, r, full, paint),
            "modules.profiles" => self.w_profiles(d, r, full, paint),
            "search.everything" => self.w_everything(d, r, full, paint),
            "aisetup.whisper" => self.w_whisper(d, r, full, paint),
            "aisetup.chat" => self.w_chat(d, r, full, paint),
            "weather.city" => {
                let mut tmp = RowRt { msg: None, ..clone_shell(r) };
                self.w_weather(d, &mut tmp, full, false)
            }
            _ => {
                let m = self.row_metrics_plain(d, r, full.w - 2.0 * PAD_X);
                if paint {
                    self.paint_label(d, r, &m, full.x + PAD_X, full.y);
                }
                m.nh
            }
        }
    }

    pub(super) fn pump_custom_fields(&mut self, cv: &Canvas) {
        let mut any = false;
        for f in self.data.fields.values_mut() {
            f.pump(cv);
            any |= f.has_pending();
        }
        if any {
            self.edits_due = true;
        }
    }

    pub(super) fn step_custom(&mut self, dt: f32) -> bool {
        let mut busy = self.data.wtoggle.step(dt);
        busy |= self.data.whatsnew.step(dt);
        for f in self.data.fields.values_mut() {
            busy |= f.step(dt);
        }
        let now = self.now_ms;
        self.data.pops.retain(|p| now - p.1 < 450.0);
        busy || !self.data.pops.is_empty() || self.data.job.is_some()
    }

    pub(super) fn custom_key(&mut self, n: &'static str, ev: &glasskit::input::InputEvent, activate: bool, cx: &mut Cx) -> glasskit::input::Response {
        let rc = glasskit::input::Response { consumed: true, redraw: true, ..glasskit::input::Response::NONE };
        match n {
            "weather.city" | "modules.profiles" => {
                let name = if n == "weather.city" { "city" } else { "profile" };
                if let Some(f) = self.data.fields.get_mut(name) {
                    let r = f.rect;
                    f.queue(ev, r);
                    self.edits_due = true;
                    if matches!(ev, glasskit::input::InputEvent::Key { code: glasskit::input::KeyCode::Enter, down: true, .. }) {
                        self.act(if n == "weather.city" { Act::CitySearch } else { Act::ProfileSave }, cx);
                    }
                    return rc;
                }
                glasskit::input::Response::NONE
            }
            "updates.whatsnew" if activate => {
                self.act(Act::Disclosure, cx);
                rc
            }
            _ => glasskit::input::Response::NONE,
        }
    }
}

fn clone_shell(r: &RowRt) -> RowRt {
    RowRt { row: r.row, ctl: Ctl::None, vis: Tween::at_rest(1.0), vis_on: true, en: Tween::at_rest(1.0), en_on: true, msg: None, status: None, hl: None, rect: Rect::default(), ctl_rect: Rect::default(), fid: r.fid, opts: Vec::new() }
}
