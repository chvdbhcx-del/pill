//! Sidebar, "open settings on a page" and the window geometry rules of the settings window.
//! Ports: `PAGES` / `buildSide` / `show` of settings.js, `settings_open` + `PAGES` + position memory of settingswin.rs,
//! `pill.exe --open settings|agents` of openarg.rs. The window itself (Win32) is not here; this is the decision logic.
use crate::settings::anim::{self, AnimSpec};
use crate::settings::schema::schema;
use crate::settings::tr::{tr, Lang, Tr};

/// One sidebar entry: a page or a separator line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavItem {
    Page(&'static str),
    Sep,
}

/// Sidebar order: how it looks, how it behaves and the keys; what the island shows; the phone; the AI; the update page.
pub const NAV: [NavItem; 13] = [
    NavItem::Page("appearance"),
    NavItem::Page("general"),
    NavItem::Page("hotkeys"),
    NavItem::Sep,
    NavItem::Page("modules"),
    NavItem::Page("live"),
    NavItem::Page("clipboard"),
    NavItem::Page("sound"),
    NavItem::Page("phone"),
    NavItem::Sep,
    NavItem::Page("ai"),
    NavItem::Sep,
    NavItem::Page("updates"),
];

/// Icon ids of the sidebar (one per page, plus "power" for Quit); the renderer owns the drawings.
pub const ICONS: [&str; 17] = ["character", "general", "hotkeys", "ai", "voice", "weather", "sound", "live", "clipboard", "modules", "updates", "aisetup", "appearance", "behavior", "search", "phone", "power"];

/// The pages `settings_open` accepts (settingswin.rs `PAGES`): the ten of the sidebar, the six that were merged into them
/// (`LEGACY`; callers in other files, saved links and `--open` still use those names) and "welcome", which starts the wizard.
pub const OPENABLE: [&str; 17] = ["character", "general", "appearance", "behavior", "search", "hotkeys", "ai", "voice", "weather", "sound", "live", "clipboard", "modules", "updates", "aisetup", "phone", "welcome"];

/// Pages that are not in the sidebar any more: (old id, the page that holds their rows now, a row at the start of the old section there).
pub const LEGACY: [(&str, &str, &str); 6] = [
    ("character", "appearance", "character.liquid"),
    ("behavior", "general", "hideInFullscreen"),
    ("search", "general", "search.engine"),
    ("weather", "modules", "net.weather"),
    ("voice", "sound", "voice.enabled"),
    ("aisetup", "ai", "aisetup.intro"),
];

/// Where a page name leads: (the sidebar page, the row to scroll to when the name was an old page). Case is the caller's business.
pub fn route(id: &str) -> Option<(&'static str, Option<&'static str>)> {
    if let Some((_, page, anchor)) = LEGACY.iter().find(|(old, _, _)| *old == id) {
        return Some((page, Some(anchor)));
    }
    NAV.iter().find_map(|n| match n {
        NavItem::Page(p) if *p == id => Some((*p, None)),
        _ => None,
    })
}

pub const SAVED_NOTE: Tr = tr("Изменения сохраняются сразу", "Changes are saved instantly");
pub const QUIT: Tr = tr("Выйти из Pill", "Quit Pill");
pub const APP_NAME: &str = "Pill";

pub fn version_line(lang: Lang, version: &str) -> String {
    if version.is_empty() {
        String::new()
    } else {
        crate::settings::tr::pick(lang, "Версия ", "Version ").to_string() + version
    }
}

/// The page opened when nothing (or something unknown) is asked for: the first one, Appearance.
pub const DEFAULT_PAGE: &str = "appearance";

/// What a request to open a page resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Page(&'static str),
    Wizard,
}

/// `settings_open(page)` / the `page=` query / the `settings:page` event: case-insensitive, unknown names are dropped.
pub fn resolve_open(page: Option<&str>) -> Option<OpenTarget> {
    let p = page?.to_ascii_lowercase();
    let id = OPENABLE.iter().find(|x| **x == p)?;
    Some(if *id == "welcome" { OpenTarget::Wizard } else { OpenTarget::Page(route(id)?.0) })
}

/// What the window shows first: the requested page, else Appearance; the wizard for "welcome".
pub fn initial(page: Option<&str>) -> OpenTarget {
    resolve_open(page).unwrap_or(OpenTarget::Page(DEFAULT_PAGE))
}

/// `show(id)` for an id that is not a page falls back to the first page; an old page name leads to the page that holds its rows now.
pub fn page_or_default(id: &str) -> &'static str {
    route(id).map(|r| r.0).unwrap_or(DEFAULT_PAGE)
}

/// The row to scroll to when a window is asked for an old page (`behavior` -> the start of the hiding rows on General), if `id` is one.
pub fn legacy_anchor(id: &str) -> Option<&'static str> {
    route(&id.to_ascii_lowercase()).and_then(|r| r.1)
}

