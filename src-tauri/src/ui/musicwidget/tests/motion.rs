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
    let (tw, th) = (tiles.iter().map(|t| t.w).max().unwrap_or(0) + 4, tiles.iter().map(|t| t.h).max().unwrap_or(0) + 4);
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
    // a few drawn frames: the cover is decoded by its first draw and fades in over the next ones
    let _ = film(&mut s, &Theme::pill_dark(), lg::WIN_REAL, region(base(Dock::Bottom)), 30.0, 14);
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

/// What one folded capsule is made of: the settings, the track and the strip it stands in (`None` = the plain bottom edge); the rest (`Default`) is
/// a playing track 61 s into 200 s, unmeasured, at 100 %.
#[derive(Default)]
struct Folded<'a> {
    extra: Value,
    title: &'a str,
    artist: &'a str,
    /// the cover (`None` = the track brings none)
    url: Option<&'a str>,
    accent: &'a str,
    bar: Option<f32>,
    /// the DPI scale (0 = 1), the position (ms; 0 = 61 s), how many of the 32 loudness bars are known, paused
    scale: f32,
    pos: u64,
    known: usize,
    paused: bool,
}

/// A widget showing `f` folded, settled and drawn a few times so that the cover is decoded; the stand that places it.
fn folded_sim(f: &Folded) -> (Sim, lg::Stand) {
    let (svc, fb) = Services::fake();
    fb.api.reply("settings_get", Ok(cfg(f.extra.clone())));
    let mut s = Sim::with_services(Box::new(Blank), svc);
    let scale = if f.scale > 0.0 { f.scale } else { 1.0 };
    s.cx.scale = scale;
    // a loudness profile as if the track had been heard (the memory is keyed by title, artist and length)
    let seed = |v: &mut MusicView| {
        if f.known > 0 {
            let bytes: [u8; 32] = std::array::from_fn(|i| if i < f.known { (60.0 + 190.0 * (0.5 + 0.5 * ((i as f32 * 0.7).sin() * (i as f32 * 0.23).cos()))) as u8 } else { 0 });
            v.seed_loudness(crate::ui::musicwidget::wave::key_of(f.title, f.artist, 200_000), bytes);
        }
    };
    let st = match f.bar {
        Some(b) => {
            let mut v = MusicView::new(&mut s.cx, Dock::Taskbar);
            seed(&mut v);
            s.view = Box::new(v.with_slot(b, Align::Left));
            bar_st(Align::Left, b, scale)
        }
        None => {
            let mut v = MusicView::new(&mut s.cx, Dock::Bottom);
            seed(&mut v);
            s.view = Box::new(v);
            lg::Stand { scale, ..base(Dock::Bottom) }
        }
    };
    s.set_refresh_hz(60.0);
    s.pump();
    let mut m = json!({ "has_session": true, "playing": !f.paused, "title": f.title, "artist": f.artist, "album": "", "app": "Spotify.exe", "position_ms": if f.pos > 0 { f.pos } else { 61_000 }, "duration_ms": 200_000, "thumb_id": if f.url.is_some() { 1 } else { 0 }, "accent": f.accent });
    if let Some(u) = f.url {
        m["thumbnail"] = json!(u);
    }
    media(&mut s, &fb, m);
    s.show();
    s.run(4000);
    cover_worker(&mut s);
    let win = if f.bar.is_some() { lg::WIN_BAR_REAL } else { lg::WIN_REAL };
    let _ = film(&mut s, &Theme::pill_dark(), win, Rect::new(0.0, 0.0, 10.0, 10.0), 30.0, 14);
    (s, st)
}

/// The folded capsule alone (its body and a margin), over the backdrop.
fn folded_tile(f: &Folded) -> Image {
    let (mut s, st) = folded_sim(f);
    let win = if f.bar.is_some() { lg::WIN_BAR_REAL } else { lg::WIN_REAL };
    let img = s.render(win, &Theme::pill_dark());
    let b = st.rect_to_real(lg::body(st, 0.0, 0.0));
    tile(&img, Rect::new((b.x - 10.0).max(0.0), (b.y - 10.0).max(0.0), lg::CAP_W + 20.0, b.h + 20.0))
}

