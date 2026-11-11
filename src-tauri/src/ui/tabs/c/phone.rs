//! The Phone tab: Notifications (the phone's notifications with Hide / Mark read / Reply), Files (a drop zone and the transfers) and Phone (link
//! state, battery, network, a QR code to pair another phone). The model is `pill_ui_core::panel::c::phone::PhoneModel`; this file owns the state
//! of the view and the `View` impl, `phone_draw.rs` paints.
//!
//! Motion (all `ui::liquid`): the sub-tab indicator glides, a notification row grows in and collapses out (the rows below follow because the
//! height is the animated value), the reply field opens with the row, "Sent" pops a check, the drop zone lights up while files hang over the
//! island, progress bars and the QR card flow. Targets are set in `step` (`sync`), `draw` only reads. Reduce motion lands everything at once.
//! Nothing polls: the model is fed by the bus events `phone:state` / `phone:notif` / `phone:files` and by command replies; frames run only while
//! something moves (or while the reply field is focused, for the caret).
use super::phone_card::Card;
use super::phone_dash::Dash;
use super::phone_img::Images;
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::b::kit::{lang_of, Calls, Clicks, Field, FlowSwitch, Icons, Lit, GROW};

use crate::ui::view::{Cx, DropEvent, DropPayload, DropPhase, Msg, View};
use glasskit::canvas::Canvas;
use glasskit::edit::EditEvent;
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, KeyCode, Response};
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::c::phone::{Action, PhoneModel, ReplyState, Sub};
use serde_json::json;
use std::collections::{HashMap, HashSet};

/// Content box of the tab (the 648 px column, 417 px body minus the 71 px chrome).
pub const SIZE: (f32, f32) = (super::super::a::W, 417.0 - super::super::a::CHROME_H);
pub(super) const HEAD_H: f32 = 34.0;
pub(super) const BODY_GAP: f32 = 8.0;
const EVENTS: [&str; 3] = ["phone:state", "phone:notif", "phone:files"];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum K {
    Sub(usize),
    Hide(String),
    Read(String),
    Reply(String),
    Send(String),
    Accept(String),
    Decline(String),
    Open(String),
    Folder(String),
    NewCode,
    /// the phone switcher pills, the quiet "Pair another phone" button, the QR panel's close button and the scrim behind it
    Pick(usize),
    Pair,
    CloseQr,
    Scrim,
}

/// Per-row motion state of a notification.
pub(super) struct RowAnim {
    /// 0 = not there (height 0, transparent), 1 = full row
    pub open: Flow,
    /// the reply strip below the row
    pub reply: Flow,
    /// the "Sent" check
    pub sent: Lit,
}

impl RowAnim {
    fn new(settled: bool) -> RowAnim {
        RowAnim { open: Flow::new(settled as u8 as f32, GROW), reply: Flow::new(0.0, GROW), sent: Lit::new(Feel::POP) }
    }
    fn moving(&self) -> bool {
        self.open.moving() || self.reply.moving() || self.sent.moving()
    }
}

/// Per-transfer motion: enter, progress.
pub(super) struct FileAnim {
    pub open: Flow,
    pub progress: Flow,
}

pub(super) fn go(f: &mut Flow, to: f32, reduced: bool) {
    if reduced {
        f.jump(to);
    } else {
        f.to(to);
    }
}

pub struct PhoneView {
    pub(super) model: PhoneModel,
    calls: Calls,
    pub(super) clicks: Clicks<K>,
    pub(super) ic: Icons,
    pub(super) field: Field,
    pub(super) img: Images,
    pub(super) rect: Rect,
    /// the sub-tab indicator: x / width relative to the tab's left edge, the chip rects of the last draw
    pub(super) seg_x: Flow,
    pub(super) seg_w: Flow,
    pub(super) seg_cache: [Rect; 3],
    seg_ready: bool,
    pub(super) rows: HashMap<String, RowAnim>,
    pub(super) files: HashMap<String, FileAnim>,
    /// Notifications, Files, and the paired view of the Phone sub-tab (only scrolls when the body is too short)
    pub(super) scroll: [ScrollView; 3],
    pub(super) card: Card,
    pub(super) dash: Dash,
    pub(super) drag: bool,
    pub(super) drop_k: Lit,
    pub(super) drop_err: Option<String>,
    pub(super) drop_err_k: Lit,
    pub(super) conn_k: Lit,
    pub(super) qr_k: Lit,
    pub(super) bat: Flow,
    pub(super) switch: FlowSwitch,
    pub(super) switch_rect: Rect,
    /// the notification whose reply field is open
    pub(super) reply_key: Option<String>,
    pub(super) qr_cache: Option<(String, Vec<String>)>,
    sends: HashSet<u64>,
    field_drag: bool,
    mounted: bool,
    shown: bool,
    pub(super) reduced: bool,
    primed: bool,
    wall_override: Option<i64>,
}

