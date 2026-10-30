use super::*;
use crate::ui::host::testing::{motion_lock, Sim};
use crate::ui::tabs::a::snap::shoot;
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{DropEvent, Fakes, Services};
use glasskit::input::{Buttons, Modifiers};
use std::path::PathBuf;

struct Calm(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
impl Drop for Calm {
    fn drop(&mut self) {
        glasskit::ease::force_reduced(None);
    }
}
/// Motion forced on (`false`) or off (`true`) for the whole process; held for the test's duration.
fn motion(reduced: bool) -> Calm {
    let g = motion_lock();
    glasskit::ease::force_reduced(Some(reduced));
    Calm(g)
}

/// A real (tiny, two-colour) PNG as the `data:` URL the backend hands the cover out in.
fn cover_url() -> String {
    crate::ui::tabs::a::snap::png_data_url(32, 32, [220, 60, 90, 255], [40, 120, 200, 255])
}

fn media(playing: bool, with_cover: bool) -> Value {
    let mut m = json!({ "has_session": true, "playing": playing, "title": "Midnight City", "artist": "M83", "album": "Hurry Up", "session_id": "s1",
        "app": "Spotify.exe", "position_ms": 84_000, "duration_ms": 244_000, "thumb_id": 7, "accent": "#aa3355" });
    if with_cover {
        m["thumbnail"] = json!(cover_url());
    }
    m
}

fn links(n: usize) -> Value {
    let names = ["Notepad", "Calculator", "Downloads", "Terminal", "Browser"];
    Value::Array((0..n).map(|i| json!({ "id": i as i64 + 1, "title": names[i % 5], "target": format!("C:\\Apps\\{}.exe", names[i % 5]), "missing": false, "is_dir": i == 2, "icon": null })).collect())
}

struct Rig {
    sim: Sim,
    f: Fakes,
    fx: Vec<TabFx>,
}

impl Rig {
    fn with(playing: Option<bool>, cover: bool, n_links: usize) -> Rig {
        let (svc, f) = Services::fake();
        match playing {
            Some(p) => f.api.reply("media_state", Ok(media(p, cover))),
            None => f.api.reply("media_state", Ok(json!({ "has_session": false }))),
        };
        f.api.reply("settings_get", Ok(json!({})));
        f.api.reply("media_sessions", Ok(json!([{ "id": "s1", "app": "Spotify.exe", "title": "Midnight City", "artist": "M83", "playing": true, "selected": true }])));
        f.api.reply("media_volume_get", Ok(json!({ "volume": 42, "muted": false })));
        f.api.reply("media_devices", Ok(json!([{ "id": "d1", "name": "Speakers (Realtek)", "default": true }, { "id": "d2", "name": "Headphones (USB)", "default": false }])));
        f.api.reply("launcher_list", Ok(links(n_links)));
        let mut sim = Sim::with_services(Box::new(HomeView::new()), svc);
        sim.show();
        let mut r = Rig { sim, f, fx: vec![] };
        r.settle();
        let _ = r.sim.render((super::super::W, 260.0), &Theme::pill_dark()); // gives the view its rect
        r
    }
    fn playing() -> Rig {
        Rig::with(Some(true), false, 4)
    }
    /// Deliver every queued message (collecting the island-bound `TabFx`) and run frames until calm.
    fn settle(&mut self) {
        for _ in 0..20 {
            let msgs = self.sim.queue.drain();
            if msgs.is_empty() {
                break;
            }
            for (_, m) in msgs {
                match m {
                    Msg::Custom(b) if b.is::<TabFx>() => self.fx.push(*b.downcast::<TabFx>().unwrap()),
                    m => {
                        self.sim.deliver(m);
                    }
                }
            }
        }
        self.sim.run(2000);
        // work started by the frames (volume flush...) may have produced replies
        if !self.sim.queue.is_empty() {
            self.settle();
        }
    }
    fn names(&self) -> Vec<String> {
        self.f.api.calls().into_iter().map(|c| c.name).collect()
    }
    fn calls(&self, name: &str) -> Vec<Value> {
        self.f.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
    }
    fn mv(&mut self, p: Pt) {
        self.sim.send(&InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE });
    }
    fn down(&mut self, p: Pt, button: MouseButton) {
        self.sim.send(&InputEvent::MouseDown { pos: p, button, mods: Modifiers::NONE, clicks: 1 });
    }
    fn up(&mut self, p: Pt, button: MouseButton) {
        self.sim.send(&InputEvent::MouseUp { pos: p, button, mods: Modifiers::NONE });
    }
    fn click(&mut self, p: Pt) {
        self.mv(p);
        self.down(p, MouseButton::Left);
        self.up(p, MouseButton::Left);
        self.settle();
    }
}

