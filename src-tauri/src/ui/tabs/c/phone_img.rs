//! Pictures that come with a mirrored notification (app icon, sender avatar: base64 PNG up to 96 px): decoded once through WIC (the island's cover
//! decoder), kept as Direct2D bitmaps per key, drawn into a circle. A picture that fails to decode is remembered as "none" (the caller draws a letter).
use crate::ui::island::gpu::{data_url_bytes, decode_cover};
use glasskit::geom::Rect;
use glasskit::Canvas;
use std::collections::HashMap;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC};

const KEEP: usize = 64;

#[derive(Default)]
pub struct Images {
    /// `(key, px)` -> bitmap (None = undecodable)
    map: HashMap<(String, u32), Option<ID2D1Bitmap1>>,
}

impl Images {
    /// Draws `b64` into the circle inscribed in `r`. False when there is nothing to draw (no data, or it did not decode).
    pub fn circle(&mut self, cv: &Canvas, key: &str, b64: Option<&str>, r: Rect) -> bool {
        let Some(b64) = b64 else { return false };
        let px = (r.w * cv.scale).ceil().max(1.0) as u32;
        let k = (key.to_string(), px);
        if !self.map.contains_key(&k) {
            if self.map.len() >= KEEP {
                self.map.clear();
            }
            let bmp = data_url_bytes(b64).and_then(|b| decode_cover(cv, &b, px)).and_then(|p| cv.gfx.from_bgra(px, px, &p).ok());
            self.map.insert(k.clone(), bmp);
        }
        let Some(Some(b)) = self.map.get(&k) else { return false };
        cv.with_clip_rr(r, r.w / 2.0, |cv| unsafe {
            let d = D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() };
            cv.dc.DrawBitmap(b, Some(&d), 1.0, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
        });
        true
    }
}
