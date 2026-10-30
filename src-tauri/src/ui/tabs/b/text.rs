//! The Text tab view (src/tab-text.js + tab-text.css): a text area with a stats foot on the left; on the right a Tools / AI switch over a
//! scrolling grid of 19 text tools (or the 9 AI tools plus the "own request" line); a QR overlay and a toast over the text area.
//!
//! The model is `pill_ui_core::panel::b::text::TextModel`. The text lives in the `Field` (a glasskit `TextInput`, so undo / selection /
//! IME-less typing are the kit's); the view mirrors `(text, selection)` into the model as `Action::Input` and applies the model's
//! `Effect::ReplaceRange` through `TextInput::insert` (one undo step, so Ctrl+Z restores the original after a tool or an AI answer).
//!
//! Input timing: `View::event` has no `Canvas`, so text input is queued in the `Field` and applied at the start of the next `draw`; the step
//! after that mirrors it into the model. Buttons answer at once (their rectangles come from the last draw).
//! Caret blink: the view has no timer, so a focused field keeps frames coming (not under reduced motion, where the caret is solid).
use super::kit::{self, focus_one, lang_of, ms_of, route_effects, Calls, Clicks, Cues, Field, Icons};
use crate::ui::view::{Cx, Msg, View};
use glasskit::canvas::Canvas;
use glasskit::ease::{curves, dur, Tween};
use glasskit::edit::EditEvent;
use glasskit::geom::{Pt, Rect};
use glasskit::input::{InputEvent, KeyCode, Response};
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::text::{Action, Mode, QrView, TextModel, TextView, AI_TOOLS, TOOLS};
use pill_ui_core::panel::b::{Effect, Lang};

// ---------------------------------------------------------------------------------------------------------------------------------------
// Geometry (tab-text.css: two columns, the right one 262 px, gap 12; 32 px tiles, gap 8; 34 px switch; 24 px foot)
// ---------------------------------------------------------------------------------------------------------------------------------------

const RIGHT_W: f32 = 262.0;
const COL_GAP: f32 = 12.0;
const TILE_W: f32 = 120.0;
const TILE_H: f32 = 32.0;
const TILE_GAP: f32 = 8.0;
const SEG_H: f32 = 34.0;
const FOOT_H: f32 = 24.0;
const QR_SIDE: f32 = 190.0;

/// Every rectangle of the tab for a given body rectangle.
#[derive(Clone, Copy, Debug)]
struct Lay {
    area: Rect,
    foot: Rect,
    seg: Rect,
    grid: Rect,
}

impl Lay {
    fn new(r: Rect) -> Lay {
        let right_w = RIGHT_W.min(r.w * 0.45);
        let left_w = r.w - right_w - COL_GAP;
        let area = Rect::new(r.x, r.y, left_w, (r.h - FOOT_H - TILE_GAP).max(0.0));
        let foot = Rect::new(r.x, area.bottom() + TILE_GAP, left_w, FOOT_H);
        let rx = r.x + left_w + COL_GAP;
        let seg = Rect::new(rx, r.y, right_w, SEG_H);
        let grid = Rect::new(rx, seg.bottom() + TILE_GAP, right_w, (r.h - SEG_H - TILE_GAP).max(0.0));
        Lay { area, foot, seg, grid }
    }

    /// The two buttons of the foot (paste, copy), icons 16 px in 24 px buttons, ink 16 px from the right edge.
    fn foot_btns(&self) -> (Rect, Rect) {
        let copy = Rect::new(self.foot.right() - 12.0 - 24.0, self.foot.y, 24.0, 24.0);
        (Rect::new(copy.x - 4.0 - 24.0, self.foot.y, 24.0, 24.0), copy)
    }

    fn seg_btn(&self, i: usize) -> Rect {
        let w = (self.seg.w - 12.0) / 2.0;
        Rect::new(self.seg.x + 4.0 + i as f32 * (w + 4.0), self.seg.y + 4.0, w, 26.0)
    }

    /// Tile `i` (0-based, tools grid) at scroll offset `off`, in window coordinates.
    fn tile(&self, i: usize, off: f32, first_row: f32) -> Rect {
        let (c, r) = ((i % 2) as f32, (i / 2) as f32);
        Rect::new(self.grid.x + c * (TILE_W + TILE_GAP), self.grid.y + first_row + r * (TILE_H + TILE_GAP) - off, TILE_W, TILE_H)
    }

    /// The "own request" row of the AI grid: input + 32 px button.
    fn ask_row(&self, off: f32) -> (Rect, Rect) {
        let w = 2.0 * TILE_W + TILE_GAP;
        let go = Rect::new(self.grid.x + w - 32.0, self.grid.y - off, 32.0, 32.0);
        (Rect::new(self.grid.x, self.grid.y - off, w - 32.0 - TILE_GAP, 32.0), go)
    }

    fn tools_content_h() -> f32 {
        let rows = TOOLS.len().div_ceil(2) as f32;
        rows * TILE_H + (rows - 1.0) * TILE_GAP
    }

    fn ai_content_h() -> f32 {
        let rows = AI_TOOLS.len().div_ceil(2) as f32;
        TILE_H + TILE_GAP + rows * TILE_H + (rows - 1.0) * TILE_GAP
    }

    /// The overlay card of the QR code: the canvas centred in the area.
    fn qr_card(&self, side: f32) -> Rect {
        self.area.centered(side, side)
    }

    fn qr_close(&self) -> Rect {
        Rect::new(self.area.right() - 12.0 - 28.0, self.area.y + 12.0, 28.0, 28.0)
    }
}

