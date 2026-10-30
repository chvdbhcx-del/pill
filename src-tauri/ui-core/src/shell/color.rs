//! Colours: RGBA, the theme palette (theme.css tokens the shell uses) and a small CSS colour parser for strings that
//! live rows / reactions pass in (`#rgb`, `#rrggbb[aa]`, `rgb()/rgba()`, `hsl()`, `color(srgb ..)`, `var(--token)`, a few names).
//! Unparseable input returns `None`; the JS `rgbOf` then fell back to `110,110,110` ([`FALLBACK_RGB`]).

/// Straight (non-premultiplied) colour; `a` in 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

pub const FALLBACK_RGB: (u8, u8, u8) = (110, 110, 110);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 1.0 }
    }
    pub const fn with_alpha(self, a: f32) -> Rgba {
        Rgba { a, ..self }
    }
    pub fn rgb_tuple(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
    pub fn from_hex(h: u32) -> Rgba {
        Rgba::rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

/// theme.css tokens used by the shell (`--bg --edge --text --accent --green --red --amber`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub bg: Rgba,
    pub edge: Rgba,
    pub text: Rgba,
    pub accent: Rgba,
    pub green: Rgba,
    pub red: Rgba,
    pub amber: Rgba,
}

pub fn palette(t: Theme) -> Palette {
    match t {
        Theme::Dark => Palette {
            bg: Rgba::from_hex(0x000000),
            edge: Rgba::from_hex(0x6e6e6e),
            text: Rgba::from_hex(0xffffff),
            accent: Rgba::from_hex(0x8f96ff),
            green: Rgba::from_hex(0x58ddab),
            red: Rgba::from_hex(0xff453a),
            amber: Rgba::from_hex(0xffb340),
        },
        Theme::Light => Palette {
            bg: Rgba::from_hex(0xf6f6f6),
            edge: Rgba::from_hex(0xb9b9b9),
            text: Rgba::from_hex(0x161616),
            accent: Rgba::from_hex(0x5b63e6),
            green: Rgba::from_hex(0x11805a),
            red: Rgba::from_hex(0xd92d20),
            amber: Rgba::from_hex(0xb26a00),
        },
    }
}

impl Palette {
    /// A theme token by name (`accent`, `green`, `red`, `amber`, `edge`, `wave`, `bg`, `text`).
    pub fn token(&self, name: &str) -> Option<Rgba> {
        Some(match name.trim().trim_start_matches("--") {
            "accent" => self.accent,
            "green" => self.green,
            "red" => self.red,
            "amber" => self.amber,
            "edge" | "wave" => self.edge,
            "bg" => self.bg,
            "text" => self.text,
            _ => return None,
        })
    }
    /// pill.css `.tc[data-tone=..] { --tone: var(--..) }`: the card tone colour (None = no tone).
    pub fn tone(&self, tone: &str) -> Option<Rgba> {
        match tone {
            "amber" | "green" | "red" | "accent" => self.token(tone),
            _ => None,
        }
    }
}

fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(p) = s.strip_suffix('%') {
        return p.trim().parse::<f64>().ok().map(|v| v / 100.0);
    }
    s.parse::<f64>().ok()
}

fn split_args(inner: &str) -> Vec<String> {
    inner.replace(['/', ','], " ").split_whitespace().map(|s| s.to_string()).collect()
}

fn chan255(s: &str) -> Option<u8> {
    let s = s.trim();
    let v = if let Some(p) = s.strip_suffix('%') { p.trim().parse::<f64>().ok()? * 2.55 } else { s.parse::<f64>().ok()? };
    Some(v.round().clamp(0.0, 255.0) as u8)
}

