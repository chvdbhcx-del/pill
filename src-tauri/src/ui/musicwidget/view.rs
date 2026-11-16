//! `MusicView`: the `View` of the standalone music widget window (an Overlay; see `mod.rs`). It reads the same feed the island's player reads
//! (`media:changed`, `media_state`), sends the same commands (`media_control`, `media_seek`) and draws with the island's own pieces (`island::gpu`
//! icons and covers, the liquid `Flow` / `Feel` springs). Behaviour lives in `logic.rs`; this file is the glue and the pixels.
//!
//! Motion (every value is a spring, every target is set in `step`/an event, never in `draw`):
//! * enter / leave: `shown` slides the body from the screen edge, `fade` is its opacity; when it has left, the view hides its window;
//! * hover: the cursor reaching the body (`ProximityEvent::Hover`) opens the capsule into the card (`expand`), leaving closes it. In the TASKBAR
//!   the folded capsule has its own buttons, so hovering opens nothing there: a click on its cover or title opens the card (`pinned`), a click
//!   on the card's cover / title (or the strip under it) or the cursor leaving folds it again;
//! * a new track: the old title and artist fade up and away while the new ones arrive (`swap`), the cover fades in (`Gpu::cover`);
//! * play / pause: the two icons cross-fade with a pop (`pp`), a pressed button dips (`press`), a hovered one lights (`hov`);
//! * the progress bar glides to each reading (`prog`).
//! Idle cost: nothing runs while the widget is hidden; while it shows, one 1 Hz tick (a `Ticker`) moves the clock and ends the grace of a pause,
//! and frames run only while a spring moves.
use super::logic::{self as lg, Btn, Hit, Presence, Stand, Track, BTNS};
use crate::ui::host::geom::{Align, Dock};
use crate::ui::island::gpu::Gpu;
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::a::core::{Tick, Ticker};
use crate::ui::view::{Cx, Msg, ProximityEvent, Ticket, View};
use glasskit::canvas::{scale_about, translation, Canvas};
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, MouseButton, Response};
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::live::lyrics::Clock;
use pill_ui_core::shell::color::Theme as ShellTheme;
use pill_ui_core::shell::settings::ShellSettings;
use serde_json::{json, Value};
use std::time::Duration;

#[path = "view_kbd.rs"]
mod kbd; // keyboard mode, UIA controls and high contrast of the widget

const TITLE: Style = Style::new(13.0, 600).ellipsis();
const SUB: Style = Style::new(11.5, 450).ellipsis();
/// The artist's line box is a little lower than the title's so that the two share a baseline (their centres differ with the size).
const SUB_DROP: f32 = 0.5;
const BIG: Style = Style::new(15.0, 650).ellipsis();
const MID: Style = Style::new(12.0, 450).ellipsis();
const SMALL: Style = Style::new(11.0, 450);
const TIMES: Style = Style::new(11.0, 500).mono();

/// The bus event `pill.rs` emits when the fullscreen guard starts / stops hiding the island: `{ "hidden": bool }`.
pub const GUARD_EVENT: &str = "pill:guard";

/// The bus event `mod.rs` emits when the place the window stands in changed (the bar it is in, which side it is anchored to, the monitor, DPI):
/// `{ "bar": logical px (0 = no bar), "align": "left" | "center" | "right" }`. In the taskbar the view slides out, is re-placed and slides in.
pub const SLOT_EVENT: &str = "music:slot";

pub struct MusicView {
    stand: Stand,
    /// the size factor as it glides to the island's `size` (`stand.k` follows it every frame), and the bar's real height (the stand has it in base px)
    kf: Flow,
    bar_real: f32,
    /// a new place in the bar waiting for the body to have left (the window is re-placed then, see `step`)
    reseat: Option<Stand>,
    settings: ShellSettings,
    os_light: bool,
    track: Track,
    art_id: u64,
    art_url: Option<String>,
    presence: Presence,
    clock: Clock,
    /// `media.widget` and `module.media`
    widget_on: bool,
    module_on: bool,
    /// the fullscreen guard hides the island, so it hides the widget too
    guard: bool,
    /// the window is on screen / its show was asked for and has not arrived yet
    vis: bool,
    asked: bool,
    /// what the presence rules say right now (the targets of `shown` and `fade`)
    want: bool,
    shown: Flow,
    fade: Flow,
    expand: Flow,
    /// the sideways shift (logical px, `Stand::dx`) of a window that was moved under the body because the island beside it changed its width
    glide: Flow,
    swap: Flow,
    pp: Flow,
    prog: Flow,
    hov: [Flow; 3],
    press: [Flow; 3],
    /// heading and sub line shown before the last track change (they fade away while `swap` runs)
    prev: (String, String),
    hot: Option<Btn>,
    down: Option<Btn>,
    /// the cursor is on the widget (proximity)
    near: bool,
    /// in the taskbar: the card was opened by a click and stays open while the cursor stays on the widget
    pinned: bool,
    /// a press that began on the header (cover / title) of the bar capsule
    down_head: bool,
    /// a seek drag in progress: the fraction under the pointer
    seek: Option<f32>,
    ticker: Option<Ticker>,
    state_ticket: Option<Ticket>,
    last_hit: Option<Rect>,
    gpu_s: Gpu,
    gpu_l: Gpu,
    /// keyboard mode (the focus ring) and the high-contrast palette (view_kbd.rs)
    kbd: kbd::Kbd,
}

