//! Geometry of the capture overlay: points / rectangles, the 8 selection handles and their hit test, move / resize / nudge,
//! square-or-aspect constraint, edge snap, and the css-px <-> frame-px <-> desktop mapping across monitors of mixed DPI.
//!
//! Units. The overlay page lives in css (logical) px of its own window `capture-N`; the frozen frame is `fw x fh` physical px.
//! The scale is derived per axis from those two sizes (`rx = fw / vw`), so a 150 % monitor next to a 100 % one needs no special
//! case (the same trick as `to_frame_rect` in src-tauri/src/capture.rs, whose tests are mirrored in tests.rs).
//! Everything here is pure `f64` / `i32` math: no windowing, no I/O.
use serde::{Deserialize, Serialize};

/// A point in css px (pointer position, selection corner) or frame px (markup ops) - the type alone does not say which; the
/// field / function doc does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

impl Pt {
    pub const fn new(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }
    pub fn dist(self, o: Pt) -> f64 {
        (self.x - o.x).hypot(self.y - o.y)
    }
}

/// A float rectangle (css px for the selection and toolbar).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    /// `norm(a, b)` of capture.js: the rectangle spanned by two corners, whichever way the user dragged.
    pub fn from_corners(a: Pt, b: Pt) -> Rect {
        Rect { x: a.x.min(b.x), y: a.y.min(b.y), w: (a.x - b.x).abs(), h: (a.y - b.y).abs() }
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn center(&self) -> Pt {
        Pt::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    /// Inclusive on all four edges, like `inside(p)` in capture.js.
    pub fn contains(&self, p: Pt) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    /// Move so the rectangle stays inside `0..vw x 0..vh` (size unchanged).
    pub fn clamp_move(&self, vw: f64, vh: f64) -> Rect {
        Rect { x: self.x.min(vw - self.w).max(0.0), y: self.y.min(vh - self.h).max(0.0), w: self.w, h: self.h }
    }
    pub fn translate(&self, dx: f64, dy: f64) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, w: self.w, h: self.h }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

/// An integer rectangle in frame (physical) px - `Rect` of capture.rs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Mouse cursor the overlay shows (`setCursor` in capture.js; `None` = the page default, the crosshair, or hidden in colour mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cursor {
    /// Page default: `crosshair` (`cursor: none` in colour mode - the lens replaces it).
    Default,
    NwseResize,
    NeswResize,
    NsResize,
    EwResize,
    Move,
    Text,
    Crosshair,
}

impl Cursor {
    /// The css cursor keyword the web page used.
    pub fn css(self) -> &'static str {
        match self {
            Cursor::Default | Cursor::Crosshair => "crosshair",
            Cursor::NwseResize => "nwse-resize",
            Cursor::NeswResize => "nesw-resize",
            Cursor::NsResize => "ns-resize",
            Cursor::EwResize => "ew-resize",
            Cursor::Move => "move",
            Cursor::Text => "text",
        }
    }
}

/// The 8 selection handles, in the order capture.html lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Handle {
    Nw,
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
}

pub const HANDLES: [Handle; 8] = [Handle::Nw, Handle::N, Handle::Ne, Handle::E, Handle::Se, Handle::S, Handle::Sw, Handle::W];

/// Pointer tolerance around the selection edges (`T8` in capture.js).
pub const HIT_TOLERANCE: f64 = 8.0;
/// A new selection smaller than this on either side is thrown away on pointer-up.
pub const MIN_SELECTION: f64 = 4.0;
/// Handle dot size in css px (`#sel i` is 9 px, centred on its anchor; 1.5 px accent border).
pub const HANDLE_SIZE: f64 = 9.0;

