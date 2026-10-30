//! Colours of the settings window that `glasskit::theme::Theme` has no field for (sidebar, title bar, hairlines, chips...), derived from the
//! theme's own darkness. Dark = the exact `settings.css` custom properties; light = the same roles on the Pill light palette (the web
//! settings page had no light theme, so this is a native refinement; `Theme::settings_dark` stays the default).
use glasskit::color::Color;
use glasskit::theme::{Metrics, Theme};

const fn c(hex: u32) -> Color {
    Color::rgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    /// window background
    pub bg: Color,
    /// sidebar background
    pub side: Color,
    pub side_line: Color,
    pub title: Color,
    pub win_border: Color,
    pub card: Color,
    pub card_line: Color,
    pub card_line_hover: Color,
    pub row_line: Color,
    pub field: Color,
    /// segmented track, key group, chips
    pub ctl: Color,
    pub ctl_on: Color,
    pub ctl_hover: Color,
    pub text: Color,
    pub text2: Color,
    pub dim: Color,
    pub faint: Color,
    pub text_ctl: Color,
    pub accent: Color,
    pub accent_bg: Color,
    pub accent_line: Color,
    pub link: Color,
    pub amber: Color,
    pub red: Color,
    pub green: Color,
    pub cap: Color,
    pub item: Color,
    pub item_hover: Color,
    pub item_on: Color,
    pub icon: Color,
    pub key_cap: Color,
    pub key_cap_rec: Color,
    pub pcard: Color,
    pub pcard_hover: Color,
    pub pcard_on: Color,
    pub pcard_on_line: Color,
    pub pcard_on_ring: Color,
    pub placeholder: Color,
    pub shadow: Color,
}

pub fn pal(th: &Theme) -> Pal {
    if th.dark {
        Pal {
            dark: true,
            bg: c(0x111013),
            side: c(0x0c0b0e),
            side_line: c(0x1a191c),
            title: c(0x1b1f2c),
            win_border: c(0x3e3a43),
            card: th.card,
            card_line: c(0x26252a),
            card_line_hover: c(0x302f35),
            row_line: c(0x272629),
            field: th.field,
            ctl: c(0x2b2a2d),
            ctl_on: c(0x4b4a4d),
            ctl_hover: c(0x343337),
            text: th.text,
            text2: th.text2,
            dim: th.dim,
            faint: th.faint,
            text_ctl: c(0xa0a0a8),
            accent: th.accent,
            accent_bg: c(0x32354b),
            accent_line: c(0x76799d),
            link: th.link,
            amber: th.amber,
            red: th.red,
            green: th.green,
            cap: c(0x757579),
            item: c(0xe6e5e8),
            item_hover: c(0x171717),
            item_on: c(0x262626),
            icon: c(0xa9a7ae),
            key_cap: c(0x404040),
            key_cap_rec: c(0x3b3f66),
            pcard: c(0x1d1c1f),
            pcard_hover: c(0x201f22),
            pcard_on: c(0x1f2036),
            pcard_on_line: c(0x5964c8),
            pcard_on_ring: c(0x232b66),
            placeholder: c(0x6a686e),
            shadow: th.shadow,
        }
    } else {
        Pal {
            dark: false,
            bg: th.bg,
            side: c(0xececef),
            side_line: c(0xdcdce0),
            title: c(0xe3e5ee),
            win_border: c(0xc9c8ce),
            card: c(0xffffff),
            card_line: c(0xdcdbe0),
            card_line_hover: c(0xcfced5),
            row_line: c(0xe7e6ea),
            field: c(0xf1f0f3),
            ctl: c(0xe3e2e7),
            ctl_on: c(0xffffff),
            ctl_hover: c(0xd9d8de),
            text: th.text,
            text2: th.text2,
            dim: th.dim,
            faint: th.faint,
            text_ctl: c(0x5f5e66),
            accent: th.accent,
            accent_bg: c(0xe0e3fb),
            accent_line: c(0x8f96e8),
            link: th.link,
            amber: th.amber,
            red: th.red,
            green: th.green,
            cap: c(0x8a8990),
            item: c(0x2b2a30),
            item_hover: c(0xe2e1e6),
            item_on: c(0xd8d7de),
            icon: c(0x6c6b73),
            key_cap: c(0xd0cfd6),
            key_cap_rec: c(0xc9cdf5),
            pcard: c(0xf3f2f5),
            pcard_hover: c(0xeceaef),
            pcard_on: c(0xe6e8fb),
            pcard_on_line: c(0x6b73e0),
            pcard_on_ring: c(0xc8ccf6),
            placeholder: c(0x9a99a1),
            shadow: th.shadow,
        }
    }
}

/// The light variant of the settings theme: Pill light colours with the Settings metrics (44x24 switch, 8 px radii).
pub fn settings_light() -> Theme {
    let mut t = Theme::pill_light();
    t.m = Metrics::SETTINGS;
    t.bg = c(0xf6f5f8);
    t.card = c(0xffffff);
    t.field = c(0xf1f0f3);
    t.btn = c(0xe6e5ea);
    t.btn_hover = c(0xdbdae0);
    t.seg_bg = c(0xe3e2e7);
    t.seg_on = c(0xffffff);
    t.track = c(0xcfced6);
    t.switch_off = c(0xcfced6);
    t.menu_bg = c(0xffffff);
    t
}

pub fn mix(a: Color, b: Color, t: f32) -> Color {
    a.mix(b, t.clamp(0.0, 1.0))
}
