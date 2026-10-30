//! The Note tab view: note chips, the rich editor, the toolbar, and the timer / reminders card (placeholder, being written).
use crate::ui::view::{Cx, View};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct NoteScreen;

impl NoteScreen {
    pub fn new() -> NoteScreen {
        NoteScreen
    }
}

impl View for NoteScreen {
    fn draw(&mut self, _cv: &Canvas, _th: &Theme, _rect: Rect, _cx: &Cx) {}
}
