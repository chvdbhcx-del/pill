//! The Text tab view: 19 text tools, QR, AI mode (placeholder, being written).
use crate::ui::view::{Cx, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct TextScreen;

impl TextScreen {
    pub fn new() -> TextScreen {
        TextScreen
    }
}

impl View for TextScreen {
    fn draw(&mut self, _cv: &Canvas, _th: &Theme, _rect: Rect, _cx: &Cx) {}
}
