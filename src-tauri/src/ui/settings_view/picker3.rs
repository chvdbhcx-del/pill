//! The island edge-style cards: liquid wave, tail, legs, glow, plain. The previews follow the reaction chips.
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use crate::ui::pets::characters::Characters;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::svgpath::{build_geometry, Affine, Path};
use pill_ui_core::settings::pickers::{pet_state_for, REACTIONS, STYLES};
use std::f32::consts::PI;

const COL_W: f32 = (CARD_W - 16.0) / 3.0;
const STAGE_H: f32 = 92.0;
const X0: f32 = 16.0;
const X1: f32 = 133.0;
const R: f32 = 13.0;
const BASE: f32 = 36.0;

fn tp(u: f32) -> f32 {
    (PI * u).sin().max(0.0).powf(0.6)
}
fn beat(t: f32) -> f32 {
    let b = 0.5 + 0.5 * (2.0 * PI * t / 1.1).cos();
    b * b
}

/// Wave offset (px) of the liquid edge at `u` in 0..1; `t` seconds on the clock, `tt` seconds since the reaction started.
pub fn wave(state: &str, u: f32, t: f32, tt: f32) -> f32 {
    match state {
        "calm" => 1.6 * (u * 8.2 - t * 0.8).sin() + 0.9 * (u * 3.8 + t * 0.5).sin(),
        "music" => (1.5 + 5.0 * beat(t)) * tp(u) * (u * 75.4 - t * 9.0).sin(),
        "ai" => 3.0 * tp(u) * (u * 25.1 - t * 2.2).sin(),
        "done" => (8.0 * (-tt / 1.1).exp() + 1.2) * tp(u) * ((u * 18.8 - t * 3.0).sin() + 0.5 * (u * 31.4 + t * 2.0).sin()),
        "attention" => 3.0 * tp(u) * (u * 138.0 - t * 14.0).sin() * (0.8 + 0.2 * (t * 9.0).sin()),
        "file" => (-tt / 0.7).exp() * 5.0 * tp(u) * (u * 22.0 - t * 4.0).sin() + 3.2 * (PI * u).sin(),
        _ => 5.0 * (PI * u).sin(),
    }
}

fn hex(h: &str) -> Color {
    let v = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    Color::rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl SettingsView {
    pub(super) fn paint_chars(&mut self, d: &Dc, fid: u32, y: f32, vp: (f32, f32)) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let cur = self.model.get_str("character");
        let ring = self.focus_ring_for(fid);
        let mut yy = y;
        for (ri, chunk) in STYLES.chunks(3).enumerate() {
            let h = chunk.iter().map(|c| self.card_h(d, c.desc.get(d.lang))).fold(0.0, f32::max);
            for (i, c) in chunk.iter().enumerate() {
                let r = Rect::new(i as f32 * (COL_W + 8.0), yy, COL_W, h);
                let title = c.name.get(d.lang).to_string();
                self.pets.cards.insert(title.clone(), r);
                if r.bottom() >= vp.0 && r.y <= vp.1 {
                    let k = key(&format!("style:{}", c.id));
                    self.ui.add(k, r, Act::StyleCard(c.id.to_string()), true, false, true);
                    let on = self.card_on_t(&format!("style:{}", c.id), c.id == cur);
                    self.card_frame(d, k, r, on, &title, c.id);
                    let stage = Rect::new(r.x + 9.0, r.y + 9.0, r.w - 18.0, STAGE_H);
                    self.paint_style_stage(d, stage, c.id);
                    self.card_text(d, r, &title, c.desc.get(d.lang));
                    if ring.0 && ring.1 && self.pets.focus_i == ri * 3 + i {
                        cv.focus_ring(r, 14.0, p.accent, 1.0);
                    }
                }
            }
            yy += h + 8.0;
        }
        yy - y + 4.0
    }

    fn body_path(&self, cv: &glasskit::canvas::Canvas, m: &Affine, wave_pts: Option<&[(f32, f32)]>) {
        let mut d = format!("M{X0} 0H{X1}");
        match wave_pts {
            Some(pts) => {
                for (x, yv) in pts {
                    d.push_str(&format!("L{x:.1} {yv:.1}"));
                }
                d.push('Z');
            }
            None => d.push_str(&format!("V{}a{R} {R} 0 0 1-{R} {R}H{}a{R} {R} 0 0 1-{R}-{R}Z", BASE - R, X0 + R)),
        }
        if let Ok(path) = Path::parse(&d) {
            if let Ok(g) = build_geometry(cv.gfx, &path, m, true) {
                if let Ok(g) = windows::core::Interface::cast::<windows::Win32::Graphics::Direct2D::ID2D1Geometry>(&g) {
                    cv.geometry(&g, Some(Color::rgb8(0, 0, 0)), None);
                }
            }
        }
    }
}

