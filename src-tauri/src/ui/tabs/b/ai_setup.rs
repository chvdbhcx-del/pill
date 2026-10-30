//! The AI setup wizard overlay (placeholder, being written).
use crate::ui::view::{Cx, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct AiSetupScreen;

impl AiSetupScreen {
    pub fn new() -> AiSetupScreen {
        AiSetupScreen
    }
}

impl View for AiSetupScreen {
    fn draw(&mut self, _cv: &Canvas, _th: &Theme, _rect: Rect, _cx: &Cx) {}
}
