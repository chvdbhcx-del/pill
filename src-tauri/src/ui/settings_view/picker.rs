//! Pet cards and island-style cards of the Character page.
use super::rows::*;
use super::SettingsView;

#[derive(Default)]
pub struct Pickers {}

impl SettingsView {
    pub(super) fn paint_pets(&mut self, _d: &Dc, _fid: u32, _y: f32, _vp: (f32, f32)) -> f32 {
        0.0
    }
    pub(super) fn paint_chars(&mut self, _d: &Dc, _fid: u32, _y: f32, _vp: (f32, f32)) -> f32 {
        0.0
    }
}