impl SettingsView {
    fn paint_style_stage(&mut self, d: &Dc, st: Rect, id: &str) {
        let cv = d.cv;
        let state = self.model.reaction.current();
        let (t, tt) = (self.pets.clock, self.pets.since);
        let s = (st.w / 149.0).max(STAGE_H / 92.0);
        let m = Affine::translate(st.x, st.y).mul(&Affine::scale(s, s));
        let reduce = self.reduce();
        cv.with_clip_rr(st, 8.0, |cv| match id {
            "liquid" => {
                let mut pts: Vec<(f32, f32)> = Vec::new();
                let mut x = X1;
                while x >= X0 {
                    let u = (x - X0) / (X1 - X0);
                    let dd = (x - X0).min(X1 - x);
                    let corner = if dd < R { R - (R * R - (R - dd) * (R - dd)).max(0.0).sqrt() } else { 0.0 };
                    let w = if reduce { wave(state, u, 0.0, 9.0) } else { wave(state, u, t, tt) };
                    pts.push((x, BASE - corner + w));
                    x -= 1.5;
                }
                self.body_path(cv, &m, Some(&pts));
                let mut line = String::new();
                for (i, (x, yv)) in pts.iter().enumerate() {
                    line.push_str(&format!("{}{x:.1} {yv:.1}", if i == 0 { "M" } else { "L" }));
                }
                if let (Some(r), Ok(path)) = (REACTIONS.iter().find(|r| r.id == state), Path::parse(&line)) {
                    if let Ok(g) = build_geometry(cv.gfx, &path, &m, false) {
                        use windows::core::Interface;
                        if let Ok(g) = g.cast::<windows::Win32::Graphics::Direct2D::ID2D1Geometry>() {
                            let c = hex(r.line).fade(if state == "sleep" { 0.7 } else { 1.0 });
                            cv.geometry(&g, None, Some((c, 1.5 * s)));
                        }
                    }
                }
                let ball = match state {
                    "ai" => Some((30.0 + (t * 24.0) % 90.0, BASE + 15.0 + 5.0 * (t * 3.0).sin(), 1.0)),
                    "done" => Some((100.0, BASE + 14.0 + 3.0 * (t * 3.0).sin(), (-tt / 0.8).exp())),
                    _ => None,
                };
                if let Some((bx, by, a)) = ball {
                    cv.fill_circle((st.x + bx * s, st.y + by * s), 3.4 * s, Color::rgba(0.0, 0.0, 0.0, a));
                }
            }
            "glow" => {
                let op = match state {
                    "calm" => 0.06,
                    "music" => 0.25 + 0.45 * beat(t),
                    "ai" => 0.22 + 0.18 * (t * 1.6).sin(),
                    "done" => 0.6 * (-tt / 1.2).exp(),
                    "attention" => 0.25 + 0.35 * if (t * 10.0).sin() > 0.0 { 1.0 } else { 0.4 },
                    "file" => 0.35,
                    _ => 0.07 + 0.04 * (t * 1.2).sin(),
                };
                let gl = REACTIONS.iter().find(|r| r.id == state).map(|r| hex(r.glow)).unwrap_or(Color::rgb8(255, 255, 255));
                for (k, grow) in [(0.25, 9.0), (0.25, 6.0), (0.25, 3.0), (0.25, 0.0)] {
                    let r = Rect::new(st.x + (74.5 - 48.0 - grow) * s, st.y + (BASE + 1.0 - 9.0 - grow * 0.6) * s, (96.0 + 2.0 * grow) * s, (18.0 + 1.2 * grow) * s);
                    cv.fill_rr(r, r.h / 2.0, gl.alpha((op * k as f32).max(0.0)));
                }
                self.body_path(cv, &m, None);
            }
            "tail" | "legs" => {
                self.body_path(cv, &m, None);
                self.paint_character(cv, st, s, id == "tail", state);
            }
            _ => self.body_path(cv, &m, None),
        });
    }

    fn paint_character(&mut self, cv: &glasskit::canvas::Canvas, st: Rect, s: f32, tail: bool, state: &str) {
        let name = pet_state_for(state);
        let reduce = self.reduce();
        let slot = if tail { &mut self.pets.tail } else { &mut self.pets.legs };
        if slot.is_none() {
            if let Ok(mut c) = Characters::new(cv.dc) {
                c.set_state(name, None);
                *slot = Some(c);
            }
        }
        if let Some(c) = slot.as_mut() {
            c.set_reduced(reduce);
            if c.state != crate::ui::pets::engine::state_index(name) {
                c.set_state(name, None);
            }
            if tail {
                c.draw_tail(cv.dc, st.x + (X1 - 4.0) * s, st.y + (BASE - 8.0) * s, s);
            } else {
                c.draw_legs(cv.dc, st.x + 74.5 * s, st.y + BASE * s, s);
                c.draw_eyes(cv.dc, st.x + 74.5 * s, st.y + 17.0 * s, s);
            }
        }
    }
}
