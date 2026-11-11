//! The animated forecast icon of the sky card: the same flat pictures as `wxicons` (48 x 48 grid), but alive. A sun turns its rays and
//! breathes, a moon twinkles with its stars, clouds drift and bob, rain streaks and flakes fall out of the cloud, a storm flashes, fog
//! bands drift. Everything is a function of the clock (`sky::cycle` / `wave` / `particle`): no state, no per-frame allocation; the cloud
//! and moon shapes are parsed once into a thread-local cache. Reduced motion draws the still `wxicons` picture instead (the caller does).
use super::sky::{cycle, particle, wave, Fall, Scene};
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::{Pt, Rect};
use glasskit::svgpath::Svg;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CLOUD: &str = super::wxicons::CLOUD;
const MOON: &str = "M20 3a15 15 0 1 0 15 21A12 12 0 0 1 20 3z";
const BOLT: &str = "M27 28l-8 12h5.5l-2.5 8 10-13h-6z";

/// What the icon needs to know about the instant it is drawn.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    /// seconds on the card's clock
    pub t: f64,
    /// 0 light, 1 normal, 2 heavy precipitation
    pub level: u8,
    /// lightning brightness 0..1 (a thunderstorm only)
    pub flash: f32,
}

#[derive(Clone, Copy)]
enum Tone {
    Pale,
    Front,
    Back,
    Grey,
    Dark,
    Fog,
}

impl Tone {
    fn fill(self) -> &'static str {
        match self {
            Tone::Pale => "#e6eaf1",
            Tone::Front => "#dfe3ea",
            Tone::Back => "#8b94a5",
            Tone::Grey => "#a2abbb",
            Tone::Dark => "#7b8496",
            Tone::Fog => "#c9cfda",
        }
    }
}

thread_local! {
    static SVGS: RefCell<HashMap<&'static str, Rc<Svg>>> = RefCell::new(HashMap::new());
}

/// A one-shape picture on the 48 grid, parsed once.
fn pic(key: &'static str, shape: impl FnOnce() -> String) -> Rc<Svg> {
    SVGS.with(|c| c.borrow_mut().entry(key).or_insert_with(|| Rc::new(Svg::parse(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 48 48\">{}</svg>", shape())))).clone())
}

/// The 48 grid mapped onto the icon's rectangle.
#[derive(Clone, Copy)]
struct Grid {
    x: f32,
    y: f32,
    s: f32,
}

impl Grid {
    fn p(&self, x: f32, y: f32) -> Pt {
        (self.x + x * self.s, self.y + y * self.s)
    }
    /// The box of a picture drawn like SVG `translate(tx ty) scale(k)`.
    fn at(&self, tx: f32, ty: f32, k: f32) -> Rect {
        Rect::new(self.x + tx * self.s, self.y + ty * self.s, 48.0 * self.s * k, 48.0 * self.s * k)
    }
}

fn cloud(cv: &Canvas, g: &Grid, tone: Tone, tx: f32, ty: f32, k: f32, alpha: f32) {
    let svg = pic(tone.fill(), || format!("<path d=\"{CLOUD}\" fill=\"{}\"/>", tone.fill()));
    let draw = |cv: &Canvas| svg.draw(cv, g.at(tx, ty, k), glasskit::color::WHITE, &super::super::icons::geom());
    if alpha >= 0.999 {
        draw(cv)
    } else {
        cv.with_opacity(alpha, draw)
    }
}

fn shape(cv: &Canvas, g: &Grid, key: &'static str, d: &'static str, fill: &str, tx: f32, ty: f32, k: f32, alpha: f32) {
    let svg = pic(key, || format!("<path d=\"{d}\" fill=\"{fill}\"/>"));
    cv.with_opacity(alpha, |cv| svg.draw(cv, g.at(tx, ty, k), glasskit::color::WHITE, &super::super::icons::geom()));
}

const SUN: Color = Color::rgb8(0xff, 0xc8, 0x3d);
const GLOW: Color = Color::rgb8(0xff, 0xd3, 0x6b);

