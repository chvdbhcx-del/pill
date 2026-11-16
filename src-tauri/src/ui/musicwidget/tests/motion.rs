//! The widget's motion, looked at: contact sheets of frame sequences with invented data (`PILL_SNAP_DIR=<dir>`; without it the test does nothing).
//! Every tile is the widget's window cropped to its body area over a flat backdrop, one tile per `step_ms` of simulated time at 60 Hz.
use super::*;
use crate::ui::tabs::a::snap::png_data_url;
use glasskit::testkit::Image;

fn art(top: [u8; 4], bottom: [u8; 4]) -> String {
    png_data_url(48, 48, top, bottom)
}

fn playing(title: &str, id: u64, url: &str, accent: &str, is_playing: bool, pos: u64) -> Value {
    json!({ "has_session": true, "playing": is_playing, "title": title, "artist": "Invented Band", "album": "Made Up LP", "app": "Spotify.exe", "position_ms": pos, "duration_ms": 200_000,
            "thumb_id": id, "thumbnail": url, "accent": accent })
}

/// `r` (logical px, scale 1) of the window over a backdrop, as BGRA.
fn tile(img: &Image, r: Rect) -> Image {
    let (x0, y0, w, h) = (r.x as u32, r.y as u32, r.w as u32, r.h as u32);
    let mut bgra = vec![255u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let p = img.px(x0 + x, y0 + y);
            let bg = [70u32 + (y * 40 / h), 78 + (y * 40 / h), 100 + (y * 30 / h)];
            let a = p[3] as u32;
            let i = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                bgra[i + 2 - c] = (p[c] as u32 + bg[c] * (255 - a) / 255).min(255) as u8;
            }
            bgra[i + 3] = 255;
        }
    }
    Image { w, h, scale: 1.0, bgra }
}

fn sheet(tiles: &[Image], cols: u32) -> Image {
    let (tw, th) = (tiles[0].w + 4, tiles[0].h + 4);
    let rows = (tiles.len() as u32).div_ceil(cols);
    let (w, h) = (tw * cols, th * rows);
    let mut bgra = vec![30u8; (w * h * 4) as usize];
    for (n, t) in tiles.iter().enumerate() {
        let (ox, oy) = ((n as u32 % cols) * tw, (n as u32 / cols) * th);
        for y in 0..t.h {
            let (d, s) = ((((oy + y) * w + ox) * 4) as usize, (y * t.w * 4) as usize);
            bgra[d..d + (t.w * 4) as usize].copy_from_slice(&t.bgra[s..s + (t.w * 4) as usize]);
        }
    }
    Image { w, h, scale: 1.0, bgra }
}

/// `n` tiles, `step_ms` of simulated time apart (frames of 60 Hz run in between; an idle window just lets the time pass).
fn film(s: &mut Sim, th: &Theme, win: (f32, f32), region: Rect, step_ms: f64, n: usize) -> Vec<Image> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(tile(&s.render(win, th), region));
        let until = s.now_ms + step_ms;
        while s.now_ms + 1000.0 / s.hz as f64 <= until && s.step_frame() {}
        s.now_ms = until;
        s.cx.now_ms = until;
    }
    out
}

/// The cover worker (a pool thread) answers a moment after a track arrives: wait for it and deliver the answer.
fn cover_worker(s: &mut Sim) {
    std::thread::sleep(std::time::Duration::from_millis(250));
    s.pump();
}

fn dir() -> Option<std::path::PathBuf> {
    let d = std::path::PathBuf::from(std::env::var("PILL_SNAP_DIR").ok()?);
    let _ = std::fs::create_dir_all(&d);
    Some(d)
}

fn playing_sim(dock: Dock, extra: Value) -> (Sim, Fakes) {
    let (mut s, f) = sim(dock, cfg(extra));
    s.set_refresh_hz(60.0);
    (s, f)
}

fn region(st: lg::Stand) -> Rect {
    let o = st.origin();
    Rect::new(o.0, o.1, lg::WIN.0 * st.k, st.win().1 * st.k)
}

fn save(d: &std::path::Path, name: &str, t: &[Image]) {
    crate::ui::host::testing::save_png(&sheet(t, 4), d.join(name));
}

