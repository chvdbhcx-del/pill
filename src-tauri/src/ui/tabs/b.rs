//! Native views of panel set B: the tabs note (with the timer ring and reminders), text, chat, agents, and the overlays ai_actions and ai_setup.
//! `create(id)` is what `ui::tabs::create` calls for a tab id; `create_overlay(name)` builds the two overlay views. See docs/native-tabs-b.md.
#![allow(dead_code)] // wired by the island view (lead merge)

mod agents;
mod ai_actions;
mod ai_setup;
mod chat;
mod chat_pop;
mod editor;
mod icons;
pub mod kit;
mod note;
mod note_doc;
mod text;
mod timer_card;

pub use crate::ui::tabs::fx::take as take_fx;
pub(crate) use chat::is_picked;

use crate::ui::view::View;

/// A fresh view for a tab of set B (`note`, `text`, `chat`, `agents`); `None` for ids of the other set.
pub fn create(id: &str) -> Option<Box<dyn View>> {
    Some(match id {
        "note" => Box::new(note::NoteScreen::new()),
        "text" => Box::new(text::TextScreen::new()),
        "chat" => Box::new(chat::ChatScreen::new()),
        "agents" => Box::new(agents::AgentsScreen::new()),
        _ => return None,
    })
}

/// A fresh view for an overlay panel: `ai_actions` (quick actions over selected text / the clipboard), `ai_setup` (the setup wizard).
pub fn create_overlay(name: &str) -> Option<Box<dyn View>> {
    Some(match name {
        "ai_actions" => Box::new(ai_actions::AiActionsScreen::new()),
        "ai_setup" => Box::new(ai_setup::AiSetupScreen::new()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_knows_set_b_only() {
        for id in ["note", "text", "chat", "agents"] {
            assert!(create(id).is_some(), "{id}");
        }
        for id in ["home", "weather", "clipboard", "", "settings"] {
            assert!(create(id).is_none(), "{id}");
        }
        assert!(create_overlay("ai_actions").is_some() && create_overlay("ai_setup").is_some() && create_overlay("x").is_none());
    }
}
