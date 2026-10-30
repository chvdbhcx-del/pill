//! Integration tests of the whole `Shell` model: state-machine transitions, timers, cards, live rows, clock/weather, reactions, settings.

use serde_json::{json, Value};

use super::capsule::{PillState, Shell};
use super::cards::{CardAction, CardSpec};
use super::clock::{CharSide, ClockSlots, SlotItem};
use super::io::{ManualClock, RecBackend};
use super::lives::LiveInput;
use super::react::{Preset, ReactOpts};
use super::tabs::TabSpec;
use super::types::{Env, Event, LiveContent, Notice, Phase};

const FRAME_MS: f64 = 1000.0 / 60.0;

struct H {
    sh: Shell<RecBackend, ManualClock>,
    t: f64,
    wall: f64,
}

impl H {
    fn new() -> H {
        H::with(json!({}))
    }
    /// A shell after `new` + `Start(settings)` with a measured 140 x 30 clock row and the stage at the expanded frame width.
    fn with(settings: Value) -> H {
        let clock = ManualClock::default();
        let sh = Shell::new(RecBackend::default(), clock, "en-US", 42);
        let mut h = H { sh, t: 5000.0, wall: clock.wall_ms as f64 };
        h.sh.handle(Event::Env(Env { inner_w: 720.0, ..Env::default() }));
        h.sh.handle(Event::LiveMeasured { w: 140.0, h: 30.0 });
        h.sh.handle_at(h.t, Event::Show); // snap to the measured row
        h.sh.handle_at(h.t, Event::Start(settings));
        h.run(100.0);
        h.sh.backend_mut().clear();
        h.sh.take_notices();
        h
    }
    fn run(&mut self, ms: f64) {
        let mut left = ms;
        while left > 0.0 {
            let d = FRAME_MS.min(left);
            self.t += d;
            self.wall += d;
            self.sh.clock_mut().wall_ms = self.wall as i64;
            self.sh.tick(d, self.t);
            left -= d;
        }
    }
    fn ev(&mut self, e: Event) {
        self.sh.handle_at(self.t, e);
    }
    fn calls(&self, cmd: &str) -> usize {
        self.sh.backend().count(cmd)
    }
    fn last(&self, cmd: &str) -> Value {
        self.sh.backend().last(cmd).cloned().unwrap_or(Value::Null)
    }
    fn expand(&mut self) {
        self.ev(Event::Expand { tab: None, section: None });
        self.run(1500.0);
    }
    fn settle(&mut self) {
        self.run(2500.0);
    }
}

fn card(id: &str) -> CardSpec {
    CardSpec { id: Some(id.into()), icon: "bolt".into(), title: format!("title {id}"), sub: "sub".into(), ..Default::default() }
}

fn tab(id: &str, order: i32) -> TabSpec {
    TabSpec { id: id.into(), title: id.into(), order, hidden: false, height: None }
}

// ---------------- startup, geometry, pill_rect ----------------

#[test]
fn startup_collapsed_capsule_geometry_and_pill_rect() {
    let mut h = H::new();
    let v = h.sh.view();
    // 140 px row -> even 140; height 30; capsule radius 15; floating 8 px below the screen edge
    assert_eq!((v.body.width, v.body.height), (140.0, 30.0));
    assert_eq!((v.body.radius_top, v.body.radius_bottom), (15.0, 15.0));
    assert_eq!(v.body.left, (720.0 - 140.0) / 2.0);
    assert_eq!(v.body.top, 8.0);
    assert_eq!(v.phase, Phase::Collapsed);
    assert_eq!(v.frame, (720.0, 120.0));
    assert!(!v.body.flare_visible); // capsule shape: no concave flares
    // the measured row (after the first Start) was reported to the backend
    assert_eq!(h.sh.hit_rect().w, 140.0);
    assert_eq!(h.sh.hit_rect().y, 8.0);
    assert_eq!(h.sh.hit_rect().x, 290.0);
    h.sh.backend_mut().clear();
    // the same geometry is never reported twice
    h.ev(Event::LiveMeasured { w: 140.0, h: 30.0 });
    h.run(1000.0);
    assert_eq!(h.calls("pill_rect"), 0);
}

#[test]
fn width_follows_the_measured_row_with_even_rounding_and_clamps() {
    let mut h = H::new();
    h.ev(Event::LiveMeasured { w: 143.2, h: 30.0 });
    h.settle();
    assert_eq!(h.sh.view().body.width, 144.0); // ceil -> 144 (even)
    h.ev(Event::LiveMeasured { w: 20.0, h: 30.0 });
    h.settle();
    assert_eq!(h.sh.view().body.width, 118.0); // min 118
    h.ev(Event::LiveMeasured { w: 900.0, h: 30.0 });
    h.settle();
    assert_eq!(h.sh.view().body.width, 500.0); // max 500
    h.sh.reserve(30.0);
    h.settle();
    assert_eq!(h.sh.view().body.width, 500.0);
    h.ev(Event::LiveMeasured { w: 200.0, h: 30.0 });
    h.settle();
    assert_eq!(h.sh.view().body.width, 230.0); // + reserved pet space
    h.sh.reserve(30.4); // rounds to the same 30: no retarget
    assert_eq!(h.sh.spring_state()[0].2, 230.0);
}

#[test]
fn notch_shape_sits_on_the_edge_with_flares_and_square_top() {
    let mut h = H::with(json!({"shape": "notch"}));
    h.run(500.0); // the 8 px floating offset eases to 0 over 380 ms
    let v = h.sh.view();
    assert_eq!(v.body.top, 0.0);
    assert_eq!((v.body.radius_top, v.body.radius_bottom), (0.0, 15.0));
    assert!(v.body.flare_visible);
    assert_eq!(v.body.flare_radius, 14.0);
    assert_eq!(h.sh.hit_rect().y, 0.0);
}

#[test]
fn size_setting_scales_everything_and_retargets_without_animation() {
    let mut h = H::with(json!({"size": 150}));
    h.run(500.0);
    let v = h.sh.view();
    assert_eq!(v.scale, 1.5);
    assert_eq!(v.body.height, 45.0);
    assert_eq!(v.body.top, 12.0); // 8 * 1.5
    assert_eq!(v.body.radius_top, 22.5);
    assert_eq!(v.body.flare_radius, 21.0);
    // the pill_rect x centres the body in the scaled collapsed frame (720 * 1.5)
    let r = h.sh.hit_rect();
    assert_eq!(r.x, (1080.0 - r.w) / 2.0);
    assert_eq!(r.h, 45.0);
    // a size change is a snap (layout signature), not a spring
    h.ev(Event::SettingsChanged(json!({"size": 100})));
    assert!(!h.sh.view().wants_frames || h.sh.spring_state()[0].1 == 0.0);
    assert_eq!(h.sh.view().body.height, 30.0);
}

// ---------------- expand / collapse ----------------

#[test]
fn expand_runs_the_spring_to_the_expanded_geometry_and_back() {
    let mut h = H::new();
    h.ev(Event::Expand { tab: None, section: None });
    assert_eq!(h.sh.state(), PillState::Expanded);
    assert_eq!(h.sh.phase(), Phase::Expanding);
    assert!(h.sh.frame_expanded());
    h.run(100.0);
    let mid = h.sh.view();
    assert!(mid.body.width > 140.0 && mid.body.width < 700.0 || mid.body.width > 680.0);
    assert!(mid.panel.visible);
    assert!(!mid.live.visible); // the capsule row fades out
    h.settle();
    let v = h.sh.view();
    assert_eq!(h.sh.phase(), Phase::Expanded);
    assert_eq!((v.body.width, v.body.height), (680.0, 331.0));
    assert_eq!((v.body.radius_top, v.body.radius_bottom), (26.0, 30.0));
    assert_eq!(v.body.top, 8.0);
    assert_eq!(v.frame, (720.0, 480.0));
    let r = h.sh.hit_rect();
    assert_eq!((r.x, r.y, r.w, r.h), (20.0, 8.0, 680.0, 331.0));
    // collapse
    h.ev(Event::Collapse);
    assert_eq!(h.sh.state(), PillState::Collapsed);
    assert_eq!(h.sh.phase(), Phase::Collapsing);
    assert!(h.sh.frame_expanded()); // the OS frame stays big until the island has folded
    h.settle();
    assert_eq!(h.sh.phase(), Phase::Collapsed);
    assert!(!h.sh.frame_expanded());
    assert_eq!(h.last("pill_set_state"), json!({"state": "collapsed"}));
    assert_eq!(h.sh.view().body.width, 140.0);
    assert_eq!(h.sh.hit_rect().w, 140.0);
}

