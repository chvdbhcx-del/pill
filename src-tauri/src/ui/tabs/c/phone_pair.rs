//! Pairing on the Phone sub-tab. No phone yet: the QR card is the main content, centred, with the short how-to, the code and "New code". A phone
//! is already paired: the QR is hidden, and "Pair another phone" slides this card in from the right as an overlay on a scrim (the info tiles stay
//! where they are, so nothing reflows); the close button, a click on the scrim and Escape slide it away. The slide is `Dash::qr` (a [`Flow`]
//! with a little overshoot, the panel is painted wider than it shows so the overshoot never opens a gap at the edge).
//!
//! [`Flow`]: crate::ui::liquid::Flow
use super::phone::{PhoneView, K};
use crate::ui::tabs::b::kit::{button, button_w, Bk};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::Tr;

/// Width of the sliding panel.
const PANEL_W: f32 = 240.0;
const BTN_H: f32 = 28.0;

/// Horizontal runs of dark modules `(col, row, length)` of a QR given as rows of `#` / `.` (fewer rects, no seams).
fn qr_runs(rows: &[String]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let b = row.as_bytes();
        let mut x = 0;
        while x < b.len() {
            if b[x] == b'#' {
                let s = x;
                while x < b.len() && b[x] == b'#' {
                    x += 1;
                }
                out.push((s, y, x - s));
            } else {
                x += 1;
            }
        }
    }
    out
}

impl PhoneView {
    /// Builds the QR rows when the pairing URL changed. Returns the URL and the code to show.
    fn qr_state(&mut self) -> (Option<String>, Option<String>) {
        let url = self.model.code().map(|c| c.url.clone());
        let code = self.model.code().map(|c| c.code.clone());
        if let Some(url) = &url {
            if self.qr_cache.as_ref().map(|c| c.0.as_str()) != Some(url.as_str()) {
                let rows = pill_ui_core::panel::b::qr::qr(url).map(|q| q.rows()).unwrap_or_default();
                self.qr_cache = Some((url.clone(), rows));
            }
        }
        (url, code)
    }

    /// The QR itself in the square `qr` (white with a quiet zone, a placeholder while there is no code), popping in on `qr_k`.
    fn paint_qr(&self, cv: &Canvas, th: &Theme, qr: Rect, has_url: bool) {
        let side = qr.w;
        let pop = self.qr_k.raw();
        cv.with_opacity(self.qr_k.at().max(0.15), |cv| {
            cv.with_scale_about(qr.center(), 0.9 + 0.1 * pop.clamp(0.0, 1.2), |cv| {
                cv.with_clip_rr(qr, 12.0, |cv| {
                    cv.fill_rect(qr, if has_url { Color::rgb8(255, 255, 255) } else { th.raised2 });
                    if let Some((_, rows)) = self.qr_cache.as_ref().filter(|_| has_url) {
                        let quiet = 4usize;
                        let px = side / (rows.len() + 2 * quiet) as f32;
                        let sc = cv.scale;
                        let snap = |v: f32| (v * sc).round() / sc;
                        for (cx, cy, run) in qr_runs(rows) {
                            let x0 = qr.x + (cx + quiet) as f32 * px;
                            let y0 = qr.y + (cy + quiet) as f32 * px;
                            let (sx, sy) = (snap(x0), snap(y0));
                            cv.fill_rect(Rect::new(sx, sy, snap(x0 + run as f32 * px) - sx, snap(y0 + px) - sy), Color::rgb8(0, 0, 0));
                        }
                    }
                });
            });
        });
    }

    fn spaced(code: &str) -> String {
        format!("{} {}", &code[..4.min(code.len())], &code[4.min(code.len())..])
    }

    fn new_code_button(&mut self, cv: &Canvas, th: &Theme, cx: f32, y: f32, live: bool) {
        let label = self.model.lang().t("Новый код", "New code");
        let bw = button_w(cv, th, label, false, Bk::Normal);
        let br = Rect::new(cx - bw / 2.0, y, bw, BTN_H);
        button(cv, th, &self.ic, br, label, None, Bk::Normal, self.clicks.hover_amt(&K::NewCode), self.clicks.press_scale(&K::NewCode, 0.96), true);
        if live {
            self.clicks.add(K::NewCode, br);
        }
    }

