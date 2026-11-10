//! The Home player card: geometry of its three styles, the motion between them, the soft reflections of the mirror style, the tint of the fill
//! style and the lyrics column.
//!
//! * `compact` is the original card (art left, text and transport on the right). `mirror` puts the cover in the middle with the transport at its
//!   sides, the title below and blurred reflections of the cover behind both sides, dissolving into the card. `fill` is the compact layout over a
//!   strong gradient of the cover's colours, with the text turning light or dark to stay readable.
//! * With a lyrics column the card gives the right [`LY_W`] px to it and every layout shrinks into the rest.
//! * Everything moves on springs (`liquid::Flow`): a style or column change glides the rectangles into their new places, the reflections, the tint and the
//!   column fade, and the lyrics scroll when the sung line changes. All flows are retargeted in `step`, never in `draw`.
use super::layout as L;
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::a::draw::{Decoded, Fit, ImageCache};
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::{Pt, Rect};
use glasskit::gfx::{col, pt, rf};
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::home::{self as hm};
use pill_ui_core::panel::a::home_player::{LyricsView, PlayerStyle, WINDOW};
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

/// Width the lyrics column takes from the right of the card (its text is `LY_W - 8` wide).
pub const LY_W: f32 = 148.0;
/// Height of one lyric slot (two lines of 13 px).
pub const LH: f32 = 34.0;
/// Side of a mirror reflection and how far it hides behind the cover.
const REFL: f32 = 96.0;
const REFL_HIDDEN: f32 = 14.0;

/// Where everything of the player card sits (tab-local px; the layout.rs card is the frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geo {
    pub art: Rect,
    pub src: Rect,
    pub title: Rect,
    pub artist: Rect,
    pub seek: Rect,
    pub t0: Rect,
    pub t1: Rect,
    pub prev: Rect,
    pub main: Rect,
    pub next: Rect,
    /// the lyrics column
    pub col: Rect,
    /// the two mirror reflections (left, right)
    pub refl: [Rect; 2],
    /// title and artist centred (mirror) or left
    pub centered: bool,
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

fn lerp_rect(a: Rect, b: Rect, k: f32) -> Rect {
    Rect::new(lerp(a.x, b.x, k), lerp(a.y, b.y, k), lerp(a.w, b.w, k).max(0.0), lerp(a.h, b.h, k).max(0.0))
}

impl Geo {
    pub fn new(style: PlayerStyle, lyrics: bool) -> Geo {
        let card = L::PLAYER;
        let wc = if lyrics { card.w - LY_W } else { card.w };
        let at = |x: f32, y: f32, w: f32, h: f32| Rect::new(card.x + x, card.y + y, w, h);
        let col = at(wc, 10.0, LY_W - 8.0, card.h - 20.0);
        if style == PlayerStyle::Mirror {
            let art = at(wc / 2.0 - 42.0, 10.0, 84.0, 84.0);
            let (l, r) = (art.x - REFL + REFL_HIDDEN, art.right() - REFL_HIDDEN);
            let ry = art.cy() - REFL / 2.0;
            return Geo {
                art,
                src: at(12.0, 8.0, 100.0, 14.0),
                title: at(12.0, 98.0, wc - 24.0, 20.0),
                artist: at(12.0, 117.0, wc - 24.0, 16.0),
                seek: at(50.0, 134.0, wc - 100.0, 16.0),
                t0: at(12.0, 136.0, 34.0, 12.0),
                t1: at(wc - 46.0, 136.0, 34.0, 12.0),
                prev: Rect::new(art.x - 16.0 - 28.0, art.cy() - 14.0, 28.0, 28.0),
                main: Rect::new(art.cx() - 18.0, art.cy() - 18.0, 36.0, 36.0),
                next: Rect::new(art.right() + 16.0, art.cy() - 14.0, 28.0, 28.0),
                col,
                refl: [Rect::new(l, ry, REFL, REFL), Rect::new(r, ry, REFL, REFL)],
                centered: true,
            };
        }
        // compact / fill: the original numbers; the text column is what is left right of the art
        let x0 = L::COL_X - card.x;
        let w = wc - x0 - 12.0;
        let half = (w / 2.0).min(120.0);
        let c = x0 + w / 2.0;
        let art = L::ART;
        Geo {
            art,
            src: at(x0, 12.0, w, 14.0),
            title: at(x0, 28.0, w, 22.0),
            artist: at(x0, 52.0, w, 18.0),
            seek: at(x0, 74.0, w, 16.0),
            t0: at(x0, 94.0, half, 12.0),
            t1: at(x0 + w - half, 94.0, half, 12.0),
            prev: at(c - 62.0, 114.0, 28.0, 28.0),
            main: at(c - 18.0, 110.0, 36.0, 36.0),
            next: at(c + 34.0, 114.0, 28.0, 28.0),
            col,
            refl: [Rect::new(art.x - 40.0, art.cy() - REFL / 2.0, REFL, REFL), Rect::new(art.right() - REFL + 40.0, art.cy() - REFL / 2.0, REFL, REFL)],
            centered: false,
        }
    }

