use super::*;
use crate::ui::host::testing::{motion_lock, Sim};
use crate::ui::tabs::a::core::Tick;
use crate::ui::tabs::a::snap::shoot;
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{Fakes, MsgQueue, Services};
use glasskit::input::{Buttons, Modifiers};
use serde_json::json;
use std::sync::Arc;

pub const NOW: i64 = 1_793_369_109_000; // 2026-10-30 14:05:09 UTC
const W: Wall = Wall { ms: NOW, tz_min: 0 };
const MIN: i64 = 60_000;
const HOUR: i64 = 60 * MIN;

struct Calm(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
impl Drop for Calm {
    fn drop(&mut self) {
        glasskit::ease::force_reduced(None);
    }
}
/// Motion forced on (or off) for the whole process; held for the test's duration.
fn motion(reduced: bool) -> Calm {
    let g = motion_lock();
    glasskit::ease::force_reduced(Some(reduced));
    Calm(g)
}

fn e(id: i64, ago_ms: i64, kind: &str, icon: &str, tone: &str, title: &str, sub: &str, read: bool) -> Value {
    json!({ "id": id, "ts": NOW - ago_ms, "kind": kind, "icon": icon, "tone": tone, "title": title, "sub": sub, "read": read })
}

/// A realistic history (producers: notifhist.rs / the cards of notif.rs, download, usb, battery, calendar, agents).
fn history() -> Value {
    json!([
        e(9, 20_000, "download", "download", "green", "report-final.pdf", "Downloaded · 2.4 MB", false),
        e(8, 12 * MIN, "usb", "usb", "", "USB drive connected", "E: · 58 GB", false),
        e(7, 50 * MIN, "battery", "battery", "amber", "Battery low", "18% · plug in the charger", true),
        e(6, 3 * HOUR, "calendar", "calendar", "accent", "Design review", "Starts in 10 minutes", true),
        e(5, 5 * HOUR, "mail", "mail", "", "Anna Petrova", "", true),
        e(4, 20 * HOUR, "agent", "terminal", "", "Agent finished", "build passed, 312 tests", true),
        e(3, 22 * HOUR, "obs", "video", "red", "Recording stopped", "00:42:10 · saved to Videos", true),
        e(2, 3 * 24 * HOUR, "notif", "bell", "", "Windows update", "", true),
        e(1, 3 * 24 * HOUR + HOUR, "steam", "download", "green", "Game update installed", "Hades II · 1.2 GB", true),
    ])
}

fn many() -> Value {
    let kinds = [("download", "download", "green"), ("usb", "usb", ""), ("battery", "battery", "amber"), ("calendar", "calendar", "accent"), ("agent", "terminal", ""), ("notif", "bell", "")];
    Value::Array(
        (0..60)
            .map(|i| {
                let (k, ic, t) = kinds[i % kinds.len()];
                e(100 - i as i64, (i as i64) * 2 * HOUR + 30 * MIN, k, ic, t, &format!("Event number {i}"), if i % 3 == 0 { "" } else { "Something happened just now" }, true)
            })
            .collect(),
    )
}

/// The view with its own context so a test can reach the private state.
struct Rig {
    v: NotifsView,
    cx: Cx,
    q: Arc<MsgQueue>,
    fakes: Fakes,
    got: Vec<TabFx>,
}

impl Rig {
    fn new(list: Value) -> Rig {
        let (svc, fakes) = Services::fake();
        fakes.api.reply("notif_history_list", Ok(list));
        fakes.api.reply("settings_get", Ok(json!({})));
        let (cx, q) = Cx::headless_queue(svc);
        Rig { v: NotifsView::new().with_now(W), cx, q, fakes, got: vec![] }
    }
    fn open(list: Value) -> Rig {
        let mut r = Rig::new(list);
        r.v.on_show(&mut r.cx);
        r.pump();
        r
    }
    fn pump(&mut self) {
        loop {
            let msgs = self.q.drain();
            if msgs.is_empty() {
                break;
            }
            for (_, m) in msgs {
                match m {
                    Msg::Custom(b) if b.is::<TabFx>() => self.got.push(*b.downcast::<TabFx>().unwrap()),
                    m => {
                        self.v.message(m, &mut self.cx);
                    }
                }
            }
        }
    }
    /// Run frames of 16 ms (a 60 Hz display) until the view is calm.
    fn settle(&mut self) {
        for _ in 0..400 {
            if !self.v.step(16.0, &mut self.cx) {
                break;
            }
        }
    }
    fn names(&self) -> Vec<String> {
        self.fakes.api.calls().into_iter().map(|c| c.name).collect()
    }
    fn calls(&self, name: &str) -> Vec<Value> {
        self.fakes.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
    }
    fn fx(&mut self) -> Vec<TabFx> {
        let mut out = std::mem::take(&mut self.got);
        for (_, m) in self.q.drain() {
            if let Msg::Custom(b) = m {
                if let Ok(f) = b.downcast::<TabFx>() {
                    out.push(*f);
                }
            }
        }
        out
    }
    fn mv(&mut self, p: (f32, f32)) {
        let ev = InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE };
        self.v.event(&ev, &mut self.cx);
    }
    fn click(&mut self, p: (f32, f32)) {
        self.mv(p);
        let down = InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 };
        let up = InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE };
        self.v.event(&down, &mut self.cx);
        self.v.event(&up, &mut self.cx);
    }
    fn row_ids(&self) -> Vec<i64> {
        self.v.items.iter().filter(|i| !i.ghost).filter_map(|i| i.id()).collect()
    }
}

