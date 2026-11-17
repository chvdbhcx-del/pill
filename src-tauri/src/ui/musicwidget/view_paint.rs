//! The pixels of the music widget (a child module of `view`). The look is the island's own player, drawn with its own pieces, and the folded capsule is
//! the open card at a small size: ONE object that grows. The card is the Home player card's body (`tabs::a::home::paint::paint_player`: the
//! cover-tinted background, the blurred reflections of the cover, the cover with its shadow and its corner shape, the title over the artist line,
//! the previous / next icons and the play disc, the source label, the times and the progress track, which shows the loudness of the track as thin
//! rounded bars). The capsule is the minimal one: the body is ALWAYS in the cover's colour (the fill style's gradient, whatever `home.player` says;
//! the ink turns light or dark with it), ONE line of text (the title, then the artist muted), the track's loudness as a mini waveform that is its
//! progress, previous, a round play button (an ink disc with the glyph cut out in the body's colour) and next. As the card opens the tint eases to
//! the card's own look for the player style (`Ink::paint`: `fk`, `cap`). Colours, radii and text styles are the theme's and the island's.
//!
//! `Ink` is everything one `draw` needs, copied out of the view so the cover cache can be borrowed mutably next to it; nothing here moves by
//! itself (every animated value arrives as a number from the view's springs).
use super::lg::{self, Btn, BTNS};
use crate::ui::musicwidget::wave::{squash, BARS};
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
    /// which ink the tint calls for as it eases: 0 = the light one, 1 = the dark one (`ink_target`; a spring in the view, so a new cover's colour
    /// turns the text over smoothly instead of flipping it)
    pub ink_dark: f32,
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

/// The two inks (`player::ink_for`): near white on a dark tint, near black on a light one.
const INK_LIGHT: Color = Color::rgb8(0xf6, 0xf6, 0xf6);
const INK_DARK: Color = Color::rgb8(0x14, 0x14, 0x14);

/// The card's two fill colours (`paint_player`): the cover's, mixed into the card colour; the accent when the cover has none.
fn fill_pair(th: &Theme, tint: Option<(Color, Color)>, accent: Color) -> (Color, Color) {
    let (fa, fb) = tint.unwrap_or((accent, accent.mix(Color::rgb8(0, 0, 0), 0.35)));
    (css::mix(fa, 84.0, th.card), css::mix(fb, 70.0, th.card))
}

/// The folded capsule's own, much calmer body: the cover's first colour mixed in at [`SOFT_TINT`] % over the card colour, running top to bottom into a
/// hardly darker version of the same hue (no sweep through a second colour, no diagonal: a wide diagonal ramp of a few levels shows as slanting
/// bands). The open card's fill style keeps its strong pair (`fill_pair`).
const SOFT_TINT: f32 = 34.0;
const SOFT_DARKEN: f32 = 0.08;

