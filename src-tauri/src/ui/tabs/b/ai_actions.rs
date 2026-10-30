//! The AI quick-action panel over the Clipboard tab (`src/ai-actions.js`, `ai-actions.css`): a full-body overlay (648 x 260) with the four
//! actions (Translate / Shorten / Fix / Explain), a working screen, the result with Copy / Paste, and the error screen.
//!
//! Model: `ai_actions::AiActions`. The Clipboard tab (set A) opens the panel with `Msg::Open({"id": clipId})` and learns that it closed from
//! `TabFx::OverlayClosed("ai_actions")` (emitted once, after the exit animation starts). The view paints nothing while closed. Esc closes
//! (the island must route Esc to the overlay first while it is open: `is_open` is observable through `TabFx` only, so the island tracks
//! open/closed itself). Keyboard: Up/Down/Left/Right + Enter or 1-4 pick an action, Esc closes.
//! Not ported: selecting the result text with the mouse (the web used `user-select: text`); Copy puts all of it on the clipboard.
use super::fx_b::{emit, TabFx};
use super::kit::{self, lang_of, ms_of, route_effects, Bk, Calls, Clicks, Cues, Icons};
use crate::ui::view::{Cx, Msg, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, KeyCode, Response};
use glasskit::scroll::ScrollView;
use std::cell::Cell;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::b::ai_actions::{Act, Action, AiActions, MenuItem, Screen, View as Vm, ACTS};
use pill_ui_core::panel::b::Reply;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum K {
    Close,
    Opt(usize),
    Cancel,
    Copy,
    Paste,
    Back,
    CloseBtn,
    Retry,
    Enable,
}

pub struct AiActionsScreen {
    model: AiActions,
    calls: Calls,
    cues: Cues,
    clicks: Clicks<K>,
    icons: Icons,
    scroll: ScrollView,
    rect: Rect,
    /// the result box (rect) and its text height, set by the paint for `event` / `step`
    out: Cell<(Rect, f32)>,
    /// the view model of the last frame (kept while the exit animation plays)
    last: Option<Vm>,
    sel: usize,
    carry: f64,
    announced_close: bool,
}

