//! `MusicView`: the `View` of the standalone music widget window (an Overlay; see `mod.rs`). It reads the same feed the island's player reads
//! (`media:changed`, `media_state`), sends the same commands (`media_control`, `media_seek`) and draws with the island's own pieces (`island::gpu`
//! icons and covers, the liquid `Flow` / `Feel` springs). Behaviour lives in `logic.rs`; this file is the glue and the pixels.
//!
//! Motion (every value is a spring, every target is set in `step`/an event, never in `draw`):
//! * enter / leave: `shown` slides the body from the screen edge (with a slight scale), `fade` is its opacity; it arrives with a soft landing and
//!   leaves without a bounce; when it has left, the view hides its window. A window that is replaced (new placement settings) or moved (another
//!   monitor, DPI, the bar) leaves the same way first and the new one comes in (`RETIRE_EVENT`, `SLOT_EVENT`);
//! * hover: the cursor reaching the body (`ProximityEvent::Hover`) opens the capsule into the card (`expand`), leaving closes it. The parts the
//!   two share (cover, title and artist, the buttons, the progress bar) MORPH between their places (`logic::parts`): the cover grows, the buttons
//!   travel, the corners round off; only the album and the times fade in. A click while it moves turns it around from where it is (springs keep
//!   their velocity). In the TASKBAR the folded capsule has its own buttons, so hovering opens nothing there: a click on its cover or title opens
//!   the card (`pinned`), a click on the card's cover / title (or the strip under it) or the cursor leaving folds it again;
//! * a new track: the old title and artist (a `Ghost` each, so several in a row never flash) fade up and away while the new ones arrive one after
//!   the other (`swap`), the cover cross-fades (`Gpu::cover_sized`), the accent colour eases to the new cover's (`acc`);
//! * play / pause: the two icons cross-fade, turning small and back, with a pop (`pp`), a pressed button dips (`press`), a hovered one lights
//!   (`hov`), prev / next nudge their way (`kick`);
//! * the progress bar glides to each reading (`prog`; a step under one device px just lands), follows the pointer closely while scrubbing, and its
//!   knob grows when it is hovered and again when it is dragged (`knob`).
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

/// The bus event `mod.rs` emits when a window of this widget is being replaced (new placement settings) or closed (the widget was turned off):
/// `{ "before": n }`. Every window made before generation `n` (`MusicView::with_gen`) slides out and closes itself.
pub const RETIRE_EVENT: &str = "music:retire";

/// How the enter and the leave move: it lands softly (a little overshoot) and it goes without a bounce, in about the time the fade needs.
const LEAVE: Feel = Feel::FADE.within(240.0);
/// The progress bar under a finger: close behind the pointer, no lag, no wobble.
const SCRUB: Feel = Feel { settle_ms: 90.0, damping: 0.9 };
/// A change of the island's `size`: the whole layout glides, never overshoots (an elastic size would make the text swim).
const SIZE: Feel = Feel::DRAG.within(340.0);
/// The accent colour follows the cover.
const ACCENT: Feel = Feel::FADE.within(320.0);
/// The smallest slide of the bar (device px) that is worth animating: below it the fill just lands (the pixel grid cannot show less).
const PROG_GLIDE_PX: f32 = 1.0;
/// The scale the body enters / leaves with (at 1 it stands at rest).
const ENTER_SCALE: f32 = 0.94;

