//! Pure geometry of the host: physical-pixel rectangles, DPI conversion (logical px everywhere above the host), the click-through hit region,
//! window anchoring and the borderless-resize hit test. Nothing here calls Win32, so all of it is unit-tested.
use glasskit::geom::Rect;

/// A rectangle in PHYSICAL (device) pixels, edges `[l, r) x [t, b)`, screen or client coordinates depending on the user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IRect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl IRect {
    pub const fn new(l: i32, t: i32, r: i32, b: i32) -> IRect {
        IRect { l, t, r, b }
    }
    pub fn from_xywh(x: i32, y: i32, w: i32, h: i32) -> IRect {
        IRect { l: x, t: y, r: x + w, b: y + h }
    }
    pub fn w(&self) -> i32 {
        self.r - self.l
    }
    pub fn h(&self) -> i32 {
        self.b - self.t
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
    pub fn cx(&self) -> i32 {
        self.l + self.w() / 2
    }
    /// Grown by `m` on every side.
    pub fn inflate(&self, m: i32) -> IRect {
        IRect { l: self.l - m, t: self.t - m, r: self.r + m, b: self.b + m }
    }
    pub fn offset(&self, dx: i32, dy: i32) -> IRect {
        IRect { l: self.l + dx, t: self.t + dy, r: self.r + dx, b: self.b + dy }
    }
    pub fn intersect(&self, o: &IRect) -> Option<IRect> {
        let r = IRect { l: self.l.max(o.l), t: self.t.max(o.t), r: self.r.min(o.r), b: self.b.min(o.b) };
        (r.r > r.l && r.b > r.t).then_some(r)
    }
    pub fn union(&self, o: &IRect) -> IRect {
        IRect { l: self.l.min(o.l), t: self.t.min(o.t), r: self.r.max(o.r), b: self.b.max(o.b) }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// DPI
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Device px per logical px for a DPI value (96 = 1.0). Never below 0.25 (a bad `GetDpiForWindow` answer of 0 must not divide by zero).
pub fn scale_from_dpi(dpi: u32) -> f32 {
    (dpi as f32 / 96.0).max(0.25)
}

/// Logical -> physical, rounded to the nearest pixel.
pub fn phys(v: f32, scale: f32) -> i32 {
    (v * scale).round() as i32
}

/// Physical -> logical.
pub fn logical(px: i32, scale: f32) -> f32 {
    px as f32 / scale
}

/// A logical size as a window size in device px (at least 1 x 1).
pub fn size_to_phys(size: (f32, f32), scale: f32) -> (i32, i32) {
    (phys(size.0, scale).max(1), phys(size.1, scale).max(1))
}

/// A logical rectangle as physical pixels, outward-rounded so a hit region never loses an edge pixel to rounding.
pub fn rect_to_phys(r: Rect, scale: f32) -> IRect {
    IRect { l: (r.x * scale).floor() as i32, t: (r.y * scale).floor() as i32, r: (r.right() * scale).ceil() as i32, b: (r.bottom() * scale).ceil() as i32 }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Click-through hit region
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Which parts of an Overlay window take the mouse. `All` is the default; `Rects` (logical px) makes everything else click-through.
#[derive(Clone, Debug, PartialEq)]
pub enum HitRegion {
    All,
    Rects(Vec<Rect>),
}

impl Default for HitRegion {
    fn default() -> Self {
        HitRegion::All
    }
}

impl HitRegion {
    /// Is the logical point (client coordinates) inside the clickable part?
    pub fn contains(&self, x: f32, y: f32) -> bool {
        match self {
            HitRegion::All => true,
            HitRegion::Rects(r) => r.iter().any(|r| r.contains(x, y)),
        }
    }

    /// Bounding box of the clickable parts in logical px (`None` for `All` or an empty region).
    pub fn bounds(&self) -> Option<Rect> {
        match self {
            HitRegion::All => None,
            HitRegion::Rects(r) => {
                let mut it = r.iter();
                let first = it.next()?;
                let (mut l, mut t, mut rr, mut b) = (first.x, first.y, first.right(), first.bottom());
                for r in it {
                    l = l.min(r.x);
                    t = t.min(r.y);
                    rr = rr.max(r.right());
                    b = b.max(r.bottom());
                }
                Some(Rect::new(l, t, rr - l, b - t))
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Anchoring and position memory
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Window position (screen, physical) for a window of `size` (physical) centred horizontally on `mon` at the top: `top` is the first row it
/// may occupy (the monitor top, or the work-area top under a top taskbar), `offset_pct` shifts the centre by that percentage of the monitor
/// width (-50..50, the `offsetX` setting). The window never leaves the monitor horizontally. Same maths as `pill.rs::layout`.
pub fn anchor_top_center(mon: IRect, top: i32, size: (i32, i32), offset_pct: f32) -> (i32, i32) {
    let cx = mon.l as f64 + mon.w() as f64 / 2.0 + offset_pct.clamp(-50.0, 50.0) as f64 / 100.0 * mon.w() as f64;
    let x = ((cx - size.0 as f64 / 2.0).round() as i32).clamp(mon.l, (mon.r - size.0).max(mon.l));
    (x, top.max(mon.t))
}

/// Resizing a window keeps its top centre: the new top-left for a window that was at `old` and is now `new_size` wide / tall.
pub fn resize_keep_top_center(old: IRect, new_size: (i32, i32)) -> IRect {
    let cx = old.l + old.w() / 2;
    IRect::from_xywh(cx - new_size.0 / 2, old.t, new_size.0, new_size.1)
}

/// True when at least a 120 x 40 px piece of the title-bar strip of a `w`-wide window at (x, y) lies on one of the monitors. A remembered
/// position that fails this (a monitor was unplugged) is ignored. Same rule as `settingswin::pos_visible`, on `IRect`s.
pub fn pos_visible(x: i32, y: i32, w: i32, mons: &[IRect]) -> bool {
    let strip = IRect::from_xywh(x, y, w, 40);
    mons.iter().any(|m| strip.intersect(m).is_some_and(|i| i.w() >= 120 && i.h() >= 40))
}

/// Centre a `w` x `h` window on `mon`.
pub fn center_on(mon: IRect, w: i32, h: i32) -> (i32, i32) {
    (mon.l + (mon.w() - w) / 2, mon.t + (mon.h() - h) / 2)
}

/// Which monitor (index) holds the point; `None` when none does.
pub fn monitor_at(mons: &[IRect], x: i32, y: i32) -> Option<usize> {
    mons.iter().position(|m| m.contains(x, y))
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Borderless window hit test (Normal windows with a view-drawn title bar)
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    None,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Resize border hit test: the point (client px) of a `w` x `h` window and the border thickness `b` (physical px). Corners get a square of
/// `2 * b` so they are easy to hit.
pub fn resize_edge(x: i32, y: i32, w: i32, h: i32, b: i32) -> Edge {
    if x < 0 || y < 0 || x >= w || y >= h {
        return Edge::None;
    }
    let c = b * 2;
    let (l, r, t, bo) = (x < b, x >= w - b, y < b, y >= h - b);
    let (lc, rc, tc, bc) = (x < c, x >= w - c, y < c, y >= h - c);
    match (l, r, t, bo) {
        (true, _, _, _) if tc => Edge::TopLeft,
        (true, _, _, _) if bc => Edge::BottomLeft,
        (true, _, _, _) => Edge::Left,
        (_, true, _, _) if tc => Edge::TopRight,
        (_, true, _, _) if bc => Edge::BottomRight,
        (_, true, _, _) => Edge::Right,
        (_, _, true, _) if lc => Edge::TopLeft,
        (_, _, true, _) if rc => Edge::TopRight,
        (_, _, true, _) => Edge::Top,
        (_, _, _, true) if lc => Edge::BottomLeft,
        (_, _, _, true) if rc => Edge::BottomRight,
        (_, _, _, true) => Edge::Bottom,
        _ => Edge::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_conversion_round_trips_at_common_scales() {
        for (dpi, s) in [(96, 1.0), (120, 1.25), (144, 1.5), (192, 2.0), (288, 3.0)] {
            assert_eq!(scale_from_dpi(dpi), s);
            for px in [0, 1, 17, 480, 3839] {
                assert_eq!(phys(logical(px, s), s), px, "{px} px at {dpi} dpi");
            }
        }
        assert_eq!(scale_from_dpi(0), 0.25, "a failed GetDpi must not divide by zero");
        assert_eq!(size_to_phys((720.0, 120.5), 1.5), (1080, 181));
        assert_eq!(size_to_phys((0.0, -3.0), 1.0), (1, 1));
    }

    #[test]
    fn hit_rects_round_outwards() {
        let r = rect_to_phys(Rect::new(10.5, 6.0, 20.0, 10.0), 1.25);
        assert_eq!(r, IRect::new(13, 7, 39, 20));
        assert!(r.contains(13, 7) && !r.contains(39, 19));
    }

    #[test]
    fn hit_region_all_and_rects() {
        assert!(HitRegion::All.contains(-5.0, 1e6));
        let h = HitRegion::Rects(vec![Rect::new(100.0, 0.0, 200.0, 40.0), Rect::new(150.0, 40.0, 100.0, 14.0)]);
        assert!(h.contains(100.0, 0.0) && h.contains(299.9, 39.9) && h.contains(200.0, 50.0));
        assert!(!h.contains(99.9, 10.0) && !h.contains(120.0, 45.0) && !h.contains(200.0, 54.0));
        assert_eq!(h.bounds(), Some(Rect::new(100.0, 0.0, 200.0, 54.0)));
        assert_eq!(HitRegion::Rects(vec![]).bounds(), None);
        assert!(!HitRegion::Rects(vec![]).contains(0.0, 0.0), "an empty region is fully click-through");
        assert_eq!(HitRegion::All.bounds(), None);
    }

    #[test]
    fn anchor_matches_the_pill_layout() {
        let mon = IRect::new(0, 0, 2560, 1440);
        assert_eq!(anchor_top_center(mon, 0, (720, 120), 0.0), (920, 0));
        // under a 48 px top taskbar
        assert_eq!(anchor_top_center(mon, 48, (720, 120), 0.0), (920, 48));
        // offsetX = 50 % pushes against the right edge and stays on the monitor
        assert_eq!(anchor_top_center(mon, 0, (720, 120), 50.0), (1840, 0));
        assert_eq!(anchor_top_center(mon, 0, (720, 120), -50.0), (0, 0));
        assert_eq!(anchor_top_center(mon, 0, (720, 120), 400.0), (1840, 0), "clamped to 50 %");
        // second monitor to the left
        let m2 = IRect::new(-1920, 0, 0, 1080);
        assert_eq!(anchor_top_center(m2, 0, (720, 120), 0.0), (-1320, 0));
        // wider than the monitor: pinned to the left edge
        assert_eq!(anchor_top_center(m2, 0, (2500, 120), 0.0).0, -1920);
    }

    #[test]
    fn resizing_keeps_the_top_centre() {
        let old = IRect::from_xywh(920, 10, 720, 120);
        let n = resize_keep_top_center(old, (720, 480));
        assert_eq!((n.l, n.t, n.w(), n.h()), (920, 10, 720, 480));
        let n = resize_keep_top_center(old, (400, 100));
        assert_eq!((n.cx(), n.t), (old.cx(), 10));
    }

    #[test]
    fn remembered_positions_must_still_be_reachable() {
        let mons = [IRect::new(0, 0, 1920, 1080), IRect::new(1920, 0, 3840, 1080)];
        assert!(pos_visible(100, 100, 840, &mons));
        assert!(pos_visible(1800, 100, 840, &mons), "straddling two monitors");
        assert!(!pos_visible(5000, 100, 840, &mons), "the monitor is gone");
        assert!(!pos_visible(-800, 100, 840, &mons), "only 40 px on screen");
        assert!(!pos_visible(100, 1070, 840, &mons), "title bar below the bottom edge");
        assert!(!pos_visible(0, 0, 840, &[]));
    }

    #[test]
    fn centring_and_monitor_lookup() {
        let m = IRect::new(100, 50, 1100, 850);
        assert_eq!(center_on(m, 400, 200), (400, 350));
        let mons = [IRect::new(0, 0, 10, 10), IRect::new(10, 0, 20, 10)];
        assert_eq!(monitor_at(&mons, 10, 5), Some(1));
        assert_eq!(monitor_at(&mons, 20, 5), None);
    }

    #[test]
    fn resize_border_hit_test() {
        let (w, h, b) = (800, 600, 6);
        assert_eq!(resize_edge(400, 300, w, h, b), Edge::None);
        assert_eq!(resize_edge(0, 300, w, h, b), Edge::Left);
        assert_eq!(resize_edge(799, 300, w, h, b), Edge::Right);
        assert_eq!(resize_edge(400, 0, w, h, b), Edge::Top);
        assert_eq!(resize_edge(400, 599, w, h, b), Edge::Bottom);
        assert_eq!(resize_edge(0, 0, w, h, b), Edge::TopLeft);
        assert_eq!(resize_edge(10, 2, w, h, b), Edge::TopLeft, "the corner square is 2x the border");
        assert_eq!(resize_edge(790, 3, w, h, b), Edge::TopRight);
        assert_eq!(resize_edge(2, 590, w, h, b), Edge::BottomLeft);
        assert_eq!(resize_edge(799, 599, w, h, b), Edge::BottomRight);
        assert_eq!(resize_edge(-1, 10, w, h, b), Edge::None);
        assert_eq!(resize_edge(800, 10, w, h, b), Edge::None);
    }
}
