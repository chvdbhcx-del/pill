//! `TabFx`: what a native panel tab asks of the ISLAND (everything that is not drawing inside the tab and not a backend call).
//!
//! The tab models of `pill-ui-core::panel::a` queue `Fx` values (toast, collapse, expand, live row, pet reaction, tab badge ...). A tab view
//! keeps the view-local ones (animation cues, focus / scroll requests of its own search box) and forwards the rest to the host that embeds it
//! as a `TabFx`, through `cx.post(Msg::Custom(Box::new(TabFx::..)))`.
//!
//! # Contract for the island view
//! * The tab view is driven with the island's own `Cx`, so `cx.post` lands in the island's `View::message` as `Msg::Custom`. Downcast it:
//!   `if let Msg::Custom(b) = msg { if let Some(fx) = b.downcast_ref::<TabFx>() { ... } }` (or `b.downcast::<TabFx>()` to own it).
//! * Every other `Msg` (`Event`, `Reply`, `Open`, other `Custom`) must be forwarded to the active tab view: the tab subscribes to its backend
//!   events and runs its commands through the same `Cx`, so the answers come back to the island first.
//! * A `TabFx` is delivered at most once, in the order the model queued it.
use pill_ui_core::panel::a::{Fx, LiveSpec};
use pill_ui_core::panel::b::{LiveRow, ToastSpec};
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq)]
pub enum TabFx {
    /// A short notice in the island (`Pill.toast`): icon name of `icons.js`, title, how long it stays.
    Toast { id: &'static str, icon: &'static str, title: String, ttl_ms: u32 },
    /// `Pill.collapse(via)`; `via` is `Some("click")` for a click that should close the island.
    Collapse { via: Option<&'static str> },
    /// `Pill.expand(tab)`: open the panel on the tab with this id.
    Expand { tab: &'static str },
    /// `Pill.setLive(key, spec)`: show a row in the folded island (`None` removes it).
    Live { key: &'static str, spec: Option<LiveSpec> },
    /// `Pill.react(key, kind, ..)`: the pet / edge reaction.
    React { key: String, kind: &'static str, color: Option<String>, ttl_ms: Option<u32> },
    Unreact { key: String },
    /// `Pill.setBadge(tab, text)`; empty text hides the badge on the tab button.
    Badge { tab: &'static str, text: String },
    // ---- set B (emitted through [`emit`]; the island drains them with [`take`] after every call it forwards to a B tab)
    /// `Pill.toast` with buttons: a card under the collapsed pill.
    Card(ToastSpec),
    /// `Pill.setLive(key, row)` of a B tab (text row); `None` clears it. (`Live` is the A tabs' music / shelf rows, which the hub owns.)
    SetLive { key: String, row: Option<LiveRow> },
    /// Liquid edge ripple of this strength.
    EdgePulse(f32),
    /// Start / stop dictation (`PillVoice.toggle`).
    VoiceToggle,
    /// Open this Settings page (`go('voice')`, `go('ai')`).
    GoPage(&'static str),
    /// An in-panel overlay of set B closed itself (`ai_actions`): the host tab (clipboard) takes its place back.
    OverlayClosed(&'static str),
    /// The island's pin state changed (pinned = stays open).
    SetPinned(bool),
    OpenCommandBar,
    /// `PillCapture.start(mode)`: collapse, then start a screenshot of this mode.
    StartCapture { mode: &'static str },
    /// The AI-actions panel over the clipboard row of this item (drawn by the island / tabs B).
    OpenClipAi { id: i64 },
    CloseClipAi,
    /// The shelf drag-pull edge glow.
    DragPull(bool),
    /// The island's edge colour (`None` = default).
    EdgeColor(Option<String>),
}

thread_local! {
    static OUT: RefCell<Vec<TabFx>> = const { RefCell::new(Vec::new()) };
}

/// The shared sink: queue an effect for the island (UI thread only). Set-B views and overlays use it (set A posts `Msg::Custom` instead).
pub fn emit(fx: TabFx) {
    OUT.with(|o| o.borrow_mut().push(fx));
}

/// Everything emitted since the last call, oldest first. The island drains it after every call it forwards to a B tab / overlay.
pub fn take() -> Vec<TabFx> {
    OUT.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

impl TabFx {
    /// The island-bound part of a model `Fx`; `None` for the view-local variants (`Anim`, `FocusSearch`, `BlurSearch`, `ScrollTo`).
    pub fn from_fx(fx: &Fx) -> Option<TabFx> {
        Some(match fx.clone() {
            Fx::Toast { id, icon, title, ttl_ms } => TabFx::Toast { id, icon, title, ttl_ms },
            Fx::Collapse { via } => TabFx::Collapse { via },
            Fx::Expand { tab } => TabFx::Expand { tab },
            Fx::Live { key, spec } => TabFx::Live { key, spec },
            Fx::React { key, kind, color, ttl_ms } => TabFx::React { key: key.to_string(), kind, color, ttl_ms },
            Fx::Unreact { key } => TabFx::Unreact { key: key.to_string() },
            Fx::Badge { tab, text } => TabFx::Badge { tab, text },
            Fx::SetPinned(b) => TabFx::SetPinned(b),
            Fx::OpenCommandBar => TabFx::OpenCommandBar,
            Fx::StartCapture { mode } => TabFx::StartCapture { mode },
            Fx::OpenClipAi { id } => TabFx::OpenClipAi { id },
            Fx::CloseClipAi => TabFx::CloseClipAi,
            Fx::DragPull(b) => TabFx::DragPull(b),
            Fx::EdgeColor(c) => TabFx::EdgeColor(c),
            Fx::Anim { .. } | Fx::FocusSearch | Fx::BlurSearch | Fx::ScrollTo { .. } => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pill_ui_core::panel::a::Anim;

    #[test]
    fn emit_then_take_is_fifo_and_clears() {
        let _ = take();
        emit(TabFx::Collapse { via: None });
        emit(TabFx::GoPage("ai"));
        assert_eq!(take(), vec![TabFx::Collapse { via: None }, TabFx::GoPage("ai")]);
        assert!(take().is_empty());
    }

    #[test]
    fn island_bound_fx_convert_and_view_local_ones_do_not() {
        assert_eq!(TabFx::from_fx(&Fx::Collapse { via: Some("click") }), Some(TabFx::Collapse { via: Some("click") }));
        assert_eq!(TabFx::from_fx(&Fx::Badge { tab: "notifs", text: "3".into() }), Some(TabFx::Badge { tab: "notifs", text: "3".into() }));
        assert!(TabFx::from_fx(&Fx::FocusSearch).is_none());
        assert!(TabFx::from_fx(&Fx::ScrollTo { target: "row:1".into() }).is_none());
        assert!(TabFx::from_fx(&Fx::Anim { target: "x".into(), anim: Anim::ring() }).is_none());
    }
}
