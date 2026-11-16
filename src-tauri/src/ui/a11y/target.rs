//! Which window owns keyboard mode. One hotkey (default Win+Alt+J) serves two windows - the island's card and the music widget - and
//! walks them in turn: first press = the island, second = the widget (when it is on screen), third = off.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Island,
    Widget,
}

/// The bus event the hotkey emits: `{ "target": "island" | "widget" | "off" }`. Every keyboard-capable view listens and takes or drops the keys.
pub const EVENT: &str = "a11y:keys";

/// Where the hotkey goes next from `active` (who has keyboard mode now); `None` = switch keyboard mode off.
pub fn next(active: Option<Target>, widget_up: bool) -> Option<Target> {
    match active {
        None => Some(Target::Island),
        Some(Target::Island) if widget_up => Some(Target::Widget),
        Some(_) => None,
    }
}

impl Target {
    pub fn name(t: Option<Target>) -> &'static str {
        match t {
            Some(Target::Island) => "island",
            Some(Target::Widget) => "widget",
            None => "off",
        }
    }

    pub fn parse(s: &str) -> Option<Target> {
        match s {
            "island" => Some(Target::Island),
            "widget" => Some(Target::Widget),
            _ => None,
        }
    }
}

static ACTIVE: AtomicU8 = AtomicU8::new(0);
static WIDGET_UP: AtomicBool = AtomicBool::new(false);

pub fn active() -> Option<Target> {
    match ACTIVE.load(Ordering::Relaxed) {
        1 => Some(Target::Island),
        2 => Some(Target::Widget),
        _ => None,
    }
}

/// A view tells that it has (or no longer has) keyboard mode.
pub fn set_active(t: Option<Target>) {
    ACTIVE.store(t.map_or(0, |t| if t == Target::Island { 1 } else { 2 }), Ordering::Relaxed);
}

/// A view that ends its keyboard mode clears only its own claim.
pub fn release(t: Target) {
    if active() == Some(t) {
        set_active(None);
    }
}

pub fn set_widget_up(up: bool) {
    WIDGET_UP.store(up, Ordering::Relaxed);
}

pub fn widget_up() -> bool {
    WIDGET_UP.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hotkey_walks_island_then_widget_then_off() {
        assert_eq!(next(None, false), Some(Target::Island));
        assert_eq!(next(Some(Target::Island), true), Some(Target::Widget));
        assert_eq!(next(Some(Target::Island), false), None, "no widget on screen: the second press ends it");
        assert_eq!(next(Some(Target::Widget), true), None);
    }

    #[test]
    fn names_round_trip() {
        for t in [Some(Target::Island), Some(Target::Widget)] {
            assert_eq!(Target::parse(Target::name(t)), t);
        }
        assert_eq!(Target::parse(Target::name(None)), None);
    }
}
