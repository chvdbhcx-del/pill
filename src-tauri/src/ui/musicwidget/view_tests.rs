//! The motion state of the view itself (springs, ghosts, accent, the bar), driven by hand: no window, no host, a clock that is just a number.
//! The behaviour on the real frame scheduler is in `tests.rs`.
use super::*;
use crate::ui::host::testing::{motion_lock, motion_on};
use crate::ui::view::Services;

struct H {
    v: MusicView,
    cx: Cx,
    now: f64,
}

fn track(title: &str, accent: &str, pos: u64) -> Value {
    json!({ "has_session": true, "playing": true, "title": title, "artist": "Band", "album": "LP", "app": "Spotify.exe", "position_ms": pos, "duration_ms": 200_000,
            "thumb_id": 0, "accent": accent })
}

/// A widget that plays `title` and has just been shown (the window asked for it), settled.
fn shown(title: &str) -> H {
    let (svc, f) = Services::fake();
    f.api.reply("settings_get", Ok(json!({ "media.widget": true, "module.media": true, "theme": "dark", "language": "en" })));
    let (mut cx, _q) = Cx::headless_queue(svc);
    let v = MusicView::new(&mut cx, Dock::Bottom);
    let mut h = H { v, cx, now: 1000.0 };
    h.apply(track(title, "#ff0000", 10_000));
    h.v.on_show(&mut h.cx);
    h.run(2000.0);
    h
}

impl H {
    fn apply(&mut self, t: Value) {
        self.cx.now_ms = self.now;
        self.v.apply(&t, &mut self.cx);
    }

    /// Frames of 4 ms (a 250 Hz screen) for `ms`; whether it still moved at the end.
    fn run(&mut self, ms: f64) -> bool {
        let mut moving = false;
        for _ in 0..(ms / 4.0) as usize {
            self.now += 4.0;
            self.cx.now_ms = self.now;
            moving = self.v.step(4.0, &mut self.cx);
        }
        moving
    }
}

#[test]
fn a_run_of_skips_never_brings_an_old_line_back_to_full() {
    let _m = motion_on();
    let mut h = shown("One");
    assert!(h.v.ghosts.is_empty() && !h.v.moving(), "settled: nothing fades, nothing moves");
    h.apply(track("Two", "#ff0000", 0));
    h.run(80.0);
    let seen = arrival(h.v.swap.at01()).0;
    assert!(seen > 0.0 && seen < 1.0, "Two is half in: {seen}");
    h.apply(track("Three", "#ff0000", 0));
    // Two fades from exactly as visible as it was; One is still fading on its own spring, never back up
    let two = h.v.ghosts.last().expect("a ghost for Two");
    assert_eq!(two.head, "Two");
    assert!((two.a.at01() - seen).abs() < 1e-3, "Two goes on from {seen}, not from full: {}", two.a.at01());
    let one = h.v.ghosts[0].a.at01();
    h.run(8.0);
    assert!(h.v.ghosts[0].a.at01() <= one + 1e-4, "One only goes down");
    assert!(h.v.ghosts.len() <= 3);
    // when everything has settled no ghost is left and nothing asks for frames
    assert!(!h.run(1500.0));
    assert!(h.v.ghosts.is_empty());
}

#[test]
fn the_new_title_comes_after_the_old_one_has_mostly_gone_and_the_artist_after_the_title() {
    for k in [0.0, 0.1, 0.25] {
        assert_eq!(arrival(k), (0.0, 0.0), "nothing new is visible yet at {k}");
    }
    let mut last = (0.0, 0.0);
    for i in 0..=20 {
        let a = arrival(i as f32 / 20.0);
        assert!(a.0 >= last.0 && a.1 >= last.1, "only grows");
        assert!(a.1 <= a.0, "the artist is never ahead of the title");
        last = a;
    }
    assert_eq!(arrival(1.0), (1.0, 1.0));
}