/// A sun on the grid: a breathing glow, rays that turn about once in 48 s, the disc.
fn sun(cv: &Canvas, g: &Grid, cx: f32, cy: f32, r: f32, t: f64) {
    let c = g.p(cx, cy);
    let b = 0.5 + 0.5 * wave(t, 6.0, 0.0);
    let gr = r * 2.8 * g.s * (0.94 + 0.08 * b);
    super::super::draw::radial(cv, Rect::new(c.0 - gr, c.1 - gr, 2.0 * gr, 2.0 * gr), gr, c, gr, gr, &[(0.0, GLOW.alpha(0.30 + 0.08 * b)), (0.4, GLOW.alpha(0.12 + 0.04 * b)), (1.0, GLOW.alpha(0.0))]);
    let a0 = cycle(t, 48.0) * 360.0;
    for i in 0..8 {
        let (s, k) = (a0 + i as f32 * 45.0).to_radians().sin_cos();
        let pulse = 0.85 + 0.15 * wave(t, 4.0, i as f32 / 8.0);
        cv.line((c.0 + k * r * 1.5 * g.s, c.1 + s * r * 1.5 * g.s), (c.0 + k * r * (1.95 + 0.06 * b) * g.s, c.1 + s * r * (1.95 + 0.06 * b) * g.s), r / 3.0 * g.s, SUN.alpha(pulse));
    }
    cv.fill_circle(c, r * g.s, SUN);
}

/// A star: a dot with a little cross that twinkles at its own pace.
fn star(cv: &Canvas, g: &Grid, i: usize, x: f32, y: f32, r: f32, t: f64) {
    let tw = 0.5 + 0.5 * wave(t, 2.4 + 0.7 * super::sky::h01(i, 9) as f64 * 3.0, super::sky::h01(i, 8));
    let a = 0.25 + 0.75 * tw * tw;
    let c = g.p(x, y);
    let col = Color::rgb8(0xdf, 0xe4, 0xff).alpha(a);
    cv.fill_circle(c, r * g.s * (0.8 + 0.3 * tw), col);
    let l = r * g.s * (1.6 + 1.6 * tw);
    cv.line((c.0 - l, c.1), (c.0 + l, c.1), 0.9, col.fade(0.55));
    cv.line((c.0, c.1 - l), (c.0, c.1 + l), 0.9, col.fade(0.55));
}

const STARS: [(f32, f32, f32); 5] = [(37.5, 12.3, 1.5), (43.0, 20.5, 1.1), (30.5, 5.0, 1.0), (44.0, 7.0, 0.9), (8.5, 10.0, 1.0)];

fn moon(cv: &Canvas, g: &Grid, tx: f32, ty: f32, k: f32, t: f64, stars: usize) {
    let bob = 0.7 * wave(t, 7.0, 0.0);
    shape(cv, g, "moon", MOON, "#dfe4ff", tx, ty + bob, k, 1.0);
    for (i, &(x, y, r)) in STARS.iter().take(stars).enumerate() {
        star(cv, g, i, x, y, r, t);
    }
}

/// Drops that fall out of the cloud: `n` of them in the strip under it.
fn drops(cv: &Canvas, g: &Grid, kind: Fall, n: usize, rate: f64, t: f64) {
    let (w, h) = (26.0, 17.0);
    for i in 0..n {
        let p = particle(i, t, kind, rate, w, h, -0.06, 1.6);
        let c = g.p(12.0 + p.x, 31.0 + p.y);
        if p.flake {
            cv.fill_circle(c, 1.9 * g.s * (0.8 + 0.4 * super::sky::h01(i, 6)), Color::rgba(1.0, 1.0, 1.0, p.a));
        } else {
            let len = 5.5 * g.s;
            cv.line((c.0 + len * 0.4, c.1 - len), c, 2.3 * g.s, Color::rgb8(0x6f, 0xb3, 0xff).alpha(p.a));
        }
    }
}

