//! The native capture overlay: one `CaptureView` per monitor (window `capture-N`, a host `Fullscreen` window), the Rust replacement of
//! `capture.html` + `capture.js`. Spec: `docs/native-capture.md`.
//!
//! The logic is `pill_ui_core::capture::CaptureModel` (selection state machine, markup history, loupe, toolbar, animation cues). This module
//! is the renderer and the wiring around it:
//!
//! * `mod.rs`     the `View`: glasskit input -> model `Input`, model `Action`s -> backend through [`io::ViewIo`], per-frame animation state
//! * `paint.rs`   the scene: frozen frame, dim + cut-out, handles, size badge, markup, toolbar, tips, loupe, flash, toast
//! * `tools.rs`   Direct2D execution of `DrawCmd`s (shared with the export), pixelation, text metrics
//! * `flatten.rs` the marked-up PNG for `capture_commit` (offscreen render + WIC encode, on a worker thread)
//! * `windows.rs` creating / reusing / hiding the host windows for `capture.rs` (the gated native path)
//! * `io.rs`      the model's I/O seam;  `icons.rs` the toolbar icons
pub mod flatten;
pub mod icons;
pub mod io;
mod paint;
pub mod tools;
pub mod windows;

#[cfg(test)]
mod tests;

use crate::ui::view::{Cx, CursorShape, Msg, View};
use glasskit::ease::{curves, Tween};
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, KeyCode, Modifiers, MouseButton, Response};
use glasskit::lang::Lang as GkLang;
use glasskit::theme::Theme;
use glasskit::Canvas;
use io::{Backend, IoState, ViewIo};
use pill_ui_core::capture as cap;
use pill_ui_core::capture::{Action, Button, CaptureModel, Input, Key, Mods, Options, Phase, Pt, Reaction};
use pill_ui_core::i18n::Lang as UiLang;
use std::collections::HashMap;
use std::sync::Arc;

/// The dim over the frozen frame fades in over this long (the screen "settles" instead of snapping); the hint follows.
pub const DIM_RAMP_MS: f64 = 140.0;
pub const HINT_IN_MS: f64 = 200.0;
/// Tooltips of the toolbar wait this long after the pointer rests on a button.
pub const TIP_DELAY_MS: f64 = 480.0;
/// A toast stays this long (plus its fades).
pub const TOAST_MS: f64 = 2200.0;
/// After a commit / cancel the overlay waits for `capture:end` this long, then closes itself (a failed command must not trap the screen).
pub const STUCK_MS: u64 = 1800;
/// After `capture:end` the window waits for the exit animations (copy flash, toolbar fade) but never longer than this.
pub const LINGER_MS: f64 = 420.0;

/// Messages the view posts to itself.
enum SelfMsg {
    /// Take the keyboard (posted after the window is shown, so the host can activate a visible window).
    Focus,
    /// A commit / cancel went out `STUCK_MS` ago for this session id: if the overlay is still up, close it.
    Stuck(u64),
}

/// Hover / press animation state of one toolbar item (CSS `transition`s of `#bar button`).
struct ItemFx {
    hover: Tween<f32>,
    press: Tween<f32>,
}

impl ItemFx {
    fn new() -> ItemFx {
        ItemFx { hover: Tween::at_rest(0.0), press: Tween::at_rest(1.0) }
    }
    fn retarget(&mut self, hover: bool, pressed: bool) {
        self.hover.retarget(if hover { 1.0 } else { 0.0 }, 0.12, curves::EASE_OUT);
        if pressed {
            self.press.retarget(0.92, 0.08, curves::EASE_OUT);
        } else {
            self.press.retarget(1.0, 0.38, curves::SPRING_POP);
        }
    }
    fn step(&mut self, dt: f32) -> bool {
        let a = self.hover.step(dt);
        let b = self.press.step(dt);
        a || b
    }
}

pub(crate) struct Toast {
    pub text: String,
    pub since: f64,
}

/// One capture overlay (one monitor).
pub struct CaptureView {
    label: String,
    model: CaptureModel,
    opts: Options,
    io: IoState,
    be: Backend,
    pub(crate) vm: cap::CaptureView,
    lang: UiLang,
    gpu: Option<paint::Gpu>,
    /// The frozen frame (RGBA) the overlay paints; outlives the model's own copy so the exit animations still have a picture.
    frame_rgba: Option<Arc<Vec<u8>>>,
    frame_dirty: bool,
    fx: HashMap<String, ItemFx>,
    armed_ms: f64,
    hover_id: Option<String>,
    hover_since: f64,
    /// The second click of a double click: delivered after its pointer-up, like the DOM's `dblclick`.
    dbl: Option<Pt>,
    /// `capture:end` arrived at this time: close once the exit animations are done.
    ending: Option<f64>,
    toast: Option<Toast>,
    now: f64,
    pub(crate) viewport: (f32, f32),
    pub(crate) scale: f32,
}

