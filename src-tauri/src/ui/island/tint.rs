//! The island's own colour for the panel's controls: the shell paints the body in the chosen colour (`island.color`), this carries it into
//! the toolkit theme so cards, buttons, tracks and menus sit on that colour with the same relation they have to black or to light grey.
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::shell::color::Rgba;

fn lum(c: Color) -> f32 {
    0.299 * c.r + 0.587 * c.g + 0.114 * c.b
}

/// `base` (the dark or the light pill theme, picked by the colour's brightness) on the body colour `body`: every surface colour keeps its
/// distance from the old body colour towards the text colour, measured in brightness, and is rebuilt from the new one.
pub fn tinted(base: Theme, body: Rgba) -> Theme {
    let bg = Color::rgb8(body.r, body.g, body.b);
    let span = lum(base.text) - lum(base.bg);
    let lift = |x: Color| -> Color {
        if span.abs() < 1e-3 {
            return x;
        }
        let t = ((lum(x) - lum(base.bg)) / span).clamp(0.0, 1.0);
        bg.mix(base.text, t).alpha(x.a)
    };
    Theme {
        bg,
        card: lift(base.card),
        card_hover: lift(base.card_hover),
        raised: lift(base.raised),
        raised2: lift(base.raised2),
        field: lift(base.field),
        track: lift(base.track),
        switch_off: lift(base.switch_off),
        btn: lift(base.btn),
        btn_hover: lift(base.btn_hover),
        seg_bg: lift(base.seg_bg),
        seg_on: lift(base.seg_on),
        scrollbar: lift(base.scrollbar),
        scrollbar_hover: lift(base.scrollbar_hover),
        menu_bg: lift(base.menu_bg),
        menu_hover: lift(base.menu_hover),
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_follow_the_body_colour_and_text_stays() {
        let navy = Rgba::rgb(0x0b, 0x1b, 0x3d);
        let t = tinted(Theme::pill_dark(), navy);
        assert_eq!(t.bg, Color::rgb8(0x0b, 0x1b, 0x3d));
        assert_eq!(t.text, Theme::pill_dark().text);
        assert!(lum(t.card) > lum(t.bg) && lum(t.raised2) > lum(t.card), "cards lie lighter than the body, in the same order as on black");
        assert!((t.card.b - t.bg.b).abs() < 0.2, "and still look like the body colour, not like grey");
        let pearl = tinted(Theme::pill_light(), Rgba::rgb(0xf6, 0xf6, 0xf6));
        assert!((pearl.card.r - Theme::pill_light().card.r).abs() < 0.003, "pearl is the light theme's own grey: nothing moves");
        let sky = tinted(Theme::pill_light(), Rgba::rgb(0xbc, 0xd9, 0xf5));
        assert!(lum(sky.card) < lum(sky.bg), "on a light colour the cards are darker");
    }
}
