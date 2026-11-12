//! The weather tiles under the daily list (wind, humidity, pressure, sun, precipitation, UV index) and the one-sentence summary above them.
//! The data is `pill_ui_core::panel::a::weather_tiles`; this file gives it motion and pixels: the tiles rise in one after another when they
//! scroll into view (the numbers count up while they appear), a new reading eases from the old one, the compass turns the short way round.
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::a::draw;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::weather_tiles::{fmt_num, uv_level, Kind, WxTile, UV_COLORS};

pub const COLS: usize = 3;
pub const GAP: f32 = 10.0;
pub const TILE_H: f32 = 108.0;
pub const SUMMARY_H: f32 = 64.0;
/// Space between the daily list and the summary, the summary and the grid.
pub const SECTION_GAP: f32 = 12.0;

const SKY: Color = Color::rgb8(0x7f, 0xb2, 0xff);
const SUN: Color = Color::rgb8(0xff, 0xcc, 0x4d);

/// The colour of a UV category (green, yellow, orange, red, violet); the light theme gets deeper tones so the number stays readable on the card.
pub fn uv_color(level: usize, dark: bool) -> Color {
    let (d, l) = [((0x6f, 0xcf, 0x7a), (0x2e, 0x8b, 0x3d)), ((0xf2, 0xd6, 0x4b), (0x9a, 0x7a, 0x00)), ((0xf5, 0x9e, 0x42), (0xc2, 0x62, 0x00)), ((0xe5, 0x4d, 0x4d), (0xc6, 0x28, 0x28)), ((0xb2, 0x84, 0xf0), (0x7b, 0x3f, 0xc9))][level.min(4)];
    let (r, g, b) = if dark { d } else { l };
    Color::rgb8(r, g, b)
}

/// Where the summary card and the grid of tiles sit below a daily list that ends at `days_bottom` (all relative to the right column's top),
/// and how tall the whole column is for `n` tiles.
pub fn layout(days_bottom: f32, n: usize) -> (f32, f32, f32) {
    let sum_y = days_bottom + SECTION_GAP;
    let grid_y = sum_y + SUMMARY_H + SECTION_GAP;
    let rows = n.div_ceil(COLS).max(1);
    (sum_y, grid_y, grid_y + rows as f32 * TILE_H + (rows - 1) as f32 * GAP + 2.0)
}

struct One {
    kind: Kind,
    /// the number on show, eased between readings
    val: Flow,
    /// the little picture: gauge, bar, drop level, sun position
    frac: Flow,
    /// the compass arrow, in unwrapped degrees so that it turns the short way
    toward: Flow,
}

pub struct Tiles {
    one: Vec<One>,
    /// 0 -> 1 when the tiles come into view: drives the staggered rise
    reveal: Flow,
}

/// `to` as an angle reachable from `from` by the shortest turn.
fn unwrap_to(from: f32, to: f32) -> f32 {
    let d = (to - from).rem_euclid(360.0);
    from + if d > 180.0 { d - 360.0 } else { d }
}

impl Tiles {
    pub fn new() -> Tiles {
        Tiles { one: vec![], reveal: Flow::new(0.0, Feel::FLOW.within(900.0)) }
    }

    /// A new forecast: same tiles as before ease to the new readings, a different set starts from its own values.
    pub fn sync(&mut self, tiles: &[WxTile]) {
        let same = self.one.len() == tiles.len() && self.one.iter().zip(tiles).all(|(o, t)| o.kind == t.kind);
        if !same {
            self.one = tiles
                .iter()
                .map(|t| One { kind: t.kind, val: Flow::px(t.num.unwrap_or(0.0) as f32, Feel::DRAG), frac: Flow::new(t.frac, Feel::FLOW), toward: Flow::new(t.toward, Feel::FLOW) })
                .collect();
            return;
        }
        for (o, t) in self.one.iter_mut().zip(tiles) {
            o.val.to(t.num.unwrap_or(0.0) as f32);
            o.frac.to(t.frac);
            let cur = o.toward.target();
            o.toward.to(unwrap_to(cur, t.toward));
        }
    }

    /// The grid is (not) in view; it rises in once and stays.
    pub fn show(&mut self, on: bool) {
        if on && self.reveal.target() < 0.5 {
            self.reveal.to(1.0);
        }
    }

    /// Back to hidden (the tab was left): the next view of the grid plays the entrance again.
    pub fn rewind(&mut self) {
        self.reveal.jump(0.0);
        self.reveal.to(0.0);
    }

