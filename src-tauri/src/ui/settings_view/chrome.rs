//! Title bar, sidebar (logo, search field, page list with the gliding indicator, Quit) and the search results popup.
use super::fields::TextField;
use super::pal::mix;
use super::rows::*;
use super::txt;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::color::Color;
use super::flow::{from_to, go, tick};
use crate::ui::liquid::{Feel, Flow};
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
/// One function in the list: its box and the step to the next one.
pub const ITEM_H: f32 = 27.0;
const STRIDE: f32 = 28.0;
/// A group heading: the first one hugs the top, the others have a gap above.
const GROUP_H0: f32 = 22.0;
const GROUP_H: f32 = 27.0;
const HEAD_H: f32 = 58.0;
/// Left edge of an item's icon and of a group heading inside the list item; the icon box (a 24 grid drawn at 16 px) and the gap to the name.
const NAV_X: f32 = 14.0;
const NAV_ICON: f32 = 16.0;
const SEARCH_H: f32 = 36.0;
const FOOT_H: f32 = 70.0;

pub fn cap_label(c: &str) -> &str {
    pill_ui_core::settings::hotkey::cap_text(c)
}

pub struct Side {
    pub ind_y: Flow,
    pub cur: &'static str,
    pub scroll: ScrollState,
    pub bar: Scrollbar,
    pub ready: bool,
    pub rect: Rect,
    pub list: Rect,
    /// The selected item must be scrolled into view at the next paint (0 no, 1 at once, 2 smoothly): the list is not measured yet when a page is chosen.
    pub reveal: u8,
    /// The height of the list window at the last paint: when the window is resized the selected item is brought back into view.
    pub view_h: f32,
}

/// Height of the heading at `i` of `NAV`: the first entry sits at the top of the list.
fn head_h(i: usize) -> f32 {
    if i == 0 {
        GROUP_H0
    } else {
        GROUP_H
    }
}

impl Side {
    pub fn new() -> Side {
        Side { ind_y: Flow::px(0.0, Feel::FLOW), cur: "", scroll: ScrollState::new(), bar: Scrollbar::new(Axis::Vertical), ready: false, rect: Rect::default(), list: Rect::default(), reveal: 0, view_h: 0.0 }
    }

    /// y of the item of page `id` inside the list (content space).
    pub fn item_y(id: &str) -> f32 {
        let mut y = 4.0;
        for (i, n) in nav::NAV.iter().enumerate() {
            match n {
                NavItem::Page(p) => {
                    if *p == id {
                        return y;
                    }
                    y += STRIDE;
                }
                NavItem::Head(_) => y += head_h(i),
            }
        }
        4.0
    }

    /// Total height of the page list (4 px top padding, headings and items, 8 px bottom padding). `item_y` of an unknown id is the top, so
    /// the sum is made here: this value is the content height of the list's `ScrollState`.
    pub fn list_h() -> f32 {
        let body: f32 = nav::NAV.iter().enumerate().map(|(i, n)| if matches!(n, NavItem::Page(_)) { STRIDE } else { head_h(i) }).sum();
        4.0 + body + 8.0
    }

    /// y of the item `id` relative to the list top, and its height: what a keyboard move must bring into view.
    pub fn item_span(id: &str) -> (f32, f32) {
        (Side::item_y(id), ITEM_H)
    }

    /// The scroll offset that shows item `id` inside a list window of `view` px that is now at `top` (None: already in view). A heading
    /// above the first item of a group comes along, so the group's name stays visible when its first page is chosen.
    pub fn reveal_offset(id: &str, top: f32, view: f32) -> Option<f32> {
        let (mut y, h) = Side::item_span(id);
        let first_of_group = nav::NAV.iter().position(|n| matches!(n, NavItem::Page(p) if *p == id)).is_some_and(|i| i > 0 && matches!(nav::NAV[i - 1], NavItem::Head(_)));
        if first_of_group {
            y -= GROUP_H - 4.0;
        }
        if y < top + 4.0 {
            Some((y - 4.0).max(0.0))
        } else if y + h > top + view - 8.0 {
            Some(y + h - view + 8.0)
        } else {
            None
        }
    }

    pub fn snap_to(&mut self, id: &'static str) {
        self.cur = id;
        self.ind_y.jump(Side::item_y(id));
        self.ready = true;
        self.reveal = 1;
    }

