//! The two CSS colour spellings the home model hands out: `#rrggbb` (cover accent) and `hsl(h, s%, l%)` (cover gradient), plus the
//! `color-mix(in srgb, a p%, b)` the card backgrounds use.
use glasskit::color::Color;

/// `#rgb`, `#rrggbb` or `hsl(h, s%, l%)`; `None` for anything else (an empty accent, `var(--accent)`).
pub fn parse(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n)?, 16).ok();
        return match h.len() {
            6 => Some(Color::rgb8(v(0, 2)?, v(2, 2)?, v(4, 2)?)),
            3 => Some(Color::rgb8(v(0, 1)? * 17, v(1, 1)? * 17, v(2, 1)? * 17)),
            _ => None,
        };
    }
    let inner = s.strip_prefix("hsl(")?.strip_suffix(')')?;
    let mut it = inner.split(',').map(|p| p.trim().trim_end_matches('%').parse::<f32>().ok());
    let (h, sat, l) = (it.next()??, it.next()?? / 100.0, it.next()?? / 100.0);
    Some(hsl(h, sat, l))
}

pub fn hsl(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::rgba(r + m, g + m, b + m, 1.0)
}

/// `color-mix(in srgb, a pct%, b)`: `pct` of `a`, the rest `b`.
pub fn mix(a: Color, pct: f32, b: Color) -> Color {
    b.mix(a, pct / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Color, r: u8, g: u8, b: u8) -> bool {
        let t = |x: f32, y: u8| (x * 255.0 - y as f32).abs() < 1.5;
        t(a.r, r) && t(a.g, g) && t(a.b, b)
    }

    #[test]
    fn hex_and_hsl_spellings_parse() {
        assert!(near(parse("#aa3355").unwrap(), 0xaa, 0x33, 0x55));
        assert!(near(parse("#f80").unwrap(), 0xff, 0x88, 0x00));
        assert!(near(parse("hsl(0, 100%, 50%)").unwrap(), 255, 0, 0));
        assert!(near(parse("hsl(120, 100%, 25%)").unwrap(), 0, 128, 0));
        assert!(near(parse("hsl(240, 50%, 50%)").unwrap(), 64, 64, 191));
        assert!(parse("var(--accent)").is_none() && parse("").is_none() && parse("#12").is_none());
    }

    #[test]
    fn mix_takes_the_percentage_of_the_first() {
        let m = mix(Color::rgb8(255, 0, 0), 25.0, Color::rgb8(0, 0, 0));
        assert!(near(m, 64, 0, 0));
    }
}