// first row: list top 32 + day header 22 = 54, 46 high -> centre 77; the remove button sits at x 610..636
const ROW0: (f32, f32) = (300.0, 77.0);
const X0: (f32, f32) = (623.0, 77.0);

// ------------------------------------------------------------------------------------------------------------------ wiring

#[test]
fn opening_loads_the_history_and_settings_and_marks_unread_as_read() {
    let r = Rig::open(history());
    assert_eq!(r.names()[..2], ["notif_history_list", "settings_get"]);
    assert_eq!(r.calls("notif_history_mark_read"), vec![json!({ "ids": [9, 8] })]);
    assert_eq!(r.row_ids(), vec![9, 8, 7, 6, 5, 4, 3, 2, 1]);
    // day headers: Today, Yesterday, Oct 27
    let days: Vec<_> = r.v.items.iter().filter_map(|i| if let Kind::Day(d) = &i.kind { Some(d.as_str()) } else { None }).collect();
    assert_eq!(days, ["Today", "Yesterday", "October 27"]);
}

#[test]
fn badge_goes_to_the_island_as_a_tab_fx() {
    let mut r = Rig::open(history());
    let fx = r.fx();
    assert!(fx.contains(&TabFx::Badge { tab: "notifs", text: "".into() }), "{fx:?}");
}

#[test]
fn the_x_of_a_row_removes_it_with_the_exact_command() {
    let _m = motion(false);
    let mut r = Rig::open(history());
    r.settle();
    r.click(X0);
    assert_eq!(r.calls("notif_history_remove"), vec![json!({ "id": 9 })]);
    assert_eq!(r.row_ids()[0], 8);
    // the removed row is a ghost until its collapse ends, then gone
    assert!(r.v.items.iter().any(|i| i.ghost && i.id() == Some(9)));
    r.settle();
    assert!(!r.v.items.iter().any(|i| i.ghost), "ghost reaped");
}

#[test]
fn clear_needs_two_clicks_and_disarms_after_three_seconds() {
    let _m = motion(false);
    let mut r = Rig::open(history());
    r.settle();
    let btn = r.v.clear_rect(None);
    let c = (btn.cx(), btn.cy());
    r.click(c);
    assert!(r.calls("notif_history_clear").is_empty());
    assert!(r.v.nview.as_ref().unwrap().clear_armed);
    // 3.5 s later the arm expires
    r.cx.now_ms = 3600.0;
    r.v.message(Msg::Custom(Box::new(Tick)), &mut r.cx);
    assert!(!r.v.nview.as_ref().unwrap().clear_armed);
    r.click(c);
    r.click(c);
    assert_eq!(r.calls("notif_history_clear").len(), 1);
    r.pump();
    r.settle();
    assert!(r.v.nview.as_ref().unwrap().empty && r.row_ids().is_empty());
}

#[test]
fn a_rejected_clear_brings_the_rows_back() {
    let _m = motion(false);
    let mut r = Rig::open(history());
    r.fakes.api.reply("notif_history_clear", Err("db busy".into()));
    r.settle();
    let btn = r.v.clear_rect(None);
    r.click((btn.cx(), btn.cy()));
    r.click((btn.cx(), btn.cy()));
    r.pump();
    assert_eq!(r.row_ids().len(), 9);
    assert!(!r.v.base.player.is_active("row:9"));
}

#[test]
fn a_toast_from_the_island_is_recorded_and_the_answer_prepends_a_row() {
    let _m = motion(false);
    let mut r = Rig::open(history());
    r.settle();
    r.fakes.api.reply("notif_history_add", Ok(e(10, 0, "download", "download", "", "setup.exe", "done", false)));
    let rec = ToastRec { id: "download:77".into(), title: "setup.exe".into(), sub: "done".into(), ..Default::default() };
    r.v.message(Msg::Custom(Box::new(rec)), &mut r.cx);
    r.pump();
    assert_eq!(r.calls("notif_history_add"), vec![json!({ "entry": { "kind": "download", "icon": "download", "tone": "", "title": "setup.exe", "sub": "done" }, "read": true })]);
    assert_eq!(r.row_ids()[0], 10);
    // the FLIP: older rows start displaced by the new row's height, the new one fades in
    assert!(r.v.base.player.is_active("list"));
    assert!(r.v.flip_new.contains(&10));
    assert!(r.v.flip.values().any(|d| *d < -40.0), "{:?}", r.v.flip);
    r.settle();
    assert!(r.v.flip.is_empty());
}