/// One invocation writes every sheet: `PILL_SNAP_DIR=<dir> cargo test --locked -p <crate> motion_sheets`.
#[test]
fn motion_sheets() {
    let Some(d) = dir() else { return };
    gfx(move || {
        let _m = motion_on();
        let th = Theme::pill_dark();
        let win = lg::WIN_REAL;
        let red = art([200, 50, 70, 255], [90, 20, 40, 255]);
        let blue = art([40, 90, 210, 255], [20, 30, 90, 255]);
        let green = art([60, 190, 110, 255], [20, 80, 50, 255]);
        let gold = art([230, 190, 60, 255], [120, 70, 20, 255]);
        let st = base(Dock::Bottom);
        let r = region(st);

        // 1. music starts, and later stops
        let (mut s, f) = playing_sim(Dock::Bottom, json!({}));
        media(&mut s, &f, playing("Invented Song", 1, &red, "#cc3246", true, 61_000));
        s.show();
        save(&d, "m1-appear.png", &film(&mut s, &th, win, r, 24.0, 16));
        media(&mut s, &f, json!({ "has_session": false }));
        save(&d, "m2-leave.png", &film(&mut s, &th, win, r, 24.0, 16));

        // 2. fold -> card -> fold, and an interrupted open
        let (mut s, f) = playing_sim(Dock::Bottom, json!({}));
        media(&mut s, &f, playing("Invented Song", 1, &red, "#cc3246", true, 61_000));
        s.show();
        s.run(4000);
        cover_worker(&mut s);
        let _ = s.render(win, &th); // the cover is on screen (decoded) before it moves
        s.send_proximity(&ProximityEvent::Hover);
        save(&d, "m3-open.png", &film(&mut s, &th, win, r, 24.0, 16));
        s.send_proximity(&ProximityEvent::Leave);
        save(&d, "m4-close.png", &film(&mut s, &th, win, r, 24.0, 16));
        s.send_proximity(&ProximityEvent::Hover);
        let mut t = film(&mut s, &th, win, r, 24.0, 4);
        s.send_proximity(&ProximityEvent::Leave);
        t.extend(film(&mut s, &th, win, r, 24.0, 12));
        save(&d, "m5-reverse.png", &t);

        // 3. a new track (folded), and another one right behind it; then in the card
        let (mut s, f) = playing_sim(Dock::Bottom, json!({}));
        media(&mut s, &f, playing("Invented Song", 1, &red, "#cc3246", true, 61_000));
        s.show();
        s.run(4000);
        let _ = s.render(win, &th);
        media(&mut s, &f, playing("Another Made Up Tune", 2, &blue, "#2850d2", true, 0));
        let mut t = film(&mut s, &th, win, r, 30.0, 6);
        media(&mut s, &f, playing("Third One", 3, &green, "#3cbe6e", true, 0));
        t.extend(film(&mut s, &th, win, r, 30.0, 10));
        save(&d, "m6-track.png", &t);
        s.send_proximity(&ProximityEvent::Hover);
        s.run(4000);
        media(&mut s, &f, playing("Fourth Tune With A Fairly Long Title", 4, &gold, "#e6be3c", true, 0));
        save(&d, "m7-track-card.png", &film(&mut s, &th, win, r, 30.0, 12));

        // 4. pause, a press on play, play again
        media(&mut s, &f, playing("Fourth Tune With A Fairly Long Title", 4, &gold, "#e6be3c", false, 5_000));
        let mut t = film(&mut s, &th, win, r, 24.0, 8);
        let (_, card) = lg::frames(Dock::Bottom, 1.0, 0.0);
        let c = st.to_real(lg::card_layout(card, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror).btn[1].center());
        s.send(&mv(c.0, c.1));
        s.send(&down(c.0, c.1));
        t.extend(film(&mut s, &th, win, r, 24.0, 4));
        s.send(&up(c.0, c.1));
        media(&mut s, &f, playing("Fourth Tune With A Fairly Long Title", 4, &gold, "#e6be3c", true, 5_000));
        t.extend(film(&mut s, &th, win, r, 24.0, 8));
        save(&d, "m8-press-pause.png", &t);

        // 5. the scrub: hover the bar, press, drag
        let bar = st.rect_to_real(lg::card_layout(card, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror).bar);
        s.send(&mv(bar.x + bar.w * 0.2, bar.cy()));
        let mut t = film(&mut s, &th, win, r, 24.0, 3);
        s.send(&down(bar.x + bar.w * 0.2, bar.cy()));
        s.send(&mv(bar.x + bar.w * 0.7, bar.cy()));
        t.extend(film(&mut s, &th, win, r, 24.0, 9));
        save(&d, "m9-scrub.png", &t);
        s.send(&up(bar.x + bar.w * 0.7, bar.cy()));
        s.run(4000);

        // 5b. prev nudges its icon; the size glides
        s.send_proximity(&ProximityEvent::Hover);
        s.run(4000);
        let prev = st.to_real(lg::parts(st, 1.0, 0.0).btn[0].center());
        s.send(&mv(prev.0, prev.1));
        s.send(&down(prev.0, prev.1));
        let mut t = film(&mut s, &th, win, r, 24.0, 3);
        s.send(&up(prev.0, prev.1));
        t.extend(film(&mut s, &th, win, r, 24.0, 9));
        save(&d, "m12-prev-kick.png", &t);
        s.send_proximity(&ProximityEvent::Leave);
        s.run(4000);
        fakes_settings(&mut s, &f, cfg(json!({ "size": 160 })));
        let r2 = region({
            let mut k = base(Dock::Bottom);
            k.k = 1.6;
            k
        });
        save(&d, "m13-size.png", &film(&mut s, &th, win, r2, 30.0, 12));

        // 6. in the taskbar: appears, opens by a click, closes
        let (svc, fb) = Services::fake();
        fb.api.reply("settings_get", Ok(cfg(json!({}))));
        let mut b = Sim::with_services(Box::new(Blank), svc);
        let v = MusicView::new(&mut b.cx, Dock::Taskbar).with_slot(48.0, Align::Left);
        b.view = Box::new(v);
        b.set_refresh_hz(60.0);
        b.pump();
        let bst = bar_st(Align::Left, 48.0, 1.0);
        let o = bst.origin();
        let br = Rect::new(o.0, lg::WIN_BAR_REAL.1 - 290.0, lg::WIN.0, 290.0);
        media(&mut b, &fb, playing("Invented Song", 1, &red, "#cc3246", true, 61_000));
        b.show();
        save(&d, "m10-bar-appear.png", &film(&mut b, &th, lg::WIN_BAR_REAL, br, 24.0, 12));
        b.run(4000);
        b.send_proximity(&ProximityEvent::Hover);
        let (cap, _) = lg::frames(bst, 0.0, 0.0);
        click(&mut b, bst.to_real(lg::capsule_layout_in(bst, cap).cover.center()));
        let mut t = film(&mut b, &th, lg::WIN_BAR_REAL, br, 24.0, 12);
        click(&mut b, bst.to_real(lg::capsule_layout_in(bst, cap).cover.center()));
        t.extend(film(&mut b, &th, lg::WIN_BAR_REAL, br, 24.0, 8));
        save(&d, "m11-bar-open-close.png", &t);
    });
}

