//! The `notifs` tab (stub until its view lands).
use crate::ui::view::View;
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::theme::Theme;

pub struct NotifsView;

impl NotifsView {
    pub fn new() -> NotifsView {
        NotifsView
    }
}

impl View for NotifsView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((super::W, super::tab_h("notifs")? - super::CHROME_H))
    }
    fn draw(&mut self, _cv: &Canvas, _theme: &Theme, _rect: Rect, _cx: &crate::ui::view::Cx) {}
}
