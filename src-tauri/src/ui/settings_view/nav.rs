//! Building pages from the schema, switching between them (exit slide, enter rise), keeping the controls in step with the model.
use super::page::*;
use super::SettingsView;
use crate::ui::view::{Cx, WindowCmd};
use glasskit::ease::{curves, Ease, Tween};
use pill_ui_core::settings::anim::AnimKind;
use pill_ui_core::settings::calls::BackendCall;
use pill_ui_core::settings::nav::OpenTarget;
use pill_ui_core::settings::schema::{schema, Control};
use serde_json::Value;

/// `cubic-bezier(.2, .8, .2, 1)`: the page rise.
pub const RISE: Ease = Ease::bezier(0.2, 0.8, 0.2, 1.0);

impl SettingsView {
    pub(super) fn fid(&mut self) -> u32 {
        self.fid_next += 1;
        self.fid_next
    }

    /// The first answer of `settings_get`: the model, the page asked for, the first frame.
    pub(super) fn begin(&mut self, merged: serde_json::Map<String, Value>, cx: &mut Cx) {
        let want = self.data.open_page.take();
        let target = self.model.begin(merged, want.as_deref());
        self.loaded = true;
        let rm = self.model.get_bool("reduceMotion");
        self.apply_reduce(rm, cx);
        match target {
            OpenTarget::Wizard => self.start_wizard(cx),
            OpenTarget::Page(p) => {
                self.side.snap_to(p);
                let pg = self.build_page(p);
                self.page = pg;
                self.enter_t = Tween::at_rest(1.0);
                self.page_shown(p, cx);
            }
        }
        cx.request_frame();
    }

    pub(super) fn apply_reduce(&mut self, on: bool, cx: &Cx) {
        if cx.window.0 != 0 {
            crate::ui::host::set_reduce_motion_setting(on);
        }
    }

    /// Fresh page `id` (a schema page id) with one control per row.
    pub(super) fn build_page(&mut self, id: &str) -> Page {
        let sc = schema();
        let lang = self.lang();
        let mut pg = Page::empty();
        pg.id = id.to_string();
        let Some(p) = sc.page(id) else { return pg };
        pg.blocks.push(Block::H1(p.heading().get(lang).to_string()));
        for g in &p.groups {
            let picker_only = g.rows.len() == 1 && matches!(g.rows[0].control, Control::PetsPicker);
            if !g.title.is_blank() && !picker_only {
                pg.blocks.push(Block::Cap(g.title.get(lang).to_string()));
            }
            let mut rows: Vec<RowRt> = Vec::new();
            for r in &g.rows {
                match r.control {
                    Control::PetsPicker | Control::CharacterPicker => {
                        if !rows.is_empty() {
                            pg.blocks.push(Block::Card(std::mem::take(&mut rows)));
                        }
                        let f = self.fid();
                        pg.fids.push(f);
                        pg.blocks.push(if matches!(r.control, Control::PetsPicker) { Block::Pets(f) } else { Block::Chars(f) });
                    }
                    _ => {
                        let f = self.fid();
                        let rt = new_row(r, &self.model, &self.th, &self.dy, lang, f);
                        if focusable(&rt.ctl) && !matches!(rt.ctl, Ctl::Custom("aisetup.intro")) {
                            pg.fids.push(f);
                        }
                        rows.push(rt);
                    }
                }
            }
            if !rows.is_empty() {
                pg.blocks.push(Block::Card(rows));
            }
            if !g.footer.is_blank() {
                pg.blocks.push(Block::Foot(g.footer.get(lang).to_string()));
            }
        }
        if !p.footnote.is_blank() {
            pg.blocks.push(Block::Foot(p.footnote.get(lang).to_string()));
        }
        if id == "updates" && !self.dir.is_empty() {
            pg.blocks.push(Block::Foot(pill_ui_core::settings::flows::data_dir_note(lang, &self.dir)));
        }
        pg
    }

