//! The collapsed content of the island: the clock row (`renderClock`), a live row and an event card, as layout + painting.
//! All numbers are css px at 100 %; the caller scales the canvas by the island size and places the content area.
use super::gpu::{c8, Gpu};
use super::rows::{accent_color, layout_row, paint_row, Anim, Hits, Meter, RowLayout, ROW_H};
use glasskit::geom::Rect;
use glasskit::text::{Style, VAlign};
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
    /// the tallest an event card may be (css px): 60 % of the work area
    pub card_max_h: f32,
    /// how much of the window is left below the top of the body (css px; `f32::MAX` = no limit, as at the bottom edge): a card that asked for a
    /// taller window than it has yet is painted shorter (its text gives lines up) and stands inside what there is, so it is never cut off
    pub room_h: f32,
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
    /// a phone card is on screen and its second row (buttons, reply field) is part of it (phonecard.rs)
    pub phone_open: bool,
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
    pub const REST: Fx<'static> = Fx { rolls: &[], entering: &[], row: (1.0, 0.0, 1.0), card: CardPaint { alpha: 1.0, scale: 1.0, check: 1.0, hover: &[], phone_open: false }, ring: (0.0, 0.0) };
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
pub fn paint_clock(cv: &Canvas, th: &Theme, gpu: &mut Gpu, row: &ClockRow, time: Option<&str>, rolls: &[(DigitRoll, f32)], entering: &[(bool, usize, f32, f32)], lay: &ClockLayout, ring: (f32, f32), x0: f32, cy: f32) -> Option<Rect> {
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
    let mut chip = None;
    for (i, it) in row.right.iter().enumerate() {
        let w = it_w_cached(it, cv);
        let (s, a) = pop(true, i);
        let r = Rect::new(x, cy - 10.0, w, 20.0);
        if matches!(it, ClockItem::Tag(_)) {
            chip = Some(r); // the only tag of the right column is the virtual desktop: a click on it switches desktops
        }
        paint_item(cv, th, gpu, it, r, s, a, ring);
        x += w + SIDE_GAP;
    }
    if let (true, Some(t)) = (row.show_time, time) {
        paint_time(cv, th, t, rolls, Rect::new(x0 + lay.time_x, cy - 9.0, lay.time_w, 18.0));
    }
    chip
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

const TITLE: Style = Style::new(14.0, 500).ellipsis();
const SUB: Style = Style::new(12.0, 400).ellipsis();
const BTN: Style = Style::new(12.0, 500);
const MAX_W: f32 = 480.0;

#[derive(Clone, Debug)]
pub struct CardLayout {
    pub w: f32,
    pub h: f32,
    pub icon: Rect,
    pub text: Rect,
    /// heights of the title and the sub line inside `text` (a text card that needs two rows wraps them to several lines)
    pub title_h: f32,
    pub sub_h: f32,
    /// the two-row layout: the painter cuts what does not fit its lines with an ellipsis
    pub stacked: bool,
    pub buttons: Vec<Rect>,
}

/// The widest a card gets when its buttons go under the text (a one-row card stops at [`MAX_W`]).
const STACK_W: f32 = 560.0;
const TITLE_PITCH: f32 = 18.0;
/// DirectWrite's line pitch of the 14 px title when it wraps (a one-line title is centred in an 18 px box)
const TITLE_LINE: f32 = 20.0;
const SUB_PITCH: f32 = 16.0;
/// The gap kept between a card and the end of its window.
const CARD_EDGE: f32 = 4.0;

/// The tallest a card may be in `room` px of window (never under a one-row card's 56).
fn fit_room(room: f32) -> f32 {
    (room - CARD_EDGE).max(56.0)
}

/// A card's tallest height (css px) when the screen is not known (the tests); the island passes 60 % of the work area.
pub const CARD_MAX_H: f32 = 520.0;

/// How many lines `w` px of text take in `avail` px (8 % slack: a line breaks at a word, so it holds a little less than its width).
fn lines_of(w: f32, avail: f32, max: f32) -> f32 {
    (w / (avail * 0.92).max(1.0)).ceil().clamp(1.0, max)
}

/// `s` (`total` px wide on one line) cut with an ellipsis so that it fits `lines` lines of `w` px; as it is when it fits.
pub fn clip_lines(s: &str, total: f32, w: f32, lines: f32) -> std::borrow::Cow<'_, str> {
    let room = w * 0.92 * lines;
    if total <= room {
        return s.into();
    }
    let keep = ((s.chars().count() as f32) * (room / total)).floor() as usize;
    let cut: String = s.chars().take(keep.saturating_sub(2).max(1)).collect();
    format!("{}\u{2026}", cut.trim_end()).into()
}

