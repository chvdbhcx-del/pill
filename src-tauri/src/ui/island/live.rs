//! The collapsed content of the island: the clock row (`renderClock`), a live row and an event card, as layout + painting.
//! All numbers are css px at 100 %; the caller scales the canvas by the island size and places the content area.
use super::gpu::{c8, Gpu};
use super::rows::{accent_color, layout_row, paint_row, Anim, Hits, Meter, RowLayout, ROW_H};
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::{Canvas, Color};
use pill_ui_core::live::model::{self as lm, ActionKind, CardBody, CardIcon, Tone};
use pill_ui_core::shell::cards::CardView;
use pill_ui_core::shell::clock::{ClockItem, ClockRow, DigitRoll, SlotItem};
use pill_ui_core::shell::types::LiveContent;
use std::collections::HashMap;

pub const TIME_STYLE: Style = Style::new(13.0, 600).mono().track(0.13);
const DATE_STYLE: Style = Style::new(12.0, 600).mono();
const TEMP_STYLE: Style = Style::new(13.0, 600).mono();
const BATT_STYLE: Style = Style::new(12.0, 600).mono();
const TAG_STYLE: Style = Style::new(11.0, 700).mono();
const TAG_PAD: f32 = 5.0;
const COL_GAP: f32 = 16.0;
const SIDE_GAP: f32 = 12.0;

/// What a content needs from the model while measuring / painting.
#[derive(Clone)]
pub struct Ctx<'a> {
    pub rows: &'a HashMap<String, lm::RowSpec>,
    pub cards: &'a HashMap<String, lm::CardSpec>,
    pub reduced: bool,
    pub ru: bool,
    /// smoothed music levels for the media row's equaliser (see `Anim::eq`)
    pub eq: Option<[f32; 4]>,
    /// where the content's own entry motions stand (the flows of `fx.rs`)
    pub fx: Fx<'a>,
}

/// The card on screen as the flows of `fx.rs` have it right now.
#[derive(Clone, Copy)]
pub struct CardPaint<'a> {
    pub alpha: f32,
    pub scale: f32,
    /// how much of the check mark is drawn
    pub check: f32,
    /// 0..1 hover amount per action button
    pub hover: &'a [f32],
}

/// Per-frame numbers of the content's own motions, resolved from the flows of `fx.rs`: digits rolling (progress), clock side items popping in
/// (right column?, index, scale, opacity), the row's enter (scale, dy, opacity) and the card.
#[derive(Clone)]
pub struct Fx<'a> {
    pub rolls: &'a [(DigitRoll, f32)],
    pub entering: &'a [(bool, usize, f32, f32)],
    pub row: (f32, f32, f32),
    pub card: CardPaint<'a>,
    /// the download ring next to the clock: (progress 0..1, or -1 while the size is unknown; angle of the turning arc in degrees)
    pub ring: (f32, f32),
}

impl Fx<'static> {
    /// Everything at rest (measuring, tests).
    pub const REST: Fx<'static> = Fx { rolls: &[], entering: &[], row: (1.0, 0.0, 1.0), card: CardPaint { alpha: 1.0, scale: 1.0, check: 1.0, hover: &[] }, ring: (0.0, 0.0) };
}

/// The download ring next to the clock (css px).
const RING_SIZE: f32 = 16.0;

