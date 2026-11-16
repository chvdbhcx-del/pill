//! The pixels of the music widget (a child module of `view`). The look is the island's own player, drawn with its own pieces: the open card is the
//! Home player card (`tabs::a::home::paint::paint_player`: the cover-tinted background, the blurred reflections of the cover at its sides, the
//! cover with its shadow, the source label, the centred title and artist line, the progress track with the times, the transport over / beside the
//! cover) and the folded capsule is the island's mini music strip (cover, one line of text, three plain round icon buttons, `island::rows`).
//! Colours, radii and text styles are the theme's and the island's; the only new thing is the progress track of the card, which shows the
//! loudness of the track as thin rounded bars in the track's place and size.
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
use pill_ui_core::panel::a::home_player::PlayerStyle;

/// The island's text styles: the music row's name (`rows::text_style(Name)`), the Home card's title (16 / 600), artist line (13 / 400), source
/// label and times.
pub(super) const NAME: Style = Style::new(13.0, 600).ellipsis();
const SUB: Style = Style::new(13.0, 400).ellipsis();
const TITLE: Style = Style::new(16.0, 600).ellipsis();
const ARTIST: Style = Style::new(13.0, 400).ellipsis();
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
    pub cap: Rect,
    pub card: Rect,
    pub art: u64,
    pub under: Option<u64>,
    pub url: Option<&'a str>,
    /// the blurred reflections (`tiny_cover` of the cover) and the geometry the island draws them in
    pub images: &'a ImageCache,
    pub blur: Option<&'a str>,
    pub fx: &'a player::PlayerFx,
}

