//! The island's silhouette: the body (different top / bottom radii), the concave flares of the notch and the liquid edge strip.
use super::gpu::cs;
use glasskit::geom::Rect;
use glasskit::gfx::pt;
use glasskit::{Canvas, Color};
use pill_ui_core::shell::liquid::{EdgeView, GlowView};
use pill_ui_core::shell::settings::EdgeStyle;
use pill_ui_core::shell::types::{BodyView, LIGHT_SHADOW};
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

/// CSS `border-radius: rt rt rb rb`, with the browser's rule that radii shrink together when they do not fit.
pub fn clamp_radii(w: f32, h: f32, rt: f32, rb: f32) -> (f32, f32) {
    let f = [1.0, if rt + rb > 0.0 { h / (rt + rb) } else { 1.0 }, if rt > 0.0 { w / (2.0 * rt) } else { 1.0 }, if rb > 0.0 { w / (2.0 * rb) } else { 1.0 }].into_iter().fold(1.0f32, f32::min);
    (rt * f, rb * f)
}

fn arc(sink: &ID2D1GeometrySink, to: (f32, f32), r: f32, cw: bool) {
    if r <= 0.0 {
        unsafe { sink.AddLine(pt(to.0, to.1)) };
        return;
    }
    unsafe {
        sink.AddArc(&D2D1_ARC_SEGMENT {
            point: pt(to.0, to.1),
            size: D2D_SIZE_F { width: r, height: r },
            rotationAngle: 0.0,
            sweepDirection: if cw { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
            arcSize: D2D1_ARC_SIZE_SMALL,
        })
    };
}

fn geometry(cv: &Canvas, build: impl FnOnce(&ID2D1GeometrySink)) -> Option<ID2D1Geometry> {
    unsafe {
        let g = cv.gfx.d2d.CreatePathGeometry().ok()?;
        let sink = g.Open().ok()?;
        build(&sink);
        sink.Close().ok()?;
        g.cast::<ID2D1Geometry>().ok()
    }
}

/// The body outline as a path geometry (clockwise).
pub fn body_geometry(cv: &Canvas, r: Rect, rt: f32, rb: f32) -> Option<ID2D1Geometry> {
    let (rt, rb) = clamp_radii(r.w, r.h, rt, rb);
    geometry(cv, |s| unsafe {
        s.BeginFigure(pt(r.x + rt, r.y), D2D1_FIGURE_BEGIN_FILLED);
        s.AddLine(pt(r.right() - rt, r.y));
        arc(s, (r.right(), r.y + rt), rt, true);
        s.AddLine(pt(r.right(), r.bottom() - rb));
        arc(s, (r.right() - rb, r.bottom()), rb, true);
        s.AddLine(pt(r.x + rb, r.bottom()));
        arc(s, (r.x, r.bottom() - rb), rb, true);
        s.AddLine(pt(r.x, r.y + rt));
        arc(s, (r.x + rt, r.y), rt, true);
        s.EndFigure(D2D1_FIGURE_END_CLOSED);
    })
}

/// Concave flare at a top corner of the notch: `left` = the one to the left of the body (its right edge touches the body).
pub fn flare_geometry(cv: &Canvas, r: Rect, left: bool) -> Option<ID2D1Geometry> {
    geometry(cv, |s| unsafe {
        if left {
            // M0 0 H1 V1 A1 1 0 0 0 0 0 Z in a unit box scaled to r
            s.BeginFigure(pt(r.x, r.y), D2D1_FIGURE_BEGIN_FILLED);
            s.AddLine(pt(r.right(), r.y));
            s.AddLine(pt(r.right(), r.bottom()));
            arc(s, (r.x, r.y), r.w, false);
        } else {
            s.BeginFigure(pt(r.right(), r.y), D2D1_FIGURE_BEGIN_FILLED);
            s.AddLine(pt(r.x, r.y));
            s.AddLine(pt(r.x, r.bottom()));
            arc(s, (r.right(), r.y), r.w, true);
        }
        s.EndFigure(D2D1_FIGURE_END_CLOSED);
    })
}

/// Paints the light-theme shadow, the body and (when `flare_a` > 0) the notch flares. `flare_a` is the animated opacity of the flares.
pub fn paint_body(cv: &Canvas, b: &BodyView, rt: f32, rb: f32, flare_a: f32, shadow_k: f32) {
    let r = Rect::new(b.left as f32, b.top as f32, b.width as f32, b.height as f32);
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let fill = cs(b.fill);
    if b.shadow && shadow_k > 0.0 {
        for (dx, dy, blur, a) in LIGHT_SHADOW {
            cv.shadow_rr(r.offset(dx as f32, 0.0), rb.max(rt), blur as f32, dy as f32, Color::rgba(0.0, 0.0, 0.0, a as f32 * shadow_k));
        }
    }
    if let Some(g) = body_geometry(cv, r, rt, rb) {
        cv.geometry(&g, Some(fill), None);
    }
    if flare_a > 0.0 && b.flare_radius > 0.0 {
        let fl = b.flare_radius as f32;
        for left in [true, false] {
            let fr = Rect::new(if left { r.x - fl } else { r.right() }, r.y, fl, fl);
            if let Some(g) = flare_geometry(cv, fr, left) {
                cv.geometry(&g, Some(fill.fade(flare_a)), None);
            }
        }
    }
}

fn line_gradient(cv: &Canvas, x0: f32, x1: f32, rgb: (u8, u8, u8), stops: &[(f64, f64)]) -> Option<ID2D1LinearGradientBrush> {
    unsafe {
        let st: Vec<D2D1_GRADIENT_STOP> = stops
            .iter()
            .map(|(o, a)| D2D1_GRADIENT_STOP { position: *o as f32, color: D2D1_COLOR_F { r: rgb.0 as f32 / 255.0, g: rgb.1 as f32 / 255.0, b: rgb.2 as f32 / 255.0, a: *a as f32 } })
            .collect();
        let coll = cv.dc.CreateGradientStopCollection(&st, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_PREMULTIPLIED).ok()?;
        cv.dc.CreateLinearGradientBrush(&D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(x0, 0.0), endPoint: pt(x1, 0.0) }, None, &coll).ok()
    }
}

