//! Layout, hit testing and drawing of the clipboard tab (the geometry is `tab-clipboard.css`: search 36 + trash 36, rows 52, footer 18).
use super::{Act, ClipboardView, Hit, ACT, BAR_H, FOOT_H, GAP, ROW_H};
use crate::ui::tabs::a::icons;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::{Pt, Rect};
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::clipboard::{Row, Tile};

/// The four regions of the tab.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    pub search: Rect,
    pub wipe: Rect,
    pub list: Rect,
    pub foot: Rect,
}

pub(crate) fn layout(r: Rect) -> Layout {
    let wipe = Rect::new(r.right() - BAR_H, r.y, BAR_H, BAR_H);
    let search = Rect::new(r.x, r.y, r.w - BAR_H - GAP, BAR_H);
    let foot = Rect::new(r.x, r.bottom() - FOOT_H, r.w, FOOT_H);
    let ly = r.y + BAR_H + GAP;
    let list = Rect::new(r.x, ly, r.w, (foot.y - 4.0 - ly).max(0.0));
    Layout { search, wipe, list, foot }
}

/// Row width: the list is 4 px wider than the content box (`margin-right: -4px`) with 8 px right padding.
fn row_w(list: Rect) -> f32 {
    list.w - 4.0
}

pub(crate) fn acts_of(kind: &str) -> &'static [Act] {
    match kind {
        "image" => &[Act::Pin, Act::Del],
        "files" => &[Act::Plain, Act::Pin, Act::Del],
        _ => &[Act::Plain, Act::Ai, Act::Pin, Act::Del],
    }
}

/// Button rectangles of a row at `r` (row-relative to the row's own rect): the buttons are flex children with the row's 12 px gap, 8 px padding.
pub(crate) fn btn_rects(kind: &str, r: Rect) -> Vec<(Act, Rect)> {
    let acts = acts_of(kind);
    let mut x = r.right() - 8.0;
    let mut out = Vec::new();
    for a in acts.iter().rev() {
        x -= ACT;
        out.push((*a, Rect::new(x, r.y + (ROW_H - ACT) / 2.0, ACT, ACT)));
        x -= 12.0;
    }
    out.reverse();
    out
}

/// (top, height) of every row in content coordinates. A row being deleted shrinks with its `Collapse` cue.
pub(crate) fn relayout(v: &mut ClipboardView) {
    let mut y = 0.0;
    v.lay.clear();
    for c in v.model.items() {
        let h = if v.collapsing.contains(&c.id) { ROW_H * (1.0 - v.base.player.sample(&format!("row:{}", c.id)).p.clamp(0.0, 1.0)) } else { ROW_H };
        v.lay.push((y, h));
        y += h;
    }
    let l = layout(v.rect);
    v.scroll.set_content_height(y, l.list.h);
}

pub(crate) fn hit(v: &mut ClipboardView, p: Pt) -> Option<Hit> {
    let l = layout(v.rect);
    if l.search.contains(p.0, p.1) {
        return Some(Hit::Search);
    }
    if l.wipe.contains(p.0, p.1) {
        return Some(Hit::Wipe);
    }
    if !l.list.contains(p.0, p.1) || p.0 >= l.list.x + row_w(l.list) {
        return None;
    }
    relayout(v);
    let cy = p.1 - l.list.y + v.scroll.offset();
    let i = v.lay.iter().position(|&(t, h)| cy >= t && cy < t + h)?;
    let rr = Rect::new(l.list.x, l.list.y - v.scroll.offset() + v.lay[i].0, row_w(l.list), ROW_H);
    let kind = &v.model.items()[i].kind;
    for (a, br) in btn_rects(kind, rr) {
        if br.contains(p.0, p.1) {
            return Some(Hit::Btn(i, a));
        }
    }
    Some(Hit::Row(i))
}

fn sub_color(t: &Theme) -> Color {
    if t.dark {
        Color::rgb8(0x72, 0x72, 0x72)
    } else {
        t.dim
    }
}

