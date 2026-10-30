//! Per-frame work: advancing every animation, collecting what the text fields did, resolving a pending "go to row", the caption region.
use super::page::*;
use super::SettingsView;
use crate::ui::view::Cx;
use glasskit::controls::Widget;
use glasskit::edit::EditEvent;
use pill_ui_core::settings::search::SearchStep;
use serde_json::json;

enum Edit {
    Commit(&'static str),
    Revert(&'static str),
    Save(&'static str),
    Search,
}

impl SettingsView {
    pub(super) fn step_all(&mut self, dt: f32, cx: &mut Cx) -> bool {
        let mut busy = false;
        busy |= self.win_t.step(dt);
        busy |= self.enter_t.step(dt);
        busy |= self.rec_t.step(dt);
        busy |= self.side.ind_y.step(dt);
        busy |= self.search.stag.step(dt);
        busy |= self.search.pop.step(dt);
        for d in self.wiz.dots.iter_mut() {
            busy |= d.step(dt);
        }
        if self.leaving.is_some() {
            busy |= self.leave_t.step(dt);
            if self.leave_t.finished() {
                if let Some(p) = self.want_page.take() {
                    let _ = p;
                    let id = self.model.current;
                    self.install_page(id, cx);
                } else {
                    self.leaving = None;
                }
            }
            busy = true;
        }
        busy |= self.side.scroll.step(dt);
        busy |= self.side.bar.step(dt);
        busy |= self.page.scroll.step(dt);
        busy |= self.page.bar.step(dt);
        busy |= self.ui.step(dt);
        busy |= self.step_rows(dt);
        busy |= self.search.field.step(dt);
        busy |= self.step_custom(dt);
        self.process_edits(cx);
        self.resolve_goto();
        if self.caption_dirty && self.loaded {
            self.caption_dirty = false;
            let w = self.size.0 - if self.maximized { 0.0 } else { 2.0 } - 138.0;
            let (x, y) = if self.maximized { (0.0, 0.0) } else { (1.0, 1.0) };
            cx.set_caption_region(vec![glasskit::geom::Rect::new(x, y, w.max(0.0), super::paint::TITLE_H)]);
        }
        busy || self.edits_due
    }

    fn step_rows(&mut self, dt: f32) -> bool {
        let mut busy = false;
        for r in self.page.rows_mut() {
            busy |= r.vis.step(dt);
            busy |= r.en.step(dt);
            if let Some((_, t)) = &mut r.msg {
                busy |= t.step(dt);
            }
            if let Some(t) = &mut r.hl {
                *t += dt;
                if *t > 1.7 {
                    r.hl = None;
                } else {
                    busy = true;
                }
            }
            match &mut r.ctl {
                Ctl::Toggle(t) | Ctl::Backend(t, _) => busy |= t.step(dt),
                Ctl::Slider(s) => busy |= s.step(dt),
                Ctl::Seg(s) => busy |= s.step(dt),
                Ctl::Select(s) => busy |= s.step(dt),
                Ctl::Field(f) => busy |= f.tf.step(dt),
                Ctl::Week(w) => {
                    for p in w.pop.iter_mut() {
                        busy |= p.step(dt);
                    }
                }
                _ => {}
            }
        }
        if let Some((_, t)) = &mut self.hl_card {
            *t += dt;
            if *t > 1.7 {
                self.hl_card = None;
            } else {
                busy = true;
            }
        }
        busy
    }

    fn process_edits(&mut self, cx: &mut Cx) {
        self.edits_due = false;
        let mut todo: Vec<Edit> = Vec::new();
        for r in self.page.rows_mut() {
            if let Ctl::Field(f) = &mut r.ctl {
                for e in f.tf.take() {
                    match e {
                        EditEvent::Submit => todo.push(if f.kind == FK::Secret { Edit::Save(r.row.id) } else { Edit::Commit(r.row.id) }),
                        EditEvent::FocusChanged(false) if f.tf.dirty && f.kind != FK::Secret => todo.push(Edit::Commit(r.row.id)),
                        EditEvent::Cancel => todo.push(Edit::Revert(r.row.id)),
                        _ => {}
                    }
                }
            }
        }
        for e in self.search.field.take() {
            if e == EditEvent::Changed {
                todo.push(Edit::Search);
            }
        }
        for e in todo {
            match e {
                Edit::Commit(id) => self.commit_field(id, cx),
                Edit::Revert(id) => {
                    self.revert_field(id);
                    self.set_focus_id(None, false);
                }
                Edit::Save(id) => self.act(super::ui::Act::SecretSave(id.to_string()), cx),
                Edit::Search => {
                    let text = self.search.field.text();
                    let step = self.model.search_input(&text);
                    self.apply_search_step(step, cx);
                    if let SearchStep::Close { .. } = step_kind(&text) {}
                }
            }
        }
    }

    fn revert_field(&mut self, id: &str) {
        let model = &self.model;
        for r in self.page.rows_mut() {
            if r.row.id == id {
                if let Ctl::Field(f) = &mut r.ctl {
                    f.tf.set_text(&field_text(r.row, model));
                }
            }
        }
    }

    fn commit_field(&mut self, id: &'static str, cx: &mut Cx) {
        let mut val = None;
        for r in self.page.rows_mut() {
            if r.row.id == id {
                if let Ctl::Field(f) = &mut r.ctl {
                    let t = f.tf.text();
                    f.tf.dirty = false;
                    val = Some((r.row, match f.kind {
                        FK::Number => t.trim().parse::<f64>().map(|n| json!(n)).unwrap_or(json!(t)),
                        _ => json!(t),
                    }));
                }
            }
        }
        if let Some((row, v)) = val {
            match self.model.try_set(row.key, v) {
                Ok(calls) => self.run(cx, calls),
                Err(_) => {}
            }
            self.revert_field(id);
            self.sync_page();
        }
    }

    /// Scrolls a search / URL target into the middle of the window and flashes it, once the page has been laid out.
    fn resolve_goto(&mut self) {
        let Some(g) = self.goto.clone() else { return };
        if self.page.rects.is_empty() || self.leaving.is_some() {
            return;
        }
        let reduce = self.reduce();
        let view = self.content_rect().h;
        let mut target = None;
        if !g.row_id.is_empty() {
            if let Some(r) = self.page.row_mut(&g.row_id) {
                r.hl = Some(0.0);
                target = Some(r.rect);
            }
        } else if let Some(rc) = self.card_rect_by_title(&g.title) {
            self.hl_card = Some((g.title.clone(), 0.0));
            target = Some(rc);
        }
        self.goto = None;
        if let Some(r) = target {
            self.page.scroll.scroll_to((r.cy() - view / 2.0).max(0.0), !reduce);
        }
    }
}

fn step_kind(_t: &str) -> SearchStep {
    SearchStep::None
}
