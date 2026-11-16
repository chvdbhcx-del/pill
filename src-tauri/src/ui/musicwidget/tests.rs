//! Headless tests of the music widget: the settings -> window mapping, the show / hide / hover / click / seek behaviour on the real frame
//! scheduler (`host::testing::Sim`), the idle cost, the privacy gate, the fullscreen guard, Reduce motion, and one render. `PILL_SNAP_DIR=<dir>`
//! writes the PNG of the render.
use super::logic::{self as lg, Btn, WIN};
use super::view::MusicView;
use super::*;
use crate::ui::host::testing::{motion_lock, motion_on, Sim};
use crate::ui::host::geom::{self, Align, Dock};
use crate::ui::view::{Cx, Effects, Fakes, Msg, ProximityEvent, Services, View};
use glasskit::geom::Rect;
use glasskit::input::{Buttons, InputEvent, Modifiers, MouseButton};
use glasskit::theme::Theme;
use serde_json::json;

mod motion; // contact sheets of the motion (PILL_SNAP_DIR)

// ------------------------------------------------------------------------------------------------------------------------------ helpers

struct Blank;
impl View for Blank {
    fn draw(&mut self, _: &glasskit::canvas::Canvas, _: &Theme, _: Rect, _: &Cx) {}
}

fn cfg(extra: Value) -> Value {
    let mut base = json!({ "media.widget": true, "module.media": true, "theme": "dark", "language": "en" });
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        b.extend(e.clone());
    }
    base
}

/// A simulated widget window on `dock`; `settings` answers `settings_get`.
fn sim(dock: Dock, settings: Value) -> (Sim, Fakes) {
    let (svc, fakes) = Services::fake();
    fakes.api.reply("settings_get", Ok(settings));
    let mut s = Sim::with_services(Box::new(Blank), svc);
    let v = MusicView::new(&mut s.cx, dock);
    s.view = Box::new(v);
    s.pump();
    (s, fakes)
}

fn state(playing: bool, title: &str, pos: u64) -> Value {
    json!({ "has_session": true, "playing": playing, "title": title, "artist": "Band", "album": "LP", "app": "Spotify.exe", "position_ms": pos, "duration_ms": 200_000,
            "thumb_id": 3, "thumbnail": "data:image/png;base64,AAAA", "accent": "#cc3366" })
}

fn media(s: &mut Sim, fakes: &Fakes, v: Value) {
    fakes.bus.emit("media:changed", v);
    s.pump();
}

/// What the host does for a hidden -> visible window: `on_show`, then frames until it settles. Returns the effects up to then.
fn show(s: &mut Sim) -> Effects {
    s.show();
    s.run(4000);
    s.take_fx()
}

fn settle(s: &mut Sim) -> usize {
    s.pump();
    s.run(4000)
}

fn mv(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
}
fn down(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseDown { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }
}
fn up(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseUp { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE }
}

fn hit_rect(fx: &Effects) -> Rect {
    fx.hit_region.as_ref().and_then(|r| r.first().copied()).expect("a hit region")
}

/// The window is the real one (`WIN_REAL`, the same for every size); the layout in `logic` is in base px at 100 %. These bring a base point or
/// rectangle of a stand into the window, where the events and the hit region are.
fn base(dock: Dock) -> lg::Stand {
    lg::Stand::from(dock)
}
fn bar_st(align: Align, bar: f32, scale: f32) -> lg::Stand {
    lg::Stand { dock: Dock::Taskbar, align, bar, scale, k: 1.0, dx: 0.0 }
}

fn calls(f: &Fakes, name: &str) -> Vec<Value> {
    f.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
}

/// A playing widget on screen, settled.
fn on_screen(dock: Dock) -> (Sim, Fakes) {
    let (mut s, f) = sim(dock, cfg(json!({})));
    media(&mut s, &f, state(true, "Song", 61_000));
    let _ = show(&mut s);
    (s, f)
}

// ------------------------------------------------------------------------------------------------------------------------------ settings -> window

fn sig(v: Value) -> Option<Sig> {
    sig_from(&|k| v.get(k).cloned().unwrap_or(Value::Null))
}

#[test]
fn off_by_default_and_the_placement_follows_the_settings() {
    assert_eq!(sig(json!({})), None, "no key: no widget");
    assert_eq!(sig(json!({ "media.widget": false })), None);
    assert_eq!(sig(json!({ "media.widget": "on" })), None, "only a real true");
    let s = sig(json!({ "media.widget": true })).unwrap();
    assert_eq!((s.dock, s.align, s.offset, s.monitor.as_str(), s.inside_work, s.in_capture), (Dock::Bottom, Align::Left, 0, "primary", true, true), "bottom left by default");
    let s = sig(json!({ "media.widget": true, "media.widget.edge": "top", "media.widget.align": "center", "media.widget.offset": -20.4, "monitor": "2", "underTaskbar": false, "showInCapture": false })).unwrap();
    assert_eq!((s.dock, s.align, s.offset, s.monitor.as_str(), s.inside_work, s.in_capture), (Dock::Top, Align::Center, -20, "2", false, false));
    let s = sig(json!({ "media.widget": true, "media.widget.align": "right", "media.widget.offset": 900, "media.widget.edge": "sideways" })).unwrap();
    assert_eq!((s.dock, s.align, s.offset), (Dock::Top, Align::Right, 50), "an unknown edge is the top like the island's; the offset is held in range");
    assert_eq!(monitor_sel("primary"), MonitorSel::Primary);
    assert_eq!(monitor_sel("active"), MonitorSel::Island, "the island follows the active monitor: the widget stands where it does");
    let s = sig(json!({ "media.widget": true, "monitor": "2", "edge.monitor": "active" })).unwrap();
    assert_eq!(s.monitor, "active", "following beats the explicit monitor");
    let s = sig(json!({ "media.widget": true, "monitor": "2", "edge.monitor": "primary" })).unwrap();
    assert_eq!(s.monitor, "2", "not following: the explicit choice stands");
    assert!(KEYS.contains(&"edge.monitor"));
    assert_eq!((monitor_sel("1"), monitor_sel("3"), monitor_sel("0"), monitor_sel("x")), (MonitorSel::Index(0), MonitorSel::Index(2), MonitorSel::Primary, MonitorSel::Primary));
    let p = placement(&sig(json!({ "media.widget": true })).unwrap());
    assert!(matches!(p, Placement::Docked { dock: Dock::Bottom, align: Align::Left, inside_work: true, margin, .. } if margin == 0.0));
}

#[test]
fn the_window_is_one_hidden_click_through_overlay_the_size_of_the_frame() {
    let sp = spec(&sig(json!({ "media.widget": true, "showInCapture": false })).unwrap()).into_spec();
    assert_eq!((sp.kind, sp.size, sp.visible, sp.topmost, sp.activate_on_show, sp.exclude_from_capture), (host::WindowKind::Overlay, lg::WIN_REAL, false, true, false, true));
    let sp = spec(&sig(json!({ "media.widget": true })).unwrap()).into_spec();
    assert!(!sp.exclude_from_capture && sp.title == "pill-music" && sp.label == "music-widget", "its own title and label, not the island's");
}

// ------------------------------------------------------------------------------------------------------------------------------ presence

