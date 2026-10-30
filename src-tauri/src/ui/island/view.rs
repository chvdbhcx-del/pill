//! `IslandView`: the always-resident `pill` window as a native `View` (Overlay). It owns the [`Core`] (shell model + live hub), the drawing
//! resources, the panel host and the pets; this file is the glue between the host's calls and those parts. See docs/native-island.md.
use super::chrome::{self, Btn, ChromeLayout, Hover};
use super::companions::{char_side, reserve_px, Companions};
use super::core::{Core, DropStep, Out, ISLAND_EVENTS};
use super::gpu::Gpu;
use super::live::{self, Painted};
use super::panel::TabHost;
use super::shapes;
use super::sys::{self, SysClock};
use crate::ui::view::{Cx, CursorShape, DropEvent, DropPayload, DropPhase, Msg, ProximityEvent, Ticket, View, Waker};
use glasskit::canvas::{scale_about, translation, mul, Canvas};
use glasskit::geom::Rect;
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
    tab_k: HashMap<String, f32>,
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
    last_hit: Vec<Rect>,
    last_pass: Option<bool>,
    last_gaze: Option<bool>,
    last_theme_light: Option<bool>,
    last_style: Option<(String, EdgeStyle)>,
    now: f64,
    started: bool,
    device_id: usize,
}

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
            tab_k: HashMap::new(),
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
            last_hit: Vec::new(),
            last_pass: None,
            last_gaze: None,
            last_theme_light: None,
            last_style: None,
            now: cx.now_ms,
            started: false,
            device_id: 0,
        };
        v.sync_env(cx);
        v.exec_outs(outs, cx);
        v.apply_settings_side(cx);
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

    /// Pets / characters / reduced motion follow the settings.
    fn apply_settings_side(&mut self, cx: &mut Cx) {
        let style = self.core.shell.settings().character;
        let changed = self.comp.apply(&self.core.settings, style);
        let cfg = self.comp.cfg.clone();
        if changed || self.last_style.is_none() {
            self.core.shell.reserve(reserve_px(&cfg));
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
        let now = cx.now_ms;
        let wake = self.core.shell.next_wakeup().map(|d| (d - now).max(0.0));
        let hub = self.core.tick_hub(now).map(|ms| ms as f64);
        self.alarm.set(match (wake, hub) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        });
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
                Out::TabHidden(id) => self.host.hide(&id, cx),
                Out::Broadcast { name, payload } => {
                    self.host.event_msg(&name, &payload, false, cx);
                }
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
        if self.light() {
            Theme::pill_light()
        } else {
            Theme::pill_dark()
        }
    }
}

fn cursor_of(h: &Hit) -> Cursor {
    match h {
        Hit::Tab(_) | Hit::Btn(_) | Hit::CardBtn(_) | Hit::RowBtn(_) | Hit::Pet => Cursor::Hand,
        _ => Cursor::Arrow,
    }
}