/// The liquid edge strip (`pill.js liqDraw`). `origin` = the strip's top-left in window px, coordinates inside are css px at 100 %
/// multiplied by `scale`. `pulse` is an extra halo strength 0..1 (the live layer's edge pulse).
pub fn paint_edge(cv: &Canvas, e: &EdgeView, origin: (f32, f32), pulse: f32, reduced: bool) {
    if !e.visible {
        return;
    }
    let s = e.scale as f32;
    let (o, up) = ((e.span / 2.0) as f32, e.up as f32);
    let m = glasskit::canvas::mul(&glasskit::canvas::scale_about((0.0, 0.0), s, s), &glasskit::canvas::translation(origin.0, origin.1));
    cv.with_transform(m, |cv| {
        if e.style == EdgeStyle::Glow {
            if let Some(g) = &e.glow {
                paint_glow(cv, g, o, s);
            }
            return;
        }
        if e.style != EdgeStyle::Liquid || e.points.len() < 2 || e.stroke.is_none() {
            return;
        }
        let Some(stroke) = &e.stroke else { return };
        let fill = cs(e.fill);
        let pts: Vec<(f32, f32)> = e.points.iter().map(|p| (o + p.0 as f32, up + p.1 as f32)).collect();
        // body-coloured fill between the straight bottom edge and the curve
        if let Some(g) = geometry(cv, |sk| unsafe {
            sk.BeginFigure(pt(pts[0].0, up - 1.0), D2D1_FIGURE_BEGIN_FILLED);
            for p in &pts {
                sk.AddLine(pt(p.0, p.1));
            }
            sk.AddLine(pt(pts[pts.len() - 1].0, up - 1.0));
            sk.EndFigure(D2D1_FIGURE_END_CLOSED);
        }) {
            cv.geometry(&g, Some(fill), None);
        }
        let half = e.half as f32;
        let Some(brush) = line_gradient(cv, o - half, o + half, stroke.rgb, &stroke.gradient) else { return };
        let line = geometry(cv, |sk| unsafe {
            sk.BeginFigure(pt(pts[0].0, pts[0].1), D2D1_FIGURE_BEGIN_HOLLOW);
            for p in &pts[1..] {
                sk.AddLine(pt(p.0, p.1));
            }
            sk.EndFigure(D2D1_FIGURE_END_OPEN);
        });
        if let Some(line) = line {
            unsafe {
                for (a, w) in &stroke.passes {
                    brush.SetOpacity(*a as f32);
                    cv.dc.DrawGeometry(&line, &brush, *w as f32, cv.res.round());
                }
                if reduced && stroke.shadow_blur > 0.0 {
                    brush.SetOpacity(0.25);
                    cv.dc.DrawGeometry(&line, &brush, 6.0, cv.res.round());
                }
                if pulse > 0.0 {
                    brush.SetOpacity(pulse.min(1.0) * 0.7);
                    cv.dc.DrawGeometry(&line, &brush, 7.0, cv.res.round());
                }
            }
        }
        if let Some(b) = &e.ball {
            let c = pt(o + b.x as f32, up + b.y as f32);
            unsafe {
                cv.res.brush().SetColor(&fill.d2d());
                cv.dc.FillEllipse(&D2D1_ELLIPSE { point: c, radiusX: b.rx as f32, radiusY: b.ry as f32 }, cv.res.brush());
            }
        }
    });
}

/// `glow` style: a soft ellipse of light under the island, approximated by stacked translucent ellipses (no blur effect).
fn paint_glow(cv: &Canvas, g: &GlowView, o: f32, _s: f32) {
    let col = cs(pill_ui_core::shell::color::Rgba { r: g.color.r, g: g.color.g, b: g.color.b, a: 1.0 });
    let n = 7;
    for i in (0..n).rev() {
        let k = (i + 1) as f32 / n as f32;
        let (rx, ry) = (g.rx as f32 + g.blur as f32 * k, g.ry as f32 + g.blur as f32 * k * 0.5);
        let a = (g.alpha as f32 * 0.32 * (1.0 - k).powi(2)).max(0.0) + g.alpha as f32 * 0.06;
        cv.with_clip(Rect::new(0.0, g.clip_above as f32, 4096.0, 4096.0), |cv| unsafe {
            cv.res.brush().SetColor(&col.alpha(a.min(1.0)).d2d());
            cv.dc.FillEllipse(&D2D1_ELLIPSE { point: pt(o + g.x as f32, g.y as f32), radiusX: rx, radiusY: ry }, cv.res.brush());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radii_shrink_together_like_css() {
        assert_eq!(clamp_radii(144.0, 30.0, 0.0, 15.0), (0.0, 15.0));
        let (t, b) = clamp_radii(144.0, 20.0, 15.0, 15.0);
        assert!((t + b - 20.0).abs() < 1e-4 && (t - b).abs() < 1e-4, "{t} {b}");
        let (t, b) = clamp_radii(10.0, 100.0, 30.0, 30.0);
        assert!((t - 5.0).abs() < 1e-4 && (b - 5.0).abs() < 1e-4);
        assert_eq!(clamp_radii(100.0, 100.0, 0.0, 0.0), (0.0, 0.0));
    }
}