fn flows<const N: usize>(x: f32, f: Feel) -> [Flow; N] {
    std::array::from_fn(|_| Flow::new(x, f))
}

impl MusicView {
    pub fn new(cx: &mut Cx, dock: Dock) -> MusicView {
        for e in ["media:changed", "settings:changed", GUARD_EVENT, SLOT_EVENT, crate::ui::a11y::target::EVENT, crate::ui::a11y::contrast::EVENT] {
            cx.subscribe(e);
        }
        let mut v = MusicView {
            stand: Stand { scale: cx.scale.max(0.25), ..dock.into() },
            kf: Flow::new(1.0, Feel::FLOW),
            bar_real: 0.0,
            reseat: None,
            settings: ShellSettings::default(),
            os_light: false,
            track: Track::default(),
            art_id: 0,
            art_url: None,
            presence: Presence::default(),
            clock: Clock::default(),
            widget_on: true,
            module_on: true,
            guard: crate::pill::widget_guard(),
            vis: false,
            asked: false,
            want: false,
            shown: Flow::new(0.0, Feel::FLOW),
            fade: Flow::new(0.0, Feel::FADE),
            expand: Flow::new(0.0, Feel::FLOW),
            glide: Flow::new(0.0, Feel::FLOW),
            swap: Flow::new(1.0, Feel::FADE.within(260.0)),
            pp: Flow::new(0.0, Feel::POP),
            prog: Flow::new(0.0, Feel::DRAG),
            hov: flows(0.0, Feel::FADE.within(160.0)),
            press: flows(0.0, Feel::SNAP),
            prev: (String::new(), String::new()),
            hot: None,
            down: None,
            near: false,
            pinned: false,
            down_head: false,
            seek: None,
            ticker: None,
            state_ticket: None,
            last_hit: None,
            gpu_s: Gpu::new(),
            gpu_l: Gpu::new(),
            kbd: kbd::Kbd::new(),
        };
        // settings first (theme, the module switch), then the first reading; both answer at once, the cover comes with the reply
        if let Ok(s) = cx.call("settings_get", json!({})) {
            v.take_settings(&s);
        }
        v.kf.jump(v.kf.target());
        v.apply_k();
        v.os_light = crate::ui::island::sys::os_prefers_light();
        v.state_ticket = Some(cx.call_async("media_state", json!({ "known": null })));
        v
    }

    /// The place in the bar the window was created for (`mod.rs` read it from the monitor); a no-op for the other docks.
    pub fn with_slot(mut self, bar: f32, align: Align) -> MusicView {
        self.stand.align = align; // the plain docks too: the body stands on the side the window is placed at
        if self.stand.dock == Dock::Taskbar {
            self.bar_real = bar;
        }
        self.apply_k();
        self
    }

    /// The size factor into the stand (and the bar's real height into base px), every frame it moves.
    fn apply_k(&mut self) {
        let k = self.kf.at().clamp(lg::KMIN * 0.9, lg::KMAX * 1.1);
        if (self.stand.k - k).abs() > 1e-6 || (self.stand.bar * k - self.bar_real).abs() > 1e-3 {
            self.stand.k = k;
            self.stand.bar = self.bar_real / k;
            self.last_hit = None;
        }
    }

    /// A stand for a bar `bar` px tall (real) and the side `align`, at the current size.
    fn stand_in_bar(&self, align: Align, bar: f32) -> Stand {
        Stand { dock: self.stand.dock, align, bar: bar / self.stand.k, scale: self.stand.scale, k: self.stand.k, dx: self.stand.dx }
    }

    /// The monitor's DPI scale (the window is a whole number of device px; the layout snaps to them, see `Stand`).
    fn rescale(&mut self, scale: f32) {
        let scale = scale.max(0.25);
        if (self.stand.scale - scale).abs() > 1e-4 {
            self.stand.scale = scale;
            if let Some(r) = self.reseat.as_mut() {
                r.scale = scale;
            }
            self.apply_k();
            self.last_hit = None;
        }
    }

