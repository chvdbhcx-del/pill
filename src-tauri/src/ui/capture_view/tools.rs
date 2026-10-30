//! Direct2D side of the capture markup: the colour parser, pixelation, and `Tools`, the per-device resources that execute ui-core
//! [`DrawCmd`]s (pen, highlighter, arrow, rect, ellipse, text, pixelate, numbered discs). The same code paints the live overlay (on the
//! host's device, in frame px under a `1/rx` transform) and the exported PNG (`flatten.rs`, on its own device, identity transform), so
//! what the user sees is what gets copied.
use glasskit::color::Color;
use glasskit::gfx::{col, pt, rf, Gfx};
use pill_ui_core::capture::{DrawCmd, IRect, Pt as CPt, Rect as CRect};
use std::cell::RefCell;
use std::collections::HashMap;
use windows::core::{Interface, HSTRING};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;

// ---------------------------------------------------------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------------------------------------------------------

/// `#rgb`, `#rrggbb`, `rgb(r,g,b)`, `rgba(r,g,b,a)` (alpha as `.85` or `0.85`) -> straight colour. Unknown text is opaque black.
pub fn parse_color(s: &str) -> Color {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = u32::from_str_radix(h, 16).unwrap_or(0);
        return match h.len() {
            3 => Color::rgb8((((v >> 8) & 0xF) * 17) as u8, (((v >> 4) & 0xF) * 17) as u8, ((v & 0xF) * 17) as u8),
            6 => Color::rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8),
            _ => Color::rgb8(0, 0, 0),
        };
    }
    if let Some(args) = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")).and_then(|r| r.strip_suffix(')')) {
        let n: Vec<f32> = args.split(',').filter_map(|p| p.trim().parse::<f32>().ok()).collect();
        if n.len() >= 3 {
            return Color::rgba(n[0] / 255.0, n[1] / 255.0, n[2] / 255.0, n.get(3).copied().unwrap_or(1.0));
        }
    }
    Color::rgb8(0, 0, 0)
}

/// Pixelation of `rect` of an RGBA frame (alpha ignored): a `cw x ch` grid of cell averages as premultiplied BGRA (alpha 255), where
/// `cw = round(w / block)`, `ch = round(h / block)` (min 1), exactly the downscale the web page does before it scales back up unsmoothed.
/// Each cell is the mean of at most 4 x 4 samples, so a full-screen blur stays cheap while the user drags it.
pub fn pixelate_cells(rgba: &[u8], fw: i32, fh: i32, rect: IRect, block: i32) -> Option<(u32, u32, Vec<u8>)> {
    let r = IRect { x: rect.x.clamp(0, fw), y: rect.y.clamp(0, fh), w: 0, h: 0 };
    let (x1, y1) = ((rect.x + rect.w).clamp(0, fw), (rect.y + rect.h).clamp(0, fh));
    let (w, h) = (x1 - r.x, y1 - r.y);
    if w < 1 || h < 1 || rgba.len() < (fw as usize) * (fh as usize) * 4 {
        return None;
    }
    let k = block.max(1) as f64;
    let cw = ((w as f64 / k).round() as i32).clamp(1, w);
    let ch = ((h as f64 / k).round() as i32).clamp(1, h);
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for j in 0..ch {
        let (ya, yb) = (r.y + (j as i64 * h as i64 / ch as i64) as i32, r.y + ((j as i64 + 1) * h as i64 / ch as i64) as i32);
        for i in 0..cw {
            let (xa, xb) = (r.x + (i as i64 * w as i64 / cw as i64) as i32, r.x + ((i as i64 + 1) * w as i64 / cw as i64) as i32);
            let (sx, sy) = (((xb - xa).max(1) as usize).div_ceil(4).max(1), ((yb - ya).max(1) as usize).div_ceil(4).max(1));
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            let mut y = ya;
            while y < yb.max(ya + 1) {
                let mut x = xa;
                while x < xb.max(xa + 1) {
                    let o = (y.min(fh - 1) as usize * fw as usize + x.min(fw - 1) as usize) * 4;
                    sr += rgba[o] as u32;
                    sg += rgba[o + 1] as u32;
                    sb += rgba[o + 2] as u32;
                    n += 1;
                    x += sx as i32;
                }
                y += sy as i32;
            }
            let n = n.max(1);
            out.extend_from_slice(&[(sb / n) as u8, (sg / n) as u8, (sr / n) as u8, 255]);
        }
    }
    Some((cw as u32, ch as u32, out))
}

/// RGBA -> premultiplied BGRA for an opaque frame (alpha forced to 255, as the capture is a screen grab). Row copy, vectorises.
pub fn rgba_to_bgra(src: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    for (d, s) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        d[0] = s[2];
        d[1] = s[1];
        d[2] = s[0];
        d[3] = 255;
    }
    out
}