impl View for IslandView {
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
                    }
                }
                self.core.bus_event(&name, &payload, cx.now_ms);
                if matches!(name.as_str(), "settings:changed" | "pill:show") {
                    self.apply_settings_side(cx);
                }
                self.host.event_msg(&name, &payload, latest, cx);
            }
            Msg::Reply { ticket, result } => match self.pending.remove(&ticket.0) {
                Some(Pending::Battery) => self.core.shell_event(Event::BatteryState(result.unwrap_or(Value::Null)), cx.now_ms),
                Some(Pending::Weather) => self.core.shell_event(Event::WeatherGet(result.unwrap_or(Value::Null)), cx.now_ms),
                Some(Pending::SettingsOpen) => self.core.shell_event(Event::SettingsOpened(result.is_ok()), cx.now_ms),
                Some(Pending::DesktopFg) => self.core.shell_event(Event::DesktopFg(result.ok().and_then(|v| v.as_bool()).unwrap_or(false)), cx.now_ms),
                Some(Pending::Answer(ev)) => self.core.dispatch_answer(ev, &result.unwrap_or(Value::Null), cx.now_ms),
                Some(Pending::Fire) | None => {
                    redraw = self.host.other_msg(Msg::Reply { ticket, result }, cx);
                }
            },
            Msg::Custom(b) => match b.downcast::<Own>() {
                Ok(o) => match *o {
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
        self.now = now;
        self.sync_env(cx);
        if let Some((w, h)) = self.pending_measure.take() {
            self.core.shell_event(Event::LiveMeasured { w, h }, now);
        }
        self.core.tick(dt_ms, now);
        self.exec(cx);
        let v = self.core.shell.view();
        let reduced = v.reduced;
        let lin = |cur: f32, target: f32, ms: f64| -> f32 {
            if reduced {
                return target;
            }
            let d = (dt_ms / ms) as f32;
            if cur < target { (cur + d).min(target) } else { (cur - d).max(target) }
        };
        self.live_a = lin(self.live_a, if v.live.visible { 1.0 } else { 0.0 }, mo::LIVE_FADE_MS);
        self.panel_a = if v.panel.visible { mo::PANEL_IN.at(v.panel.shown_age_ms, reduced).opacity as f32 } else { lin(self.panel_a, 0.0, mo::PANEL_OUT_MS) };
        self.flare_a = lin(self.flare_a, if v.body.flare_visible { 1.0 } else { 0.0 }, 200.0);
        let mut tabs_moving = false;
        for t in &v.tabs {
            let k = self.tab_k.entry(t.id.clone()).or_insert(if t.active { 1.0 } else { 0.0 });
            let target = if t.active { 1.0 } else { 0.0 };
            let nk = if reduced { target } else if *k < target { (*k + (dt_ms / mo::DUR_MED_MS) as f32).min(target) } else { (*k - (dt_ms / mo::DUR_MED_MS) as f32).max(target) };
            tabs_moving |= nk != target;
            *k = nk;
        }
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
        let pets_busy = self.comp.tick(dt_ms as f32);
        self.update_window_requests(&v, cx);
        let host_busy = self.host.step(dt_ms, now, cx);
        let rows_busy = self.loops(&v);
        let pulse_busy = self.core.pulse.is_some_and(|(_, t0)| now - t0 < 800.0);
        v.wants_frames || self.pending_measure.is_some() || pets_busy || host_busy || tabs_moving || rows_busy || pulse_busy || self.gpu.gliding() || self.live_a != (if v.live.visible { 1.0 } else { 0.0 }) || self.flare_a != (if v.body.flare_visible { 1.0 } else { 0.0 })
            || (self.panel_a != 0.0 && !v.panel.visible)
            || v.live.swap_age_ms < 400.0 && !reduced
            || v.live.ghost.is_some()
            || !v.live.rolls.is_empty()
            || (v.panel.visible && v.panel.shown_age_ms < 400.0 && !reduced)
            || matches!(v.live.content, LiveContent::Card(_))
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.now = cx.now_ms;
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
        resp
    }

    fn on_proximity(&mut self, ev: &ProximityEvent, cx: &mut Cx) -> Response {
        let now = cx.now_ms;
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
        let now = cx.now_ms;
        let files = matches!(ev.payload, Some(DropPayload::Files(_)));
        match ev.phase {
            DropPhase::Enter if files => {
                if !self.core.shell.view().panel.visible {
                    self.core.drop_phase(DropStep::Enter, now);
                }
                self.exec(cx);
                return Response::consumed();
            }
            DropPhase::Over => return Response::consumed(),
            DropPhase::Leave => self.core.drop_phase(DropStep::Leave, now),
            DropPhase::Drop => {
                if let Some(DropPayload::Files(f)) = &ev.payload {
                    let paths: Vec<String> = f.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                    let home = self.host.cur.as_deref() == Some("home") && self.core.shell.view().panel.visible;
                    self.core.drop_phase(DropStep::Landed, now);
                    self.call(cx, if home { "launcher_add" } else { "shelf_add" }, json!({ "paths": paths }), Pending::Fire);
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
        shapes::paint_body(cv, b, b.radius_top as f32, b.radius_bottom as f32, self.flare_a, 1.0);
        let ctx = live::Ctx { rows: &self.core.rows, cards: &self.core.cards, reduced: v.reduced, ru: self.core.ru() };
        // ---- everything inside the body is clipped to its silhouette
        let mut painted = Painted::default();
        cv.with_clip_rr(body, rb.max(rt), |cv| {
            if self.live_a > 0.0 {
                let pad = if self.comp.cfg.id.is_empty() { 0.0 } else { reserve_px(&self.comp.cfg) as f32 };
                let (pl, pr) = if self.comp.cfg.side == crate::ui::pets::engine::Side::Left { (pad, 0.0) } else { (0.0, pad) };
                let legs_slot = if self.comp.style == EdgeStyle::Legs { 30.0f32 } else { 0.0 };
                let (cs_, slot_dx) = match (legs_slot > 0.0, char_side(&self.comp.cfg)) {
                    (true, CharSide::Left) => (legs_slot, legs_slot / 2.0),
                    (true, _) => (legs_slot, -legs_slot / 2.0),
                    _ => (0.0, 0.0),
                };
                let _ = cs_;
                let local = Rect::new(pl + slot_dx, 0.0, body.w / s - pl - pr, body.h / s);
                let m = mul(&scale_about((0.0, 0.0), s, s), &translation(body.x, body.y));
                cv.with_transform(m, |cv| {
                    let alpha = self.live_a;
                    if let Some(g) = &v.live.ghost {
                        let a = mo::LIVE_GHOST_OUT.at(g.age_ms, v.reduced).opacity as f32;
                        live::paint_content(cv, &th, &mut self.gpu, &ctx, &g.content, &[], &[], local, alpha * a, 1e9, now, None, &mut Painted::default());
                    }
                    let swap = if v.reduced { 1.0 } else { mo::LIVE_SWAP.at(v.live.swap_age_ms, false).opacity as f32 };
                    live::paint_content(cv, &th, &mut self.gpu, &ctx, &v.live.content, &v.live.rolls, &v.live.entering, local, alpha * swap, v.live.swap_age_ms, now, self.hover_card_btn, &mut painted);
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
            // pets: on the capsule floor while closed, in the header gap while open
            self.pet_rect = None;
            if !self.comp.cfg.id.is_empty() {
                if self.live_a > 0.0 {
                    if let Some(r) = self.comp.cap_box(body, s) {
                        self.pet_rect = Some(r);
                        self.comp.draw_pet(cv, r, self.live_a);
                    }
                } else if self.panel_a > 0.0 && self.chrome.pet_slot.w >= 56.0 {
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
                let tab_k = self.tab_k.clone();
                let mut meter = LiveMeter(cv);
                let ph = body.h / s;
                self.chrome = chrome::layout(&v, ru, ph, &|id, active| tab_k.get(id).copied().unwrap_or(if active { 1.0 } else { 0.0 }), &mut meter);
                let m = mul(&scale_about((0.0, 0.0), s, s), &translation(ox, oy));
                cv.with_transform(m, |cv| {
                    cv.with_opacity(self.panel_a, |cv| chrome::paint(cv, &th, &mut self.gpu, &v, &self.chrome, self.hover, ru));
                    self.host.draw(cv, &th, self.chrome.views, cx, now, self.panel_a);
                });
            } else {
                self.chrome = ChromeLayout::default();
            }
        });
        self.painted = painted;
        // privacy dots in the top-right corner
        let (cam, mic) = self.core.privacy;
        if (cam || mic) && self.live_a > 0.0 {
            let k = if v.reduced { 1.0 } else { 0.775 + 0.225 * (std::f64::consts::TAU * now / 2400.0).cos() as f32 };
            let mut x = body.right() - 11.0 * s - 6.0 * s;
            for (on, c) in [(mic, glasskit::Color::rgb8(0xff, 0x9f, 0x0a)), (cam, glasskit::Color::rgb8(0x30, 0xd1, 0x58))] {
                if on {
                    cv.fill_circle((x + 3.0 * s, body.y + 10.0 * s), 5.0 * s, c.alpha(0.18 * self.live_a));
                    cv.fill_circle((x + 3.0 * s, body.y + 10.0 * s), 3.0 * s, c.fade(k * self.live_a));
                    x -= 9.0 * s;
                }
            }
        }
        // ---- the liquid edge and the capsule characters hang below / outside the body
        let pulse = self.core.pulse.map_or(0.0, |(a, t0)| a * (1.0 - ((now - t0) / 800.0) as f32).clamp(0.0, 1.0));
        shapes::paint_edge(cv, &v.edge, (v.strip.x as f32, v.strip.y as f32), pulse, v.reduced);
        match self.comp.style {
            EdgeStyle::Tail => self.comp.draw_tail(cv, body, rb, s),
            EdgeStyle::Legs => {
                let side = char_side(&self.comp.cfg);
                let cy = body.y + ((body.h / s - 2.0) / 2.0) * s;
                let cx_ = match side {
                    CharSide::Right => body.right() - 18.0 * s,
                    _ => body.x + 18.0 * s,
                };
                self.comp.draw_legs_eyes(cv, body, (cx_, cy), s);
            }
            _ => {}
        }
        let _ = ShellTheme::Dark;
    }
}

impl IslandView {
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
                Elem::Check { .. } => true,
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
    pub fn debug_measured(&self) -> (f64, f64) {
        self.measured
    }
}
