//! Colour-picker loupe (capture mode `color`): the sample position in frame px, the 15x15 magnified neighbourhood, hex / rgb
//! formatting, and where the lens box sits next to the pointer. Port of `setPos`, `at`, `drawLens` and `pickCommit` in capture.js.
//!
//! The lens is `2*R+1 = 15` cells of a 135 px canvas (`CELL = 135/15 = 9` px each); the middle cell is outlined (3 px black, then
//! 1.5 px white). The lens box is offset 26 px from the sample, flipping to the other side of the pointer when it would leave the
//! viewport (`lens_pos`). The pixel marker is a 14 px square centred on the sampled pixel.
use super::geom::{Pt, Scale};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Cells either side of the centre (`R` in capture.js).
pub const RADIUS: i32 = 7;
/// Cells per side (`N = 2R+1`).
pub const GRID: i32 = 2 * RADIUS + 1;
/// Lens canvas size in css px.
pub const CANVAS: f64 = 135.0;
/// One cell in css px.
pub const CELL: f64 = CANVAS / GRID as f64;
/// Gap between the sample and the lens box.
pub const OFFSET: f64 = 26.0;
/// Box size used before it has been measured (`offsetWidth || 147`, `offsetHeight || 215`).
pub const DEFAULT_BOX: (f64, f64) = (147.0, 215.0);
/// Pixel marker size.
pub const MARKER: f64 = 14.0;
/// Arrow-key step (frame px); Shift multiplies it.
pub const STEP: i32 = 1;
pub const STEP_BIG: i32 = 10;
/// Colour of a cell outside the frame (`#111`).
pub const OUTSIDE: [u8; 3] = [0x11, 0x11, 0x11];

/// `#RRGGBB`, upper case (`hex2`).
pub fn hex_of(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// `rgb(r, g, b)` as the lens prints it.
pub fn rgb_text(c: [u8; 3]) -> String {
    format!("rgb({}, {}, {})", c[0], c[1], c[2])
}

/// Parse `#rgb`-less `#RRGGBB` / `RRGGBB`.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// `hsl(h, s%, l%)` - extra format the native lens can show next to hex / rgb.
pub fn hsl_text(c: [u8; 3]) -> String {
    let (r, g, b) = (c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    let (h, s) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if mx == r { ((g - b) / d).rem_euclid(6.0) } else if mx == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
        (h * 60.0, s)
    };
    format!("hsl({}, {}%, {}%)", h.round() as i32, (s * 100.0).round() as i32, (l * 100.0).round() as i32)
}

/// One cell of the magnified grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    /// Column / row inside the grid, 0..GRID.
    pub i: u8,
    pub j: u8,
    pub rgb: [u8; 3],
    /// False when the cell lies outside the frame (painted [`OUTSIDE`]).
    pub inside: bool,
}

/// Everything the renderer draws for the lens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoupeView {
    pub visible: bool,
    /// 225 cells, row-major.
    pub cells: Vec<Cell>,
    /// Pixel centre cell outline rectangle inside the canvas: `(x, y, w, h)` (`R*cell+1`, `cell-2`).
    pub centre_box: (f64, f64, f64, f64),
    pub hex: String,
    pub rgb: String,
    pub hsl: String,
    /// Swatch colour (same as `hex`).
    pub swatch: String,
    /// `x 12  y 34` (two spaces, as the page prints it).
    pub keys: String,
    /// Sample position, frame px.
    pub sample: (i32, i32),
    /// Lens box top-left, css px.
    pub box_pos: Pt,
    /// Pixel marker centre, css px.
    pub marker: Pt,
}

/// Loupe state for one overlay.
#[derive(Clone, Debug, Default)]
pub struct Loupe {
    pub fw: i32,
    pub fh: i32,
    pub cx: i32,
    pub cy: i32,
    px: Option<Arc<Vec<u8>>>,
    /// Lens box size as measured by the renderer (falls back to [`DEFAULT_BOX`]).
    pub box_size: Option<(f64, f64)>,
}

