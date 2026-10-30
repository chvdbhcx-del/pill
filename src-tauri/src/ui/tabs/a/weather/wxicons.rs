//! The coloured forecast icons of `tab-weather.js` (`PillWxIcon`: 48 x 48 grid, flat colours), built as SVG markup and drawn with glasskit's
//! `svgpath`. One parsed `Svg` per icon name lives in a thread-local cache.
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::svgpath::Svg;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CLOUD: &str = "M14 38h21a8 8 0 0 0 1.2-15.9A11 11 0 0 0 15.1 20 9 9 0 0 0 14 38z";

fn sun(cx: f32, cy: f32, r: f32) -> String {
    let mut rays = String::new();
    for a in [0.0f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0] {
        let (x, y) = (a.to_radians().cos(), a.to_radians().sin());
        rays += &format!("<path d=\"M{:.1} {:.1}L{:.1} {:.1}\"/>", cx + x * r * 1.5, cy + y * r * 1.5, cx + x * r * 1.95, cy + y * r * 1.95);
    }
    format!("<g stroke=\"#ffc83d\" stroke-width=\"{}\" stroke-linecap=\"round\">{rays}</g><circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"#ffc83d\"/>", r / 3.0)
}

fn moon(x: f32, y: f32, s: f32) -> String {
    format!(
        "<g transform=\"translate({x} {y}) scale({s})\"><path d=\"M20 3a15 15 0 1 0 15 21A12 12 0 0 1 20 3z\" fill=\"#dfe4ff\"/><circle cx=\"31\" cy=\"6\" r=\"1.5\" fill=\"#dfe4ff\"/><circle cx=\"36\" cy=\"13\" r=\"1.1\" fill=\"#dfe4ff\"/></g>"
    )
}

fn cloud(fill: &str, tx: f32, ty: f32, s: f32) -> String {
    format!("<path transform=\"translate({tx} {ty}) scale({s})\" d=\"{CLOUD}\" fill=\"{fill}\"/>")
}

/// Inner markup of the icon `name` (unknown names fall back to `cloud`, like the web).
pub fn markup(name: &str) -> String {
    match name {
        "clear-day" => sun(24.0, 24.0, 9.0),
        "clear-night" => moon(5.0, 6.0, 1.05),
        "partly-day" => sun(18.0, 17.0, 7.0) + &cloud("#e6eaf1", 5.0, 5.0, 0.9),
        "partly-night" => moon(2.0, 1.0, 0.8) + &cloud("#e6eaf1", 5.0, 5.0, 0.9),
        "fog" => cloud("#c9cfda", 0.0, -6.0, 0.95) + "<g stroke=\"#aab3c2\" stroke-width=\"3\" stroke-linecap=\"round\"><path d=\"M9 35h30M14 41h20\"/></g>",
        "rain" => cloud("#a2abbb", 0.0, -6.0, 0.95) + "<g stroke=\"#6fb3ff\" stroke-width=\"3\" stroke-linecap=\"round\"><path d=\"M17 36l-2.5 6M25 36l-2.5 6M33 36l-2.5 6\"/></g>",
        "sleet" => {
            cloud("#a2abbb", 0.0, -6.0, 0.95)
                + "<g stroke=\"#6fb3ff\" stroke-width=\"3\" stroke-linecap=\"round\"><path d=\"M17 36l-2.5 6M33 36l-2.5 6\"/></g><circle cx=\"25\" cy=\"39.5\" r=\"2.2\" fill=\"#fff\"/>"
        }
        "snow" => cloud("#a2abbb", 0.0, -6.0, 0.95) + "<g fill=\"#fff\"><circle cx=\"16\" cy=\"38\" r=\"2.3\"/><circle cx=\"25\" cy=\"42\" r=\"2.3\"/><circle cx=\"34\" cy=\"38\" r=\"2.3\"/></g>",
        "thunder" => cloud("#7b8496", 0.0, -6.0, 0.95) + "<path d=\"M27 28l-8 12h5.5l-2.5 8 10-13h-6z\" fill=\"#ffd23d\"/>",
        _ => cloud("#8b94a5", 7.0, -6.0, 0.85) + &cloud("#dfe3ea", 0.0, 2.0, 0.95),
    }
}

thread_local! {
    static CACHE: RefCell<HashMap<String, Rc<Svg>>> = RefCell::new(HashMap::new());
}

/// Draw the forecast icon `name` into `r`.
pub fn draw(cv: &Canvas, name: &str, r: Rect) {
    let svg = CACHE.with(|c| {
        c.borrow_mut().entry(name.to_string()).or_insert_with(|| Rc::new(Svg::parse(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 48 48\">{}</svg>", markup(name))))).clone()
    });
    svg.draw(cv, r, glasskit::color::WHITE, &super::super::icons::geom());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_parses_without_warnings() {
        for n in ["clear-day", "clear-night", "partly-day", "partly-night", "cloud", "fog", "rain", "sleet", "snow", "thunder", "unknown"] {
            let s = Svg::parse(&format!("<svg viewBox=\"0 0 48 48\">{}</svg>", markup(n)));
            assert!(s.shape_count() >= 2, "{n}");
            assert!(s.warnings().is_empty(), "{n}: {:?}", s.warnings());
        }
    }
}
