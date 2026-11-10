//! Custom widgets of the Modules page: "What is on Home" (chips plus a little schematic that relays itself as chips are switched) and
//! "Island tabs" (chips; an unlit tab leaves the tab strip but still opens by its hotkey and from the command bar).
use super::page::RowRt;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use crate::ui::tabs::a::home::slots::Slots;
use crate::ui::view::Cx;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::panel::a::home_mods::{self as hmod, On, PARTS};
use pill_ui_core::settings::tr::pick;
use pill_ui_core::shell::tabs::HIDEABLE;
use serde_json::{json, Value};

/// The schematic is the real tab at this fraction of its size.
const SCALE: f32 = 1.0 / 3.0;
/// Room reserved for it: the tallest tab (all three left parts and the strip, 330 px) plus the frame's padding.
const PREVIEW_H: f32 = 330.0 * SCALE + 16.0;

/// The captions above the chip groups: (ru, en, first part, one past the last).
const GROUPS: [(&str, &str, usize, usize); 3] = [("Слева", "On the left", 0, 3), ("Плитки", "Tiles", 3, 11), ("Внизу", "At the bottom", 11, 12)];

impl SettingsView {
    /// The parts that are on, from the settings the page shows (a missing key is the part's default).
    pub(super) fn home_on(&self) -> On {
        On::from_get(|k| match self.model.get(k) {
            Value::Null => None,
            v => Some(v),
        })
    }

    /// Titles of the tab chips come from the tab list of the island.
    fn tab_chip_title(&self, d: &Dc, id: &str) -> String {
        crate::ui::island::chrome::tab_title(id, d.lang == pill_ui_core::settings::tr::Lang::Ru)
    }

    pub(super) fn w_home(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let mut y = full.y + m.nh - 11.0 + 6.0;
        let on = self.home_on();
        for (ru, en, a, b) in GROUPS {
            if paint {
                d.cv.text(Rect::new(x0, y, w, 14.0), pick(d.lang, ru, en), Style::new(11.0, 600), d.p.dim);
            }
            y += 20.0;
            let mut x = x0;
            for (i, part) in PARTS.iter().enumerate().take(b).skip(a) {
                let name = format!("home:{}", part.key);
                let label = pick(d.lang, part.ru, part.en);
                let cw = self.chip(d, false, 0.0, 0.0, &name, label, &[], false, Act::None);
                if x > x0 && x + cw > x0 + w {
                    x = x0;
                    y += 38.0;
                }
                self.chip(d, paint, x, y, &name, label, &[], on.0[i], Act::HomeChip(i));
                x += cw + 8.0;
            }
            y += 30.0 + 12.0;
        }
        // the schematic: the Home tab at a third of its size, laid out by the very plan the island uses
        let box_w = hmod::W * SCALE + 16.0;
        if paint {
            let (cv, p) = (d.cv, d.p);
            let frame_h = self.data.home.height() * SCALE + 16.0;
            let frame = Rect::new(x0, y, box_w, frame_h.min(PREVIEW_H));
            cv.fill_rr(frame, 10.0, p.field);
            cv.stroke_rr(frame.inset(0.5), 9.5, 1.0, p.card_line);
            let (ox, oy) = (frame.x + 8.0, frame.y + 8.0);
            for i in 0..hmod::N {
                let (Some(rc), a) = (self.data.home.rect(i), self.data.home.alpha(i)) else { continue };
                if a <= 0.004 {
                    continue;
                }
                let g = self.data.home.grow(i);
                let rr = Rect::new(ox + rc.x * SCALE, oy + rc.y * SCALE, rc.w * SCALE, rc.h * SCALE);
                cv.with_opacity(a, |cv| {
                    cv.with_scale_about(rr.center(), g, |cv| {
                        let (fill, line) = match PARTS[i].kind {
                            hmod::Kind::Player => (p.accent_bg, p.accent_line),
                            hmod::Kind::Recent | hmod::Kind::Volume => (p.ctl_on, p.card_line),
                            _ => (p.ctl, p.card_line),
                        };
                        cv.fill_rr(rr, 3.0, fill);
                        cv.stroke_rr(rr.inset(0.5), 2.5, 1.0, line);
                        if i == hmod::STRIP {
                            for k in 0..8 {
                                cv.fill_rr(Rect::new(rr.x + 6.0 + k as f32 * 13.0, rr.cy() - 4.0, 8.0, 8.0), 2.0, p.faint);
                            }
                        } else if i == 0 {
                            cv.fill_rr(Rect::new(rr.x + 5.0, rr.y + 5.0, 30.0, 30.0), 3.0, p.accent_line);
                        }
                    });
                });
            }
        }
        y += PREVIEW_H + 14.0;
        y - full.y
    }

    pub(super) fn w_tabs(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let mut y = full.y + m.nh - 11.0 + 6.0;
        let hidden = self.hidden_tabs();
        let mut x = x0;
        for id in HIDEABLE {
            let name = format!("tab:{id}");
            let label = self.tab_chip_title(d, id);
            let cw = self.chip(d, false, 0.0, 0.0, &name, &label, &[], false, Act::None);
            if x > x0 && x + cw > x0 + w {
                x = x0;
                y += 38.0;
            }
            self.chip(d, paint, x, y, &name, &label, &[], !hidden.iter().any(|h| h == id), Act::TabChip(id));
            x += cw + 8.0;
        }
        y += 30.0 + 14.0;
        y - full.y
    }

    fn hidden_tabs(&self) -> Vec<String> {
        self.model.get("tabs.hidden").as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
    }

    /// A chip of the Home widget was clicked: flip that part.
    pub(super) fn home_chip(&mut self, i: usize, cx: &mut Cx) {
        let Some(part) = PARTS.get(i) else { return };
        let now = !self.home_on().0[i];
        if now {
            self.start_pop(key(&format!("chip:home:{}", part.key)));
        }
        if let Ok(calls) = self.model.try_set(part.key, json!(now)) {
            self.run(cx, calls);
        }
        self.sync_page();
    }

    /// A chip of the tab widget was clicked: flip that tab in or out of the strip.
    pub(super) fn tab_chip(&mut self, id: &'static str, cx: &mut Cx) {
        let mut hidden = self.hidden_tabs();
        let now_shown = hidden.iter().any(|h| h == id);
        if now_shown {
            hidden.retain(|h| h != id);
            self.start_pop(key(&format!("chip:tab:{id}")));
        } else {
            hidden.push(id.to_string());
        }
        if let Ok(calls) = self.model.try_set("tabs.hidden", json!(hidden)) {
            self.run(cx, calls);
        }
        self.sync_page();
    }

    /// Per frame (from `step_custom`): the schematic chases the plan of the parts that are on.
    pub(super) fn step_home_preview(&mut self, dt_ms: f64) -> bool {
        let plan = hmod::plan(&self.home_on());
        let slots: &mut Slots = &mut self.data.home;
        slots.set_plan(plan);
        slots.step(dt_ms)
    }
}