    pub fn lerp(&self, to: &Geo, k: f32) -> Geo {
        let l = |a: Rect, b: Rect| lerp_rect(a, b, k);
        Geo {
            art: l(self.art, to.art),
            src: l(self.src, to.src),
            title: l(self.title, to.title),
            artist: l(self.artist, to.artist),
            seek: l(self.seek, to.seek),
            t0: l(self.t0, to.t0),
            t1: l(self.t1, to.t1),
            prev: l(self.prev, to.prev),
            main: l(self.main, to.main),
            next: l(self.next, to.next),
            col: l(self.col, to.col),
            refl: [l(self.refl[0], to.refl[0]), l(self.refl[1], to.refl[1])],
            centered: to.centered,
        }
    }
}

/// The 24 x 24 colour sample of a cover (straight RGBA, `cover::downsample`) as a premultiplied BGRA picture: drawn big with smooth scaling it is the
/// cover blurred, which is all the mirror reflections need.
pub fn tiny_cover(sample: &[u8]) -> Decoded {
    let n = pill_ui_core::panel::a::cover::SAMPLE as u32;
    let bgra = sample
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u32;
            let pm = |c: u8| (c as u32 * a / 255) as u8;
            [pm(p[2]), pm(p[1]), pm(p[0]), p[3]]
        })
        .collect();
    Decoded { w: n, h: n, bgra }
}

/// White or near-black text for a surface of this colour.
pub fn ink_for(bg: Color) -> Color {
    let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let lum = 0.2126 * lin(bg.r) + 0.7152 * lin(bg.g) + 0.0722 * lin(bg.b);
    if lum > 0.34 {
        Color::rgb8(0x14, 0x14, 0x14)
    } else {
        Color::rgb8(0xf6, 0xf6, 0xf6)
    }
}

/// The other one of the two inks (the glyph on a button that takes the ink as its fill).
pub fn ink_inverse(ink: Color) -> Color {
    if ink.r > 0.5 {
        Color::rgb8(0x14, 0x14, 0x14)
    } else {
        Color::rgb8(0xf6, 0xf6, 0xf6)
    }
}

/// How a lyric line looks at distance `d` lines from the sung one, centred `dy` px from the middle of the column (half height `half`):
/// `(alpha, bright)`. The sung line is bright and opaque, the neighbours dim and the far ones nearly gone; the edges dissolve.
pub fn line_look(d: f32, dy: f32, half: f32) -> (f32, f32) {
    let a = d.abs();
    let bright = (1.0 - a).clamp(0.0, 1.0);
    let near = (1.0 - a * 0.30).clamp(0.0, 1.0);
    let edge = ((half - dy.abs()) / 26.0).clamp(0.0, 1.0);
    ((0.30 + 0.70 * near) * edge * if a < 0.5 { 1.0 } else { 0.78 }, bright)
}