/// The text of a stacked card cut to the lines it was given.
fn fit_text<'s>(cv: &Canvas, lay: &CardLayout, s: &'s str, st: Style, w: f32, h: f32, pitch: f32) -> std::borrow::Cow<'s, str> {
    if lay.stacked {
        clip_lines(s, cv.text_width(s, st), w, h / pitch)
    } else {
        s.into()
    }
}

/// A text card whose text and buttons do not fit one row: the icon and the text (the title and the sub line wrap to as many lines as they need) on
/// top, the buttons under it: side by side while every label fits its share on one line, else one full-width button per option with its label
/// wrapped (the button grows with its lines). The card never gets taller than `max_h`: the text gives lines up first, then the labels (the painter
/// cuts what does not fit with an ellipsis).
fn stacked(title: &str, sub: Option<&str>, icon_w: f32, btn_ws: &[f32], m: &dyn Meter, max_h: f32) -> CardLayout {
    let (pad_x, gap, bgap, top, mid, bot) = (20.0, 12.0, 8.0, 12.0, 10.0, 12.0);
    let lead = pad_x * 2.0 + icon_w + if icon_w > 0.0 { gap } else { 0.0 };
    let (tw, sw) = (m.w(title, TITLE) + 6.0, sub.map_or(0.0, |s| m.w(s, SUB) + 6.0));
    let n = btn_ws.len() as f32;
    let sum: f32 = btn_ws.iter().sum();
    let w = (lead + tw.max(sw).max(sum + bgap * (n - 1.0))).clamp(280.0, STACK_W);
    let (avail, row_w) = (w - lead, w - 2.0 * pad_x);
    let free = row_w - bgap * (n - 1.0);
    let side = sum <= free;
    let mut bl: Vec<f32> = if side { vec![1.0; btn_ws.len()] } else { btn_ws.iter().map(|b| lines_of(b - 32.0, row_w - 32.0, 8.0)).collect() };
    let (mut tl, mut sl) = (lines_of(tw, avail, 40.0), if sub.is_some() { lines_of(sw, avail, 12.0) } else { 0.0 });
    let btn_h = |l: f32| if l <= 1.0 { 32.0 } else { 16.0 + SUB_PITCH * l };
    let total = |tl: f32, sl: f32, bl: &[f32]| {
        let th_ = TITLE_LINE * tl + if sub.is_some() { 4.0 + SUB_PITCH * sl } else { 0.0 };
        top + icon_w.max(th_) + mid + if side { 32.0 } else { bl.iter().map(|l| btn_h(*l)).sum::<f32>() + bgap * (n - 1.0) } + bot
    };
    while total(tl, sl, &bl) > max_h && (tl > 1.0 || sl > 1.0) {
        if sl >= tl && sl > 1.0 {
            sl -= 1.0;
        } else if tl > 1.0 {
            tl -= 1.0;
        } else {
            sl -= 1.0;
        }
    }
    while total(tl, sl, &bl) > max_h && !side && bl.iter().any(|l| *l > 1.0) {
        let i = (0..bl.len()).max_by(|a, b| bl[*a].total_cmp(&bl[*b])).unwrap_or(0);
        bl[i] -= 1.0;
    }
    let (title_h, sub_h) = (TITLE_LINE * tl, SUB_PITCH * sl);
    let th_ = title_h + if sub.is_some() { 4.0 + sub_h } else { 0.0 };
    let row1 = icon_w.max(th_);
    let h = total(tl, sl, &bl);
    let by = top + row1 + mid;
    let mut buttons = Vec::new();
    if !side {
        let mut y = by;
        for l in &bl {
            buttons.push(Rect::new(pad_x, y, row_w, btn_h(*l)));
            y += btn_h(*l) + bgap;
        }
    } else {
        let widths: Vec<f32> = if n == 1.0 {
            vec![btn_ws[0].min(row_w)]
        } else if btn_ws.iter().all(|b| *b <= free / n) {
            vec![free / n; btn_ws.len()]
        } else {
            btn_ws.iter().map(|b| b + (free - sum) / n).collect()
        };
        let mut bx = if n == 1.0 { w - pad_x - widths[0] } else { pad_x };
        for bw in widths {
            buttons.push(Rect::new(bx, by, bw, 32.0));
            bx += bw + bgap;
        }
    }
    CardLayout {
        w,
        h,
        icon: Rect::new(pad_x, top + (row1 - icon_w) / 2.0, icon_w, icon_w),
        text: Rect::new(lead - pad_x, top + (row1 - th_) / 2.0, avail, th_),
        title_h,
        sub_h,
        stacked: true,
        buttons,
    }
}

