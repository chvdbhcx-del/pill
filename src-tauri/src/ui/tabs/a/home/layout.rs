//! Geometry of the Home tab in a 648 x 260 box (the grid of `tab-home.css`: player card 412x154, volume row 412x38, 2x4 tiles 110x44, launcher
//! strip 648x52, 8 px gaps). Everything is in tab-local logical px; the view offsets by the rect it is drawn into.
use glasskit::geom::Rect;

pub const PLAYER: Rect = Rect::new(0.0, 0.0, 412.0, 154.0);
pub const ART: Rect = Rect::new(12.0, 12.0, 88.0, 88.0);
pub const COL_X: f32 = 116.0;
pub const COL_W: f32 = 284.0;
pub const SRC: Rect = Rect::new(COL_X, 12.0, COL_W, 14.0);
pub const TITLE: Rect = Rect::new(COL_X, 28.0, COL_W, 22.0);
pub const ARTIST: Rect = Rect::new(COL_X, 52.0, COL_W, 18.0);
pub const SEEK: Rect = Rect::new(COL_X, 74.0, COL_W, 16.0);
pub const T0: Rect = Rect::new(COL_X, 94.0, 120.0, 12.0);
pub const T1: Rect = Rect::new(COL_X + COL_W - 120.0, 94.0, 120.0, 12.0);
pub const PREV: Rect = Rect::new(196.0, 114.0, 28.0, 28.0);
pub const MAIN: Rect = Rect::new(240.0, 110.0, 36.0, 36.0);
pub const NEXT: Rect = Rect::new(292.0, 114.0, 28.0, 28.0);
pub const VOL_ROW: Rect = Rect::new(0.0, 162.0, 412.0, 38.0);
pub const MUTE: Rect = Rect::new(8.0, 169.0, 24.0, 24.0);
pub const VOL: Rect = Rect::new(40.0, 173.0, 190.0, 16.0);
pub const VOL_NUM: Rect = Rect::new(238.0, 173.0, 28.0, 16.0);
pub const DEV: Rect = Rect::new(274.0, 168.0, 132.0, 26.0);
pub const STRIP: Rect = Rect::new(0.0, 208.0, 648.0, 52.0);
/// `.th-apps` padding-left, icon size, gap (pitch 50), the "+" tile.
pub const APPS_X: f32 = 4.0;
pub const APP: f32 = 40.0;
pub const APP_PITCH: f32 = 50.0;
pub const ADD: f32 = 44.0;
pub const MENU_W: f32 = 261.0;
pub const MENU_ROW: f32 = 34.0;
pub const MENU_MAX_H: f32 = 173.0;

pub fn tile(i: usize) -> Rect {
    Rect::new(420.0 + (i % 2) as f32 * 118.0, (i / 2) as f32 * 52.0, 110.0, 44.0)
}

/// The i-th launcher icon (centred in the strip).
pub fn app(i: usize) -> Rect {
    Rect::new(STRIP.x + APPS_X + i as f32 * APP_PITCH, STRIP.y + (STRIP.h - APP) / 2.0, APP, APP)
}

/// The "+" tile after `n` icons.
pub fn add(n: usize) -> Rect {
    Rect::new(STRIP.x + APPS_X + n as f32 * APP_PITCH, STRIP.y + (STRIP.h - ADD) / 2.0, ADD, ADD)
}

/// The remove badge of an icon: 16 px, 9 px above and 6 px right of the icon box.
pub fn app_x(app: Rect) -> Rect {
    Rect::new(app.right() + 6.0 - 16.0, app.y - 9.0, 16.0, 16.0)
}

/// The output-device list: opens upward over the player card, right edge on the volume row's.
pub fn menu(n: usize) -> Rect {
    let h = (10.0 + MENU_ROW * n as f32).min(MENU_MAX_H);
    Rect::new(VOL_ROW.right() - MENU_W, VOL_ROW.y - 6.0 - h, MENU_W, h)
}

/// How many launcher icons fit before the strip would scroll (the web strip scrolls sideways; see the gap list).
pub fn strip_capacity() -> usize {
    ((STRIP.w - 2.0 * APPS_X - ADD) / APP_PITCH).floor() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_adds_up_like_the_css() {
        assert_eq!(PLAYER.bottom() + 8.0, VOL_ROW.y);
        assert_eq!(VOL_ROW.bottom() + 8.0, STRIP.y);
        assert_eq!(STRIP.bottom(), 260.0);
        assert_eq!(tile(7).right(), 648.0);
        assert_eq!(tile(7).bottom(), 200.0);
        assert_eq!(tile(0).x, PLAYER.right() + 8.0);
        // controls row: 28 + 16 + 36 + 16 + 28 centred in the 284 px column
        assert_eq!(NEXT.right() - PREV.x, 124.0);
        assert_eq!(PREV.x - COL_X, COL_X + COL_W - NEXT.right());
        assert_eq!(DEV.right() + 6.0, VOL_ROW.right());
        assert_eq!(VOL.right() + 8.0, VOL_NUM.x);
        assert_eq!(VOL_NUM.right() + 8.0, DEV.x);
    }

    #[test]
    fn menu_opens_above_the_row_and_caps_at_five_rows() {
        assert_eq!(menu(1).h, 44.0);
        assert_eq!(menu(10).h, 173.0);
        assert_eq!(menu(3).bottom(), VOL_ROW.y - 6.0);
        assert_eq!(menu(3).right(), VOL_ROW.right());
        assert_eq!(app(0).y, 208.0 + 6.0);
        assert_eq!(app_x(app(0)).right(), app(0).right() + 6.0);
        assert_eq!(strip_capacity(), 11);
    }
}
