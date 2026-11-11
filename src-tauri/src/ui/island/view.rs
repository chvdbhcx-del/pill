//! `IslandView`: the always-resident `pill` window as a native `View` (Overlay). It owns the [`Core`] (shell model + live hub), the drawing
//! resources, the panel host and the pets; this file is the glue between the host's calls and those parts. See docs/native-island.md.
use super::chrome::{self, Btn, ChromeLayout, HeaderFx, Hover};
use super::companions::{char_side, reserve_px, Companions};
use super::core::{Core, DropStep, Out, ISLAND_EVENTS};
use super::fx::{self, LiveFx};
use super::gpu::Gpu;
use super::live::{self, Painted};
use super::panel::TabHost;
use super::shapes;
use super::sys::{self, SysClock};
use super::tabfx::Act;
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{Cx, CursorShape, DropEvent, DropPayload, DropPhase, Msg, ProximityEvent, Ticket, View, Waker};
use glasskit::canvas::{scale_about, translation, mul, Canvas};
use glasskit::geom::Rect;
use crate::ui::liquid::{Feel, Flow};
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::theme::Theme;
use pill_ui_core::live::model::{Action, Query};
use pill_ui_core::shell::clock::CharSide;
use pill_ui_core::shell::color::Theme as ShellTheme;
use pill_ui_core::shell::motion as mo;
use pill_ui_core::shell::settings::EdgeStyle;
use pill_ui_core::shell::types::{Env as ShellEnv, Event, LiveContent, Phase, TabDir, View as ShellView};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Messages the view posts to itself.
enum Own {
    /// the timer thread: something is due
    Wake,
    /// draw measured new content: tell the shell
    Remeasure,
    /// a delayed hub query
    Query(Query),
    /// the screenshot a tab asked for starts once the island has folded away
    Capture(&'static str),
}

/// Wakes the idle window at the next scheduled moment (shell timers, hub ticks). Idle = zero CPU: this thread only sleeps.
struct Alarm {
    st: Arc<(Mutex<(Option<Instant>, bool)>, Condvar)>,
}

impl Alarm {
    fn new(w: Waker) -> Alarm {
        let st = Arc::new((Mutex::new((None::<Instant>, false)), Condvar::new()));
        let s2 = st.clone();
        std::thread::Builder::new()
            .name("island-alarm".into())
            .spawn(move || {
                let (m, cv) = &*s2;
                let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if g.1 {
                        return;
                    }
                    match g.0 {
                        None => g = cv.wait(g).unwrap_or_else(|e| e.into_inner()),
                        Some(t) => {
                            let now = Instant::now();
                            if t <= now {
                                g.0 = None;
                                w.post_custom(Own::Wake);
                            } else {
                                g = cv.wait_timeout(g, t - now).unwrap_or_else(|e| e.into_inner()).0;
                            }
                        }
                    }
                }
            })
            .ok();
        Alarm { st }
    }

    fn set(&self, in_ms: Option<f64>) {
        let (m, cv) = &*self.st;
        let mut g = m.lock().unwrap_or_else(|e| e.into_inner());
        g.0 = in_ms.map(|ms| Instant::now() + Duration::from_micros((ms.max(0.0) * 1000.0) as u64));
        cv.notify_all();
    }
}

impl Drop for Alarm {
    fn drop(&mut self) {
        let (m, cv) = &*self.st;
        m.lock().unwrap_or_else(|e| e.into_inner()).1 = true;
        cv.notify_all();
    }
}

