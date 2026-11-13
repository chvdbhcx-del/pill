//! Building pages from the schema, switching between them (exit slide, enter rise), keeping the controls in step with the model.
use super::page::*;
use super::SettingsView;
use crate::ui::view::{Cx, WindowCmd};
use super::flow::{from_to, go};
use pill_ui_core::settings::anim::AnimKind;
use pill_ui_core::settings::calls::BackendCall;
use pill_ui_core::settings::nav::OpenTarget;
use crate::ui::liquid::{Feel, Flow};
use pill_ui_core::settings::schema::{self, schema, Control};
use serde_json::Value;

impl SettingsView {
    pub(super) fn fid(&mut self) -> u32 {
        self.fid_next += 1;
        self.fid_next
    }

    /// The first answer of `settings_get`: the model, the page asked for, the first frame.
    pub(super) fn begin(&mut self, merged: serde_json::Map<String, Value>, cx: &mut Cx) {
        let want = self.data.open_page.take();
        let anchor = want.as_deref().and_then(pill_ui_core::settings::nav::legacy_anchor);
        let target = self.model.begin(merged, want.as_deref());
        self.loaded = true;
        self.search.refresh_lang(self.lang());
        let rm =self.model.get_bool("reduceMotion");
        self.apply_reduce(rm, cx);
        match target {
            OpenTarget::Wizard => self.start_wizard(cx),
            OpenTarget::Page(p) => {
                self.side.snap_to(p);
                let pg = self.build_page(p);
                self.page = pg;
                self.enter_t.jump(1.0);
                self.focus.set_order(&self.focus_order());
                self.page_shown(p, cx);
                self.goto_anchor(anchor);
            }
        }
        cx.request_frame();
    }

    /// Opens or closes the "Advanced" section of the page on screen (animated; at once with `instant` or Reduce motion).
    /// Remembered until the window closes, never stored.
    pub(super) fn set_advanced(&mut self, on: bool, instant: bool) {
        let reduce = self.reduce() || instant;
        let id = self.model.current;
        let header = {
            let Some(a) = self.page.adv.as_mut() else { return };
            if a.on == on {
                return;
            }
            a.on = on;
            go(&mut a.t, on as u8 as f32, reduce);
            a.fid
        };
        if on {
            self.adv_open.insert(id);
        } else {
            self.adv_open.remove(id);
        }
        self.page.laid_out = false;
        // a row that folds away cannot keep the focus: it goes to the header
        let lost = !on && self.focus.focused().is_some_and(|f| self.page.rows().any(|r| r.adv && r.fid == f));
        self.focus.set_order(&self.focus_order());
        if lost {
            self.set_focus_id(Some(header), false);
        }
    }

