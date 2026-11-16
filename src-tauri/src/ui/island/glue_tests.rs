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

fn settings(extra: Value) -> Value {
    let mut s = json!({"size":100,"theme":"dark","clock24":true,"shape":"notch","character":"liquid","pet":"none","pet.side":"right","pet.size":"m",
        "musicPill":"title","net.weather":true,"live.obs":false,"language":"auto"});
    s.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    s
}

impl G {
    fn new() -> G {
        G::with(json!({}))
    }

    /// The island with `extra` on top of the usual test settings.
    fn with(extra: Value) -> G {
        let (svc, fakes): (Services, Fakes) = Services::fake();
        fakes.api.reply("settings_get", Ok(settings(extra)));
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
        self.draw_h(480)
    }

    /// One frame drawn into a window `h` px tall (the folded frame is 120, the open one 480).
    fn draw_h(&mut self, h: u32) -> Image {
        let th = Theme::pill_dark();
        let rect = Rect::new(0.0, 0.0, 720.0, h as f32);
        let (cx, v) = (&self.sim.cx, &self.v);
        H.with(|hn| hn.render(720, h, 1.0, Color::rgba(0.0, 0.0, 0.0, 0.0), |cv| v.borrow_mut().draw(cv, &th, rect, cx)))
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

#[test]
fn the_system_monitor_meters_sit_over_the_controls_tiles() {
    let mut g = G::new();
    g.emit("sysmon:tick", json!({"cpu": 37.0, "ram": {"usedMb": 8192.0, "totalMb": 32768.0}, "gpus": [{"index":0,"name":"RTX","util":55.0,"tempC":61.0,"memUsedMb":4096.0,"memTotalMb":24576.0,"powerW":120.0}]}));
    assert!(g.v.borrow().core.sys_meters.as_ref().is_some_and(|m| m.visible && m.rows.len() >= 3), "the hub made the meters");
    g.expand("controls");
    g.emit("sysmon:tick", json!({"cpu": 42.0, "ram": {"usedMb": 8192.0, "totalMb": 32768.0}, "gpus": []}));
    g.settle(40);
    let img = g.draw();
    let views = g.v.borrow().dbg_views_rect();
    let (ox, oy, s) = g.v.borrow().dbg_panel_origin();
    // the meter row: 44 px under the header, labels and bars are lit pixels
    let row = Rect::new(ox + views.x * s, oy + views.y * s, views.w * s, 44.0 * s);
    let lit = img.count(Some(row), |p| p[3] > 0 && (p[0] > 60 || p[1] > 60 || p[2] > 60));
    assert!(lit > 100, "the meter row is painted: {lit}");
}

#[test]
fn every_motion_settles_so_the_flows_stop_asking_for_frames() {
    use glasskit::input::Buttons;
    let _m = crate::ui::host::testing::motion_on();
    let mut g = G::new();
    // a card comes in and flows to rest (the shell's own edge may keep frames coming while a card is up; the island's flows must not)
    g.emit("capture:done", json!({"kind": "screenshot", "path": "C:\\a\\shot.png", "copied": true, "w": 1920, "h": 1080}));
    assert!(matches!(g.v.borrow().core.shell.view().live.content, pill_ui_core::shell::types::LiveContent::Card(_)), "the card is up");
    g.sim.run(300);
    assert!(!g.v.borrow().dbg_frames_wanted(), "the card's flows have settled");
    // opening the panel and hovering a tab: the highlight eases in (monotone, no jump) and settles
    g.expand("home");
    g.sim.run(200);
    let _ = g.draw();
    let (x, y) = g.v.borrow().dbg_tab_center("clipboard").expect("the clipboard tab is laid out");
    g.sim.send(&InputEvent::MouseMove { pos: (x, y), mods: Modifiers::default(), buttons: Buttons::default() });
    let mut seen = Vec::new();
    for _ in 0..120 {
        g.sim.step_frame();
        seen.push(g.v.borrow().dbg_hover_of("clipboard"));
    }
    assert!(seen[0] < 0.4 && seen.windows(2).all(|w| w[1] >= w[0] - 1e-4) && *seen.last().unwrap() > 0.99, "{seen:?}");
    assert!(!g.v.borrow().dbg_frames_wanted(), "settled");
    // leaving turns the highlight back out smoothly
    g.sim.send(&InputEvent::MouseLeave);
    let mut back = Vec::new();
    for _ in 0..120 {
        g.sim.step_frame();
        back.push(g.v.borrow().dbg_hover_of("clipboard"));
    }
    assert!(back.windows(2).all(|w| w[1] <= w[0] + 1e-4) && *back.last().unwrap() < 0.01, "{back:?}");
    // collapsing hands the strip back and everything rests
    g.esc();
    g.sim.run(300);
    assert!(!g.v.borrow().dbg_frames_wanted());
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The bottom edge (`edge.position` = bottom): the island stands on the window's floor and grows upward.
// ---------------------------------------------------------------------------------------------------------------------------------------

fn bottom_island() -> G {
    G::with(json!({ "edge.position": "bottom" }))
}

/// Lit (opaque) pixels of `img` inside `r`.
fn opaque(img: &Image, r: Rect) -> usize {
    img.count(Some(r), |p| p[3] > 200)
}

#[test]
fn the_folded_bottom_island_stands_on_the_window_floor_and_the_top_one_hangs_from_the_ceiling() {
    let mut g = bottom_island();
    let img = g.draw_h(120);
    assert!(g.v.borrow().at_bottom());
    assert!(opaque(&img, Rect::new(0.0, 60.0, 720.0, 60.0)) > 800, "the body is in the lower half");
    assert_eq!(opaque(&img, Rect::new(0.0, 0.0, 720.0, 50.0)), 0, "nothing in the upper part");
    let body = g.v.borrow().dbg_body_win();
    assert!(body.bottom() <= 120.0 && body.bottom() > 100.0 && body.y > 60.0, "{body:?}");
    // the same window with the top edge: mirrored
    let mut t = G::new();
    let img = t.draw_h(120);
    assert!(opaque(&img, Rect::new(0.0, 0.0, 720.0, 60.0)) > 800);
    assert_eq!(opaque(&img, Rect::new(0.0, 70.0, 720.0, 50.0)), 0);
    let tb = t.v.borrow().dbg_body_win();
    assert!((tb.y - 0.0).abs() < 9.0 && (tb.h - body.h).abs() < 0.01, "{tb:?}");
    // the click area is the body and its liquid strip, on the same side
    let hit = g.v.borrow().dbg_hit();
    assert_eq!(hit.len(), 1);
    assert!(hit[0].y > 40.0 && (hit[0].bottom() - body.bottom()).abs() < 0.5, "{:?} vs {body:?}", hit[0]);
    assert!(hit[0].h > body.h, "the liquid strip is part of it");
    // the host is told which screen row to watch for the edge reveal
    assert_eq!(g.sim.take_fx().edge_bottom, Some(true));
    assert_eq!(t.sim.take_fx().edge_bottom, Some(false));
}

#[test]
fn the_open_bottom_island_has_its_header_at_the_floor_and_the_content_above_it() {
    let mut g = bottom_island();
    g.expand("home");
    let _ = g.draw();
    let body = g.v.borrow().dbg_body_win();
    assert!(body.bottom() > 460.0 && body.y < 150.0, "the panel grew upward from the floor: {body:?}");
    let views = g.v.borrow().dbg_views_rect();
    let (ox, oy, s) = g.v.borrow().dbg_panel_origin();
    assert!((oy - body.y).abs() < 0.01);
    let (tx, ty) = g.v.borrow().dbg_tab_center("home").expect("laid out");
    assert!(ty > body.bottom() - 60.0 && ty < body.bottom(), "the tab strip is the lowest thing: {ty} in {body:?}");
    assert!(oy + (views.bottom()) * s < ty, "the tab view ends above the header");
    assert!((views.y - 11.0).abs() < 0.01, "{views:?}");
    // pixels: the tab view is painted above the header, the header strip is lit
    let img = g.draw();
    let r = Rect::new(ox + views.x * s, oy + views.y * s, views.w * s, views.h * s);
    assert!(img.count(Some(r), |p| p[3] > 0 && (p[0] > 30 || p[1] > 30 || p[2] > 30)) > 150, "the home tab draws");
    // a click on a header tab (mirrored hit test) switches the tab
    let clip = g.v.borrow().dbg_tab_center("clipboard").expect("clipboard tab");
    click_at(&mut g, clip);
    assert_eq!(g.v.borrow().dbg_tab().as_deref(), Some("clipboard"));
    let _ = tx;
    // folding brings it back to the floor and everything rests
    g.esc();
    g.sim.run(400);
    let _ = g.draw_h(120);
    assert!(g.v.borrow().dbg_body_win().y > 60.0);
    assert!(!g.v.borrow().dbg_frames_wanted());
}

#[test]
fn the_open_top_island_is_unchanged_header_first() {
    let mut g = G::new();
    g.expand("home");
    let _ = g.draw();
    let (_, ty) = g.v.borrow().dbg_tab_center("home").expect("laid out");
    let body = g.v.borrow().dbg_body_win();
    assert!(ty < body.y + 60.0, "the header is at the top of the body: {ty} in {body:?}");
    assert!(body.y < 12.0, "{body:?}");
}

/// Steps frames (drawing each, so the window size is known) until `done`, at most `max`; the number of frames.
fn frames_until(g: &mut G, max: usize, mut each: impl FnMut(&G), done: impl Fn(&G) -> bool) -> usize {
    for i in 0..max {
        if done(g) {
            return i;
        }
        g.sim.step_frame();
        let _ = g.draw_h(120);
        each(g);
    }
    max
}

#[test]
fn changing_the_edge_slides_out_moves_the_window_unseen_and_slides_in() {
    let _m = crate::ui::host::testing::motion_on();
    let mut g = G::new();
    let _ = g.draw_h(120);
    let top_body = g.v.borrow().dbg_body_win();
    assert!(g.calls("pill_edge").is_empty());
    g.emit("settings:changed", json!({ "edge.position": "bottom" }));
    // the very first moment: still at the top, nothing has jumped, the window has not been asked to move
    assert!(!g.v.borrow().at_bottom() && g.calls("pill_edge").is_empty());
    let mut ys = vec![];
    let mut swapped: Option<(Rect, Rect)> = None;
    let mut last = g.v.borrow().dbg_body_win();
    for _ in 0..400 {
        g.sim.step_frame();
        let _ = g.draw_h(120);
        let now = g.v.borrow().dbg_body_win();
        if swapped.is_none() && g.v.borrow().at_bottom() {
            swapped = Some((last, now));
        }
        ys.push(now.y);
        last = now;
    }
    let (before, after) = swapped.expect("the island took the bottom edge");
    // it left through the top edge and arrived through the bottom: the swap happens with the body out of the window both times
    assert!(before.bottom() <= 0.0, "slid fully out through the top before the window moved: {before:?}");
    assert!(after.y >= 120.0, "and starts below the new window's floor: {after:?}");
    assert_eq!(g.calls("pill_edge"), vec![json!({ "bottom": true, "align": "center" })], "the window is asked once");
    assert!(ys.iter().cloned().fold(f32::MAX, f32::min) < top_body.y - 20.0, "it really moved out first");
    let end = g.v.borrow().dbg_body_win();
    assert!(end.bottom() <= 120.0 && end.y > 60.0, "settled on the floor: {end:?}");
    assert!(!g.v.borrow().dbg_frames_wanted() && g.v.borrow().dbg_tuck_px().abs() < 0.2, "and rests (the window may park again)");
    assert!(!g.v.borrow().dbg_sliding(), "the edge change asks for no more frames once it has landed");
    // before the swap the body only moved toward the top edge; after it only toward the floor
    let swap_i = ys.iter().position(|y| *y >= 120.0).unwrap();
    assert!(ys[..swap_i].windows(2).all(|w| w[1] <= w[0] + 0.01), "out: {:?}", &ys[..swap_i]);
    assert_eq!(g.sim.take_fx().edge_bottom, Some(true));
    // and back to the top
    g.emit("settings:changed", json!({ "edge.position": "top" }));
    g.sim.run(400);
    let _ = g.draw_h(120);
    assert!(!g.v.borrow().at_bottom());
    assert_eq!(g.calls("pill_edge"), vec![json!({ "bottom": true, "align": "center" }), json!({ "bottom": false, "align": "center" })]);
    assert!(g.v.borrow().dbg_body_win().y < 12.0);
}

#[test]
fn the_left_corner_island_stands_against_the_left_side_and_grows_up() {
    use crate::ui::host::geom::Align;
    let mut g = G::with(json!({ "edge.position": "bottom", "edge.align": "left" }));
    let img = g.draw_h(120);
    assert_eq!(g.v.borrow().dbg_align(), Align::Left);
    let body = g.v.borrow().dbg_body_win();
    assert!((body.x - 20.0).abs() < 1.0, "the folded body stands on the 20 px pad: {body:?}");
    assert!(body.bottom() > 100.0 && body.y > 60.0, "and on the floor: {body:?}");
    assert!(opaque(&img, Rect::new(0.0, 60.0, 360.0, 60.0)) > 800 && opaque(&img, Rect::new(380.0, 0.0, 340.0, 120.0)) == 0, "only the left part of the window is painted");
    let hit = g.v.borrow().dbg_hit();
    assert!(hit[0].x >= 19.0 && hit[0].right() < 380.0, "the click area follows the body: {:?}", hit[0]);
    // the pointer is mapped back into the shell's centred layout
    let (sx, _) = g.v.borrow().dbg_shell_pt((body.cx(), body.cy()));
    let sb = g.v.borrow().core.shell.view().body;
    assert!((sx - (sb.left + sb.width / 2.0)).abs() < 0.5, "{sx} vs {}", sb.left + sb.width / 2.0);
    // the host is told about the side too, and the open panel (already 20 px in) is where it always was
    let fx = g.sim.take_fx();
    assert_eq!((fx.edge_bottom, fx.edge_align), (Some(true), Some(Align::Left)));
    g.expand("home");
    let _ = g.draw();
    let open = g.v.borrow().dbg_body_win();
    assert!((open.x - 20.0).abs() < 1.0 && open.bottom() > 460.0, "{open:?}");
    // a click on a tab still lands on it
    let clip = g.v.borrow().dbg_tab_center("clipboard").expect("clipboard tab");
    click_at(&mut g, clip);
    assert_eq!(g.v.borrow().dbg_tab().as_deref(), Some("clipboard"));
    // the top edge ignores the setting: the island stays in the middle
    let mut t = G::with(json!({ "edge.position": "top", "edge.align": "left" }));
    let _ = t.draw_h(120);
    assert_eq!(t.v.borrow().dbg_align(), Align::Center);
    let tb = t.v.borrow().dbg_body_win();
    assert!((tb.cx() - 360.0).abs() < 1.0, "{tb:?}");
}

#[test]
fn the_right_corner_island_stands_against_the_right_side() {
    let mut g = G::with(json!({ "edge.position": "bottom", "edge.align": "right" }));
    let _ = g.draw_h(120);
    let body = g.v.borrow().dbg_body_win();
    assert!((720.0 - body.right() - 20.0).abs() < 1.0, "{body:?}");
    let sx = g.v.borrow().dbg_shell_pt((body.cx(), body.cy())).0;
    let sb = g.v.borrow().core.shell.view().body;
    assert!((sx - (sb.left + sb.width / 2.0)).abs() < 0.5);
}

#[test]
fn changing_the_side_slides_out_asks_for_the_corner_and_slides_in() {
    use crate::ui::host::geom::Align;
    let _m = crate::ui::host::testing::motion_on();
    let mut g = bottom_island();
    let _ = g.draw_h(120);
    let centre = g.v.borrow().dbg_body_win();
    g.emit("settings:changed", json!({ "edge.align": "left" }));
    assert_eq!(g.v.borrow().dbg_align(), Align::Center, "nothing jumps: the slide comes first");
    assert!(g.calls("pill_edge").is_empty());
    let mut lowest = 0.0f32;
    for _ in 0..400 {
        g.sim.step_frame();
        let _ = g.draw_h(120);
        lowest = lowest.max(g.v.borrow().dbg_body_win().y);
    }
    assert!(lowest >= 120.0, "the body left through the floor before the window moved: {lowest}");
    assert_eq!(g.calls("pill_edge"), vec![json!({ "bottom": true, "align": "left" })]);
    let end = g.v.borrow().dbg_body_win();
    assert!((end.x - 20.0).abs() < 1.0 && end.x < centre.x - 50.0 && end.bottom() <= 120.0 && end.y > 60.0, "settled in the corner: {end:?}");
    assert!(!g.v.borrow().dbg_sliding());
    // leaving the bottom edge brings the middle back whatever the side said
    g.emit("settings:changed", json!({ "edge.position": "top" }));
    g.sim.run(400);
    let _ = g.draw_h(120);
    assert_eq!(g.v.borrow().dbg_align(), Align::Center);
    assert_eq!(g.calls("pill_edge").last(), Some(&json!({ "bottom": false, "align": "center" })));
}

#[test]
fn the_backend_hears_where_the_body_stands_in_a_corner() {
    let mut g = G::with(json!({ "edge.position": "bottom", "edge.align": "left" }));
    let _ = g.draw_h(120);
    g.settle(10);
    let rects = g.calls("pill_rect");
    let last = rects.last().expect("the shell reported its body");
    assert!((last["x"].as_f64().unwrap() - 20.0).abs() < 1.0, "the edge reveal watches the corner, not the middle: {last}");
}

#[test]
fn with_reduced_motion_the_edge_changes_at_once() {
    let mut g = G::new();
    g.sim.cx.reduced_motion = true;
    let _ = g.draw_h(120);
    g.emit("settings:changed", json!({ "edge.position": "bottom" }));
    g.sim.run(20);
    let _ = g.draw_h(120);
    assert!(g.v.borrow().at_bottom(), "no slide");
    assert_eq!(g.calls("pill_edge"), vec![json!({ "bottom": true, "align": "center" })]);
    assert!(g.v.borrow().dbg_tuck_px() == 0.0 && g.v.borrow().dbg_body_win().y > 60.0);
}

#[test]
fn a_hidden_island_takes_the_new_edge_without_a_slide() {
    let mut g = G::new();
    let _ = g.draw_h(120);
    g.emit("pill:hide", json!({ "reason": "guard" }));
    g.emit("settings:changed", json!({ "edge.position": "bottom" }));
    assert!(g.v.borrow().at_bottom(), "nothing is on screen, so nothing to animate");
    assert_eq!(g.calls("pill_edge"), vec![json!({ "bottom": true, "align": "center" })]);
}

#[test]
fn the_pointer_is_mirrored_for_the_bottom_island() {
    use glasskit::input::Buttons;
    let mut g = bottom_island();
    let _ = g.draw_h(480);
    let body = g.v.borrow().dbg_body_win();
    // the liquid edge reaches for a pointer that approaches from above: the shell sees it above its body, i.e. below the shell's bottom edge
    let pos = (body.cx(), body.y - 6.0);
    g.sim.send(&InputEvent::MouseMove { pos, mods: Modifiers::default(), buttons: Buttons::default() });
    let (sx, sy) = g.v.borrow().dbg_shell_pt(pos);
    let sb = g.v.borrow().core.shell.view().body;
    assert!((sx - body.cx() as f64).abs() < 0.01 && sy > sb.top + sb.height, "the pointer is below the shell's body: {sy} vs {}", sb.top + sb.height);
    assert!(g.v.borrow().dbg_hit()[0].contains(pos.0, pos.1 + 3.0) || g.v.borrow().dbg_hit()[0].y <= body.y);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The phone notification card (phonecard.rs): the folded island grows on hover, Hide / Mark read / Reply work, the island settles.
// ---------------------------------------------------------------------------------------------------------------------------------------

fn phone_notif(key: &str) -> Value {
    json!({ "kind": "posted", "notif": { "key": key, "pkg": "org.telegram.messenger", "app": "Telegram", "title": "Вадим", "text": "привет, как дела?", "ongoing": false, "replay": false, "fresh": true, "canReply": true, "avatar": null, "icon": null } })
}

fn move_to(g: &mut G, p: (f32, f32)) {
    g.sim.send(&InputEvent::MouseMove { pos: p, mods: Modifiers::default(), buttons: glasskit::input::Buttons::NONE });
    for _ in 0..3 {
        let _ = g.draw();
        g.settle(30);
    }
}

fn click_at(g: &mut G, p: (f32, f32)) {
    g.sim.send(&InputEvent::MouseDown { pos: p, button: glasskit::input::MouseButton::Left, mods: Modifiers::default(), clicks: 1 });
    g.sim.send(&InputEvent::MouseUp { pos: p, button: glasskit::input::MouseButton::Left, mods: Modifiers::default() });
    let _ = g.draw();
    g.settle(30);
}

/// Moves the simulated clock on and lets the island notice (its own timers run on the sim clock).
fn advance(g: &mut G, ms: f64) {
    g.sim.now_ms += ms;
    g.sim.frame.request();
    g.settle(6);
}

fn card_title(g: &G) -> Option<String> {
    g.v.borrow().core.shell.card_view().map(|c| c.spec.title)
}

fn with_phone_card() -> G {
    let mut g = G::new();
    g.emit("phone:notif", phone_notif("k1"));
    for _ in 0..3 {
        let _ = g.draw();
        g.settle(40);
    }
    g
}

#[test]
fn a_phone_message_is_a_card_that_grows_on_hover_and_answers() {
    let _m = crate::ui::host::testing::motion_on();
    let mut g = with_phone_card();
    assert_eq!(card_title(&g).as_deref(), Some("Вадим · Telegram"), "the Telegram-style card is up");
    let h0 = g.v.borrow().dbg_body_h();
    let pts = g.v.borrow().dbg_phone_points().expect("the card was laid out");
    move_to(&mut g, pts[4]);
    let h1 = g.v.borrow().dbg_body_h();
    assert!(h1 > h0 + 30.0, "the island grew to show the buttons and the field: {h0} -> {h1}");
    let pts = g.v.borrow().dbg_phone_points().unwrap();
    // Mark read runs the card's action and the card goes
    click_at(&mut g, pts[1]);
    let acts = g.calls("phone_notif_action");
    assert_eq!(acts, vec![json!({ "key": "k1", "action": "read" })]);
}

#[test]
fn typing_a_reply_sends_it_and_the_result_card_replaces_the_message() {
    let _m = crate::ui::host::testing::motion_on();
    let mut g = with_phone_card();
    let pts = g.v.borrow().dbg_phone_points().unwrap();
    move_to(&mut g, pts[4]);
    let pts = g.v.borrow().dbg_phone_points().unwrap();
    let _ = g.sim.take_fx();
    click_at(&mut g, pts[2]); // the field
    assert_eq!(g.sim.take_fx().focus, Some(true), "the keyboard was taken");
    for ch in "ок".chars() {
        g.sim.send(&InputEvent::Char(ch));
        let _ = g.draw();
        g.settle(4);
    }
    g.sim.send(&InputEvent::Key { code: KeyCode::Enter, mods: Modifiers::default(), down: true, repeat: false });
    for _ in 0..3 {
        let _ = g.draw();
        g.settle(30);
    }
    assert_eq!(g.calls("phone_notif_action"), vec![json!({ "key": "k1", "action": "reply", "text": "ок" })]);
    // the phone confirms: the same card id now says "Reply sent", then the clock returns
    g.emit("phone:notif", json!({ "kind": "reply", "key": "k1", "ok": true }));
    assert_eq!(card_title(&g).as_deref(), Some("Reply sent"));
    assert_eq!(g.sim.take_fx().focus, Some(false), "the keyboard went back");
}

#[test]
fn the_phone_card_stays_while_hovered_and_the_island_goes_quiet_afterwards() {
    let _m = crate::ui::host::testing::motion_on();
    let mut g = with_phone_card();
    let pts = g.v.borrow().dbg_phone_points().unwrap();
    move_to(&mut g, pts[4]);
    // a hover holds the card well past its 7 s
    advance(&mut g, 20_000.0);
    assert!(card_title(&g).is_some(), "still there while the pointer rests on it");
    g.sim.send(&InputEvent::MouseLeave);
    g.settle(10);
    advance(&mut g, 3_500.0);
    for _ in 0..4 {
        let _ = g.draw();
        g.settle(40);
    }
    assert!(card_title(&g).is_none(), "gone a few seconds after the pointer left");
    assert!(!g.v.borrow().dbg_frames_wanted(), "nothing keeps asking for frames");
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// In the taskbar (`edge.position` = taskbar): the folded island stands centred in the bar, the open one floats above it. The window is the
// frame plus the bar's height (`pill.rs`): 120 + 48 folded, 480 + 48 open, 200 + 48 with a card; `pill:taskbar` says how tall the bar is.
// ---------------------------------------------------------------------------------------------------------------------------------------

const BAR: f32 = 48.0;
const FOLDED_WIN: u32 = 120 + BAR as u32;
const OPEN_WIN: u32 = 480 + BAR as u32;

/// Frames drawn into a window `h` px tall until the island rests; the last picture.
fn rest_in(g: &mut G, h: u32) -> Image {
    let mut img = g.draw_h(h);
    for _ in 0..8 {
        g.settle(60);
        img = g.draw_h(h);
    }
    img
}

/// The island standing in a 48 px bar at the left, drawn at rest in its folded window.
fn bar_island(extra: Value) -> G {
    let mut s = json!({ "edge.position": "taskbar", "edge.align": "left" });
    s.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let mut g = G::with(s);
    let _ = g.draw_h(FOLDED_WIN);
    g.emit("pill:taskbar", json!({ "h": BAR }));
    let _ = rest_in(&mut g, FOLDED_WIN);
    g
}

/// Opens the island (a window `OPEN_WIN` tall, as `pill.rs` makes it) and lets it settle.
fn open_in_bar(g: &mut G, tab: &str) {
    g.emit("pill:expand", json!({ "tab": tab, "via": "test" }));
    let _ = rest_in(g, OPEN_WIN);
}

/// How many opaque pixels `img` has in a 3 x 3 square whose bottom-left corner is the bottom-left of `body`: 9 = a square corner, ~3 = a round one.
fn corner_px(img: &Image, body: Rect) -> usize {
    opaque(img, Rect::new(body.x, body.bottom() - 3.0, 3.0, 3.0))
}

#[test]
fn a_taskbar_island_with_no_bar_is_the_plain_bottom_island() {
    let mut g = G::with(json!({ "edge.position": "taskbar" }));
    let img = g.draw_h(120);
    let (in_bar, lift, ..) = g.v.borrow().dbg_bar();
    assert!(g.v.borrow().at_bottom() && !g.v.borrow().in_bar() && !in_bar && lift == 0.0, "it mirrors like the bottom edge, nothing is lifted");
    let body = g.v.borrow().dbg_body_win();
    assert!(body.bottom() > 119.0 && body.y > 60.0, "{body:?}");
    assert!(corner_px(&img, body) >= 9, "flat against the floor, as at the bottom edge: {}", corner_px(&img, body));
    assert_eq!(g.sim.take_fx().edge_bottom, Some(true), "the host watches the bottom row");
    // a bar of height 0 (auto-hidden, or on the top) is the same
    g.emit("pill:taskbar", json!({ "h": 0 }));
    assert!(!g.v.borrow().in_bar());
}

#[test]
fn the_folded_island_stands_centred_in_the_bar_round_on_every_side() {
    let mut g = bar_island(json!({}));
    let img = rest_in(&mut g, FOLDED_WIN);
    assert!(g.v.borrow().in_bar());
    let body = g.v.borrow().dbg_body_win();
    let gap = FOLDED_WIN as f32 - body.bottom();
    assert!((body.h - 30.0).abs() < 0.6, "a plain capsule: {body:?}");
    assert!((gap - (BAR - body.h) / 2.0).abs() < 0.6, "the same room under and over it in the bar: {gap} for {body:?}");
    assert!(body.y >= FOLDED_WIN as f32 - BAR, "inside the bar strip: {body:?}");
    assert!((body.x - 20.0).abs() < 1.0, "left side: on the pad: {body:?}");
    // a pill in the bar, not a notch of the screen edge: round at the floor side as well, and no flares at its feet
    assert!(corner_px(&img, body) <= 5, "round at the bottom: {}", corner_px(&img, body));
    assert!(opaque(&img, Rect::new(0.0, 0.0, 720.0, FOLDED_WIN as f32 - BAR - 4.0)) < 40, "nothing above the bar but the liquid edge's hint");
    // the click area follows the body
    let hit = g.v.borrow().dbg_hit();
    assert!(hit[0].contains(body.cx(), body.cy()) && hit[0].x >= 19.0, "{:?} vs {body:?}", hit[0]);
    // and the pointer is mapped back through the lift
    let (_, sy) = g.v.borrow().dbg_shell_pt((body.cx(), body.cy()));
    let sb = g.v.borrow().core.shell.view().body;
    assert!((sy - (sb.top + sb.height / 2.0)).abs() < 0.6, "{sy} in {sb:?}");
    // at rest: no frames asked for, nothing moving
    assert!(!g.v.borrow().dbg_bar_moving() && !g.v.borrow().dbg_frames_wanted(), "at rest the bar placement asks for no frames");
}

#[test]
fn a_bigger_island_is_cut_down_to_fit_the_bar_and_gets_its_size_back() {
    let win = (120.0 * 2.0 + BAR) as u32; // size 200 %: the frame doubles, the bar does not
    let mut g = G::with(json!({ "edge.position": "taskbar", "edge.align": "left", "size": 200 }));
    let _ = g.draw_h(win);
    let tall = g.v.borrow().dbg_body_win().h;
    assert!((tall - 60.0).abs() < 0.6, "twice the capsule when it stands on a plain edge: {tall}");
    g.emit("pill:taskbar", json!({ "h": BAR }));
    let _ = rest_in(&mut g, win);
    let body = g.v.borrow().dbg_body_win();
    assert!((body.h - 40.0).abs() < 0.6, "the bar leaves 4 px above and below: {body:?}");
    assert!(body.y >= win as f32 - BAR - 0.5 && body.bottom() <= win as f32 + 0.01, "inside the bar: {body:?}");
    // the bar goes away (auto-hide): the island is the plain bottom island again, full size
    g.emit("pill:taskbar", json!({ "h": 0 }));
    let _ = rest_in(&mut g, win);
    let body = g.v.borrow().dbg_body_win();
    assert!((body.h - 60.0).abs() < 0.6 && body.bottom() > win as f32 - 0.6, "back to the floor and full size: {body:?}");
    assert_eq!(g.v.borrow().dbg_bar().1, 0.0);
}

#[test]
fn a_thin_bar_is_not_outgrown_and_the_open_island_never_covers_it() {
    for bar in [40.0f32, 52.0] {
        let mut g = G::with(json!({ "edge.position": "taskbar", "edge.align": "left" }));
        let (folded, open) = (120 + bar as u32, 480 + bar as u32);
        let _ = g.draw_h(folded);
        g.emit("pill:taskbar", json!({ "h": bar }));
        let _ = rest_in(&mut g, folded);
        let b = g.v.borrow().dbg_body_win();
        assert!(b.y >= folded as f32 - bar && b.bottom() <= folded as f32 + 0.01, "{bar} px bar: {b:?}");
        g.emit("pill:expand", json!({ "tab": "home", "via": "test" }));
        let _ = rest_in(&mut g, open);
        let o = g.v.borrow().dbg_body_win();
        assert!(o.bottom() <= open as f32 - bar, "{bar} px bar: the panel stands above the bar: {o:?}");
        assert!(o.y >= 0.0, "and is not cut off at the top of the window: {o:?}");
    }
}

#[test]
fn the_open_island_floats_above_the_bar_with_its_header_at_the_bottom() {
    let mut g = bar_island(json!({}));
    open_in_bar(&mut g, "home");
    let (in_bar, lift, w, raised) = g.v.borrow().dbg_bar();
    assert!(in_bar && w > 0.99 && raised > 0.99 && lift > 40.0, "{lift} {w} {raised}");
    let body = g.v.borrow().dbg_body_win();
    assert!((body.bottom() - (OPEN_WIN as f32 - BAR - 8.0)).abs() < 1.5, "8 px above the bar: {body:?}");
    assert!(body.h > 200.0 && body.y >= 0.0, "{body:?}");
    let (_, ty) = g.v.borrow().dbg_tab_center("home").expect("laid out");
    assert!(ty > body.bottom() - 60.0 && ty < body.bottom(), "the tab strip is the lowest thing: {ty} in {body:?}");
    assert!((body.x - 20.0).abs() < 1.0, "{body:?}");
    // a click on a tab lands on it
    let clip = g.v.borrow().dbg_tab_center("clipboard").expect("clipboard tab");
    let _ = g.draw_h(OPEN_WIN);
    click_at_h(&mut g, clip, OPEN_WIN);
    assert_eq!(g.v.borrow().dbg_tab().as_deref(), Some("clipboard"));
}

/// `click_at` for a window of another height.
fn click_at_h(g: &mut G, p: (f32, f32), h: u32) {
    g.sim.send(&InputEvent::MouseDown { pos: p, button: glasskit::input::MouseButton::Left, mods: Modifiers::default(), clicks: 1 });
    g.sim.send(&InputEvent::MouseUp { pos: p, button: glasskit::input::MouseButton::Left, mods: Modifiers::default() });
    let _ = g.draw_h(h);
    g.settle(30);
}

#[test]
fn opening_and_folding_glide_between_the_bar_and_the_float_without_a_jump() {
    let _m = crate::ui::host::testing::motion_on();
    let mut g = bar_island(json!({}));
    let rest = g.v.borrow().dbg_body_win().bottom();
    // the window is already the open frame (pill.rs enlarges it first), so only the body moves
    let _ = g.draw_h(OPEN_WIN);
    let rest = rest + (OPEN_WIN - FOLDED_WIN) as f32;
    g.emit("pill:expand", json!({ "tab": "home", "via": "test" }));
    let mut path = vec![];
    for _ in 0..200 {
        let _ = g.draw_h(OPEN_WIN);
        g.sim.step_frame();
        path.push(g.v.borrow().dbg_body_win().bottom());
    }
    let end = OPEN_WIN as f32 - BAR - 8.0;
    assert!((path[0] - rest).abs() < 12.0 && (path.last().unwrap() - end).abs() < 1.5, "{} -> {} (rest {rest}, float {end})", path[0], path.last().unwrap());
    let worst = path.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(worst < 14.0, "no frame jumps: the largest step is {worst}");
    assert!(path.iter().all(|b| *b <= rest + 1.0 && *b >= end - 3.0), "it rises between the two and does not dip below the bar");
    // and back
    g.esc();
    let mut back = vec![];
    for _ in 0..300 {
        let _ = g.draw_h(OPEN_WIN);
        g.sim.step_frame();
        back.push(g.v.borrow().dbg_body_win().bottom());
    }
    let worst = back.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(worst < 14.0, "folding: the largest step is {worst}");
    assert!((back.last().unwrap() - rest).abs() < 0.8, "settled in the bar again: {} vs {rest}", back.last().unwrap());
    assert!(!g.v.borrow().dbg_bar_moving());
}

#[test]
fn an_event_card_floats_above_the_bar_and_the_island_sinks_back_into_it() {
    let _m = crate::ui::host::testing::motion_on();
    let win = 200 + BAR as u32;
    let mut g = bar_island(json!({}));
    g.emit("phone:notif", phone_notif("k1"));
    for _ in 0..3 {
        let _ = g.draw_h(win);
        g.settle(40);
    }
    assert!(card_title(&g).is_some(), "the card is up");
    let (_, _, _, raised) = g.v.borrow().dbg_bar();
    let b = g.v.borrow().dbg_body_win();
    assert!(raised > 0.99 && b.bottom() <= win as f32 - BAR + 0.5, "above the bar: {b:?} raised {raised}");
    assert!((b.bottom() - (win as f32 - BAR - 8.0)).abs() < 1.5, "the same 8 px above the bar as the open panel (a card rests at another translateY): {b:?}");
    assert!(b.h > 30.0 && b.y >= 0.0, "{b:?}");
    advance(&mut g, 12_000.0);
    for _ in 0..4 {
        let _ = g.draw_h(win);
        g.settle(40);
    }
    let _ = rest_in(&mut g, win);
    assert!(card_title(&g).is_none(), "the card is gone");
    let b = g.v.borrow().dbg_body_win();
    assert!((win as f32 - b.bottom() - (BAR - b.h) / 2.0).abs() < 0.8, "centred in the bar again: {b:?}");
}

#[test]
fn with_reduced_motion_the_island_takes_its_place_in_the_bar_at_once() {
    let mut g = G::with(json!({ "edge.position": "taskbar", "edge.align": "left" }));
    g.sim.cx.reduced_motion = true;
    let _ = g.draw_h(FOLDED_WIN);
    g.emit("pill:taskbar", json!({ "h": BAR }));
    g.sim.run(3);
    let _ = g.draw_h(FOLDED_WIN);
    let b = g.v.borrow().dbg_body_win();
    assert!((FOLDED_WIN as f32 - b.bottom() - (BAR - b.h) / 2.0).abs() < 0.8, "{b:?}");
    assert!(!g.v.borrow().dbg_bar_moving());
}

#[test]
fn changing_to_the_taskbar_slides_out_and_asks_for_the_bar_placement() {
    let _m = crate::ui::host::testing::motion_on();
    let mut g = G::new();
    let _ = g.draw_h(120);
    g.emit("settings:changed", json!({ "edge.position": "taskbar" }));
    assert!(!g.v.borrow().at_bottom() && g.calls("pill_edge").is_empty(), "nothing jumps: the slide comes first");
    g.sim.run(400);
    let _ = g.draw_h(FOLDED_WIN);
    assert!(g.v.borrow().at_bottom());
    assert_eq!(g.calls("pill_edge"), vec![json!({ "bottom": true, "align": "center", "bar": true })], "the window is asked once, with the bar flag");
    // pill.rs answers with the bar it found
    g.emit("pill:taskbar", json!({ "h": BAR }));
    let _ = rest_in(&mut g, FOLDED_WIN);
    let b = g.v.borrow().dbg_body_win();
    assert!(g.v.borrow().in_bar() && b.y >= FOLDED_WIN as f32 - BAR && b.bottom() <= FOLDED_WIN as f32, "settled in the bar: {b:?}");
    // from the taskbar to the plain bottom edge: no bar flag, the body lands on the floor
    g.emit("settings:changed", json!({ "edge.position": "bottom" }));
    g.sim.run(400);
    let _ = g.draw_h(120);
    assert_eq!(g.calls("pill_edge").last(), Some(&json!({ "bottom": true, "align": "center" })));
    assert!(!g.v.borrow().in_bar());
    g.emit("pill:taskbar", json!({ "h": 0 }));
    let _ = rest_in(&mut g, 120);
    assert!(g.v.borrow().dbg_body_win().bottom() > 119.0, "flat on the floor again");
}

#[test]
fn the_island_publishes_its_real_folded_width_while_it_stands_in_the_bar() {
    let mut g = bar_island(json!({}));
    let w0 = g.v.borrow().dbg_body_win().w.round() as i32;
    assert!(w0 > 100 && g.v.borrow().dbg_fold_w() == w0, "the clock capsule's width: {w0} vs {}", g.v.borrow().dbg_fold_w());
    // the live row grows the capsule: the new width goes out, not the old ceiling
    g.emit("media:changed", media_payload());
    let _ = rest_in(&mut g, FOLDED_WIN);
    let w1 = g.v.borrow().dbg_body_win().w.round() as i32;
    assert!(w1 != w0 && g.v.borrow().dbg_fold_w() == w1, "{w0} -> {w1}, published {}", g.v.borrow().dbg_fold_w());
    // an open island floats: the folded width is kept (the widget keeps its place while the panel is up)
    open_in_bar(&mut g, "home");
    assert_eq!(g.v.borrow().dbg_fold_w(), w1);
    // and when the bar goes away there is nothing to keep clear of
    g.emit("pill:taskbar", json!({ "h": 0 }));
    let _ = rest_in(&mut g, OPEN_WIN);
    assert_eq!(g.v.borrow().dbg_fold_w(), 0);
}