    pub fn move_to(&mut self, id: &'static str, reduce: bool) {
        let to = Side::item_y(id);
        self.cur = id;
        if !self.ready {
            self.ind_y.jump(to);
            self.ready = true;
            self.reveal = 1;
        } else {
            go(&mut self.ind_y, to, reduce);
            self.reveal = if reduce { 1 } else { 2 };
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

/// Seconds between two results coming in.
const STAGGER: f32 = 0.026;

pub struct SearchUi {
    pub field: TextField,
    /// the rows on show; they stay while the popup flows shut, so it can close instead of vanishing
    pub rows: Vec<ResRow>,
    /// per row: seconds still to wait, and how far it has come in (0..1)
    pub row_a: Vec<(f32, Flow)>,
    /// the popup: 0 = shut, 1 = open
    pub pop: Flow,
    /// its height in px
    pub hgt: Flow,
    pub rect: Rect,
}

impl SearchUi {
    pub fn new(th: &Theme) -> SearchUi {
        let mut field = TextField::new(false, th);
        field.input.style = Style::new(12.0, 400);
        field.input.placeholder = search::PLACEHOLDER.get(Lang::Ru).to_string();
        SearchUi { field, rows: Vec::new(), row_a: Vec::new(), pop: Flow::new(0.0, Feel::POP), hgt: Flow::px(38.0, Feel::FLOW), rect: Rect::default() }
    }

    pub fn refresh_lang(&mut self, lang: Lang) {
        self.field.input.placeholder = search::PLACEHOLDER.get(lang).to_string();
    }

    /// The popup flows shut; its rows stay until it has gone.
    pub fn close(&mut self, reduce: bool) {
        go(&mut self.pop, 0.0, reduce);
        if reduce {
            self.rows.clear();
            self.row_a.clear();
        }
    }

    /// The popup is on screen or on its way in / out.
    pub fn shown(&self, open: bool) -> bool {
        open || self.pop.at() > 0.01 || self.pop.moving()
    }

    /// Advances the popup, its height and the staggered rows; true while any moves.
    pub fn step(&mut self, dt: f32, open: bool) -> bool {
        let mut busy = tick(&mut self.pop, dt);
        busy |= tick(&mut self.hgt, dt);
        for (wait, f) in self.row_a.iter_mut() {
            if *wait > 0.0 {
                *wait -= dt;
                busy = true;
                if *wait <= 0.0 {
                    f.to(1.0);
                }
            } else {
                busy |= tick(f, dt);
            }
        }
        if !open && self.pop.target() == 0.0 && !self.pop.moving() && !self.rows.is_empty() {
            self.rows.clear();
            self.row_a.clear();
        }
        busy
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
        // the chosen function is scrolled into the window (once the list is measured; a first paint lands there at once); a window that was
        // resized also takes its selected function back into view
        let resized = (self.side.view_h - list.h).abs() > 0.5;
        if (self.side.reveal != 0 || resized) && list.h > 0.0 {
            if let Some(to) = Side::reveal_offset(self.side.cur, self.side.scroll.target(), list.h) {
                self.side.scroll.scroll_to(to.min(self.side.scroll.max_offset()), self.side.reveal == 2 && !d.reduced);
            }
            self.side.reveal = 0;
        }
        self.side.view_h = list.h;
        let off = self.side.scroll.offset();
        cv.with_clip(list, |cv| {
            let ox = list.x + 10.0;
            let oy = list.y - off;
            let iw = w - 20.0;
            if self.side.ready {
                let ir = Rect::new(ox, oy + self.side.ind_y.at(), iw, ITEM_H);
                cv.fill_rr(ir, 9.0, p.item_on);
                // the accent bar rides on the same glide: the selected function reads at a glance in dark and light
                cv.fill_rr(Rect::new(ir.x + 4.0, ir.y + 8.0, 3.0, ITEM_H - 16.0), 1.5, p.accent);
            }
            let mut y = 4.0;
            let mut idx = 0u32;
            for (i, n) in nav::NAV.iter().enumerate() {
                match n {
                    NavItem::Head(t) => {
                        let hh = head_h(i);
                        cv.text(Rect::new(ox + NAV_X, oy + y + hh - 20.0, iw - 20.0, 14.0), &txt::upper(t.get(d.lang)), Style::new(10.0, 600).track(0.8).ellipsis(), p.cap);
                        y += hh;
                    }
                    NavItem::Page(id) => {
                        let r = Rect::new(ox, oy + y, iw, ITEM_H);
                        let on = self.side.cur == *id;
                        let k = key(&format!("nav:{id}"));
                        self.ui.add(k, r, Act::Nav(id.to_string()), false, false, list.contains(r.cx(), r.cy().clamp(list.y, list.bottom() - 1.0)));
                        let hv = self.ui.hover_of(k);
                        let pr = self.ui.press_of(k);
                        let fr = self.focus_ring_for(FID_NAV + idx);
                        let pg = schema().page(id);
                        let name = pg.map(|pg| pg.title.get(d.lang)).unwrap_or("");
                        // how much of the item is selected, read off the indicator that glides to it (no state of its own): the icon and the
                        // name take the selected colours as the indicator arrives and let go as it leaves
                        let sel = if self.side.ready { 1.0 - ((self.side.ind_y.at() - y).abs() / STRIDE).min(1.0) } else { on as u8 as f32 };
                        self.paint_nav_item(d, r, pg.map_or("", |pg| pg.icon), name, on, sel, hv, pr, p.item, p.icon, fr);
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
        self.paint_nav_item(d, qr, "power", nav::QUIT.get(d.lang), false, 0.0, 0.0, pr, p.red, p.red, fr);
    }

    /// One line of the list: icon, name. Idle: the dim icon colour; hovered: the text colour; selected: the accent (`sel` 0..1 is how far the
    /// selection indicator has come to this item). All three are mixes of values that already flow, so nothing here sets a target.
    #[allow(clippy::too_many_arguments)]
    fn paint_nav_item(&mut self, d: &Dc, r: Rect, icon: &str, name: &str, on: bool, sel: f32, hv: f32, pr: f32, text: Color, ic: Color, fr: (bool, bool)) {
        let (cv, p) = (d.cv, d.p);
        let scale = 1.0 - 0.02 * pr.max(0.0);
        cv.with_scale_about(r.center(), scale, |cv| {
            if !on && hv > 0.0 && text != p.red {
                cv.fill_rr(r, 10.0, p.item_hover.fade(hv));
            }
            let quit = text == p.red;
            let c_text = if quit { text } else { mix(text, p.text, sel) };
            let c_icon = if quit { ic } else { mix(mix(ic, p.text, hv), p.accent, sel) };
            if !icon.is_empty() {
                self.icons.side(cv, icon, Rect::new(r.x + NAV_X, r.cy() - NAV_ICON / 2.0, NAV_ICON, NAV_ICON), c_icon);
            }
            let tx = r.x + NAV_X + NAV_ICON + 10.0;
            cv.text(Rect::new(tx, r.y, r.right() - tx - 6.0, r.h), name, Style::new(13.0, if on { 600 } else { 400 }).ellipsis(), c_text);
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
        let open = self.model.search.open;
        let item_h = 42.0;
        let r = Rect::new(sr.x, sr.bottom() + 4.0, sr.w, self.search.hgt.at().max(8.0));
        let pop = self.search.pop.at();
        cv.with_opacity(self.search.pop.at01(), |cv| {
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
                            let (k, rise) = self.search.row_a.get(i).map_or((1.0, 0.0), |(_, f)| (f.at01(), (1.0 - f.at()) * 6.0));
                            let ir = Rect::new(r.x + 4.0, r.y + 4.0 + i as f32 * item_h + rise, r.w - 8.0, item_h);
                            let kk = key(&format!("sres:{i}"));
                            if open {
                                self.ui.add(kk, ir, Act::SearchPick(i), false, false, true);
                            }
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
        self.search_shut();
    }

    /// The results popup flows shut (its rows stay until it is gone).
    pub(super) fn search_shut(&mut self) {
        let reduce = self.reduce();
        self.search.close(reduce);
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
        if !self.model.search.open {
            return;
        }
        let reduce = self.reduce();
        let n = self.search.rows.len();
        let h = if n == 0 { 4.0 + 30.0 + 4.0 } else { (4.0 + n as f32 * 42.0 + 4.0).min(320.0) };
        let from_shut = !self.search.pop.moving() && self.search.pop.at() < 0.01;
        if from_shut {
            // opening: every result comes in one after the other, the popup pops out of the field
            self.search.row_a.clear();
            self.search.hgt.jump(h);
            self.search.pop.jump(0.0);
        } else {
            go(&mut self.search.hgt, h, reduce);
        }
        go(&mut self.search.pop, 1.0, reduce);
        // typing on: rows that were already there stay, only the new ones stagger in
        let have = self.search.row_a.len().min(n);
        self.search.row_a.truncate(n);
        for i in have..n {
            let mut f = Flow::new(0.0, Feel::FLOW);
            let wait = if reduce { 0.0 } else { (i - have) as f32 * STAGGER };
            if wait <= 0.0 {
                from_to(&mut f, 0.0, 1.0, reduce);
            }
            self.search.row_a.push((wait, f));
        }
    }
}
