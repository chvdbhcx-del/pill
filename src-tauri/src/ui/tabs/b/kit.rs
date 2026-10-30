//! Shared plumbing of the set-B views (note, text, chat, agents, ai_actions, ai_setup): running the models' `BackendCall`s, hover / press
//! animation of retained click targets, animation cues, the line-icon registry, button / chip / card painting in the look of `theme.css`,
//! and a text field whose input is deferred to the next draw (a `View::event` has no `Canvas`, but `TextInput` needs one to hit-test).
//!
//! # The View pattern used by every set-B view
//! * `event()`  : pointer / key input. Click targets live in a [`Clicks`] filled by the LAST draw, so hover / press / click are answered at
//!   once. Text input is queued in a [`Field`] (or the rich editor) and applied at the start of the next `draw` (one frame).
//! * `message()`: `Msg::Event` -> `model.on_event`, `Msg::Reply` -> [`Calls::take`] -> `model.on_reply`, both followed by [`Calls::run`].
//! * `step()`   : `model.tick(dt)`, [`Cues`] and control animations, effects drained with [`route_effects`]; returns "still moving".
//! * `draw()`   : lay out from `rect` (never assume 0,0), refill `Clicks`, paint with the helpers below.
use super::fx_b::{emit, TabFx};
use crate::ui::view::{Cx, Ticket};
use glasskit::canvas::Canvas;
use glasskit::ease::{curves, dur, Ease, Lerp, Tween};
use glasskit::edit::{EditEvent, TextInput};
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::svgpath::{GeomCache, Svg};
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::{Anim, AnimCue, BackendCall, Effect, Reply};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

// ---------------------------------------------------------------------------------------------------------------------------------------
// Backend calls
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Runs the `BackendCall`s a model returns through `Cx::call_async` and maps the ticket of each reply back to the model's `tag`.
#[derive(Default)]
pub struct Calls {
    map: HashMap<u64, u64>,
}

impl Calls {
    pub fn new() -> Calls {
        Calls::default()
    }

    /// Start every call (the facade runs them on its worker pool).
    pub fn run(&mut self, cx: &mut Cx, calls: Vec<BackendCall>) {
        for c in calls {
            let t = cx.call_async(c.cmd, c.args);
            self.map.insert(t.0, c.tag);
        }
    }

    /// The `(tag, Reply)` of a `Msg::Reply`, `None` when the ticket was not started here.
    pub fn take(&mut self, ticket: Ticket, result: Result<Value, String>) -> Option<(u64, Reply)> {
        let tag = self.map.remove(&ticket.0)?;
        Some((tag, match result {
            Ok(v) => Reply::ok(v),
            Err(e) => Reply::err(&e),
        }))
    }

    pub fn pending(&self) -> usize {
        self.map.len()
    }
}

/// Handles the effects that need no view state (clipboard, island effects) and returns the rest (`ScrollToBottom`, `Focus`, `SetEditorHtml`,
/// `EditorExec`, `ReplaceRange`, `ScrollIntoView`) for the view.
pub fn route_effects(fx: Vec<Effect>, cx: &mut Cx) -> Vec<Effect> {
    let mut rest = Vec::new();
    for e in fx {
        match e {
            Effect::CopyText(t) => cx.set_clipboard_text(&t),
            Effect::Card(c) => emit(TabFx::Card(c)),
            Effect::SetLive { key, row } => emit(TabFx::SetLive { key, row }),
            Effect::React { key, kind, ttl_ms } => emit(TabFx::React { key, kind, ttl_ms }),
            Effect::Unreact(k) => emit(TabFx::Unreact(k)),
            Effect::EdgePulse(s) => emit(TabFx::EdgePulse(s)),
            Effect::SetBadge { tab, count } => emit(TabFx::SetBadge { tab, count }),
            Effect::CollapsePill => emit(TabFx::CollapsePill),
            Effect::VoiceToggle => emit(TabFx::VoiceToggle),
            Effect::GoPage(p) => emit(TabFx::GoPage(p)),
            other => rest.push(other),
        }
    }
    rest
}

