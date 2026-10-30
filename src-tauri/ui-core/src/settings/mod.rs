//! Settings window logic: a declarative schema plus the state machines around it (no tauri, no windows crate).
//!
//! A renderer (glasskit controls: Toggle, Slider, Segmented, Select, Button, TextInput, hotkey capture) draws pages straight
//! from `schema()`; it never hand-builds a page. Everything the web window did in `settings.js` is split like this:
//!
//! | module | what |
//! |---|---|
//! | `tr` | `Lang` and the `Tr(ru, en)` text pair every label uses |
//! | `schema` | `Page > Group > Row`, `Control`, `Cond`, the lookups; `pages/*` hold the data (15 pages) |
//! | `model` | `SettingsModel`: values, defaults, `try_set` -> `Vec<BackendCall>`, validation, `on_event`, row visibility |
//! | `calls` | `BackendCall`: command name + camelCase args of every `invoke` the page makes, plus `Ui*` effects |
//! | `hotkey` | action table, combo parsing/conflicts, the key recorder state machine |
//! | `search` | Ctrl+F: matching, ranking, highlight ranges, index builder, result list state |
//! | `wizard` | first-run wizard steps, gate, completion flag |
//! | `pickers` | island styles, reaction chips, pet cards by group |
//! | `providers` | AI / voice provider chips and the patches they write |
//! | `aisetup` | AI setup page: Whisper plan (what a click downloads), Ollama steps, found servers |
//! | `flows` | messages for every backend answer, update/confirm/progress flows, polled status lines |
//! | `whatsnew` | CHANGELOG notes and the after-update card |
//! | `nav` | sidebar, `settings_open` page names, window geometry, `pill.exe --open` |
//! | `anim` | animation kinds as data (page slide, row expand, feedback, search stagger, wizard slide), Reduce motion |
//!
//! Standing rules kept here: no key is invented or renamed (users' settings.json keeps working; `tests.rs` compares the schema
//! with `settings.rs` defaults), network features stay opt-in (`net.ai`, `net.weather`, `live.mail`, `live.calendar`, `live.obs`,
//! `voice.enabled` default to off), secrets never travel through `settings_set` (they go to DPAPI storage via their own
//! command), and every animation has an instant variant for Reduce motion. Frame rates and refresh options are copied
//! exactly (`fps`: auto/30/60/120/144/240, `edgeIdleFps`: 15/30/60); nothing here lowers them.
pub mod aisetup;
pub mod anim;
pub mod calls;
pub mod flows;
pub mod hotkey;
pub mod model;
pub mod nav;
mod pages;
pub mod pickers;
pub mod providers;
pub mod schema;
pub mod search;
pub mod tr;
pub mod whatsnew;
pub mod wizard;

#[cfg(test)]
mod tests;

pub use calls::BackendCall;
pub use model::{SetError, SettingsModel};
pub use schema::{schema, Control, Page, Row};
pub use tr::{Lang, Tr};