#[test]
fn collapse_squashes_below_the_final_size_then_recovers() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.ev(Event::Collapse);
    let mut min_w = f64::MAX;
    for _ in 0..90 {
        h.run(FRAME_MS);
        min_w = min_w.min(h.sh.view().body.width);
    }
    assert!(min_w < 140.0, "squash below the capsule width: {min_w}");
    h.settle();
    assert_eq!(h.sh.view().body.width, 140.0);
}

#[test]
fn expand_waits_for_the_os_frame_up_to_50ms() {
    let mut h = H::new();
    h.ev(Event::Env(Env { inner_w: 560.0, ..Env::default() })); // the surface is still the old size
    h.ev(Event::Expand { tab: None, section: None });
    assert_eq!(h.sh.phase(), Phase::Expanding);
    assert_eq!(h.sh.spring_state()[0].2, 140.0, "spring not started yet"); // target unchanged
    h.run(30.0);
    assert_eq!(h.sh.spring_state()[0].2, 140.0);
    // the frame arrives
    h.ev(Event::Env(Env { inner_w: 720.0, ..Env::default() }));
    h.run(FRAME_MS * 2.0);
    assert_eq!(h.sh.spring_state()[0].2, 680.0);
    // the same without the frame arriving: the 50 ms cap starts it anyway
    let mut h = H::new();
    h.ev(Event::Env(Env { inner_w: 560.0, ..Env::default() }));
    h.ev(Event::Expand { tab: None, section: None });
    h.run(70.0);
    assert_eq!(h.sh.spring_state()[0].2, 680.0);
}

#[test]
fn expand_while_expanded_is_a_noop_for_the_spring_and_collapse_requires_expanded() {
    let mut h = H::new();
    h.do_collapse_noop();
    h.expand();
    h.settle();
    h.sh.backend_mut().clear();
    h.ev(Event::Expand { tab: None, section: None });
    h.settle();
    assert_eq!(h.calls("pill_rect"), 0);
    assert_eq!(h.sh.phase(), Phase::Expanded);
}

impl H {
    fn do_collapse_noop(&mut self) {
        self.ev(Event::Collapse);
        self.run(500.0);
        assert_eq!(self.calls("pill_set_state"), 0);
    }
}

#[test]
fn reduced_motion_snaps_to_the_end_state_at_once() {
    let mut h = H::with(json!({"reduceMotion": true}));
    h.ev(Event::Expand { tab: None, section: None });
    // no frames needed: the body is already at the expanded geometry
    let v = h.sh.view();
    assert_eq!((v.body.width, v.body.height), (680.0, 331.0));
    assert!(v.reduced);
    h.run(FRAME_MS * 2.0); // one last loop pass, then it stops: no loop under reduced motion
    assert!(!h.sh.wants_frames());
    assert_eq!(h.sh.phase(), Phase::Expanded);
    h.ev(Event::Collapse);
    assert_eq!(h.sh.view().body.width, 140.0);
    assert!(!h.sh.frame_expanded());
    assert_eq!(h.sh.phase(), Phase::Collapsed);
}

#[test]
fn os_reduced_motion_is_honoured_too() {
    let mut h = H::new();
    h.ev(Event::Env(Env { os_reduce_motion: true, ..Env::default() }));
    assert!(h.sh.reduce());
    h.ev(Event::Expand { tab: None, section: None });
    assert_eq!(h.sh.view().body.width, 680.0);
}

#[test]
fn pinned_blocks_unforced_collapse_but_not_the_api_collapse() {
    let mut h = H::with(json!({"pinned": true}));
    h.expand();
    h.settle();
    h.ev(Event::Escape);
    h.ev(Event::BackgroundClick);
    h.ev(Event::Collapse);
    assert_eq!(h.sh.state(), PillState::Expanded);
    h.sh.collapse("");
    assert_eq!(h.sh.state(), PillState::Collapsed);
}

#[test]
fn escape_and_background_click_collapse() {
    let mut h = H::new();
    h.expand();
    h.ev(Event::Escape);
    assert_eq!(h.sh.state(), PillState::Collapsed);
    h.expand();
    h.ev(Event::BackgroundClick);
    assert_eq!(h.sh.state(), PillState::Collapsed);
}

#[test]
fn click_collapse_uses_the_deeper_squash_tuning() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.sh.collapse("click");
    h.run(FRAME_MS);
    // squash: stiffness 150 on the width channel
    let mut g = H::new();
    g.expand();
    g.settle();
    g.sh.collapse("");
    g.run(FRAME_MS);
    assert_eq!(h.sh.spring_state()[0].2, 140.0);
    assert_eq!(g.sh.spring_state()[0].2, 140.0);
    let mut min_click = f64::MAX;
    let mut min_plain = f64::MAX;
    for _ in 0..120 {
        h.run(FRAME_MS);
        g.run(FRAME_MS);
        min_click = min_click.min(h.sh.view().body.height);
        min_plain = min_plain.min(g.sh.view().body.height);
    }
    assert!(min_click <= min_plain + 1e-9);
}

#[test]
fn hotkey_asks_the_backend_to_toggle() {
    let mut h = H::new();
    h.ev(Event::Hotkey);
    assert_eq!(h.calls("pill_toggle"), 1);
    h.sh.expand(Some("note"), "hotkey");
    assert_eq!(h.last("pill_set_state"), json!({"state": "expanded", "tab": "note", "via": "hotkey"}));
    h.sh.expand(None, "");
    assert_eq!(h.last("pill_set_state"), json!({"state": "expanded", "tab": null, "via": "page"}));
}

// ---------------- hover / leave / typing ----------------

#[test]
fn hover_intent_opens_after_250ms_of_rest() {
    let mut h = H::new();
    h.ev(Event::Hover);
    assert_eq!(h.sh.phase(), Phase::Hover);
    h.run(200.0);
    assert_eq!(h.calls("pill_set_state"), 0);
    h.run(100.0);
    assert_eq!(h.last("pill_set_state"), json!({"state": "expanded", "tab": null, "via": "hover"}));
}

#[test]
fn hover_intent_restarts_while_the_pointer_moves_and_leave_cancels() {
    let mut h = H::new();
    h.ev(Event::Hover);
    for i in 0..4 {
        h.run(100.0);
        h.ev(Event::Cursor(Some((340.0 + i as f64 * 10.0, 20.0)))); // moved > 2 px: the timer restarts
    }
    assert_eq!(h.calls("pill_set_state"), 0, "the pointer never rested");
    h.run(300.0);
    assert_eq!(h.calls("pill_set_state"), 1);
    // leave before the delay
    let mut h = H::new();
    h.ev(Event::Hover);
    h.run(100.0);
    h.ev(Event::Leave);
    h.run(500.0);
    assert_eq!(h.calls("pill_set_state"), 0);
}

#[test]
fn small_pointer_jitter_does_not_restart_hover_intent() {
    let mut h = H::new();
    h.ev(Event::Cursor(Some((340.0, 20.0))));
    h.ev(Event::Hover);
    h.run(100.0);
    h.ev(Event::Cursor(Some((341.0, 20.5))));
    h.run(180.0);
    assert_eq!(h.calls("pill_set_state"), 1);
}

#[test]
fn leaving_the_expanded_pill_collapses_after_380ms_unless_back_or_typing() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.ev(Event::Hover);
    h.ev(Event::Leave);
    h.run(300.0);
    assert_eq!(h.sh.state(), PillState::Expanded);
    h.ev(Event::Hover); // back in time
    h.run(500.0);
    assert_eq!(h.sh.state(), PillState::Expanded);
    h.ev(Event::Leave);
    h.run(400.0);
    assert_eq!(h.sh.state(), PillState::Collapsed);
    // typing keeps it open
    let mut h = H::new();
    h.expand();
    h.settle();
    h.ev(Event::TextFocus(true));
    assert_eq!(h.last("pill_activate"), json!({"on": true}));
    h.ev(Event::Leave);
    h.run(600.0);
    assert_eq!(h.sh.state(), PillState::Expanded);
    // focus leaves while the pointer is away: collapse after 300 ms
    h.ev(Event::TextFocus(false));
    assert_eq!(h.last("pill_activate"), json!({"on": false}));
    h.run(250.0);
    assert_eq!(h.sh.state(), PillState::Expanded);
    h.run(100.0);
    assert_eq!(h.sh.state(), PillState::Collapsed);
}

