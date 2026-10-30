//! The collapsed content of the island: the clock row (`renderClock`), a live row and an event card, as layout + painting.
//! All numbers are css px at 100 %; the caller scales the canvas by the island size and places the content area.
use super::gpu::{c8, Gpu};
use super::rows::{accent_color, layout_row, paint_row, Anim, Hits, Meter, RowLayout, ROW_H};
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::{Canvas, Color};
use pill_ui_core::live::model::{self as lm, ActionKind, CardBody, CardIcon, Tone};
use pill_ui_core::shell::cards::{CardPhase, CardView};
use pill_ui_core::shell::clock::{ClockItem, ClockRow, DigitRoll, SlotItem};
use pill_ui_core::shell::motion::{self as mo, AnimSpec};
use pill_ui_core::shell::types::LiveContent;
use std::collections::HashMap;

pub const TIME_STYLE: Style = Style::new(13.0, 600).mono().track(0.13);
const DATE_STYLE: Style = Style::new(12.0, 600);
const TEMP_STYLE: Style = Style::new(13.0, 600);
const BATT_STYLE: Style = Style::new(12.0, 600);
const COL_GAP: f32 = 16.0;
const SIDE_GAP: f32 = 12.0;

/// What a content needs from the model while measuring / painting.
pub struct Ctx<'a> {
    pub rows: &'a HashMap<String, lm::RowSpec>,
    pub cards: &'a HashMap<String, lm::CardSpec>,
    pub reduced: bool,
    pub ru: bool,
}

fn item_w(it: &ClockItem, m: &dyn Meter) -> f32 {
    match it {
        ClockItem::WxIcon { .. } => 20.0,
        ClockItem::Date(t) => m.w(t, DATE_STYLE),
        ClockItem::Battery { percent, .. } => 20.0 + 4.0 + m.w(&format!("{}%", percent.round() as i32), BATT_STYLE),
        ClockItem::Temp(t) => m.w(&format!("{t}°"), TEMP_STYLE),
        ClockItem::Slot(SlotItem::WxIcon(_)) => 20.0,
        ClockItem::Slot(SlotItem::Temp(t)) => m.w(&format!("{t}°"), TEMP_STYLE),
        ClockItem::Slot(SlotItem::Text(t)) => m.w(t, TEMP_STYLE),
        ClockItem::Slot(SlotItem::Spec(_)) => 0.0,
    }
}