    /// Makes `id` the page on screen, now (no exit) or after the old one slid out.
    pub(super) fn show_page(&mut self, id: &str, cx: &mut Cx) {
        if !self.loaded {
            self.data.open_page = Some(id.to_string());
            return;
        }
        let has_old = !self.page.id.is_empty() && self.leaving.is_none();
        let plan = self.model.show(id, has_old);
        self.search_close_silently();
        self.side.move_to(plan.page, self.reduce());
        if self.wiz.active() {
            self.wiz.end_chrome();
        }
        if let (Some(exit), true) = (plan.exit, has_old && !self.page.wizard) {
            self.leaving = Some(std::mem::replace(&mut self.page, Page::empty()));
            self.leave_t = Tween::new(0.0, 1.0, exit.dur_ms as f32 / 1000.0, curves::EASE_IN);
            self.want_page = Some(plan.page.to_string());
        } else if self.leaving.is_some() {
            self.want_page = Some(plan.page.to_string());
        } else {
            self.install_page(plan.page, cx);
        }
        cx.request_frame();
    }

    pub(super) fn install_page(&mut self, id: &str, cx: &mut Cx) {
        let id: &'static str = self.model.current;
        let pg = self.build_page(id);
        self.leaving = None;
        self.want_page = None;
        self.page = pg;
        let spec = self.model.anim(AnimKind::PageEnter);
        self.enter_t = Tween::new(0.0, 1.0, spec.dur_ms as f32 / 1000.0, RISE);
        self.ui.reset();
        self.grab = None;
        self.open_select = None;
        self.focus.set_order(&self.focus_order());
        self.page_shown(id, cx);
    }

    /// Everything that is fetched when a page shows up.
    pub(super) fn page_shown(&mut self, id: &str, cx: &mut Cx) {
        self.stop_ticker();
        let mut calls: Vec<BackendCall> = Vec::new();
        match id {
            "general" => calls.push(BackendCall::AutostartGet),
            "hotkeys" => calls.push(BackendCall::LauncherList),
            "appearance" => calls.push(BackendCall::PillMonitors),
            "sound" => {
                calls.push(BackendCall::MediaDevices);
                calls.push(BackendCall::MediaCaptureDevices);
            }
            "clipboard" => calls.push(BackendCall::ClipStatus),
            "search" => calls.push(BackendCall::EverythingStatus),
            "ai" => {
                calls.push(BackendCall::AiConfig);
                calls.push(BackendCall::NetFeaturesStatus);
            }
            "voice" => {
                calls.push(BackendCall::VoiceHasKey { provider: Some(self.model.get_str("voice.provider")) });
                calls.push(BackendCall::NetFeaturesStatus);
            }
            "aisetup" => {
                calls.push(BackendCall::AisetupStatus);
                calls.push(BackendCall::AiConfig);
            }
            "updates" => calls.push(BackendCall::SettingsInfo),
            "live" => {
                calls.extend([BackendCall::ObsStatus, BackendCall::SteamStatus, BackendCall::NotifStatus, BackendCall::LiveplusIcsStatus, BackendCall::MailStatus]);
                self.ensure_ticker(cx, 1000);
            }
            "weather" => {}
            _ => {}
        }
        self.run(cx, calls);
    }

    pub(super) fn focus_order(&self) -> Vec<u32> {
        let mut v = vec![chrome::FID_SEARCH];
        v.extend(self.side.fids());
        v.extend(self.page.fids.iter().copied());
        v
    }
}

use super::chrome;

