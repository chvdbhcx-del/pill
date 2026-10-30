//! Toolbar icons: the inner SVG markup of `capture.js` (`own`) and `icons.js` (24 x 24 grid, 1.7 stroke, round caps), plus drawings of
//! the four native-extension tools (highlighter, ellipse, numbered step, redo). Painted by glasskit's SVG reader, cached per device.
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::svgpath::{GeomCache, Svg};
use glasskit::Canvas;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Inner markup for an icon name of `ToolbarVm` (`ItemVm::icon`).
pub fn markup(name: &str) -> &'static str {
    match name {
        // capture.js `own`
        "select" => r#"<path d="m5 3 14 7.500-6.200 2L10.500 19z"/>"#,
        "rect" => r#"<rect x="4" y="5.500" width="16" height="13" rx="2"/>"#,
        "arrow" => r#"<path d="M5 19 19 5M9 5h10v10"/>"#,
        "pen" => r#"<path d="M3 17c3-6 5-9 7-9s0 8 3 8 3-6 8-9"/>"#,
        "blur" => r#"<rect x="4" y="4" width="6" height="6"/><rect x="14" y="4" width="6" height="6" opacity=".45"/><rect x="9" y="9" width="6" height="6"/><rect x="4" y="14" width="6" height="6" opacity=".45"/><rect x="14" y="14" width="6" height="6"/>"#,
        "text" => r#"<path d="M5 7V4.500h14V7M12 4.500v15M9 19.500h6"/>"#,
        "undo" => r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>"#,
        "save" => r#"<path d="M5 3h11l3 3v15H5z"/><path d="M8 3v6h8V3M8 21v-7h8v7"/>"#,
        // icons.js
        "copy" => r#"<rect x="8.500" y="8.500" width="12" height="12" rx="2.500"/><path d="M15.500 8.500V6a2.500 2.500 0 0 0-2.500-2.500H6A2.500 2.500 0 0 0 3.500 6v7A2.500 2.500 0 0 0 6 15.500h2.500"/>"#,
        "video" => r#"<path d="m16 13 5.200 3.500a.5.5 0 0 0 .8-.4V7.900a.5.5 0 0 0-.8-.4L16 11"/><rect x="2" y="6" width="14" height="12" rx="2"/>"#,
        "check" => r#"<path d="M20 6 9 17l-5-5"/>"#,
        "x" => r#"<path d="M18 6 6 18M6 6l12 12"/>"#,
        "pin" => r#"<path d="M12 17v5"/><path d="M9 10.8a2 2 0 0 1-1.1 1.8l-1.8.9A2 2 0 0 0 5 15.2V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.8a2 2 0 0 0-1.1-1.8l-1.8-.9A2 2 0 0 1 15 10.8V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#,
        "shelf" => r#"<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>"#,
        // native extensions (own drawings)
        "highlighter" => r#"<path d="m9 11-6 6v3h9l3-3"/><path d="m22 12-4.600 4.600a2 2 0 0 1-2.800 0l-5.200-5.200a2 2 0 0 1 0-2.800L14 4"/>"#,
        "ellipse" => r#"<ellipse cx="12" cy="12" rx="9" ry="6.500"/>"#,
        "step" => r#"<circle cx="12" cy="12" r="8.500"/><path d="M10.200 9.800 12.600 8.600V15.600"/>"#,
        "redo" => r#"<path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0 0 12h3"/>"#,
        _ => "",
    }
}

/// Parsed icons + the geometry cache of one Direct2D device.
#[derive(Default)]
pub struct IconSet {
    icons: RefCell<HashMap<&'static str, Rc<Svg>>>,
    cache: GeomCache,
}

impl IconSet {
    pub fn new() -> IconSet {
        IconSet::default()
    }

    /// Paints icon `name` into `r` (a 24-unit art box scaled to `r`) in `tint`.
    pub fn draw(&self, cv: &Canvas, name: &'static str, r: Rect, tint: Color) {
        if name.is_empty() {
            return;
        }
        let svg = self.icons.borrow_mut().entry(name).or_insert_with(|| Rc::new(Svg::icon(markup(name), 1.7))).clone();
        svg.draw(cv, r, tint, &self.cache);
    }
}
