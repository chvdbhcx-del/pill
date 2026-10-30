//! The native command bar's pure logic: a view-model + `Action` enum + `on_event` + `tick` (see `model`), with the uninstall panel in `uninstall`.
//! Rendering (Direct2D, glasskit controls) and the real backend calls live elsewhere; nothing in this directory touches tauri or windows.
pub mod anim;
pub mod ask;
pub mod keys;
pub mod layout;
pub mod lang;
pub mod model;
pub mod protocol;
pub mod query;
pub mod results;
pub mod strings;
pub mod uninstall;
pub mod wave;

pub use anim::{Anim, Ease, Kind as AnimKind, Target as AnimTarget};
pub use keys::{KeyEvent, RunMode};
pub use lang::{Lang, Tr};
pub use model::{Action, Bar, BarEvent, BarView};
pub use protocol::BackendCall;