/// The title and the artist of a track that was replaced: they fade up and away on their own spring, whatever the next track does.
struct Ghost {
    head: String,
    sub: String,
    a: Flow,
}

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
    /// the cover before the last change: it stays behind the new one while that fades in
    art_prev: Option<u64>,
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
    /// 0 -> 1 when the text of a new track arrives (the artist a beat after the title)
    swap: Flow,
    /// the text of the tracks that were replaced, fading away (at most a few: a run of skips)
    ghosts: Vec<Ghost>,
    pp: Flow,
    prog: Flow,
    /// the progress bar's knob: small while the card is open, bigger under the pointer, biggest while it is dragged
    knob: Flow,
    /// the pointer is over the progress bar
    bar_hot: bool,
    hov: [Flow; 3],
    press: [Flow; 3],
    /// the little push prev / next give their icon in their direction
    kick: [Flow; 2],
    /// the accent colour (r, g, b in 0..1), easing to the cover's
    acc: [Flow; 3],
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
    gpu: Gpu,
    /// which window this is (`with_gen`); a `RETIRE_EVENT` for a later one makes it leave and close. `u64::MAX` = never (the settings preview)
    gen: u64,
    retired: bool,
    /// keyboard mode (the focus ring) and the high-contrast palette (view_kbd.rs)
    kbd: kbd::Kbd,
}

fn flows<const N: usize>(x: f32, f: Feel) -> [Flow; N] {
    std::array::from_fn(|_| Flow::new(x, f))
}

/// How hard prev / next push their icon (px per second of the little spring: about 3 px at its peak).
const KICK_V: f32 = 80.0;

