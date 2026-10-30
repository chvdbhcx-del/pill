//! Custom widgets, part 2: launcher shortcuts, mode profiles, weather city, Everything.
use super::fields::TextField;
use super::page::RowRt;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::controls::ButtonKind;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::flows::{self, Msg as FMsg};
use pill_ui_core::settings::hotkey;
use pill_ui_core::settings::tr::pick;

impl SettingsView {
    /// A text field owned by a custom widget (made on first use), drawn into `r`.
    pub(super) fn paint_cfield(&mut self, d: &Dc, name: &'static str, r: Rect, placeholder: &str) {
        let mut f = self.data.fields.remove(name).unwrap_or_else(|| TextField::new(false, &self.th));
        f.input.placeholder = placeholder.to_string();
        f.draw(d.cv, r, d.th);
        self.data.fields.insert(name, f);
    }

    pub(super) fn w_launcher(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let mut y = full.y + 12.0;
        y += self.para_at(d, paint, x0, y, w, hotkey::LAUNCHER_FOOTNOTE.get(d.lang), st_hint(), p.faint) + 12.0;
        if self.data.launcher.is_empty() {
            if paint {
                cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), p.row_line);
                cv.text(Rect::new(x0, y + 11.0, w, 18.0), hotkey::LAUNCHER_EMPTY_TITLE.get(d.lang), st_title(), p.text);
            }
            let h = self.para_at(d, paint, x0, y + 31.0, w, hotkey::LAUNCHER_EMPTY_HINT.get(d.lang), st_hint(), p.dim);
            return y + (31.0 + h + 11.0).max(56.0) - full.y;
        }
        for l in self.data.launcher.clone() {
            if paint {
                cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), p.row_line);
                cv.fill_rr(Rect::new(x0, y + 16.0, 24.0, 24.0), 6.0, p.ctl);
                cv.text(Rect::new(x0 + 36.0, y, w - 36.0 - 180.0, 56.0), &l.title, Style::new(13.0, 500).ellipsis(), p.text);
                let combo = self.model.combo_of(&l.action);
                let kw = keys_w(cv, &combo);
                let ring = self.focus_ring_for(r.fid);
                self.paint_keys(d, Rect::new(full.right() - PAD_X - kw, y + 12.0, kw, 32.0), &combo, Some(&l.action), ring, true);
            }
            y += 56.0;
        }
        y - full.y
    }

    pub(super) fn w_profiles(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let mut y = full.y + m.nh;
        let list = self.model.profiles();
        if list.is_empty() {
            y += self.para_at(d, paint, x0, y, w, flows::profiles_empty().get(d.lang), st_hint(), p.dim) + 12.0;
        }
        for (name, on) in list {
            if paint {
                cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), p.row_line);
                cv.text(Rect::new(x0, y + 11.0, w - 190.0, 18.0), &name, st_title(), p.text);
                cv.text(Rect::new(x0, y + 29.0, w - 190.0, 16.0), &flows::profile_enabled_line(d.lang, on), st_hint(), p.dim);
                let del = self.btn_at(d, true, &format!("prof:del:{name}"), pick(d.lang, "Удалить", "Delete"), ButtonKind::Danger, 0.0, Some(full.right() - PAD_X), y + 12.0, Act::ProfileDelete(name.clone()), true);
                self.btn_at(d, true, &format!("prof:apply:{name}"), pick(d.lang, "Применить", "Apply"), ButtonKind::Normal, 0.0, Some(del.x - 8.0), y + 12.0, Act::ProfileApply(name.clone()), true);
            }
            y += 56.0;
        }
        if paint {
            cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), p.row_line);
            let save = self.btn_at(d, true, "prof:save", pick(d.lang, "Сохранить", "Save"), ButtonKind::Primary, 0.0, Some(full.right() - PAD_X), y + 12.0, Act::ProfileSave, true);
            self.paint_cfield(d, "profile", Rect::new(x0, y + 12.0, save.x - 8.0 - x0, 32.0), pick(d.lang, "Название профиля", "Profile name"));
        }
        y + 56.0 - full.y
    }
}