pub(crate) fn draw(v: &mut ClipboardView, cv: &Canvas, t: &Theme, rect: Rect) {
    v.rect = rect;
    v.base.rect = rect;
    let l = layout(rect);
    v.vm();
    let vm = v.vm.take().expect("vm");
    if v.input.placeholder != vm.search_placeholder {
        v.input.placeholder = vm.search_placeholder.clone();
    }
    v.drain_edit(cv, l.search);
    relayout(v);
    {
        let tops: std::collections::HashMap<i64, f32> = v.model.items().iter().zip(v.lay.iter()).map(|(c, &(top, _))| (c.id, top)).collect();
        v.tops = tops;
    }
    if !v.base.player.is_active("list") {
        v.flip_from.clear();
    }

    // ---- search + wipe
    v.input.draw(cv, l.search, t);
    icons::draw(cv, "search", Rect::new(l.search.x + 12.0, l.search.cy() - 8.0, 16.0, 16.0), t.dim, 1.7);
    draw_wipe(v, cv, t, l.wipe);

    // ---- list
    let vr = &*v;
    if vm.rows.is_empty() {
        draw_empty(vr, cv, t, l.list, vm.empty.as_deref().unwrap_or(""));
    } else {
        vr.scroll.draw(cv, l.list, t, |cv, content| {
            let off = vr.scroll.offset();
            let rw = row_w(l.list);
            for (i, row) in vm.rows.iter().enumerate() {
                let Some(&(top, h)) = vr.lay.get(i) else { break };
                if top + ROW_H < off - 40.0 || top > off + l.list.h + 40.0 {
                    continue;
                }
                draw_row(vr, cv, t, row, Rect::new(0.0, top, rw, ROW_H), h, content.w);
            }
        });
    }

    // ---- footer
    draw_foot(cv, t, l.foot, &vm);
    v.vm = Some(vm);
}

fn draw_wipe(v: &ClipboardView, cv: &Canvas, t: &Theme, r: Rect) {
    let hv = v.wipe_hover.value().clamp(0.0, 1.0);
    let arm = v.arm.value().clamp(0.0, 1.0);
    let pressed = if v.down_is(Hit::Wipe) { 1.0 - 0.06 * v.press.value() } else { 1.0 };
    cv.with_scale_about(r.center(), pressed, |cv| {
        cv.fill_rr(r, 12.0, t.card.mix(t.card_hover, hv));
        if arm > 0.003 {
            cv.fill_rr(r, 12.0, t.red_bg.fade(arm));
        }
        let c = t.icon.mix(t.text, hv).mix(t.red, arm);
        icons::draw(cv, "trash", r.centered(17.0, 17.0), c, 1.7);
    });
}

fn draw_empty(v: &ClipboardView, cv: &Canvas, t: &Theme, list: Rect, text: &str) {
    let s = v.base.player.sample("empty");
    let st = Style::new(13.0, 400).center().wrap();
    let w = list.w - 40.0;
    let (_, h) = cv.measure(text, st, w);
    let r = Rect::new(list.x + 20.0, list.cy() - h / 2.0 + s.dy, w, h + 4.0);
    cv.with_opacity(s.alpha, |cv| cv.text(r, text, st, t.dim));
}

fn tile_icon(cv: &Canvas, t: &Theme, row: &Row, r: Rect) {
    match &row.tile {
        Tile::Files => icons::draw(cv, "folder", r.centered(17.0, 17.0), t.dim, 1.6),
        Tile::Text => icons::draw(cv, "text", r.centered(18.0, 18.0), t.icon, 1.5),
        Tile::Image(_) => {}
    }
}

fn draw_row(v: &ClipboardView, cv: &Canvas, t: &Theme, row: &Row, r0: Rect, h: f32, _cw: f32) {
    let s = v.base.player.sample(&format!("row:{}", row.id));
    let hl = v.hl.get(&row.id).map_or(0.0, |x| x.value().clamp(0.0, 1.0));
    let (mut alpha, mut dy) = (s.alpha, s.dy - hl);
    if let Some(&old) = v.flip_from.get(&row.id) {
        let lp = v.base.player.sample("list").p;
        dy += (old - r0.y) * (1.0 - lp);
    } else if !v.flip_from.is_empty() && v.base.player.is_active("list") {
        let lp = v.base.player.sample("list").p.clamp(0.0, 1.0);
        alpha *= lp;
        dy += 4.0 * (1.0 - lp);
    }
    if alpha <= 0.003 || h < 1.0 {
        return;
    }
    let draw = |cv: &Canvas| {
        cv.with_offset(s.dx, dy, |cv| {
            cv.with_opacity(alpha.clamp(0.0, 1.0), |cv| {
                let r = r0;
                if hl > 0.003 {
                    cv.fill_rr(r, 12.0, t.raised.fade(hl));
                }
                let tile = Rect::new(r.x + 12.0, r.y + (ROW_H - 32.0) / 2.0, 32.0, 32.0);
                cv.fill_rr(tile, 8.0, t.card.mix(t.raised2, hl));
                let key = format!("clip:{}", row.id);
                if matches!(row.tile, Tile::Image(_)) {
                    v.base.images.draw(cv, &key, tile, 8.0, 1.0, crate::ui::tabs::a::draw::Fit::Cover);
                } else {
                    tile_icon(cv, t, row, tile);
                }
                let btns = btn_rects(kind_of(row), r);
                let left = btns.first().map_or(r.right() - 8.0, |b| b.1.x) - 12.0;
                let bx = tile.right() + 12.0;
                let bw = (left - bx).max(0.0);
                cv.text(Rect::new(bx, r.y + 9.0, bw, 18.0), &row.title, Style::new(13.0, 400).ellipsis(), t.text);
                draw_sub(cv, t, row, Rect::new(bx, r.y + 29.0, bw, 14.0));
                if hl > 0.01 {
                    cv.with_opacity(hl, |cv| {
                        for (a, br) in &btns {
                            draw_btn(v, cv, t, row, *a, *br);
                        }
                    });
                }
            })
        })
    };
    if h < ROW_H - 0.5 {
        cv.with_clip(Rect::new(r0.x, r0.y, r0.w, h), draw);
    } else {
        draw(cv);
    }
}