/// Converts `dt` of a `View::step` to what the models want: whole milliseconds, at least 1 so a 240 Hz frame is never rounded away
/// (the remainder is carried in `carry`).
pub fn ms_of(dt_ms: f64, carry: &mut f64) -> u32 {
    *carry += dt_ms;
    let whole = carry.floor();
    *carry -= whole;
    whole as u32
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Click targets with hover / press animation
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Retained click targets: the draw registers `(key, rect)`, `event` answers hover / press / click against the rects of the last draw.
/// Hover mixes colours over 120 ms, a press shrinks to .96 in 80 ms and springs back in 380 ms (`theme.css` tokens); reduced motion jumps.
pub struct Clicks<K: Clone + Eq + Hash> {
    rects: Vec<(K, Rect)>,
    hover: Option<K>,
    down: Option<K>,
    hov: HashMap<K, Tween<f32>>,
    prs: HashMap<K, Tween<f32>>,
    /// keys whose rect is a text-like area (cursor I-beam instead of the hand)
    pub text_keys: Vec<K>,
    pub reduced: bool,
}

impl<K: Clone + Eq + Hash> Default for Clicks<K> {
    fn default() -> Self {
        Clicks { rects: Vec::new(), hover: None, down: None, hov: HashMap::new(), prs: HashMap::new(), text_keys: Vec::new(), reduced: false }
    }
}

impl<K: Clone + Eq + Hash> Clicks<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start of a draw: forget last frame's rects (hover / press animations stay).
    pub fn begin(&mut self) {
        self.rects.clear();
    }

    /// Register a target; later registrations are on top.
    pub fn add(&mut self, k: K, r: Rect) {
        self.rects.push((k, r));
    }

    pub fn rect_of(&self, k: &K) -> Option<Rect> {
        self.rects.iter().rev().find(|(x, _)| x == k).map(|(_, r)| *r)
    }

    pub fn hit(&self, p: Pt) -> Option<K> {
        self.rects.iter().rev().find(|(_, r)| r.contains(p.0, p.1)).map(|(k, _)| k.clone())
    }

    pub fn hovered(&self) -> Option<&K> {
        self.hover.as_ref()
    }

    pub fn is_down(&self, k: &K) -> bool {
        self.down.as_ref() == Some(k)
    }

    /// Eased hover amount 0..1 of `k`.
    pub fn hover_amt(&self, k: &K) -> f32 {
        self.hov.get(k).map_or(0.0, |t| t.value().clamp(0.0, 1.0))
    }

    /// Scale of a pressed target (1 at rest, .96 held, a springy overshoot above 1 on release); `depth` .96 for buttons, .92 for icon buttons.
    pub fn press_scale(&self, k: &K, depth: f32) -> f32 {
        1.0 - (1.0 - depth) * self.prs.get(k).map_or(0.0, |t| t.value())
    }

    fn retarget(map: &mut HashMap<K, Tween<f32>>, k: &K, to: f32, d: f32, e: Ease, reduced: bool) {
        let t = map.entry(k.clone()).or_insert_with(|| Tween::at_rest(0.0));
        if reduced {
            t.jump(to);
        } else {
            t.retarget(to, d, e);
        }
    }

    fn set_hover(&mut self, to: Option<K>) -> bool {
        if self.hover == to {
            return false;
        }
        if let Some(old) = self.hover.take() {
            Self::retarget(&mut self.hov, &old, 0.0, dur::FAST, curves::EASE_OUT, self.reduced);
        }
        if let Some(n) = &to {
            Self::retarget(&mut self.hov, n, 1.0, dur::FAST, curves::EASE_OUT, self.reduced);
        }
        self.hover = to;
        true
    }

    /// Pointer input. Returns the response and the key that was clicked (press and release on the same target).
    pub fn event(&mut self, ev: &InputEvent) -> (Response, Option<K>) {
        let mut r = Response::NONE;
        let mut clicked = None;
        match ev {
            InputEvent::MouseMove { pos, .. } => {
                let over = self.hit(*pos);
                if self.set_hover(over.clone()) {
                    r.redraw = true;
                }
                if let Some(k) = &over {
                    r.cursor = Some(if self.text_keys.contains(k) { Cursor::Text } else { Cursor::Hand });
                }
                if let Some(d) = self.down.clone() {
                    r.capture = true;
                    let inside = over.as_ref() == Some(&d);
                    Self::retarget(&mut self.prs, &d, inside as u8 as f32, dur::XS, curves::EASE_OUT, self.reduced);
                    r.redraw = true;
                }
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => {
                if let Some(k) = self.hit(*pos) {
                    self.set_hover(Some(k.clone()));
                    Self::retarget(&mut self.prs, &k, 1.0, dur::XS, curves::EASE_OUT, self.reduced);
                    self.down = Some(k);
                    r = Response { consumed: true, redraw: true, capture: true, cursor: Some(Cursor::Hand), ..Response::NONE };
                }
            }
            InputEvent::MouseUp { pos, button: MouseButton::Left, .. } => {
                if let Some(d) = self.down.take() {
                    Self::retarget(&mut self.prs, &d, 0.0, dur::SLOW, curves::SPRING_POP, self.reduced);
                    let over = self.hit(*pos);
                    self.set_hover(over.clone());
                    if over.as_ref() == Some(&d) {
                        clicked = Some(d);
                    }
                    r = Response { consumed: true, redraw: true, changed: clicked.is_some(), ..Response::NONE };
                }
            }
            InputEvent::MouseLeave => {
                if self.set_hover(None) {
                    r.redraw = true;
                }
            }
            _ => {}
        }
        (r, clicked)
    }

    /// Advance the animations by `dt` seconds; true while any is moving.
    pub fn step(&mut self, dt: f32) -> bool {
        let mut busy = false;
        for t in self.hov.values_mut().chain(self.prs.values_mut()) {
            busy |= t.step(dt);
        }
        // drop settled entries of keys that are gone, so a list that churns does not grow the maps
        if self.hov.len() > 64 {
            self.hov.retain(|k, t| !t.finished() || t.value() > 0.0 || self.hover.as_ref() == Some(k));
            self.prs.retain(|_, t| !t.finished() || t.value() != 0.0);
        }
        busy
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Animation cues
// ---------------------------------------------------------------------------------------------------------------------------------------

/// What a view applies to the element a cue targets: opacity, a vertical offset, a scale about its centre, and the raw progress 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub alpha: f32,
    pub dy: f32,
    pub scale: f32,
    pub p: f32,
}

impl Xf {
    pub const ID: Xf = Xf { alpha: 1.0, dy: 0.0, scale: 1.0, p: 1.0 };
}

struct Live {
    target: String,
    kind: Anim,
    t_ms: f32,
    dur_ms: f32,
}

/// Plays the `AnimCue`s a model emits. Look up an element's transform with [`Cues::xf`] while painting; `step` returns true while any plays.
/// With reduced motion nothing is stored (the models already emit no cues then, this is the second line of defence).
#[derive(Default)]
pub struct Cues {
    live: Vec<Live>,
    pub reduced: bool,
}

impl Cues {
    pub fn new() -> Cues {
        Cues::default()
    }

    pub fn play(&mut self, c: AnimCue) {
        if self.reduced {
            return;
        }
        let dur_ms = match c.kind {
            Anim::Enter { ms, .. } | Anim::Exit { ms, .. } | Anim::Pop { ms, .. } => (ms as f32).max(60.0),
            Anim::Press => 260.0,
            Anim::Collapse | Anim::Expand => 220.0,
            Anim::Flip => 380.0,
            Anim::Stagger => 400.0,
            // looping kinds are painted by the view from its own clock
            Anim::TypingDots | Anim::Caret | Anim::Spin | Anim::ProgressSweep => return,
        };
        self.live.retain(|l| l.target != c.target);
        self.live.push(Live { target: c.target, kind: c.kind, t_ms: 0.0, dur_ms });
    }

    pub fn play_all(&mut self, cues: Vec<AnimCue>) {
        for c in cues {
            self.play(c);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    pub fn step(&mut self, dt_ms: f32) -> bool {
        for l in &mut self.live {
            l.t_ms += dt_ms;
        }
        self.live.retain(|l| l.t_ms < l.dur_ms);
        !self.live.is_empty()
    }

    /// The transform of `target` right now (identity when nothing plays on it).
    pub fn xf(&self, target: &str) -> Xf {
        let Some(l) = self.live.iter().find(|l| l.target == target) else { return Xf::ID };
        let p = (l.t_ms / l.dur_ms).clamp(0.0, 1.0);
        match l.kind {
            Anim::Enter { y, .. } => Xf { alpha: curves::EASE_OUT.at(p), dy: y * (1.0 - curves::SPRING_SETTLE.at(p)), scale: 1.0, p },
            Anim::Exit { y, scale, .. } => {
                let e = curves::EASE_IN.at(p);
                Xf { alpha: 1.0 - e, dy: y * e, scale: f32::lerp(1.0, scale, e), p }
            }
            Anim::Pop { from_scale, .. } => Xf { alpha: curves::EASE_OUT.at((p * 2.0).min(1.0)), dy: 0.0, scale: f32::lerp(from_scale, 1.0, curves::SPRING_POP.at(p)), p },
            Anim::Press => Xf { alpha: 1.0, dy: 0.0, scale: 1.0 - 0.04 * (p * std::f32::consts::PI).sin(), p },
            Anim::Collapse => Xf { alpha: 1.0 - curves::EASE_IN.at(p), dy: 0.0, scale: 1.0, p },
            Anim::Expand => Xf { alpha: curves::EASE_OUT.at(p), dy: 0.0, scale: 1.0, p },
            _ => Xf { p, ..Xf::ID },
        }
    }

    /// Paint `f` under the transform of `target` (opacity + offset + scale about `about`).
    pub fn apply(&self, cv: &Canvas, target: &str, about: Pt, f: impl FnOnce(&Canvas)) {
        let x = self.xf(target);
        if x == Xf::ID {
            return f(cv);
        }
        cv.with_opacity(x.alpha, |cv| cv.with_offset(0.0, x.dy, |cv| cv.with_scale_about(about, x.scale, f)));
    }
}

/// Phase 0..1 of a looping animation of period `period_ms` from the host clock (typing dots, caret, spinner sweep).
pub fn phase(now_ms: f64, period_ms: f64) -> f32 {
    ((now_ms % period_ms) / period_ms) as f32
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Icons
// ---------------------------------------------------------------------------------------------------------------------------------------

/// The line icons of `icons.js` (+ any a view adds) as cached SVGs: `icons.draw(cv, "bell", rect, color)`.
pub struct Icons {
    cache: GeomCache,
    svgs: RefCell<HashMap<(String, u32), Rc<Svg>>>,
    extra: HashMap<String, String>,
}

impl Default for Icons {
    fn default() -> Self {
        Icons::new()
    }
}

impl Icons {
    pub fn new() -> Icons {
        Icons { cache: GeomCache::new(), svgs: RefCell::new(HashMap::new()), extra: HashMap::new() }
    }

    /// Register a view's own icon (inner SVG markup of a 24 x 24 grid).
    pub fn add(&mut self, name: &str, markup: &str) {
        self.extra.insert(name.to_string(), markup.to_string());
    }

    fn svg(&self, name: &str, sw: f32) -> Option<Rc<Svg>> {
        let key = (name.to_string(), (sw * 100.0) as u32);
        if let Some(s) = self.svgs.borrow().get(&key) {
            return Some(s.clone());
        }
        let markup = self.extra.get(name).map(|s| s.as_str()).or_else(|| super::icons::markup(name))?;
        let s = Rc::new(Svg::icon(markup, sw));
        self.svgs.borrow_mut().insert(key, s.clone());
        Some(s)
    }

    /// Stroke 1.7 (the web default).
    pub fn draw(&self, cv: &Canvas, name: &str, r: Rect, c: Color) {
        self.draw_sw(cv, name, r, c, 1.7);
    }

    pub fn draw_sw(&self, cv: &Canvas, name: &str, r: Rect, c: Color, sw: f32) {
        if let Some(s) = self.svg(name, sw) {
            s.draw(cv, r, c, &self.cache);
        }
    }

    pub fn has(&self, name: &str) -> bool {
        self.extra.contains_key(name) || super::icons::markup(name).is_some()
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Painting helpers (theme.css building blocks)
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bk {
    Normal,
    Primary,
    Ghost,
    Danger,
    /// round icon-only button
    Icon,
}

/// A `.btn`: fill / label colours mixed by `hover`, scaled by `pscale` (from [`Clicks::press_scale`]), faded when disabled.
#[allow(clippy::too_many_arguments)]
pub fn button(cv: &Canvas, th: &Theme, ic: &Icons, r: Rect, label: &str, icon: Option<&str>, kind: Bk, hover: f32, pscale: f32, enabled: bool) {
    let (bg, fg) = match kind {
        Bk::Normal => (th.btn.mix(th.btn_hover, hover), th.text),
        Bk::Primary => (th.accent.mix(th.accent_hover, hover), th.on_accent),
        Bk::Ghost => (th.hover.alpha(0.0).mix(th.hover, hover), th.dim.mix(th.text, hover)),
        Bk::Danger => (th.btn.mix(th.btn.mix(th.red, th.red_bg.a), hover), th.red),
        Bk::Icon => (th.hover.alpha(0.0).mix(th.hover, hover), th.icon.mix(th.text, hover)),
    };
    let rad = if kind == Bk::Icon { r.h / 2.0 } else { th.m.btn_r };
    cv.with_opacity(if enabled { 1.0 } else { 0.45 }, |cv| {
        cv.with_scale_about(r.center(), pscale, |cv| {
            cv.fill_rr(r, rad, bg);
            let mut st = th.btn_text();
            if kind == Bk::Primary {
                st = st.weight(600);
            }
            match (icon, label.is_empty()) {
                (Some(i), true) => ic.draw(cv, i, r.centered(18.0, 18.0), fg),
                (Some(i), false) => {
                    let tw = cv.text_width(label, st);
                    let x0 = r.cx() - (18.0 + 6.0 + tw) / 2.0;
                    ic.draw(cv, i, Rect::new(x0, r.cy() - 9.0, 18.0, 18.0), fg);
                    cv.text(Rect::new(x0 + 24.0, r.y, tw + 2.0, r.h), label, st, fg);
                }
                (None, _) => cv.text(r, label, st.center(), fg),
            }
        });
    });
}

/// Natural width of a `button` with this label (+ optional icon).
pub fn button_w(cv: &Canvas, th: &Theme, label: &str, icon: bool, kind: Bk) -> f32 {
    if kind == Bk::Icon {
        return 28.0;
    }
    let mut st = th.btn_text();
    if kind == Bk::Primary {
        st = st.weight(600);
    }
    let tw = if label.is_empty() { 0.0 } else { cv.text_width(label, st).ceil() };
    tw + if icon { 18.0 + if tw > 0.0 { 6.0 } else { 0.0 } } else { 0.0 } + 2.0 * th.m.btn_pad_x
}

/// A `.card`: the raised tile with the pill radius.
pub fn card(cv: &Canvas, th: &Theme, r: Rect) {
    cv.fill_rr(r, th.m.radius, th.card);
}

/// A small label chip (`.chip`): `on` = accent-soft with the accent label, else the raised fill.
pub fn chip(cv: &Canvas, th: &Theme, r: Rect, label: &str, on: bool, hover: f32, pscale: f32) {
    cv.with_scale_about(r.center(), pscale, |cv| {
        let bg = if on { th.accent_soft } else { th.raised.mix(th.raised2, hover * 0.6) };
        cv.fill_rr(r, r.h / 2.0, bg);
        let fg = if on { th.accent } else { th.text2.mix(th.text, hover) };
        cv.text(r.inset_xy(10.0, 0.0), label, th.small(500).ellipsis(), fg);
    });
}

/// The one-line toast of a tab (`.tn-toast` and friends): a rounded pill with the text, opacity `k`.
pub fn flash(cv: &Canvas, th: &Theme, area: Rect, text: &str, bad: bool, k: f32) {
    if k <= 0.0 || text.is_empty() {
        return;
    }
    let st = th.small(500);
    let w = (cv.text_width(text, st) + 24.0).min(area.w - 16.0);
    let r = Rect::new(area.cx() - w / 2.0, area.y, w, 26.0);
    cv.with_opacity(k, |cv| {
        cv.shadow_rr(r, 13.0, 10.0, 3.0, th.shadow);
        cv.fill_rr(r, 13.0, if bad { th.card } else { th.raised2 });
        if bad {
            cv.fill_rr(r, 13.0, th.red_bg);
        }
        cv.text(r.inset_xy(12.0, 0.0), text, st.ellipsis().center(), if bad { th.red } else { th.text });
    });
}

/// A 1 px divider.
pub fn hline(cv: &Canvas, th: &Theme, x0: f32, x1: f32, y: f32) {
    cv.fill_rect(Rect::new(x0, y, (x1 - x0).max(0.0), 1.0), th.line);
}

/// Text at `r` with a style, returning nothing (shorter than `cv.text` call sites with a `Style`).
pub fn txt(cv: &Canvas, r: Rect, s: &str, st: Style, c: Color) {
    cv.text(r, s, st, c);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Text field with deferred input
// ---------------------------------------------------------------------------------------------------------------------------------------

/// A `TextInput` plus the rectangle of the last draw and an inbox. `View::event` calls [`Field::queue`] / [`Field::predict`]; the draw calls
/// [`Field::pump`] with its canvas, which applies the queued input and collects the `EditEvent`s for the view to read in the next `step`.
pub struct Field {
    pub ti: TextInput,
    pub rect: Rect,
    inbox: Vec<InputEvent>,
    pub events: Vec<EditEvent>,
    dragging: bool,
    focus_req: bool,
}

impl Field {
    pub fn new(multiline: bool, th: &Theme) -> Field {
        Field { ti: TextInput::new(multiline, th), rect: Rect::default(), inbox: Vec::new(), events: Vec::new(), dragging: false, focus_req: false }
    }

    pub fn is_focused(&self) -> bool {
        self.ti.is_focused()
    }

    /// Does this event belong to the field (pointer inside / dragging, key input while focused)?
    pub fn wants(&self, ev: &InputEvent) -> bool {
        match ev {
            InputEvent::MouseDown { pos, .. } | InputEvent::Wheel { pos, .. } => self.rect.contains(pos.0, pos.1),
            InputEvent::MouseMove { pos, .. } | InputEvent::MouseUp { pos, .. } => self.dragging || self.rect.contains(pos.0, pos.1),
            InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_) => self.ti.is_focused(),
            _ => false,
        }
    }

    /// Queue an input event for the next draw and answer the host now.
    pub fn queue(&mut self, ev: &InputEvent) -> Response {
        let r = self.predict(ev);
        if matches!(ev, InputEvent::MouseDown { button: MouseButton::Left, .. }) {
            self.dragging = true;
        }
        if matches!(ev, InputEvent::MouseUp { .. }) {
            self.dragging = false;
        }
        self.inbox.push(ev.clone());
        r
    }

    /// The response the field would give, without the layout (cursor, capture, consumed).
    pub fn predict(&self, ev: &InputEvent) -> Response {
        match ev {
            InputEvent::MouseDown { button: MouseButton::Left, .. } => Response { consumed: true, redraw: true, capture: true, focus: true, cursor: Some(Cursor::Text), ..Response::NONE },
            InputEvent::MouseMove { .. } => Response { redraw: self.dragging, capture: self.dragging, cursor: Some(Cursor::Text), ..Response::NONE },
            InputEvent::MouseUp { .. } => Response { consumed: true, redraw: true, ..Response::NONE },
            InputEvent::Wheel { .. } => Response { consumed: self.ti.model.multiline, redraw: true, ..Response::NONE },
            InputEvent::Key { code, .. } => {
                // keys the field leaves to the focus chain
                let tab = matches!(code, KeyCode::Tab);
                Response { consumed: !tab, redraw: !tab, ..Response::NONE }
            }
            InputEvent::Char(_) => Response { consumed: true, redraw: true, ..Response::NONE },
            _ => Response::NONE,
        }
    }

    /// Apply the queued input with a canvas and keep the scroll extents current; call at the start of the draw with the field's rect.
    /// Returns true when the field asked for the keyboard focus (a click): the view must focus it and blur the others.
    pub fn pump(&mut self, cv: &Canvas, rect: Rect) -> bool {
        self.rect = rect;
        let evs = std::mem::take(&mut self.inbox);
        for ev in evs {
            let r = self.ti.event(&ev, rect, cv);
            self.focus_req |= r.focus;
        }
        self.ti.update(rect, cv);
        self.events.extend(self.ti.take_events());
        std::mem::take(&mut self.focus_req)
    }

    pub fn has_input(&self) -> bool {
        !self.inbox.is_empty()
    }

    /// Edit events since the last call.
    pub fn take_events(&mut self) -> Vec<EditEvent> {
        std::mem::take(&mut self.events)
    }

    /// Where the IME candidate window belongs (the caret, logical px).
    pub fn ime_rect(&self, cv: &Canvas) -> Option<Rect> {
        let (x, y, h) = self.ti.caret_point(self.rect, cv, self.ti.model.caret())?;
        Some(Rect::new(x, y, 1.0, h))
    }
}

/// Focus exactly one of `fields` (`None` blurs all).
pub fn focus_one(fields: &mut [&mut Field], which: Option<usize>) {
    for (i, f) in fields.iter_mut().enumerate() {
        f.ti.set_focused(Some(i) == which);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::{Cx, Services};
    use glasskit::input::{Buttons, Modifiers};
    use pill_ui_core::panel::b::Anim;
    use serde_json::json;

    fn mv(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
    }
    fn down(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseDown { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }
    }
    fn up(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseUp { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE }
    }

    #[test]
    fn calls_map_tickets_to_tags_and_carry_errors() {
        let (svc, fakes) = Services::fake();
        fakes.api.reply("note_list", Ok(json!([1])));
        fakes.api.reply("note_save", Err("disk full".into()));
        let (mut cx, q) = Cx::headless_queue(svc);
        let mut calls = Calls::new();
        calls.run(&mut cx, vec![BackendCall::new("note_list", json!({}), 7), BackendCall::new("note_save", json!({}), 9)]);
        assert_eq!(calls.pending(), 2);
        let mut got = Vec::new();
        for (_, m) in q.drain() {
            if let crate::ui::view::Msg::Reply { ticket, result } = m {
                got.push(calls.take(ticket, result).unwrap());
            }
        }
        assert_eq!(got[0], (7, Reply::ok(json!([1]))));
        assert_eq!(got[1], (9, Reply::err("disk full")));
        assert_eq!(calls.pending(), 0);
        assert!(calls.take(Ticket(u64::MAX), Ok(Value::Null)).is_none());
    }

    #[test]
    fn clicks_hover_press_click() {
        let mut c: Clicks<&'static str> = Clicks::new();
        c.add("a", Rect::new(0.0, 0.0, 50.0, 20.0));
        c.add("b", Rect::new(60.0, 0.0, 50.0, 20.0));
        let (r, k) = c.event(&mv(10.0, 10.0));
        assert!(r.redraw && k.is_none() && r.cursor == Some(Cursor::Hand));
        assert!(c.step(0.06) && c.hover_amt(&"a") > 0.1);
        let (r, _) = c.event(&down(10.0, 10.0));
        assert!(r.consumed && r.capture);
        c.step(0.1);
        assert!(c.press_scale(&"a", 0.96) < 0.97);
        let (_, k) = c.event(&up(12.0, 10.0));
        assert_eq!(k, Some("a"));
        // press on a, release on b: no click
        c.event(&down(10.0, 10.0));
        let (_, k) = c.event(&up(70.0, 10.0));
        assert_eq!(k, None);
        // a click in empty space is not consumed
        let (r, _) = c.event(&down(200.0, 10.0));
        assert!(!r.consumed);
        for _ in 0..200 {
            c.step(1.0 / 60.0);
        }
        assert_eq!(c.press_scale(&"a", 0.96), 1.0);
    }

    #[test]
    fn clicks_reduced_motion_jumps() {
        let mut c: Clicks<u8> = Clicks::new();
        c.reduced = true;
        c.add(1, Rect::new(0.0, 0.0, 10.0, 10.0));
        c.event(&mv(5.0, 5.0));
        assert_eq!(c.hover_amt(&1), 1.0);
        c.event(&down(5.0, 5.0));
        assert!(c.press_scale(&1, 0.96) < 0.97);
    }

    #[test]
    fn cues_enter_ends_at_identity_and_exit_at_zero_alpha() {
        let mut cues = Cues::new();
        cues.play(AnimCue { target: "row".into(), kind: Anim::Enter { y: 6.0, ms: 180 } });
        let x0 = cues.xf("row");
        assert!(x0.alpha < 0.05 && x0.dy > 5.0);
        assert!(cues.step(90.0));
        let mid = cues.xf("row");
        assert!(mid.alpha > 0.3 && mid.dy < 6.0 && mid.dy > 0.0);
        assert!(!cues.step(200.0));
        assert_eq!(cues.xf("row"), Xf::ID);
        cues.play(AnimCue { target: "x".into(), kind: Anim::Exit { y: -4.0, scale: 0.96, ms: 160 } });
        cues.step(150.0);
        assert!(cues.xf("x").alpha < 0.3);
        assert_eq!(cues.xf("unknown"), Xf::ID);
    }

    #[test]
    fn reduced_motion_plays_nothing() {
        let mut cues = Cues::new();
        cues.reduced = true;
        cues.play(AnimCue { target: "a".into(), kind: Anim::Press });
        assert!(cues.is_empty());
    }

    #[test]
    fn ms_of_carries_the_fraction() {
        let mut carry = 0.0;
        let total: u32 = (0..240).map(|_| ms_of(1000.0 / 240.0, &mut carry)).sum();
        assert!((999..=1000).contains(&total), "{total}");
    }

    #[test]
    fn effects_split_into_island_and_view() {
        let (svc, fakes) = Services::fake();
        let mut cx = Cx::headless_with(svc);
        let _ = super::super::fx_b::take();
        let rest = route_effects(vec![Effect::CopyText("hi".into()), Effect::CollapsePill, Effect::GoPage("ai"), Effect::ScrollToBottom, Effect::Focus("x")], &mut cx);
        assert_eq!(rest, vec![Effect::ScrollToBottom, Effect::Focus("x")]);
        assert_eq!(cx.clipboard_text().as_deref(), Some("hi"));
        assert_eq!(super::super::fx_b::take(), vec![TabFx::CollapsePill, TabFx::GoPage("ai")]);
        let _ = fakes;
    }

    #[test]
    fn icon_registry_knows_the_web_set() {
        let mut ic = Icons::new();
        assert!(ic.has("bell") && ic.has("terminal") && !ic.has("nope"));
        ic.add("mine", "<path d=\"M1 1\"/>");
        assert!(ic.has("mine"));
    }
}