enum Pending {
    Fire,
    Battery,
    Weather,
    SettingsOpen,
    DesktopFg,
    Answer(&'static str),
    /// the answer of `kbd_state` / `vdesk_state`: handed to the core as this chip event
    Chip(&'static str),
    /// the reply typed into a phone card (phonecard.rs): an error frees the field and says so
    PhoneReply,
}

#[derive(Clone, Debug, PartialEq)]
enum Hit {
    Tab(usize),
    Btn(Btn),
    CardBtn(usize),
    RowBtn(usize),
    Pet,
    Views,
    Body,
    None,
}

pub struct IslandView {
    pub core: Core,
    gpu: Gpu,
    host: TabHost,
    comp: Companions,
    alarm: Alarm,
    pending: HashMap<u64, Pending>,
    win: (f32, f32),
    env: Option<ShellEnv>,
    light_checked: f64,
    os_light: bool,
    live_a: f32,
    panel_a: f32,
    flare_a: f32,
    painted: Painted,
    chrome: ChromeLayout,
    panel_origin: (f32, f32, f32),
    pet_rect: Option<Rect>,
    hover: Hover,
    hover_card_btn: Option<usize>,
    pointer: Option<(f32, f32)>,
    measured: (f64, f64),
    pending_measure: Option<(f64, f64)>,
    want_tab: Option<String>,
    /// the meter sample last handed to the Controls tab
    sent_meters: Option<pill_ui_core::live::sysmon::MetersView>,
    last_hit: Vec<Rect>,
    last_pass: Option<bool>,
    last_gaze: Option<bool>,
    last_theme_light: Option<bool>,
    last_style: Option<(String, EdgeStyle)>,
    now: f64,
    started: bool,
    device_id: usize,
    /// the media row's equaliser: spectrum slices as last delivered / as drawn (eased in `step`), and when the last frame arrived
    eq_target: [f32; 4],
    eq: [f32; 4],
    eq_seen: f64,
    /// last time something moved for real (a transition, input): ambient frames run slower only after `CALM_AFTER_MS` of calm
    busy_at: f64,
    /// the frame cap currently asked of the host
    cap: Option<u32>,
    /// the display rate seen from the real frame intervals (the rate the host reports can be wrong): the shortest interval so far
    fast_hz: f32,
    /// the glow while an alert waits and the download ring next to the clock (ambient.rs)
    amb: super::ambient::Ambient,
    /// one of them is only breathing / turning: frames may run at a calm rate
    amb_slow: bool,
    /// the open tab asks for frames only for an ambient animation (the weather sky): paced like the other ambient frames
    tab_amb: bool,
    /// springs behind `live_a` / `panel_a` / `flare_a`: they keep their velocity when the target flips half-way
    live_sp: Flow,
    panel_sp: Flow,
    flare_sp: Flow,
    /// the flows of the live strip (swap, card, digits, dots, pulse, body colour) and of the header (tab highlights, hover, badges)
    fx: LiveFx,
    hdr: HeaderFx,
    /// the phone notification card: hover growth, reply field (phonecard.rs)
    phone: super::phonecard::PhoneUi,
}

/// The panel content fades in on this feel, after a short head start of the body; it leaves a little quicker.
const PANEL_IN: Feel = Feel::FADE.within(200.0);
const PANEL_OUT: Feel = Feel::FADE.within(120.0);
const PANEL_DELAY_MS: f64 = 40.0;

/// How long the full frame rate stays after the last real movement.
const CALM_AFTER_MS: f64 = 600.0;

impl IslandView {
    /// Built on the host thread. Reads the settings, subscribes to the island's events and starts the model.
    pub fn new(cx: &mut Cx) -> IslandView {
        Self::with_parts(cx, SysClock::real(), &sys::os_lang())
    }

    pub fn with_parts(cx: &mut Cx, clock: SysClock, os_lang: &str) -> IslandView {
        for e in ISLAND_EVENTS {
            if *e == "audio:bands" {
                cx.subscribe_latest(e);
            } else {
                cx.subscribe(e);
            }
        }
        let mut core = Core::new(os_lang, clock);
        let settings = cx.call("settings_get", Value::Null).ok().filter(|v| v.is_object()).unwrap_or_else(|| json!({}));
        let outs = core.start(settings, cx.now_ms);
        let mut v = IslandView {
            core,
            gpu: Gpu::new(),
            host: TabHost::new(),
            comp: Companions::new(),
            alarm: Alarm::new(cx.waker()),
            pending: HashMap::new(),
            win: (720.0, 120.0),
            env: None,
            light_checked: f64::NEG_INFINITY,
            os_light: false,
            live_a: 1.0,
            panel_a: 0.0,
            flare_a: 1.0,
            painted: Painted::default(),
            chrome: ChromeLayout::default(),
            panel_origin: (0.0, 0.0, 1.0),
            pet_rect: None,
            hover: Hover::default(),
            hover_card_btn: None,
            pointer: None,
            measured: (0.0, 0.0),
            pending_measure: None,
            want_tab: None,
            sent_meters: None,
            last_hit: Vec::new(),
            last_pass: None,
            last_gaze: None,
            last_theme_light: None,
            last_style: None,
            eq_target: [0.0; 4],
            eq: [0.0; 4],
            eq_seen: f64::NEG_INFINITY,
            busy_at: cx.now_ms,
            cap: None,
            fast_hz: 0.0,
            amb: Default::default(),
            amb_slow: false,
            tab_amb: false,
            live_sp: Flow::new(1.0, Feel::FADE.within(mo::LIVE_FADE_MS as f32)),
            panel_sp: Flow::new(0.0, PANEL_OUT),
            flare_sp: Flow::new(1.0, Feel::FADE.within(200.0)),
            fx: LiveFx::new(),
            hdr: HeaderFx::new(),
            phone: super::phonecard::PhoneUi::new(&Theme::pill_dark()),
            now: cx.now_ms,
            started: false,
            device_id: 0,
        };
        v.sync_env(cx);
        v.exec_outs(outs, cx);
        v.ask_chips(cx);
        v.apply_settings_side(cx);
        // the tabs with background duties exist from the start (docs/native-tabs-a.md section 3)
        v.host.warm(&["home", "shelf", "notifs", "clipboard"], cx);
        v.exec(cx);
        v.started = true;
        v
    }

    // ------------------------------------------------------------------------------------------------ plumbing

    fn light(&self) -> bool {
        self.core.shell.settings().resolved_theme(self.os_light) == ShellTheme::Light
    }

    fn sync_env(&mut self, cx: &mut Cx) {
        if cx.now_ms - self.light_checked > 2000.0 {
            self.light_checked = cx.now_ms;
            self.os_light = sys::os_prefers_light();
        }
        let e = ShellEnv { inner_w: self.win.0 as f64, dpr: cx.scale as f64, os_prefers_light: self.os_light, os_reduce_motion: cx.reduced_motion, occluded: false };
        if self.env != Some(e) {
            self.env = Some(e);
            self.core.set_env(e, cx.now_ms);
        }
    }

    /// A chip beside the time that is switched on but has no value yet (the backend announces changes only): ask for it once.
    fn ask_chips(&mut self, cx: &mut Cx) {
        let on = |k: &str| self.core.settings.get(k).is_some_and(|v| *v == Value::Bool(true));
        let desk = self.core.settings.get("live.virtualDesktop").and_then(|v| v.as_str()).is_some_and(|s| s != "off");
        if on("live.keyboardLayoutAlways") && !self.core.chips.has_layout() {
            self.call(cx, "kbd_state", Value::Null, Pending::Chip("kbd:now"));
        }
        if desk && !self.core.chips.has_desktop() {
            self.call(cx, "vdesk_state", Value::Null, Pending::Chip("vdesk:now"));
        }
    }

    /// Pets / characters / reduced motion follow the settings.
    fn apply_settings_side(&mut self, cx: &mut Cx) {
        let style = self.core.shell.settings().character;
        let changed = self.comp.apply(&self.core.settings, style);
        let cfg = self.comp.cfg.clone();
        if changed || self.last_style.is_none() {
            self.core.shell.reserve(reserve_px(&cfg));
            self.core.shell.reserve_height(super::pet_extras::capsule_min_h(&cfg));
            self.core.shell_event(Event::Character { side: char_side(&cfg), legs: style == EdgeStyle::Legs }, cx.now_ms);
        }
        self.last_style = Some((cfg.id, style));
        self.comp.set_reduced(cx.reduced_motion);
        self.comp.set_theme(self.light());
        if cx.window.0 != 0 {
            if let Some(b) = self.core.settings.get("reduceMotion").and_then(|v| v.as_bool()) {
                crate::ui::host::set_reduce_motion_setting(b);
            }
        }
    }

    fn call(&mut self, cx: &mut Cx, cmd: &str, args: Value, p: Pending) {
        let t: Ticket = cx.call_async(cmd, args);
        if !matches!(p, Pending::Fire) {
            self.pending.insert(t.0, p);
        }
    }

    /// Executes the commands the shell queued and the outs of the core.
    fn exec(&mut self, cx: &mut Cx) {
        let _ = self.drain_fx(cx);
        for _ in 0..8 {
            let cmds = self.core.take_commands();
            let outs = self.core.take_outs();
            if cmds.is_empty() && outs.is_empty() {
                break;
            }
            for (cmd, args) in cmds {
                self.run_cmd(cx, &cmd, args);
            }
            self.exec_outs(outs, cx);
        }
        self.arm_alarm(cx.now_ms);
    }

    /// Next moment something is due while the window idles: shell timers, the hub tick, the pet's idle behaviour (blink / glance / twitch).
    fn arm_alarm(&mut self, now: f64) {
        let wake = self.core.shell.next_wakeup().map(|d| (d - now).max(0.0));
        let hub = self.core.tick_hub(now).map(|ms| ms as f64);
        let pet = self.comp.wake_in(now);
        self.alarm.set([wake, hub, pet].into_iter().flatten().reduce(f64::min));
    }

    /// What the tab views asked of the island since the last call: the B tabs through the shared sink, the A tabs through `Msg::Custom` (handled
    /// in `message`); cards shown meanwhile are recorded by the notifications tab.
    fn drain_fx(&mut self, cx: &mut Cx) -> bool {
        let mut any = false;
        for _ in 0..4 {
            let fx = crate::ui::tabs::b::take_fx();
            let recs = self.core.take_toast_recs();
            if fx.is_empty() && recs.is_empty() {
                break;
            }
            any = true;
            for f in fx {
                self.apply_tab_fx(f, cx);
            }
            for r in recs {
                self.host.custom_to("notifs", Box::new(r), cx);
            }
        }
        // the system monitor's meters belong to the Controls tab (CPU / RAM / GPU over the tiles)
        if self.core.sys_meters != self.sent_meters && self.host.exists("controls") {
            if let Some(m) = self.core.sys_meters.clone() {
                self.sent_meters = Some(m.clone());
                self.host.custom_to("controls", Box::new(crate::ui::tabs::a::Meters(m)), cx);
                any = true;
            }
        }
        any
    }

    fn apply_tab_fx(&mut self, fx: TabFx, cx: &mut Cx) {
        for a in self.core.tab_fx(fx, cx.now_ms) {
            match a {
                Act::Call { cmd, args } => self.run_cmd(cx, cmd, args),
                Act::Capture { mode } => {
                    if mode == "record_screen" && self.core.rec_tile.is_some() {
                        // a running recording is stopped by the same tile, the island stays
                        self.call(cx, "capture_start", json!({ "mode": mode }), Pending::Fire);
                    } else {
                        self.core.shell.collapse("");
                        let w = cx.waker();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(450));
                            w.post_custom(Own::Capture(mode));
                        });
                    }
                }
                Act::OpenAi(id) => self.host.open_overlay("ai_actions", json!({ "id": id }), cx),
                Act::CloseAi => self.host.close_overlay(cx),
                Act::AiClosed => self.host.overlay_closed(),
            }
        }
        self.core.pump_notices();
    }

    /// What the phone card asked for (phonecard.rs): keep it on screen, take / give back the keyboard, send the reply.
    fn apply_phone_outs(&mut self, cx: &mut Cx) {
        use super::phonecard::PhoneOut;
        for o in self.phone.take_outs() {
            match o {
                PhoneOut::Ttl(ms) => self.core.shell_event(Event::CardTtl(ms), cx.now_ms),
                PhoneOut::Focus(on) => self.core.shell_event(Event::TextFocus(on), cx.now_ms),
                PhoneOut::Reply { key, text } => self.call(cx, "phone_notif_action", json!({ "key": key, "action": "reply", "text": text }), Pending::PhoneReply),
            }
        }
    }

    fn run_cmd(&mut self, cx: &mut Cx, cmd: &str, args: Value) {
        match cmd {
            // keyboard focus is the window's: the native Overlay is no-activate until asked
            "pill_activate" => cx.set_focus(args.get("on").and_then(|v| v.as_bool()).unwrap_or(false)),
            "pill_rect" => {
                self.call(cx, "pill_rect", args, Pending::Fire);
            }
            "battery_state" => self.call(cx, cmd, args, Pending::Battery),
            "weather_get" => self.call(cx, cmd, args, Pending::Weather),
            "settings_open" => self.call(cx, cmd, args, Pending::SettingsOpen),
            "pill_desktop_fg" => self.call(cx, cmd, args, Pending::DesktopFg),
            _ => self.call(cx, cmd, args, Pending::Fire),
        }
    }

    fn run_action(&mut self, cx: &mut Cx, a: Action) {
        match a {
            Action::Noop => {}
            Action::Invoke { cmd, args } => self.call(cx, &cmd, args, Pending::Fire),
            Action::CopyText(t) => cx.set_clipboard_text(&t),
            Action::TimerAgain => {
                let secs = self.core.settings.get("timer.last").and_then(|v| v.as_i64()).unwrap_or(300);
                self.call(cx, "timer_start", json!({ "seconds": secs }), Pending::Fire);
            }
        }
    }

    fn exec_outs(&mut self, outs: Vec<Out>, cx: &mut Cx) {
        for o in outs {
            match o {
                Out::Query(q) => {
                    if q.delay_ms == 0 {
                        self.call(cx, q.cmd, q.args.clone(), Pending::Answer(q.as_event));
                    } else {
                        let (w, ms) = (cx.waker(), q.delay_ms);
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(ms as u64));
                            w.post_custom(Own::Query(q));
                        });
                    }
                }
                Out::Run(a) => self.run_action(cx, a),
                Out::TabShown(id) => self.want_tab = Some(id),
                Out::TabHidden(id) => {
                    if self.host.overlay_open {
                        self.host.close_overlay(cx);
                    }
                    self.host.hide(&id, cx)
                }
                Out::Broadcast { name, payload } => match self.amb.take(&name, &payload) {
                    super::ambient::Taken::No => {
                        self.host.event_msg(&name, &payload, false, cx);
                    }
                    taken => {
                        if let super::ambient::Taken::RingSlot(on) = taken {
                            use pill_ui_core::shell::clock::{ClockSlots, SlotItem};
                            self.core.shell.set_clock_slots(ClockSlots { left: None, right: on.then_some(SlotItem::Ring) });
                        }
                        cx.request_frame();
                    }
                },
                Out::Blur => cx.set_focus(false),
                Out::Dictate => self.call(cx, "voice_toggle", Value::Null, Pending::Fire),
            }
        }
    }

    // ------------------------------------------------------------------------------------------------ geometry / hit

    fn hit_region(&self, v: &ShellView) -> Vec<Rect> {
        let s = v.scale as f32;
        let b = &v.body;
        if v.phase == Phase::AutoHidden || b.away {
            return vec![Rect::new(b.left as f32, 0.0, b.width as f32, 2.0)];
        }
        let mut r = vec![Rect::new(b.left as f32, b.top as f32, b.width as f32, b.height as f32 + 14.0 * s)];
        if v.panel.visible {
            r[0].h = b.height as f32 + 14.0 * s;
        }
        r
    }

    fn update_window_requests(&mut self, v: &ShellView, cx: &mut Cx) {
        let hit = self.hit_region(v);
        if hit != self.last_hit {
            cx.set_hit_region(hit.clone());
            self.last_hit = hit;
        }
        let pass = self.core.shell.settings().pass_through && v.phase != Phase::Expanded && !v.panel.visible;
        if self.last_pass != Some(pass) {
            self.last_pass = Some(pass);
            cx.set_pass_through(pass);
        }
        let gaze = self.comp.wants_gaze() && !matches!(v.phase, Phase::Hidden);
        if self.last_gaze != Some(gaze) {
            self.last_gaze = Some(gaze);
            cx.set_gaze(gaze);
        }
    }

    fn locate(&self, p: (f32, f32)) -> Hit {
        let (ox, oy, s) = self.panel_origin;
        if self.panel_a > 0.05 {
            let l = ((p.0 - ox) / s, (p.1 - oy) / s);
            for (i, t) in self.chrome.tabs.iter().enumerate() {
                if t.rect.contains(l.0, l.1) {
                    return Hit::Tab(i);
                }
            }
            for (b, r) in &self.chrome.buttons {
                if r.contains(l.0, l.1) {
                    return Hit::Btn(*b);
                }
            }
            if self.chrome.vpn.is_some_and(|r| r.contains(l.0, l.1)) {
                return Hit::Btn(Btn::Vpn); // the VPN chip switches the VPN
            }
            if self.chrome.views.contains(l.0, l.1) {
                return Hit::Views;
            }
        }
        if let Some(r) = self.pet_rect {
            if r.contains(p.0, p.1) {
                return Hit::Pet;
            }
        }
        for (i, r) in self.painted.card_buttons.iter().enumerate() {
            if r.contains(self.local_live(p).0, self.local_live(p).1) {
                return Hit::CardBtn(i);
            }
        }
        for (r, idx) in &self.painted.row_hits {
            if r.contains(self.local_live(p).0, self.local_live(p).1) {
                return Hit::RowBtn(*idx);
            }
        }
        Hit::Body
    }

    /// window px -> the live row's local css px at 100 % (origin = the body's top-left)
    fn local_live(&self, p: (f32, f32)) -> (f32, f32) {
        let v = self.core.shell.view();
        let s = v.scale as f32;
        ((p.0 - v.body.left as f32) / s, (p.1 - v.body.top as f32) / s)
    }

    fn views_event(&self, ev: &InputEvent) -> InputEvent {
        let (ox, oy, s) = self.panel_origin;
        let v = self.chrome.views;
        // window px -> panel css px -> the view's own origin; the position is also divided by the island size
        let mut e = ev.clone();
        match &mut e {
            InputEvent::MouseMove { pos, .. } | InputEvent::MouseDown { pos, .. } | InputEvent::MouseUp { pos, .. } | InputEvent::Wheel { pos, .. } => {
                *pos = ((pos.0 - ox) / s - v.x, (pos.1 - oy) / s - v.y);
            }
            _ => {}
        }
        e
    }

    // ------------------------------------------------------------------------------------------------ drawing

    fn theme(&self) -> Theme {
        let base = if self.light() { Theme::pill_light() } else { Theme::pill_dark() };
        match self.core.shell.settings().island_color() {
            Some(c) => super::tint::tinted(base, c),
            None => base,
        }
    }
}

