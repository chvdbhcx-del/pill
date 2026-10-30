//! The sidebar icons and caption glyphs: the inner SVG markup of `settings.js` `IC` (24x24 line icons), drawn with `glasskit::svgpath`.
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::svgpath::{GeomCache, Svg};
use std::collections::HashMap;

const IC: [(&str, &str); 16] = [
    ("character", r##"<circle cx="12" cy="12" r="9"/><path d="M8.5 14a4.2 4.2 0 0 0 7 0"/><path d="M9 9.5h.01M15 9.5h.01"/>"##),
    ("general", r##"<path d="M21 6h-9M8 6H3M21 12h-3M14 12H3M21 18h-9M8 18H3"/><circle cx="10" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="18" r="2"/>"##),
    ("hotkeys", r##"<rect x="2.5" y="5.5" width="19" height="13" rx="2.5"/><path d="M6.5 10h.01M10 10h.01M14 10h.01M17.5 10h.01M7 14.5h10"/>"##),
    ("ai", r##"<rect x="4" y="8" width="16" height="12" rx="3"/><path d="M12 8V4.5M9 13.5h.01M15 13.5h.01M2.5 14v2M21.5 14v2"/><circle cx="12" cy="4" r="1"/>"##),
    ("voice", r##"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>"##),
    ("weather", r##"<path d="M12 2v2M4.9 4.9l1.4 1.4M20 12h2M19.1 4.9l-1.4 1.4M15.9 12.6a4 4 0 0 0-5.9-4.1"/><path d="M13 22H7a5 5 0 1 1 4.9-6H13a3 3 0 0 1 0 6z"/>"##),
    ("sound", r##"<path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>"##),
    ("live", r##"<path d="M13 2.5 5 13.5h6l-1 8 8-11h-6z"/>"##),
    ("clipboard", r##"<rect x="5" y="4.5" width="14" height="17" rx="3"/><path d="M9 4.5V4a1.5 1.5 0 0 1 1.5-1.5h3A1.5 1.5 0 0 1 15 4v.5"/><path d="M9 11h.01M9 15.5h.01M12.5 11H15M12.5 15.5H15"/>"##),
    ("modules", r##"<rect x="3.5" y="3.5" width="7" height="7" rx="1.8"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.8"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.8"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.8"/>"##),
    ("updates", r##"<path d="M20 11a8 8 0 0 0-14.5-4M4 4v4h4M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4"/>"##),
    ("aisetup", r##"<path d="M12 3.5v11M7.5 10.5l4.5 4.5 4.5-4.5M4.5 15.5v3a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-3"/>"##),
    ("appearance", r##"<circle cx="12" cy="12" r="4"/><path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M18.4 5.6 17 7M7 17l-1.4 1.4"/>"##),
    ("behavior", r##"<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>"##),
    ("search", r##"<circle cx="10.5" cy="10.5" r="6.5"/><path d="M15.5 15.5 21 21"/>"##),
    ("power", r##"<path d="M12 3v8"/><path d="M6.8 6.6a8 8 0 1 0 10.4 0"/>"##),
];

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
        for (k, v) in IC {
            side.insert(k, Svg::icon(v, 1.6));
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
