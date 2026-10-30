//! Painting the control of a row (toggle, slider, segmented, select, key caps, fields, weekday chips, buttons).
use super::page::*;
use super::pal::mix;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::controls::{Button, Widget};
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::hotkey;
use pill_ui_core::settings::model::WEEKDAYS;
use pill_ui_core::settings::schema::{Action, ButtonStyle, Control};

impl SettingsView {
    pub(super) fn paint_ctl(&mut self, d: &Dc, r: &mut RowRt, m: &RowM, full: Rect, enabled: bool) {
        let (cv, th, p) = (d.cv, d.th, d.p);
        let inner_x = full.x + PAD_X;
        let inner_w = full.w - 2.0 * PAD_X;
        let under = full.y + 11.0 + m.title_h + m.hint_h + m.msg_h + 10.0;
        let cr = if m.block {
            Rect::new(inner_x, under, if matches!(r.ctl, Ctl::Seg(_) | Ctl::Week(_)) { m.cw.min(inner_w) } else { inner_w }, m.ch)
        } else {
            Rect::new(full.right() - PAD_X - m.cw, full.y + (m.nh - m.ch) / 2.0, m.cw, m.ch)
        };
        r.ctl_rect = cr;
        let ring = self.focus_ring_for(r.fid);
        let rid = r.row.id;
        match &mut r.ctl {
            Ctl::None | Ctl::Custom(_) => {}
            Ctl::Toggle(t) | Ctl::Backend(t, _) => {
                t.set_enabled(enabled);
                t.set_focus(ring.0, ring.1);
                // the switch knob is white in settings.css; the theme's `thumb` colour belongs to the slider
                let tt = glasskit::theme::Theme { thumb: glasskit::color::Color::rgb8(255, 255, 255), ..*th };
                t.draw(cv, &tt, cr);
            }
            Ctl::Slider(s) => {
                s.set_enabled(enabled);
                s.set_focus(ring.0, ring.1);
                s.draw(cv, th, Rect::new(cr.x, cr.y, 140.0, 16.0));
                let f: fn(f32) -> String = s.format.unwrap_or(fmt_plain);
                cv.text(Rect::new(cr.right() - 48.0, cr.y - 2.0, 48.0, 20.0), &f(s.shown()), Style::new(12.0, 400).right().mono(), p.text2);
            }
            Ctl::Seg(s) => {
                s.set_enabled(enabled);
                s.set_focus(ring.0, ring.1);
                s.draw(cv, th, cr);
            }
            Ctl::Select(s) => {
                s.set_enabled(enabled);
                s.set_focus(ring.0, ring.1);
                s.draw(cv, th, cr);
            }
            Ctl::Hotkey => {
                let action = match &r.row.control {
                    Control::Hotkey { action } => *action,
                    _ => "",
                };
                let combo = self.model.combo_of(action);
                self.paint_keys(d, cr, &combo, Some(action), ring, enabled);
            }
            Ctl::Keys(c) => {
                let c = c.clone();
                self.paint_keys(d, cr, &c, None, (false, false), false);
            }
            Ctl::Field(f) => {
                let mut fr = cr;
                if f.kind == FK::Secret {
                    fr = self.paint_secret_buttons(d, rid, cr, enabled);
                    let has = self.data.has_secret(rid);
                    let ph = self.secret_placeholder(d.lang, rid, has);
                    if !ph.is_empty() && f.tf.input.placeholder != ph {
                        f.tf.input.placeholder = ph;
                    }
                }
                f.tf.input.enabled = enabled;
                f.tf.draw(cv, fr, th);
            }
            Ctl::Week(w) => {
                let days = self.model.quiet_days();
                let st = Style::new(13.0, 400);
                let mut x = cr.x;
                for (i, (id, label)) in WEEKDAYS.iter().enumerate() {
                    let on = days.iter().any(|dd| dd == id);
                    let lab = label.get(d.lang);
                    let cw = cv.text_width(lab, st) + 24.0;
                    let rc = Rect::new(x, cr.y, cw, 30.0);
                    let k = key(&format!("day:{id}"));
                    self.ui.add(k, rc, Act::Day(id.to_string()), true, false, enabled);
                    let (hv, pr) = (self.ui.hover_of(k), self.ui.press_of(k));
                    let scale = (1.0 - 0.07 * pr.max(0.0)) * w.pop[i].value();
                    let bg = if on { p.accent_bg } else { mix(p.ctl, p.ctl_hover, hv) };
                    cv.with_scale_about(rc.center(), scale, |cv| {
                        cv.fill_rr(rc, 8.0, bg);
                        if on {
                            cv.stroke_rr(rc.inset(0.5), 7.5, 1.0, p.accent_line);
                        }
                        cv.text(rc, lab, Style::new(13.0, 400).center(), if on { p.text } else { p.item });
                    });
                    if ring.0 && ring.1 && w.cur == i {
                        cv.focus_ring(rc, 8.0, p.accent, 1.0);
                    }
                    x += cw + 8.0;
                }
            }
            Ctl::Button => {
                let (label, style) = match &r.row.control {
                    Control::Button { label, style, .. } => (label.get(d.lang), *style),
                    _ => ("", ButtonStyle::Normal),
                };
                let k = key(&format!("rowbtn:{rid}"));
                let (kind, small) = btn_style(style);
                let label = self.button_label(d.lang, rid, label);
                let busy = self.busy_button(rid);
                let b = self.ui.button(k, || Button::new(&label).kind(kind).small(small));
                b.label = label;
                b.kind = kind;
                b.small = small;
                b.state_mut().set_enabled(enabled && !busy);
                b.set_focus(ring.0, ring.1);
                b.draw(cv, th, cr);
                self.ui.add(k, cr, Act::RowButton(rid.to_string()), true, true, enabled && !busy);
            }
        }
    }
}


#[allow(dead_code)]
fn _keep(_: Action) {
    let _ = hotkey::NOT_ASSIGNED;
}
