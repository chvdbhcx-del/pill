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
    pub(super) fn paint_card(&mut self, d: &Dc, rows: &mut [RowRt], x: f32, y: f32, vp: (f32, f32)) -> f32 {
        let inner_w = CARD_W - 2.0 - 2.0 * PAD_X;
        let mut metrics: Vec<(RowM, f32)> = Vec::with_capacity(rows.len());
        let mut total = 0.0;
        for r in rows.iter() {
            let v = r.vis.value().clamp(0.0, 1.0);
            let m = if v > 0.001 { self.row_metrics(d, r, inner_w) } else { RowM::default() };
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
        let vis = r.vis.value().clamp(0.0, 1.0);
        let en = r.en.value().clamp(0.4, 1.0);
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
                if let Some(t) = r.hl {
                    let a = hl_alpha(t, d.reduced);
                    if a > 0.0 {
                        cv.fill_rect(Rect::new(full.x, full.y, full.w, m.nh), p.accent.alpha(a));
                    }
                }
            });
        });
    }
}