    /// The card opens on hover at the screen edges; in the taskbar only a click opens it (see `pinned`).
    fn hover_opens(&self) -> bool {
        !self.stand.in_bar()
    }

    /// The settings window's preview runs a widget of its own in its sandbox and parks it once it has settled: the 1 Hz tick stops, the widget
    /// stays drawn as it is; a `Tick` message (the preview posts one when it wakes) starts it again.
    pub fn preview_sleep(&mut self) {
        self.ticker = None;
    }

    /// The window stands somewhere else now. Hidden: just take it (a show re-places it). On screen: slide out first, the new place is taken and the
    /// window re-placed when the body has left (`step`), and it slides in again, so the body never jumps across the screen.
    fn reslot(&mut self, to: Stand, cx: &mut Cx) {
        if self.stand.dock != Dock::Taskbar {
            return;
        }
        self.bar_real = to.bar * to.k;
        if self.vis {
            self.reseat = Some(to);
            self.sync(cx);
        } else {
            self.stand = to;
            self.last_hit = None;
        }
    }

    // ------------------------------------------------------------------------------------------------------------ state

    fn take_settings(&mut self, s: &Value) {
        if !s.is_object() {
            return;
        }
        self.settings.apply_json(s);
        self.contrast_refresh();
        self.refresh_lang();
        self.widget_on = s.get("media.widget").and_then(Value::as_bool).unwrap_or(false);
        self.module_on = s.get("module.media") != Some(&Value::Bool(false));
        self.presence.set_enabled(self.widget_on && self.module_on);
        if let Some(pct) = s.get("size").and_then(Value::as_f64) {
            let k = lg::size_k(pct);
            if (k - self.kf.target()).abs() > 1e-6 {
                self.kf.to(k);
                if !self.vis {
                    self.kf.jump(k); // nobody sees it: no animation, no frames
                }
            }
        }
    }

    fn theme(&self) -> Theme {
        if let Some(p) = &self.kbd.hc {
            return crate::ui::a11y::contrast::theme(p); // high contrast: the system colours, flat and opaque
        }
        let light = self.settings.resolved_theme(self.os_light) == ShellTheme::Light;
        let base = if light { Theme::pill_light() } else { Theme::pill_dark() };
        match self.settings.island_color() {
            Some(c) => crate::ui::island::tint::tinted(base, c),
            None => base,
        }
    }

    fn accent(&self, th: &Theme) -> Color {
        if self.kbd.hc.is_some() {
            return th.accent; // the cover's colour has no place in high contrast
        }
        self.track.accent.map_or(th.accent, |[r, g, b]| Color::rgb8(r, g, b))
    }

    /// What answers the cursor now (the hit region and the clicks use the same numbers as `draw`).
    fn body_now(&self) -> Rect {
        self.stand.rect_to_real(lg::hit_rect(self.stand, self.expand.at().clamp(0.0, 1.05), self.slide()))
    }

    fn slide(&self) -> f32 {
        (1.0 - self.shown.at()) * lg::slide_distance()
    }

    fn open(&self) -> f32 {
        self.expand.at().clamp(0.0, 1.05)
    }

    /// `pos` is in the real window; the layout is in base px.
    fn hit_at(&self, pos: (f32, f32)) -> Hit {
        lg::hit(self.stand, self.open(), self.slide(), self.stand.to_base(pos))
    }

    fn pos_now(&self, now: f64) -> u64 {
        self.clock.pos(now as i64)
    }

    /// A reading of the player (`media:changed` or the reply of `media_state`).
    fn apply(&mut self, v: &Value, cx: &mut Cx) {
        let t = Track::from_json(v);
        let now = cx.now_ms;
        if !t.has_session {
            // nothing plays: keep what was shown (it leaves with its animation), only the presence changes
            self.track.has_session = false;
            self.track.playing = false;
            self.presence.update(false, false, now);
            self.sync(cx);
            return;
        }
        match &t.thumbnail {
            Some(url) => {
                self.art_id = t.thumb_id;
                self.art_url = Some(url.clone());
            }
            None if t.thumb_id != self.art_id => {
                // the cover changed but the event left it out: ask once (the reply carries it)
                self.art_id = t.thumb_id;
                self.art_url = None;
                if self.state_ticket.is_none() {
                    self.state_ticket = Some(cx.call_async("media_state", json!({ "known": null })));
                }
            }
            None => {}
        }
        if self.track.has_session && !self.track.title.is_empty() && t.identity() != self.track.identity() {
            self.prev = (self.track.heading(), self.track.sub());
            self.swap.jump(0.0);
            self.swap.to(1.0);
        }
        if t.playing != self.track.playing || !self.track.has_session {
            self.pp.to(t.playing as u8 as f32);
            self.pp.nudge(if t.playing { 2.5 } else { -2.5 });
        }
        self.clock.set(t.position_ms, t.duration_ms, t.playing, now as i64);
        self.presence.update(true, t.playing, now);
        if self.seek.is_none() {
            self.prog.to(lg::fraction(t.position_ms, t.duration_ms));
        }
        self.track = Track { thumbnail: None, ..t };
        self.sync(cx);
    }

