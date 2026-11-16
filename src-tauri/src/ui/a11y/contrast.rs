//! High contrast / forced colours for the island. Windows' own high-contrast themes put system colours (`COLOR_WINDOW`, `COLOR_WINDOWTEXT`,
//! `COLOR_HIGHLIGHT`...) in the user's hands; when one is on, the island drops its glass and its tints and paints with exactly those
//! colours, with 2 px outlines and an opaque body. `Look > High contrast` (`a11y.contrast`) overrides: Auto = follow Windows, On = always (with
//! a built-in black-and-white palette when Windows has none), Off = never. Reduce motion is a separate switch and stays that way.
//!
//! `pick` and `theme` are pure; `read_system` is the one Win32 call (SPI_GETHIGHCONTRAST + GetSysColor).
use glasskit::theme::Theme;
use glasskit::Color;

/// The setting `a11y.contrast`.
pub const KEY: &str = "a11y.contrast";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Auto,
    On,
    Off,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "on" => Mode::On,
            "off" => Mode::Off,
            _ => Mode::Auto,
        }
    }
}

/// The system colours a high-contrast theme sets (what the Win32 `COLOR_*` indices answer).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SysColors {
    pub window: Color,
    pub window_text: Color,
    pub highlight: Color,
    pub highlight_text: Color,
    pub gray_text: Color,
    pub btn_face: Color,
    pub btn_text: Color,
    pub hotlight: Color,
}

/// What the island paints with while high contrast is on. Every colour is opaque.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color,
    pub text: Color,
    pub disabled: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub face: Color,
    pub face_text: Color,
    pub link: Color,
    /// the 2 px outline of the body and of controls
    pub outline: Color,
}

pub const OUTLINE_W: f32 = 2.0;

impl SysColors {
    /// Windows' "High Contrast Black" (the fallback of `Mode::On`, and the test fixture).
    pub const BLACK: SysColors = SysColors {
        window: Color::rgb8(0, 0, 0),
        window_text: Color::rgb8(255, 255, 255),
        highlight: Color::rgb8(0x1A, 0xEB, 0xFF),
        highlight_text: Color::rgb8(0, 0, 0),
        gray_text: Color::rgb8(0x3F, 0xF2, 0x3F),
        btn_face: Color::rgb8(0, 0, 0),
        btn_text: Color::rgb8(255, 255, 255),
        hotlight: Color::rgb8(255, 255, 0),
    };
    /// Windows' "High Contrast White".
    pub const WHITE: SysColors = SysColors {
        window: Color::rgb8(255, 255, 255),
        window_text: Color::rgb8(0, 0, 0),
        highlight: Color::rgb8(0x37, 0x00, 0x6E),
        highlight_text: Color::rgb8(255, 255, 255),
        gray_text: Color::rgb8(0x60, 0x00, 0x00),
        btn_face: Color::rgb8(255, 255, 255),
        btn_text: Color::rgb8(0, 0, 0),
        hotlight: Color::rgb8(0, 0, 0x80),
    };

    pub fn palette(&self) -> Palette {
        Palette {
            bg: self.window,
            text: self.window_text,
            disabled: self.gray_text,
            accent: self.highlight,
            on_accent: self.highlight_text,
            face: self.btn_face,
            face_text: self.btn_text,
            link: self.hotlight,
            outline: self.btn_text,
        }
    }
}

/// Which palette the island paints with: `None` = the normal look. `sys` is what Windows says (`Some` only while a high-contrast theme is on).
pub fn pick(mode: Mode, sys: Option<SysColors>) -> Option<Palette> {
    match mode {
        Mode::Off => None,
        Mode::Auto => sys.map(|s| s.palette()),
        Mode::On => Some(sys.unwrap_or(SysColors::BLACK).palette()),
    }
}

