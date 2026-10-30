//! The capsule state machine: `Shell`, the model behind pill.js. Collapsed / hover / expanding / expanded / collapsing / cards / hidden /
//! auto-hide, the body geometry (springs + `pill_rect`), cursor feed for the liquid edge, tabs bookkeeping, settings application.
//!
//! Driving it: `Shell::new(backend, clock, os_lang, seed)`, then feed [`Event`]s with `handle`, call `tick(dt_ms, now_ms)` once per
//! display frame (also when `wants_frames()` is false, at least at `next_wakeup()`, so timers fire), draw `view()`.
//! Companion files extend the same struct: `rowlive.rs` (live row, clock, cards, weather) and `view.rs` (the output).
//!
//! Faithfulness notes: every timer of the JS (`setTimeout`) is a [`TimerKey`]; the shared rAF scheduler becomes `tick_active` + the `fps`
//! cap accumulator in `tick`; `Pill.state` is [`PillState`]; DOM measurements arrive as events (`LiveMeasured`, `Env`).

use serde_json::{json, Value};

use crate::shell::cards::{CardEvent, Cards, NO_OPEN_AFTER_DISMISS_MS};
use crate::shell::clock::{Battery, CharSide, ClockSlots, ClockState, DigitRoll, Weather};
use crate::shell::color::{palette, Palette};
use crate::shell::io::{Backend, Clock, Local};
use crate::shell::lives::Lives;
use crate::shell::liquid::{smooth, Ctx, Liquid};
use crate::shell::quiet::{exempt_toast, quiet_now, QUIET_SRC};
use crate::shell::react::{Paint, Preset, ReactOpts, Reactions, Times};
use crate::shell::settings::{js_round, Lang, ShellSettings, ThemeSetting};
use crate::shell::spring::{BodySprings, Mode, Target};
use crate::shell::tabs::{tab_off, Tab, TabSpec, Tabs};
use crate::shell::timers::{TimerKey, Timers};
use crate::shell::types::{Env, Event, LiveContent, Notice, Rect};