fn cursor_of(h: &Hit) -> Cursor {
    match h {
        Hit::Tab(_) | Hit::Btn(_) | Hit::CardBtn(_) | Hit::RowBtn(_) | Hit::Pet => Cursor::Hand,
        _ => Cursor::Arrow,
    }
}

/// Audio frames older than this (ms) no longer drive the media row's equaliser.
const EQ_FRESH_MS: f64 = 600.0;

/// The four equaliser bars from the spectrum: bass, low-mid, high-mid and treble slices, each averaged and lifted a little (0..1).
fn eq_levels(bands: &[f64]) -> [f32; 4] {
    let n = bands.len();
    let mut out = [0.0f32; 4];
    for (i, o) in out.iter_mut().enumerate() {
        // slice edges at 0, 5/32, 12/32, 20/32, 1 of the band count: the low end is narrow because the low bands carry the beat
        let edge = |k: usize| n * [0, 5, 12, 20, 32][k] / 32;
        let s = &bands[edge(i)..edge(i + 1).max(edge(i) + 1).min(n)];
        if !s.is_empty() {
            *o = ((s.iter().sum::<f64>() / s.len() as f64) as f32 * 1.25).clamp(0.0, 1.0);
        }
    }
    out
}

/// One step of a level towards its goal: quick attack, slower release, so the bars snap to a hit and fall back smoothly.
fn ease_level(cur: f32, goal: f32, dt_ms: f32) -> f32 {
    let tau = if goal > cur { 45.0 } else { 130.0 };
    cur + (goal - cur) * (1.0 - (-dt_ms / tau).exp())
}

