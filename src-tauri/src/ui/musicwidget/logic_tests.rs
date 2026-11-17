//! Tests of the widget's pure part (`logic.rs`).
use super::*;
use serde_json::json;

fn playing(title: &str) -> Value {
    json!({ "has_session": true, "playing": true, "title": title, "artist": "Band", "album": "LP", "app": "Spotify.exe", "position_ms": 61_000, "duration_ms": 200_000,
            "thumb_id": 7, "thumbnail": "data:image/png;base64,AAAA", "accent": "#aa3355" })
}

#[test]
fn a_media_event_becomes_a_track_and_a_bad_one_becomes_nothing_in_particular() {
    let t = Track::from_json(&playing("Song"));
    assert_eq!((t.has_session, t.playing, t.title.as_str(), t.artist.as_str(), t.position_ms, t.duration_ms, t.thumb_id), (true, true, "Song", "Band", 61_000, 200_000, 7));
    assert_eq!(t.accent, Some([0xaa, 0x33, 0x55]));
    assert_eq!(t.thumbnail.as_deref(), Some("data:image/png;base64,AAAA"));
    let none = Track::from_json(&Value::Null);
    assert_eq!(none, Track::default());
    let t = Track::from_json(&json!({ "has_session": true, "accent": "orange", "thumbnail": "", "position_ms": -5.0, "duration_ms": 1.5e3 }));
    assert_eq!((t.accent, t.thumbnail, t.position_ms, t.duration_ms), (None, None, 0, 1500));
    assert_eq!(parse_hex("#Гї12345"), None);
    assert_eq!(parse_hex("#0a0B0c"), Some([10, 11, 12]));
}

#[test]
fn the_lines_fall_back_to_the_player_and_a_track_change_is_not_a_seek() {
    let mut t = Track::from_json(&playing("Song"));
    assert_eq!((t.heading().as_str(), t.sub().as_str()), ("Song", "Band"));
    let id = t.identity();
    t.position_ms = 99_000;
    t.playing = false;
    assert_eq!(t.identity(), id, "a seek and a pause keep the track");
    t.title = "Other".into();
    assert_ne!(t.identity(), id);
    t.artist.clear();
    assert_eq!(t.sub(), "Spotify", "no artist: the player's name");
    t.title.clear();
    assert_eq!((t.heading().as_str(), t.sub().as_str()), ("Spotify", ""));
}

#[test]
fn times_and_fractions_are_plain() {
    assert_eq!((clock_text(0), clock_text(61_000), clock_text(3_599_999), clock_text(3_661_000)), ("0:00".into(), "1:01".into(), "59:59".into(), "1:01:01".into()));
    assert_eq!((fraction(50, 200), fraction(300, 200), fraction(5, 0)), (0.25, 1.0, 0.0));
}

#[test]
fn the_widget_shows_while_playing_and_for_a_while_after_a_pause_and_never_without_the_switch() {
    let mut p = Presence::default();
    p.update(true, true, 0.0);
    assert!(!p.visible(0.0), "the module switch has not been read yet: privacy first");
    p.set_enabled(true);
    assert!(p.visible(0.0));
    p.update(true, false, 10_000.0);
    assert!(p.visible(10_000.0 + PAUSE_GRACE_MS - 1.0), "paused recently: still there");
    assert!(!p.visible(10_000.0 + PAUSE_GRACE_MS), "the grace is over");
    p.update(true, false, 100_000.0);
    assert!(!p.visible(100_001.0), "paused events do not extend the grace");
    p.update(true, true, 200_000.0);
    assert!(p.visible(200_000.0));
    p.update(false, false, 200_100.0);
    assert!(!p.visible(200_100.0), "no session, no widget");
    p.update(true, true, 300_000.0);
    p.set_enabled(false);
    assert!(!p.visible(300_000.0), "module.media off: gone at once");
}

#[test]
fn a_player_that_starts_paused_does_not_call_the_widget_up() {
    let mut p = Presence::default();
    p.set_enabled(true);
    p.update(true, false, 5_000.0);
    assert!(!p.visible(5_000.0));
    p.update(true, true, 6_000.0);
    p.update(true, false, 9_000.0);
    assert!(p.visible(9_000.0), "but a pause after playing does");
}

#[test]
fn the_body_grows_away_from_the_dock_edge_and_stays_on_it() {
    let (b0, b1) = (body(Dock::Bottom, 0.0, 0.0), body(Dock::Bottom, 1.0, 0.0));
    assert_eq!((b0.bottom(), b1.bottom(), b0.h, b1.h), (WIN.1 - PAD, WIN.1 - PAD, CAPSULE_H, CARD_H), "the bottom floor does not move while it grows up");
    assert!(b1.y >= PAD - 0.01, "and the open card fits in the window");
    let (t0, t1) = (body(Dock::Top, 0.0, 0.0), body(Dock::Top, 1.0, 0.0));
    assert_eq!((t0.y, t1.y, t1.bottom()), (PAD, PAD, PAD + CARD_H), "the top ceiling stays and the card hangs down");
    assert_eq!((b0.x, b0.w, b1.w), (PAD, CAP_W, BODY_W), "the capsule is narrower than the card and the body widens as it opens");
    // leaving: slid toward the screen edge, out of the window (it is clipped there)
    assert!(body(Dock::Bottom, 0.0, slide_distance()).y > b0.y && body(Dock::Top, 0.0, slide_distance()).y < t0.y);
    assert!(body(Dock::Bottom, 0.0, slide_distance()).y >= WIN.1 - PAD - CAPSULE_H + slide_distance() - 0.01);
    // a spring may overshoot a little: never a negative height
    assert!(body(Dock::Top, -0.5, 0.0).h > 0.0);
}

