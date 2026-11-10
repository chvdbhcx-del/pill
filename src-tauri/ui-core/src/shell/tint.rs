//! The island's own colour: nine ready presets and any `#RRGGBB`, the contrast rule that decides between light and dark text on it,
//! the theme toggle of the hotkey and the card-duration factor. Pure data and arithmetic; the shell and the settings page share it.
//!
//! Settings: `island.color` (a ready colour of [`PRESETS`], or any hex) and `island.colorCustom` (a typed hex that wins when valid).
//! An empty value keeps the theme's own body colour (black, or light grey in the light theme).

use crate::settings::tr::{tr, Tr};
use crate::shell::color::{Rgba, Theme};

/// A ready colour: the hex stored in `island.color` and its names.
pub struct ColorPreset {
    pub hex: &'static str,
    pub name: Tr,
}

/// The nine ready colours (the default, black, is the empty value and has no entry here).
pub const PRESETS: [ColorPreset; 9] = [
    ColorPreset { hex: "#1c1c1e", name: tr("Графит", "Graphite") },
    ColorPreset { hex: "#0b1b3d", name: tr("Полночь", "Midnight") },
    ColorPreset { hex: "#0c3b47", name: tr("Океан", "Ocean") },
    ColorPreset { hex: "#10301f", name: tr("Хвоя", "Forest") },
    ColorPreset { hex: "#2c1448", name: tr("Слива", "Plum") },
    ColorPreset { hex: "#4a1020", name: tr("Вино", "Wine") },
    ColorPreset { hex: "#e8a33d", name: tr("Янтарь", "Amber") },
    ColorPreset { hex: "#bcd9f5", name: tr("Небо", "Sky") },
    ColorPreset { hex: "#f6f6f6", name: tr("Жемчуг", "Pearl") },
];

/// `#rgb`, `#rrggbb` (the `#` is optional); anything else is not a colour.
pub fn parse_hex(s: &str) -> Option<Rgba> {
    let h = s.trim().trim_start_matches('#');
    if !h.is_ascii() || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        6 => Some(Rgba::from_hex(v)),
        3 => {
            let d = |sh: u32| (((v >> sh) & 0xf) * 17) as u8;
            Some(Rgba::rgb(d(8), d(4), d(0)))
        }
        _ => None,
    }
}

/// Perceived brightness 0..1 (Rec. 601 weights on the sRGB values, the usual rule of thumb for "dark text or light text").
pub fn luma(c: Rgba) -> f32 {
    (0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32) / 255.0
}

/// True when dark text reads better on `c` (the light theme's text on the island).
pub fn wants_dark_text(c: Rgba) -> bool {
    luma(c) > 0.6
}

/// The theme whose text colours suit the body colour `c`.
pub fn theme_for(c: Rgba) -> Theme {
    if wants_dark_text(c) {
        Theme::Light
    } else {
        Theme::Dark
    }
}

/// The hotkey "dark / light": the `theme` value to store and whether the colour has to go (a colour decides the text by itself, so the
/// toggle drops it). `theme` is the stored setting, `os_light` the Windows app mode (for "auto").
pub fn toggled_theme(theme: &str, color: Option<Rgba>, os_light: bool) -> (&'static str, bool) {
    let now_light = match color {
        Some(c) => wants_dark_text(c),
        None => match theme {
            "light" => true,
            "auto" => os_light,
            _ => false,
        },
    };
    (if now_light { "dark" } else { "light" }, color.is_some())
}

/// `cards.duration` -> factor on every card's lifetime: short 0.6, normal 1, long 2.
pub fn card_scale(s: &str) -> f64 {
    match s {
        "short" => 0.6,
        "long" => 2.0,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_in_both_lengths_and_refuses_the_rest() {
        assert_eq!(parse_hex("#1c1c1e"), Some(Rgba::rgb(0x1c, 0x1c, 0x1e)));
        assert_eq!(parse_hex("FFF"), Some(Rgba::rgb(255, 255, 255)));
        assert_eq!(parse_hex(" #0af "), Some(Rgba::rgb(0, 0xaa, 0xff)));
        for bad in ["", "#", "#12", "#12345", "#1234567", "red", "#gggggg", "#12 456", "#ééé"] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn every_preset_parses_and_the_text_follows_the_brightness() {
        for p in PRESETS.iter() {
            assert!(parse_hex(p.hex).is_some(), "{}", p.hex);
            assert!(p.name.get(crate::settings::tr::Lang::Ru) != p.name.get(crate::settings::tr::Lang::En));
        }
        let dark_text: Vec<&str> = PRESETS.iter().filter(|p| wants_dark_text(parse_hex(p.hex).unwrap())).map(|p| p.hex).collect();
        assert_eq!(dark_text, ["#e8a33d", "#bcd9f5", "#f6f6f6"], "the bright ones get dark text, the deep ones keep white");
        assert_eq!(theme_for(Rgba::rgb(0, 0, 0)), Theme::Dark);
        assert_eq!(theme_for(Rgba::rgb(255, 255, 255)), Theme::Light);
        assert_eq!(theme_for(Rgba::rgb(255, 59, 48)), Theme::Dark, "a saturated red keeps white text");
    }

    #[test]
    fn the_theme_toggle_flips_what_is_shown_now() {
        assert_eq!(toggled_theme("dark", None, false), ("light", false));
        assert_eq!(toggled_theme("light", None, true), ("dark", false));
        assert_eq!(toggled_theme("auto", None, true), ("dark", false));
        assert_eq!(toggled_theme("auto", None, false), ("light", false));
        // a bright colour shows dark text = "light": the toggle goes dark and drops the colour
        assert_eq!(toggled_theme("dark", Some(Rgba::rgb(246, 246, 246)), false), ("dark", true));
        assert_eq!(toggled_theme("light", Some(Rgba::rgb(11, 27, 61)), false), ("light", true));
    }

    #[test]
    fn card_duration_factors() {
        assert_eq!((card_scale("short"), card_scale("normal"), card_scale("long"), card_scale("?")), (0.6, 1.0, 2.0, 1.0));
    }
}