/// The folded capsule in the island's player modes and themes, with and without a cover, with long and short names and in the heights of a
/// taskbar, and the way it opens into the card.
#[test]
fn capsule_sheets() {
    let Some(d) = dir() else { return };
    gfx(move || {
        let _m = motion_on();
        let red = art([200, 50, 70, 255], [90, 20, 40, 255]);
        let blue = art([40, 90, 210, 255], [20, 30, 90, 255]);
        let gold = art([230, 190, 60, 255], [120, 70, 20, 255]);
        let f = |extra: Value, title: &'static str, artist: &'static str, url: Option<&str>, accent: &'static str, bar: Option<f32>| Folded { extra, title, artist, url: url.map(|u| Box::leak(u.to_string().into_boxed_str()) as &str), accent, bar, ..Default::default() };
        // 1. the three modes, dark and light
        let mut t = Vec::new();
        for theme in ["dark", "light"] {
            for mode in ["mirror", "compact", "fill"] {
                t.push(folded_tile(&f(json!({ "theme": theme, "home.player": mode }), "Invented Song", "Invented Band", Some(&red), "#cc3246", None)));
            }
        }
        save(&d, "c1-modes.png", &t);
        // 2. other covers, no cover, long and short names, a Cyrillic one, the capitals of the owner's screenshot
        let t = [
            folded_tile(&f(json!({}), "Invented Song", "Invented Band", Some(&blue), "#2850d2", None)),
            folded_tile(&f(json!({}), "Invented Song", "Invented Band", Some(&gold), "#e6be3c", None)),
            folded_tile(&f(json!({}), "Invented Song", "Invented Band", None, "#cc3246", None)),
            folded_tile(&f(json!({ "theme": "light" }), "Invented Song", "Invented Band", None, "#cc3246", None)),
            folded_tile(&f(json!({}), "PER 3 DOSE", "ZAPRAVKA", Some(&red), "#cc3246", None)),
            folded_tile(&f(json!({}), "A Title That Is Much Too Long To Fit In The Folded Capsule At All", "An Artist With A Long Name Too", Some(&blue), "#2850d2", None)),
            folded_tile(&f(json!({}), "Short", "", Some(&gold), "#e6be3c", None)),
            folded_tile(&f(json!({}), "Очень длинное название трека которое не помещается", "Исполнитель", Some(&red), "#cc3246", None)),
        ];
        save(&d, "c2-covers-and-names.png", &t);
        // 3. the heights of a taskbar: 56 (no bar), 52, 48, 40, 32
        let t: Vec<Image> = [None, Some(56.0), Some(48.0), Some(44.0), Some(40.0), Some(32.0)]
            .iter()
            .map(|b| folded_tile(&f(json!({}), "Invented Song", "Invented Band", Some(&red), "#cc3246", *b)))
            .collect();
        save(&d, "c3-bar-heights.png", &t);
        let t: Vec<Image> = [Some(48.0), Some(40.0), Some(32.0)]
            .iter()
            .map(|b| folded_tile(&f(json!({ "theme": "light" }), "A Title That Is Much Too Long To Fit", "An Artist", Some(&blue), "#2850d2", *b)))
            .collect();
        save(&d, "c4-bar-light-long.png", &t);
        // 4. folded -> open -> folded, in each mode (a frame every 24 ms)
        let th = Theme::pill_dark();
        let r = region(base(Dock::Bottom));
        for (name, mode) in [("mirror", "mirror"), ("compact", "compact"), ("fill", "fill")] {
            let (mut s, _) = folded_sim(&f(json!({ "home.player": mode }), "Invented Song", "Invented Band", Some(&red), "#cc3246", None));
            s.send_proximity(&ProximityEvent::Hover);
            let mut t = film(&mut s, &th, lg::WIN_REAL, r, 24.0, 16);
            s.send_proximity(&ProximityEvent::Leave);
            t.extend(film(&mut s, &th, lg::WIN_REAL, r, 24.0, 16));
            save(&d, &format!("c5-morph-{name}.png"), &t);
        }
        // 5. the same in the light theme and in a bar
        let (mut s, _) = folded_sim(&f(json!({ "theme": "light" }), "Invented Song", "Invented Band", Some(&blue), "#2850d2", None));
        s.send_proximity(&ProximityEvent::Hover);
        save(&d, "c6-morph-light.png", &film(&mut s, &th, lg::WIN_REAL, r, 24.0, 16));
    });
}

