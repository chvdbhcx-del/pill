//! The pixels of the music widget (a child module of `view`). The look is the island's own player, drawn with its own pieces, and the folded capsule is
//! the open card at a small size: ONE object that grows. Both have the Home player card's body (`tabs::a::home::paint::paint_player`: the
//! cover-tinted background, the blurred reflections of the cover, the cover with its shadow and its corner shape, the title over the artist line,
//! the previous / next icons and the smoked play disc); the card adds the source label, the times and the progress track, which shows the loudness
//! of the track as thin rounded bars; the capsule adds a hairline of progress. Colours, radii and text styles are the theme's and the island's.
//!
//! `Ink` is everything one `draw` needs, copied out of the view so the cover cache can be borrowed mutably next to it; nothing here moves by
//! itself (every animated value arrives as a number from the view's springs).
use super::lg::{self, Btn, BTNS};
use crate::ui::musicwidget::wave::BARS;
use super::{arrival, Ghost};
use crate::ui::island::gpu::Gpu;
use crate::ui::liquid::Flow;
use crate::ui::tabs::a::draw::{self, ImageCache};
use crate::ui::tabs::a::home::{css, player};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use pill_ui_core::panel::a::home_player::PlayerStyle;

/// The Home card's text styles: the title (16 / 600), the artist line (13 / 400), the source label and the times. The capsule has the same two
/// styles, scaled with its height (`lg::capsule_text_sizes`).
const TITLE: Style = Style::new(16.0, 600).ellipsis();
const ARTIST: Style = Style::new(13.0, 400).ellipsis();
/// How far from its end the capsule's text fades out when it is cut (no marquee, no "...").
const FADE_W: f32 = 22.0;
const SRC: Style = Style::new(10.0, 600).track(0.6).ellipsis();
const TIMES: Style = Style::new(11.0, 400).mono();

/// The loudness bars: the gap between them, the height of a bar that is not measured yet (the island's track is 4 px), the tallest bar.
const BAR_GAP: f32 = 3.0;
const BAR_MIN: f32 = 4.0;
const BAR_MAX: f32 = 14.0;

/// The island's track colour (`paint_player`): a grey that suits the theme, taking the card's ink on the fill style.
fn track_color(th: &Theme) -> Color {
    if th.dark {
        Color::rgb8(0x35, 0x35, 0x35)
    } else {
        Color::rgb8(0xc9, 0xc9, 0xc9)
    }
}

/// Everything `draw` needs.
pub(super) struct Ink<'a> {
    pub st: lg::Stand,
    pub th: &'a Theme,
    /// the cover's accent (what the island's seek fill takes): eases to each new cover's
    pub accent: Color,
    /// the cover's two colours (the card's tint), when the cover has them
    pub tint: Option<(Color, Color)>,
    pub hc: bool,
    pub track: &'a lg::Track,
    pub swap: f32,
    pub ghosts: &'a [Ghost],
    pub pp: f32,
    pub hov: &'a [Flow; 3],
    pub press: &'a [Flow; 3],
    /// prev's and next's push (px, signed: toward where the track goes)
    pub kick: [f32; 2],
    pub frac: f32,
    /// 0 = nobody near the track, 0.75 under the pointer, 1 while it is dragged: the bars grow a little with it
    pub knob: f32,
    pub pos: u64,
    pub now: f64,
    /// the loudness bars: their heights now (0..1) and which are measured
    pub heights: [f32; BARS],
    pub known: [bool; BARS],
    /// the expansion and the places the shared parts stand in at it
    pub p: f32,
    pub pt: lg::Parts,
    /// the body now (the shape that grows from the capsule to the card) and its corner radius
    pub body: Rect,
    pub rad: f32,
    pub cap: Rect,
    pub card: Rect,
    pub art: u64,
    pub under: Option<u64>,
    pub url: Option<&'a str>,
    /// the blurred reflections: `tiny_cover` of the cover, in the island's image cache
    pub images: &'a ImageCache,
    pub blur: Option<&'a str>,
}