impl Default for PhoneView {
    fn default() -> Self {
        PhoneView::new()
    }
}

impl PhoneView {
    pub fn new() -> PhoneView {
        let th = Theme::pill_dark();
        let mut ic = Icons::new();
        for (n, m) in ICONS {
            ic.add(n, m);
        }
        let mut field = Field::new(false, &th);
        field.ti.radius = 15.0;
        field.ti.style = Style::new(12.0, 400);
        PhoneView {
            model: PhoneModel::new(pill_ui_core::panel::b::Lang::En),
            calls: Calls::new(),
            clicks: Clicks::new(),
            ic,
            field,
            img: Images::default(),
            rect: Rect::default(),
            seg_x: Flow::px(0.0, Feel::FLOW),
            seg_w: Flow::px(0.0, Feel::FLOW),
            seg_cache: [Rect::default(); 3],
            seg_ready: false,
            rows: HashMap::new(),
            files: HashMap::new(),
            scroll: [ScrollView::new(), ScrollView::new(), ScrollView::new()],
            card: Card::new(),
            dash: Dash::new(),
            drag: false,
            drop_k: Lit::new(Feel::FLOW),
            drop_err: None,
            drop_err_k: Lit::new(Feel::FADE),
            conn_k: Lit::new(Feel::FADE),
            qr_k: Lit::new(Feel::POP),
            bat: Flow::new(0.0, Feel::DRAG),
            switch: FlowSwitch::new(false),
            switch_rect: Rect::default(),
            reply_key: None,
            qr_cache: None,
            sends: HashSet::new(),
            field_drag: false,
            mounted: false,
            shown: false,
            reduced: false,
            primed: false,
            wall_override: None,
        }
    }

    /// Pin the wall clock (tests): time labels are `wall - when`.
    pub fn with_wall(mut self, ms: i64) -> PhoneView {
        self.wall_override = Some(ms);
        self
    }

    pub(super) fn wall_pinned(&self) -> bool {
        self.wall_override.is_some()
    }

