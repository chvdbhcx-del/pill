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
        if !r.step_real(4.0) {
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
    assert!(!r.step_real(4.0), "an idle tab costs no real frames (the sky only breathes)");
    wheel(&mut r, 300.0);
    frames(&mut r, 400);
    assert!(r.v.scroll.offset() > 100.0, "the column scrolled: {}", r.v.scroll.offset());
    assert!(r.v.tiles.revealing(), "the tiles came into view");
    let mut n = 0;
    while r.step_real(4.0) && n < 3000 {
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

/// The grid of the tab scrolled to its end: the picture and the screen rect of each of the six cells.
fn grid_at_the_end(r: &mut Rig, th: &Theme) -> (glasskit::testkit::Image, Vec<Rect>) {
    wheel(r, 2000.0);
    frames(r, 2000);
    let mut n = 0;
    while r.step_real(4.0) && n < 3000 {
        r.cx.now_ms += 4.0;
        n += 1;
    }
    let img = render_view_cx(&mut r.v, &r.cx, (super::super::super::W, H), 1.0, th);
    let days = r.v.days_rect();
    let w = (days.w - 2.0 * tiles::GAP) / tiles::COLS as f32;
    let (_, grid_y, _) = tiles::layout(r.v.o().h, 6);
    let off = r.v.scroll.offset();
    let cells = (0..6).map(|i| Rect::new(days.x + (i % 3) as f32 * (w + tiles::GAP), grid_y + (i / 3) as f32 * (tiles::TILE_H + tiles::GAP) - off, w, tiles::TILE_H)).collect();
    (img, cells)
}

#[test]
fn the_uv_tile_ends_the_first_row_and_five_tiles_leave_no_hole() {
    let _m = motion(true);
    let th = Theme::pill_dark();
    // a UV index of 3 (Moderate, yellow): painted in the third cell, in no other cell of the first row
    let mut r = Rig::on(with_tile_data(forecast("lightrain", "rain", 4.4)));
    let kinds: Vec<_> = ok_view(&r).tiles.iter().map(|t| t.kind).collect();
    assert_eq!(kinds[2], pill_ui_core::panel::a::weather_tiles::Kind::Uv);
    let (img, c) = grid_at_the_end(&mut r, &th);
    let card = |r: Rect| img.count(Some(r), |p| p[3] > 0);
    assert!(c.iter().all(|&r| card(r) > 5000), "six cards: {:?}", c.iter().map(|&r| card(r)).collect::<Vec<_>>());
    let yellow = |r: Rect| img.count(Some(r), |p| p[3] == 255 && p[0] > 220 && p[1] > 190 && p[2] < 110);
    assert!(yellow(c[2]) > 20, "the UV value and gauge are drawn in yellow in the third cell: {}", yellow(c[2]));
    assert_eq!((yellow(c[0]), yellow(c[1]), yellow(c[3])), (0, 0, 0));
    // no UV: five tiles, the first five cells are filled and only the last is empty
    let mut f = with_tile_data(forecast("lightrain", "rain", 4.4));
    f["now"].as_object_mut().unwrap().remove("uv");
    f["now"].as_object_mut().unwrap().remove("uv_max");
    let mut r = Rig::on(f);
    assert_eq!(ok_view(&r).tiles.len(), 5);
    let (img, c) = grid_at_the_end(&mut r, &th);
    let card = |r: Rect| img.count(Some(r), |p| p[3] > 0);
    assert!(c[..5].iter().all(|&r| card(r) > 5000), "five cards: {:?}", c.iter().map(|&r| card(r)).collect::<Vec<_>>());
    assert_eq!(card(c[5]), 0, "the sixth cell stays empty");
    assert_eq!(img.count(Some(c[2]), |p| p[3] == 255 && p[0] > 220 && p[1] > 190 && p[2] < 110), 0, "no UV colour where it would be");
}

/// `PILL_TABS_DUMP=<dir> cargo test --release --locked -p pill the_weather_tiles_with_uv_snapshots` writes the tab at the top and scrolled to
/// the tiles, dark and light, English and Russian (UV index 6 = High, peak 8).
#[test]
fn the_weather_tiles_with_uv_snapshots() {
    use glasskit::lang::Lang;
    let _m = motion(true);
    for (lang, ln) in [(Lang::En, "en"), (Lang::Ru, "ru")] {
        for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
            let mut f = with_tile_data(forecast("clearsky", "clear-day", 17.0));
            f["now"]["uv"] = json!(6.0);
            f["now"]["uv_max"] = json!(8.0);
            let mut r = Rig::open_in(json!({ "net.weather": true, "clock24": true }), Some(f), lang);
            r.frame();
            for _ in 0..400 {
                // the entrance of the left card lands (its ambient breathing is not counted as motion by `frames`)
                r.cx.now_ms += 4.0;
                r.v.step(4.0, &mut r.cx);
                r.cx.take_ambient();
            }
            let top = crate::ui::tabs::a::snap::shoot(&mut r.v, &r.cx, &th, 1.0, "weather-uv", &format!("top-{ln}-{tn}"));
            assert!(top.count(None, |p| p[3] > 0) > 10_000);
            wheel(&mut r, 120.0);
            frames(&mut r, 400);
            let one = crate::ui::tabs::a::snap::shoot(&mut r.v, &r.cx, &th, 1.0, "weather-uv", &format!("one-notch-{ln}-{tn}"));
            assert!(one.count(None, |p| p[3] > 0) > 10_000);
            wheel(&mut r, 2000.0);
            frames(&mut r, 2000);
            crate::ui::tabs::a::snap::shoot(&mut r.v, &r.cx, &th, 1.0, "weather-uv", &format!("end-{ln}-{tn}"));
        }
    }
}
