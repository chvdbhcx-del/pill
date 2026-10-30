//! Title bar, sidebar (logo, search field, page list with the gliding indicator, Quit) and the search results popup.
use super::fields::TextField;
use super::pal::mix;
use super::rows::*;
use super::txt;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::color::Color;
use glasskit::ease::{curves, Tween};
use glasskit::geom::Rect;
use glasskit::scroll::{Axis, ScrollState, Scrollbar};
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::settings::nav::{self, NavItem};
use pill_ui_core::settings::schema::schema;
use pill_ui_core::settings::search::{self, SearchBox};
use pill_ui_core::settings::tr::Lang;
use std::ops::Range;

pub const FID_SEARCH: u32 = 1;
const FID_NAV: u32 = 1000;
pub const FID_QUIT: u32 = 1999;
const ITEM_H: f32 = 36.0;
const STRIDE: f32 = 38.0;
const SEP_H: f32 = 17.0;
const HEAD_H: f32 = 64.0;
const SEARCH_H: f32 = 36.0;
const FOOT_H: f32 = 84.0;

pub fn cap_label(c: &str) -> &str {
    pill_ui_core::settings::hotkey::cap_text(c)
}

pub struct Side {
    pub ind_y: Tween<f32>,
    pub cur: &'static str,
    pub scroll: ScrollState,
    pub bar: Scrollbar,
    pub ready: bool,
    pub rect: Rect,
    pub list: Rect,
}

impl Side {
    pub fn new() -> Side {
        Side { ind_y: Tween::at_rest(0.0), cur: "", scroll: ScrollState::new(), bar: Scrollbar::new(Axis::Vertical), ready: false, rect: Rect::default(), list: Rect::default() }
    }

    /// y of the item of page `id` inside the list (content space).
    pub fn item_y(id: &str) -> f32 {
        let mut y = 4.0;
        for n in nav::NAV.iter() {
            match n {
                NavItem::Page(p) => {
                    if *p == id {
                        return y;
                    }
                    y += STRIDE;
                }
                NavItem::Sep => y += SEP_H,
            }
        }
        4.0
    }

    pub fn list_h() -> f32 {
        Side::item_y("~") + 8.0
    }

    pub fn snap_to(&mut self, id: &'static str) {
        self.cur = id;
        self.ind_y = Tween::at_rest(Side::item_y(id));
        self.ready = true;
    }

    pub fn move_to(&mut self, id: &'static str, reduce: bool) {
        let to = Side::item_y(id);
        self.cur = id;
        if !self.ready || reduce {
            self.ind_y = Tween::at_rest(to);
            self.ready = true;
        } else {
            self.ind_y.retarget(to, 0.4, curves::SPRING_POP);
        }
    }

    pub fn fids(&self) -> Vec<u32> {
        let mut v: Vec<u32> = (0..nav::NAV.iter().filter(|n| matches!(n, NavItem::Page(_))).count() as u32).map(|i| FID_NAV + i).collect();
        v.push(FID_QUIT);
        v
    }
}

impl Default for Side {
    fn default() -> Self {
        Side::new()
    }
}

/// One line of the results list, copied out of the model when the hits change.
#[derive(Clone)]
pub struct ResRow {
    pub title: String,
    pub tag: String,
    pub ranges: Vec<Range<usize>>,
}

pub struct SearchUi {
    pub field: TextField,
    pub rows: Vec<ResRow>,
    /// seconds into the stagger
    pub stag: Tween<f32>,
    pub pop: Tween<f32>,
    pub rect: Rect,
}

impl SearchUi {
    pub fn new(th: &Theme) -> SearchUi {
        let mut field = TextField::new(false, th);
        field.input.style = Style::new(12.0, 400);
        field.input.placeholder = search::PLACEHOLDER.get(Lang::Ru).to_string();
        SearchUi { field, rows: Vec::new(), stag: Tween::at_rest(10.0), pop: Tween::at_rest(1.0), rect: Rect::default() }
    }

    pub fn refresh_lang(&mut self, lang: Lang) {
        self.field.input.placeholder = search::PLACEHOLDER.get(lang).to_string();
    }
}

impl SettingsView {
    pub(super) fn paint_title(&mut self, d: &Dc, title: Rect) {
        let (cv, p) = (d.cv, d.p);
        cv.fill_rect(title, p.title);
        let names = [("min", Act::CapMin), (if self.maximized { "rest" } else { "max" }, Act::CapMax), ("close", Act::CapClose)];
        let mut x = title.right() - 46.0 * 3.0;
        for (i, (g, act)) in names.into_iter().enumerate() {
            let r = Rect::new(x, title.y, 46.0, title.h);
            let k = key(&format!("cap:{i}"));
            self.ui.add(k, r, act, false, false, true);
            let hv = self.ui.hover_of(k);
            let close = i == 2;
            let hover_bg = if close { Color::rgb8(0xc4, 0x2b, 0x1c) } else if p.dark { Color::rgba(1.0, 1.0, 1.0, 0.08) } else { Color::rgba(0.0, 0.0, 0.0, 0.07) };
            cv.fill_rect(r, hover_bg.fade(hv));
            let ic = if close && hv > 0.0 { mix(p.text, Color::rgb8(255, 255, 255), hv) } else { p.text };
            self.icons.cap(cv, g, r.centered(10.0, 10.0), ic);
            x += 46.0;
        }
        self.caption = Rect::new(title.x, title.y, title.w - 138.0, title.h);
    }

