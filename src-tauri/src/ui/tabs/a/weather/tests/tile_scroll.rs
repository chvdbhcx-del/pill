//! The weather tiles under the daily list: six of them from the forecast's own numbers, a one-sentence summary, a column that scrolls to
//! them, tiles that rise in when they come into view, readings that ease when a new forecast arrives.
use super::*;

fn with_tile_data(mut f: Value) -> Value {
    f["now"]["pressure"] = json!(1012.0);
    f["now"]["uv"] = json!(3.0);
    f["now"]["uv_max"] = json!(5.0);
    f
}

fn ok_view(r: &Rig) -> &pill_ui_core::panel::a::weather::OkView {
    match &r.v.vw {
        Vw::Ok(v) => v,
        _ => panic!("no forecast"),
    }
}

fn wheel(r: &mut Rig, dy: f32) {
    let c = r.v.right_rect().center();
    r.send(InputEvent::Wheel { pos: c, delta: (0.0, dy), mods: Modifiers::NONE, precise: false });
}

fn frames(r: &mut Rig, n: usize) {
    for _ in 0..n {
        r.cx.now_ms += 4.0;
        if !r.v.step(4.0, &mut r.cx) {
            break;
        }
    }
}

#[test]
fn the_forecast_brings_six_tiles_and_one_sentence() {
    let _m = motion(false);
    let r = Rig::on(with_tile_data(forecast("lightrain", "rain", 4.4)));
    let v = ok_view(&r);
    assert_eq!(v.tiles.len(), 6);
    assert_eq!(v.tiles[2].num, Some(1012.0));
    assert!(v.summary.starts_with("Light rain now,") && v.summary.ends_with('.') && !v.summary[..v.summary.len() - 1].contains(". "), "{}", v.summary);
    // a forecast from before the tiles existed still shows what it has
    let r = Rig::on(forecast("lightrain", "rain", 4.4));
    assert_eq!(ok_view(&r).tiles.len(), 4, "no pressure and no UV tile without data");
}

#[test]
fn the_tiles_wait_below_the_fold_and_rise_in_when_the_column_scrolls() {
    let _m = motion(false);
    let mut r = Rig::on(with_tile_data(forecast("lightrain", "rain", 4.4)));
    r.frame();
    assert_eq!(r.v.scroll.offset(), 0.0);
    assert!(!r.v.tiles.revealing(), "not in view yet");
    assert!(!r.v.step(4.0, &mut r.cx), "an idle tab costs no frames");
    wheel(&mut r, 300.0);
    frames(&mut r, 400);
    assert!(r.v.scroll.offset() > 100.0, "the column scrolled: {}", r.v.scroll.offset());
    assert!(r.v.tiles.revealing(), "the tiles came into view");
    let mut n = 0;
    while r.v.step(4.0, &mut r.cx) && n < 3000 {
        r.cx.now_ms += 4.0;
        n += 1;
    }
    assert!(n < 3000 && r.v.tiles.settled(), "the grid settles and the tab idles ({n} frames)");
    let img = render_view_cx(&mut r.v, &r.cx, (super::super::super::W, H), 1.0, &Theme::pill_dark());
    assert!(img.count(None, |p| p[3] > 0) > 10_000);
}

#[test]
fn a_new_forecast_eases_the_readings_instead_of_jumping() {
    let _m = motion(false);
    let mut r = Rig::on(with_tile_data(forecast("lightrain", "rain", 4.4)));
    wheel(&mut r, 400.0);
    frames(&mut r, 2000);
    let before = r.v.tiles.one_val(1);
    let mut f2 = with_tile_data(forecast("lightrain", "rain", 4.4));
    f2["now"]["humidity"] = json!(41.0);
    r.v.model_mut().on_result("weather_get", &Value::Null, &f2);
    let c = r.cx.now_ms;
    r.v.pump(&mut r.cx, vec![]);
    r.cx.now_ms = c;
    frames(&mut r, 6);
    let mid = r.v.tiles.one_val(1);
    assert!(mid < before && mid > 41.0, "between the old and the new reading: {before} -> {mid}");
    frames(&mut r, 2000);
    assert!((r.v.tiles.one_val(1) - 41.0).abs() < 0.2);
}

#[test]
fn the_unit_buttons_follow_the_scroll_and_stay_inside_the_column() {
    let _m = motion(false);
    let mut r = Rig::on(with_tile_data(forecast("lightrain", "rain", 4.4)));
    r.f.api.reply("settings_set", Ok(json!({ "net.weather": true, "weather.units": "imperial" })));
    wheel(&mut r, 120.0);
    frames(&mut r, 400);
    let off = r.v.scroll.offset();
    assert!(off > 30.0);
    // the pill moved up with the column: its old place is now the tiles / summary, not a button
    let old = r.v.unit_rect(1).center();
    r.click(old);
    assert!(r.calls("settings_set").is_empty(), "{:?}", r.calls("settings_set"));
    let moved = r.v.unit_rect(1).offset(0.0, -off).center();
    if r.v.right_rect().contains(moved.0, moved.1) {
        r.click(moved);
        assert_eq!(r.calls("settings_set").len(), 1);
    }
}

#[test]
fn reduced_motion_shows_the_tiles_at_once() {
    let _m = motion(true);
    let mut r = Rig::on(with_tile_data(forecast("lightrain", "rain", 4.4)));
    wheel(&mut r, 400.0);
    frames(&mut r, 50);
    assert!(r.v.tiles.settled());
}