#[test]
fn nothing_playing_nothing_on_screen_and_no_frames() {
    let (mut s, f) = sim(Dock::Bottom, cfg(json!({})));
    assert_eq!(calls(&f, "media_state").len(), 1, "it asks for the player once at the start");
    s.run(10); // the first frame of a fresh window
    assert_eq!(s.run(100), 0, "idle");
    let fx = s.take_fx();
    assert!(!fx.show && !fx.hide, "it never asks for a window without a track");
    media(&mut s, &f, json!({ "has_session": false, "playing": false }));
    assert!(!s.take_fx().show);
    media(&mut s, &f, state(false, "Song", 0));
    assert!(!s.take_fx().show, "a player that is already paused does not call it up");
    assert!(!s.frame.wants_frame());
}

#[test]
fn a_playing_track_calls_the_window_up_and_it_settles_in_the_corner_with_the_hit_region_on_the_capsule() {
    let _m = motion_on();
    let (mut s, f) = sim(Dock::Bottom, cfg(json!({})));
    media(&mut s, &f, state(true, "Song", 61_000));
    assert!(s.take_fx().show, "the view asks the host to show its window");
    s.show();
    assert!(s.run(4000) > 10, "the enter animation runs");
    let fx = s.take_fx();
    let r = hit_rect(&fx);
    assert_eq!((r.x, r.w, r.h), (lg::PAD, lg::BODY_W, lg::CAPSULE_H), "the capsule, nothing more");
    assert!((r.bottom() - (lg::WIN_REAL.1 - lg::PAD)).abs() < 0.2, "standing on the bottom edge: {r:?}");
    assert!(!s.frame.wants_frame(), "settled: no frames at all");
    assert!(!fx.hide, "and it stays");
}

#[test]
fn a_top_widget_hangs_from_the_top_edge() {
    let _m = motion_on();
    let (mut s, f) = sim(Dock::Top, cfg(json!({})));
    media(&mut s, &f, state(true, "Song", 0));
    let fx = show(&mut s);
    let r = hit_rect(&fx);
    assert_eq!((r.y, r.h), (lg::PAD, lg::CAPSULE_H));
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    let r = hit_rect(&s.take_fx());
    assert!((r.y - lg::PAD).abs() < 0.2 && (r.h - lg::CARD_H).abs() < 0.3, "the card grows downward: {r:?}");
}

#[test]
fn a_pause_keeps_it_for_a_while_and_then_it_leaves_by_itself() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    media(&mut s, &f, state(false, "Song", 70_000));
    settle(&mut s);
    assert!(!s.take_fx().hide, "paused a moment ago: still there");
    // the tick of the last second of the grace: still there; the first after it: it leaves
    s.now_ms += lg::PAUSE_GRACE_MS - 5_000.0;
    s.cx.now_ms = s.now_ms;
    s.deliver(Msg::Custom(Box::new(crate::ui::tabs::a::core::Tick)));
    assert_eq!(s.run(10), 0, "still inside the grace: nothing moves");
    s.now_ms += 6_000.0;
    s.cx.now_ms = s.now_ms;
    s.deliver(Msg::Custom(Box::new(crate::ui::tabs::a::core::Tick)));
    assert!(s.run(4000) > 5, "the leave animation");
    assert!(s.take_fx().hide, "and then the window goes away");
}

#[test]
fn playing_again_during_the_leave_brings_it_back_and_a_new_session_after_it_left_shows_again() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    media(&mut s, &f, json!({ "has_session": false }));
    s.run(8); // half way out
    media(&mut s, &f, state(true, "Song", 0));
    assert!(s.run(4000) > 0);
    let fx = s.take_fx();
    assert!(!fx.hide && (hit_rect(&fx).bottom() - (lg::WIN_REAL.1 - lg::PAD)).abs() < 0.3, "it turned around and settled in place");
    media(&mut s, &f, json!({ "has_session": false }));
    assert!(s.run(4000) > 0);
    assert!(s.take_fx().hide);
    s.hide();
    media(&mut s, &f, state(true, "Next", 0));
    assert!(s.take_fx().show, "a window that left is called up again");
}

// ------------------------------------------------------------------------------------------------------------------------------ privacy and the guard

#[test]
fn the_media_module_switch_is_the_privacy_gate() {
    let (mut s, f) = sim(Dock::Bottom, cfg(json!({ "module.media": false })));
    media(&mut s, &f, state(true, "Song", 0));
    let fx = s.take_fx();
    assert!(!fx.show, "module.media off: no track is read, nothing shows");
    s.run(10);
    assert_eq!(s.run(50), 0);
    // switching it on later asks for the state
    let before = calls(&f, "media_state").len();
    fakes_settings(&mut s, &f, cfg(json!({})));
    assert!(calls(&f, "media_state").len() > before, "back on: it asks for the player");
    // and switching it off while the widget shows takes it away at once
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    fakes_settings(&mut s, &f, cfg(json!({ "module.media": false })));
    assert!(s.run(4000) > 0);
    assert!(s.take_fx().hide, "gone, with its title");
}

fn fakes_settings(s: &mut Sim, f: &Fakes, v: Value) {
    f.bus.emit("settings:changed", v);
    s.pump();
}

#[test]
fn the_fullscreen_guard_hides_it_with_the_island_and_it_comes_back_after() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    f.bus.emit(GUARD_EVENT, json!({ "hidden": true }));
    s.pump();
    assert!(s.run(4000) > 0);
    assert!(s.take_fx().hide, "a fullscreen app: the widget leaves");
    s.hide();
    f.bus.emit("media:changed", state(true, "Song", 70_000));
    s.pump();
    assert!(!s.take_fx().show, "and does not come back while the guard holds, whatever plays");
    f.bus.emit(GUARD_EVENT, json!({ "hidden": false }));
    s.pump();
    assert!(s.take_fx().show, "the guard let go: it is called up again");
}

// ------------------------------------------------------------------------------------------------------------------------------ hover, clicks, seek

#[test]
fn hovering_opens_the_card_and_leaving_closes_it_and_it_idles_both_times() {
    let _m = motion_on();
    let (mut s, _f) = on_screen(Dock::Bottom);
    s.send_proximity(&ProximityEvent::Hover);
    assert!(s.run(4000) > 10, "it opens with a spring");
    let r = hit_rect(&s.take_fx());
    assert!((r.h - lg::CARD_H).abs() < 0.3 && (r.bottom() - (lg::WIN_REAL.1 - lg::PAD)).abs() < 0.3, "the card: {r:?}");
    assert!(!s.frame.wants_frame());
    s.send_proximity(&ProximityEvent::Leave);
    assert!(s.run(4000) > 10);
    let r = hit_rect(&s.take_fx());
    assert!((r.h - lg::CAPSULE_H).abs() < 0.3, "the capsule again: {r:?}");
    assert!(!s.frame.wants_frame(), "settled: no frames");
}

#[test]
fn the_buttons_send_the_media_commands_of_the_islands_player() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    let st = base(Dock::Bottom);
    let (cap, _) = lg::frames(Dock::Bottom, 0.0, 0.0);
    let l = lg::capsule_layout(cap);
    for (b, action) in [(Btn::Prev, "prev"), (Btn::Play, "play_pause"), (Btn::Next, "next")] {
        let c = st.to_real(l.btn[b.index()].center());
        s.send(&mv(c.0, c.1));
        s.send(&down(c.0, c.1));
        assert!(calls(&f, "media_control").iter().all(|a| a["action"] != action), "not before the button is released");
        assert!(s.run(4000) > 0, "the press dips and the hover lights");
        s.send(&up(c.0, c.1));
        s.run(4000);
        assert_eq!(calls(&f, "media_control").last().unwrap(), &json!({ "action": action }));
    }
    assert_eq!(calls(&f, "media_control").len(), 3);
    // a press that is dragged off the button does nothing
    let c = st.to_real(l.btn[1].center());
    let cover = st.to_real(l.cover.center());
    s.send(&down(c.0, c.1));
    s.send(&up(cover.0, cover.1));
    assert_eq!(calls(&f, "media_control").len(), 3);
    // a click on the cover (the body) is swallowed; outside the body nothing is ours
    assert!(s.send(&down(cover.0, cover.1)).consumed);
    assert!(!s.send(&down(1.0, 1.0)).consumed);
}