/// The springs and rectangles of the player card.
pub struct PlayerFx {
    pub geo: Geo,
    from: Geo,
    to: Geo,
    k: Flow,
    pub mir: Flow,
    pub fill: Flow,
    ly_a: Flow,
    ly_pos: Flow,
    ly_key: Option<u64>,
    init: bool,
}

impl PlayerFx {
    pub fn new() -> PlayerFx {
        let g = Geo::new(PlayerStyle::Mirror, false);
        PlayerFx {
            geo: g,
            from: g,
            to: g,
            k: Flow::new(1.0, Feel::FLOW),
            mir: Flow::new(1.0, Feel::FADE.within(320.0)),
            fill: Flow::new(0.0, Feel::FADE.within(420.0)),
            ly_a: Flow::new(0.0, Feel::FADE.within(320.0)),
            ly_pos: Flow::new(0.0, Feel::FLOW),
            ly_key: None,
            init: false,
        }
    }

    /// Retarget everything from the view-model and advance by `dt_ms`; true while something still moves.
    pub fn step(&mut self, p: &hm::Player, dt_ms: f64) -> bool {
        let col = p.lyrics.is_some() && !p.none;
        let target = Geo::new(p.style, col);
        let (mir, fill) = ((p.style == PlayerStyle::Mirror) as u8 as f32, (p.style == PlayerStyle::Fill) as u8 as f32);
        if !self.init {
            // first sight: already in place, nothing slides in from a default
            self.init = true;
            (self.geo, self.from, self.to) = (target, target, target);
            self.mir.jump(mir);
            self.fill.jump(fill);
            self.ly_a.jump(col as u8 as f32);
        } else if target != self.to {
            self.from = self.geo;
            self.to = target;
            self.k.jump(0.0);
        }
        self.k.to(1.0);
        self.mir.to(mir);
        self.fill.to(fill);
        self.ly_a.to(col as u8 as f32);
        if let Some(l) = &p.lyrics {
            let t = l.cur.map_or(-0.6, |c| c as f32);
            // another song, or a seek further than the window shows: no long scroll over lines that are not there
            if self.ly_key != Some(l.key) || (t - self.ly_pos.at()).abs() > WINDOW as f32 - 1.0 {
                self.ly_pos.jump(t);
            }
            self.ly_key = Some(l.key);
            self.ly_pos.to(t);
        }
        let mut busy = false;
        for f in [&mut self.k, &mut self.mir, &mut self.fill, &mut self.ly_a, &mut self.ly_pos] {
            f.step(dt_ms);
            busy |= f.moving();
        }
        self.geo = self.from.lerp(&self.to, self.k.at());
        busy
    }

    /// The theme the card is painted with: the text turns to `ink` as the fill tint comes in.
    pub fn themed(&self, th: &Theme, ink: Color) -> Theme {
        let k = self.fill.at01();
        if k < 0.004 {
            return *th;
        }
        Theme { text: th.text.mix(ink, k), text2: th.text2.mix(ink.fade(0.84), k), dim: th.dim.mix(ink.fade(0.62), k), hover: th.hover.mix(ink.fade(0.16), k), ..*th }
    }

    /// The blurred reflections behind the cover. `key` = the tiny pre-blurred copy of the cover (or the cover itself while that is not ready).
    pub fn paint_reflections(&self, cv: &Canvas, images: &ImageCache, key: Option<&str>, ox: f32, oy: f32) {
        let k = self.mir.at01();
        let Some(key) = key.filter(|_| k > 0.01) else { return };
        for (i, r) in self.geo.refl.iter().enumerate() {
            let r = r.offset(ox, oy);
            // the mask is brightest at the edge hidden behind the cover and dissolves in every direction
            let inner: Pt = if i == 0 { (r.right() - REFL_HIDDEN, r.cy()) } else { (r.x + REFL_HIDDEN, r.cy()) };
            with_radial_mask(cv, r, inner, r.w * 0.9, r.h * 0.6, &[(0.0, 1.0), (0.5, 0.5), (1.0, 0.0)], 0.8 * k, |cv| {
                images.draw(cv, key, r, 0.0, 1.0, Fit::Cover);
            });
        }
    }

