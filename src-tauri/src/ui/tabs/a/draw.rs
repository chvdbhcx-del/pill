//! Drawing helpers the stock `glasskit::Canvas` lacks: gradient fills, and bitmaps (covers, thumbnails) with a size-capped cache.
//!
//! * Gradients: [`linear`], [`radial`] (straight RGBA stops, CSS-like).
//! * Bitmaps: [`decode_bytes`] / [`decode_file`] turn encoded images (PNG, JPEG, ICO, BMP, GIF, WebP when the codec exists) into premultiplied BGRA
//!   on ANY thread; [`ImageCache`] keeps the decoded pixels under a byte cap (LRU) and makes the Direct2D bitmap lazily on the UI thread
//!   (re-made when the device changed). [`ImageCache::request`] decodes on the worker pool and answers with a `Msg::Custom(ImageReady)`.
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::{Pt, Rect};
use glasskit::gfx::{col, pt, rf};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

// ------------------------------------------------------------------------------------------------------------------------------ gradients

fn stops_of(cv: &Canvas, stops: &[(f32, Color)]) -> Option<ID2D1GradientStopCollection1> {
    let st: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|&(p, c)| D2D1_GRADIENT_STOP { position: p, color: col(c.r, c.g, c.b, c.a) }).collect();
    unsafe { cv.dc.CreateGradientStopCollection(&st, D2D1_COLOR_SPACE_SRGB, D2D1_COLOR_SPACE_SRGB, D2D1_BUFFER_PRECISION_8BPC_UNORM, D2D1_EXTEND_MODE_CLAMP, D2D1_COLOR_INTERPOLATION_MODE_STRAIGHT).ok() }
}

/// Fill the rounded rectangle `r` (radius `rad`) with a linear gradient from `a` to `b` (logical px) through `stops` (position 0..1, straight RGBA).
pub fn linear(cv: &Canvas, r: Rect, rad: f32, a: Pt, b: Pt, stops: &[(f32, Color)]) {
    let Some(coll) = stops_of(cv, stops) else { return };
    unsafe {
        let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES { startPoint: pt(a.0, a.1), endPoint: pt(b.0, b.1) };
        let base: &ID2D1GradientStopCollection = &coll;
        let Ok(br) = cv.dc.CreateLinearGradientBrush(&props, None, base) else { return };
        let rad = rad.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        if rad > 0.5 {
            cv.dc.FillRoundedRectangle(&glasskit::gfx::rr(r.x, r.y, r.w, r.h, rad), &br);
        } else {
            cv.dc.FillRectangle(&rf(r.x, r.y, r.right(), r.bottom()), &br);
        }
    }
}

/// CSS `linear-gradient(<deg>deg, ...)` over `r`: 0deg points up, 90deg right, 180deg down (the gradient line of the CSS spec: it reaches both corners).
pub fn linear_deg(cv: &Canvas, r: Rect, rad: f32, deg: f32, stops: &[(f32, Color)]) {
    let t = deg.to_radians();
    let (sx, sy) = (t.sin(), -t.cos());
    let half = (r.w * sx.abs() + r.h * sy.abs()) / 2.0;
    let c = r.center();
    linear(cv, r, rad, (c.0 - sx * half, c.1 - sy * half), (c.0 + sx * half, c.1 + sy * half), stops);
}

/// Fill `r` (radius `rad`) with a radial gradient centred at `c`, radii `rx` / `ry`.
pub fn radial(cv: &Canvas, r: Rect, rad: f32, c: Pt, rx: f32, ry: f32, stops: &[(f32, Color)]) {
    let Some(coll) = stops_of(cv, stops) else { return };
    unsafe {
        let props = D2D1_RADIAL_GRADIENT_BRUSH_PROPERTIES { center: pt(c.0, c.1), gradientOriginOffset: pt(0.0, 0.0), radiusX: rx, radiusY: ry };
        let base: &ID2D1GradientStopCollection = &coll;
        let Ok(br) = cv.dc.CreateRadialGradientBrush(&props, None, base) else { return };
        let rad = rad.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
        if rad > 0.5 {
            cv.dc.FillRoundedRectangle(&glasskit::gfx::rr(r.x, r.y, r.w, r.h, rad), &br);
        } else {
            cv.dc.FillRectangle(&rf(r.x, r.y, r.right(), r.bottom()), &br);
        }
    }
}

