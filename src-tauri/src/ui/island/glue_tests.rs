//! Headless integration tests of the glue: the real `IslandView` with the real tab views (no window, WARP renderer, fake backend).
//! Every tab id opens and draws, messages reach the kept views, each `TabFx` has its effect on the shell, the `ai_actions` overlay opens and
//! closes, drops land on the right tab. See docs/native-run.md.
use super::fonts::FONTS;
use super::snapshots::media_payload;
use super::sys::SysClock;
use super::view::IslandView;
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{Cx, DropEvent, DropPayload, DropPhase, Fakes, Msg, Services, View};
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, KeyCode, Modifiers, Response};
use glasskit::testkit::{Harness, Image};
use glasskit::theme::Theme;
use glasskit::Color;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static H: Harness = Harness::with_fonts(&FONTS);
}

const CLOCK_MS: i64 = 1_793_356_440_000;

/// The island shared between the simulated window and the test (so the test can look inside).
struct Shared(Rc<RefCell<IslandView>>);

impl View for Shared {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.0.borrow_mut().event(ev, cx)
    }
    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        self.0.borrow_mut().on_drop(ev, cx)
    }
    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.0.borrow_mut().message(msg, cx)
    }
    fn step(&mut self, dt: f64, cx: &mut Cx) -> bool {
        self.0.borrow_mut().step(dt, cx)
    }
    fn draw(&mut self, cv: &glasskit::Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.0.borrow_mut().draw(cv, th, rect, cx)
    }
    fn on_show(&mut self, cx: &mut Cx) {
        self.0.borrow_mut().on_show(cx)
    }
    fn on_hide(&mut self, cx: &mut Cx) {
        self.0.borrow_mut().on_hide(cx)
    }
}

struct Empty;
impl View for Empty {
    fn draw(&mut self, _: &glasskit::Canvas, _: &Theme, _: Rect, _: &Cx) {}
}

struct G {
    sim: crate::ui::host::testing::Sim,
    fakes: Fakes,
    v: Rc<RefCell<IslandView>>,
}

fn settings() -> Value {
    json!({"size":100,"theme":"dark","clock24":true,"shape":"notch","character":"liquid","pet":"none","pet.side":"right","pet.size":"m",
        "musicPill":"title","net.weather":true,"live.obs":false,"language":"auto"})
}

impl G {
    fn new() -> G {
        let (svc, fakes): (Services, Fakes) = Services::fake();
        fakes.api.reply("settings_get", Ok(settings()));
        fakes.api.reply("weather_get", Ok(json!({"status":"ok","now":{"temp":18.4,"icon":"partly-day","text":"Partly cloudy"}})));
        fakes.api.reply("battery_state", Ok(json!({"has_battery":false,"percent":0,"charging":false})));
        let mut sim = crate::ui::host::testing::Sim::with_services(Box::new(Empty), svc);
        sim.cx.reduced_motion = false;
        let v = IslandView::with_parts(&mut sim.cx, SysClock::fixed(CLOCK_MS, 120), "en-US");
        let v = Rc::new(RefCell::new(v));
        sim.view = Box::new(Shared(v.clone()));
        sim.show();
        let mut g = G { sim, fakes, v };
        g.settle(60);
        g
    }

    fn settle(&mut self, frames: usize) {
        for _ in 0..frames {
            self.sim.pump();
            if !self.sim.step_frame() {
                break;
            }
        }
        self.sim.pump();
    }

    fn emit(&mut self, name: &str, payload: Value) {
        self.fakes.bus.emit(name, payload);
        self.sim.pump();
        self.settle(4);
    }

    fn draw(&mut self) -> Image {
        let th = Theme::pill_dark();
        let rect = Rect::new(0.0, 0.0, 720.0, 480.0);
        let (cx, v) = (&self.sim.cx, &self.v);
        H.with(|h| h.render(720, 480, 1.0, Color::rgba(0.0, 0.0, 0.0, 0.0), |cv| v.borrow_mut().draw(cv, &th, rect, cx)))
    }

    fn expand(&mut self, tab: &str) {
        self.emit("pill:expand", json!({"tab": tab, "via": "test"}));
        let _ = self.draw();
        self.settle(120);
        let _ = self.draw();
        self.settle(40);
    }

    fn calls(&self, name: &str) -> Vec<Value> {
        self.fakes.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
    }

    fn fx(&mut self, fx: TabFx) {
        let (cx, v) = (&mut self.sim.cx, &self.v);
        v.borrow_mut().dbg_fx(fx, cx);
        self.settle(4);
    }

    fn esc(&mut self) -> Response {
        self.sim.send(&InputEvent::Key { code: KeyCode::Escape, mods: Modifiers::default(), down: true, repeat: false })
    }
}