#[test]
fn the_card_seeks_by_a_click_or_a_drag_and_commits_once_on_release() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    let (_, card) = lg::frames(Dock::Bottom, 0.0, 0.0);
    let bar = base(Dock::Bottom).rect_to_real(lg::card_layout(card).bar);
    let y = bar.cy();
    let r = s.send(&down(bar.x + bar.w * 0.25, y));
    assert!(r.capture && r.consumed, "the press is captured so the drag keeps going outside");
    s.send(&mv(bar.x + bar.w * 0.5, y + 40.0));
    s.send(&mv(bar.x + bar.w * 0.75, y));
    assert!(calls(&f, "media_seek").is_empty(), "nothing is sent while dragging");
    s.run(4000);
    s.send(&up(bar.x + bar.w * 0.75, y));
    let seeks = calls(&f, "media_seek");
    assert_eq!(seeks, vec![json!({ "ms": 150_000 })], "once, where it was released: 75 % of 200 s");
    // dragging past the ends holds the position inside the track
    s.send(&down(bar.x + 10.0, y));
    s.send(&mv(bar.x + bar.w + 500.0, y));
    s.send(&up(bar.x + bar.w + 500.0, y));
    assert_eq!(calls(&f, "media_seek").last().unwrap(), &json!({ "ms": 200_000 }));
    // while it was held the card stayed open even though the cursor left
    s.send_proximity(&ProximityEvent::Leave);
    s.run(4000);
    assert!((hit_rect(&s.take_fx()).h - lg::CAPSULE_H).abs() < 0.3, "released and left: closed");
}

#[test]
fn a_new_track_cross_fades_and_then_everything_idles() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    assert!(!s.frame.wants_frame());
    media(&mut s, &f, state(true, "Another song", 0));
    assert!(s.run(4000) > 10, "the old text goes while the new one comes");
    assert!(!s.frame.wants_frame(), "and it settles");
    // a seek or a pause is not a new track: the text does not move (the progress glides and the icon turns, that is all)
    let n = {
        media(&mut s, &f, state(true, "Another song", 120_000));
        s.run(4000)
    };
    assert!(n < 200, "the progress bar glides, nothing else: {n} frames");
    assert!(!s.frame.wants_frame());
}

#[test]
fn the_one_second_tick_moves_the_clock_only_while_it_plays() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    s.now_ms += 1_000.0;
    s.cx.now_ms = s.now_ms;
    assert!(s.deliver(Msg::Custom(Box::new(crate::ui::tabs::a::core::Tick))).redraw, "a playing track redraws its progress");
    s.run(4000);
    media(&mut s, &f, state(false, "Song", 62_000));
    s.run(4000);
    s.now_ms += 1_000.0;
    s.cx.now_ms = s.now_ms;
    assert!(!s.deliver(Msg::Custom(Box::new(crate::ui::tabs::a::core::Tick))).redraw, "a paused one does not");
    // a foreign message is not ours
    assert!(!s.deliver(Msg::Custom(Box::new(42u8))).redraw);
}

#[test]
fn reduce_motion_lands_everything_at_once() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(true));
    let (mut s, f) = sim(Dock::Bottom, cfg(json!({})));
    media(&mut s, &f, state(true, "Song", 0));
    s.show();
    assert!(s.run(4) <= 2, "no enter animation");
    assert_eq!(hit_rect(&s.take_fx()).h, lg::CAPSULE_H);
    s.send_proximity(&ProximityEvent::Hover);
    assert!(s.run(4) <= 2);
    assert_eq!(hit_rect(&s.take_fx()).h, lg::CARD_H, "the card opens in one frame");
    media(&mut s, &f, json!({ "has_session": false }));
    s.run(4);
    let hid = s.take_fx().hide;
    glasskit::ease::force_reduced(None);
    assert!(hid, "and leaves in one frame");
}

#[test]
fn the_window_asks_for_exactly_its_frame_and_names_itself_for_a_screen_reader() {
    let (mut s, f) = on_screen(Dock::Bottom);
    assert_eq!(s.view.size_hint(), Some(lg::WIN_REAL));
    assert_eq!(s.view.accessible().as_deref(), Some("Music: Song - Band"));
    media(&mut s, &f, json!({ "has_session": true, "playing": true, "title": "", "app": "Spotify.exe", "thumb_id": 3 }));
    assert_eq!(s.view.accessible().as_deref(), Some("Music: Spotify"));
}

#[test]
fn hiding_stops_the_tick_and_nothing_is_left_running() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    s.hide();
    s.run(10);
    assert!(!s.frame.wants_frame());
    f.bus.emit("media:changed", state(true, "Song", 0));
    assert!(s.queue.len() >= 1, "events still arrive while it is away: they are what calls it up");
}

// ------------------------------------------------------------------------------------------------------------------------------ one render

/// Runs `f` on one long-lived thread of its own (the covers and text layouts keep Direct2D objects; see the preview tests for the story).
fn gfx(f: impl FnOnce() + Send + 'static) {
    use std::sync::{mpsc, Mutex, OnceLock};
    type Job = Box<dyn FnOnce() + Send>;
    static TX: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let tx = TX.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new().name("music-gfx".into()).spawn(move || rx.into_iter().for_each(|job| job())).expect("a thread");
        Mutex::new(tx)
    });
    let (done_tx, done_rx) = mpsc::channel();
    let job: Job = Box::new(move || {
        let _ = done_tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
    });
    tx.lock().unwrap_or_else(|e| e.into_inner()).send(job).expect("the gfx thread is alive");
    if let Err(p) = done_rx.recv().expect("an answer") {
        std::panic::resume_unwind(p);
    }
}