impl AiActionsScreen {
    pub fn new() -> AiActionsScreen {
        AiActionsScreen {
            model: AiActions::new(pill_ui_core::panel::b::Lang::En),
            calls: Calls::new(),
            cues: Cues::new(),
            clicks: Clicks::new(),
            icons: Icons::new(),
            scroll: ScrollView::new(),
            rect: Rect::default(),
            out: Cell::new((Rect::default(), 0.0)),
            last: None,
            sel: 0,
            carry: 0.0,
            announced_close: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.model.is_open()
    }

    /// Run a model action and everything it asks for.
    fn act(&mut self, a: Action, cx: &mut Cx) {
        let c = self.model.on_action(a);
        self.calls.run(cx, c);
        self.pump(cx);
    }

    fn pump(&mut self, cx: &mut Cx) {
        let _view_only = route_effects(self.model.fx.take_effects(), cx);
        self.cues.reduced = cx.reduced_motion;
        self.cues.play_all(self.model.fx.take_anims());
        if self.model.take_closed() {
            emit(TabFx::OverlayClosed("ai_actions"));
            self.announced_close = true;
        }
        cx.request_frame();
    }

    fn click(&mut self, k: K, cx: &mut Cx) {
        match k {
            K::Close | K::CloseBtn => self.act(Action::Close, cx),
            K::Opt(i) => self.act(Action::Run(ACTS[i.min(ACTS.len() - 1)]), cx),
            K::Cancel => self.act(Action::Cancel, cx),
            K::Copy => self.act(Action::Copy, cx),
            K::Paste => self.act(Action::Paste, cx),
            K::Back => self.act(Action::Menu, cx),
            K::Retry => {
                if let Some(a) = self.model.retry_act() {
                    self.act(Action::Run(a), cx);
                }
            }
            K::Enable => self.act(Action::EnableAi, cx),
        }
    }
}

impl Default for AiActionsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl View for AiActionsScreen {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        if !self.model.is_open() {
            return Response::NONE;
        }
        self.clicks.reduced = cx.reduced_motion;
        if let Some((code, _, _)) = ev.key_down() {
            let in_menu = matches!(self.last.as_ref().and_then(|v| v.screen.as_ref()), Some(Screen::Menu(_)));
            match code {
                KeyCode::Escape => {
                    self.act(Action::Close, cx);
                    return Response { consumed: true, redraw: true, ..Response::NONE };
                }
                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right if in_menu => {
                    // a 2 x 2 grid in reading order
                    self.sel = match code {
                        KeyCode::Left => self.sel.saturating_sub(1),
                        KeyCode::Right => (self.sel + 1).min(3),
                        KeyCode::Up => self.sel.saturating_sub(2),
                        _ => (self.sel + 2).min(3),
                    };
                    return Response { consumed: true, redraw: true, ..Response::NONE };
                }
                KeyCode::Enter if in_menu => {
                    self.click(K::Opt(self.sel), cx);
                    return Response { consumed: true, redraw: true, ..Response::NONE };
                }
                KeyCode::Char(c) if in_menu && ('1'..='4').contains(&c) => {
                    self.click(K::Opt(c as usize - '1' as usize), cx);
                    return Response { consumed: true, redraw: true, ..Response::NONE };
                }
                _ => {}
            }
        }
        let (mut r, clicked) = self.clicks.event(ev);
        if let Some(k) = clicked {
            self.click(k, cx);
            r.redraw = true;
        }
        let (out, th_h) = self.out.get();
        self.scroll.set_content_height(th_h, out.h - 16.0);
        if !r.consumed && out.h > 0.0 {
            r = r.merge(self.scroll.event(ev, out));
        }
        // the overlay covers the whole body: nothing below may react while it is open
        if ev.pos().is_some_and(|p| self.rect.contains(p.0, p.1)) {
            r.consumed = true;
            if r.cursor.is_none() {
                r.cursor = Some(Cursor::Arrow);
            }
        }
        r
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Open(args) => {
                let id = args["id"].as_i64().or_else(|| args["id"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0);
                self.model.set_lang(lang_of(cx));
                self.sel = 0;
                self.announced_close = false;
                self.act(Action::Open { id }, cx);
            }
            Msg::Reply { ticket, result } => {
                if let Some((tag, r)) = self.calls.take(ticket, result) {
                    let r: Reply = r;
                    let c = self.model.on_reply(tag, r);
                    self.calls.run(cx, c);
                    self.pump(cx);
                }
            }
            Msg::Event { .. } | Msg::Custom(_) => {}
        }
        Response::redraw()
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        // the island collapsed: the panel goes with it (the model cancels a running request)
        if self.model.is_open() {
            self.act(Action::Close, cx);
        }
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.cues.reduced = cx.reduced_motion;
        self.clicks.reduced = cx.reduced_motion;
        self.model.set_lang(lang_of(cx));
        let _ = ms_of(dt_ms, &mut self.carry);
        let dt = (dt_ms / 1000.0) as f32;
        let (out, th_h) = self.out.get();
        self.scroll.set_content_height(th_h, (out.h - 16.0).max(0.0));
        let mut busy = self.cues.step(dt_ms as f32);
        busy |= self.clicks.step(dt);
        busy |= self.scroll.step(dt);
        // the spinner turns while a request runs
        busy | (matches!(self.last.as_ref().and_then(|v| v.screen.as_ref()), Some(Screen::Working { .. })) && !cx.reduced_motion)
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.rect = rect;
        let v = self.model.view();
        if v.open {
            self.last = Some(v);
        } else if self.cues.xf("ai-panel").alpha >= 1.0 && self.cues.is_empty() {
            self.last = None;
        }
        let Some(v) = self.last.clone() else { return };
        let panel = |cv: &Canvas| self.paint(cv, th, rect, &v, cx);
        let x = self.cues.xf("ai-panel");
        if x.alpha >= 1.0 && x.dy == 0.0 {
            panel(cv);
        } else {
            cv.with_opacity(x.alpha, |cv| cv.with_offset(0.0, x.dy, panel));
        }
    }
}