    /// The lyrics column (absolute positions; call it inside the card's clip).
    pub fn paint_lyrics(&self, cv: &Canvas, th: &Theme, v: &LyricsView, ox: f32, oy: f32) {
        let a = self.ly_a.at01();
        if a < 0.01 {
            return;
        }
        let col = self.geo.col.offset(ox, oy);
        let pos = self.ly_pos.at();
        for (i, text) in &v.lines {
            let d = *i as f32 - pos;
            let cy = col.cy() + d * LH;
            let (alpha, bright) = line_look(d, cy - col.cy(), col.h / 2.0);
            if alpha < 0.02 {
                continue;
            }
            let r = Rect::new(col.x, cy - LH / 2.0, col.w, LH);
            let c = th.dim.mix(th.text, bright);
            let s = if text.is_empty() { "\u{266a}" } else { text.as_str() };
            cv.with_opacity(alpha * a, |cv| cv.with_clip(r, |cv| cv.text(r, s, Style::new(13.0, 600).wrap(), c)));
        }
    }
}

/// `f` draws through a radial opacity mask: full at `centre`, fading to nothing at `rx` / `ry`, and as a whole at `opacity`.
#[allow(clippy::too_many_arguments)]
fn with_radial_mask(cv: &Canvas, bounds: Rect, centre: Pt, rx: f32, ry: f32, stops: &[(f32, f32)], opacity: f32, f: impl FnOnce(&Canvas)) {
    unsafe {
        let st: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|&(p, a)| D2D1_GRADIENT_STOP { position: p, color: col(1.0, 1.0, 1.0, a) }).collect();
        let brush = cv
            .dc
            .CreateGradientStopCollection(&st, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT)
            .and_then(|coll| {
                let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: pt(centre.0, centre.1), gradientOriginOffset: pt(0.0, 0.0), radiusX: rx, radiusY: ry };
                let c: &ID2D1GradientStopCollection = &coll;
                cv.dc.CreateRadialGradientBrush(&props, None, c)
            });
        let Ok(brush) = brush else { return f(cv) };
        let mut lp = D2D1_LAYER_PARAMETERS1 {
            contentBounds: rf(bounds.x, bounds.y, bounds.right(), bounds.bottom()),
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: glasskit::gfx::mat(1.0, 0.0, 0.0),
            opacity: opacity.clamp(0.0, 1.0),
            opacityBrush: ManuallyDrop::new(brush.cast::<ID2D1Brush>().ok()),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        cv.dc.PushLayer(&lp, None::<&ID2D1Layer>);
        f(cv);
        cv.dc.PopLayer();
        ManuallyDrop::drop(&mut lp.opacityBrush);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    fn player(style: PlayerStyle, lyrics: bool) -> hm::Player {
        let mut v = hm::Home::new().view(&pill_ui_core::panel::a::EnTr).player;
        v.none = false;
        v.style = style;
        v.lyrics = lyrics.then(|| LyricsView { lines: vec![(0, "a".into()), (1, "b".into())], cur: Some(0), key: 1 });
        v
    }

    #[test]
    fn compact_without_lyrics_is_the_original_card() {
        let g = Geo::new(PlayerStyle::Compact, false);
        assert_eq!((g.art, g.src, g.title, g.artist, g.seek), (L::ART, L::SRC, L::TITLE, L::ARTIST, L::SEEK));
        assert_eq!((g.t0, g.t1, g.prev, g.main, g.next), (L::T0, L::T1, L::PREV, L::MAIN, L::NEXT));
        assert_eq!(Geo::new(PlayerStyle::Fill, false), Geo { refl: g.refl, ..g }, "fill only differs in its paint");
    }

    #[test]
    fn every_style_fits_inside_the_card_with_or_without_the_column() {
        for style in [PlayerStyle::Mirror, PlayerStyle::Fill, PlayerStyle::Compact] {
            for lyrics in [false, true] {
                let g = Geo::new(style, lyrics);
                let content = L::PLAYER.w - if lyrics { LY_W } else { 0.0 };
                for (name, r) in [("art", g.art), ("src", g.src), ("title", g.title), ("artist", g.artist), ("seek", g.seek), ("t0", g.t0), ("t1", g.t1), ("prev", g.prev), ("main", g.main), ("next", g.next)] {
                    assert!(r.x >= L::PLAYER.x && r.right() <= L::PLAYER.x + content + 0.01 && r.y >= L::PLAYER.y && r.bottom() <= L::PLAYER.bottom(), "{style:?} lyrics={lyrics}: {name} {r:?} leaves the card");
                }
                if lyrics {
                    assert!(g.col.x >= L::PLAYER.x + content - 0.01 && g.col.right() <= L::PLAYER.right(), "{style:?}: column {:?}", g.col);
                }
                // the transport never overlaps itself
                assert!(g.prev.right() <= g.main.x + 0.01 || style == PlayerStyle::Mirror, "{style:?}");
                assert!(g.main.right() <= g.next.x + 0.01 || style == PlayerStyle::Mirror, "{style:?}");
            }
        }
        let m = Geo::new(PlayerStyle::Mirror, false);
        assert!((m.art.cx() - L::PLAYER.cx()).abs() < 0.01, "the cover sits in the middle");
        assert!(m.centered && m.seek.cy() > m.artist.bottom());
    }

    #[test]
    fn text_ink_follows_the_surface() {
        assert_eq!(ink_for(Color::rgb8(250, 230, 120)).r, Color::rgb8(0x14, 0x14, 0x14).r, "a light tint gets dark text");
        assert_eq!(ink_for(Color::rgb8(40, 20, 90)).r, Color::rgb8(0xf6, 0xf6, 0xf6).r, "a dark tint gets light text");
    }

    #[test]
    fn the_sung_line_is_bright_and_the_others_fade_with_distance() {
        let at = |d: f32| line_look(d, d * LH, 67.0);
        let (a0, b0) = at(0.0);
        let (a1, b1) = at(1.0);
        let (a2, _) = at(2.0);
        assert!(a0 > a1 && a1 > a2 && b0 == 1.0 && b1 == 0.0, "{a0} {a1} {a2}");
        assert_eq!(line_look(0.0, 80.0, 67.0).0, 0.0, "outside the column nothing shows");
        assert!(at(0.5).1 == 0.5 && at(-1.0).1 == 0.0);
    }

    #[test]
    fn a_style_change_glides_the_rectangles_and_a_still_card_idles() {
        let _m = motion_on();
        let mut fx = PlayerFx::new();
        assert!(!fx.step(&player(PlayerStyle::Compact, false), 16.0), "first sight: in place, nothing moves");
        assert_eq!(fx.geo, Geo::new(PlayerStyle::Compact, false));
        let from = fx.geo.art.x;
        assert!(fx.step(&player(PlayerStyle::Mirror, false), 16.0), "a new style starts to move");
        let mid = fx.geo.art.x;
        assert!(mid > from && mid < Geo::new(PlayerStyle::Mirror, false).art.x, "the cover glides: {from} -> {mid}");
        for _ in 0..200 {
            fx.step(&player(PlayerStyle::Mirror, false), 16.0);
        }
        assert!(!fx.step(&player(PlayerStyle::Mirror, false), 16.0), "settled: the card idles");
        assert_eq!(fx.geo, Geo::new(PlayerStyle::Mirror, false));
        // the column makes room, then the sung line scrolls the lyrics
        assert!(fx.step(&player(PlayerStyle::Mirror, true), 16.0));
        let mut p = player(PlayerStyle::Mirror, true);
        for _ in 0..200 {
            fx.step(&p, 16.0);
        }
        assert_eq!(fx.geo, Geo::new(PlayerStyle::Mirror, true));
        p.lyrics.as_mut().unwrap().cur = Some(1);
        assert!(fx.step(&p, 16.0), "the next line scrolls in");
        assert!(fx.ly_pos.at() > 0.0 && fx.ly_pos.at() < 1.0);
    }
}