/// `img` blown up `k` times (nearest), for a person to judge pixels.
fn zoom(img: &Image, k: u32) -> Image {
    let mut bgra = vec![0u8; (img.w * k * img.h * k * 4) as usize];
    for y in 0..img.h * k {
        for x in 0..img.w * k {
            let (s, o) = (img.px(x / k, y / k), ((y * img.w * k + x) * 4) as usize);
            bgra[o..o + 4].copy_from_slice(&[s[2], s[1], s[0], 255]);
        }
    }
    Image { w: img.w * k, h: img.h * k, scale: 1.0, bgra }
}

/// The capsule in the taskbar strip at the sizes of the owner's screens, 3x for a person to judge pixels: a dark teal cover, a Russian title, with
/// the loudness measured and not (`strip.png`), and the body's colour over several covers in three heights (`body-<h>.png`).
/// `PILL_SNAP_DIR=<dir> cargo test --locked --bin pill strip_sheets`.
#[test]
fn strip_sheets() {
    let Some(d) = dir() else { return };
    gfx(move || {
        let _m = motion_on();
        let teal = art([70, 100, 110, 255], [20, 34, 44, 255]);
        let mut t = Vec::new();
        for bar in [Some(40.0f32), Some(48.0), None] {
            for known in [0usize, 32] {
                let f = Folded { extra: json!({}), title: "Вечно молодой", artist: "Смысловые Галлюцинации", url: Some(&teal), accent: "#3a7080", bar, known, ..Default::default() };
                t.push(zoom(&folded_tile(&f), 3));
            }
        }
        crate::ui::host::testing::save_png(&sheet(&t, 2), d.join("strip.png"));
        // the body over a dark teal, a bright red, a near-black, a near-white, a grey and a saturated blue cover
        let covers: [(&str, [u8; 4], [u8; 4], &str); 6] = [
            ("teal", [60, 90, 100, 255], [30, 50, 62, 255], "#3a7080"),
            ("red", [220, 40, 60, 255], [150, 20, 40, 255], "#dc283c"),
            ("black", [14, 14, 16, 255], [6, 6, 8, 255], "#101012"),
            ("white", [244, 242, 238, 255], [226, 224, 220, 255], "#f4f2ee"),
            ("grey", [120, 120, 124, 255], [96, 96, 100, 255], "#78787c"),
            ("blue", [40, 90, 220, 255], [24, 50, 150, 255], "#285adc"),
        ];
        for (name, bar) in [("56", None), ("40", Some(48.0f32)), ("32", Some(40.0))] {
            let t: Vec<Image> = covers
                .iter()
                .map(|(_, a, b, acc)| {
                    let u = art(*a, *b);
                    zoom(&folded_tile(&Folded { extra: json!({}), title: "Вечно молодой", artist: "Смысловые Галлюцинации", url: Some(&u), accent: acc, bar, known: 32, ..Default::default() }), 2)
                })
                .collect();
            crate::ui::host::testing::save_png(&sheet(&t, 2), d.join(format!("body-{name}.png")));
        }
    });
}