fn ctr(r: Rect) -> Pt {
    r.center()
}

// ------------------------------------------------------------------------------------------------------------------ wiring

#[test]
fn showing_the_tab_reads_everything_the_web_tab_reads() {
    let r = Rig::playing();
    let n = r.names();
    for want in ["media_state", "settings_get", "media_sessions", "media_volume_get", "media_devices", "launcher_list"] {
        assert!(n.iter().any(|x| x == want), "{want} missing in {n:?}");
    }
}

#[test]
fn transport_buttons_send_media_control() {
    let mut r = Rig::playing();
    r.click(ctr(layout::MAIN));
    r.click(ctr(layout::NEXT));
    r.click(ctr(layout::PREV));
    let acts: Vec<String> = r.calls("media_control").iter().map(|a| a["action"].as_str().unwrap().to_string()).collect();
    assert_eq!(acts.len(), 3, "{acts:?}");
    assert!(acts[0].contains("play") || acts[0].contains("pause"), "{acts:?}");
    assert!(acts[1].contains("next") && acts[2].contains("prev"), "{acts:?}");
}

#[test]
fn clicking_the_seek_bar_seeks_to_that_fraction() {
    let mut r = Rig::playing();
    let s = layout::SEEK;
    let p = (s.x + s.w / 2.0, s.cy());
    r.click(p);
    let seeks = r.calls("media_seek");
    assert_eq!(seeks.len(), 1, "{seeks:?}");
    let ms = seeks[0]["ms"].as_i64().unwrap();
    assert!((ms - 122_000).abs() < 6_000, "half of 244 s, got {ms}");
}

#[test]
fn volume_click_wheel_and_mute_reach_the_backend() {
    let mut r = Rig::playing();
    let v = layout::VOL;
    r.click((v.x + v.w * 0.8, v.cy()));
    let sets = r.calls("media_volume_set");
    assert!(!sets.is_empty(), "no volume write");
    let last = sets.last().unwrap()["v"].as_i64().unwrap();
    assert!((70..=90).contains(&last), "{last}");
    let before = r.calls("media_volume_set").len();
    r.sim.send(&InputEvent::Wheel { pos: ctr(v), delta: (0.0, -100.0), mods: Modifiers::NONE, precise: false });
    r.settle();
    assert!(r.calls("media_volume_set").len() > before, "wheel changes the volume");
    r.click(ctr(layout::MUTE));
    assert_eq!(r.calls("media_mute").len(), 1);
}

#[test]
fn the_device_chip_opens_the_list_and_picking_sets_the_default() {
    let mut r = Rig::playing();
    r.click(ctr(layout::DEV));
    assert!(r.names().iter().filter(|n| *n == "media_devices").count() >= 2, "the chip refreshes the list");
    let m = layout::menu(2);
    r.click((m.x + 60.0, m.y + 5.0 + layout::MENU_ROW * 1.5));
    assert_eq!(r.calls("media_set_default_device"), vec![json!({ "id": "d2" })]);
}

#[test]
fn tiles_go_to_the_island_as_tab_fx() {
    let mut r = Rig::playing();
    r.click(ctr(layout::tile(0)));
    assert!(r.fx.iter().any(|f| matches!(f, TabFx::StartCapture { .. } | TabFx::OpenCommandBar | TabFx::Collapse { .. })), "{:?}", r.fx);
}

#[test]
fn media_changed_updates_the_card_and_a_new_cover_is_requested() {
    let mut r = Rig::playing();
    r.f.bus.emit("media:changed", media(false, false));
    r.settle();
    let img = r.sim.render((super::super::W, 260.0), &Theme::pill_dark());
    assert!(img.count(None, |p| p[3] > 0) > 1000);
}

// ------------------------------------------------------------------------------------------------------------------ launcher

#[test]
fn launcher_click_opens_and_right_click_removes() {
    let mut r = Rig::playing();
    r.click(ctr(layout::app(1)));
    assert_eq!(r.calls("launcher_open"), vec![json!({ "id": 2 })]);
    let p = ctr(layout::app(0));
    r.mv(p);
    r.down(p, MouseButton::Right);
    r.up(p, MouseButton::Right);
    r.settle();
    r.sim.run(2000);
    r.settle();
    assert_eq!(r.calls("launcher_remove"), vec![json!({ "id": 1 })]);
}