#[test]
fn the_capsule_and_the_card_draw_inside_their_body_and_the_rest_of_the_window_stays_clear() {
    gfx(|| {
        let _m = motion_on();
        let th = Theme::pill_dark();
        let (mut s, f) = on_screen(Dock::Bottom);
        let lit = |img: &glasskit::testkit::Image, r: Rect| img.count(Some(r), |p| p[3] > 8);
        let st0 = base(Dock::Bottom);
        let capsule = s.render(lg::WIN_REAL, &th);
        let body = st0.rect_to_real(lg::body(Dock::Bottom, 0.0, 0.0));
        assert!(lit(&capsule, body) as f32 > body.w * body.h * 0.9, "the capsule fills its body");
        let above = Rect::new(0.0, 0.0, lg::WIN_REAL.0, lg::WIN_REAL.1 - lg::PAD - lg::CAPSULE_H - 14.0);
        assert_eq!(lit(&capsule, above), 0, "nothing is drawn above the folded capsule: that is click-through glass");
        // text and the cover placeholder make the capsule more than one flat colour
        let colours = |img: &glasskit::testkit::Image| {
            let mut set = std::collections::HashSet::new();
            let l = lg::capsule_layout(body);
            for r in [l.cover, l.text] {
                for y in (r.y as u32..r.bottom() as u32).step_by(2) {
                    for x in (r.x as u32..r.right() as u32).step_by(2) {
                        set.insert(img.at(x as f32, y as f32));
                    }
                }
            }
            set.len()
        };
        assert!(colours(&capsule) > 6, "cover and text are drawn");
        s.send_proximity(&ProximityEvent::Hover);
        s.run(4000);
        let card = s.render(lg::WIN_REAL, &th);
        let open = st0.rect_to_real(lg::body(Dock::Bottom, 1.0, 0.0));
        assert!(lit(&card, Rect::new(open.x, open.y, open.w, open.h - lg::CAPSULE_H)) > 20_000, "the card covers the part above the capsule");
        assert_eq!(lit(&card, Rect::new(0.0, 0.0, lg::WIN_REAL.0, open.y - 8.0)), 0, "and nothing beyond it");
        // the same widget at the top edge is the mirror image in the window, not a copy of the bottom one
        let (mut t, f2) = sim(Dock::Top, cfg(json!({})));
        media(&mut t, &f2, state(true, "Song", 61_000));
        show(&mut t);
        let top = t.render(lg::WIN_REAL, &th);
        assert!(lit(&top, lg::body(Dock::Top, 0.0, 0.0)) > 15_000 && lit(&top, Rect::new(0.0, lg::CAPSULE_H + lg::PAD + 14.0, lg::WIN_REAL.0, 100.0)) == 0);
        // in the taskbar: the compact capsule fills its strip and nothing is drawn above the bar; opened, the card floats above the bar
        let (mut b, _f3, _) = bar_on_screen(Align::Left, 48.0);
        let st = bar_st(Align::Left, 48.0, 1.0);
        let in_bar = b.render(lg::WIN_BAR_REAL, &th);
        let cap = st.rect_to_real(lg::body(st, 0.0, 0.0));
        assert!(lit(&in_bar, cap) as f32 > cap.w * cap.h * 0.9, "the capsule fills its body in the bar");
        assert_eq!(lit(&in_bar, Rect::new(0.0, 0.0, lg::WIN_BAR_REAL.0, lg::WIN_BAR_REAL.1 - 48.0 - 14.0)), 0, "nothing above the bar: that is click-through glass");
        assert!(cap.bottom() <= lg::WIN_BAR_REAL.1 && cap.y >= lg::WIN_BAR_REAL.1 - 48.0, "inside the strip");
        open_by_click(&mut b, st);
        let floating = b.render(lg::WIN_BAR_REAL, &th);
        let open = st.rect_to_real(lg::body(st, 1.0, 0.0));
        assert!(lit(&floating, Rect::new(open.x, open.y, open.w, open.h)) > 20_000, "the open card is drawn");
        assert_eq!(lit(&floating, Rect::new(0.0, 0.0, lg::WIN_BAR_REAL.0, open.y - 8.0)), 0, "and nothing beyond it");
        assert!(open.bottom() <= lg::WIN_BAR_REAL.1 - 48.0 - geom::BAR_FLOAT_GAP + 0.5, "above the bar, not over it");
        if let Ok(dir) = std::env::var("PILL_SNAP_DIR") {
            let _ = std::fs::create_dir_all(&dir);
            crate::ui::host::testing::save_png(&card, std::path::Path::new(&dir).join("music-widget-card.png"));
            crate::ui::host::testing::save_png(&floating, std::path::Path::new(&dir).join("music-widget-taskbar.png"));
        }
    });
}

// ------------------------------------------------------------------------------------------------------------------------------ size

#[test]
fn the_widget_scales_with_the_island_size_and_a_change_glides_from_its_corner_and_then_idles() {
    let _m = motion_on();
    // starts at 150 %: the capsule is the capsule x 1.5, standing 1.5 x PAD from the corner, in the same fixed window
    let (mut s, f) = sim(Dock::Bottom, cfg(json!({ "size": 150 })));
    media(&mut s, &f, state(true, "Song", 61_000));
    let r = hit_rect(&show(&mut s));
    assert_eq!(s.view.size_hint(), Some(lg::WIN_REAL), "the window never changes");
    assert!((r.w - lg::BODY_W * 1.5).abs() < 0.6 && (r.h - lg::CAPSULE_H * 1.5).abs() < 0.6, "{r:?}");
    assert!((r.x - lg::PAD * 1.5).abs() < 0.6 && (lg::WIN_REAL.1 - r.bottom() - lg::PAD * 1.5).abs() < 0.6, "it stands off its corner by the scaled margin: {r:?}");
    assert!(!s.frame.wants_frame(), "settled: no frames");
    // the island's slider goes to 100 %: a glide (frames), the corner holds, nothing moves afterwards
    fakes_settings(&mut s, &f, cfg(json!({ "size": 100 })));
    assert!(s.run(4000) > 8, "the size glides");
    let fx = s.take_fx();
    let r2 = hit_rect(&fx);
    assert!((r2.w - lg::BODY_W).abs() < 0.6 && (r2.h - lg::CAPSULE_H).abs() < 0.6 && (r2.x - lg::PAD).abs() < 0.6, "{r2:?}");
    assert!(!fx.relayout && !fx.hide, "the window is neither moved nor re-created");
    assert!(!s.frame.wants_frame(), "and it idles");
    assert_eq!(s.run(100), 0);
    // the same change with no change of size costs nothing
    fakes_settings(&mut s, &f, cfg(json!({ "size": 100 })));
    assert!(s.run(100) <= 1, "at most the one frame every settings event draws");
    assert!(!s.frame.wants_frame());
    // a click lands where the scaled body is: the play button of the 200 % capsule
    fakes_settings(&mut s, &f, cfg(json!({ "size": 200 })));
    s.run(4000);
    let mut st = base(Dock::Bottom);
    st.k = 2.0;
    let (cap, _) = lg::frames(st, 0.0, 0.0);
    let play = st.to_real(lg::capsule_layout(cap).btn[1].center());
    click(&mut s, play);
    assert_eq!(calls(&f, "media_control").last().map(|c| c["action"].clone()), Some(json!("play_pause")));
    // the card at 200 % is twice the card, hovering opens it, and the hit region follows
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    let card = hit_rect(&s.take_fx());
    assert!((card.w - 2.0 * lg::BODY_W).abs() < 0.6 && (card.h - 2.0 * lg::CARD_H).abs() < 0.6, "{card:?}");
    assert!(card.right() <= lg::WIN_REAL.0 && card.y >= 0.0, "inside the window");
}

#[test]
fn reduce_motion_lands_a_size_change_at_once_and_a_hidden_widget_takes_it_without_frames() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(true));
    let (mut s, f) = sim(Dock::Top, cfg(json!({})));
    media(&mut s, &f, state(true, "Song", 0));
    s.show();
    s.run(4);
    s.take_fx();
    fakes_settings(&mut s, &f, cfg(json!({ "size": 200 })));
    let n = s.run(4);
    let r = hit_rect(&s.take_fx());
    glasskit::ease::force_reduced(None);
    assert!(n <= 2, "no glide: {n} frames");
    assert!((r.h - lg::CAPSULE_H * 2.0).abs() < 0.6 && (r.y - lg::PAD * 2.0).abs() < 0.6, "{r:?}");
    // hidden: nothing to animate, nothing to draw
    let (mut h, f2) = sim(Dock::Bottom, cfg(json!({})));
    h.run(10); // the first frame of a fresh window
    fakes_settings(&mut h, &f2, cfg(json!({ "size": 150 })));
    h.run(10);
    assert_eq!(h.run(50), 0);
    assert!(!h.frame.wants_frame());
}