impl Handle {
    pub fn name(self) -> &'static str {
        match self {
            Handle::Nw => "nw",
            Handle::N => "n",
            Handle::Ne => "ne",
            Handle::E => "e",
            Handle::Se => "se",
            Handle::S => "s",
            Handle::Sw => "sw",
            Handle::W => "w",
        }
    }
    pub fn west(self) -> bool {
        matches!(self, Handle::Nw | Handle::Sw | Handle::W)
    }
    pub fn east(self) -> bool {
        matches!(self, Handle::Ne | Handle::Se | Handle::E)
    }
    pub fn north(self) -> bool {
        matches!(self, Handle::Nw | Handle::Ne | Handle::N)
    }
    pub fn south(self) -> bool {
        matches!(self, Handle::Sw | Handle::Se | Handle::S)
    }
    pub fn is_corner(self) -> bool {
        matches!(self, Handle::Nw | Handle::Ne | Handle::Se | Handle::Sw)
    }
    /// The resize cursor (`CUR` in capture.js).
    pub fn cursor(self) -> Cursor {
        match self {
            Handle::Nw | Handle::Se => Cursor::NwseResize,
            Handle::Ne | Handle::Sw => Cursor::NeswResize,
            Handle::N | Handle::S => Cursor::NsResize,
            Handle::E | Handle::W => Cursor::EwResize,
        }
    }
    /// Where the handle dot sits on `sel` (the `left/top` percentages of `#sel i[data-h=..]`).
    pub fn anchor(self, sel: &Rect) -> Pt {
        let x = if self.west() { sel.x } else if self.east() { sel.right() } else { sel.x + sel.w / 2.0 };
        let y = if self.north() { sel.y } else if self.south() { sel.bottom() } else { sel.y + sel.h / 2.0 };
        Pt::new(x, y)
    }
}

/// Result of [`hit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// Outside the selection and its edge tolerance.
    Outside,
    /// Inside the body.
    Inside,
    /// On an edge / corner within [`HIT_TOLERANCE`].
    Edge(Handle),
}

/// Port of `hit(p)` in capture.js, including the priority order: corners, then edges, then the body.
pub fn hit(sel: &Rect, p: Pt) -> Hit {
    let t = HIT_TOLERANCE;
    let nl = (p.x - sel.x).abs() <= t;
    let nr = (p.x - sel.right()).abs() <= t;
    let nt = (p.y - sel.y).abs() <= t;
    let nb = (p.y - sel.bottom()).abs() <= t;
    let ix = p.x > sel.x - t && p.x < sel.right() + t;
    let iy = p.y > sel.y - t && p.y < sel.bottom() + t;
    if nt && nl {
        return Hit::Edge(Handle::Nw);
    }
    if nt && nr {
        return Hit::Edge(Handle::Ne);
    }
    if nb && nl {
        return Hit::Edge(Handle::Sw);
    }
    if nb && nr {
        return Hit::Edge(Handle::Se);
    }
    if nt && ix {
        return Hit::Edge(Handle::N);
    }
    if nb && ix {
        return Hit::Edge(Handle::S);
    }
    if nl && iy {
        return Hit::Edge(Handle::W);
    }
    if nr && iy {
        return Hit::Edge(Handle::E);
    }
    if sel.contains(p) {
        Hit::Inside
    } else {
        Hit::Outside
    }
}

/// Dragging the body: the original rectangle shifted by the pointer delta, kept inside the viewport.
pub fn move_selection(orig: &Rect, grab: Pt, p: Pt, vw: f64, vh: f64) -> Rect {
    orig.translate(p.x - grab.x, p.y - grab.y).clamp_move(vw, vh)
}

/// Dragging a handle: the edges named by the handle follow the pointer; the rectangle is re-normalised so dragging past the
/// opposite edge flips it instead of going negative.
pub fn resize_selection(orig: &Rect, h: Handle, p: Pt) -> Rect {
    let (mut l, mut r, mut t, mut b) = (orig.x, orig.right(), orig.y, orig.bottom());
    if h.west() {
        l = p.x;
    }
    if h.east() {
        r = p.x;
    }
    if h.north() {
        t = p.y;
    }
    if h.south() {
        b = p.y;
    }
    Rect::from_corners(Pt::new(l, t), Pt::new(r, b))
}