impl Ink<'_> {
    /// The whole body: the card's tinted body and reflections over the whole shape (they grow with it), the shared parts (cover, buttons, progress)
    /// travelling between their places, the capsule's text fading out as the card's lines, the source label, the times and the loudness bars fade in.
    pub fn paint(&self, cv: &Canvas, gpu: &mut Gpu) {
        let (st, p, pt) = (self.st, self.p, self.pt);
        let (capl, cardl) = (lg::capsule_layout_in(st, self.cap), lg::card_layout(self.card, st.style));
        let card_a = ((p - 0.3) / 0.7).clamp(0.0, 1.0);
        let cap_a = (1.0 - p * 1.7).clamp(0.0, 1.0);
        // the fill style turns the text light or dark with its tint, folded and open alike
        let fill = st.style == PlayerStyle::Fill && !self.hc;
        let (ca, cb) = self.fill_colors();
        let ink = player::ink_for(ca.mix(cb, 0.5));
        let fk = if fill { 1.0 } else { 0.0 };
        let thc = self.themed(ink, fk);
        let th = &thc;
        self.card_bg(cv, fill, (ca, cb));
        if st.style == PlayerStyle::Mirror && !self.hc && self.cap.h >= REFL_MIN_H {
            // the cover's reflections: the one to its right is always there (softer while it is small, so that the text on it reads), the one to its
            // left grows with the card, and the one coming in from the far end of the capsule is gone by the time the card is open
            let q = p.clamp(0.0, 1.0);
            let r = &pt.refl;
            player::paint_reflection(cv, self.images, self.blur, r[2], true, 0.6 * (1.0 - q));
            player::paint_reflection(cv, self.images, self.blur, r[1], false, lg::lerp(0.6, 1.0, q));
            player::paint_reflection(cv, self.images, self.blur, r[0], true, q);
        }
        // the cover: the island's shadow under it, scaled with it
        if !self.hc {
            let k = pt.cover.w / 84.0;
            cv.shadow_rr(pt.cover, pt.cover_r, 16.0 * k, 6.0 * k, Color::rgba(0.0, 0.0, 0.0, 0.35));
        }
        let px = ((lg::COVER * st.bs() / 32.0).ceil().max(1.0) * 32.0) as u32;
        gpu.cover_sized(cv, self.th, self.art, self.url, pt.cover, self.now, pt.cover_r, px, self.under);
        // the capsule's text (the title over the artist, or one line), riding up with the cover as the body rises
        if cap_a > 0.01 {
            cv.with_opacity(cap_a, |cv| self.capsule_text(cv, th, capl.text.offset(0.0, pt.cover.y - capl.cover.y)));
        }
        if card_a > 0.01 {
            cv.with_opacity(card_a, |cv| {
                cv.text(cardl.src, &lg::app_name(&self.track.app).to_uppercase(), SRC, th.dim);
                let c = cardl.centered;
                let (ts, as_) = if c { (TITLE.center(), ARTIST.center()) } else { (TITLE, ARTIST) };
                self.lines(cv, th, cardl.title, cardl.sub, ts, as_);
                cv.text(cardl.elapsed, &lg::clock_text(self.pos), TIMES, th.dim);
                let rem = if self.track.duration_ms > 0 { format!("-{}", lg::clock_text(self.track.duration_ms.saturating_sub(self.pos))) } else { "--:--".to_string() };
                cv.text(cardl.total, &rem, TIMES.right(), th.dim);
            });
        }
        // the progress: the capsule's hairline becomes the loudness bars of the card, in the island's accent fill and track colours
        let tc = if self.hc { self.th.border } else { track_color(self.th) }.mix(ink.fade(0.28), fk);
        let acc = self.accent.mix(ink, fk);
        if card_a < 0.99 && capl.bar.h > 0.0 {
            cv.with_opacity(1.0 - card_a, |cv| self.hairline(cv, pt.bar, tc, acc));
        }
        if card_a > 0.01 {
            cv.with_opacity(card_a, |cv| self.bars(cv, pt.bar, card_a, tc, acc));
        }
        for b in BTNS {
            self.button(cv, gpu, th, b, pt.btn[b.index()], ink, fk);
        }
    }

    /// The card's two fill colours (`paint_player`): the cover's, mixed into the card colour; the accent when the cover has none.
    fn fill_colors(&self) -> (Color, Color) {
        let (fa, fb) = self.tint.unwrap_or((self.accent, self.accent.mix(Color::rgb8(0, 0, 0), 0.35)));
        (css::mix(fa, 84.0, self.th.card), css::mix(fb, 70.0, self.th.card))
    }

    /// The theme the card's text is drawn with: it turns to `ink` as the fill tint comes in (`PlayerFx::themed`).
    fn themed(&self, ink: Color, k: f32) -> Theme {
        let th = *self.th;
        if k < 0.004 {
            return th;
        }
        Theme { text: th.text.mix(ink, k), text2: th.text2.mix(ink.fade(0.84), k), dim: th.dim.mix(ink.fade(0.62), k), hover: th.hover.mix(ink.fade(0.16), k), ..th }
    }

    /// The Home card's background over the whole body (the capsule's too): the cover's gradient over the card colour, else a tinted corner glow of
    /// the accent; the whole tint on the fill style. The recipe is the card's, its rectangle is the body, so it grows with the shape.
    fn card_bg(&self, cv: &Canvas, fill: bool, (ca, cb): (Color, Color)) {
        let (th, b, rad) = (self.th, self.body, self.rad);
        cv.fill_rr(b, rad, th.card);
        if self.hc {
            return;
        }
        if let Some((a, c)) = self.tint {
            draw::linear_deg(cv, b, rad, 100.0, &[(0.0, css::mix(a, 30.0, th.card)), (0.28, css::mix(c, 14.0, th.card)), (0.52, th.card), (1.0, th.card)]);
        } else {
            let base = if th.dark { Color::rgb8(0x1b, 0x1b, 0x1b) } else { Color::rgb8(255, 255, 255) };
            let mixed = css::mix(self.accent, if th.dark { 17.0 } else { 22.0 }, base);
            draw::radial(cv, b, rad, (b.x, b.y), b.w * 1.3, b.h * 1.5, &[(0.0, mixed), (0.72, th.card), (1.0, th.card)]);
        }
        if fill {
            draw::linear_deg(cv, b, rad, 118.0, &[(0.0, ca), (1.0, cb)]);
        }
    }

    /// The capsule's progress: a hairline track and the part that has played.
    fn hairline(&self, cv: &Canvas, r: Rect, track: Color, fill: Color) {
        if r.h <= 0.0 {
            return;
        }
        cv.fill_rr(r, r.h / 2.0, track);
        let w = (r.w * self.frac.clamp(0.0, 1.0)).clamp(0.0, r.w);
        if w > 0.5 {
            cv.fill_rr(Rect::new(r.x, r.y, w.max(r.h), r.h), r.h / 2.0, fill);
        }
    }

    /// The card's progress track as the loudness of the track: a bar for every slice, centred on the track's line, the ones the playhead has passed
    /// in the accent and the rest in the track colour. A slice not measured yet is a short flat bar, dimmer. The bars grow in one after the other as
    /// the card opens (`open` 0..1), the colour follows the playhead (it glides when the user seeks) and the bars swell a little under the pointer.
    fn bars(&self, cv: &Canvas, r: Rect, open: f32, track: Color, fill: Color) {
        let head = self.frac.clamp(0.0, 1.0) * BARS as f32;
        let swell = 1.0 + 0.12 * self.knob.clamp(0.0, 1.1);
        for i in 0..BARS {
            let (x, w) = lg::wave_cell(r, BARS, BAR_GAP, i);
            let g = ((open - i as f32 / BARS as f32 * 0.5) / 0.5).clamp(0.0, 1.0);
            let g = g * g * (3.0 - 2.0 * g);
            let natural = if self.known[i] { BAR_MIN + (BAR_MAX - BAR_MIN) * ((self.heights[i] - 0.2) / 0.8).clamp(0.0, 1.0) } else { BAR_MIN };
            let h = natural * swell * g;
            if h < 0.5 {
                continue;
            }
            let (rest, played) = if self.known[i] { (track, fill) } else { (track.fade(0.55), fill.fade(0.5)) };
            let col = rest.mix(played, (head - i as f32).clamp(0.0, 1.0));
            cv.fill_rr(Rect::new(x, r.cy() - h / 2.0, w, h), (w / 2.0).min(h / 2.0), col);
        }
    }

    /// Two lines of text that cross-fade when the track changes: the old pair rises and fades, the new one rises into place, the artist a beat
    /// after the title.
    fn lines(&self, cv: &Canvas, th: &Theme, head: Rect, sub: Rect, hs: Style, ss: Style) {
        for g in self.ghosts {
            let a = g.a.at01();
            let rise = -4.0 * (1.0 - a);
            cv.with_opacity(a, |cv| {
                cv.text(head.offset(0.0, rise), &g.head, hs, th.text);
                cv.text(sub.offset(0.0, rise), &g.line, ss, th.text2);
            });
        }
        let (h, s) = (self.track.heading(), self.track.artist_line());
        let (kh, ks) = arrival(self.swap);
        cv.with_opacity(kh, |cv| cv.text(head.offset(0.0, 4.0 * (1.0 - kh)), &h, hs, th.text));
        cv.with_opacity(ks, |cv| cv.text(sub.offset(0.0, 4.0 * (1.0 - ks)), &s, ss, th.text2));
    }

    /// The capsule's text in `r`, the track change cross-fade included: the title (the card's title style, scaled) and the artist (the card's artist
    /// style) on ONE line when both fit whole, else the title over the artist, each with a faded end when it is cut; in a bar's strip too low for two
    /// lines, one line the artist of which gives way (`lg::capsule_lines`). The old text rises away and the new one rises in, the artist a beat later.
    fn capsule_text(&self, cv: &Canvas, th: &Theme, r: Rect) {
        let sizes = lg::capsule_text_sizes(self.cap.h);
        let (ts, ss) = (Style { size: sizes.0, ellipsis: false, ..TITLE }, Style { size: sizes.1, ellipsis: false, ..ARTIST });
        let (kh, ks) = arrival(self.swap);
        let pair = |cv: &Canvas, h: &str, s: &str, (kh, ks): (f32, f32), (dh, ds): (f32, f32)| {
            let (wt, wa) = (cv.text_width(h, ts), if s.is_empty() { 0.0 } else { cv.text_width(s, ss) });
            match lg::capsule_lines(r.w, self.cap.h, wt, wa) {
                lg::Lines::Two => {
                    let (rt, ra) = lg::two_lines(r, sizes);
                    cv.with_opacity(kh, |cv| faded(cv, rt.offset(0.0, dh), h, ts, th.text));
                    cv.with_opacity(ks, |cv| faded(cv, ra.offset(0.0, ds), s, ss, th.text2));
                }
                lg::Lines::One => {
                    let (hw, sub) = lg::bar_line(r.w, wt, wa);
                    cv.with_opacity(kh, |cv| faded(cv, Rect::new(r.x, r.y + dh, hw.ceil() + 1.0, r.h).intersect_w(r.w), h, ts, th.text));
                    if let Some((sx, sw)) = sub {
                        cv.with_opacity(ks, |cv| faded(cv, Rect::new(r.x + sx, r.y + ds, sw.ceil() + 1.0, r.h).intersect_w(r.w - sx), s, ss, th.text2));
                    }
                }
            }
        };
        for g in self.ghosts {
            let a = g.a.at01();
            pair(cv, &g.head, &g.sub, (a, a), (-3.0 * (1.0 - a), -3.0 * (1.0 - a)));
        }
        pair(cv, &self.track.heading(), &self.track.sub(), (kh, ks), (3.0 * (1.0 - kh), 3.0 * (1.0 - ks)));
    }

    /// A button, the card's way at the capsule's size too: previous / next are a plain icon with a hover disc; play is the Home card's main button,
    /// a smoked disc (a solid disc on the other styles, the card's ink on the fill tint).
    #[allow(clippy::too_many_arguments)]
    fn button(&self, cv: &Canvas, gpu: &mut Gpu, th: &Theme, b: Btn, r: Rect, ink: Color, fk: f32) {
        let (h, q) = (self.hov[b.index()].at01(), self.press[b.index()].at().clamp(0.0, 1.0));
        let c = r.center();
        match b {
            Btn::Prev | Btn::Next => {
                let ir = r.centered(r.w * 0.64, r.w * 0.64);
                let kick = if b == Btn::Prev { self.kick[0] } else { self.kick[1] };
                cv.with_scale_about(c, 1.0 - 0.06 * q, |cv| {
                    // the island's hover disc: 14 px round a 28 px button
                    cv.fill_circle(c, r.w / 2.0, th.hover.fade(h));
                    gpu.icon(cv, if b == Btn::Prev { "prev" } else { "next" }, ir.offset(kick, 0.0), th.text2.mix(th.text, h), 1.7);
                });
            }
            Btn::Play => {
                let ir = r.centered(r.w * 0.56, r.w * 0.56);
                let (bg, fg) = if th.dark { (Color::rgb8(255, 255, 255), Color::rgb8(0, 0, 0)) } else { (Color::rgb8(0x15, 0x15, 0x15), Color::rgb8(255, 255, 255)) };
                let mir = (self.st.style == PlayerStyle::Mirror) as u8 as f32;
                let (bg, fg) = (bg.mix(ink, fk).mix(Color::rgba(0.0, 0.0, 0.0, 0.52), mir), fg.mix(player::ink_inverse(ink), fk).mix(Color::rgb8(255, 255, 255), mir));
                let s = (1.0 - 0.06 * q) * (1.0 + 0.06 * h);
                cv.with_scale_about(c, s, |cv| {
                    cv.fill_circle(c, r.w / 2.0, bg);
                    // the play and pause icons cross-fade, the leaving one shrinking away and the arriving one growing in, with the spring's pop
                    let k = self.pp.clamp(0.0, 1.0);
                    let pop = 1.0 + 0.10 * (self.pp - 1.0).abs().min(1.0) * (self.pp > 1.0) as u8 as f32;
                    cv.with_scale_about(c, pop, |cv| {
                        cv.with_opacity(1.0 - k, |cv| cv.with_scale_about(c, 1.0 - 0.3 * k, |cv| gpu.icon(cv, "play", ir, fg, 1.7)));
                        cv.with_opacity(k, |cv| cv.with_scale_about(c, 0.7 + 0.3 * k, |cv| gpu.icon(cv, "pause", ir, fg, 1.7)));
                    });
                });
            }
        }
    }
}

