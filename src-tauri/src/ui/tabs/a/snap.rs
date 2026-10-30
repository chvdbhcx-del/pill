//! Test helpers for the set-A tab views: a backdrop wrapper (the island's body colour behind a transparent tab) and PNG snapshots.
//!
//! `PILL_TABS_DUMP=<dir>` chooses where PNGs go; by default `<target>/tabs-a-snaps/<tab>/<name>.png` (never committed).
use crate::ui::host::testing::{render_view_cx, save_png};
use crate::ui::view::{Cx, DropEvent, Msg, ProximityEvent, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, Response};
use glasskit::testkit::Image;
use glasskit::theme::Theme;
use std::path::PathBuf;

/// Paints `theme.bg` (the island body) behind the tab and forwards everything else.
pub struct OnBg(pub Box<dyn View>);

impl View for OnBg {
    fn size_hint(&self) -> Option<(f32, f32)> {
        self.0.size_hint()
    }
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.0.event(ev, cx)
    }
    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        self.0.on_drop(ev, cx)
    }
    fn on_proximity(&mut self, ev: &ProximityEvent, cx: &mut Cx) -> Response {
        self.0.on_proximity(ev, cx)
    }
    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.0.message(msg, cx)
    }
    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.0.step(dt_ms, cx)
    }
    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx) {
        cv.fill_rect(rect, theme.bg);
        self.0.draw(cv, theme, rect, cx)
    }
    fn on_show(&mut self, cx: &mut Cx) {
        self.0.on_show(cx)
    }
    fn on_hide(&mut self, cx: &mut Cx) {
        self.0.on_hide(cx)
    }
    fn on_dpi(&mut self, scale: f32, cx: &mut Cx) {
        self.0.on_dpi(scale, cx)
    }
}

/// `<dir>/<tab>/<name>.png`
pub fn path(tab: &str, name: &str) -> PathBuf {
    let base = std::env::var_os("PILL_TABS_DUMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("target")).join("tabs-a-snaps"));
    base.join(tab).join(format!("{name}.png"))
}

/// Render `view` (its own `size_hint`) on the theme's body colour at `scale` and write `<tab>/<name>.png`. Returns the pixels for assertions.
pub fn shoot(view: &mut dyn View, cx: &Cx, theme: &Theme, scale: f32, tab: &str, name: &str) -> Image {
    let size = view.size_hint().expect("tab views have a size hint");
    let mut bg = OnBgRef(view);
    let img = render_view_cx(&mut bg, cx, size, scale, theme);
    save_png(&img, path(tab, name));
    img
}

struct OnBgRef<'a>(&'a mut dyn View);
impl View for OnBgRef<'_> {
    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx) {
        cv.fill_rect(rect, theme.bg);
        self.0.draw(cv, theme, rect, cx)
    }
}

/// A `w` x `h` PNG whose upper half is `top` and lower half `bottom` (RGBA). Encoded by hand (stored deflate blocks): no Direct3D device, no COM, so
/// tests that run in parallel never fight over a second WARP device.
pub fn png_bytes(w: u32, h: u32, top: [u8; 4], bottom: [u8; 4]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 4 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0); // filter: none
        let c = if y < h / 2 { top } else { bottom };
        for _ in 0..w {
            raw.extend_from_slice(&c);
        }
    }
    // zlib stream of stored blocks
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        z.push(chunks.peek().is_none() as u8);
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a1, mut a2) = (1u32, 0u32);
    for b in &raw {
        a1 = (a1 + *b as u32) % 65521;
        a2 = (a2 + a1) % 65521;
    }
    z.extend_from_slice(&((a2 << 16) | a1).to_be_bytes());
    let crc = |d: &[u8]| {
        let mut c = 0xffff_ffffu32;
        for b in d {
            c ^= *b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    };
    let chunk = |out: &mut Vec<u8>, t: &[u8; 4], d: &[u8]| {
        out.extend_from_slice(&(d.len() as u32).to_be_bytes());
        let mut td = t.to_vec();
        td.extend_from_slice(d);
        out.extend_from_slice(&td);
        out.extend_from_slice(&crc(&td).to_be_bytes());
    };
    let mut out = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// The same as a `data:image/png;base64,` URL (what `clip_image` and the media thumbnail answer with).
pub fn png_data_url(w: u32, h: u32, top: [u8; 4], bottom: [u8; 4]) -> String {
    use base64::Engine;
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png_bytes(w, h, top, bottom)))
}