/// Horizontal runs of dark modules as `(col, row, run length)`; fewer rects than modules and no seams between neighbours.
pub fn qr_runs(rows: &[String]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let b = row.as_bytes();
        let mut x = 0;
        while x < b.len() {
            if b[x] == b'#' {
                let s = x;
                while x < b.len() && b[x] == b'#' {
                    x += 1;
                }
                out.push((s, y, x - s));
            } else {
                x += 1;
            }
        }
    }
    out
}

/// Snap a logical coordinate to the device pixel grid.
fn snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum K {
    Tool(usize),
    Ai(usize),
    ModeTools,
    ModeAi,
    Paste,
    Copy,
    QrClose,
    AskGo,
}

pub struct TextScreen {
    model: TextModel,
    view: TextView,
    calls: Calls,
    clicks: Clicks<K>,
    cues: Cues,
    ic: Icons,
    area: Field,
    ask: Field,
    sv: [ScrollView; 2],
    mode_t: Tween<f32>,
    qr_t: Tween<f32>,
    qr_last: Option<QrView>,
    toast_t: Tween<f32>,
    toast_last: (String, bool),
    carry: f64,
    last_sel: (usize, usize),
    text_dirty: bool,
    dirty: bool,
    reduced: bool,
    lang: Lang,
    lay: Lay,
    drag: Option<u8>,
    /// IME candidate window position computed by the last draw
    ime: Option<Rect>,
    now_ms: f64,
}