#[test]
fn a_reversed_open_turns_around_from_where_it_is_with_no_jump() {
    let _m = motion_on();
    let mut h = shown("Song");
    h.v.on_proximity(&ProximityEvent::Hover, &mut h.cx);
    h.run(90.0);
    let open = h.v.expand.at();
    assert!(open > 0.1 && open < 0.95, "caught half way: {open}");
    let v = h.v.expand.velocity();
    assert!(v > 0.0, "and still opening");
    h.v.on_proximity(&ProximityEvent::Leave, &mut h.cx);
    h.run(4.0);
    let after = h.v.expand.at();
    assert!((after - open).abs() < 0.08, "one frame later it is where it was, not somewhere else: {open} -> {after}");
    assert!(after >= open - 1e-3, "it carries on a little before turning (the spring keeps its velocity)");
    h.run(1500.0);
    assert_eq!(h.v.expand.at(), 0.0);
    assert!(!h.v.moving(), "and it idles folded");
}

#[test]
fn the_accent_eases_to_the_new_cover_and_lands_exactly() {
    let _m = motion_on();
    let mut h = shown("Song");
    assert_eq!(h.v.accent().r, 1.0);
    h.apply(track("Other", "#0000ff", 0));
    h.run(60.0);
    let c = h.v.accent();
    assert!(c.r > 0.01 && c.r < 0.99 && c.b > 0.01 && c.b < 0.99, "between red and blue: {c:?}");
    assert!(h.v.moving(), "easing");
    assert!(!h.run(1500.0), "settled");
    let c = h.v.accent();
    assert_eq!((c.r, c.g, c.b), (0.0, 0.0, 1.0));
    // a hidden widget takes it at once: nobody sees it
    let (svc, f) = Services::fake();
    f.api.reply("settings_get", Ok(json!({ "media.widget": true, "module.media": true })));
    let (mut cx, _q) = Cx::headless_queue(svc);
    let mut v = MusicView::new(&mut cx, Dock::Top);
    v.apply(&track("A", "#00ff00", 0), &mut cx);
    assert!(!v.acc.iter().any(Flow::moving) && v.accent().g == 1.0);
}

#[test]
fn a_step_of_the_bar_under_a_pixel_just_lands_and_a_bigger_one_glides() {
    let _m = motion_on();
    let mut h = shown("Song");
    h.v.prog.jump(0.5);
    let one_px = 1.0 / lg::card_layout(Rect::new(0.0, 0.0, lg::BODY_W, lg::CARD_H), h.v.stand.style).bar.w / h.v.stand.bs();
    h.v.glide_prog(0.5 + 0.5 * one_px);
    assert!(!h.v.prog.moving() && h.v.prog.at() == 0.5 + 0.5 * one_px, "half a device px: it lands, no frames");
    h.v.glide_prog(0.5 + 3.0 * one_px);
    assert!(h.v.prog.moving(), "three px: a glide");
    assert!(!h.run(1500.0));
    assert!((h.v.prog.at() - (0.5 + 3.0 * one_px)).abs() < 1e-3);
    // a long jump (a seek) is not slower than 400 ms to settle in spirit: it is done well inside a second
    h.v.glide_prog(0.9);
    assert!(!h.run(1000.0));
}

#[test]
fn the_knob_grows_on_hover_and_again_on_drag_and_goes_when_the_card_closes() {
    let _m = motion_on();
    let mut h = shown("Song");
    assert_eq!(h.v.knob.target(), 0.0, "folded: no handle");
    h.v.on_proximity(&ProximityEvent::Hover, &mut h.cx);
    assert_eq!(h.v.knob.target(), 0.35, "the card is open: a dot");
    h.v.bar_hot = true;
    h.v.aim_knob();
    assert_eq!(h.v.knob.target(), 0.75, "under the pointer: bigger");
    h.v.seek = Some(0.3);
    h.v.aim_knob();
    assert_eq!(h.v.knob.target(), 1.0, "dragged: biggest");
    h.v.seek = None;
    h.v.on_proximity(&ProximityEvent::Leave, &mut h.cx);
    assert_eq!(h.v.knob.target(), 0.0);
}