fn lin(c: f32) -> f32 {
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Color) -> f32 {
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio of two opaque colours (1..21).
pub fn ratio(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// A light body (the text is darker than the body): the shell paints a light or a dark theme from this.
pub fn is_light(p: &Palette) -> bool {
    luminance(p.bg) > 0.4
}

/// The toolkit theme for the panel's controls under high contrast: flat, opaque, system colours; no translucent surface, no shadow.
pub fn theme(p: &Palette) -> Theme {
    let base = if is_light(p) { Theme::pill_light() } else { Theme::pill_dark() };
    let clear = Color::rgba(0.0, 0.0, 0.0, 0.0);
    Theme {
        bg: p.bg,
        card: p.bg,
        card_hover: p.bg,
        raised: p.bg,
        raised2: p.bg,
        field: p.bg,
        hover: p.accent,
        border: p.outline,
        line: p.outline,
        track: p.disabled,
        switch_off: p.disabled,
        btn: p.face,
        btn_hover: p.accent,
        seg_bg: p.bg,
        seg_on: p.accent,
        seg_hover_text: p.on_accent,
        text: p.text,
        text2: p.text,
        dim: p.text,
        faint: p.disabled,
        icon: p.text,
        on_accent: p.on_accent,
        accent: p.accent,
        accent_hover: p.accent,
        accent_soft: p.bg,
        slider_fill: p.accent,
        thumb: p.text,
        green: p.text,
        green_bg: p.bg,
        red: p.text,
        red_bg: p.bg,
        amber: p.text,
        amber_bg: p.bg,
        link: p.link,
        shadow: clear,
        tip_bg: p.bg,
        tip_text: p.text,
        scrollbar: p.text,
        scrollbar_hover: p.accent,
        menu_bg: p.bg,
        menu_hover: p.accent,
        ..base
    }
}

/// The `#rrggbb` the shell takes as the body colour (`a11y.hc.bg`); empty = no high contrast.
pub fn hex(c: Color) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

/// Windows' own answer: the system colours while a high-contrast theme is on, `None` otherwise.
pub fn read_system() -> Option<SysColors> {
    use windows::Win32::Graphics::Gdi::{GetSysColor, COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, SYS_COLOR_INDEX};
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    let mut hc = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    // SAFETY: `hc` is a valid HIGHCONTRASTW with cbSize set, as SPI_GETHIGHCONTRAST requires
    let ok = unsafe { SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, Some(&mut hc as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)) };
    if ok.is_err() || hc.dwFlags.0 & HCF_HIGHCONTRASTON.0 == 0 {
        return None;
    }
    let col = |i: SYS_COLOR_INDEX| {
        let v = unsafe { GetSysColor(i) }; // 0x00BBGGRR
        Color::rgb8((v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8)
    };
    Some(SysColors {
        window: col(COLOR_WINDOW),
        window_text: col(COLOR_WINDOWTEXT),
        highlight: col(COLOR_HIGHLIGHT),
        highlight_text: col(COLOR_HIGHLIGHTTEXT),
        gray_text: col(COLOR_GRAYTEXT),
        btn_face: col(COLOR_BTNFACE),
        btn_text: col(COLOR_BTNTEXT),
        hotlight: col(COLOR_HOTLIGHT),
    })
}

/// What `WM_SETTINGCHANGE` / `WM_SYSCOLORCHANGE` / `WM_THEMECHANGED` means for the island: the bus event that makes views re-read.
pub const EVENT: &str = "a11y:contrast";

/// Is this `WM_SETTINGCHANGE` possibly about the high-contrast switch? (`wParam` SPI_SETHIGHCONTRAST = 0x43, or the "HighContrast" string in
/// `lParam`, or the broadcast with wParam 0 that theme switches send.)
pub fn setting_change_matters(wparam: usize, text: Option<&str>) -> bool {
    wparam == 0x43 || wparam == 0 || text.is_some_and(|t| t.eq_ignore_ascii_case("HighContrast") || t.eq_ignore_ascii_case("ImmersiveColorSet"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_parses_and_defaults_to_auto() {
        assert_eq!((Mode::parse("on"), Mode::parse("off"), Mode::parse("auto"), Mode::parse("")), (Mode::On, Mode::Off, Mode::Auto, Mode::Auto));
    }

    #[test]
    fn the_override_decides_when_windows_is_or_is_not_in_high_contrast() {
        let w = Some(SysColors::WHITE);
        assert_eq!(pick(Mode::Auto, None), None, "normal Windows: the normal look");
        assert_eq!(pick(Mode::Auto, w).unwrap().bg, SysColors::WHITE.window, "Auto follows Windows");
        assert_eq!(pick(Mode::Off, w), None, "Off wins over Windows");
        assert_eq!(pick(Mode::On, None).unwrap().bg, SysColors::BLACK.window, "On without a Windows theme: the built-in black palette");
        assert_eq!(pick(Mode::On, w).unwrap().bg, SysColors::WHITE.window, "On with a Windows theme uses its colours");
    }

    #[test]
    fn text_and_controls_stand_out_in_the_stock_themes() {
        for s in [SysColors::BLACK, SysColors::WHITE] {
            let p = s.palette();
            assert!(ratio(p.text, p.bg) >= 7.0, "text on the body");
            assert!(ratio(p.on_accent, p.accent) >= 4.5, "text on a highlighted control");
            assert!(ratio(p.outline, p.bg) >= 7.0, "the 2 px outline");
            assert!(ratio(p.link, p.bg) >= 4.5, "links");
            let t = theme(&p);
            assert_eq!((t.text, t.bg, t.accent), (p.text, p.bg, p.accent));
            assert_eq!(t.shadow.a, 0.0, "no shadow");
            assert_eq!((t.card.a, t.raised.a, t.field.a, t.menu_bg.a), (1.0, 1.0, 1.0, 1.0), "no translucent glass");
        }
        assert!(!is_light(&SysColors::BLACK.palette()) && is_light(&SysColors::WHITE.palette()));
    }

    #[test]
    fn hex_and_setting_change() {
        assert_eq!(hex(Color::rgb8(0x1A, 0xEB, 0xFF)), "#1aebff");
        assert!(setting_change_matters(0x43, None));
        assert!(setting_change_matters(0, Some("HighContrast")));
        assert!(!setting_change_matters(0x2F, Some("Environment")));
    }
}
