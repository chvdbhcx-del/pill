//! The data of every settings page, in sidebar order. `build()` is called once by `schema::schema()`.
//!
//! Row counts and wording come from `src/settings.js` (+ `ai-setup.js`, `voice-ui.js`, `settings-live-ext.js`); defaults from the
//! JS `DEF` table, which equals `settings.rs::defaults()` wherever the latter has the key (checked in `tests.rs`).
use crate::settings::schema::Page;

mod ai;
mod base;
mod system;

/// Schema ids and rows are `&'static str`; the few dynamic ones (hotkey.<action>) are leaked once, at schema build.
pub(crate) fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub fn build() -> Vec<Page> {
    vec![
        base::character(),
        base::general(),
        base::appearance(),
        base::behavior(),
        base::hotkeys(),
        system::modules(),
        system::live(),
        system::clipboard(),
        system::search(),
        system::sound(),
        system::weather(),
        ai::ai(),
        ai::aisetup(),
        ai::voice(),
        ai::updates(),
    ]
}
