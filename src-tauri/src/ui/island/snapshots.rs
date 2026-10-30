//! Headless snapshots of the island (WARP, Medeo Sans) in the states the web reference harness renders (`tools/dev/island-web-ref.mjs`,
//! pictures in `target-agent/web-ref`). PNGs land in `target-agent/island-native/<scenario>.png` (+ `.grey.png` on #6b6f76) for a side by side
//! look; the assertions only check what is certain (sizes, transparency, that the right thing is on screen).
use super::fonts::FONTS;
use super::sys::SysClock;
use super::view::IslandView;
use crate::ui::host::testing::Sim;
use crate::ui::view::{Cx, Fakes, Services, View};
use glasskit::geom::Rect;
use glasskit::testkit::{Harness, Image};
use glasskit::theme::Theme;
use glasskit::Color;
use serde_json::{json, Value};
use std::path::PathBuf;

thread_local! {
    static H: Harness = Harness::with_fonts(&FONTS);
}

/// 2026-10-30 12:34:00 at UTC+2, the instant the web harness freezes.
const CLOCK_MS: i64 = 1_793_356_440_000;

struct Empty;
impl View for Empty {
    fn draw(&mut self, _: &glasskit::Canvas, _: &Theme, _: Rect, _: &Cx) {}
}

fn out_dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target-agent").join("island-native");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn base(patch: Value) -> Value {
    let mut s = json!({"size":100,"theme":"dark","clock24":true,"shape":"notch","character":"liquid","pet":"none","pet.side":"right","pet.size":"m",
        "musicPill":"title","net.weather":true,"live.obs":false,"language":"auto"});
    for (k, v) in patch.as_object().into_iter().flatten() {
        s[k] = v.clone();
    }
    s
}

/// The web harness' cover: 64 x 64, r = 40 + 3x, g = 90 + 2y, b = 200 - x - y, as a PNG data URL.
pub fn cover_data_url() -> String {
    use base64::Engine as _;
    let mut raw = Vec::new();
    for y in 0..64u32 {
        for x in 0..64u32 {
            raw.extend_from_slice(&[(40 + 3 * x).min(255) as u8, (90 + 2 * y).min(255) as u8, 200u32.saturating_sub(x + y) as u8, 255]);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 64, 64);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&raw).unwrap();
    }
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out))
}

pub fn media_payload() -> Value {
    json!({"has_session":true,"playing":true,"title":"Midnight City","artist":"M83","album":"Hurry Up, We're Dreaming","session_id":"spotify","app":"Spotify.exe",
        "position_ms":61000,"duration_ms":243000,"thumb_id":1,"accent":"#2a7bc8","thumbnail": cover_data_url()})
}

fn wx() -> Value {
    json!({"temp":18.4,"icon":"partly-day","text":"Partly cloudy"})
}

pub struct Scn {
    pub sim: Sim,
    pub fakes: Fakes,
    pub size: (f32, f32),
}

impl Scn {
    pub fn new(settings: Value, size: (f32, f32), scale: f32, reduced: bool) -> Scn {
        let (svc, fakes): (Services, Fakes) = Services::fake();
        fakes.api.reply("settings_get", Ok(settings));
        fakes.api.reply("weather_get", Ok(json!({"status":"ok","now": wx()})));
        fakes.api.reply("battery_state", Ok(json!({"has_battery":false,"percent":0,"charging":false})));
        let mut sim = Sim::with_services(Box::new(Empty), svc);
        sim.cx.scale = scale;
        sim.cx.reduced_motion = reduced;
        let v = IslandView::with_parts(&mut sim.cx, SysClock::fixed(CLOCK_MS, 120), "en-US");
        sim.view = Box::new(v);
        sim.show();
        let mut s = Scn { sim, fakes, size };
        s.settle(60);
        s
    }

    pub fn settle(&mut self, frames: usize) {
        for _ in 0..frames {
            self.sim.pump();
            if !self.sim.step_frame() {
                break;
            }
        }
        self.sim.pump();
    }

    pub fn emit(&mut self, name: &str, payload: Value) {
        self.fakes.bus.emit(name, payload);
        self.sim.pump();
        self.settle(4);
    }

