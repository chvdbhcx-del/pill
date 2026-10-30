//! Custom widgets, part 3: the AI setup cards (local Whisper, local chat servers and Ollama) and the progress box of a running job.
use super::page::RowRt;
use super::rows::*;
use super::ui::Act;
use super::SettingsView;
use glasskit::controls::{ButtonKind, Widget};
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::aisetup::{self as ais, OllamaStep, WhisperAction};
use pill_ui_core::settings::flows;
use pill_ui_core::settings::tr::pick;
use serde_json::Value;

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}
fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

impl SettingsView {
    /// The progress box of the running job: text, Cancel, bar.
    pub(super) fn job_box(&mut self, d: &Dc, x: f32, y: f32, w: f32) {
        let (cv, p) = (d.cv, d.p);
        let Some(q) = self.data.progress.clone() else { return };
        let view = if q.id.starts_with("everything-") { flows::everything_progress(d.lang, &q) } else { flows::aisetup_progress(d.lang, &q) };
        let cancel = self.btn_at(d, true, "job:cancel", flows::CANCEL.get(d.lang), ButtonKind::Danger, 0.0, Some(x + w), y, Act::JobCancel, true);
        super::txt::line(cv, Rect::new(x, y, cancel.x - 8.0 - x, 32.0), &view.text, Style::new(12.0, 400), p.text2);
        self.bar(d, Rect::new(x, y + 38.0, w, 6.0), view.percent, view.indeterminate);
    }

    fn job_h(&self, id: &str) -> f32 {
        if self.data.job.as_deref() == Some(id) { 12.0 + 44.0 } else { 0.0 }
    }

    pub(super) fn w_whisper(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let mut y = full.y + m.nh - 11.0 + 6.0;
        let wj = self.data.status.get("whisper").cloned().unwrap_or(Value::Null);
        if wj.is_null() {
            return m.nh;
        }
        let (state, ok) = ais::whisper_state(d.lang, &wj);
        y += self.para_at(d, paint, x0, y, w, &state, st_hint(), if ok { p.green } else { p.dim }) + 10.0;
        let sel = ais::selected_model(&wj, &self.data.sel_model).to_string();
        let mut x = x0;
        for mo in arr(&wj, "models") {
            let id = s(mo, "id");
            let name = ais::model_name(id);
            let cw = self.chip(d, false, 0.0, 0.0, &format!("wm:{id}"), name, &[], false, Act::None);
            self.chip(d, paint, x, y, &format!("wm:{id}"), name, &[], id == sel, Act::WhisperModel(id.to_string()));
            x += cw + 8.0;
        }
        y += 30.0 + 8.0;
        y += self.para_at(d, paint, x0, y, w, ais::model_note(&sel).get(d.lang), st_hint(), p.dim) + 8.0;
        let plan = ais::whisper_plan(d.lang, &wj, &sel);
        for n in &plan.need {
            let line = format!("{} · {} · {}", n.what, n.host, flows::size(d.lang, n.size));
            y += self.para_at(d, paint, x0 + 8.0, y, w - 8.0, &line, st_hint(), p.dim);
        }
        if !plan.need.is_empty() {
            y += 8.0;
        }
        let busy = self.data.job.is_some();
        match &plan.action {
            WhisperAction::Download { label, .. } => {
                self.btn_at(d, paint, "wh:primary", label, ButtonKind::Primary, x0, None, y, Act::WhisperPrimary, !busy);
                y += 32.0 + 8.0;
            }
            WhisperAction::Enable { label, .. } => {
                self.btn_at(d, paint, "wh:primary", label, ButtonKind::Primary, x0, None, y, Act::WhisperPrimary, !busy);
                y += 32.0 + 8.0;
            }
            WhisperAction::Running => y += self.para_at(d, paint, x0, y, w, ais::WHISPER_ALL_GOOD.get(d.lang), st_hint(), p.green) + 8.0,
        }
        if paint && self.job_h("whisper") > 0.0 {
            self.job_box(d, x0, y + 4.0, w);
        }
        y += self.job_h("whisper");
        if let Some((id, msg)) = self.data.job_result.clone() {
            if id == "whisper" && self.data.job.is_none() {
                y += self.para_at(d, paint, x0, y, w, &msg.text, st_hint(), msg_color(&p, msg.kind)) + 8.0;
            }
        }
        if ais::whisper_switch_visible(&wj) {
            let on = wj.get("enabled").and_then(Value::as_bool).unwrap_or(false);
            let label = pick(d.lang, "Голос в текст через Whisper", "Voice to text through Whisper");
            if paint {
                cv.text(Rect::new(x0, y, w - 60.0, 24.0), label, Style::new(13.0, 400), p.text);
                self.data.wtoggle.set_enabled(true);
                let tr = Rect::new(full.right() - PAD_X - 44.0, y, 44.0, 24.0);
                let tt = glasskit::theme::Theme { thumb: glasskit::color::Color::rgb8(255, 255, 255), ..*d.th };
                self.data.wtoggle.draw(cv, &tt, tr);
                self.ui.add(super::ui::key("wh:switch"), tr, Act::WhisperSwitch, true, false, true);
            }
            let _ = on;
            y += 24.0 + 8.0;
        }
        y += self.para_at(d, paint, x0, y, w, &ais::whisper_folder_note(d.lang, s(&self.data.status, "dir")), st_hint(), p.faint) + 6.0;
        if flows_installed(&wj) {
            let label = if self.data.del.is_armed(self.now_ms as u64) { flows::REALLY_DELETE.get(d.lang) } else { flows::DELETE_FROM_DISK.get(d.lang) };
            self.btn_at(d, paint, "wh:del", label, ButtonKind::Danger, x0, None, y, Act::WhisperDelete, !busy);
            y += 32.0;
        }
        y + 12.0 - full.y
    }
}

fn flows_installed(w: &Value) -> bool {
    w.get("engine").and_then(|e| e.get("installed")).and_then(Value::as_bool).unwrap_or(false) || arr(w, "models").iter().any(|m| m.get("installed").and_then(Value::as_bool).unwrap_or(false))
}
