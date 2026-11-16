//! Keyboard-first use and accessibility: the control model shared by the focus ring, the keyboard routing and the UI Automation provider (`nav`),
//! the animated focus ring (`ring`), who owns keyboard mode (`target`) and high contrast (`contrast`). See docs/native-a11y.md.
pub mod contrast;
pub mod nav;
pub mod ring;
pub mod target;