/// Shift = square, or any locked aspect `ratio = w / h` (extension: the web page had neither). The corner `p` is pulled
/// onto the line through `anchor` that keeps the ratio, using the larger of the two drag distances so the box never shrinks
/// under the pointer. `ratio == 1.0` is the square.
pub fn lock_ratio(anchor: Pt, p: Pt, ratio: f64) -> Pt {
    if !(ratio.is_finite() && ratio > 0.0) {
        return p;
    }
    let (dx, dy) = (p.x - anchor.x, p.y - anchor.y);
    let (ax, ay) = (dx.abs(), dy.abs());
    let (w, h) = if ax / ratio >= ay { (ax, ax / ratio) } else { (ay * ratio, ay) };
    Pt::new(anchor.x + w * if dx < 0.0 { -1.0 } else { 1.0 }, anchor.y + h * if dy < 0.0 { -1.0 } else { 1.0 })
}

/// `lock_ratio` for a handle drag: the corner opposite to a corner handle is the anchor. Edge handles are left alone (only one
/// axis moves there, a ratio would fight the user).
pub fn resize_locked(orig: &Rect, h: Handle, p: Pt, ratio: f64) -> Rect {
    if !h.is_corner() {
        return resize_selection(orig, h, p);
    }
    let anchor = Pt::new(if h.west() { orig.right() } else { orig.x }, if h.north() { orig.bottom() } else { orig.y });
    Rect::from_corners(anchor, lock_ratio(anchor, p, ratio))
}

/// Keep a point inside the viewport.
pub fn clamp_pt(p: Pt, vw: f64, vh: f64) -> Pt {
    Pt::new(p.x.max(0.0).min(vw), p.y.max(0.0).min(vh))
}

/// Arrow-key nudge of the selection (extension): one frame pixel per press, `big` (Shift) = 10, converted to css px so a nudge
/// is one *physical* pixel on any DPI. Stays inside the viewport.
pub fn nudge(sel: &Rect, dx: i32, dy: i32, big: bool, scale: Scale, vw: f64, vh: f64) -> Rect {
    let st = if big { 10.0 } else { 1.0 };
    sel.translate(dx as f64 * st / scale.rx, dy as f64 * st / scale.ry).clamp_move(vw, vh)
}

/// Ctrl+Arrow (extension): grow / shrink the right and bottom edges by whole frame pixels, never below one pixel.
pub fn nudge_size(sel: &Rect, dw: i32, dh: i32, big: bool, scale: Scale, vw: f64, vh: f64) -> Rect {
    let st = if big { 10.0 } else { 1.0 };
    let w = (sel.w + dw as f64 * st / scale.rx).max(1.0 / scale.rx).min(vw - sel.x);
    let h = (sel.h + dh as f64 * st / scale.ry).max(1.0 / scale.ry).min(vh - sel.y);
    Rect::new(sel.x, sel.y, w, h)
}

/// Edge snapping (extension, off by default): selection edges within `threshold` css px of a guide jump onto it.
/// The viewport edges are always guides; `guides_x` / `guides_y` add more (window borders the host knows about).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snap {
    pub threshold: f64,
    pub guides_x: Vec<f64>,
    pub guides_y: Vec<f64>,
}

impl Default for Snap {
    fn default() -> Snap {
        Snap { threshold: 6.0, guides_x: Vec::new(), guides_y: Vec::new() }
    }
}

fn snap_1d(v: f64, guides: &[f64], thr: f64) -> Option<f64> {
    guides.iter().copied().filter(|g| (g - v).abs() <= thr).min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()))
}

