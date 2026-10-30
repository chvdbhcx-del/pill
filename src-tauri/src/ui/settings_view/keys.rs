//! Key caps (hotkey rows, wizard step 3) and the Save / Remove buttons beside a secret field.
use super::pal::mix;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::color::Color;
use glasskit::controls::{Button, ButtonKind, Widget};
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::hotkey::{self, key_caps};
use pill_ui_core::settings::tr::pick;

const REC_TEXT: Color = Color::rgb8(0xcf, 0xd5, 0xff);

impl SettingsView {
    /// The partially pressed modifiers while a recorder waits for the key.
    pub(super) fn rec_caps(&self, action: &str) -> Vec<&'static str> {
        if self.model.recorder.is_recording(action) {
            self.rec_live.clone()
        } else {
            Vec::new()
        }
    }

    /// The key caps box. `action` = Some: a recorder (click / Enter starts it); None: read-only.
    pub(super) fn paint_keys(&mut self, d: &Dc, r: Rect, combo: &str, action: Option<&str>, ring: (bool, bool), enabled: bool) {
        let (cv, p) = (d.cv, d.p);
        let recording = action.is_some_and(|a| self.model.recorder.is_recording(a));
        let k = key(&format!("keys:{}", action.unwrap_or("-")));
        if let Some(a) = action {
            self.ui.add(k, r, Act::Recorder(a.to_string()), true, false, enabled);
        }
        let hv = self.ui.hover_of(k);
        let rec = if recording { self.rec_t.value().max(0.0001) } else { 0.0 };
        cv.fill_rr(r, 8.0, mix(p.ctl, p.ctl_hover, hv));
        if recording || rec > 0.0 {
            cv.stroke_rr(r.inset(1.0), 7.0, 2.0, p.accent.fade(rec.max(if recording { 1.0 } else { 0.0 })));
        }
        let cap_st = Style::new(11.0, 500);
        let draw_caps = |cv: &glasskit::canvas::Canvas, caps: &[String], right: f32, tail: Option<&str>| {
            let mut x = right - 5.0;
            let mut cells: Vec<(String, f32)> = caps.iter().map(|c| (hotkey::cap_text(c).to_string(), 0.0)).collect();
            for c in cells.iter_mut() {
                c.1 = (cv.text_width(&c.0, cap_st) + 14.0).max(22.0);
            }
            if let Some(t) = tail {
                x -= cv.text_width(t, Style::new(12.0, 400)) + 2.0;
                cv.text(Rect::new(x + 2.0, r.y, 40.0, r.h), t, Style::new(12.0, 400), REC_TEXT);
            }
            for (t, w) in cells.iter().rev() {
                let cr = Rect::new(x - w, r.cy() - 11.0, *w, 22.0);
                cv.fill_rr(cr, 6.0, if recording { p.key_cap_rec } else { p.key_cap });
                cv.text(cr, t, cap_st.center(), p.text);
                x -= w + 4.0;
            }
        };
        if recording {
            let live = self.rec_caps(action.unwrap_or(""));
            if live.is_empty() {
                cv.text(r, hotkey::PRESS_SHORTCUT.get(d.lang), Style::new(12.0, 400).center(), REC_TEXT);
            } else {
                let caps: Vec<String> = live.iter().map(|s| s.to_string()).collect();
                let w: f32 = caps.iter().map(|c| (cv.text_width(hotkey::cap_text(c), cap_st) + 14.0).max(22.0)).sum::<f32>() + 4.0 * caps.len() as f32 + 18.0;
                draw_caps(cv, &caps, r.cx() + w / 2.0, Some(" …"));
            }
        } else {
            let caps = key_caps(combo);
            if caps.is_empty() {
                cv.text(r, hotkey::NOT_ASSIGNED.get(d.lang), Style::new(12.0, 400).center(), Color::rgb8(0x8a, 0x88, 0x90));
            } else {
                draw_caps(cv, &caps, r.right(), None);
            }
        }
        if ring.0 && ring.1 {
            cv.focus_ring(r, 8.0, p.accent, 1.0);
        }
    }

    /// Save (and Remove when a key is stored) to the right of a secret field; returns the field's remaining rectangle.
    pub(super) fn paint_secret_buttons(&mut self, d: &Dc, rid: &'static str, r: Rect, enabled: bool) -> Rect {
        let (cv, th) = (d.cv, d.th);
        let mut right = r.right();
        let mut items: Vec<(&str, ButtonKind, Act, String)> = Vec::new();
        if self.data.has_secret(rid) && self.secret_removable(rid) {
            items.push(("rm", ButtonKind::Danger, Act::SecretRemove(rid.to_string()), pick(d.lang, "Удалить", "Remove").to_string()));
        }
        items.push(("save", ButtonKind::Normal, Act::SecretSave(rid.to_string()), pick(d.lang, "Сохранить", "Save").to_string()));
        for (tag, kind, act, label) in items {
            let k = key(&format!("sec:{rid}:{tag}"));
            let b = self.ui.button(k, || Button::new(&label).kind(kind));
            b.label = label;
            let w = b.natural_size(cv, th).0;
            let br = Rect::new(right - w, r.y, w, 32.0);
            b.state_mut().set_enabled(enabled);
            b.draw(cv, th, br);
            self.ui.add(k, br, act, true, true, enabled);
            right -= w + 8.0;
        }
        Rect::new(r.x, r.y, (right - r.x).max(60.0), r.h)
    }
}