impl CaptureView {
    /// A view for window `label` whose client area is `vw x vh` logical px. Arms at once when the backend already has a session for it.
    pub fn new(cx: &mut Cx, label: &str, vw: f64, vh: f64) -> CaptureView {
        CaptureView::with_backend(cx, label, vw, vh, Backend::real(), Self::default_options(cx))
    }

    /// The options the app uses: the web page's behaviour, plus the native extensions when `capture.extras` is on.
    pub fn default_options(cx: &Cx) -> Options {
        let extras = crate::settings::get_bool("capture.extras", false);
        let base = if extras { Options { extended_tools: true, ext_actions: true, edit_ops: true, ..Options::default() } } else { Options::web_parity() };
        Options { reduce_motion: cx.reduced_motion, ..base }
    }

    pub fn with_backend(cx: &mut Cx, label: &str, vw: f64, vh: f64, be: Backend, opts: Options) -> CaptureView {
        cx.subscribe("capture:begin");
        cx.subscribe("capture:end");
        cx.subscribe("capture:done");
        cx.set_cursor(CursorShape::Crosshair);
        let model = CaptureModel::new(label, vw, vh, opts.clone());
        let lang = ui_lang(cx.lang);
        let vm = model.view(&lang);
        let mut v = CaptureView {
            label: label.to_string(),
            model,
            opts,
            io: IoState::default(),
            be,
            vm,
            lang,
            gpu: None,
            frame_rgba: None,
            frame_dirty: false,
            fx: HashMap::new(),
            armed_ms: 0.0,
            hover_id: None,
            hover_since: 0.0,
            dbl: None,
            ending: None,
            toast: None,
            now: cx.now_ms,
            viewport: (vw as f32, vh as f32),
            scale: cx.scale,
        };
        v.begin(cx);
        v
    }

    /// Fetches the session (`capture_session`), the frame, tells the backend we are ready and shows the window.
    pub fn begin(&mut self, cx: &mut Cx) {
        if cx.reduced_motion != self.opts.reduce_motion {
            self.opts.reduce_motion = cx.reduced_motion;
            let (vw, vh) = self.model.viewport();
            self.model = CaptureModel::new(&self.label, vw, vh, self.opts.clone());
        }
        self.sync_now(cx);
        self.input(Input::Begin, cx);
        if self.io.yielded {
            self.frame_rgba = self.io.frame.clone();
            self.frame_dirty = true;
            self.armed_ms = cx.now_ms;
            self.ending = None;
            self.toast = None;
            self.fx.clear();
            self.hover_id = None;
            cx.show_self();
            if self.io.focus {
                cx.post(Msg::Custom(Box::new(SelfMsg::Focus)));
            }
            cx.request_frame();
        }
    }

    fn sync_now(&mut self, cx: &Cx) {
        self.now = cx.now_ms;
        self.lang = ui_lang(cx.lang);
        self.scale = cx.scale;
    }

    /// Feeds the model one event, runs what it asked for (jobs, toasts, the stuck guard) and refreshes the view-model.
    fn input(&mut self, ev: Input, cx: &mut Cx) -> Reaction {
        self.io.begin_call(cx.now_ms as u64);
        let was = self.model.phase();
        let r = {
            let mut io = ViewIo { cx, st: &mut self.io, be: &self.be };
            self.model.on_input(ev, &mut io)
        };
        self.after(was, &r, cx);
        r
    }

