//! Drawing of the Controls tab: the colour tokens of `.tab-controls`, the tab's own line icons (stroke 1.8, not the `icons.js` set), tiles, power
//! tiles and the Bluetooth popover. Every function paints one element from plain values; interaction state lives in the view.
use super::layout::{self, RADIUS};
use crate::ui::tabs::a::icons;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::text::{Align, Style};
use glasskit::theme::Theme;
use pill_ui_core::panel::a::controls::{DeviceRow, PowerView, Popover, TileView};

/// `--tc-*` of `tab-controls.css` for the dark / light theme.
#[derive(Clone, Copy, Debug)]
pub struct Tok {
    pub on: Color,
    pub off: Color,
    pub hover: Color,
    pub circle_off: Color,
    pub accent: Color,
    pub accent_fg: Color,
    pub fg: Color,
    pub dim: Color,
    pub warn: Color,
    /// `--bg-raised` (popover) and `--border`
    pub raised: Color,
    pub border: Color,
}

fn hex(v: u32) -> Color {
    Color::rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Tok {
    pub fn of(th: &Theme) -> Tok {
        let (on, off, hover, circle_off, dim, accent_fg) = if th.dark {
            (hex(0x1f1f1f), hex(0x0f0f0f), hex(0x2a2a2a), hex(0x272727), Color::rgba(1.0, 1.0, 1.0, 0.46), hex(0x0b0b14))
        } else {
            (hex(0xe4e4ea), hex(0xf2f2f7), hex(0xd9d9e0), hex(0xd4d4da), Color::rgba(0.0, 0.0, 0.0, 0.5), hex(0x101026))
        };
        Tok { on, off, hover, circle_off, accent: hex(0x8ea2ff), accent_fg, fg: th.text, dim, warn: hex(0xff6b5e), raised: th.card, border: th.border }
    }
}

/// Inner SVG of each icon of `IC` in `tab-controls.js` (24 x 24, stroke 1.8).
pub fn icon_markup(name: &str) -> Option<(&'static str, &'static str)> {
    Some(match name {
        "wifi" => ("tc-wifi", r#"<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.8 16a5 5 0 0 1 6.4 0"/><circle cx="12" cy="19.3" r="0.8" fill="currentColor"/>"#),
        "eth" => ("tc-eth", r#"<rect x="3" y="4" width="18" height="12" rx="3"/><path d="M8 16v3h8v-3M8 8.5v3M12 8.5v3M16 8.5v3"/>"#),
        "bt" => ("tc-bt", r#"<path d="M7 7.5l10 9-5 4.5V3l5 4.5-10 9"/>"#),
        "btoff" => ("tc-btoff", r#"<path d="M7 7.5l10 9-5 4.5V12M12 3l5 4.5-3 2.7M4 4l16 16"/>"#),
        "theme" => ("tc-theme", r#"<circle cx="12" cy="12" r="9"/><path d="M12 3v18" /><path d="M12 7a5 5 0 0 1 0 10z" fill="currentColor"/>"#),
        "list" => ("tc-list", r#"<path d="M9 7h11M9 12h11M9 17h11"/><circle cx="5" cy="7" r="0.9" fill="currentColor"/><circle cx="5" cy="12" r="0.9" fill="currentColor"/><circle cx="5" cy="17" r="0.9" fill="currentColor"/>"#),
        "lock" => ("tc-lock", r#"<rect x="5" y="11" width="14" height="9" rx="2.5"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>"#),
        "sleep" => ("tc-sleep", r#"<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>"#),
        "restart" => ("tc-restart", r#"<path d="M20 12a8 8 0 1 1-2.6-5.9"/><path d="M20 4v4.5h-4.5"/>"#),
        "power" => ("tc-power", r#"<path d="M12 3v9"/><path d="M6.3 6.6a8 8 0 1 0 11.4 0"/>"#),
        _ => return None,
    })
}

pub fn icon(cv: &Canvas, name: &str, r: Rect, c: Color) {
    if let Some((key, inner)) = icon_markup(name) {
        icons::draw_markup(cv, key, inner, r, c, 1.8);
    }
}

/// What a tile looks like right now (all animated amounts already evaluated).
pub struct TileLook {
    /// 0..1 eased "lit" amount (on-state colours)
    pub on: f32,
    /// 0..1 eased hover amount
    pub hover: f32,
    /// press scale of the whole tile (`.96`) and of the icon circle (`.92`)
    pub scale: f32,
    pub circle_scale: f32,
    /// opacity of the icon circle (the busy pulse)
    pub circle_alpha: f32,
    pub list_hover: f32,
    pub ring: f32,
}

pub fn tile(cv: &Canvas, th: &Theme, k: &Tok, r: Rect, t: &TileView, l: &TileLook) {
    let dim_tile = t.disabled && !t.busy;
    let draw = |cv: &Canvas| {
        cv.with_scale_about(r.center(), l.scale, |cv| {
            let base = k.off.mix(k.on, l.on);
            let hov_to = k.hover.mix(k.on.mix(k.fg, 0.12), l.on);
            cv.fill_rr(r, RADIUS, base.mix(hov_to, l.hover));
            let c = layout::icon_circle(r);
            cv.with_opacity(l.circle_alpha, |cv| {
                cv.with_scale_about(c.center(), l.circle_scale, |cv| {
                    cv.fill_circle(c.center(), c.w / 2.0, k.circle_off.mix(k.accent, l.on));
                    icon(cv, t.icon, c.centered(20.0, 20.0), k.dim.mix(k.accent_fg, l.on));
                });
            });
            let (nm, st) = layout::tile_text(r);
            cv.text(nm, &t.name, Style::new(14.0, 600).ellipsis(), k.fg);
            cv.text(st, &t.state, Style::new(12.0, 400).ellipsis(), k.dim);
            if t.list_button.is_some() && !t.disabled {
                let lb = layout::list_button(r);
                cv.fill_circle(lb.center(), 12.0, k.circle_off.mix(k.hover, l.list_hover));
                icon(cv, "list", lb.centered(14.0, 14.0), k.dim.mix(k.fg, l.list_hover));
            }
        });
        cv.focus_ring(r, RADIUS, th.accent, l.ring);
    };
    if dim_tile {
        cv.with_opacity(0.45, draw);
    } else {
        draw(cv);
    }
}

pub struct PowerLook {
    pub hover: f32,
    pub scale: f32,
    /// opacity of the red wash (the draining arm countdown); 0 when no cue runs
    pub wash: f32,
    pub ring: f32,
}

pub fn power(cv: &Canvas, th: &Theme, k: &Tok, r: Rect, p: &PowerView, l: &PowerLook) {
    cv.with_scale_about(r.center(), l.scale, |cv| {
        cv.fill_rr(r, RADIUS, k.off.mix(k.hover, l.hover));
        if l.wash > 0.004 {
            cv.fill_rr(r, RADIUS, k.warn.alpha(l.wash));
        }
        let (ic, lb) = layout::power_parts(r);
        let grow = 1.0 + 0.08 * l.hover;
        let icol = if p.armed { k.warn } else { k.dim.mix(k.fg, l.hover) };
        icon(cv, p.icon, ic.centered(21.0 * grow, 21.0 * grow), icol);
        cv.text(lb, &p.label, Style::new(13.0, 600).center(), k.fg);
    });
    cv.focus_ring(r, RADIUS, th.accent, l.ring);
}

/// The popover at `pop` (its placement already includes the enter / exit offset); `list_scroll` is the scrolled-by amount of the device list.
pub struct PopLook {
    pub button_hover: f32,
    pub button_scale: f32,
    pub scroll: f32,
}

pub fn popover(cv: &Canvas, k: &Tok, pop: &layout::Pop, p: &Popover, l: &PopLook) {
    let o = pop.outer;
    cv.shadow_rr(o, RADIUS, 30.0, 10.0, Color::rgba(0.0, 0.0, 0.0, 0.55));
    cv.fill_rr(o, RADIUS, k.raised);
    cv.stroke_rr(o, RADIUS, 1.0, k.border);
    let h = Rect::new(pop.header.x + 8.0, pop.header.y + 2.0, pop.header.w - 16.0, 16.0);
    cv.text(h, &p.title, Style::new(12.0, 600).ellipsis(), k.dim);
    cv.with_clip(pop.list, |cv| {
        if let Some(e) = &p.empty {
            cv.text(Rect::new(pop.list.x + 8.0, pop.list.y + 16.0, pop.list.w - 16.0, 16.0), e, Style::new(12.0, 400).center(), k.dim);
        } else {
            for (i, row) in p.rows.iter().enumerate() {
                let y = pop.list.y + i as f32 * (layout::ROW_H + layout::ROW_GAP) - l.scroll;
                device_row(cv, k, Rect::new(pop.list.x, y, pop.list.w, layout::ROW_H), row);
            }
        }
    });
    let b = pop.button;
    cv.with_scale_about(b.center(), l.button_scale, |cv| {
        cv.fill_rr(b, 10.0, k.on.mix(k.hover, l.button_hover));
        cv.text(b, &p.more, Style::new(12.0, 600).center().ellipsis(), k.fg);
    });
}

fn device_row(cv: &Canvas, k: &Tok, r: Rect, d: &DeviceRow) {
    cv.fill_circle((r.x + 8.0 + 4.0, r.cy()), 4.0, if d.connected { k.accent } else { k.circle_off });
    let right = r.right() - 8.0;
    let sw = if d.detail.is_empty() { 0.0 } else { cv.text_width(&d.detail, Style::new(11.0, 400)) + 8.0 };
    cv.text(Rect::new(r.x + 8.0 + 8.0 + 8.0, r.y, (right - sw - (r.x + 24.0)).max(0.0), r.h), &d.name, Style::new(13.0, 400).ellipsis(), k.fg);
    if sw > 0.0 {
        let mut st = Style::new(11.0, 400);
        st.align = Align::Right;
        cv.text(Rect::new(right - sw, r.y, sw, r.h), &d.detail, st, k.dim);
    }
}

