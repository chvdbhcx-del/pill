//! The frame: window background and border, title bar, sidebar, the page (blocks, scrolling, enter / exit animation), floating popups.
use super::page::*;
use super::rows::*;
use super::txt;
use super::SettingsView;
use crate::ui::view::Cx;
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;

pub const SIDE_W: f32 = 211.0;
pub const TITLE_H: f32 = 30.0;

/// The window split into its parts.
#[derive(Clone, Copy)]
pub struct Parts {
    pub inner: Rect,
    pub title: Rect,
    pub side: Option<Rect>,
    pub content: Rect,
}

pub fn parts(rect: Rect, maximized: bool, with_side: bool) -> Parts {
    let inner = if maximized { rect } else { rect.inset(1.0) };
    let (title, body) = inner.cut_top(TITLE_H);
    if with_side {
        let (side, content) = body.cut_left(SIDE_W);
        Parts { inner, title, side: Some(side), content }
    } else {
        Parts { inner, title, side: None, content: body }
    }
}

/// x of the 532 px column: `max(24px, round(down, (100% - 532px) / 2))`.
pub fn col_x(content_w: f32) -> f32 {
    (((content_w - CARD_W) / 2.0).floor()).max(24.0)
}

impl SettingsView {
    pub(super) fn paint(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx) {
        if theme.dark != self.th.dark || theme.accent != self.th.accent {
            self.th = *theme;
            self.pal = super::pal::pal(theme);
        } else {
            self.th = *theme;
        }
        self.reduced = cx.reduced_motion;
        self.now_ms = cx.now_ms;
        let size = (rect.w, rect.h);
        if size != self.size {
            self.size = size;
            self.caption_dirty = true;
        }
        let p = self.pal;
        cv.fill_rect(rect, p.bg);
        let d = Dc { cv, th: theme, p, lang: self.lang(), now_ms: cx.now_ms, reduced: self.reduce() };
        self.ui.begin();
        let wizard = self.wiz.active() || self.page.wizard;
        let parts = parts(rect, self.maximized, !wizard);
        let win_k = self.win_t.value().clamp(0.0, 1.0);
        cv.with_opacity(win_k, |cv| {
            cv.with_offset(0.0, (1.0 - win_k) * 14.0, |cv| {
                let d = Dc { cv, th: d.th, p: d.p, lang: d.lang, now_ms: d.now_ms, reduced: d.reduced };
                if self.loaded {
                    self.pump_fields(cv);
                    self.paint_content(&d, &parts);
                    if let Some(side) = parts.side {
                        self.paint_side(&d, side);
                    }
                }
                self.paint_title(&d, parts.title);
                if self.loaded && !wizard {
                    self.paint_search_results(&d);
                }
            });
        });
        if !self.maximized {
            cv.stroke_rr(rect.inset(0.5), 0.0, 1.0, p.win_border);
        }
    }

    /// Text fields can only run their events against a canvas: replay the queued ones now.
    fn pump_fields(&mut self, cv: &Canvas) {
        self.search.field.pump(cv);
        let mut any = false;
        for r in self.page.rows_mut() {
            if let Ctl::Field(f) = &mut r.ctl {
                f.tf.pump(cv);
                any |= f.tf.has_pending();
            }
        }
        any |= self.search.field.has_pending();
        self.pump_custom_fields(cv);
        if any {
            self.edits_due = true;
        }
    }
}