/// A widget with the card open (settings `extra`; a track heard before up to `known` bars of 32), settled and drawn once so that the cover is decoded.
fn open_sim(extra: Value, known: usize, url: &str, accent: &str) -> Sim {
    let (svc, f) = Services::fake();
    f.api.reply("settings_get", Ok(cfg(extra)));
    let mut s = Sim::with_services(Box::new(Blank), svc);
    let mut v = MusicView::new(&mut s.cx, Dock::Bottom);
    // a loudness profile as if the track had been heard (the memory is keyed by title, artist and length)
    let bytes: [u8; 32] = std::array::from_fn(|i| if i < known { (60.0 + 190.0 * (0.5 + 0.5 * ((i as f32 * 0.7).sin() * (i as f32 * 0.23).cos()))) as u8 } else { 0 });
    v.seed_loudness(crate::ui::musicwidget::wave::key_of("Invented Song", "Invented Band", 200_000), bytes);
    s.view = Box::new(v);
    s.set_refresh_hz(60.0);
    s.pump();
    media(&mut s, &f, playing("Invented Song", 1, url, accent, true, 61_000));
    s.show();
    s.run(4000);
    cover_worker(&mut s);
    s.send_proximity(&ProximityEvent::Hover);
    s.run(4000);
    let _ = s.render(lg::WIN_REAL, &Theme::pill_dark());
    s.run(4000);
    s
}

