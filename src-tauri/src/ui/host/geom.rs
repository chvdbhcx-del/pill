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

/// Which screen edge the island hangs from (the `edge.position` setting).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dock {
    #[default]
    Top,
    Bottom,
    /// Inside the bottom taskbar strip (a widget of the bar), growing upward from it; where there is no such bar it is `Bottom`.
    Taskbar,
}

impl Dock {
    /// `"bottom"` = the bottom edge, `"taskbar"` = inside the taskbar; anything else (also an empty or unknown value) = the top, the default.
    pub fn from_setting(v: &str) -> Dock {
        if v.eq_ignore_ascii_case("bottom") {
            Dock::Bottom
        } else if v.eq_ignore_ascii_case("taskbar") {
            Dock::Taskbar
        } else {
            Dock::Top
        }
    }

    /// The island stands on the bottom edge of its window and grows upward (the bottom edge and the taskbar).
    pub fn at_bottom(self) -> bool {
        self != Dock::Top
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(c: u8) -> Option<Dock> {
        [Dock::Top, Dock::Bottom, Dock::Taskbar].get(c as usize).copied()
    }
}

/// Window position (screen, physical) for a window of `size` (physical) hanging from the `dock` edge of `mon`, centred horizontally. `work` is
/// the monitor's work area (the monitor minus the taskbar): `under_taskbar` keeps the window inside it (below a top taskbar, above a bottom
/// one; a taskbar on another side changes nothing for the edge), off it uses the physical edge of the monitor. `Dock::Taskbar` is the bottom edge
/// here (the fallback where there is no plain bottom bar; the bar itself is [`taskbar_position`]). `margin` (physical px) moves the
/// window away from that edge. Bottom grows UP: the window's bottom edge is the anchor, so a taller window reaches further up.
pub fn dock_position(dock: Dock, mon: IRect, work: IRect, under_taskbar: bool, margin: i32, size: (i32, i32), offset_pct: f32) -> (i32, i32) {
    dock_position_aligned(dock, Align::Center, mon, work, under_taskbar, margin, size, offset_pct)
}

/// Where along the edge the island stands (the `edge.align` setting, used with the bottom edge).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Center,
    Left,
    Right,
}

