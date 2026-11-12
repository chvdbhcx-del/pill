//! Painting a card: its rows, their controls, the dividers; registering the hits of the page.
use super::page::*;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::controls::{Button, Widget};
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::model::WEEKDAYS;
use pill_ui_core::settings::schema::{ButtonStyle, Control};

impl SettingsView {
    /// Paints a card at (`x`, `y`) of width `CARD_W` and returns its height (0 when every row is collapsed).
    /// `vp` is the visible band in content space: rows outside it are measured but not painted.
    /// `k`: how far open the "Advanced" section is when this card is part of it (1 for every other card).
    pub(super) fn paint_card(&mut self, d: &Dc, rows: &mut [RowRt], x: f32, y: f32, vp: (f32, f32), k: f32) -> f32 {
        let inner_w = CARD_W - 2.0 - 2.0 * PAD_X;
        let mut metrics: Vec<(RowM, f32)> = Vec::with_capacity(rows.len());
        let mut total = 0.0;
        for r in rows.iter_mut() {
            if r.adv {
                r.ak = k;
            }
            // the row opens / closes on a spring: a little overshoot opens it a touch too far, never below nothing
            let v = r.vis.at().max(0.0) * k.max(0.0);
            let mut m = if v > 0.001 { self.row_metrics(d, r, inner_w) } else { RowM::default() };
            if v > 0.001 {
                if !r.mh_seen || (r.kick == 0 && !r.mh.moving() && (r.mh.target() - m.msg_need).abs() > 0.01) {
                    // first sight, or a re-wrap after a resize / language change: take the new height at once, nothing to glide from
                    r.mh.jump(m.msg_need);
                    r.mh_seen = true;
                    m = self.row_metrics(d, r, inner_w);
                } else if (r.mh.target() - m.msg_need).abs() > 0.01 {
                    super::flow::go(&mut r.mh, m.msg_need, d.reduced);
                }
            }
            let h = m.nh * v;
            total += h;
            metrics.push((m, h));
        }
        if total < 0.5 {
            return 0.0;
        }
        let card = Rect::new(x, y, CARD_W, total + 2.0);
        let (p, cv) = (d.p, d.cv);
        if card.bottom() >= vp.0 && card.y <= vp.1 {
            cv.fill_rr(card, 12.0, p.card);
            cv.stroke_rr(card.inset(0.5), 11.5, 1.0, p.card_line);
        }
        let mut yy = y + 1.0;
        let mut first = true;
        for (i, r) in rows.iter_mut().enumerate() {
            let (m, h) = metrics[i];
            r.rect = Rect::new(x + 1.0, yy, CARD_W - 2.0, m.nh);
            if h < 0.5 {
                continue;
            }
            if yy + h >= vp.0 && yy <= vp.1 {
                self.paint_row(d, r, &m, h, first);
            } else {
                r.ctl_rect = Rect::new(r.rect.right() - PAD_X - m.cw, r.rect.y + (m.nh - m.ch) / 2.0, m.cw, m.ch);
            }
            first = false;
            yy += h;
        }
        total + 2.0
    }

    fn paint_row(&mut self, d: &Dc, r: &mut RowRt, m: &RowM, h: f32, first: bool) {
        let (cv, p) = (d.cv, d.p);
        let full = r.rect;
        let vis = r.vis.at01() * r.ak.clamp(0.0, 1.0);
        let en = r.en.at().clamp(0.4, 1.0);
        let enabled = r.en_on;
        cv.with_clip(Rect::new(full.x, full.y, full.w, h), |cv| {
            cv.with_opacity(vis * en, |cv| {
                if !first {
                    cv.fill_rect(Rect::new(full.x, full.y, full.w, 1.0), p.row_line);
                }
                let x = full.x + PAD_X;
                if let Ctl::Custom(name) = &r.ctl {
                    let name: &'static str = name;
                    self.custom_paint(d, r, name, full);
                    r.ctl_rect = full;
                } else {
                    self.paint_label(d, r, m, x, full.y);
                    self.paint_ctl(d, r, m, full, enabled);
                }
                if let Some(pulse) = &r.hl {
                    let a = pulse.alpha(d.reduced);
                    if a > 0.0 {
                        cv.fill_rect(Rect::new(full.x, full.y, full.w, m.nh), p.accent.alpha(a));
                    }
                }
            });
        });
    }
}