#[test]
fn reduce_motion_lands_every_new_spring_in_one_frame() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(true));
    let mut h = shown("One");
    h.apply(track("Two", "#0000ff", 0));
    h.v.on_proximity(&ProximityEvent::Hover, &mut h.cx);
    h.v.kick[1].nudge(80.0);
    h.v.bar_hot = true;
    h.v.aim_knob();
    let moving = h.run(4.0);
    let (c, ghosts, swap, expand, knob, kick) = (h.v.accent(), h.v.ghosts.len(), h.v.swap.at(), h.v.expand.at(), h.v.knob.at(), h.v.kick[1].at());
    glasskit::ease::force_reduced(None);
    assert!(!moving, "one frame and it is done");
    assert_eq!((c.r, c.b, ghosts, swap, expand, knob, kick), (0.0, 1.0, 0, 1.0, 1.0, 0.75, 0.0));
}

#[test]
fn a_playing_widget_with_nothing_moving_asks_for_no_frames() {
    let _m = motion_on();
    let mut h = shown("Song");
    assert!(!h.v.moving());
    assert!(!h.run(40.0), "step says: no more frames");
    // the once-a-second tick moves the bar by a hair: it lands or glides and settles again
    h.now += 1000.0;
    h.cx.now_ms = h.now;
    let _ = h.v.tick(&mut h.cx);
    assert!(!h.run(1500.0));
}

#[test]
fn the_bar_follows_the_pointer_closely_while_scrubbing_and_settles_back_to_its_own_feel_after() {
    use glasskit::input::{Buttons, Modifiers};
    let _m = motion_on();
    let mut h = shown("Song");
    h.v.on_proximity(&ProximityEvent::Hover, &mut h.cx);
    h.run(1500.0);
    assert_eq!(h.v.expand.at(), 1.0, "the card is open");
    let bar = h.v.stand.rect_to_real(lg::parts(h.v.stand, 1.0, 0.0).bar);
    let at = |f: f32| (bar.x + bar.w * f, bar.cy());
    let (down, mv) = (
        |p: (f32, f32)| InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 },
        |p: (f32, f32)| InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE },
    );
    h.v.event(&down(at(0.2)), &mut h.cx);
    assert_eq!(h.v.prog.feel(), SCRUB, "a heavy follow would lag behind a finger");
    h.v.event(&mv(at(0.7)), &mut h.cx);
    h.run(120.0);
    assert!((h.v.prog.at() - 0.7).abs() < 0.04, "120 ms after the pointer got there the fill is there too: {}", h.v.prog.at());
    assert_eq!(h.v.knob.target(), 1.0, "and the knob is at its biggest");
    h.v.event(&InputEvent::MouseUp { pos: at(0.7), button: MouseButton::Left, mods: Modifiers::NONE }, &mut h.cx);
    assert_eq!(h.v.prog.feel(), Feel::DRAG, "released: the ordinary glide again");
}

#[test]
fn the_card_takes_the_islands_player_style_from_the_settings() {
    use pill_ui_core::panel::a::home_player::PlayerStyle;
    let mut h = shown("Song");
    assert_eq!(h.v.stand.style, PlayerStyle::Mirror, "the island's default");
    let mirror = lg::parts(h.v.stand, 1.0, 0.0);
    h.v.take_settings(&json!({ "media.widget": true, "home.player": "compact" }));
    assert_eq!(h.v.stand.style, PlayerStyle::Compact);
    let compact = lg::parts(h.v.stand, 1.0, 0.0);
    assert!(compact.cover.x < mirror.cover.x && compact.cover.w > mirror.cover.w, "the compact card has the big cover at the left");
    h.v.take_settings(&json!({ "media.widget": true }));
    assert_eq!(h.v.stand.style, PlayerStyle::Mirror, "no setting: the default again");
}