impl Ink<'_> {
    /// The whole body: what only the card has (the tinted background, the reflections, the lines, the times, the bars) fades in over the shared
    /// parts (the cover, the buttons, the progress), what only the capsule has (its line) fades out.
    pub fn paint(&self, cv: &Canvas, gpu: &mut Gpu) {
        let (st, p, pt) = (self.st, self.p, self.pt);
        let (capl, cardl) = (lg::capsule_layout_in(st, self.cap), lg::card_layout(self.card, st.style));
        let card_a = ((p - 0.3) / 0.7).clamp(0.0, 1.0);
        let cap_a = (1.0 - p * 1.7).clamp(0.0, 1.0);
        // the fill style turns the card's text light or dark with its tint
        let fill = st.style == PlayerStyle::Fill && !self.hc;
        let (ca, cb) = self.fill_colors();
        let ink = player::ink_for(ca.mix(cb, 0.5));
        let fk = if fill { card_a } else { 0.0 };
        let thc = self.themed(ink, fk);
        let rad = lg::CARD_R;
        if card_a > 0.01 {
            cv.with_opacity(card_a, |cv| {
                self.card_bg(cv, rad, fill, (ca, cb));
                if st.style == PlayerStyle::Mirror && !self.hc {
                    self.fx.paint_reflections(cv, self.images, self.blur, self.card.x, self.card.y);
                }
            });
        }
        // the cover: the island's shadow grows under it with the card
        if card_a > 0.01 && !self.hc {
            cv.with_opacity(card_a, |cv| cv.shadow_rr(pt.cover, pt.cover_r, 16.0, 6.0, Color::rgba(0.0, 0.0, 0.0, 0.35)));
        }
        let px = ((lg::COVER * st.bs() / 32.0).ceil().max(1.0) * 32.0) as u32;
        gpu.cover_sized(cv, self.th, self.art, self.url, pt.cover, self.now, pt.cover_r, px, self.under);
        // the capsule's one line (the island's music row: the name, then the artist), riding up with the cover as the body rises
        if cap_a > 0.01 {
            cv.with_opacity(cap_a, |cv| self.line(cv, capl.text.offset(0.0, pt.cover.y - capl.cover.y), NAME, SUB));
        }
        if card_a > 0.01 {
            cv.with_opacity(card_a, |cv| {
                let th = &thc;
                cv.text(cardl.src, &lg::app_name(&self.track.app).to_uppercase(), SRC, th.dim);
                let c = cardl.centered;
                let (ts, as_) = if c { (TITLE.center(), ARTIST.center()) } else { (TITLE, ARTIST) };
                self.lines(cv, th, cardl.title, cardl.sub, ts, as_);
                cv.text(cardl.elapsed, &lg::clock_text(self.pos), TIMES, th.dim);
                let rem = if self.track.duration_ms > 0 { format!("-{}", lg::clock_text(self.track.duration_ms.saturating_sub(self.pos))) } else { "--:--".to_string() };
                cv.text(cardl.total, &rem, TIMES.right(), th.dim);
            });
        }
        // the progress: the capsule's hairline (the island's 4 px track, thinner) becomes the loudness bars of the card
        let tc = track_color(self.th).mix(ink.fade(0.28), fk);
        let acc = self.accent.mix(ink, fk);
        if card_a < 0.99 {
            cv.with_opacity(1.0 - card_a, |cv| self.hairline(cv, pt.bar, tc, acc));
        }
        if card_a > 0.01 {
            cv.with_opacity(card_a, |cv| self.bars(cv, pt.bar, card_a, tc, acc));
        }
        for b in BTNS {
            self.button(cv, gpu, b, pt.btn[b.index()], card_a, ink, fk);
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

    /// The Home card's background: the cover's gradient over the card colour, else a tinted corner glow of the accent; the whole tint on the fill style.
    fn card_bg(&self, cv: &Canvas, rad: f32, fill: bool, (ca, cb): (Color, Color)) {
        let (th, card) = (self.th, self.card);
        cv.fill_rr(card, rad, th.card);
        if self.hc {
            return;
        }
        if let Some((a, b)) = self.tint {
            draw::linear_deg(cv, card, rad, 100.0, &[(0.0, css::mix(a, 30.0, th.card)), (0.28, css::mix(b, 14.0, th.card)), (0.52, th.card), (1.0, th.card)]);
        } else {
            let base = if th.dark { Color::rgb8(0x1b, 0x1b, 0x1b) } else { Color::rgb8(255, 255, 255) };
            let mixed = css::mix(self.accent, if th.dark { 17.0 } else { 22.0 }, base);
            draw::radial(cv, card, rad, (card.x, card.y), card.w * 1.3, card.h * 1.5, &[(0.0, mixed), (0.72, th.card), (1.0, th.card)]);
        }
        if fill {
            draw::linear_deg(cv, card, rad, 118.0, &[(0.0, ca), (1.0, cb)]);
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

    /// The capsule's ONE line, the track change cross-fade included: the name and then the artist in the dimmer ink after it (`r` is centred
    /// vertically by the style). The artist gives way first (`lg::bar_line`): it is cut to what a whole title leaves, and dropped before the title
    /// is cut.
    fn line(&self, cv: &Canvas, r: Rect, hs: Style, ss: Style) {
        let th = self.th;
        let k = arrival(self.swap).0;
        let pair = |cv: &Canvas, h: &str, s: &str, dy: f32| {
            let (hw, sub) = lg::bar_line(r.w, cv.text_width(h, hs), if s.is_empty() { 0.0 } else { cv.text_width(s, ss) });
            // a hair of slack: a layout box exactly as wide as its text may still decide to cut it
            cv.text(Rect::new(r.x, r.y + dy, (hw.ceil() + 1.0).min(r.w), r.h), h, hs.ellipsis(), th.text);
            if let Some((sx, sw)) = sub {
                cv.text(Rect::new(r.x + sx, r.y + dy, (sw.ceil() + 1.0).min(r.w - sx), r.h), s, ss.ellipsis(), th.text2);
            }
        };
        for g in self.ghosts {
            let a = g.a.at01();
            cv.with_opacity(a, |cv| pair(cv, &g.head, &g.sub, -3.0 * (1.0 - a)));
        }
        cv.with_opacity(k, |cv| pair(cv, &self.track.heading(), &self.track.sub(), 3.0 * (1.0 - k)));
    }

    /// A button, the island's way: previous / next are a plain icon with a hover disc; the play button too in the capsule (the island's mini row),
    /// and in the card it is the Home card's main button, a smoked disc over the cover (a solid disc beside it, on the fill tint the card's ink).
    #[allow(clippy::too_many_arguments)]
    fn button(&self, cv: &Canvas, gpu: &mut Gpu, b: Btn, r: Rect, open: f32, ink: Color, fk: f32) {
        let th = self.th;
        let (h, q) = (self.hov[b.index()].at01(), self.press[b.index()].at().clamp(0.0, 1.0));
        let c = r.center();
        let size = r.w * 0.64;
        let ir = r.centered(size, size);
        // the island's hover disc: 14 px round a 28 px button
        let hov = |cv: &Canvas, k: f32| cv.fill_circle(c, r.w / 2.0 * (1.0 - 0.12 * q), th.hover.fade(k));
        match b {
            Btn::Prev | Btn::Next => {
                let kick = if b == Btn::Prev { self.kick[0] } else { self.kick[1] };
                cv.with_scale_about(c, 1.0 - 0.06 * q, |cv| {
                    hov(cv, h);
                    gpu.icon(cv, if b == Btn::Prev { "prev" } else { "next" }, ir.offset(kick, 0.0), th.text2.mix(th.text, h).mix(th.text, 1.0 - open), 1.7);
                });
            }
            Btn::Play => {
                let dark = th.dark;
                let (bg, fg) = if dark { (Color::rgb8(255, 255, 255), Color::rgb8(0, 0, 0)) } else { (Color::rgb8(0x15, 0x15, 0x15), Color::rgb8(255, 255, 255)) };
                let mir = (self.st.style == PlayerStyle::Mirror) as u8 as f32;
                let (bg, fg) = (bg.mix(ink, fk).mix(Color::rgba(0.0, 0.0, 0.0, 0.52), mir), fg.mix(player::ink_inverse(ink), fk).mix(Color::rgb8(255, 255, 255), mir));
                let s = (1.0 - 0.06 * q) * (1.0 + 0.06 * h * open);
                cv.with_scale_about(c, s, |cv| {
                    // folded: the music row's plain icon with the hover disc; open: the disc fades in under the glyph and the glyph takes its colour
                    hov(cv, h * (1.0 - open));
                    cv.fill_circle(c, r.w / 2.0, bg.fade(open));
                    let ic = th.text.mix(fg, open);
                    // the play and pause icons cross-fade, the leaving one shrinking away and the arriving one growing in, with the spring's pop
                    let k = self.pp.clamp(0.0, 1.0);
                    let pop = 1.0 + 0.10 * (self.pp - 1.0).abs().min(1.0) * (self.pp > 1.0) as u8 as f32;
                    cv.with_scale_about(c, pop, |cv| {
                        cv.with_opacity(1.0 - k, |cv| cv.with_scale_about(c, 1.0 - 0.3 * k, |cv| gpu.icon(cv, "play", ir, ic, 1.7)));
                        cv.with_opacity(k, |cv| cv.with_scale_about(c, 0.7 + 0.3 * k, |cv| gpu.icon(cv, "pause", ir, ic, 1.7)));
                    });
                });
            }
        }
    }
}