#[test]
fn hover_never_opens_over_a_card_or_right_after_one_closed() {
    let mut h = H::new();
    h.sh.toast(card("a"));
    h.ev(Event::Hover);
    h.run(600.0);
    assert_eq!(h.calls("pill_set_state"), 0, "a card on screen is the thing to click");
    // dismiss: the pointer still rests where its button was for 900 ms
    h.ev(Event::CardAction(0)); // no actions: ignored
    h.sh.toast(CardSpec { id: Some("b".into()), actions: vec![CardAction { label: "OK".into(), id: "ok".into(), ..Default::default() }], ..card("b") });
    h.run(5000.0); // a ends by ttl, b shows (hovering all the while)
    assert!(h.sh.cards().has_current());
    h.ev(Event::CardAction(0));
    h.ev(Event::Leave);
    h.ev(Event::Hover);
    h.run(300.0);
    assert_eq!(h.calls("pill_set_state"), 0, "within 900 ms of the dismiss");
    h.run(1500.0);
    h.ev(Event::Leave);
    h.ev(Event::Hover);
    h.run(300.0);
    assert_eq!(h.calls("pill_set_state"), 1);
}

#[test]
fn cursor_anywhere_pulls_the_edge_only_when_close_and_motion_is_on() {
    let mut h = H::new();
    // under the capsule (x centre 360, bottom at 8 + 30 = 38)
    h.ev(Event::Cursor(Some((360.0, 50.0))));
    assert!(h.sh.liquid().want > 0.0);
    assert!(h.sh.liquid().b_amp >= 14.0);
    h.run(500.0);
    assert!(h.sh.liquid().inn > 0.1);
    // far away: nothing
    h.ev(Event::Cursor(Some((360.0, 400.0))));
    assert_eq!(h.sh.liquid().want, 0.0);
    // gone
    h.ev(Event::Cursor(Some((360.0, 50.0))));
    h.ev(Event::Cursor(None));
    assert_eq!(h.sh.liquid().want, 0.0);
    h.run(2000.0);
    assert_eq!(h.sh.liquid().inn, 0.0);
    // reduced motion: no pull
    let mut r = H::with(json!({"reduceMotion": true}));
    r.ev(Event::Cursor(Some((360.0, 50.0))));
    assert_eq!(r.sh.liquid().want, 0.0);
}

#[test]
fn a_dragged_file_makes_the_edge_reach_for_the_cursor() {
    let mut h = H::new();
    h.sh.set_drag_pull(true);
    h.ev(Event::Cursor(Some((360.0, 300.0)))); // far below
    assert!(h.sh.liquid().want >= 0.9 * 0.0);
    assert_eq!(h.sh.liquid().b_amp, 21.0);
    h.sh.set_drag_pull(false);
    h.ev(Event::Cursor(Some((360.0, 300.0))));
    assert_eq!(h.sh.liquid().want, 0.0);
}

// ---------------- hide / show / auto-hide ----------------

#[test]
fn hide_force_collapses_without_animation_and_show_restores() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.ev(Event::Hide);
    assert!(h.sh.is_hidden());
    assert_eq!(h.sh.phase(), Phase::Hidden);
    assert_eq!(h.sh.state(), PillState::Collapsed);
    assert_eq!(h.sh.view().body.width, 140.0); // no spring
    assert!(!h.sh.wants_frames());
    assert_eq!(h.last("pill_set_state"), json!({"state": "collapsed"}));
    assert!(!h.sh.frame_expanded());
    // while hidden: timers that would work are gone
    h.sh.backend_mut().clear();
    h.run(120_000.0);
    assert_eq!(h.calls("battery_state"), 0);
    assert_eq!(h.calls("weather_get"), 0);
    h.ev(Event::Show);
    assert_eq!(h.sh.phase(), Phase::Collapsed);
    assert_eq!(h.calls("battery_state"), 1);
}

#[test]
fn hide_drops_cards_and_pointer_state() {
    let mut h = H::new();
    h.sh.toast(card("a"));
    h.sh.toast(card("b"));
    h.ev(Event::Hover);
    h.ev(Event::Hide);
    assert!(!h.sh.cards().has_current());
    assert_eq!(h.sh.cards().queue_len(), 0);
    assert!(!h.sh.is_hovering());
    assert_eq!(h.last("pill_cards"), json!({"on": false}));
    // a toast while hidden is dropped
    h.sh.toast(card("c"));
    assert!(!h.sh.cards().has_current());
    assert_eq!(h.sh.cards().queue_len(), 0);
}

#[test]
fn auto_hide_slides_the_body_up_after_1500ms_idle_and_reveals_on_activity() {
    let mut h = H::with(json!({"autoHide": true}));
    h.run(1300.0);
    assert!(!h.sh.is_away());
    h.run(200.0);
    assert!(h.sh.is_away());
    assert_eq!(h.sh.phase(), Phase::AutoHidden);
    assert!(h.sh.view().body.away);
    assert!(h.sh.view().body.top < -30.0); // translateY(-130 %)
    assert_eq!(h.calls("pill_auto_hide"), 0);
    h.run(400.0);
    assert_eq!(h.last("pill_auto_hide"), json!({"on": true}));
    // a card asks to be revealed
    h.sh.toast(card("x"));
    assert_eq!(h.last("pill_auto_hide"), json!({"on": false}));
    assert!(!h.sh.cards().has_current(), "away: the card waits");
    // the backend un-hides: pill:show clears `away`, the queued card appears
    h.ev(Event::Show);
    assert!(!h.sh.is_away());
    assert!(h.sh.cards().has_current());
}

#[test]
fn auto_hide_respects_hover_cards_and_the_reduced_slide() {
    let mut h = H::with(json!({"autoHide": true}));
    h.ev(Event::Hover);
    h.run(3000.0);
    assert!(!h.sh.is_away());
    h.ev(Event::Leave);
    h.run(1400.0);
    assert!(!h.sh.is_away());
    h.run(200.0);
    assert!(h.sh.is_away());
    // reduced motion: the backend hide follows after 20 ms
    let mut r = H::with(json!({"autoHide": true, "reduceMotion": true}));
    r.run(1520.0);
    assert!(r.sh.is_away());
    r.run(40.0);
    assert_eq!(r.last("pill_auto_hide"), json!({"on": true}));
    // switching the setting off cancels the pending hide
    let mut c = H::with(json!({"autoHide": true}));
    c.run(1000.0);
    c.ev(Event::SettingsChanged(json!({"autoHide": false})));
    c.run(2000.0);
    assert!(!c.sh.is_away());
}

#[test]
fn auto_hide_keep_desktop_asks_the_backend_first() {
    let mut h = H::with(json!({"autoHide": true, "autoHideKeepDesktop": true}));
    h.run(1600.0);
    assert_eq!(h.calls("pill_desktop_fg"), 1);
    assert!(!h.sh.is_away());
    h.ev(Event::DesktopFg(true)); // desktop in front: stay, ask again later
    assert!(!h.sh.is_away());
    h.run(1600.0);
    assert_eq!(h.calls("pill_desktop_fg"), 2);
    h.ev(Event::DesktopFg(false));
    assert!(h.sh.is_away());
}

#[test]
fn expanding_cancels_a_pending_auto_hide_and_collapse_rearms_it() {
    let mut h = H::with(json!({"autoHide": true}));
    h.run(1000.0);
    h.expand();
    h.run(3000.0);
    assert!(!h.sh.is_away());
    h.ev(Event::Collapse);
    h.settle();
    h.run(1600.0);
    assert!(h.sh.is_away());
}

// ---------------- cards ----------------