#[test]
fn the_slot_asked_of_the_bar_is_the_capsule_at_100_percent_and_the_window_side_gap_is_the_same_at_every_size() {
    let s = sig(json!({ "media.widget": true, "media.widget.edge": "taskbar", "media.widget.align": "left" })).unwrap();
    match placement(&s) {
        Placement::InBar { ask, .. } => assert_eq!((ask.body_w, ask.pad), (lg::BAR_FOLD_W, lg::BAR_PAD), "taskbar::widget_place scales the width by the size; the pad never changes"),
        p => panic!("{p:?}"),
    }
    assert_eq!(size_k(), 1.0, "the default");
    // the size is not part of the window's identity: a change of it never re-creates the window
    assert!(!KEYS.contains(&"size") && ISLAND_KEYS.contains(&"size"));
}

// ------------------------------------------------------------------------------------------------------------------------------ in the taskbar

/// A widget of the taskbar dock that was told it stands in a `bar` px tall bar anchored to `align`, playing and settled; the hit region it
/// settled on comes with it.
fn bar_on_screen(align: Align, bar: f32) -> (Sim, Fakes, Rect) {
    let (svc, fakes) = Services::fake();
    fakes.api.reply("settings_get", Ok(cfg(json!({}))));
    let mut s = Sim::with_services(Box::new(Blank), svc);
    let v = MusicView::new(&mut s.cx, Dock::Taskbar).with_slot(bar, align);
    s.view = Box::new(v);
    s.pump();
    media(&mut s, &fakes, state(true, "Song", 61_000));
    let r = hit_rect(&show(&mut s));
    (s, fakes, r)
}

/// A click at `at` (move, press, release).
fn click(s: &mut Sim, at: (f32, f32)) {
    s.send(&mv(at.0, at.1));
    s.send(&down(at.0, at.1));
    s.send(&up(at.0, at.1));
}

/// What the cursor does to open the card of the bar widget: it arrives (the host's `Hover`) and clicks the cover.
fn open_by_click(s: &mut Sim, st: lg::Stand) {
    s.send_proximity(&ProximityEvent::Hover);
    let (cap, _) = lg::frames(st, 0.0, 0.0);
    click(s, st.to_real(lg::capsule_layout_in(st, cap).cover.center()));
    s.run(4000);
}

fn slot_event(s: &mut Sim, f: &Fakes, bar: f64, align: &str) {
    f.bus.emit(SLOT_EVENT, json!({ "bar": bar, "align": align }));
    s.pump();
}

#[test]
fn the_taskbar_edge_is_a_dock_of_its_own_with_a_taller_window_and_its_own_placement() {
    let s = sig(json!({ "media.widget": true, "media.widget.edge": "taskbar", "media.widget.align": "right", "media.widget.offset": 12, "monitor": "2" })).unwrap();
    assert_eq!((s.dock, s.align), (Dock::Taskbar, Align::Right));
    let sp = spec(&s).into_spec();
    assert_eq!((sp.kind, sp.size, sp.visible, sp.topmost, sp.activate_on_show), (host::WindowKind::Overlay, lg::WIN_BAR_REAL, false, true, false), "the card floats above the bar: a taller window");
    assert!(lg::WIN_BAR_REAL.1 > lg::WIN_REAL.1 && lg::WIN_BAR_REAL.0 == lg::WIN_REAL.0);
    match placement(&s) {
        Placement::InBar { monitor, ask } => {
            assert_eq!(monitor, MonitorSel::Index(1));
            assert_eq!((ask.align, ask.offset_pct, ask.body_w, ask.pad, ask.inside_work), (Align::Right, 12.0, lg::BAR_FOLD_W, lg::BAR_PAD, true));
        }
        p => panic!("{p:?}"),
    }
    // the other edges keep their placement and window
    for (edge, dock) in [("top", Dock::Top), ("bottom", Dock::Bottom)] {
        let s = sig(json!({ "media.widget": true, "media.widget.edge": edge })).unwrap();
        assert_eq!(s.dock, dock);
        assert!(matches!(placement(&s), Placement::Docked { .. }));
        assert_eq!(spec(&s).into_spec().size, lg::WIN_REAL);
    }
    // a changed edge re-creates the window; a change of the island's place alone does not (it only re-checks the place)
    assert!(KEYS.contains(&"media.widget.edge") && !KEYS.contains(&"edge.position"));
    assert!(ISLAND_KEYS.contains(&"edge.position") && ISLAND_KEYS.contains(&"edge.align") && ISLAND_KEYS.contains(&"offsetX") && ISLAND_KEYS.contains(&"size"));
    assert_eq!((align_name(Align::Left), align_name(Align::Center), align_name(Align::Right)), ("left", "center", "right"));
}

#[test]
fn in_the_bar_the_capsule_settles_inside_the_strip_and_costs_nothing() {
    let _m = motion_on();
    for bar in [40.0f32, 48.0, 52.0] {
        let (mut s, _f, r) = bar_on_screen(Align::Left, bar);
        assert_eq!(s.view.size_hint(), Some(lg::WIN_BAR_REAL));
        let top = lg::WIN_BAR_REAL.1 - bar;
        assert!(r.y >= top - 0.2 && r.bottom() <= lg::WIN_BAR_REAL.1 + 0.2, "{bar}: {r:?} inside the strip");
        assert!(((r.y - top) - (lg::WIN_BAR_REAL.1 - r.bottom())).abs() < 0.3, "{bar}: centred in the bar");
        assert_eq!((r.x, r.w), (lg::BAR_PAD, lg::BAR_FOLD_W));
        assert!(!s.frame.wants_frame(), "settled: no frames");
        assert_eq!(s.run(200), 0, "and nothing moves by itself");
    }
}

#[test]
fn hovering_the_bar_capsule_opens_nothing_a_click_on_the_cover_opens_the_card_upward_and_leaving_folds_it() {
    let _m = motion_on();
    let (mut s, _f, folded) = bar_on_screen(Align::Left, 48.0);
    let st = bar_st(Align::Left, 48.0, 1.0);
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    assert_eq!(s.run(50), 0, "the capsule has its own buttons: hovering moves nothing");
    let still = s.take_fx().hit_region.map_or(folded, |r| r[0]);
    assert!((still.y - folded.y).abs() < 0.01 && (still.h - folded.h).abs() < 0.01, "and the card stays folded: {still:?}");
    // a click on the cover opens it with a spring
    let (cap, _) = lg::frames(st, 0.0, 0.0);
    let l = lg::capsule_layout_in(st, cap);
    click(&mut s, st.to_real(l.cover.center()));
    assert!(s.run(4000) > 10, "it opens with a spring");
    let open = hit_rect(&s.take_fx());
    assert!((open.y - (lg::WIN_BAR_REAL.1 - 48.0 - geom::BAR_FLOAT_GAP - lg::CARD_H)).abs() < 0.5, "the card floats 8 px above the bar: {open:?}");
    assert!(open.y < folded.y && open.bottom() >= folded.bottom() - 0.5, "it grew upward and still covers where the capsule stood: {open:?} vs {folded:?}");
    assert!(!s.frame.wants_frame());
    // the cursor resting on the capsule's place keeps the card open (no flicker)
    s.send_proximity(&ProximityEvent::Hover);
    s.run(50);
    let fx = s.take_fx();
    assert!(!s.frame.wants_frame() && fx.hit_region.as_ref().map_or(true, |r| (r[0].y - open.y).abs() < 0.01), "and it stays where it is");
    // another click on the card's cover folds it, the cursor leaving folds it too
    let (_, card) = lg::frames(st, 1.0, 0.0);
    click(&mut s, st.to_real(lg::card_layout(card).title.center()));
    assert!(s.run(4000) > 10);
    let back = hit_rect(&s.take_fx());
    assert!((back.y - folded.y).abs() < 0.5 && (back.h - folded.h).abs() < 0.5, "back in the bar: {back:?}");
    assert!(!s.frame.wants_frame(), "settled: no frames");
    click(&mut s, st.to_real(l.text.center()));
    s.run(4000);
    assert!(hit_rect(&s.take_fx()).y < folded.y - 20.0, "the title opens it as well");
    s.send_proximity(&ProximityEvent::Leave);
    assert!(s.run(4000) > 10);
    let gone = hit_rect(&s.take_fx());
    assert!((gone.y - folded.y).abs() < 0.5, "the cursor went away: folded again, nothing pinned: {gone:?}");
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    assert!(!s.frame.wants_frame() && s.take_fx().hit_region.map_or(true, |r| (r[0].y - folded.y).abs() < 0.5), "and coming back does not reopen it");
}

