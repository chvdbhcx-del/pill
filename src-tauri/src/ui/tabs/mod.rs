//! Registry of the native panel tabs. The island view asks `create(id)` for a tab view when the user opens it.
//! Set A (home, weather, controls, notifs, clipboard, shelf) lives in `a.rs`, set B (note, text, chat, agents, ai) in `b.rs`.
use crate::ui::view::View;
pub mod a;
pub mod b;
pub mod fx;

/// A fresh view for the tab `id` (ids as in pill-ui-core panel::a::tabs), or None while it is not ported yet.
#[allow(dead_code)] // used once the island view lands
pub fn create(id: &str) -> Option<Box<dyn View>> {
    a::create(id).or_else(|| b::create(id))
}