#[test]
fn a_card_takes_over_the_row_and_the_island_grows_to_it() {
    let mut h = H::new();
    h.sh.toast(card("a"));
    assert_eq!(h.sh.phase(), Phase::Cards);
    assert_eq!(h.last("pill_cards"), json!({"on": true}));
    assert_eq!(h.sh.view().frame, (720.0, 200.0));
    assert!(matches!(h.sh.view().live.content, LiveContent::Card(_)));
    assert!(h.sh.view().live.tall);
    // the renderer measures the card (300 x 56): the island resizes to it with round ends
    h.ev(Event::LiveMeasured { w: 300.0, h: 56.0 });
    h.settle();
    let v = h.sh.view();
    assert_eq!((v.body.width, v.body.height), (300.0, 56.0));
    assert_eq!((v.body.radius_top, v.body.radius_bottom), (24.0, 24.0));
}

#[test]
fn cards_queue_one_at_a_time_and_use_the_default_ttl() {
    let mut h = H::new();
    h.sh.toast(card("a"));
    h.sh.toast(card("b"));
    assert_eq!(h.sh.cards().current_id(), Some("a"));
    assert_eq!(h.sh.cards().queue_len(), 1);
    h.run(3900.0);
    assert_eq!(h.sh.cards().current_id(), Some("a"));
    h.run(400.0);
    assert_eq!(h.sh.cards().current_id(), Some("b")); // 4000 ttl + 180 out
    h.run(4300.0);
    assert!(!h.sh.cards().has_current());
    assert!(matches!(h.sh.view().live.content, LiveContent::Clock { .. }));
    // the OS frame shrinks 600 ms after the island folded back
    assert_eq!(h.sh.view().frame, (720.0, 200.0));
    h.run(700.0);
    assert_eq!(h.sh.view().frame, (720.0, 120.0));
    assert_eq!(h.last("pill_cards"), json!({"on": false}));
}

#[test]
fn card_action_notifies_and_dismisses() {
    let mut h = H::new();
    let c = CardSpec { actions: vec![CardAction { label: "Open".into(), id: "open".into(), ..Default::default() }], ..card("copy") };
    h.sh.toast(c);
    h.ev(Event::CardAction(0));
    let n = h.sh.take_notices();
    assert!(n.contains(&Notice::CardAction { card_id: Some("copy".into()), action_id: "open".into() }));
    h.run(200.0);
    assert!(!h.sh.cards().has_current());
    h.ev(Event::CardAction(5)); // nothing showing / out of range: ignored
}

#[test]
fn same_id_toast_replaces_the_showing_card() {
    let mut h = H::new();
    h.sh.toast(card("dl"));
    h.run(500.0);
    let epoch = h.sh.view().live.swap_epoch;
    h.sh.toast(CardSpec { title: "newer".into(), ..card("dl") });
    assert_eq!(h.sh.cards().current_spec().unwrap().title, "newer");
    assert_eq!(h.sh.cards().queue_len(), 0);
    assert!(h.sh.view().live.swap_epoch > epoch, "the new card swaps in");
}

#[test]
fn custom_card_ttl() {
    let mut h = H::new();
    h.sh.toast(CardSpec { ttl_ms: 1200.0, ..card("t") });
    h.run(1100.0);
    assert!(h.sh.cards().has_current());
    h.run(400.0);
    assert!(!h.sh.cards().has_current());
}

#[test]
fn expanded_pill_holds_cards_back_until_it_collapses() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.sh.toast(card("a"));
    assert!(!h.sh.cards().has_current());
    assert_eq!(h.sh.cards().queue_len(), 1);
    h.ev(Event::Collapse);
    h.settle();
    assert!(h.sh.cards().has_current());
}

#[test]
fn card_tone_colours_the_edge() {
    let mut h = H::new();
    h.sh.toast(CardSpec { tone: "amber".into(), ..card("w") });
    let v = h.sh.view();
    assert_eq!(v.wave_color, Some(v.palette.amber));
    assert_eq!(h.sh.liquid().edge, v.palette.amber);
    h.run(5000.0);
    assert_eq!(h.sh.view().wave_color, None);
}

#[test]
fn reduced_motion_removes_cards_without_an_out_phase() {
    let mut h = H::with(json!({"reduceMotion": true}));
    h.sh.toast(CardSpec { ttl_ms: 500.0, ..card("a") });
    h.run(520.0);
    assert!(!h.sh.cards().has_current());
}

// ---------------- quiet hours ----------------

fn quiet_clock(h: &mut H, hour: i64) {
    // 2026-10-30 12:00 UTC + (hour - 12) h
    let base = ManualClock::default().wall_ms;
    h.wall = (base + (hour - 12) * 3_600_000) as f64;
    h.sh.clock_mut().wall_ms = h.wall as i64;
}

#[test]
fn quiet_hours_drop_toasts_except_timers_reminders_and_urgent() {
    let mut h = H::with(json!({"quiet.on": true, "quiet.from": "22:00", "quiet.to": "07:00"}));
    quiet_clock(&mut h, 23);
    h.run(FRAME_MS);
    assert!(h.sh.quiet());
    h.sh.toast(card("copy"));
    assert!(!h.sh.cards().has_current());
    h.sh.toast(card("timer-5"));
    assert_eq!(h.sh.cards().current_id(), Some("timer-5"));
    h.run(5000.0);
    h.sh.toast(card("remind:3"));
    assert!(h.sh.cards().has_current());
    h.run(5000.0);
    h.sh.toast(CardSpec { urgent: true, ..card("x") });
    assert!(h.sh.cards().has_current());
    // daytime: shown again
    h.run(5000.0);
    quiet_clock(&mut h, 12);
    h.run(FRAME_MS);
    assert!(!h.sh.quiet());
    h.sh.toast(card("copy"));
    assert!(h.sh.cards().has_current());
}

#[test]
fn quiet_hours_mute_event_chips_and_reactions_of_muted_sources_only() {
    let mut h = H::with(json!({"quiet.on": true}));
    quiet_clock(&mut h, 23);
    h.run(FRAME_MS);
    h.sh.set_live("download", Some(LiveInput { priority: 5.0, spec: json!({"x": 1}), ..Default::default() }));
    assert!(h.sh.lives().is_empty());
    h.sh.set_live("download", Some(LiveInput { priority: 5.0, urgent: true, spec: json!({"x": 1}), ..Default::default() }));
    assert_eq!(h.sh.lives().len(), 1);
    h.sh.set_live("music", Some(LiveInput { priority: 1.0, spec: json!({}), ..Default::default() }));
    assert_eq!(h.sh.lives().len(), 2);
    h.sh.react("copy", "done", &ReactOpts::default());
    assert!(!h.sh.reactions().has("copy"));
    h.sh.react("media", "music", &ReactOpts::default());
    assert!(h.sh.reactions().has("media"));
}

#[test]
fn the_quiet_sweep_every_20s_removes_what_was_queued_before_the_window_began() {
    let mut h = H::with(json!({"quiet.on": true}));
    quiet_clock(&mut h, 21); // not yet quiet
    h.run(FRAME_MS);
    h.sh.toast(CardSpec { ttl_ms: 60_000.0, ..card("showing") });
    h.sh.toast(card("queued-copy"));
    h.sh.toast(card("timer-9"));
    h.sh.set_live("agents", Some(LiveInput { priority: 3.0, spec: json!({}), ..Default::default() }));
    h.sh.react("agent-done", "done", &ReactOpts::default());
    assert_eq!(h.sh.lives().len(), 1);
    quiet_clock(&mut h, 23); // the window begins
    h.run(21_000.0);
    assert!(h.sh.lives().is_empty());
    assert!(!h.sh.reactions().has("agent-done"));
    assert!(!h.sh.cards().queued_ids().contains(&Some("queued-copy")));
    assert!(!h.sh.cards().has_current(), "the non-exempt card on screen was removed");
    assert!(h.sh.cards().queued_ids().contains(&Some("timer-9")));
}

// ---------------- live row ----------------

fn slot(prio: f64) -> LiveInput {
    LiveInput { priority: prio, spec: json!({"p": prio}), ..Default::default() }
}

#[test]
fn live_slot_beats_the_clock_and_the_highest_priority_wins() {
    let mut h = H::new();
    assert!(matches!(h.sh.view().live.content, LiveContent::Clock { .. }));
    h.sh.set_live("music", Some(slot(2.0)));
    h.sh.set_live("agents", Some(slot(5.0)));
    match h.sh.view().live.content {
        LiveContent::Slot { id, .. } => assert_eq!(id, "agents"),
        other => panic!("{other:?}"),
    }
    h.sh.set_live("agents", None);
    match h.sh.view().live.content {
        LiveContent::Slot { id, .. } => assert_eq!(id, "music"),
        other => panic!("{other:?}"),
    }
    h.sh.set_live("music", None);
    assert!(matches!(h.sh.view().live.content, LiveContent::Clock { .. }));
}