impl Snap {
    fn xs(&self, vw: f64) -> Vec<f64> {
        let mut g = vec![0.0, vw];
        g.extend_from_slice(&self.guides_x);
        g
    }
    fn ys(&self, vh: f64) -> Vec<f64> {
        let mut g = vec![0.0, vh];
        g.extend_from_slice(&self.guides_y);
        g
    }
    /// Snap a moved rectangle: whichever of its left / right (top / bottom) edges is closer to a guide wins, size unchanged.
    pub fn snap_move(&self, r: &Rect, vw: f64, vh: f64) -> Rect {
        let (gx, gy) = (self.xs(vw), self.ys(vh));
        let sx = [snap_1d(r.x, &gx, self.threshold).map(|g| g - r.x), snap_1d(r.right(), &gx, self.threshold).map(|g| g - r.right())];
        let sy = [snap_1d(r.y, &gy, self.threshold).map(|g| g - r.y), snap_1d(r.bottom(), &gy, self.threshold).map(|g| g - r.bottom())];
        let best = |s: [Option<f64>; 2]| s.into_iter().flatten().min_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(0.0);
        r.translate(best(sx), best(sy))
    }
    /// Snap a single dragged point (a new selection corner or the moving edge of a resize).
    pub fn snap_point(&self, p: Pt, vw: f64, vh: f64) -> Pt {
        Pt::new(snap_1d(p.x, &self.xs(vw), self.threshold).unwrap_or(p.x), snap_1d(p.y, &self.ys(vh), self.threshold).unwrap_or(p.y))
    }
}

// ------------------------------------------------------------------------------------------------- frame mapping

/// css px -> frame px scale of one overlay window, per axis (`rx()` / `ry()` in capture.js).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    pub rx: f64,
    pub ry: f64,
}

impl Scale {
    /// `fw / max(1, vw)` per axis - the `Math.max(1, innerWidth)` guard of the page.
    pub fn new(fw: i32, fh: i32, vw: f64, vh: f64) -> Scale {
        Scale { rx: fw as f64 / vw.max(1.0), ry: fh as f64 / vh.max(1.0) }
    }
    pub const IDENTITY: Scale = Scale { rx: 1.0, ry: 1.0 };
    /// css point -> frame point (`fp` in capture.js).
    pub fn to_frame(&self, p: Pt) -> Pt {
        Pt::new(p.x * self.rx, p.y * self.ry)
    }
    pub fn to_css(&self, p: Pt) -> Pt {
        Pt::new(p.x / self.rx, p.y / self.ry)
    }
}

