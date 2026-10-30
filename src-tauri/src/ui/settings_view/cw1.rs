//! Custom widgets, part 1: provider chips (AI, voice), reaction chips, settings backup, the "what is new" disclosure.
use super::pal::mix;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::controls::ButtonKind;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::pickers::{REACTIONS, REACTION_LABEL};
use pill_ui_core::settings::providers::{self, Provider};
use pill_ui_core::settings::whatsnew;

const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

impl SettingsView {
    pub(super) fn w_providers(&mut self, d: &Dc, ai: bool, full: Rect, paint: bool) -> f32 {
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let list: &[Provider] = if ai { &providers::AI[..] } else { &providers::VOICE[..] };
        let cur = if ai { providers::ai_current(&self.model.get_str("ai.provider"), &self.model.get_str("ai.baseUrl")) } else { providers::voice_current(&self.model.get_str("voice.provider")) };
        let (mut x, mut y) = (x0, full.y + 12.0);
        for pr in list {
            let mut badges: Vec<(String, bool)> = Vec::new();
            if pr.free {
                badges.push((providers::BADGE_FREE.get(d.lang).to_string(), false));
            }
            if !pr.tag.is_blank() {
                badges.push((pr.tag.get(d.lang).to_string(), false));
            }
            let name = pr.name.get(d.lang);
            let id = format!("{}:{}", if ai { "ai" } else { "voice" }, pr.id);
            let kind = if ai { "ai" } else { "voice" };
            let cw = self.chip(d, false, 0.0, 0.0, &id, name, &badges, false, Act::None);
            if x > x0 && x + cw > x0 + w {
                x = x0;
                y += 38.0;
            }
            self.chip(d, paint, x, y, &id, name, &badges, pr.id == cur, Act::Provider(kind, pr.id.to_string()));
            x += cw + 8.0;
        }
        y += 30.0 + 12.0;
        let sel = list.iter().find(|p| p.id == cur).unwrap_or(&list[list.len() - 1]);
        y += self.para_at(d, paint, x0, y, w, sel.help.get(d.lang), st_hint(), d.p.dim);
        if !sel.key_url.is_empty() {
            let label = providers::GET_KEY.get(d.lang);
            let lw = d.cv.text_width(label, st_hint());
            let r = Rect::new(x0, y, lw + 2.0, 16.0);
            if paint {
                let k = key(&format!("link:{}", sel.key_url));
                self.ui.add(k, r, Act::OpenLink(sel.key_url.to_string()), true, false, true);
                d.cv.text(r, label, st_hint(), mix(d.p.link, d.p.text, self.ui.hover_of(k) * 0.4));
            }
            y += 16.0;
        }
        if sel.region {
            let wtxt = if ai { providers::AI_REGION_WARNING } else { providers::VOICE_REGION_WARNING };
            y += 8.0;
            y += self.para_at(d, paint, x0, y, w, wtxt.get(d.lang), st_hint(), d.p.amber);
        }
        y + 12.0 - full.y
    }

    pub(super) fn w_reactions(&mut self, d: &Dc, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let (mut x, mut y) = (x0, full.y + 12.0);
        let lab = REACTION_LABEL.get(d.lang);
        let lw = cv.text_width(lab, Style::new(12.0, 400));
        if paint {
            cv.text(Rect::new(x, y, lw + 2.0, 24.0), lab, Style::new(12.0, 400), p.dim);
        }
        x += lw + 4.0 + 4.0;
        for r in REACTIONS.iter() {
            let name = r.name.get(d.lang);
            let tw = cv.text_width(name, Style::new(12.0, 400));
            let cw = tw + 20.0;
            if x + cw > x0 + w && x > x0 {
                x = x0;
                y += 28.0;
            }
            if paint {
                let rc = Rect::new(x, y, cw, 24.0);
                let k = key(&format!("rchip:{}", r.id));
                self.ui.add(k, rc, Act::Reaction(r.id), true, false, true);
                let hv = self.ui.hover_of(k);
                let on = self.model.reaction.current() == r.id;
                if on {
                    cv.fill_rr(rc, 8.0, p.ctl_on);
                } else {
                    cv.fill_rr(rc, 8.0, p.ctl.fade(hv));
                }
                cv.text(rc, name, Style::new(12.0, 400).center(), if on { p.text } else { mix(p.text_ctl, p.text, hv) });
            }
            x += cw + 4.0;
        }
        y + 24.0 + 12.0 - full.y
    }

    pub(super) fn w_backup(&mut self, d: &Dc, r: &super::page::RowRt, full: Rect, paint: bool) -> f32 {
        let m = self.row_metrics_plain(d, r, full.w - 2.0 * PAD_X - 190.0);
        if paint {
            self.paint_label(d, r, &m, full.x + PAD_X, full.y);
            let y = full.y + (m.nh - 32.0) / 2.0;
            let imp = self.btn_at(d, true, "backup:import", pill_ui_core::settings::tr::pick(d.lang, "Импорт", "Import"), ButtonKind::Normal, 0.0, Some(full.right() - PAD_X), y, Act::Import, true);
            self.btn_at(d, true, "backup:export", pill_ui_core::settings::tr::pick(d.lang, "Экспорт", "Export"), ButtonKind::Normal, 0.0, Some(imp.x - 8.0), y, Act::Export, true);
        }
        m.nh
    }

    pub(super) fn w_whatsnew(&mut self, d: &Dc, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let open = self.data.whatsnew.value().clamp(0.0, 1.0);
        let mut notes: Vec<String> = whatsnew::PAGE_NOTES.iter().map(|t| t.get(d.lang).to_string()).collect();
        if let Some(n) = whatsnew::notes_for(CHANGELOG, &self.version) {
            notes.extend(n);
        }
        let w = full.w - 2.0 * PAD_X - 22.0;
        let heights: Vec<f32> = notes.iter().map(|n| super::txt::wrapped_h(cv, n, Style::new(12.0, 400), w, 18.0)).collect();
        let body: f32 = heights.iter().sum::<f32>() + 16.0;
        if paint {
            let sr = Rect::new(full.x, full.y, full.w, 56.0);
            let k = key("whatsnew:summary");
            self.ui.add(k, sr, Act::Disclosure, true, false, true);
            let ang = (-45.0 + 90.0 * open).to_radians();
            let c = (full.x + PAD_X + 3.0, full.cy().min(full.y + 28.0));
            let (s, co) = ang.sin_cos();
            let pt = |x: f32, y: f32| (c.0 + x * co - y * s, c.1 + x * s + y * co);
            cv.line(pt(-3.0, -3.0), pt(3.0, 0.0), 1.5, p.dim);
            cv.line(pt(3.0, 0.0), pt(-3.0, 3.0), 1.5, p.dim);
            cv.text(Rect::new(full.x + PAD_X + 14.0, full.y, 300.0, 56.0), pill_ui_core::settings::tr::pick(d.lang, "Что нового", "What is new"), Style::new(13.0, 500), p.text);
            let mut y = full.y + 56.0;
            cv.with_clip(Rect::new(full.x, y, full.w, body * open), |cv| {
                for (i, n) in notes.iter().enumerate() {
                    cv.fill_circle((full.x + PAD_X + 6.0, y + 9.0), 1.8, p.dim);
                    super::txt::para(cv, Rect::new(full.x + PAD_X + 22.0, y, w, heights[i]), n, Style::new(12.0, 400), p.dim);
                    y += heights[i];
                }
            });
        }
        56.0 + body * open
    }
}