/// What the island's live strip says, as plain text for a screen reader: the clock's time, a card's title and subtitle, or the visible
/// text of a slot row (every `text` / `title` / `sub` / `label` string of its spec, in order, without repeats).
fn spoken(content: &LiveContent) -> String {
    fn walk(v: &Value, out: &mut Vec<String>, depth: u8) {
        if depth > 5 {
            return;
        }
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    match (k.as_str(), x) {
                        ("text" | "title" | "sub" | "label", Value::String(s)) if !s.trim().is_empty() => {
                            if !out.iter().any(|o| o == s.trim()) {
                                out.push(s.trim().to_string());
                            }
                        }
                        _ => walk(x, out, depth + 1),
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out, depth + 1)),
            _ => {}
        }
    }
    match content {
        LiveContent::None => String::new(),
        LiveContent::Clock { time, .. } => time.clone().unwrap_or_default(),
        LiveContent::Card(c) => [c.spec.title.as_str(), c.spec.sub.as_str(), if c.spec.title.is_empty() { c.spec.html.as_str() } else { "" }].iter().filter(|s| !s.trim().is_empty()).map(|s| s.trim()).collect::<Vec<_>>().join(", "),
        LiveContent::Slot { spec, .. } => {
            let mut out = Vec::new();
            walk(spec, &mut out, 0);
            out.join(", ")
        }
    }
}

impl View for IslandView {
    fn accessible(&self) -> Option<String> {
        Some(spoken(&self.core.shell.view().live.content))
    }

