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

/// The sixteen pages of the old sidebar: the single place the rows are defined.
fn old_pages() -> Vec<Page> {
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
        phone::phone(),
        ai::ai(),
        ai::aisetup(),
        ai::voice(),
        ai::updates(),
    ]
}

/// The old pages as they were, for the tests that hold the new layout to them.
#[cfg(test)]
pub(crate) fn old_pages_for_tests() -> Vec<Page> {
    old_pages()
}

/// The pages of the sidebar, one per function, in the order of `nav::NAV`. The rows are defined by the sixteen old page builders; `layout`
/// arranges them (see there).
pub fn build() -> Vec<Page> {
    use layout as l;
    let mut pool = l::Pool::new(old_pages());
    let p = &mut pool;
    let pages = vec![
        // Island
        l::look(p),
        l::edge(p),
        l::pet(p),
        l::clock(p),
        l::tabs(p),
        l::animation(p),
        // Behaviour
        l::general(p),
        l::hotkeys(p),
        // Features
        l::weather(p),
        l::media(p),
        l::volume(p),
        l::clipboard(p),
        l::screenshots(p),
        l::recording(p),
        l::notes(p),
        l::launcher(p),
        l::shelf(p),
        l::text(p),
        l::controls(p),
        l::vpn(p),
        l::voice(p),
        // Events
        l::notifications(p),
        l::downloads(p),
        l::battery(p),
        l::keyboard(p),
        l::desktops(p),
        l::usb(p),
        l::obs(p),
        l::calendar(p),
        l::mail(p),
        // Phone
        l::phone(p),
        // AI
        l::ai(p),
        l::agents(p),
        l::localai(p),
        // App
        l::updates(p),
    ];
    pool.finish();
    pages
}