#[test]
fn the_bar_buttons_work_folded_without_opening_the_card_and_the_screen_edge_widget_still_opens_on_hover() {
    let _m = motion_on();
    let (mut s, f, folded) = bar_on_screen(Align::Left, 48.0);
    let st = bar_st(Align::Left, 48.0, 1.0);
    s.send_proximity(&ProximityEvent::Hover);
    let (cap, _) = lg::frames(st, 0.0, 0.0);
    let l = lg::capsule_layout_in(st, cap);
    for (i, act) in ["prev", "play_pause", "next"].into_iter().enumerate() {
        click(&mut s, st.to_real(l.btn[i].center()));
        assert_eq!(calls(&f, "media_control").last().map(|c| c["action"].clone()), Some(json!(act)));
    }
    s.run(4000);
    let r = s.take_fx().hit_region.map_or(folded, |r| r[0]);
    assert!((r.y - folded.y).abs() < 0.01, "three button clicks and the card never opened: {r:?}");
    // the same hover at the bottom edge opens the card as before
    let (mut b, _f2) = on_screen(Dock::Bottom);
    b.send_proximity(&ProximityEvent::Hover);
    assert!(b.run(4000) > 10);
    assert!(hit_rect(&b.take_fx()).h > lg::CARD_H - 1.0);
}

#[test]
fn the_open_card_in_the_bar_still_answers_its_buttons_and_the_folded_capsule_its_own() {
    let _m = motion_on();
    let (mut s, f, _) = bar_on_screen(Align::Right, 48.0);
    let st = bar_st(Align::Right, 48.0, 1.0);
    let (cap, _) = lg::frames(st, 0.0, 0.0);
    let play = st.to_real(lg::capsule_layout_in(st, cap).btn[1].center());
    s.send(&mv(play.0, play.1));
    s.send(&down(play.0, play.1));
    s.send(&up(play.0, play.1));
    assert_eq!(calls(&f, "media_control").last().map(|c| c["action"].clone()), Some(json!("play_pause")), "the compact capsule's play button works");
    open_by_click(&mut s, st);
    let (_, card) = lg::frames(st, 1.0, 0.0);
    let next = st.to_real(lg::card_layout(card).btn[2].center());
    s.send(&down(next.0, next.1));
    s.send(&up(next.0, next.1));
    assert_eq!(calls(&f, "media_control").last().map(|c| c["action"].clone()), Some(json!("next")), "and so does the card that floats above the bar");
}

#[test]
fn reduce_motion_lands_the_bar_capsule_and_its_card_at_once() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(true));
    let (svc, f) = Services::fake();
    f.api.reply("settings_get", Ok(cfg(json!({}))));
    let mut s = Sim::with_services(Box::new(Blank), svc);
    let v = MusicView::new(&mut s.cx, Dock::Taskbar).with_slot(48.0, Align::Center);
    s.view = Box::new(v);
    s.pump();
    media(&mut s, &f, state(true, "Song", 0));
    s.show();
    let frames_in = s.run(4);
    let folded = hit_rect(&s.take_fx());
    s.send_proximity(&ProximityEvent::Hover);
    let st = bar_st(Align::Center, 48.0, 1.0);
    let (cap, _) = lg::frames(st, 0.0, 0.0);
    click(&mut s, st.to_real(lg::capsule_layout_in(st, cap).cover.center()));
    let frames_open = s.run(4);
    let open = hit_rect(&s.take_fx());
    glasskit::ease::force_reduced(None);
    assert!(frames_in <= 2 && frames_open <= 2, "no animation: {frames_in} / {frames_open} frames");
    assert_eq!(folded.h, bar_st(Align::Center, 48.0, 1.0).fold_h());
    assert_eq!(open.w, lg::BODY_W);
    assert!(open.y < folded.y, "the card is up in one frame");
}

#[test]
fn a_new_place_in_the_bar_slides_it_out_re_places_the_window_and_slides_it_back_in() {
    let _m = motion_on();
    let (mut s, f, before) = bar_on_screen(Align::Left, 48.0);
    slot_event(&mut s, &f, 40.0, "right"); // the island took the left side, or the bar got smaller
    assert!(s.run(4000) > 5, "the body moves: out, then in");
    let fx = s.take_fx();
    assert!(fx.relayout, "the host is asked to put the window in its new place while the body is away");
    assert!(!fx.hide, "the window itself stays");
    let after = hit_rect(&fx);
    assert_eq!(after.right(), lg::WIN_BAR_REAL.0 - lg::BAR_PAD, "now anchored to the right side");
    assert!((after.h - 32.0).abs() < 0.3 && after.h < before.h, "and fitted to the smaller bar: {after:?}");
    assert!(!s.frame.wants_frame(), "settled again");
    // the bar goes away: the plain bottom widget, in the same window
    slot_event(&mut s, &f, 0.0, "right");
    s.run(4000);
    let plain = hit_rect(&s.take_fx());
    assert_eq!((plain.h, lg::WIN_BAR_REAL.1 - plain.bottom()), (lg::CAPSULE_H, lg::PAD));
}

#[test]
fn a_new_place_while_it_is_away_is_taken_without_ceremony() {
    let _m = motion_on();
    let (mut s, f) = sim(Dock::Taskbar, cfg(json!({})));
    slot_event(&mut s, &f, 48.0, "center");
    assert!(!s.take_fx().relayout, "nothing is on screen: nothing to re-place (a show does it)");
    media(&mut s, &f, state(true, "Song", 0));
    let r = hit_rect(&show(&mut s));
    assert!((r.cx() - lg::WIN_BAR_REAL.0 / 2.0).abs() < 0.3 && r.w == lg::BAR_FOLD_W, "it came in at the place it was told: {r:?}");
    // the other docks do the same for the new place their window is given (another monitor, DPI): out, re-placed, in
    let (mut s, f) = on_screen(Dock::Bottom);
    slot_event(&mut s, &f, 0.0, "right");
    assert!(s.run(4000) > 5, "the body slides out and in");
    let fx = s.take_fx();
    assert!(fx.relayout && !fx.hide, "the window is re-placed while the body is away and stays");
    assert!(!s.frame.wants_frame(), "settled again");
}

#[test]
fn the_bar_widget_leaves_with_the_guard_and_its_pending_move_does_not_bring_it_back() {
    let _m = motion_on();
    let (mut s, f, _) = bar_on_screen(Align::Left, 48.0);
    slot_event(&mut s, &f, 40.0, "right");
    f.bus.emit(GUARD_EVENT, json!({ "hidden": true }));
    s.pump();
    s.run(4000);
    let fx = s.take_fx();
    assert!(fx.hide, "a fullscreen app in front: it goes away and stays away");
    s.hide();
    f.bus.emit(GUARD_EVENT, json!({ "hidden": false }));
    s.pump();
    assert!(s.take_fx().show, "and comes back when the guard lets go");
}

