//! The clipboard tab (648 x 260): search box, history list with thumbnails / pins / actions, footer. Port of `src/tab-clipboard.js` + `.css`.
//!
//! * Logic lives in `pill_ui_core::panel::a::clipboard::Clipboard` (search debounce, paste flow, wipe arming, selection). This view feeds it
//!   input as `Action`s, runs its `BackendCall`s on workers (`Base::calls`), feeds the answers back with `on_result`, plays its `Anim` cues
//!   (`row:<id>` stagger / exit / collapse, `list` FLIP, `empty` enter) and forwards island-bound `Fx` as `TabFx`.
//! * The search field is a glasskit `TextInput`. `View::event` has no canvas to measure text with, so input meant for the field is QUEUED and
//!   fed to the widget at the start of the next `draw` (the host repaints after every event that returns `redraw`).
//! * Time: the model clock is driven from `Cx::now_ms` on every step / tick / message. A background `Ticker` (100 ms while the field has the
//!   keyboard or a model timer runs, 1 s otherwise) wakes the view; nothing else costs a frame.
//! * Keyboard: the tab asks for it with `cx.set_focus(true)` when the search box is focused, and gives it back with `set_focus(false)`.
mod paint;
#[cfg(test)]
mod tests;

use super::core::{Base, Incoming, Local, Ticker};
use super::draw::Source;
use crate::ui::view::{Cx, Msg, View};
use base64::Engine;
use glasskit::canvas::Canvas;
use glasskit::ease::{curves, dur, Tween};
use glasskit::edit::{EditEvent, TextInput};
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::lang::Lang;
use glasskit::scroll::ScrollView;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::clipboard::{Action, Clipboard, Key, View as Vm};
use pill_ui_core::panel::a::{BackendCall, EnTr, RuTr, Tr};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

