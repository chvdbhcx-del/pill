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