impl SettingsView {
    pub(super) fn w_weather(&mut self, d: &Dc, r: &mut RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        r.status = Some(FMsg::plain(flows::weather_city_title(d.lang, &self.model.get_str("weather.city"))));
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let mut y = full.y + m.nh - 11.0 + 8.0;
        if paint {
            let b = self.btn_at(d, true, "city:search", pick(d.lang, "Найти", "Search"), ButtonKind::Normal, 0.0, Some(full.right() - PAD_X), y, Act::CitySearch, true);
            self.paint_cfield(d, "city", Rect::new(x0, y, b.x - 8.0 - x0, 32.0), pick(d.lang, "Название города", "City name"));
        }
        y += 32.0 + 4.0;
        for (i, c) in self.data.cities.clone().iter().enumerate() {
            if paint {
                let rc = Rect::new(x0, y, w, 32.0);
                let k = key(&format!("city:{i}"));
                self.ui.add(k, rc, Act::CityPick(i), true, false, true);
                cv.fill_rr(rc, 8.0, p.ctl.fade(self.ui.hover_of(k)));
                cv.text(Rect::new(rc.x + 12.0, rc.y, rc.w - 24.0, 32.0), &c.label, Style::new(13.0, 400).ellipsis(), p.item);
            }
            y += 32.0;
        }
        y + 12.0 - full.y
    }

    pub(super) fn w_everything(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let ev = self.data.ev.clone();
        let busy = self.data.job.as_deref().is_some_and(|j| j.starts_with("everything-"));
        let tw = w - 220.0;
        let text = ev.as_ref().map(|e| e.text.clone()).unwrap_or_default();
        let text_h = if text.is_empty() { 0.0 } else { 2.0 + super::txt::wrapped_h(cv, &text, st_hint(), tw, 16.0) };
        let mut nh = (11.0 + 18.0 + text_h + 11.0).max(56.0);
        if paint {
            cv.text(Rect::new(x0, full.y + 11.0, tw, 18.0), r.row.label.get(d.lang), st_title(), p.text);
            if let Some(e) = &ev {
                super::txt::para(cv, Rect::new(x0, full.y + 31.0, tw, text_h), &e.text, st_hint(), if e.ok { p.green } else { p.dim });
                let mut right = full.right() - PAD_X;
                let cy = full.y + (nh - 32.0) / 2.0;
                if e.show_run {
                    let b = self.btn_at(d, true, "ev:run", pick(d.lang, "Запустить установщик", "Run installer"), ButtonKind::Normal, 0.0, Some(right), cy, Act::EvRun, !busy);
                    right = b.x - 8.0;
                }
                if e.show_download {
                    self.btn_at(d, true, "ev:dl", pick(d.lang, "Скачать", "Download"), ButtonKind::Normal, 0.0, Some(right), cy, Act::EvDownload, !busy);
                }
            }
        }
        if busy {
            if paint {
                cv.fill_rect(Rect::new(full.x, full.y + nh, full.w, 1.0), p.row_line);
                self.job_box(d, x0, full.y + nh + 12.0, w);
            }
            nh += 12.0 + 52.0;
        }
        if let Some((id, m)) = self.data.job_result.clone() {
            if id.starts_with("everything-") && !busy {
                let h = 2.0 + super::txt::wrapped_h(cv, &m.text, st_hint(), w, 16.0);
                if paint {
                    cv.fill_rect(Rect::new(full.x, full.y + nh, full.w, 1.0), p.row_line);
                    super::txt::para(cv, Rect::new(x0, full.y + nh + 12.0, w, h), &m.text, st_hint(), msg_color(&p, m.kind));
                }
                nh += 12.0 + h + 12.0;
            }
        }
        nh
    }
}
