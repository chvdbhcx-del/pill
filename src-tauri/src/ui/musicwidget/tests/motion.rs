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
        let c = st.to_real(lg::card_layout(card).btn[1].center());
        s.send(&mv(c.0, c.1));
        s.send(&down(c.0, c.1));
        t.extend(film(&mut s, &th, win, r, 24.0, 4));
        s.send(&up(c.0, c.1));
        media(&mut s, &f, playing("Fourth Tune With A Fairly Long Title", 4, &gold, "#e6be3c", true, 5_000));
        t.extend(film(&mut s, &th, win, r, 24.0, 8));
        save(&d, "m8-press-pause.png", &t);

        // 5. the scrub: hover the bar, press, drag
        let bar = st.rect_to_real(lg::card_layout(card).bar);
        s.send(&mv(bar.x + bar.w * 0.2, bar.cy()));
        let mut t = film(&mut s, &th, win, r, 24.0, 3);
        s.send(&down(bar.x + bar.w * 0.2, bar.cy()));
        s.send(&mv(bar.x + bar.w * 0.7, bar.cy()));
        t.extend(film(&mut s, &th, win, r, 24.0, 9));
        save(&d, "m9-scrub.png", &t);

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
