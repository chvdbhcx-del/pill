//! The animated sky card: the ambient frames the host paces, the counting temperature, the lightning, the pointer lean, Reduce motion,
//! and pictures of the conditions (`PILL_TABS_DUMP=<dir> cargo test ... left_card_pictures -- --exact`, alone: parallel GPU renders crash).
use super::*;
use crate::ui::tabs::a::weather::leftcard::lean_for;
use crate::ui::tabs::a::weather::sky::Scene;

fn ok_view(r: &Rig) -> &pill_ui_core::panel::a::weather::OkView {
    match &r.v.vw {
        Vw::Ok(v) => v,
        _ => panic!("no forecast"),
    }
}

/// Steps `secs` of 33 ms frames; (steps that asked for frames, steps marked ambient, flashes started, brightest flash)
fn run(r: &mut Rig, secs: f64) -> (u32, u32, u32, f32) {
    let (mut asked, mut amb, mut flashes, mut peak, mut last_on) = (0, 0, 0, 0.0f32, -1.0e9f64);
    let n = (secs / 0.033) as usize;
    for _ in 0..n {
        r.cx.now_ms += 33.0;
        let more = r.v.step(33.0, &mut r.cx);
        asked += more as u32;
        amb += r.cx.take_ambient() as u32;
        let f = r.v.left.flash();
        peak = peak.max(f);
        if f > 0.0 {
            // the two pulses of one flash are 0.4 s apart: they are one flash
            flashes += (r.cx.now_ms - last_on > 1500.0) as u32;
            last_on = r.cx.now_ms;
        }
    }
    (asked, amb, flashes, peak)
}

#[test]
fn the_open_tab_breathes_as_ambient_frames_and_a_hidden_one_is_silent() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("clearsky", "clear-day", 17.0));
    run(&mut r, 3.0); // the entrance and the counting are over
    let t0 = r.v.left.clock();
    let (asked, amb, ..) = run(&mut r, 3.0);
    assert!(asked > 80 && asked == amb, "every frame is ambient once nothing else moves: {asked} asked, {amb} ambient");
    assert!(r.v.left.clock() > t0 + 2.5, "the clock runs");
    // closed: nothing is asked and the clock stands still
    r.v.on_hide(&mut r.cx);
    let t1 = r.v.left.clock();
    let (asked, amb, ..) = run(&mut r, 1.0);
    assert_eq!((asked, amb), (0, 0));
    assert_eq!(r.v.left.clock(), t1);
    // shown again: breathing again
    r.v.on_show(&mut r.cx);
    r.pump();
    r.frame();
    assert!(run(&mut r, 3.0).1 > 20);
}

#[test]
fn real_motion_is_not_reported_as_ambient() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("clearsky", "clear-day", 17.0));
    run(&mut r, 3.0);
    r.v.left.rewind(false); // the number counts again: a real movement
    r.cx.now_ms += 33.0;
    assert!(r.v.step(33.0, &mut r.cx));
    assert!(!r.cx.take_ambient(), "counting is not ambient: the host keeps the full rate for it");
}

#[test]
fn reduce_motion_means_a_still_card() {
    let _m = motion(true);
    let mut r = Rig::on(forecast("heavyrainandthunder", "thunder", 21.0));
    r.cx.reduced_motion = true;
    r.v.base.reduced = true;
    run(&mut r, 1.0);
    let t0 = r.v.left.clock();
    let (asked, amb, flashes, _) = run(&mut r, 20.0);
    assert_eq!((asked, amb, flashes), (0, 0, 0), "no frames, no flashes");
    assert_eq!(r.v.left.clock(), t0);
    assert_eq!(r.v.left.count_text(), "21°", "the number is simply there");
    // the pointer does not move the icon
    r.send(InputEvent::MouseMove { pos: (10.0, 10.0), mods: Modifiers::NONE, buttons: Buttons::NONE });
    run(&mut r, 1.0);
    assert_eq!(r.v.left.lean_at(), (0.0, 0.0));
    // and it still draws (the still picture)
    let img = render_view_cx(&mut r.v, &r.cx, (super::super::super::W, H), 1.0, &Theme::pill_dark());
    assert!(img.count(None, |p| p[3] > 0) > 10_000);
}

#[test]
fn the_temperature_counts_up_when_the_tab_opens_and_lands_exactly() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("lightrain", "rain", 4.4));
    run(&mut r, 3.0);
    assert_eq!(r.v.left.count_text(), "4°");
    r.v.on_hide(&mut r.cx);
    r.v.on_show(&mut r.cx);
    r.pump();
    assert_eq!(r.v.left.count_text(), "0°", "it starts again from zero");
    run(&mut r, 3.0);
    assert_eq!(r.v.left.count_text(), "4°");
}