pub fn layout_card(c: &lm::CardSpec, m: &dyn Meter) -> CardLayout {
    layout_card_open(c, m, false, CARD_MAX_H)
}

/// `open`: only a phone card has one, its second row (hover / typing) is part of the card.
pub fn layout_card_open(c: &lm::CardSpec, m: &dyn Meter, open: bool, max_h: f32) -> CardLayout {
    if let CardBody::Phone(p) = &c.body {
        let l = super::phonecard::layout(p, m, open);
        return CardLayout { w: l.w, h: l.h, icon: l.avatar, text: l.text, title_h: TITLE_PITCH, sub_h: SUB_PITCH, stacked: false, buttons: l.buttons.to_vec() };
    }
    let (pad_x, pad_y, gap) = (20.0, 8.0, 12.0);
    let btn_ws: Vec<f32> = c.actions.iter().map(|a| (m.w(&a.label, BTN) + 32.0).max(32.0)).collect();
    let actions_w = if btn_ws.is_empty() { 0.0 } else { btn_ws.iter().sum::<f32>() + 8.0 * (btn_ws.len() - 1) as f32 + 8.0 };
    let icon_w = match (&c.icon, &c.body) {
        (_, CardBody::Notification { .. }) => 32.0,
        (_, CardBody::Phone(_)) => 0.0,
        (None, CardBody::Color { .. }) => 40.0,
        (None, _) => 0.0,
        _ => 40.0,
    };
    let (tw, th_) = match &c.body {
        CardBody::Text { title, sub } => {
            // a few px of slack: text that exactly fills its box is ellipsised by the painter ("Reply se...")
            let tw = m.w(title, TITLE).max(sub.as_deref().map_or(0.0, |s| m.w(s, SUB))) + 6.0;
            (tw, 18.0 + if sub.is_some() { 4.0 + 16.0 } else { 0.0 })
        }
        CardBody::Notification { title, body, max_title_lines, max_body_lines, .. } => {
            let tw = m.w(title, TITLE).min(MAX_W - 2.0 * pad_x - icon_w - gap).max(120.0);
            let bw = body.as_deref().map_or(0.0, |b| m.w(b, Style::new(12.0, 400)).min(300.0));
            let body_lines = body.as_deref().map_or(0, |b| ((m.w(b, Style::new(12.0, 400)) / (MAX_W - 2.0 * pad_x - icon_w - gap - actions_w).max(120.0)).ceil() as u32).clamp(1, *max_body_lines as u32));
            let _ = max_title_lines;
            (tw.max(bw), 18.0 + if body.is_some() { 4.0 + 16.0 * body_lines as f32 } else { 0.0 })
        }
        CardBody::Phone(_) => (0.0, 0.0),
        CardBody::Activity { title, sub, progress, chip } => {
            let chip_w = chip.as_deref().map_or(0.0, |c| m.w(c, SUB) + 10.0);
            let tw = (m.w(title, TITLE) + chip_w).max(sub.as_deref().map_or(0.0, |s| m.w(s, SUB))).max(if progress.is_some() { 140.0 } else { 0.0 }) + 6.0;
            (tw, 18.0 + if sub.is_some() { 4.0 + 16.0 } else { 0.0 } + if progress.is_some() { 6.0 + 4.0 } else { 0.0 })
        }
        CardBody::Color { hex, rgb, history, .. } => (m.w(rgb, SUB).max(m.w(hex, TITLE)).max(history.len() as f32 * 20.0), 18.0 + 4.0 + 16.0 + if history.is_empty() { 0.0 } else { 24.0 }),
    };
    let one_row = pad_x * 2.0 + icon_w + if icon_w > 0.0 { gap } else { 0.0 } + tw + if actions_w > 0.0 { gap + actions_w - 8.0 } else { 0.0 };
    if let (CardBody::Text { title, sub }, true) = (&c.body, one_row > MAX_W && !btn_ws.is_empty()) {
        return stacked(title, sub.as_deref(), icon_w, &btn_ws, m, max_h);
    }
    let w = one_row.clamp(160.0, MAX_W);
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
    CardLayout { w, h, icon, text: Rect::new(tx, cy - th_ / 2.0, (tr - tx).max(0.0), th_), title_h: TITLE_PITCH, sub_h: SUB_PITCH, stacked: false, buttons }
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
        (_, CardBody::Phone(_)) => {}
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
            // a card that needed two rows wraps its text (its lines are taller than one), else one line cut with an ellipsis
            let wrapped = |st: Style, h: f32, one: f32| if lay.stacked || h > one { Style { wrap: true, ellipsis: false, valign: VAlign::Top, ..st } } else { st };
            // what does not fit its lines is cut with an ellipsis (a card that grew to the screen's limit)
            cv.text(Rect::new(tr.x, tr.y, tr.w, lay.title_h), &fit_text(cv, lay, title, TITLE, tr.w, lay.title_h, TITLE_LINE), wrapped(TITLE, lay.title_h, TITLE_PITCH), th.text);
            if let Some(s) = sub {
                cv.text(Rect::new(tr.x, tr.y + lay.title_h + 4.0, tr.w, lay.sub_h), &fit_text(cv, lay, s, SUB, tr.w, lay.sub_h, SUB_PITCH), wrapped(SUB, lay.sub_h, SUB_PITCH), th.dim);
            }
        }
        CardBody::Notification { title, body, .. } => {
            cv.text(Rect::new(tr.x, tr.y, tr.w, 18.0), title, TITLE, th.text);
            if let Some(b) = body {
                cv.text(Rect::new(tr.x, tr.y + 22.0, tr.w, tr.h - 22.0), b, Style::new(12.0, 400).wrap(), th.dim);
            }
        }
        CardBody::Phone(_) => {}
        CardBody::Activity { title, sub, progress, chip } => {
            let cw = chip.as_deref().map_or(0.0, |c| cv.text_width(c, SUB) + 10.0);
            cv.text(Rect::new(tr.x, tr.y, (tr.w - cw).max(0.0), 18.0), title, TITLE, th.text);
            if let Some(c) = chip {
                cv.text(Rect::new(tr.x + tr.w - cw, tr.y + 1.0, cw, 18.0), c, SUB.right(), th.accent);
            }
            let mut y = tr.y + 18.0;
            if let Some(s) = sub {
                cv.text(Rect::new(tr.x, tr.y + 22.0, tr.w, 16.0), s, SUB, th.dim);
                y += 20.0;
            }
            if let Some(p) = progress {
                // the bar sweeps to its value while the card comes in (`check` is the card's own 0..1 entry ramp)
                let k = 1.0 - (1.0 - check.clamp(0.0, 1.0)).powi(3);
                let bar = Rect::new(tr.x, y + 6.0, (tr.w - 6.0).max(0.0), 4.0);
                cv.fill_rr(bar, 2.0, th.raised2);
                let w = if *p > 0.0 { (bar.w * p.clamp(0.0, 1.0) * k).max(4.0) } else { 0.0 };
                cv.fill_rr(Rect::new(bar.x, bar.y, w, bar.h), 2.0, th.accent);
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
        let ink = if white { Color::rgb8(0, 0, 0) } else { th.text2 };
        if r.h > 33.0 {
            // a full-width button of a stacked card: the label wraps over its lines
            let (tw, lines) = (r.w - 32.0, ((r.h - 16.0) / SUB_PITCH).round());
            cv.text(Rect::new(r.x + 16.0, r.y, tw, r.h), &clip_lines(&a.label, cv.text_width(&a.label, BTN), tw, lines), Style { wrap: true, ellipsis: false, ..BTN.center() }, ink);
        } else {
            cv.text(r, &a.label, BTN.center().ellipsis(), ink);
        }
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
            let l = layout_card_open(c, m, ctx.fx.card.phone_open, ctx.card_max_h);
            (l.w, l.h)
        }),
    }
}