/// Selection rectangle from the overlay (css px `[x, y, w, h]` inside a `vw x vh` viewport; w / h may be negative when the
/// user dragged up / left) -> physical px inside a frame of `fw x fh`. Clamped; `None` when empty (< 2 px).
/// Mirror of `to_frame_rect` in src-tauri/src/capture.rs (the backend re-derives the crop from the same numbers).
pub fn to_frame_rect(css: [f64; 4], vw: f64, vh: f64, fw: i32, fh: i32) -> Option<IRect> {
    if vw < 1.0 || vh < 1.0 || css.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (mut x, mut y, mut w, mut h) = (css[0], css[1], css[2], css[3]);
    if w < 0.0 {
        x += w;
        w = -w;
    }
    if h < 0.0 {
        y += h;
        h = -h;
    }
    let (sx, sy) = (fw as f64 / vw, fh as f64 / vh);
    let x0 = ((x * sx).round() as i64).clamp(0, fw as i64) as i32;
    let y0 = ((y * sy).round() as i64).clamp(0, fh as i64) as i32;
    let x1 = (((x + w) * sx).round() as i64).clamp(0, fw as i64) as i32;
    let y1 = (((y + h) * sy).round() as i64).clamp(0, fh as i64) as i32;
    (x1 - x0 >= 2 && y1 - y0 >= 2).then_some(IRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// A point in css px -> frame px (clamped to the frame). Mirror of `to_frame_point` in capture.rs.
pub fn to_frame_point(x: f64, y: f64, vw: f64, vh: f64, fw: i32, fh: i32) -> Option<(i32, i32)> {
    if vw < 1.0 || vh < 1.0 || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let px = ((x * fw as f64 / vw).floor() as i64).clamp(0, fw as i64 - 1) as i32;
    let py = ((y * fh as f64 / vh).floor() as i64).clamp(0, fh as i64 - 1) as i32;
    Some((px, py))
}

/// The crop `exportPng()` of capture.js flattens: selection scaled and rounded, at least 1 px, cut at the frame edge.
pub fn export_rect(sel: &Rect, scale: Scale, fw: i32, fh: i32) -> IRect {
    let x = (sel.x * scale.rx).round() as i32;
    let y = (sel.y * scale.ry).round() as i32;
    let w = ((sel.w * scale.rx).round() as i32).max(1).min(fw - x);
    let h = ((sel.h * scale.ry).round() as i32).max(1).min(fh - y);
    IRect { x, y, w, h }
}

// ------------------------------------------------------------------------------------------------- monitors

/// One monitor in the virtual desktop, physical px (negative coordinates are fine). `scale` is the DPI scale (1.0, 1.5, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorGeom {
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub scale: f64,
}

impl MonitorGeom {
    pub fn new(index: usize, x: i32, y: i32, w: i32, h: i32, scale: f64) -> MonitorGeom {
        MonitorGeom { label: Desktop::label(index), x, y, w, h, scale }
    }
    /// The overlay window's css viewport: physical size / DPI scale.
    pub fn viewport(&self) -> (f64, f64) {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        (self.w as f64 / s, self.h as f64 / s)
    }
    pub fn contains_phys(&self, p: Pt) -> bool {
        p.x >= self.x as f64 && p.y >= self.y as f64 && p.x < (self.x + self.w) as f64 && p.y < (self.y + self.h) as f64
    }
    /// Virtual-desktop physical point -> this overlay's css point.
    pub fn phys_to_view(&self, p: Pt) -> Pt {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        Pt::new((p.x - self.x as f64) / s, (p.y - self.y as f64) / s)
    }
    /// This overlay's css point -> virtual-desktop physical point.
    pub fn view_to_phys(&self, p: Pt) -> Pt {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        Pt::new(self.x as f64 + p.x * s, self.y as f64 + p.y * s)
    }
    /// A frame-px rectangle of this monitor -> virtual-desktop px (what start_recording receives: `mon.x + r.x`).
    pub fn frame_to_screen(&self, r: IRect) -> IRect {
        IRect { x: self.x + r.x, y: self.y + r.y, w: r.w, h: r.h }
    }
}

/// All monitors of a capture session. Overlay `i` is the window labelled `capture-i`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Desktop {
    pub monitors: Vec<MonitorGeom>,
}

impl Desktop {
    pub fn label(index: usize) -> String {
        format!("capture-{index}")
    }
    pub fn index_of(label: &str) -> Option<usize> {
        label.strip_prefix("capture-")?.parse().ok()
    }
    pub fn by_label(&self, label: &str) -> Option<&MonitorGeom> {
        self.monitors.iter().find(|m| m.label == label)
    }
    /// Bounding box of all monitors in physical px.
    pub fn bounds(&self) -> Option<IRect> {
        let l = self.monitors.iter().map(|m| m.x).min()?;
        let t = self.monitors.iter().map(|m| m.y).min()?;
        let r = self.monitors.iter().map(|m| m.x + m.w).max()?;
        let b = self.monitors.iter().map(|m| m.y + m.h).max()?;
        Some(IRect { x: l, y: t, w: r - l, h: b - t })
    }
    /// The monitor under a physical desktop point (the cursor decides which overlay gets the keyboard focus).
    pub fn monitor_at(&self, p: Pt) -> Option<&MonitorGeom> {
        self.monitors.iter().find(|m| m.contains_phys(p))
    }
    /// Like `monitor_at` but never fails: the nearest monitor when the point is in a gap (non-rectangular layouts).
    pub fn nearest(&self, p: Pt) -> Option<&MonitorGeom> {
        self.monitor_at(p).or_else(|| {
            self.monitors.iter().min_by(|a, b| {
                let d = |m: &MonitorGeom| {
                    let cx = p.x.clamp(m.x as f64, (m.x + m.w) as f64);
                    let cy = p.y.clamp(m.y as f64, (m.y + m.h) as f64);
                    (p.x - cx).hypot(p.y - cy)
                };
                d(a).total_cmp(&d(b))
            })
        })
    }
}