    fn on_dpi(&mut self, _scale: f32, cx: &mut Cx) {
        self.sync_env(cx);
        self.exec(cx);
        cx.request_frame();
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.now = cx.now_ms;
        self.sync_env(cx);
        let mut redraw = true;
        match msg {
            Msg::Event { name, payload, latest } => {
                if name == "audio:bands" {
                    if let Some(b) = payload.get("b").and_then(|b| b.as_array()) {
                        let bands: Vec<f64> = b.iter().filter_map(|x| x.as_f64()).collect();
                        self.comp.feed_audio(&bands, cx.now_ms);
                        self.eq_target = eq_levels(&bands);
                        self.eq_seen = cx.now_ms;
                    }
                }
                self.core.bus_event(&name, &payload, cx.now_ms);
                if matches!(name.as_str(), "settings:changed" | "pill:show") {
                    self.apply_settings_side(cx);
                }
                if name == "settings:changed" {
                    self.ask_chips(cx);
                }
                self.host.event_msg(&name, &payload, latest, cx);
            }
            Msg::Reply { ticket, result } => match self.pending.remove(&ticket.0) {
                Some(Pending::Battery) => self.core.shell_event(Event::BatteryState(result.unwrap_or(Value::Null)), cx.now_ms),
                Some(Pending::Weather) => self.core.shell_event(Event::WeatherGet(result.unwrap_or(Value::Null)), cx.now_ms),
                Some(Pending::SettingsOpen) => self.core.shell_event(Event::SettingsOpened(result.is_ok()), cx.now_ms),
                Some(Pending::DesktopFg) => self.core.shell_event(Event::DesktopFg(result.ok().and_then(|v| v.as_bool()).unwrap_or(false)), cx.now_ms),
                Some(Pending::Answer(ev)) => self.core.dispatch_answer(ev, &result.unwrap_or(Value::Null), cx.now_ms),
                Some(Pending::Chip(ev)) => {
                    if let Some(v) = result.ok().filter(|v| v.is_object()) {
                        self.core.bus_event(ev, &v, cx.now_ms);
                    }
                }
                Some(Pending::PhoneReply) => {
                    if let Err(e) = result {
                        self.phone.reply_failed();
                        let ru = self.core.ru();
                        let (t, tone) = (if ru { "Не удалось ответить" } else { "Reply failed" }, pill_ui_core::live::model::Tone::Amber);
                        self.core.show_card(pill_ui_core::live::model::CardSpec::text("phone-reply-failed", 4000, t, Some(e)).icon(pill_ui_core::live::model::IconName::Warn).tone(tone));
                    }
                }
                Some(Pending::Fire) | None => {
                    redraw = self.host.other_msg(Msg::Reply { ticket, result }, cx);
                }
            },
            Msg::Custom(b) => match b.downcast::<TabFx>() {
                Ok(fx) => self.apply_tab_fx(*fx, cx),
                Err(b) => match b.downcast::<Own>() {
                Ok(o) => match *o {
                    Own::Capture(mode) => self.call(cx, "capture_start", json!({ "mode": mode }), Pending::Fire),
                    Own::Wake => self.core.tick(0.0, cx.now_ms),
                    Own::Remeasure => {
                        if let Some((w, h)) = self.pending_measure.take() {
                            self.core.shell_event(Event::LiveMeasured { w, h }, cx.now_ms);
                        }
                    }
                    Own::Query(q) => self.call(cx, q.cmd, q.args.clone(), Pending::Answer(q.as_event)),
                },
                Err(other) => redraw = self.host.other_msg(Msg::Custom(other), cx),
                },
            },
            m @ Msg::Open(_) => redraw = self.host.other_msg(m, cx),
        }
        self.exec(cx);
        if redraw {
            Response::redraw()
        } else {
            Response::NONE
        }
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        let now = cx.now_ms;
        if (2.0..9.5).contains(&dt_ms) {
            self.fast_hz = self.fast_hz.max((1000.0 / dt_ms) as f32); // vsync frames of a fast display
        }
        self.now = now;
        self.sync_env(cx);
        if let Some((w, h)) = self.pending_measure.take() {
            self.core.shell_event(Event::LiveMeasured { w, h }, now);
        }
        self.core.tick(dt_ms, now);
        self.exec(cx);
        let v = self.core.shell.view();
        let reduced = v.reduced;
        self.live_a = self.live_sp.chase(if v.live.visible { 1.0 } else { 0.0 }, dt_ms);
        // the panel's content follows the body: it fades in a moment after the body starts to grow, and out quickly
        let want_panel = v.panel.visible && (reduced || v.panel.shown_age_ms >= PANEL_DELAY_MS);
        let feel = if v.panel.visible { PANEL_IN } else { PANEL_OUT };
        if self.panel_sp.feel() != feel {
            self.panel_sp.set_feel(feel);
        }
        if reduced {
            self.panel_sp.jump(if want_panel { 1.0 } else { 0.0 });
        }
        self.panel_a = self.panel_sp.chase(if want_panel { 1.0 } else { 0.0 }, dt_ms);
        self.flare_a = self.flare_sp.chase(if v.body.flare_visible { 1.0 } else { 0.0 }, dt_ms);
        // equaliser bars follow the music; with no audio frames for a while they ease down
        let goal = if now - self.eq_seen < EQ_FRESH_MS { self.eq_target } else { [0.0; 4] };
        for (cur, g) in self.eq.iter_mut().zip(goal) {
            *cur = ease_level(*cur, g, dt_ms as f32);
        }
        let hover_tab = self.hover.tab.and_then(|i| self.chrome.tabs.get(i)).map(|t| t.id.clone());
        let hdr_busy = self.hdr.step(&v, hover_tab.as_deref(), self.hover.btn, self.hover.pressed, dt_ms);
        let inp = fx::Inputs { rows: Some(&self.core.rows), hover_btn: self.hover_card_btn, privacy: self.core.privacy, pulse: self.core.pulse, pad: self.content_pad() };
        let fx_busy = self.fx.step(&v, inp, dt_ms);
        let phone_busy = {
            let cur = self.core.shell.cards().current_id().and_then(|id| self.core.cards.get(id));
            self.phone.step(cur, dt_ms, now, reduced)
        };
        self.apply_phone_outs(cx);
        let (amb_busy, amb_loop) = self.amb.step(v.reduced, dt_ms);
        self.amb_slow = amb_loop;
        // the tab view follows the shell's current tab while the panel is open
        if v.panel.visible {
            if let Some(id) = v.current_tab.clone() {
                if self.host.cur.as_deref() != Some(&id) {
                    self.host.show(&id, v.tab_dir, self.core.ru(), cx, now);
                }
            }
        }
        self.want_tab = None;
        // pets and characters
        let (name, color) = self.core.reaction.clone();
        self.comp.set_reaction(&name, &color);
        self.comp.set_reduced(reduced);
        self.comp.set_theme(self.light());
        let pets_busy = self.comp.tick(now);
        self.arm_alarm(now);
        self.update_window_requests(&v, cx);
        let host_busy = self.host.step(dt_ms, now, cx);
        // a tab whose only motion is ambient (the weather sky) does not hold the window at the full rate
        self.tab_amb = self.host.ambient();
        if self.drain_fx(cx) {
            self.exec(cx);
            cx.request_frame();
        }
        let rows_busy = self.loops(&v);
        let fades = self.live_sp.moving() || self.flare_sp.moving() || self.panel_sp.moving() || (v.panel.visible && v.panel.shown_age_ms < PANEL_DELAY_MS);
        crate::ui::host::perf::why(&[
            ("shell", v.wants_frames),
            ("measure", self.pending_measure.is_some()),
            ("pets", pets_busy),
            ("host", host_busy),
            ("header", hdr_busy),
            ("rows", rows_busy),
            ("fx", fx_busy),
            ("phone", phone_busy),
            ("glide", self.gpu.gliding()),
            ("fades", fades),
        ]);
        self.pace_ambient(&v, cx, now, hdr_busy || host_busy || fx_busy || phone_busy || fades || amb_busy);
        v.wants_frames || phone_busy || self.pending_measure.is_some() || pets_busy || host_busy || hdr_busy || rows_busy || fx_busy || self.gpu.gliding() || fades || amb_busy || amb_loop || self.tab_amb
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.now = cx.now_ms;
        self.wake_full_rate(cx);
        // a phone card on screen: its reply field and arrow come first, the rest (hover, buttons, dismissal) stays with the shell
        let pr = if self.phone.active() {
            let local = ev.pos().map(|p| self.local_live(p));
            let r = self.phone.event(ev, local);
            self.apply_phone_outs(cx);
            if r.consumed {
                self.exec(cx);
                return r;
            }
            r
        } else {
            Response::NONE
        };
        let mut resp = Response::NONE;
        match ev {
            InputEvent::MouseMove { pos, .. } => {
                self.pointer = Some(*pos);
                let h = self.locate(*pos);
                let nh = Hover { tab: if let Hit::Tab(i) = h { Some(i) } else { None }, btn: if let Hit::Btn(b) = h { Some(b) } else { None }, pressed: self.hover.pressed };
                let hc = if let Hit::CardBtn(i) = h { Some(i) } else { None };
                if nh != self.hover || hc != self.hover_card_btn {
                    self.hover = nh;
                    self.hover_card_btn = hc;
                    resp = Response::redraw();
                }
                self.core.shell_event(Event::Cursor(Some((pos.0 as f64, pos.1 as f64))), cx.now_ms);
                resp = resp.with_cursor(cursor_of(&h));
                if h == Hit::Views {
                    resp = resp.merge(self.host.event(&self.views_event(ev), cx));
                }
            }
            InputEvent::MouseLeave => {
                self.phone.leave();
                self.pointer = None;
                self.hover = Hover::default();
                self.hover_card_btn = None;
                self.core.shell_event(Event::MouseLeave, cx.now_ms);
                resp = Response::redraw();
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => {
                let h = self.locate(*pos);
                self.hover.pressed = true;
                resp = Response::redraw();
                match h {
                    Hit::Tab(i) => {
                        let id = self.chrome.tabs[i].id.clone();
                        self.core.shell_event(Event::TabClicked(id), cx.now_ms);
                    }
                    Hit::Btn(Btn::Dict) => self.core.shell_event(Event::DictButton, cx.now_ms),
                    Hit::Btn(Btn::Pin) => {
                        let pinned = !self.core.shell.view().chrome.pinned;
                        self.core.shell_event(Event::PinButton, cx.now_ms);
                        self.call(cx, "settings_set", json!({ "patch": { "pinned": pinned } }), Pending::Fire);
                    }
                    Hit::Btn(Btn::Settings) => self.core.shell_event(Event::SettingsButton, cx.now_ms),
                    Hit::Btn(Btn::Vpn) => self.call(cx, "vpn_toggle", serde_json::Value::Null, Pending::Fire),
                    Hit::CardBtn(i) => self.core.shell_event(Event::CardAction(i), cx.now_ms),
                    Hit::RowBtn(idx) => {
                        let act = self.painted.row_id.as_ref().and_then(|id| self.core.rows.get(id)).and_then(|r| match r.elements.get(idx) {
                            Some(pill_ui_core::live::model::Elem::Button { action, .. }) => Some(action.clone()),
                            _ => None,
                        });
                        if let Some(a) = act {
                            self.run_action(cx, a);
                        }
                    }
                    Hit::Pet => self.comp.poke(),
                    Hit::Views => resp = resp.merge(self.host.event(&self.views_event(ev), cx)),
                    Hit::Body | Hit::None => {
                        // a click on a card (outside its buttons) runs its first action
                        let id = self.core.shell.cards().current_id().map(String::from);
                        if let Some(c) = id.and_then(|i| self.core.cards.get(&i)) {
                            if c.click_first && !c.actions.is_empty() && self.painted.card_rect.is_some_and(|r| r.contains(self.local_live(*pos).0, self.local_live(*pos).1)) {
                                self.core.shell_event(Event::CardAction(0), cx.now_ms);
                            }
                        }
                    }
                }
            }
            InputEvent::MouseUp { pos, .. } => {
                self.hover.pressed = false;
                resp = Response::redraw();
                if self.locate(*pos) == Hit::Views || self.panel_a > 0.05 {
                    resp = resp.merge(self.host.event(&self.views_event(ev), cx));
                }
            }
            InputEvent::Key { code: KeyCode::Escape, down: true, .. } => {
                let r = self.host.event(ev, cx);
                if !r.consumed {
                    self.core.shell_event(Event::Escape, cx.now_ms);
                }
                resp = r.merge(Response::redraw());
            }
            InputEvent::Wheel { pos, .. } => {
                if self.locate(*pos) == Hit::Views {
                    resp = self.host.event(&self.views_event(ev), cx);
                }
            }
            InputEvent::Focus(false) => {
                self.core.shell_event(Event::TextFocus(false), cx.now_ms);
                resp = self.host.event(ev, cx);
            }
            _ => resp = self.host.event(ev, cx),
        }
        self.exec(cx);
        pr.merge(resp)
    }

    fn on_proximity(&mut self, ev: &ProximityEvent, cx: &mut Cx) -> Response {
        let now = cx.now_ms;
        if !matches!(ev, ProximityEvent::Gaze(_)) {
            self.wake_full_rate(cx); // the pet's gaze is ambient; hover / leave / zone start real motion
        }
        match ev {
            ProximityEvent::Hover => self.core.shell_event(Event::Hover, now),
            ProximityEvent::Leave => self.core.shell_event(Event::Leave, now),
            ProximityEvent::Zone(p) => self.core.shell_event(Event::Cursor(p.map(|p| (p.0 as f64, p.1 as f64))), now),
            ProximityEvent::Gaze(p) => {
                let s = cx.scale.max(0.1);
                self.comp.gaze(Some((p.0 / s, p.1 / s)));
            }
            ProximityEvent::TopEdge => {
                if self.core.shell.is_away() {
                    self.call(cx, "pill_auto_hide", json!({ "on": false }), Pending::Fire);
                }
            }
        }
        self.exec(cx);
        Response::redraw()
    }

    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        self.wake_full_rate(cx);
        let now = cx.now_ms;
        let files = matches!(ev.payload, Some(DropPayload::Files(_)));
        let open = self.core.shell.view().panel.visible;
        // panel open on Home / Shelf: the tab decides (Home: launcher strip first, Shelf: the whole tab); what it does not take falls through
        if open && self.panel_a > 0.05 && matches!(self.host.cur.as_deref(), Some("home") | Some("shelf") | Some("phone")) && (files || ev.phase == DropPhase::Leave) {
            let (ox, oy, s) = self.panel_origin;
            let v = self.chrome.views;
            let mut local = ev.clone();
            local.pos = ((ev.pos.0 - ox) / s - v.x, (ev.pos.1 - oy) / s - v.y);
            let r = self.host.on_drop(&local, cx);
            self.exec(cx);
            if r.consumed || ev.phase == DropPhase::Leave {
                return Response::redraw().merge(r);
            }
        }
        match ev.phase {
            DropPhase::Enter if files => {
                if !open {
                    self.core.drop_phase(DropStep::Enter, now);
                    // the shelf's glow on the edge while files hang over the folded island (Pill.setDragPull)
                    self.core.shell.set_drag_pull(true);
                }
                self.exec(cx);
                return Response::consumed();
            }
            DropPhase::Over => return Response::consumed(),
            DropPhase::Leave => {
                self.core.drop_phase(DropStep::Leave, now);
                self.core.shell.set_drag_pull(false);
            }
            DropPhase::Drop => {
                self.core.shell.set_drag_pull(false);
                if let Some(DropPayload::Files(f)) = &ev.payload {
                    let paths: Vec<String> = f.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                    self.core.drop_phase(DropStep::Landed, now);
                    self.call(cx, "shelf_add", json!({ "paths": paths }), Pending::Fire);
                }
            }
            _ => {}
        }
        self.exec(cx);
        Response::redraw()
    }