/// Everything painting a content reports back for hit testing.
#[derive(Default)]
pub struct Painted {
    pub row_hits: Hits,
    pub row_id: Option<String>,
    /// the virtual-desktop chip beside the time (click / wheel switch desktops), local css px like the row's hits
    pub chip: Option<Rect>,
    pub card_buttons: Vec<Rect>,
    pub card_rect: Option<Rect>,
    /// a phone card is the content: the island paints it itself (phonecard.rs) at this rect, with this opacity and scale
    pub phone: Option<(Rect, f32, f32)>,
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
            cv.with_opacity(alpha, |cv| out.chip = paint_clock(cv, th, gpu, row, time.as_deref(), ctx.fx.rolls, ctx.fx.entering, &lay, ctx.fx.ring, x0, cy));
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
    let lay = layout_card_open(card, &m, ctx.fx.card.phone_open, ctx.card_max_h.min(fit_room(ctx.room_h)));
    let k = ctx.fx.card;
    if matches!(card.body, CardBody::Phone(_)) {
        // anchored at the top: the second row is revealed by the growing body instead of sliding the first row
        let (x0, y0) = (area.cx() - lay.w / 2.0, area.y);
        let r = Rect::new(x0, y0, lay.w, lay.h);
        out.card_rect = Some(r);
        out.phone = Some((r, alpha * k.alpha, k.scale));
        return;
    }
    // centred in the body, but inside the window while that is still growing to the card
    let mut y0 = area.cy() - lay.h / 2.0;
    if ctx.room_h < f32::MAX {
        y0 = y0.min(ctx.room_h - lay.h - CARD_EDGE).max(0.0);
    }
    let x0 = area.cx() - lay.w / 2.0;
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

