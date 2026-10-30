//! The Chat tab view (placeholder, being written).
use crate::ui::view::{Cx, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct ChatScreen;

impl ChatScreen {
    pub fn new() -> ChatScreen {
        ChatScreen
    }
}

impl View for ChatScreen {
    fn draw(&mut self, _cv: &Canvas, _th: &Theme, _rect: Rect, _cx: &Cx) {}
}