fn col_w(items: &[ClockItem], m: &dyn Meter) -> f32 {
    items.iter().map(|i| item_w(i, m)).sum::<f32>() + SIDE_GAP * items.len().saturating_sub(1) as f32
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClockLayout {
    pub w: f32,
    pub left_w: f32,
    pub right_w: f32,
    pub time_w: f32,
    /// x of the time, of the right column (row-relative, padding included)
    pub time_x: f32,
    pub right_x: f32,
}

/// Three columns `[left | time | right]`, the side columns equal (`--cw`) so the time stays on the capsule axis.
pub fn layout_clock(row: &ClockRow, time: Option<&str>, m: &dyn Meter) -> ClockLayout {
    let (lw, rw) = (col_w(&row.left, m), col_w(&row.right, m));
    let cw = lw.max(rw).max(if row.eic { 30.0 } else { 0.0 }).ceil();
    let (lcol, rcol) = (if row.pcl { 0.0 } else { cw }, if row.pcr { 0.0 } else { cw });
    let time_w = if row.show_time { time.map_or(0.0, |t| m.w(t, TIME_STYLE)) } else { 0.0 };
    let mut x = 16.0;
    let mut gaps = 0;
    if !row.pcl {
        x += lcol;
        gaps += 1;
    }
    let time_x = x + if gaps > 0 { COL_GAP } else { 0.0 };
    let mut end = time_x + time_w;
    let right_x = end + if !row.pcr { COL_GAP } else { 0.0 };
    end = right_x + rcol;
    ClockLayout { w: end + 16.0, left_w: lcol, right_w: rcol, time_w, time_x, right_x }
}

fn ease(t: f32) -> f32 {
    mo::EASE_OUT.eval(t as f64) as f32
}

/// The clock row; `x0` = left edge, `cy` = vertical centre.
pub fn paint_clock(cv: &Canvas, th: &Theme, gpu: &mut Gpu, row: &ClockRow, time: Option<&str>, rolls: &[(DigitRoll, f64)], entering: &[(bool, usize, f64)], lay: &ClockLayout, x0: f32, cy: f32, reduced: bool) {
    let pop = |right: bool, i: usize| -> (f32, f32) {
        entering.iter().find(|e| e.0 == right && e.1 == i).map_or((1.0, 1.0), |e| {
            let a = mo::RESHAPE_POP.at(e.2, reduced);
            (a.scale as f32, if e.2 >= mo::RESHAPE_POP_FADE_MS { 1.0 } else { ease((e.2 / mo::RESHAPE_POP_FADE_MS) as f32) })
        })
    };
    // left column: right aligned against the time
    let mut x = x0 + 16.0 + lay.left_w;
    for (i, it) in row.left.iter().enumerate().rev() {
        let w = it_w_cached(it, cv);
        x -= w;
        let (s, a) = pop(false, i);
        paint_item(cv, th, gpu, it, Rect::new(x, cy - 10.0, w, 20.0), s, a);
        x -= SIDE_GAP;
    }
    // right column: left aligned against the time
    let mut x = x0 + lay.right_x;
    for (i, it) in row.right.iter().enumerate() {
        let w = it_w_cached(it, cv);
        let (s, a) = pop(true, i);
        paint_item(cv, th, gpu, it, Rect::new(x, cy - 10.0, w, 20.0), s, a);
        x += w + SIDE_GAP;
    }
    if let (true, Some(t)) = (row.show_time, time) {
        paint_time(cv, th, t, rolls, Rect::new(x0 + lay.time_x, cy - 9.0, lay.time_w, 18.0));
    }
}

struct CvMeter<'a, 'b>(&'a Canvas<'b>);
impl Meter for CvMeter<'_, '_> {
    fn w(&self, s: &str, st: Style) -> f32 {
        self.0.text_width(s, st)
    }
}
fn it_w_cached(it: &ClockItem, cv: &Canvas) -> f32 {
    item_w(it, &CvMeter(cv))
}

fn paint_time(cv: &Canvas, th: &Theme, time: &str, rolls: &[(DigitRoll, f64)], r: Rect) {
    let chars: Vec<char> = time.chars().collect();
    let mut x = r.x;
    for (i, ch) in chars.iter().enumerate() {
        let s = ch.to_string();
        let w = cv.text_width(&s, TIME_STYLE);
        let cell = Rect::new(x, r.y, w, r.h);
        match rolls.iter().find(|(d, _)| d.index == i) {
            Some((d, age)) => {
                let p = ease((*age / mo::DIGIT_ROLL_MS).clamp(0.0, 1.0) as f32);
                cv.with_clip(cell, |cv| {
                    cv.text(cell.offset(0.0, -r.h * p), &d.from.to_string(), TIME_STYLE, th.text.fade(1.0 - p));
                    cv.text(cell.offset(0.0, r.h * (1.0 - p)), &d.to.to_string(), TIME_STYLE, th.text.fade(p));
                });
            }
            None => cv.text(cell, &s, TIME_STYLE, th.text),
        }
        x += w;
    }
}

fn paint_item(cv: &Canvas, th: &Theme, gpu: &mut Gpu, it: &ClockItem, r: Rect, scale: f32, alpha: f32) {
    cv.with_opacity(alpha, |cv| {
        cv.with_scale_about(r.center(), scale, |cv| match it {
            ClockItem::WxIcon { id, .. } | ClockItem::Slot(SlotItem::WxIcon(id)) => gpu.wx_icon(cv, id, r),
            ClockItem::Date(t) => cv.text(Rect::new(r.x, r.cy() - 8.0, r.w, 16.0), t, DATE_STYLE, th.text.fade(0.6)),
            ClockItem::Temp(t) | ClockItem::Slot(SlotItem::Temp(t)) => cv.text(Rect::new(r.x, r.cy() - 9.0, r.w + 2.0, 18.0), &format!("{t}°"), TEMP_STYLE, th.text),
            ClockItem::Slot(SlotItem::Text(t)) => cv.text(Rect::new(r.x, r.cy() - 9.0, r.w + 2.0, 18.0), t, TEMP_STYLE, th.text),
            ClockItem::Slot(SlotItem::Spec(_)) => {}
            ClockItem::Battery { percent, low, charging } => {
                let c = if *low { th.red } else { th.text };
                cv.stroke_rr(Rect::new(r.x, r.cy() - 5.5, 18.0, 11.0), 3.5, 1.5, c);
                let fillw = ((18.0 - 3.0) * (*percent as f32 / 100.0).clamp(0.0, 1.0)).max(0.0);
                cv.fill_rr(Rect::new(r.x + 1.5, r.cy() - 4.0, fillw, 8.0), 1.5, if *charging { th.green } else { c });
                cv.fill_rr(Rect::new(r.x + 18.5, r.cy() - 2.0, 2.0, 4.0), 1.0, c);
                cv.text(Rect::new(r.x + 24.0, r.cy() - 8.0, r.w, 16.0), &format!("{}%", percent.round() as i32), BATT_STYLE, th.text);
            }
        })
    });
}

// ------------------------------------------------------------------------------------------------ cards

const TITLE: Style = Style::new(14.0, 600).ellipsis();
const SUB: Style = Style::new(12.0, 500).ellipsis();
const BTN: Style = Style::new(12.0, 600);
const MAX_W: f32 = 480.0;

#[derive(Clone, Debug)]
pub struct CardLayout {
    pub w: f32,
    pub h: f32,
    pub icon: Rect,
    pub text: Rect,
    pub buttons: Vec<Rect>,
}

pub fn layout_card(c: &lm::CardSpec, m: &dyn Meter) -> CardLayout {
    let (pad_x, pad_y, gap) = (20.0, 8.0, 12.0);
    let btn_ws: Vec<f32> = c.actions.iter().map(|a| (m.w(&a.label, BTN) + 32.0).max(32.0)).collect();
    let actions_w = if btn_ws.is_empty() { 0.0 } else { btn_ws.iter().sum::<f32>() + 8.0 * (btn_ws.len() - 1) as f32 + 8.0 };
    let icon_w = match (&c.icon, &c.body) {
        (_, CardBody::Notification { .. }) => 32.0,
        (None, CardBody::Color { .. }) => 40.0,
        (None, _) => 0.0,
        _ => 40.0,
    };
    let (tw, th_) = match &c.body {
        CardBody::Text { title, sub } => {
            let tw = m.w(title, TITLE).max(sub.as_deref().map_or(0.0, |s| m.w(s, SUB)));
            (tw, 18.0 + if sub.is_some() { 4.0 + 16.0 } else { 0.0 })
        }
        CardBody::Notification { title, body, max_title_lines, max_body_lines, .. } => {
            let tw = m.w(title, TITLE).min(MAX_W - 2.0 * pad_x - icon_w - gap).max(120.0);
            let bw = body.as_deref().map_or(0.0, |b| m.w(b, Style::new(12.0, 400)).min(300.0));
            let body_lines = body.as_deref().map_or(0, |b| ((m.w(b, Style::new(12.0, 400)) / (MAX_W - 2.0 * pad_x - icon_w - gap - actions_w).max(120.0)).ceil() as u32).clamp(1, *max_body_lines as u32));
            let _ = max_title_lines;
            (tw.max(bw), 18.0 + if body.is_some() { 4.0 + 16.0 * body_lines as f32 } else { 0.0 })
        }
        CardBody::Color { hex, rgb, history, .. } => (m.w(rgb, SUB).max(m.w(hex, TITLE)).max(history.len() as f32 * 20.0), 18.0 + 4.0 + 16.0 + if history.is_empty() { 0.0 } else { 24.0 }),
    };
    let w = (pad_x * 2.0 + icon_w + if icon_w > 0.0 { gap } else { 0.0 } + tw + if actions_w > 0.0 { gap + actions_w - 8.0 } else { 0.0 }).clamp(160.0, MAX_W);
    let h = (pad_y * 2.0 + icon_w.max(th_).max(if actions_w > 0.0 { 32.0 } else { 0.0 })).max(48.0);
    let cy = h / 2.0;
    let icon = Rect::new(pad_x, cy - icon_w / 2.0, icon_w, icon_w);
    let mut bx = w - pad_x;
    let mut buttons = Vec::new();
    for bw in btn_ws.iter().rev() {
        bx -= bw;
        buttons.push(Rect::new(bx, cy - 16.0, *bw, 32.0));
        bx -= 8.0;
    }
    buttons.reverse();
    let tx = pad_x + icon_w + if icon_w > 0.0 { gap } else { 0.0 };
    let tr = if buttons.is_empty() { w - pad_x } else { buttons[0].x - gap };
    CardLayout { w, h, icon, text: Rect::new(tx, cy - th_ / 2.0, (tr - tx).max(0.0), th_), buttons }
}

fn tone_colors(t: Tone, th: &Theme) -> (Color, Color) {
    match t {
        Tone::Green => (th.green, th.green_bg),
        Tone::Amber => (th.amber, th.amber_bg),
        Tone::Red => (th.red, th.red_bg),
        Tone::Accent => (th.accent, th.accent_soft),
        Tone::Neutral => (th.text, th.raised),
    }
}

/// Paints a card at `x0, y0` (top-left). Returns the button rects (window-independent, same space as the arguments).
pub fn paint_card(cv: &Canvas, th: &Theme, gpu: &mut Gpu, c: &lm::CardSpec, lay: &CardLayout, x0: f32, y0: f32, t: f64, hover: Option<usize>) -> Vec<Rect> {
    let (tone, tone_bg) = tone_colors(c.tone, th);
    let ir = lay.icon.offset(x0, y0);
    match (&c.icon, &c.body) {
        (_, CardBody::Notification { app, .. }) => {
            cv.fill_circle(ir.center(), 16.0, th.raised);
            let letter: String = app.chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
            cv.text(ir, &letter, Style::new(14.0, 600).center(), th.text);
        }
        (Some(CardIcon::Check), _) => {
            cv.fill_circle(ir.center(), 20.0, tone_bg);
            let k = (((t) / 360.0) as f32).clamp(0.0, 1.0);
            let (a, b, d) = ((ir.cx() - 6.0, ir.cy()), (ir.cx() - 1.5, ir.cy() + 5.0), (ir.cx() + 7.0, ir.cy() - 5.0));
            let s1 = (k * 2.0).clamp(0.0, 1.0);
            let s2 = (k * 2.0 - 1.0).clamp(0.0, 1.0);
            cv.line(a, (a.0 + (b.0 - a.0) * s1, a.1 + (b.1 - a.1) * s1), 2.0, tone);
            if s2 > 0.0 {
                cv.line(b, (b.0 + (d.0 - b.0) * s2, b.1 + (d.1 - b.1) * s2), 2.0, tone);
            }
        }
        (Some(CardIcon::Name(n)), _) => {
            cv.fill_circle(ir.center(), 20.0, tone_bg);
            gpu.icon(cv, n.as_str(), Rect::new(ir.cx() - 10.0, ir.cy() - 10.0, 20.0, 20.0), tone, 1.7);
        }
        (Some(CardIcon::Letter(l)), _) => {
            cv.fill_circle(ir.center(), 20.0, th.raised);
            cv.text(ir, l, Style::new(14.0, 600).center(), th.text);
        }
        (None, CardBody::Color { hex, .. }) => {
            let col = lm::Rgba::from_hex(hex).map(c8).unwrap_or(th.raised2);
            cv.fill_rr(ir, 12.0, col);
            cv.stroke_rr(ir, 12.0, 1.0, th.border);
        }
        (None, _) => {}
    }
    let tr = lay.text.offset(x0, y0);
    match &c.body {
        CardBody::Text { title, sub } => {
            cv.text(Rect::new(tr.x, tr.y, tr.w, 18.0), title, TITLE, th.text);
            if let Some(s) = sub {
                cv.text(Rect::new(tr.x, tr.y + 22.0, tr.w, 16.0), s, SUB, th.dim);
            }
        }
        CardBody::Notification { title, body, .. } => {
            cv.text(Rect::new(tr.x, tr.y, tr.w, 18.0), title, TITLE, th.text);
            if let Some(b) = body {
                cv.text(Rect::new(tr.x, tr.y + 22.0, tr.w, tr.h - 22.0), b, Style::new(12.0, 400).wrap(), th.dim);
            }
        }
        CardBody::Color { hex, rgb, copied, history } => {
            cv.text(Rect::new(tr.x, tr.y, tr.w, 18.0), hex, TITLE, th.text);
            cv.text(Rect::new(tr.x, tr.y + 22.0, tr.w, 16.0), &format!("{rgb} · {copied}"), SUB, th.dim);
            for (i, s) in history.iter().take(9).enumerate() {
                if let Some(col) = lm::Rgba::from_hex(&s.hex).map(c8) {
                    cv.fill_circle((tr.x + 8.0 + i as f32 * 20.0, tr.y + 46.0), 8.0, col);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (i, (a, br)) in c.actions.iter().zip(&lay.buttons).enumerate() {
        let r = br.offset(x0, y0);
        let white = a.kind == ActionKind::White;
        let base = if white { Color::rgb8(255, 255, 255) } else { th.raised };
        let bg = if hover == Some(i) { if white { base.mix(Color::rgb8(220, 220, 220), 0.5) } else { rows_hover(base) } } else { base };
        cv.fill_rr(r, 16.0, bg);
        cv.text(r, &a.label, BTN.center(), if white { Color::rgb8(0, 0, 0) } else { th.text2 });
        out.push(r);
    }
    out
}

fn rows_hover(c: Color) -> Color {
    c.mix(Color::rgb8(255, 255, 255), 0.12)
}

// ------------------------------------------------------------------------------------------------ content

/// Row-level enter animation (`RowSpec::anim.enter`) on top of the swap fade: (scale, dy, opacity) at `age_ms` since the row appeared.
pub fn row_enter(a: lm::Anim, age_ms: f64, reduced: bool) -> (f32, f32, f32) {
    if reduced || age_ms >= super::core::anim_ms(a) {
        return (1.0, 0.0, 1.0);
    }
    let p = (age_ms / super::core::anim_ms(a)).clamp(0.0, 1.0);
    match a {
        lm::Anim::Pop => (0.92 + 0.08 * mo::eval_linear(&mo::SPRING_POP, p) as f32, 0.0, ease((age_ms / 220.0).clamp(0.0, 1.0) as f32)),
        lm::Anim::Slide => (1.0, -8.0 * (1.0 - ease(p as f32)), ease(p as f32)),
        lm::Anim::Fade => (1.0, 0.0, ease(p as f32)),
        _ => (1.0, 0.0, 1.0),
    }
}

/// Natural size of a content (css px at 100 %); `None` when there is nothing to size.
pub fn measure(content: &LiveContent, ctx: &Ctx, m: &dyn Meter, th: &Theme) -> Option<(f32, f32)> {
    match content {
        LiveContent::None => None,
        LiveContent::Clock { row, time } => Some((layout_clock(row, time.as_deref(), m).w, 24.0)),
        LiveContent::Slot { id, .. } => ctx.rows.get(id).map(|r| {
            let l = layout_row(r, m, th);
            (l.w, l.h)
        }),
        LiveContent::Card(cv) => cv.spec.id.as_ref().and_then(|id| ctx.cards.get(id)).map(|c| {
            let l = layout_card(c, m);
            (l.w, l.h)
        }),
    }
}

/// Everything painting a content reports back for hit testing.
#[derive(Default)]
pub struct Painted {
    pub row_hits: Hits,
    pub row_id: Option<String>,
    pub card_buttons: Vec<Rect>,
    pub card_rect: Option<Rect>,
}

/// Paints `content` centred in `area` (local css px at 100 %), with `alpha` and the entry animation of age `age_ms`.
#[allow(clippy::too_many_arguments)]
pub fn paint_content(
    cv: &Canvas, th: &Theme, gpu: &mut Gpu, ctx: &Ctx, content: &LiveContent, rolls: &[(DigitRoll, f64)], entering: &[(bool, usize, f64)], area: Rect, alpha: f32, age_ms: f64, t: f64, hover_btn: Option<usize>, out: &mut Painted,
) {
    if alpha <= 0.0 {
        return;
    }
    let cy = area.y + (area.h - 2.0) / 2.0;
    match content {
        LiveContent::None => {}
        LiveContent::Clock { row, time } => {
            let m = CvMeter(cv);
            let lay = layout_clock(row, time.as_deref(), &m);
            let x0 = area.cx() - lay.w / 2.0;
            cv.with_opacity(alpha, |cv| paint_clock(cv, th, gpu, row, time.as_deref(), rolls, entering, &lay, x0, cy, ctx.reduced));
        }
        LiveContent::Slot { id, .. } => {
            let Some(row) = ctx.rows.get(id) else { return };
            let m = CvMeter(cv);
            let lay: RowLayout = layout_row(row, &m, th);
            let x0 = area.cx() - lay.w / 2.0;
            let (sc, dy, a2) = row_enter(row.anim.enter, age_ms, ctx.reduced);
            let cy = if row.tall { area.cy() } else { cy };
            out.row_id = Some(id.clone());
            cv.with_opacity(alpha * a2, |cv| {
                cv.with_scale_about((area.cx(), cy), sc, |cv| {
                    cv.with_offset(0.0, dy, |cv| paint_row(cv, th, gpu, row, &lay, x0, cy, Anim { t, age: age_ms, reduced: ctx.reduced }, &mut out.row_hits));
                })
            });
        }
        LiveContent::Card(view) => paint_card_view(cv, th, gpu, ctx, view, area, alpha, t, hover_btn, out),
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_card_view(cv: &Canvas, th: &Theme, gpu: &mut Gpu, ctx: &Ctx, view: &CardView, area: Rect, alpha: f32, t: f64, hover_btn: Option<usize>, out: &mut Painted) {
    let Some(card) = view.spec.id.as_ref().and_then(|id| ctx.cards.get(id)) else { return };
    let m = CvMeter(cv);
    let lay = layout_card(card, &m);
    let (x0, y0) = (area.cx() - lay.w / 2.0, area.cy() - lay.h / 2.0);
    let (spec, age): (&AnimSpec, f64) = match view.phase {
        CardPhase::In { since_ms } => (&mo::CARD_IN, t - since_ms),
        CardPhase::Out { since_ms } => (&mo::CARD_OUT, t - since_ms),
    };
    let p = spec.at(age.max(0.0), ctx.reduced);
    out.card_rect = Some(Rect::new(x0, y0, lay.w, lay.h));
    cv.with_opacity(alpha * p.opacity as f32, |cv| {
        cv.with_scale_about((area.cx(), y0), p.scale as f32, |cv| {
            out.card_buttons = paint_card(cv, th, gpu, card, &lay, x0, y0, age.max(0.0), hover_btn);
        })
    });
}

#[allow(dead_code)]
pub fn row_h() -> f32 {
    ROW_H
}

#[allow(dead_code)]
pub fn accent_of(a: Option<lm::Accent>, th: &Theme) -> Color {
    accent_color(a, th)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::rows::Mono;
    use pill_ui_core::shell::clock::ClockItem;

    fn clock(left: Vec<ClockItem>, right: Vec<ClockItem>) -> ClockRow {
        ClockRow { left, show_time: true, right, eic: false, pcl: false, pcr: false }
    }

    #[test]
    fn side_columns_are_equal_so_the_time_sits_on_the_axis() {
        let m = Mono(0.5);
        let r = clock(vec![ClockItem::WxIcon { id: "cloud".into(), glyph: "cloud", cls: "" }], vec![ClockItem::Temp(18)]);
        let l = layout_clock(&r, Some("12:34"), &m);
        assert_eq!(l.left_w, l.right_w);
        assert!((l.time_x - (16.0 + l.left_w + 16.0)).abs() < 1e-3);
        let total_left = l.time_x;
        let total_right = l.w - (l.time_x + l.time_w);
        assert!((total_left - total_right).abs() < 1e-3, "axis: {total_left} vs {total_right}");
    }

    #[test]
    fn a_collapsed_pet_column_gives_its_room_back() {
        let m = Mono(0.5);
        let mut r = clock(vec![], vec![ClockItem::Temp(18)]);
        let full = layout_clock(&r, Some("12:34"), &m).w;
        r.pcl = true;
        assert!(layout_clock(&r, Some("12:34"), &m).w < full);
    }

    #[test]
    fn card_width_follows_content_and_never_exceeds_480() {
        let m = Mono(0.5);
        let short = lm::CardSpec::text("a", 1000, "Done", Some("sub".into())).icon(lm::IconName::Check);
        let long = lm::CardSpec::text("b", 1000, "x".repeat(400), None).icon(lm::IconName::Check);
        let (a, b) = (layout_card(&short, &m), layout_card(&long, &m));
        assert!(a.w < b.w && b.w == 480.0 && a.w >= 160.0);
        assert_eq!(a.h, 56.0, "icon 40 + 2 x 8 padding");
        let with_btn = short.clone().actions(vec![lm::CardAction::close("Close")]);
        let c = layout_card(&with_btn, &m);
        assert_eq!(c.buttons.len(), 1);
        assert_eq!(c.buttons[0].h, 32.0);
        assert!(c.buttons[0].right() <= c.w - 20.0 + 1e-3 && c.text.right() <= c.buttons[0].x);
    }

    #[test]
    fn row_enter_animations_end_at_rest_and_reduced_motion_skips_them() {
        assert_eq!(row_enter(lm::Anim::Pop, 0.0, true), (1.0, 0.0, 1.0));
        assert_eq!(row_enter(lm::Anim::Pop, 1000.0, false), (1.0, 0.0, 1.0));
        let (s, _, a) = row_enter(lm::Anim::Pop, 0.0, false);
        assert!((s - 0.92).abs() < 1e-3 && a == 0.0);
        let (_, dy, _) = row_enter(lm::Anim::Slide, 0.0, false);
        assert!(dy < 0.0);
    }
}