    fn ask(title: &str, sub: &str, labels: &[&str]) -> lm::CardSpec {
        let acts = labels.iter().enumerate().map(|(i, l)| lm::CardAction::new(*l, if i == 0 { ActionKind::White } else { ActionKind::Normal }, lm::Action::Noop)).collect();
        lm::CardSpec::text("q", 1000, title, Some(sub.into())).icon(lm::IconName::Sparkles).actions(acts)
    }

    /// Everything of a card inside it, the text above the buttons, the buttons apart.
    fn assert_tidy(l: &CardLayout) {
        let inside = |r: &Rect| r.x >= -1e-3 && r.y >= -1e-3 && r.right() <= l.w + 1e-3 && r.bottom() <= l.h + 1e-3;
        assert!(inside(&l.icon) && inside(&l.text) && l.buttons.iter().all(inside), "{l:?}");
        assert!(l.buttons.windows(2).all(|p| p[0].right() <= p[1].x + 1e-3 || p[0].bottom() <= p[1].y + 1e-3), "{l:?}");
        assert!(l.buttons.iter().all(|b| l.icon.right() <= b.x || l.icon.bottom() <= b.y) && l.buttons.iter().all(|b| l.text.bottom() <= b.y + 1e-3), "{l:?}");
    }

    #[test]
    fn a_card_that_does_not_fit_one_row_puts_its_buttons_under_the_text() {
        let m = Mono(0.5);
        let q = ask("Тест: появилась ли на острове карточка с этими вариантами и кнопками?", "Claude Code asks", &["Да, вижу на острове", "Нет, только в терминале"]);
        let l = layout_card(&q, &m);
        assert_tidy(&l);
        assert!(l.w > MAX_W && l.w <= STACK_W, "wider than a one-row card, not past the cap: {}", l.w);
        assert!(l.title_h > TITLE_PITCH, "the long question wraps");
        assert!(l.buttons.iter().all(|b| b.y > l.text.bottom() && b.h == 32.0 && b.w >= m.w("Нет, только в терминале", BTN) + 32.0 - 1e-3), "every label has its whole width: {l:?}");
        // three long options and a long sub line, the same rules
        let p = ask("Allow Bash?", &"git push --force origin feature/very-long-branch ".repeat(3), &["Deny", "Allow once", "Always allow in this project"]);
        let l = layout_card(&p, &m);
        assert_tidy(&l);
        assert!(l.w <= STACK_W && l.sub_h > SUB_PITCH && l.buttons.len() == 3);
        // labels that cannot fit even a full row are shrunk, never pushed out of the card
        let wide = ask("Pick one", "", &["A".repeat(60).as_str(), "B".repeat(60).as_str(), "C".repeat(60).as_str()]);
        assert_tidy(&layout_card(&wide, &m));
    }

