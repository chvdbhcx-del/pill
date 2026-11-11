//! The paired Phone sub-tab (`phone_dash.rs` / `phone_pair.rs`): the two tiles fit the body without scrolling in both languages, the QR is hidden
//! until "Pair another phone" asks for it, the phone pills switch the shown phone, and the review pictures (`PILL_TABS_DUMP=<dir>`).
use super::tests::{rich, rig, state_with, status, Rig};
use glasskit::input::Modifiers;
use super::*;
use crate::ui::host::testing::{motion_on, render_view_cx};
use serde_json::{json, Value};

fn phone(id: &str, name: &str, connected: bool, route: &str) -> Value {
    let mut d = rich(connected, 64, 5);
    d["id"] = json!(id);
    d["name"] = json!(name);
    d["route"] = json!(route);
    d
}

fn two(a_on: bool, b_on: bool) -> Value {
    status(true, json!([phone("d1", "Pixel 8", a_on, if a_on { "lan" } else { "none" }), phone("d2", "Galaxy S24", b_on, if b_on { "relay" } else { "none" })]))
}

/// Paints the tab in a box `h` px high (the island may give less than `SIZE`).
fn draw_at(r: &mut Rig, h: f32) {
    render_view_cx(&mut r.v, &r.cx, (SIZE.0, h), 1.0, &Theme::pill_dark());
}

fn open(lang: glasskit::lang::Lang, state: Value) -> Rig {
    let mut r = rig(true, json!([]));
    r.cx.lang = lang;
    r.click(K::Sub(2));
    r.feed("phone:state", state);
    r.settle();
    r
}

const LANGS: [glasskit::lang::Lang; 2] = [glasskit::lang::Lang::En, glasskit::lang::Lang::Ru];
/// the island's smallest body (300 px of panel), the default one
const HEIGHTS: [f32; 2] = [300.0, SIZE.1];

fn assert_fits(r: &mut Rig, what: &str) {
    for h in HEIGHTS {
        draw_at(r, h);
        let f = r.v.dash.fit;
        let sv = &r.v.scroll[2];
        assert!(!sv.state.scrollable(), "{what} @ {h}: content {} in view {}", sv.state.content(), sv.state.viewport());
        assert!(f.need <= f.tile_h + 0.5, "{what} @ {h}: {f:?}");
        assert_eq!(f.clipped, 0, "{what} @ {h}: a text is cut off ({f:?})");
    }
}

#[test]
fn one_phone_fits_the_body_without_scrolling_in_en_and_ru() {
    for lang in LANGS {
        let mut r = open(lang, state_with(rich(true, 85, 12)));
        assert_fits(&mut r, &format!("online {lang:?}"));
        let mut r = open(lang, state_with(rich(false, 85, 12)));
        assert_fits(&mut r, &format!("offline {lang:?}"));
        assert!(r.v.dash.conn.at() < 0.01, "dimmed");
    }
}

#[test]
fn two_phones_fit_too_with_the_pill_row() {
    for lang in LANGS {
        let mut r = open(lang, two(true, false));
        assert_fits(&mut r, &format!("two {lang:?}"));
        assert_eq!(r.v.dash.pill_cache.len(), 2);
        let mut r = open(lang, two(true, true));
        assert_fits(&mut r, &format!("two online {lang:?}"));
    }
}

#[test]
fn a_bare_old_phone_fits_and_a_too_short_body_falls_back_to_the_scroll_view() {
    let mut r = open(glasskit::lang::Lang::En, state_with(json!({ "id": "d1", "name": "Old", "connected": true, "battery": 20, "charging": false, "network": "mobile", "dnd": false })));
    assert_fits(&mut r, "old phone");
    let mut r = open(glasskit::lang::Lang::En, state_with(rich(true, 85, 12)));
    draw_at(&mut r, 190.0);
    assert!(r.v.scroll[2].state.scrollable(), "the fallback keeps the whole picture reachable");
    assert!(r.v.dash.fit.need > r.v.dash.fit.tile_h - 1.0 && r.v.dash.fit.clipped == 0, "{:?}", r.v.dash.fit);
}

