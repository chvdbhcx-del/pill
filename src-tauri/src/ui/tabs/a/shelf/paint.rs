//! Shelf-specific painting the kit lacks: dashed rounded outlines, the document tile, the empty-state art.
use crate::ui::tabs::a::icons;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::svgpath::{Svg, SvgStyle};
use glasskit::text::Style;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `#rrggbb` to a colour (the model's tile colours); grey on garbage.
pub fn hex(s: &str) -> Color {
    let h = s.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).unwrap_or(0x7c8090);
    Color::rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

thread_local! {
    static DASH: RefCell<HashMap<(u32, u32, u32), Rc<Svg>>> = RefCell::new(HashMap::new());
    static DOC: Rc<(Svg, Svg)> = Rc::new((
        Svg::from_fragment(r#"<path d="M6 0H18.5L28 8.64V30A6 6 0 0 1 22 36H6A6 6 0 0 1 0 30V6A6 6 0 0 1 6 0Z" fill="currentColor" stroke="none"/>"#, (0.0, 0.0, 28.0, 36.0), SvgStyle::line_icon(1.0)),
        Svg::from_fragment(r#"<path d="M18.5 0V8.64H28Z" fill="currentColor" stroke="none"/>"#, (0.0, 0.0, 28.0, 36.0), SvgStyle::line_icon(1.0)),
    ));
    static ART: Rc<[Svg; 4]> = {
        let st = || SvgStyle::line_icon(1.6);
        let vb = (0.0, 0.0, 120.0, 84.0);
        Rc::new([
            Svg::from_fragment(r#"<path d="M44 6h22l10 10v26a4 4 0 0 1-4 4H44a4 4 0 0 1-4-4V10a4 4 0 0 1 4-4z"/><path d="M66 6v10h10"/><path d="M47 28h22M47 36h14"/>"#, vb, st()),
            Svg::from_fragment(r#"<path d="M14 50h92l-9 24a4 4 0 0 1-3.7 2.6H26.7A4 4 0 0 1 23 74z" fill="currentColor" stroke="none"/>"#, vb, st()),
            Svg::from_fragment(r#"<path d="M14 50h92l-9 24a4 4 0 0 1-3.7 2.6H26.7A4 4 0 0 1 23 74z"/>"#, vb, st()),
            Svg::from_fragment(r#"<path d="M30 60h60"/>"#, vb, st()),
        ])
    };
}

/// A 1 px dashed outline inside `r` (CSS `border: 1px dashed`), radius `rad`.
pub fn dashed_rr(cv: &Canvas, r: Rect, rad: f32, color: Color) {
    let key = ((r.w * 2.0) as u32, (r.h * 2.0) as u32, rad as u32);
    let svg = DASH.with(|d| {
        d.borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let inner = format!(r#"<rect x="0.5" y="0.5" width="{}" height="{}" rx="{}" stroke-dasharray="3.4 3.4"/>"#, r.w - 1.0, r.h - 1.0, rad - 0.5);
                Rc::new(Svg::from_fragment(&inner, (0.0, 0.0, r.w, r.h), SvgStyle::line_icon(1.0)))
            })
            .clone()
    });
    svg.draw(cv, r, color, &icons::geom());
}

/// The document tile (`.sh-doc`): 28 x 36 with a folded corner and the extension label, centred in `at`.
pub fn doc_tile(cv: &Canvas, at: Rect, label: &str, color: Color) {
    let r = at.centered(28.0, 36.0);
    DOC.with(|d| {
        d.0.draw(cv, r, color, &icons::geom());
        d.1.draw(cv, r, Color::rgba(1.0, 1.0, 1.0, 0.32), &icons::geom());
    });
    let st = Style::new(8.5, 600).center().track(0.17);
    cv.text(Rect::new(r.x, r.bottom() - 7.0 - 9.0, r.w, 9.0), label, st, Color::rgba(1.0, 1.0, 1.0, 1.0));
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `sh-card` / `sh-tray` keyframes (3.6 s loop) at `t_ms`; `still` = the reduced-motion frame (card resting in the tray).
pub fn art_phase(t_ms: f64, still: bool) -> (f32, f32, f32) {
    if still {
        return (9.0, 1.0, 0.0);
    }
    let p = ((t_ms % 3600.0) / 3600.0) as f32;
    let seg = |a: f32, b: f32| smooth((p - a) / (b - a));
    let (ty, op) = if p < 0.16 {
        (-14.0 + 4.0 * seg(0.0, 0.16), seg(0.0, 0.16))
    } else if p < 0.5 {
        (-10.0 + 25.0 * seg(0.16, 0.5), 1.0)
    } else {
        (15.0, 1.0)
    };
    let tray = if p < 0.48 {
        0.0
    } else if p < 0.54 {
        2.5 * seg(0.48, 0.54)
    } else if p < 0.64 {
        2.5 * (1.0 - seg(0.54, 0.64))
    } else {
        0.0
    };
    (ty, op, tray)
}

/// The empty-state illustration (120 x 84) at `at`; `body` is the colour behind the tray, `ink` the line colour.
pub fn empty_art(cv: &Canvas, at: Rect, t_ms: f64, still: bool, body: Color, ink: Color) {
    let (ty, op, tray) = art_phase(t_ms, still);
    let art = ART.with(|a| a.clone());
    let g = icons::geom();
    cv.with_opacity(op, |cv| cv.with_offset(0.0, ty, |cv| art[0].draw(cv, at, ink, &g)));
    cv.with_offset(0.0, tray, |cv| {
        art[1].draw(cv, at, body, &g);
        art[2].draw(cv, at, ink, &g);
        art[3].draw(cv, at, ink, &g);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_the_model_palette() {
        let c = hex("#e8494f");
        assert!((c.r - 232.0 / 255.0).abs() < 1e-3 && (c.b - 79.0 / 255.0).abs() < 1e-3);
        assert_eq!(hex("nonsense").r, hex("#7c8090").r);
    }

    #[test]
    fn the_art_loop_drops_the_card_into_the_tray_then_bounces_the_tray() {
        let (ty0, op0, _) = art_phase(0.0, false);
        let (ty1, op1, _) = art_phase(0.5 * 3600.0, false);
        assert!(ty0 < -13.0 && op0 < 0.01 && ty1 > 14.0 && op1 == 1.0);
        assert!(art_phase(0.54 * 3600.0, false).2 > 2.0);
        assert_eq!(art_phase(1234.0, true), (9.0, 1.0, 0.0));
    }
}
