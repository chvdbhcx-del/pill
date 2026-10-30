//! Direct2D drawing of a `Sim`: walks the node tree, applies transforms / opacity layers / drop-shadow filters, fills and strokes shapes.
//! Geometry (path -> ID2D1PathGeometry) and stroke styles are cached; nothing is allocated per frame except the geometry of a path whose
//! `d` is animating (stickman limbs, lens iris) and the stroke style of a dash that is moving (pulse).
use super::interp::{self, Mat};
use super::model::*;
use super::path::{Path, Seg};
use super::sim::{Sim, Tint};
use glasskit::gfx::{bmp_props, col, fx_f32, fx_vec4};
use glasskit::Rect;
use std::mem::ManuallyDrop;
use windows::core::{Interface, Result};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

fn m3(m: Mat) -> Matrix3x2 {
    Matrix3x2 { M11: m[0], M12: m[1], M21: m[2], M22: m[3], M31: m[4], M32: m[5] }
}

fn from_m3(m: &Matrix3x2) -> Mat {
    [m.M11, m.M12, m.M21, m.M22, m.M31, m.M32]
}

fn pt(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

#[derive(PartialEq, Clone, Copy)]
struct StrokeKey {
    lc: u8,
    lj: u8,
    n: u8,
    dash: [u32; 4],
    off: u32,
}

struct Off {
    a: ID2D1Bitmap1,
    b: ID2D1Bitmap1,
    w: u32,
    h: u32,
}

pub struct Renderer {
    factory: ID2D1Factory1,
    brush: ID2D1SolidColorBrush,
    geoms: Vec<Option<ID2D1PathGeometry>>,
    lens: Vec<f32>,
    strokes: Vec<(StrokeKey, ID2D1StrokeStyle)>,
    stroke_next: usize,
    off: Option<Off>,
    in_off: bool,
    pet_id: String,
    fx: Option<ID2D1Effect>,
}

/// per-draw constants
struct Ctx<'a> {
    sim: &'a Sim,
    /// device pixels per viewBox unit along the box (for filter radii that are defined in CSS px of a 96 px tall pet)
    px: f32,
    base: Mat,
}

fn tinted(p: &Paint, ovc: Option<[f32; 4]>, tint: &Tint) -> Option<D2D1_COLOR_F> {
    let p = match ovc {
        Some(c) => Paint { on: true, c, p: 0.0, da: 0.0 },
        None => *p,
    };
    if !p.on {
        return None;
    }
    let k = (p.p * tint.k).clamp(0.0, 1.0);
    let (r, g, b) = if p.c[3] == 0.0 && p.da > 0.0 {
        (tint.accent[0], tint.accent[1], tint.accent[2])
    } else {
        (p.c[0] + (tint.accent[0] - p.c[0]) * k, p.c[1] + (tint.accent[1] - p.c[1]) * k, p.c[2] + (tint.accent[2] - p.c[2]) * k)
    };
    let a = (p.c[3] + p.da * tint.k).clamp(0.0, 1.0);
    Some(col(r, g, b, a))
}

impl Renderer {
    pub fn new(dc: &ID2D1DeviceContext) -> Result<Renderer> {
        unsafe {
            let factory: ID2D1Factory = dc.GetFactory()?;
            let factory: ID2D1Factory1 = factory.cast()?;
            let brush = dc.CreateSolidColorBrush(&col(1.0, 1.0, 1.0, 1.0), None)?;
            Ok(Renderer { factory, brush, geoms: Vec::new(), lens: Vec::new(), strokes: Vec::new(), stroke_next: 0, off: None, in_off: false, pet_id: String::new(), fx: None })
        }
    }

    /// drop the caches of the previous pet
    pub fn reset(&mut self, pet: &Pet) {
        self.geoms = (0..pet.paths.len()).map(|_| None).collect();
        self.lens = vec![f32::NAN; pet.paths.len()];
        self.strokes.clear();
        self.stroke_next = 0;
        self.pet_id = pet.id.clone();
    }

