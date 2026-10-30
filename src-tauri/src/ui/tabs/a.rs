//! Native panel tab views of set A: home (media + launcher strip), weather, controls, notifications, clipboard, shelf. Set B (note, text, chat, ...)
//! is `b.rs`. Each tab is a `View` that owns its `pill_ui_core::panel::a` model, feeds it backend events and results, plays its animation cues and
//! draws with glasskit. See `docs/native-tabs-a.md`.
//!
//! # Host contract (what the island view does with a tab)
//! * `create(id)` builds the view on the UI thread. `size_hint()` is the tab's content size in logical px: [`W`] x (`tabH` - [`CHROME_H`]), i.e. the
//!   `#views` box of `pill.css` (680 px panel - 2 x 16 px side padding; 60 px header above, 11 px below).
//! * The tab draws inside the `rect` given to `draw` (any origin) and reads input in the SAME coordinate space as the rect it was last drawn into.
//!   It paints no background: the island's black body (or the light one) is behind it. Use the island's `Theme` (`pill_dark` / `pill_light`).
//! * The island drives the tab with its OWN `Cx`: call `on_show` when the tab becomes the visible one and `on_hide` when it stops being (a hidden or
//!   dropped tab costs nothing: the model clock ticker stops, no frames are requested).
//! * Every `Msg` that is not a [`TabFx`] must be forwarded to the active tab's `message` (backend events it subscribed to, the replies of its
//!   commands, its tick and its image-decode answers all travel through the island's queue); a tab that does not know a message ignores it.
//! * Island-bound requests (toast, collapse, live row, pet reaction, badge ...) are posted as `Msg::Custom(Box<TabFx>)`: see `fx.rs`.
//! * `View::on_drop` is the tab's file drop (home launcher, shelf); an Overlay island forwards drops that land inside the tab area.
#![allow(dead_code)] // wired by the island view; until then only the tests use the tabs

pub mod core;
pub mod draw;
pub mod icons;
mod icons_data;
pub mod play;
#[cfg(test)]
pub mod snap;

mod clipboard;
mod controls;
mod home;
mod notifs;
mod shelf;
mod weather;

use crate::ui::view::View;

pub(crate) use home::is_cover_ready;

/// Width of a tab's content box: the 680 px panel minus 16 px padding each side.
pub const W: f32 = 648.0;
/// Panel chrome around a tab body: `tabH` (the shell's body height, 331 for most tabs) minus this is the content height (60 px header + 11 px bottom).
pub const CHROME_H: f32 = 71.0;

/// `tabH` (CSS px, the pill body height) of the tab: its registered height, or the shell default.
pub fn tab_h(id: &str) -> Option<f32> {
    Some(match id {
        "home" | "clipboard" | "shelf" | "controls" | "notifs" => 331.0,
        "weather" => 417.0,
        _ => return None,
    })
}

/// A fresh view for the set-A tab `id`, `None` for ids of set B.
pub fn create(id: &str) -> Option<Box<dyn View>> {
    Some(match id {
        "home" => Box::new(home::HomeView::new()),
        "weather" => Box::new(weather::WeatherView::new()),
        "controls" => Box::new(controls::ControlsView::new()),
        "notifs" => Box::new(notifs::NotifsView::new()),
        "clipboard" => Box::new(clipboard::ClipboardView::new()),
        "shelf" => Box::new(shelf::ShelfView::new()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_exactly_the_set_a_ids_with_their_content_size() {
        for id in ["home", "weather", "controls", "notifs", "clipboard", "shelf"] {
            let v = create(id).unwrap_or_else(|| panic!("{id}"));
            let (w, h) = v.size_hint().expect("size hint");
            assert_eq!(w, W, "{id}");
            assert_eq!(h, tab_h(id).unwrap() - CHROME_H, "{id}");
        }
        for id in ["note", "text", "chat", "agents", "settings", "nope"] {
            assert!(create(id).is_none(), "{id}");
        }
    }
}
