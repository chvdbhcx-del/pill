//! Behaviour tests: clicks reach the model and the backend, navigation, search, the recorder, the wizard, gating and reduced motion.
use super::tests::{boot_page, click};
use super::ui::Act;
use super::*;
use crate::ui::host::testing::{motion_lock, Sim};
use crate::ui::view::{Fakes, Services};
use glasskit::input::{Buttons, KeyCode, Modifiers, MouseButton};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// A view the test keeps a handle to (the host owns a `Box<dyn View>`).
struct Probe(Rc<RefCell<SettingsView>>);

impl View for Probe {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.0.borrow_mut().event(ev, cx)
    }
    fn message(&mut self, m: Msg, cx: &mut Cx) -> Response {
        self.0.borrow_mut().message(m, cx)
    }
    fn step(&mut self, dt: f64, cx: &mut Cx) -> bool {
        self.0.borrow_mut().step(dt, cx)
    }
    fn draw(&mut self, cv: &Canvas, t: &Theme, r: Rect, cx: &Cx) {
        self.0.borrow_mut().draw(cv, t, r, cx)
    }
    fn on_show(&mut self, cx: &mut Cx) {
        self.0.borrow_mut().on_show(cx)
    }
}

pub(super) struct T {
    pub sim: Sim,
    pub v: Rc<RefCell<SettingsView>>,
    pub fakes: Fakes,
}

pub(super) fn boot(settings: Value, page: &str) -> T {
    let (svc, fakes) = Services::fake();
    fakes.api.reply("settings_get", Ok(settings)).reply("settings_info", Ok(json!({"version": "0.5.3", "dir": "C:/x"}))).reply("hotkey_set", Ok(json!({"ok": true, "combo": "Ctrl+Alt+K"})));
    let mut sim = Sim::with_services(Box::new(Probe(Rc::new(RefCell::new(unsafe_placeholder())))), svc);
    let v = Rc::new(RefCell::new(SettingsView::new(&mut sim.cx, &json!({ "page": page }))));
    sim.view = Box::new(Probe(v.clone()));
    sim.pump();
    sim.show();
    let mut t = T { sim, v, fakes };
    t.settle();
    t
}

fn unsafe_placeholder() -> SettingsView {
    let mut cx = Cx::headless();
    SettingsView::new(&mut cx, &json!({}))
}

impl T {
    /// Frames, replies, frames: a rendered frame is what gives the view its layout.
    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.sim.pump();
            self.sim.run(300);
            let th = Theme::settings_dark();
            let _ = self.sim.render((840.0, 760.0), &th);
        }
    }
    pub fn click_at(&mut self, r: Rect) {
        click(&mut self.sim, r.cx(), r.cy());
        self.settle();
    }
    pub fn click_act(&mut self, a: &Act) {
        let r = self.v.borrow().hit_rect(a).unwrap_or_else(|| panic!("no hit for {a:?}"));
        self.click_at(r);
    }
    pub fn calls(&self, name: &str) -> Vec<Value> {
        self.fakes.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
    }
    pub fn key(&mut self, code: KeyCode, mods: Modifiers) {
        self.sim.send(&InputEvent::Key { code, mods, down: true, repeat: false });
        self.sim.send(&InputEvent::Key { code, mods, down: false, repeat: false });
        self.settle();
    }
    pub fn text(&mut self, s: &str) {
        for c in s.chars() {
            self.sim.send(&InputEvent::Char(c));
        }
        self.settle();
    }
    pub fn current(&self) -> &'static str {
        self.v.borrow().model.current
    }
}

#[test]
fn toggle_click_writes_the_setting() {
    let _g = motion_lock();
    let mut t = boot(json!({}), "appearance");
    let r = t.v.borrow().ctl_rect_of("showDate").expect("row");
    t.click_at(r);
    assert!(t.calls("settings_set").iter().any(|a| a["patch"]["showDate"] == json!(true)), "{:?}", t.calls("settings_set"));
    assert!(t.v.borrow().model.get_bool("showDate"));
}

#[test]
fn segmented_click_keeps_the_option_type() {
    let _g = motion_lock();
    let mut t = boot(json!({}), "appearance");
    let r = t.v.borrow().ctl_rect_of("fps").expect("row");
    // the last segment is 240
    t.click_at(Rect::new(r.right() - 12.0, r.y, 10.0, r.h));
    assert!(t.calls("settings_set").iter().any(|a| a["patch"]["fps"] == json!(240)), "{:?}", t.calls("settings_set"));
}

#[test]
fn slider_click_snaps_to_the_step() {
    let _g = motion_lock();
    let mut t = boot(json!({}), "appearance");
    let r = t.v.borrow().ctl_rect_of("size").expect("row");
    t.click_at(Rect::new(r.x + 139.0, r.y, 1.0, 16.0));
    let set: Vec<Value> = t.calls("settings_set");
    assert!(set.iter().any(|a| a["patch"]["size"] == json!(200)), "{set:?}");
}