// ------------------------------------------------------------------------------------------------------------------------------ the bar capsule, looked at

/// One look at the bar capsule: the window rendered at `scale`, its bottom `bar` rows composited over a fake dark taskbar strip, scaled up 2x
/// (nearest) so a person can judge pixels. Invented data only.
fn bar_look(scale: f32, bar: f32, title: &str, artist: &str, playing: bool, cover: bool, open: bool) -> glasskit::testkit::Image {
    let (svc, f) = Services::fake();
    f.api.reply("settings_get", Ok(cfg(json!({}))));
    let mut s = Sim::with_services(Box::new(Blank), svc);
    s.cx.scale = scale;
    let v = MusicView::new(&mut s.cx, Dock::Taskbar).with_slot(bar, Align::Left);
    s.view = Box::new(v);
    s.pump();
    let mut st = state(true, title, 61_000);
    st["artist"] = json!(artist);
    if !cover {
        st["thumb_id"] = json!(0);
        st["thumbnail"] = json!("");
    }
    media(&mut s, &f, st.clone());
    let _ = show(&mut s);
    if !playing {
        st["playing"] = json!(false);
        media(&mut s, &f, st);
        s.run(4000);
    }
    if open {
        open_by_click(&mut s, bar_st(Align::Left, bar, scale));
    }
    let th = Theme::pill_dark();
    let img = s.render(lg::WIN_BAR_REAL, &th);
    // the strip: from the window's floor up `bar` logical px (rounded to device px like the OS does); the open card shows above it
    let bar_p = geom::phys(bar, scale) as u32;
    let top = if open { 0 } else { img.h.saturating_sub(bar_p + 4) };
    let (w, h) = (((lg::BAR_PAD + lg::PAD + lg::BAR_FOLD_W) * scale).ceil() as u32 + 8, img.h - top);
    let k = 2u32;
    let mut out = vec![0u8; (w * k * h * k * 4) as usize];
    for y in 0..h * k {
        for x in 0..w * k {
            let (sx, sy) = ((x / k).min(img.w - 1), top + y / k);
            let strip = sy + bar_p >= img.h;
            let bg: [u32; 3] = if strip { [34, 34, 38] } else { [70, 80, 100] };
            let p = img.px(sx, sy);
            let a = p[3] as u32;
            let i = ((y * w * k + x) * 4) as usize;
            // premultiplied RGBA over an opaque background
            out[i] = ((p[2] as u32 + bg[2] * (255 - a) / 255).min(255)) as u8;
            out[i + 1] = ((p[1] as u32 + bg[1] * (255 - a) / 255).min(255)) as u8;
            out[i + 2] = ((p[0] as u32 + bg[0] * (255 - a) / 255).min(255)) as u8;
            out[i + 3] = 255;
        }
    }
    glasskit::testkit::Image { w: w * k, h: h * k, scale: 1.0, bgra: out }
}

/// Stacks images vertically with a gap.
fn stack(parts: &[glasskit::testkit::Image]) -> glasskit::testkit::Image {
    let w = parts.iter().map(|p| p.w).max().unwrap_or(1);
    let h: u32 = parts.iter().map(|p| p.h + 6).sum();
    let mut bgra = vec![255u8; (w * h * 4) as usize];
    for px in bgra.chunks_exact_mut(4) {
        px.copy_from_slice(&[200, 200, 200, 255]);
    }
    let mut y0 = 0;
    for p in parts {
        for y in 0..p.h {
            let (d, s) = (((y0 + y) * w * 4) as usize, (y * p.w * 4) as usize);
            bgra[d..d + (p.w * 4) as usize].copy_from_slice(&p.bgra[s..s + (p.w * 4) as usize]);
        }
        y0 += p.h + 6;
    }
    glasskit::testkit::Image { w, h, scale: 1.0, bgra }
}

/// The bar capsule at every DPI, bar height, length of text, state and with / without a cover: it must draw (and the checks below hold);
/// `PILL_SNAP_DIR=<dir>` writes one PNG per DPI for a look with your own eyes.
#[test]
fn the_bar_capsule_draws_at_every_dpi_with_short_and_long_text_and_stays_inside_its_body() {
    gfx(|| {
        let _m = motion_on();
        let long_t = "A very long title that goes on and on and does not fit the capsule at all";
        let long_a = "Some Artist With A Long Name Feat. Another One";
        let cases: [(&str, &str, bool, bool); 8] = [
            ("Life Is Narcosis", "IVOXYGEN", true, true),
            ("No Surrender", "SOTA, Bad Omens", true, true),
            (long_t, "Short", true, true),
            ("Short", long_a, true, false),
            ("Очень длинное название трека на русском языке", "Исполнитель", true, true),
            ("Тишина", "МУККА", false, true),
            ("Paused one", "Band", false, false),
            (long_t, long_a, true, false),
        ];
        let dir = std::env::var("PILL_SNAP_DIR").ok();
        for (dpi, scale) in [(100, 1.0f32), (125, 1.25), (150, 1.5)] {
            let mut rows = Vec::new();
            for (bar, rest) in [(48.0f32, true), (40.0, false), (52.0, false)] {
                for (i, (t, a, playing, cover)) in cases.iter().enumerate() {
                    if !rest && i > 1 {
                        break;
                    }
                    let img = bar_look(scale, bar, t, a, *playing, *cover, false);
                    assert!(img.w > 100 && img.h > 20);
                    rows.push(img);
                }
            }
            rows.push(bar_look(scale, 48.0, "Life Is Narcosis", "IVOXYGEN", true, true, true));
            if let Some(d) = &dir {
                let _ = std::fs::create_dir_all(d);
                crate::ui::host::testing::save_png(&stack(&rows), std::path::Path::new(d).join(format!("bar-capsule-{dpi}.png")));
            }
        }
    });
}

// ------------------------------------------------------------------------------------------------------------------------------ the open card, looked at

/// The base window of `st` inside the rendered real window, composited over a flat backdrop and scaled 2x (nearest) for a person to judge pixels.
fn card_look(img: &glasskit::testkit::Image, st: lg::Stand, scale: f32) -> glasskit::testkit::Image {
    let o = st.origin();
    let (x0, y0) = ((o.0 * scale).round() as u32, (o.1 * scale).round() as u32);
    let (w, h) = (((lg::WIN.0 * st.k) * scale).round() as u32, ((lg::WIN.1 * st.k) * scale).round() as u32);
    let k = 2u32;
    let mut out = vec![0u8; (w * k * h * k * 4) as usize];
    for y in 0..h * k {
        for x in 0..w * k {
            let p = img.px(x0 + x / k, y0 + y / k);
            let a = p[3] as u32;
            let bg = [96u32, 104, 122];
            let i = ((y * w * k + x) * 4) as usize;
            out[i] = ((p[2] as u32 + bg[2] * (255 - a) / 255).min(255)) as u8;
            out[i + 1] = ((p[1] as u32 + bg[1] * (255 - a) / 255).min(255)) as u8;
            out[i + 2] = ((p[0] as u32 + bg[0] * (255 - a) / 255).min(255)) as u8;
            out[i + 3] = 255;
        }
    }
    glasskit::testkit::Image { w: w * k, h: h * k, scale: 1.0, bgra: out }
}

