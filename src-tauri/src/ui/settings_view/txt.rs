//! Text helpers on top of `Canvas`: wrapped paragraphs with the web page's line pitch, single lines with ellipsis, runs with highlighted ranges.
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::text::{Align, Style, VAlign};
use std::ops::Range;

/// The line pitch DirectWrite uses for wrapped text of `size` (`text.rs`: `(size * 1.32).round()`).
fn dw_pitch(size: f32) -> f32 {
    (size * 1.32).round().max(1.0)
}

/// Number of lines `s` takes when wrapped to `w`.
pub fn lines(cv: &Canvas, s: &str, st: Style, w: f32) -> usize {
    if s.is_empty() {
        return 0;
    }
    let st = Style { wrap: true, valign: VAlign::Top, ..st };
    let (_, h) = cv.measure(s, st, w.max(8.0));
    ((h / dw_pitch(st.size)).round() as usize).max(1)
}

/// Height of `s` wrapped to `w` on a `pitch` px line grid (the CSS `line-height`).
pub fn wrapped_h(cv: &Canvas, s: &str, st: Style, w: f32, pitch: f32) -> f32 {
    lines(cv, s, st, w) as f32 * pitch
}

/// Wrapped text drawn from the top-left of `r`. DirectWrite's pitch is a pixel tighter than the page's `line-height`; the box is sized on the
/// page's grid by `wrapped_h` and the text starts on its first line.
pub fn para(cv: &Canvas, r: Rect, s: &str, st: Style, c: Color) {
    cv.text(r, s, Style { wrap: true, valign: VAlign::Top, align: Align::Left, ..st }, c);
}

/// One line, vertically centred in `r`, cut with an ellipsis.
pub fn line(cv: &Canvas, r: Rect, s: &str, st: Style, c: Color) {
    cv.text(r, s, Style { wrap: false, ellipsis: true, valign: VAlign::Middle, ..st }, c);
}

/// A line made of runs: `hits` are char ranges drawn in `hot` and `hot_weight`, the rest in `c`. Returns nothing; clipped to `r`.
pub fn highlighted(cv: &Canvas, r: Rect, s: &str, st: Style, c: Color, hits: &[Range<usize>], hot: Color, hot_weight: u16) {
    if hits.is_empty() {
        line(cv, r, s, st, c);
        return;
    }
    let chars: Vec<char> = s.chars().collect();
    let mut x = r.x;
    let mut at = 0usize;
    let mut runs: Vec<(String, bool)> = Vec::new();
    for h in hits {
        let (a, b) = (h.start.min(chars.len()), h.end.min(chars.len()));
        if a > at {
            runs.push((chars[at..a].iter().collect(), false));
        }
        if b > a {
            runs.push((chars[a..b].iter().collect(), true));
        }
        at = at.max(b);
    }
    if at < chars.len() {
        runs.push((chars[at..].iter().collect(), false));
    }
    cv.with_clip(r, |cv| {
        for (t, on) in runs {
            let stl = if on { Style { weight: hot_weight, ..st } } else { st };
            let w = cv.text_width(&t, Style { wrap: false, ..stl });
            cv.text(Rect::new(x, r.y, w + 2.0, r.h), &t, Style { wrap: false, ellipsis: false, valign: VAlign::Middle, ..stl }, if on { hot } else { c });
            x += w;
        }
    });
}

/// `s` upper-cased (the `.cap` captions are `text-transform: uppercase`).
pub fn upper(s: &str) -> String {
    s.to_uppercase()
}