/// The owner's own case at the owner's screen (100%: the strip 32 px tall), 6x for a person to judge pixels: a yellow cover (the screenshot) and a
/// teal, a red, a near-black, a near-white and a grey one, «Утро» / «Дайте танк (!)» at 80 %, at the three heights (`u-<h>.png`).
/// `PILL_SNAP_DIR=<dir> cargo test --locked --bin pill strip_user_case`.
#[test]
fn strip_user_case() {
    let Some(d) = dir() else { return };
    gfx(move || {
        let _m = motion_on();
        let covers: [(&str, [u8; 4], [u8; 4], &str); 6] = [
            ("yellow", [250, 215, 60, 255], [240, 190, 40, 255], "#fad73c"),
            ("teal", [70, 100, 110, 255], [20, 34, 44, 255], "#3a7080"),
            ("red", [200, 50, 70, 255], [90, 20, 40, 255], "#cc3246"),
            ("black", [14, 14, 16, 255], [6, 6, 8, 255], "#0e0e10"),
            ("white", [244, 242, 238, 255], [226, 224, 220, 255], "#f4f2ee"),
            ("grey", [120, 120, 124, 255], [96, 96, 100, 255], "#78787c"),
        ];
        for (name, bar) in [("56", None), ("40", Some(48.0f32)), ("32", Some(40.0))] {
            let t: Vec<Image> = covers
                .iter()
                .map(|(_, a, b, acc)| {
                    let u = art(*a, *b);
                    zoom(&folded_tile(&Folded { extra: json!({}), title: "Утро", artist: "Дайте танк (!)", url: Some(&u), accent: acc, bar, pos: 160_000, known: 32, ..Default::default() }), 3)
                })
                .collect();
            crate::ui::host::testing::save_png(&sheet(&t, 2), d.join(format!("u-{name}.png")));
        }
    });
}

/// A track nothing is measured of has a calm thin line for its progress in the strip, not a row of dots; once it is measured the bars are bars.
#[test]
fn an_unmeasured_track_shows_a_thin_line_and_a_measured_one_shows_bars_in_the_strip() {
    gfx(|| {
        let _m = motion_on();
        let teal = art([70, 100, 110, 255], [20, 34, 44, 255]);
        let height_of_light = |known: usize, bar: Option<f32>| {
            let (mut s, st) = folded_sim(&Folded { extra: json!({}), title: "Invented Song", artist: "Invented Band", url: Some(&teal), accent: "#3a7080", bar, known, ..Default::default() });
            let win = if bar.is_some() { lg::WIN_BAR_REAL } else { lg::WIN_REAL };
            let img = s.render(win, &Theme::pill_dark());
            let l = lg::capsule_layout_in(st, lg::body(st, 0.0, 0.0));
            let r = st.rect_to_real(l.bar);
            let light = |p: [u8; 4]| p[3] > 200 && p[0] as u32 + p[1] as u32 + p[2] as u32 > 3 * 90;
            bounds_in(&img, r, light).map(|b| (b.3 - b.1, b.2 - b.0))
        };
        // a strip draws the line for the progress, measured or not; the full height draws bars once they are measured
        let (h, w) = height_of_light(0, Some(40.0)).expect("the played part of the line is drawn");
        assert!(h <= 3 && w >= 4, "a thin line: {h} px tall, {w} wide");
        let (h, _) = height_of_light(32, Some(40.0)).expect("the line is drawn");
        assert!(h <= 3, "a strip keeps its line even when the track is measured: {h} px tall");
        let (h, _) = height_of_light(32, None).expect("the bars are drawn");
        assert!(h >= 6, "bars, not a line: {h} px tall");
    });
}

#[test]
fn the_folded_capsule_is_tinted_by_the_cover_and_flat_in_high_contrast() {
    gfx(|| {
        let _m = motion_on();
        let red = art([200, 50, 70, 255], [90, 20, 40, 255]);
        let f = |extra: Value| Folded { extra, title: "Invented Song", artist: "Invented Band", url: Some(Box::leak(red.clone().into_boxed_str())), accent: "#cc3246", ..Default::default() };
        let reddish = |p: [u8; 4]| p[3] > 200 && p[0] as i32 > p[1] as i32 + 12 && p[0] as i32 > p[2] as i32 + 6;
        let count = |extra: Value| {
            let (mut s, st) = folded_sim(&f(extra));
            let img = s.render(lg::WIN_REAL, &Theme::pill_dark());
            let bb = lg::body(st, 0.0, 0.0);
            let l = lg::capsule_layout_in(st, bb);
            // the body to the right of the cover, over the text and the buttons (nothing of the cover or the waveform's accent)
            let r = st.rect_to_real(Rect::new(l.text.x, bb.y + 3.0, l.btn[2].right() - l.text.x, 30.0));
            img.count(Some(r), reddish)
        };
        let tinted = count(json!({}));
        let flat = count(json!({ "a11y.contrast": "on" }));
        assert!(tinted > 300, "the cover's colour is in the body: {tinted}");
        assert_eq!(flat, 0, "high contrast: the system colours, flat and opaque, no tint and no reflections");
        // the same for the light theme: still the card's body, not a plain one
        assert!(count(json!({ "theme": "light" })) > 100);
    });
}