impl Default for TextScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TextScreen {
    pub fn new() -> TextScreen {
        let th = Theme::pill_dark();
        let model = TextModel::new(Lang::En);
        let view = model.view();
        let mut ic = Icons::new();
        ic.add("t-ai", r#"<path d="M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8z"/><path d="M18.5 15v4M16.5 17h4"/>"#);
        ic.add("t-paste", r#"<rect x="5" y="4" width="14" height="17" rx="2.5"/><path d="M9 4.5h6M12 17v-6M9.5 13.5 12 11l2.5 2.5"/>"#);
        ic.add("t-copy", r#"<rect x="9" y="9" width="11" height="11" rx="2.5"/><path d="M15 9V6.5A2.5 2.5 0 0 0 12.5 4h-6A2.5 2.5 0 0 0 4 6.5v6A2.5 2.5 0 0 0 6.5 15H9"/>"#);
        ic.add("t-x", r#"<path d="M6 6l12 12M18 6 6 18"/>"#);
        let mut area = Field::new(true, &th);
        area.ti.pad = (16.0, 16.0);
        area.ti.radius = 16.0;
        area.ti.style = Style::new(14.0, 500);
        let mut ask = Field::new(false, &th);
        ask.ti.radius = 10.0;
        ask.ti.style = Style::new(12.0, 400);
        let mut s = TextScreen {
            model,
            view,
            calls: Calls::new(),
            clicks: Clicks::new(),
            cues: Cues::new(),
            ic,
            area,
            ask,
            sv: [ScrollView::new(), ScrollView::new()],
            mode_t: Tween::at_rest(0.0),
            qr_t: Tween::at_rest(0.0),
            qr_last: None,
            toast_t: Tween::at_rest(0.0),
            toast_last: (String::new(), false),
            carry: 0.0,
            last_sel: (0, 0),
            text_dirty: false,
            dirty: true,
            reduced: false,
            lang: Lang::En,
            lay: Lay::new(Rect::new(0.0, 0.0, 648.0, 260.0)),
            drag: None,
            ime: None,
            now_ms: 0.0,
        };
        s.refresh();
        s
    }

    /// Pull language and reduced motion from the host context (cheap; called by every entry point).
    fn ctx(&mut self, cx: &Cx) {
        let lang = lang_of(cx);
        if lang != self.lang {
            self.lang = lang;
            self.model.set_lang(lang);
            self.dirty = true;
        }
        if cx.reduced_motion != self.reduced || self.model.fx.reduce_motion != cx.reduced_motion {
            self.reduced = cx.reduced_motion;
            self.model.fx.reduce_motion = self.reduced;
            self.clicks.reduced = self.reduced;
            self.cues.reduced = self.reduced;
            for f in [&mut self.area, &mut self.ask] {
                f.ti.reduced = Some(self.reduced);
            }
            for v in &mut self.sv {
                v.state.reduced = Some(self.reduced);
                v.bar.reduced = Some(self.reduced);
            }
        }
    }

    fn refresh(&mut self) {
        self.view = self.model.view();
        self.area.ti.placeholder = self.view.placeholder.clone();
        self.ask.ti.placeholder = self.view.ask_placeholder.clone();
        if let Some(q) = &self.view.qr {
            self.qr_last = Some(q.clone());
        }
        if let Some(t) = &self.view.toast {
            self.toast_last = t.clone();
        }
        self.dirty = false;
    }

    fn go(&mut self, to: &mut Tween<f32>, v: f32, d: f32) {
        if self.reduced {
            to.jump(v);
        } else {
            to.retarget(v, d, curves::EASE_OUT);
        }
    }

    /// Feed one action to the model and run what it asks for.
    fn act(&mut self, a: Action, cx: &mut Cx) {
        let calls = self.model.on_action(a);
        self.calls.run(cx, calls);
        self.dirty = true;
    }

    fn click(&mut self, k: K, cx: &mut Cx) {
        match k {
            K::Tool(i) => {
                if let Some(b) = self.view.tools.get(i) {
                    let t = b.tool;
                    self.act(Action::Run(t), cx);
                }
            }
            K::Ai(i) => {
                if let Some(b) = self.view.ai.get(i) {
                    let t = b.tool;
                    self.act(Action::AiRun(t), cx);
                }
            }
            K::ModeTools => self.set_mode(Mode::Tools, cx),
            K::ModeAi => self.set_mode(Mode::Ai, cx),
            K::Paste => {
                let t = cx.clipboard_text();
                self.act(Action::Paste(t), cx);
            }
            K::Copy => self.act(Action::Copy, cx),
            K::QrClose => self.act(Action::CloseQr, cx),
            K::AskGo => self.act(Action::AskGo, cx),
        }
    }

    fn set_mode(&mut self, m: Mode, cx: &mut Cx) {
        if self.model.mode() != m {
            if m == Mode::Tools {
                self.ask.ti.set_focused(false);
            }
            self.act(Action::SetMode(m), cx);
        }
    }

    /// Text area / ask line -> model; model effects -> text area.
    fn sync(&mut self, cx: &mut Cx) {
        let mut evs = self.area.take_events();
        evs.extend(self.area.ti.take_events());
        for e in evs {
            match e {
                EditEvent::Changed => self.text_dirty = true,
                EditEvent::Cancel if self.view.qr.is_some() => self.act(Action::CloseQr, cx),
                _ => {}
            }
        }
        let (a, b) = self.area.ti.selection();
        let sel = (a.min(b), a.max(b));
        if self.text_dirty || sel != self.last_sel {
            self.text_dirty = false;
            self.last_sel = sel;
            let text = self.area.ti.text();
            self.act(Action::Input { text, sel }, cx);
        }
        let mut evs = self.ask.take_events();
        evs.extend(self.ask.ti.take_events());
        for e in evs {
            match e {
                EditEvent::Changed => {
                    let q = self.ask.ti.text();
                    self.act(Action::AskChanged(q), cx);
                }
                EditEvent::Submit => {
                    let q = self.ask.ti.text();
                    self.act(Action::AskChanged(q), cx);
                    self.act(Action::AskGo, cx);
                }
                _ => {}
            }
        }
    }

    fn drain_model(&mut self, cx: &mut Cx) {
        let fx = route_effects(self.model.take_effects(), cx);
        for e in fx {
            if let Effect::ReplaceRange { start, end, text } = e {
                self.area.ti.set_selection(start, end);
                self.area.ti.insert(&text);
                // the mirror is already right; a Changed event follows and re-sends the (normalised) text
                self.text_dirty = true;
            }
        }
        self.cues.play_all(self.model.take_anims());
    }

    #[cfg(test)]
    fn settle(&mut self, cx: &mut Cx) {
        for _ in 0..8 {
            self.step(1000.0, cx);
        }
    }
}

impl View for TextScreen {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.ctx(cx);
        // Esc closes the QR overlay
        if let Some((KeyCode::Escape, _, false)) = ev.key_down() {
            if self.view.qr.is_some() {
                self.click(K::QrClose, cx);
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
        }
        match ev {
            InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_) => {
                for f in [&mut self.area, &mut self.ask] {
                    if f.is_focused() {
                        return f.queue(ev);
                    }
                }
                Response::NONE
            }
            InputEvent::MouseDown { .. } | InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. } | InputEvent::Wheel { .. } | InputEvent::MouseLeave => self.pointer(ev, cx),
            _ => Response::NONE,
        }
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.ctx(cx);
        let dt = (dt_ms / 1000.0) as f32;
        self.sync(cx);
        let ms = ms_of(dt_ms, &mut self.carry);
        if ms > 0 {
            self.model.tick(ms);
        }
        self.drain_model(cx);
        if self.dirty || self.view.toast.is_some() || self.toast_t.value() > 0.0 {
            self.refresh();
        }
        // animation targets follow the model
        let mode_to = (self.view.mode == Mode::Ai) as u8 as f32;
        if self.mode_t.target() != mode_to {
            let mut t = self.mode_t.clone();
            self.go(&mut t, mode_to, dur::MED);
            self.mode_t = t;
        }
        let qr_to = self.view.qr.is_some() as u8 as f32;
        if self.qr_t.target() != qr_to {
            let mut t = self.qr_t.clone();
            self.go(&mut t, qr_to, if qr_to > 0.0 { dur::MED } else { dur::EXIT });
            self.qr_t = t;
        }
        let toast_to = self.view.toast.is_some() as u8 as f32;
        if self.toast_t.target() != toast_to {
            let mut t = self.toast_t.clone();
            self.go(&mut t, toast_to, if toast_to > 0.0 { dur::MED } else { dur::EXIT });
            self.toast_t = t;
        }
        let mut busy = self.clicks.step(dt);
        busy |= self.cues.step(dt_ms as f32);
        busy |= self.mode_t.step(dt);
        busy |= self.qr_t.step(dt);
        busy |= self.toast_t.step(dt);
        for v in &mut self.sv {
            busy |= v.step(dt);
        }
        busy |= self.area.ti.step(dt);
        busy |= self.ask.ti.step(dt);
        // queued input / just-produced edit events need another frame; AI busy pulses; a toast counts down; a focused caret blinks
        busy |= self.area.has_input() || self.ask.has_input();
        busy |= self.model.is_ai_busy() || self.view.toast.is_some();
        busy |= !self.reduced && (self.area.is_focused() || self.ask.is_focused());
        busy |= self.text_dirty || self.dirty;
        if let Some(r) = self.ime.take() {
            cx.set_ime_rect(r);
        }
        busy
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.ctx(cx);
        if let Msg::Reply { ticket, result } = msg {
            if let Some((tag, reply)) = self.calls.take(ticket, result) {
                let calls = self.model.on_reply(tag, reply);
                self.calls.run(cx, calls);
                self.dirty = true;
            }
        }
        Response::redraw()
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.ctx(cx);
        self.dirty = true;
        cx.request_frame();
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.reduced = cx.reduced_motion;
        self.now_ms = cx.now_ms;
        let lay = Lay::new(rect);
        self.lay = lay;
        self.clicks.begin();
        self.draw_left(cv, th, &lay, cx);
        self.draw_right(cv, th, &lay);
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------------------------------------------------------------------

impl TextScreen {
    fn grid_idx(&self) -> usize {
        (self.view.mode == Mode::Ai) as usize
    }

    fn ask_visible(&self, p: Pt) -> bool {
        self.view.mode == Mode::Ai && self.lay.grid.contains(p.0, p.1) && self.ask.rect.contains(p.0, p.1)
    }

    fn pointer(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        // a drag that started in a field stays with it
        if let Some(which) = self.drag {
            let f = if which == 0 { &mut self.area } else { &mut self.ask };
            let r = f.queue(ev);
            if matches!(ev, InputEvent::MouseUp { .. }) {
                self.drag = None;
            }
            return r;
        }
        let (r, clicked) = self.clicks.event(ev);
        if let Some(k) = clicked {
            self.click(k, cx);
        }
        if r.consumed {
            return r;
        }
        let pos = ev.pos();
        let over_qr = self.view.qr.is_some() && pos.is_some_and(|p| self.lay.area.contains(p.0, p.1));
        if over_qr {
            // the overlay swallows clicks and wheel over the text area
            return Response { consumed: matches!(ev, InputEvent::MouseDown { .. } | InputEvent::Wheel { .. }), ..r };
        }
        if let (InputEvent::MouseDown { .. }, Some(p)) = (ev, pos) {
            if self.lay.area.contains(p.0, p.1) {
                self.drag = Some(0);
                return self.area.queue(ev);
            }
            if self.ask_visible(p) {
                self.drag = Some(1);
                return self.ask.queue(ev);
            }
        }
        if let Some(p) = pos {
            if matches!(ev, InputEvent::MouseMove { .. }) && r.cursor.is_none() {
                if self.lay.area.contains(p.0, p.1) || self.ask_visible(p) {
                    return Response { cursor: Some(glasskit::input::Cursor::Text), ..r };
                }
            }
            if matches!(ev, InputEvent::Wheel { .. }) && self.lay.area.contains(p.0, p.1) {
                return self.area.queue(ev);
            }
        }
        // wheel / scrollbar of the visible grid
        let i = self.grid_idx();
        let rs = self.sv[i].event(ev, self.lay.grid);
        r.merge(rs)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------------------------------------------------------------------

impl TextScreen {
    fn draw_left(&mut self, cv: &Canvas, th: &Theme, lay: &Lay, cx: &Cx) {
        let _ = cx;
        // text area (queued input first, so typing shows in this very frame)
        let a_focus = self.area.pump(cv, lay.area);
        self.area.ti.draw(cv, lay.area, th);
        if self.area.is_focused() {
            self.ime = self.area.ime_rect(cv);
        }
        // QR overlay
        let qt = self.qr_t.value().clamp(0.0, 1.0);
        if qt > 0.0 {
            if let Some(q) = self.qr_last.clone() {
                self.draw_qr(cv, th, lay, &q, qt);
            }
        }
        // toast
        let tt = self.toast_t.value().clamp(0.0, 1.0);
        if tt > 0.0 {
            let (text, bad) = self.toast_last.clone();
            let st = th.small(600);
            let w = (cv.text_width(&text, st) + 24.0).min(lay.area.w * 0.9);
            let r = Rect::new(lay.area.cx() - w / 2.0, lay.area.bottom() - 12.0 - 32.0, w, 32.0);
            cv.with_opacity(tt, |cv| {
                cv.with_offset(0.0, 8.0 * (1.0 - curves::SPRING_SETTLE.at(tt)), |cv| {
                    cv.shadow_rr(r, 12.0, 10.0, 4.0, th.shadow);
                    let (bg, fg) = if bad { (Color::rgb8(0x5a, 0x1f, 0x24), Color::rgb8(0xff, 0xd9, 0xdc)) } else { (th.raised2, th.text) };
                    cv.fill_rr(r, 12.0, bg);
                    cv.text(r.inset_xy(12.0, 0.0), &text, st.ellipsis().center(), fg);
                });
            });
        }
        // foot: the counter and the paste / copy buttons
        let (paste, copy) = lay.foot_btns();
        let count = Rect::new(lay.foot.x + 16.0, lay.foot.y, (paste.x - 4.0 - lay.foot.x - 16.0).max(0.0), FOOT_H);
        cv.text(count, &self.view.count_line, Style::new(11.0, 400).mono().ellipsis(), th.dim);
        for (k, r, icon) in [(K::Paste, paste, "t-paste"), (K::Copy, copy, "t-copy")] {
            self.clicks.add(k.clone(), r);
            let h = self.clicks.hover_amt(&k);
            let s = self.clicks.press_scale(&k, 0.92);
            cv.with_scale_about(r.center(), s, |cv| {
                cv.fill_rr(r, 8.0, th.card.fade(h));
                self.ic.draw_sw(cv, icon, r.centered(16.0, 16.0), th.dim.mix(th.text, h), 1.8);
            });
        }
        // the QR close button is a click target while the overlay is up
        if self.view.qr.is_some() {
            self.clicks.add(K::QrClose, lay.qr_close());
        }
        if a_focus {
            focus_one(&mut [&mut self.area, &mut self.ask], Some(0));
        }
    }

    fn draw_qr(&mut self, cv: &Canvas, th: &Theme, lay: &Lay, q: &QrView, t: f32) {
        cv.with_opacity(t, |cv| {
            cv.fill_rr(lay.area, 16.0, th.card.fade(0.92));
            let total = ((q.size + 2 * q.quiet) * q.px) as f32;
            let total = total.min(QR_SIDE.max(total));
            let card = lay.qr_card(total);
            let s = cv.scale;
            let card = Rect::new(snap(card.x, s), snap(card.y, s), total, total);
            let pop = 0.96 + 0.04 * curves::SPRING_SETTLE.at(t);
            cv.with_scale_about(card.center(), pop, |cv| {
                cv.with_clip_rr(card, 12.0, |cv| {
                    cv.fill_rect(card, Color::rgb8(255, 255, 255));
                    let px = q.px as f32;
                    for (x, y, run) in qr_runs(&q.rows) {
                        let x0 = card.x + (x + q.quiet) as f32 * px;
                        let y0 = card.y + (y + q.quiet) as f32 * px;
                        let (sx, sy) = (snap(x0, s), snap(y0, s));
                        cv.fill_rect(Rect::new(sx, sy, snap(x0 + run as f32 * px, s) - sx, snap(y0 + px, s) - sy), Color::rgb8(0, 0, 0));
                    }
                });
            });
            let r = lay.qr_close();
            let h = self.clicks.hover_amt(&K::QrClose);
            let ps = self.clicks.press_scale(&K::QrClose, 0.9);
            cv.with_scale_about(r.center(), ps, |cv| {
                cv.fill_rr(r, 14.0, th.raised2);
                cv.fill_rr(r, 14.0, th.hover.fade(h));
                self.ic.draw_sw(cv, "t-x", r.centered(14.0, 14.0), th.text, 1.8);
            });
        });
    }

    fn draw_right(&mut self, cv: &Canvas, th: &Theme, lay: &Lay) {
        // the Tools / AI switch with a sliding thumb
        cv.fill_rr(lay.seg, 12.0, th.card);
        let mt = self.mode_t.value();
        let (b0, b1) = (lay.seg_btn(0), lay.seg_btn(1));
        let thumb = Rect::new(b0.x + (b1.x - b0.x) * mt, b0.y, b0.w, b0.h);
        cv.fill_rr(thumb, 8.0, th.raised2);
        for (i, k) in [K::ModeTools, K::ModeAi].into_iter().enumerate() {
            let r = lay.seg_btn(i);
            self.clicks.add(k.clone(), r);
            let on = if i == 0 { 1.0 - mt } else { mt };
            let h = self.clicks.hover_amt(&k);
            let col = th.dim.mix(th.text, on.clamp(0.0, 1.0).max(h * 0.6));
            let s = self.clicks.press_scale(&k, 0.96);
            let st = th.small(600);
            cv.with_scale_about(r.center(), s, |cv| {
                if i == 0 {
                    cv.text(r, &self.view.tools_label, st.center(), col);
                } else {
                    let tw = cv.text_width(&self.view.ai_label, st);
                    let x0 = r.cx() - (14.0 + 6.0 + tw) / 2.0;
                    self.ic.draw_sw(cv, "t-ai", Rect::new(x0, r.cy() - 7.0, 14.0, 14.0), col, 1.8);
                    cv.text(Rect::new(x0 + 20.0, r.y, tw + 2.0, r.h), &self.view.ai_label, st, col);
                }
            });
        }
        // grids
        let tools_a = (1.0 - mt).clamp(0.0, 1.0);
        let ai_a = mt.clamp(0.0, 1.0);
        if tools_a > 0.001 {
            self.draw_tools(cv, th, lay, tools_a, (1.0 - tools_a) * -6.0);
        }
        if ai_a > 0.001 {
            self.draw_ai(cv, th, lay, ai_a, (1.0 - ai_a) * 6.0);
        }
    }

    fn tile(&mut self, cv: &Canvas, th: &Theme, k: K, r: Rect, view: Rect, label: &str, op: f32, active: bool) {
        // clip the click target to the visible part of the grid
        let (x0, y0, x1, y1) = (r.x.max(view.x), r.y.max(view.y), r.right().min(view.right()), r.bottom().min(view.bottom()));
        if active && x1 > x0 && y1 > y0 {
            self.clicks.add(k.clone(), Rect::new(x0, y0, x1 - x0, y1 - y0));
        }
        let h = self.clicks.hover_amt(&k);
        let down = self.clicks.is_down(&k);
        let mut s = self.clicks.press_scale(&k, 0.96);
        let id = match &k {
            K::Tool(i) => self.view.tools.get(*i).map(|b| b.tool.id()),
            K::Ai(i) => self.view.ai.get(*i).map(|b| ai_id(b.tool)),
            _ => None,
        };
        if let Some(id) = id {
            s *= self.cues.xf(id).scale;
        }
        cv.with_opacity(op, |cv| {
            cv.with_scale_about(r.center(), s, |cv| {
                let bg = if down { th.raised2 } else { th.card.mix(th.raised, h) };
                cv.fill_rr(r, 12.0, bg);
                cv.text(r.inset_xy(8.0, 0.0), label, th.small(600).center().ellipsis(), th.text);
            });
        });
    }

    fn draw_tools(&mut self, cv: &Canvas, th: &Theme, lay: &Lay, alpha: f32, dy: f32) {
        let view = lay.grid;
        self.sv[0].set_content_height(Lay::tools_content_h(), view.h);
        let off = self.sv[0].offset();
        let active = self.view.mode == Mode::Tools;
        let labels: Vec<String> = self.view.tools.iter().map(|b| b.label.clone()).collect();
        cv.with_opacity(alpha, |cv| {
            cv.with_offset(0.0, dy, |cv| {
                cv.with_clip(view, |cv| {
                    for (i, l) in labels.iter().enumerate() {
                        let r = lay.tile(i, off, 0.0);
                        if r.bottom() < view.y || r.y > view.bottom() {
                            continue;
                        }
                        self.tile(cv, th, K::Tool(i), r, view, l, 1.0, active);
                    }
                });
            });
        });
        self.sv[0].bar.draw(cv, view, &self.sv[0].state, th);
    }

    fn draw_ai(&mut self, cv: &Canvas, th: &Theme, lay: &Lay, alpha: f32, dy: f32) {
        let view = lay.grid;
        self.sv[1].set_content_height(Lay::ai_content_h(), view.h);
        let off = self.sv[1].offset();
        let active = self.view.mode == Mode::Ai;
        let busy = self.view.ai_busy;
        let any_running = self.view.ai.iter().any(|b| b.running);
        let pulse = 1.0 - 0.45 * (0.5 - 0.5 * (kit::phase(self.now_ms, 1100.0) * std::f32::consts::TAU).cos());
        let items: Vec<(String, bool)> = self.view.ai.iter().map(|b| (b.label.clone(), b.running)).collect();
        let (ask_r, go_r) = lay.ask_row(off);
        cv.with_opacity(alpha, |cv| {
            cv.with_offset(0.0, dy, |cv| {
                cv.with_clip(view, |cv| {
                    // the own-request line
                    let a_focus = self.ask.pump(cv, ask_r);
                    self.ask.ti.draw(cv, ask_r, th);
                    if a_focus && active {
                        focus_one(&mut [&mut self.area, &mut self.ask], Some(1));
                    }
                    let k = K::AskGo;
                    let vis = go_r.bottom() > view.y && go_r.y < view.bottom();
                    if active && vis {
                        self.clicks.add(k.clone(), Rect::new(go_r.x, go_r.y.max(view.y), go_r.w, go_r.bottom().min(view.bottom()) - go_r.y.max(view.y)));
                    }
                    let h = self.clicks.hover_amt(&k);
                    let s = self.clicks.press_scale(&k, 0.96);
                    let custom_run = busy && !any_running;
                    let op = if custom_run { pulse } else if busy { 0.45 } else { 1.0 };
                    cv.with_opacity(op, |cv| {
                        cv.with_scale_about(go_r.center(), s, |cv| {
                            let bg = if custom_run { th.raised2 } else { th.raised.mix(th.raised2, h) };
                            cv.fill_rr(go_r, 10.0, bg);
                            self.ic.draw_sw(cv, "t-ai", go_r.centered(14.0, 14.0), th.text, 1.8);
                        });
                    });
                    for (i, (label, running)) in items.iter().enumerate() {
                        let r = lay.tile(i, off, TILE_H + TILE_GAP);
                        if r.bottom() < view.y || r.y > view.bottom() {
                            continue;
                        }
                        let op = if *running { pulse } else if busy { 0.45 } else { 1.0 };
                        self.tile(cv, th, K::Ai(i), r, view, label, op, active);
                        if *running {
                            // the running tile keeps the pressed fill
                            cv.with_opacity(pulse, |cv| cv.fill_rr(r, 12.0, th.raised2.fade(0.5)));
                        }
                    }
                });
            });
        });
        self.sv[1].bar.draw(cv, view, &self.sv[1].state, th);
    }
}

fn ai_id(t: pill_ui_core::panel::b::text::AiTool) -> &'static str {
    t.id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::Sim;
    use crate::ui::tabs::b::kit::snap;
    use crate::ui::view::Services;
    use glasskit::input::{Buttons, Modifiers, MouseButton};
    use pill_ui_core::panel::b::text::{apply, stats, Tool};
    use serde_json::{json, Value};

    const FIXTURE: &str = include_str!("../../../../../tools/dev/fixtures/tabs-b/text.json");

    fn mv(p: Pt) -> InputEvent {
        InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE }
    }
    fn down(p: Pt) -> InputEvent {
        InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }
    }
    fn up(p: Pt) -> InputEvent {
        InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE }
    }
    fn click_at(s: &mut TextScreen, cx: &mut Cx, p: Pt) {
        s.event(&mv(p), cx);
        s.event(&down(p), cx);
        s.event(&up(p), cx);
        s.settle(cx);
    }

    /// A screen with `text` in the area and the selection `sel`, drawn once so the click targets exist.
    fn screen(text: &str, sel: Option<(usize, usize)>, cx: &mut Cx) -> TextScreen {
        let mut s = TextScreen::new();
        s.area.ti.set_text(text);
        if let Some((a, b)) = sel {
            s.area.ti.set_selection(a, b);
        }
        s.text_dirty = true;
        s.settle(cx);
        s
    }

    fn fixture(name: &str) -> Value {
        let v: Value = serde_json::from_str(FIXTURE).unwrap();
        v["scenarios"][name].clone()
    }

    const SAMPLE: &str = "ghbdtn  vbh\nGhbdtn vbh\n  b  \nb\na\nb";

    #[test]
    fn all_text_tools_act_like_text_apply_and_undo_restores() {
        for (i, tool) in TOOLS.iter().enumerate() {
            if matches!(tool, Tool::Qr | Tool::Count) {
                continue;
            }
            let (svc, _f) = Services::fake();
            let mut cx = Cx::headless_with(svc);
            let src = match tool {
                Tool::JPretty | Tool::JMin => r#"{"a":[1,2],"b":"x"}"#,
                Tool::B64d => "aGk=",
                Tool::UrlD => "a%20b%26c",
                _ => SAMPLE,
            };
            let mut s = screen(src, None, &mut cx);
            s.click(K::Tool(i), &mut cx);
            s.settle(&mut cx);
            let want = apply(*tool, src).unwrap();
            assert_eq!(s.area.ti.text(), want, "{}", tool.id());
            if want != src {
                assert!(s.area.ti.model.undo(), "{} is one undo step", tool.id());
                assert_eq!(s.area.ti.text(), src, "undo {}", tool.id());
            }
            s.area.ti.model.redo();
            s.settle(&mut cx);
            assert_eq!(s.model.text(), s.area.ti.text(), "{}: model mirror", tool.id());
        }
    }

    #[test]
    fn bad_input_flashes_an_error_and_keeps_the_text() {
        let (svc, _f) = Services::fake();
        let mut cx = Cx::headless_with(svc);
        let i = TOOLS.iter().position(|t| *t == Tool::JPretty).unwrap();
        let mut s = screen("{nope", None, &mut cx);
        s.click(K::Tool(i), &mut cx);
        s.settle(&mut cx);
        assert_eq!(s.area.ti.text(), "{nope");
        assert_eq!(s.view.toast, Some(("Not valid JSON".to_string(), true)));
    }

    #[test]
    fn a_tool_works_on_the_selection_only() {
        let (svc, _f) = Services::fake();
        let mut cx = Cx::headless_with(svc);
        let i = TOOLS.iter().position(|t| *t == Tool::Upper).unwrap();
        let mut s = screen("hello brave world", Some((6, 11)), &mut cx);
        s.click(K::Tool(i), &mut cx);
        s.settle(&mut cx);
        assert_eq!(s.area.ti.text(), "hello BRAVE world");
        assert!(s.area.ti.model.undo());
        assert_eq!(s.area.ti.text(), "hello brave world");
    }

    #[test]
    fn stats_tool_shows_the_count_in_a_toast_and_the_foot_line_follows_the_text() {
        let (svc, _f) = Services::fake();
        let mut cx = Cx::headless_with(svc);
        let i = TOOLS.iter().position(|t| *t == Tool::Count).unwrap();
        let mut s = screen("One two. Three!", None, &mut cx);
        s.click(K::Tool(i), &mut cx);
        s.step(16.0, &mut cx);
        let (t, bad) = s.view.toast.clone().unwrap();
        assert!(t.starts_with("3 words") && !bad, "{t}");
        let st = stats("One two. Three!");
        assert!(s.view.count_line.starts_with(&format!("{} chars", st.chars)), "{}", s.view.count_line);
    }

    #[test]
    fn qr_tool_opens_the_overlay_and_the_runs_cover_exactly_the_dark_modules() {
        let (svc, _f) = Services::fake();
        let mut cx = Cx::headless_with(svc);
        let i = TOOLS.iter().position(|t| *t == Tool::Qr).unwrap();
        let mut s = screen("https://example.com/pill", None, &mut cx);
        s.click(K::Tool(i), &mut cx);
        s.settle(&mut cx);
        let q = s.view.qr.clone().expect("qr view");
        let dark: usize = q.rows.iter().map(|r| r.matches('#').count()).sum();
        let runs = qr_runs(&q.rows);
        assert_eq!(runs.iter().map(|r| r.2).sum::<usize>(), dark);
        assert!(runs.len() < dark, "runs merge neighbours");
        assert!(s.qr_t.value() > 0.99);
        s.click(K::QrClose, &mut cx);
        s.settle(&mut cx);
        assert!(s.view.qr.is_none() && s.qr_t.value() < 0.01);
    }

    #[test]
    fn ai_tool_issues_ai_transform_and_applies_the_answer_with_one_undo() {
        let (svc, fakes) = Services::fake();
        fakes.api.reply("ai_transform", Ok(json!({ "ok": true, "text": "Better text" })));
        let (mut cx, q) = Cx::headless_queue(svc);
        let mut s = screen("bad txt", None, &mut cx);
        s.set_mode(Mode::Ai, &mut cx);
        s.click(K::Ai(1), &mut cx); // Fix errors
        let calls = fakes.api.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "ai_transform");
        assert_eq!(calls[0].args, json!({ "text": "bad txt", "action": "fix", "instruction": null }));
        assert!(s.model.is_ai_busy());
        for (_, m) in q.drain() {
            s.message(m, &mut cx);
        }
        s.settle(&mut cx);
        assert_eq!(s.area.ti.text(), "Better text");
        assert!(!s.model.is_ai_busy());
        assert!(s.area.ti.model.undo());
        assert_eq!(s.area.ti.text(), "bad txt");
    }

    #[test]
    fn a_second_ai_click_cancels_and_an_empty_text_asks_for_text() {
        let (svc, fakes) = Services::fake();
        let (mut cx, _q) = Cx::headless_queue(svc);
        let mut s = screen("", None, &mut cx);
        s.click(K::Ai(0), &mut cx);
        s.step(16.0, &mut cx);
        assert_eq!(s.view.toast.as_ref().map(|t| t.1), Some(true));
        assert!(fakes.api.calls().is_empty());
        s.area.ti.set_text("hello");
        s.text_dirty = true;
        s.step(16.0, &mut cx);
        s.click(K::Ai(0), &mut cx);
        s.click(K::Ai(0), &mut cx);
        let names: Vec<_> = fakes.api.calls().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["ai_transform", "ai_transform_cancel"]);
    }

