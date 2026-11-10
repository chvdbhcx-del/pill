//! Measuring and painting the rows of a card. A row is the web page's `.row`: 56 px minimum, 16 px side padding, label (13/500) over a hint
//! (11, dim) on the left, the control on the right; "block" rows put the control under the text. Each row owns one retained glasskit control.
use super::page::*;
use super::pal::Pal;
use super::txt;
use super::ui::{key, Act};
use super::SettingsView;
use glasskit::canvas::Canvas;
use glasskit::controls::{Button, ButtonKind, Widget};
use super::flow::{from_to, Pulse};
use crate::ui::liquid::{Feel, Flow};
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::settings::flows::Kind;
use pill_ui_core::settings::schema::{ButtonStyle, Control, TextKind};
use pill_ui_core::settings::tr::Lang;

/// Everything a paint pass needs besides the view itself.
pub struct Dc<'a> {
    pub cv: &'a Canvas<'a>,
    pub th: &'a Theme,
    pub p: Pal,
    pub lang: Lang,
    pub now_ms: f64,
    pub reduced: bool,
}

pub const CARD_W: f32 = 532.0;
pub const PAD_X: f32 = 16.0;

pub fn st_title() -> Style {
    Style::new(13.0, 500)
}
pub fn st_hint() -> Style {
    Style::new(11.0, 400)
}
pub fn st_body() -> Style {
    Style::new(13.0, 400)
}

pub fn msg_color(p: &Pal, k: Kind) -> glasskit::color::Color {
    match k {
        Kind::Plain => p.dim,
        Kind::Ok => p.green,
        Kind::Bad => p.red,
        Kind::Warn => p.amber,
    }
}

/// Width the key caps of a combo take (each cap: text + 14 padding, min 22; 4 px gaps; 5 px inset), at least 160.
pub fn keys_w(cv: &Canvas, combo: &str) -> f32 {
    let caps = pill_ui_core::settings::hotkey::key_caps(combo);
    if caps.is_empty() {
        return 160.0;
    }
    let st = Style::new(11.0, 500);
    let w: f32 = caps.iter().map(|c| (cv.text_width(pill_ui_core::settings::hotkey::cap_text(c), st) + 14.0).max(22.0)).sum::<f32>() + 4.0 * (caps.len() as f32 - 1.0) + 10.0;
    w.max(160.0)
}

pub fn btn_style(s: ButtonStyle) -> (ButtonKind, bool) {
    match s {
        ButtonStyle::Normal => (ButtonKind::Normal, false),
        ButtonStyle::Small => (ButtonKind::Normal, true),
        ButtonStyle::Primary => (ButtonKind::Primary, false),
        ButtonStyle::Danger => (ButtonKind::Danger, false),
    }
}

impl SettingsView {
    /// The control's size for a row of text width `w` (inline) and whether the row puts it under the text.
    pub(super) fn ctl_size(&mut self, d: &Dc, r: &RowRt, inner_w: f32) -> (f32, f32, bool) {
        let block_flag = r.row.is_block(d.lang);
        match &r.ctl {
            Ctl::None => (0.0, 0.0, false),
            Ctl::Toggle(_) | Ctl::Backend(..) => (d.th.m.sw_w, d.th.m.sw_h, false),
            Ctl::Slider(_) => (140.0 + 16.0 + 48.0, 16.0, false),
            Ctl::Seg(s) => {
                let (w, h) = s.natural_size(d.cv, d.th);
                (w, h, block_flag)
            }
            Ctl::Select(s) => {
                let (w, _) = s.natural_size(d.cv, d.th);
                (w.clamp(230.0, 250.0), 32.0, block_flag)
            }
            Ctl::Hotkey => {
                let action = match &r.row.control {
                    Control::Hotkey { action } => *action,
                    _ => "",
                };
                let combo = self.model.combo_of(action);
                let live = self.rec_caps(action);
                let w = if live.is_empty() { keys_w(d.cv, &combo) } else { 160.0f32.max(live.len() as f32 * 40.0) };
                (w, 32.0, false)
            }
            Ctl::Keys(c) => (keys_w(d.cv, c), 32.0, false),
            Ctl::Field(f) => match f.kind {
                FK::Lines { rows, .. } => (inner_w, (rows as f32 * 20.0 + 16.0).max(80.0), true),
                FK::Secret => (inner_w, 32.0, true),
                FK::Number | FK::Time => (110.0, 32.0, block_flag),
                FK::Text => {
                    let long = matches!(&r.row.control, Control::Text { kind: TextKind::Url | TextKind::Path | TextKind::Host, .. }) || block_flag;
                    if long {
                        (inner_w, 32.0, true)
                    } else {
                        (230.0, 32.0, false)
                    }
                }
            },
            Ctl::Week(_) => (inner_w, 30.0, true),
            Ctl::Button => {
                let (label, style) = match &r.row.control {
                    Control::Button { label, style, .. } => (label.get(d.lang), *style),
                    _ => ("", ButtonStyle::Normal),
                };
                let k = key(&format!("rowbtn:{}", r.row.id));
                let (kind, small) = btn_style(style);
                let (cv, th) = (d.cv, d.th);
                let b = self.ui.button(k, || Button::new(label).kind(kind).small(small));
                b.label = label.to_string();
                b.kind = kind;
                b.small = small;
                let (w, _) = b.natural_size(cv, th);
                (w, 32.0, false)
            }
            Ctl::Custom(_) => (0.0, 0.0, false),
        }
    }
}