    pub(super) fn wall(&self) -> i64 {
        self.wall_override.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64))
    }

    pub(super) fn link_off(&self) -> bool {
        self.model.empty_text().is_some()
    }

    fn ctx(&mut self, cx: &Cx) {
        self.model.set_lang(lang_of(cx));
        if cx.reduced_motion != self.reduced {
            self.reduced = cx.reduced_motion;
            self.clicks.reduced = self.reduced;
            self.field.ti.reduced = Some(self.reduced);
            for s in &mut self.scroll {
                s.state.reduced = Some(self.reduced);
                s.bar.reduced = Some(self.reduced);
            }
        }
    }

    fn act(&mut self, a: Action, cx: &mut Cx) {
        let c = self.model.on_action(a);
        self.calls.run(cx, c);
        cx.request_frame();
    }

    fn click(&mut self, k: K, cx: &mut Cx) {
        match k {
            K::Sub(i) => {
                if let Some(s) = Sub::ALL.get(i) {
                    self.act(Action::Select(*s), cx);
                }
            }
            K::Hide(key) => self.act(Action::Hide(key), cx),
            K::Read(key) => self.act(Action::MarkRead(key), cx),
            K::Reply(key) => {
                if self.reply_key.as_ref() == Some(&key) {
                    self.close_reply(cx);
                    return;
                }
                self.close_reply(cx);
                self.field.ti.set_text("");
                self.act(Action::ReplyOpen(key.clone()), cx);
                if self.model.rows().iter().any(|r| r.key == key && r.reply == ReplyState::Open) {
                    self.reply_key = Some(key);
                    self.field.ti.set_focused(true);
                }
            }
            K::Send(key) => {
                let text = self.field.ti.text();
                self.act(Action::ReplySend(key, text), cx);
            }
            K::Accept(id) => self.act(Action::Accept(id), cx),
            K::Decline(id) => self.act(Action::Decline(id), cx),
            K::Open(id) => self.act(Action::OpenFile(id), cx),
            K::Folder(id) => self.act(Action::Reveal(id), cx),
            K::NewCode => self.act(Action::NewCode, cx),
            K::Pick(i) => {
                self.pick(i);
                cx.request_frame();
            }
            K::Pair => {
                self.dash.qr_open = !self.dash.qr_open;
                cx.request_frame();
            }
            K::CloseQr | K::Scrim => {
                self.dash.qr_open = false;
                cx.request_frame();
            }
        }
    }

    fn close_reply(&mut self, cx: &mut Cx) {
        if let Some(k) = self.reply_key.take() {
            self.act(Action::ReplyClose(k), cx);
        }
        self.field.ti.set_focused(false);
    }

    /// Enter in the reply field sends, Escape closes it.
    fn field_events(&mut self, cx: &mut Cx) {
        let mut evs = self.field.take_events();
        evs.extend(self.field.ti.take_events());
        for e in evs {
            let Some(key) = self.reply_key.clone() else { continue };
            match e {
                EditEvent::Submit => {
                    let text = self.field.ti.text();
                    self.act(Action::ReplySend(key, text), cx);
                }
                EditEvent::Cancel => self.close_reply(cx),
                _ => {}
            }
        }
    }

    /// Every motion target, derived from the model. Called from `step` only.
    fn sync(&mut self) {
        let reduced = self.reduced;
        if !self.primed && self.model.loaded() {
            // what came with the first list read is not news: it is simply there
            for r in self.model.rows() {
                self.rows.insert(r.key.clone(), RowAnim::new(true));
            }
            for f in self.model.files() {
                self.files.insert(f.id.clone(), FileAnim { open: Flow::new(1.0, GROW), progress: Flow::new(f.progress().unwrap_or(0.0), Feel::DRAG) });
            }
            self.primed = true;
        }
        let mut gone = Vec::new();
        for r in self.model.rows() {
            let a = self.rows.entry(r.key.clone()).or_insert_with(|| RowAnim::new(false));
            go(&mut a.open, if r.leaving { 0.0 } else { 1.0 }, reduced);
            go(&mut a.reply, (r.reply != ReplyState::Closed) as u8 as f32, reduced);
            a.sent.set(r.reply == ReplyState::Sent);
            if reduced {
                a.sent.step(1e6);
            }
            if r.leaving && !a.open.moving() && a.open.at() <= 0.01 {
                gone.push(r.key.clone());
            }
        }
        for k in gone {
            self.model.reap(&k);
        }
        let keep: HashSet<&str> = self.model.rows().iter().map(|r| r.key.as_str()).collect();
        self.rows.retain(|k, _| keep.contains(k.as_str()));
        // a reply field that is not needed any more
        if let Some(k) = self.reply_key.clone() {
            let open = self.model.rows().iter().any(|r| r.key == k && !r.leaving && matches!(r.reply, ReplyState::Open | ReplyState::Sending | ReplyState::Failed(_)));
            if !open {
                self.reply_key = None;
                self.field.ti.set_focused(false);
                self.field.ti.set_text("");
            }
        }
        for f in self.model.files() {
            let a = self.files.entry(f.id.clone()).or_insert_with(|| FileAnim { open: Flow::new(0.0, GROW), progress: Flow::new(0.0, Feel::DRAG) });
            go(&mut a.open, 1.0, reduced);
            go(&mut a.progress, f.progress().unwrap_or(if f.status == "done" { 1.0 } else { 0.0 }), reduced);
        }
        let ids: HashSet<&str> = self.model.files().iter().map(|f| f.id.as_str()).collect();
        self.files.retain(|k, _| ids.contains(k.as_str()));
        // the sub-tab indicator
        if self.seg_ready {
            let r = self.seg_cache[self.model.sub() as usize];
            go(&mut self.seg_x, r.x, reduced);
            go(&mut self.seg_w, r.w, reduced);
        }
        self.drop_k.set(self.drag && self.model.sub() == Sub::Files);
        self.drop_err_k.set(self.drop_err.is_some());
        self.conn_k.set(self.model.link().connected());
        self.qr_k.set(self.model.code().is_some());
        let bat = self.shown().and_then(|p| p.battery).unwrap_or(0) as f32 / 100.0;
        go(&mut self.bat, bat, reduced);
        if let Some(p) = self.shown().cloned() {
            self.card.sync(&p, reduced);
        }
        self.dash_sync();
        self.switch.set_on(self.model.link().enabled, !reduced);
    }

    pub(super) fn note_segments(&mut self, cache: [Rect; 3]) {
        if !self.seg_ready {
            self.seg_ready = true;
            let r = cache[self.model.sub() as usize];
            self.seg_x.jump(r.x);
            self.seg_w.jump(r.w);
        }
        self.seg_cache = cache;
    }

    fn mount(&mut self, cx: &mut Cx) {
        if self.mounted {
            return;
        }
        self.mounted = true;
        for e in EVENTS {
            cx.subscribe(e);
        }
        let c = self.model.mount();
        self.calls.run(cx, c);
    }

    /// The scroll view of the current sub-tab (Phone has none).
    pub(super) fn body_rect(&self) -> Rect {
        let r = if self.rect.w > 0.0 { self.rect } else { Rect::new(0.0, 0.0, SIZE.0, SIZE.1) };
        Rect::new(r.x, r.y + HEAD_H + BODY_GAP, r.w, (r.h - HEAD_H - BODY_GAP).max(0.0))
    }

    fn pointer(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        if self.field_drag {
            let r = self.field.queue(ev);
            if matches!(ev, InputEvent::MouseUp { .. }) {
                self.field_drag = false;
            }
            return r;
        }
        if self.link_off() {
            let r = self.switch.event(ev, self.switch_rect);
            if r.changed {
                self.act(Action::Enable, cx);
            }
            if r.consumed || r.capture {
                return r;
            }
        }
        let (r, clicked) = self.clicks.event(ev);
        if let Some(k) = clicked {
            self.click(k, cx);
        }
        if r.consumed {
            return r;
        }
        let pos = ev.pos();
        if let (InputEvent::MouseDown { .. }, Some(p)) = (ev, pos) {
            if self.reply_key.is_some() && self.field.rect.contains(p.0, p.1) {
                self.field_drag = true;
                return self.field.queue(ev);
            }
        }
        if let Some(p) = pos {
            if matches!(ev, InputEvent::MouseMove { .. }) && r.cursor.is_none() && self.reply_key.is_some() && self.field.rect.contains(p.0, p.1) {
                return Response { cursor: Some(glasskit::input::Cursor::Text), ..r };
            }
        }
        let (i, body) = match self.model.sub() {
            Sub::Notifs => (0, self.body_rect()),
            Sub::Files => (1, self.body_rect()),
            Sub::Phone => (2, self.body_rect()),
        };
        r.merge(self.scroll[i].event(ev, body))
    }
}