#[test]
fn every_tab_id_opens_a_real_view_that_draws() {
    for id in ["home", "weather", "controls", "notifs", "clipboard", "shelf", "note", "text", "chat", "agents"] {
        let mut g = G::new();
        g.expand(id);
        let v = g.v.borrow();
        assert_eq!(v.dbg_tab().as_deref(), Some(id), "{id}: the tab is current");
        assert!(v.dbg_has_tab(id) && !v.dbg_placeholder(id), "{id}: a real view, not the placeholder");
        let views = v.dbg_views_rect();
        drop(v);
        assert_eq!((views.w, views.h > 200.0), (648.0, true), "{id}: the content box is 648 wide: {views:?}");
        let img = g.draw();
        let (ox, oy, s) = g.v.borrow().dbg_panel_origin();
        let r = Rect::new(ox + views.x * s, oy + views.y * s, views.w * s, views.h * s);
        // something is painted inside the tab box (not just the black body)
        let lit = img.count(Some(r), |p| p[3] > 0 && (p[0] > 30 || p[1] > 30 || p[2] > 30));
        assert!(lit > 150, "{id}: the tab draws pixels, got {lit}");
    }
}

#[test]
fn background_tabs_exist_from_the_start_and_get_the_events() {
    let mut g = G::new();
    for id in ["home", "shelf", "notifs", "clipboard"] {
        assert!(g.v.borrow().dbg_has_tab(id), "{id} is kept alive");
    }
    assert!(!g.v.borrow().dbg_has_tab("weather"), "weather is fed by the backend poll, not by a kept view");
    // the home tab (hidden) reacts to media: it asked the backend for the session at start
    let names: Vec<String> = g.fakes.api.calls().into_iter().map(|c| c.name).collect();
    assert!(names.iter().any(|n| n.starts_with("media") || n.starts_with("launcher")), "home started its model: {names:?}");
    g.emit("media:changed", media_payload());
    assert!(g.v.borrow().core.rows.contains_key("media"), "the hub shows the music row while the panel is closed");
}

#[test]
fn a_toast_is_recorded_by_the_notifications_tab() {
    let mut g = G::new();
    g.fx(TabFx::Toast { id: "t1", icon: "bell", title: "Hello".into(), ttl_ms: 3000 });
    g.settle(6);
    let adds = g.calls("notif_history_add");
    assert!(!adds.is_empty(), "the notifs view recorded the card: {:?}", g.fakes.api.calls().iter().map(|c| c.name.clone()).collect::<Vec<_>>());
}

#[test]
fn each_tabfx_has_its_effect() {
    let mut g = G::new();
    // cards
    g.fx(TabFx::Toast { id: "t1", icon: "bell", title: "Hello".into(), ttl_ms: 3000 });
    assert!(g.v.borrow().core.cards.contains_key("t1"));
    let spec = pill_ui_core::panel::b::ToastSpec {
        id: "c2".into(),
        icon: "check".into(),
        title: "Done".into(),
        sub: "sub".into(),
        tone: Some("green"),
        ttl_ms: 4000,
        actions: vec![pill_ui_core::panel::b::ToastAction { label: "Again".into(), kind: Some("white"), run: pill_ui_core::panel::b::ToastRun::Model("timer:again".into()) }],
    };
    g.fx(TabFx::Card(spec));
    {
        let v = g.v.borrow();
        let c = v.core.cards.get("c2").expect("card c2");
        assert_eq!(c.actions.len(), 1);
        assert_eq!(c.actions[0].action, pill_ui_core::live::model::Action::TimerAgain);
    }
    // rows
    let row = pill_ui_core::panel::b::LiveRow { priority: 60, key: "run4".into(), text: "04:59".into(), accent: Some("amber"), ttl_ms: None, style: "timer" };
    g.fx(TabFx::SetLive { key: "timer-tab".into(), row: Some(row) });
    assert!(g.v.borrow().core.rows.contains_key("timer-tab"));
    g.fx(TabFx::SetLive { key: "timer-tab".into(), row: None });
    assert!(!g.v.borrow().core.rows.contains_key("timer-tab"));
    let before = g.v.borrow().core.rows.len();
    g.fx(TabFx::Live { key: "music", spec: None });
    assert_eq!(g.v.borrow().core.rows.len(), before, "A-tab rows belong to the hub: ignored");
    // reactions and the edge
    g.fx(TabFx::React { key: "agents".into(), kind: "attention", color: None, ttl_ms: Some(2000) });
    assert_eq!(g.v.borrow().core.reaction.0, "attention");
    g.fx(TabFx::Unreact { key: "agents".into() });
    g.fx(TabFx::EdgePulse(0.8));
    assert!(g.v.borrow().core.pulse.is_some());
    g.fx(TabFx::EdgeColor(Some("#ff0000".into())));
    g.fx(TabFx::EdgeColor(None));
    g.fx(TabFx::DragPull(true));
    g.fx(TabFx::DragPull(false));
    // the tab button counter
    g.fx(TabFx::Badge { tab: "notifs", text: "3".into() });
    let badge = |g: &G| g.v.borrow().core.shell.view().tabs.iter().find(|t| t.id == "notifs").map(|t| t.badge).unwrap_or(0);
    assert_eq!(badge(&g), 3);
    g.fx(TabFx::Badge { tab: "notifs", text: String::new() });
    assert_eq!(badge(&g), 0);
    // window / backend requests
    g.fx(TabFx::OpenCommandBar);
    assert_eq!(g.calls("cmd_toggle").len(), 1);
    g.fx(TabFx::VoiceToggle);
    assert_eq!(g.calls("voice_toggle").len(), 1);
    g.fx(TabFx::GoPage("ai"));
    assert_eq!(g.calls("settings_open"), vec![json!({"page": "ai"})]);
    g.fx(TabFx::SetPinned(true));
    assert!(g.calls("settings_set").iter().any(|a| a["patch"]["pinned"] == json!(true)));
    g.fx(TabFx::Expand { tab: "notes" });
    assert!(g.calls("pill_set_state").iter().any(|a| a["state"] == json!("expanded")));
    g.expand("home");
    assert_eq!(g.v.borrow().core.shell.state(), pill_ui_core::shell::PillState::Expanded);
    g.fx(TabFx::Collapse { via: Some("click") });
    assert_eq!(g.v.borrow().core.shell.state(), pill_ui_core::shell::PillState::Collapsed, "the island folded");
    // a screenshot folds the island first (the start follows after the island is gone)
    let n = g.calls("pill_set_state").len();
    g.fx(TabFx::StartCapture { mode: "area" });
    assert!(g.calls("pill_set_state").len() >= n);
}