#[test]
fn the_tinted_body_grows_with_the_shape_and_nothing_pops_while_the_capsule_opens_and_closes() {
    gfx(|| {
        let _m = motion_on();
        let red = art([200, 50, 70, 255], [90, 20, 40, 255]);
        let (mut s, st) = folded_sim(&Folded { extra: json!({}), title: "Invented Song", artist: "Invented Band", url: Some(&red), accent: "#cc3246", ..Default::default() });
        let th = Theme::pill_dark();
        let bb = lg::body(st, 0.0, 0.0);
        // a spot in the lower part of the body, clear of the cover and the text, that stays inside it all the way up
        let at = st.to_real((bb.x + 150.0, bb.bottom() - 2.0));
        let mut prev: Option<[u8; 4]> = None;
        let mut worst = 0i32;
        let mut step = |s: &mut Sim| {
            let img = s.render(lg::WIN_REAL, &th);
            let p = img.px(at.0 as u32, at.1 as u32);
            if let Some(q) = prev {
                worst = worst.max((0..4).map(|i| (p[i] as i32 - q[i] as i32).abs()).max().unwrap());
            }
            prev = Some(p);
            let until = s.now_ms + 16.0;
            while s.now_ms + 1000.0 / s.hz as f64 <= until && s.step_frame() {}
            s.now_ms = until;
            s.cx.now_ms = until;
        };
        s.send_proximity(&ProximityEvent::Hover);
        for _ in 0..40 {
            step(&mut s);
        }
        s.send_proximity(&ProximityEvent::Leave);
        for _ in 0..40 {
            step(&mut s);
        }
        assert!(worst < 48, "the body's colour at a fixed spot changes by {worst} in one 16 ms frame: a pop");
        assert!(prev.is_some_and(|p| p[3] == 255), "the spot is inside the body");
    });
}

/// The folded capsule alone at its DPI scale, over the backdrop, zoomed to about twice the size on a 100 % screen (nearest, for judging pixels).
fn zoom_tile(f: &Folded) -> Image {
    let (mut s, st) = folded_sim(f);
    let scale = if f.scale > 0.0 { f.scale } else { 1.0 };
    let win = if f.bar.is_some() { lg::WIN_BAR_REAL } else { lg::WIN_REAL };
    let th = Theme::pill_dark();
    let img = s.render(win, &th);
    let b = st.rect_to_real(lg::body(st, 0.0, 0.0));
    let (x0, y0) = (((b.x - 8.0) * scale).max(0.0) as u32, ((b.y - 8.0) * scale).max(0.0) as u32);
    let (w, h) = (((lg::CAP_W + 16.0) * scale) as u32, ((b.h + 16.0) * scale) as u32);
    let z = (2.0 / scale).round().max(1.0) as u32;
    let mut bgra = vec![255u8; (w * z * h * z * 4) as usize];
    for y in 0..h * z {
        for x in 0..w * z {
            let p = img.px((x0 + x / z).min(img.w - 1), (y0 + y / z).min(img.h - 1));
            let bg = [70u32 + (y / z * 40 / h), 78 + (y / z * 40 / h), 100 + (y / z * 30 / h)];
            let a = p[3] as u32;
            let i = ((y * w * z + x) * 4) as usize;
            for c in 0..3 {
                bgra[i + 2 - c] = (p[c] as u32 + bg[c] * (255 - a) / 255).min(255) as u8;
            }
            bgra[i + 3] = 255;
        }
    }
    Image { w: w * z, h: h * z, scale: 1.0, bgra }
}