    /// No phone is paired: the QR card is the content.
    pub(super) fn draw_pair_main(&mut self, cv: &Canvas, th: &Theme, body: Rect) {
        let l = self.model.lang();
        let (w, h) = (body.w.min(468.0), body.h.min(280.0));
        let card = Rect::new(body.cx() - w / 2.0, body.y + (body.h - h) / 2.0, w, h);
        cv.fill_rr(card, 16.0, th.raised);
        let side = (h - 40.0).clamp(96.0, 200.0);
        let qr = Rect::new(card.x + 20.0, card.cy() - side / 2.0, side, side);
        let (url, code) = self.qr_state();
        self.paint_qr(cv, th, qr, url.is_some());
        // the column right of the QR, centred as a block
        let (cx0, cw) = (qr.right() + 20.0, card.right() - 20.0 - (qr.right() + 20.0));
        let hs = Style::new(12.0, 400).wrap();
        let how = l.t("Наведи камеру телефона на код или открой Pill Phone и нажми «Сканировать QR-код».", "Scan with the phone camera, or open Pill Phone and tap Scan QR code.");
        let how_h = cv.measure(how, hs, cw).1.ceil();
        let block = 22.0 + 6.0 + how_h + 14.0 + 24.0 + 12.0 + BTN_H;
        let mut y = card.cy() - block / 2.0;
        cv.text(Rect::new(cx0, y, cw, 22.0), l.t("Подключить телефон", "Pair your phone"), Style::new(15.0, 600).ellipsis(), th.text);
        y += 28.0;
        cv.text(Rect::new(cx0, y, cw, how_h), how, hs, th.dim);
        y += how_h + 14.0;
        if let Some(code) = code {
            cv.text(Rect::new(cx0, y, cw, 24.0), &Self::spaced(&code), Style::new(17.0, 600).mono().ellipsis(), th.text);
        }
        y += 36.0;
        let bw = button_w(cv, th, l.t("Новый код", "New code"), false, Bk::Normal);
        self.new_code_button(cv, th, cx0 + bw / 2.0, y, true);
    }

    /// The pairing overlay of a paired phone: scrim over the tiles, the panel sliding in from the right.
    pub(super) fn draw_qr_panel(&mut self, cv: &Canvas, th: &Theme, body: Rect) {
        let s = self.dash.qr.at();
        if s < 0.003 && !self.dash.qr_open {
            return;
        }
        let k = s.clamp(0.0, 1.0);
        let l = self.model.lang();
        cv.fill_rr(body, 16.0, th.bg.fade(0.6 * k));
        if self.dash.qr_open {
            self.clicks.add(K::Scrim, body);
        }
        let x = body.right() - PANEL_W * s;
        let panel = Rect::new(x, body.y, PANEL_W + 40.0, body.h);
        cv.with_clip(body, |cv| {
            cv.fill_rr(panel, 16.0, th.raised);
            cv.stroke_rr(panel.inset(0.5), 15.5, 1.0, th.border);
        });
        let (url, code) = self.qr_state();
        cv.with_clip(body, |cv| {
            cv.with_opacity(k, |cv| {
                cv.text(Rect::new(x + 16.0, body.y + 12.0, PANEL_W - 16.0 - 48.0, 24.0), l.t("Подключить ещё телефон", "Pair another phone"), Style::new(13.0, 600).ellipsis(), th.text);
                let xr = Rect::new(x + PANEL_W - 12.0 - 24.0, body.y + 12.0, 24.0, 24.0);
                let hv = self.clicks.hover_amt(&K::CloseQr);
                cv.fill_circle(xr.center(), 12.0, th.hover.alpha(0.0).mix(th.hover, hv));
                self.ic.draw_sw(cv, "x", xr.centered(14.0, 14.0), th.dim.mix(th.text, hv), 1.8);
                let side = (body.h - 12.0 - 24.0 - 10.0 - 8.0 - 22.0 - 8.0 - BTN_H - 14.0).clamp(96.0, 150.0);
                let qr = Rect::new(x + PANEL_W / 2.0 - side / 2.0, body.y + 12.0 + 24.0 + 10.0, side, side);
                self.paint_qr(cv, th, qr, url.is_some());
                if let Some(code) = code {
                    cv.text(Rect::new(x + 12.0, qr.bottom() + 8.0, PANEL_W - 24.0, 22.0), &Self::spaced(&code), Style::new(14.0, 600).mono().center(), th.text2);
                }
                self.new_code_button(cv, th, x + PANEL_W / 2.0, body.bottom() - 14.0 - BTN_H, self.dash.qr_open);
                if self.dash.qr_open {
                    self.clicks.add(K::CloseQr, xr);
                }
            });
        });
    }
}