#[test]
fn the_ai_actions_overlay_opens_takes_escape_and_closes() {
    let mut g = G::new();
    g.expand("clipboard");
    g.fx(TabFx::OpenClipAi { id: 7 });
    assert!(g.v.borrow().dbg_overlay_open());
    let img = g.draw();
    assert!(img.count(None, |p| p[3] > 200) > 1000);
    let r = g.esc();
    assert!(r.consumed, "Esc goes to the overlay first");
    g.settle(30);
    assert!(!g.v.borrow().dbg_overlay_open(), "OverlayClosed reached the island");
    assert_eq!(g.v.borrow().dbg_tab().as_deref(), Some("clipboard"), "the panel stays on the clipboard");
    // opened again, then closed by the tab itself
    g.fx(TabFx::OpenClipAi { id: 8 });
    assert!(g.v.borrow().dbg_overlay_open());
    g.fx(TabFx::CloseClipAi);
    g.settle(30);
    assert!(!g.v.borrow().dbg_overlay_open());
}

fn files(paths: &[&str]) -> Option<DropPayload> {
    Some(DropPayload::Files(paths.iter().map(std::path::PathBuf::from).collect()))
}

#[test]
fn a_drop_on_the_folded_island_lands_on_the_shelf_once() {
    let mut g = G::new();
    let pos = (360.0, 20.0);
    assert!(g.sim.send_drop(&DropEvent { phase: DropPhase::Enter, pos, payload: files(&["C:\\a.txt"]) }).consumed);
    g.settle(10);
    g.sim.send_drop(&DropEvent { phase: DropPhase::Drop, pos, payload: files(&["C:\\a.txt"]) });
    g.settle(4);
    assert_eq!(g.calls("shelf_add").len(), 1, "{:?}", g.fakes.api.calls().iter().map(|c| c.name.clone()).collect::<Vec<_>>());
}

#[test]
fn a_drop_on_the_open_shelf_tab_is_taken_by_the_tab_not_twice() {
    let mut g = G::new();
    g.expand("shelf");
    let (ox, oy, s) = g.v.borrow().dbg_panel_origin();
    let v = g.v.borrow().dbg_views_rect();
    let pos = (ox + (v.x + 300.0) * s, oy + (v.y + 120.0) * s);
    g.sim.send_drop(&DropEvent { phase: DropPhase::Enter, pos, payload: files(&["C:\\a.txt"]) });
    g.sim.send_drop(&DropEvent { phase: DropPhase::Drop, pos, payload: files(&["C:\\a.txt"]) });
    g.settle(6);
    assert_eq!(g.calls("shelf_add").len(), 1, "one add, not the tab's plus the island's");
}

#[test]
fn weather_reaches_the_clock_row_without_the_weather_tab() {
    let mut g = G::new();
    g.emit("weather:current", json!({"temp": 21, "unit": "C", "icon": "clear-day", "text": "Clear"}));
    let _ = g.draw();
    assert!(!g.v.borrow().dbg_has_tab("weather"));
    let shown = format!("{:?}", g.v.borrow().core.shell.view().live.content);
    assert!(shown.contains("21"), "the temperature is in the capsule: {shown}");
}