/// The open card at the bottom and the top edge, at two DPIs and two sizes (invented track): the outermost ring of the base window stays clear
/// (the shadow is not cut off by the window), and `PILL_SNAP_DIR=<dir>` writes `card-<n>.png` for a look with your own eyes.
#[test]
fn the_open_card_is_even_on_every_side_and_its_shadow_is_not_cut_off() {
    gfx(|| {
        let _m = motion_on();
        let th = Theme::pill_dark();
        let dir = std::env::var("PILL_SNAP_DIR").ok();
        let mut sheet = Vec::new();
        for (name, dock, scale, size) in [("bottom-100", Dock::Bottom, 1.0f32, 100), ("top-100", Dock::Top, 1.0, 100), ("bottom-150dpi", Dock::Bottom, 1.5, 100), ("top-size150", Dock::Top, 1.0, 150), ("bottom-size200-125dpi", Dock::Bottom, 1.25, 200)] {
            let (svc, f) = Services::fake();
            f.api.reply("settings_get", Ok(cfg(json!({ "size": size }))));
            let mut s = Sim::with_services(Box::new(Blank), svc);
            s.cx.scale = scale;
            let v = MusicView::new(&mut s.cx, dock);
            s.view = Box::new(v);
            s.pump();
            let mut st_json = state(true, "Midnight Echo", 3_000);
            st_json["artist"] = json!("The Lanterns");
            st_json["album"] = json!("Paper Moons");
            st_json["duration_ms"] = json!(240_000);
            media(&mut s, &f, st_json);
            let _ = show(&mut s);
            s.send_proximity(&ProximityEvent::Hover);
            s.run(4000);
            let img = s.render(lg::WIN_REAL, &th);
            let mut st = lg::Stand::from(dock);
            st.scale = scale;
            st.k = lg::size_k(size as f64);
            // the ring: the outermost device px of the base window (the window's own edge on the dock side) carries no ink at all
            let o = st.origin();
            let (x0, y0) = ((o.0 * scale).round() as u32, (o.1 * scale).round() as u32);
            let (w, h) = (((lg::WIN.0 * st.k) * scale).round() as u32, ((lg::WIN.1 * st.k) * scale).round() as u32);
            let mut worst = 0u8;
            for x in 0..w {
                worst = worst.max(img.px(x0 + x, y0)[3]).max(img.px(x0 + x, y0 + h - 1)[3]);
            }
            for y in 0..h {
                worst = worst.max(img.px(x0, y0 + y)[3]).max(img.px(x0 + w - 1, y0 + y)[3]);
            }
            assert!(worst <= 2, "{name}: the shadow reaches the window's edge (alpha {worst}): it would be cut off");
            // the card is drawn: the padded body is lit, the corners of the window are clear
            let card = st.rect_to_real(lg::body(st, 1.0, 0.0));
            assert!(img.count(Some(card), |p| p[3] > 8) as f32 > card.w * card.h * 0.9, "{name}: the card fills its body");
            sheet.push(card_look(&img, st, scale));
        }
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
            for (i, img) in sheet.iter().enumerate() {
                crate::ui::host::testing::save_png(img, std::path::Path::new(d).join(format!("card-{i}.png")));
            }
            crate::ui::host::testing::save_png(&stack(&sheet), std::path::Path::new(d).join("card-sheet.png"));
        }
    });
}

#[test]
fn the_open_card_has_one_and_the_same_corner_at_all_four_corners() {
    gfx(|| {
        let _m = motion_on();
        let th = Theme::pill_dark();
        for dock in [Dock::Bottom, Dock::Top] {
            let (mut s, _f) = on_screen(dock);
            s.send_proximity(&ProximityEvent::Hover);
            s.run(4000);
            let img = s.render(lg::WIN_REAL, &th);
            let open = base(dock).rect_to_real(lg::body(dock, 1.0, 0.0));
            // solid pixels in the square of one corner radius at each corner: a round corner leaves the same number of them at all four
            let r = lg::CARD_R;
            let corner = |x: f32, y: f32| img.count(Some(Rect::new(x, y, r, r)), |p| p[3] > 200);
            let c = [corner(open.x, open.y), corner(open.right() - r, open.y), corner(open.x, open.bottom() - r), corner(open.right() - r, open.bottom() - r)];
            let (lo, hi) = (*c.iter().min().unwrap(), *c.iter().max().unwrap());
            assert!(hi - lo <= (r * r * 0.02) as usize + 2, "{dock:?}: the drawn corners differ (TL, TR, BL, BR): {c:?}");
        }
    });
}

// ------------------------------------------------------------------------------------------------------------------------------ keyboard mode, UIA, high contrast

fn key(s: &mut Sim, code: glasskit::input::KeyCode, mods: Modifiers) -> glasskit::input::Response {
    let r = s.send(&InputEvent::Key { code, mods, down: true, repeat: false });
    s.run(400);
    r
}

#[test]
fn the_widget_lists_its_buttons_and_the_position_slider() {
    use crate::ui::a11y::nav::{Role, Value as CV};
    let (mut s, f) = on_screen(Dock::Bottom);
    // folded: the three buttons only
    let cs = s.view.controls();
    assert_eq!(cs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Previous track", "Pause", "Next track"], "a playing track offers Pause");
    assert!(cs.iter().all(|c| c.role == Role::Button && c.rect.w > 10.0));
    // opened (hover): the position slider joins, with the track's own range and value
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    let cs = s.view.controls();
    let bar = cs.iter().find(|c| c.role == Role::Slider).expect("the position bar");
    assert!(matches!(bar.value, CV::Range { min, max, cur, .. } if min == 0.0 && max == 200_000.0 && (cur - 61_000.0).abs() < 3000.0), "{:?}", bar.value);
    // the slider is set by a screen reader: a seek to that position
    let taken = s.view.control_act(bar.id, crate::ui::a11y::nav::Act::Set(100_000.0), &mut s.cx);
    assert!(taken);
    assert_eq!(calls(&f, "media_seek").last(), Some(&json!({ "ms": 100_000 })));
    // a pause changes the name of the middle button
    media(&mut s, &f, state(false, "Song", 61_000));
    assert_eq!(s.view.controls()[1].name, "Play");
}

#[test]
fn keyboard_mode_space_arrows_and_escape() {
    use glasskit::input::KeyCode as K;
    let (mut s, f) = on_screen(Dock::Bottom);
    f.bus.emit("a11y:keys", json!({ "target": "widget" }));
    s.pump();
    s.run(4000);
    assert!(s.view.focused_control().is_some(), "keyboard mode: the ring stands on a button");
    assert!(s.view.controls().iter().any(|c| c.role == crate::ui::a11y::nav::Role::Slider), "the card opened for the keys");
    key(&mut s, K::Space, Modifiers::NONE);
    assert_eq!(calls(&f, "media_control").last(), Some(&json!({ "action": "play_pause" })));
    key(&mut s, K::Right, Modifiers::NONE);
    let seek = calls(&f, "media_seek").last().cloned().expect("Right seeks");
    assert!((seek["ms"].as_u64().unwrap() as i64 - 66_000).abs() < 3000, "5 s forward: {seek}");
    key(&mut s, K::Left, Modifiers::SHIFT);
    assert_eq!(calls(&f, "media_control").last(), Some(&json!({ "action": "prev" })), "Shift+Left is the previous track");
    // a letter is not ours; Tab moves the ring and Enter presses what it stands on
    assert!(!key(&mut s, K::Char('a'), Modifiers::NONE).consumed);
    let before = s.view.focused_control();
    key(&mut s, K::Tab, Modifiers::NONE);
    assert_ne!(s.view.focused_control(), before);
    key(&mut s, K::Escape, Modifiers::NONE);
    assert_eq!(s.view.focused_control(), None, "Esc leaves");
}
