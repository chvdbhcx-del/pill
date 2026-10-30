//! What a click does, how a changed control reaches the model, and what the backend's answers change on screen.
use super::events::Change;
use super::page::*;
use super::rows::new_msg_tween;
use super::ui::Act;
use super::{Local, Pend, SettingsView};
use crate::ui::view::{Cx, Msg, WindowCmd};
use pill_ui_core::settings::calls::{self, BackendCall};
use pill_ui_core::settings::flows::{self, Msg as FMsg};
use pill_ui_core::settings::schema::{Action, Commit, Control};
use pill_ui_core::settings::search::SearchStep;
use pill_ui_core::settings::tr::pick;
use serde_json::{json, Value};

impl SettingsView {
    pub(super) fn act(&mut self, a: Act, cx: &mut Cx) {
        cx.request_frame();
        match a {
            Act::None => {}
            Act::CapMin => cx.window_cmd(WindowCmd::Minimize),
            Act::CapMax => cx.window_cmd(WindowCmd::ToggleMaximize),
            Act::CapClose => cx.close_self(),
            Act::Nav(p) => self.show_page(&p, cx),
            Act::Quit => {
                self.call(cx, BackendCall::SettingsQuit, "");
            }
            Act::SearchPick(i) => {
                let step = self.model.search_pick(i);
                self.apply_search_step(step, cx);
            }
            Act::RowButton(id) => self.row_button(&id, cx),
            Act::SecretSave(id) => self.secret_save(&id, cx),
            Act::SecretRemove(id) => self.secret_remove(&id, cx),
            Act::Day(d) => {
                let days = self.model.toggle_day(&d);
                self.pop_day(&d);
                let calls = self.model.set("quiet.days", json!(days));
                self.run(cx, calls);
                self.sync_page();
            }
            Act::WizSkip => {
                let calls = self.model.wizard.skip();
                self.run(cx, calls);
            }
            Act::WizBack => {
                self.model.wizard.back();
                self.rebuild_wizard();
            }
            Act::WizNext => match self.model.wizard.advance() {
                Some(calls) => self.run(cx, calls),
                None => self.rebuild_wizard(),
            },
            Act::WizOpen(p) => {
                let calls = self.model.wizard.finish_to(p);
                self.run(cx, calls);
            }
            Act::Recorder(action) => {
                if self.model.recorder.start(&action).is_some() || self.model.recorder.is_recording(&action) {
                    self.rec_live.clear();
                    self.rec_changed();
                    self.row_message(&action, FMsg::plain(""));
                }
            }
            other => self.custom_act(other, cx),
        }
    }

    fn pop_day(&mut self, id: &str) {
        let idx = pill_ui_core::settings::model::WEEKDAYS.iter().position(|d| d.0 == id);
        let on = self.model.quiet_days().iter().any(|d| d == id);
        let reduce = self.reduce();
        if let (Some(i), true) = (idx, on) {
            for r in self.page.rows_mut() {
                if let Ctl::Week(w) = &mut r.ctl {
                    w.cur = i;
                    w.pop[i] = if reduce { glasskit::ease::Tween::at_rest(1.0) } else { glasskit::ease::Tween::new(0.88, 1.0, 0.4, glasskit::ease::curves::SPRING_POP) };
                }
            }
        }
    }

    pub(super) fn apply_change(&mut self, c: Change, cx: &mut Cx) {
        if let Some(b) = c.backend {
            let calls = self.model.set_backend_toggle(b, c.value.as_bool().unwrap_or(false));
            for call in calls {
                self.call(cx, call, c.row.id);
            }
            return;
        }
        match self.model.try_set(c.row.key, c.value) {
            Ok(calls) => self.run(cx, calls),
            Err(_) => {}
        }
        self.sync_page();
        cx.request_frame();
    }

    /// Puts a message under a row's hint (rows are found by id, or by the hotkey action they edit).
    pub(super) fn row_message(&mut self, id_or_action: &str, m: FMsg) {
        let reduce = self.reduce();
        for r in self.page.rows_mut() {
            let hit = r.row.id == id_or_action || matches!(&r.row.control, Control::Hotkey { action } if *action == id_or_action);
            if hit {
                r.msg = if m.text.is_empty() { None } else { Some((m.clone(), if reduce { glasskit::ease::Tween::at_rest(1.0) } else { new_msg_tween() })) };
            }
        }
    }

    pub(super) fn apply_search_step(&mut self, step: SearchStep, cx: &mut Cx) {
        match step {
            SearchStep::None => {}
            SearchStep::Paint { fresh } => self.refresh_search_rows(fresh),
            SearchStep::Close { clear } => {
                self.search.rows.clear();
                if clear {
                    self.search.field.set_text("");
                }
            }
            SearchStep::Blur => self.set_focus_id(None, false),
            SearchStep::Go(t) => {
                self.search.rows.clear();
                self.search.field.set_text("");
                if t.page != self.model.current || self.wiz.active() {
                    self.show_page(&t.page, cx);
                }
                if t.row {
                    self.goto = Some(super::Goto { row_id: t.row_id, title: t.title });
                }
            }
        }
    }