/// What the window does when a page is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct ShowPlan {
    pub page: &'static str,
    /// The heading (h1) of the page, in `lang`.
    pub heading: &'static str,
    /// Old page slides out (None: first page, wizard, or Reduce motion).
    pub exit: Option<AnimSpec>,
    pub enter: AnimSpec,
    /// The sidebar indicator glides to the new item.
    pub indicator: AnimSpec,
    pub scroll_to_top: bool,
}

pub fn show_plan(id: &str, lang: Lang, reduce: bool, wizard: bool, has_old: bool) -> ShowPlan {
    let page = page_or_default(id);
    let (exit, enter) = anim::page_switch(reduce, wizard);
    ShowPlan {
        page,
        heading: schema().page(page).map(|p| p.heading().get(lang)).unwrap_or(""),
        exit: if has_old { exit } else { None },
        enter,
        indicator: anim::spec(anim::AnimKind::SidebarIndicator, reduce),
        scroll_to_top: true,
    }
}

// ------------------------------------------------------------------------------------------------ window geometry

/// Wide enough for the page and the live preview panel next to it (a saved size always wins).
pub const DEFAULT_SIZE: (f64, f64) = (1180.0, 760.0);
pub const MIN_SIZE: (f64, f64) = (700.0, 520.0);
pub const MAX_SIZE: f64 = 4000.0;

/// `settingsWin.w` / `settingsWin.h` -> the creation size (default 1180 x 760, clamped to 700..4000 / 520..4000).
pub fn initial_size(w: Option<f64>, h: Option<f64>) -> (f64, f64) {
    (w.unwrap_or(DEFAULT_SIZE.0).clamp(MIN_SIZE.0, MAX_SIZE), h.unwrap_or(DEFAULT_SIZE.1).clamp(MIN_SIZE.1, MAX_SIZE))
}

/// The settings keys the window remembers its geometry in (never exported, see settings_backup `is_runtime`).
pub const GEOMETRY_KEYS: [&str; 5] = ["settingsWin.x", "settingsWin.y", "settingsWin.w", "settingsWin.h", "settingsWin.max"];

/// True when at least a 120 x 40 px piece of the title bar area lies on one of the monitors (x, y, w, h).
pub fn pos_visible(x: i32, y: i32, w: i32, _h: i32, mons: &[(i32, i32, i32, i32)]) -> bool {
    mons.iter().any(|&(mx, my, mw, mh)| {
        let ix = (x + w).min(mx + mw) - x.max(mx);
        let iy = (y + 40).min(my + mh) - y.max(my);
        ix >= 120 && iy >= 40
    })
}

/// Saved geometry is stored debounced (700 ms after the last move/resize) and on close.
pub const SAVE_DEBOUNCE_MS: u64 = 700;

/// The window is created hidden and shown after the first paint; if the page never runs, it shows itself after this long.
pub const SHOW_SAFETY_NET_MS: u64 = 2500;

// ------------------------------------------------------------------------------------------------ pill.exe --open

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LaunchTarget {
    Agents,
    Settings,
}

