//! The expanded island's chrome: the tab strip, the header buttons (dictation, pin, settings), the VPN chip and the area the tab view lives in.
//! Layout is in css px at 100 % with the origin at the panel's top-left (`#panel`); the caller scales the canvas by the island size.
use super::gpu::Gpu;
use super::rows::Meter;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Canvas;
use pill_ui_core::panel::a::tabs::standard_tabs;
use pill_ui_core::shell::types::View as ShellView;

pub const TAB_LABEL: Style = Style::new(12.0, 600);
const BADGE: Style = Style::new(11.0, 600);
const VPN: Style = Style::new(11.0, 600).track(0.22);
pub const PANEL_W: f32 = 680.0;
pub const HEAD_H: f32 = 60.0;
pub const VIEWS_BOTTOM: f32 = 11.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Dict,
    Pin,
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabBox {
    pub id: String,
    pub rect: Rect,
    pub icon: &'static str,
    /// width of the label (rounded up to whole px, `--tw`)
    pub tw: f32,
    /// 0..1: how far the tab is open (label visible)
    pub k: f32,
    pub badge: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChromeLayout {
    pub tabs: Vec<TabBox>,
    pub buttons: Vec<(Btn, Rect)>,
    pub vpn: Option<Rect>,
    /// the free gap of the header between the tabs and the buttons (where the pet stands while open)
    pub pet_slot: Rect,
    /// where the tab view is drawn
    pub views: Rect,
}

pub fn tab_icon(id: &str) -> &'static str {
    standard_tabs().iter().find(|t| t.id == id).map_or("home", |t| t.icon)
}

pub fn tab_title(id: &str, ru: bool) -> String {
    standard_tabs().iter().find(|t| t.id == id).map(|t| if ru { t.ru } else { t.en }.to_string()).unwrap_or_else(|| id.to_string())
}