impl SettingsView {
    /// The `Ui*` effects of the model: things the old page did inline.
    pub(super) fn ui_effect(&mut self, cx: &mut Cx, c: BackendCall) {
        match c {
            BackendCall::UiSetLanguage { .. } | BackendCall::UiRebuildSidebar => {}
            BackendCall::UiRerender => self.rebuild(cx),
            BackendCall::UiShowPage { page } => self.show_page(&page, cx),
            BackendCall::UiStartWizard => self.start_wizard(cx),
            BackendCall::UiPickFile => self.pick_file(cx),
            BackendCall::UiSyncControls => self.sync_page(),
            BackendCall::UiRedrawWizard => self.rebuild_wizard(),
            BackendCall::UiEndWizardChrome => self.wiz.end_chrome(),
            BackendCall::UiSetReduceMotion { on } => self.apply_reduce(on, cx),
            BackendCall::UiMinimize => cx.window_cmd(WindowCmd::Minimize),
            BackendCall::UiToggleMaximize => cx.window_cmd(WindowCmd::ToggleMaximize),
            _ => {}
        }
        cx.request_frame();
    }

    /// Language change: the sidebar and the page text are rebuilt in place (scroll position kept).
    pub(super) fn rebuild(&mut self, cx: &mut Cx) {
        if self.wiz.active() {
            self.rebuild_wizard();
            return;
        }
        let off = self.page.scroll.offset();
        let id = self.model.current;
        let pg = self.build_page(id);
        self.page = pg;
        self.enter_t = Tween::at_rest(1.0);
        self.page.scroll.scroll_to(off, false);
        self.focus.set_order(&self.focus_order());
        self.search.refresh_lang(self.lang());
        self.page_shown(id, cx);
    }

    /// Rewrites every control from the model. Visibility and enabled changes animate (row expand / collapse, the opacity fade).
    pub(super) fn sync_page(&mut self) {
        let reduce = self.reduce();
        let ex = self.model.anim(AnimKind::RowExpand).dur_ms as f32 / 1000.0;
        let co = self.model.anim(AnimKind::RowCollapse).dur_ms as f32 / 1000.0;
        let model = &self.model;
        let lang = model.lang();
        let dy = &self.dy;
        for r in self.page.rows_mut().chain(std::iter::empty()) {
            sync_row(r, model, dy, lang, reduce, ex, co);
        }
    }
}

pub(super) fn sync_row(r: &mut RowRt, model: &pill_ui_core::settings::SettingsModel, dy: &Dyn, lang: pill_ui_core::settings::tr::Lang, _reduce: bool, ex: f32, co: f32) {
    let st = model.row_state(r.row);
    if st.visible != r.vis_on {
        r.vis_on = st.visible;
        r.vis.retarget(st.visible as u8 as f32, if st.visible { ex } else { co }, curves::EASE_OUT);
    }
    if st.enabled != r.en_on {
        r.en_on = st.enabled;
        r.en.retarget(if st.enabled { 1.0 } else { 0.5 }, 0.22, curves::EASE_OUT);
    }
    let key = r.row.key;
    let rect = r.ctl_rect;
    match &mut r.ctl {
        Ctl::Toggle(t) => {
            if t.on() != model.get_bool(key) {
                t.set_on(model.get_bool(key), true);
            }
        }
        Ctl::Backend(t, _) => {
            if t.on() != model.autostart() {
                t.set_on(model.autostart(), true);
            }
        }
        Ctl::Slider(s) => {
            let v = model.get(key).as_f64().unwrap_or(s.min as f64) as f32;
            if (s.value() - v).abs() > f32::EPSILON && !s.dragging() {
                s.set_value(v, true);
            }
        }
        Ctl::Seg(s) => {
            r.opts = options_of(r.row, dy, lang);
            if let Some(i) = selected_index(&r.opts, &model.get(key)) {
                s.select(i, rect, true);
            }
        }
        Ctl::Select(s) => {
            r.opts = options_of(r.row, dy, lang);
            s.set_selected(selected_index(&r.opts, &model.get(key)));
        }
        Ctl::Field(f) => {
            if !f.tf.focused() && !f.tf.dirty && f.kind != FK::Secret {
                f.tf.set_text(&field_text(r.row, model));
            }
        }
        _ => {}
    }
}