    #[test]
    fn typing_and_the_own_request_line_go_through_the_deferred_input() {
        let (svc, fakes) = Services::fake();
        let mut sim = Sim::with_services(Box::new(TextScreen::new()), svc);
        sim.show();
        let th = Theme::pill_dark();
        let lay = Lay::new(Rect::new(0.0, 0.0, 648.0, 260.0));
        let c = lay.area.center();
        sim.send(&down(c));
        sim.send(&up(c));
        sim.render((648.0, 260.0), &th);
        for ch in "hola".chars() {
            sim.send(&InputEvent::Char(ch));
        }
        sim.render((648.0, 260.0), &th);
        sim.run(10);
        // switch to AI, type a request, Enter -> custom action
        let ai = lay.seg_btn(1).center();
        sim.send(&mv(ai));
        sim.send(&down(ai));
        sim.send(&up(ai));
        sim.run(120);
        sim.render((648.0, 260.0), &th);
        let (ask, _) = lay.ask_row(0.0);
        sim.send(&down(ask.center()));
        sim.send(&up(ask.center()));
        sim.render((648.0, 260.0), &th);
        for ch in "to German".chars() {
            sim.send(&InputEvent::Char(ch));
        }
        sim.send(&InputEvent::Key { code: KeyCode::Enter, mods: Modifiers::NONE, down: true, repeat: false });
        sim.render((648.0, 260.0), &th);
        sim.run(10);
        sim.pump();
        let calls = fakes.api.calls();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0].args, json!({ "text": "hola", "action": "custom", "instruction": "to German" }));
    }

    #[test]
    fn idle_view_asks_for_no_frames_and_reduced_motion_jumps() {
        let (svc, _f) = Services::fake();
        let mut cx = Cx::headless_with(svc);
        let mut s = screen("x", None, &mut cx);
        s.settle(&mut cx);
        assert!(!s.step(16.0, &mut cx), "settled and unfocused: idle");
        cx.reduced_motion = true;
        s.set_mode(Mode::Ai, &mut cx);
        s.step(16.0, &mut cx);
        assert_eq!(s.mode_t.value(), 1.0, "reduced motion: the switch jumps");
    }

    #[test]
    fn scenarios_fixture_parses() {
        let sc = fixture("tools");
        assert!(sc["text"].is_string());
        for n in ["tools", "qr", "ai-busy", "stats"] {
            assert!(fixture(n)["text"].is_string(), "{n}");
        }
    }

    // ------------------------------------------------------------ snapshots

    fn shoot_all(name: &str, mut s: TextScreen, cx: &mut Cx) {
        s.settle(cx);
        for dark in [true, false] {
            for scale in [1.0, 2.0] {
                cx.scale = scale;
                let img = snap::shoot(&mut s, cx, name, snap::BODY, scale, dark);
                assert!(img.count(None, |p| p[3] > 0) > 1000, "{name} drew something");
            }
        }
    }

    #[test]
    fn snapshots() {
        let _g = crate::ui::host::testing::motion_lock();
        let (svc, _f) = Services::fake();
        let mut cx = Cx::headless_with(svc);
        cx.now_ms = 275.0;
        // tools: the sample paragraph with a selection, hover on a tile
        let f = fixture("tools");
        let mut s = screen(f["text"].as_str().unwrap(), Some((0, 31)), &mut cx);
        s.area.ti.set_focused(true);
        s.settle(&mut cx);
        snap::shoot(&mut s, &cx, "text-tools-prime", snap::BODY, 1.0, true); // registers the click targets
        let p = s.lay.tile(1, 0.0, 0.0).center();
        s.event(&mv(p), &mut cx); // hover on the second tile
        shoot_all("text-tools", s, &mut cx);
        // stats toast
        let i = TOOLS.iter().position(|t| *t == Tool::Count).unwrap();
        let mut s = screen(fixture("stats")["text"].as_str().unwrap(), None, &mut cx);
        s.click(K::Tool(i), &mut cx);
        shoot_all("text-stats", s, &mut cx);
        // qr
        let i = TOOLS.iter().position(|t| *t == Tool::Qr).unwrap();
        let mut s = screen(fixture("qr")["text"].as_str().unwrap(), None, &mut cx);
        s.click(K::Tool(i), &mut cx);
        shoot_all("text-qr", s, &mut cx);
        // ai busy
        let (svc, _f) = Services::fake();
        let mut cx2 = Cx::headless_with(svc);
        cx2.now_ms = 275.0;
        let mut s = screen(fixture("ai-busy")["text"].as_str().unwrap(), None, &mut cx2);
        s.set_mode(Mode::Ai, &mut cx2);
        s.click(K::Ai(2), &mut cx2);
        shoot_all("text-ai-busy", s, &mut cx2);
        // russian + reduced motion, once
        let (svc, _f) = Services::fake();
        let mut cx3 = Cx::headless_with(svc);
        cx3.lang = glasskit::lang::Lang::Ru;
        cx3.reduced_motion = true;
        let mut s = screen("Привет, мир. Это проверка.", Some((0, 6)), &mut cx3);
        s.settle(&mut cx3);
        snap::shoot(&mut s, &cx3, "text-ru-reduced", snap::BODY, 1.0, true);
    }
}
