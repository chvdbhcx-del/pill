//! The `shelf` tab (stub until its view lands).
use crate::ui::view::View;
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct ShelfView;

impl ShelfView {
    pub fn new() -> ShelfView {
        ShelfView
    }
}

impl View for ShelfView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((super::W, super::tab_h("shelf")? - super::CHROME_H))
    }
    fn draw(&mut self, _cv: &Canvas, _theme: &Theme, _rect: Rect, _cx: &crate::ui::view::Cx) {}
}