impl MusicView {
    pub fn new(cx: &mut Cx, dock: Dock) -> MusicView {
        for e in ["media:changed", "settings:changed", GUARD_EVENT, SLOT_EVENT, RETIRE_EVENT, crate::ui::a11y::target::EVENT, crate::ui::a11y::contrast::EVENT] {
            cx.subscribe(e);
        }
        let mut v = MusicView {
            stand: Stand { scale: cx.scale.max(0.25), ..dock.into() },
            kf: Flow::new(1.0, SIZE),
            bar_real: 0.0,
            reseat: None,
            settings: ShellSettings::default(),
            os_light: false,
            track: Track::default(),
            art_id: 0,
            art_url: None,
            art_prev: None,
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
            swap: Flow::new(1.0, Feel::FADE.within(300.0)),
            ghosts: Vec::new(),
            pp: Flow::new(0.0, Feel::POP),
            prog: Flow::new(0.0, Feel::DRAG),
            knob: Flow::new(0.0, Feel::FADE.within(140.0)),
            bar_hot: false,
            hov: flows(0.0, Feel::FADE.within(160.0)),
            press: flows(0.0, Feel::SNAP),
            kick: std::array::from_fn(|_| Flow::px(0.0, Feel::POP)),
            acc: flows(0.0, ACCENT),
            hot: None,
            down: None,
            near: false,
            pinned: false,
            down_head: false,
            seek: None,
            ticker: None,
            state_ticket: None,
            last_hit: None,
            gpu: Gpu::new(),
            gen: u64::MAX,
            retired: false,
            kbd: kbd::Kbd::new(),
        };
        // settings first (theme, the module switch), then the first reading; both answer at once, the cover comes with the reply
        if let Ok(s) = cx.call("settings_get", json!({})) {
            v.take_settings(&s);
        }
        v.kf.jump(v.kf.target());
        v.apply_k();
        v.follow_accent(true);
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

    /// Which window this is: a `RETIRE_EVENT` for a later generation makes it slide out and close (`mod.rs::sync`).
    pub fn with_gen(mut self, gen: u64) -> MusicView {
        self.gen = gen;
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
        if self.retired {
            return; // it is leaving for good
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
        self.follow_accent(!self.vis);
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

    /// The colour the accent is heading for: the cover's, else the theme's.
    fn accent_goal(&self, th: &Theme) -> Color {
        if self.kbd.hc.is_some() {
            return th.accent; // the cover's colour has no place in high contrast
        }
        self.track.accent.map_or(th.accent, |[r, g, b]| Color::rgb8(r, g, b))
    }

    /// Points the accent springs at the goal; `at_once` lands them there (nobody is looking).
    fn follow_accent(&mut self, at_once: bool) {
        let g = self.accent_goal(&self.theme());
        for (f, v) in self.acc.iter_mut().zip([g.r, g.g, g.b]) {
            if (f.target() - v).abs() > 1e-4 || at_once {
                f.to(v);
                if at_once {
                    f.jump(v);
                }
            }
        }
    }

    /// The accent as it is on screen now.
    fn accent(&self) -> Color {
        Color::rgba(self.acc[0].at().clamp(0.0, 1.0), self.acc[1].at().clamp(0.0, 1.0), self.acc[2].at().clamp(0.0, 1.0), 1.0)
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
        if t.thumb_id != self.art_id && self.art_id != 0 {
            self.art_prev = Some(self.art_id); // the old cover stays behind the new one while it fades in
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
        // another track while the widget is on screen: what it shows now fades away on a spring of its own (so a run of skips never flashes an
        // old line back to full), the new text arrives, the bar starts over
        let changed = self.vis && !self.track.heading().is_empty() && t.identity() != self.track.identity();
        if changed {
            let mut a = Flow::new(self.swap.at01(), Feel::FADE.within(170.0));
            a.to(0.0);
            self.ghosts.push(Ghost { head: self.track.heading(), sub: self.track.sub(), a });
            if self.ghosts.len() > 3 {
                self.ghosts.remove(0);
            }
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
            let f = lg::fraction(t.position_ms, t.duration_ms);
            if changed {
                self.prog.jump(f);
            } else {
                self.glide_prog(f);
            }
        }
        self.track = Track { thumbnail: None, ..t };
        self.follow_accent(!self.vis);
        self.sync(cx);
    }

    /// The progress bar heads for `f`: a step the pixel grid cannot show just lands, a bigger one glides for about as long as it is long (a
    /// short track or a seek is a visible step; an ordinary second of a long one is not).
    fn glide_prog(&mut self, f: f32) {
        let bar_px = (lg::BODY_W - 2.0 * lg::CARD_PAD) * self.stand.bs();
        let d = (f - self.prog.at()).abs() * bar_px;
        if d < PROG_GLIDE_PX {
            self.prog.jump(f);
        } else {
            self.prog.set_feel(Feel::DRAG.within((160.0 + 40.0 * d).min(400.0)));
            self.prog.to(f);
        }
    }

    /// Re-decide whether the widget is wanted, start / stop its window and its tick. Cheap; call after anything that may change the answer.
    fn sync(&mut self, cx: &mut Cx) {
        let want = self.widget_on && !self.guard && !self.retired && self.reseat.is_none() && self.presence.visible(cx.now_ms);
        if !self.retired {
            crate::ui::a11y::target::set_widget_up(want);
        }
        if want != self.want {
            self.want = want;
            self.shown.set_feel(if want { Feel::FLOW } else { LEAVE });
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
        self.aim_knob();
    }

    fn aim_knob(&mut self) {
        let k = if self.seek.is_some() {
            1.0
        } else if self.bar_hot {
            0.75
        } else if self.near || self.kbd.on {
            0.35
        } else {
            0.0
        };
        self.knob.to(k);
    }

    fn tick(&mut self, cx: &mut Cx) -> Response {
        self.sync(cx);
        if self.vis && self.track.playing && self.seek.is_none() {
            self.glide_prog(lg::fraction(self.pos_now(cx.now_ms), self.track.duration_ms));
            return Response::redraw();
        }
        Response::NONE
    }

    fn command(&mut self, b: Btn, cx: &mut Cx) {
        let _ = cx.call_async("media_control", json!({ "action": b.action() }));
        match b {
            Btn::Prev => self.kick[0].nudge(-KICK_V),
            Btn::Next => self.kick[1].nudge(KICK_V),
            Btn::Play => {}
        }
    }

    fn commit_seek(&mut self, f: f32, cx: &mut Cx) {
        let ms = (f as f64 * self.track.duration_ms as f64).round() as u64;
        let _ = cx.call_async("media_seek", json!({ "ms": ms }));
        self.clock.set(ms, self.track.duration_ms, self.track.playing, cx.now_ms as i64);
        self.prog.set_feel(Feel::DRAG);
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
            || self.knob.moving()
            || self.ghosts.iter().any(|g| g.a.moving())
            || self.hov.iter().chain(self.press.iter()).chain(self.kick.iter()).chain(self.acc.iter()).any(Flow::moving)
            // the new cover fades in on draws only a frame can make (it is not drawn while the window is away or still invisible)
            || (self.vis && self.fade.at01() > 0.002 && self.gpu.cover_fading(self.art_id))
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
                self.bar_hot = false;
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
                    let bar = lg::parts(self.stand, self.open(), self.slide()).bar;
                    let f = lg::seek_at(bar, self.stand.to_base(*pos).0);
                    self.seek = Some(f);
                    self.prog.to(f);
                    return Response { redraw: true, consumed: true, capture: true, cursor: Some(Cursor::Hand), ..Response::NONE };
                }
                self.rescale(cx.scale);
                let h = self.hit_at(*pos);
                let hot = if let Hit::Button(b) = h { Some(b) } else { None };
                let changed = self.set_hot(hot);
                // the knob grows under the pointer
                let on_bar = matches!(h, Hit::Seek(_)) && self.track.duration_ms > 0;
                let bar_changed = std::mem::replace(&mut self.bar_hot, on_bar) != on_bar;
                if bar_changed {
                    self.aim_knob();
                }
                let r = Response { redraw: changed || bar_changed, ..Response::NONE };
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
                    self.prog.set_feel(SCRUB);
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
                let b = std::mem::take(&mut self.bar_hot);
                if b {
                    self.aim_knob();
                }
                Response { redraw: c || b, ..Response::NONE }
            }
            _ => Response::NONE,
        }
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Event { name, payload, .. } => match name.as_str() {
                "media:changed" => {
                    if self.module_on && !self.retired {
                        self.apply(&payload, cx);
                    }
                }
                RETIRE_EVENT => {
                    // a newer window takes over (new placement settings) or the widget was turned off: this one slides out and closes
                    if self.gen < payload.get("before").and_then(Value::as_u64).unwrap_or(0) && !self.retired {
                        self.retired = true;
                        if !self.vis && !self.asked {
                            cx.close_self(); // nothing to slide out
                        }
                        self.sync(cx);
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
                SLOT_EVENT if self.retired => {}
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
        self.knob.step(dt);
        for f in self.hov.iter_mut().chain(self.press.iter_mut()).chain(self.kick.iter_mut()).chain(self.acc.iter_mut()) {
            f.step(dt);
        }
        for g in self.ghosts.iter_mut() {
            g.a.step(dt);
        }
        self.ghosts.retain(|g| g.a.moving());
        if self.vis {
            self.follow_accent(false); // a theme or contrast change eases too
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
            if self.retired {
                cx.close_self(); // it has left for good: a newer window stands in its place
            } else {
                cx.hide_self(); // it has left: the window goes away and costs nothing
            }
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
        // what changed while it was away is simply there: no old text fading out, no cover to cross-fade from, the accent already the cover's
        self.swap.jump(1.0);
        self.ghosts.clear();
        self.art_prev = None;
        self.follow_accent(true);
        // a jump lands with no target left: head for the presence again, so the enter animation runs from the edge
        self.shown.set_feel(Feel::FLOW);
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
        self.bar_hot = false;
        self.knob.jump(0.0);
        self.prog.set_feel(Feel::DRAG);
        self.ghosts.clear();
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
        let pos = if let Some(f) = self.seek { (f as f64 * self.track.duration_ms as f64).round() as u64 } else { self.pos_now(cx.now_ms) };
        // the layout is in base px (100 %): draw it scaled by the size factor, from where the base window stands in the real one
        let (st, (ox, oy)) = (self.stand, self.stand.origin());
        // it enters and leaves a little smaller, standing on the edge it docks to
        let anchor = (body.cx(), if st.dock.at_bottom() || st.in_bar() { body.bottom() } else { body.y });
        let grow = lg::lerp(ENTER_SCALE, 1.0, self.shown.at01());
        let (cap, card) = lg::frames(st, p, dy);
        let ink = Ink {
            st,
            th: &th,
            accent: self.accent(),
            track: &self.track,
            swap: self.swap.at01(),
            ghosts: &self.ghosts,
            pp: self.pp.at(),
            hov: &self.hov,
            press: &self.press,
            kick: [self.kick[0].at(), self.kick[1].at()],
            frac: self.prog.at().clamp(0.0, 1.0),
            knob: self.knob.at().clamp(0.0, 1.1),
            pos,
            now: cx.now_ms,
            p,
            pt: lg::parts(st, p, dy),
            cap,
            card,
            art: self.art_id,
            under: self.art_prev,
            url: self.art_url.as_deref(),
        };
        let gpu = &mut self.gpu;
        let hc = self.kbd.hc.is_some();
        cv.with_transform(translation(ox, oy), |cv| cv.with_transform(scale_about((0.0, 0.0), st.k, st.k), |cv| cv.with_opacity(alpha, |cv| cv.with_scale_about(anchor, grow, |cv| {
            // in the bar the folded capsule has only a few px of strip below it (the window ends at the screen's edge): a tighter shadow that
            // does not reach the floor, so it is never cut off, growing to the island's own while the card rises. Elsewhere the shadow stays
            // inside the window's PAD on every side, at the top edge too (it falls the other way there)
            let (blur, sdy) = if st.in_bar() { (4.0 + 4.0 * p.clamp(0.0, 1.0), -1.0) } else { (lg::SHADOW_BLUR, if st.dock.at_bottom() { -lg::SHADOW_DY } else { lg::SHADOW_DY }) };
            if !hc {
                cv.shadow_rr(body, rad, blur, sdy, Color::rgba(0.0, 0.0, 0.0, 0.45));
            }
            cv.fill_rr(body, rad, th.bg.alpha(if hc { 1.0 } else { 0.97 }));
            cv.stroke_rr(body, rad, if hc { crate::ui::a11y::contrast::OUTLINE_W } else { 1.0 }, th.border);
            cv.with_clip_rr(body, rad, |cv| ink.paint(cv, gpu));
        }))));
        self.kbd_paint(cv, &th, alpha);
    }
}

/// A text style between two (the title grows into the card's bigger one as the capsule opens).
fn mix_style(a: Style, b: Style, t: f32) -> Style {
    Style { size: lg::lerp(a.size, b.size, t), weight: lg::lerp(a.weight as f32, b.weight as f32, t).round() as u16, ..a }
}

/// Everything `draw` needs, copied out of the view so the cover cache can be borrowed mutably next to it.
struct Ink<'a> {
    st: Stand,
    th: &'a Theme,
    accent: Color,
    track: &'a Track,
    swap: f32,
    ghosts: &'a [Ghost],
    pp: f32,
    hov: &'a [Flow; 3],
    press: &'a [Flow; 3],
    /// prev's and next's push (px, signed: toward where the track goes)
    kick: [f32; 2],
    frac: f32,
    knob: f32,
    pos: u64,
    now: f64,
    /// the expansion and the places the shared parts stand in at it
    p: f32,
    pt: lg::Parts,
    cap: Rect,
    card: Rect,
    art: u64,
    under: Option<u64>,
    url: Option<&'a str>,
}

impl Ink<'_> {
    /// The whole body: the shared parts at their morphing places, then what only the card has (album, times) fading in.
    fn paint(&self, cv: &Canvas, gpu: &mut Gpu) {
        let (st, p, pt) = (self.st, self.p, self.pt);
        let (capl, cardl) = (lg::capsule_layout_in(st, self.cap), lg::card_layout(self.card));
        let card_a = ((p - 0.3) / 0.7).clamp(0.0, 1.0);
        self.cover(cv, gpu, pt.cover, pt.cover_r);
        if st.in_bar() {
            // the capsule's one line and the card's two do not become each other: they cross-fade
            let cap_a = (1.0 - p * 1.7).clamp(0.0, 1.0);
            if cap_a > 0.01 {
                // the line rides up with the cover as the body rises
                cv.with_opacity(cap_a, |cv| self.line(cv, capl.text.offset(0.0, pt.cover.y - capl.cover.y), TITLE, SUB));
            }
            if card_a > 0.01 {
                cv.with_opacity(card_a, |cv| self.lines(cv, cardl.title, cardl.sub, BIG, MID));
            }
        } else {
            self.lines(cv, pt.head, pt.sub, mix_style(TITLE, BIG, p), mix_style(SUB, MID, p));
        }
        if card_a > 0.01 {
            cv.with_opacity(card_a, |cv| {
                let album = self.track.album.trim();
                if !album.is_empty() {
                    let r = if st.in_bar() { cardl.album } else { Rect::new(pt.sub.x, pt.sub.bottom() + 4.0, pt.sub.w, 16.0) };
                    cv.text(r, album, SMALL.ellipsis(), self.th.faint);
                }
                // the times hang under the ends of the track as it travels
                cv.text(Rect::new(pt.bar.x, pt.bar.bottom() + 2.0, 80.0, 14.0), &lg::clock_text(self.pos), TIMES, self.th.dim);
                if self.track.duration_ms > 0 {
                    cv.text(Rect::new(pt.bar.right() - 80.0, pt.bar.bottom() + 2.0, 80.0, 14.0), &lg::clock_text(self.track.duration_ms), TIMES.right(), self.th.dim);
                }
            });
        }
        // a bar capsule too thin for a hairline has none to grow from: the card's track fades in instead
        let bar_a = if st.in_bar() && capl.bar.h <= 0.0 { card_a } else { 1.0 };
        if bar_a > 0.01 {
            cv.with_opacity(bar_a, |cv| self.bar(cv, pt.bar, self.knob * card_a));
        }
        for b in BTNS {
            self.button(cv, gpu, b, pt.btn[b.index()]);
        }
    }

    /// Two lines of text that cross-fade when the track changes: the old pair rises and fades, the new one rises into place, the artist a beat
    /// after the title.
    fn lines(&self, cv: &Canvas, head: Rect, sub: Rect, hs: Style, ss: Style) {
        let th = self.th;
        for g in self.ghosts {
            let a = g.a.at01();
            let rise = -4.0 * (1.0 - a);
            cv.with_opacity(a, |cv| {
                cv.text(head.offset(0.0, rise), &g.head, hs, th.text);
                cv.text(sub.offset(0.0, rise), &g.sub, ss, th.dim);
            });
        }
        let (h, s) = (self.track.heading(), self.track.sub());
        // the title arrives once the old one has mostly gone, the artist a beat after it: never two lines laid over each other
        let (kh, ks) = (((self.swap - 0.25) / 0.75).clamp(0.0, 1.0), ((self.swap - 0.4) / 0.6).clamp(0.0, 1.0));
        cv.with_opacity(kh, |cv| cv.text(head.offset(0.0, 4.0 * (1.0 - kh)), &h, hs, th.text));
        cv.with_opacity(ks, |cv| cv.text(sub.offset(0.0, 4.0 * (1.0 - ks)), &s, ss, th.dim));
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
            Btn::Prev => gpu.icon(cv, "prev", ir.offset(self.kick[0], 0.0), th.text, 1.8),
            Btn::Next => gpu.icon(cv, "next", ir.offset(self.kick[1], 0.0), th.text, 1.8),
            Btn::Play => {
                // the play and pause icons cross-fade, the leaving one shrinking away and the arriving one growing in, with the spring's pop
                let k = self.pp.clamp(0.0, 1.0);
                let pop = 1.0 + 0.10 * (self.pp - 1.0).abs().min(1.0) * (self.pp > 1.0) as u8 as f32;
                cv.with_scale_about(c, pop, |cv| {
                    cv.with_opacity(1.0 - k, |cv| cv.with_scale_about(c, 1.0 - 0.3 * k, |cv| gpu.icon(cv, "play", ir, th.text, 1.8)));
                    cv.with_opacity(k, |cv| cv.with_scale_about(c, 0.7 + 0.3 * k, |cv| gpu.icon(cv, "pause", ir, th.text, 1.8)));
                });
            }
        }
    }

    /// The track; `knob` 0 = no handle, 0.35 = a dot, 0.75 under the pointer, 1 while it is dragged (the track thickens a little with it).
    fn bar(&self, cv: &Canvas, r: Rect, knob: f32) {
        let th = self.th;
        let grow = r.h * 0.4 * ((knob - 0.35) / 0.65).clamp(0.0, 1.0);
        let r = Rect::new(r.x, r.y - grow / 2.0, r.w, r.h + grow);
        cv.fill_rr(r, r.h / 2.0, th.track);
        let w = (r.w * self.frac).clamp(0.0, r.w);
        if w > 0.5 {
            cv.fill_rr(Rect::new(r.x, r.y, w.max(r.h), r.h), r.h / 2.0, self.accent);
        }
        if knob > 0.01 {
            // the knob stays inside the track's ends, so the track keeps the card's padding (cover, times and track share their left and right edges)
            let kr = (r.h * 0.9 + 1.0) * knob.min(1.1) * 0.9;
            cv.fill_circle(((r.x + w).clamp(r.x + kr, (r.right() - kr).max(r.x + kr)), r.cy()), kr, th.text);
        }
    }

    /// The album cover in the base rectangle `r`. The picture is decoded ONCE for the card's size (a whole number of 32 px so a glide of the
    /// size does not decode it over and over) and drawn scaled into `r` as it grows from the capsule's cover to the card's: sharp at any size
    /// factor. While a new one fades in the old one stays behind it. `rad` is the corner radius in base px.
    fn cover(&self, cv: &Canvas, gpu: &mut Gpu, r: Rect, rad: f32) {
        let px = ((lg::COVER * self.st.bs() / 32.0).ceil().max(1.0) * 32.0) as u32;
        gpu.cover_sized(cv, self.th, self.art, self.url, r, self.now, rad, px, self.under);
    }

    /// The track change cross-fade of the bar capsule: ONE line, the heading and then the sub line in the dimmer ink after it (`r` is centred
    /// vertically by the style). The artist gives way first (`lg::bar_line`): it is cut to what a whole title leaves, and dropped before the title
    /// is cut. The two sizes share one baseline (`SUB_DROP`).
    fn line(&self, cv: &Canvas, r: Rect, hs: Style, ss: Style) {
        let th = self.th;
        let k = ((self.swap - 0.25) / 0.75).clamp(0.0, 1.0);
        let pair = |cv: &Canvas, h: &str, s: &str, dy: f32| {
            let (hw, sub) = lg::bar_line(r.w, cv.text_width(h, hs), if s.is_empty() { 0.0 } else { cv.text_width(s, ss) });
            // a hair of slack: a layout box exactly as wide as its text may still decide to cut it
            cv.text(Rect::new(r.x, r.y + dy, (hw.ceil() + 1.0).min(r.w), r.h), h, hs.ellipsis(), th.text);
            if let Some((sx, sw)) = sub {
                cv.text(Rect::new(r.x + sx, r.y + dy + SUB_DROP, (sw.ceil() + 1.0).min(r.w - sx), r.h), s, ss.ellipsis(), th.dim);
            }
        };
        for g in self.ghosts {
            let a = g.a.at01();
            cv.with_opacity(a, |cv| pair(cv, &g.head, &g.sub, -3.0 * (1.0 - a)));
        }
        cv.with_opacity(k, |cv| pair(cv, &self.track.heading(), &self.track.sub(), 3.0 * (1.0 - k)));
    }
}