/// `args` is the full argv (program name first). The first `--open` decides; an unknown or missing value yields None.
/// The ONLY accepted forms are `--open agents|settings` and `--open=agents|settings` (case-insensitive); nothing is executed.
pub fn parse_open_arg(args: &[String]) -> Option<LaunchTarget> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let value = if a.eq_ignore_ascii_case("--open") { it.next().map(String::as_str) } else { a.get(..7).filter(|p| p.eq_ignore_ascii_case("--open=")).map(|_| &a[7..]) };
        if let Some(v) = value {
            return match v.to_ascii_lowercase().as_str() {
                "agents" => Some(LaunchTarget::Agents),
                "settings" => Some(LaunchTarget::Settings),
                _ => None,
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Option<LaunchTarget> {
        parse_open_arg(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn open_arg_accepts_only_the_two_fixed_words() {
        assert_eq!(p(&["pill.exe", "--open", "agents"]), Some(LaunchTarget::Agents));
        assert_eq!(p(&["pill.exe", "--open=Settings"]), Some(LaunchTarget::Settings));
        assert_eq!(p(&["pill.exe", "--other", "--OPEN", "AGENTS"]), Some(LaunchTarget::Agents));
    }

    #[test]
    fn open_arg_ignores_everything_else() {
        for a in [&["pill.exe"][..], &["pill.exe", "--open"], &["pill.exe", "--open", "calc.exe"], &["pill.exe", "--open=", "agents"], &["pill.exe", "--open", ""], &["pill.exe", "agents"], &["--open", "agents"], &["pill.exe", "--open", "agents "], &["pill.exe", "--öpen=agents"]] {
            assert_eq!(p(a), None, "{a:?}");
        }
    }

    #[test]
    fn sidebar_lists_every_page_once_in_schema_order() {
        let pages: Vec<&str> = NAV.iter().filter_map(|n| if let NavItem::Page(p) = n { Some(*p) } else { None }).collect();
        let ids: Vec<&str> = schema().pages.iter().map(|p| p.id).collect();
        let mut sorted_nav = pages.clone();
        sorted_nav.sort();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(sorted_nav, sorted_ids, "every page is in the sidebar exactly once");
        assert_eq!(pages.len(), 10);
        assert!(pages.len() <= 11, "the sidebar stays short");
        assert_eq!(NAV.iter().filter(|n| **n == NavItem::Sep).count(), 3);
        for pg in &schema().pages {
            assert!(ICONS.contains(&pg.icon), "icon {}", pg.icon);
            assert!(OPENABLE.contains(&pg.id), "settings_open must accept {}", pg.id);
        }
        assert!(ICONS.contains(&"power"));
    }

    #[test]
    fn open_requests_resolve_like_settingswin() {
        assert_eq!(resolve_open(Some("Hotkeys")), Some(OpenTarget::Page("hotkeys")));
        assert_eq!(resolve_open(Some("WELCOME")), Some(OpenTarget::Wizard));
        assert_eq!(resolve_open(Some("calc.exe")), None);
        assert_eq!(resolve_open(None), None);
        assert_eq!(initial(Some("bogus")), OpenTarget::Page("appearance"));
        assert_eq!(initial(Some("updates")), OpenTarget::Page("updates"));
        assert_eq!(page_or_default("nope"), "appearance");
    }

    #[test]
    fn every_old_page_name_still_opens_the_page_that_holds_its_rows() {
        // the sixteen ids of the old sidebar (+ the wizard): each resolves, none to a page that does not exist
        let old = ["character", "general", "appearance", "behavior", "search", "hotkeys", "ai", "voice", "weather", "sound", "live", "clipboard", "modules", "updates", "aisetup", "phone"];
        for id in old {
            let Some(OpenTarget::Page(p)) = resolve_open(Some(id)) else { panic!("{id} does not open") };
            assert!(schema().page(p).is_some(), "{id} -> {p} is not a page");
            assert_eq!(page_or_default(id), p);
            assert_eq!(show_plan(id, Lang::En, true, false, false).page, p);
        }
        assert_eq!(resolve_open(Some("WELCOME")), Some(OpenTarget::Wizard));
        for (id, page) in [("character", "appearance"), ("behavior", "general"), ("search", "general"), ("weather", "modules"), ("voice", "sound"), ("aisetup", "ai"), ("live", "live")] {
            assert_eq!(resolve_open(Some(id)), Some(OpenTarget::Page(page)), "{id}");
        }
        // an old name also names the row to scroll to; a page that is still there does not
        assert_eq!(legacy_anchor("Behavior"), Some("hideInFullscreen"));
        assert_eq!(legacy_anchor("hotkeys"), None);
        for (old, page, anchor) in LEGACY {
            assert!(!NAV.contains(&NavItem::Page(old)), "{old} is not in the sidebar any more");
            let row = schema().row(anchor).unwrap_or_else(|| panic!("anchor {anchor} of {old}"));
            assert_eq!(schema().page_of(row.id).map(|p| p.id), Some(page), "anchor {anchor} of {old} lives on {page}");
        }
    }

    #[test]
    fn show_plan_animates_unless_reduced_or_in_the_wizard() {
        let a = show_plan("hotkeys", Lang::Ru, false, false, true);
        assert_eq!((a.page, a.heading), ("hotkeys", "Горячие клавиши"));
        assert!(a.exit.is_some() && !a.enter.is_instant() && !a.indicator.is_instant() && a.scroll_to_top);
        assert!(show_plan("hotkeys", Lang::En, false, false, false).exit.is_none(), "nothing to slide out the first time");
        assert!(show_plan("hotkeys", Lang::En, false, true, true).exit.is_none(), "no exit in the wizard");
        let r = show_plan("general", Lang::En, true, false, true);
        assert!(r.exit.is_none() && r.enter.is_instant() && r.indicator.is_instant());
        assert_eq!(r.heading, "General");
    }

    #[test]
    fn window_size_and_position_memory() {
        assert_eq!(initial_size(None, None), (1180.0, 760.0));
        assert_eq!(initial_size(Some(100.0), Some(10.0)), (700.0, 520.0));
        assert_eq!(initial_size(Some(9000.0), Some(9000.0)), (4000.0, 4000.0));
        let mons = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        assert!(pos_visible(100, 100, 840, 760, &mons));
        assert!(pos_visible(2000, 50, 840, 760, &mons));
        assert!(!pos_visible(5000, 50, 840, 760, &mons), "monitor unplugged");
        assert!(!pos_visible(-900, 100, 840, 760, &mons), "only 0 px visible");
        assert!(!pos_visible(100, 1070, 840, 760, &mons), "title bar below the screen");
        assert!(pos_visible(-720, 100, 840, 760, &[(-1920, 0, 1920, 1080), (0, 0, 1920, 1080)]));
        assert_eq!(version_line(Lang::En, "0.3.8"), "Version 0.3.8");
        assert_eq!(version_line(Lang::Ru, ""), "");
    }
}