#[test]
fn live_slot_ttl_falls_back_to_the_clock_at_expiry() {
    let mut h = H::new();
    h.sh.set_live("vol", Some(LiveInput { ttl_ms: 1500, ..slot(9.0) }));
    h.run(1400.0);
    assert!(matches!(h.sh.view().live.content, LiveContent::Slot { .. }));
    h.run(200.0);
    assert!(matches!(h.sh.view().live.content, LiveContent::Clock { .. }));
    assert!(h.sh.lives().is_empty());
}

#[test]
fn same_key_edits_in_place_without_a_swap_but_a_new_key_swaps() {
    let mut h = H::new();
    h.sh.set_live("music", Some(LiveInput { key: "t1".into(), spec: json!({"pos": 1}), ..slot(2.0) }));
    let e = h.sh.view().live.swap_epoch;
    h.sh.set_live("music", Some(LiveInput { key: "t1".into(), spec: json!({"pos": 2}), ..slot(2.0) }));
    assert_eq!(h.sh.view().live.swap_epoch, e, "edited in place: no re-render");
    h.sh.set_live("music", Some(LiveInput { key: "t2".into(), spec: json!({"pos": 0}), ..slot(2.0) }));
    assert_eq!(h.sh.view().live.swap_epoch, e + 1);
    assert!(h.sh.view().live.ghost.is_some(), "the old row fades out as a ghost");
    h.run(200.0);
    assert!(h.sh.view().live.ghost.is_none());
}

#[test]
fn swap_animations_are_skipped_under_reduced_motion() {
    let mut h = H::with(json!({"reduceMotion": true}));
    let e = h.sh.view().live.swap_epoch;
    h.sh.set_live("a", Some(slot(1.0)));
    h.sh.set_live("b", Some(slot(2.0)));
    assert_eq!(h.sh.view().live.swap_epoch, e);
    assert!(h.sh.view().live.ghost.is_none());
}

#[test]
fn accent_colours_the_edge_and_starts_the_loop() {
    let mut h = H::new();
    h.sh.set_live("rec", Some(LiveInput { accent: "#ff453a".into(), ..slot(7.0) }));
    let v = h.sh.view();
    assert_eq!(v.wave_color.map(|c| c.rgb_tuple()), Some((0xff, 0x45, 0x3a)));
    assert_eq!(h.sh.liquid().edge.rgb_tuple(), (0xff, 0x45, 0x3a));
    assert!(h.sh.liquid().playing);
    h.sh.set_live("rec", None);
    assert!(!h.sh.liquid().playing);
    assert_eq!(h.sh.view().wave_color, None);
}

#[test]
fn a_tall_slot_sizes_the_island_by_its_own_height() {
    let mut h = H::new();
    h.sh.set_live("shelf", Some(LiveInput { tall: true, ..slot(8.0) }));
    h.ev(Event::LiveMeasured { w: 260.0, h: 64.0 });
    h.settle();
    let v = h.sh.view();
    assert!(v.live.tall);
    assert_eq!((v.body.width, v.body.height, v.body.radius_bottom), (260.0, 64.0, 24.0));
    h.sh.set_live("shelf", None);
    h.ev(Event::LiveMeasured { w: 140.0, h: 30.0 });
    h.settle();
    assert_eq!(h.sh.view().body.height, 30.0);
    assert!(!h.sh.view().live.tall);
}

// ---------------- clock row ----------------

#[test]
fn clock_row_shows_the_time_and_rolls_the_changed_digit_on_the_minute() {
    let mut h = H::new();
    let time = |h: &H| match h.sh.view().live.content {
        LiveContent::Clock { time, .. } => time.unwrap(),
        _ => panic!(),
    };
    assert_eq!(time(&h), "12:00");
    h.run(59_000.0);
    assert_eq!(time(&h), "12:00");
    // the render timer fires 30 ms after the minute boundary
    h.run(1100.0);
    assert_eq!(time(&h), "12:01");
    let v = h.sh.view();
    assert_eq!(v.live.rolls.len(), 1); // 0 -> 1 in the last digit
    assert_eq!((v.live.rolls[0].0.from, v.live.rolls[0].0.to, v.live.rolls[0].0.index), ('0', '1', 4));
    h.run(300.0);
    assert!(h.sh.view().live.rolls.is_empty(), "rolls expire after 280 ms");
}

#[test]
fn seconds_re_render_every_second_and_never_roll() {
    let mut h = H::with(json!({"showSeconds": true}));
    let secs = |h: &H| match h.sh.view().live.content {
        LiveContent::Clock { time, .. } => time.unwrap(),
        _ => panic!(),
    };
    let a = secs(&h);
    h.run(2100.0);
    let b = secs(&h);
    assert_ne!(a, b);
    assert!(b.len() == 8);
    assert!(h.sh.view().live.rolls.is_empty());
}

#[test]
fn twelve_hour_clock_without_am_pm() {
    let mut h = H::with(json!({"clock24": false}));
    h.run(FRAME_MS);
    match h.sh.view().live.content {
        LiveContent::Clock { time, .. } => assert_eq!(time.unwrap(), "12:00"),
        _ => panic!(),
    }
    quiet_clock(&mut h, 15);
    h.ev(Event::SettingsChanged(json!({"clock24": false, "showDate": true})));
    h.run(FRAME_MS);
    match h.sh.view().live.content {
        LiveContent::Clock { time, row, .. } => {
            assert_eq!(time.unwrap(), "3:00");
            assert_eq!(row.left.len(), 1); // the date
        }
        _ => panic!(),
    }
}

#[test]
fn local_offset_shifts_the_displayed_time() {
    let mut h = H::new();
    h.sh.clock_mut().offset_min = 180;
    h.run(65_000.0);
    match h.sh.view().live.content {
        LiveContent::Clock { time, .. } => assert_eq!(time.unwrap(), "15:01"),
        _ => panic!(),
    }
}

#[test]
fn clock_slots_replace_the_side_items() {
    let mut h = H::new();
    h.sh.set_clock_slots(ClockSlots { left: None, right: Some(SlotItem::Temp(16)) });
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => assert_eq!(row.right.len(), 1),
        _ => panic!(),
    }
    h.sh.set_clock_slots(ClockSlots::default());
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => assert!(row.right.is_empty()),
        _ => panic!(),
    }
}

// ---------------- weather / battery ----------------

fn weather_ok(temp: f64) -> Value {
    json!({"status": "ok", "now": {"temp": temp, "icon": "partly-day", "text": "Partly cloudy"}})
}

#[test]
fn weather_is_fetched_when_wanted_and_shown_on_the_icon_side() {
    let mut h = H::with(json!({"net.weather": true}));
    assert_eq!(h.calls("weather_get"), 0); // cleared by the harness; the Start call happened before
    h.ev(Event::SettingsChanged(json!({"net.weather": false})));
    h.ev(Event::SettingsChanged(json!({"net.weather": true})));
    assert_eq!(h.calls("weather_get"), 1);
    assert_eq!(h.last("weather_get"), json!({"force": false, "tz": 0}));
    h.ev(Event::WeatherGet(weather_ok(16.4)));
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => {
            assert_eq!(row.left.len(), 1); // icon
            assert_eq!(row.right.len(), 1); // 16
        }
        _ => panic!(),
    }
    assert!(h.sh.view().live.swap_epoch > 0);
    // the pet is on the other side: the icon moves to the right column
    h.ev(Event::Character { side: CharSide::Right, legs: false });
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => {
            assert!(row.left.is_empty());
            assert_eq!(row.right.len(), 2);
        }
        _ => panic!(),
    }
    // refreshed every 15 minutes
    h.sh.backend_mut().clear();
    h.run(15.0 * 60_000.0 + 100.0);
    assert_eq!(h.calls("weather_get"), 1);
}

