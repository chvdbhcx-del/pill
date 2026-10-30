//! shell: owned by one agent; see ui-core/README.md.
//!
//! The pure model behind pill.js (the capsule): a deterministic state machine plus a renderer-facing [`View`].
//!
//! * [`capsule::Shell`] - the orchestrator. Feed [`types::Event`]s with `handle`, call `tick(dt_ms, now_ms)` once per display frame, draw `view()`.
//!   Commands for the Rust backend leave through [`io::Backend`]; wall time comes from [`io::Clock`].
//! * [`react`] - edge reaction presets + priority resolver; [`liquid`] - the height-field edge simulation; [`lives`] - live-row slot choice;
//!   [`clock`] - clock / weather / battery row; [`cards`] - event-card queue; [`quiet`] - quiet hours; [`spring`] / [`motion`] - the body
//!   spring tables and CSS animation tables; [`tabs`] - tab registry; [`settings`] - typed `ShellSettings`.
pub mod capsule;
pub mod cards;
pub mod clock;
pub mod color;
pub mod io;
pub mod lives;
pub mod liquid;
pub mod motion;
pub mod quiet;
pub mod react;
pub mod settings;
pub mod spring;
pub mod tabs;
pub mod timers;
pub mod types;
mod rowlive;
#[cfg(test)]
mod tests;
mod view;

pub use capsule::{PillState, Shell};
pub use cards::{CardAction, CardSpec};
pub use clock::{CharSide, ClockSlots, SlotItem};
pub use io::{Backend, Clock};
pub use lives::{LiveInput, LiveSlot};
pub use react::{Preset, ReactOpts};
pub use settings::ShellSettings;
pub use types::{Event, Notice, Phase, View};
