//! Drawing primitives of the custom widgets: chips, small buttons, text blocks, progress bar. Every primitive measures; with `paint` it also
//! draws and registers its hit, so a widget runs its layout twice (measure for the height, paint for the frame) with the same code.
use super::pal::mix;
use super::rows::*;
use super::txt;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::controls::{Button, ButtonKind, Widget};
use glasskit::ease::curves;
use glasskit::geom::Rect;
use glasskit::text::Style;

impl SettingsView {
    /// Pop scale of a chip that was just switched on (`ChipPop`: .88 -> 1 over 400 ms on the spring-pop curve).
    pub(super) fn pop_scale(&self, k: u64, now_ms: f64) -> f32 {
        match self.data.pops.iter().find(|p| p.0 == k) {
            Some((_, t0)) => {
                let u = ((now_ms - t0) / 400.0).clamp(0.0, 1.0) as f32;
                0.88 + 0.12 * curves::SPRING_POP.at(u)
            }
            None => 1.0,
        }
    }

    pub(super) fn start_pop(&mut self, k: u64) {
        if !self.reduce() {
            self.data.pops.retain(|p| p.0 != k);
            self.data.pops.push((k, self.now_ms));
        }
    }

    /// A chip (provider, day): returns its width. `badges` are (text, accent-coloured) pairs.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn chip(&mut self, d: &Dc, paint: bool, x: f32, y: f32, name: &str, label: &str, badges: &[(String, bool)], on: bool, act: Act) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let st = Style::new(13.0, 400);
        let tw = cv.text_width(label, st);
        let bst = Style::new(11.0, 600);
        let bws: Vec<f32> = badges.iter().map(|b| cv.text_width(&b.0, bst) + 12.0).collect();
        let w = 24.0 + tw + bws.iter().map(|b| b + 8.0).sum::<f32>();
        if !paint {
            return w;
        }
        let r = Rect::new(x, y, w, 30.0);
        let k = key(&format!("chip:{name}"));
        self.ui.add(k, r, act, true, false, true);
        let (hv, pr) = (self.ui.hover_of(k), self.ui.press_of(k));
        let scale = (1.0 - 0.07 * pr.max(0.0)) * if on { self.pop_scale(k, d.now_ms) } else { 1.0 };
        cv.with_scale_about(r.center(), scale, |cv| {
            cv.fill_rr(r, 8.0, if on { p.accent_bg } else { mix(p.ctl, p.ctl_hover, hv) });
            if on {
                cv.stroke_rr(r.inset(0.5), 7.5, 1.0, p.accent_line);
            }
            cv.text(Rect::new(r.x + 12.0, r.y, tw + 2.0, r.h), label, st, if on { p.text } else { p.item });
            let mut bx = r.x + 12.0 + tw + 8.0;
            for (i, b) in badges.iter().enumerate() {
                let br = Rect::new(bx, r.cy() - 8.0, bws[i], 16.0);
                cv.fill_rr(br, 4.0, if p.dark { glasskit::color::Color::rgb8(0x26, 0x44, 0x3d) } else { p.green.alpha(0.18) });
                cv.text(br, &b.0, bst.center(), p.green);
                bx += bws[i] + 8.0;
            }
        });
        w
    }

    /// A small button (32 px high, as beside controls in the web rows) at `x` (its left edge) or ending at `right` when `x` is NaN.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn btn_at(&mut self, d: &Dc, paint: bool, id: &str, label: &str, kind: ButtonKind, x: f32, right: Option<f32>, y: f32, act: Act, enabled: bool) -> Rect {
        let (cv, th) = (d.cv, d.th);
        let k = key(&format!("cb:{id}"));
        let b = self.ui.button(k, || Button::new(label).kind(kind).small(true));
        b.label = label.to_string();
        b.kind = kind;
        b.small = true;
        let w = b.natural_size(cv, th).0;
        let r = match right {
            Some(rt) => Rect::new(rt - w, y, w, 32.0),
            None => Rect::new(x, y, w, 32.0),
        };
        if paint {
            let b = self.ui.button(k, || Button::new(label));
            b.state_mut().set_enabled(enabled);
            b.draw(cv, th, r);
            self.ui.add(k, r, act, true, true, enabled);
        }
        r
    }

    /// A wrapped paragraph at (x, y) of width w; returns its height.
    pub(super) fn para_at(&mut self, d: &Dc, paint: bool, x: f32, y: f32, w: f32, s: &str, st: Style, c: glasskit::color::Color) -> f32 {
        let h = txt::wrapped_h(d.cv, s, st, w, 16.0);
        if paint && h > 0.0 {
            txt::para(d.cv, Rect::new(x, y, w, h), s, st, c);
        }
        h
    }

    /// The progress box: bar of 6 px, filled to `percent` or an indeterminate stripe sliding on its own.
    pub(super) fn bar(&mut self, d: &Dc, r: Rect, percent: u8, indeterminate: bool) {
        let (cv, p) = (d.cv, d.p);
        cv.fill_rr(r, r.h / 2.0, p.ctl);
        if indeterminate {
            let seg = r.w * 0.3;
            let t = if d.reduced { 0.35 } else { ((d.now_ms / 1400.0) % 1.0) as f32 };
            let x = r.x - seg + (r.w + seg) * t;
            cv.with_clip_rr(r, r.h / 2.0, |cv| cv.fill_rr(Rect::new(x, r.y, seg, r.h), r.h / 2.0, p.accent));
        } else if percent > 0 {
            cv.fill_rr(Rect::new(r.x, r.y, (r.w * percent as f32 / 100.0).max(r.h), r.h), r.h / 2.0, p.accent);
        }
    }
}
