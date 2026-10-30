//! The Character page cards: island edge styles (liquid, tail, legs, glow, plain) and the pets, each with a live preview that loops.
use super::pal::mix;
use super::rows::*;
use super::txt;
use super::ui::{key, Act};
use super::SettingsView;
use crate::ui::pets::characters::Characters;
use crate::ui::pets::engine::{box_size, PetEngine, Place, Side, Size};
use glasskit::color::Color;
use glasskit::ease::{curves, Tween};
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::pickers::{self, STYLES};
use std::collections::HashMap;

#[derive(Default)]
pub struct Pickers {
    pub state: &'static str,
    pub engines: HashMap<String, PetEngine>,
    pub tail: Option<Characters>,
    pub legs: Option<Characters>,
    pub sel: HashMap<String, Tween<f32>>,
    /// (title, rect in page space) of every card painted last frame: where "go to" aims
    pub cards: HashMap<String, Rect>,
    pub focus_i: usize,
    pub count_pets: usize,
    pub clock: f32,
    pub since: f32,
}

const COL_W: f32 = (CARD_W - 16.0) / 3.0;
const STAGE_H: f32 = 92.0;

impl SettingsView {
    pub(super) fn pets_set_state(&mut self, st: &'static str) {
        self.pets.state = st;
        self.pets.since = 0.0;
        let name = if st == "ai" { "thinking" } else { st };
        for e in self.pets.engines.values_mut() {
            e.set_state(name, None);
        }
        for c in [self.pets.tail.as_mut(), self.pets.legs.as_mut()].into_iter().flatten() {
            c.set_state(name, None);
        }
    }

    pub(super) fn step_pickers(&mut self, dt: f32) -> bool {
        let mut busy = false;
        for t in self.pets.sel.values_mut() {
            busy |= t.step(dt);
        }
        if self.model.current != "character" || self.wiz.active() {
            return busy;
        }
        let reduce = self.reduce();
        self.pets.clock += dt;
        self.pets.since += dt;
        if reduce {
            return busy;
        }
        for e in self.pets.engines.values_mut() {
            e.set_reduced(false);
            e.tick(dt * 1000.0);
        }
        for c in [self.pets.tail.as_mut(), self.pets.legs.as_mut()].into_iter().flatten() {
            c.set_clock(self.pets.clock, self.pets.since, [0.0, 0.0]);
            c.tick(dt * 1000.0);
        }
        true
    }

    pub(super) fn card_rect_by_title(&self, t: &str) -> Option<Rect> {
        self.pets.cards.get(t).copied()
    }

    pub(super) fn card_on_t(&mut self, id: &str, on: bool) -> f32 {
        let reduce = self.reduce();
        let t = self.pets.sel.entry(id.to_string()).or_insert_with(|| Tween::at_rest(on as u8 as f32));
        let to = on as u8 as f32;
        if (t.target() - to).abs() > 0.01 {
            if reduce {
                *t = Tween::at_rest(to);
            } else {
                t.retarget(to, 0.22, curves::EASE_OUT);
            }
        }
        t.value().clamp(0.0, 1.0)
    }

    /// The card frame: hover / selected look, press scale; returns the stage rectangle inside it.
    pub(super) fn card_frame(&mut self, d: &Dc, k: u64, r: Rect, on: f32, title: &str, id_key: &str) {
        let (cv, p) = (d.cv, d.p);
        let (hv, pr) = (self.ui.hover_of(k), self.ui.press_of(k));
        let scale = 1.0 - 0.02 * pr.max(0.0);
        cv.with_scale_about(r.center(), scale, |cv| {
            let bg = mix(mix(p.pcard, p.pcard_hover, hv), p.pcard_on, on);
            if on > 0.01 {
                cv.stroke_rr(r.inset(-1.0), 15.0, 2.0, p.pcard_on_ring.fade(on));
            }
            cv.fill_rr(r, 14.0, bg);
            let line = mix(Color::rgba(0.0, 0.0, 0.0, 0.0), if on > 0.01 { p.pcard_on_line } else { p.card_line }, (hv * 0.8).max(on));
            cv.stroke_rr(r.inset(0.5), 13.5, 1.0, line);
            if let Some(a) = self.hl_card.as_ref().filter(|a| a.0 == title).map(|a| a.1) {
                let al = hl_alpha(a, d.reduced);
                cv.fill_rr(r, 14.0, p.accent.alpha(al));
            }
        });
        let _ = id_key;
    }

    pub(super) fn card_text(&mut self, d: &Dc, r: Rect, name: &str, desc: &str) {
        let (cv, p) = (d.cv, d.p);
        cv.text(Rect::new(r.x + 12.0, r.y + 9.0 + STAGE_H + 8.0, r.w - 24.0, 18.0), name, Style::new(13.0, 600).ellipsis(), p.text);
        txt::para(cv, Rect::new(r.x + 12.0, r.y + 9.0 + STAGE_H + 8.0 + 20.0, r.w - 24.0, 80.0), desc, st_hint(), p.dim);
    }

    pub(super) fn card_h(&self, d: &Dc, desc: &str) -> f32 {
        let n = txt::lines(d.cv, desc, st_hint(), COL_W - 24.0).max(1) as f32;
        1.0 + 8.0 + STAGE_H + 8.0 + 18.0 + 2.0 + 16.0 * n + 12.0 + 1.0
    }
}