/// The same for a sub-rectangle (flatten only uploads the crop).
pub fn crop_rgba_to_bgra(src: &[u8], fw: i32, r: IRect) -> Vec<u8> {
    let mut out = Vec::with_capacity((r.w.max(0) * r.h.max(0) * 4) as usize);
    for y in r.y..r.y + r.h {
        let o = ((y * fw + r.x) * 4) as usize;
        for p in src[o..o + (r.w * 4) as usize].chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    out
}

/// The frame the pixelate command samples.
#[derive(Clone, Copy)]
pub struct FrameSrc<'a> {
    pub rgba: &'a [u8],
    pub fw: i32,
    pub fh: i32,
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Device resources
// ---------------------------------------------------------------------------------------------------------------------------------------

type PixKey = (i32, i32, i32, i32, i32);

/// Resources tied to ONE Direct2D device (a new one is made when the host rebuilds its device).
pub struct Tools {
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    dashed: ID2D1StrokeStyle,
    family: HSTRING,
    fmts: RefCell<HashMap<(u32, u16), IDWriteTextFormat>>,
    pix: RefCell<HashMap<PixKey, ID2D1Bitmap1>>,
}

fn family(gfx: &Gfx) -> HSTRING {
    unsafe {
        let mut coll: Option<IDWriteFontCollection> = None;
        if gfx.dw.GetSystemFontCollection(&mut coll, false).is_ok() {
            if let Some(c) = coll {
                for name in ["Segoe UI Variable Text", "Segoe UI"] {
                    let (mut i, mut ok) = (0u32, windows::Win32::Foundation::BOOL(0));
                    if c.FindFamilyName(&HSTRING::from(name), &mut i, &mut ok).is_ok() && ok.as_bool() {
                        return HSTRING::from(name);
                    }
                }
            }
        }
    }
    HSTRING::from("Segoe UI")
}

impl Tools {
    pub fn new(gfx: &Gfx) -> windows::core::Result<Tools> {
        unsafe {
            let brush = gfx.dc.CreateSolidColorBrush(&col(1.0, 1.0, 1.0, 1.0), None)?;
            let props = D2D1_STROKE_STYLE_PROPERTIES1 {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
                transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
            };
            let round: ID2D1StrokeStyle = gfx.d2d.CreateStrokeStyle(&props, None)?.into();
            let dash = D2D1_STROKE_STYLE_PROPERTIES1 { startCap: D2D1_CAP_STYLE_FLAT, endCap: D2D1_CAP_STYLE_FLAT, dashCap: D2D1_CAP_STYLE_FLAT, dashStyle: D2D1_DASH_STYLE_DASH, ..props };
            let dashed: ID2D1StrokeStyle = gfx.d2d.CreateStrokeStyle(&dash, None)?.into();
            Ok(Tools { brush, round, dashed, family: family(gfx), fmts: RefCell::new(HashMap::new()), pix: RefCell::new(HashMap::new()) })
        }
    }

    pub fn brush(&self, c: Color) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&c.d2d()) };
        &self.brush
    }

    /// Dashed outline (selected op, text box).
    pub fn dashed(&self) -> &ID2D1StrokeStyle {
        &self.dashed
    }

    /// A text layout of the markup font (`600 {px}px`, Segoe UI Variable Text), unwrapped.
    pub fn text_layout(&self, gfx: &Gfx, s: &str, px: f32, weight: u16) -> Option<IDWriteTextLayout> {
        let key = (px.to_bits(), weight);
        let fmt = {
            let mut m = self.fmts.borrow_mut();
            if let Some(f) = m.get(&key) {
                f.clone()
            } else {
                let f = unsafe {
                    gfx.dw.CreateTextFormat(&self.family, None, DWRITE_FONT_WEIGHT(weight as i32), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, px, &HSTRING::from("en-us")).ok()?
                };
                unsafe {
                    let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                }
                m.insert(key, f.clone());
                f
            }
        };
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe { gfx.dw.CreateTextLayout(&w, &fmt, 8192.0, px * 3.0).ok() }
    }

    /// (width, height, baseline-to-top) of `s` in the markup font: real metrics for the editor box and the selection outline.
    pub fn measure(&self, gfx: &Gfx, s: &str, px: f32, weight: u16) -> (f32, f32) {
        let Some(l) = self.text_layout(gfx, s, px, weight) else { return (s.chars().count() as f32 * px * 0.6, px * 1.25) };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let _ = l.GetMetrics(&mut m);
        }
        (m.widthIncludingTrailingWhitespace.max(m.width), m.height)
    }

    /// Caret x offset (px from the text origin) before char index `ch`.
    pub fn caret_x(&self, gfx: &Gfx, s: &str, px: f32, weight: u16, ch: usize) -> f32 {
        let Some(l) = self.text_layout(gfx, s, px, weight) else { return 0.0 };
        let utf16: u32 = s.chars().take(ch).map(|c| c.len_utf16() as u32).sum();
        let (mut x, mut y, mut m) = (0f32, 0f32, DWRITE_HIT_TEST_METRICS::default());
        unsafe {
            let _ = l.HitTestTextPosition(utf16, false, &mut x, &mut y, &mut m);
        }
        x
    }

    /// Drop the pixelate bitmaps (the frame changed or the cache grew).
    pub fn clear_pix(&self) {
        self.pix.borrow_mut().clear();
    }

    fn pix_bitmap(&self, gfx: &Gfx, src: FrameSrc, rect: IRect, block: i32) -> Option<(ID2D1Bitmap1, D2D_RECT_F)> {
        let key = (rect.x, rect.y, rect.w, rect.h, block);
        let dest = rf(rect.x as f32, rect.y as f32, (rect.x + rect.w) as f32, (rect.y + rect.h) as f32);
        if let Some(b) = self.pix.borrow().get(&key) {
            return Some((b.clone(), dest));
        }
        let (cw, ch, px) = pixelate_cells(src.rgba, src.fw, src.fh, rect, block)?;
        let bmp = gfx.from_bgra(cw, ch, &px).ok()?;
        let mut m = self.pix.borrow_mut();
        if m.len() > 48 {
            m.clear(); // a long drag makes one entry per frame: start over (a miss costs one tiny upload)
        }
        m.insert(key, bmp.clone());
        Some((bmp, dest))
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // Drawing (the caller has begun the draw and set the transform: coordinates are frame px)
    // ------------------------------------------------------------------------------------------------------------------------------

    pub fn draw_cmd(&self, gfx: &Gfx, cmd: &DrawCmd, src: FrameSrc, k: f32) {
        let dc = &gfx.dc;
        let k = k.max(0.01); // opacity multiplier (the insert animation)
        unsafe {
            match cmd {
                DrawCmd::Polyline { pts, color, width, alpha } => {
                    if pts.is_empty() {
                        return;
                    }
                    let c = parse_color(color).fade(*alpha as f32 * k);
                    let w = *width as f32;
                    if pts.len() == 1 {
                        let p = pts[0];
                        dc.DrawLine(pt(p.x as f32, p.y as f32), pt(p.x as f32 + 0.1, p.y as f32), self.brush(c), w, &self.round);
                        return;
                    }
                    if let Ok(g) = gfx.d2d.CreatePathGeometry() {
                        if let Ok(sink) = g.Open() {
                            sink.BeginFigure(pt(pts[0].x as f32, pts[0].y as f32), D2D1_FIGURE_BEGIN_HOLLOW);
                            for p in &pts[1..] {
                                sink.AddLine(pt(p.x as f32, p.y as f32));
                            }
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                            if sink.Close().is_ok() {
                                dc.DrawGeometry(&g, self.brush(c), w, &self.round);
                            }
                        }
                    }
                }
                DrawCmd::StrokeRect { rect, color, width } => {
                    let c = parse_color(color).fade(k);
                    dc.DrawRectangle(&rf(rect.x as f32, rect.y as f32, rect.right() as f32, rect.bottom() as f32), self.brush(c), *width as f32, &self.round);
                }
                DrawCmd::StrokeEllipse { rect, color, width } => {
                    let c = parse_color(color).fade(k);
                    let e = D2D1_ELLIPSE { point: pt(rect.center().x as f32, rect.center().y as f32), radiusX: (rect.w / 2.0) as f32, radiusY: (rect.h / 2.0) as f32 };
                    dc.DrawEllipse(&e, self.brush(c), *width as f32, &self.round);
                }
                DrawCmd::Segments { segs, color, width } => {
                    let c = parse_color(color).fade(k);
                    if let Ok(g) = gfx.d2d.CreatePathGeometry() {
                        if let Ok(sink) = g.Open() {
                            for (a, b) in segs {
                                sink.BeginFigure(pt(a.x as f32, a.y as f32), D2D1_FIGURE_BEGIN_HOLLOW);
                                sink.AddLine(pt(b.x as f32, b.y as f32));
                                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                            }
                            if sink.Close().is_ok() {
                                dc.DrawGeometry(&g, self.brush(c), *width as f32, &self.round);
                            }
                        }
                    }
                }
                DrawCmd::Pixelate { rect, block } => {
                    if let Some((bmp, dest)) = self.pix_bitmap(gfx, src, *rect, *block) {
                        dc.DrawBitmap(&bmp, Some(&dest), k, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
                    }
                }
                DrawCmd::Text { text, at, color, font_px, weight, outline_width, .. } => {
                    self.draw_text_outlined(gfx, text, *at, parse_color(color).fade(k), *font_px as f32, *weight, *outline_width as f32, k);
                }
                DrawCmd::Disc { center, radius, color, label, label_px } => {
                    let c = parse_color(color).fade(k);
                    let r = *radius as f32;
                    let e = D2D1_ELLIPSE { point: pt(center.x as f32, center.y as f32), radiusX: r, radiusY: r };
                    dc.FillEllipse(&e, self.brush(c));
                    dc.DrawEllipse(&e, self.brush(Color::rgba(1.0, 1.0, 1.0, 0.9 * k)), (r * 0.12).max(1.0), &self.round);
                    // label: black on light discs, white on dark ones
                    let lum = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
                    let ink = if lum > 0.6 { Color::rgb8(0, 0, 0) } else { Color::rgb8(255, 255, 255) }.fade(k);
                    if let Some(l) = self.text_layout(gfx, label, *label_px as f32, 700) {
                        let mut m = DWRITE_TEXT_METRICS::default();
                        let _ = l.GetMetrics(&mut m);
                        dc.DrawTextLayout(pt(center.x as f32 - m.width / 2.0, center.y as f32 - m.height / 2.0), &l, self.brush(ink), D2D1_DRAW_TEXT_OPTIONS_NONE);
                    }
                }
            }
        }
    }

    /// Canvas `strokeText` (round, 3 px, `rgba(0,0,0,.85)`) under `fillText`: the outline is the text drawn at a ring of offsets.
    pub fn draw_text_outlined(&self, gfx: &Gfx, text: &str, at: CPt, fill: Color, px: f32, weight: u16, outline_w: f32, k: f32) {
        let Some(l) = self.text_layout(gfx, text, px, weight) else { return };
        let (x, y) = (at.x as f32, at.y as f32 - px * TOP_SHIFT);
        let r = (outline_w / 2.0).max(1.0);
        unsafe {
            let dark = Color::rgba(0.0, 0.0, 0.0, 0.85 * k);
            for i in 0..16 {
                let a = i as f32 * std::f32::consts::TAU / 16.0;
                gfx.dc.DrawTextLayout(pt(x + r * a.cos(), y + r * a.sin()), &l, self.brush(dark), D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            gfx.dc.DrawTextLayout(pt(x, y), &l, self.brush(fill), D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
    }

    /// Bounds (frame px) of a draw command, text measured with the real font: the selection outline and the editor box use it.
    pub fn cmd_bounds(&self, gfx: &Gfx, cmd: &DrawCmd) -> Option<CRect> {
        let from_pts = |pts: &[CPt], pad: f64| {
            let (mut l, mut t, mut r, mut b) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for p in pts {
                l = l.min(p.x);
                t = t.min(p.y);
                r = r.max(p.x);
                b = b.max(p.y);
            }
            (!pts.is_empty()).then(|| CRect::new(l - pad, t - pad, r - l + 2.0 * pad, b - t + 2.0 * pad))
        };
        match cmd {
            DrawCmd::Polyline { pts, width, .. } => from_pts(pts, width / 2.0),
            DrawCmd::StrokeRect { rect, width, .. } | DrawCmd::StrokeEllipse { rect, width, .. } => Some(CRect::new(rect.x - width / 2.0, rect.y - width / 2.0, rect.w + width, rect.h + width)),
            DrawCmd::Segments { segs, width, .. } => {
                let pts: Vec<CPt> = segs.iter().flat_map(|(a, b)| [*a, *b]).collect();
                from_pts(&pts, width / 2.0)
            }
            DrawCmd::Pixelate { rect, .. } => Some(CRect::new(rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64)),
            DrawCmd::Text { text, at, font_px, weight, .. } => {
                let (w, h) = self.measure(gfx, text, *font_px as f32, *weight);
                Some(CRect::new(at.x, at.y - *font_px * TOP_SHIFT as f64, w as f64, h as f64))
            }
            DrawCmd::Disc { center, radius, .. } => Some(CRect::new(center.x - radius, center.y - radius, 2.0 * radius, 2.0 * radius)),
        }
    }
}

/// Canvas `textBaseline = 'top'` puts the em-box top at `y`; a DirectWrite layout starts at the line top, which sits this fraction of an
/// em higher for Segoe UI (ascent 1.08 + descent 0.25 = 1.33 em lines vs the 1 em box, the surplus split by the ascent side). Tuned on
/// the web comparison snapshots.
pub const TOP_SHIFT: f32 = 0.1;

/// Identity of the device context behind `gfx`, to know when the host rebuilt its device.
pub fn device_id(gfx: &Gfx) -> usize {
    gfx.dc.as_raw() as usize
}