impl AiActionsScreen {
    fn paint(&self, cv: &Canvas, th: &Theme, rect: Rect, v: &Vm, cx: &Cx) {
        // everything is painted over an opaque body so the tab below does not show through
        cv.fill_rect(rect, th.bg);
        let c = &self.clicks;
        let ic = &self.icons;
        let pad = 4.0;
        // header: sparkles, title, close
        let head = Rect::new(rect.x, rect.y + pad, rect.w, 28.0);
        ic.draw(cv, "sparkles", Rect::new(head.x + 12.0, head.cy() - 8.0, 16.0, 16.0), th.accent);
        cv.text(Rect::new(head.x + 36.0, head.y, head.w - 36.0 - 36.0, head.h), &v.title, th.small(600).size(13.0).ellipsis(), th.text);
        let x_r = Rect::new(head.right() - 32.0, head.y, 28.0, 28.0);
        kit::button(cv, th, ic, x_r, "", Some("x"), Bk::Icon, c.hover_amt(&K::Close), c.press_scale(&K::Close, 0.92), true);
        let body = Rect::new(rect.x, head.bottom() + 8.0, rect.w, rect.bottom() - head.bottom() - 8.0);
        let body_x = self.cues.xf("ai-body");
        cv.with_opacity(body_x.alpha, |cv| {
            cv.with_offset(0.0, body_x.dy, |cv| match &v.screen {
                Some(Screen::Menu(items)) => self.menu(cv, th, body, items),
                Some(Screen::Working { cancel_label, working_label }) => self.working(cv, th, body, working_label, cancel_label, cx),
                Some(Screen::Result { text, clipped_note, copy, paste, back, close }) => self.result(cv, th, body, text, clipped_note.as_deref(), [copy, paste, back, close]),
                Some(Screen::Error { text, retry, enable, back, close }) => self.error(cv, th, body, text, retry.as_deref(), enable.as_deref(), back, close),
                None => {}
            })
        });
    }

    fn menu(&self, cv: &Canvas, th: &Theme, body: Rect, items: &[MenuItem]) {
        let grid = Rect::new(body.x + 4.0, body.y, body.w - 8.0, 52.0 * 2.0 + 8.0);
        for (i, it) in items.iter().enumerate() {
            let r = Rect::new(grid.x + (i % 2) as f32 * (grid.w / 2.0 + 4.0), grid.y + (i / 2) as f32 * 60.0, grid.w / 2.0 - 4.0, 52.0);
            let k = K::Opt(i);
            let h = self.clicks.hover_amt(&k).max(if self.sel == i && self.is_keyboard() { 1.0 } else { 0.0 });
            cv.with_scale_about(r.center(), self.clicks.press_scale(&k, 0.98), |cv| {
                cv.fill_rr(r, 12.0, th.card.mix(th.raised, h));
                let ib = Rect::new(r.x + 12.0, r.cy() - 16.0, 32.0, 32.0);
                cv.fill_rr(ib, 9.0, th.raised.mix(th.raised2, h));
                self.icons.draw(cv, it.icon, ib.centered(18.0, 18.0), th.dim.mix(th.accent, h));
                let tx = Rect::new(ib.right() + 12.0, r.y, r.right() - ib.right() - 24.0, r.h);
                cv.text(Rect::new(tx.x, r.cy() - 19.0, tx.w, 18.0), &it.label, th.body().ellipsis(), th.text);
                cv.text(Rect::new(tx.x, r.cy() + 1.0, tx.w, 14.0), &it.sub, th.caption().ellipsis(), th.dim);
                if h > 0.0 && self.sel == i && self.is_keyboard() {
                    cv.focus_ring(r, 12.0, th.accent, 1.0);
                }
            });
        }
    }

    /// Arrow keys light an option; the pointer hover does the same by itself.
    fn is_keyboard(&self) -> bool {
        self.clicks.hovered().is_none()
    }