pub const BAR_H: f32 = 36.0;
pub const GAP: f32 = 8.0;
pub const ROW_H: f32 = 52.0;
pub const FOOT_H: f32 = 18.0;
pub const ACT: f32 = 26.0;
/// Events the tab listens to.
const EVENTS: &[&str] = &["clip:added", "clip:open", "settings:changed"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Act {
    Plain,
    Ai,
    Pin,
    Del,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    Search,
    Wipe,
    Row(usize),
    Btn(usize, Act),
}

pub struct ClipboardView {
    pub(crate) model: Clipboard,
    pub(crate) base: Base,
    pub(crate) input: TextInput,
    pub(crate) scroll: ScrollView,
    /// the model's view-model, rebuilt when `dirty` (it formats strings)
    pub(crate) vm: Option<Vm>,
    pub(crate) dirty: bool,
    lang: Lang,
    /// events for the search field, fed to it in `draw` (it needs a canvas)
    pending_edit: Vec<InputEvent>,
    edit_drag: bool,
    pub(crate) focused: bool,
    down: Option<Hit>,
    pub(crate) hover: Option<Hit>,
    /// per-row highlight amount (hover / selection), by clip id
    pub(crate) hl: HashMap<i64, Tween<f32>>,
    pub(crate) hover_btn: Tween<f32>,
    pub(crate) press: Tween<f32>,
    pub(crate) wipe_hover: Tween<f32>,
    pub(crate) arm: Tween<f32>,
    /// where each row was drawn last frame (content coordinates), for the FLIP of the next list
    pub(crate) tops: HashMap<i64, f32>,
    /// rows that moved: id -> old top, applied while the `list` cue plays
    pub(crate) flip_from: HashMap<i64, f32>,
    last_ms: Option<f64>,
    fast_until: f64,
    ticker: Option<(u64, Ticker)>,
    pub(crate) rect: Rect,
    /// (top, height) of every row in content coordinates, rebuilt by `paint::relayout`
    pub(crate) lay: Vec<(f32, f32)>,
    /// rows being deleted (their `Collapse` cue shrinks the height)
    pub(crate) collapsing: std::collections::HashSet<i64>,
    pub(crate) sel_id: Option<i64>,
}

impl ClipboardView {
    pub fn new() -> ClipboardView {
        ClipboardView::with_input(TextInput::new(false, &Theme::pill_dark()))
    }

    pub(crate) fn with_input(mut input: TextInput) -> ClipboardView {
        input.pad = (36.0, 0.0);
        input.radius = 12.0;
        ClipboardView {
            model: Clipboard::new(),
            base: Base::new(),
            input,
            scroll: {
                let mut s = ScrollView::new();
                s.keys = false;
                s
            },
            vm: None,
            dirty: true,
            lang: Lang::Ru,
            pending_edit: Vec::new(),
            edit_drag: false,
            focused: false,
            down: None,
            hover: None,
            hl: HashMap::new(),
            hover_btn: Tween::at_rest(0.0),
            press: Tween::at_rest(0.0),
            wipe_hover: Tween::at_rest(0.0),
            arm: Tween::at_rest(0.0),
            tops: HashMap::new(),
            flip_from: HashMap::new(),
            last_ms: None,
            fast_until: 0.0,
            ticker: None,
            rect: Rect::new(0.0, 0.0, super::W, 260.0),
            lay: Vec::new(),
            collapsing: Default::default(),
            sel_id: None,
        }
    }

    pub(crate) fn tr(&self) -> &'static dyn Tr {
        if self.lang == Lang::Ru {
            &RuTr
        } else {
            &EnTr
        }
    }

    pub(crate) fn vm(&mut self) -> &Vm {
        if self.dirty || self.vm.is_none() {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            self.vm = Some(self.model.view(self.tr(), now));
            self.dirty = false;
        }
        self.vm.as_ref().unwrap()
    }

    // ------------------------------------------------------------------------------------------------------------ plumbing

    fn run(&mut self, cx: &mut Cx, calls: Vec<BackendCall>) {
        self.base.calls.run(cx, calls);
    }

    /// After every model interaction: route its effects, refresh the view-model, wake the ticker.
    fn settle(&mut self, cx: &mut Cx) {
        let fxs = self.model.take_fx();
        let locals = self.base.route_fx(cx, fxs);
        for l in locals {
            match l {
                Local::FocusSearch => self.focus_search(cx, true),
                Local::BlurSearch => self.focus_search(cx, false),
                Local::ScrollTo(t) => self.scroll_to_row(&t),
            }
        }
        if self.model.query() != self.input.text() && !self.focused_has_unsent_edit() {
            let q = self.model.query().to_string();
            self.input.set_text(&q);
        }
        self.select_hl();
        self.dirty = true;
        self.retune_ticker(cx);
        cx.request_frame();
    }

    fn focused_has_unsent_edit(&self) -> bool {
        !self.pending_edit.is_empty()
    }

    fn focus_search(&mut self, cx: &mut Cx, on: bool) {
        self.focused = on;
        self.input.set_focused(on);
        self.model.set_search_focused(on);
        cx.set_focus(on);
        self.retune_ticker(cx);
    }

    fn scroll_to_row(&mut self, target: &str) {
        let Some(id) = target.strip_prefix("row:").and_then(|s| s.parse::<i64>().ok()) else { return };
        let Some(i) = self.model.items().iter().position(|c| c.id == id) else { return };
        let (top, vh) = (i as f32 * ROW_H, self.scroll.state.viewport());
        let off = self.scroll.state.target();
        if top < off {
            self.scroll.state.scroll_to(top, true);
        } else if top + ROW_H > off + vh {
            self.scroll.state.scroll_to(top + ROW_H - vh, true);
        }
    }

    /// Advance the model clock to `cx.now_ms`; returns its due calls.
    fn sync_clock(&mut self, cx: &mut Cx) {
        let now = cx.now_ms;
        let dt = self.last_ms.map_or(0.0, |l| (now - l).max(0.0));
        self.last_ms = Some(now);
        if dt > 0.0 {
            self.input.step((dt / 1000.0) as f32);
            let calls = self.model.tick(dt);
            if !calls.is_empty() {
                self.run(cx, calls);
            }
        }
    }

    fn want_ticker_ms(&self, now: f64) -> u64 {
        if self.focused || now < self.fast_until {
            100
        } else {
            1000
        }
    }

    fn retune_ticker(&mut self, cx: &mut Cx) {
        if !self.base.is_shown() {
            return;
        }
        let ms = self.want_ticker_ms(cx.now_ms);
        if self.ticker.as_ref().map(|t| t.0) != Some(ms) {
            self.ticker = Some((ms, Ticker::start(cx.waker(), Duration::from_millis(ms))));
        }
    }

    fn fast(&mut self, cx: &mut Cx, ms: f64) {
        self.fast_until = self.fast_until.max(cx.now_ms + ms);
        self.retune_ticker(cx);
    }

    pub(crate) fn act(&mut self, cx: &mut Cx, a: Action) {
        let fast = matches!(a, Action::Search(_) | Action::Activate { .. } | Action::PastePlain(_) | Action::Wipe | Action::Delete(_));
        let calls = self.model.on_action(a);
        self.run(cx, calls);
        if fast {
            self.fast(cx, 700.0);
        }
        self.settle(cx);
    }

    fn set_hl(&mut self, id: i64, on: bool) {
        let v = if on { 1.0 } else { 0.0 };
        let t = self.hl.entry(id).or_insert_with(|| Tween::at_rest(0.0));
        t.retarget(v, dur::FAST, curves::EASE_OUT);
    }

    fn select_hl(&mut self) {
        let cur = self.model.items().get(self.model.selected()).map(|c| c.id);
        // selection highlight: the selected row, plus the hovered one while the pointer is on the list
        if cur != self.sel_id {
            if let Some(o) = self.sel_id {
                self.set_hl(o, false);
            }
            if let Some(c) = cur {
                self.set_hl(c, true);
            }
            self.sel_id = cur;
        }
    }

    fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
        let b64 = url.split_once(";base64,")?.1;
        base64::engine::general_purpose::STANDARD.decode(b64).ok()
    }

    fn on_result(&mut self, cx: &mut Cx, cmd: &'static str, args: Value, result: Result<Value, String>) {
        // a rejected command: the JS page did nothing (the toast / reload of a failed paste included)
        let Ok(res) = result else { return };
        if cmd == "clip_list" {
            self.flip_from = self.tops.clone();
        }
        let tr = self.tr();
        let calls = self.model.on_result(cmd, &args, &res, tr);
        match cmd {
            "clip_image" => {
                let id = args.get("id").and_then(Value::as_i64).unwrap_or(0);
                if let Some(bytes) = res.as_str().and_then(Self::data_url_bytes) {
                    self.base.images.request(&cx.waker(), &format!("clip:{id}"), Source::Bytes(bytes));
                }
            }
            "clip_list" => {
                let ids: Vec<i64> = self.model.items().iter().map(|c| c.id).collect();
                for t in self.base.player.finished() {
                    if let Some(id) = t.strip_prefix("row:").and_then(|s| s.parse::<i64>().ok()) {
                        if !ids.contains(&id) || self.collapsing.contains(&id) {
                            self.base.player.release(&t);
                        }
                    }
                }
                self.hl.retain(|id, _| ids.contains(id));
                self.collapsing.retain(|id| self.base.player.is_active(&format!("row:{id}")));
                self.sel_id = None; // re-select below
                if self.model.items().is_empty() {
                    self.scroll.state.scroll_to(0.0, false);
                }
            }
            _ => {}
        }
        self.run(cx, calls);
        self.select_hl();
        self.settle(cx);
    }

    // ------------------------------------------------------------------------------------------------------------ input

    pub(crate) fn down_is(&self, h: Hit) -> bool {
        self.down == Some(h)
    }

    fn forward_edit(&mut self, ev: &InputEvent) {
        self.pending_edit.push(ev.clone());
    }

    fn hit_at(&mut self, p: glasskit::geom::Pt) -> Option<Hit> {
        paint::hit(self, p)
    }

    fn click(&mut self, cx: &mut Cx, hit: Hit, shift: bool) {
        let items: Vec<i64> = self.model.items().iter().map(|c| c.id).collect();
        match hit {
            Hit::Wipe => self.act(cx, Action::Wipe),
            Hit::Row(i) => {
                if let Some(&id) = items.get(i) {
                    self.act(cx, Action::Activate { id, shift });
                }
            }
            Hit::Btn(i, a) => {
                if let Some(&id) = items.get(i) {
                    let action = match a {
                        Act::Plain => Action::PastePlain(id),
                        Act::Ai => Action::Ai(id),
                        Act::Pin => Action::Pin(id),
                        Act::Del => {
                            self.collapsing.insert(id);
                            Action::Delete(id)
                        }
                    };
                    self.act(cx, action);
                }
            }
            Hit::Search => {}
        }
    }

    fn key_event(&mut self, ev: &InputEvent, code: KeyCode, mods: glasskit::input::Modifiers, cx: &mut Cx) -> Response {
        let consumed = Response { consumed: true, redraw: true, ..Response::NONE };
        match code {
            KeyCode::Escape => {
                if self.model.wants_escape() {
                    self.act(cx, Action::Key(Key::Escape));
                    return consumed;
                }
                if self.focused {
                    self.focus_search(cx, false);
                    return Response::redraw();
                }
                Response::NONE
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Enter => {
                let k = match code {
                    KeyCode::Up => Key::Up,
                    KeyCode::Down => Key::Down,
                    _ => Key::Enter { shift: mods.shift },
                };
                if self.model.items().is_empty() {
                    return Response::NONE;
                }
                self.act(cx, Action::Key(k));
                self.select_hl();
                consumed
            }
            _ if self.focused => {
                self.forward_edit(ev);
                consumed
            }
            _ => Response::NONE,
        }
    }
}