/// `ks`: open fraction per tab id (missing = 0 / 1 by `active`). `panel_h` = the panel's height in css px at 100 %.
pub fn layout(v: &ShellView, ru: bool, panel_h: f32, ks: &dyn Fn(&str, bool) -> f32, m: &dyn Meter) -> ChromeLayout {
    let mut x = 16.0;
    let mut tabs = Vec::new();
    for t in v.tabs.iter().filter(|t| !t.hidden && !t.off) {
        let tw = (m.w(&tab_title(&t.id, ru), TAB_LABEL) + 0.5).ceil();
        let k = ks(&t.id, t.active).clamp(0.0, 1.0);
        let badge_w = if t.active && t.badge > 0 { 8.0 + (m.w(&t.badge.to_string(), BADGE) + 10.0).max(18.0) } else { 0.0 };
        let w = 38.0 + k * (10.0 + tw + badge_w);
        tabs.push(TabBox { id: t.id.clone(), rect: Rect::new(x, 14.0, w, 32.0), icon: tab_icon(&t.id), tw, k, badge: if t.active { t.badge } else { 0 } });
        x += w + 4.0;
    }
    let tabs_end = x - 4.0;
    let mut bx = PANEL_W - 16.0;
    let mut buttons = Vec::new();
    for b in [Btn::Settings, Btn::Pin, Btn::Dict] {
        bx -= 28.0;
        buttons.push((b, Rect::new(bx, 16.0, 28.0, 28.0)));
        bx -= 8.0;
    }
    buttons.reverse();
    let mut vpn = None;
    if !v.chrome.vpn.is_empty() {
        let w = 8.0 + 15.0 + 4.0 + m.w(&v.chrome.vpn, VPN) + 10.0;
        bx -= w - 8.0 + 4.0;
        vpn = Some(Rect::new(bx, 20.0, w, 24.0));
        bx -= 4.0;
    }
    let slot_l = tabs_end + 8.0;
    let pet_slot = Rect::new(slot_l, 14.0, (bx - slot_l).max(0.0), 32.0);
    ChromeLayout { tabs, buttons, vpn, pet_slot, views: Rect::new(16.0, HEAD_H, PANEL_W - 32.0, (panel_h - HEAD_H - VIEWS_BOTTOM).max(0.0)) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hover {
    pub tab: Option<usize>,
    pub btn: Option<Btn>,
    pub pressed: bool,
}

/// Paints the header. `hover` drives the hover fill; `t_ms` is the host clock.
pub fn paint(cv: &Canvas, th: &Theme, gpu: &mut Gpu, v: &ShellView, lay: &ChromeLayout, hover: Hover, ru: bool) {
    for (i, t) in lay.tabs.iter().enumerate() {
        let r = t.rect;
        let on = v.tabs.iter().any(|x| x.id == t.id && x.active);
        let hov = hover.tab == Some(i);
        let press = if hov && hover.pressed { 0.96 } else { 1.0 };
        cv.with_scale_about(r.center(), press, |cv| {
            if t.k > 0.0 {
                cv.fill_rr(r, 16.0, th.raised.fade(t.k));
            }
            if hov && t.k < 1.0 {
                cv.fill_rr(r, 16.0, th.hover.fade(1.0 - t.k));
            }
            let col = if on || hov { th.text } else { th.icon };
            gpu.icon(cv, t.icon, Rect::new(r.x + 10.0, r.cy() - 9.0, 18.0, 18.0), col, if on { 1.8 } else { 1.6 });
            if t.k > 0.01 {
                let lr = Rect::new(r.x + 10.0 + 18.0 + 8.0, r.cy() - 8.0, t.tw, 16.0);
                cv.with_clip(Rect::new(lr.x, r.y, (lr.w * t.k).max(0.0), r.h), |cv| cv.text(lr, &tab_title(&t.id, ru), TAB_LABEL, th.text.fade(t.k)));
                if t.badge > 0 {
                    let bw = (cv.text_width(&t.badge.to_string(), BADGE) + 10.0).max(18.0);
                    let br = Rect::new(lr.right() + 8.0, r.cy() - 9.0, bw, 18.0);
                    cv.fill_rr(br, 9.0, th.raised2.fade(t.k));
                    cv.text(br, &t.badge.to_string(), BADGE.center(), th.text.fade(t.k));
                }
            }
        });
    }
    if let Some(r) = lay.vpn {
        cv.fill_rr(r, 12.0, th.green_bg);
        gpu.icon(cv, "shield", Rect::new(r.x + 8.0, r.cy() - 7.5, 15.0, 15.0), th.green, 1.7);
        cv.text(Rect::new(r.x + 8.0 + 15.0 + 4.0, r.y, r.w, r.h), &v.chrome.vpn, VPN, th.green);
    }
    for (b, r) in &lay.buttons {
        let hov = hover.btn == Some(*b);
        let (icon, on, cur) = match b {
            Btn::Dict => ("wave", false, false),
            Btn::Pin => ("pin", v.chrome.pinned, false),
            Btn::Settings => ("settings", false, v.chrome.settings_current),
        };
        let press = if hov && hover.pressed { 0.96 } else { 1.0 };
        cv.with_scale_about(r.center(), press, |cv| {
            if on {
                cv.fill_circle(r.center(), 16.0, th.raised);
                cv.fill_circle(r.center(), 14.0, th.raised);
            } else if hov {
                cv.fill_circle(r.center(), 14.0, th.hover);
            }
            let col = if on || cur || hov { th.text } else if *b == Btn::Dict { th.icon } else { th.text2 };
            gpu.icon(cv, icon, Rect::new(r.cx() - 9.0, r.cy() - 9.0, 18.0, 18.0), col, 1.6);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::rows::Mono;
    use pill_ui_core::shell::types::{ChromeView, TabView};

    fn view(active: &str, vpn: &str) -> ShellView {
        // only the fields the layout reads
        let mut v = crate::ui::island::tests_support::blank_view();
        v.tabs = ["home", "clipboard", "shelf", "controls"].iter().enumerate().map(|(i, id)| TabView { id: id.to_string(), title: id.to_string(), order: i as i32, hidden: false, off: false, active: *id == active, badge: 0 }).collect();
        v.chrome = ChromeView { vpn: vpn.into(), pinned: false, settings_current: false };
        v
    }

    #[test]
    fn the_active_tab_is_wider_and_the_header_buttons_sit_at_the_right_edge() {
        let v = view("home", "");
        let lay = layout(&v, false, 338.0, &|_, active| if active { 1.0 } else { 0.0 }, &Mono(0.5));
        assert_eq!(lay.tabs.len(), 4);
        assert!(lay.tabs[0].rect.w > 38.0 + 10.0 && lay.tabs[1].rect.w == 38.0);
        assert_eq!(lay.tabs[0].rect.x, 16.0);
        assert!((lay.tabs[1].rect.x - (16.0 + lay.tabs[0].rect.w + 4.0)).abs() < 1e-3, "4 px between tabs");
        let (_, settings) = lay.buttons[2];
        assert_eq!(settings.right(), 680.0 - 16.0);
        assert_eq!((settings.w, settings.h), (28.0, 28.0));
        assert_eq!(lay.buttons[1].1.right() + 8.0, settings.x);
        assert_eq!(lay.views, Rect::new(16.0, 60.0, 648.0, 338.0 - 71.0));
    }

    #[test]
    fn the_vpn_chip_takes_room_from_the_pet_slot() {
        let without = layout(&view("home", ""), false, 338.0, &|_, a| if a { 1.0 } else { 0.0 }, &Mono(0.5));
        let with = layout(&view("home", "DE"), false, 338.0, &|_, a| if a { 1.0 } else { 0.0 }, &Mono(0.5));
        assert!(with.vpn.is_some() && with.pet_slot.w < without.pet_slot.w);
    }

    #[test]
    fn tab_titles_follow_the_language() {
        assert_eq!(tab_title("clipboard", true), "Буфер");
        assert_eq!(tab_title("clipboard", false), "Clipboard");
        assert_eq!(tab_icon("notifs"), "bell");
    }
}