fn alpha_of(s: Option<&String>) -> Option<f32> {
    match s {
        None => Some(1.0),
        Some(a) => parse_num(a).map(|v| v.clamp(0.0, 1.0) as f32),
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0) / 360.0;
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// Parse a CSS colour; `pal` resolves `var(--accent)` style references.
pub fn parse_css(input: &str, pal: &Palette) -> Option<Rgba> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let low = s.to_ascii_lowercase();
    if let Some(h) = low.strip_prefix('#') {
        let d: Option<Vec<u32>> = h.chars().map(|c| c.to_digit(16)).collect();
        let d = d?;
        return match d.len() {
            3 | 4 => {
                let c = |i: usize| (d[i] * 17) as u8;
                Some(Rgba { r: c(0), g: c(1), b: c(2), a: if d.len() == 4 { c(3) as f32 / 255.0 } else { 1.0 } })
            }
            6 | 8 => {
                let c = |i: usize| (d[i] * 16 + d[i + 1]) as u8;
                Some(Rgba { r: c(0), g: c(2), b: c(4), a: if d.len() == 8 { c(6) as f32 / 255.0 } else { 1.0 } })
            }
            _ => None,
        };
    }
    match low.as_str() {
        "white" => return Some(Rgba::rgb(255, 255, 255)),
        "black" => return Some(Rgba::rgb(0, 0, 0)),
        "transparent" => return Some(Rgba { r: 0, g: 0, b: 0, a: 0.0 }),
        "currentcolor" => return Some(pal.text),
        _ => {}
    }
    let open = low.find('(')?;
    let close = low.rfind(')')?;
    if close < open {
        return None;
    }
    let (func, inner) = (low[..open].trim(), &low[open + 1..close]);
    match func {
        "var" => {
            let mut parts = inner.splitn(2, ',');
            let name = parts.next()?.trim();
            pal.token(name).or_else(|| parts.next().and_then(|fb| parse_css(fb, pal)))
        }
        "rgb" | "rgba" => {
            let a = split_args(inner);
            if a.len() < 3 {
                return None;
            }
            Some(Rgba { r: chan255(&a[0])?, g: chan255(&a[1])?, b: chan255(&a[2])?, a: alpha_of(a.get(3))? })
        }
        "hsl" | "hsla" => {
            let a = split_args(inner);
            if a.len() < 3 {
                return None;
            }
            let h = a[0].trim_end_matches("deg").parse::<f64>().ok()?;
            let (r, g, b) = hsl_to_rgb(h, parse_num(&a[1])?, parse_num(&a[2])?);
            Some(Rgba { r, g, b, a: alpha_of(a.get(3))? })
        }
        "color" => {
            let a = split_args(inner);
            if a.first().map(|s| s.as_str()) != Some("srgb") || a.len() < 4 {
                return None;
            }
            let c = |s: &str| parse_num(s).map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
            Some(Rgba { r: c(&a[1])?, g: c(&a[2])?, b: c(&a[3])?, a: alpha_of(a.get(4))? })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_forms() {
        let p = palette(Theme::Dark);
        assert_eq!(parse_css("#d8836a", &p), Some(Rgba::rgb(0xd8, 0x83, 0x6a)));
        assert_eq!(parse_css("#fff", &p), Some(Rgba::rgb(255, 255, 255)));
        assert_eq!(parse_css("#11223380", &p).unwrap().a, 128.0 / 255.0);
        assert_eq!(parse_css("#12", &p), None);
        assert_eq!(parse_css("#ggg", &p), None);
    }

    #[test]
    fn functions_and_vars() {
        let p = palette(Theme::Dark);
        assert_eq!(parse_css("rgb(10, 20, 30)", &p), Some(Rgba::rgb(10, 20, 30)));
        assert_eq!(parse_css("rgba(10 20 30 / 0.5)", &p), Some(Rgba { r: 10, g: 20, b: 30, a: 0.5 }));
        assert_eq!(parse_css("color(srgb 1 0 0)", &p), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(parse_css("hsl(0, 100%, 50%)", &p), Some(Rgba::rgb(255, 0, 0)));
        assert_eq!(parse_css("var(--accent)", &p), Some(p.accent));
        assert_eq!(parse_css("var(--nope, #000)", &p), Some(Rgba::rgb(0, 0, 0)));
        assert_eq!(parse_css("var(--nope)", &p), None);
        assert_eq!(parse_css("banana", &p), None);
        assert_eq!(parse_css("", &p), None);
    }

    #[test]
    fn palettes_differ_by_theme() {
        assert_eq!(palette(Theme::Dark).bg, Rgba::rgb(0, 0, 0));
        assert_eq!(palette(Theme::Light).edge, Rgba::from_hex(0xb9b9b9));
        assert_eq!(palette(Theme::Dark).tone("amber"), Some(palette(Theme::Dark).amber));
        assert_eq!(palette(Theme::Dark).tone("zzz"), None);
    }
}
