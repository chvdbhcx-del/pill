//! The page area: lays the blocks out in the 532 px column, paints them with the enter / exit animation, scrolls.
use super::page::*;
use super::paint::{col_x, Parts};
use super::rows::*;
use super::txt;
use super::SettingsView;
use glasskit::geom::Rect;
use glasskit::text::Style;

fn dc2<'a, 'b: 'a>(cv: &'a glasskit::canvas::Canvas<'a>, d: &Dc<'b>) -> Dc<'a> {
    Dc { cv, th: d.th, p: d.p, lang: d.lang, now_ms: d.now_ms, reduced: d.reduced }
}

impl SettingsView {
    pub(super) fn paint_content(&mut self, d: &Dc, parts: &Parts) {
        let content = parts.content;
        let cv = d.cv;
        if let Some(mut old) = self.leaving.take() {
            let t = self.leave_t.value().clamp(0.0, 1.0);
            let n = self.ui.hits.len();
            cv.with_opacity(1.0 - t, |cv| {
                cv.with_offset(-10.0 * t, 0.0, |cv| {
                    let d2 = dc2(cv, d);
                    self.paint_page(&d2, &mut old, content, false);
                });
            });
            self.ui.hits.truncate(n);
            self.leaving = Some(old);
            return;
        }
        let e = self.enter_t.value().clamp(0.0, 1.0);
        let spec = if self.wiz.active() { self.wiz.enter_dx() } else { (0.0, 8.0) };
        let mut page = std::mem::replace(&mut self.page, Page::empty());
        cv.with_opacity(e, |cv| {
            cv.with_offset((1.0 - e) * spec.0, (1.0 - e) * spec.1, |cv| {
                let d2 = dc2(cv, d);
                self.paint_page(&d2, &mut page, content, true);
            });
        });
        self.page = page;
    }

    fn paint_page(&mut self, d: &Dc, page: &mut Page, content: Rect, live: bool) {
        let cv = d.cv;
        let cx0 = col_x(content.w);
        let scroll = page.scroll.offset();
        let origin = (content.x + cx0, content.y - scroll);
        if live {
            self.ui.page_origin = origin;
            self.ui.page_clip = content;
        }
        let vp = (scroll, scroll + content.h);
        page.rects.clear();
        let mut y = 24.0;
        cv.with_clip(content, |cv| {
            cv.with_offset(origin.0, origin.1, |cv| {
                let d3 = dc2(cv, d);
                y = self.paint_blocks(&d3, page, vp, scroll, cx0, content);
            });
        });
        let total = y + 40.0;
        if live && ((page.content_h - total).abs() > 0.5 || page.scroll.viewport() != content.h) {
            page.content_h = total;
            page.scroll.set_metrics(total, content.h);
        }
        if live {
            page.bar.draw(cv, content, &page.scroll, d.th);
        }
    }
}

impl SettingsView {
    fn paint_blocks(&mut self, d: &Dc, page: &mut Page, vp: (f32, f32), scroll: f32, cx0: f32, content: Rect) -> f32 {
        let cv = d.cv;
        let mut y = 24.0;
        let mut prev_h1 = false;
        for bi in 0..page.blocks.len() {
            let top = y;
            let mut blk = std::mem::replace(&mut page.blocks[bi], Block::Spacer);
            match &mut blk {
                Block::H1(t) => {
                    y += 8.0;
                    if y + 24.0 >= vp.0 && y <= vp.1 {
                        cv.text(Rect::new(4.0, y, CARD_W - 8.0, 24.0), t, Style::new(20.0, 600), d.p.text);
                    }
                    y += 24.0 + 16.0;
                    prev_h1 = true;
                }
                Block::Cap(t) => {
                    if bi != 0 && !prev_h1 {
                        y += 24.0;
                    }
                    if y + 16.0 >= vp.0 && y <= vp.1 {
                        cv.text(Rect::new(4.0, y, CARD_W - 8.0, 16.0), &txt::upper(t), Style::new(11.0, 600).track(0.88), d.p.cap);
                    }
                    y += 16.0 + 8.0;
                    prev_h1 = false;
                }
                Block::Foot(t) | Block::Note(t) => {
                    let w = CARD_W - 8.0;
                    let h = txt::wrapped_h(cv, t, st_hint(), w, 16.0);
                    if y + h >= vp.0 && y <= vp.1 {
                        txt::para(cv, Rect::new(4.0, y, w, h), t, st_hint(), d.p.faint);
                    }
                    y += h + if matches!(blk, Block::Note(_)) { 8.0 } else { 4.0 };
                    prev_h1 = false;
                }
                Block::Card(rows) => {
                    let h = self.paint_card(d, rows, 0.0, y, vp);
                    if h > 0.0 {
                        y += h + 12.0;
                    }
                    prev_h1 = false;
                }
                Block::Pets(f) => {
                    y += self.paint_pets(d, *f, y, vp);
                    prev_h1 = false;
                }
                Block::Chars(f) => {
                    y += self.paint_chars(d, *f, y, vp);
                    prev_h1 = false;
                }
                Block::WizTop => {
                    y += self.paint_wiz_top(d, y);
                    prev_h1 = false;
                }
                Block::WizNav => {
                    y += self.paint_wiz_nav(d, y);
                    prev_h1 = false;
                }
                Block::Spacer => {}
            }
            page.rects.push(Rect::new(0.0, top, CARD_W, y - top));
            page.blocks[bi] = blk;
        }
        let work = Rect::new(-cx0, scroll, content.w, content.h);
        for r in page.rows_mut() {
            if let Ctl::Select(s) = &mut r.ctl {
                s.set_work_area(work);
                if s.popup_visible() {
                    s.draw_popup(cv, d.th);
                }
            }
        }
        y
    }
}
