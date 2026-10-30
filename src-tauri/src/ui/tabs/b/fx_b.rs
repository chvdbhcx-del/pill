//! The side effects of the set-B tabs that belong to the ISLAND (not to the tab): a card in the collapsed pill, a live row, the edge ripple, a
//! badge on the tab button, collapsing the pill after a paste, dictation, jumping to a Settings page.
//!
//! `TabFx` is the same name TABS-A defines in `ui/tabs/fx.rs` (with its own variants). This file only has the variants set B needs, so the lead
//! can merge by adding these to the shared enum and replacing [`emit`]'s body with the shared sink (or `impl From<fx_b::TabFx> for fx::TabFx`
//! and push the converted value). A view calls [`emit`] on the UI thread; the island drains with [`take`] after every call it forwards to a tab.
use pill_ui_core::panel::b::{LiveRow, ToastSpec};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq)]
pub enum TabFx {
    /// `Pill.toast`: a card under the collapsed pill.
    Card(ToastSpec),
    /// `Pill.setLive(key, row)`; `None` clears the row.
    SetLive { key: String, row: Option<LiveRow> },
    /// `Pill.react(key, kind, ttl)`.
    React { key: String, kind: &'static str, ttl_ms: Option<u32> },
    Unreact(String),
    /// Liquid edge ripple of this strength.
    EdgePulse(f32),
    /// Counter on a tab button.
    SetBadge { tab: &'static str, count: u32 },
    /// Collapse the pill (after a paste).
    CollapsePill,
    /// Start / stop dictation (`PillVoice.toggle`).
    VoiceToggle,
    /// Open this Settings page (`go('voice')`, `go('ai')`).
    GoPage(&'static str),
    /// An in-panel overlay of set B closed itself (`ai_actions`): the host tab (clipboard) takes its place back.
    OverlayClosed(&'static str),
}

thread_local! {
    static OUT: RefCell<Vec<TabFx>> = const { RefCell::new(Vec::new()) };
}

/// Queue an effect for the island (UI thread only).
pub fn emit(fx: TabFx) {
    OUT.with(|o| o.borrow_mut().push(fx));
}

/// Everything emitted since the last call, oldest first.
pub fn take() -> Vec<TabFx> {
    OUT.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_then_take_is_fifo_and_clears() {
        let _ = take();
        emit(TabFx::CollapsePill);
        emit(TabFx::GoPage("ai"));
        assert_eq!(take(), vec![TabFx::CollapsePill, TabFx::GoPage("ai")]);
        assert!(take().is_empty());
    }
}