    pub fn draw(&mut self, bg: Color) -> Image {
        let (w, h) = self.size;
        let scale = self.sim.cx.scale;
        let rect = Rect::new(0.0, 0.0, w, h);
        let th = Theme::pill_dark();
        H.with(|hh| hh.render(w.ceil() as u32, h.ceil() as u32, scale, bg, |cv| self.sim.view.draw(cv, &th, rect, &self.sim.cx)))
    }

    /// Lets the animations run for `ms` of simulated time, redrawing so the view can measure, then draws.
    pub fn shot(&mut self, name: &str, ms: f64) -> Image {
        let frames = (ms * self.sim.hz as f64 / 1000.0) as usize;
        let transparent = Color::rgba(0.0, 0.0, 0.0, 0.0);
        let _ = self.draw(transparent); // first draw measures the live row
        self.settle(frames / 2);
        let _ = self.draw(transparent);
        self.settle(frames);
        let img = self.draw(transparent);
        let grey = self.draw(Color::rgb8(0x6b, 0x6f, 0x76));
        H.with(|h| {
            let _ = img.save_png(h, out_dir().join(format!("{name}.png")));
            let _ = grey.save_png(h, out_dir().join(format!("{name}.grey.png")));
        });
        img
    }
}

fn opaque(img: &Image) -> Option<(u32, u32, u32, u32)> {
    img.bounds(|p| p[3] > 200)
}

#[test]
fn clock_weather() {
    let mut s = Scn::new(base(json!({})), (720.0, 120.0), 1.0, false);
    s.emit("weather:current", wx());
    let img = s.shot("clock_weather", 900.0);
    let (x0, y0, x1, y1) = opaque(&img).expect("the body is drawn");
    assert_eq!(y0, 0, "the notch hangs from the top edge");
    assert!((y1 - y0) >= 28 && (y1 - y0) <= 52, "capsule height incl. the edge: {}", y1 - y0);
    let w = x1 - x0;
    assert!((140..=170).contains(&w), "collapsed clock width {w}");
    assert!(((x0 + x1) as i32 / 2 - 360).abs() <= 2, "centred");
}

#[test]
fn clock_with_date_is_wider() {
    let mut s = Scn::new(base(json!({"showDate": true})), (720.0, 120.0), 1.0, false);
    s.emit("weather:current", wx());
    let img = s.shot("clock_date", 900.0);
    let (x0, _, x1, _) = opaque(&img).unwrap();
    assert!(x1 - x0 > 170, "date widens the capsule: {}", x1 - x0);
}

#[test]
fn light_theme_and_scale_two() {
    let mut s = Scn::new(base(json!({"theme": "light"})), (720.0, 120.0), 1.0, false);
    s.emit("weather:current", wx());
    let img = s.shot("light", 900.0);
    let (x0, y0, x1, y1) = opaque(&img).unwrap();
    let mid = img.at(((x0 + x1) / 2) as f32, (y0 + 4) as f32);
    assert!(mid[0] > 200 && mid[1] > 200, "light body is light: {mid:?}");
    let _ = y1;
    let mut s2 = Scn::new(base(json!({})), (720.0, 120.0), 2.0, false);
    s2.emit("weather:current", wx());
    let img2 = s2.shot("scale2", 900.0);
    assert_eq!((img2.w, img2.h), (1440, 240));
    let (a, _, b, _) = opaque(&img2).unwrap();
    assert!(b - a >= 280, "twice the pixels: {}", b - a);
}

#[test]
fn music_row_and_the_pet_tint() {
    let mut s = Scn::new(base(json!({})), (720.0, 120.0), 1.0, false);
    s.emit("weather:current", wx());
    s.emit("media:changed", media_payload());
    let img = s.shot("music", 900.0);
    let (x0, _, x1, _) = opaque(&img).unwrap();
    assert!(x1 - x0 > 150, "the music row is wider than the clock: {}", x1 - x0);
    // mini mode shows the cover
    let mut m = Scn::new(base(json!({"musicPill": "mini"})), (720.0, 120.0), 1.0, false);
    m.emit("weather:current", wx());
    m.emit("media:changed", media_payload());
    let img = m.shot("music_mini", 1000.0);
    let (x0, _, x1, _) = opaque(&img).unwrap();
    assert!(x1 - x0 > 200, "{}", x1 - x0);
    // reduced motion: same layout, no animation
    let mut r = Scn::new(base(json!({"reduceMotion": true})), (720.0, 120.0), 1.0, true);
    r.emit("media:changed", media_payload());
    r.shot("reduced", 400.0);
    assert!(!r.sim.frame.wants_frame(), "reduced motion settles to idle");
}

