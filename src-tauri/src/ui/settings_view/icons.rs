//! The sidebar icons and caption glyphs: the inner SVG markup of `settings.js` `IC` (24x24 line icons) plus the island's line-icon set, drawn with `glasskit::svgpath`.
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::svgpath::{GeomCache, Svg};
use std::collections::HashMap;

/// The sidebar glyphs the island's line-icon set (`island::gpu::Gpu::glyph`, same 24 grid, same style) does not have, or has differently.
/// Every id of `nav::ICONS` is here or there; `every_sidebar_icon_has_a_glyph` holds the two lists together.
const IC: [(&str, &str); 14] = [
    ("character", r##"<circle cx="12" cy="12" r="9"/><path d="M8.5 14a4.2 4.2 0 0 0 7 0"/><path d="M9 9.5h.01M15 9.5h.01"/>"##),
    ("general", r##"<path d="M21 6h-9M8 6H3M21 12h-3M14 12H3M21 18h-9M8 18H3"/><circle cx="10" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="18" r="2"/>"##),
    ("weather", r##"<path d="M12 2v2M4.9 4.9l1.4 1.4M20 12h2M19.1 4.9l-1.4 1.4M15.9 12.6a4 4 0 0 0-5.9-4.1"/><path d="M13 22H7a5 5 0 1 1 4.9-6H13a3 3 0 0 1 0 6z"/>"##),
    ("updates", r##"<path d="M20 11a8 8 0 0 0-14.5-4M4 4v4h4M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4"/>"##),
    ("palette", r##"<path d="M12 3a9 9 0 1 0 0 18c1.4 0 2-.9 2-1.8 0-1.4-1.3-1.6-1.3-2.9 0-1 .8-1.6 1.8-1.6H17a4 4 0 0 0 4-4C21 6.6 17 3 12 3z"/><path d="M7.5 11h.01M10 7.5h.01M14.5 7.5h.01"/>"##),
    ("paw", r##"<circle cx="5.5" cy="10.5" r="1.6"/><circle cx="9" cy="6" r="1.6"/><circle cx="15" cy="6" r="1.6"/><circle cx="18.5" cy="10.5" r="1.6"/><path d="M12 12c-2.8 0-5 2.8-5 5.2 0 1.5 1.1 2.3 2.5 2.3 1 0 1.6-.5 2.5-.5s1.5.5 2.5.5c1.4 0 2.5-.8 2.5-2.3 0-2.4-2.2-5.2-5-5.2z"/>"##),
    ("clock", r##"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.2 2"/>"##),
    ("motion", r##"<path d="M3.5 20.5C5 8 12 5.5 20.5 3.5"/><circle cx="3.5" cy="20.5" r="1.4"/><circle cx="20.5" cy="3.5" r="1.4"/>"##),
    ("desktops", r##"<rect x="3" y="8" width="13" height="11" rx="2"/><path d="M8 5h11a2 2 0 0 1 2 2v9"/>"##),
    ("obs", r##"<circle cx="12" cy="12" r="2.5"/><path d="M7.8 7.8a6 6 0 0 0 0 8.4M16.2 7.8a6 6 0 0 1 0 8.4M5 5a10 10 0 0 0 0 14M19 5a10 10 0 0 1 0 14"/>"##),
    ("chip", r##"<rect x="6" y="6" width="12" height="12" rx="2"/><rect x="9.5" y="9.5" width="5" height="5" rx="1"/><path d="M9 2.5V6M15 2.5V6M9 18v3.5M15 18v3.5M2.5 9H6M2.5 15H6M18 9h3.5M18 15h3.5"/>"##),
    ("clipboard", r##"<rect x="5" y="4.5" width="14" height="17" rx="3"/><path d="M9 4.5V4a1.5 1.5 0 0 1 1.5-1.5h3A1.5 1.5 0 0 1 15 4v.5"/><path d="M9 11h.01M9 15.5h.01M12.5 11H15M12.5 15.5H15"/>"##),
    ("phone", r##"<rect x="6.5" y="2.5" width="11" height="19" rx="2.8"/><path d="M10.5 18.5h3"/>"##),
    ("power", r##"<path d="M12 3v8"/><path d="M6.8 6.6a8 8 0 1 0 10.4 0"/>"##),
];

/// The markup of sidebar icon `id`: the window's own glyph, else the island's.
pub fn glyph(id: &str) -> Option<&'static str> {
    IC.iter().find(|(k, _)| *k == id).map(|(_, v)| *v).or_else(|| crate::ui::island::gpu::Gpu::glyph(id))
}

/// Caption glyphs (10x10 boxes, stroke 1): minimise, maximise, restore, close.
const CAP: [(&str, &str); 4] = [
    ("min", r##"<path d="M0 5.5h10"/>"##),
    ("max", r##"<rect x=".5" y=".5" width="9" height="9" rx="1.2"/>"##),
    ("rest", r##"<rect x=".5" y="2.5" width="7" height="7" rx="1.2"/><path d="M2.5 2.5V1.7A1.2 1.2 0 0 1 3.7.5h4.6a1.2 1.2 0 0 1 1.2 1.2v4.6a1.2 1.2 0 0 1-1.2 1.2h-.8"/>"##),
    ("close", r##"<path d="M.5.5l9 9M9.5.5l-9 9"/>"##),
];

/// Small extra glyphs used by rows and widgets (24 box): check, chevron, cross.
const EXTRA: [(&str, &str); 3] = [
    ("check", r##"<path d="M20 6 9 17l-5-5"/>"##),
    ("chev", r##"<path d="M9 5l7 7-7 7"/>"##),
    ("x", r##"<path d="M6 6l12 12M18 6 6 18"/>"##),
];

pub struct Icons {
    side: HashMap<&'static str, Svg>,
    cap: HashMap<&'static str, Svg>,
    extra: HashMap<&'static str, Svg>,
    cache: GeomCache,
}

impl Icons {
    pub fn new() -> Icons {
        let mut side = HashMap::new();
        for k in pill_ui_core::settings::nav::ICONS {
            if let Some(v) = glyph(k) {
                side.insert(k, Svg::icon(v, 1.6));
            }
        }
        let mut cap = HashMap::new();
        for (k, v) in CAP {
            let doc = format!("<svg viewBox=\"0 0 10 10\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1\" stroke-linecap=\"round\" stroke-linejoin=\"round\">{v}</svg>");
            cap.insert(k, Svg::parse(&doc));
        }
        let mut extra = HashMap::new();
        for (k, v) in EXTRA {
            extra.insert(k, Svg::icon(v, 2.0));
        }
        Icons { side, cap, extra, cache: GeomCache::new() }
    }

    pub fn side(&self, cv: &Canvas, id: &str, r: Rect, c: Color) {
        if let Some(s) = self.side.get(id) {
            s.draw(cv, r, c, &self.cache);
        }
    }
    pub fn cap(&self, cv: &Canvas, id: &str, r: Rect, c: Color) {
        if let Some(s) = self.cap.get(id) {
            s.draw(cv, r, c, &self.cache);
        }
    }
    pub fn extra(&self, cv: &Canvas, id: &str, r: Rect, c: Color) {
        if let Some(s) = self.extra.get(id) {
            s.draw(cv, r, c, &self.cache);
        }
    }
}

impl Default for Icons {
    fn default() -> Self {
        Icons::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sidebar_icon_has_a_glyph() {
        for id in pill_ui_core::settings::nav::ICONS {
            assert!(glyph(id).is_some(), "no glyph for sidebar icon {id}");
        }
    }
}