impl View for ClipboardView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((super::W, super::tab_h("clipboard")? - super::CHROME_H))
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.lang = cx.lang;
        self.base.show(cx, EVENTS, 0);
        self.model.set_os_reduce_motion(cx.reduced_motion);
        if let Ok(v) = cx.call("settings_get", Value::Null) {
            self.model.set_settings(&v);
        }
        self.last_ms = Some(cx.now_ms);
        self.scroll.state.scroll_to(0.0, false);
        self.tops.clear();
        let calls = self.model.on_show();
        self.run(cx, calls);
        self.select_hl();
        self.settle(cx);
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.model.on_hide();
        self.ticker = None;
        self.base.hide();
        if self.focused {
            self.focus_search(cx, false);
        }
        self.settle(cx);
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.lang = cx.lang;
        match self.base.classify(msg, cx) {
            Incoming::Event { name, payload } => {
                let calls = self.model.on_event(&name, &payload);
                self.run(cx, calls);
                if name == "clip:added" || name == "clip:open" {
                    self.fast(cx, 700.0);
                }
                self.settle(cx);
            }
            Incoming::Result { cmd, args, result } => self.on_result(cx, cmd, args, result),
            Incoming::Tick(_) => {
                self.sync_clock(cx);
                self.settle(cx);
            }
            Incoming::Image(_) => self.dirty = true,
            Incoming::Other(_) => return Response::NONE,
        }
        Response::redraw()
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.sync_clock(cx);
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.base.player.step(dt_ms);
        busy |= self.scroll.step(dt);
        busy |= self.input.busy();
        for t in self.hl.values_mut() {
            busy |= t.step(dt);
        }
        busy |= self.hover_btn.step(dt) | self.press.step(dt) | self.wipe_hover.step(dt) | self.arm.step(dt);
        let armed = self.vm().wipe_armed;
        self.arm.retarget(armed as u8 as f32, dur::MED, curves::EASE_OUT);
        self.hl.retain(|_, t| !t.finished() || t.target() > 0.0);
        if busy {
            cx.request_frame();
        }
        busy
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.lang = cx.lang;
        match ev {
            InputEvent::MouseMove { pos, buttons, .. } => {
                if self.edit_drag {
                    self.forward_edit(ev);
                    return Response { redraw: true, capture: true, cursor: Some(Cursor::Text), ..Response::NONE };
                }
                let hit = self.hit_at(*pos);
                let mut r = Response::NONE;
                if hit != self.hover {
                    self.hover = hit;
                    r.redraw = true;
                    match hit {
                        Some(Hit::Btn(..)) => {
                            self.hover_btn = Tween::new(0.0, 1.0, dur::FAST, curves::EASE_OUT);
                        }
                        _ => self.hover_btn = Tween::at_rest(0.0),
                    }
                    self.wipe_hover.retarget((hit == Some(Hit::Wipe)) as u8 as f32, dur::FAST, curves::EASE_OUT);
                    if let Some(Hit::Row(i) | Hit::Btn(i, _)) = hit {
                        self.act(cx, Action::Hover(i));
                        self.select_hl();
                    }
                }
                let _ = buttons;
                r.cursor = match hit {
                    Some(Hit::Search) => Some(Cursor::Text),
                    Some(_) => Some(Cursor::Hand),
                    None => None,
                };
                r
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => {
                let hit = self.hit_at(*pos);
                self.down = hit;
                if hit == Some(Hit::Search) {
                    if !self.focused {
                        self.focus_search(cx, true);
                    }
                    self.edit_drag = true;
                    self.forward_edit(ev);
                    return Response { consumed: true, redraw: true, capture: true, focus: true, cursor: Some(Cursor::Text), ..Response::NONE };
                }
                if self.focused && !matches!(hit, Some(Hit::Row(_) | Hit::Btn(..))) {
                    self.focus_search(cx, false);
                }
                if hit.is_some() {
                    self.press = Tween::new(0.0, 1.0, dur::XS, curves::EASE_OUT);
                    return Response { consumed: true, redraw: true, capture: true, ..Response::NONE };
                }
                Response::redraw()
            }
            InputEvent::MouseUp { pos, button: MouseButton::Left, mods } => {
                if self.edit_drag {
                    self.edit_drag = false;
                    self.forward_edit(ev);
                    return Response::redraw();
                }
                let hit = self.hit_at(*pos);
                let down = self.down.take();
                self.press = Tween::new(self.press.value(), 0.0, dur::SLOW, curves::SPRING_POP);
                if let (Some(h), Some(d)) = (hit, down) {
                    if h == d {
                        self.click(cx, h, mods.shift);
                    }
                }
                Response { consumed: down.is_some(), redraw: true, ..Response::NONE }
            }
            InputEvent::MouseUp { .. } if self.edit_drag => {
                self.edit_drag = false;
                Response::redraw()
            }
            InputEvent::MouseLeave => {
                self.hover = None;
                self.wipe_hover.retarget(0.0, dur::FAST, curves::EASE_OUT);
                Response::redraw()
            }
            InputEvent::Wheel { .. } => {
                let list = paint::layout(self.rect).list;
                self.scroll.event(ev, list)
            }
            InputEvent::Key { code, mods, down: true, .. } => {
                // ctrl-chords belong to the field when it has the keyboard; everything else is the list's
                self.key_event(ev, *code, *mods, cx)
            }
            InputEvent::Key { down: false, .. } if self.focused => {
                self.forward_edit(ev);
                Response::NONE
            }
            InputEvent::Char(c) => {
                if self.focused {
                    self.forward_edit(ev);
                    return Response { consumed: true, redraw: true, ..Response::NONE };
                }
                if c.is_control() {
                    return Response::NONE;
                }
                self.act(cx, Action::Typed(*c));
                if *c != '/' {
                    self.forward_edit(ev);
                }
                Response { consumed: true, redraw: true, ..Response::NONE }
            }
            InputEvent::Focus(false) => {
                self.input.set_focused(false);
                Response::redraw()
            }
            _ => Response::NONE,
        }
    }

    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx) {
        let _ = cx;
        paint::draw(self, cv, theme, rect);
    }
}

impl ClipboardView {
    /// Feed the queued field events to the widget (needs a canvas) and forward text changes to the model. Called from `draw`.
    pub(crate) fn drain_edit(&mut self, cv: &Canvas, search: Rect) {
        let evs = std::mem::take(&mut self.pending_edit);
        for ev in evs {
            self.input.event(&ev, search, cv);
        }
        self.input.update(search, cv);
        for e in self.input.take_events() {
            if e == EditEvent::Changed {
                let q = self.input.text();
                if q != self.model.query() {
                    // no call comes back from `Search`: the debounced load is the model's tick
                    let _ = self.model.on_action(Action::Search(q));
                    self.dirty = true;
                }
            }
        }
    }
}