#[test]
fn the_capsule_and_the_card_lay_their_parts_out_inside_themselves() {
    for dock in [Dock::Bottom, Dock::Top] {
        let (cap, card) = frames(dock, 0.0, 0.0);
        let l = capsule_layout_in(dock, cap);
        for r in [l.cover, l.text, l.btn[0], l.btn[1], l.btn[2], l.bar] {
            assert!(r.x >= cap.x && r.right() <= cap.right() && r.y >= cap.y && r.bottom() <= cap.bottom(), "{r:?} in {cap:?}");
        }
        assert!(l.text.w > 100.0 && l.btn[0].right() <= l.btn[1].x + 0.01 && l.btn[1].right() <= l.btn[2].x + 0.01, "the buttons do not overlap");
        assert!(l.text.right() <= l.btn[0].x, "the text stops before the buttons");
        let c = card_layout(card, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror);
        for r in [c.cover, c.src, c.title, c.sub, c.bar, c.elapsed, c.total, c.btn[0], c.btn[1], c.btn[2]] {
            assert!(r.x >= card.x && r.right() <= card.right() && r.y >= card.y && r.bottom() <= card.bottom(), "{r:?} in {card:?}");
        }
        assert!(c.cover.bottom() <= c.title.y && c.title.y < c.sub.y && c.sub.bottom() <= c.bar.y, "cover, title, artist line and the track stack down the card");
    }
}

#[test]
fn clicks_mean_what_the_open_state_says() {
    let d = Dock::Bottom;
    let (cap, card) = frames(d, 0.0, 0.0);
    let play_c = capsule_layout_in(d, cap).btn[1].center();
    assert_eq!(hit(d, 0.0, 0.0, play_c), Hit::Button(Btn::Play));
    assert_eq!(hit(d, 0.0, 0.0, capsule_layout_in(d, cap).btn[0].center()), Hit::Button(Btn::Prev));
    assert_eq!(hit(d, 0.0, 0.0, capsule_layout_in(d, cap).cover.center()), Hit::Body);
    assert_eq!(hit(d, 0.0, 0.0, (2.0, 2.0)), Hit::None, "outside the body the window is click-through");
    // the closed capsule does not answer where the card's buttons will be
    let c = card_layout(card, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror);
    assert_eq!(hit(d, 0.0, 0.0, (c.btn[2].center().0, c.cover.y + 4.0)), Hit::None);
    assert_eq!(hit(d, 1.0, 0.0, c.btn[2].center()), Hit::Button(Btn::Next));
    let mid = c.bar.x + c.bar.w / 2.0;
    assert_eq!(hit(d, 1.0, 0.0, (mid, c.bar.cy())), Hit::Seek(0.5));
    assert_eq!(hit(d, 1.0, 0.0, (c.bar.x - 3.0, c.bar.cy() + 6.0)), Hit::Seek(0.0), "the grab strip is a little bigger than the bar");
    assert_eq!(hit(d, 1.0, 0.0, c.title.center()), Hit::Body);
    // half way: the buttons answer where they are drawn, between their two places (the capsule's while it is mostly closed)
    assert_eq!(hit(d, CARD_AT - 0.01, 0.0, parts(d, CARD_AT - 0.01, 0.0).btn[1].center()), Hit::Button(Btn::Play));
    assert_eq!(hit(d, 0.0, 0.0, play_c), Hit::Button(Btn::Play));
    assert_eq!(seek_at(c.bar, c.bar.x + c.bar.w * 3.0), 1.0);
}