impl View for PhoneView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some(SIZE)
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.shown = true;
        self.ctx(cx);
        self.mount(cx);
        let wall = self.wall();
        let c = self.model.on_show(wall);
        self.calls.run(cx, c);
        cx.request_frame();
    }

    fn on_hide(&mut self, _cx: &mut Cx) {
        self.shown = false;
        self.dash.qr_open = false;
        self.field.ti.set_focused(false);
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.ctx(cx);
        match ev {
            InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_) => {
                // Escape slides the QR panel of a paired phone away
                if self.dash.qr_open && !self.field.is_focused() {
                    if let Some((KeyCode::Escape, _, false)) = ev.key_down() {
                        self.dash.qr_open = false;
                        cx.request_frame();
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                }
                if !self.field.is_focused() {
                    return Response::NONE;
                }
                if let Some((KeyCode::Escape, _, false)) = ev.key_down() {
                    self.close_reply(cx);
                    return Response { consumed: true, redraw: true, ..Response::NONE };
                }
                self.field.queue(ev)
            }
            InputEvent::MouseDown { .. } | InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. } | InputEvent::Wheel { .. } | InputEvent::MouseLeave => self.pointer(ev, cx),
            _ => Response::NONE,
        }
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.ctx(cx);
        match msg {
            Msg::Event { name, payload, .. } => {
                let wall = self.wall();
                self.model.set_wall(wall);
                let c = self.model.on_event(&name, &payload);
                self.calls.run(cx, c);
            }
            Msg::Reply { ticket, result } => {
                if self.sends.remove(&ticket.0) {
                    // a file the user dropped could not be offered (no phone, too big ...)
                    self.drop_err = result.err();
                } else if let Some((tag, r)) = self.calls.take(ticket, result) {
                    let c = self.model.on_reply(tag, r);
                    self.calls.run(cx, c);
                }
            }
            Msg::Open(_) | Msg::Custom(_) => {}
        }
        cx.request_frame();
        Response::redraw()
    }

    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        if self.model.sub() != Sub::Files {
            return Response::NONE;
        }
        let files = |p: &Option<DropPayload>| -> Option<Vec<String>> {
            match p {
                Some(DropPayload::Files(f)) => Some(f.iter().map(|x| x.to_string_lossy().into_owned()).collect()),
                _ => None,
            }
        };
        match ev.phase {
            DropPhase::Enter => {
                if files(&ev.payload).is_none() {
                    return Response::NONE;
                }
                self.drag = true;
                self.drop_err = None;
            }
            DropPhase::Over => return Response::consumed(),
            DropPhase::Leave => self.drag = false,
            DropPhase::Drop => {
                self.drag = false;
                let Some(paths) = files(&ev.payload) else { return Response::NONE };
                self.drop_err = None;
                for p in paths {
                    let t = cx.call_async("phone_send_file", json!({ "path": p }));
                    self.sends.insert(t.0);
                }
            }
        }
        cx.request_frame();
        if matches!(ev.phase, DropPhase::Leave) { Response::redraw() } else { Response { consumed: true, redraw: true, ..Response::NONE } }
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.ctx(cx);
        let wall = self.wall();
        self.model.set_wall(wall);
        let timers = self.model.tick(cx.now_ms);
        self.field_events(cx);
        self.sync();
        let mut busy = timers;
        let dt = (dt_ms / 1000.0) as f32;
        for f in [&mut self.seg_x, &mut self.seg_w, &mut self.bat] {
            f.step(dt_ms);
            busy |= f.moving();
        }
        for l in [&mut self.drop_k, &mut self.drop_err_k, &mut self.conn_k, &mut self.qr_k] {
            busy |= l.step(dt_ms);
        }
        for a in self.rows.values_mut() {
            a.open.step(dt_ms);
            a.reply.step(dt_ms);
            a.sent.step(dt_ms);
            busy |= a.moving();
        }
        for a in self.files.values_mut() {
            a.open.step(dt_ms);
            a.progress.step(dt_ms);
            busy |= a.open.moving() || a.progress.moving();
        }
        busy |= self.card.step(dt_ms);
        busy |= self.dash.step(dt_ms);
        busy |= self.clicks.step(dt);
        busy |= self.switch.step(dt);
        for s in &mut self.scroll {
            busy |= s.step(dt);
        }
        busy |= self.field.ti.step(dt);
        busy |= self.field.has_input();
        // the caret blinks while the reply field is focused
        busy |= !self.reduced && self.field.is_focused();
        busy
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.rect = rect;
        self.draw_all(cv, th, cx);
    }
}