/// Zoomed folded capsules (`PILL_SNAP_DIR=<dir>`): the covers (bright, dark, colourful, none) in both themes, the states (progress, measured or not,
/// paused, long and missing text), the modes, high contrast and the scales.
#[test]
fn capsule_zoom_sheets() {
    let Some(d) = dir() else { return };
    gfx(move || {
        let _m = motion_on();
        let cover = |a: [u8; 4], b: [u8; 4]| -> &'static str { Box::leak(art(a, b).into_boxed_str()) };
        let yellow = cover([250, 222, 40, 255], [232, 176, 14, 255]);
        let white = cover([252, 252, 252, 255], [226, 226, 226, 255]);
        let black = cover([16, 16, 18, 255], [0, 0, 0, 255]);
        let green = cover([70, 200, 120, 255], [20, 90, 56, 255]);
        let blue = cover([40, 90, 210, 255], [20, 30, 90, 255]);
        let red = cover([200, 50, 70, 255], [90, 20, 40, 255]);
        let f = |extra: Value, url: Option<&'static str>, accent: &'static str| Folded { extra, title: "Midnight City", artist: "M83", url, accent, known: 32, pos: 80_000, ..Default::default() };
        let covers = [(Some(yellow), "#fade28"), (Some(white), "#fcfcfc"), (Some(black), "#101012"), (Some(green), "#46c878"), (Some(blue), "#2850d2"), (Some(red), "#cc3246"), (None, "#cc3246")];
        let mut t = Vec::new();
        for (u, a) in covers {
            for theme in ["dark", "light"] {
                t.push(zoom_tile(&f(json!({ "theme": theme }), u, a)));
            }
        }
        crate::ui::host::testing::save_png(&sheet(&t, 2), d.join("z1-covers.png"));
        // states: not measured / half / all, played 0 / 40 / 100 %, paused, a long title, no artist, Cyrillic, no cover and paused
        let g = |known: usize, pos: u64, paused: bool, title: &'static str, artist: &'static str| Folded { known, pos, paused, title, artist, url: Some(blue), accent: "#2850d2", ..Default::default() };
        let t = [
            zoom_tile(&g(0, 1_000, false, "Midnight City", "M83")),
            zoom_tile(&g(16, 80_000, false, "Midnight City", "M83")),
            zoom_tile(&g(32, 1_000, false, "Midnight City", "M83")),
            zoom_tile(&g(32, 80_000, false, "Midnight City", "M83")),
            zoom_tile(&g(32, 199_500, false, "Midnight City", "M83")),
            zoom_tile(&g(32, 80_000, true, "Midnight City", "M83")),
            zoom_tile(&g(32, 80_000, false, "A Title That Is Much Too Long To Fit In The Folded Capsule At All", "An Artist")),
            zoom_tile(&g(32, 80_000, false, "Short", "")),
            zoom_tile(&g(32, 80_000, false, "Очень длинное название трека", "Исполнитель")),
            zoom_tile(&g(32, 80_000, false, "Song", "Short Band")),
        ];
        crate::ui::host::testing::save_png(&sheet(&t, 2), d.join("z2-states.png"));
        // modes and high contrast, dark and light
        let t: Vec<Image> = [
            json!({ "home.player": "mirror" }),
            json!({ "home.player": "compact" }),
            json!({ "home.player": "fill" }),
            json!({ "theme": "light", "home.player": "mirror" }),
            json!({ "theme": "light", "home.player": "fill" }),
            json!({ "a11y.contrast": "on" }),
        ]
        .into_iter()
        .map(|e| zoom_tile(&f(e, Some(green), "#46c878")))
        .collect();
        crate::ui::host::testing::save_png(&sheet(&t, 2), d.join("z3-modes.png"));
        // scales and the heights of a taskbar
        let mut t: Vec<Image> = [1.0f32, 1.25, 1.5, 2.0].iter().map(|&sc| zoom_tile(&Folded { scale: sc, ..f(json!({}), Some(yellow), "#fade28") })).collect();
        for b in [56.0f32, 48.0, 44.0, 40.0, 32.0] {
            t.push(zoom_tile(&Folded { bar: Some(b), ..f(json!({}), Some(blue), "#2850d2") }));
        }
        t.push(zoom_tile(&Folded { bar: Some(48.0), scale: 1.5, ..f(json!({ "theme": "light" }), Some(green), "#46c878") }));
        crate::ui::host::testing::save_png(&sheet(&t, 2), d.join("z4-scales.png"));
    });
}