#[test]
fn weather_off_or_nocity_clears_the_row_items() {
    let mut h = H::with(json!({"net.weather": true}));
    h.ev(Event::WeatherCurrent(json!({"temp": 7, "icon": "snow"})));
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => assert_eq!(row.left.len() + row.right.len(), 2),
        _ => panic!(),
    }
    h.ev(Event::WeatherGet(json!({"status": "nocity"})));
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => assert_eq!(row.left.len() + row.right.len(), 0),
        _ => panic!(),
    }
    h.ev(Event::WeatherCurrent(json!({"temp": null})));
    h.ev(Event::WeatherCurrent(json!({"temp": "nan"})));
}

#[test]
fn weather_switch_off_clears_the_forecast_and_stops_fetching() {
    let mut h = H::with(json!({"net.weather": true}));
    h.ev(Event::WeatherCurrent(json!({"temp": 7, "icon": "snow"})));
    h.ev(Event::SettingsChanged(json!({"module.weather": false})));
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => assert!(row.left.is_empty() && row.right.is_empty()),
        _ => panic!(),
    }
    h.sh.backend_mut().clear();
    h.run(20.0 * 60_000.0);
    assert_eq!(h.calls("weather_get"), 0);
}

#[test]
fn show_refreshes_a_stale_forecast_but_not_a_fresh_one() {
    let mut h = H::with(json!({"net.weather": true}));
    h.ev(Event::SettingsChanged(json!({"net.weather": false})));
    h.ev(Event::SettingsChanged(json!({"net.weather": true})));
    h.ev(Event::WeatherGet(weather_ok(10.0)));
    h.sh.backend_mut().clear();
    h.ev(Event::Hide);
    h.run(5.0 * 60_000.0);
    h.ev(Event::Show);
    assert_eq!(h.calls("weather_get"), 0, "5 min old: just re-arm");
    h.ev(Event::Hide);
    h.run(20.0 * 60_000.0);
    h.ev(Event::Show);
    assert_eq!(h.calls("weather_get"), 1, "older than 15 min: one refresh");
}

#[test]
fn battery_is_polled_every_minute_and_only_a_change_redraws() {
    let mut h = H::new();
    h.run(61_000.0);
    assert_eq!(h.calls("battery_state"), 1);
    let b = json!({"has_battery": true, "percent": 80, "charging": false});
    let e = h.sh.view().live.swap_epoch;
    h.ev(Event::BatteryState(b.clone()));
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => assert_eq!(row.right.len(), 1),
        _ => panic!(),
    }
    let e2 = h.sh.view().live.swap_epoch;
    assert!(e2 >= e);
    h.ev(Event::BatteryState(b)); // identical: nothing
    assert_eq!(h.sh.view().live.swap_epoch, e2);
    // switched off: no more polling
    h.ev(Event::SettingsChanged(json!({"showBattery": false})));
    h.sh.backend_mut().clear();
    h.run(130_000.0);
    assert_eq!(h.calls("battery_state"), 0);
}

// ---------------- reactions through the shell ----------------

#[test]
fn reactions_paint_the_edge_notify_observers_and_resolve_by_priority() {
    let mut h = H::new();
    h.sh.react("media", "music", &ReactOpts::default());
    h.sh.react("agents", "thinking", &ReactOpts::default());
    assert_eq!(h.sh.liquid().react, Preset::Thinking);
    let n = h.sh.take_notices();
    assert!(n.contains(&Notice::Reaction { name: Preset::Thinking, color: String::new() }));
    assert_eq!(h.sh.liquid().edge.rgb_tuple(), (0x9e, 0x8f, 0xe8));
    h.sh.unreact("agents");
    assert_eq!(h.sh.liquid().react, Preset::Music);
    assert_eq!(h.sh.liquid().rt, 0.0);
}

#[test]
fn a_ttl_reaction_expires_via_the_timers() {
    let mut h = H::new();
    h.sh.react("shelf", "file", &ReactOpts::ttl(2000));
    assert_eq!(h.sh.liquid().react, Preset::File);
    h.run(1900.0);
    assert_eq!(h.sh.liquid().react, Preset::File);
    h.run(300.0);
    assert_eq!(h.sh.liquid().react, Preset::Calm);
}

#[test]
fn preview_reaction_returns_after_its_ttl() {
    let mut h = H::new();
    let applied = h.sh.set_reaction("attention", &ReactOpts::ttl(6000));
    assert_eq!(applied, Preset::Attention);
    h.run(5800.0);
    assert_eq!(h.sh.liquid().react, Preset::Attention);
    h.run(400.0);
    assert_eq!(h.sh.liquid().react, Preset::Calm);
    assert_eq!(h.sh.set_reaction("bogus", &ReactOpts::default()), Preset::Calm);
}

#[test]
fn real_audio_is_a_music_baseline_that_ends_2_5s_after_the_last_frame() {
    let mut h = H::new();
    let frame = || super::liquid::Bands { b: (0..32).map(|k| 0.8 - 0.01 * k as f64).collect(), rms: 0.3, t: None };
    h.ev(Event::AudioBands(frame()));
    assert_eq!(h.sh.liquid().react, Preset::Music);
    for _ in 0..20 {
        h.run(100.0);
        h.ev(Event::AudioBands(frame()));
    }
    assert_eq!(h.sh.liquid().react, Preset::Music);
    h.run(2400.0);
    assert_eq!(h.sh.liquid().react, Preset::Music);
    h.run(400.0);
    assert_eq!(h.sh.liquid().react, Preset::Calm);
}

#[test]
fn five_idle_minutes_put_the_edge_to_sleep_and_the_pointer_wakes_it() {
    let mut h = H::new();
    h.run(299_000.0);
    assert_eq!(h.sh.liquid().react, Preset::Calm);
    h.run(2000.0);
    assert_eq!(h.sh.liquid().react, Preset::Sleep);
    h.ev(Event::Cursor(Some((360.0, 20.0))));
    assert_eq!(h.sh.liquid().react, Preset::Calm);
    // hovering keeps it awake
    let mut g = H::new();
    g.ev(Event::Hover);
    g.run(310_000.0);
    assert_ne!(g.sh.liquid().react, Preset::Sleep);
}

#[test]
fn edge_style_setting_applies_live() {
    let mut h = H::new();
    h.ev(Event::SettingsChanged(json!({"character": "none"})));
    assert!(!h.sh.view().edge.visible);
    h.ev(Event::SettingsChanged(json!({"character": "glow"})));
    assert!(h.sh.view().edge.glow.is_some());
    h.ev(Event::SettingsChanged(json!({"character": "liquid"})));
    assert!(!h.sh.view().edge.points.is_empty());
    h.sh.set_edge_color("#00ff00");
    assert_eq!(h.sh.liquid().edge.rgb_tuple(), (0, 255, 0));
    h.sh.set_edge_color("");
    assert_ne!(h.sh.liquid().edge.rgb_tuple(), (0, 255, 0));
}

// ---------------- frame loop ----------------

#[test]
fn the_fps_setting_caps_the_loop_and_auto_does_not() {
    let count = |fps: Value, display_hz: f64| {
        let mut h = H::with(json!({"fps": fps, "edgeIdle": true}));
        h.sh.react("a", "attention", &ReactOpts::default()); // keep the loop busy
        let t0 = h.sh.liquid().t;
        let mut n = 0;
        let step = 1000.0 / display_hz;
        for _ in 0..(display_hz as usize) {
            h.t += step;
            h.wall += step;
            h.sh.clock_mut().wall_ms = h.wall as i64;
            let before = h.sh.liquid().t;
            h.sh.tick(step, h.t);
            if h.sh.liquid().t != before {
                n += 1;
            }
        }
        let _ = t0;
        n
    };
    let n_auto = count(json!("auto"), 144.0);
    assert_eq!(n_auto, 144, "no cap: every display frame");
    // the cap skips display frames (acc < 1000/fps - 1.5 ms), so on a 144 Hz display it quantises to whole frames
    let n30 = count(json!(30), 144.0);
    assert!((28..=31).contains(&n30), "30 fps cap on a 144 Hz display: {n30}");
    let n60 = count(json!(60), 144.0);
    assert!((40..=62).contains(&n60), "{n60}");
    assert!(n30 < n60 && n60 < n_auto);
    // on a 60 Hz display a 60 cap passes every frame
    assert!(count(json!(60), 60.0) >= 59);
    let n240_on_144 = count(json!(240), 144.0);
    assert!(n240_on_144 >= 143, "a cap above the display rate never slows it: {n240_on_144}");
}