    /// Re-decide whether the widget is wanted, start / stop its window and its tick. Cheap; call after anything that may change the answer.
    fn sync(&mut self, cx: &mut Cx) {
        let want = self.widget_on && !self.guard && self.reseat.is_none() && self.presence.visible(cx.now_ms);
        crate::ui::a11y::target::set_widget_up(want);
        if want != self.want {
            self.want = want;
            self.shown.to(want as u8 as f32);
            self.fade.to(want as u8 as f32);
            cx.request_frame();
        }
        if want && !self.vis && !self.asked {
            self.asked = true;
            cx.show_self();
        }
        if want && self.ticker.is_none() {
            self.ticker = Some(Ticker::start(cx.waker(), Duration::from_millis(1000)));
        } else if !want {
            self.ticker = None;
        }
        self.retarget();
    }

    fn retarget(&mut self) {
        let open = if self.hover_opens() { self.near } else { self.pinned && self.near } || self.seek.is_some() || self.kbd.on;
        self.expand.to((self.want && open) as u8 as f32);
    }

    fn tick(&mut self, cx: &mut Cx) -> Response {
        self.sync(cx);
        if self.vis && self.track.playing && self.seek.is_none() {
            self.prog.to(lg::fraction(self.pos_now(cx.now_ms), self.track.duration_ms));
            return Response::redraw();
        }
        Response::NONE
    }

    fn command(&mut self, b: Btn, cx: &mut Cx) {
        let _ = cx.call_async("media_control", json!({ "action": b.action() }));
    }

    fn commit_seek(&mut self, f: f32, cx: &mut Cx) {
        let ms = (f as f64 * self.track.duration_ms as f64).round() as u64;
        let _ = cx.call_async("media_seek", json!({ "ms": ms }));
        self.clock.set(ms, self.track.duration_ms, self.track.playing, cx.now_ms as i64);
        self.prog.to(f);
    }

    fn set_hot(&mut self, h: Option<Btn>) -> bool {
        if self.hot == h {
            return false;
        }
        if let Some(o) = self.hot {
            self.hov[o.index()].to(0.0);
        }
        if let Some(n) = h {
            self.hov[n.index()].to(1.0);
        }
        self.hot = h;
        true
    }

    fn moving(&self) -> bool {
        self.shown.moving()
            || self.fade.moving()
            || self.expand.moving()
            || self.glide.moving()
            || self.kf.moving()
            || self.swap.moving()
            || self.pp.moving()
            || self.prog.moving()
            || self.hov.iter().chain(self.press.iter()).any(Flow::moving)
            || self.kbd.ring.moving()
    }
}

impl View for MusicView {
    fn accessible(&self) -> Option<String> {
        let (h, s) = (self.track.heading(), self.track.sub());
        Some(if s.is_empty() { format!("Music: {h}") } else { format!("Music: {h} - {s}") })
    }

    fn controls(&self) -> Vec<crate::ui::a11y::nav::Control> {
        self.kbd_controls()
    }

    fn control_act(&mut self, id: u32, act: crate::ui::a11y::nav::Act, cx: &mut Cx) -> bool {
        self.kbd_act(id, act, cx)
    }

    fn focused_control(&self) -> Option<u32> {
        self.kbd.on.then(|| self.kbd.focus).flatten()
    }

    fn size_hint(&self) -> Option<(f32, f32)> {
        Some(self.stand.real_win())
    }

    fn on_dpi(&mut self, scale: f32, cx: &mut Cx) {
        self.rescale(scale);
        cx.request_frame();
    }