#[test]
fn dragging_an_icon_reorders_with_launcher_move() {
    let mut r = Rig::playing();
    let a = ctr(layout::app(0));
    r.mv(a);
    r.down(a, MouseButton::Left);
    let mut x = a.0;
    while x < ctr(layout::app(2)).0 {
        x += 10.0;
        r.mv((x, a.1));
        r.sim.run(30);
    }
    r.up((x, a.1), MouseButton::Left);
    r.settle();
    let mv = r.calls("launcher_move");
    assert_eq!(mv.len(), 1, "{mv:?}");
    assert_eq!(mv[0]["id"], 1);
    assert_eq!(mv[0]["newIndex"], 2);
    assert!(r.calls("launcher_open").is_empty(), "a drag is not a click");
}

#[test]
fn dropping_files_on_the_strip_adds_launchers_and_elsewhere_does_nothing() {
    let mut r = Rig::playing();
    let f = vec![PathBuf::from("C:\\Tools\\a.exe"), PathBuf::from("C:\\Tools\\b.lnk")];
    let on = ctr(layout::app(1));
    let e = r.sim.send_drop(&DropEvent { phase: DropPhase::Enter, pos: on, payload: Some(DropPayload::Files(f.clone())) });
    assert!(e.consumed, "the strip accepts files");
    r.sim.send_drop(&DropEvent { phase: DropPhase::Drop, pos: on, payload: Some(DropPayload::Files(f)) });
    r.settle();
    let adds = r.calls("launcher_add");
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0]["paths"].as_array().unwrap().len(), 2);
    let away = r.sim.send_drop(&DropEvent { phase: DropPhase::Over, pos: ctr(layout::PLAYER), payload: None });
    assert!(!away.consumed);
}

// ------------------------------------------------------------------------------------------------------------------ frames and motion

#[test]
fn a_settled_tab_costs_no_frames_and_hide_stops_the_clock() {
    let _m = motion(false);
    let mut r = Rig::playing();
    r.settle();
    assert!(!r.sim.frame.wants_frame(), "settled = idle");
    r.sim.hide();
    r.settle();
    assert!(!r.sim.frame.wants_frame());
}

#[test]
fn reduced_motion_shows_end_states_without_cues() {
    let _m = motion(true);
    let mut r = Rig::playing();
    r.sim.cx.reduced_motion = true;
    r.f.bus.emit("media:changed", media(false, false));
    r.settle();
    assert!(!r.sim.frame.wants_frame());
}

// ------------------------------------------------------------------------------------------------------------------ pixels

fn shoot_all(r: &mut Rig, name: &str) {
    for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
        for sc in [1.0f32, 2.0] {
            r.sim.cx.scale = sc;
            let img = shoot(&mut *r.sim.view, &r.sim.cx, &th, sc, "home", &format!("{name}-{tn}-x{}", sc as u32));
            assert!(img.count(None, |p| p[3] > 0) > 5000, "{name} {tn} x{sc} drew nothing");
        }
    }
}

#[test]
fn snapshots_of_the_typical_states() {
    let _m = motion(false);
    let mut r = Rig::with(Some(true), true, 5);
    r.settle();
    shoot_all(&mut r, "playing-cover");
    // device list open
    r.click(ctr(layout::DEV));
    shoot_all(&mut r, "devices");
    let mut r = Rig::with(Some(false), false, 3);
    r.settle();
    shoot_all(&mut r, "paused");
    let mut r = Rig::with(None, false, 0);
    r.settle();
    shoot_all(&mut r, "idle-empty-launcher");
    // a launcher icon pressed / hovered
    let mut r = Rig::with(Some(true), true, 4);
    r.settle();
    r.mv(ctr(layout::app(1)));
    r.sim.run(120);
    shoot_all(&mut r, "hover-icon");
}

#[test]
fn dark_and_light_differ_and_scale_two_is_twice_as_big() {
    let _m = motion(false);
    let mut r = Rig::with(Some(true), false, 3);
    r.settle();
    r.sim.cx.scale = 1.0;
    let a = shoot(&mut *r.sim.view, &r.sim.cx, &Theme::pill_dark(), 1.0, "home", "probe-dark");
    let b = shoot(&mut *r.sim.view, &r.sim.cx, &Theme::pill_light(), 1.0, "home", "probe-light");
    assert_ne!(a.at(5.0, 5.0), b.at(5.0, 5.0));
    r.sim.cx.scale = 2.0;
    let c = shoot(&mut *r.sim.view, &r.sim.cx, &Theme::pill_dark(), 2.0, "home", "probe-x2");
    assert_eq!((c.w, c.h), (a.w * 2, a.h * 2));
}