#[test]
fn frames_are_wanted_only_while_something_runs_and_the_window_is_shown() {
    // Note (finding): after any disturbance the JS field settles to e ~ 0.37 (pinned end samples vs target), above the 0.08 "still"
    // threshold, so the liquid loop does not quiesce by itself; it is the springs, hide, occlusion and reduced motion that stop frames.
    let mut h = H::new();
    assert!(h.sh.wants_frames());
    h.ev(Event::Hide);
    assert!(!h.sh.wants_frames());
    h.ev(Event::Show);
    assert!(h.sh.wants_frames());
    let mut r = H::with(json!({"reduceMotion": true}));
    r.run(100.0);
    assert!(!r.sh.wants_frames(), "reduced motion: no loop at all");
    r.sh.react("a", "attention", &ReactOpts::default());
    assert!(!r.sh.wants_frames(), "kick() is a no-op under reduced motion");
}

#[test]
fn occluded_window_pauses_frames_but_not_timers() {
    let mut h = H::new();
    h.sh.react("a", "attention", &ReactOpts::ttl(1000));
    h.ev(Event::Env(Env { occluded: true, ..Env::default() }));
    assert!(!h.sh.wants_frames());
    let t = h.sh.liquid().t;
    h.run(1500.0);
    assert_eq!(h.sh.liquid().t, t);
    assert_eq!(h.sh.liquid().react, Preset::Calm, "the ttl timer still fired");
    assert!(h.sh.next_wakeup().is_some());
}

#[test]
fn next_wakeup_reports_the_nearest_deadline() {
    let mut h = H::new();
    let w = h.sh.next_wakeup().unwrap();
    assert!(w > h.t);
    h.ev(Event::Hover);
    assert!(h.sh.next_wakeup().unwrap() <= h.t + 250.0 + 1.0);
}

#[test]
fn dt_is_clamped_to_50ms() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.ev(Event::Collapse);
    // a 2 s stall: one step integrates at most 50 ms
    h.t += 2000.0;
    h.sh.tick(2000.0, h.t);
    assert!(h.sh.view().body.width > 200.0, "the spring only advanced 50 ms");
}

// ---------------- tabs ----------------

#[test]
fn first_visible_tab_is_selected_and_clicking_persists_last_tab() {
    let mut h = H::new();
    h.sh.register_tab(tab("note", 20));
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("note")); // nothing was chosen: the first visible one
    h.sh.register_tab(tab("home", 10)); // a lower order registered later takes over until the user picks
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("home"));
    assert_eq!(h.calls("settings_set"), 0, "auto-selection is not persisted");
    h.sh.take_notices();
    h.ev(Event::TabClicked("note".into()));
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("note"));
    assert_eq!(h.last("settings_set"), json!({"patch": {"lastTab": "note"}}));
    h.ev(Event::TabClicked("note".into()));
    assert_eq!(h.calls("settings_set"), 1, "already current");
    assert_eq!(h.sh.view().tab_dir, super::types::TabDir::Right);
    h.ev(Event::TabClicked("home".into()));
    assert_eq!(h.sh.view().tab_dir, super::types::TabDir::Left);
    // a click means the user chose: later registrations no longer steal the selection
    h.sh.register_tab(tab("clipboard", 5));
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("home"));
}

#[test]
fn the_expanded_body_height_follows_the_current_tab_with_the_tab_spring() {
    let mut h = H::new();
    h.sh.register_tab(tab("home", 10));
    h.sh.register_tab(tab("note", 20));
    h.expand();
    h.settle();
    assert_eq!(h.sh.view().body.height, 331.0);
    h.ev(Event::TabClicked("note".into()));
    assert_eq!(h.sh.spring_state()[1].2, 417.0);
    h.settle();
    assert_eq!(h.sh.view().body.height, 417.0);
    assert_eq!(h.sh.hit_rect().h, 417.0);
    let n = h.sh.take_notices();
    assert!(n.contains(&Notice::TabShown("note".into())));
    assert!(n.contains(&Notice::TabHidden("home".into())));
}

#[test]
fn expand_with_a_tab_selects_it_and_unknown_or_off_tabs_are_ignored() {
    let mut h = H::with(json!({"module.chat": false}));
    h.sh.register_tab(tab("home", 10));
    h.sh.register_tab(tab("chat", 20));
    h.sh.register_tab(tab("weather", 30));
    h.ev(Event::Expand { tab: Some("weather".into()), section: None });
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("weather"));
    h.settle();
    assert_eq!(h.sh.view().body.height, 413.0);
    h.ev(Event::Collapse);
    h.settle();
    h.ev(Event::Expand { tab: Some("chat".into()), section: None }); // switched off in Settings > Modules
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("weather"));
    h.ev(Event::Collapse);
    h.settle();
    h.ev(Event::Expand { tab: Some("nope".into()), section: None });
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("weather"));
}

#[test]
fn switching_a_module_off_moves_the_open_tab_and_resizes() {
    let mut h = H::new();
    h.sh.register_tab(tab("home", 10));
    h.sh.register_tab(tab("note", 20));
    h.expand();
    h.ev(Event::TabClicked("note".into()));
    h.settle();
    assert_eq!(h.sh.view().body.height, 417.0);
    h.ev(Event::SettingsChanged(json!({"module.note": false})));
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("home"));
    h.settle();
    assert_eq!(h.sh.view().body.height, 331.0);
    assert!(h.sh.view().tabs.iter().any(|t| t.id == "note" && t.off));
}

#[test]
fn last_tab_from_settings_is_restored_at_start() {
    let clock = ManualClock::default();
    let mut sh = Shell::new(RecBackend::default(), clock, "en-US", 1);
    sh.register_tab(tab("home", 10));
    sh.register_tab(tab("note", 20));
    sh.handle(Event::Start(json!({"lastTab": "note"})));
    assert_eq!(sh.tabs().cur.as_deref(), Some("note"));
    assert_eq!(sh.backend().count("settings_set"), 0, "restoring does not write back");
}

#[test]
fn badges_and_header_state() {
    let mut h = H::new();
    h.sh.register_tab(tab("home", 10));
    h.sh.set_badge("home", 3);
    assert_eq!(h.sh.view().tabs.iter().find(|t| t.id == "home").unwrap().badge, 3);
    h.sh.set_vpn("de-extra");
    h.ev(Event::PinButton);
    let v = h.sh.view();
    assert_eq!(v.chrome.vpn, "DE-");
    assert!(v.chrome.pinned);
    h.ev(Event::PinButton);
    assert!(!h.sh.view().chrome.pinned);
}

#[test]
fn settings_button_opens_the_window_and_folds_or_falls_back_to_the_tab() {
    let mut h = H::new();
    h.sh.register_tab(tab("home", 10));
    h.expand();
    h.settle();
    h.ev(Event::SettingsButton);
    assert_eq!(h.last("settings_open"), json!({"page": "general"}));
    h.ev(Event::SettingsOpened(true));
    assert_eq!(h.sh.state(), PillState::Collapsed);
    // unavailable: the in-pill settings tab
    let mut g = H::new();
    g.sh.register_tab(tab("home", 10));
    g.expand();
    g.settle();
    g.ev(Event::SettingsButton);
    g.ev(Event::SettingsOpened(false));
    assert_eq!(g.sh.tabs().cur.as_deref(), Some("settings"));
    assert!(g.sh.view().chrome.settings_current);
    g.ev(Event::SettingsButton); // pressed again: back to the previous tab
    assert_eq!(g.sh.tabs().cur.as_deref(), Some("home"));
}

#[test]
fn command_bar_settings_request_opens_the_window_instead_of_expanding() {
    let mut h = H::new();
    h.ev(Event::OpenTab { tab: Some("settings".into()), section: Some("ai".into()) });
    assert_eq!(h.last("settings_open"), json!({"page": "ai"}));
    assert_eq!(h.sh.state(), PillState::Collapsed);
    h.ev(Event::SettingsOpened(true)); // frame was not expanded: nothing to give back
    assert_eq!(h.sh.state(), PillState::Collapsed);
    // an unknown page falls back to general
    h.ev(Event::OpenTab { tab: Some("settings".into()), section: Some("zzz".into()) });
    assert_eq!(h.last("settings_open"), json!({"page": "general"}));
    // a failed open expands onto the fallback settings tab
    h.ev(Event::SettingsOpened(false));
    assert_eq!(h.sh.state(), PillState::Expanded);
    assert_eq!(h.sh.tabs().cur.as_deref(), Some("settings"));
    h.ev(Event::OpenTab { tab: Some("note".into()), section: None }); // only settings is handled
}