    /// An old page name was asked for: scroll to where its rows stand now (no flash; a collapsed section opens for it).
    pub(super) fn goto_anchor(&mut self, anchor: Option<&'static str>) {
        if let Some(row_id) = anchor {
            self.goto = Some(super::Goto { row_id: row_id.to_string(), title: String::new(), quiet: true });
        }
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
        // the trailing `advanced` groups sit behind one disclosure: closed unless it was opened earlier in this window
        let open = self.adv_open.contains(id);
        let mut adv_at = None;
        for g in &p.groups {
            if g.advanced && adv_at.is_none() {
                let titles: Vec<&str> = p.advanced_groups().map(|g| g.title.get(lang)).filter(|t| !t.is_empty()).collect();
                let sub = if titles.is_empty() { schema::ADVANCED_HINT.get(lang).to_string() } else { titles.join(" · ") };
                let f = self.fid();
                pg.fids.push(f);
                pg.adv = Some(Adv { t: Flow::new(open as u8 as f32, Feel::FLOW), on: open, sub, fid: f });
                adv_at = Some(pg.blocks.len());
                pg.blocks.push(Block::Adv(0));
            }
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
                        let mut rt = new_row(r, &self.model, &self.th, &self.dy, lang, f);
                        rt.adv = g.advanced;
                        rt.ak = if g.advanced { open as u8 as f32 } else { 1.0 };
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
        if let Some(i) = adv_at {
            pg.blocks[i] = Block::Adv(pg.blocks.len() - i - 1);
        }
        if !p.footnote.is_blank() {
            pg.blocks.push(Block::Foot(p.footnote.get(lang).to_string()));
        }
        if id == "general" && !self.dir.is_empty() {
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
        let anchor = pill_ui_core::settings::nav::legacy_anchor(id);
        let plan = self.model.show(id, has_old);
        self.search_close_silently();
        self.side.move_to(plan.page, self.reduce());
        if self.wiz.active() {
            self.wiz.end_chrome();
        }
        if let (Some(exit), true) = (plan.exit.filter(|e| !e.is_instant()), has_old && !self.page.wizard && !self.reduce()) {
            self.leaving = Some(std::mem::replace(&mut self.page, Page::empty()));
            // a page that was still rising in turns around where it is: the leave starts at what is left of it, with its momentum
            self.leave_t.jump(1.0 - self.enter_t.at01());
            self.leave_t.nudge(-self.enter_t.velocity());
            self.leave_t.to(1.0);
            self.want_page = Some(plan.page.to_string());
        } else if self.leaving.is_some() {
            self.want_page = Some(plan.page.to_string());
        } else {
            self.install_page(plan.page, cx);
        }
        self.goto_anchor(anchor);
        cx.request_frame();
    }

    pub(super) fn install_page(&mut self, id: &str, cx: &mut Cx) {
        let id: &'static str = self.model.current;
        let pg = self.build_page(id);
        self.leaving = None;
        self.want_page = None;
        self.page = pg;
        let reduce = self.reduce() || self.model.anim(AnimKind::PageEnter).is_instant();
        from_to(&mut self.enter_t, 0.0, 1.0, reduce);
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
            // General: the autostart state, the data folder (settings backup) and the file search (Everything) status
            "general" => calls.extend([BackendCall::AutostartGet, BackendCall::SettingsInfo, BackendCall::EverythingStatus]),
            "hotkeys" => calls.push(BackendCall::LauncherList),
            "look" => calls.push(BackendCall::PillMonitors),
            // the output and input devices
            "volume" => {
                calls.push(BackendCall::MediaDevices);
                calls.push(BackendCall::MediaCaptureDevices);
            }
            // Voice to text: everything the old Voice page asked for, and the local Whisper install
            "voice" => {
                calls.push(BackendCall::VoiceHasKey { provider: Some(self.model.get_str("voice.provider")) });
                calls.push(BackendCall::NetFeaturesStatus);
                calls.push(BackendCall::AisetupStatus);
            }
            "clipboard" => calls.push(BackendCall::ClipStatus),
            "ai" => {
                calls.push(BackendCall::AiConfig);
                calls.push(BackendCall::NetFeaturesStatus);
                calls.push(BackendCall::AisetupStatus);
            }
            // the local AI setup (the old AI setup page)
            "localai" => {
                calls.push(BackendCall::NetFeaturesStatus);
                calls.push(BackendCall::AisetupStatus);
            }
            "updates" => calls.push(BackendCall::SettingsInfo),
            // the pages with a status line poll it while they are on screen
            id if !live_status(id).is_empty() => {
                calls.extend(live_status(id));
                self.ensure_ticker(cx, 1000);
            }
            "phone" => {
                calls.extend([BackendCall::PhoneStatus, BackendCall::PhoneApps]);
                if self.model.get_bool("phone.enabled") {
                    calls.push(BackendCall::PhonePairCode { renew: false });
                }
                self.ensure_ticker(cx, 2000);
            }
            _ => {}
        }
        self.run(cx, calls);
    }

    pub(super) fn focus_order(&self) -> Vec<u32> {
        let mut v = vec![chrome::FID_SEARCH];
        v.extend(self.side.fids());
        // Tab does not stop on rows folded away in a closed "Advanced" section
        let folded: Vec<u32> = if self.page.adv.as_ref().is_some_and(|a| !a.on) { self.page.rows().filter(|r| r.adv).map(|r| r.fid).collect() } else { Vec::new() };
        v.extend(self.page.fids.iter().copied().filter(|f| !folded.contains(f)));
        v
    }
}

use super::chrome;

/// The status calls of a page with a polled status line (Notifications, Downloads, OBS, Calendar, Mail); empty for every other page.
pub(super) fn live_status(page: &str) -> Vec<BackendCall> {
    match page {
        "notifications" => vec![BackendCall::NotifStatus],
        "downloads" => vec![BackendCall::SteamStatus],
        "obs" => vec![BackendCall::ObsStatus],
        "calendar" => vec![BackendCall::LiveplusIcsStatus],
        "mail" => vec![BackendCall::MailStatus],
        _ => Vec::new(),
    }
}

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
        // the text changed language under a still page: it settles in softly instead of swapping at once
        let reduce = self.reduce();
        from_to(&mut self.enter_t, 0.5, 1.0, reduce);
        self.page.scroll.scroll_to(off, false);
        self.focus.set_order(&self.focus_order());
        self.search.refresh_lang(self.lang());
        self.page_shown(id, cx);
    }

    /// Rewrites every control from the model. Visibility and enabled changes animate (row expand / collapse, the opacity fade).
    pub(super) fn sync_page(&mut self) {
        let reduce = self.reduce();
        let model = &self.model;
        let lang = model.lang();
        let dy = &self.dy;
        for r in self.page.rows_mut().chain(std::iter::empty()) {
            sync_row(r, model, dy, lang, reduce);
        }
    }
}

pub(super) fn sync_row(r: &mut RowRt, model: &pill_ui_core::settings::SettingsModel, dy: &Dyn, lang: pill_ui_core::settings::tr::Lang, reduce: bool) {
    let st = model.row_state(r.row);
    if st.visible != r.vis_on {
        r.vis_on = st.visible;
        go(&mut r.vis, st.visible as u8 as f32, reduce);
    }
    if st.enabled != r.en_on {
        r.en_on = st.enabled;
        go(&mut r.en, if st.enabled { 1.0 } else { 0.5 }, reduce);
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