    fn after(&mut self, was: Phase, r: &Reaction, cx: &mut Cx) {
        for a in &r.actions {
            if matches!(a, Action::PinToScreen) {
                // the backend has no `capture_pin`: the file is saved, say so instead of failing silently
                self.toast(cx, &cap_tr(self.lang, "Закрепление на экране пока недоступно — снимок сохранён", "Pin to screen is not available yet — the shot was saved"));
            }
        }
        for job in self.io.jobs.drain(..) {
            flatten::spawn_commit(job, self.io.frame.clone(), self.model.frame_size(), cx.caller.clone(), self.be.raw.clone(), cx.waker(), self.model.session_id());
        }
        if self.model.phase() == Phase::Committed && was != Phase::Committed {
            let (w, id) = (cx.waker(), self.model.session_id());
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(STUCK_MS));
                w.post_custom(SelfMsg::Stuck(id));
            });
        }
        self.refresh(cx);
    }

    /// Re-derives the view-model and the per-item animation targets.
    pub(crate) fn refresh(&mut self, cx: &mut Cx) {
        self.vm = self.model.view(&self.lang);
        let bar = self.vm.toolbar.as_ref().or(self.vm.ghost.as_ref().and_then(|g| g.toolbar.as_ref())).cloned();
        if let Some(tb) = bar {
            for it in &tb.items {
                if it.id == "sep" {
                    continue;
                }
                self.fx.entry(it.id.clone()).or_insert_with(ItemFx::new).retarget(it.hover, it.pressed);
            }
            let h = tb.items.iter().find(|i| i.hover).map(|i| i.id.clone());
            if h != self.hover_id {
                self.hover_id = h;
                self.hover_since = cx.now_ms;
            }
        } else {
            self.fx.clear();
            self.hover_id = None;
        }
        // IME candidate window next to the text caret
        if let Some(t) = &self.vm.text_edit {
            cx.set_ime_rect(Rect::new(t.pos.x as f32, t.pos.y as f32, 1.0, 20.0));
        }
    }

    pub(crate) fn toast(&mut self, cx: &mut Cx, text: &str) {
        self.toast = Some(Toast { text: text.to_string(), since: cx.now_ms });
        cx.request_frame();
    }

    fn cursor_for(&self, p: Pt) -> CursorShape {
        if self.vm.toolbar.as_ref().is_some_and(|t| t.rect.contains(p)) {
            return CursorShape::Arrow;
        }
        match self.vm.cursor {
            cap::Cursor::Default | cap::Cursor::Crosshair => CursorShape::Crosshair,
            cap::Cursor::Text => CursorShape::Text,
            cap::Cursor::Move => CursorShape::Move,
            cap::Cursor::NsResize => CursorShape::ResizeNs,
            cap::Cursor::EwResize => CursorShape::ResizeEw,
            // the host has no diagonal resize cursors yet: the four-way arrow reads as "grab the corner"
            cap::Cursor::NwseResize | cap::Cursor::NeswResize => CursorShape::Move,
        }
    }

    fn text_open(&self) -> bool {
        self.model.text_input().is_some()
    }

    /// True while the overlay still has something to animate (frames keep coming at the display rate), false = idle.
    fn busy(&self, now: f64) -> bool {
        let ramp = !self.opts.reduce_motion && now - self.armed_ms < DIM_RAMP_MS.max(HINT_IN_MS) + 60.0;
        let tip_wait = self.hover_id.is_some() && now - self.hover_since < TIP_DELAY_MS + 160.0;
        let toast = self.toast.as_ref().is_some_and(|t| now - t.since < TOAST_MS + 400.0);
        self.model.animating() || ramp || tip_wait || toast || self.text_open() || self.fx.values().any(|f| !f.hover.finished() || !f.press.finished())
    }
}

fn ui_lang(l: GkLang) -> UiLang {
    if l == GkLang::Ru {
        UiLang::Ru
    } else {
        UiLang::En
    }
}

pub(crate) fn cap_tr(l: UiLang, ru: &str, en: &str) -> String {
    use pill_ui_core::i18n::Tr;
    l.t(ru, en)
}

fn pt(p: (f32, f32)) -> Pt {
    Pt::new(p.0 as f64, p.1 as f64)
}

fn mods(m: Modifiers) -> Mods {
    Mods { shift: m.shift, ctrl: m.ctrl, alt: m.alt, meta: m.meta }
}

/// glasskit key -> model key. While the text tool is open only control keys go through (typed characters arrive as `Char`).
pub(crate) fn map_key(code: KeyCode, m: Modifiers, editing: bool) -> Option<Key> {
    Some(match code {
        KeyCode::Escape => Key::Escape,
        KeyCode::Enter => Key::Enter,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Char(c) if !editing && m.any_command() => Key::Char(c.to_ascii_lowercase()),
        _ => return None,
    })
}

