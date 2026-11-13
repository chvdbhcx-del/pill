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
            let t = self.leave_t.at01();
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
        let e = self.enter_t.at01();
        let ea = self.enter_t.at();
        let spec = if self.wiz.active() { self.wiz.enter_dx() } else { (0.0, 8.0) };
        let mut page = std::mem::replace(&mut self.page, Page::empty());
        cv.with_opacity(e, |cv| {
            cv.with_offset((1.0 - ea) * spec.0, (1.0 - ea) * spec.1, |cv| {
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
    /// The "Advanced" header: a card-like line with a chevron that turns as the section opens, its name and what is inside.
    /// Returns the height it takes (the blocks of the section follow and grow out of the space below it).
    fn paint_adv_header(&mut self, d: &Dc, y: f32, vp: (f32, f32), open: f32, sub: &str, fid: u32) -> f32 {
        const H: f32 = 52.0;
        let (cv, p) = (d.cv, d.p);
        let r = Rect::new(0.0, y, CARD_W, H);
        let k = super::ui::key("adv:header");
        self.ui.add(k, r, super::ui::Act::Advanced, true, false, true);
        if r.bottom() >= vp.0 && r.y <= vp.1 {
            let (hv, pr) = (self.ui.hover_of(k), self.ui.press_of(k));
            let scale = 1.0 - 0.01 * pr.max(0.0);
            cv.with_scale_about(r.center(), scale, |cv| {
                cv.fill_rr(r, 12.0, p.card);
                cv.fill_rr(r, 12.0, p.item_hover.fade(hv));
                cv.stroke_rr(r.inset(0.5), 11.5, 1.0, p.card_line);
                let ang = (90.0 * open.clamp(0.0, 1.0)).to_radians();
                let c = (r.x + PAD_X + 3.0, r.cy());
                let (s, co) = ang.sin_cos();
                let pt = |x: f32, y: f32| (c.0 + x * co - y * s, c.1 + x * s + y * co);
                cv.line(pt(-3.0, -3.0), pt(3.0, 0.0), 1.5, p.dim);
                cv.line(pt(3.0, 0.0), pt(-3.0, 3.0), 1.5, p.dim);
                let x = r.x + PAD_X + 14.0;
                cv.text(Rect::new(x, r.y + 9.0, r.w - 2.0 * PAD_X - 14.0, 18.0), pill_ui_core::settings::schema::ADVANCED.get(d.lang), st_title(), p.text);
                cv.text(Rect::new(x, r.y + 27.0, r.w - 2.0 * PAD_X - 14.0, 16.0), sub, st_hint().ellipsis(), p.dim);
            });
            let fr = self.focus_ring_for(fid);
            if fr.0 && fr.1 {
                cv.focus_ring(r, 12.0, p.accent, 1.0);
            }
        }
        H + 8.0
    }

    fn paint_blocks(&mut self, d: &Dc, page: &mut Page, vp: (f32, f32), scroll: f32, cx0: f32, content: Rect) -> f32 {
        let cv = d.cv;
        let mut y = 24.0;
        let mut prev_h1 = false;
        // the blocks of the "Advanced" section scale with how open it is: `adv_left` of them still follow the header
        let mut adv_left = 0usize;
        let mut ak = 1.0f32;
        for bi in 0..page.blocks.len() {
            let top = y;
            let k = if adv_left > 0 {
                adv_left -= 1;
                ak
            } else {
                1.0
            };
            let kh = k.max(0.0);
            let ko = k.clamp(0.0, 1.0);
            let shut = kh < 0.002;
            let mut blk = std::mem::replace(&mut page.blocks[bi], Block::Spacer);
            match &mut blk {
                Block::H1(t) => {
                    y += 8.0;
                    if y + 24.0 >= vp.0 && y <= vp.1 {
                        cv.text(Rect::new(4.0, y, CARD_W - 8.0, 24.0), t, Style::new(20.0, 600), d.p.text);
                    }
                    y += 24.0 + 20.0;
                    prev_h1 = true;
                }
                Block::Cap(t) => {
                    if !shut {
                        if bi != 0 && !prev_h1 {
                            y += 24.0 * kh;
                        }
                        if y + 16.0 >= vp.0 && y <= vp.1 {
                            cv.with_opacity(ko, |cv| cv.text(Rect::new(4.0, y, CARD_W - 8.0, 16.0), &txt::upper(t), Style::new(11.0, 600).track(0.88), d.p.cap));
                        }
                        y += (16.0 + 8.0) * kh;
                        prev_h1 = false;
                    }
                }
                Block::Foot(t) | Block::Note(t) => {
                    if !shut {
                        let w = CARD_W - 8.0;
                        let h = txt::wrapped_h(cv, t, st_hint(), w, 16.0);
                        let hk = h * kh;
                        if y + hk >= vp.0 && y <= vp.1 {
                            cv.with_clip(Rect::new(0.0, y, CARD_W, hk), |cv| cv.with_opacity(ko, |cv| txt::para(cv, Rect::new(4.0, y, w, h), t, st_hint(), d.p.faint)));
                        }
                        y += hk + if matches!(blk, Block::Note(_)) { 8.0 } else { 4.0 } * kh;
                        prev_h1 = false;
                    }
                }
                Block::Card(rows) => {
                    let h = self.paint_card(d, rows, 0.0, y, vp, k);
                    if h > 0.0 {
                        y += h + 12.0 * kh;
                    }
                    prev_h1 = false;
                }
                Block::Adv(n) => {
                    let (t, sub, fid) = page.adv.as_ref().map(|a| (a.t.at(), a.sub.clone(), a.fid)).unwrap_or((1.0, String::new(), 0));
                    y += self.paint_adv_header(d, y, vp, t, &sub, fid);
                    adv_left = *n;
                    ak = t;
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
        page.laid_out = true;
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