impl Align {
    /// `"left"` / `"right"`; anything else (also an empty or unknown value) = the centre, the default.
    pub fn from_setting(v: &str) -> Align {
        match v.to_ascii_lowercase().as_str() {
            "left" => Align::Left,
            "right" => Align::Right,
            _ => Align::Center,
        }
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(c: u8) -> Option<Align> {
        [Align::Center, Align::Left, Align::Right].get(c as usize).copied()
    }
}

/// Logical px between the window's side and the body of a left / right aligned island (the open panel is 20 px inside the 720 px frame, so
/// an open island stands exactly where it would have stood anyway).
pub const ALIGN_PAD: f32 = 20.0;

/// [`dock_position`] with the island standing at the `align` side: `Left` / `Right` put the WINDOW against that side of the monitor (of the
/// work area with `under_taskbar`: a taskbar on that side is not covered) and `offset_pct` does not apply; the body is moved to
/// [`ALIGN_PAD`] from that side inside the window by the view ([`align_body_dx`]), so the island grows from there and never leaves the monitor.
pub fn dock_position_aligned(dock: Dock, align: Align, mon: IRect, work: IRect, under_taskbar: bool, margin: i32, size: (i32, i32), offset_pct: f32) -> (i32, i32) {
    let side = |l: i32, r: i32| (l, r);
    let (wl, wr) = if under_taskbar { side(work.l.max(mon.l), work.r.min(mon.r)) } else { side(mon.l, mon.r) };
    let x = match align {
        Align::Center => anchor_top_center(mon, 0, size, offset_pct).0,
        Align::Left => wl,
        Align::Right => wr - size.0,
    };
    let x = if align == Align::Center { x } else { x.clamp(mon.l, (mon.r - size.0).max(mon.l)) };
    match dock {
        Dock::Top => (x, (if under_taskbar { work.t.max(mon.t) } else { mon.t } + margin).max(mon.t)),
        Dock::Bottom | Dock::Taskbar => (x, ((if under_taskbar { work.b.min(mon.b) } else { mon.b } - margin) - size.1).max(mon.t)),
    }
}

/// How far the view moves a body that the shell laid out centred in the window (`body_left` / `body_w` / `win_w`, all the same unit) so that
/// it stands `pad` from the left / right side of the window. 0 for the centre.
pub fn align_body_dx(align: Align, win_w: f32, body_left: f32, body_w: f32, pad: f32) -> f32 {
    match align {
        Align::Center => 0.0,
        Align::Left => pad - body_left,
        Align::Right => win_w - pad - body_w - body_left,
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The taskbar strip (`edge.position` = taskbar)
// ---------------------------------------------------------------------------------------------------------------------------------------

/// A bar thinner than this (logical px) is not a taskbar the island can live in (an auto-hidden bar leaves 0..2 px of work area behind).
pub const BAR_MIN_H: f32 = 24.0;
/// Logical px kept free above and below the folded island inside the bar.
pub const BAR_FIT_MARGIN: f32 = 4.0;
/// Logical px between the free span's side (the monitor edge or the tray) and the folded island.
pub const BAR_SIDE_PAD: f32 = 8.0;
/// Logical px between the open island and the bar it floats above.
pub const BAR_FLOAT_GAP: f32 = 8.0;

/// The bottom taskbar of a monitor as the island needs it (physical px).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    /// height of the bar strip (monitor bottom minus work-area bottom)
    pub h: i32,
    /// where the island may stand horizontally: from the monitor's left edge to the notification area (the tray), or the monitor's right edge
    pub span: (i32, i32),
}

/// The taskbar of `mon` when it is a plain, visible bar along the BOTTOM: the work area is the monitor minus a strip at the bottom and nothing
/// else (a bar at the top / left / right, an auto-hidden bar and a monitor without one give `None`, so the island falls back to the plain bottom
/// edge). `tray` is the notification area's rectangle when it is known (`taskbar::tray_rect`): the free span ends [`BAR_SIDE_PAD`] before it.
pub fn bottom_bar(mon: IRect, work: IRect, scale: f32, tray: Option<IRect>) -> Option<Bar> {
    let h = mon.b - work.b;
    let plain = work.l == mon.l && work.r == mon.r && work.t == mon.t;
    if !plain || h < phys(BAR_MIN_H, scale) || h > mon.h() / 3 {
        return None;
    }
    // the tray counts when it is a piece of the bar on the right half of the monitor
    let right = tray.filter(|t| t.r > t.l && t.b > work.b - 1 && t.l > mon.cx() && t.l < mon.r).map_or(mon.r, |t| t.l - phys(BAR_SIDE_PAD, scale));
    Some(Bar { h, span: (mon.l, right.max(mon.l)) })
}

/// Window position (screen, physical) of the island window of `size` inside `bar`: the window's BOTTOM is the monitor's bottom (the window
/// covers the bar and everything above it as far as it is tall; only its hit region is clickable). `pad` is how far the shell keeps the body from
/// the window's side (the view's [`ALIGN_PAD`]), so the body of a left / right island stands [`BAR_SIDE_PAD`] from the span's side; `min_body` is the
/// narrowest folded body, which must stay inside the span; `offset_pct` (`offsetX`) moves the island away from its side / from the middle.
pub fn taskbar_position(align: Align, mon: IRect, bar: &Bar, scale: f32, size: (i32, i32), pad: i32, min_body: i32, offset_pct: f32) -> (i32, i32) {
    let side = phys(BAR_SIDE_PAD, scale);
    let (lo, hi) = (bar.span.0 + side, (bar.span.1 - side).max(bar.span.0 + side));
    let min_body = min_body.min(hi - lo).max(0);
    let shift = (offset_pct.clamp(-50.0, 50.0) as f64 / 100.0 * mon.w() as f64).round() as i32;
    let x = match align {
        Align::Left => (lo + shift).clamp(lo, hi - min_body) - pad,
        Align::Right => (hi + shift).clamp(lo + min_body, hi) + pad - size.0,
        Align::Center => ((lo + hi) / 2 + shift).clamp(lo + min_body / 2, hi - min_body / 2) - size.0 / 2,
    };
    (x, (mon.b - size.1).max(mon.t))
}

/// The tallest folded capsule that fits a bar `bar_h` tall (all logical px).
pub fn bar_fold_cap(bar_h: f32) -> f32 {
    (bar_h - 2.0 * BAR_FIT_MARGIN).max(16.0)
}

/// Where the body's bottom edge stands above the screen's bottom edge (logical px): `raised` 0 = the folded body centred in the bar, 1 = the
/// open island floating [`BAR_FLOAT_GAP`] above it. In between it blends, so the folded pill glides up into the open one.
pub fn bar_gap(bar_h: f32, body_h: f32, raised: f32) -> f32 {
    let folded = (bar_h - body_h) / 2.0;
    let floating = bar_h + BAR_FLOAT_GAP;
    folded + (floating - folded) * raised.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The music widget in the taskbar strip (`media.widget.edge` = taskbar)
// ---------------------------------------------------------------------------------------------------------------------------------------

/// The widget lives in bars up to this tall (logical px); a taller strip is the plain bottom edge for it (its window has room for this much).
pub const WIDGET_BAR_MAX_H: f32 = 72.0;
/// Logical px of the bar the widget keeps clear on the island's side when both stand in it (the folded island is 118..500 wide, 300 at the
/// default size is the usual width of a capsule with a title). A ceiling, not a measurement: the two windows do not talk to each other.
pub const ISLAND_BAR_W: f32 = 300.0;

/// Where the island stands in the same bar (`edge.position` = taskbar), read from its settings: what the widget must stay clear of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IslandBar {
    pub align: Align,
    /// `offsetX`
    pub offset_pct: f32,
    /// the width kept for it, physical px
    pub w: i32,
}

/// Where the folded widget's body stands in the bar (physical px): the side it is anchored to and its left edge / width. A relocated widget
/// (`align` differs from the one asked for) is what the view lays itself out by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarSlot {
    pub align: Align,
    pub x: i32,
    pub w: i32,
}

/// The taskbar the widget can live in: [`bottom_bar`] that is not taller than [`WIDGET_BAR_MAX_H`].
pub fn widget_bar(mon: IRect, work: IRect, scale: f32, tray: Option<IRect>) -> Option<Bar> {
    bottom_bar(mon, work, scale, tray).filter(|b| b.h <= phys(WIDGET_BAR_MAX_H, scale))
}

/// The widget's slot in `bar`: `align` / `offset_pct` (only for the centre) as asked, a folded body `body_w` wide. When the island stands in the
/// same bar and the asked place is not clear of it by a side pad, the widget goes to the OPPOSITE side (left <-> right); a centred widget (or one
/// whose opposite side is taken too) takes the larger free stretch beside the island's side, centred in it. Pure.
pub fn widget_bar_slot(align: Align, mon: IRect, bar: &Bar, scale: f32, body_w: i32, offset_pct: f32, island: Option<IslandBar>) -> BarSlot {
    let side = phys(BAR_SIDE_PAD, scale);
    let (lo, hi) = (bar.span.0 + side, (bar.span.1 - side).max(bar.span.0 + side));
    let w = body_w.min(hi - lo).max(0);
    let at = |a: Align, pct: f32, w: i32| -> i32 {
        let shift = (pct.clamp(-50.0, 50.0) as f64 / 100.0 * mon.w() as f64).round() as i32;
        match a {
            Align::Left => (lo + shift).clamp(lo, hi - w),
            Align::Right => (hi + shift).clamp(lo + w, hi) - w,
            Align::Center => ((lo + hi) / 2 + shift).clamp(lo + w / 2, hi - w / 2) - w / 2,
        }
    };
    let own = BarSlot { align, x: at(align, if align == Align::Center { offset_pct } else { 0.0 }, w), w };
    let Some(isl) = island else { return own };
    let iw = isl.w.clamp(0, hi - lo);
    let il = at(isl.align, isl.offset_pct, iw);
    let ir = il + iw;
    let clear = |x: i32| x + w + side <= il || x >= ir + side;
    if clear(own.x) {
        return own;
    }
    if align != Align::Center {
        let other = if align == Align::Left { Align::Right } else { Align::Left };
        let x = at(other, 0.0, w);
        if clear(x) {
            return BarSlot { align: other, x, w };
        }
    }
    // the free stretches left and right of the island; the side away from it first (right of a left / centred island)
    let (left, right) = ((lo, il - side), (ir + side, hi));
    let (first, second) = if isl.align == Align::Right { (left, right) } else { (right, left) };
    let room = |f: (i32, i32)| f.1 - f.0;
    let f = if room(first) >= w {
        first
    } else if room(second) >= w {
        second
    } else if room(first) >= room(second) {
        first
    } else {
        second
    };
    BarSlot { align: Align::Center, x: ((f.0 + f.1 - w) / 2).clamp(lo, hi - w), w }
}

/// Window position (screen, physical) of the widget window of `size` for `slot`: its bottom is the monitor's bottom (it covers the bar and the
/// room above it for the open card), its body stands where the slot says, `pad` from the window's side for a left / right slot.
pub fn widget_bar_window(mon: IRect, slot: &BarSlot, pad: i32, size: (i32, i32)) -> (i32, i32) {
    let x = match slot.align {
        Align::Left => slot.x - pad,
        Align::Right => slot.x + slot.w + pad - size.0,
        Align::Center => slot.x + slot.w / 2 - size.0 / 2,
    };
    (x, (mon.b - size.1).max(mon.t))
}

/// What the music widget asks of its placement in or beside the bar (all the widget's own numbers, logical px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetAsk {
    pub align: Align,
    /// `media.widget.offset`, only for the centre
    pub offset_pct: f32,
    /// the folded body's width in the bar
    pub body_w: f32,
    /// the room between the body and the window's side
    pub pad: f32,
    /// `underTaskbar`, for the plain bottom edge where there is no bar to stand in
    pub inside_work: bool,
}

