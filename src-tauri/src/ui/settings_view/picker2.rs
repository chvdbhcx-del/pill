//! The pet grid and the island-style grid: layout in rows of three equal cards, live previews.
use super::pal::mix;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use crate::ui::pets::engine::{box_size, PetEngine, Place, Side, Size};
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::pickers::{self, STYLES};

const COL_W: f32 = (CARD_W - 16.0) / 3.0;
const STAGE_H: f32 = 92.0;

impl SettingsView {
    pub(super) fn paint_pets(&mut self, d: &Dc, fid: u32, y: f32, vp: (f32, f32)) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let sections = self.model.pets.sections(d.lang);
        let cur = self.model.get_str("pet");
        let side = if self.model.get_str("pet.side") == "left" { Side::Left } else { Side::Right };
        let size = Size::parse(&self.model.get_str("pet.size"));
        let ring = self.focus_ring_for(fid);
        let state = if self.pets.state.is_empty() { "calm" } else { self.pets.state };
        let state = if state == "ai" { "thinking" } else { state };
        let mut yy = y;
        let mut ci = 0usize;
        for (si, sec) in sections.iter().enumerate() {
            if si > 0 || y > 80.0 {
                yy += 24.0;
            }
            if yy + 16.0 >= vp.0 && yy <= vp.1 {
                cv.text(Rect::new(4.0, yy, CARD_W - 8.0, 16.0), &super::txt::upper(&sec.caption), Style::new(11.0, 600).track(0.88), p.cap);
            }
            yy += 16.0 + 8.0;
            for chunk in sec.cards.chunks(3) {
                let h = chunk.iter().map(|c| self.card_h(d, &c.desc)).fold(0.0, f32::max);
                for (i, c) in chunk.iter().enumerate() {
                    let r = Rect::new(i as f32 * (COL_W + 8.0), yy, COL_W, h);
                    self.pets.cards.insert(c.name.clone(), r);
                    if r.bottom() >= vp.0 && r.y <= vp.1 {
                        let k = key(&format!("pet:{}", c.id));
                        self.ui.add(k, r, Act::PetCard(c.id.clone()), true, false, true);
                        let on = self.card_on_t(&format!("pet:{}", c.id), c.id == cur);
                        self.card_frame(d, k, r, on, &c.name, &c.id);
                        let stage = Rect::new(r.x + 9.0, r.y + 9.0, r.w - 18.0, STAGE_H);
                        self.paint_stage(d, stage, &c.id, state, side, size);
                        self.card_text(d, r, &c.name, &c.desc);
                        if ring.0 && ring.1 && self.pets.focus_i == ci {
                            cv.focus_ring(r, 14.0, p.accent, 1.0);
                        }
                    }
                    ci += 1;
                }
                yy += h + 8.0;
            }
        }
        self.pets.count_pets = ci;
        yy - y + 4.0
    }

    fn paint_stage(&mut self, d: &Dc, st: Rect, id: &str, state: &str, side: Side, size: Size) {
        let (cv, p) = (d.cv, d.p);
        let bg = if p.dark { Color::rgb8(0x14, 0x13, 0x16) } else { p.ctl };
        cv.fill_rr(st, 8.0, bg);
        if id == "none" {
            cv.text(Rect::new(st.x, st.y + 36.0, st.w, 20.0), "\u{2014}", Style::new(18.0, 400).center(), Color::rgb8(0x55, 0x53, 0x5a));
            return;
        }
        cv.with_clip_rr(st, 8.0, |cv| {
            let nr = Rect::new(st.cx() - 74.0, st.y, 148.0, 60.0);
            cv.fill_rr(nr, 30.0, Color::rgb8(0, 0, 0));
            cv.fill_rect(Rect::new(nr.x, nr.y, nr.w, 30.0), Color::rgb8(0, 0, 0));
            cv.text(Rect::new(st.cx() - 58.0, st.y, 116.0, 56.0), "12:34", Style::new(15.0, 600).mono(), Color::rgb8(0x9a, 0x9a, 0xa4));
        });
        let (w, h) = box_size(id, size, Place::Stage, 1.0);
        let x = if side == Side::Right { st.cx() + 62.0 - w } else { st.cx() - 62.0 };
        let r = Rect::new(x, st.y + 56.0 - h, w, h);
        let o = self.ui.page_origin;
        let wr = Rect::new(r.x + o.0, r.y + o.1, r.w, r.h);
        let reduce = self.reduce();
        let scale = cv.scale;
        if !self.pets.engines.contains_key(id) {
            if let Ok(e) = PetEngine::with_context(cv.dc) {
                self.pets.engines.insert(id.to_string(), e);
            }
        }
        if let Some(e) = self.pets.engines.get_mut(id) {
            e.set_reduced(reduce);
            if e.preview(cv.dc, id, state, side, wr, scale).is_err() {
                self.pets.engines.remove(id);
            }
        }
    }
}