// ---------------- settings, language, theme, misc ----------------

#[test]
fn language_follows_the_setting_and_notifies() {
    let mut h = H::new();
    assert_eq!(h.sh.lang(), super::settings::Lang::En);
    h.ev(Event::SettingsChanged(json!({"language": "ru"})));
    assert_eq!(h.sh.lang(), super::settings::Lang::Ru);
    assert!(h.sh.take_notices().contains(&Notice::LangChanged(super::settings::Lang::Ru)));
    h.ev(Event::SettingsChanged(json!({"language": "auto", "showDate": true})));
    assert_eq!(h.sh.lang(), super::settings::Lang::En);
    // the date follows the language
    h.ev(Event::SettingsChanged(json!({"language": "ru"})));
    match h.sh.view().live.content {
        LiveContent::Clock { row, .. } => match &row.left[0] {
            super::clock::ClockItem::Date(d) => assert_eq!(d, "пт 30 окт"),
            other => panic!("{other:?}"),
        },
        _ => panic!(),
    }
}

#[test]
fn theme_auto_follows_the_os_and_fill_alpha_follows_transparency() {
    let mut h = H::with(json!({"theme": "auto", "opacityCol": 20, "opacityExp": 40}));
    let v = h.sh.view();
    assert_eq!(v.theme, super::color::Theme::Dark);
    assert!((v.body.fill.a - 0.8).abs() < 1e-6);
    h.ev(Event::Env(Env { os_prefers_light: true, ..Env::default() }));
    let v = h.sh.view();
    assert_eq!(v.theme, super::color::Theme::Light);
    assert_eq!(v.body.fill.r, 0xf6);
    assert!(v.body.shadow);
    h.expand();
    assert!((h.sh.view().body.fill.a - 0.6).abs() < 1e-6);
}

#[test]
fn pass_through_has_its_own_transparency() {
    let h = H::with(json!({"passThrough": true, "passThroughOpacity": 50, "opacityCol": 10}));
    assert!((h.sh.view().body.fill.a - 0.5).abs() < 1e-6);
}

#[test]
fn view_places_the_edge_strip_under_the_body() {
    let h = H::new();
    let v = h.sh.view();
    assert_eq!(v.strip.w, 720.0);
    assert_eq!(v.strip.h, 50.0);
    assert_eq!(v.strip.x, v.body.left + v.body.width / 2.0 - 360.0);
    assert_eq!(v.strip.y, v.body.top + v.body.height - 4.0);
    assert!(!v.edge.points.is_empty());
    assert_eq!(v.edge.half, 140.0 / 2.0 - 15.0);
}

#[test]
fn the_same_seed_and_inputs_give_identical_views() {
    let run = || {
        let mut h = H::new();
        h.sh.react("a", "thinking", &ReactOpts::default());
        h.ev(Event::Cursor(Some((360.0, 50.0))));
        h.expand();
        h.run(3000.0);
        h.sh.view()
    };
    assert_eq!(run(), run());
}

#[test]
fn window_resize_repositions_the_body_on_whole_device_pixels() {
    let mut h = H::new();
    h.ev(Event::Env(Env { inner_w: 721.0, dpr: 1.5, ..Env::default() }));
    let left = h.sh.view().body.left;
    assert_eq!(left, ((721.0f64 - 140.0) / 2.0 * 1.5 + 0.5).floor() / 1.5);
}

// ---------------- races and edge cases ----------------

#[test]
fn re_expanding_during_the_collapse_animation_never_gives_the_frame_back() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.sh.backend_mut().clear();
    h.ev(Event::Collapse);
    h.run(60.0);
    assert_eq!(h.sh.phase(), Phase::Collapsing);
    h.ev(Event::Expand { tab: None, section: None }); // the pointer came back
    h.settle();
    assert_eq!(h.sh.state(), PillState::Expanded);
    assert!(h.sh.frame_expanded());
    assert_eq!(h.calls("pill_set_state"), 0, "the pending collapse completion saw state != collapsed");
    assert_eq!(h.sh.view().body.width, 680.0);
}

#[test]
fn a_card_during_the_collapse_animation_starts_at_once_and_survives_the_frame_handback() {
    let mut h = H::new();
    h.expand();
    h.settle();
    h.ev(Event::Collapse);
    h.sh.toast(card("late")); // next() only checks away / hidden / expanded
    assert_eq!(h.sh.cards().current_id(), Some("late"));
    h.run(1500.0);
    assert!(!h.sh.frame_expanded());
    assert_eq!(h.sh.cards().current_id(), Some("late"));
    assert_eq!(h.sh.phase(), Phase::Cards);
}

#[test]
fn cards_wait_while_away_and_start_after_show() {
    let mut h = H::with(json!({"autoHide": true}));
    h.run(2200.0);
    assert!(h.sh.is_away());
    h.sh.toast(card("n"));
    assert!(!h.sh.cards().has_current());
    h.ev(Event::Show);
    assert_eq!(h.sh.cards().current_id(), Some("n"));
}

#[test]
fn dictation_button_toasts_soon_until_the_voice_module_is_wired() {
    let mut h = H::new();
    h.ev(Event::DictButton);
    assert_eq!(h.sh.cards().current_id(), Some("dict-soon"));
    assert_eq!(h.sh.cards().current_spec().unwrap().title, "Dictation is coming soon");
    h.run(2200.0); // 1800 ms ttl + out
    assert!(!h.sh.cards().has_current());
    h.sh.set_dictate_available(true);
    h.ev(Event::DictButton);
    assert!(h.sh.take_notices().contains(&Notice::Dictate));
    assert!(!h.sh.cards().has_current());
    let mut ru = H::with(json!({"language": "ru"}));
    ru.ev(Event::DictButton);
    assert_eq!(ru.sh.cards().current_spec().unwrap().title, "Диктовка скоро");
}

#[test]
fn events_before_start_use_the_defaults() {
    let mut sh = Shell::new(RecBackend::default(), ManualClock::default(), "ru-RU", 1);
    assert_eq!(sh.lang(), super::settings::Lang::Ru);
    assert_eq!(sh.view().body.height, 30.0);
    sh.handle(Event::Hover);
    sh.handle(Event::Leave);
    sh.handle(Event::Collapse);
    sh.handle(Event::SettingsOpened(true)); // no pending open: ignored
    sh.handle(Event::DesktopFg(true)); // no pending query: ignored
    assert_eq!(sh.state(), PillState::Collapsed);
}

#[test]
fn spring_state_and_mode_table_are_exposed_for_the_renderer() {
    let mut h = H::new();
    h.ev(Event::Expand { tab: None, section: None });
    let s = h.sh.spring_state();
    assert_eq!(s[0].2, 680.0); // width target
    assert_eq!(s[1].2, 331.0); // height target
    let modes = super::spring::all_modes();
    assert_eq!(modes.len(), 5);
    assert_eq!(modes[0].1.w, Some((420.0, 0.6)));
}

#[test]
fn text_focus_sets_activation_for_keyboard_input() {
    let mut h = H::new();
    h.ev(Event::TextFocus(true));
    h.ev(Event::TextFocus(false));
    let on: Vec<Value> = h.sh.backend().all("pill_activate").into_iter().cloned().collect();
    assert_eq!(on, vec![json!({"on": true}), json!({"on": false})]);
}

#[test]
fn hidden_shell_ignores_the_hover_open_and_keeps_zero_work() {
    let mut h = H::new();
    h.ev(Event::Hide);
    h.sh.backend_mut().clear();
    h.ev(Event::Hover);
    h.run(500.0);
    h.sh.set_live("a", Some(slot(1.0)));
    h.run(70_000.0);
    // the hover timer fired (the JS does not check `hidden` there); everything polling stays quiet
    assert_eq!(h.calls("battery_state"), 0);
    assert_eq!(h.calls("weather_get"), 0);
}
