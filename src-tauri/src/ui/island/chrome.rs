//! The expanded island's chrome: the tab strip, the header buttons (dictation, pin, settings), the VPN chip and the area the tab view lives in.
//! Layout is in css px at 100 % with the origin at the panel's top-left (`#panel`); the caller scales the canvas by the island size.
use super::gpu::Gpu;
use super::rows::Meter;
use crate::ui::liquid::{Feel, Flow};
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Canvas;
use pill_ui_core::panel::a::tabs::standard_tabs;
use pill_ui_core::shell::types::{TabView, View as ShellView};
use std::collections::HashMap;

pub const TAB_LABEL: Style = Style::new(12.0, 600);
const BADGE: Style = Style::new(11.0, 600);
const VPN: Style = Style::new(11.0, 600).track(0.22);
pub const PANEL_W: f32 = 680.0;
pub const HEAD_H: f32 = 60.0;
pub const VIEWS_BOTTOM: f32 = 11.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// the count shown in the badge and how present the badge is (0..1, a little over while it pops)
    pub badge: u32,
    pub bk: f32,
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
    let bk = |t: &TabView| (if t.active && t.badge > 0 { 1.0 } else { 0.0 }, t.badge);
    layout_with(v, ru, panel_h, ks, &bk, (&v.chrome.vpn, if v.chrome.vpn.is_empty() { 0.0 } else { 1.0 }), m)
}