fn kind_of(row: &Row) -> &'static str {
    match row.tile {
        Tile::Image(_) => "image",
        Tile::Files => "files",
        Tile::Text => "text",
    }
}

fn draw_sub(cv: &Canvas, t: &Theme, row: &Row, r: Rect) {
    let st = Style::new(11.0, 500).ellipsis();
    let mut x = r.x;
    if row.pinned {
        icons::draw_markup(cv, "pin-fill", PIN_FILLED, Rect::new(x, r.cy() - 5.5, 11.0, 11.0), t.accent, 1.4);
        x += 15.0;
    }
    let mut s = String::new();
    if let Some(fc) = &row.file_count {
        s.push_str(fc);
        s.push_str(" · ");
    }
    s.push_str(&row.source);
    if !row.source.is_empty() {
        s.push_str(" · ");
    }
    s.push_str(&row.ago);
    cv.text(Rect::new(x, r.y, (r.right() - x).max(0.0), r.h), &s, st, sub_color(t));
}

/// `icons.js` `pin` with the fill on (the pinned look).
const PIN_FILLED: &str = r#"<path d="M12 17v5"/><path fill="currentColor" d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#;

fn draw_btn(v: &ClipboardView, cv: &Canvas, t: &Theme, row: &Row, a: Act, r: Rect) {
    let idx = row.index;
    let over = v.hover == Some(Hit::Btn(idx, a));
    let hv = if over { v.hover_btn.value().clamp(0.0, 1.0) } else { 0.0 };
    let sc = if v.down_is(Hit::Btn(idx, a)) { 1.0 - 0.1 * v.press.value() } else { 1.0 };
    cv.with_scale_about(r.center(), sc, |cv| {
        if hv > 0.003 {
            cv.fill_rr(r, 8.0, t.raised2.fade(hv));
        }
        let mut c = t.dim.mix(t.text, hv);
        let ic = r.centered(14.0, 14.0);
        match a {
            Act::Plain => icons::draw(cv, "text", ic, c, 1.9),
            Act::Ai => icons::draw(cv, "sparkles", ic, c, 1.7),
            Act::Pin => {
                if row.pinned {
                    c = t.accent;
                    icons::draw_markup(cv, "pin-fill", PIN_FILLED, ic, c, 1.6);
                } else {
                    icons::draw(cv, "pin", ic, c, 1.6);
                }
            }
            Act::Del => icons::draw(cv, "x", ic, c, 1.9),
        }
    });
}

fn draw_foot(cv: &Canvas, t: &Theme, f: Rect, vm: &pill_ui_core::panel::a::clipboard::View) {
    let st = Style::new(11.0, 400);
    let keys_w = 12.0 + 4.0 + cv.text_width("Enter", st);
    let mut right = f.right() - 4.0;
    // updown + Enter
    icons::draw(cv, "updown", Rect::new(right - keys_w, f.cy() - 6.0, 12.0, 12.0), t.faint, 1.8);
    cv.text(Rect::new(right - keys_w + 16.0, f.y, keys_w - 16.0 + 2.0, f.h), "Enter", st, t.faint);
    right -= keys_w + 12.0;
    if let Some(chip) = &vm.private_chip {
        let cs = Style::new(11.0, 600);
        let w = cv.text_width(chip, cs) + 16.0;
        let r = Rect::new(right - w, f.y, w, 18.0);
        cv.fill_rr(r, 9.0, t.red_bg);
        cv.text(r, chip, cs.center(), t.red);
        right -= w + 12.0;
    }
    cv.text(Rect::new(f.x + 4.0, f.y, (right - f.x - 4.0).max(0.0), f.h), &vm.hint, st.ellipsis(), t.faint);
}