#[test]
fn recording_capsule() {
    let mut s = Scn::new(base(json!({})), (720.0, 120.0), 1.0, false);
    s.emit("rec:state", json!({"recording":true,"startedAt": CLOCK_MS - 65_000,"mode":"screen","fps":30}));
    let img = s.shot("recording", 1000.0);
    let (x0, _, x1, _) = opaque(&img).unwrap();
    assert!((240..=290).contains(&(x1 - x0)), "the recording capsule is about 258 wide: {}", x1 - x0);
}

#[test]
fn cards_toast_notification_uninstall() {
    for (name, ev, payload) in [
        ("toast", "capture:done", json!({"kind":"screenshot","path":"C:\\Users\\demo\\Pictures\\Pill\\shot-0042.png","copied":true,"w":1920,"h":1080})),
        ("notification", "live:notification", json!({"app":"Slack","title":"Anna K.","body":"Are we still on for the 3 pm review? I pushed the new build.","id":"n1"})),
        ("uninstall", "uninstall:ready", json!({"name":"Contoso Editor","kind":"app","count":3})),
    ] {
        let mut s = Scn::new(base(json!({})), (720.0, 200.0), 1.0, false);
        s.emit("weather:current", wx());
        s.emit(ev, payload);
        let img = s.shot(name, 1300.0);
        let (x0, y0, x1, y1) = opaque(&img).unwrap_or_else(|| panic!("{name}: nothing drawn"));
        assert!(y1 - y0 >= 44, "{name}: the island grew to the card ({})", y1 - y0);
        assert!(x1 - x0 >= 160 && x1 - x0 <= 500, "{name}: width {}", x1 - x0);
    }
}

#[test]
fn expanded_with_the_placeholder_tab() {
    let mut s = Scn::new(base(json!({})), (720.0, 480.0), 1.0, false);
    s.emit("pill:expand", json!({"tab":"home","via":"test"}));
    let img = s.shot("expanded", 1200.0);
    let (x0, y0, x1, y1) = opaque(&img).unwrap();
    assert!((660..=700).contains(&(x1 - x0)), "expanded body width {}", x1 - x0);
    assert!(y1 - y0 > 250, "expanded body height {}", y1 - y0);
    // the tab strip and header buttons are drawn: light pixels inside the dark body
    assert!(img.count(Some(Rect::new(30.0, 20.0, 660.0, 40.0)), |p| p[0] > 120) > 100);
}

#[test]
fn pets_and_characters() {
    for (name, patch, music) in [
        ("pet_cat", json!({"pet":"cat"}), false),
        ("pet_eyes", json!({"pet":"eyes"}), false),
        ("music_pet", json!({"pet":"cat"}), true),
        ("char_legs", json!({"character":"legs"}), false),
        ("char_tail", json!({"character":"tail"}), false),
    ] {
        let mut s = Scn::new(base(patch), (720.0, 120.0), 1.0, false);
        s.emit("weather:current", wx());
        if music {
            s.emit("media:changed", media_payload());
        }
        let img = s.shot(name, 1400.0);
        let (x0, _, x1, _) = opaque(&img).unwrap_or_else(|| panic!("{name}: nothing drawn"));
        if name.starts_with("pet") || name == "music_pet" {
            assert!(x1 - x0 > 160, "{name}: the capsule makes room for the pet: {}", x1 - x0);
        }
    }
}

#[test]
fn the_capsule_is_sized_from_the_measured_row_like_the_web() {
    // web: the collapsed clock + weather capsule is 150 px wide (clock.rs columns + pill.js sizeLive / colTarget)
    let mut s = Scn::new(base(json!({})), (720.0, 120.0), 1.0, false);
    s.emit("weather:current", wx());
    s.shot("measure", 600.0);
    let v = s.sim.view.as_mut() as *mut dyn View as *mut IslandView;
    // SAFETY: the view of this scenario is an IslandView (set in Scn::new) and the pointer does not outlive the borrow
    let (m, body) = unsafe { ((*v).debug_measured(), (*v).core.shell.view().body) };
    assert!((m.0 - 148.7).abs() < 1.5, "measured row width {m:?}");
    assert_eq!(body.width, 150.0);
    assert_eq!(body.left, 285.0);
}