#[test]
fn the_shared_parts_stand_where_the_capsule_and_the_card_put_them_and_travel_smoothly_between() {
    let near = |a: Rect, b: Rect| (a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01 && (a.w - b.w).abs() < 0.01 && (a.h - b.h).abs() < 0.01;
    for st in [Stand::from(Dock::Bottom), Stand::from(Dock::Top), in_bar(Align::Left, 48.0), in_bar(Align::Right, 48.0), in_bar(Align::Center, 40.0)] {
        let (a, b) = (capsule_layout_in(st, frames(st, 0.0, 0.0).0), card_layout(frames(st, 1.0, 0.0).1, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror));
        let (p0, p1) = (parts(st, 0.0, 0.0), parts(st, 1.0, 0.0));
        assert!(near(p0.cover, a.cover) && (0..3).all(|i| near(p0.btn[i], a.btn[i])), "folded: the capsule's places");
        assert!(near(p1.cover, b.cover) && (0..3).all(|i| near(p1.btn[i], b.btn[i])) && near(p1.bar, b.bar) && near(p1.head, b.title) && near(p1.sub, b.sub), "open: the card's places");
        assert_eq!((p1.cover_r, p1.cover.w), (CARD_COVER_R, 84.0), "the Home card's cover (the mirror style)");
        // in between: the cover only grows, the corners only round off, nobody jumps
        let mut prev = p0;
        for i in 1..=40 {
            let q = parts(st, i as f32 / 40.0, 0.0);
            assert!(q.cover.w >= prev.cover.w - 0.01 && (q.cover.w - prev.cover.w) < 4.0, "{st:?}: the cover grows in small steps");
            assert!((q.cover.y - prev.cover.y).abs() < 8.0 && (q.btn[1].cx() - prev.btn[1].cx()).abs() < 6.0, "{st:?}: the buttons travel, they do not jump");
            prev = q;
        }
        // a spring's overshoot stays a sane rectangle
        assert!(parts(st, 1.08, 0.0).cover.w > 0.0 && parts(st, -0.3, 0.0).cover.w > 0.0);
    }
    // a bar capsule too thin for a hairline grows its track from the card's
    let thin = in_bar(Align::Left, 24.0);
    assert!(near(parts(thin, 0.0, 0.0).bar, card_layout(frames(thin, 0.0, 0.0).1, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror).bar));
}

// ----- in the taskbar

fn in_bar(align: Align, bar: f32) -> Stand {
    Stand { dock: Dock::Taskbar, align, bar, scale: 1.0, k: 1.0, dx: 0.0, style: Default::default() }
}

/// The bar heights of the common taskbars at the common scalings (and the odd ones, 110 / 175 %), as the view is told them: the bar in
/// logical px (a whole number of device px over the scale) and the scale.
fn bars() -> Vec<(f32, f32)> {
    let mut v = Vec::new();
    for dpi in [96u32, 106, 120, 144, 168, 192] {
        let s = geom::scale_from_dpi(dpi);
        for bar in [40.0f32, 48.0, 52.0] {
            v.push((geom::phys(bar, s) as f32 / s, s));
        }
    }
    v
}

#[test]
fn the_folded_capsule_sits_inside_the_bar_centred_and_fits_it() {
    for (bar, scale) in bars() {
        for align in [Align::Left, Align::Center, Align::Right] {
            let st = Stand { align, bar, scale, ..in_bar(align, bar) };
            let b = body(st, 0.0, 0.0);
            // the window is a whole number of device px: its floor (the monitor's bottom) is the rounded height
            let sf = st.surface();
            assert!((sf.1 - WIN_BAR.1).abs() <= 0.5 / scale + 1e-3 && (sf.1 * scale - (sf.1 * scale).round()).abs() < 1e-2);
            let (floor, top) = (sf.1, sf.1 - bar);
            assert!(b.y >= top - 0.01 && b.bottom() <= floor + 0.01, "{bar}@{scale}: {b:?} inside the strip [{top}, {floor}]");
            assert!(((b.y - top) - (floor - b.bottom())).abs() < 0.01, "{bar}@{scale}: centred in the bar");
            // whole device px on every edge, the same margin above and below
            for v in [b.y, b.bottom(), b.x, b.right()] {
                assert!((v * scale - (v * scale).round()).abs() < 0.02, "{bar}@{scale}: {v} is on a device px: {b:?}");
            }
            assert!(b.h <= bar - 8.0 + 0.01 && b.h <= CAPSULE_H && b.h >= 16.0 - 1.0 / scale, "{bar}@{scale}: {} tall", b.h);
            assert!((b.w - BAR_FOLD_W).abs() <= 0.5 / scale + 1e-3);
            match align {
                Align::Left => assert!((b.x - PAD).abs() <= 0.5 / scale + 1e-3),
                Align::Right => assert!((b.right() - (WIN_BAR.0 - PAD)).abs() <= 0.5 / scale + 1e-3),
                Align::Center => assert!((b.cx() - sf.0 / 2.0).abs() <= 0.5 / scale + 1e-3),
            }
            assert!(b.x >= 0.0 && b.right() <= sf.0, "inside the window");
        }
    }
    // the tallest bar the widget lives in still has the open card inside the window
    for bar in [24.0, 40.0, 72.0] {
        let b = body(in_bar(Align::Left, bar), 1.0, 0.0);
        assert!(b.y >= PAD - 0.01 && (WIN_BAR.1 - b.bottom() - (bar + geom::BAR_FLOAT_GAP)).abs() < 0.01, "{bar}: the open card floats 8 px above the bar: {b:?}");
        assert_eq!((b.w, b.h), (BODY_W, CARD_H));
    }
}

#[test]
fn opening_raises_the_capsule_out_of_the_bar_without_a_jump_and_closing_puts_it_back() {
    let st = in_bar(Align::Left, 48.0);
    let mut prev = body(st, 0.0, 0.0);
    for i in 1..=20 {
        let b = body(st, i as f32 / 20.0, 0.0);
        assert!(b.bottom() <= prev.bottom() + 0.01 && b.y <= prev.y + 0.01, "it only rises while it opens");
        assert!((b.bottom() - prev.bottom()).abs() < 12.0 && (b.y - prev.y).abs() < 12.0, "in small steps: {prev:?} -> {b:?}");
        assert!(b.x >= PAD - 0.01 && b.x == PAD, "the anchored side stays put");
        prev = b;
    }
    // the same on the right side: the right edge stays where the bar's slot ends while the card widens to the left
    let r = in_bar(Align::Right, 48.0);
    assert_eq!((body(r, 0.0, 0.0).right(), body(r, 1.0, 0.0).right()), (WIN_BAR.0 - PAD, WIN_BAR.0 - PAD));
    // overshoot of a spring and a slide out (leaving) keep it a sane rectangle
    assert!(body(st, -0.4, 0.0).h > 0.0 && body(st, 1.05, 0.0).y >= 0.0);
    assert!(body(st, 0.0, slide_distance()).y >= WIN_BAR.1 - 0.01, "leaving slides the capsule down out of the window");
}

#[test]
fn the_capsule_parts_fit_every_bar() {
    for (bar, scale) in bars() {
        let st = Stand { scale, ..in_bar(Align::Left, bar) };
        let (cap, _) = frames(st, 0.0, 0.0);
        let l = capsule_layout_in(st, cap);
        let tag = format!("{bar}@{scale}");
        for r in [l.cover, l.text, l.btn[0], l.btn[1], l.btn[2]] {
            assert!(r.x >= cap.x - 0.01 && r.right() <= cap.right() + 0.01 && r.y >= cap.y - 0.01 && r.bottom() <= cap.bottom() + 0.01, "{tag}: {r:?} in {cap:?}");
        }
        assert!(l.cover.h >= 14.0 - 1.0 / scale && l.cover.h <= cap.h - 6.0, "{tag}: a small cover with room round it");
        assert!(l.btn[0].right() <= l.btn[1].x + 0.01 && l.btn[1].right() <= l.btn[2].x + 0.01, "the buttons do not overlap");
        assert!(l.cover.right() < l.text.x && l.text.right() <= l.btn[0].x && l.text.w > 100.0, "{tag}: one line of text between the cover and the buttons: {:?}", l.text);
        // the progress is a mini waveform between the text and the buttons, or it is gone
        if l.bar.h > 0.0 {
            assert!(l.bar.x >= l.text.right() && l.bar.right() <= l.btn[0].x && l.bar.y >= cap.y && l.bar.bottom() <= cap.bottom(), "{tag}: {:?}", l.bar);
        }
    }
    // a bar so thin that nothing fits beside the text has no waveform at all
    let thin = capsule_layout_in(in_bar(Align::Left, 24.0), frames(in_bar(Align::Left, 24.0), 0.0, 0.0).0);
    assert_eq!(thin.bar.h, 0.0);
}

#[test]
fn the_bar_capsule_is_symmetrical_centred_and_its_waveform_stays_on_the_straight_part() {
    for (bar, scale) in bars() {
        let st = Stand { scale, ..in_bar(Align::Right, bar) };
        let (cap, _) = frames(st, 0.0, 0.0);
        let l = capsule_layout_in(st, cap);
        let tag = format!("{bar}@{scale}");
        let tol = 1.0 / scale + 0.01;
        // the ends: the cover's edge and the last button's edge stand a little in from the rounded ends, on both sides alike (within the rounding)
        let left = l.cover.x - cap.x;
        let right = cap.right() - l.btn[2].right();
        assert!(left >= cap.h * 0.1 && right >= cap.h * 0.1 && (left - right).abs() <= cap.h * 0.08 + tol, "{tag}: {left} on the left, {right} on the right");
        // the cover's corner arcs stay inside the capsule's left end: the arc's centre is `r` from the cover's corner, the end is a circle of radius h / 2
        let r = l.cover.w * COVER_R_RATIO;
        let (cx, cy) = (cap.x + cap.h / 2.0, cap.cy());
        let d = ((l.cover.x + r - cx).powi(2) + (l.cover.y + r - cy).powi(2)).sqrt() + r;
        assert!(d <= cap.h / 2.0 + tol, "{tag}: the cover's corner reaches {d} of {}", cap.h / 2.0);
        // everything on the centre line: the same room above and below, on whole device px
        for r in [l.cover, l.btn[0], l.btn[1], l.btn[2]] {
            assert!(((r.y - cap.y) - (cap.bottom() - r.bottom())).abs() < 0.01, "{tag}: {r:?} centred in {cap:?}");
            assert!(((r.y - cap.y) * scale - ((r.y - cap.y) * scale).round()).abs() < 0.02, "{tag}: on a device px");
        }
        assert!(l.cover.h <= cap.h - 6.0 + 0.01 && l.btn[1].h <= cap.h - 6.0 + 0.01, "{tag}: room round both");
        // the waveform lies in the straight part of the capsule (the rounded ends are never reached)
        if l.bar.h > 0.0 {
            assert!(l.bar.x >= cap.x + cap.h / 2.0 - 0.01 && l.bar.right() <= cap.right() - cap.h / 2.0 + 0.01, "{tag}: {:?}", l.bar);
            assert!(((l.bar.y - cap.y) - (cap.bottom() - l.bar.bottom())).abs() < 0.01, "{tag}: centred");
        }
    }
}

#[test]
fn the_artist_gives_way_before_the_title_is_cut() {
    assert_eq!(bar_line(200.0, 80.0, 60.0), (80.0, Some((80.0 + LINE_GAP, 60.0))), "both fit");
    assert_eq!(bar_line(200.0, 150.0, 90.0), (150.0, Some((150.0 + LINE_GAP, 46.0))), "a whole title and what is left of the artist, cut");
    assert_eq!(bar_line(200.0, 175.0, 90.0), (175.0, None), "no room worth an artist: it is dropped and the title is whole");
    assert_eq!(bar_line(200.0, 260.0, 50.0), (200.0, None), "only a title alone on the line is cut");
    assert_eq!(bar_line(200.0, 200.0, 50.0), (200.0, None));
    assert_eq!(bar_line(200.0, 150.0, 20.0).1, Some((150.0 + LINE_GAP, 20.0)), "a short artist needs only its own length");
    assert_eq!(bar_line(200.0, 80.0, 0.0), (80.0, None), "no artist, nothing to place");
    for head in [0.0, 40.0, 120.0, 199.0, 400.0] {
        let (hw, sub) = bar_line(200.0, head, 120.0);
        assert!(hw <= 200.0 && sub.is_none_or(|(x, w)| x >= hw && x + w <= 200.0 + 0.001), "{head}: nothing runs out of the line");
        if hw < head {
            assert!(sub.is_none(), "{head}: the title is cut, so the artist is gone");
        }
    }
}

#[test]
fn in_the_bar_a_click_on_the_cover_or_the_title_toggles_the_card_and_the_buttons_stay_buttons() {
    let st = in_bar(Align::Left, 48.0);
    let (cap, _) = frames(st, 0.0, 0.0);
    let l = capsule_layout_in(st, cap);
    assert_eq!(hit(st, 0.0, 0.0, l.cover.center()), Hit::Header);
    assert_eq!(hit(st, 0.0, 0.0, l.text.center()), Hit::Header);
    assert_eq!(hit(st, 0.0, 0.0, (l.text.right() + 1.0, l.text.cy())), Hit::Header, "the gap before the buttons too");
    for (i, b) in BTNS.iter().enumerate() {
        assert_eq!(hit(st, 0.0, 0.0, l.btn[i].center()), Hit::Button(*b));
    }
    let (_, card) = frames(st, 1.0, 0.0);
    let c = card_layout(card, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror);
    // (the main button of the mirror style is over the middle of the cover: a click on the rest of it is the header's)
    assert_eq!(hit(st, 1.0, 0.0, (c.cover.x + 4.0, c.cover.y + 4.0)), Hit::Header);
    assert_eq!(hit(st, 1.0, 0.0, c.cover.center()), Hit::Button(Btn::Play));
    assert_eq!(hit(st, 1.0, 0.0, c.title.center()), Hit::Header);
    assert_eq!(hit(st, 1.0, 0.0, c.src.center()), Hit::Body, "elsewhere on the card is just the card");
    assert_eq!(hit(st, 1.0, 0.0, c.btn[0].center()), Hit::Button(Btn::Prev));
    // the screen-edge widgets have no header: hover opens them
    let (cap, _) = frames(Dock::Bottom, 0.0, 0.0);
    assert_eq!(hit(Dock::Bottom, 0.0, 0.0, capsule_layout_in(Dock::Bottom, cap).cover.center()), Hit::Body);
}

#[test]
fn in_the_bar_the_hover_strip_reaches_down_to_the_folded_capsule_and_the_clicks_follow_the_open_state() {
    let st = in_bar(Align::Left, 48.0);
    let (cap, _) = frames(st, 0.0, 0.0);
    let play = capsule_layout_in(st, cap).btn[1].center();
    assert_eq!(hit(st, 0.0, 0.0, play), Hit::Button(Btn::Play), "the small buttons of the folded capsule");
    assert_eq!(hit(st, 0.0, 0.0, (2.0, 2.0)), Hit::None, "outside the body the window is click-through");
    // open: the card floats 8 px over the bar; the cursor still resting on the capsule's place stays on the hover strip
    let open = hit_rect(st, 1.0, 0.0);
    let folded = body(st, 0.0, 0.0);
    assert!(open.contains(folded.cx(), folded.cy()) && open.bottom() >= folded.bottom() - 0.01, "{open:?} reaches {folded:?}");
    assert_eq!(hit(st, 1.0, 0.0, (folded.cx(), folded.cy())), Hit::Header, "the strip the card floats over folds it again with a click");
    let (_, card) = frames(st, 1.0, 0.0);
    let c = card_layout(card, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror);
    assert_eq!(hit(st, 1.0, 0.0, c.btn[1].center()), Hit::Button(Btn::Play), "the card's buttons float above the bar");
    assert!(matches!(hit(st, 1.0, 0.0, (c.bar.cx(), c.bar.cy())), Hit::Seek(f) if (f - 0.5).abs() < 0.01));
    // nothing above the open card and nothing under the window's floor answers
    assert_eq!(hit(st, 1.0, 0.0, (c.bar.cx(), card.y - 6.0)), Hit::None);
    // the plain bodies keep their own rectangle
    assert_eq!(hit_rect(Dock::Bottom, 0.5, 0.0), body(Dock::Bottom, 0.5, 0.0));
}

#[test]
fn a_taskbar_dock_without_a_bar_is_the_plain_bottom_widget_in_the_taller_window() {
    let st = in_bar(Align::Right, 0.0);
    assert!(!st.in_bar() && st.win() == WIN_BAR && Stand::from(Dock::Bottom).win() == WIN);
    for p in [0.0, 0.5, 1.0] {
        let (a, b) = (body(st, p, 0.0), body(Dock::Bottom, p, 0.0));
        assert_eq!((a.w, a.h, WIN_BAR.1 - a.bottom()), (b.w, b.h, PAD), "p={p}: standing on the floor of its window, {PAD} px up");
        assert_eq!(a.right(), WIN_BAR.0 - PAD, "p={p}: and on the side it is anchored to");
    }
    assert_eq!(frames(st, 0.3, 0.0), (body(st, 0.0, 0.0), body(st, 1.0, 0.0)));
    assert_eq!(capsule_layout_in(st, body(st, 0.0, 0.0)).btn[1], capsule_layout_in(Dock::Bottom, body(st, 0.0, 0.0)).btn[1], "the roomy layout");
    assert_eq!(Stand::from(Dock::Top).fold_h(), CAPSULE_H);
    assert!(win_fits());
}

/// The window of the taskbar dock holds the card at the tallest bar and the widest body.
fn win_fits() -> bool {
    let b = body(in_bar(Align::Center, geom::WIDGET_BAR_MAX_H), 1.0, 0.0);
    b.x >= 0.0 && b.y >= 0.0 && b.right() <= WIN_BAR.0 && b.bottom() <= WIN_BAR.1
}

// ----- the card, looked at

#[test]
fn the_card_is_the_home_player_card_in_the_islands_style_and_centred_on_the_cover() {
    use crate::ui::tabs::a::home::layout::PLAYER;
    use pill_ui_core::panel::a::home_player::PlayerStyle;
    assert_eq!((BODY_W, CARD_H), (PLAYER.w, PLAYER.h), "the card is as big as the island's player card");
    let c = Rect::new(0.0, 0.0, BODY_W, CARD_H);
    // mirror (the island's default): the cover in the middle, previous and next beside it, the main button over it
    let l = card_layout(c, PlayerStyle::Mirror);
    assert!(l.centered && (l.cover.cx() - c.cx()).abs() < 0.01 && (l.btn[1].center().0 - l.cover.cx()).abs() < 0.01 && (l.btn[1].center().1 - l.cover.cy()).abs() < 0.01);
    assert!(l.btn[0].right() < l.cover.x && l.btn[2].x > l.cover.right(), "previous and next stand either side of the cover");
    assert!(((l.btn[0].cy() - l.cover.cy()).abs() < 0.01) && ((l.btn[2].cy() - l.cover.cy()).abs() < 0.01), "on one line with it");
    assert!(l.bar.w > 280.0 && l.bar.y > l.sub.bottom(), "the progress track under the lines, full width");
    assert!(l.elapsed.x < l.bar.x + 4.0 && l.total.right() > l.bar.right() - 4.0, "elapsed at the left, remaining at the right");
    // compact (the original card): the cover at the left, the text and the transport in the column beside it
    let k = card_layout(c, PlayerStyle::Compact);
    assert!(!k.centered && k.cover.w == 88.0 && k.title.x > k.cover.right() && k.btn[0].x > k.cover.right());
    // the transport is symmetrical in every style (the main button between the other two)
    for st in [PlayerStyle::Mirror, PlayerStyle::Compact, PlayerStyle::Fill] {
        let l = card_layout(c, st);
        assert!(((l.btn[1].cx() - l.btn[0].cx()) - (l.btn[2].cx() - l.btn[1].cx())).abs() < 0.01, "{st:?}");
        assert!(l.btn[1].w >= 36.0 && l.btn[0].w >= 28.0, "hittable");
    }
    // one radius at the card: the island's; the capsule is a full pill that closes into it
    assert_eq!(body_radius(1.0, CARD_H), CARD_R);
    assert_eq!(CARD_R, 14.0);
    assert_eq!(body_radius(0.0, CAPSULE_H), CAPSULE_H / 2.0, "the capsule is a full pill");
    assert!(body_radius(0.5, 120.0) < CAPSULE_H / 2.0 && body_radius(0.5, 120.0) > CARD_R);
    // the shadow stays inside the window's margin on every side, at the top edge too
    assert!(SHADOW_BLUR + SHADOW_DY < PAD);
}

#[test]
fn the_loudness_bars_tile_the_track_and_the_seek_agrees_with_them() {
    let r = card_layout(Rect::new(0.0, 0.0, BODY_W, CARD_H), pill_ui_core::panel::a::home_player::PlayerStyle::Mirror).bar;
    let n = 32;
    let (x0, w) = wave_cell(r, n, 3.0, 0);
    let (xl, wl) = wave_cell(r, n, 3.0, n - 1);
    assert!((x0 - r.x).abs() < 1e-4 && (xl + wl - r.right()).abs() < 1e-3 && w > 4.0, "{w}");
    let (x1, _) = wave_cell(r, n, 3.0, 1);
    assert!((x1 - (x0 + w + 3.0)).abs() < 1e-4);
    // a click in the middle of the bar i seeks to (about) the middle of slice i
    for i in [0usize, 7, 31] {
        let (x, w) = wave_cell(r, n, 3.0, i);
        assert_eq!((seek_at(r, x + w / 2.0) * n as f32) as usize, i);
    }
}

#[test]
fn the_card_in_the_top_window_is_the_mirror_of_the_bottom_one() {
    // top: the body hangs PAD below the ceiling; bottom: it stands PAD above the floor; the open card is as tall as the window allows
    let (t, b) = (body(Dock::Top, 1.0, 0.0), body(Dock::Bottom, 1.0, 0.0));
    assert_eq!((t.y, WIN.1 - b.bottom()), (PAD, PAD));
    assert_eq!((t.h, b.h, WIN.1 - t.bottom()), (CARD_H, CARD_H, PAD), "the same padding under the card at the top edge");
    assert_eq!(t.x, b.x);
}

// ----- the size

fn on_device_px(v: f32, scale: f32) -> bool {
    (v * scale - (v * scale).round()).abs() < 0.02
}

#[test]
fn the_glide_shift_moves_the_whole_layout_along_the_window_and_nothing_else() {
    for (align, scale) in [(Align::Center, 1.0), (Align::Left, 1.5), (Align::Right, 2.0)] {
        let at = |dx| Stand { dock: Dock::Taskbar, align, bar: 48.0, scale, k: 1.0, dx, style: Default::default() };
        let (o, s) = (at(0.0).origin(), at(30.0).origin());
        assert!((s.0 - o.0 - 30.0).abs() <= 0.5 / scale && s.1 == o.1, "{align:?} @{scale}: {o:?} -> {s:?}");
        let r = at(0.0).rect_to_real(Rect::new(10.0, 10.0, 50.0, 20.0));
        let t = at(-30.0).rect_to_real(Rect::new(10.0, 10.0, 50.0, 20.0));
        assert!((r.x - t.x - 30.0).abs() <= 0.5 / scale && r.y == t.y && (r.w, r.h) == (t.w, t.h));
    }
}

#[test]
fn size_follows_the_island_in_range_and_the_default_is_the_old_layout() {
    assert_eq!((size_k(100.0), size_k(80.0), size_k(200.0), size_k(50.0), size_k(900.0)), (1.0, 0.8, 2.0, KMIN, KMAX));
    assert_eq!(size_k(125.0), 1.25);
    // at 100 % the body stands PAD from its corner exactly where it always did
    let b = Stand::from(Dock::Bottom);
    assert_eq!(b.origin(), (0.0, WIN_REAL.1 - WIN.1));
    assert_eq!(b.rect_to_real(body(b, 0.0, 0.0)), Rect::new(PAD, WIN_REAL.1 - PAD - CAPSULE_H, CAP_W, CAPSULE_H));
    let t = Stand::from(Dock::Top);
    assert_eq!(t.rect_to_real(body(t, 0.0, 0.0)).y, PAD);
    assert_eq!(t.to_base(t.to_real((40.0, 50.0))), (40.0, 50.0));
}

#[test]
fn every_size_edge_align_bar_and_dpi_keeps_the_body_in_its_window_on_device_pixels_and_anchored() {
    let scales = [1.0f32, 1.25, 1.5, 1.75, 2.0];
    for &pct in &[80.0, 100.0, 125.0, 150.0, 175.0, 200.0] {
        let k = size_k(pct);
        for &scale in &scales {
            for align in [Align::Left, Align::Center, Align::Right] {
                for (dock, bar_real) in [(Dock::Bottom, 0.0f32), (Dock::Top, 0.0), (Dock::Taskbar, 0.0), (Dock::Taskbar, 40.0), (Dock::Taskbar, 48.0), (Dock::Taskbar, 52.0), (Dock::Taskbar, 72.0)] {
                    let bar_real = geom::phys(bar_real, scale) as f32 / scale;
                    let st = Stand { dock, align, bar: bar_real / k, scale, k, dx: 0.0, style: Default::default() };
                    let win = st.real_win();
                    let tag = format!("{pct}% {dock:?} {align:?} bar {bar_real} @{scale}");
                    let tol = 1.5 / scale;
                    let o = st.origin();
                    assert!(on_device_px(o.0, scale) && on_device_px(o.1, scale), "{tag}: the origin is on a device px: {o:?}");
                    for p in [0.0f32, 0.5, 1.0] {
                        let r = st.rect_to_real(body(st, p, 0.0));
                        assert!(r.x >= -0.01 && r.y >= -0.01 && r.right() <= win.0 + 0.01 && r.bottom() <= win.1 + 0.01, "{tag} p={p}: {r:?} inside {win:?}");
                        if p == 0.0 || p == 1.0 {
                            for v in [r.x, r.y, r.right(), r.bottom()] {
                                assert!(on_device_px(v, scale), "{tag} p={p}: {v} is on a device px: {r:?}");
                            }
                        }
                        // the hit region always holds the body, and a point of the body maps back into it
                        let h = st.rect_to_real(hit_rect(st, p, 0.0));
                        assert!(h.x <= r.x + 0.01 && h.right() >= r.right() - 0.01 && h.y <= r.y + 0.01 && h.bottom() >= r.bottom() - 0.01, "{tag}");
                        let c = st.to_base((r.cx(), r.cy()));
                        let b = body(st, p, 0.0);
                        assert!((c.0 - b.cx()).abs() < 0.01 && (c.1 - b.cy()).abs() < 0.01, "{tag}: to_base undoes to_real");
                    }
                    let (cap, card) = (st.rect_to_real(body(st, 0.0, 0.0)), st.rect_to_real(body(st, 1.0, 0.0)));
                    // the card is the card at any size
                    assert!((card.w - BODY_W * k).abs() <= tol && (card.h - CARD_H * k).abs() <= tol, "{tag}: the card scales: {card:?}");
                    if st.in_bar() {
                        // the capsule is as big as the size says but never taller than the bar leaves it (the old cap), centred in the bar
                        let cap_h = geom::bar_fold_cap(bar_real).min(CAPSULE_H * k);
                        assert!(cap.h <= cap_h + tol && cap.h >= (cap_h - 2.0 / scale).max(1.0), "{tag}: {} tall, cap {cap_h}", cap.h);
                        let (floor, top) = (win.1, win.1 - bar_real);
                        assert!(cap.y >= top - 0.01 && cap.bottom() <= floor + 0.01, "{tag}: inside the strip");
                        assert!(((cap.y - top) - (floor - cap.bottom())).abs() < 0.01, "{tag}: centred in the bar");
                        assert!(cap.h <= bar_real - 8.0 + 0.01 || bar_real < 24.0, "{tag}: never beyond what the bar allows");
                        assert!((cap.w - st.px(BAR_FOLD_W) * k).abs() <= tol, "{tag}: the width follows the size");
                        match align {
                            Align::Left => assert!((cap.x - BAR_PAD).abs() <= tol, "{tag}: anchored to the slot's left: {cap:?}"),
                            Align::Right => assert!((win.0 - BAR_PAD - cap.right()).abs() <= tol, "{tag}: anchored to the slot's right"),
                            Align::Center => assert!((cap.cx() - win.0 / 2.0).abs() <= tol, "{tag}: centred on the slot"),
                        }
                        // the open card floats the same gap above the bar, in the window
                        assert!((win.1 - bar_real - k * geom::BAR_FLOAT_GAP - card.bottom()).abs() <= tol, "{tag}: floats over the bar: {card:?}");
                    } else {
                        assert!((cap.h - CAPSULE_H * k).abs() <= tol && (cap.w - CAP_W * k).abs() <= tol, "{tag}: the capsule scales: {cap:?}");
                        // the margins to the corner / edge are PAD at the size, so the shadow (PAD) is never cut by the window
                        let m = PAD * k;
                        match align {
                            Align::Left => assert!((cap.x - m).abs() <= tol, "{tag}: {cap:?}"),
                            Align::Right => assert!((win.0 - cap.right() - m).abs() <= tol, "{tag}"),
                            Align::Center => assert!((cap.cx() - win.0 / 2.0).abs() <= tol, "{tag}"),
                        }
                        if dock == Dock::Top {
                            assert!((card.y - m).abs() <= tol && (card.bottom() + m - (WIN.1 * k)).abs() <= tol, "{tag}: the card hangs from the top edge: {card:?}");
                        } else {
                            assert!((win.1 - cap.bottom() - m).abs() <= tol && (win.1 - card.bottom() - m).abs() <= tol, "{tag}: it stands PAD above the floor");
                        }
                        let reach = (SHADOW_BLUR + SHADOW_DY) * k;
                        assert!(card.x - reach >= -0.01 && card.right() + reach <= win.0 + 0.01 && card.y - reach >= -0.01 && card.bottom() + reach <= win.1 + 0.01, "{tag}: the shadow fits in the window");
                    }
                }
            }
        }
    }
}

#[test]
fn the_corner_the_body_grows_from_does_not_move_while_the_size_changes() {
    // the animation of a size change: the dock corner / side stays put, the body grows away from it (no jump of the window either: it is fixed)
    for (dock, align) in [(Dock::Bottom, Align::Left), (Dock::Bottom, Align::Right), (Dock::Top, Align::Left), (Dock::Top, Align::Center), (Dock::Taskbar, Align::Left)] {
        let mut prev: Option<Rect> = None;
        for i in 0..=24 {
            let k = 0.8 + 1.2 * i as f32 / 24.0;
            let st = Stand { dock, align, bar: 0.0, scale: 1.0, k, dx: 0.0, style: Default::default() };
            let r = st.rect_to_real(body(st, 0.0, 0.0));
            let corner = (match align { Align::Left => r.x, Align::Right => r.right(), Align::Center => r.cx() }, if dock == Dock::Top { r.y } else { r.bottom() });
            if let Some(p) = prev {
                let pc = (match align { Align::Left => p.x, Align::Right => p.right(), Align::Center => p.cx() }, if dock == Dock::Top { p.y } else { p.bottom() });
                // the anchor side moves only with the margin (PAD * dk), and never the way the growth goes
                assert!((corner.0 - pc.0).abs() <= PAD * 0.06 + 1.01 && (corner.1 - pc.1).abs() <= PAD * 0.06 + 1.01, "{dock:?} {align:?} k={k}: {pc:?} -> {corner:?}");
                assert!(r.w >= p.w - 1.0 && r.h >= p.h - 1.0, "it only grows");
            }
            prev = Some(r);
        }
    }
}

#[test]
fn the_capsule_is_the_card_in_small_one_cover_shape_and_reflections_that_spread_with_it() {
    use pill_ui_core::panel::a::home_player::PlayerStyle;
    for style in [PlayerStyle::Mirror, PlayerStyle::Compact, PlayerStyle::Fill] {
        for st in [Stand { style, ..Stand::from(Dock::Bottom) }, Stand { style, ..in_bar(Align::Left, 48.0) }, Stand { style, ..in_bar(Align::Right, 40.0) }] {
            let (p0, p1) = (parts(st, 0.0, 0.0), parts(st, 1.0, 0.0));
            // the same cover shape at every size: the corner radius is the same share of the side
            for p in [0.0, 0.3, 0.7, 1.0] {
                let q = parts(st, p, 0.0);
                assert!((q.cover_r / q.cover.w - COVER_R_RATIO).abs() < 1e-4, "{style:?} p={p}");
            }
            assert!((p1.cover_r - CARD_COVER_R).abs() < 0.3 || style != PlayerStyle::Mirror, "{style:?}: the card's own radius at the end: {}", p1.cover_r);
            // the cover is bigger inside the capsule than the old 40, and still smaller than the card's
            assert!(p0.cover.w > 20.0 && p0.cover.w < p1.cover.w, "{style:?}: {} -> {}", p0.cover.w, p1.cover.w);
            // the reflections: the capsule's pair at the start, the card's at the end, and nothing jumps in between
            let (a, b) = (capsule_layout_in(st, frames(st, 0.0, 0.0).0), card_layout(frames(st, 1.0, 0.0).1, style));
            assert!((0..3).all(|i| p0.refl[i] == a.refl[i] && p1.refl[i] == b.refl[i]));
            let mut prev = p0;
            for i in 1..=40 {
                let q = parts(st, i as f32 / 40.0, 0.0);
                assert!((0..3).all(|j| (q.refl[j].x - prev.refl[j].x).abs() < 12.0 && (q.refl[j].w - prev.refl[j].w).abs() < 4.0), "{style:?}: the reflections spread in small steps");
                // the ones beside the cover are always about as tall as the cover, so their hidden edges stay behind it as it grows
                for j in 0..2 {
                    assert!(q.refl[j].h >= q.cover.h && q.refl[j].h <= q.cover.h * 1.3 + 0.5 || style != PlayerStyle::Mirror, "{style:?} p={}: {} over {}", i as f32 / 40.0, q.refl[j].h, q.cover.h);
                    assert!((q.refl[j].cy() - q.cover.cy()).abs() < 0.6, "{style:?}: centred on the cover");
                }
                prev = q;
            }
            // the capsule's two reflections: one bleeds out of the cover, one comes in from the far end
            assert!(a.refl[1].x < a.cover.right() && a.refl[1].right() > a.cover.right(), "the cover's reflection starts behind the cover and reaches past it");
            assert!(a.refl[2].right() > frames(st, 0.0, 0.0).0.right(), "the far one is brightest at the far end, past the capsule's end");
        }
    }
}

#[test]
fn the_text_is_one_line_in_the_cards_size_scaled_with_the_capsule_and_the_waveform_gets_fewer_bars_in_a_strip() {
    // 14.5 at the full height (the mockup's 12.5 at 48), never unreadably small in a strip, never bigger than the card's title
    assert!((capsule_text_size(CAPSULE_H) - 14.5).abs() < 0.01 && (capsule_text_size(48.0) - 12.5).abs() < 0.05);
    assert_eq!((capsule_text_size(20.0), capsule_text_size(90.0)), (11.5, 14.5));
    assert_eq!((wave_n(CAPSULE_H), wave_n(40.0), wave_n(39.0), wave_n(28.0)), (12, 12, 8, 8));
    // the line sits in the text rect, centred on the capsule's height (the text is vertically centred in a rect as tall as the capsule)
    for h in [32.0, 44.0, 48.0, CAPSULE_H] {
        let c = Rect::new(0.0, 0.0, CAP_W, h);
        let tx = capsule_layout(c, 1.0).text;
        assert!(tx.y == 0.0 && tx.h == h && tx.w > 80.0, "{h}: {tx:?}");
    }
}

#[test]
fn the_waveform_is_a_strip_of_two_px_bars_with_equal_gaps_between_the_text_and_the_buttons() {
    for (h, k) in [(CAPSULE_H, 1.0f32), (CAPSULE_H, 2.0), (48.0, 1.0), (48.0, 1.25), (40.0, 1.5), (32.0, 1.0)] {
        let c = Rect::new(0.0, 0.0, if h < CAPSULE_H { BAR_FOLD_W } else { CAP_W }, h);
        let l = capsule_layout(c, k);
        let n = wave_n(h) as f32;
        let bw = l.bar.w / (2.0 * n - 1.0);
        assert!(bw >= 1.0 - 0.01 && bw <= 2.0 + 0.01, "{h}@{k}: a thin bar: {bw}");
        assert!(l.bar.h > 0.0 && l.bar.h <= h - 8.0 && ((l.bar.y - c.y) - (c.bottom() - l.bar.bottom())).abs() < 0.01, "{h}@{k}: centred: {:?}", l.bar);
        assert!(l.text.right() < l.bar.x && l.bar.right() < l.btn[0].x, "{h}@{k}: text, waveform, buttons in a row");
        assert!(((l.bar.x - l.text.right()) - (l.btn[0].x - l.bar.right())).abs() < 1.0 / k + 0.01, "{h}@{k}: the same room on both sides");
    }
}
