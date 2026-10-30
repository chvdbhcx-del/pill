//! Pure geometry of the shelf tab (numbers of `tab-shelf.css`): the zone with its 1 px border, the 36 px head, the scrolling list of 102 x 104 chips.
use glasskit::geom::{Pt, Rect};

pub const CHIP_W: f32 = 102.0;
pub const CHIP_H: f32 = 104.0;
/// `.sh-list { gap: 4px 0 }`
pub const ROW_GAP: f32 = 4.0;
/// `.sh-list { padding: 0 8px }`
pub const LIST_PAD: f32 = 8.0;
pub const HEAD_H: f32 = 36.0;
/// `.sh-zone { gap: 8px }`
pub const GAP: f32 = 8.0;
pub const BORDER: f32 = 1.0;
pub const ZONE_R: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The bordered zone (the whole tab).
    pub zone: Rect,
    /// The head row (Drag all / Clear); zero height while the shelf is empty.
    pub head: Rect,
    /// The scrolling viewport of the chips.
    pub list: Rect,
}

pub fn frame(rect: Rect, head_visible: bool) -> Frame {
    let inner = rect.inset(BORDER);
    if !head_visible {
        return Frame { zone: rect, head: Rect::new(inner.x, inner.y, inner.w, 0.0), list: Rect::new(inner.x, inner.y, inner.w, 0.0) };
    }
    let (head, rest) = inner.cut_top(HEAD_H);
    let (_, list) = rest.cut_top(GAP);
    Frame { zone: rect, head, list }
}

/// Chips per row (6 in the 648 px tab).
pub fn cols(list_w: f32) -> usize {
    (((list_w - 2.0 * LIST_PAD) / CHIP_W).floor() as usize).max(1)
}

/// Chip `i` in content coordinates (origin = the list's top-left, before scrolling).
pub fn chip_rect(i: usize, cols: usize) -> Rect {
    let (r, c) = (i / cols, i % cols);
    Rect::new(LIST_PAD + c as f32 * CHIP_W, r as f32 * (CHIP_H + ROW_GAP), CHIP_W, CHIP_H)
}

pub fn content_h(n: usize, cols: usize) -> f32 {
    if n == 0 {
        return 0.0;
    }
    let rows = (n + cols - 1) / cols;
    rows as f32 * CHIP_H + (rows - 1) as f32 * ROW_GAP
}

/// The chip under a content-space point.
pub fn hit_chip(p: Pt, n: usize, cols: usize) -> Option<usize> {
    if p.0 < LIST_PAD || p.1 < 0.0 {
        return None;
    }
    let c = ((p.0 - LIST_PAD) / CHIP_W).floor() as usize;
    if c >= cols {
        return None;
    }
    let row = (p.1 / (CHIP_H + ROW_GAP)).floor() as usize;
    let i = row * cols + c;
    (i < n && chip_rect(i, cols).contains(p.0, p.1)).then_some(i)
}

/// The remove button of a chip (`.sh-x`: 18 px, 4 px from the top-right corner).
pub fn x_btn(chip: Rect) -> Rect {
    Rect::new(chip.right() - 4.0 - 18.0, chip.y + 4.0, 18.0, 18.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_648_tab_has_a_214_px_list_and_six_columns() {
        let f = frame(Rect::new(0.0, 0.0, 648.0, 260.0), true);
        assert_eq!(f.head.h, 36.0);
        assert_eq!(f.list.h, 214.0);
        assert_eq!(f.list.y, 1.0 + 36.0 + 8.0);
        assert_eq!(cols(f.list.w), 6);
        assert_eq!(content_h(12, 6), 2.0 * 104.0 + 4.0);
        assert_eq!(content_h(0, 6), 0.0);
        assert_eq!(content_h(7, 6), 212.0);
    }

    #[test]
    fn hit_testing_finds_the_chip_and_not_the_gaps() {
        let (n, c) = (8, 6);
        assert_eq!(hit_chip((10.0, 5.0), n, c), Some(0));
        assert_eq!(hit_chip((LIST_PAD + 102.0 * 5.0 + 1.0, 5.0), n, c), Some(5));
        assert_eq!(hit_chip((LIST_PAD + 1.0, 104.0 + 2.0), n, c), None, "row gap");
        assert_eq!(hit_chip((LIST_PAD + 1.0, 108.0 + 1.0), n, c), Some(6));
        assert_eq!(hit_chip((LIST_PAD + 102.0 * 2.0 + 1.0, 109.0), n, c), None, "past the last chip");
        assert_eq!(hit_chip((2.0, 5.0), n, c), None, "left padding");
        let x = x_btn(chip_rect(0, 6));
        assert!(x.contains(chip_rect(0, 6).right() - 10.0, chip_rect(0, 6).y + 10.0));
    }

    #[test]
    fn an_empty_shelf_has_no_head() {
        let f = frame(Rect::new(0.0, 0.0, 648.0, 260.0), false);
        assert_eq!((f.head.h, f.list.h), (0.0, 0.0));
    }
}