#[test]
fn a_new_value_counts_to_it_instead_of_jumping() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("lightrain", "rain", 4.4));
    run(&mut r, 3.0);
    r.f.api.reply("weather_get", Ok(forecast("lightrain", "rain", 12.0)));
    let p = r.v.rf_rect().center();
    r.send(InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE });
    r.send(InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
    r.send(InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE });
    r.cx.now_ms += 33.0;
    r.v.step(33.0, &mut r.cx);
    let mid = r.v.left.count_text().to_string();
    assert!(["4°", "5°", "6°"].contains(&mid.as_str()), "first frames sit near the old value: {mid}");
    run(&mut r, 3.0);
    assert_eq!(r.v.left.count_text(), "12°");
}

#[test]
fn only_a_thunderstorm_flashes_and_softly_and_rarely() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("heavyrainandthunder", "thunder", 21.0));
    assert_eq!(r.v.left.scene(), Scene::Thunder);
    let (_, _, flashes, peak) = run(&mut r, 62.0);
    assert!((6..=16).contains(&flashes), "one flash every 4 to 8 s: {flashes} in a minute");
    assert!(peak > 0.5 && peak <= 1.0, "peak {peak}");
    let mut dry = Rig::on(forecast("lightrain", "rain", 4.0));
    assert_eq!(run(&mut dry, 40.0).2, 0, "rain alone never flashes");
}

#[test]
fn the_icon_leans_towards_the_pointer_a_few_px_and_comes_back() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("clearsky", "clear-day", 17.0));
    let card = Rect::new(0.0, 0.0, 214.0, H);
    assert_eq!(lean_for(card.center(), card), (0.0, 0.0));
    let (x, y) = lean_for((card.right(), card.bottom()), card);
    assert!(x > 3.0 && x < 4.0 && y > 3.0 && y < 4.0, "a few px at most: {x} {y}");
    assert_eq!(lean_for((-500.0, -500.0), card), (-x, -y), "clamped");
    r.send(InputEvent::MouseMove { pos: (200.0, 300.0), mods: Modifiers::NONE, buttons: Buttons::NONE });
    run(&mut r, 1.5);
    let at = r.v.left.lean_at();
    assert!(at.0 > 2.0 && at.0 <= 3.6 && at.1 > 0.5 && at.1 <= 3.6, "{at:?}");
    // outside the card (the right column) and leaving the window: back to rest
    r.send(InputEvent::MouseMove { pos: (400.0, 100.0), mods: Modifiers::NONE, buttons: Buttons::NONE });
    run(&mut r, 1.5);
    assert_eq!(r.v.left.lean_at(), (0.0, 0.0));
    r.send(InputEvent::MouseMove { pos: (20.0, 20.0), mods: Modifiers::NONE, buttons: Buttons::NONE });
    r.send(InputEvent::MouseLeave);
    run(&mut r, 1.5);
    assert_eq!(r.v.left.lean_at(), (0.0, 0.0));
}

#[test]
fn the_symbol_code_sets_how_hard_it_rains() {
    let _m = motion(false);
    for (sym, want) in [("lightrain", 0), ("rain", 1), ("heavyrain", 2), ("heavyrainandthunder", 2), ("lightsnowshowers_day", 0), ("cloudy", 1)] {
        let r = Rig::on(forecast(sym, "rain", 4.0));
        assert_eq!(ok_view(&r).intensity, want, "{sym}");
    }
}

#[test]
fn the_scene_and_the_number_follow_a_new_forecast() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("clearsky", "clear-day", 17.0));
    run(&mut r, 3.0);
    assert_eq!(r.v.left.scene(), Scene::ClearDay);
    r.f.api.reply("weather_get", Ok(forecast("snow", "snow", -4.0)));
    let rf = r.v.rf_rect();
    r.click(rf.center());
    run(&mut r, 3.0);
    assert_eq!(r.v.left.scene(), Scene::Snow);
    assert_eq!(r.v.left.count_text(), "-4°");
}

// ------------------------------------------------------------------------------------------------------------------ pictures

#[test]
fn left_card_pictures() {
    let _m = motion(false);
    for (name, sym, icon, t) in [
        ("clear-day", "clearsky", "clear-day", 17.0),
        ("clear-night", "clearsky", "clear-night", 6.0),
        ("partly", "partlycloudy", "partly-day", 5.0),
        ("cloudy", "cloudy", "cloud", 9.0),
        ("rain", "heavyrain", "rain", 4.4),
        ("drizzle", "lightrain", "rain", 8.0),
        ("thunder", "heavyrainandthunder", "thunder", 21.0),
        ("snow", "snow", "snow", -4.0),
        ("fog", "fog", "fog", 3.0),
        ("sleet", "sleet", "sleet", 1.0),
    ] {
        let mut r = Rig::on(forecast(sym, icon, t));
        run(&mut r, 4.0);
        r.v.left.set_clock(37.3);
        if icon == "thunder" {
            r.v.left.force_flash(0.85);
        }
        for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
            r.cx.scale = 3.0;
            let img = shoot(&mut r.v, &r.cx, &th, 3.0, "weather", &format!("sky-{name}-{tn}"));
            assert!(img.count(None, |p| p[3] > 0) > 4000, "{name} {tn} drew nothing");
        }
    }
}
