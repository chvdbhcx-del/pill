//! The Agents tab: a read-only list of the coding-agent sessions (Claude Code, Codex), waiting first. Port of the sessions tab of
//! `src/agents-live.js` + `agents-live.css` (`.ag-row`). The model (`panel::b::agents::AgentsModel`) also feeds the live row / card / badge of
//! the collapsed pill; those leave through `kit::route_effects` as `TabFx`.
//!
//! Motion (all liquid, see `ui::liquid`): a freshly opened tab staggers its rows in (28 ms steps), later updates FLIP (a row that moved
//! slides to its new place with its speed kept, a new row drops in, a gone one fades out), a state change pops the row's dot, the empty
//! text fades in, and working / waiting dots breathe (1.6 s / 1.3 s). Reduced motion lands everything and the dots stay still.
use super::kit::{lang_of, ms_of, route_effects, Calls, Cues, Lit};
use crate::ui::view::{Cx, Msg, Ticket, View};
use glasskit::canvas::Canvas;
use crate::ui::liquid::{Feel, Flow};
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, Response};
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::b::agents::{AgentsModel, Row};
use pill_ui_core::panel::b::{Anim, Lang};
use serde_json::{json, Value};

const TOP_PAD: f32 = 4.0;
const RIGHT_PAD: f32 = 8.0;
const ROW_PAD_X: f32 = 12.0;
const ROW_H: f32 = 44.0;
const ROW_H_TITLE: f32 = 46.0;
const DOT: f32 = 8.0;
const MAX_STAGGER: usize = 8;
/// seconds between two rows of a stagger
const STAGGER: f32 = 0.028;

fn row_h(r: &Row) -> f32 {
    if r.title.is_empty() {
        ROW_H
    } else {
        ROW_H_TITLE
    }
}

