//! Icons of the native bar: the web UI's line glyphs (`glyphs.rs`, drawn with glasskit's `Svg`, one parsed copy per (name, stroke width)) and the
//! shell icons of app / file / folder rows (`cmd_icon` answers a PNG data URL; decoded here, uploaded as a Direct2D bitmap on first draw and
//! kept per URL; the bitmaps are dropped when the graphics device changes, the decoded pixels stay).
use super::glyphs::glyph;
use base64::Engine;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::gfx::rf;
use glasskit::svgpath::{GeomCache, Svg, SvgStyle};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::rc::Rc;
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC};

/// The vector glyphs. Not `Send` (Direct2D geometries); lives in the view.
#[derive(Default)]
pub struct IconSet {
    svgs: RefCell<HashMap<(String, u32), Rc<Svg>>>,
    cache: GeomCache,
}

impl IconSet {
    pub fn new() -> IconSet {
        IconSet::default()
    }

    fn svg(&self, key: &str, sw: f32, make: impl FnOnce() -> Option<Svg>) -> Option<Rc<Svg>> {
        let k = (key.to_string(), sw.to_bits());
        if let Some(s) = self.svgs.borrow().get(&k) {
            return Some(s.clone());
        }
        let s = Rc::new(make()?);
        self.svgs.borrow_mut().insert(k, s.clone());
        Some(s)
    }

    /// A web glyph by name (`icons.js`), stroke width `sw` in 24-unit space, fitted into `dest`.
    pub fn draw(&self, cv: &Canvas, name: &str, sw: f32, dest: Rect, tint: Color) {
        if let Some(s) = self.svg(name, sw, || glyph(name).map(|inner| Svg::icon(inner, sw))) {
            s.draw(cv, dest, tint, &self.cache);
        }
    }

    /// Inline markup in a 16 x 16 box (the uninstall panel's own marks: tick, chevron, shield, spinner arc...).
    pub fn draw16(&self, cv: &Canvas, key: &str, inner: &str, sw: f32, dest: Rect, tint: Color) {
        let k = format!("16:{key}");
        if let Some(s) = self.svg(&k, sw, || Some(Svg::from_fragment(inner, (0.0, 0.0, 16.0, 16.0), SvgStyle::line_icon(sw)))) {
            s.draw(cv, dest, tint, &self.cache);
        }
    }
}

/// PNG bytes (any 8/16-bit colour type) -> (w, h, straight RGBA).
pub fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.width == 0 || info.height == 0 || info.width > 1024 || info.height > 1024 {
        return None;
    }
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some((info.width, info.height, rgba))
}

/// `data:image/png;base64,....` (what `cmd_icon` answers) -> premultiplied BGRA ready for Direct2D.
pub fn decode_data_url(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let b64 = url.strip_prefix("data:image/png;base64,")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let (w, h, mut px) = decode_png(&bytes)?;
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        let (r, g, b) = (p[0], p[1], p[2]);
        p[0] = ((b as u32 * a + 127) / 255) as u8;
        p[1] = ((g as u32 * a + 127) / 255) as u8;
        p[2] = ((r as u32 * a + 127) / 255) as u8;
    }
    Some((w, h, px))
}

struct Entry {
    w: u32,
    h: u32,
    bgra: Option<Rc<Vec<u8>>>,
    bmp: Option<ID2D1Bitmap1>,
}

/// Shell icons by data URL. Bounded like the model's own cache (300 ids): the oldest URL goes first.
#[derive(Default)]
pub struct ShellIcons {
    map: RefCell<HashMap<u64, Entry>>,
    order: RefCell<Vec<u64>>,
    dev: Cell<(usize, usize)>,
}

const SHELL_MAX: usize = 320;

fn key_of(url: &str) -> u64 {
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    h.finish()
}

impl ShellIcons {
    pub fn new() -> ShellIcons {
        ShellIcons::default()
    }

    /// Number of decoded icons kept.
    pub fn len(&self) -> usize {
        self.map.borrow().len()
    }

    /// Draws the icon of `url` stretched into `dest`; false when it cannot be drawn (empty / not a PNG data URL): the caller draws the glyph.
    pub fn draw(&self, cv: &Canvas, url: &str, dest: Rect, alpha: f32) -> bool {
        if url.is_empty() {
            return false;
        }
        let dev = (cv.gfx as *const _ as usize, cv.dc.as_raw() as usize);
        if self.dev.get() != dev {
            self.dev.set(dev);
            for e in self.map.borrow_mut().values_mut() {
                e.bmp = None; // another device: the bitmaps are not valid any more
            }
        }
        let k = key_of(url);
        let mut map = self.map.borrow_mut();
        if !map.contains_key(&k) {
            let e = match decode_data_url(url) {
                Some((w, h, px)) => Entry { w, h, bgra: Some(Rc::new(px)), bmp: None },
                None => Entry { w: 0, h: 0, bgra: None, bmp: None },
            };
            let mut order = self.order.borrow_mut();
            order.push(k);
            if order.len() > SHELL_MAX {
                let old = order.remove(0);
                map.remove(&old);
            }
            map.insert(k, e);
        }
        let Some(e) = map.get_mut(&k) else { return false };
        let Some(px) = e.bgra.clone() else { return false };
        if e.bmp.is_none() {
            e.bmp = cv.gfx.from_bgra(e.w, e.h, &px).ok();
        }
        let Some(b) = e.bmp.as_ref() else { return false };
        let r = rf(dest.x, dest.y, dest.right(), dest.bottom());
        unsafe { cv.dc.DrawBitmap(b, Some(&r), alpha, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None) };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_data_url(w: u32, h: u32, rgba: &[u8]) -> String {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(Cursor::new(&mut out), w, h);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header().unwrap().write_image_data(rgba).unwrap();
        }
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out))
    }

    #[test]
    fn data_urls_decode_to_premultiplied_bgra() {
        // 2 x 1: opaque red, half-transparent green
        let url = png_data_url(2, 1, &[255, 0, 0, 255, 0, 200, 0, 128]);
        let (w, h, px) = decode_data_url(&url).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(&px[0..4], &[0, 0, 255, 255], "red is BGRA 0,0,255,255");
        assert_eq!(px[3 + 4], 128);
        assert_eq!(px[1 + 4], ((200u32 * 128 + 127) / 255) as u8, "green is premultiplied");
    }

    #[test]
    fn junk_does_not_decode() {
        assert!(decode_data_url("").is_none());
        assert!(decode_data_url("data:image/png;base64,@@@").is_none());
        assert!(decode_data_url("data:image/png;base64,AAAA").is_none());
        assert!(decode_data_url("data:text/plain;base64,AAAA").is_none());
    }

    #[test]
    fn every_glyph_the_model_names_exists() {
        for n in ["apps", "trash", "file", "folder", "calc", "convert", "globe", "power", "terminal", "sparkles", "copy", "refresh", "clipboard", "note", "shelf", "bell", "home", "settings", "bolt", "search", "mic", "enter", "shield", "pin", "folderOpen", "lock", "moon", "logout", "check", "x"] {
            assert!(glyph(n).is_some(), "{n}");
        }
        assert!(glyph("nope").is_none());
    }
}