// ------------------------------------------------------------------------------------------------------------------------------ decoding

/// Decoded pixels: premultiplied BGRA, `w * h * 4` bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>,
}

impl Decoded {
    pub fn bytes(&self) -> usize {
        self.bgra.len()
    }
}

pub(crate) fn com_init() {
    // MTA on pool threads; on the UI thread (STA) this returns RPC_E_CHANGED_MODE, which is fine: COM is up either way
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn wic() -> Option<IWICImagingFactory> {
    com_init();
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok() }
}

fn frame_to_bgra(f: &IWICImagingFactory, frame: &IWICBitmapFrameDecode, max_side: u32) -> Option<Decoded> {
    unsafe {
        let (mut iw, mut ih) = (0u32, 0u32);
        frame.GetSize(&mut iw, &mut ih).ok()?;
        if iw == 0 || ih == 0 {
            return None;
        }
        let k = (max_side as f32 / iw.max(ih) as f32).min(1.0);
        let (w, h) = (((iw as f32 * k).round() as u32).max(1), ((ih as f32 * k).round() as u32).max(1));
        let conv = f.CreateFormatConverter().ok()?;
        if k < 1.0 {
            let sc = f.CreateBitmapScaler().ok()?;
            sc.Initialize(frame, w, h, WICBitmapInterpolationModeFant).ok()?;
            conv.Initialize(&sc, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        } else {
            conv.Initialize(frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        }
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut bgra).ok()?;
        Some(Decoded { w, h, bgra })
    }
}

/// Decode an encoded image held in memory, shrinking it so its longer side is at most `max_side` (never enlarging). Thread-safe.
pub fn decode_bytes(bytes: &[u8], max_side: u32) -> Option<Decoded> {
    if bytes.len() < 8 {
        return None;
    }
    let f = wic()?;
    unsafe {
        let stream = f.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let dec = f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = dec.GetFrame(0).ok()?;
        frame_to_bgra(&f, &frame, max_side)
    }
}

/// Decode an image file (see [`decode_bytes`]). Thread-safe.
pub fn decode_file(path: &Path, max_side: u32) -> Option<Decoded> {
    let f = wic()?;
    unsafe {
        let dec = f.CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = dec.GetFrame(0).ok()?;
        frame_to_bgra(&f, &frame, max_side)
    }
}

/// Decode a `data:<mime>;base64,<payload>` URL (the form the backend hands covers and icons out in). Thread-safe.
pub fn decode_data_url(url: &str, max_side: u32) -> Option<Decoded> {
    use base64::Engine;
    let (_, payload) = url.strip_prefix("data:")?.split_once(";base64,")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(payload.trim()).ok()?;
    decode_bytes(&bytes, max_side)
}

/// A small solid-colour BGRA test image (tests build covers without files).
pub fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Decoded {
    let a = rgba[3] as u32;
    let px = [(rgba[2] as u32 * a / 255) as u8, (rgba[1] as u32 * a / 255) as u8, (rgba[0] as u32 * a / 255) as u8, rgba[3]];
    Decoded { w, h, bgra: px.iter().cycle().take((w * h * 4) as usize).copied().collect() }
}

/// Where an image comes from.
#[derive(Clone, Debug)]
pub enum Source {
    Bytes(Vec<u8>),
    File(PathBuf),
    /// `data:...;base64,...`, decoded on the worker.
    DataUrl(String),
}

/// The answer to [`ImageCache::request`], posted as `Msg::Custom(Box<ImageReady>)` to the view. Feed it to [`ImageCache::ready`].
pub struct ImageReady {
    pub key: String,
    pub img: Option<Decoded>,
}

// ------------------------------------------------------------------------------------------------------------------------------ the cache

/// Default cap of decoded pixels per view (covers and thumbnails are small; 24 MB holds ~100 thumbnails of 256 px).
pub const DEFAULT_CAP: usize = 24 * 1024 * 1024;

struct Entry {
    img: Decoded,
    /// (device context identity, bitmap); re-made when the device context changed (device lost)
    bmp: RefCell<Option<(usize, ID2D1Bitmap1)>>,
    used: Cell<u64>,
}

/// How an image fills its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// `object-fit: cover`: scale to fill, crop the overflow around the centre.
    Cover,
    /// `object-fit: contain`: scale to fit inside, letterboxed.
    Contain,
    /// Stretch to the box.
    Fill,
}