#[test]
fn the_cover_worker_brings_the_blurred_cover_and_the_tint_which_ease_from_the_last_cover() {
    use crate::ui::tabs::a::draw::solid;
    let _m = motion_on();
    let mut h = shown("Song");
    let red = (Color::rgb8(200, 50, 70), Color::rgb8(90, 20, 40));
    let blue = (Color::rgb8(40, 90, 210), Color::rgb8(20, 30, 90));
    let with_cover = |id: u64| json!({ "has_session": true, "playing": true, "title": "Song", "artist": "Band", "app": "Spotify.exe", "position_ms": 0, "duration_ms": 200_000, "thumb_id": id, "thumbnail": "data:image/png;base64,AAAA" });
    h.apply(with_cover(7));
    let key = h.v.cov_key.clone().expect("the worker was asked for this cover");
    h.apply(with_cover(7));
    assert_eq!(h.v.cov_key.as_deref(), Some(key.as_str()), "once per cover, not per reading");
    assert!(!h.v.cover_ready(CoverReady { key: "cover:1:1".into(), colors: Some(red), tiny: None }), "an older cover's answer is ignored");
    assert!(h.v.cover_ready(CoverReady { key: key.clone(), colors: Some(red), tiny: Some(solid(24, 24, [200, 50, 70, 255])) }));
    assert!(h.v.images.contains(h.v.blur.as_deref().unwrap()) && h.v.cov_tint() == Some(red), "the first tint lands at once");
    // another cover: the tint eases from the last one, and lands exactly
    h.apply(with_cover(8));
    let key2 = h.v.cov_key.clone().unwrap();
    assert!(h.v.cover_ready(CoverReady { key: key2, colors: Some(blue), tiny: None }));
    assert!(h.v.moving(), "it eases");
    h.run(100.0);
    let mid = h.v.cov_tint().unwrap();
    assert!(mid != red && mid != blue, "between the two");
    h.run(1500.0);
    assert_eq!(h.v.cov_tint(), Some(blue));
    assert!(!h.v.moving());
}

/// The relative luminance and the WCAG contrast of two colours.
fn lum(c: Color) -> f32 {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

fn contrast(a: Color, b: Color) -> f32 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn the_capsules_ink_is_always_light_on_its_dark_body_and_reads_on_the_cover_end_for_every_tint() {
    let _m = motion_on();
    let (l, d) = (Color::rgb8(0xf6, 0xf6, 0xf6), Color::rgb8(0x14, 0x14, 0x14));
    for th in [Theme::pill_dark(), Theme::pill_light()] {
        let tint = |c: Color| Some((c, c));
        let name = if th.dark { "dark" } else { "light" };
        let (white, yellow, black, blue) = (Color::rgb8(250, 250, 250), Color::rgb8(250, 222, 40), Color::rgb8(4, 4, 6), Color::rgb8(40, 90, 210));
        let ink = |t, a: Color| paint::ink_target(&th, t, a, false);
        // the folded body runs from the cover's deep tone into black, on either theme: the light ink, whatever the cover is
        for c in [white, yellow, black, blue] {
            assert_eq!(ink(tint(c), c), 0.0, "{name} theme, rgb({}, {}, {})", c.r, c.g, c.b);
        }
        assert_eq!(ink(None, Color::rgb8(10, 10, 12)), 0.0);
        // the open fill card is strong: a bright cover gets the dark ink and a dark one the light ink there on both themes
        assert_eq!((paint::ink_target(&th, tint(yellow), yellow, true), paint::ink_target(&th, tint(black), black, true)), (1.0, 0.0));
        // the title is readable on the cover end of the capsule (its lightest part) on every tint of a sweep of the colour cube, and on the open fill
        // card's strong body with the ink it picks
        for r in (0..=255).step_by(51) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(51) {
                    let c = Color::rgb8(r as u8, g as u8, b as u8);
                    let cr = contrast(l, paint::soft_pair(&th, tint(c), c).0);
                    assert!(cr >= 3.5, "rgb({r},{g},{b}): the light ink reads at {cr:.2}:1 on the capsule's cover end");
                    let k = paint::ink_target(&th, tint(c), c, true);
                    let cr = contrast(if k > 0.5 { d } else { l }, paint::body_mid(&th, tint(c), c, true));
                    assert!(cr >= 3.5, "rgb({r},{g},{b}) on the {name} theme (strong): the ink reads at {cr:.2}:1");
                }
            }
        }
    }
}