/// `(id, y, height)` of every row, in content coordinates.
fn targets(rows: &[Row]) -> Vec<(String, f32, f32)> {
    let mut y = TOP_PAD;
    rows.iter()
        .map(|r| {
            let t = (r.id.clone(), y, row_h(r));
            y += t.2;
            t
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Row animator (FLIP)
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// no animation: rows are simply there
    Jump,
    /// the tab was just opened: rows come in one after another
    Stagger,
    /// an update while open: rows slide, new ones drop in, gone ones fade
    Flip,
}

struct Slot {
    id: String,
    y: Flow,
    alpha: Flow,
    rise: Flow,
    h: f32,
    leaving: bool,
    /// seconds before an entering row starts (the stagger)
    wait: f32,
}

impl Slot {
    /// The enter starts: fade in, rise to the row's place.
    fn go(&mut self) {
        self.alpha.to(1.0);
        self.rise.to(0.0);
    }
}

/// Where each row is drawn right now. Pure: no canvas, unit-tested below.
#[derive(Default)]
struct Flipper {
    slots: Vec<Slot>,
}

impl Flipper {
    fn enter(y: f32, h: f32, id: String, delay: f32) -> Slot {
        let mut s = Slot { id, y: Flow::px(y, Feel::FLOW), alpha: Flow::new(0.0, Feel::FADE), rise: Flow::px(6.0, Feel::FLOW), h, leaving: false, wait: delay };
        if delay <= 0.0 {
            s.go();
        }
        s
    }

    fn rest(y: f32, h: f32, id: String) -> Slot {
        Slot { id, y: Flow::px(y, Feel::FLOW), alpha: Flow::new(1.0, Feel::FADE), rise: Flow::px(0.0, Feel::FLOW), h, leaving: false, wait: 0.0 }
    }

    fn set(&mut self, t: &[(String, f32, f32)], mode: Mode, reduced: bool) {
        let mode = if reduced { Mode::Jump } else { mode };
        match mode {
            Mode::Jump => self.slots = t.iter().map(|(id, y, h)| Self::rest(*y, *h, id.clone())).collect(),
            Mode::Stagger => {
                self.slots = t.iter().enumerate().map(|(i, (id, y, h))| Self::enter(*y, *h, id.clone(), i.min(MAX_STAGGER) as f32 * STAGGER)).collect();
            }
            Mode::Flip => {
                let mut next: Vec<Slot> = Vec::with_capacity(t.len());
                for (id, y, h) in t {
                    match self.slots.iter().position(|s| s.id == *id && !s.leaving) {
                        Some(i) => {
                            let mut s = self.slots.remove(i);
                            s.h = *h;
                            s.y.to(*y);
                            next.push(s);
                        }
                        None => next.push(Self::enter(*y, *h, id.clone(), 0.0)),
                    }
                }
                // what is left was removed: fade it where it stands
                for mut s in std::mem::take(&mut self.slots) {
                    if !s.leaving {
                        s.leaving = true;
                        s.alpha.to(0.0);
                    }
                    next.push(s);
                }
                self.slots = next;
            }
        }
    }

    fn step(&mut self, dt: f32) -> bool {
        let ms = dt as f64 * 1000.0;
        let mut busy = false;
        for s in &mut self.slots {
            let mut enter_ms = ms;
            if s.wait > 0.0 {
                busy = true;
                if s.wait > dt {
                    s.wait -= dt;
                    enter_ms = 0.0;
                } else {
                    enter_ms = (dt - s.wait) as f64 * 1000.0;
                    s.wait = 0.0;
                    s.go();
                }
            }
            s.y.step(ms);
            s.alpha.step(enter_ms);
            s.rise.step(enter_ms);
            busy |= s.y.moving() || s.alpha.moving() || s.rise.moving();
        }
        self.slots.retain(|s| !(s.leaving && !s.alpha.moving()));
        busy
    }

    /// `(y, alpha, rise)` of a live row.
    fn pos(&self, id: &str) -> Option<(f32, f32, f32)> {
        self.slots.iter().find(|s| s.id == id && !s.leaving).map(|s| (s.y.at(), s.alpha.at01(), s.rise.at()))
    }

    /// `(id, y, h, alpha)` of the rows that are fading out.
    fn ghosts(&self) -> impl Iterator<Item = (&str, f32, f32, f32)> {
        self.slots.iter().filter(|s| s.leaving).map(|s| (s.id.as_str(), s.y.at(), s.h, s.alpha.at01()))
    }

    fn moving(&self) -> bool {
        self.slots.iter().any(|s| s.wait > 0.0 || s.y.moving() || s.alpha.moving() || s.rise.moving())
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------------------------------------------------

pub struct AgentsScreen {
    model: AgentsModel,
    calls: Calls,
    cues: Cues,
    flip: Flipper,
    scroll: ScrollView,
    rect: Rect,
    mounted: bool,
    shown: bool,
    fixed_now: Option<i64>,
    last_ms: Option<f64>,
    carry: f64,
    reduced: bool,
    settings: Option<Ticket>,
    /// a settings:changed event waits for the settings read
    after_settings: bool,
    pulse: bool,
    /// the empty-list text fades in
    empty_k: Lit,
}

impl Default for AgentsScreen {
    fn default() -> Self {
        AgentsScreen::new()
    }
}

impl AgentsScreen {
    pub fn new() -> AgentsScreen {
        AgentsScreen {
            model: AgentsModel::new(Lang::En),
            calls: Calls::new(),
            cues: Cues::new(),
            flip: Flipper::default(),
            scroll: ScrollView::new(),
            rect: Rect::default(),
            mounted: false,
            shown: false,
            fixed_now: None,
            last_ms: None,
            carry: 0.0,
            reduced: false,
            settings: None,
            after_settings: false,
            pulse: false,
            empty_k: Lit::new(Feel::FADE),
        }
    }

    /// Pin the wall clock (tests, fixtures): ages are `now - session.ts`.
    pub fn with_now(mut self, ms: i64) -> AgentsScreen {
        self.fixed_now = Some(ms);
        self
    }

    fn wall_ms() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
    }

    fn mount(&mut self, cx: &mut Cx) {
        if self.mounted {
            return;
        }
        self.mounted = true;
        cx.subscribe("agents:changed");
        cx.subscribe("agents:done");
        cx.subscribe("settings:changed");
        self.settings = Some(cx.call_async("settings_get", json!({})));
        let c = self.model.mount();
        self.calls.run(cx, c);
    }

    /// Everything a model call produced: effects to the island, cues to the animators, then re-layout of the rows.
    fn pump(&mut self, cx: &mut Cx) {
        let _view_only = route_effects(self.model.fx.take_effects(), cx);
        let mut mode = Mode::Jump;
        for c in self.model.fx.take_anims() {
            match (c.target.as_str(), c.kind) {
                ("agents-list", Anim::Stagger) => mode = Mode::Stagger,
                ("agents-list", Anim::Flip) => mode = Mode::Flip,
                (t, _) if t.starts_with("dot:") => self.cues.play(c),
                _ => {}
            }
        }
        let v = self.model.view();
        self.pulse = v.rows.iter().any(|r| r.state == "working" || r.state == "waiting");
        self.flip.set(&targets(&v.rows), mode, self.reduced);
    }

    fn content_h(&self) -> f32 {
        targets(&self.model.view().rows).last().map_or(0.0, |(_, y, h)| y + h) + 4.0
    }
}

impl View for AgentsScreen {
    fn event(&mut self, ev: &InputEvent, _cx: &mut Cx) -> Response {
        if self.rect.w <= 0.0 {
            return Response::NONE;
        }
        let list = self.rect;
        self.scroll.event(ev, list)
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Event { name, payload, .. } => {
                if name == "settings:changed" {
                    // the model compares the live switch before / after, so read the setting first
                    self.after_settings = true;
                    self.settings = Some(cx.call_async("settings_get", json!({})));
                } else {
                    let c = self.model.on_event(&name, &payload);
                    self.calls.run(cx, c);
                    self.pump(cx);
                }
            }
            Msg::Reply { ticket, result } => {
                if self.settings == Some(ticket) {
                    self.settings = None;
                    if let Ok(cfg) = result {
                        self.model.set_live_on(live_on(&cfg));
                    }
                    if std::mem::take(&mut self.after_settings) {
                        let c = self.model.on_event("settings:changed", &Value::Null);
                        self.calls.run(cx, c);
                    }
                    self.pump(cx);
                } else if let Some((tag, r)) = self.calls.take(ticket, result) {
                    let c = self.model.on_reply(tag, r);
                    self.calls.run(cx, c);
                    self.pump(cx);
                }
            }
            Msg::Open(_) | Msg::Custom(_) => {}
        }
        Response::redraw()
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.shown = true;
        self.reduced = cx.reduced_motion;
        self.cues.reduced = self.reduced;
        self.model.set_lang(lang_of(cx));
        self.model.set_now(self.fixed_now.unwrap_or_else(Self::wall_ms));
        self.last_ms = Some(cx.now_ms);
        self.mount(cx);
        self.model.on_show();
        self.pump(cx);
        cx.request_frame();
    }

    fn on_hide(&mut self, _cx: &mut Cx) {
        self.shown = false;
        self.model.on_hide();
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.reduced = cx.reduced_motion;
        self.cues.reduced = self.reduced;
        self.model.set_lang(lang_of(cx));
        // the models run on the real clock: after an idle gap the repaint timer and the ages catch up
        let real = self.last_ms.map_or(dt_ms, |l| (cx.now_ms - l).max(0.0));
        self.last_ms = Some(cx.now_ms);
        let ms = ms_of(real, &mut self.carry);
        if ms > 0 {
            self.model.tick(ms);
        }
        self.pump(cx);
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.flip.step(dt);
        busy |= self.cues.step(dt_ms as f32);
        busy |= self.scroll.step(dt);
        busy |= self.empty_k.step(dt_ms);
        busy | (self.pulse && !self.reduced && self.shown)
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.rect = rect;
        self.model.set_lang(lang_of(cx));
        let v = self.model.view();
        let w = rect.w - RIGHT_PAD;
        self.scroll.set_content_height(self.content_h(), rect.h);
        self.empty_k.set(v.empty_text.is_none());
        if let Some(text) = &v.empty_text {
            let r = Rect::new(rect.x + ROW_PAD_X, rect.y + TOP_PAD + 20.0, (w - 2.0 * ROW_PAD_X).max(10.0), rect.h);
            let st = Style::new(12.0, 400);
            cv.with_opacity(1.0 - self.empty_k.at(), |cv| cv.text(r, text, st.wrap(), th.dim));
            return;
        }
        let now = cx.now_ms;
        let pulse_on = !cx.reduced_motion;
        self.scroll.draw(cv, rect, th, |cv, _c| {
            let draw_row = |cv: &Canvas, r: &Row, y: f32, h: f32, alpha: f32, rise: f32| {
                let rr = Rect::new(0.0, y + rise, w, h);
                cv.with_opacity(alpha, |cv| {
                    let (col, period) = match r.state.as_str() {
                        "waiting" => (th.amber, 1300.0),
                        "working" => (th.accent, 1600.0),
                        "done" => (th.green, 0.0),
                        _ => (th.dim, 0.0),
                    };
                    let breathe = if period > 0.0 && pulse_on { 0.5 - 0.5 * ((now % period) / period * std::f64::consts::TAU).cos() as f32 } else { 0.0 };
                    let pop = self.cues.xf(&format!("dot:{}", r.id));
                    let c = (rr.x + ROW_PAD_X + DOT / 2.0, rr.cy());
                    let rad = DOT / 2.0 * (1.0 - 0.2 * breathe) * pop.scale;
                    cv.fill_circle(c, rad, col.fade((1.0 - 0.55 * breathe) * pop.alpha));
                    let state_st = Style::new(11.0, 400).mono();
                    let sw = cv.text_width(&r.state_text, state_st).ceil() + 1.0;
                    let state_r = Rect::new(rr.right() - ROW_PAD_X - sw, rr.y, sw, rr.h);
                    cv.text(state_r, &r.state_text, state_st, if r.state == "waiting" { th.amber } else { th.dim });
                    let bx = rr.x + ROW_PAD_X + DOT + 12.0;
                    let bw = (state_r.x - 12.0 - bx).max(20.0);
                    if r.title.is_empty() {
                        cv.text(Rect::new(bx, rr.y, bw, rr.h), &r.name, Style::new(13.0, 500).ellipsis(), th.text);
                    } else {
                        cv.text(Rect::new(bx, rr.y + 6.0, bw, 18.0), &r.name, Style::new(13.0, 500).ellipsis(), th.text);
                        cv.text(Rect::new(bx, rr.y + 26.0, bw, 14.0), &r.title, Style::new(11.0, 400).ellipsis(), th.dim);
                    }
                });
            };
            let tgt = targets(&v.rows);
            for (r, (_, ty, h)) in v.rows.iter().zip(&tgt) {
                let (y, a, rise) = self.flip.pos(&r.id).unwrap_or((*ty, 1.0, 0.0));
                draw_row(cv, r, y, *h, a, rise);
            }
            // rows that left fade where they stood (their data is gone, so only a dim bar of the right size remains: name unknown)
            for (_id, y, h, a) in self.flip.ghosts() {
                cv.with_opacity(a * 0.5, |cv| cv.fill_rr(Rect::new(ROW_PAD_X, y + h / 2.0 - 4.0, w - 2.0 * ROW_PAD_X, 8.0), 4.0, th.hover));
            }
        });
    }
}

/// `Pill.liveOn('agents')`: the setting `live.agents`, on unless it is explicitly false.
fn live_on(cfg: &Value) -> bool {
    cfg.get("live.agents").and_then(Value::as_bool).unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::{motion_lock, Sim};
    use crate::ui::tabs::fx::{self as fx_b, TabFx};
    use crate::ui::tabs::b::kit::snap;
    use crate::ui::view::Services;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/tabs-b/agents.json");

    fn fixture() -> Value {
        serde_json::from_str(FIXTURE).unwrap()
    }

    fn rows(ids: &[&str]) -> Vec<(String, f32, f32)> {
        ids.iter().enumerate().map(|(i, id)| (id.to_string(), TOP_PAD + i as f32 * 44.0, 44.0)).collect()
    }

    // ---- the animator ----

    #[test]
    fn stagger_delays_each_row_and_ends_settled() {
        let _g = crate::ui::host::testing::motion_on(); // another test may flip the process-wide reduced-motion override; animations on whatever the OS says
        let mut f = Flipper::default();
        f.set(&rows(&["a", "b", "c"]), Mode::Stagger, false);
        let a0 = f.pos("a").unwrap();
        let c0 = f.pos("c").unwrap();
        assert!(a0.1 < 0.05 && a0.2 > 5.0, "rows start transparent and low: {a0:?}");
        f.step(0.1);
        assert!(f.pos("a").unwrap().1 > f.pos("c").unwrap().1, "the first row is ahead of the third ({c0:?})");
        for _ in 0..80 {
            f.step(1.0 / 60.0);
        }
        assert!(!f.moving());
        assert_eq!(f.pos("c").unwrap(), (TOP_PAD + 88.0, 1.0, 0.0));
    }

    #[test]
    fn flip_slides_moved_rows_drops_new_and_fades_gone() {
        let _g = crate::ui::host::testing::motion_on();
        let mut f = Flipper::default();
        f.set(&rows(&["a", "b"]), Mode::Jump, false);
        f.set(&rows(&["b", "n", "a"]), Mode::Flip, false);
        f.step(0.05);
        let (ya, _, _) = f.pos("a").unwrap();
        assert!(ya > TOP_PAD && ya < TOP_PAD + 88.0, "a slides from slot 0 to slot 2, now at {ya}");
        assert!(f.pos("n").unwrap().1 < 1.0, "the new row fades in");
        f.set(&rows(&["b", "n"]), Mode::Flip, false);
        assert_eq!(f.ghosts().count(), 1, "a fades out in place");
        for _ in 0..60 {
            f.step(1.0 / 60.0);
        }
        assert_eq!(f.ghosts().count(), 0);
        assert!(!f.moving());
    }

    #[test]
    fn a_row_retargeted_mid_slide_keeps_its_speed_overshoots_a_little_and_settles() {
        let _g = crate::ui::host::testing::motion_on();
        let mut f = Flipper::default();
        f.set(&rows(&["a", "b"]), Mode::Jump, false);
        f.set(&rows(&["b", "a"]), Mode::Flip, false);
        for _ in 0..6 {
            f.step(1.0 / 60.0);
        }
        let (y, ..) = f.pos("a").unwrap();
        assert!(y > TOP_PAD && y < TOP_PAD + 44.0, "on its way down: {y}");
        f.set(&rows(&["a", "b"]), Mode::Flip, false); // turn around
        let (y2, ..) = f.pos("a").unwrap();
        assert_eq!(y, y2, "no jump on retarget");
        f.step(1.0 / 60.0);
        assert!(f.pos("a").unwrap().0 >= y - 0.01, "it carries on a little before turning");
        let (mut peak, mut frames) = (0.0f32, 0);
        while f.step(1.0 / 60.0) {
            peak = peak.max(f.pos("b").unwrap().0);
            frames += 1;
            assert!(frames < 120, "settles");
        }
        assert!(peak < TOP_PAD + 44.0 + 10.0, "bounded overshoot: {peak}");
        assert_eq!(f.pos("a").unwrap().0, TOP_PAD);
    }

    #[test]
    fn reduced_motion_jumps() {
        let mut f = Flipper::default();
        f.set(&rows(&["a", "b"]), Mode::Stagger, true);
        assert_eq!(f.pos("b").unwrap(), (TOP_PAD + 44.0, 1.0, 0.0));
        f.set(&rows(&["b"]), Mode::Flip, true);
        assert_eq!(f.ghosts().count(), 0);
        assert!(!f.moving());
    }

    // ---- wiring ----

    fn sim_for(name: &str) -> (Sim, crate::ui::view::Fakes) {
        let fx = fixture();
        let sc = &fx["scenarios"][name];
        let (svc, fakes) = Services::fake();
        for (cmd, v) in sc["replies"].as_object().unwrap() {
            fakes.api.reply(cmd, Ok(v.clone()));
        }
        let mut sim = Sim::with_services(Box::new(AgentsScreen::new().with_now(fx["now"].as_i64().unwrap())), svc);
        sim.show();
        sim.pump();
        for e in sc["events"].as_array().unwrap() {
            fakes.bus.emit(e["name"].as_str().unwrap(), e["payload"].clone());
        }
        sim.pump();
        (sim, fakes)
    }

    #[test]
    fn mount_subscribes_and_lists_then_badges_the_tab() {
        let _ = fx_b::take();
        let (mut sim, fakes) = sim_for("list");
        let names: Vec<String> = fakes.api.calls().into_iter().map(|c| c.name).collect();
        assert!(names.contains(&"agents_list".to_string()) && names.contains(&"settings_get".to_string()), "{names:?}");
        for e in ["agents:changed", "agents:done", "settings:changed"] {
            assert_eq!(fakes.bus.subscribers(e), 1, "{e}");
        }
        let out = fx_b::take();
        assert!(out.contains(&TabFx::Badge { tab: "agents", text: "1".into() }), "{out:?}");
        // the "agents" row belongs to the island's live layer (spinner + status): the tab must not overwrite it with a plain text row
        assert!(!out.iter().any(|f| matches!(f, TabFx::SetLive { key, .. } | TabFx::React { key, .. } | TabFx::Unreact { key } if key == "agents")), "{out:?}");
        assert!(sim.run(600) > 0, "the rows stagger in, then the frames stop... or the dots keep breathing");
    }

    #[test]
    fn an_event_replaces_the_list_and_a_state_change_pops_the_dot() {
        let _ = fx_b::take();
        let (mut sim, _fakes) = sim_for("state_change");
        let out = fx_b::take();
        assert!(out.contains(&TabFx::Badge { tab: "agents", text: "1".into() }), "the event's snapshot (1 waiting) won over the list reply: {out:?}");
        assert!(sim.run(30) > 0, "the dot breathes, so frames keep coming");
    }

    #[test]
    fn live_switch_off_is_read_from_settings() {
        assert!(live_on(&json!({})));
        assert!(live_on(&json!({ "live.agents": true })));
        assert!(!live_on(&json!({ "live.agents": false })));
    }

    #[test]
    fn idle_without_working_rows_costs_no_frames() {
        let _m = motion_lock();
        glasskit::ease::force_reduced(Some(false));
        let (mut sim, _f) = sim_for("empty");
        sim.run(600);
        assert!(!sim.frame.wants_frame(), "an empty list settles");
        glasskit::ease::force_reduced(None);
    }

    // ---- snapshots ----

    fn shoot_all(name: &str, scenario: &str) {
        let _m = motion_lock();
        glasskit::ease::force_reduced(Some(false));
        for (dark, scale, reduced) in [(true, 1.0, false), (true, 2.0, false), (false, 1.0, false), (false, 2.0, true)] {
            let (mut sim, _f) = sim_for(scenario);
            sim.cx.reduced_motion = reduced;
            sim.cx.lang = glasskit::lang::Lang::En;
            // let the stagger finish
            sim.run(240);
            sim.cx.now_ms = 400.0;
            let img = snap::shoot(sim.view.as_mut(), &sim.cx, name, snap::BODY, scale, dark);
            assert!(img.count(None, |p| p[3] > 0) > 100);
        }
        glasskit::ease::force_reduced(None);
    }

    #[test]
    fn snapshot_list() {
        shoot_all("agents-list", "list");
    }

    #[test]
    fn snapshot_empty() {
        shoot_all("agents-empty", "empty");
    }

    #[test]
    fn rows_are_drawn_in_their_state_colours() {
        let _m = motion_lock();
        let (mut sim, _f) = sim_for("list");
        sim.cx.reduced_motion = true;
        sim.run(10);
        sim.cx.now_ms = 0.0; // the breathing phase at its start: full size, full opacity
        let img = snap::shoot(sim.view.as_mut(), &sim.cx, "agents-colours", snap::BODY, 1.0, true);
        // the waiting dot is amber (first row), the working dot is the periwinkle accent (second row)
        let amber = img.count(Some(Rect::new(8.0, 4.0, 24.0, 46.0)), |p| p[0] > 200 && p[1] > 120 && p[2] < 100);
        let accent = img.count(Some(Rect::new(8.0, 50.0, 24.0, 46.0)), |p| p[2] > 200 && p[0] > 110);
        assert!(amber > 5, "amber dot pixels: {amber}");
        assert!(accent > 5, "accent dot pixels: {accent}");
    }
}
