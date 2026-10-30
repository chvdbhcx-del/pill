//! The AI quick-action overlay (placeholder, being written).
use crate::ui::view::{Cx, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct AiActionsScreen;

impl AiActionsScreen {
    pub fn new() -> AiActionsScreen {
        AiActionsScreen
    }
}

impl View for AiActionsScreen {
    fn draw(&mut self, _cv: &Canvas, _th: &Theme, _rect: Rect, _cx: &Cx) {}
}