    fn on_proximity(&mut self, ev: &ProximityEvent, cx: &mut Cx) -> Response {
        match ev {
            ProximityEvent::Hover => self.near = true,
            ProximityEvent::Leave => {
                self.near = false;
                self.pinned = false; // the card the click opened folds when the cursor goes away
                self.set_hot(None);
            }
            _ => return Response::NONE,
        }
        self.retarget();
        cx.request_frame();
        Response::redraw()
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        if let Some(r) = self.kbd_input(ev, cx) {
            return r; // keyboard mode took the key
        }
        match ev {
            InputEvent::MouseMove { pos, .. } => {
                if self.seek.is_some() {
                    let bar = lg::card_layout(lg::frames(self.stand, self.open(), self.slide()).1).bar;
                    let f = lg::seek_at(bar, self.stand.to_base(*pos).0);
                    self.seek = Some(f);
                    self.prog.to(f);
                    return Response { redraw: true, consumed: true, capture: true, cursor: Some(Cursor::Hand), ..Response::NONE };
                }
                self.rescale(cx.scale);
                let h = self.hit_at(*pos);
                let hot = if let Hit::Button(b) = h { Some(b) } else { None };
                let changed = self.set_hot(hot);
                let r = Response { redraw: changed, ..Response::NONE };
                if hot.is_some() || matches!(h, Hit::Seek(_) | Hit::Header) {
                    r.with_cursor(Cursor::Hand)
                } else {
                    r
                }
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => match self.hit_at(*pos) {
                Hit::Header => {
                    self.down_head = true;
                    Response { consumed: true, redraw: true, capture: true, ..Response::NONE }
                }
                Hit::Button(b) => {
                    self.down = Some(b);
                    self.press[b.index()].to(1.0);
                    Response { consumed: true, redraw: true, capture: true, ..Response::NONE }
                }
                Hit::Seek(f) if self.track.duration_ms > 0 => {
                    self.seek = Some(f);
                    self.prog.to(f);
                    self.retarget();
                    Response { consumed: true, redraw: true, capture: true, cursor: Some(Cursor::Hand), ..Response::NONE }
                }
                Hit::Body => Response::consumed(),
                _ => Response::NONE,
            },
            InputEvent::MouseUp { pos, button: MouseButton::Left, .. } => {
                let mut r = Response::NONE;
                if std::mem::take(&mut self.down_head) {
                    if self.hit_at(*pos) == Hit::Header {
                        self.pinned = !self.pinned;
                        self.near |= self.pinned; // a click is the cursor being here, whether or not the proximity event came first
                        self.retarget();
                        cx.request_frame();
                    }
                    r = Response { consumed: true, redraw: true, ..Response::NONE };
                }
                if let Some(b) = self.down.take() {
                    self.press[b.index()].to(0.0);
                    if self.hit_at(*pos) == Hit::Button(b) {
                        self.command(b, cx);
                    }
                    r = Response { consumed: true, redraw: true, ..Response::NONE };
                }
                if let Some(f) = self.seek.take() {
                    self.commit_seek(f, cx);
                    self.retarget();
                    r = Response { consumed: true, redraw: true, ..Response::NONE };
                }
                r
            }
            InputEvent::MouseLeave => {
                let c = self.set_hot(None);
                Response { redraw: c, ..Response::NONE }
            }
            _ => Response::NONE,
        }
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Event { name, payload, .. } => match name.as_str() {
                "media:changed" => {
                    if self.module_on {
                        self.apply(&payload, cx);
                    }
                }
                "settings:changed" => {
                    let was = self.module_on;
                    self.take_settings(&payload);
                    self.os_light = crate::ui::island::sys::os_prefers_light();
                    if !self.module_on {
                        self.presence.update(false, false, cx.now_ms); // the privacy switch: gone at once, nothing kept on show
                        self.track = Track::default();
                    } else if !was && self.state_ticket.is_none() {
                        self.state_ticket = Some(cx.call_async("media_state", json!({ "known": null })));
                    }
                    self.sync(cx);
                }
                crate::ui::a11y::target::EVENT => self.kbd_event(&name, &payload, cx),
                crate::ui::a11y::contrast::EVENT => self.contrast_refresh(),
                GUARD_EVENT => {
                    self.guard = payload.get("hidden").and_then(Value::as_bool).unwrap_or(false);
                    self.sync(cx);
                }
                SLOT_EVENT => {
                    let bar = payload.get("bar").and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as f32;
                    let align = Align::from_setting(payload.get("align").and_then(Value::as_str).unwrap_or("left"));
                    let to = self.stand_in_bar(align, bar);
                    match payload.get("dx").and_then(Value::as_f64) {
                        // only the place along the bar moved (the island's width changed): the window is already there, the body glides across
                        Some(dx) if self.vis && to.align == self.stand.align && self.reseat.is_none() => {
                            self.glide.jump(dx as f32);
                            self.glide.to(0.0);
                            self.stand.dx = self.glide.at();
                            self.last_hit = None;
                            cx.request_frame();
                        }
                        _ => self.reslot(to, cx),
                    }
                }
                _ => return Response::NONE,
            },
            Msg::Reply { ticket, result } => {
                if self.state_ticket == Some(ticket) {
                    self.state_ticket = None;
                    if let (Ok(v), true) = (result, self.module_on) {
                        self.apply(&v, cx);
                    }
                } else {
                    return Response::NONE; // the answer of a transport command: the state event tells what happened
                }
            }
            Msg::Custom(b) => match b.downcast::<Tick>() {
                Ok(_) => return self.tick(cx),
                Err(_) => return Response::NONE,
            },
            Msg::Open(_) => return Response::NONE,
        }
        if self.vis { Response::redraw() } else { Response::NONE } // a hidden window draws nothing: no frame for it
    }