    pub(super) fn paint_side(&mut self, d: &Dc, side: Rect) {
        let (cv, p) = (d.cv, d.p);
        self.side.rect = side;
        cv.fill_rect(side, p.side);
        cv.fill_rect(Rect::new(side.right() - 1.0, side.y, 1.0, side.h), p.side_line);
        let w = side.w - 1.0;
        // head: logo + name + version
        let lx = side.x + 22.0;
        let ly = side.y + 20.0;
        cv.fill_rr(Rect::new(lx, ly, 54.0, 32.0), 14.0, Color::rgb8(0, 0, 0));
        cv.fill_rect(Rect::new(lx, ly, 54.0, 14.0), Color::rgb8(0, 0, 0));
        cv.fill_rr(Rect::new(lx + 14.0, ly + 20.0, 26.0, 7.0), 4.0, Color::rgb8(0x2c, 0x2c, 0x30));
        let nx = lx + 54.0 + 12.0;
        cv.text(Rect::new(nx, ly, 100.0, 18.0), "Pill", Style::new(16.0, 600), p.text);
        let ver = nav::version_line(d.lang, &self.version);
        cv.text(Rect::new(nx, ly + 18.0, 110.0, 14.0), &ver, Style::new(11.0, 400), p.faint);
        // search field
        let sr = Rect::new(side.x + 10.0, side.y + HEAD_H, w - 20.0, 30.0);
        self.search.field.draw(cv, sr, d.th);
        self.search.rect = sr;
        // page list
        let list = Rect::new(side.x, side.y + HEAD_H + SEARCH_H, w, (side.h - HEAD_H - SEARCH_H - FOOT_H).max(0.0));
        self.side.list = list;
        self.side.scroll.set_metrics(Side::list_h(), list.h);
        let off = self.side.scroll.offset();
        cv.with_clip(list, |cv| {
            let ox = list.x + 10.0;
            let oy = list.y - off;
            let iw = w - 20.0;
            if self.side.ready {
                let ir = Rect::new(ox, oy + self.side.ind_y.value(), iw, ITEM_H);
                cv.fill_rr(ir, 10.0, p.item_on);
            }
            let mut y = 4.0;
            let mut idx = 0u32;
            for n in nav::NAV.iter() {
                match n {
                    NavItem::Sep => {
                        cv.fill_rect(Rect::new(ox + 12.0, oy + y + 8.0, iw - 24.0, 1.0), if p.dark { Color::rgb8(0x1c, 0x1b, 0x1f) } else { p.side_line });
                        y += SEP_H;
                    }
                    NavItem::Page(id) => {
                        let r = Rect::new(ox, oy + y, iw, ITEM_H);
                        let on = self.side.cur == *id;
                        let k = key(&format!("nav:{id}"));
                        self.ui.add(k, r, Act::Nav(id.to_string()), false, false, list.contains(r.cx(), r.cy().clamp(list.y, list.bottom() - 1.0)));
                        let hv = self.ui.hover_of(k);
                        let pr = self.ui.press_of(k);
                        let fr = self.focus_ring_for(FID_NAV + idx);
                        let icon = schema().page(id).map(|pg| pg.icon).unwrap_or("general");
                        let name = schema().page(id).map(|pg| pg.title.get(d.lang)).unwrap_or("");
                        self.paint_nav_item(d, r, icon, name, on, hv, pr, p.item, p.icon, fr);
                        y += STRIDE;
                        idx += 1;
                    }
                }
            }
        });
        self.side.bar.draw(cv, list, &self.side.scroll, d.th);
        // foot: note + Quit
        let fy = side.bottom() - FOOT_H;
        cv.fill_rect(Rect::new(side.x, fy, w, 1.0), p.side_line);
        cv.text(Rect::new(side.x + 22.0, fy + 12.0, w - 30.0, 16.0), nav::SAVED_NOTE.get(d.lang), Style::new(11.0, 400), p.faint);
        let qr = Rect::new(side.x + 10.0, fy + 12.0 + 24.0, w - 20.0, ITEM_H);
        let k = key("nav:quit");
        self.ui.add(k, qr, Act::Quit, false, false, true);
        let hv = self.ui.hover_of(k);
        let pr = self.ui.press_of(k);
        let fr = self.focus_ring_for(FID_QUIT);
        cv.fill_rr(qr, 10.0, p.red.alpha(0.10 * hv));
        self.paint_nav_item(d, qr, "power", nav::QUIT.get(d.lang), false, 0.0, pr, p.red, p.red, fr);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_nav_item(&mut self, d: &Dc, r: Rect, icon: &str, name: &str, on: bool, hv: f32, pr: f32, text: Color, ic: Color, fr: (bool, bool)) {
        let (cv, p) = (d.cv, d.p);
        let scale = 1.0 - 0.02 * pr.max(0.0);
        cv.with_scale_about(r.center(), scale, |cv| {
            if !on && hv > 0.0 && text != p.red {
                cv.fill_rr(r, 10.0, p.item_hover.fade(hv));
            }
            let c_text = if on { p.text } else { text };
            let c_icon = if on { p.text } else { ic };
            self.icons.side(cv, icon, Rect::new(r.x + 12.0, r.cy() - 8.0, 16.0, 16.0), c_icon);
            cv.text(Rect::new(r.x + 12.0 + 16.0 + 12.0, r.y, r.w - 52.0, r.h), name, Style::new(13.0, 400).ellipsis(), c_text);
        });
        if fr.0 && fr.1 {
            cv.focus_ring(r, 10.0, p.accent, 1.0);
        }
    }
}

impl SettingsView {
    pub(super) fn paint_search_results(&mut self, d: &Dc) {
        if !self.model.search.open {
            return;
        }
        let (cv, p) = (d.cv, d.p);
        let sr = self.search.rect;
        let n = self.search.rows.len().max(1);
        let item_h = 42.0;
        let h = if self.search.rows.is_empty() { 4.0 + 30.0 + 4.0 } else { (4.0 + n as f32 * item_h + 4.0).min(320.0) };
        let r = Rect::new(sr.x, sr.bottom() + 4.0, sr.w, h);
        let pop = self.search.pop.value().clamp(0.0, 1.2);
        let seconds = self.search.stag.value();
        cv.with_opacity(pop.min(1.0), |cv| {
            cv.with_scale_about((r.cx(), r.y), 0.97 + 0.03 * pop, |cv| {
                cv.with_offset(0.0, (1.0 - pop) * -6.0, |cv| {
                    cv.shadow_rr(r, 10.0, 30.0, 10.0, p.shadow);
                    cv.fill_rr(r, 10.0, p.card);
                    cv.stroke_rr(r.inset(0.5), 9.5, 1.0, p.card_line);
                    if self.search.rows.is_empty() {
                        cv.text(Rect::new(r.x + 12.0, r.y + 4.0, r.w - 24.0, 30.0), search::NOTHING_FOUND.get(d.lang), Style::new(11.0, 400), p.dim);
                        return;
                    }
                    let active = self.model.search.active;
                    cv.with_clip(r, |cv| {
                        for (i, row) in self.search.rows.clone().iter().enumerate() {
                            let k = (seconds - i as f32 * 0.026) / 0.3;
                            let k = curves::EASE_OUT.at(k.clamp(0.0, 1.0));
                            let ir = Rect::new(r.x + 4.0, r.y + 4.0 + i as f32 * item_h + (1.0 - k) * 6.0, r.w - 8.0, item_h);
                            let kk = key(&format!("sres:{i}"));
                            self.ui.add(kk, ir, Act::SearchPick(i), false, false, true);
                            let hv = self.ui.hover_of(kk);
                            cv.with_opacity(k, |cv| {
                                let on = active == Some(i) || hv > 0.0;
                                if on {
                                    cv.fill_rr(ir, 8.0, p.ctl);
                                }
                                txt::highlighted(cv, Rect::new(ir.x + 8.0, ir.y + 6.0, ir.w - 16.0, 16.0), &row.title, Style::new(12.0, 500), p.text, &row.ranges, p.accent, 700);
                                txt::line(cv, Rect::new(ir.x + 8.0, ir.y + 22.0, ir.w - 16.0, 14.0), &row.tag, Style::new(11.0, 400), p.dim);
                            });
                        }
                    });
                });
            });
        });
    }

    pub(super) fn search_close_silently(&mut self) {
        let _ = self.model.search.close(false);
        self.search.rows.clear();
    }

    /// Copies the model's hits into display rows (title, tag, highlight ranges).
    pub(super) fn refresh_search_rows(&mut self, fresh: bool) {
        let lang = self.lang();
        let query = self.model.search.query.clone();
        let hits = self.model.search.hits.clone();
        let entries = self.model.search_index().to_vec();
        self.search.rows = hits
            .iter()
            .filter_map(|h| entries.get(h.index).map(|e| ResRow { title: e.title.clone(), tag: SearchBox::result_tag(e, lang), ranges: search::highlight_ranges(&e.title, &query) }))
            .collect();
        if fresh {
            let total = self.search.rows.len() as f32 * 0.026 + 0.3;
            self.search.stag = if self.reduce() { Tween::at_rest(total + 1.0) } else { Tween::new(0.0, total + 1.0, total + 1.0, glasskit::ease::Ease::Linear) };
            self.search.pop = Tween::new(0.0, 1.0, if self.reduce() { 0.0 } else { 0.3 }, curves::SPRING_POP);
        }
    }
}