/// Draws the animated icon of `scene` into `r` (square).
pub fn draw(cv: &Canvas, scene: Scene, r: Rect, p: &Pose) {
    let g = Grid { x: r.x, y: r.y, s: r.w.min(r.h) / 48.0 };
    let t = p.t;
    match scene {
        Scene::ClearDay => sun(cv, &g, 24.0, 24.0, 9.0, t),
        Scene::ClearNight => moon(cv, &g, 5.0, 6.0, 1.05, t, 5),
        Scene::PartlyDay => {
            sun(cv, &g, 18.0, 17.0, 7.0, t);
            cloud(cv, &g, Tone::Pale, 5.0 + 2.5 * wave(t, 9.0, 0.0), 5.0 + 0.6 * wave(t, 5.0, 0.2), 0.9, 1.0);
        }
        Scene::PartlyNight => {
            moon(cv, &g, 2.0, 1.0, 0.8, t, 3);
            cloud(cv, &g, Tone::Pale, 5.0 + 2.5 * wave(t, 9.0, 0.0), 5.0 + 0.6 * wave(t, 5.0, 0.2), 0.9, 1.0);
        }
        Scene::Cloudy => {
            cloud(cv, &g, Tone::Back, 7.0 + 3.0 * wave(t, 13.0, 0.1), -6.0 + 0.8 * wave(t, 5.0, 0.0), 0.85, 1.0);
            cloud(cv, &g, Tone::Grey, -3.0 + 2.0 * wave(t, 17.0, 0.5), -1.0 + 0.6 * wave(t, 6.5, 0.4), 0.55, 0.8);
            cloud(cv, &g, Tone::Front, 0.0 - 3.0 * wave(t, 9.0, 0.3), 2.0 + 1.0 * wave(t, 6.0, 0.6), 0.95, 1.0);
        }
        Scene::Fog => {
            for (i, (y, x0, x1)) in [(34.5, 9.0, 39.0), (40.5, 14.0, 34.0), (45.5, 18.0, 30.0)].into_iter().enumerate() {
                let dx = (4.0 - i as f32) * wave(t, 7.0 + 2.0 * i as f64, i as f32 * 0.3);
                let a = 0.55 + 0.35 * wave(t, 5.0 + i as f64, i as f32 * 0.2);
                cv.line(g.p(x0 + dx, y), g.p(x1 + dx, y), 3.0 * g.s, Color::rgb8(0xaa, 0xb3, 0xc2).alpha(a));
            }
            cloud(cv, &g, Tone::Fog, 1.5 * wave(t, 18.0, 0.0), -6.0 + 0.5 * wave(t, 6.0, 0.0), 0.95, 1.0);
        }
        Scene::Rain | Scene::Sleet | Scene::Snow | Scene::Thunder => {
            let kind = match scene {
                Scene::Snow => Fall::Flake,
                Scene::Sleet => Fall::Mixed,
                _ => Fall::Rain,
            };
            let n = scene.icon_drops(p.level).map_or(0, |d| d.1);
            drops(cv, &g, kind, n, scene.fall_rate(p.level), t);
            let (dx, dy) = (1.5 * wave(t, 10.0, 0.0), 0.7 * wave(t, 6.0, 0.25));
            let tone = if scene == Scene::Thunder { Tone::Dark } else { Tone::Grey };
            cloud(cv, &g, tone, dx, -6.0 + dy, 0.95, 1.0);
            if scene == Scene::Thunder {
                let f = p.flash.clamp(0.0, 1.0);
                if f > 0.01 {
                    cloud(cv, &g, Tone::Front, dx, -6.0 + dy, 0.95, 0.5 * f);
                    let (c, gr) = (g.p(25.0, 38.0), 17.0 * g.s);
                    let glow = Color::rgb8(0xff, 0xe5, 0x8a);
                    super::super::draw::radial(cv, Rect::new(c.0 - gr, c.1 - gr, 2.0 * gr, 2.0 * gr), gr, c, gr, gr, &[(0.0, glow.alpha(0.55 * f)), (0.5, glow.alpha(0.18 * f)), (1.0, glow.alpha(0.0))]);
                }
                shape(cv, &g, "bolt", BOLT, "#ffd23d", 0.0, 0.0, 1.0, 0.30 + 0.70 * f);
            }
        }
    }
}
