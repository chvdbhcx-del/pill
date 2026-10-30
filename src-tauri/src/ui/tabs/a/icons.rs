//! The line icons of `src/icons.js` (`PillIcons`, 24 x 24 grid, round caps) drawn with glasskit's `svgpath`. One parsed `Svg` per (name, stroke width)
//! lives in a thread-local cache; draw with [`draw`]. Names are the keys of `icons.js` (`home`, `clipboard`, `shelf`, `bell`, `play`, `pause`, ...).
use super::icons_data::ICONS;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::svgpath::{GeomCache, Svg};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `pillIcon`'s default stroke width.
pub const SW: f32 = 1.7;

thread_local! {
    static CACHE: RefCell<HashMap<(&'static str, u32), Rc<Svg>>> = RefCell::new(HashMap::new());
    static GEOM: Rc<GeomCache> = Rc::new(GeomCache::new());
}

fn markup(name: &str) -> Option<(&'static str, &'static str)> {
    ICONS.iter().find(|(n, _)| *n == name).copied()
}

/// Is there an icon of this name?
pub fn has(name: &str) -> bool {
    markup(name).is_some()
}

/// The icon `name` tinted with `color` (`currentColor`) fitted into `r`, stroke width `sw` (1.7 is the web default; 1.6 / 1.8 for tab buttons).
/// Unknown names draw nothing.
pub fn draw(cv: &Canvas, name: &str, r: Rect, color: Color, sw: f32) {
    let Some((key, inner)) = markup(name) else { return };
    let svg = CACHE.with(|c| c.borrow_mut().entry((key, (sw * 100.0).round() as u32)).or_insert_with(|| Rc::new(Svg::icon(inner, sw))).clone());
    GEOM.with(|g| svg.draw(cv, r, color, g));
}

/// Draw SVG inner markup that is not in `icons.js` (a tab's own inline icon), cached by `key`.
pub fn draw_markup(cv: &Canvas, key: &'static str, inner: &str, r: Rect, color: Color, sw: f32) {
    let svg = CACHE.with(|c| c.borrow_mut().entry((key, (sw * 100.0).round() as u32)).or_insert_with(|| Rc::new(Svg::icon(inner, sw))).clone());
    GEOM.with(|g| svg.draw(cv, r, color, g));
}

/// The shared geometry cache (for code that draws its own `Svg` values).
pub fn geom() -> Rc<GeomCache> {
    GEOM.with(|g| g.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_of_icons_js_parses_and_has_shapes() {
        assert!(ICONS.len() >= 60, "the generated table lost entries: {}", ICONS.len());
        for (n, inner) in ICONS {
            let svg = Svg::icon(inner, SW);
            assert!(svg.shape_count() > 0, "icon `{n}` has no shapes");
            assert!(svg.warnings().is_empty(), "icon `{n}`: {:?}", svg.warnings());
        }
        assert!(has("home") && has("bell") && !has("nope"));
    }
}