    fn draw(&mut self, cv: &Canvas, _th: &Theme, rect: Rect, cx: &Cx) {
        // Direct2D objects belong to one device: after a device loss every cache of this view is stale
        let dev = windows::core::Interface::as_raw(cv.dc) as usize;
        if dev != self.device_id {
            if self.device_id != 0 {
                self.gpu = Gpu::new();
                self.comp.drop_device_resources();
            }
            self.device_id = dev;
        }
        if (self.win.0 - rect.w).abs() > 0.01 || (self.win.1 - rect.h).abs() > 0.01 {
            self.win = (rect.w, rect.h);
            cx.post(Msg::Custom(Box::new(Own::Remeasure)));
            self.pending_measure = self.pending_measure.or(Some(self.measured));
        }
        let v = self.core.shell.view();
        if v.phase == Phase::Hidden {
            return;
        }
        let th = self.theme();
        let s = v.scale as f32;
        let now = cx.now_ms;
        let b = &v.body;
        let body = Rect::new(b.left as f32, b.top as f32, b.width as f32, b.height as f32);
        let (rt, rb) = shapes::clamp_radii(body.w, body.h, b.radius_top as f32, b.radius_bottom as f32);
        self.amb.glow.paint(cv, body, rb.max(rt), s);
        shapes::paint_body(cv, b, b.radius_top as f32, b.radius_bottom as f32, self.flare_a, self.fx.shadow(), self.fx.fill());
        let ctx = live::Ctx { rows: &self.core.rows, cards: &self.core.cards, reduced: v.reduced, ru: self.core.ru(), eq: (now - self.eq_seen < EQ_FRESH_MS).then_some(self.eq), fx: { let mut f = live::Fx { ring: self.amb.ring.paint(), ..self.fx.fx() }; f.card.phone_open = self.phone.open_target(); f } };
        // ---- everything inside the body is clipped to its silhouette
        let mut painted = Painted::default();
        cv.with_clip_rr(body, rb.max(rt), |cv| {
            if self.live_a > 0.0 {
                let legs_slot = if self.comp.style == EdgeStyle::Legs { 30.0f32 } else { 0.0 };
                let [pl, pr, slot_dx] = self.fx.pad();
                let local = Rect::new(pl + slot_dx, 0.0, (body.w / s - pl - pr).max(0.0), body.h / s);
                let m = mul(&scale_about((0.0, 0.0), s, s), &translation(body.x, body.y));
                cv.with_transform(m, |cv| {
                    let alpha = self.live_a;
                    if let Some(g) = &v.live.ghost {
                        let gctx = live::Ctx { fx: live::Fx { ring: ctx.fx.ring, ..self.fx.ghost_fx() }, ..ctx.clone() };
                        live::paint_content(cv, &th, &mut self.gpu, &gctx, &g.content, local, alpha * self.fx.ghost(), 1e9, now, &mut Painted::default());
                    }
                    live::paint_content(cv, &th, &mut self.gpu, &ctx, &v.live.content, local, alpha * self.fx.swap(), v.live.swap_age_ms, now, &mut painted);
                    // the phone card paints itself: avatar, texts, the second row with the reply field
                    if let (Some((r, a, sc)), LiveContent::Card(cvw)) = (painted.phone, &v.live.content) {
                        if let Some(pill_ui_core::live::model::CardBody::Phone(p)) = cvw.spec.id.as_ref().and_then(|id| self.core.cards.get(id)).map(|c| &c.body) {
                            let mut rects = Vec::new();
                            cv.with_opacity(a, |cv| cv.with_scale_about((r.cx(), r.y), sc, |cv| rects = self.phone.paint(cv, &th, &mut self.gpu, p, (r.x, r.y), now, ctx.fx.card.hover)));
                            painted.card_buttons = rects;
                        }
                    }
                });
                // the size the capsule should have (screen px incl. the island size and the legs' eye slot)
                let m = LiveMeter(cv);
                if let Some((w, h)) = live::measure(&v.live.content, &ctx, &m, &th) {
                    let (w, h) = ((w + legs_slot) as f64 * v.scale, h as f64 * v.scale);
                    if (w - self.measured.0).abs() > 0.01 || (h - self.measured.1).abs() > 0.01 {
                        self.measured = (w, h);
                        self.pending_measure = Some((w, h));
                        cx.post(Msg::Custom(Box::new(Own::Remeasure)));
                    }
                }
            }
            // pets: on the capsule floor while closed, in the header gap while open (both while one fades into the other)
            self.pet_rect = None;
            if !self.comp.cfg.id.is_empty() {
                if self.live_a > 0.0 {
                    if let Some(r) = self.comp.cap_box(body, s, super::companions::text_mid(body.y, body.h, s)) {
                        self.pet_rect = (!self.comp.pet_gone()).then_some(r);
                        self.comp.draw_pet(cv, r, self.live_a);
                    }
                }
                if self.panel_a > 0.0 && self.chrome.pet_slot.w >= 56.0 {
                    let (w, h) = crate::ui::pets::engine::box_size(&self.comp.cfg.id, self.comp.cfg.size, crate::ui::pets::engine::Place::Head, 1.0);
                    let slot = self.chrome.pet_slot;
                    let x = if self.comp.cfg.side == crate::ui::pets::engine::Side::Left { slot.x + 6.0 } else { slot.right() - 6.0 - w };
                    let (ox, oy, _) = self.panel_origin;
                    let r = Rect::new(ox + x * s, oy + (slot.bottom() - h) * s, w * s, h * s);
                    self.comp.draw_pet(cv, r, self.panel_a);
                }
            }
            // ---- the open panel
            if self.panel_a > 0.0 {
                let ox = body.x + b.panel_left as f32;
                let oy = body.y;
                self.panel_origin = (ox, oy, s);
                let ru = self.core.ru();
                let mut meter = LiveMeter(cv);
                let ph = body.h / s;
                self.chrome = self.hdr.layout(&v, ru, ph, &mut meter);
                let m = mul(&scale_about((0.0, 0.0), s, s), &translation(ox, oy));
                cv.with_transform(m, |cv| {
                    cv.with_opacity(self.panel_a, |cv| chrome::paint(cv, &th, &mut self.gpu, &self.chrome, &self.hdr, ru));
                    self.host.draw(cv, &th, self.chrome.views, cx, now, self.panel_a);
                });
            } else {
                self.chrome = ChromeLayout::default();
            }
        });
        self.painted = painted;
        // privacy dots in the top-right corner: they pop in, and the other one slides over when one leaves
        let (mic, cam) = (self.fx.dot(0), self.fx.dot(1));
        if (cam > 0.002 || mic > 0.002) && self.live_a > 0.0 {
            let breath = if v.reduced { 1.0 } else { 0.775 + 0.225 * (std::f64::consts::TAU * now / 2400.0).cos() as f32 };
            let mut x = body.right() - 11.0 * s - 6.0 * s;
            for (k, c) in [(mic, glasskit::Color::rgb8(0xff, 0x9f, 0x0a)), (cam, glasskit::Color::rgb8(0x30, 0xd1, 0x58))] {
                if k > 0.002 {
                    let a = k.min(1.0) * self.live_a;
                    cv.fill_circle((x + 3.0 * s, body.y + 10.0 * s), 5.0 * s * k, c.alpha(0.18 * a));
                    cv.fill_circle((x + 3.0 * s, body.y + 10.0 * s), 3.0 * s * k, c.fade(breath * a));
                }
                x -= 9.0 * s * k;
            }
        }
        // ---- the liquid edge and the capsule characters hang below / outside the body
        let pulse = self.fx.pulse();
        if self.comp.take_created() {
            cx.post(Msg::Custom(Box::new(Own::Wake))); // a fresh engine: tick + draw it so it starts moving
        }
        shapes::paint_edge(cv, &v.edge, (v.strip.x as f32, v.strip.y as f32), pulse, v.reduced);
        match self.comp.style {
            EdgeStyle::Tail => self.comp.draw_tail(cv, body, rb, s),
            EdgeStyle::Legs => {
                let side = char_side(&self.comp.cfg);
                // the characters' eyes are drawn about 0.75 px lower than the pet's eye line: they sit on the same text line
                let cy = super::companions::text_mid(body.y, body.h, s) - 0.75 * s;
                let cx_ = match side {
                    CharSide::Right => body.right() - 18.0 * s,
                    _ => body.x + 18.0 * s,
                };
                // the legs stand on the liquid edge: they follow its sag under them
                let lx = cx_.clamp(body.x + 14.0 * s, (body.right() - 14.0 * s).max(body.x + 14.0 * s));
                let drop = super::pet_extras::edge_offset_at(&v.edge.points, ((lx - body.cx()) / s) as f64) as f32 * s;
                self.comp.draw_legs_eyes(cv, body, (cx_, cy), s, drop);
            }
            _ => {}
        }
        let _ = ShellTheme::Dark;
    }
}