impl View for CaptureView {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.sync_now(cx);
        let mut resp = Response::NONE;
        let mut pos = None;
        let reaction = match ev {
            InputEvent::MouseMove { pos: p, mods: m, .. } => {
                pos = Some(pt(*p));
                Some(self.input(Input::PointerMove { pos: pt(*p), mods: mods(*m) }, cx))
            }
            InputEvent::MouseDown { pos: p, button, mods: m, clicks } => {
                pos = Some(pt(*p));
                let b = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    _ => Button::Middle,
                };
                if *button == MouseButton::Left {
                    resp.capture = true;
                    if *clicks >= 2 {
                        self.dbl = Some(pt(*p));
                    }
                }
                let r = self.input(Input::PointerDown { pos: pt(*p), button: b, mods: mods(*m) }, cx);
                // a click gives this window the keyboard (the overlay of another monitor may hold it)
                if *button == MouseButton::Left {
                    cx.set_focus(true);
                }
                Some(r)
            }
            InputEvent::MouseUp { pos: p, button, mods: m } => {
                pos = Some(pt(*p));
                let b = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    _ => Button::Middle,
                };
                let mut r = self.input(Input::PointerUp { pos: pt(*p), button: b, mods: mods(*m) }, cx);
                if let (Some(d), MouseButton::Left) = (self.dbl.take(), button) {
                    let r2 = self.input(Input::DoubleClick { pos: d }, cx);
                    r.actions.extend(r2.actions);
                    r.redraw |= r2.redraw;
                }
                Some(r)
            }
            InputEvent::MouseLeave => {
                if self.model.phase() != Phase::Dragging {
                    // nothing under the pointer any more: clear the toolbar hover
                    Some(self.input(Input::PointerMove { pos: Pt::new(-1.0e4, -1.0e4), mods: Mods::NONE }, cx))
                } else {
                    None
                }
            }
            InputEvent::Key { code, mods: m, down: true, .. } => {
                let editing = self.text_open();
                if editing && m.ctrl && *code == KeyCode::Char('V') {
                    cx.clipboard_text().map(|t| self.input(Input::Text(t.replace(['\r', '\n'], " ")), cx))
                } else {
                    map_key(*code, *m, editing).map(|k| self.input(Input::Key { key: k, mods: mods(*m) }, cx))
                }
            }
            InputEvent::Char(c) if self.text_open() => Some(self.input(Input::Text(c.to_string()), cx)),
            _ => None,
        };
        if let Some(r) = reaction {
            resp.redraw = r.redraw;
            resp.consumed = r.prevent_default;
        }
        if let Some(p) = pos {
            cx.set_cursor(self.cursor_for(p));
        }
        resp
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.sync_now(cx);
        match msg {
            Msg::Event { name, payload, .. } => match name.as_str() {
                "capture:begin" => {
                    self.begin(cx);
                    return Response::redraw();
                }
                "capture:end" => {
                    self.input(Input::End, cx);
                    // let the exit animations (copy flash, toolbar fade, a toast) finish, then close
                    if self.model.animating() || self.toast.is_some() {
                        self.ending = Some(cx.now_ms);
                        cx.request_frame();
                    } else {
                        self.close_overlay(cx);
                    }
                    return Response::redraw();
                }
                "capture:done" => {
                    // pin / shelf after a save: the saved path is only known now
                    if payload.get("kind").and_then(|k| k.as_str()) == Some("screenshot") {
                        if let (Some(path), Some(after)) = (payload.get("path").and_then(|p| p.as_str()), self.model.take_after_save()) {
                            if after == cap::AfterSave::Shelf {
                                let call = after.call(path);
                                cx.call_async(&call.cmd, call.args);
                            }
                        }
                    }
                }
                _ => {}
            },
            Msg::Reply { result: Err(e), .. } => {
                eprintln!("capture: backend call failed: {e}");
                if self.model.phase() == Phase::Committed {
                    self.close_overlay(cx);
                }
            }
            Msg::Custom(b) => match b.downcast::<SelfMsg>() {
                Ok(m) => match *m {
                    SelfMsg::Focus => cx.set_focus(true),
                    SelfMsg::Stuck(id) => {
                        let _ = id;
                        if self.model.phase() == Phase::Committed {
                            self.close_overlay(cx);
                        }
                    }
                },
                Err(b) => {
                    if let Ok(done) = b.downcast::<flatten::Done>() {
                        if !done.0 {
                            // the markup could not be rendered: the plain commit already went out without it
                            self.toast(cx, &cap_tr(self.lang, "Разметка не применилась — сохранён снимок без неё", "The markup could not be applied — the plain shot was saved"));
                        }
                    }
                }
            },
            _ => {}
        }
        Response::NONE
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.sync_now(cx);
        let now = cx.now_ms;
        let mut busy = self.model.tick(now as u64);
        let dt = (dt_ms / 1000.0) as f32;
        for f in self.fx.values_mut() {
            busy |= f.step(dt);
        }
        if let Some(t) = &self.toast {
            if now - t.since > TOAST_MS + 400.0 {
                self.toast = None;
            }
        }
        self.vm = self.model.view(&self.lang);
        if let Some(e) = self.ending {
            if !self.model.animating() && self.toast.is_none() || now - e > LINGER_MS.max(TOAST_MS) {
                self.ending = None;
                self.close_overlay(cx);
                return false;
            }
        }
        busy || self.busy(now)
    }

    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx) {
        paint::paint(self, cv, theme, rect, cx);
    }

    fn on_hide(&mut self, _cx: &mut Cx) {
        // give the megabytes back: the frame (CPU copy and GPU bitmap) is only useful while the overlay is up
        self.frame_rgba = None;
        self.gpu = None;
    }

    fn on_dpi(&mut self, scale: f32, _cx: &mut Cx) {
        self.scale = scale;
    }
}

impl CaptureView {
    /// Hide the window (the model was reset by `capture:end` or the guard fired) and drop the picture.
    fn close_overlay(&mut self, cx: &mut Cx) {
        cx.hide_self();
    }
}