#[test]
fn the_qr_is_hidden_for_a_paired_phone_and_pair_another_phone_slides_it_in() {
    let _m = motion_on();
    let mut r = open(glasskit::lang::Lang::En, state_with(rich(true, 85, 12)));
    r.shoot();
    assert!(r.v.qr_cache.is_none(), "no QR built while it is hidden");
    assert!(r.v.clicks.rect_of(&K::NewCode).is_none() && r.v.clicks.rect_of(&K::Pair).is_some());
    r.click(K::Pair);
    assert!(r.v.dash.qr_open);
    r.frames(4);
    let mid = r.v.dash.qr.at();
    assert!(mid > 0.0 && mid < 1.0, "sliding in: {mid}");
    r.settle();
    assert!((r.v.dash.qr.at() - 1.0).abs() < 0.01);
    r.shoot();
    assert!(r.v.qr_cache.is_some());
    assert!(r.v.clicks.rect_of(&K::NewCode).is_some() && r.v.clicks.rect_of(&K::Pair).is_none(), "the panel covers the tiles");
    r.click(K::NewCode);
    assert!(r.calls().contains(&("phone_pair_code".into(), json!({ "renew": true }))));
    // Escape slides it away
    let esc = InputEvent::Key { code: KeyCode::Escape, mods: Modifiers::NONE, down: true, repeat: false };
    assert!(r.v.event(&esc, &mut r.cx).consumed);
    assert!(!r.v.dash.qr_open);
    r.frames(4);
    assert!(r.v.dash.qr.at() < 1.0, "on its way out");
    r.settle();
    assert!(r.v.dash.qr.at().abs() < 0.01);
    assert!(!r.frames(2), "idle again");
    // the close button and the scrim close it as well
    r.click(K::Pair);
    r.settle();
    r.click(K::CloseQr);
    assert!(!r.v.dash.qr_open);
    r.settle();
    r.click(K::Pair);
    r.settle();
    r.click(K::Scrim);
    assert!(!r.v.dash.qr_open);
    r.settle();
    // a new pairing closes the panel by itself
    r.click(K::Pair);
    r.settle();
    r.feed("phone:state", two(true, true));
    r.frames(2);
    assert!(!r.v.dash.qr_open);
}

#[test]
fn nobody_paired_shows_the_qr_card_as_the_content() {
    let mut r = open(glasskit::lang::Lang::Ru, status(true, json!([])));
    r.shoot();
    assert!(r.v.qr_cache.is_some());
    assert!(r.v.clicks.rect_of(&K::NewCode).is_some() && r.v.clicks.rect_of(&K::Pair).is_none());
    assert!(r.v.shown().is_none());
}

#[test]
fn the_pills_switch_the_shown_phone_and_the_selection_glides() {
    let _m = motion_on();
    let mut r = open(glasskit::lang::Lang::En, two(true, true));
    assert_eq!(r.v.shown().map(|d| d.name.as_str()), Some("Pixel 8"), "the first connected one");
    let x0 = r.v.dash.pill_x.at();
    r.click(K::Pick(1));
    assert_eq!(r.v.shown().map(|d| d.name.as_str()), Some("Galaxy S24"));
    r.frames(3);
    let mid = r.v.dash.pill_x.at();
    assert!(mid > x0 && (mid - r.v.dash.pill_cache[1].x).abs() > 0.5, "gliding: {x0} -> {mid}");
    r.settle();
    assert!((r.v.dash.pill_x.at() - r.v.dash.pill_cache[1].x).abs() < 0.5);
    assert!(!r.frames(2));
    // the other phone going offline keeps the choice
    r.feed("phone:state", two(false, true));
    r.settle();
    assert_eq!(r.v.shown().map(|d| d.name.as_str()), Some("Galaxy S24"));
}

/// Review pictures (written only with `PILL_TABS_DUMP=<dir>`; run it alone, parallel GPU renders crash):
/// `cargo test --locked --workspace -- --exact ui::tabs::c::phone::dash_tests::snapshot_dash`
#[test]
fn snapshot_dash() {
    if std::env::var_os("PILL_TABS_DUMP").is_none() {
        return;
    }
    let _m = motion_on();
    for (lang, ln) in [(glasskit::lang::Lang::En, "en"), (glasskit::lang::Lang::Ru, "ru")] {
        let cases: Vec<(&str, Value, bool)> = vec![
            ("connected", state_with(rich(true, 85, 12)), false),
            ("offline", state_with(rich(false, 85, 12)), false),
            ("qr-open", state_with(rich(true, 85, 12)), true),
            ("none", status(true, json!([])), false),
            ("two", two(true, false), false),
        ];
        for (name, st, qr) in cases {
            let mut r = open(lang, st);
            if qr {
                r.click(K::Pair);
                r.settle();
            }
            r.frames(30);
            for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
                crate::ui::tabs::a::snap::shoot(&mut r.v, &r.cx, &th, 2.0, "phone", &format!("{name}-{ln}-{tn}"));
            }
            // the island's smallest body (300 px of panel)
            let img = render_view_cx(&mut r.v, &r.cx, (SIZE.0, 300.0), 2.0, &Theme::pill_dark());
            crate::ui::host::testing::save_png(&img, crate::ui::tabs::a::snap::path("phone", &format!("{name}-{ln}-dark-300")));
        }
    }
}