impl IslandView {
    /// The content area's side padding and the legs' eye-slot shift, as the pet / characters need them now (css px): where the flows head to.
    fn content_pad(&self) -> [f32; 3] {
        let pad = if self.comp.cfg.id.is_empty() { 0.0 } else { reserve_px(&self.comp.cfg) as f32 };
        let (pl, pr) = if self.comp.cfg.side == crate::ui::pets::engine::Side::Left { (pad, 0.0) } else { (0.0, pad) };
        let slot = if self.comp.style == EdgeStyle::Legs { 30.0f32 } else { 0.0 };
        let dx = match (slot > 0.0, char_side(&self.comp.cfg)) {
            (true, CharSide::Left) => slot / 2.0,
            (true, _) => -slot / 2.0,
            _ => 0.0,
        };
        [pl, pr, dx]
    }

    /// Ambient frames (the pet breathing, the equaliser, the liquid edge) do not need the display's full rate: on a fast screen they run at
    /// half of it. Any transition or input brings the full rate back at once and keeps it for `CALM_AFTER_MS` after the last movement.
    fn pace_ambient(&mut self, v: &ShellView, cx: &mut Cx, now: f64, busy: bool) {
        let springs = self.core.shell.spring_state().iter().any(|(x, vel, t)| vel.abs() > 0.02 || (x - t).abs() > 0.02);
        // an open panel counts as moving, unless its tab only breathes (ambient): then the panel has settled and 30 fps are plenty
        let moving = busy
            || springs
            || ((v.panel.visible || self.panel_a != 0.0) && !self.tab_amb)
            || self.pending_measure.is_some()
            || self.gpu.gliding();
        if moving {
            self.busy_at = now;
        }
        let hz = self.fast_hz.max(cx.refresh_hz);
        // a glow that breathes or a ring that turns is slow by nature: 30 frames a second are plenty, even on a 60 Hz screen
        let want = if now - self.busy_at > CALM_AFTER_MS && self.tab_amb {
            Some(30)
        } else if now - self.busy_at > CALM_AFTER_MS && hz > 100.0 {
            Some((hz / 2.0) as u32)
        } else if now - self.busy_at > CALM_AFTER_MS && self.amb_slow {
            Some(30)
        } else {
            None
        };
        crate::ui::host::perf::why(&[
            ("p.springs", springs),
            ("p.moving", moving),
            ("p.busy", busy),
            ("p.panel", v.panel.visible || self.panel_a != 0.0),
            ("p.gpu", self.gpu.gliding()),
            ("p.fast_screen", hz > 100.0),
            ("p.host_says_fast", cx.refresh_hz > 100.0),
            ("p.cap_wanted", want.is_some()),
        ]);
        if want != self.cap {
            self.cap = want;
            cx.set_fps_cap(want);
        }
    }