    fn step(&mut self, dt: f64, cx: &mut Cx) -> bool {
        self.rescale(cx.scale);
        self.kf.step(dt);
        self.apply_k();
        self.shown.step(dt);
        self.fade.step(dt);
        self.expand.step(dt);
        self.glide.step(dt);
        self.stand.dx = self.glide.at();
        self.swap.step(dt);
        self.pp.step(dt);
        self.prog.step(dt);
        for f in self.hov.iter_mut().chain(self.press.iter_mut()) {
            f.step(dt);
        }
        let kbd_busy = self.kbd_step(dt, cx);
        let body = self.body_now();
        if self.last_hit != Some(body) {
            self.last_hit = Some(body);
            cx.set_hit_region(vec![body]);
        }
        let moving = self.moving() || kbd_busy;
        if !self.want && self.vis && !moving {
            if let Some(to) = self.reseat.take() {
                // it has left to be re-placed: take the new place, put the window there and come back in
                self.stand = to;
                self.last_hit = None;
                cx.relayout_self();
                self.sync(cx);
                if self.want {
                    cx.request_frame();
                    return true;
                }
            }
            cx.hide_self(); // it has left: the window goes away and costs nothing
        }
        moving
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.vis = true;
        self.asked = false;
        self.os_light = crate::ui::island::sys::os_prefers_light();
        self.shown.jump(0.0);
        self.fade.jump(0.0);
        self.expand.jump(0.0);
        self.last_hit = None;
        // a jump lands with no target left: head for the presence again, so the enter animation runs from the edge
        self.shown.to(self.want as u8 as f32);
        self.fade.to(self.want as u8 as f32);
        self.sync(cx);
        self.prog.jump(lg::fraction(self.pos_now(cx.now_ms), self.track.duration_ms));
        cx.request_frame();
    }

    fn on_hide(&mut self, _cx: &mut Cx) {
        self.vis = false;
        self.asked = false;
        self.near = false;
        self.pinned = false;
        self.down_head = false;
        self.seek = None;
        self.down = None;
        self.hot = None;
        self.expand.jump(0.0);
        if let Some(to) = self.reseat.take() {
            self.stand = to; // a re-placement that the guard or a pause turned into a leave: the show places it
        }
        self.ticker = None; // nothing runs while it is away; the next event calls it up again
    }

    fn draw(&mut self, cv: &Canvas, _theme: &Theme, _rect: Rect, cx: &Cx) {
        self.rescale(cx.scale);
        let th = self.theme();
        let alpha = self.fade.at01();
        if alpha <= 0.002 {
            return;
        }
        let p = self.open();
        let dy = self.slide();
        let body = lg::body(self.stand, p, dy);
        let rad = lg::body_radius(p, body.h);
        let accent = self.accent(&th);
        let pos = if let Some(f) = self.seek { (f as f64 * self.track.duration_ms as f64).round() as u64 } else { self.pos_now(cx.now_ms) };
        // the layout is in base px (100 %): draw it scaled by the size factor, from where the base window stands in the real one
        let (st, (ox, oy)) = (self.stand, self.stand.origin());
        cv.with_transform(translation(ox, oy), |cv| cv.with_transform(scale_about((0.0, 0.0), st.k, st.k), |cv| cv.with_opacity(alpha, |cv| {
            // in the bar the folded capsule has only a few px of strip below it (the window ends at the screen's edge): a tighter shadow that
            // does not reach the floor, so it is never cut off, growing to the island's own while the card rises. Elsewhere the shadow stays
            // inside the window's PAD on every side, at the top edge too (it falls the other way there)
            let (blur, sdy) = if self.stand.in_bar() { (4.0 + 4.0 * p.clamp(0.0, 1.0), -1.0) } else { (lg::SHADOW_BLUR, if self.stand.dock.at_bottom() { -lg::SHADOW_DY } else { lg::SHADOW_DY }) };
            let hc = self.kbd.hc.is_some();
            if !hc {
                cv.shadow_rr(body, rad, blur, sdy, Color::rgba(0.0, 0.0, 0.0, 0.45));
            }
            cv.fill_rr(body, rad, th.bg.alpha(if hc { 1.0 } else { 0.97 }));
            cv.stroke_rr(body, rad, if hc { crate::ui::a11y::contrast::OUTLINE_W } else { 1.0 }, th.border);
            let (cap, card) = lg::frames(self.stand, p, dy);
            let cap_a = (1.0 - p * 1.7).clamp(0.0, 1.0);
            let card_a = ((p - 0.3) / 0.7).clamp(0.0, 1.0);
            let ink = Ink { st: self.stand, th: &th, accent, track: &self.track, swap: self.swap.at01(), prev: &self.prev, pp: self.pp.at(), hov: &self.hov, press: &self.press, frac: self.prog.at().clamp(0.0, 1.0), pos, now: cx.now_ms };
            cv.with_clip_rr(body, rad, |cv| {
                if cap_a > 0.01 {
                    cv.with_opacity(cap_a, |cv| ink.capsule(cv, cap, &mut self.gpu_s, self.art_id, self.art_url.as_deref()));
                }
                if card_a > 0.01 {
                    cv.with_opacity(card_a, |cv| ink.card(cv, card, &mut self.gpu_l, self.art_id, self.art_url.as_deref(), self.seek.is_some() || self.near));
                }
            });
        })));
        self.kbd_paint(cv, &th, alpha);
    }
}

