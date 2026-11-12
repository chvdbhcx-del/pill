//! The data of every settings page, in sidebar order (`layout` arranges the rows the old page builders define). `build()` is called once by `schema::schema()`.
//!
//! Row counts and wording come from `src/settings.js` (+ `ai-setup.js`, `voice-ui.js`, `settings-live-ext.js`); defaults from the
//! JS `DEF` table, which equals `settings.rs::defaults()` wherever the latter has the key (checked in `tests.rs`).
use crate::settings::schema::Page;

mod ai;
mod base;
mod layout;
mod phone;
mod system;

/// Schema ids and rows are `&'static str`; the few dynamic ones (hotkey.<action>) are leaked once, at schema build.
pub(crate) fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// The ten pages of the sidebar. The rows are defined by the sixteen old page builders; `layout` arranges them (see there).
pub fn build() -> Vec<Page> {
    let mut pool = layout::Pool::new(vec![
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
        phone::phone(),
        ai::ai(),
        ai::aisetup(),
        ai::voice(),
        ai::updates(),
    ]);
    let pages = vec![
        layout::appearance(&mut pool),
        layout::general(&mut pool),
        layout::hotkeys(&mut pool),
        layout::modules(&mut pool),
        layout::live(&mut pool),
        layout::clipboard(&mut pool),
        layout::sound(&mut pool),
        layout::phone(&mut pool),
        layout::ai(&mut pool),
        layout::updates(&mut pool),
    ];
    pool.finish();
    pages
}
