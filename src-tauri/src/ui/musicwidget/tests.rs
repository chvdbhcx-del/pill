//! Headless tests of the music widget: the settings -> window mapping, the show / hide / hover / click / seek behaviour on the real frame
//! scheduler (`host::testing::Sim`), the idle cost, the privacy gate, the fullscreen guard, Reduce motion, and one render. `PILL_SNAP_DIR=<dir>`
//! writes the PNG of the render.
use super::logic::{self as lg, Btn, WIN};
use super::view::MusicView;
use super::*;
use crate::ui::host::testing::{motion_lock, motion_on, Sim};
use crate::ui::host::geom::{Align, Dock};
use crate::ui::view::{Cx, Effects, Fakes, Msg, ProximityEvent, Services, View};
use glasskit::geom::Rect;
use glasskit::input::{Buttons, InputEvent, Modifiers, MouseButton};
use glasskit::theme::Theme;
use serde_json::json;

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
    assert_eq!((monitor_sel("1"), monitor_sel("3"), monitor_sel("0"), monitor_sel("x")), (MonitorSel::Index(0), MonitorSel::Index(2), MonitorSel::Primary, MonitorSel::Primary));
    let p = placement(&sig(json!({ "media.widget": true })).unwrap());
    assert!(matches!(p, Placement::Docked { dock: Dock::Bottom, align: Align::Left, inside_work: true, margin, .. } if margin == 0.0));
}

#[test]
fn the_window_is_one_hidden_click_through_overlay_the_size_of_the_frame() {
    let sp = spec(&sig(json!({ "media.widget": true, "showInCapture": false })).unwrap()).into_spec();
    assert_eq!((sp.kind, sp.size, sp.visible, sp.topmost, sp.activate_on_show, sp.exclude_from_capture), (host::WindowKind::Overlay, WIN, false, true, false, true));
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
    assert!((r.bottom() - (WIN.1 - lg::PAD)).abs() < 0.2, "standing on the bottom edge: {r:?}");
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
    assert!(!fx.hide && (hit_rect(&fx).bottom() - (WIN.1 - lg::PAD)).abs() < 0.3, "it turned around and settled in place");
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
    assert!((r.h - lg::CARD_H).abs() < 0.3 && (r.bottom() - (WIN.1 - lg::PAD)).abs() < 0.3, "the card: {r:?}");
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
    let (cap, _) = lg::frames(Dock::Bottom, 0.0);
    let l = lg::capsule_layout(cap);
    for (b, action) in [(Btn::Prev, "prev"), (Btn::Play, "play_pause"), (Btn::Next, "next")] {
        let c = l.btn[b.index()].center();
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
    let c = l.btn[1].center();
    s.send(&down(c.0, c.1));
    s.send(&up(l.cover.center().0, l.cover.center().1));
    assert_eq!(calls(&f, "media_control").len(), 3);
    // a click on the cover (the body) is swallowed; outside the body nothing is ours
    assert!(s.send(&down(l.cover.center().0, l.cover.center().1)).consumed);
    assert!(!s.send(&down(1.0, 1.0)).consumed);
}

#[test]
fn the_card_seeks_by_a_click_or_a_drag_and_commits_once_on_release() {
    let _m = motion_on();
    let (mut s, f) = on_screen(Dock::Bottom);
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    let (_, card) = lg::frames(Dock::Bottom, 0.0);
    let bar = lg::card_layout(card).bar;
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
    assert_eq!(s.view.size_hint(), Some(WIN));
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
        let capsule = s.render(WIN, &th);
        let body = lg::body(Dock::Bottom, 0.0, 0.0);
        assert!(lit(&capsule, body) as f32 > body.w * body.h * 0.9, "the capsule fills its body");
        let above = Rect::new(0.0, 0.0, WIN.0, WIN.1 - lg::PAD - lg::CAPSULE_H - 14.0);
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
        let card = s.render(WIN, &th);
        let open = lg::body(Dock::Bottom, 1.0, 0.0);
        assert!(lit(&card, Rect::new(open.x, open.y, open.w, open.h - lg::CAPSULE_H)) > 20_000, "the card covers the part above the capsule");
        assert_eq!(lit(&card, Rect::new(0.0, 0.0, WIN.0, open.y - 8.0)), 0, "and nothing beyond it");
        // the same widget at the top edge is the mirror image in the window, not a copy of the bottom one
        let (mut t, f2) = sim(Dock::Top, cfg(json!({})));
        media(&mut t, &f2, state(true, "Song", 61_000));
        show(&mut t);
        let top = t.render(WIN, &th);
        assert!(lit(&top, lg::body(Dock::Top, 0.0, 0.0)) > 15_000 && lit(&top, Rect::new(0.0, lg::CAPSULE_H + lg::PAD + 14.0, WIN.0, 100.0)) == 0);
        if let Ok(dir) = std::env::var("PILL_SNAP_DIR") {
            let _ = std::fs::create_dir_all(&dir);
            crate::ui::host::testing::save_png(&card, std::path::Path::new(&dir).join("music-widget-card.png"));
        }
    });
}