/// Where the widget window of `size` (physical) goes on a monitor: in the bar ([`widget_bar_slot`]) with the slot, or, where there is no bar the
/// widget can live in (none / at a side / auto-hidden / too tall), on the plain bottom edge exactly like the island (`dock_position_aligned`) with
/// `None`. Pure: `tray` and `island` come from the caller.
pub fn widget_place(mon: IRect, work: IRect, scale: f32, tray: Option<IRect>, island: Option<IslandBar>, ask: &WidgetAsk, size: (i32, i32)) -> ((i32, i32), Option<(Bar, BarSlot)>) {
    match widget_bar(mon, work, scale, tray) {
        Some(bar) => {
            let slot = widget_bar_slot(ask.align, mon, &bar, scale, phys(ask.body_w, scale), ask.offset_pct, island);
            (widget_bar_window(mon, &slot, phys(ask.pad, scale), size), Some((bar, slot)))
        }
        None => (dock_position_aligned(Dock::Bottom, ask.align, mon, work, ask.inside_work, 0, size, ask.offset_pct), None),
    }
}

/// Mirrors a rectangle's vertical position inside a window `h` tall: the island is laid out hanging from the top; for the bottom dock every
/// vertical position is `h - (y + height)`.
pub fn mirror_y(y: f32, height: f32, h: f32) -> f32 {
    h - (y + height)
}

/// Resizing a window keeps its top centre: the new top-left for a window that was at `old` and is now `new_size` wide / tall.
pub fn resize_keep_top_center(old: IRect, new_size: (i32, i32)) -> IRect {
    let cx = old.l + old.w() / 2;
    IRect::from_xywh(cx - new_size.0 / 2, old.t, new_size.0, new_size.1)
}