impl Loupe {
    /// New loupe over an RGBA frame; the sample starts at the centre (`floor(fw/2)`, `floor(fh/2)`). The frame must be exactly
    /// `fw*fh*4` bytes - the caller checks (the page cancels the session on a mismatch).
    pub fn new(fw: i32, fh: i32, rgba: Arc<Vec<u8>>) -> Loupe {
        Loupe { fw, fh, cx: fw / 2, cy: fh / 2, px: Some(rgba), box_size: None }
    }
    pub fn has_frame(&self) -> bool {
        self.px.is_some()
    }
    /// Release the pixels (`clearAll`).
    pub fn clear(&mut self) {
        self.px = None;
    }
    /// `setPos`: floor and clamp into the frame. Takes frame px (the page multiplies the pointer css position by `rx`/`ry`).
    pub fn set_pos(&mut self, x: f64, y: f64) {
        self.cx = (x.floor() as i64).clamp(0, (self.fw - 1).max(0) as i64) as i32;
        self.cy = (y.floor() as i64).clamp(0, (self.fh - 1).max(0) as i64) as i32;
    }
    /// Arrow-key move by whole frame pixels.
    pub fn step(&mut self, dx: i32, dy: i32, big: bool) {
        let st = if big { STEP_BIG } else { STEP };
        self.set_pos((self.cx + dx * st) as f64, (self.cy + dy * st) as f64);
    }
    /// `at(x, y)`: RGB of a frame pixel, `None` outside the frame or without pixels.
    pub fn at(&self, x: i32, y: i32) -> Option<[u8; 3]> {
        let px = self.px.as_ref()?;
        if x < 0 || y < 0 || x >= self.fw || y >= self.fh {
            return None;
        }
        let o = (y as usize * self.fw as usize + x as usize) * 4;
        Some([*px.get(o)?, *px.get(o + 1)?, *px.get(o + 2)?])
    }
    /// The colour under the sample.
    pub fn current(&self) -> Option<[u8; 3]> {
        self.at(self.cx, self.cy)
    }
    /// Sample centre in css px (`(cx+0.5)/rx`): where the marker sits and what `pickCommit` sends.
    pub fn css_centre(&self, scale: Scale) -> Pt {
        Pt::new((self.cx as f64 + 0.5) / scale.rx, (self.cy as f64 + 0.5) / scale.ry)
    }
    /// Top-left of the lens box: right / below the sample, flipping to the left / above when it would leave the viewport.
    pub fn lens_pos(&self, scale: Scale, vw: f64, vh: f64) -> Pt {
        let c = self.css_centre(scale);
        let (w, h) = self.box_size.unwrap_or(DEFAULT_BOX);
        let x = if c.x + OFFSET + w > vw { c.x - OFFSET - w } else { c.x + OFFSET };
        let y = if c.y + OFFSET + h > vh { c.y - OFFSET - h } else { c.y + OFFSET };
        Pt::new(x, y)
    }
    /// The full view, or `None` when the sample has no pixel (the page then leaves the previous lens untouched).
    pub fn view(&self, scale: Scale, vw: f64, vh: f64) -> Option<LoupeView> {
        let c = self.current()?;
        let mut cells = Vec::with_capacity((GRID * GRID) as usize);
        for j in 0..GRID {
            for i in 0..GRID {
                let p = self.at(self.cx - RADIUS + i, self.cy - RADIUS + j);
                cells.push(Cell { i: i as u8, j: j as u8, rgb: p.unwrap_or(OUTSIDE), inside: p.is_some() });
            }
        }
        let hex = hex_of(c);
        Some(LoupeView {
            visible: true,
            cells,
            centre_box: (RADIUS as f64 * CELL + 1.0, RADIUS as f64 * CELL + 1.0, CELL - 2.0, CELL - 2.0),
            swatch: hex.clone(),
            hex,
            rgb: rgb_text(c),
            hsl: hsl_text(c),
            keys: format!("x {}  y {}", self.cx, self.cy),
            sample: (self.cx, self.cy),
            box_pos: self.lens_pos(scale, vw, vh),
            marker: self.css_centre(scale),
        })
    }
}