/// Decoded images by key, LRU under a byte cap. UI thread only.
pub struct ImageCache {
    cap: usize,
    map: HashMap<String, Entry>,
    pending: HashSet<String>,
    bytes: usize,
    clock: Cell<u64>,
    /// longest side kept when decoding for this cache
    pub max_side: u32,
}

impl ImageCache {
    pub fn new(cap_bytes: usize, max_side: u32) -> ImageCache {
        ImageCache { cap: cap_bytes, map: HashMap::new(), pending: HashSet::new(), bytes: 0, clock: Cell::new(0), max_side }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn is_pending(&self, key: &str) -> bool {
        self.pending.contains(key)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Bytes of decoded pixels held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn size(&self, key: &str) -> Option<(u32, u32)> {
        self.map.get(key).map(|e| (e.img.w, e.img.h))
    }

    pub fn remove(&mut self, key: &str) {
        if let Some(e) = self.map.remove(key) {
            self.bytes -= e.img.bytes();
        }
        self.pending.remove(key);
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.pending.clear();
        self.bytes = 0;
    }

    /// Store decoded pixels under `key` (replaces), evicting the least recently drawn images while over the cap. An image bigger than the
    /// whole cap is not stored.
    pub fn insert(&mut self, key: &str, img: Decoded) {
        self.pending.remove(key);
        self.remove(key);
        if img.bytes() > self.cap {
            return;
        }
        self.bytes += img.bytes();
        self.clock.set(self.clock.get() + 1);
        self.map.insert(key.to_string(), Entry { img, bmp: RefCell::new(None), used: Cell::new(self.clock.get()) });
        while self.bytes > self.cap {
            let Some(old) = self.map.iter().filter(|(k, _)| k.as_str() != key).min_by_key(|(_, e)| e.used.get()).map(|(k, _)| k.clone()) else { break };
            self.remove(&old);
        }
    }

    /// Decode on the UI thread (small, already-in-memory images: a media cover). `false` when the bytes are not an image.
    pub fn insert_bytes(&mut self, key: &str, bytes: &[u8]) -> bool {
        match decode_bytes(bytes, self.max_side) {
            Some(d) => {
                self.insert(key, d);
                true
            }
            None => false,
        }
    }

    /// Decode `src` on the worker pool unless `key` is cached or already requested; the answer arrives as `Msg::Custom(ImageReady)`.
    pub fn request(&mut self, waker: &crate::ui::view::Waker, key: &str, src: Source) {
        if self.map.contains_key(key) || !self.pending.insert(key.to_string()) {
            return;
        }
        let (w, key, max) = (waker.clone(), key.to_string(), self.max_side);
        let job = move || {
            let img = match &src {
                Source::Bytes(b) => decode_bytes(b, max),
                Source::File(p) => decode_file(p, max),
                Source::DataUrl(u) => decode_data_url(u, max),
            };
            w.post_custom(ImageReady { key, img });
        };
        if !glasskit::jobs::pool::global().submit(Box::new(job)) {
            eprintln!("tabs: worker pool full, an image decode was dropped");
        }
    }

    /// Take a worker's answer. Returns the key when an image was stored (the view repaints), `None` for a failed decode.
    pub fn ready(&mut self, r: ImageReady) -> Option<String> {
        self.pending.remove(&r.key);
        let ImageReady { key, img } = r;
        img.map(|d| {
            self.insert(&key, d);
            key
        })
    }

    fn bitmap(&self, cv: &Canvas, e: &Entry) -> Option<ID2D1Bitmap1> {
        let id = cv.dc.as_raw() as usize;
        let mut slot = e.bmp.borrow_mut();
        if let Some((d, b)) = slot.as_ref() {
            if *d == id {
                return Some(b.clone());
            }
        }
        let b = cv.gfx.from_bgra(e.img.w, e.img.h, &e.img.bgra).ok()?;
        *slot = Some((id, b.clone()));
        Some(b)
    }

    /// Draw `key` into `dest` with rounded corners `rad` and opacity `alpha`. False when the image is not cached (draw a placeholder).
    pub fn draw(&self, cv: &Canvas, key: &str, dest: Rect, rad: f32, alpha: f32, fit: Fit) -> bool {
        let Some(e) = self.map.get(key) else { return false };
        self.clock.set(self.clock.get() + 1);
        e.used.set(self.clock.get());
        let Some(bmp) = self.bitmap(cv, e) else { return false };
        let (iw, ih) = (e.img.w as f32, e.img.h as f32);
        let (src, dst) = match fit {
            Fit::Fill => (Rect::new(0.0, 0.0, iw, ih), dest),
            Fit::Cover => {
                let k = (dest.w / iw).max(dest.h / ih);
                let (sw, sh) = (dest.w / k, dest.h / k);
                (Rect::new((iw - sw) / 2.0, (ih - sh) / 2.0, sw, sh), dest)
            }
            Fit::Contain => {
                let k = (dest.w / iw).min(dest.h / ih);
                (Rect::new(0.0, 0.0, iw, ih), dest.centered(iw * k, ih * k))
            }
        };
        let paint = |cv: &Canvas| unsafe {
            cv.dc.DrawBitmap(&bmp, Some(&rf(dst.x, dst.y, dst.right(), dst.bottom())), alpha.clamp(0.0, 1.0), D2D1_INTERPOLATION_MODE_LINEAR, Some(&rf(src.x, src.y, src.right(), src.bottom())), None);
        };
        if rad > 0.5 {
            cv.with_clip_rr(dst, rad, paint);
        } else {
            paint(cv);
        }
        true
    }
}

impl Default for ImageCache {
    fn default() -> ImageCache {
        ImageCache::new(DEFAULT_CAP, 256)
    }
}

/// Cheap shared handle used where several draw closures need the same cache.
pub type SharedImages = Rc<RefCell<ImageCache>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_evicts_the_least_recently_used_under_the_cap() {
        let mut c = ImageCache::new(1000, 64);
        c.insert("a", solid(10, 10, [255, 0, 0, 255])); // 400 bytes
        c.insert("b", solid(10, 10, [0, 255, 0, 255]));
        assert_eq!((c.len(), c.bytes()), (2, 800));
        c.insert("c", solid(10, 10, [0, 0, 255, 255])); // over the cap: "a" (oldest) goes
        assert!(!c.contains("a") && c.contains("b") && c.contains("c"));
        assert_eq!(c.bytes(), 800);
        c.insert("huge", solid(30, 30, [1, 2, 3, 255])); // 3600 bytes > cap: not stored
        assert!(!c.contains("huge"));
        c.remove("b");
        assert_eq!(c.bytes(), 400);
    }

    #[test]
    fn png_bytes_decode_to_premultiplied_bgra_and_shrink() {
        // a 2x2 PNG written by the test helper (opaque blue)
        let bytes = crate::ui::tabs::a::snap::png_bytes(2, 2, [0, 0, 255, 255], [0, 0, 255, 255]);
        let d = decode_bytes(&bytes, 64).expect("decodes");
        assert_eq!((d.w, d.h), (2, 2));
        assert_eq!(&d.bgra[..4], &[255, 0, 0, 255]);
        let dir = std::env::temp_dir().join(format!("pill-tabs-a-draw-test-{}.png", std::process::id()));
        std::fs::write(&dir, crate::ui::tabs::a::snap::png_bytes(8, 4, [10, 20, 30, 255], [10, 20, 30, 255])).unwrap();
        let d2 = decode_file(&dir, 4).expect("file decodes");
        assert_eq!((d2.w, d2.h), (4, 2), "longer side shrunk to 4, aspect kept");
        assert!(decode_bytes(b"not an image at all", 64).is_none());
        let _ = std::fs::remove_file(&dir);
    }
}