/// The smallest capsule (a bar's strip) that still has the reflections: below it there is no room for them to read as anything.
const REFL_MIN_H: f32 = 34.0;

trait WidthCap {
    fn intersect_w(self, w: f32) -> Rect;
}

impl WidthCap for Rect {
    /// The same rectangle no wider than `w`.
    fn intersect_w(self, w: f32) -> Rect {
        Rect::new(self.x, self.y, self.w.min(w.max(0.0)), self.h)
    }
}

/// One line of text in `r`: whole when it fits, else cut with a faded end (the last [`FADE_W`] px dissolve), never "..." and never a marquee.
fn faded(cv: &Canvas, r: Rect, s: &str, st: Style, c: Color) {
    if s.is_empty() || r.w <= 0.0 {
        return;
    }
    if cv.text_width(s, st) <= r.w + 0.5 {
        cv.text(r, s, st, c);
        return;
    }
    let fw = FADE_W.min(r.w * 0.5);
    with_fade_right(cv, r, fw, |cv| cv.with_clip(r, |cv| cv.text(r, s, st, c)));
}

/// `f` draws through a horizontal opacity mask: full until `fw` before the right edge of `bounds`, nothing at the edge.
fn with_fade_right(cv: &Canvas, bounds: Rect, fw: f32, f: impl FnOnce(&Canvas)) {
    use glasskit::gfx::{col, pt, rf};
    unsafe {
        let st = [D2D1_GRADIENT_STOP { position: 0.0, color: col(1.0, 1.0, 1.0, 1.0) }, D2D1_GRADIENT_STOP { position: 1.0, color: col(1.0, 1.0, 1.0, 0.0) }];
        let brush = cv
            .dc
            .CreateGradientStopCollection(&st, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT)
            .and_then(|coll| {
                let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(bounds.right() - fw, bounds.cy()), endPoint: pt(bounds.right(), bounds.cy()) };
                let c: &ID2D1GradientStopCollection = &coll;
                cv.dc.CreateLinearGradientBrush(&props, None, c)
            });
        let Ok(brush) = brush else { return f(cv) };
        let mut lp = D2D1_LAYER_PARAMETERS1 {
            contentBounds: rf(bounds.x, bounds.y, bounds.right(), bounds.bottom()),
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: glasskit::gfx::mat(1.0, 0.0, 0.0),
            opacity: 1.0,
            opacityBrush: ManuallyDrop::new(brush.cast::<ID2D1Brush>().ok()),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        cv.dc.PushLayer(&lp, None::<&ID2D1Layer>);
        f(cv);
        cv.dc.PopLayer();
        ManuallyDrop::drop(&mut lp.opacityBrush);
    }
}