fn item_w(it: &ClockItem, m: &dyn Meter) -> f32 {
    match it {
        ClockItem::WxIcon { .. } => 20.0,
        ClockItem::Date(t) => m.w(t, DATE_STYLE),
        ClockItem::Battery { percent, .. } => 20.0 + 4.0 + m.w(&format!("{}%", percent.round() as i32), BATT_STYLE),
        ClockItem::Temp(t) => m.w(&format!("{t}°"), TEMP_STYLE),
        ClockItem::Tag(t) => m.w(t, TAG_STYLE) + 2.0 * TAG_PAD,
        ClockItem::Slot(SlotItem::WxIcon(_)) => 20.0,
        ClockItem::Slot(SlotItem::Temp(t)) => m.w(&format!("{t}°"), TEMP_STYLE),
        ClockItem::Slot(SlotItem::Text(t)) => m.w(t, TEMP_STYLE),
        ClockItem::Slot(SlotItem::Spec(_)) => 0.0,
        ClockItem::Slot(SlotItem::Ring) => RING_SIZE,
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

/// The clock row; `x0` = left edge, `cy` = vertical centre.
pub fn paint_clock(cv: &Canvas, th: &Theme, gpu: &mut Gpu, row: &ClockRow, time: Option<&str>, rolls: &[(DigitRoll, f32)], entering: &[(bool, usize, f32, f32)], lay: &ClockLayout, ring: (f32, f32), x0: f32, cy: f32) {
    let pop = |right: bool, i: usize| -> (f32, f32) { entering.iter().find(|e| e.0 == right && e.1 == i).map_or((1.0, 1.0), |e| (e.2, e.3)) };
    // left column: right aligned against the time
    let mut x = x0 + 16.0 + lay.left_w;
    for (i, it) in row.left.iter().enumerate().rev() {
        let w = it_w_cached(it, cv);
        x -= w;
        let (s, a) = pop(false, i);
        paint_item(cv, th, gpu, it, Rect::new(x, cy - 10.0, w, 20.0), s, a, ring);
        x -= SIDE_GAP;
    }
    // right column: left aligned against the time
    let mut x = x0 + lay.right_x;
    for (i, it) in row.right.iter().enumerate() {
        let w = it_w_cached(it, cv);
        let (s, a) = pop(true, i);
        paint_item(cv, th, gpu, it, Rect::new(x, cy - 10.0, w, 20.0), s, a, ring);
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

fn paint_time(cv: &Canvas, th: &Theme, time: &str, rolls: &[(DigitRoll, f32)], r: Rect) {
    let chars: Vec<char> = time.chars().collect();
    let mut x = r.x;
    for (i, ch) in chars.iter().enumerate() {
        let s = ch.to_string();
        let w = cv.text_width(&s, TIME_STYLE);
        let cell = Rect::new(x, r.y, w, r.h);
        match rolls.iter().find(|(d, _)| d.index == i) {
            Some((d, p)) => {
                // the roll is a flow: the position may swing a hair past the end, the opacity may not
                let (p, k) = (*p, p.clamp(0.0, 1.0));
                cv.with_clip(cell, |cv| {
                    cv.text(cell.offset(0.0, -r.h * p), &d.from.to_string(), TIME_STYLE, th.text.fade(1.0 - k));
                    cv.text(cell.offset(0.0, r.h * (1.0 - p)), &d.to.to_string(), TIME_STYLE, th.text.fade(k));
                });
            }
            None => cv.text(cell, &s, TIME_STYLE, th.text),
        }
        x += w;
    }
}

fn paint_item(cv: &Canvas, th: &Theme, gpu: &mut Gpu, it: &ClockItem, r: Rect, scale: f32, alpha: f32, ring: (f32, f32)) {
    cv.with_opacity(alpha, |cv| {
        cv.with_scale_about(r.center(), scale, |cv| match it {
            ClockItem::WxIcon { id, .. } | ClockItem::Slot(SlotItem::WxIcon(id)) => gpu.wx_icon(cv, id, r),
            ClockItem::Date(t) => cv.text(Rect::new(r.x, r.cy() - 8.0, r.w, 16.0), t, DATE_STYLE, th.text.fade(0.6)),
            ClockItem::Temp(t) | ClockItem::Slot(SlotItem::Temp(t)) => cv.text(Rect::new(r.x, r.cy() - 9.0, r.w + 2.0, 18.0), &format!("{t}°"), TEMP_STYLE, th.text),
            ClockItem::Slot(SlotItem::Text(t)) => cv.text(Rect::new(r.x, r.cy() - 9.0, r.w + 2.0, 18.0), t, TEMP_STYLE, th.text),
            ClockItem::Tag(t) => {
                cv.fill_rr(Rect::new(r.x, r.cy() - 7.5, r.w, 15.0), 5.0, th.text.fade(0.14));
                cv.text(Rect::new(r.x + TAG_PAD, r.cy() - 8.0, r.w, 16.0), t, TAG_STYLE, th.text.fade(0.85));
            }
            ClockItem::Slot(SlotItem::Spec(_)) => {}
            ClockItem::Slot(SlotItem::Ring) => {
                // same look as the ring of the download chip: a track and an accent arc (a known percent, or a short arc that turns)
                let (c, rr) = (r.center(), RING_SIZE / 2.0 - 1.25);
                cv.stroke_circle(c, rr, 2.5, th.raised2);
                if ring.0 < 0.0 {
                    cv.arc(c, rr, 2.5, ring.1, 100.0, th.accent);
                } else {
                    cv.arc(c, rr, 2.5, 0.0, 360.0 * ring.0.clamp(0.0, 1.0), th.accent);
                }
            }
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
pub fn paint_card(cv: &Canvas, th: &Theme, gpu: &mut Gpu, c: &lm::CardSpec, lay: &CardLayout, x0: f32, y0: f32, check: f32, hover: &[f32]) -> Vec<Rect> {
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
            let k = check.clamp(0.0, 1.0);
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
        let h = hover.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let bg = if white { base.mix(Color::rgb8(220, 220, 220), 0.5 * h) } else { rows_hover(base, h) };
        cv.fill_rr(r, 16.0, bg);
        cv.text(r, &a.label, BTN.center(), if white { Color::rgb8(0, 0, 0) } else { th.text2 });
        out.push(r);
    }
    out
}

fn rows_hover(c: Color, amount: f32) -> Color {
    c.mix(Color::rgb8(255, 255, 255), 0.12 * amount)
}

// ------------------------------------------------------------------------------------------------ content

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

/// Paints `content` centred in `area` (local css px at 100 %), with `alpha`; its own entry motions come from `ctx.fx`, `age_ms` is how long
/// it has been there (things that draw themselves start over when it goes back).
#[allow(clippy::too_many_arguments)]
pub fn paint_content(cv: &Canvas, th: &Theme, gpu: &mut Gpu, ctx: &Ctx, content: &LiveContent, area: Rect, alpha: f32, age_ms: f64, t: f64, out: &mut Painted) {
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
            cv.with_opacity(alpha, |cv| paint_clock(cv, th, gpu, row, time.as_deref(), ctx.fx.rolls, ctx.fx.entering, &lay, ctx.fx.ring, x0, cy));
        }
        LiveContent::Slot { id, .. } => {
            let Some(row) = ctx.rows.get(id) else { return };
            let m = CvMeter(cv);
            let lay: RowLayout = layout_row(row, &m, th);
            // while the capsule is still narrower than the row (state change, pet column already reserved) the row must not spill out on the
            // left (the leading icon / spinner would be clipped and the text pushed to the edge): pin it to the left, the right end is revealed
            let x0 = (area.cx() - lay.w / 2.0).max(area.x);
            let (sc, dy, a2) = ctx.fx.row;
            let cy = if row.tall { area.cy() } else { cy };
            out.row_id = Some(id.clone());
            cv.with_opacity(alpha * a2, |cv| {
                cv.with_scale_about((area.cx(), cy), sc, |cv| {
                    cv.with_offset(0.0, dy, |cv| paint_row(cv, th, gpu, row, &lay, x0, cy, Anim { t, age: age_ms, reduced: ctx.reduced, eq: ctx.eq }, &mut out.row_hits));
                })
            });
        }
        LiveContent::Card(view) => paint_card_view(cv, th, gpu, ctx, view, area, alpha, out),
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_card_view(cv: &Canvas, th: &Theme, gpu: &mut Gpu, ctx: &Ctx, view: &CardView, area: Rect, alpha: f32, out: &mut Painted) {
    let Some(card) = view.spec.id.as_ref().and_then(|id| ctx.cards.get(id)) else { return };
    let m = CvMeter(cv);
    let lay = layout_card(card, &m);
    let (x0, y0) = (area.cx() - lay.w / 2.0, area.cy() - lay.h / 2.0);
    let k = ctx.fx.card;
    out.card_rect = Some(Rect::new(x0, y0, lay.w, lay.h));
    cv.with_opacity(alpha * k.alpha, |cv| {
        cv.with_scale_about((area.cx(), y0), k.scale, |cv| {
            out.card_buttons = paint_card(cv, th, gpu, card, &lay, x0, y0, k.check, k.hover);
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
}
