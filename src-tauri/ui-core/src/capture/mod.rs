//! Capture overlay logic, native port of src/capture.js + src/capture-live.js (the Rust backend is src-tauri/src/capture.rs).
//!
//! The backend freezes every monitor and opens one overlay window per monitor (`capture-0`, `capture-1`, ...). Each window runs
//! one [`CaptureModel`]: it asks `capture_session` / `capture_frame` for its monitor, reacts to pointer and keyboard [`Input`]s,
//! keeps the selection / markup / loupe state, and talks to the backend only through [`CaptureIo`] as [`BackendCall`]s
//! (`capture_commit {label, vw, vh, action, rect | x, y}`, `capture_cancel`, `capture_ready`, ...). The renderer draws
//! [`CaptureModel::view`] every frame and samples the animations in it.
//!
//! * [`geom`]    - points / rectangles, 8-handle hit test, move / resize / nudge / Shift-square / snap, css <-> frame <-> desktop
//!                 mapping over monitors of mixed DPI (`to_frame_rect` and `to_frame_point` mirror capture.rs bit for bit).
//! * [`markup`]  - tools, palette, ops in frame px, the `DrawCmd` draw list (port of `drawOp`), undo / redo history, op hit-testing.
//! * [`loupe`]   - colour picker: 15x15 magnified grid, hex / rgb / hsl text, lens placement.
//! * [`toolbar`] - toolbar items with rectangles, tips (ru/en), shortcuts, enabled / on / hover state, placement rule.
//! * [`anim`]    - animation kinds as data (handles pop, toolbar enter / leave, loupe follow, op insert, copy flash, ...), with
//!                 Reduce motion folded in; nothing here limits a frame rate.
//! * [`input`]   - `Mode`, `Phase`, `Input`, `Action`, `Options`; [`view`] - the `CaptureView` view-model; [`model`] - the machine.
//! * [`io`]      - `BackendCall`, `CommitArgs`, the `CaptureIo` trait and the `FakeIo` used by tests.
//! * [`live`]    - pill side: recording capsule + timer, Home tile, `capture:done` result cards, start plan.
//!
//! Fidelity: every constant, shortcut and ru/en label of the web page is kept (tolerance 8 px, minimum selection 4 px, palette,
//! 3 px strokes, 15x15 lens, bar metrics ...). Native extensions (Shift = square, arrow nudge, snap, redo, highlighter / ellipse /
//! numbered steps, editable ops, pin / shelf) are opt-in through [`Options`] except the harmless nudge and Shift-square, which
//! `Options::web_parity()` switches off.
pub mod anim;
pub mod geom;
pub mod input;
pub mod io;
pub mod live;
pub mod loupe;
pub mod markup;
pub mod model;
pub mod toolbar;
pub mod view;

pub use anim::{Anim, AnimKind, AnimSet, AnimSpec, AnimTarget};
pub use geom::{Cursor, Desktop, Handle, IRect, MonitorGeom, Pt, Rect, Scale, Snap};
pub use input::{Action, AfterSave, Button, Input, Key, Mode, Mods, Options, Phase, Reaction};
pub use io::{BackendCall, CaptureIo, CommitArgs, CommitKind, FakeIo, SessionInfo};
pub use markup::{DrawCmd, History, Op, OpKind, Tool, COLORS};
pub use model::{ArmOutcome, CaptureModel};
pub use view::CaptureView;

#[cfg(test)]
mod tests;