/// Line icons of this tab (24 x 24 grid, stroke 1.7).
const ICONS: [(&str, &str); 15] = [
    ("p-x", r#"<path d="M6 6l12 12M18 6 6 18"/>"#),
    ("p-check", r#"<path d="M20 6 9 17l-5-5"/>"#),
    ("p-reply", r#"<path d="M9 17 4 12l5-5"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>"#),
    ("p-send", r#"<path d="M5 12h14M13 6l6 6-6 6"/>"#),
    ("p-down", r#"<path d="M12 4v13M6 12l6 6 6-6"/>"#),
    ("p-up", r#"<path d="M12 20V7M6 12l6-6 6 6"/>"#),
    ("p-bolt", r#"<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>"#),
    ("p-wifi", r#"<path d="M2 9a15 15 0 0 1 20 0M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0"/><path d="M12 19.5h.01"/>"#),
    ("p-mobile", r#"<path d="M3 20h.01M8 20v-5M13 20V10M18 20V4"/>"#),
    ("p-folder", r#"<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#),
    ("p-file", r#"<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>"#),
    ("p-moon", r#"<path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"/>"#),
    ("p-upload", r#"<path d="M12 15V4M7 9l5-5 5 5"/><path d="M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/>"#),
    ("p-phone", r#"<rect x="5" y="2" width="14" height="20" rx="2.5"/><path d="M11 18.5h2"/>"#),
    ("p-plane", r#"<path d="M21 15.5v-2L13 9V4.5a1.5 1.5 0 0 0-3 0V9l-8 4.5v2l8-2.5V18l-2 1.5V21l3.5-1 3.5 1v-1.5L13 18v-5z"/>"#),
];

#[cfg(test)]
mod dash_tests;
#[cfg(test)]
mod tests;