    /// Input: the full frame rate right now (a capped window would hold the first reaction back by up to one capped frame).
    fn wake_full_rate(&mut self, cx: &mut Cx) {
        self.busy_at = cx.now_ms;
        if self.cap.is_some() {
            self.cap = None;
            cx.set_fps_cap(None);
        }
    }

    /// Looping row animations that need frames at the display rate (equaliser, spinner, pulsing dot, ring).
    fn loops(&self, v: &ShellView) -> bool {
        use pill_ui_core::live::model::Elem;
        if v.reduced {
            return false;
        }
        fn looping(e: &Elem) -> bool {
            match e {
                Elem::Eq { playing, .. } => *playing,
                Elem::Spinner { .. } => true,
                Elem::Dot { pulse_ms, .. } => pulse_ms.is_some(),
                Elem::Ring { value, .. } => value.is_none(),
                Elem::Group(g) => g.iter().any(looping),
                _ => false,
            }
        }
        match &v.live.content {
            LiveContent::Slot { id, .. } => self.core.rows.get(id).is_some_and(|r| r.elements.iter().any(looping)),
            _ => false,
        }
    }
}

struct LiveMeter<'a, 'b>(&'a Canvas<'b>);
impl super::rows::Meter for LiveMeter<'_, '_> {
    fn w(&self, s: &str, st: glasskit::text::Style) -> f32 {
        self.0.text_width(s, st)
    }
}

#[allow(dead_code)]
fn _cursor_shape(c: Cursor) -> CursorShape {
    c.into()
}

#[allow(dead_code)]
fn _unused(_: TabDir) {}

#[cfg(test)]
impl IslandView {
    pub fn dbg_tab(&self) -> Option<String> {
        self.host.cur.clone()
    }
    pub fn dbg_has_tab(&self, id: &str) -> bool {
        self.host.has(id)
    }
    pub fn dbg_placeholder(&self, id: &str) -> bool {
        self.host.is_placeholder(id)
    }
    pub fn dbg_overlay_open(&self) -> bool {
        self.host.overlay_open
    }
    /// Applies a tab effect as if a tab had posted it.
    pub fn dbg_fx(&mut self, fx: TabFx, cx: &mut Cx) {
        self.apply_tab_fx(fx, cx);
        self.exec(cx);
    }
    pub fn dbg_views_rect(&self) -> Rect {
        self.chrome.views
    }
    /// Window px of the middle of the header tab `id` (as last laid out).
    pub fn dbg_tab_center(&self, id: &str) -> Option<(f32, f32)> {
        let (ox, oy, s) = self.panel_origin;
        self.chrome.tabs.iter().find(|t| t.id == id).map(|t| (ox + t.rect.cx() * s, oy + t.rect.cy() * s))
    }
    /// Window px of the phone card's pieces (tests): the two buttons, the reply field and the send arrow; None while no card is laid out.
    pub fn dbg_phone_points(&self) -> Option<Vec<(f32, f32)>> {
        let (lay, o) = self.phone.dbg_lay()?;
        let v = self.core.shell.view();
        let s = v.scale as f32;
        let at = |r: Rect| (v.body.left as f32 + (o.0 + r.cx()) * s, v.body.top as f32 + (o.1 + r.cy()) * s);
        Some(vec![at(lay.buttons[0]), at(lay.buttons[1]), at(lay.field), at(lay.send), at(lay.avatar)])
    }
    pub fn dbg_body_h(&self) -> f64 {
        self.core.shell.view().body.height
    }
    pub fn dbg_hover_of(&self, id: &str) -> f32 {
        self.hdr.hover_of(id)
    }
    pub fn dbg_frames_wanted(&self) -> bool {
        self.fx.moving() || self.hdr.moving() || self.phone.moving()
    }
    pub fn dbg_panel_origin(&self) -> (f32, f32, f32) {
        self.panel_origin
    }
    pub fn debug_measured(&self) -> (f64, f64) {
        self.measured
    }
    pub fn debug_pet_rect(&self) -> Option<Rect> {
        self.pet_rect
    }
}

#[cfg(test)]
mod eq_tests {
    use super::*;

    #[test]
    fn each_bar_follows_its_own_slice_of_the_spectrum() {
        assert_eq!(eq_levels(&[]), [0.0; 4]);
        assert_eq!(eq_levels(&[0.0; 32]), [0.0; 4], "silence is flat");
        let mut b = [0.0f64; 32];
        b[..5].fill(0.8); // only the bass
        let l = eq_levels(&b);
        assert!(l[0] > 0.9 && l[1] == 0.0 && l[2] == 0.0 && l[3] == 0.0, "{l:?}");
        let mut b = [0.0f64; 32];
        b[20..].fill(0.4); // only the treble
        let l = eq_levels(&b);
        assert!(l[3] > 0.4 && l[0] == 0.0, "{l:?}");
        assert!(eq_levels(&[1.0; 32]).iter().all(|&x| x == 1.0), "clamped to 1");
        assert_eq!(eq_levels(&[0.5; 3]).len(), 4, "a short spectrum does not panic");
    }

    #[test]
    fn levels_rise_faster_than_they_fall() {
        let up = ease_level(0.0, 1.0, 16.0);
        let down = 1.0 - ease_level(1.0, 0.0, 16.0);
        assert!(up > down && up > 0.2 && down > 0.0, "{up} {down}");
        assert_eq!(ease_level(0.5, 0.5, 16.0), 0.5);
    }
}

#[cfg(test)]
mod spoken_tests {
    use super::*;

    #[test]
    fn the_strip_is_read_out_as_plain_text() {
        assert_eq!(spoken(&LiveContent::None), "");
        let slot = |spec: Value| LiveContent::Slot { id: "agents".into(), key: "k".into(), spec };
        assert_eq!(spoken(&slot(json!({"elements": [{"type": "text", "text": "Agent working"}, {"type": "dot", "color": "green"}]}))), "Agent working");
        // order is kept, repeats and blanks are dropped, nesting is walked
        assert_eq!(spoken(&slot(json!({"a": {"title": "Now playing"}, "b": [{"label": "Song"}, {"label": "Song"}, {"text": "  "}]}))).split(", ").count(), 2);
        assert_eq!(spoken(&slot(json!(null))), "");
        let deep = (0..20).fold(json!({"text": "buried"}), |v, _| json!({ "x": v }));
        assert_eq!(spoken(&slot(deep)), "", "very deep specs are not walked");
    }
}