    fn geometry(&mut self, path: &Path, evenodd: bool) -> Result<ID2D1PathGeometry> {
        unsafe {
            let g = self.factory.CreatePathGeometry()?;
            let sink = g.Open()?;
            sink.SetFillMode(if evenodd { D2D1_FILL_MODE_ALTERNATE } else { D2D1_FILL_MODE_WINDING });
            let mut open = false;
            let mut start = (0.0, 0.0);
            for s in &path.segs {
                match *s {
                    Seg::M(x, y) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        }
                        sink.BeginFigure(pt(x, y), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                        start = (x, y);
                    }
                    Seg::L(x, y) => {
                        if !open {
                            sink.BeginFigure(pt(start.0, start.1), D2D1_FIGURE_BEGIN_FILLED);
                            open = true;
                        }
                        sink.AddLine(pt(x, y));
                    }
                    Seg::C(a, b, c, d, e, f) => {
                        if !open {
                            sink.BeginFigure(pt(start.0, start.1), D2D1_FIGURE_BEGIN_FILLED);
                            open = true;
                        }
                        sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: pt(a, b), point2: pt(c, d), point3: pt(e, f) });
                    }
                    Seg::Z => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                            open = false;
                        }
                    }
                }
            }
            if open {
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close()?;
            Ok(g.into())
        }
    }

    fn stroke_style(&mut self, lc: u8, lj: u8, dash: &Dash, off: f32, sw: f32, scale: f32) -> Option<ID2D1StrokeStyle> {
        // CSS dash lengths are user units; D2D measures them in stroke widths. A solid line (no dash, or an all-zero gap) needs no dash style.
        let sw = sw.max(0.01);
        let n = dash.n as usize;
        let solid = n == 0 || (n >= 2 && dash.v[1] <= 0.0) || dash.v[..n].iter().all(|v| *v <= 0.0);
        let mut d = [0u32; 4];
        let mut vals = [0f32; 4];
        if !solid {
            for q in 0..n {
                vals[q] = (dash.v[q] * scale / sw).max(0.0);
                d[q] = vals[q].to_bits();
            }
        }
        let key = StrokeKey { lc, lj, n: if solid { 0 } else { dash.n }, dash: d, off: if solid { 0 } else { (off * scale / sw * 50.0).round().to_bits() } };
        if let Some((_, s)) = self.strokes.iter().find(|(k, _)| *k == key) {
            return Some(s.clone());
        }
        let cap = match lc {
            1 => D2D1_CAP_STYLE_ROUND,
            2 => D2D1_CAP_STYLE_SQUARE,
            _ => D2D1_CAP_STYLE_FLAT,
        };
        let join = match lj {
            1 => D2D1_LINE_JOIN_ROUND,
            2 => D2D1_LINE_JOIN_BEVEL,
            _ => D2D1_LINE_JOIN_MITER_OR_BEVEL,
        };
        let props = D2D1_STROKE_STYLE_PROPERTIES1 {
            startCap: cap,
            endCap: cap,
            dashCap: cap,
            lineJoin: join,
            miterLimit: 4.0,
            dashStyle: if solid { D2D1_DASH_STYLE_SOLID } else { D2D1_DASH_STYLE_CUSTOM },
            dashOffset: if solid { 0.0 } else { off * scale / sw },
            transformType: D2D1_STROKE_TRANSFORM_TYPE_NORMAL,
        };
        // an odd-length dash list repeats itself in CSS, which D2D does not do: double it
        let mut list = [0f32; 8];
        let mut m = 0;
        if !solid {
            for _ in 0..(if n % 2 == 1 { 2 } else { 1 }) {
                for q in 0..n {
                    list[m] = vals[q].max(0.001);
                    m += 1;
                }
            }
        }
        let s: ID2D1StrokeStyle = unsafe { self.factory.CreateStrokeStyle(&props, if solid { None } else { Some(&list[..m]) }) }.ok()?.into();
        if self.strokes.len() < 48 {
            self.strokes.push((key, s.clone()));
        } else {
            self.stroke_next = (self.stroke_next + 1) % 48;
            self.strokes[self.stroke_next] = (key, s.clone());
        }
        Some(s)
    }

    /// Draw `sim` into `rect` (logical px; `scale` = device px per logical px, used for stroke styles and the filter buffers).
    pub fn draw(&mut self, dc: &ID2D1DeviceContext, sim: &Sim, rect: Rect, scale: f32) -> Result<()> {
        let pet = &sim.pet;
        if self.pet_id != pet.id {
            self.reset(pet);
        }
        unsafe {
            let mut base_m = Matrix3x2::identity();
            dc.GetTransform(&mut base_m);
            let base = from_m3(&base_m);
            let k = rect.h / pet.h; // viewBox unit -> logical px
            let root = interp::mul(interp::translate(rect.x, rect.y), interp::scale(k, k));
            let ctx = Ctx { sim, px: rect.h * scale, base };
            self.node(dc, &ctx, 0, interp::mul(base, root), false)?;
            if pet.id == "ghost" {
                self.ghost_shadow(dc, sim, rect, base)?;
            }
            dc.SetTransform(&base_m);
        }
        Ok(())
    }

    /// `.pet[data-pet="ghost"]::after`: a faint floor shadow that shrinks as the ghost rises (pets.css pet-shadow, 3.8 s)
    fn ghost_shadow(&mut self, dc: &ID2D1DeviceContext, sim: &Sim, rect: Rect, base: Mat) -> Result<()> {
        let (mut sx, mut op) = (1.0, 1.0);
        if !sim.reduced {
            let p = ((sim.now % 3800.0) / 3800.0) as f32;
            let q = if p < 0.5 { p * 2.0 } else { (1.0 - p) * 2.0 };
            let e = super::ease::bezier([0.42, 0.0, 0.58, 1.0], q);
            sx = 1.0 - 0.3 * e;
            op = 1.0 - 0.45 * e;
        }
        let (w, h) = (rect.w * 0.56, rect.h * 0.07);
        let (cx, cy) = (rect.x + rect.w / 2.0, rect.y + rect.h - h / 2.0);
        unsafe {
            dc.SetTransform(&m3(interp::mul(base, interp::translate(cx, cy))));
            self.brush.SetColor(&col(1.0, 1.0, 1.0, 0.16 * op));
            dc.FillEllipse(&D2D1_ELLIPSE { point: pt(0.0, 0.0), radiusX: w / 2.0 * sx, radiusY: h / 2.0 }, &self.brush);
        }
        Ok(())
    }

    fn node(&mut self, dc: &ID2D1DeviceContext, c: &Ctx, i: usize, parent: Mat, hid: bool) -> Result<()> {
        let s = *c.sim.style(i);
        if s.dn {
            return Ok(());
        }
        let world = interp::mul(parent, c.sim.local_matrix(i));
        // drop-shadow filters (the tint halo of the rig, the glow of the bot's face, the outline of the 8-bit pet): only while visible
        if !self.in_off && s.nfl > 0 {
            let mut live = [None; 4];
            let mut n = 0;
            for sh in &s.fl[..s.nfl as usize] {
                if let Some(cc) = tinted(&sh.c, None, &c.sim.tint) {
                    if cc.a > 0.003 {
                        live[n] = Some((*sh, cc));
                        n += 1;
                    }
                }
            }
            if n > 0 {
                return self.filtered(dc, c, i, parent, hid, &live[..n]);
            }
        }
        self.body(dc, c, i, world, hid)
    }

    fn body(&mut self, dc: &ID2D1DeviceContext, c: &Ctx, i: usize, world: Mat, hid: bool) -> Result<()> {
        let sim = c.sim;
        let s = sim.style(i);
        let ov = sim.overrides(i);
        let node = &sim.pet.nodes[i];
        let hidden = hid || ov.vis == Some(false);
        let opacity = ov.o.unwrap_or(s.o).clamp(0.0, 1.0);
        let kids = &sim.pet.children[i];
        let mut draws = !hidden && !s.vh && !matches!(node.shape, Shape::None);
        let fill = if draws { tinted(&s.fi, ov.fi, &sim.tint) } else { None };
        let sw = ov.sw.unwrap_or(s.sw);
        let stroke = if draws && sw > 0.0 { tinted(&s.st, ov.st, &sim.tint) } else { None };
        if fill.is_none() && stroke.is_none() {
            draws = false;
        }
        let layer = opacity < 0.999 && (!kids.is_empty() || (fill.is_some() && stroke.is_some()));
        unsafe {
            if node.crisp {
                dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
            }
            if layer {
                let lp = D2D1_LAYER_PARAMETERS1 {
                    contentBounds: D2D_RECT_F { left: -1.0e5, top: -1.0e5, right: 1.0e5, bottom: 1.0e5 },
                    geometricMask: ManuallyDrop::new(None),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: m3(interp::IDENT),
                    opacity,
                    opacityBrush: ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                };
                dc.PushLayer(&lp, None::<&ID2D1Layer>);
            }
            if draws {
                dc.SetTransform(&m3(world));
                let a = if layer { 1.0 } else { opacity };
                let scale = c.px / c.sim.pet.h.max(1.0) * 1.0;
                let dash = ov.da.unwrap_or(s.da);
                let dof = ov.dof.unwrap_or(s.dof);
                let plen = if node.path_length > 0.0 { self.path_len(sim, i, node.path_length) } else { 1.0 };
                let _ = scale;
                match &node.shape {
                    Shape::Circle { cx, cy, r } => {
                        let e = D2D1_ELLIPSE { point: pt(*cx, *cy), radiusX: *r, radiusY: *r };
                        if let Some(mut f) = fill {
                            f.a *= a;
                            self.brush.SetColor(&f);
                            dc.FillEllipse(&e, &self.brush);
                        }
                        if let Some(mut st) = stroke {
                            st.a *= a;
                            self.brush.SetColor(&st);
                            let ss = self.stroke_style(s.lc, s.lj, &scale_dash(dash, plen), dof * plen, sw, 1.0);
                            dc.DrawEllipse(&e, &self.brush, sw, ss.as_ref());
                        }
                    }
                    Shape::Ellipse { cx, cy, rx, ry } => {
                        let e = D2D1_ELLIPSE { point: pt(*cx, *cy), radiusX: *rx, radiusY: *ry };
                        if let Some(mut f) = fill {
                            f.a *= a;
                            self.brush.SetColor(&f);
                            dc.FillEllipse(&e, &self.brush);
                        }
                        if let Some(mut st) = stroke {
                            st.a *= a;
                            self.brush.SetColor(&st);
                            let ss = self.stroke_style(s.lc, s.lj, &scale_dash(dash, plen), dof * plen, sw, 1.0);
                            dc.DrawEllipse(&e, &self.brush, sw, ss.as_ref());
                        }
                    }
                    Shape::Rect { x, y, w, h, rx, ry } => {
                        let r = D2D_RECT_F { left: *x, top: *y, right: x + w, bottom: y + h };
                        let rounded = *rx > 0.0 || *ry > 0.0;
                        let rr = D2D1_ROUNDED_RECT { rect: r, radiusX: rx.min(w / 2.0), radiusY: ry.min(h / 2.0) };
                        if let Some(mut f) = fill {
                            f.a *= a;
                            self.brush.SetColor(&f);
                            if rounded {
                                dc.FillRoundedRectangle(&rr, &self.brush);
                            } else {
                                dc.FillRectangle(&r, &self.brush);
                            }
                        }
                        if let Some(mut st) = stroke {
                            st.a *= a;
                            self.brush.SetColor(&st);
                            let ss = self.stroke_style(s.lc, s.lj, &scale_dash(dash, plen), dof * plen, sw, 1.0);
                            if rounded {
                                dc.DrawRoundedRectangle(&rr, &self.brush, sw, ss.as_ref());
                            } else {
                                dc.DrawRectangle(&r, &self.brush, sw, ss.as_ref());
                            }
                        }
                    }
                    Shape::Path(_) => {
                        if let Some(p) = sim.path_of(i) {
                            let dynamic = ov.has_d || sim.dynamic_path(i);
                            let idx = if s.d != NO_PATH { s.d } else if let Shape::Path(x) = node.shape { x } else { NO_PATH };
                            let geom = if dynamic {
                                Some(self.geometry(p, node.evenodd)?)
                            } else {
                                let slot = idx as usize;
                                if slot < self.geoms.len() {
                                    if self.geoms[slot].is_none() {
                                        self.geoms[slot] = Some(self.geometry(p, node.evenodd)?);
                                    }
                                    self.geoms[slot].clone()
                                } else {
                                    None
                                }
                            };
                            if let Some(g) = geom {
                                if let Some(mut f) = fill {
                                    f.a *= a;
                                    self.brush.SetColor(&f);
                                    dc.FillGeometry(&g, &self.brush, None::<&ID2D1Brush>);
                                }
                                if let Some(mut st) = stroke {
                                    st.a *= a;
                                    self.brush.SetColor(&st);
                                    let ss = self.stroke_style(s.lc, s.lj, &scale_dash(dash, plen), dof * plen, sw, 1.0);
                                    dc.DrawGeometry(&g, &self.brush, sw, ss.as_ref());
                                }
                            }
                        }
                    }
                    Shape::None => {}
                }
            }
            for k in kids {
                self.node(dc, c, *k as usize, world, hidden)?;
            }
            if layer {
                dc.PopLayer();
            }
            if node.crisp {
                dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            }
        }
        Ok(())
    }

    fn path_len(&mut self, sim: &Sim, i: usize, pl: f32) -> f32 {
        // dash lengths of `pathLength="100"` paths are fractions of the real length
        let idx = match sim.pet.nodes[i].shape {
            Shape::Path(x) => x as usize,
            _ => return 1.0,
        };
        if idx >= self.lens.len() {
            return 1.0;
        }
        if self.lens[idx].is_nan() {
            let g = sim.path_of(i).and_then(|p| self.geometry(p, false).ok());
            self.lens[idx] = g.and_then(|g| unsafe { g.ComputeLength(None, 0.25).ok() }).unwrap_or(pl);
        }
        self.lens[idx] / pl
    }

    /// Render the subtree into an offscreen buffer, run it through the drop-shadow chain (CSS applies filters one after the other) and composite.
    fn filtered(&mut self, dc: &ID2D1DeviceContext, c: &Ctx, i: usize, parent: Mat, hid: bool, shadows: &[Option<(Shadow, D2D1_COLOR_F)>]) -> Result<()> {
        let world = interp::mul(parent, c.sim.local_matrix(i));
        let k = c.px / 96.0; // CSS px of a 96 px tall pet -> device px
        let pet = &c.sim.pet;
        let mut margin = 4.0f32;
        for s in shadows.iter().flatten() {
            margin += (s.0.b * 3.0 + s.0.x.abs() + s.0.y.abs()) * k;
        }
        // device bounds of the whole pet box under the base transform
        let b = c.base;
        let kk = (c.px / pet.h) / ((b[0] * b[0] + b[1] * b[1]).sqrt().max(1e-3)).max(1e-3);
        let _ = kk;
        let org = (parent[4], parent[5]); // device position of the box origin (root matrix = base * T * S)
        let dev_w = pet.w * parent[0].hypot(parent[1]);
        let dev_h = pet.h * parent[2].hypot(parent[3]);
        let (ow, oh) = ((dev_w + 2.0 * margin).ceil() as u32 + 1, (dev_h + 2.0 * margin).ceil() as u32 + 1);
        let (ox, oy) = ((org.0 - margin).floor(), (org.1 - margin).floor());
        if self.off.as_ref().is_none_or(|o| o.w < ow || o.h < oh) {
            let p = bmp_props(D2D1_BITMAP_OPTIONS_TARGET, D2D1_ALPHA_MODE_PREMULTIPLIED);
            let sz = D2D_SIZE_U { width: ow.max(8), height: oh.max(8) };
            unsafe {
                self.off = Some(Off { a: dc.CreateBitmap(sz, None, 0, &p)?, b: dc.CreateBitmap(sz, None, 0, &p)?, w: sz.width, h: sz.height });
            }
        }
        if self.fx.is_none() {
            self.fx = Some(unsafe { dc.CreateEffect(&CLSID_D2D1Shadow)? });
        }
        let region = D2D_RECT_F { left: 0.0, top: 0.0, right: ow as f32, bottom: oh as f32 }; // only the used part of the (possibly larger) buffer
        let (a, bb) = {
            let o = self.off.as_ref().unwrap();
            (o.a.clone(), o.b.clone())
        };
        let fx = self.fx.clone().unwrap();
        unsafe {
            let prev = dc.GetTarget().ok();
            let mut base_m = Matrix3x2::identity();
            dc.GetTransform(&mut base_m);
            // 1) the subtree itself into A (device space shifted by -origin)
            dc.SetTarget(&a);
            dc.Clear(Some(&col(0.0, 0.0, 0.0, 0.0)));
            self.in_off = true;
            let shifted = interp::mul(interp::translate(-ox, -oy), world);
            let r = self.body(dc, c, i, shifted, hid);
            self.in_off = false;
            r?;
            // 2) the chain: B = shadow(A) + A, then A = shadow(B) + B ...
            dc.SetTransform(&m3(interp::IDENT));
            let (mut src, mut dst) = (a.clone(), bb.clone());
            for s in shadows.iter().flatten() {
                dc.SetTarget(&dst);
                dc.Clear(Some(&col(0.0, 0.0, 0.0, 0.0)));
                fx.SetInput(0, &src, true);
                fx_f32(&fx, 0, (s.0.b * k).max(0.0))?; // CSS drop-shadow: the blur value IS the standard deviation
                fx_vec4(&fx, 1, [s.1.r, s.1.g, s.1.b, s.1.a])?;
                dc.DrawImage(&fx.GetOutput()?, Some(&pt(s.0.x * k, s.0.y * k)), Some(&region), D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                dc.DrawImage(&src, None, Some(&region), D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
                std::mem::swap(&mut src, &mut dst);
            }
            // 3) onto the real target
            match prev {
                Some(t) => dc.SetTarget(&t),
                None => dc.SetTarget(None::<&ID2D1Image>),
            }
            dc.SetTransform(&m3(interp::IDENT));
            dc.DrawImage(&src, Some(&pt(ox, oy)), Some(&region), D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            dc.SetTransform(&base_m);
        }
        Ok(())
    }
}

/// sRGB -> linear (the Shadow effect colour is in linear space)
fn lin(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn scale_dash(d: Dash, plen: f32) -> Dash {
    let mut o = d;
    for q in 0..d.n as usize {
        o.v[q] = d.v[q] * plen;
    }
    o
}
