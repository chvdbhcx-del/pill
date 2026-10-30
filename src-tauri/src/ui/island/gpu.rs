//! Per-window drawing resources of the island: the line-icon and forecast-icon SVGs (parsed once, geometry cached), cover pictures decoded
//! through WIC (cached by their id, `object-fit: cover`, cross-fade in), the value tweens of bars / rings, and small colour helpers.
use super::icons_data::ICONS;
use super::wx_data::WX_ICONS;
use base64::Engine as _;
use glasskit::geom::Rect;
use glasskit::svgpath::{GeomCache, Svg, SvgStyle};
use glasskit::theme::Theme;
use glasskit::{Canvas, Color};
use pill_ui_core::live::model::Rgba;
use std::collections::HashMap;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap1, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::STREAM_SEEK_SET;

pub fn c8(c: Rgba) -> Color {
    Color::rgba(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0, c.3 as f32 / 255.0)
}

/// shell colour (straight, alpha 0..1) -> glasskit colour
pub fn cs(c: pill_ui_core::shell::color::Rgba) -> Color {
    Color::rgba(c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, c.a)
}

/// Icons the web drew inside individual live-*.js files (not in icons.js): same 24 grid, same stroke style.
pub const EXTRA_ICONS: &[(&str, &str)] = &[
    ("mail", r#"<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="m3.5 7.5 8.5 6 8.5-6"/>"#),
    ("usb", r#"<path d="M12 20V5M9 8l3-3 3 3"/><circle cx="12" cy="20" r="1.5"/><path d="M12 13l-4-2v-2M12 11l4-2v-1"/>"#),
    ("calendar", r#"<rect x="3.5" y="5" width="17" height="15.5" rx="3"/><path d="M3.5 10h17M8 3v4M16 3v4"/>"#),
    ("keyboard", r#"<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6.5 10h.01M10 10h.01M14 10h.01M17.5 10h.01M7 14h10"/>"#),
    ("timer", r#"<circle cx="12" cy="13" r="8"/><path d="M12 9v4l2.5 2M9.5 2.5h5"/>"#),
    ("warn", r#"<path d="M12 4 2.8 19.5h18.4z"/><path d="M12 10v4.5M12 17.5h.01"/>"#),
];

struct Cover {
    bmp: Option<ID2D1Bitmap1>,
    born: f64,
    px: u32,
}

#[derive(Default)]
pub struct Gpu {
    svgs: HashMap<(String, u32), Svg>,
    wx: HashMap<String, Svg>,
    gc: GeomCache,
    covers: HashMap<u64, Cover>,
    glides: HashMap<String, (f32, f32, f64)>,
}

/// `data:image/...;base64,....` (or a bare base64 string) -> bytes
pub fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
    let b64 = url.split_once(";base64,").map(|x| x.1).or_else(|| url.strip_prefix("base64,")).unwrap_or(url);
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()
}

/// Decodes any WIC-readable picture to premultiplied BGRA, scaled (Fant) to cover `px` x `px` and cropped around the centre.
pub fn decode_cover(cv: &Canvas, bytes: &[u8], px: u32) -> Option<Vec<u8>> {
    unsafe {
        let f = &cv.gfx.wic;
        let st = f.CreateStream().ok()?;
        st.InitializeFromMemory(bytes).ok()?;
        let _ = STREAM_SEEK_SET;
        let dec = f.CreateDecoderFromStream(&st, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let fr = dec.GetFrame(0).ok()?;
        let (mut iw, mut ih) = (0u32, 0u32);
        fr.GetSize(&mut iw, &mut ih).ok()?;
        if iw == 0 || ih == 0 {
            return None;
        }
        let k = (px as f32 / iw as f32).max(px as f32 / ih as f32);
        let (sw, sh) = (((iw as f32 * k).ceil() as u32).max(px), ((ih as f32 * k).ceil() as u32).max(px));
        let sc = f.CreateBitmapScaler().ok()?;
        sc.Initialize(&fr, sw, sh, WICBitmapInterpolationModeFant).ok()?;
        let cl = f.CreateBitmapClipper().ok()?;
        cl.Initialize(&sc, &WICRect { X: ((sw - px) / 2) as i32, Y: ((sh - px) / 2) as i32, Width: px as i32, Height: px as i32 }).ok()?;
        let cvt = f.CreateFormatConverter().ok()?;
        cvt.Initialize(&cl, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let mut buf = vec![0u8; (px * px * 4) as usize];
        cvt.CopyPixels(std::ptr::null(), px * 4, &mut buf).ok()?;
        Some(buf)
    }
}

impl Gpu {
    pub fn new() -> Gpu {
        Gpu::default()
    }

    /// A line icon of the web set (`icons.js`), stroke `sw`, tinted `color`, fitted into `r`.
    pub fn icon(&mut self, cv: &Canvas, name: &str, r: Rect, color: Color, sw: f32) {
        let key = (name.to_string(), (sw * 10.0) as u32);
        if !self.svgs.contains_key(&key) {
            let inner = ICONS.iter().chain(EXTRA_ICONS.iter()).find(|(n, _)| *n == name).map(|(_, m)| *m).unwrap_or("");
            self.svgs.insert(key.clone(), Svg::icon(inner, sw));
        }
        if let Some(s) = self.svgs.get(&key) {
            s.draw(cv, r, color, &self.gc);
        }
    }

    pub fn has_icon(name: &str) -> bool {
        ICONS.iter().chain(EXTRA_ICONS.iter()).any(|(n, _)| *n == name)
    }

    /// The coloured forecast icon `id` (one of `clock::WX_IDS`) in a square `r`.
    pub fn wx_icon(&mut self, cv: &Canvas, id: &str, r: Rect) {
        if !self.wx.contains_key(id) {
            let inner = WX_ICONS.iter().find(|(n, _)| *n == id).or_else(|| WX_ICONS.iter().find(|(n, _)| *n == "cloud")).map(|(_, m)| *m).unwrap_or("");
            self.wx.insert(id.to_string(), Svg::from_fragment(inner, (0.0, 0.0, 48.0, 48.0), SvgStyle::default()));
        }
        if let Some(s) = self.wx.get(id) {
            s.draw(cv, r, Color::rgb8(255, 255, 255), &self.gc);
        }
    }

    /// Album cover (or the placeholder), 7 px corners; a new picture fades in over 250 ms.
    pub fn cover(&mut self, cv: &Canvas, th: &Theme, id: u64, url: Option<&str>, r: Rect, t: f64) {
        let px = (r.w * cv.scale).ceil().max(1.0) as u32;
        let stale = self.covers.get(&id).is_none_or(|c| c.px != px);
        if stale {
            let bmp = url
                .and_then(data_url_bytes)
                .and_then(|b| decode_cover(cv, &b, px))
                .and_then(|p| cv.gfx.from_bgra(px, px, &p).ok());
            self.covers.insert(id, Cover { bmp, born: t, px });
            if self.covers.len() > 8 {
                let keep = id;
                self.covers.retain(|k, _| *k == keep);
            }
        }
        let c = &self.covers[&id];
        cv.fill_rr(r, 7.0, th.raised2);
        let Some(b) = &c.bmp else {
            self.icon_placeholder(cv, th, r);
            return;
        };
        let a = (((t - c.born) / 250.0) as f32).clamp(0.0, 1.0);
        cv.with_clip_rr(r, 7.0, |cv| unsafe {
            let d = D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() };
            cv.dc.DrawBitmap(b, Some(&d), a, D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC, None, None);
        });
    }

    fn icon_placeholder(&mut self, cv: &Canvas, th: &Theme, r: Rect) {
        self.icon(cv, "note2", r.inset(r.w * 0.25), th.dim, 1.7);
    }

    /// Value tween toward `target` over about `ms` (exponential approach; `reduced` jumps).
    pub fn glide(&mut self, key: &str, target: f32, ms: f64, t: f64, reduced: bool) -> f32 {
        let e = self.glides.entry(key.to_string()).or_insert((target, target, t));
        if reduced || ms <= 0.0 {
            *e = (target, target, t);
            return target;
        }
        let dt = (t - e.2).clamp(0.0, 100.0);
        e.2 = t;
        e.1 = target;
        e.0 += (target - e.0) * (1.0 - (-dt / (ms / 3.0)).exp()) as f32;
        if (e.0 - target).abs() < 0.0005 {
            e.0 = target;
        }
        e.0
    }

    /// True while any tween has not reached its target (the view keeps asking for frames).
    pub fn gliding(&self) -> bool {
        self.glides.values().any(|g| (g.0 - g.1).abs() > 0.0005)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_urls_decode_and_garbage_does_not() {
        assert_eq!(data_url_bytes("data:image/png;base64,AAEC").unwrap(), vec![0, 1, 2]);
        assert_eq!(data_url_bytes("AAEC").unwrap(), vec![0, 1, 2]);
        assert!(data_url_bytes("data:image/png;base64,@@@").is_none());
    }

    #[test]
    fn glide_approaches_and_reduced_motion_jumps() {
        let mut g = Gpu::new();
        assert_eq!(g.glide("a", 0.0, 300.0, 0.0, false), 0.0);
        let mid = g.glide("a", 1.0, 300.0, 50.0, false);
        assert!(mid > 0.0 && mid < 1.0);
        assert!(g.gliding());
        for i in 0..100 {
            g.glide("a", 1.0, 300.0, 100.0 + i as f64 * 16.0, false);
        }
        assert_eq!(g.glide("a", 1.0, 300.0, 5000.0, false), 1.0);
        assert!(!g.gliding());
        assert_eq!(g.glide("b", 0.2, 300.0, 0.0, true), 0.2);
        assert_eq!(g.glide("b", 0.9, 300.0, 16.0, true), 0.9);
    }

    #[test]
    fn every_icon_the_live_layer_names_exists() {
        use pill_ui_core::live::model::IconName::*;
        for n in [Area, Bell, Battery, Bolt, Calendar, Check, Clipboard, Copy, Download, File, Folder, Keyboard, Mail, Mic, Monitor, Mute, Next, Note2, Pause, Play, Plus, Prev, Qr, ScanText, Shelf, Sparkles, Terminal, Timer, Trash, Usb, Video, Volume, Warn, X] {
            assert!(Gpu::has_icon(n.as_str()), "{}", n.as_str());
        }
        for must in ["home", "settings", "pin", "wave", "bell", "x", "check", "play", "pause", "prev", "next", "note2", "copy", "download", "volume", "mute", "mic", "sparkles", "video", "monitor", "area", "folder", "file", "battery", "bolt", "terminal", "trash", "plus", "qr", "scantext", "shelf", "clipboard"] {
            assert!(Gpu::has_icon(must), "{must}");
        }
    }
}
