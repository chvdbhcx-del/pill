//! Pure geometry of the Controls tab (numbers of `src/tab-controls.css`): 4 / 3 / 2 columns of 172 px tiles, the 76 px power row, the 24 px list
//! button and the Bluetooth popover. `bounds` is the 648 x 260 content box.
use glasskit::geom::Rect;

pub const GAP: f32 = 12.0;
pub const TILE_H: f32 = 172.0;
pub const POWER_H: f32 = 76.0;
pub const RADIUS: f32 = 18.0;
pub const POP_W: f32 = 270.0;
pub const POP_TOP: f32 = 44.0;
pub const ROW_H: f32 = 32.0;
pub const ROW_GAP: f32 = 2.0;
pub const LIST_MAX_H: f32 = 180.0;

/// Tile rectangles for `n` toggle tiles: `repeat(4, 1fr)`, 9fr 8fr 9fr for three, halves for two (`.tc-bigrow[data-n]`).
pub fn tiles(b: Rect, n: usize) -> Vec<Rect> {
    let weights: Vec<f32> = match n {
        3 => vec![9.0, 8.0, 9.0],
        2 => vec![1.0, 1.0],
        _ => vec![1.0; 4],
    };
    let cols = weights.len();
    let free = b.w - GAP * (cols as f32 - 1.0);
    let sum: f32 = weights.iter().sum();
    let mut x = b.x;
    weights
        .iter()
        .take(n)
        .map(|w| {
            let cw = free * w / sum;
            let r = Rect::new(x, b.y, cw, TILE_H);
            x += cw + GAP;
            r
        })
        .collect()
}

/// The four power tiles under the toggle row.
pub fn power(b: Rect) -> Vec<Rect> {
    let y = b.y + TILE_H + GAP;
    (0..4).map(|i| Rect::new(b.x, y, b.w, POWER_H).col(i, 4, GAP)).collect()
}

/// The 24 px round list button in the top right corner of a tile.
pub fn list_button(tile: Rect) -> Rect {
    Rect::new(tile.right() - 12.0 - 24.0, tile.y + 12.0, 24.0, 24.0)
}

/// The icon circle in the top left corner of a tile.
pub fn icon_circle(tile: Rect) -> Rect {
    Rect::new(tile.x + 12.0, tile.y + 12.0, 40.0, 40.0)
}

/// Where a tile's name and state lines sit (12 px inset, bottom aligned: state 16 px, 2 px gap, name 18 px).
pub fn tile_text(tile: Rect) -> (Rect, Rect) {
    let w = tile.w - 24.0;
    (Rect::new(tile.x + 12.0, tile.bottom() - 12.0 - 16.0 - 2.0 - 18.0, w, 18.0), Rect::new(tile.x + 12.0, tile.bottom() - 12.0 - 16.0, w, 16.0))
}

/// Icon box and label line of a power tile: icon 21 + gap 7 + label 16 = 44 in 76, with the 2 px optical nudge.
pub fn power_parts(tile: Rect) -> (Rect, Rect) {
    let top = tile.y + (POWER_H - 2.0 - 44.0) / 2.0 + 2.0;
    (Rect::new(tile.cx() - 10.5, top, 21.0, 21.0), Rect::new(tile.x + 4.0, top + 21.0 + 7.0, tile.w - 8.0, 16.0))
}

/// The popover: outer box (border included), header line, scrolling list viewport + content height, and the "connect in Windows settings" button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pop {
    pub outer: Rect,
    pub header: Rect,
    pub list: Rect,
    pub content_h: f32,
    pub button: Rect,
}

/// `rows` device rows, or the one-line empty text when `rows == 0`.
pub fn popover(b: Rect, rows: usize) -> Pop {
    let content_h = if rows == 0 { 48.0 } else { rows as f32 * ROW_H + (rows as f32 - 1.0) * ROW_GAP };
    let list_h = content_h.min(LIST_MAX_H);
    let inner = 1.0 + 10.0;
    let h = inner + 26.0 + list_h + 8.0 + 32.0 + inner;
    let outer = Rect::new(b.cx() - POP_W / 2.0, b.y + POP_TOP, POP_W, h);
    let x = outer.x + inner;
    let w = POP_W - 2.0 * inner;
    let header = Rect::new(x, outer.y + inner, w, 26.0);
    let list = Rect::new(x, header.bottom(), w, list_h);
    let button = Rect::new(x, list.bottom() + 8.0, w, 32.0);
    Pop { outer, header, list, content_h, button }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Rect = Rect::new(0.0, 0.0, 648.0, 260.0);

    #[test]
    fn four_tiles_are_153_wide_on_whole_pixels() {
        let t = tiles(B, 4);
        assert_eq!(t.len(), 4);
        assert!(t.iter().all(|r| r.w == 153.0 && r.h == 172.0));
        assert_eq!(t[3].right(), 648.0);
        assert_eq!(t[1].x, 165.0);
    }

    #[test]
    fn three_tiles_are_9_8_9_and_two_are_halves() {
        let t = tiles(B, 3);
        assert_eq!((t[0].w, t[1].w, t[2].w), (216.0, 192.0, 216.0));
        assert_eq!(t[2].right(), 648.0);
        let t2 = tiles(B, 2);
        assert_eq!((t2[0].w, t2[1].x), (318.0, 330.0));
    }

    #[test]
    fn the_stack_is_260_high() {
        let p = power(B);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].bottom(), 260.0);
        assert_eq!(p[0].y, 184.0);
        assert_eq!(p[3].right(), 648.0);
    }

    #[test]
    fn power_group_is_centred_with_the_nudge() {
        let (ic, lb) = power_parts(power(B)[0]);
        assert_eq!(ic.y - 184.0, 17.0);
        assert_eq!(lb.y - 184.0, 45.0);
        assert_eq!(lb.bottom() - 184.0, 61.0);
    }

    #[test]
    fn popover_hangs_under_the_middle_and_caps_its_list() {
        let p = popover(B, 3);
        assert_eq!(p.outer.x, 189.0);
        assert_eq!(p.outer.y, 44.0);
        assert_eq!(p.content_h, 3.0 * 32.0 + 4.0);
        assert_eq!(p.outer.h, 22.0 + 26.0 + 100.0 + 8.0 + 32.0);
        let big = popover(B, 20);
        assert_eq!(big.list.h, 180.0);
        assert!(big.content_h > 180.0);
        assert_eq!(popover(B, 0).list.h, 48.0);
    }
}