#[test]
fn settings_changed_event_reaches_the_model() {
    let mut r = Rig::open(history());
    r.fakes.bus.emit("settings:changed", json!({ "history.keep": false }));
    r.pump();
    assert!(r.v.nview.as_ref().unwrap().off);
}

#[test]
fn reduced_motion_plays_no_cue_and_needs_no_frames() {
    let _m = motion(true);
    let mut r = Rig::new(history());
    r.cx.reduced_motion = true;
    r.v.on_show(&mut r.cx);
    r.pump();
    assert!(!r.v.base.player.busy());
    let btn = r.v.clear_rect(None);
    r.click((btn.cx(), btn.cy()));
    r.click((btn.cx(), btn.cy()));
    r.pump();
    assert!(!r.v.base.player.busy(), "clear lands at once");
    assert!(r.v.nview.as_ref().unwrap().empty);
}

#[test]
fn wheel_scrolls_a_long_history_and_idle_costs_no_frames() {
    let _m = motion(false);
    let (svc, fakes) = Services::fake();
    fakes.api.reply("notif_history_list", Ok(many()));
    let mut sim = Sim::with_services(Box::new(NotifsView::new().with_now(W)), svc);
    sim.show();
    sim.pump();
    sim.pump();
    sim.run(600);
    { let v = &mut sim.view; let _ = v; }
    assert!(!sim.frame.wants_frame(), "settled = idle {}", sim.frames_run);
    let _ = sim.render((super::super::W, 260.0), &Theme::pill_dark()); // gives the view its rect
    let ev = InputEvent::Wheel { pos: (300.0, 120.0), delta: (0.0, 3.0), precise: false, mods: Modifiers::NONE };
    sim.send(&ev);
    assert!(sim.run(600) > 0, "the scroll glides over frames");
    assert!(!sim.frame.wants_frame());
    let img = sim.render((super::super::W, 260.0), &Theme::pill_dark());
    assert!(img.count(None, |p| p[3] > 0) > 1000);
}

// ------------------------------------------------------------------------------------------------------------------ pixels

fn light() -> Theme {
    Theme::pill_light()
}

/// Dark + light, scale 1 and 2 of the current state.
fn shoot_all(r: &mut Rig, name: &str) {
    for (th, tn) in [(Theme::pill_dark(), "dark"), (light(), "light")] {
        for sc in [1.0f32, 2.0] {
            r.cx.scale = sc;
            let img = shoot(&mut r.v, &r.cx, &th, sc, "notifs", &format!("{name}-{tn}-x{}", sc as u32));
            assert!(img.count(None, |p| p[3] > 0) > 5000, "{name} {tn} x{sc} drew nothing");
        }
    }
}

#[test]
fn snapshots_of_the_typical_states() {
    let _m = motion(false);
    // empty
    let mut r = Rig::open(json!([]));
    r.settle();
    shoot_all(&mut r, "empty");
    // a few items; the unread dots and bold titles are still there because the tab was just opened
    let mut r = Rig::open(history());
    r.settle();
    shoot_all(&mut r, "few");
    // hover on the first row (the remove button replaces the time)
    r.mv(ROW0);
    r.settle();
    shoot_all(&mut r, "hover");
    r.mv(X0);
    r.settle();
    shoot_all(&mut r, "hover-x");
    // armed clear button
    r.mv((5.0, 200.0));
    let btn = r.v.clear_rect(None);
    r.click((btn.cx(), btn.cy()));
    r.settle();
    shoot_all(&mut r, "armed");
    // many items scrolled down so the sticky day header rides over rows
    let mut r = Rig::open(many());
    r.settle();
    r.v.draw_once_for_tests();
    r.v.scroll.state.scroll_to(540.0, false);
    shoot_all(&mut r, "many-scrolled");
    // history switched off
    let mut r = Rig::open(json!([]));
    r.fakes.bus.emit("settings:changed", json!({ "history.keep": false }));
    r.pump();
    r.settle();
    shoot_all(&mut r, "off");
    // Russian
    let mut r = Rig::new(history());
    r.cx.lang = Lang::Ru;
    r.v.on_show(&mut r.cx);
    r.pump();
    r.settle();
    shoot_all(&mut r, "ru");
    // reduced motion: the same state with no cue ever played
    let _m2 = ();
    let mut r = Rig::new(history());
    r.cx.reduced_motion = true;
    r.v.on_show(&mut r.cx);
    r.pump();
    shoot_all(&mut r, "reduced");
}

#[test]
fn enter_cue_starts_rows_hidden_then_settles_visible() {
    let _m = motion(false);
    let mut r = Rig::open(history());
    // right after opening the stagger has just begun: the last rows are still invisible
    assert!(r.v.base.player.is_active("row:1"));
    assert_eq!(r.v.base.player.sample("row:1").alpha, 0.0);
    r.settle();
    assert_eq!(r.v.base.player.sample("row:1").alpha, 1.0);
}

impl NotifsView {
    /// Tests: give the view a rect without a canvas (what the first draw does).
    fn draw_once_for_tests(&mut self) {
        self.base.rect = Rect::new(0.0, 0.0, super::super::W, 260.0);
        self.refresh();
    }
}