/// [`layout`] with the eased parts: `bk` = (presence, count) of a tab's badge, `vpn` = (text, presence) of the VPN chip.
pub fn layout_with(v: &ShellView, ru: bool, panel_h: f32, ks: &dyn Fn(&str, bool) -> f32, bk: &dyn Fn(&TabView) -> (f32, u32), vpn: (&str, f32), m: &dyn Meter) -> ChromeLayout {
    let mut x = 16.0;
    let mut tabs = Vec::new();
    for t in v.tabs.iter().filter(|t| !t.hidden && !t.off) {
        let tw = (m.w(&tab_title(&t.id, ru), TAB_LABEL) + 0.5).ceil();
        let k = ks(&t.id, t.active).clamp(0.0, 1.0);
        let (bp, count) = bk(t);
        let badge_w = if count > 0 { (8.0 + (m.w(&count.to_string(), BADGE) + 10.0).max(18.0)) * bp.max(0.0) } else { 0.0 };
        let w = 38.0 + k * (10.0 + tw + badge_w);
        tabs.push(TabBox { id: t.id.clone(), rect: Rect::new(x, 14.0, w, 32.0), icon: tab_icon(&t.id), tw, k, badge: count, bk: bp });
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
    let mut vpn_rect = None;
    if !vpn.0.is_empty() && vpn.1 > 0.0 {
        let w = 8.0 + 15.0 + 4.0 + m.w(vpn.0, VPN) + 10.0;
        bx -= (w - 8.0 + 4.0) * vpn.1;
        vpn_rect = Some(Rect::new(bx, 20.0, w, 24.0));
        bx -= 4.0 * vpn.1;
    }
    let slot_l = tabs_end + 8.0;
    let pet_slot = Rect::new(slot_l, 14.0, (bx - slot_l).max(0.0), 32.0);
    ChromeLayout { tabs, buttons, vpn: vpn_rect, pet_slot, views: Rect::new(16.0, HEAD_H, PANEL_W - 32.0, (panel_h - HEAD_H - VIEWS_BOTTOM).max(0.0)) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hover {
    pub tab: Option<usize>,
    pub btn: Option<Btn>,
    pub pressed: bool,
}

/// The header's own liquid state: how far each tab is open, the hover / press amounts of the tabs and buttons, the pressed look of the pin and
/// the settings button, the badge counts and the VPN chip appearing. Stepped by the view; `moving()` says whether frames are still needed.
pub struct HeaderFx {
    tabs: HashMap<String, TabFlow>,
    btns: [(Flow, Flow); 3],
    pin: Flow,
    cur: Flow,
    vpn: Flow,
    vpn_text: String,
    busy: bool,
}

struct TabFlow {
    open: Flow,
    hover: Flow,
    press: Flow,
    badge: Flow,
    count: u32,
}

const HOVER: Feel = Feel::FADE.within(130.0);

fn btn_ix(b: Btn) -> usize {
    match b {
        Btn::Dict => 0,
        Btn::Pin => 1,
        Btn::Settings => 2,
    }
}

impl Default for HeaderFx {
    fn default() -> Self {
        HeaderFx::new()
    }
}

impl HeaderFx {
    pub fn new() -> HeaderFx {
        let pair = || (Flow::new(0.0, HOVER), Flow::new(0.0, Feel::SNAP));
        HeaderFx { tabs: HashMap::new(), btns: [pair(), pair(), pair()], pin: Flow::new(0.0, HOVER), cur: Flow::new(0.0, HOVER), vpn: Flow::new(0.0, Feel::POP), vpn_text: String::new(), busy: false }
    }

    pub fn moving(&self) -> bool {
        self.busy
    }

    /// One frame. `hover_tab` = id of the tab under the pointer, `pressed` = the mouse button is down on it.
    pub fn step(&mut self, v: &ShellView, hover_tab: Option<&str>, hover_btn: Option<Btn>, pressed: bool, dt_ms: f64) -> bool {
        for t in &v.tabs {
            let open = if t.active { 1.0 } else { 0.0 };
            let (bt, count) = (if t.active && t.badge > 0 { 1.0 } else { 0.0 }, t.badge);
            // a tab seen for the first time starts at rest, not from nothing
            let e = self.tabs.entry(t.id.clone()).or_insert_with(|| TabFlow {
                open: Flow::new(open, Feel::FADE.within(220.0)),
                hover: Flow::new(0.0, HOVER),
                press: Flow::new(0.0, Feel::SNAP),
                badge: Flow::new(bt, Feel::POP),
                count,
            });
            let h = hover_tab == Some(t.id.as_str());
            e.open.to(open);
            e.hover.to(if h { 1.0 } else { 0.0 });
            e.press.to(if h && pressed { 1.0 } else { 0.0 });
            if count > 0 {
                e.count = count; // the badge keeps its last number while it fades out
            }
            e.badge.to(bt);
        }
        for (i, b) in [Btn::Dict, Btn::Pin, Btn::Settings].into_iter().enumerate() {
            let h = hover_btn == Some(b);
            self.btns[i].0.to(if h { 1.0 } else { 0.0 });
            self.btns[i].1.to(if h && pressed { 1.0 } else { 0.0 });
        }
        self.pin.to(if v.chrome.pinned { 1.0 } else { 0.0 });
        self.cur.to(if v.chrome.settings_current { 1.0 } else { 0.0 });
        if !v.chrome.vpn.is_empty() {
            self.vpn_text = v.chrome.vpn.clone();
        }
        self.vpn.to(if v.chrome.vpn.is_empty() { 0.0 } else { 1.0 });
        let mut busy = false;
        let mut each = |f: &mut Flow| {
            f.step(dt_ms);
            busy |= f.moving();
        };
        for e in self.tabs.values_mut() {
            each(&mut e.open);
            each(&mut e.hover);
            each(&mut e.press);
            each(&mut e.badge);
        }
        for (a, b) in self.btns.iter_mut() {
            each(a);
            each(b);
        }
        each(&mut self.pin);
        each(&mut self.cur);
        each(&mut self.vpn);
        self.busy = busy;
        busy
    }

    /// How far the tab `id` is open (0..1); `active` is the fallback before the first step.
    pub fn open(&self, id: &str, active: bool) -> f32 {
        self.tabs.get(id).map_or(if active { 1.0 } else { 0.0 }, |e| e.open.at01())
    }

    #[cfg(test)]
    pub fn hover_of(&self, id: &str) -> f32 {
        self.tab(id).0
    }

    fn tab(&self, id: &str) -> (f32, f32) {
        self.tabs.get(id).map_or((0.0, 0.0), |e| (e.hover.at01(), e.press.at01()))
    }

    /// The layout of the header with every eased part applied.
    pub fn layout(&self, v: &ShellView, ru: bool, panel_h: f32, m: &dyn Meter) -> ChromeLayout {
        let bk = |t: &TabView| self.tabs.get(&t.id).map_or((if t.active && t.badge > 0 { 1.0 } else { 0.0 }, t.badge), |e| (e.badge.at(), e.count));
        layout_with(v, ru, panel_h, &|id, active| self.open(id, active), &bk, (&self.vpn_text, self.vpn.at().max(0.0)), m)
    }
}

/// Paints the header; the hover / press / open amounts come from `fx`.
pub fn paint(cv: &Canvas, th: &Theme, gpu: &mut Gpu, lay: &ChromeLayout, fx: &HeaderFx, ru: bool) {
    for t in lay.tabs.iter() {
        let r = t.rect;
        let (hov, press) = fx.tab(&t.id);
        let lit = t.k.max(hov);
        cv.with_scale_about(r.center(), 1.0 - 0.04 * press, |cv| {
            if t.k > 0.0 {
                cv.fill_rr(r, 16.0, th.raised.fade(t.k));
            }
            if hov > 0.0 && t.k < 1.0 {
                cv.fill_rr(r, 16.0, th.hover.fade(hov * (1.0 - t.k)));
            }
            let col = th.icon.mix(th.text, lit);
            gpu.icon(cv, t.icon, Rect::new(r.x + 10.0, r.cy() - 9.0, 18.0, 18.0), col, 1.6 + (2.0 * t.k).round() / 10.0);
            if t.k > 0.01 {
                let lr = Rect::new(r.x + 10.0 + 18.0 + 8.0, r.cy() - 8.0, t.tw, 16.0);
                cv.with_clip(Rect::new(lr.x, r.y, (lr.w * t.k).max(0.0), r.h), |cv| cv.text(lr, &tab_title(&t.id, ru), TAB_LABEL, th.text.fade(t.k)));
                if t.badge > 0 && t.bk > 0.01 {
                    let bw = (cv.text_width(&t.badge.to_string(), BADGE) + 10.0).max(18.0);
                    let br = Rect::new(lr.right() + 8.0, r.cy() - 9.0, bw, 18.0);
                    let a = t.k * t.bk.clamp(0.0, 1.0);
                    // the badge pops: it grows with its presence, which swings a little past 1
                    cv.with_scale_about(br.center(), (0.7 + 0.3 * t.bk).max(0.0), |cv| {
                        cv.fill_rr(br, 9.0, th.raised2.fade(a));
                        cv.text(br, &t.badge.to_string(), BADGE.center(), th.text.fade(a));
                    });
                }
            }
        });
    }
    if let Some(r) = lay.vpn {
        let k = fx.vpn.at();
        let a = k.clamp(0.0, 1.0);
        cv.with_scale_about(r.center(), (0.8 + 0.2 * k).max(0.0), |cv| {
            cv.fill_rr(r, 12.0, th.green_bg.fade(a));
            gpu.icon(cv, "shield", Rect::new(r.x + 8.0, r.cy() - 7.5, 15.0, 15.0), th.green.fade(a), 1.7);
            cv.text(Rect::new(r.x + 8.0 + 15.0 + 4.0, r.y, r.w, r.h), &fx.vpn_text, VPN, th.green.fade(a));
        });
    }
    for (b, r) in &lay.buttons {
        let (icon, on) = match b {
            Btn::Dict => ("wave", 0.0),
            Btn::Pin => ("pin", fx.pin.at01()),
            Btn::Settings => ("settings", fx.cur.at01()),
        };
        let (hov, press) = (fx.btns[btn_ix(*b)].0.at01(), fx.btns[btn_ix(*b)].1.at01());
        cv.with_scale_about(r.center(), 1.0 - 0.04 * press, |cv| {
            if *b == Btn::Pin && on > 0.0 {
                cv.fill_circle(r.center(), 16.0, th.raised.fade(on));
                cv.fill_circle(r.center(), 14.0, th.raised.fade(on));
            }
            if hov > 0.0 {
                cv.fill_circle(r.center(), 14.0, th.hover.fade(hov * (1.0 - if *b == Btn::Pin { on } else { 0.0 })));
            }
            let rest = if *b == Btn::Dict { th.icon } else { th.text2 };
            gpu.icon(cv, icon, Rect::new(r.cx() - 9.0, r.cy() - 9.0, 18.0, 18.0), rest.mix(th.text, on.max(hov)), 1.6);
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
    fn hover_and_press_flow_in_and_out_and_the_header_settles() {
        let _m = crate::ui::host::testing::motion_on();
        let v = view("home", "");
        let mut fx = HeaderFx::new();
        assert!(!fx.step(&v, None, None, false, 8.0), "a header at rest asks for nothing");
        let mut seen = Vec::new();
        for _ in 0..60 {
            fx.step(&v, Some("shelf"), None, false, 8.0);
            seen.push(fx.tab("shelf").0);
        }
        assert!(seen[0] > 0.0 && seen[0] < 0.1, "it starts softly: {}", seen[0]);
        assert!(seen.windows(2).all(|w| w[1] >= w[0]) && *seen.last().unwrap() == 1.0, "{seen:?}");
        assert!(!fx.moving(), "settled");
        // pressed: a quick spring of its own; leaving takes the hover back out the same way
        for _ in 0..40 {
            fx.step(&v, Some("shelf"), None, true, 8.0);
        }
        assert!((fx.tab("shelf").1 - 1.0).abs() < 0.05);
        let mid = fx.tab("shelf").0;
        fx.step(&v, None, None, false, 8.0);
        assert!(fx.tab("shelf").0 <= mid && fx.moving());
        for _ in 0..80 {
            fx.step(&v, None, None, false, 8.0);
        }
        assert_eq!(fx.tab("shelf"), (0.0, 0.0));
        assert!(!fx.moving());
    }

    #[test]
    fn the_vpn_chip_and_a_badge_pop_in_and_the_pet_slot_follows_smoothly() {
        let _m = crate::ui::host::testing::motion_on();
        let mut v = view("home", "");
        let mut fx = HeaderFx::new();
        fx.step(&v, None, None, false, 8.0);
        let w0 = fx.layout(&v, false, 338.0, &Mono(0.5)).pet_slot.w;
        v.chrome.vpn = "DE".into();
        let mut widths = Vec::new();
        for _ in 0..60 {
            fx.step(&v, None, None, false, 8.0);
            widths.push(fx.layout(&v, false, 338.0, &Mono(0.5)).pet_slot.w);
        }
        let end = *widths.last().unwrap();
        assert!(end < w0 && widths[0] > end, "the slot shrinks as the chip arrives, not at once: {widths:?}");
        assert!(widths[0] - widths[1] < (w0 - end) * 0.5, "no jump on the first frame");
        v.chrome.vpn = String::new();
        for _ in 0..80 {
            fx.step(&v, None, None, false, 8.0);
        }
        assert!((fx.layout(&v, false, 338.0, &Mono(0.5)).pet_slot.w - w0).abs() < 0.01, "back where it was");
        // a badge on the active tab grows the tab with a pop
        v.tabs[0].badge = 3;
        let base = fx.layout(&v, false, 338.0, &Mono(0.5)).tabs[0].rect.w;
        let mut grown = Vec::new();
        for _ in 0..80 {
            fx.step(&v, None, None, false, 8.0);
            grown.push(fx.layout(&v, false, 338.0, &Mono(0.5)).tabs[0].rect.w);
        }
        assert!(grown[0] - base < 4.0 && *grown.last().unwrap() > base + 10.0, "{base} -> {grown:?}");
        assert!(grown.iter().cloned().fold(0.0, f32::max) > *grown.last().unwrap(), "it overshoots a little before it rests");
    }

    #[test]
    fn tab_titles_follow_the_language() {
        assert_eq!(tab_title("clipboard", true), "Буфер");
        assert_eq!(tab_title("clipboard", false), "Clipboard");
        assert_eq!(tab_icon("notifs"), "bell");
    }
}
