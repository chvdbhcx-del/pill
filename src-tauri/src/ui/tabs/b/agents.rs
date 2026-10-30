//! The Agents tab view (placeholder, being written).
use crate::ui::view::{Cx, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct AgentsScreen;

impl AgentsScreen {
    pub fn new() -> AgentsScreen {
        AgentsScreen
    }
}

impl View for AgentsScreen {
    fn draw(&mut self, _cv: &Canvas, _th: &Theme, _rect: Rect, _cx: &Cx) {}
}