/// Measured pieces of one row.
#[derive(Clone, Copy, Default)]
pub struct RowM {
    pub nh: f32,
    pub cw: f32,
    pub ch: f32,
    pub block: bool,
    pub title_h: f32,
    pub hint_h: f32,
    pub msg_h: f32,
    /// the height the message / status lines would take in full (`msg_h` flows towards it)
    pub msg_need: f32,
    pub label_w: f32,
    /// y of the label column's first line relative to the row top
    pub top: f32,
}

impl SettingsView {
    pub(super) fn row_metrics(&mut self, d: &Dc, r: &RowRt, inner_w: f32) -> RowM {
        if let Ctl::Custom(name) = &r.ctl {
            let h = self.custom_h(d, r, name, inner_w + 2.0 * PAD_X);
            return RowM { nh: h, ..RowM::default() };
        }
        let (cw, ch, block) = self.ctl_size(d, r, inner_w);
        let label_w = if block || cw == 0.0 { inner_w } else { (inner_w - cw - 16.0).max(80.0) };
        let title = r.row.label.get(d.lang);
        let hint = r.row.hint.get(d.lang);
        let title_h = txt::wrapped_h(d.cv, title, st_title(), label_w, 18.0).max(18.0);
        let hint_h = if hint.is_empty() { 0.0 } else { 2.0 + txt::wrapped_h(d.cv, hint, st_hint(), label_w, 16.0) };
        let mut msg_h = 0.0;
        if let Some(m) = r.live_msg() {
            if !m.text.is_empty() {
                msg_h += 2.0 + txt::wrapped_h(d.cv, &m.text, st_hint(), label_w, 16.0);
            }
        }
        if let Some(m) = &r.status {
            if !m.text.is_empty() {
                msg_h += 2.0 + txt::wrapped_h(d.cv, &m.text, st_hint(), label_w, 16.0);
            }
        }
        // what the lines need vs what is shown now: the shown height flows towards the need (see `paint_card`)
        let msg_need = msg_h;
        if r.mh_seen {
            msg_h = r.mh.at().max(0.0);
        }
        let text_h = title_h + hint_h + msg_h;
        let nh = if block { 11.0 + text_h + 10.0 + ch + 14.0 } else { (11.0 + text_h + 11.0).max(56.0).max(ch + 24.0) };
        RowM { nh, cw, ch, block, title_h, hint_h, msg_h, msg_need, label_w, top: 11.0 }
    }

    /// Paints the label column of a row: title, hint, message and status lines.
    pub(super) fn paint_label(&mut self, d: &Dc, r: &RowRt, m: &RowM, x: f32, y: f32) {
        let (cv, p) = (d.cv, &d.p);
        let mut yy = y + m.top;
        cv.text(Rect::new(x, yy, m.label_w, m.title_h), r.row.label.get(d.lang), Style { wrap: true, ..st_title() }, p.text);
        yy += m.title_h;
        let hint = r.row.hint.get(d.lang);
        if !hint.is_empty() {
            yy += 2.0;
            let h = txt::wrapped_h(cv, hint, st_hint(), m.label_w, 16.0);
            txt::para(cv, Rect::new(x, yy, m.label_w, h), hint, st_hint(), p.dim);
            yy += h;
        }
        if let Some(s) = &r.status {
            if !s.text.is_empty() {
                yy += 2.0;
                let h = txt::wrapped_h(cv, &s.text, st_hint(), m.label_w, 16.0);
                let k = r.grow();
                cv.with_opacity(k, |cv| txt::para(cv, Rect::new(x, yy, m.label_w, h), &s.text, st_hint(), msg_color(p, s.kind)));
                yy += h;
            }
        }
        if let Some((s, f)) = &r.msg {
            if !s.text.is_empty() {
                let (k, rise) = (f.at01(), (1.0 - f.at()) * 6.0);
                yy += 2.0;
                let h = txt::wrapped_h(cv, &s.text, st_hint(), m.label_w, 16.0);
                cv.with_opacity(k, |cv| txt::para(cv, Rect::new(x, yy + rise, m.label_w, h), &s.text, st_hint(), msg_color(p, s.kind)));
            }
        }
    }
}

/// A message under a row's hint: fades in and rises 6 px (it flows from 0 to 1).
pub fn new_msg_flow(reduce: bool) -> Flow {
    let mut f = Flow::new(0.0, Feel::FLOW);
    from_to(&mut f, 0.0, 1.0, reduce);
    f
}

pub fn act_for_row(r: &RowRt) -> Act {
    Act::RowButton(r.row.id.to_string())
}
