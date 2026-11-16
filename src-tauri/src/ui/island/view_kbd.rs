//! Keyboard mode, UI Automation controls and high contrast of the island (a child module of `view`, so it reaches the view's own fields).
//! The pure rules are in `ui/a11y` (focus order, key routing, the glide of the ring, the contrast palette); this file is the glue:
//!  * `Win+Alt+J` (`hotkey.keys`, `islandkeys.rs`) opens the card and emits `a11y:keys { target: "island" }`: the view asks the shell for the
//!    keyboard the way a text field does (`TextFocus(true)` -> `pill_activate` -> `Cx::set_focus`: the window takes the foreground, remembering
//!    the user's window), and the focus ring appears on the selected tab;
//!  * Tab / Shift+Tab / the arrows move the ring, Enter and Space act on the control it stands on, Esc closes the card and gives the keyboard
//!    back, Alt+1..9 jump to the N-th tab, Left / Right on a tab switch tabs. Keys that are not navigation (typing, media keys) are left alone;
//!  * a click, or a tab asking for the keyboard for a text field, ends keyboard mode without touching the focus;
//!  * `controls()` lists the strip's tabs, the header buttons, the VPN chip, the card's and the row's buttons and the open tab's controls from the
//!    same rectangles `locate` hit-tests, for UI Automation and for the ring; `control_act` clicks them.
use super::*;
use crate::ui::a11y::contrast::{self, Mode};
use crate::ui::a11y::nav::{self, Act, Control, NavOut, Role, Value as CV};
use crate::ui::a11y::ring::FocusRing;
use crate::ui::a11y::target::{self, Target};
use glasskit::input::Modifiers;

/// Control ids of the island window (the open tab's controls are `VIEW0 + their own id`).
const TAB0: u32 = 100;
const BTN0: u32 = 200;
const CARD0: u32 = 300;
const ROW0: u32 = 400;
const CHIP: u32 = 500;
const VIEW0: u32 = 1000;
/// A fresh keyboard mode waits this long for the card to open and lay out before it gives up.
const OPEN_WAIT_MS: f64 = 1800.0;

pub(super) struct Kbd {
    pub on: bool,
    pub focus: Option<u32>,
    pub ring: FocusRing,
    /// the card is still opening: the ring stands on the selected tab as soon as the strip exists
    first: bool,
    since: f64,
    /// `TextFocus(true)` raised by keyboard mode itself (not a tab's text field)
    own: bool,
    /// a click made by the keyboard (Enter on a control): it must not end keyboard mode
    synth: bool,
    mode: Mode,
    /// the high-contrast palette the island paints with now
    pub hc: Option<contrast::Palette>,
    hc_sent: String,
}

impl Kbd {
    pub fn new() -> Kbd {
        Kbd { on: false, focus: None, ring: FocusRing::new(), first: false, since: 0.0, own: false, synth: false, mode: Mode::Auto, hc: None, hc_sent: String::new() }
    }
}

fn btn_name(b: Btn, ru: bool) -> &'static str {
    match (b, ru) {
        (Btn::Dict, true) => "Диктовка",
        (Btn::Dict, false) => "Dictation",
        (Btn::Pin, true) => "Закрепить остров",
        (Btn::Pin, false) => "Pin the island",
        (Btn::Settings, true) => "Настройки",
        (Btn::Settings, false) => "Settings",
        (Btn::Vpn, _) => "VPN",
    }
}

impl IslandView {
    // ------------------------------------------------------------------------------------------------ controls (UIA and the ring)