    fn working(&self, cv: &Canvas, th: &Theme, body: Rect, label: &str, cancel: &str, cx: &Cx) {
        let st = th.body();
        let tw = cv.text_width(label, st);
        let bw = kit::button_w(cv, th, cancel, false, Bk::Ghost);
        let total = 16.0 + 12.0 + tw + 12.0 + bw;
        let x0 = body.cx() - total / 2.0;
        let cy = body.y + body.h / 2.0 - 10.0;
        let sp = Rect::new(x0, cy - 8.0, 16.0, 16.0);
        cv.stroke_circle(sp.center(), 7.0, 2.0, th.accent_soft);
        let start = if cx.reduced_motion { 0.0 } else { kit::phase(cx.now_ms, if cx.reduced_motion { 2400.0 } else { 800.0 }) * 360.0 };
        cv.arc(sp.center(), 7.0, 2.0, start, 90.0, th.accent);
        cv.text(Rect::new(sp.right() + 12.0, cy - 10.0, tw + 2.0, 20.0), label, st, th.text2);
        let br = Rect::new(sp.right() + 12.0 + tw + 12.0, cy - 15.0, bw, 30.0);
        let k = K::Cancel;
        self.btn_at(cv, th, br, cancel, None, Bk::Ghost, &k);
    }

    fn btn_at(&self, cv: &Canvas, th: &Theme, r: Rect, label: &str, icon: Option<&str>, kind: Bk, k: &K) {
        kit::button(cv, th, &self.icons, r, label, icon, kind, self.clicks.hover_amt(k), self.clicks.press_scale(k, 0.96), true);
    }

    fn buttons(&self, cv: &Canvas, th: &Theme, body: Rect, left: &[(&str, Option<&str>, Bk, K)], right: &[(&str, Bk, K)]) -> Rect {
        let h = 30.0;
        let y = body.bottom() - h - 4.0;
        let mut x = body.x + 4.0;
        for (label, icon, kind, k) in left {
            let w = kit::button_w(cv, th, label, icon.is_some(), *kind).max(30.0);
            let r = Rect::new(x, y, w, h);
            self.btn_at(cv, th, r, label, *icon, *kind, k);
            x += w + 8.0;
        }
        let mut rx = body.right() - 4.0;
        for (label, kind, k) in right.iter().rev() {
            let w = kit::button_w(cv, th, label, false, *kind);
            let r = Rect::new(rx - w, y, w, h);
            self.btn_at(cv, th, r, label, None, *kind, k);
            rx -= w + 8.0;
        }
        Rect::new(body.x, y, body.w, h)
    }

    #[allow(clippy::too_many_arguments)]
    fn result(&self, cv: &Canvas, th: &Theme, body: Rect, text: &str, note: Option<&str>, labels: [&String; 4]) {
        let btns = self.buttons(cv, th, body, &[(labels[0], Some("copy"), Bk::Normal, K::Copy), (labels[1], Some("enter"), Bk::Primary, K::Paste)], &[(labels[2], Bk::Ghost, K::Back), (labels[3], Bk::Ghost, K::CloseBtn)]);
        let note_h = if note.is_some() { 14.0 + 8.0 } else { 0.0 };
        let out = Rect::new(body.x + 4.0, body.y, body.w - 8.0, btns.y - 8.0 - note_h - body.y);
        cv.fill_rr(out, 12.0, th.card);
        let inner = out.inset_xy(12.0, 8.0);
        let st = th.body().wrap();
        let (_, th_h) = cv.measure(text, st, inner.w);
        self.out.set((out, th_h));
        let off = self.scroll.offset().min((th_h - inner.h).max(0.0));
        cv.with_clip(inner, |cv| cv.text(Rect::new(inner.x, inner.y - off, inner.w, th_h.max(inner.h)), text, st, th.text));
        if let Some(n) = note {
            cv.text(Rect::new(body.x + 12.0, out.bottom() + 6.0, body.w - 24.0, 14.0), n, th.caption(), th.faint);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn error(&self, cv: &Canvas, th: &Theme, body: Rect, text: &str, retry: Option<&str>, enable: Option<&str>, back: &str, close: &str) {
        let mut left: Vec<(&str, Option<&str>, Bk, K)> = Vec::new();
        if let Some(e) = enable {
            left.push((e, None, Bk::Primary, K::Enable));
        } else if let Some(r) = retry {
            left.push((r, None, Bk::Normal, K::Retry));
        }
        let btns = self.buttons(cv, th, body, &left, &[(back, Bk::Ghost, K::Back), (close, Bk::Ghost, K::CloseBtn)]);
        let out = Rect::new(body.x + 4.0, body.y, body.w - 8.0, btns.y - 8.0 - body.y);
        cv.fill_rr(out, 12.0, th.red_bg);
        cv.text(out.inset(12.0), text, th.body().wrap(), th.red);
    }
}