// ---- CFG (sizes in css px at 100 %; keep in sync with pill.css and the frames in pill.rs) ----
pub const COL_H: f64 = 30.0;
pub const COL_MIN_W: f64 = 118.0;
pub const COL_MAX_W: f64 = 500.0;
/// collapsed OS frame 720x120 (same width as the expanded one so the island never shifts on a frame resize)
pub const COL_FRAME_W: f64 = 720.0;
pub const COL_FRAME_H: f64 = 120.0;
/// 720x200 while an event card is shown (pill.rs CARD_FRAME)
pub const CARD_FRAME_H: f64 = 200.0;
pub const EXP_W: f64 = 680.0;
pub const EXP_FRAME_W: f64 = 720.0;
pub const EXP_FRAME_H: f64 = 480.0;
pub const EXP_TOP: f64 = 8.0;
pub const WAVE_PAD: f64 = 28.0;
pub const CAP_TOP: f64 = 8.0;
pub const EXP_R: f64 = 30.0;
pub const EXP_TOP_R: f64 = 26.0;
/// hover intent: the pointer has to rest this long on the pill
pub const HOVER_OPEN_MS: f64 = 250.0;
/// the pointer left the expanded pill: collapse after this
pub const LEAVE_COLLAPSE_MS: f64 = 380.0;
/// auto-hide idle delay and slide
pub const AWAY_DELAY_MS: f64 = 1500.0;
pub const AWAY_SLIDE_MS: f64 = 340.0;
pub const AWAY_SLIDE_REDUCED_MS: f64 = 20.0;
/// the OS frame must reach the expanded size within this, else the spring starts anyway
pub const EXP_FRAME_WAIT_MS: f64 = 50.0;
/// timer covering a paused rAF during that wait
pub const EXP_FRAME_TIMER_MS: f64 = 55.0;
pub const QUIET_SWEEP_MS: f64 = 20_000.0;
pub const BATTERY_POLL_MS: f64 = 60_000.0;
pub const WEATHER_MS: f64 = 15.0 * 60_000.0;
/// longest frame step the loop integrates (`1/20` s)
pub const MAX_DT_S: f64 = 1.0 / 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PillState {
    Collapsed,
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Done {
    CollapseFinal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingOpen {
    FromExpand,
    FromButton,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExpWait {
    pub t0: f64,
    pub fw: f64,
}

/// `bodyTy()` state: the mirrored CSS translateY transition of `#body`.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Ty {
    pub from: f64,
    pub to: f64,
    pub t0: Option<f64>,
}

/// What `paintBody` wrote to the DOM.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct BodyPaint {
    pub w: f64,
    pub h: f64,
    pub left: f64,
    pub panel_left: f64,
    pub r: f64,
    pub t: f64,
}

pub struct Shell<B: Backend, C: Clock> {
    pub(super) backend: B,
    pub(super) clock: C,
    pub(super) s: ShellSettings,
    pub(super) os_lang: String,
    pub(super) lang: Lang,
    pub(super) env: Env,
    /// `performance.now()` of the last tick / `handle_at`
    pub(super) now: f64,
    /// `Date.now()` at the last tick
    pub(super) wall: i64,
    // --- flags of the IIFE ---
    pub(super) hidden: bool,
    pub(super) away: bool,
    pub(super) hovering: bool,
    pub(super) no_open_until: f64,
    pub(super) typing: bool,
    pub(super) frame_exp: bool,
    pub(super) state: PillState,
    // --- the shared frame loop ---
    pub(super) tick_active: bool,
    pub(super) cap_acc: f64,
    // --- body ---
    pub(super) sp: BodySprings,
    pub(super) paint: BodyPaint,
    pub(super) ty: Ty,
    pub(super) pending: Vec<Done>,
    pub(super) last_rect: Option<Rect>,
    pub(super) last_mv: (f64, f64),
    // --- edge + reactions ---
    pub(super) liq: Liquid,
    pub(super) rx: Reactions,
    pub(super) wave_c: String,
    // --- live row ---
    pub(super) lives: Lives,
    pub(super) live_key: String,
    pub(super) live_tall: bool,
    pub(super) root_tall: bool,
    pub(super) clock_state: ClockState,
    pub(super) battery: Option<Battery>,
    pub(super) wx: Option<Weather>,
    pub(super) wx_at: Option<i64>,
    pub(super) wx_net: Option<bool>,
    pub(super) slots: ClockSlots,
    pub(super) measure: (f64, f64),
    pub(super) pet_res: f64,
    pub(super) content: LiveContent,
    pub(super) ghost: Option<(LiveContent, f64)>,
    pub(super) swap_epoch: u64,
    pub(super) swap_at: f64,
    pub(super) rolls: Vec<(DigitRoll, f64)>,
    pub(super) entering: Vec<(bool, usize, f64)>,
    pub(super) char_side: CharSide,
    pub(super) char_legs: bool,
    // --- tabs ---
    pub(super) tabs: Tabs,
    pub(super) exp_wait: Option<ExpWait>,
    pub(super) panel_shown_at: f64,
    // --- cards ---
    pub(super) cards: Cards,
    pub(super) carding: bool,
    pub(super) card_gen: u64,
    pub(super) shown_card: u64,
    // --- settings bookkeeping ---
    pub(super) applied_layout: Option<String>,
    pub(super) applied_clock: Option<String>,
    pub(super) pending_open: Option<PendingOpen>,
    pub(super) pending_fg: bool,
    pub(super) dictate: bool,
    pub(super) notices: Vec<Notice>,
    pub(super) timers: Timers,
}

impl<B: Backend, C: Clock> Shell<B, C> {
    pub fn new(backend: B, clock: C, os_lang: &str, seed: u64) -> Shell<B, C> {
        let wall = clock.wall_ms();
        let mut sh = Shell {
            backend,
            clock,
            s: ShellSettings::default(),
            os_lang: os_lang.to_string(),
            lang: crate::shell::settings::lang_from_os(os_lang),
            env: Env::default(),
            now: 0.0,
            wall,
            hidden: false,
            away: false,
            hovering: false,
            no_open_until: 0.0,
            typing: false,
            frame_exp: false,
            state: PillState::Collapsed,
            tick_active: false,
            cap_acc: 0.0,
            sp: BodySprings::default(),
            paint: BodyPaint::default(),
            ty: Ty::default(),
            pending: Vec::new(),
            last_rect: None,
            last_mv: (0.0, 0.0),
            liq: Liquid::new(seed),
            rx: Reactions::new(),
            wave_c: String::new(),
            lives: Lives::new(),
            live_key: String::new(),
            live_tall: false,
            root_tall: false,
            clock_state: ClockState::default(),
            battery: None,
            wx: None,
            wx_at: None,
            wx_net: None,
            slots: ClockSlots::default(),
            measure: (0.0, 0.0),
            pet_res: 0.0,
            content: LiveContent::None,
            ghost: None,
            swap_epoch: 0,
            swap_at: f64::NEG_INFINITY,
            rolls: Vec::new(),
            entering: Vec::new(),
            char_side: CharSide::Unset,
            char_legs: false,
            tabs: Tabs::default(),
            exp_wait: None,
            panel_shown_at: f64::NEG_INFINITY,
            cards: Cards::new(),
            carding: false,
            card_gen: 0,
            shown_card: 0,
            applied_layout: None,
            applied_clock: None,
            pending_open: None,
            pending_fg: false,
            dictate: false,
            notices: Vec::new(),
            timers: Timers::new(),
        };
        // the hidden fallback Settings tab (order 900) is registered at load
        sh.tabs.list.push(Tab { spec: TabSpec { id: "settings".into(), title: "Settings".into(), order: 900, hidden: true, height: None }, off: false, badge: 0, warned: false });
        sh.timers.set(TimerKey::Quiet, QUIET_SWEEP_MS);
        sh.apply_settings(); // the JS calls applySettings() once at load, before settings_get answers
        sh
    }

    // ---------- accessors ----------
    pub fn backend(&self) -> &B {
        &self.backend
    }
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
    pub fn clock_mut(&mut self) -> &mut C {
        &mut self.clock
    }
    pub fn settings(&self) -> &ShellSettings {
        &self.s
    }
    pub fn lang(&self) -> Lang {
        self.lang
    }
    pub fn state(&self) -> PillState {
        self.state
    }
    pub fn liquid(&self) -> &Liquid {
        &self.liq
    }
    pub fn reactions(&self) -> &Reactions {
        &self.rx
    }
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
    pub fn is_away(&self) -> bool {
        self.away
    }
    pub fn is_hovering(&self) -> bool {
        self.hovering
    }
    pub fn frame_expanded(&self) -> bool {
        self.frame_exp
    }
    pub fn cards(&self) -> &Cards {
        &self.cards
    }
    pub fn tabs(&self) -> &Tabs {
        &self.tabs
    }
    pub fn lives(&self) -> &Lives {
        &self.lives
    }
    /// `Pill.fps`: user cap of the frame loop (0 = display rate).
    pub fn fps_cap(&self) -> u32 {
        self.s.fps_cap
    }
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }
    /// The spring targets/values the renderer may feed to its own solver: `(x, v, target)` of w, h, r, t.
    pub fn spring_state(&self) -> [(f64, f64, f64); 4] {
        let f = |s: &crate::shell::spring::Spring| (s.x, s.v, s.t);
        [f(&self.sp.w), f(&self.sp.h), f(&self.sp.r), f(&self.sp.t)]
    }

    // ---------- derived values ----------
    pub fn reduce(&self) -> bool {
        self.s.reduce_motion || self.env.os_reduce_motion
    }
    pub fn scale(&self) -> f64 {
        self.s.scale()
    }
    pub(super) fn capsule(&self) -> bool {
        self.s.is_capsule()
    }
    pub(super) fn snap_px(&self, v: f64) -> f64 {
        js_round(v * self.env.dpr) / self.env.dpr
    }
    pub(super) fn times(&self) -> Times {
        Times { mono: self.now, wall: self.wall }
    }
    pub(super) fn palette(&self) -> Palette {
        palette(self.s.resolved_theme(self.env.os_prefers_light))
    }
    pub(super) fn body_alpha(&self) -> f64 {
        if self.state == PillState::Expanded {
            self.s.op_exp()
        } else {
            self.s.op_col()
        }
    }
    pub(super) fn local(&self) -> Local {
        Local::from_wall(self.wall, self.clock.utc_offset_min())
    }
    pub fn quiet(&self) -> bool {
        quiet_now(&self.s.quiet, &self.local())
    }
    pub(super) fn tr(&self, ru: &'static str, en: &'static str) -> &'static str {
        match self.lang {
            Lang::Ru => ru,
            Lang::En => en,
        }
    }
    pub(super) fn invoke(&mut self, cmd: &str, args: Value) {
        self.backend.invoke(cmd, args);
    }
    fn can_animate(&self) -> bool {
        !(self.reduce() || self.hidden || self.env.occluded)
    }

    // ---------- liquid glue ----------
    pub(super) fn liq_ctx(&self) -> Ctx {
        Ctx {
            scale: self.scale(),
            w: self.sp.w.x,
            h: self.sp.h.x,
            r: self.sp.r.x,
            t: self.sp.t.x,
            expanded: self.state == PillState::Expanded,
            reduce: self.reduce(),
            now_ms: self.now,
            edge_idle: self.s.edge_idle,
            edge_cones: self.s.edge_cones,
            idle_fps: self.s.idle_fps as f64,
            spring_busy: self.sp.busy,
        }
    }
    pub(super) fn liq_colors(&mut self) {
        let pal = self.palette();
        let fill = pal.bg.with_alpha(self.body_alpha() as f32);
        let wave = self.wave_c.clone();
        let reduce = self.reduce();
        self.liq.resolve_colors(&pal, &wave, fill, reduce);
    }
    /// `kick()`: run the frame loop (never under reduced motion).
    pub(super) fn kick(&mut self) {
        if !self.reduce() {
            self.tick_active = true;
        }
    }
    pub(super) fn liq_static(&mut self) {
        let c = self.liq_ctx();
        self.liq.render_static(&c);
    }
    pub(super) fn liq_playing(&mut self, on: bool) {
        let c = self.liq_ctx();
        let kick = self.liq.set_playing(on, &c);
        self.liq_colors();
        if kick {
            self.kick();
        } else if !self.tick_active {
            self.liq_static();
        }
    }
    /// `paintReact` consequences.
    pub(super) fn apply_paint(&mut self, p: Option<Paint>) {
        let Some(p) = p else { return };
        self.liq.paint_react(p.name, &p.color);
        self.notices.push(Notice::Reaction { name: p.name, color: p.color });
        self.liq_colors();
        if !self.reduce() {
            self.kick();
        } else if !self.tick_active {
            self.liq_static();
        }
    }

    // ---------- reactions API (Pill.react / unreact / setReaction) ----------
    pub fn react(&mut self, source: &str, name: &str, o: &ReactOpts) {
        let (t, q) = (self.times(), self.quiet());
        let p = self.rx.react(source, name, o, &t, q);
        self.apply_paint(p);
    }
    pub fn unreact(&mut self, source: &str) {
        let t = self.times();
        let p = self.rx.unreact(source, &t);
        self.apply_paint(p);
    }
    /// Low-level setter / preview; returns the preset applied.
    pub fn set_reaction(&mut self, name: &str, o: &ReactOpts) -> Preset {
        let t = self.times();
        let (p, paint) = self.rx.set_reaction(name, o, &t);
        self.apply_paint(paint);
        p
    }
    /// Activity or the pointer: wake from sleep and restart the idle timer.
    pub(super) fn poke(&mut self) {
        let t = self.times();
        let p = self.rx.poke(&t);
        self.apply_paint(p);
    }
    pub fn set_edge_color(&mut self, c: &str) {
        self.liq.override_color = c.to_string();
        self.liq_colors();
        if !self.tick_active {
            self.liq_static();
        }
    }
    /// `Pill.setDragPull`: files are dragged over the island.
    pub fn set_drag_pull(&mut self, on: bool) {
        if on == self.liq.drag {
            return;
        }
        self.liq.drag = on;
        if !on && self.liq.pull_active() {
            self.kick();
        }
    }
    /// `applyCharacter`: the Character page's edge style, live.
    pub(super) fn apply_character(&mut self) {
        let v = self.s.character;
        if v == self.liq.style {
            return;
        }
        self.liq.style = v;
        self.liq_colors();
        self.kick();
        if !self.tick_active {
            self.liq_static();
        }
    }

    // ---------- body geometry ----------
    /// `bodyTy()`: current translateY of `#body`; also notes the moment a new transition starts.
    pub(super) fn body_ty(&mut self) -> f64 {
        if self.away {
            return -1.3 * self.sp.h.x; // .away: translateY(-130%)
        }
        let to = self.rest_ty();
        let now = self.now;
        let p = if self.reduce() { 1.0 } else { ((now - self.ty.t0.unwrap_or(f64::NEG_INFINITY)) / crate::shell::motion::BODY_TRANSLATE_MS).min(1.0) };
        let cur = self.ty.from + (self.ty.to - self.ty.from) * crate::shell::motion::ease_out_quart(p);
        if self.ty.t0.is_none() {
            // first call: the page loaded at rest
            self.ty = Ty { from: to, to, t0: Some(f64::NEG_INFINITY) };
            return to;
        }
        if to != self.ty.to {
            self.ty.from = cur;
            self.ty.to = to;
            self.ty.t0 = Some(now);
        }
        cur
    }
    fn rest_ty(&self) -> f64 {
        if self.state == PillState::Expanded {
            self.snap_px(EXP_TOP * self.scale())
        } else if self.capsule() {
            self.snap_px(CAP_TOP * self.scale())
        } else {
            0.0
        }
    }
    /// Non-mutating `translateY` for the view.
    pub(super) fn current_ty(&self) -> f64 {
        if self.away {
            return -1.3 * self.sp.h.x;
        }
        match self.ty.t0 {
            None => self.rest_ty(),
            Some(t0) => {
                let p = if self.reduce() { 1.0 } else { ((self.now - t0) / crate::shell::motion::BODY_TRANSLATE_MS).clamp(0.0, 1.0) };
                self.ty.from + (self.ty.to - self.ty.from) * crate::shell::motion::ease_out_quart(p)
            }
        }
    }
    /// `paintBody(imp)`
    pub(super) fn paint_body(&mut self, imp: bool) {
        let s = self.scale();
        let (r, tp) = (self.sp.r.x.max(0.0), self.sp.t.x.max(0.0));
        self.paint = BodyPaint {
            w: self.sp.w.x,
            h: self.sp.h.x,
            left: self.snap_px((self.env.inner_w - self.sp.w.x) / 2.0),
            panel_left: self.snap_px((self.sp.w.x - EXP_W * s) / 2.0),
            r,
            t: tp,
        };
        let d = (self.sp.h.x - self.liq.last_h) / s;
        self.liq.last_h = self.sp.h.x;
        if imp && d.abs() > 0.02 && !self.reduce() && !self.hidden {
            let c = self.liq_ctx();
            self.liq.impulse(d, &c);
        }
    }
    pub(super) fn col_target(&self) -> Target {
        let s = self.scale();
        let hgt = COL_H * s;
        let ev = |x: f64| {
            let x = x.ceil();
            x + (x % 2.0)
        };
        let (mw, mh) = self.measure;
        let wide = ev((COL_MAX_W * s).min((COL_MIN_W * s).max(mw + self.pet_res * s)));
        if (self.cards.has_current() && self.live_key == "card") || self.live_tall {
            // the island grown to an event card (or a tall live widget): its own content height, round ends
            let hh = ev(hgt.max(mh));
            let rr = (24.0 * s).min(hh / 2.0);
            return Target { w: wide, h: hh, r: rr, t: if self.capsule() { rr } else { 0.0 } };
        }
        Target { w: wide, h: hgt, r: hgt / 2.0, t: if self.capsule() { hgt / 2.0 } else { 0.0 } }
    }
    pub(super) fn exp_target(&mut self) -> Target {
        let s = self.scale();
        let (hh, warn) = self.tabs.body_height(EXP_FRAME_H, EXP_TOP, WAVE_PAD);
        let _ = warn; // the JS console.warn; the renderer team can log through `Notice` if they want it
        Target { w: EXP_W * s, h: hh * s, r: EXP_R * s, t: EXP_TOP_R * s }
    }
    /// Current expanded panel height (css px at 100 %).
    pub fn panel_height(&mut self) -> f64 {
        self.tabs.body_height(EXP_FRAME_H, EXP_TOP, WAVE_PAD).0
    }
    pub(super) fn report_rect(&mut self, tg: &Target) {
        let exp = self.state == PillState::Expanded;
        let s = self.scale();
        let fw = (if self.frame_exp { EXP_FRAME_W } else { COL_FRAME_W }) * s;
        let y = (if exp {
            EXP_TOP
        } else if self.capsule() {
            CAP_TOP
        } else {
            0.0
        }) * s;
        let r = Rect { x: (fw - tg.w) / 2.0, y, w: tg.w, h: tg.h };
        if self.last_rect != Some(r) {
            self.last_rect = Some(r);
            self.invoke("pill_rect", json!({"x": r.x, "y": r.y, "w": r.w, "h": r.h}));
        }
    }
    /// `retarget(snapNow, done, mode)`
    pub(super) fn retarget(&mut self, snap_now: bool, done: Option<Done>, mode: Mode) {
        self.body_ty(); // note the moment the CSS translateY transition of #body starts
        self.liq_colors();
        let tg = if self.state == PillState::Expanded { self.exp_target() } else { self.col_target() };
        if snap_now {
            if let Some(d) = done {
                self.pending.push(d);
            }
            self.snap(&tg);
        } else {
            self.go(&tg, done, mode);
        }
        self.report_rect(&tg);
    }
    fn go(&mut self, tg: &Target, done: Option<Done>, mode: Mode) {
        if let Some(d) = done {
            self.pending.push(d);
        }
        if !self.can_animate() {
            self.snap(tg);
            return;
        }
        self.sp.retarget(tg, mode);
        self.kick();
    }
    fn snap(&mut self, tg: &Target) {
        self.sp.snap(tg);
        self.paint_body(false);
        if !self.tick_active {
            self.liq_static();
        }
        self.finish();
    }
    fn finish(&mut self) {
        let fs: Vec<Done> = self.pending.drain(..).collect();
        for d in fs {
            match d {
                Done::CollapseFinal => self.collapse_final(),
            }
        }
        if self.state == PillState::Collapsed {
            self.next(); // queued event cards run whichever retarget settled last
        }
    }

    // ---------- expand / collapse ----------
    /// `Pill.expand(tab, via)`: ask the backend (it answers with `pill:expand`).
    pub fn expand(&mut self, tab: Option<&str>, via: &str) {
        self.invoke("pill_set_state", json!({"state": "expanded", "tab": tab, "via": if via.is_empty() { "page" } else { via }}));
    }
    /// `Pill.collapse(how)`: forced; `how == "click"` squashes deeper (after a paste).
    pub fn collapse(&mut self, how: &str) {
        self.do_collapse(true, how == "click");
    }
    fn open_settings(&mut self, page: Option<&str>) {
        const PAGES: [&str; 14] = ["general", "appearance", "behavior", "search", "character", "hotkeys", "ai", "voice", "weather", "sound", "live", "clipboard", "modules", "updates"];
        let page = page.filter(|p| PAGES.contains(p)).unwrap_or("general");
        self.invoke("settings_open", json!({"page": page}));
    }
    pub(super) fn do_expand(&mut self, tab: Option<&str>, section: Option<&str>) {
        if tab == Some("settings") {
            // the command bar asks for the settings tab: open the window instead and give the frame back
            self.pending_open = Some(PendingOpen::FromExpand);
            self.open_settings(section);
            return;
        }
        let tab = if tab == Some("settings_fallback") { Some("settings") } else { tab };
        let was = self.state == PillState::Expanded;
        let frame_was = self.frame_exp;
        let fw = EXP_FRAME_W * self.scale() - 2.0;
        // the OS window grows just before this event: start the spring after the first frame in which the page already has the expanded
        // frame size (capped at 50 ms), so the surface resize does not eat the first spring frames
        let wait = !was && !frame_was && self.can_animate() && self.env.inner_w < fw;
        self.exp_wait = if wait { Some(ExpWait { t0: self.now, fw }) } else { None };
        self.state = PillState::Expanded;
        self.frame_exp = true;
        self.cancel_away();
        if !was {
            self.panel_shown_at = self.now;
        }
        match tab {
            Some(t) if self.tabs.get(t).is_some_and(|x| !x.off) => {
                self.tabs.chosen = true;
                if self.tabs.cur.as_deref() != Some(t) {
                    self.activate(t, true);
                } else if !was {
                    self.notices.push(Notice::TabShown(t.to_string()));
                }
            }
            _ => {
                if !was {
                    if let Some(c) = self.tabs.cur.clone() {
                        self.notices.push(Notice::TabShown(c));
                    }
                }
            }
        }
        if !was {
            if self.exp_wait.is_none() {
                self.retarget(false, None, Mode::Expand);
                return;
            }
            self.timers.set(TimerKey::ExpFrame, self.now + EXP_FRAME_TIMER_MS);
        }
    }
    /// The frame-size wait is over (frame arrived / 50 ms passed / timer): start the expand spring.
    pub(super) fn exp_go(&mut self) {
        if self.exp_wait.take().is_none() {
            return;
        }
        self.timers.clear(TimerKey::ExpFrame);
        if self.state == PillState::Expanded {
            self.retarget(false, None, Mode::Expand);
        }
    }
    pub(super) fn do_collapse(&mut self, force: bool, deep: bool) {
        if self.state != PillState::Expanded {
            return;
        }
        if self.s.pinned && !force {
            return;
        }
        self.state = PillState::Collapsed;
        self.notices.push(Notice::Blur);
        if let Some(c) = self.tabs.cur.clone() {
            self.notices.push(Notice::TabHidden(c));
        }
        self.retarget(false, Some(Done::CollapseFinal), if deep { Mode::Squash } else { Mode::Collapse });
    }
    fn collapse_final(&mut self) {
        if self.state != PillState::Collapsed {
            return; // re-expanded during the animation
        }
        self.frame_exp = false;
        self.invoke("pill_set_state", json!({"state": "collapsed"}));
        let tg = self.col_target();
        self.report_rect(&tg);
        self.schedule_away();
        self.next();
    }
    /// Hidden by the backend: no animation at all.
    fn force_collapse(&mut self) {
        let was = self.state == PillState::Expanded;
        self.state = PillState::Collapsed;
        if was {
            if let Some(c) = self.tabs.cur.clone() {
                self.notices.push(Notice::TabHidden(c));
            }
        }
        if self.frame_exp {
            self.frame_exp = false;
            self.invoke("pill_set_state", json!({"state": "collapsed"}));
        }
        self.retarget(true, None, Mode::Resize);
    }

    // ---------- tabs ----------
    /// `Pill.register(o)` (the tab module mounts its own view on `Notice::TabMounted`).
    pub fn register_tab(&mut self, spec: TabSpec) {
        if spec.id.is_empty() || self.tabs.has(&spec.id) {
            return;
        }
        let id = spec.id.clone();
        let hidden = spec.hidden;
        let off = tab_off(&id, &self.s);
        self.tabs.list.push(Tab { spec, off, badge: 0, warned: false });
        self.notices.push(Notice::TabMounted(id));
        if !self.tabs.chosen && !hidden {
            if let Some(first) = self.tabs.first_visible() {
                self.activate(&first, false);
            }
        }
    }
    pub fn set_badge(&mut self, id: &str, n: u32) {
        if let Some(t) = self.tabs.get_mut(id) {
            if !t.spec.hidden {
                t.badge = n;
            }
        }
    }
    pub(super) fn activate(&mut self, id: &str, persist: bool) {
        let Some(tb) = self.tabs.get(id) else { return };
        if self.tabs.cur.as_deref() == Some(id) {
            return;
        }
        let tb_hidden = tb.spec.hidden;
        // the island reopens on the last used tab
        if persist && !tb_hidden && self.s.last_tab != id {
            self.s.last_tab = id.to_string();
            self.invoke("settings_set", json!({"patch": {"lastTab": id}}));
        }
        if let Some(old) = self.tabs.cur.clone() {
            self.tabs.dir = self.tabs.direction(&old, id);
            let old_hidden = self.tabs.get(&old).is_some_and(|t| t.spec.hidden);
            if !old_hidden {
                self.tabs.prev = Some(old.clone());
            }
            if self.state == PillState::Expanded {
                self.notices.push(Notice::TabHidden(old));
            }
        }
        self.tabs.cur = Some(id.to_string());
        if self.state == PillState::Expanded {
            self.notices.push(Notice::TabShown(id.to_string()));
            if self.exp_wait.is_none() {
                self.retarget(false, None, Mode::Tab); // tabs differ in height: the panel height follows its own spring
            }
        }
    }
    /// Settings > Modules: a module that is off hides its tab (live, from settings:changed).
    pub(super) fn apply_modules(&mut self) {
        let mut changed = false;
        let ids: Vec<String> = self.tabs.list.iter().map(|t| t.spec.id.clone()).collect();
        for id in ids {
            let off = tab_off(&id, &self.s);
            if let Some(t) = self.tabs.get_mut(&id) {
                if off != t.off {
                    t.off = off;
                    changed = true;
                }
            }
        }
        if !changed {
            return;
        }
        let cur_off = self.tabs.cur.as_deref().and_then(|c| self.tabs.get(c)).is_some_and(|t| t.off);
        if cur_off {
            if let Some(first) = self.tabs.first_visible() {
                self.activate(&first, false); // the open tab was switched off: go to the first one that is left
            }
        }
        if self.state == PillState::Expanded {
            self.retarget(false, None, Mode::Tab);
        }
    }

    // ---------- auto-hide ----------
    pub(super) fn cancel_away(&mut self) {
        self.timers.clear(TimerKey::Away);
    }
    pub(super) fn schedule_away(&mut self) {
        self.cancel_away();
        if self.s.auto_hide && !self.away && !self.hidden && self.state == PillState::Collapsed && !self.hovering && !self.cards.has_current() && self.cards.queue_len() == 0 {
            self.timers.set(TimerKey::Away, self.now + AWAY_DELAY_MS);
        }
    }
    fn slide_away(&mut self) {
        if !self.s.auto_hide || self.away || self.state != PillState::Collapsed || self.hovering || self.cards.has_current() || self.cards.queue_len() > 0 {
            return;
        }
        self.away = true;
        let d = if self.reduce() { AWAY_SLIDE_REDUCED_MS } else { AWAY_SLIDE_MS };
        self.timers.set(TimerKey::AwaySlide, self.now + d);
    }
    pub(super) fn reveal(&mut self) {
        if self.away {
            self.invoke("pill_auto_hide", json!({"on": false}));
        }
    }

    // ---------- cursor ----------
    /// `setCursor(x, y)`: window-relative css px; `None` = cursor gone. Drives the liquid edge's cursor pull.
    pub(super) fn set_cursor(&mut self, p: Option<(f64, f64)>) {
        let Some((x, y)) = p.filter(|_| !self.reduce() && !self.hidden) else {
            self.liq.want = 0.0;
            if self.liq.pull_active() {
                self.kick();
            }
            return;
        };
        self.poke();
        if (x - self.last_mv.0).hypot(y - self.last_mv.1) > 2.0 {
            self.last_mv = (x, y);
            if self.timers.has(TimerKey::Open) && self.hovering && self.state != PillState::Expanded {
                self.arm_open(); // hover intent: the pointer has to rest
            }
        }
        // body box from the spring state (no layout read): left / width / height are what paintBody writes, the vertical offset is the
        // CSS translateY transition of #body
        let s = self.scale();
        let bw = self.sp.w.x;
        let bcx = self.snap_px((self.env.inner_w - bw) / 2.0) + bw / 2.0;
        let bbot = self.body_ty() + self.sp.h.x;
        let half = bw / s / 2.0;
        let lx = (x - bcx) / s;
        let dyb = (y - bbot) / s;
        let expd = self.state == PillState::Expanded;
        // measured: collapsed sag up to 14 px (19-20 with the pointer inside), onset ~25 px before contact; expanded: 15-25 px just under
        // the edge, a +5..11 px bulge while the pointer is above it (up to ~200 px)
        let (mut f, mut a);
        if dyb > 0.0 {
            f = 1.0 - dyb / 25.0;
            a = if expd { 20.0 } else { 14.0 };
        } else {
            let up = -dyb;
            f = 1.0;
            a = if expd { 11.0 * (1.0 - up / 220.0).max(0.0) } else { 14.0 + 5.0 * smooth(up / 12.0) };
        }
        f *= (1.0 - ((lx.abs() - half).max(0.0)) / 60.0).max(0.0);
        if self.liq.drag {
            a = a.max(21.0); // a file drag hovers: the edge always reaches for it
            f = f.max(0.9);
        }
        self.liq.want = if a > 0.5 { smooth(f) } else { 0.0 };
        self.liq.b_amp = a;
        self.liq.cx = lx.clamp(-half, half);
        if self.liq.want > 0.0 || self.liq.pull_active() {
            self.kick();
        }
    }
    fn arm_open(&mut self) {
        self.timers.set(TimerKey::Open, self.now + HOVER_OPEN_MS);
    }

    // ---------- settings ----------
    pub(super) fn apply_settings(&mut self) {
        let s = self.scale();
        let layout_sig = format!("{}|{:?}|{}", s, self.s.shape, self.env.dpr);
        let clock_sig = format!(
            "{}|{}|{}|{}|{}|{}|{}|{:?}",
            self.s.clock24, self.s.show_time, self.s.show_date, self.s.show_temp, self.s.show_wx_icon, self.s.show_seconds, self.s.show_battery, self.lang
        );
        let vpn = self.s.vpn_country.clone();
        self.set_vpn(&vpn);
        self.apply_modules();
        // every settings_set (also the lastTab write on a tab switch) comes back as settings:changed: only touch what actually changed
        if self.applied_clock.as_deref() != Some(&clock_sig) {
            self.applied_clock = Some(clock_sig);
            self.invalidate_live();
            self.render_live();
        }
        if self.applied_layout.as_deref() != Some(&layout_sig) {
            self.applied_layout = Some(layout_sig);
            self.retarget(true, None, Mode::Resize);
        }
        self.liq_colors();
        if self.s.auto_hide {
            self.schedule_away();
        } else {
            self.cancel_away();
        }
    }
    /// `Pill.setVpn(cc)`: country badge, upper-cased, at most 3 chars.
    pub fn set_vpn(&mut self, cc: &str) {
        self.s.vpn_country = cc.to_uppercase().chars().take(3).collect();
    }
    /// `typeof Pill.dictate === 'function'`: the voice module handles the dictation button (else a "coming soon" toast shows).
    pub fn set_dictate_available(&mut self, on: bool) {
        self.dictate = on;
    }
    pub fn set_pinned(&mut self, v: bool) {
        self.s.pinned = v;
    }
    /// `Pill.reserve(px)`: widen the collapsed capsule by `px` css px (a pet lives there).
    pub fn reserve(&mut self, px: f64) {
        let px = js_round(px.max(0.0));
        if px == self.pet_res {
            return;
        }
        self.pet_res = px;
        if self.state == PillState::Collapsed && !self.hidden {
            self.retarget(false, None, Mode::Resize);
        }
    }
    fn check_lang(&mut self) {
        let want = self.s.resolved_lang(&self.os_lang);
        if want != self.lang {
            // the JS reloaded the page; the native UI switches live and tells everyone
            self.lang = want;
            self.notices.push(Notice::LangChanged(want));
            self.invalidate_live();
            self.applied_clock = None;
            self.apply_settings();
        }
    }

    // ---------- events ----------
    /// Feed one event. Uses the time of the last `tick` / `handle_at`.
    pub fn handle(&mut self, ev: Event) {
        match ev {
            Event::Start(v) => {
                self.s.apply_json(&v);
                self.apply_settings();
                self.check_lang();
                self.apply_character();
                self.poke();
                let last = self.s.last_tab.clone();
                if !self.tabs.chosen && !last.is_empty() && self.tabs.get(&last).is_some_and(|t| !t.spec.hidden && !t.off) {
                    self.activate(&last, false);
                }
                self.schedule_away();
                self.poll_battery();
                self.refresh_weather();
            }
            Event::SettingsChanged(v) => {
                self.s.apply_json(&v);
                self.apply_settings();
                self.check_lang();
                if Some(self.s.wx_wanted()) != self.wx_net {
                    self.refresh_weather();
                }
                self.apply_character();
            }
            Event::Show => {
                self.hidden = false;
                self.wx_on_show();
                self.away = false;
                self.retarget(true, None, Mode::Resize);
                self.render_live();
                self.poll_battery();
                self.schedule_away();
                self.next();
            }
            Event::Hide => {
                // stop every timer and animation: zero work while hidden
                self.hidden = true;
                self.timers.clear(TimerKey::Live);
                self.timers.clear(TimerKey::Battery);
                self.timers.clear(TimerKey::Weather);
                self.liq.clear_pull();
                self.cancel_away();
                self.drop_cards();
                self.hovering = false;
                self.force_collapse();
            }
            Event::Expand { tab, section } => {
                self.poke();
                self.do_expand(tab.as_deref(), section.as_deref());
            }
            Event::OpenTab { tab, section } => {
                if tab.as_deref() == Some("settings") {
                    self.do_expand(Some("settings"), section.as_deref());
                }
            }
            Event::Collapse => self.do_collapse(false, false),
            Event::Hover => {
                self.hovering = true;
                self.poke();
                self.cancel_away();
                self.timers.clear(TimerKey::Leave);
                if self.state != PillState::Expanded {
                    self.arm_open();
                }
            }
            Event::Leave => {
                self.hovering = false;
                self.timers.clear(TimerKey::Open);
                if self.state == PillState::Expanded && !self.typing {
                    self.timers.set(TimerKey::Leave, self.now + LEAVE_COLLAPSE_MS);
                } else {
                    self.schedule_away();
                }
            }
            Event::Cursor(p) => self.set_cursor(p),
            Event::MouseLeave => self.set_cursor(None),
            Event::AudioBands(b) => {
                if self.liq.set_bands(&b, self.now) {
                    // real sound = music baseline; the resolver timer re-checks `until`
                    if !self.rx.extend("audio", self.wall + 2500) {
                        self.react("audio", "music", &ReactOpts::ttl(2500));
                    }
                }
                let c = self.liq_ctx();
                if self.liq.music_on(&c) {
                    self.kick();
                }
            }
            Event::WeatherCurrent(v) => self.set_wx(Weather::from_json(&v)),
            Event::BatteryState(v) => self.on_battery(&v),
            Event::WeatherGet(v) => self.on_weather_get(&v),
            Event::SettingsOpened(ok) => match self.pending_open.take() {
                Some(PendingOpen::FromExpand) => {
                    if ok {
                        if self.frame_exp || self.state == PillState::Expanded {
                            if self.state == PillState::Expanded {
                                self.do_collapse(true, false);
                            } else {
                                self.invoke("pill_set_state", json!({"state": "collapsed"}));
                            }
                        }
                    } else {
                        self.do_expand(Some("settings_fallback"), None);
                    }
                }
                Some(PendingOpen::FromButton) => {
                    if ok {
                        self.do_collapse(true, false);
                    } else {
                        self.tabs.chosen = true;
                        self.activate("settings", false);
                    }
                }
                None => {}
            },
            Event::DesktopFg(fg) => {
                if std::mem::take(&mut self.pending_fg) {
                    if fg {
                        self.schedule_away();
                    } else {
                        self.slide_away();
                    }
                }
            }
            Event::TextFocus(on) => {
                self.typing = on;
                self.invoke("pill_activate", json!({"on": on}));
                if !on && self.state == PillState::Expanded && !self.hovering {
                    self.timers.set(TimerKey::FocusOut, self.now + 300.0);
                }
            }
            Event::Escape | Event::BackgroundClick => self.do_collapse(false, false),
            Event::Hotkey => self.invoke("pill_toggle", json!({})),
            Event::SettingsButton => {
                if self.tabs.cur.as_deref() == Some("settings") {
                    let back = self.tabs.prev.clone().filter(|p| self.tabs.has(p)).or_else(|| self.tabs.first_visible());
                    if let Some(b) = back {
                        self.activate(&b, false);
                    }
                    return;
                }
                self.pending_open = Some(PendingOpen::FromButton);
                self.open_settings(Some("general"));
            }
            Event::PinButton => self.s.pinned = !self.s.pinned,
            Event::DictButton => {
                if self.dictate {
                    self.notices.push(Notice::Dictate); // Pill.dictate is wired: the voice module starts dictation
                    return;
                }
                let title = self.tr("Диктовка скоро", "Dictation is coming soon").to_string();
                self.toast(crate::shell::cards::CardSpec { id: Some("dict-soon".into()), icon: "wave".into(), title, ttl_ms: 1800.0, ..Default::default() });
            }
            Event::TabClicked(id) => {
                self.tabs.chosen = true;
                self.activate(&id, true);
            }
            Event::CardAction(i) => {
                let act = self.cards.current_spec().and_then(|c| c.actions.get(i).map(|a| (c.id.clone(), a.id.clone())));
                if let Some((card_id, action_id)) = act {
                    self.notices.push(Notice::CardAction { card_id, action_id });
                    self.dismiss();
                }
            }
            Event::Env(e) => {
                let old = self.env;
                self.env = e;
                // onResize: offsets, body, strip; a monitor / DPI change re-runs the layout
                self.paint_body(false);
                if !self.tick_active {
                    self.liq_static();
                }
                if (old.os_prefers_light != e.os_prefers_light && self.s.theme == ThemeSetting::Auto) || old.os_reduce_motion != e.os_reduce_motion {
                    self.apply_settings();
                }
            }
            Event::LiveMeasured { w, h } => {
                self.measure = (w, h);
                if self.state == PillState::Collapsed && !self.hidden {
                    self.retarget(false, None, Mode::Resize);
                }
            }
            Event::Character { side, legs } => {
                let changed = side != self.char_side || legs != self.char_legs;
                self.char_side = side;
                self.char_legs = legs;
                if changed {
                    self.relayout_live();
                }
            }
        }
    }

    /// `handle` at an explicit monotonic time (events that arrive between two frames).
    pub fn handle_at(&mut self, now_ms: f64, ev: Event) {
        if now_ms > self.now {
            self.now = now_ms;
        }
        self.wall = self.clock.wall_ms();
        self.handle(ev);
    }

    // ---------- time ----------
    /// Earliest moment (monotonic ms) something is scheduled; the host should `tick` no later than this even when no frames are wanted.
    pub fn next_wakeup(&self) -> Option<f64> {
        [self.timers.next_deadline(), self.rx.next_deadline(), self.cards.next_deadline()].into_iter().flatten().fold(None, |a: Option<f64>, d| Some(a.map_or(d, |x| x.min(d))))
    }
    /// The frame loop wants a tick every display frame (springs / liquid moving, or waiting for the expanded OS frame).
    pub fn wants_frames(&self) -> bool {
        (self.tick_active && !self.hidden && !self.env.occluded) || self.exp_wait.is_some()
    }

    /// One display frame. `dt_ms` = time since the previous call, `now_ms` = monotonic time.
    pub fn tick(&mut self, dt_ms: f64, now_ms: f64) {
        self.now = now_ms;
        self.wall = self.clock.wall_ms();
        self.run_timers();
        // the expanded-frame wait polls once per frame
        if let Some(w) = self.exp_wait {
            if self.env.inner_w >= w.fw || self.now - w.t0 >= EXP_FRAME_WAIT_MS {
                self.exp_go();
            }
        }
        if !self.tick_active || self.hidden || self.env.occluded {
            return;
        }
        // user frame cap: skip this display frame, dt stays measured from the last executed one
        self.cap_acc += dt_ms;
        let cap = self.s.fps_cap;
        if cap > 0 && self.cap_acc < 1000.0 / cap as f64 - 1.5 {
            return;
        }
        let dt = (self.cap_acc / 1000.0).clamp(0.0, MAX_DT_S);
        self.cap_acc = 0.0;
        if !self.frame(dt) {
            self.tick_active = false;
        }
    }

    /// The registered `tick(dt)` of pill.js: springs, then the liquid edge. Returns true while something still moves.
    fn frame(&mut self, dt: f64) -> bool {
        let was_busy = self.sp.busy;
        let (s_move, settled) = self.sp.step(dt);
        if was_busy {
            self.paint_body(true);
        }
        let c = self.liq_ctx();
        let l_move = self.liq.step(dt, &c);
        if settled {
            self.finish(); // the Rust frame shrink / queued cards must not wait for the liquid to stop wobbling
        }
        s_move || l_move
    }

    fn run_timers(&mut self) {
        let now = self.now;
        // reactions and cards own their deadlines
        let busy = self.hovering || self.state != PillState::Collapsed;
        let t = self.times();
        let p = self.rx.on_time(&t, busy);
        self.apply_paint(p);
        for ev in self.cards.on_time(now, self.reduce()) {
            match ev {
                CardEvent::Expired => self.no_open_until = now + NO_OPEN_AFTER_DISMISS_MS,
                CardEvent::Ended => {
                    self.next();
                    if !self.cards.has_current() {
                        self.render_live();
                        self.cards.arm_frame_off(now);
                    }
                    self.schedule_away();
                }
                CardEvent::FrameOff => {
                    self.carding = false;
                    self.invoke("pill_cards", json!({"on": false}));
                }
            }
        }
        while let Some(k) = self.timers.take_due(now) {
            match k {
                TimerKey::Open => {
                    if self.hovering && self.state != PillState::Expanded && !self.cards.has_current() && now >= self.no_open_until {
                        self.expand(None, "hover"); // a card on screen is the thing to click: hover does not replace it
                    }
                }
                TimerKey::Leave => {
                    if !self.hovering && !self.typing {
                        self.do_collapse(false, false);
                    }
                }
                TimerKey::FocusOut => {
                    if !self.typing && !self.hovering {
                        self.do_collapse(false, false);
                    }
                }
                TimerKey::Away => {
                    if self.s.auto_hide_keep_desktop {
                        self.pending_fg = true; // 'Do not hide on the desktop': stay while the desktop is in front
                        self.invoke("pill_desktop_fg", json!({}));
                    } else {
                        self.slide_away();
                    }
                }
                TimerKey::AwaySlide => {
                    if self.away {
                        self.invoke("pill_auto_hide", json!({"on": true}));
                    }
                }
                TimerKey::Live => self.render_live(),
                TimerKey::Battery => self.poll_battery(),
                TimerKey::Weather => self.refresh_weather(),
                TimerKey::ExpFrame => self.exp_go(),
                TimerKey::Quiet => {
                    self.timers.set(TimerKey::Quiet, now + QUIET_SWEEP_MS);
                    self.quiet_sweep();
                }
            }
        }
    }

    /// Quiet hours: what was queued before the window began is dropped too (every 20 s).
    fn quiet_sweep(&mut self) {
        if !self.quiet() {
            return;
        }
        self.cards.drop_queued_where(|c| !exempt_toast(c.id.as_deref(), c.urgent));
        if self.cards.has_current() {
            let (id, urgent) = (self.cards.current_id().map(|s| s.to_string()), self.cards.current_urgent());
            if !exempt_toast(id.as_deref(), urgent) {
                self.cards.end_current();
            }
        }
        let mut ch = false;
        for id in QUIET_SRC {
            if self.lives.delete(id) {
                ch = true;
            }
            self.unreact(id);
        }
        if ch && !self.hidden {
            self.render_live();
        }
    }
}