/// The capsule's body is the cover's own deep colour at its end, a clean tone of the same hue (a yellow turns amber, not olive) and near black at the far end.
#[test]
fn the_capsule_body_runs_from_a_clean_deep_tone_of_the_cover_into_black() {
    let th = Theme::pill_dark();
    let ends = |c: Color| paint::soft_pair(&th, Some((c, c)), c);
    let (left, right) = ends(Color::rgb8(250, 215, 40));
    assert!(left.r > left.g && left.g > left.b * 1.8 && (left.g / left.r) < 0.85, "yellow turns deep amber, not olive: {left:?}");
    assert!(right.r < 0.08 && right.g < 0.08 && right.b < 0.08, "near black at the far end: {right:?}");
    let (red, _) = ends(Color::rgb8(210, 40, 60));
    assert!(red.r > 0.3 && red.r > red.g * 2.5, "a red stays a clear red: {red:?}");
    let (pale, _) = ends(Color::rgb8(245, 243, 240));
    assert!((0.3..0.5).contains(&pale.r), "a pale cover gives a mid grey, not a glowing one: {pale:?}");
}

/// The folded body reaches its dark end a little past the middle, as the open card's does, and holds it to the far end; the open card spans the whole body.
#[test]
fn the_folded_body_is_plain_dark_from_a_little_past_the_middle_and_the_open_card_spans_the_whole_body() {
    let (a, b) = (Color::rgb8(180, 120, 20), Color::rgb8(10, 10, 10));
    assert!((0.55..=0.6).contains(&paint::BODY_END), "{}", paint::BODY_END);
    let s = paint::ease_stops(a, b, 1.0, 1.0);
    assert!(s[0].0 == 0.0 && (s[4].0 - paint::BODY_END).abs() < 1e-6 && s[5].0 == 1.0, "{s:?}");
    assert!(s.windows(2).all(|w| w[0].0 <= w[1].0), "stops in order: {s:?}");
    assert!((s[4].1.r - s[5].1.r).abs() < 1e-6 && (s[4].1.g - s[5].1.g).abs() < 1e-6, "dark from the end on: {s:?}");
    // smooth: no step bigger than a third of the whole change between neighbouring stops (no seam)
    let total = s[0].1.r - s[4].1.r;
    assert!(s.windows(2).all(|w| (w[0].1.r - w[1].1.r).abs() <= total / 2.0 + 1e-4), "{s:?}");
    let card = paint::ease_stops(a, b, 0.0, 1.0);
    assert!((card[4].0 - 1.0).abs() < 1e-6, "the card's gradient spans the whole body: {card:?}");
}

#[test]
fn the_ink_glides_when_the_body_turns_light_or_dark_and_lands_at_once_while_nobody_looks() {
    let _m = motion_on();
    let mut h = shown("Song");
    let with_cover = |id: u64| json!({ "has_session": true, "playing": true, "title": "Song", "artist": "Band", "app": "Spotify.exe", "position_ms": 0, "duration_ms": 200_000, "thumb_id": id, "thumbnail": "data:image/png;base64,AAAA" });
    h.apply(with_cover(7));
    let key = h.v.cov_key.clone().unwrap();
    assert!(h.v.cover_ready(CoverReady { key, colors: Some((Color::rgb8(250, 230, 60), Color::rgb8(240, 200, 40))), tiny: None }));
    h.run(1500.0);
    assert_eq!(h.v.ink.at01(), 0.0, "light ink on the dark capsule");
    // the open fill card has a strong, bright body for a bright cover: the ink turns over on its spring, in steps, and lands on the dark ink
    h.v.stand.style = PlayerStyle::Fill;
    h.v.expand.to(1.0);
    h.v.follow_ink(false);
    assert_eq!(h.v.ink.target(), 1.0);
    assert!(h.v.ink.at01() < 0.5 && h.v.moving(), "it has not flipped yet");
    h.run(60.0);
    let mid = h.v.ink.at01();
    assert!(mid > 0.0 && mid < 1.0, "on its way: {mid}");
    h.run(1500.0);
    assert_eq!(h.v.ink.at01(), 1.0);
    // nobody looks (the window is hidden): it lands without a frame
    h.v.on_hide(&mut h.cx);
    h.v.expand.jump(0.0);
    h.v.follow_ink(true);
    assert_eq!((h.v.ink.at01(), h.v.ink.moving()), (0.0, false));
}