/// The relative luminance of a colour, 0 (black) .. 1 (white).
fn luma(c: Color) -> f32 {
    let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

fn soft_pair(th: &Theme, tint: Option<(Color, Color)>, accent: Color) -> (Color, Color) {
    let c = tint.map_or(accent, |t| t.0);
    // on a dark card a pale cover would wash the body into a dirty grey: its share eases off as the cover gets lighter
    let pct = if th.dark { SOFT_TINT * (1.0 - 0.45 * luma(c).sqrt()) } else { SOFT_TINT };
    let a = css::mix(c, pct, th.card);
    (a, a.mix(Color::rgb8(0, 0, 0), SOFT_DARKEN))
}

/// The WCAG contrast of two colours.
fn contrast(a: Color, b: Color) -> f32 {
    let (x, y) = (luma(a), luma(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The colour the ink has to read on: the middle of the capsule's body, or (`strong`: the card is open in the fill style) of the card's.
pub(super) fn body_mid(th: &Theme, tint: Option<(Color, Color)>, accent: Color, strong: bool) -> Color {
    let (a, b) = if strong { fill_pair(th, tint, accent) } else { soft_pair(th, tint, accent) };
    a.mix(b, 0.5)
}

/// Which ink the tint calls for, 0 = light, 1 = dark (what the view's `ink` spring heads for): the one that reads best on the
/// body. The island's `player::ink_for` flips at a luminance of 0.34, which keeps white on a mid-tone (a salmon pink reads at 2.8:1); the
/// capsule's text is small and half of it is muted, so the dark ink is chosen as soon as it reads 1.3 times better than the light one (a
/// luminance of about 0.22): the title keeps at least 3.6:1 (light ink) / 4.7:1 (dark ink) on every tint.
pub(super) fn ink_target(th: &Theme, tint: Option<(Color, Color)>, accent: Color, strong: bool) -> f32 {
    let body = body_mid(th, tint, accent, strong);
    (contrast(INK_DARK, body) >= 1.31 * contrast(INK_LIGHT, body)) as u8 as f32
}

/// What the capsule's colours are made of, once per `paint`.
#[derive(Clone, Copy)]
struct Look {
    ink: Color,
    /// the other ink (the glyph on a button that takes the ink as its fill)
    inv: Color,
    /// how much of the fill tint is in the body now: 1 in the capsule, easing to the card's (1 for the fill style, else 0) as it opens
    fk: f32,
    /// how much of the folded capsule is left (1 folded, 0 open; always 0 in high contrast, which has no tint)
    cap: f32,
    fill: (Color, Color),
}

impl Ink<'_> {
    /// The whole body: the tinted body (and the reflections of the open card) over the whole shape (they grow with it), the shared parts (cover,
    /// buttons, progress) travelling between their places, the capsule's line of text fading out as the card's lines, the source label, the times
    /// and the loudness bars fade in.
    pub fn paint(&self, cv: &Canvas, gpu: &mut Gpu) {
        let (st, p, pt) = (self.st, self.p, self.pt);
        let (capl, cardl) = (lg::capsule_layout_in(st, self.cap), lg::card_layout(self.card, st.style));
        let card_a = ((p - 0.3) / 0.7).clamp(0.0, 1.0);
        let cap_a = (1.0 - p * 3.6).clamp(0.0, 1.0);
        // the capsule's body is the fill style's, always: the cover's colour. As the card opens it eases to the card's own look for the style
        // (still the tint for the fill style); high contrast has none of it
        let q = p.clamp(0.0, 1.0);
        let cap = if self.hc { 0.0 } else { 1.0 - q * q * (3.0 - 2.0 * q) };
        let fc = (st.style == PlayerStyle::Fill && !self.hc) as u8 as f32;
        let fk = fc + (1.0 - fc) * cap;
        // the body's two colours: the capsule's calm tint, easing into the card's own (strong for the fill style) as it opens
        let (card_fill, soft) = (self.fill_colors(), soft_pair(self.th, self.tint, self.accent));
        let fill = (card_fill.0.mix(soft.0, cap), card_fill.1.mix(soft.1, cap));
        let (ink, inv) = (INK_LIGHT.mix(INK_DARK, self.ink_dark), INK_DARK.mix(INK_LIGHT, self.ink_dark));
        let look = Look { ink, inv, fk, cap, fill };
        let thc = self.themed(ink, fk);
        let th = &thc;
        // the ramp of the folded capsule is vertical; it swings to the card's own diagonal as it opens
        self.card_bg(cv, fk, fill, 118.0 + 62.0 * cap);
        if st.style == PlayerStyle::Mirror && !self.hc && self.cap.h >= REFL_MIN_H {
            // the cover's reflections belong to the open card (the folded capsule is one clean colour): they spread out of the cover as it opens
            let r = &pt.refl;
            player::paint_reflection(cv, self.images, self.blur, r[1], false, q);
            player::paint_reflection(cv, self.images, self.blur, r[0], true, q);
        }
        // the cover: the island's shadow under it, scaled with it
        if !self.hc {
            let k = pt.cover.w / 84.0;
            cv.shadow_rr(pt.cover, pt.cover_r, 16.0 * k, 6.0 * k, Color::rgba(0.0, 0.0, 0.0, 0.35));
        }
        let px = ((lg::COVER * st.bs() / 32.0).ceil().max(1.0) * 32.0) as u32;
        gpu.cover_sized(cv, self.th, self.art, self.url, pt.cover, self.now, pt.cover_r, px, self.under);
        // the capsule's line of text, riding up with the cover as the body rises
        if cap_a > 0.01 {
            let muted = th.text2.mix(th.text.fade(0.7), fk);
            cv.with_opacity(cap_a, |cv| self.capsule_text(cv, th, muted, capl.text.offset(0.0, pt.cover.y - capl.cover.y)));
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
        // the progress: the capsule's mini waveform becomes the loudness bars of the card, in the island's accent fill and track colours (the
        // cover tint's ink and its dim on the capsule)
        let tc = if self.hc { self.th.border } else { track_color(self.th) }.mix(ink.fade(0.28), fk);
        let acc = self.accent.mix(ink, fk);
        let mini_a = (1.0 - p * 2.6).clamp(0.0, 1.0);
        if mini_a > 0.01 && capl.bar.h > 0.0 {
            // the little waveform keeps its own size and only travels with the progress track's place
            let r = Rect::new(pt.bar.x, pt.bar.cy() - capl.bar.h / 2.0, capl.bar.w, capl.bar.h);
            cv.with_opacity(mini_a, |cv| self.mini_wave(cv, r, tc, acc));
        }
        if card_a > 0.01 {
            cv.with_opacity(card_a, |cv| self.bars(cv, pt.bar, card_a, tc, acc));
        }
        for b in BTNS {
            self.button(cv, gpu, th, b, pt.btn[b.index()], &look);
        }
        // the capsule's inner edge: one light (or dark) hairline inside the shape, so a body in the colour of whatever is behind it still reads
        if cap > 0.01 {
            cv.stroke_rr(self.body.inset(0.5), (self.rad - 0.5).max(0.0), 1.0, ink.fade(0.12 * cap));
        }
    }

    /// The card's two fill colours (`paint_player`): the cover's, mixed into the card colour; the accent when the cover has none.
    fn fill_colors(&self) -> (Color, Color) {
        fill_pair(self.th, self.tint, self.accent)
    }

    /// The colour of the fill gradient (`card_bg`, 118 degrees across the body) at the point `q`.
    fn body_at(&self, q: (f32, f32), (ca, cb): (Color, Color)) -> Color {
        let t = 118.0f32.to_radians();
        let (sx, sy) = (t.sin(), -t.cos());
        let b = self.body;
        let half = (b.w * sx.abs() + b.h * sy.abs()) / 2.0;
        let c = b.center();
        ca.mix(cb, (0.5 + ((q.0 - c.0) * sx + (q.1 - c.1) * sy) / (2.0 * half.max(1.0))).clamp(0.0, 1.0))
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
    /// the accent; the whole tint (`fk` of it) on the fill style and the folded capsule. The recipe is the card's, its rectangle is the body, so it grows with the shape.
    fn card_bg(&self, cv: &Canvas, fk: f32, (ca, cb): (Color, Color), deg: f32) {
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
        if fk > 0.004 {
            draw::linear_deg(cv, b, rad, deg, &[(0.0, ca.fade(fk)), (1.0, cb.fade(fk))]);
        }
    }

    /// The capsule's progress: the track's loudness as a few thin rounded bars (the card's 32 squashed into `lg::wave_n`), the ones the playhead has
    /// passed in the ink, the rest dim; a bar not measured yet is a short flat one, dimmer. The colour follows the playhead bar by bar (it glides
    /// with the progress spring) and the heights are the springs of `Loud`, so a bar that gets measured grows to its height.
    fn mini_wave(&self, cv: &Canvas, r: Rect, track: Color, fill: Color) {
        let n = lg::wave_n(self.cap.h);
        let bars = squash(&self.heights, &self.known, n);
        if !bars.iter().any(|b| b.1) {
            return self.mini_line(cv, r, track, fill);
        }
        let gap = r.w / (2 * n - 1) as f32;
        let head = self.frac.clamp(0.0, 1.0) * n as f32;
        let k = self.st.bs();
        let (lo, hi) = (r.h * 0.2, r.h * 0.92);
        // a bar's height on device px with the parity of the strip, so that it stands centred without a half-pixel row
        let rp = (r.h * k).round() as i32;
        for (i, &(h, known)) in bars.iter().enumerate() {
            let (x, w) = lg::wave_cell(r, n, gap, i);
            let nat = if known { lo + (hi - lo) * ((h - 0.2) / 0.8).clamp(0.0, 1.0) } else { r.h * 0.3 };
            let mut hp = (nat * k).round() as i32;
            if (rp - hp) % 2 != 0 {
                hp += 1;
            }
            let h = hp.max(1) as f32 / k;
            let (rest, played) = if known { (track, fill) } else { (track.fade(0.8), fill.fade(0.6)) };
            let col = rest.mix(played, (head - i as f32).clamp(0.0, 1.0));
            cv.fill_rr(Rect::new(x, r.cy() - h / 2.0, w, h), (w / 2.0).min(h / 2.0), col);
        }
    }

    /// The mini waveform of a track nothing is measured of yet (a stream, a first listen): not a row of dots but a calm thin rounded line, the played part
    /// of it in the ink, which the bars replace as they are learned.
    fn mini_line(&self, cv: &Canvas, r: Rect, track: Color, fill: Color) {
        let k = self.st.bs();
        // 2 device px, 3 when the strip is an odd number of them (so it stands centred on a whole pixel row)
        let h = if (r.h * k).round() as i32 % 2 == 0 { 2.0 } else { 3.0 } / k;
        let line = Rect::new(r.x, r.cy() - h / 2.0, r.w, h);
        cv.fill_rr(line, h / 2.0, track.fade(0.8));
        let w = (r.w * self.frac.clamp(0.0, 1.0)).max(if self.frac > 0.0 { h } else { 0.0 });
        if w > 0.0 {
            cv.fill_rr(Rect::new(r.x, line.y, w, h), h / 2.0, fill);
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

    /// The capsule's ONE line of text in `r`, the track change cross-fade included: the title (semi-bold), then the artist after a "· " in the muted
    /// colour. The artist gives way first (cut to what is left behind a whole title, dropped when that is hardly anything, `lg::bar_line`); a part
    /// that is cut has a faded end. The old text rises away and the new one rises in, the artist a beat later.
    fn capsule_text(&self, cv: &Canvas, th: &Theme, muted: Color, r: Rect) {
        let size = lg::capsule_text_size(self.cap.h);
        // soft: a medium title and a regular artist, the title a touch under the full ink (a white 600 on a dark body glares at this size)
        let (ts, ss) = (Style::new(size, 500), Style::new(size, 400));
        let title_ink = th.text.fade(0.9);
        let (kh, ks) = arrival(self.swap);
        let line = |cv: &Canvas, h: &str, s: &str, (kh, ks): (f32, f32), d: (f32, f32)| {
            let sub = if s.is_empty() { String::new() } else { format!("\u{b7} {s}") };
            let (wt, wa) = (cv.text_width(h, ts), if sub.is_empty() { 0.0 } else { cv.text_width(&sub, ss) });
            let (hw, subp) = lg::bar_line(r.w, wt, wa);
            cv.with_opacity(kh, |cv| faded(cv, Rect::new(r.x, r.y + d.0, hw.ceil() + 1.0, r.h).intersect_w(r.w), h, ts, title_ink));
            if let Some((sx, sw)) = subp {
                cv.with_opacity(ks, |cv| faded(cv, Rect::new(r.x + sx, r.y + d.1, sw.ceil() + 1.0, r.h).intersect_w(r.w - sx), &sub, ss, muted));
            }
        };
        for g in self.ghosts {
            let a = g.a.at01();
            let d = -3.0 * (1.0 - a);
            line(cv, &g.head, &g.sub, (a, a), (d, d));
        }
        line(cv, &self.track.heading(), &self.track.sub(), (kh, ks), (3.0 * (1.0 - kh), 3.0 * (1.0 - ks)));
    }

    /// A button, the card's way at the capsule's size too: previous / next are a plain icon with a hover disc; play is the Home card's main button. In
    /// the folded capsule (`look.cap`) it is a solid disc in the ink with the glyph cut out in the colour of the body behind it; opened, it is the
    /// card's own (a solid disc on the other styles, the card's ink on the fill tint, smoked on the mirror style).
    fn button(&self, cv: &Canvas, gpu: &mut Gpu, th: &Theme, b: Btn, r: Rect, look: &Look) {
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
                let mir = (self.st.style == PlayerStyle::Mirror) as u8 as f32 * (1.0 - look.cap);
                let (bg, fg) = (bg.mix(look.ink, look.fk).mix(Color::rgba(0.0, 0.0, 0.0, 0.52), mir), fg.mix(look.inv, look.fk).mix(Color::rgb8(255, 255, 255), mir));
                let fg = fg.mix(self.body_at(c, look.fill), look.cap);
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