    const LONG_Q: &str = "Какой из подходов выбрать для нового слоя хранения данных: оставить текущую базу, перейти на отдельный сервис или хранить всё в файлах рядом с проектом, учитывая, что синхронизация между машинами понадобится позже?";
    const LONG_OPT: &str = "Перейти на отдельный сервис и переписать слой доступа к данным целиком, с миграцией старых записей и временной поддержкой обоих форматов";

    #[test]
    fn long_options_stack_one_full_width_button_each_and_the_card_grows_to_fit() {
        let m = Mono(0.5);
        let c = ask(LONG_Q, "Claude Code спрашивает", &[LONG_OPT, "Оставить текущую базу и ничего не менять, пока не станет понятно, что нужно", "Хранить всё в файлах рядом с проектом, без отдельной базы", "Отложить решение"]);
        let l = layout_card(&c, &m);
        assert_tidy(&l);
        assert!(l.stacked && l.w <= STACK_W && l.h <= CARD_MAX_H, "{l:?}");
        assert_eq!(l.buttons.len(), 4);
        assert!(l.buttons.iter().all(|b| b.x == 20.0 && (b.w - (l.w - 40.0)).abs() < 1e-3), "one full-width button per option: {l:?}");
        assert!(l.buttons.windows(2).all(|p| (p[1].y - p[0].bottom() - 8.0).abs() < 1e-3), "evenly spaced, one under the other");
        assert!(l.buttons[0].h > 32.0 && l.buttons[3].h == 32.0, "a long label makes its button taller, a short one stays one line: {l:?}");
        assert!(l.title_h >= 3.0 * TITLE_PITCH, "the whole question has its lines");
        assert!((l.buttons[3].bottom() + 12.0 - l.h).abs() < 1e-3, "the card ends right under the last button");
        // short labels beside each other, long ones under each other
        let near = layout_card(&ask(LONG_Q, "", &["Да", "Нет"]), &m);
        assert!(near.buttons[0].y == near.buttons[1].y && near.buttons[0].h == 32.0);
    }

    #[test]
    fn a_card_never_outgrows_the_screens_limit_the_text_gives_lines_up_first() {
        let m = Mono(0.5);
        let c = ask(&LONG_Q.repeat(4), "Claude Code спрашивает", &[LONG_OPT, LONG_OPT, LONG_OPT, LONG_OPT]);
        let full = layout_card_open(&c, &m, false, 5000.0);
        assert!(full.h > 400.0, "unlimited it is tall: {}", full.h);
        for cap in [420.0, 300.0, 240.0] {
            let l = layout_card_open(&c, &m, false, cap);
            assert_tidy(&l);
            assert!(l.h <= cap + 1e-3, "{cap}: {}", l.h);
            assert!(l.title_h >= TITLE_PITCH && l.sub_h >= SUB_PITCH, "at least a line of each stays");
        }
    }

    #[test]
    fn text_that_does_not_fit_its_lines_is_cut_with_an_ellipsis() {
        assert_eq!(clip_lines("short", 40.0, 100.0, 1.0), "short");
        let s = "word ".repeat(40);
        let cut = clip_lines(&s, 400.0, 100.0, 2.0);
        assert!(cut.ends_with('\u{2026}') && cut.chars().count() < s.chars().count() / 2, "{cut}");
        assert!(clip_lines("x", 1000.0, 1.0, 1.0).ends_with('\u{2026}'), "never empty, never a panic");
    }

    #[test]
    fn a_card_that_fits_one_row_keeps_it() {
        let m = Mono(0.5);
        let l = layout_card(&ask("Allow Bash?", "ls", &["Deny", "Allow"]), &m);
        assert!(l.w <= MAX_W && l.h == 56.0 && l.buttons.iter().all(|b| b.y == l.buttons[0].y && b.x > l.text.x), "{l:?}");
        assert_eq!((l.title_h, l.sub_h), (TITLE_PITCH, SUB_PITCH));
    }
}