    pub(super) fn search_key(&mut self, ev: &glasskit::input::InputEvent, cx: &mut Cx) -> glasskit::input::Response {
        use glasskit::input::{InputEvent, KeyCode};
        use pill_ui_core::settings::search::SearchKey;
        let rc = glasskit::input::Response { consumed: true, redraw: true, ..glasskit::input::Response::NONE };
        if let InputEvent::Key { code, down: true, .. } = ev {
            let k = match code {
                KeyCode::Escape => Some(SearchKey::Escape),
                KeyCode::Down if self.model.search.open => Some(SearchKey::Down),
                KeyCode::Up if self.model.search.open => Some(SearchKey::Up),
                KeyCode::Enter if self.model.search.open => Some(SearchKey::Enter),
                _ => None,
            };
            if let Some(k) = k {
                let step = self.model.search_key(k);
                self.apply_search_step(step, cx);
                return rc;
            }
        }
        let r = self.search.rect;
        self.search.field.queue(ev, r);
        self.edits_due = true;
        rc
    }
}

impl SettingsView {
    fn row_def(&self, id: &str) -> Option<&'static pill_ui_core::settings::schema::Row> {
        pill_ui_core::settings::schema::schema().row(id).or_else(|| self.page.rows().find(|r| r.row.id == id).map(|r| r.row))
    }

    pub(super) fn row_button(&mut self, id: &str, cx: &mut Cx) {
        match id {
            "wizard.hotkeys" => return self.act(Act::WizOpen("hotkeys"), cx),
            "wizard.ai" => return self.act(Act::WizOpen("aisetup"), cx),
            _ => {}
        }
        let Some(Control::Button { action, .. }) = self.row_def(id).map(|r| r.control.clone()) else { return };
        let lang = self.lang();
        match action {
            Action::UpdateCheck => {
                if let Some((call, msg)) = self.model.update.click(lang) {
                    self.row_message(id, msg);
                    self.call(cx, call, id);
                }
            }
            Action::ClipWipe => {
                if self.data.wipe.click(self.now_ms as u64) {
                    self.call(cx, BackendCall::ClipWipe, id);
                }
            }
            Action::AiSetupLater => {
                let calls = pill_ui_core::settings::aisetup::later_calls(self.data.ai_done, self.data.job.is_some());
                self.data.ai_done = !self.data.ai_done;
                self.run(cx, calls);
            }
            Action::PlaySignal => {
                let tone = self.model.get_str("sound.reminderTone");
                self.call(cx, BackendCall::SoundPlay { tone }, id);
            }
            Action::RunWizard => self.start_wizard(cx),
            a => {
                if let Some(c) = calls::action_call(a) {
                    self.call(cx, c, id);
                }
            }
        }
    }

    pub(super) fn button_label(&self, lang: pill_ui_core::settings::tr::Lang, rid: &str, default: &str) -> String {
        match rid {
            "updates.check" => self.model.update.button_label(lang).to_string(),
            "clipboard.wipe" if self.data.wipe.is_armed(self.now_ms as u64) => flows::WIPE_YES.get(lang).to_string(),
            "aisetup.later" => pill_ui_core::settings::aisetup::later_button(lang, self.data.ai_done).to_string(),
            _ => default.to_string(),
        }
    }

    pub(super) fn busy_button(&self, rid: &str) -> bool {
        rid == "updates.check" && self.model.update.is_busy()
    }

    pub(super) fn secret_removable(&self, rid: &str) -> bool {
        rid == "ai.key" || rid == "voice.key"
    }

    pub(super) fn secret_placeholder(&self, lang: pill_ui_core::settings::tr::Lang, rid: &str, has: bool) -> String {
        match rid {
            "ai.key" => flows::ai_key_placeholder(lang, has).to_string(),
            "voice.key" => flows::voice_key_placeholder(lang, &self.data.voice_source).to_string(),
            "mail.password" => flows::mail_password_placeholder(lang, has).to_string(),
            _ => String::new(),
        }
    }

    fn field_text_of(&mut self, id: &str) -> String {
        let mut t = String::new();
        for r in self.page.rows_mut() {
            if r.row.id == id {
                if let Ctl::Field(f) = &mut r.ctl {
                    t = f.tf.text();
                    f.tf.set_text("");
                }
            }
        }
        t
    }

    fn secret_save(&mut self, id: &str, cx: &mut Cx) {
        let v = self.field_text_of(id);
        let v = v.trim().to_string();
        if v.is_empty() {
            return;
        }
        let Some(Control::Text { commit: Commit::Backend(a), .. }) = self.row_def(id).map(|r| r.control.clone()) else { return };
        let call = match a {
            Action::AiSetKey => BackendCall::AiSetKey { key: v },
            Action::VoiceSetKey => BackendCall::VoiceSetKey { provider: self.model.get_str("voice.provider"), key: v },
            Action::ObsSetPassword => BackendCall::ObsSetPassword { password: v },
            Action::MailSetPassword => BackendCall::MailSetPassword { password: v },
            _ => return,
        };
        self.call(cx, call, id);
    }

    fn secret_remove(&mut self, id: &str, cx: &mut Cx) {
        let call = match id {
            "ai.key" => BackendCall::AiSetKey { key: String::new() },
            _ => BackendCall::VoiceSetKey { provider: self.model.get_str("voice.provider"), key: String::new() },
        };
        self.call(cx, call, id);
    }
}
