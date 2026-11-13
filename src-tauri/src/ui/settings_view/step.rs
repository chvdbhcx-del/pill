//! Per-frame work: advancing every animation, collecting what the text fields did, resolving a pending "go to row", the caption region.
use super::flow::{tick, Pulse};
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
        busy |= tick(&mut self.win_t, dt);
        busy |= tick(&mut self.enter_t, dt);
        busy |= tick(&mut self.rec_t, dt);
        busy |= tick(&mut self.side.ind_y, dt);
        busy |= self.search.step(dt, self.model.search.open);
        for d in self.wiz.dots.iter_mut() {
            busy |= tick(d, dt);
        }
        if self.leaving.is_some() {
            busy |= tick(&mut self.leave_t, dt);
            // the old page is gone once it has (almost) arrived at its exit; the next one rises while that last bit settles
            if self.leave_t.at() >= 0.97 {
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
        if let Some(a) = &mut self.page.adv {
            busy |= tick(&mut a.t, dt);
        }
        busy |= self.ui.step(dt);
        busy |= self.step_rows(dt);
        busy |= self.search.field.step(dt);
        busy |= self.step_custom(dt);
        busy |= self.step_phone(dt, cx);
        busy |= self.step_pickers(dt);
        busy |= self.step_preview(dt, cx);
        self.process_edits(cx);
        busy |= self.resolve_goto();
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
        let reduce = self.reduce();
        for r in self.page.rows_mut() {
            busy |= tick(&mut r.vis, dt);
            busy |= tick(&mut r.en, dt);
            busy |= tick(&mut r.mh, dt);
            if r.kick > 0 {
                // a message / status just changed: the next paint sets the new height target, so stay awake until it has
                r.kick -= 1;
                busy = true;
            }
            if let Some((_, f)) = &mut r.msg {
                busy |= tick(f, dt);
                if f.target() == 0.0 && !f.moving() {
                    r.msg = None;
                }
            }
            if let Some(p) = &mut r.hl {
                if p.step(dt, reduce) {
                    busy = true;
                } else {
                    r.hl = None;
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
                        busy |= tick(p, dt);
                    }
                }
                _ => {}
            }
        }
        if let Some((_, p)) = &mut self.hl_card {
            if p.step(dt, reduce) {
                busy = true;
            } else {
                self.hl_card = None;
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
    /// A target inside a closed "Advanced" section opens it first (at once: the scroll target must hold still) and waits for the layout.
    /// An old page name (`quiet`) scrolls to the caption of its section instead and flashes nothing. True while it still has to wait.
    fn resolve_goto(&mut self) -> bool {
        let Some(g) = self.goto.clone() else { return false };
        if self.page.rects.is_empty() || self.leaving.is_some() {
            return false;
        }
        if !g.row_id.is_empty() && self.page.rows().any(|r| r.row.id == g.row_id && r.adv) && self.page.adv.as_ref().is_some_and(|a| !a.on) {
            self.set_advanced(true, true);
        }
        if !self.page.laid_out {
            return true;
        }
        let reduce = self.reduce();
        let view = self.content_rect().h;
        let mut target = None;
        if g.quiet {
            // the top of the section: the caption above the card that holds the row
            let at = self.page.blocks.iter().position(|b| matches!(b, Block::Card(rows) if rows.iter().any(|r| r.row.id == g.row_id)));
            // (a picker gallery - the edge styles, the pets - is part of the section: it brings its caption, or is the start itself when it draws its own)
            let start = at.and_then(|i| (0..i).rev().find(|j| matches!(self.page.blocks[*j], Block::Cap(_) | Block::Pets(_) | Block::Chars(_))));
            let start = start.map(|j| if j > 0 && matches!(self.page.blocks[j], Block::Pets(_) | Block::Chars(_)) && matches!(self.page.blocks[j - 1], Block::Cap(_)) { j - 1 } else { j });
            let top = start.and_then(|j| self.page.rects.get(j)).map(|r| r.y);
            self.goto = None;
            if let Some(y) = top.or(at.and_then(|i| self.page.rects.get(i)).map(|r| r.y)) {
                self.page.scroll.scroll_to((y - 16.0).max(0.0), !reduce);
            }
            return false;
        }
        if !g.row_id.is_empty() {
            if let Some(r) = self.page.row_mut(&g.row_id) {
                r.hl = Some(Pulse::new());
                target = Some(r.rect);
            }
        } else if let Some(rc) = self.card_rect_by_title(&g.title) {
            self.hl_card = Some((g.title.clone(), Pulse::new()));
            target = Some(rc);
        }
        self.goto = None;
        if let Some(r) = target {
            self.page.scroll.scroll_to((r.cy() - view / 2.0).max(0.0), !reduce);
        }
        false
    }
}

fn step_kind(_t: &str) -> SearchStep {
    SearchStep::None
}