    pub fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = false;
        for f in std::iter::once(&mut self.reveal).chain(self.one.iter_mut().flat_map(|o| [&mut o.val, &mut o.frac, &mut o.toward])) {
            f.step(dt_ms);
            busy |= f.moving();
        }
        busy
    }

    /// How far tile `i` has come in, 0..1 (a little past 1 while it settles).
    fn rise(&self, i: usize) -> f32 {
        // tile i starts when the entrance is i tenths along and takes half of it; at rest every tile is exactly 1, the springy
        // overshoot of the entrance lifts the early ones a little past it
        let r = self.reveal.at();
        let x = (r - 0.1 * i as f32) / 0.5;
        if x <= 1.0 {
            x.max(0.0)
        } else {
            1.0 + (r - 1.0).max(0.0) * 0.5
        }
    }

    /// Whether the grid has been asked to come in.
    #[cfg(test)]
    pub fn revealing(&self) -> bool {
        self.reveal.target() > 0.5
    }

    /// The number tile `i` shows right now, before the count-up of its entrance.
    #[cfg(test)]
    pub fn one_val(&self, i: usize) -> f32 {
        self.one[i].val.at()
    }

    pub fn settled(&self) -> bool {
        !self.reveal.moving() && self.reveal.target() > 0.5
    }

    /// The summary sentence in a card at `r`.
    pub fn paint_summary(&self, cv: &Canvas, th: &Theme, r: Rect, text: &str) {
        let p = self.rise(0).clamp(0.0, 1.0);
        cv.with_opacity(p, |cv| {
            let r = r.offset(0.0, (1.0 - p) * 10.0);
            cv.fill_rr(r, 16.0, th.card);
            cv.text(Rect::new(r.x + 16.0, r.y + 10.0, r.w - 32.0, r.h - 20.0), text, Style::new(12.0, 400).wrap(), th.text2);
        });
    }

    /// The grid of tiles inside `grid` (its width is split in three).
    pub fn paint(&self, cv: &Canvas, th: &Theme, grid: Rect, tiles: &[WxTile], ru: bool) {
        let w = (grid.w - (COLS - 1) as f32 * GAP) / COLS as f32;
        for (i, (t, o)) in tiles.iter().zip(&self.one).enumerate() {
            let r = Rect::new(grid.x + (i % COLS) as f32 * (w + GAP), grid.y + (i / COLS) as f32 * (TILE_H + GAP), w, TILE_H);
            let p = self.rise(i);
            let a = p.clamp(0.0, 1.0);
            if a <= 0.004 {
                continue;
            }
            cv.with_opacity(a, |cv| {
                let r = r.offset(0.0, (1.0 - p) * 16.0);
                self.tile(cv, th, r, t, o, p.clamp(0.0, 1.0), ru);
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tile(&self, cv: &Canvas, th: &Theme, r: Rect, t: &WxTile, o: &One, p: f32, ru: bool) {
        cv.fill_rr(r, 16.0, th.card);
        let (x, y, iw) = (r.x + 12.0, r.y, r.w - 24.0);
        cv.text(Rect::new(x, y + 10.0, iw - if t.kind == Kind::Wind { 34.0 } else { 0.0 }, 14.0), &t.title, Style::new(11.0, 600).ellipsis(), th.dim);
        // the number counts up while the tile comes in, then follows the readings
        let value = match t.num {
            Some(_) => format!("{}{}{}", t.lead, fmt_num((o.val.at() * p) as f64, t.decimals, ru), t.unit),
            None => t.text.clone(),
        };
        let vc = match (t.kind, t.num) {
            (Kind::Uv, Some(u)) => uv_color(uv_level(u), th.dark),
            _ => th.text,
        };
        cv.text(Rect::new(x, y + 26.0, iw, 28.0), &value, Style::new(21.0, 600).mono().ellipsis(), vc);
        cv.text(Rect::new(x, y + 76.0, iw, 28.0), &t.sub, Style::new(11.0, 400).wrap(), th.dim);
        let track = if th.dark { Color::rgba(1.0, 1.0, 1.0, 0.12) } else { Color::rgba(0.0, 0.0, 0.0, 0.1) };
        let bar = Rect::new(x, y + 60.0, iw, 6.0);
        let f = o.frac.at().clamp(0.0, 1.0);
        match t.kind {
            Kind::Wind => {
                let c = (r.right() - 12.0 - 14.0, y + 10.0 + 14.0);
                cv.stroke_circle(c, 14.0, 1.2, th.faint);
                cv.line((c.0, c.1 - 14.0), (c.0, c.1 - 10.5), 1.4, th.dim);
                let a = o.toward.at().to_radians();
                let (s, k) = a.sin_cos();
                let at = |d: f32, side: f32| (c.0 + d * s + side * k, c.1 - d * k + side * s);
                cv.line(at(-6.0, 0.0), at(9.0, 0.0), 1.8, th.text);
                cv.line(at(9.0, 0.0), at(3.5, -3.5), 1.8, th.text);
                cv.line(at(9.0, 0.0), at(3.5, 3.5), 1.8, th.text);
                // the strength under the number: a bar like the others
                cv.fill_rr(bar, 3.0, track);
                cv.fill_rr(Rect::new(bar.x, bar.y, (bar.w * f).max(6.0), bar.h), 3.0, SKY);
            }
            Kind::Humidity | Kind::Precip => {
                cv.fill_rr(bar, 3.0, track);
                cv.fill_rr(Rect::new(bar.x, bar.y, (bar.w * f).max(6.0), bar.h), 3.0, SKY);
            }
            Kind::Pressure | Kind::Uv => {
                if t.kind == Kind::Uv {
                    let at = [0.0, 0.3, 0.55, 0.8, 1.0];
                    let stops: Vec<(f32, Color)> = UV_COLORS.iter().zip(at).map(|(c, p)| (p, Color::rgb8(c[0], c[1], c[2]))).collect();
                    draw::linear(cv, bar, 3.0, (bar.x, bar.cy()), (bar.right(), bar.cy()), &stops);
                } else {
                    cv.fill_rr(bar, 3.0, track);
                }
                let c = (bar.x + 4.0 + f * (bar.w - 8.0), bar.cy());
                cv.fill_circle(c, 5.5, Color::rgba(0.0, 0.0, 0.0, 0.3));
                cv.fill_circle(c, 4.5, Color::rgb8(255, 255, 255));
            }
            Kind::Sun => {
                // the day's arc with the sun on it; the sun sits on the horizon before sunrise and after sunset
                let base = y + 68.0;
                let at = |u: f32| (bar.x + u * bar.w, base - 14.0 * (std::f32::consts::PI * u).sin());
                cv.line((bar.x, base), (bar.right(), base), 1.0, th.faint);
                let mut prev = at(0.0);
                for k in 1..=24 {
                    let q = at(k as f32 / 24.0);
                    cv.line(prev, q, 1.4, th.dim);
                    prev = q;
                }
                let d = at(f);
                cv.fill_circle(d, 4.8, SUN.fade(0.35));
                cv.fill_circle(d, 3.4, SUN);
            }
        }
    }
}

impl super::WeatherView {
    /// Per frame: keep the scroll metrics, let the grid rise in when it comes into view, step the springs. True while anything moves.
    pub(super) fn step_tiles(&mut self, dt_ms: f64) -> bool {
        let right = self.right_rect();
        self.scroll.set_content_height(self.right_content_h(), right.h);
        let mut busy = self.scroll.step((dt_ms / 1000.0) as f32);
        if let super::Vw::Ok(v) = &self.vw {
            if !v.tiles.is_empty() {
                let (_, grid_y, _) = layout(self.o().h, v.tiles.len());
                // the first row counts as in view once it clears the bottom edge of the column by a little
                self.tiles.show(self.scroll.offset() + right.h > grid_y + 28.0);
            }
        }
        busy |= self.tiles.step(dt_ms);
        busy
    }

    /// A small chevron at the foot of the column while there is more below (it fades as the column scrolls).
    pub(super) fn draw_more_hint(&self, cv: &Canvas, th: &Theme, right: Rect, has_more: bool) {
        let a = (1.0 - self.scroll.offset() / 80.0).clamp(0.0, 1.0);
        if !has_more || a < 0.02 {
            return;
        }
        let c = (right.cx(), right.bottom() - 10.0);
        let col = th.faint.fade(a);
        cv.line((c.0 - 5.0, c.1 - 2.0), (c.0, c.1 + 3.0), 1.6, col);
        cv.line((c.0, c.1 + 3.0), (c.0 + 5.0, c.1 - 2.0), 1.6, col);
    }
}

impl Default for Tiles {
    fn default() -> Self {
        Tiles::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    fn tile(kind: Kind, num: f64, frac: f32, toward: f32) -> WxTile {
        WxTile { kind, title: "t".into(), lead: String::new(), unit: "u".into(), num: Some(num), decimals: 0, text: String::new(), sub: "s".into(), frac, toward }
    }

    fn run(t: &mut Tiles) -> usize {
        let mut n = 0;
        while n < 3000 && t.step(1000.0 / 240.0) {
            n += 1;
        }
        n
    }

    #[test]
    fn the_column_gets_taller_with_the_rows() {
        let (s, g, h6) = layout(346.0, 6);
        assert_eq!((s, g), (358.0, 434.0));
        assert_eq!(h6, 434.0 + 2.0 * 108.0 + 10.0 + 2.0);
        assert!(layout(346.0, 3).2 < h6 && layout(346.0, 0).2 == layout(346.0, 3).2);
    }

    #[test]
    fn uv_colour_follows_the_category_and_differs_per_theme() {
        let all: Vec<_> = (0..5).map(|l| uv_color(l, true)).collect();
        assert!((0..5).all(|i| (0..i).all(|j| all[i] != all[j])), "five distinct colours");
        assert!((0..5).all(|l| uv_color(l, true) != uv_color(l, false)));
        assert_eq!(uv_color(99, true), uv_color(4, true));
    }

    #[test]
    fn the_compass_turns_the_short_way_round() {
        assert_eq!(unwrap_to(350.0, 10.0), 370.0);
        assert_eq!(unwrap_to(10.0, 350.0), -10.0);
        assert_eq!(unwrap_to(90.0, 180.0), 180.0);
        assert_eq!(unwrap_to(370.0, 355.0), 355.0);
    }

    #[test]
    fn tiles_rise_in_one_after_another_and_the_grid_then_idles() {
        let _m = motion_on();
        let mut t = Tiles::new();
        let data = vec![tile(Kind::Wind, 5.0, 0.3, 45.0), tile(Kind::Humidity, 80.0, 0.8, 0.0), tile(Kind::Uv, 3.0, 0.3, 0.0)];
        t.sync(&data);
        assert!(!t.step(4.0), "before the grid is in view nothing moves");
        assert_eq!(t.rise(0), 0.0);
        t.show(true);
        assert!(t.step(4.0));
        for _ in 0..50 {
            t.step(4.0);
        }
        assert!(t.rise(0) > t.rise(1) && t.rise(1) > t.rise(2), "a staggered rise: {} {} {}", t.rise(0), t.rise(1), t.rise(2));
        let n = run(&mut t);
        assert!(n > 20 && n < 3000, "{n}");
        assert!(t.settled() && (0..3).all(|i| (t.rise(i) - 1.0).abs() < 0.02), "every tile is in place");
        assert!(!t.step(4.0));
    }

    #[test]
    fn a_new_reading_eases_and_the_wind_arrow_takes_the_short_turn() {
        let _m = motion_on();
        let mut t = Tiles::new();
        t.sync(&[tile(Kind::Wind, 5.0, 0.2, 350.0)]);
        t.show(true);
        run(&mut t);
        t.sync(&[tile(Kind::Wind, 9.0, 0.5, 10.0)]);
        assert!(t.step(4.0));
        let mid = t.one[0].val.at();
        assert!(mid > 5.0 && mid < 9.0, "the number is between the readings: {mid}");
        assert_eq!(t.one[0].toward.target(), 370.0, "350 -> 10 is a 20 degree turn, not 340");
        run(&mut t);
        assert!((t.one[0].val.at() - 9.0).abs() < 0.05);
        // an unchanged set is not rebuilt, a different set starts from its own numbers
        t.sync(&[tile(Kind::Humidity, 50.0, 0.5, 0.0)]);
        assert_eq!(t.one[0].val.at(), 50.0);
    }

    #[test]
    fn rewinding_replays_the_entrance_and_reduced_motion_lands_at_once() {
        {
            let _m = motion_on();
            let mut t = Tiles::new();
            t.sync(&[tile(Kind::Wind, 5.0, 0.2, 0.0)]);
            t.show(true);
            run(&mut t);
            t.rewind();
            assert_eq!(t.rise(0), 0.0);
        }
        let _g = crate::ui::host::testing::motion_lock();
        glasskit::ease::force_reduced(Some(true));
        let mut t = Tiles::new();
        t.sync(&[tile(Kind::Wind, 5.0, 0.2, 0.0)]);
        t.show(true);
        t.step(1.0);
        glasskit::ease::force_reduced(None);
        assert!((t.rise(0) - 1.0).abs() < 0.1 && !t.step(1.0));
    }
}