/// Everything `draw` needs, copied out of the view so the two cover caches can be borrowed mutably next to it.
struct Ink<'a> {
    st: Stand,
    th: &'a Theme,
    accent: Color,
    track: &'a Track,
    swap: f32,
    prev: &'a (String, String),
    pp: f32,
    hov: &'a [Flow; 3],
    press: &'a [Flow; 3],
    frac: f32,
    pos: u64,
    now: f64,
}

impl Ink<'_> {
    /// Two lines of text that cross-fade when the track changes: the old pair rises and fades, the new one rises into place.
    fn lines(&self, cv: &Canvas, head: Rect, sub: Rect, hs: Style, ss: Style) {
        let th = self.th;
        let (h, s) = (self.track.heading(), self.track.sub());
        let k = self.swap;
        if k < 0.999 {
            cv.with_opacity(1.0 - k, |cv| {
                cv.text(head.offset(0.0, -4.0 * k), &self.prev.0, hs, th.text);
                cv.text(sub.offset(0.0, -4.0 * k), &self.prev.1, ss, th.dim);
            });
        }
        cv.with_opacity(k.max(0.0), |cv| {
            cv.text(head.offset(0.0, 4.0 * (1.0 - k)), &h, hs, th.text);
            cv.text(sub.offset(0.0, 4.0 * (1.0 - k)), &s, ss, th.dim);
        });
    }

    fn button(&self, cv: &Canvas, gpu: &mut Gpu, b: Btn, r: Rect) {
        let th = self.th;
        let (h, q) = (self.hov[b.index()].at01(), self.press[b.index()].at().clamp(0.0, 1.0));
        let c = r.center();
        let rad = r.w / 2.0 * (1.0 - 0.12 * q);
        if b == Btn::Play {
            cv.fill_circle(c, rad, self.accent.alpha(0.20 + 0.16 * h));
        } else if h > 0.01 {
            cv.fill_circle(c, rad, th.hover.fade(h));
        }
        let ir = r.inset(r.w * 0.24 + 0.5 * q * 3.0);
        match b {
            Btn::Prev => gpu.icon(cv, "prev", ir, th.text, 1.8),
            Btn::Next => gpu.icon(cv, "next", ir, th.text, 1.8),
            Btn::Play => {
                // the play and pause icons cross-fade with the spring's pop
                let k = self.pp.clamp(0.0, 1.0);
                let pop = 1.0 + 0.10 * (self.pp - 1.0).abs().min(1.0) * (self.pp > 1.0) as u8 as f32;
                cv.with_scale_about(c, pop, |cv| {
                    cv.with_opacity(1.0 - k, |cv| gpu.icon(cv, "play", ir, th.text, 1.8));
                    cv.with_opacity(k, |cv| gpu.icon(cv, "pause", ir, th.text, 1.8));
                });
            }
        }
    }

    fn bar(&self, cv: &Canvas, r: Rect, knob: f32) {
        let th = self.th;
        cv.fill_rr(r, r.h / 2.0, th.track);
        let w = (r.w * self.frac).clamp(0.0, r.w);
        if w > 0.5 {
            cv.fill_rr(Rect::new(r.x, r.y, w.max(r.h), r.h), r.h / 2.0, self.accent);
        }
        if knob > 0.01 {
            // the knob stays inside the track's ends, so the track keeps the card's padding (cover, times and track share their left and right edges)
            let kr = (r.h * 0.9 + 1.0) * knob;
            cv.fill_circle(((r.x + w).clamp(r.x + kr, (r.right() - kr).max(r.x + kr)), r.cy()), kr, th.text);
        }
    }

    /// The album cover in the base rectangle `r`, drawn in the REAL window's coordinates (the picture is decoded at its real size, so it is sharp at
    /// any size factor): `rad` is the corner radius in base px.
    fn cover(&self, cv: &Canvas, gpu: &mut Gpu, art: u64, url: Option<&str>, r: Rect, rad: f32) {
        let (k, (ox, oy)) = (self.st.k, self.st.origin());
        let real = self.st.rect_to_real(r);
        cv.with_transform(scale_about((0.0, 0.0), 1.0 / k, 1.0 / k), |cv| cv.with_transform(translation(-ox, -oy), |cv| gpu.cover_r(cv, self.th, art, url, real, self.now, rad * k)));
    }

    /// The track change cross-fade of the bar capsule: ONE line, the heading and then the sub line in the dimmer ink after it (`r` is centred
    /// vertically by the style). The artist gives way first (`lg::bar_line`): it is cut to what a whole title leaves, and dropped before the title
    /// is cut. The two sizes share one baseline (`SUB_DROP`).
    fn line(&self, cv: &Canvas, r: Rect, hs: Style, ss: Style) {
        let th = self.th;
        let k = self.swap;
        let pair = |cv: &Canvas, h: &str, s: &str, dy: f32| {
            let (hw, sub) = lg::bar_line(r.w, cv.text_width(h, hs), if s.is_empty() { 0.0 } else { cv.text_width(s, ss) });
            // a hair of slack: a layout box exactly as wide as its text may still decide to cut it
            cv.text(Rect::new(r.x, r.y + dy, (hw.ceil() + 1.0).min(r.w), r.h), h, hs.ellipsis(), th.text);
            if let Some((sx, sw)) = sub {
                cv.text(Rect::new(r.x + sx, r.y + dy + SUB_DROP, (sw.ceil() + 1.0).min(r.w - sx), r.h), s, ss.ellipsis(), th.dim);
            }
        };
        if k < 0.999 {
            cv.with_opacity(1.0 - k, |cv| pair(cv, &self.prev.0, &self.prev.1, -3.0 * k));
        }
        cv.with_opacity(k.max(0.0), |cv| pair(cv, &self.track.heading(), &self.track.sub(), 3.0 * (1.0 - k)));
    }

    fn capsule(&self, cv: &Canvas, c: Rect, gpu: &mut Gpu, art: u64, url: Option<&str>) {
        let l = lg::capsule_layout_in(self.st, c);
        if self.st.in_bar() {
            self.cover(cv, gpu, art, url, l.cover, lg::bar_cover_radius(c.h));
        } else {
            self.cover(cv, gpu, art, url, l.cover, 7.0);
        }
        if self.st.in_bar() {
            self.line(cv, l.text, TITLE, SUB);
        } else {
            self.lines(cv, Rect::new(l.text.x, l.text.y + 3.0, l.text.w, 18.0), Rect::new(l.text.x, l.text.y + 21.0, l.text.w, 15.0), TITLE, SUB);
        }
        for b in BTNS {
            self.button(cv, gpu, b, l.btn[b.index()]);
        }
        if l.bar.h > 0.0 {
            self.bar(cv, l.bar, 0.0);
        }
    }

    fn card(&self, cv: &Canvas, c: Rect, gpu: &mut Gpu, art: u64, url: Option<&str>, grab: bool) {
        let th = self.th;
        let l = lg::card_layout(c);
        self.cover(cv, gpu, art, url, l.cover, lg::card_cover_radius());
        self.lines(cv, l.title, l.sub, BIG, MID);
        let album = self.track.album.trim();
        if !album.is_empty() {
            cv.text(l.album, album, SMALL.ellipsis(), th.faint);
        }
        self.bar(cv, l.bar, grab as u8 as f32 * 0.9);
        cv.text(l.elapsed, &lg::clock_text(self.pos), TIMES, th.dim);
        if self.track.duration_ms > 0 {
            cv.text(l.total, &lg::clock_text(self.track.duration_ms), TIMES.right(), th.dim);
        }
        for b in BTNS {
            self.button(cv, gpu, b, l.btn[b.index()]);
        }
    }
}