    /// Everything a keyboard or a screen reader can reach, in window logical px.
    pub(super) fn kbd_controls(&self) -> Vec<Control> {
        let v = self.core.shell.view();
        let ru = self.core.ru();
        let mut out = Vec::new();
        if self.panel_a > 0.05 {
            let (ox, oy, s) = self.panel_origin;
            let win = |r: Rect| Rect::new(ox + r.x * s, oy + r.y * s, r.w * s, r.h * s);
            for (i, t) in self.chrome.tabs.iter().enumerate().filter(|(_, t)| t.pres >= 0.5) {
                let selected = v.current_tab.as_deref() == Some(t.id.as_str());
                out.push(Control::new(TAB0 + i as u32, chrome::tab_title(&t.id, ru), Role::Tab, win(t.rect)).with(CV::Selected(selected)));
            }
            for (b, r) in &self.chrome.buttons {
                let c = Control::new(BTN0 + *b as u32, btn_name(*b, ru), Role::Button, win(*r));
                out.push(if *b == Btn::Pin { c.with(CV::Toggled(v.chrome.pinned)) } else { c });
            }
            if let Some(r) = self.chrome.vpn {
                out.push(Control::new(BTN0 + Btn::Vpn as u32, btn_name(Btn::Vpn, ru), Role::Button, win(r)));
            }
            let vw = self.chrome.views;
            for c in self.host.controls() {
                let r = Rect::new(vw.x + c.rect.x, vw.y + c.rect.y, c.rect.w, c.rect.h);
                out.push(Control { id: VIEW0 + c.id, rect: win(r), ..c });
            }
        }
        // the live strip: a card's buttons, a row's buttons, the desktop chip (their rects are local css px of the body)
        let (sc, b) = (v.scale as f32, self.win_body(&v));
        let live = |r: Rect| Rect::new(b.x + r.x * sc, b.y + r.y * sc, r.w * sc, r.h * sc);
        let card = self.core.shell.cards().current_id().and_then(|id| self.core.cards.get(id));
        for (i, r) in self.painted.card_buttons.iter().enumerate() {
            let name = card.and_then(|c| c.actions.get(i)).map(|a| a.label.clone()).unwrap_or_default();
            out.push(Control::new(CARD0 + i as u32, name, Role::Button, live(*r)));
        }
        if let Some(row) = self.painted.row_id.as_ref().and_then(|id| self.core.rows.get(id)) {
            for (r, idx) in &self.painted.row_hits {
                let name = match row.elements.get(*idx) {
                    Some(pill_ui_core::live::model::Elem::Button { label, tip, .. }) => label.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| tip.clone()),
                    _ => String::new(),
                };
                out.push(Control::new(ROW0 + *idx as u32, name, Role::Button, live(*r)));
            }
        }
        if let Some(r) = self.painted.chip {
            out.push(Control::new(CHIP, if ru { "Рабочий стол" } else { "Virtual desktop" }, Role::Button, live(r)));
        }
        out
    }

    /// Click control `id` the way the pointer would: a press and a release at its centre, through the view's own `event` (so a tab, a header
    /// button and a card button all run the code their mouse path runs).
    fn kbd_click(&mut self, id: u32, cx: &mut Cx) -> bool {
        let Some(c) = self.kbd_controls().into_iter().find(|c| c.id == id) else { return false };
        self.click_at(c.rect.center(), cx);
        true
    }

    fn click_at(&mut self, p: (f32, f32), cx: &mut Cx) {
        let was = std::mem::replace(&mut self.kbd.synth, true);
        self.event(&InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }, cx);
        self.event(&InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE }, cx);
        self.kbd.synth = was;
    }

    /// A screen reader (or the keyboard) acts on a control.
    pub(super) fn kbd_act(&mut self, id: u32, act: Act, cx: &mut Cx) -> bool {
        match act {
            Act::Invoke | Act::Toggle | Act::Select => {
                self.kbd.focus = self.kbd.on.then_some(id).or(self.kbd.focus);
                self.kbd_click(id, cx)
            }
            Act::Set(_) | Act::Step(_) => {
                // a slider of the open tab: the pointer sets it by a click at the fraction of its track, so the keyboard and a screen reader do the same
                let Some(c) = self.kbd_controls().into_iter().find(|c| c.id == id) else { return false };
                let CV::Range { min, max, cur, step } = c.value else { return false };
                if max <= min {
                    return false;
                }
                let to = match act {
                    Act::Set(v) => v,
                    Act::Step(n) => cur + n as f64 * step,
                    _ => cur,
                }
                .clamp(min, max);
                let f = ((to - min) / (max - min)) as f32;
                self.click_at((c.rect.x + (f * c.rect.w).clamp(0.5, c.rect.w - 0.5), c.rect.cy()), cx);
                true
            }
        }
    }

    // ------------------------------------------------------------------------------------------------ the mode

    /// The hotkey / a bus event: take (`Island`) or drop the keys.
    pub(super) fn kbd_event(&mut self, name: &str, payload: &Value, cx: &mut Cx) {
        match name {
            target::EVENT => {
                if payload.get("target").and_then(Value::as_str).and_then(Target::parse) == Some(Target::Island) {
                    self.kbd_enter(cx);
                } else if self.kbd.on {
                    self.kbd_leave(cx, false);
                }
            }
            contrast::EVENT | "settings:changed" => self.contrast_refresh(cx),
            _ => {}
        }
    }

    fn kbd_enter(&mut self, cx: &mut Cx) {
        let now = cx.now_ms;
        self.kbd.on = true;
        self.kbd.focus = None;
        self.kbd.first = true;
        self.kbd.since = now;
        target::set_active(Some(Target::Island));
        // the keyboard, the way a text field asks for it: the window takes the foreground (remembering the user's window) and the card stays
        // open while the cursor is away
        self.kbd.own = true;
        self.core.shell_event(Event::TextFocus(true), now);
        self.exec(cx);
        self.kbd.own = false;
        self.wake_full_rate(cx);
        cx.request_frame();
    }

    /// Esc / the hotkey again: leave keyboard mode, give the keyboard back (the user's window gets the foreground), and with `close` fold the card.
    fn kbd_leave(&mut self, cx: &mut Cx, close: bool) {
        self.kbd_stop();
        let now = cx.now_ms;
        self.core.shell_event(Event::TextFocus(false), now);
        if close {
            self.core.shell_event(Event::Escape, now);
        }
        self.exec(cx);
        cx.request_frame();
    }

    /// Keyboard mode ends without touching the focus (the user clicked, or a tab took the keyboard for a text field).
    fn kbd_stop(&mut self) {
        self.kbd.on = false;
        self.kbd.focus = None;
        self.kbd.first = false;
        target::release(Target::Island);
    }

    /// `pill_activate` from the shell: a tab or the phone card wants the keyboard for typing - keyboard mode steps aside.
    pub(super) fn kbd_activate_hint(&mut self, on: bool) {
        if on && self.kbd.on && !self.kbd.own {
            self.kbd_stop();
        }
    }

    /// Input before the normal handling: navigation keys while keyboard mode is on, and the click that ends it.
    pub(super) fn kbd_input(&mut self, ev: &InputEvent, cx: &mut Cx) -> Option<Response> {
        if !self.kbd.on {
            return None;
        }
        match ev {
            InputEvent::MouseDown { .. } if !self.kbd.synth => {
                self.kbd_stop(); // the user took the mouse: the ring fades, the keyboard stays where it is
                None
            }
            InputEvent::Focus(false) => {
                self.kbd_stop();
                None
            }
            InputEvent::Key { code, mods, down: true, .. } if !self.host.overlay_open => {
                let key = nav::nav_key(*code, *mods)?;
                let cs = self.kbd_controls();
                // Escape while a text control of the open tab is being edited belongs to it (the tab view answers `consumed` in `host.event`)
                Some(match nav::route(&cs, self.kbd.focus, key) {
                    NavOut::Focus(id) => {
                        self.kbd.focus = Some(id);
                        Response::consumed().merge(Response::redraw())
                    }
                    NavOut::Act(id, act) => {
                        self.kbd.focus = Some(id);
                        self.kbd_act(id, act, cx);
                        Response::consumed().merge(Response::redraw())
                    }
                    NavOut::Close => {
                        self.kbd_leave(cx, true);
                        Response::consumed().merge(Response::redraw())
                    }
                    NavOut::Pass => return None,
                })
            }
            _ => None,
        }
    }

    /// Per frame: stand the ring on the focused control (or on the selected tab while the card opens), let it glide. True while it moves.
    pub(super) fn kbd_step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        let mut wait = false;
        if self.kbd.on {
            let cs = self.kbd_controls();
            if self.kbd.first {
                let first = cs.iter().find(|c| c.value == CV::Selected(true)).or_else(|| cs.first());
                if let Some(c) = first {
                    self.kbd.focus = Some(c.id);
                    self.kbd.first = false;
                } else if cx.now_ms - self.kbd.since > OPEN_WAIT_MS {
                    self.kbd_leave(cx, false); // the card never opened (a fullscreen guard, a hidden island): give the keyboard back
                } else {
                    wait = true;
                }
            }
            // the focused control went away (a tab was taken off, the card closed): the ring goes with it
            if self.kbd.focus.is_some_and(|f| !cs.iter().any(|c| c.id == f)) && !self.kbd.first {
                self.kbd.focus = cs.iter().find(|c| c.value == CV::Selected(true)).map(|c| c.id);
            }
            let aim = self.kbd.focus.and_then(|f| cs.iter().find(|c| c.id == f)).map(|c| (c.id, c.rect));
            self.kbd.ring.aim(aim);
        } else {
            self.kbd.ring.aim(None);
        }
        self.kbd.ring.step(dt_ms) | wait
    }

    // ------------------------------------------------------------------------------------------------ high contrast

    /// Re-read `a11y.contrast` and what Windows says; hand the shell the body colour (or take it away) and keep the palette for the controls.
    pub(super) fn contrast_refresh(&mut self, cx: &mut Cx) {
        let mode = self.core.settings.get(contrast::KEY).and_then(Value::as_str).map(Mode::parse).unwrap_or_default();
        let sys = if mode == Mode::Off { None } else { contrast::read_system() };
        let hc = contrast::pick(mode, sys);
        let hex = hc.map(|p| contrast::hex(p.bg)).unwrap_or_default();
        self.kbd.mode = mode;
        if hc != self.kbd.hc || hex != self.kbd.hc_sent {
            self.kbd.hc = hc;
            self.kbd.hc_sent = hex.clone();
            self.core.shell_event(Event::SettingsChanged(json!({ "a11y.hc.bg": hex })), cx.now_ms);
            self.exec(cx);
            cx.request_frame();
        }
    }

    /// (tests) high contrast is on
    pub fn dbg_hc(&self) -> bool {
        self.kbd.hc.is_some()
    }

    /// The ring, and under high contrast the 2 px outline of the body. Called at the end of `draw`, in window logical px.
    pub(super) fn kbd_paint(&self, cv: &Canvas, th: &Theme, body: Rect, radius: f32) {
        if let Some(p) = &self.kbd.hc {
            cv.stroke_rr(body.inset(contrast::OUTLINE_W / 2.0), (radius - contrast::OUTLINE_W / 2.0).max(0.0), contrast::OUTLINE_W, p.outline);
        }
        self.kbd.ring.paint(cv, th.accent, self.kbd.hc.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_names_are_spoken_in_both_languages() {
        for b in [Btn::Dict, Btn::Pin, Btn::Settings, Btn::Vpn] {
            assert!(!btn_name(b, true).is_empty() && !btn_name(b, false).is_empty());
        }
        assert_ne!(btn_name(Btn::Pin, true), btn_name(Btn::Pin, false));
    }

    #[test]
    fn the_id_ranges_do_not_overlap() {
        // 64 tabs, a handful of buttons, 100 card / row buttons each
        assert!(TAB0 + 64 < BTN0 && BTN0 + 8 < CARD0 && CARD0 + 100 < ROW0 && ROW0 + 100 < CHIP && CHIP < VIEW0);
    }
}