/// Resizing a window keeps its bottom centre (the island at the bottom edge grows and shrinks upward).
pub fn resize_keep_bottom_center(old: IRect, new_size: (i32, i32)) -> IRect {
    let cx = old.l + old.w() / 2;
    IRect::from_xywh(cx - new_size.0 / 2, old.b - new_size.1, new_size.0, new_size.1)
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
    fn the_top_dock_is_the_old_anchor() {
        let mon = IRect::new(0, 0, 2560, 1440);
        let work = IRect::new(0, 48, 2560, 1440); // a 48 px taskbar on top
        assert_eq!(dock_position(Dock::Top, mon, work, true, 0, (720, 120), 0.0), (920, 48));
        assert_eq!(dock_position(Dock::Top, mon, work, false, 0, (720, 120), 0.0), (920, 0));
        assert_eq!(dock_position(Dock::Top, mon, work, false, 14, (720, 120), 50.0), (1840, 14));
    }

    #[test]
    fn the_bottom_dock_sits_on_the_edge_and_grows_up() {
        let mon = IRect::new(0, 0, 2560, 1440);
        // taskbar at the bottom (48 px): above it with the switch on, over it with the switch off
        let work = IRect::new(0, 0, 2560, 1392);
        assert_eq!(dock_position(Dock::Bottom, mon, work, true, 0, (720, 120), 0.0), (920, 1272));
        assert_eq!(dock_position(Dock::Bottom, mon, work, false, 0, (720, 120), 0.0), (920, 1320));
        // the expanded frame keeps the same bottom edge and reaches 360 px further up
        assert_eq!(dock_position(Dock::Bottom, mon, work, true, 0, (720, 480), 0.0), (920, 912));
        // a margin lifts the window off the edge
        assert_eq!(dock_position(Dock::Bottom, mon, work, false, 10, (720, 120), 0.0), (920, 1310));
        // offset behaves as for the top dock
        assert_eq!(dock_position(Dock::Bottom, mon, work, true, 0, (720, 120), -50.0).0, 0);
        assert_eq!(dock_position(Dock::Bottom, mon, work, true, 0, (720, 120), 50.0).0, 1840);
    }

    #[test]
    fn a_taskbar_on_another_side_does_not_move_the_bottom_dock() {
        let mon = IRect::new(0, 0, 1920, 1080);
        for work in [IRect::new(0, 48, 1920, 1080), IRect::new(60, 0, 1920, 1080), IRect::new(0, 0, 1860, 1080)] {
            assert_eq!(dock_position(Dock::Bottom, mon, work, true, 0, (720, 120), 0.0), (600, 960), "work area {work:?}");
            assert_eq!(dock_position(Dock::Bottom, mon, work, false, 0, (720, 120), 0.0), (600, 960));
        }
        // and a top taskbar still pushes the top dock down while a bottom one does not
        assert_eq!(dock_position(Dock::Top, mon, IRect::new(0, 0, 1920, 1032), true, 0, (720, 120), 0.0), (600, 0));
    }

    #[test]
    fn docking_follows_dpi_and_the_monitor() {
        // physical sizes at 100 / 125 / 150 / 200 %: the window of 720 x 120 logical px is always flush with the bottom of the work area
        for dpi in [96u32, 120, 144, 192] {
            let s = scale_from_dpi(dpi);
            let mon = IRect::new(0, 0, (2560.0 * s) as i32, (1440.0 * s) as i32);
            let taskbar = phys(48.0, s);
            let work = IRect::new(0, 0, mon.r, mon.b - taskbar);
            let size = size_to_phys((720.0, 120.0), s);
            let (x, y) = dock_position(Dock::Bottom, mon, work, true, 0, size, 0.0);
            assert_eq!(y + size.1, work.b, "{dpi} dpi: the bottom edge is the work-area bottom");
            assert_eq!(x, (mon.w() - size.0) / 2, "{dpi} dpi: centred");
            let (_, y2) = dock_position(Dock::Bottom, mon, work, false, 0, size, 0.0);
            assert_eq!(y2 + size.1, mon.b, "{dpi} dpi: over the taskbar the window touches the monitor edge");
        }
        // a second monitor to the left and above the primary
        let m2 = IRect::new(-1920, -300, 0, 780);
        assert_eq!(dock_position(Dock::Bottom, m2, m2, true, 0, (720, 120), 0.0), (-1320, 660));
        // a window taller than the monitor is pinned to its top
        assert_eq!(dock_position(Dock::Bottom, m2, m2, true, 0, (720, 2000), 0.0).1, -300);
    }

    // ---- the taskbar strip ----

    /// A monitor of 2560 x 1440 logical px at `dpi` with a bottom taskbar `bar` logical px tall: (monitor, work area, scale).
    fn bar_screen(dpi: u32, bar: f32) -> (IRect, IRect, f32) {
        let s = scale_from_dpi(dpi);
        let mon = IRect::new(0, 0, phys(2560.0, s), phys(1440.0, s));
        (mon, IRect::new(0, 0, mon.r, mon.b - phys(bar, s)), s)
    }

    #[test]
    fn the_taskbar_dock_is_a_third_edge_that_stands_at_the_bottom() {
        assert_eq!((Dock::from_setting("taskbar"), Dock::from_setting("TaskBar"), Dock::from_setting("bottom")), (Dock::Taskbar, Dock::Taskbar, Dock::Bottom));
        assert_eq!(Dock::from_setting("in the taskbar"), Dock::Top, "an unknown value is still the top");
        assert!(Dock::Taskbar.at_bottom() && Dock::Bottom.at_bottom() && !Dock::Top.at_bottom());
        for d in [Dock::Top, Dock::Bottom, Dock::Taskbar] {
            assert_eq!(Dock::from_code(d.code()), Some(d));
        }
        assert_eq!(Dock::from_code(9), None);
    }

    #[test]
    fn a_plain_bottom_bar_is_found_at_every_height_and_dpi() {
        for dpi in [96u32, 120, 144, 192] {
            for bar in [40.0f32, 48.0, 52.0] {
                let (mon, work, s) = bar_screen(dpi, bar);
                let b = bottom_bar(mon, work, s, None).unwrap_or_else(|| panic!("{bar} px at {dpi} dpi"));
                assert_eq!(b.h, phys(bar, s), "{bar} px at {dpi} dpi");
                assert_eq!(b.span, (0, mon.r), "no tray known: the whole width");
            }
        }
    }

    #[test]
    fn no_plain_bottom_bar_means_the_plain_bottom_edge() {
        let mon = IRect::new(0, 0, 2560, 1440);
        // top, left, right bars
        assert_eq!(bottom_bar(mon, IRect::new(0, 48, 2560, 1440), 1.0, None), None, "a bar on top");
        assert_eq!(bottom_bar(mon, IRect::new(60, 0, 2560, 1440), 1.0, None), None, "a bar on the left");
        assert_eq!(bottom_bar(mon, IRect::new(0, 0, 2500, 1440), 1.0, None), None, "a bar on the right");
        // auto-hide: the whole monitor, or a stub of a few px
        assert_eq!(bottom_bar(mon, mon, 1.0, None), None, "auto-hidden");
        assert_eq!(bottom_bar(mon, IRect::new(0, 0, 2560, 1438), 1.0, None), None, "a 2 px stub is not a bar");
        // a bar at the bottom plus another one at a side
        assert_eq!(bottom_bar(mon, IRect::new(60, 0, 2560, 1392), 1.0, None), None);
        // a second monitor without a taskbar, left of the primary and higher up
        let m2 = IRect::new(-1920, -300, 0, 780);
        assert_eq!(bottom_bar(m2, m2, 1.0, None), None);
        // a second monitor WITH its own bar works on its own coordinates
        let b = bottom_bar(m2, IRect::new(-1920, -300, 0, 732), 1.0, None).unwrap();
        assert_eq!((b.h, b.span), (48, (-1920, 0)));
        // an absurdly tall strip is no bar either
        assert_eq!(bottom_bar(mon, IRect::new(0, 0, 2560, 400), 1.0, None), None);
    }

    #[test]
    fn the_free_span_ends_before_the_tray() {
        let (mon, work, s) = bar_screen(120, 48.0);
        let bar_top = work.b;
        let tray = IRect::new(2800, bar_top, mon.r, mon.b); // 400 px of notification area on the right
        let b = bottom_bar(mon, work, s, Some(tray)).unwrap();
        assert_eq!(b.span, (0, 2800 - phys(BAR_SIDE_PAD, s)));
        // a tray rectangle that is nonsense (left half, off the bar, empty) is ignored
        for t in [IRect::new(100, bar_top, 500, mon.b), IRect::new(2800, 0, mon.r, 40), IRect::new(2800, bar_top, 2800, mon.b), IRect::new(mon.r + 10, bar_top, mon.r + 100, mon.b)] {
            assert_eq!(bottom_bar(mon, work, s, Some(t)).unwrap().span, (0, mon.r), "{t:?}");
        }
        // second monitor to the right of the primary: its own tray
        let m2 = IRect::new(2560, 0, 4480, 1080);
        let w2 = IRect::new(2560, 0, 4480, 1032);
        let t2 = IRect::new(4100, 1032, 4480, 1080);
        assert_eq!(bottom_bar(m2, w2, 1.0, Some(t2)).unwrap().span, (2560, 4100 - 8));
    }

    #[test]
    fn the_taskbar_island_stands_in_the_bar_at_its_side() {
        for dpi in [96u32, 120, 144, 192] {
            for bar in [40.0f32, 48.0, 52.0] {
                let (mon, work, s) = bar_screen(dpi, bar);
                let tray = IRect::new(mon.r - phys(300.0, s), work.b, mon.r, mon.b);
                let b = bottom_bar(mon, work, s, Some(tray)).unwrap();
                let size = size_to_phys((720.0, 120.0 + bar), s); // the window is as tall as the frame plus the bar
                let (pad, minb) = (phys(ALIGN_PAD, s), phys(118.0, s));
                let side = phys(BAR_SIDE_PAD, s);
                let at = |a, off| taskbar_position(a, mon, &b, s, size, pad, minb, off);
                // left: the body (window + pad) is a side pad from the monitor's left edge; the window's bottom is the monitor's bottom
                let (x, y) = at(Align::Left, 0.0);
                assert_eq!((x + pad, y + size.1), (side, mon.b), "{bar} px at {dpi} dpi, left");
                // right: the body's right edge (window right - pad) is a side pad before the tray
                let (x, y) = at(Align::Right, 0.0);
                assert_eq!((x + size.0 - pad, y + size.1), (b.span.1 - side, mon.b), "{bar} px at {dpi} dpi, right");
                // middle of the free span
                let (x, _) = at(Align::Center, 0.0);
                assert_eq!(x + size.0 / 2, (side + b.span.1 - side) / 2, "{bar} px at {dpi} dpi, middle");
            }
        }
    }

    #[test]
    fn the_offset_moves_the_taskbar_island_but_never_off_the_span() {
        let (mon, work, s) = bar_screen(96, 48.0);
        let b = bottom_bar(mon, work, s, Some(IRect::new(2200, work.b, 2560, mon.b))).unwrap();
        let size = (720, 168);
        let near = Bar { h: 48, span: (0, 1000) };
        let at = |a, off| taskbar_position(a, mon, &b, s, size, 20, 118, off).0;
        assert_eq!(at(Align::Left, 0.0) + 20, 8);
        assert_eq!(at(Align::Left, 10.0) + 20, 8 + 256, "10 % of the monitor to the right");
        assert_eq!(at(Align::Left, -30.0) + 20, 8, "left of the side is the side");
        assert_eq!(at(Align::Left, 50.0) + 20, 8 + 1280, "half the monitor");
        // a tray in the middle of the monitor: the shift stops where the narrowest body still fits before it
        assert_eq!(taskbar_position(Align::Left, mon, &near, 1.0, size, 20, 118, 50.0).0 + 20, 1000 - 8 - 118 - 8 + 8, "and never over the tray");
        assert_eq!(at(Align::Right, 0.0) + 720 - 20, 2200 - 8 - 8);
        assert_eq!(at(Align::Right, -10.0) + 720 - 20, 2200 - 16 - 256);
        assert_eq!(at(Align::Right, 50.0) + 720 - 20, 2200 - 16, "right of the tray stays before the tray");
        assert_eq!(at(Align::Right, -50.0) + 720 - 20, 2184 - 1280);
        assert_eq!(taskbar_position(Align::Right, mon, &near, 1.0, size, 20, 118, -50.0).0 + 720 - 20, 8 + 118, "the left end keeps the narrowest body inside");
        assert_eq!(at(Align::Center, 0.0) + 360, (8 + 2200 - 16) / 2);
        assert_eq!(at(Align::Center, 50.0) + 360, 2200 - 16 - 59);
        // a span narrower than the body does not panic and puts the island at the side
        let tiny = Bar { h: 48, span: (0, 100) };
        assert_eq!(taskbar_position(Align::Left, mon, &tiny, 1.0, size, 20, 118, 0.0).0 + 20, 8);
        assert_eq!(taskbar_position(Align::Center, mon, &tiny, 1.0, size, 20, 118, 0.0).1, mon.b - 168);
    }

    #[test]
    fn the_taskbar_window_stays_on_its_own_monitor() {
        // a second monitor to the left of the primary and higher up: bottom = that monitor's bottom
        let m2 = IRect::new(-1920, -300, 0, 780);
        let b = bottom_bar(m2, IRect::new(-1920, -300, 0, 732), 1.0, Some(IRect::new(-400, 732, 0, 780))).unwrap();
        let (x, y) = taskbar_position(Align::Left, m2, &b, 1.0, (720, 168), 20, 118, 0.0);
        assert_eq!((x + 20, y + 168), (-1920 + 8, 780));
        // a window taller than the monitor is pinned to its top
        assert_eq!(taskbar_position(Align::Left, m2, &b, 1.0, (720, 2000), 20, 118, 0.0).1, -300);
    }

    #[test]
    fn the_folded_capsule_fits_the_bar_and_rises_when_it_opens() {
        for (bar, cap) in [(40.0, 32.0), (48.0, 40.0), (52.0, 44.0)] {
            assert_eq!(bar_fold_cap(bar), cap);
            // the default 30 px capsule fits every bar; folded it is centred, with the same room above and below
            let gap = bar_gap(bar, 30.0, 0.0);
            assert_eq!(gap, (bar - 30.0) / 2.0);
            assert!(gap >= BAR_FIT_MARGIN - 0.001, "{bar}: {gap}");
            // the capsule at its cap keeps the margin
            assert_eq!(bar_gap(bar, cap, 0.0), BAR_FIT_MARGIN);
            assert_eq!(bar_gap(bar, 30.0, 1.0), bar + BAR_FLOAT_GAP, "open: above the bar");
        }
        // in between it glides: monotonic and continuous
        let mut last = bar_gap(48.0, 30.0, 0.0);
        for i in 1..=10 {
            let g = bar_gap(48.0, 30.0, i as f32 / 10.0);
            assert!(g > last, "{g} > {last}");
            last = g;
        }
        assert_eq!(bar_gap(48.0, 30.0, 7.0), bar_gap(48.0, 30.0, 1.0), "raised is clamped");
        assert_eq!(bar_fold_cap(10.0), 16.0, "never a degenerate capsule");
    }

    #[test]
    fn the_left_island_stands_in_the_corner_of_the_work_area_and_grows_up() {
        let mon = IRect::new(0, 0, 2560, 1440);
        let work = IRect::new(0, 0, 2560, 1392); // taskbar at the bottom
        let at = |a, under, size| dock_position_aligned(Dock::Bottom, a, mon, work, under, 0, size, 30.0);
        assert_eq!(at(Align::Left, true, (720, 120)), (0, 1272), "offsetX does not apply to a corner");
        assert_eq!(at(Align::Left, true, (720, 480)), (0, 912), "the open frame keeps the corner and reaches up");
        assert_eq!(at(Align::Left, false, (720, 120)), (0, 1320));
        assert_eq!(at(Align::Right, true, (720, 120)), (2560 - 720, 1272));
        assert_eq!(at(Align::Center, true, (720, 120)).0, 920 + 2560 * 30 / 100, "the centre still follows offsetX");
        // a taskbar on the left keeps the corner clear of it with the switch on, covers it with the switch off
        let left_bar = IRect::new(60, 0, 2560, 1440);
        assert_eq!(dock_position_aligned(Dock::Bottom, Align::Left, mon, left_bar, true, 0, (720, 120), 0.0).0, 60);
        assert_eq!(dock_position_aligned(Dock::Bottom, Align::Left, mon, left_bar, false, 0, (720, 120), 0.0).0, 0);
        // and a right taskbar for the right corner
        let right_bar = IRect::new(0, 0, 2500, 1440);
        assert_eq!(dock_position_aligned(Dock::Bottom, Align::Right, mon, right_bar, true, 0, (720, 120), 0.0).0, 2500 - 720);
        // a window wider than the monitor never leaves it on the left
        let tiny = IRect::new(0, 0, 500, 400);
        assert_eq!(dock_position_aligned(Dock::Bottom, Align::Right, tiny, tiny, true, 0, (720, 120), 0.0).0, 0);
    }

    #[test]
    fn alignment_follows_dpi_and_the_monitor_at_both_edges() {
        for dpi in [96u32, 120, 144, 192] {
            let s = scale_from_dpi(dpi);
            let mon = IRect::new(0, 0, (2560.0 * s) as i32, (1440.0 * s) as i32);
            let work = IRect::new(0, 0, mon.r, mon.b - phys(48.0, s));
            let size = size_to_phys((720.0, 120.0), s);
            for dock in [Dock::Top, Dock::Bottom, Dock::Taskbar] {
                let (x, y) = dock_position_aligned(dock, Align::Left, mon, work, true, 0, size, 0.0);
                assert_eq!(x, 0, "{dpi} dpi {dock:?}");
                match dock {
                    Dock::Top => assert_eq!(y, 0),
                    Dock::Bottom | Dock::Taskbar => assert_eq!(y + size.1, work.b),
                }
                let (x, _) = dock_position_aligned(dock, Align::Right, mon, work, true, 0, size, 0.0);
                assert_eq!(x + size.0, mon.r, "{dpi} dpi {dock:?}");
            }
        }
        // a second monitor to the left of the primary, partly above it: the corner is the corner of THAT monitor
        let m2 = IRect::new(-1920, -300, 0, 780);
        assert_eq!(dock_position_aligned(Dock::Bottom, Align::Left, m2, m2, true, 0, (720, 120), 0.0), (-1920, 660));
        assert_eq!(dock_position_aligned(Dock::Bottom, Align::Right, m2, m2, true, 0, (720, 120), 0.0), (-720, 660));
        // a monitor on the right with 150 % scale
        let m3 = IRect::new(2560, 0, 4480, 1080);
        assert_eq!(dock_position_aligned(Dock::Bottom, Align::Left, m3, m3, true, 0, (1080, 180), 0.0), (2560, 900));
    }

    #[test]
    fn the_body_moves_inside_the_window_to_its_corner() {
        // folded capsule 200 wide, centred in a 720 window (left 260); pad 20
        assert_eq!(align_body_dx(Align::Center, 720.0, 260.0, 200.0, 20.0), 0.0);
        assert_eq!(align_body_dx(Align::Left, 720.0, 260.0, 200.0, 20.0), -240.0, "260 -> 20");
        assert_eq!(align_body_dx(Align::Right, 720.0, 260.0, 200.0, 20.0), 240.0, "260 + 200 -> 700");
        // the open panel (680 wide, left 20) is already where the corner wants it
        assert_eq!(align_body_dx(Align::Left, 720.0, 20.0, 680.0, 20.0), 0.0);
        assert_eq!(align_body_dx(Align::Right, 720.0, 20.0, 680.0, 20.0), 0.0);
        // whatever the width, the left body edge stays on the pad while it grows
        for w in [118.0, 300.0, 500.0, 680.0] {
            let left = (720.0 - w) / 2.0;
            assert_eq!(left + align_body_dx(Align::Left, 720.0, left, w, 20.0), 20.0);
            assert_eq!(left + w + align_body_dx(Align::Right, 720.0, left, w, 20.0), 700.0);
        }
    }

    #[test]
    fn the_align_setting_defaults_to_the_centre_and_codes_round_trip() {
        assert_eq!((Align::from_setting("left"), Align::from_setting("RIGHT")), (Align::Left, Align::Right));
        for v in ["center", "", "middle", "left "] {
            assert_eq!(Align::from_setting(v), Align::Center, "{v:?}");
        }
        for a in [Align::Center, Align::Left, Align::Right] {
            assert_eq!(Align::from_code(a.code()), Some(a));
        }
        assert_eq!(Align::from_code(9), None);
    }

    #[test]
    fn the_dock_setting_defaults_to_the_top() {
        assert_eq!(Dock::from_setting("bottom"), Dock::Bottom);
        assert_eq!(Dock::from_setting("Bottom"), Dock::Bottom);
        for v in ["top", "", "left", "bottom "] {
            assert_eq!(Dock::from_setting(v), Dock::Top, "{v:?}");
        }
    }

    #[test]
    fn mirroring_is_its_own_inverse() {
        assert_eq!(mirror_y(8.0, 36.0, 120.0), 76.0);
        assert_eq!(mirror_y(mirror_y(8.0, 36.0, 120.0), 36.0, 120.0), 8.0);
        assert_eq!(mirror_y(0.0, 120.0, 120.0), 0.0);
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
    fn resizing_keeps_the_bottom_centre() {
        let old = IRect::from_xywh(920, 1320, 720, 120);
        let n = resize_keep_bottom_center(old, (720, 480));
        assert_eq!((n.l, n.t, n.b, n.w()), (920, 960, 1440, 720), "taller: grows upward");
        let n = resize_keep_bottom_center(old, (400, 100));
        assert_eq!((n.cx(), n.b), (old.cx(), old.b));
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

    // ----- the music widget in the bar

    const W_BODY: f32 = 288.0;
    const W_PAD: f32 = 12.0;

    fn slot_in(dpi: u32, bar: f32, tray_w: f32, align: Align, off: f32, island: Option<(Align, f32)>) -> (IRect, Bar, BarSlot) {
        let (mon, work, s) = bar_screen(dpi, bar);
        let tray = (tray_w > 0.0).then(|| IRect::new(mon.r - phys(tray_w, s), work.b, mon.r, mon.b));
        let b = widget_bar(mon, work, s, tray).unwrap_or_else(|| panic!("{bar} px at {dpi} dpi"));
        let isl = island.map(|(a, o)| IslandBar { align: a, offset_pct: o, w: phys(ISLAND_BAR_W, s) });
        (mon, b, widget_bar_slot(align, mon, &b, s, phys(W_BODY, s), off, isl))
    }

    #[test]
    fn the_widget_stands_in_every_common_bar_at_its_side_and_never_over_the_tray() {
        for dpi in [96u32, 120, 144, 192] {
            for bar in [40.0f32, 48.0, 52.0] {
                for tray in [0.0f32, 300.0] {
                    for align in [Align::Left, Align::Center, Align::Right] {
                        let (mon, b, slot) = slot_in(dpi, bar, tray, align, 0.0, None);
                        let s = scale_from_dpi(dpi);
                        let (lo, hi) = (b.span.0 + phys(BAR_SIDE_PAD, s), b.span.1 - phys(BAR_SIDE_PAD, s));
                        assert_eq!((slot.align, slot.w), (align, phys(W_BODY, s)), "{bar}@{dpi} {align:?}");
                        assert!(slot.x >= lo && slot.x + slot.w <= hi, "{bar}@{dpi} {align:?} tray {tray}: [{}, {}) in [{lo}, {hi})", slot.x, slot.x + slot.w);
                        match align {
                            Align::Left => assert_eq!(slot.x, lo, "a side pad from the monitor edge"),
                            Align::Right => assert_eq!(slot.x + slot.w, hi, "a side pad before the tray (or the edge)"),
                            Align::Center => assert!((slot.x + slot.w / 2 - (lo + hi) / 2).abs() <= 1),
                        }
                        let size = (phys(344.0, s), phys(276.0, s));
                        let (x, y) = widget_bar_window(mon, &slot, phys(W_PAD, s), size);
                        assert_eq!(y + size.1, mon.b, "the window's floor is the monitor's bottom");
                        match align {
                            Align::Left => assert_eq!(x + phys(W_PAD, s), slot.x),
                            Align::Right => assert_eq!(x + size.0 - phys(W_PAD, s), slot.x + slot.w),
                            Align::Center => assert!((x + size.0 / 2 - (slot.x + slot.w / 2)).abs() <= 1),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn only_a_visible_plain_bottom_bar_of_a_sensible_height_holds_the_widget() {
        let mon = IRect::new(0, 0, 2560, 1440);
        assert!(widget_bar(mon, IRect::new(0, 0, 2560, 1440 - 48), 1.0, None).is_some());
        assert!(widget_bar(mon, IRect::new(0, 0, 2560, 1440 - 72), 1.0, None).is_some(), "72 px is the tallest");
        assert!(widget_bar(mon, IRect::new(0, 0, 2560, 1440 - 73), 1.0, None).is_none(), "taller: the plain bottom edge");
        assert!(widget_bar(mon, IRect::new(0, 48, 2560, 1440), 1.0, None).is_none(), "a bar on top");
        assert!(widget_bar(mon, IRect::new(60, 0, 2560, 1440), 1.0, None).is_none(), "a bar on the left");
        assert!(widget_bar(mon, IRect::new(0, 0, 2500, 1440), 1.0, None).is_none(), "a bar on the right");
        assert!(widget_bar(mon, mon, 1.0, None).is_none(), "an auto-hidden bar");
        // the limit is in logical px: 72 logical at 200 % is 144 px
        assert!(widget_bar(IRect::new(0, 0, 5120, 2880), IRect::new(0, 0, 5120, 2880 - 144), 2.0, None).is_some());
        assert!(widget_bar(IRect::new(0, 0, 5120, 2880), IRect::new(0, 0, 5120, 2880 - 150), 2.0, None).is_none());
    }

    #[test]
    fn a_centred_widget_moves_with_the_offset_and_stops_at_the_tray() {
        let (_, b, mid) = slot_in(96, 48.0, 300.0, Align::Center, 0.0, None);
        assert_eq!(b.span.1, 2560 - 300 - 8);
        let (_, _, left) = slot_in(96, 48.0, 300.0, Align::Center, -50.0, None);
        assert_eq!(left.x, 8, "all the way left: a side pad from the monitor edge");
        let (_, _, right) = slot_in(96, 48.0, 300.0, Align::Center, 50.0, None);
        assert_eq!(right.x + right.w, b.span.1 - 8, "all the way right: before the tray");
        assert!(left.x < mid.x && mid.x < right.x);
        // the offset belongs to the centre: a left widget ignores it
        let (_, _, l) = slot_in(96, 48.0, 300.0, Align::Left, 30.0, None);
        assert_eq!(l.x, 8);
    }

    #[test]
    fn beside_an_island_on_the_same_side_the_widget_takes_the_other_side() {
        let (_, b, s) = slot_in(96, 48.0, 300.0, Align::Left, 0.0, Some((Align::Left, 0.0)));
        assert_eq!((s.align, s.x + s.w), (Align::Right, b.span.1 - 8), "left + left: the widget goes right");
        let (_, _, s) = slot_in(96, 48.0, 300.0, Align::Right, 0.0, Some((Align::Right, 0.0)));
        assert_eq!((s.align, s.x), (Align::Left, 8), "right + right: the widget goes left");
        // different sides: nothing to avoid
        let (_, _, s) = slot_in(96, 48.0, 300.0, Align::Left, 0.0, Some((Align::Right, 0.0)));
        assert_eq!((s.align, s.x), (Align::Left, 8));
        let (_, _, s) = slot_in(96, 48.0, 300.0, Align::Right, 0.0, Some((Align::Left, 0.0)));
        assert_eq!(s.align, Align::Right);
    }

    #[test]
    fn a_centred_widget_keeps_clear_of_the_island() {
        let clear = |slot: &BarSlot, island: (i32, i32)| slot.x + slot.w + 8 <= island.0 || slot.x >= island.1 + 8;
        // an island on the left leaves the middle alone
        let (_, _, s) = slot_in(96, 48.0, 300.0, Align::Center, 0.0, Some((Align::Left, 0.0)));
        assert_eq!(s.align, Align::Center);
        assert!(clear(&s, (8, 308)));
        let (_, _, own) = slot_in(96, 48.0, 300.0, Align::Center, 0.0, None);
        assert_eq!(s.x, own.x, "and the widget stays where it was asked to stand");
        // dragged over the island by its offset: pushed to the free stretch on its far side
        let (_, b, s) = slot_in(96, 48.0, 300.0, Align::Center, -50.0, Some((Align::Left, 0.0)));
        assert!(clear(&s, (8, 308)) && s.x >= 308, "{s:?}");
        assert!(s.x + s.w <= b.span.1 - 8);
        // an island in the middle: the widget goes to its right
        let (_, b, s) = slot_in(96, 48.0, 300.0, Align::Center, 0.0, Some((Align::Center, 0.0)));
        let il = (b.span.0 + 8 + b.span.1 - 8) / 2 - 150;
        assert!(clear(&s, (il, il + 300)) && s.x > il, "{s:?}");
        // an island on the right, the widget dragged over it: it goes to the island's left
        let (_, b, s) = slot_in(96, 48.0, 300.0, Align::Center, 50.0, Some((Align::Right, 0.0)));
        let ir_l = b.span.1 - 8 - 300;
        assert!(clear(&s, (ir_l, b.span.1 - 8)) && s.x + s.w < ir_l, "{s:?}");
    }

    #[test]
    fn a_widget_beside_a_middle_island_flips_only_when_it_has_to() {
        // island centred: a left widget is far enough away already, a right one too
        let (_, _, l) = slot_in(96, 48.0, 300.0, Align::Left, 0.0, Some((Align::Center, 0.0)));
        let (_, _, r) = slot_in(96, 48.0, 300.0, Align::Right, 0.0, Some((Align::Center, 0.0)));
        assert_eq!((l.align, r.align), (Align::Left, Align::Right));
        // a narrow bar (a small monitor): the island takes most of it, the widget takes what is left (centred in the larger stretch)
        let mon = IRect::new(0, 0, 700, 400);
        let b = bottom_bar(mon, IRect::new(0, 0, 700, 352), 1.0, None).unwrap();
        let isl = IslandBar { align: Align::Left, offset_pct: 0.0, w: 300 };
        let s = widget_bar_slot(Align::Left, mon, &b, 1.0, 288, 0.0, Some(isl));
        assert_eq!(s.align, Align::Right);
        assert!(s.x >= 8 + 300 + 8 && s.x + s.w <= 700 - 8, "{s:?}");
        // no room anywhere: it still stands inside the bar
        let tiny = IRect::new(0, 0, 500, 400);
        let b = bottom_bar(tiny, IRect::new(0, 0, 500, 352), 1.0, None).unwrap();
        let s = widget_bar_slot(Align::Left, tiny, &b, 1.0, 288, 0.0, Some(isl));
        assert!(s.x >= 8 && s.x + s.w <= 492, "{s:?}");
    }

    #[test]
    fn the_widget_slot_survives_absurd_input() {
        let mon = IRect::new(0, 0, 300, 200);
        let b = Bar { h: 48, span: (0, 100) };
        for align in [Align::Left, Align::Center, Align::Right] {
            for isl in [None, Some(IslandBar { align: Align::Center, offset_pct: 900.0, w: 5000 })] {
                let s = widget_bar_slot(align, mon, &b, 1.0, 288, -900.0, isl);
                assert!(s.w <= 84 && s.x >= 8, "{s:?}");
            }
        }
    }

    #[test]
    fn the_widget_window_is_in_the_bar_where_there_is_one_and_on_the_plain_bottom_edge_where_not() {
        let ask = |align| WidgetAsk { align, offset_pct: 0.0, body_w: W_BODY, pad: W_PAD, inside_work: true };
        let size = (344, 276);
        let mon = IRect::new(0, 0, 2560, 1440);
        // a plain bottom bar: the window's floor is the monitor's bottom, over the bar
        let ((x, y), slot) = widget_place(mon, IRect::new(0, 0, 2560, 1392), 1.0, None, None, &ask(Align::Left), size);
        assert_eq!((x, y + size.1, slot.is_some()), (8 - 12, 1440, true));
        // no bar the widget can live in: exactly the island's plain bottom placement (above a bottom bar the work area leaves, else the edge)
        for work in [IRect::new(0, 48, 2560, 1440), IRect::new(60, 0, 2560, 1440), IRect::new(0, 0, 2500, 1440), mon, IRect::new(0, 0, 2560, 1440 - 200)] {
            for align in [Align::Left, Align::Right, Align::Center] {
                let (pos, slot) = widget_place(mon, work, 1.0, None, None, &ask(align), size);
                assert_eq!(slot, None, "{work:?}");
                assert_eq!(pos, dock_position_aligned(Dock::Bottom, align, mon, work, true, 0, size, 0.0), "{work:?} {align:?}");
            }
        }
        // a bar that is there but taller than the widget lives in
        let (pos, slot) = widget_place(mon, IRect::new(0, 0, 2560, 1440 - 100), 1.0, None, None, &ask(Align::Left), size);
        assert!(slot.is_none() && pos.1 + size.1 == 1440 - 100, "{pos:?}");
        // the tray ends the span
        let tray = IRect::new(2200, 1392, 2560, 1440);
        let (_, slot) = widget_place(mon, IRect::new(0, 0, 2560, 1392), 1.0, Some(tray), None, &ask(Align::Right), size);
        assert_eq!(slot.map(|(b, s)| (b.h, s.x + s.w)), Some((48, 2200 - 8 - 8)));
    }
}