fn open_variant(extra: Value, known: usize, url: &str, accent: &str) -> Image {
    let mut s = open_sim(extra, known, url, accent);
    tile(&s.render(lg::WIN_REAL, &Theme::pill_dark()), region(base(Dock::Bottom)))
}

/// The bounding box of the pixels inside `r` for which `f` holds.
fn bounds_in(img: &Image, r: Rect, f: impl Fn([u8; 4]) -> bool) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in r.y as u32..r.bottom() as u32 {
        for x in r.x as u32..r.right() as u32 {
            if f(img.px(x, y)) {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x1 > 0).then_some((x0, y0, x1, y1))
}

#[test]
fn the_loudness_bars_are_painted_in_the_accent_up_to_the_playhead_and_in_the_track_colour_after_it() {
    gfx(|| {
        let _m = motion_on();
        let red = art([200, 50, 70, 255], [90, 20, 40, 255]);
        let st = base(Dock::Bottom);
        let (_, card) = lg::frames(st, 1.0, 0.0);
        let bar = st.rect_to_real(lg::card_layout(card, pill_ui_core::panel::a::home_player::PlayerStyle::Mirror).bar);
        let accent = |p: [u8; 4]| (p[0] as i32 - 204).abs() < 34 && (p[1] as i32 - 50).abs() < 34 && (p[2] as i32 - 70).abs() < 34 && p[3] > 240;
        let part = |a: f32, b: f32| Rect::new(bar.x + bar.w * a, bar.y, bar.w * (b - a), bar.h);
        // a measured track: tall bars up to the playhead (61 s of 200 s), none of the accent after it
        let mut s = open_sim(json!({}), 32, &red, "#cc3246");
        let img = s.render(lg::WIN_REAL, &Theme::pill_dark());
        let played = img.count(Some(part(0.0, 0.28)), accent);
        assert!(played > 30, "the played part is in the accent: {played}");
        assert_eq!(img.count(Some(part(0.34, 1.0)), accent), 0, "and nothing after the playhead");
        let (_, y0, _, y1) = bounds_in(&img, part(0.0, 0.28), accent).expect("accent bars");
        assert!(y1 - y0 >= 8, "measured bars have a height of their own: {}", y1 - y0);
        // the grey of the rest is drawn (the island's track colour)
        assert!(img.count(Some(part(0.4, 0.95)), |p| p[3] > 240 && (p[0] as i32 - 0x35).abs() < 10 && (p[1] as i32 - 0x35).abs() < 10) > 20);
        // a track not heard before: flat placeholders as thin as the island's track
        let mut s = open_sim(json!({}), 0, &red, "#cc3246");
        let img = s.render(lg::WIN_REAL, &Theme::pill_dark());
        let (_, y0, _, y1) = bounds_in(&img, part(0.0, 0.28), |p| accent(p) || (p[3] > 200 && p[0] > 90)).unwrap_or((0, 0, 0, 0));
        assert!(y1 - y0 <= 5, "unmeasured bars are flat: {}", y1 - y0);
    });
}

/// The same card in the island's three player styles, light and dark, and with a measured / half measured / unmeasured track.
#[test]
fn variant_sheets() {
    let Some(d) = dir() else { return };
    gfx(move || {
        let _m = motion_on();
        let red = art([200, 50, 70, 255], [90, 20, 40, 255]);
        let blue = art([40, 90, 210, 255], [20, 30, 90, 255]);
        let t = [
            open_variant(json!({}), 32, &red, "#cc3246"),
            open_variant(json!({}), 14, &blue, "#2850d2"),
            open_variant(json!({}), 0, &blue, "#2850d2"),
            open_variant(json!({ "theme": "light" }), 32, &red, "#cc3246"),
            open_variant(json!({ "home.player": "compact" }), 32, &red, "#cc3246"),
            open_variant(json!({ "home.player": "fill" }), 32, &blue, "#2850d2"),
            open_variant(json!({ "home.player": "compact", "theme": "light" }), 14, &blue, "#2850d2"),
            open_variant(json!({ "home.player": "fill", "theme": "light" }), 32, &red, "#cc3246"),
        ];
        save(&d, "m14-variants.png", &t);
    });
}
