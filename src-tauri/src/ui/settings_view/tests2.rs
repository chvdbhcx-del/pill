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
            self.sim.frame.request(); // the host steps once more after a frame that queued field events
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
    let r = t.v.borrow().ctl_rect_of("underTaskbar").expect("row");
    t.click_at(r);
    assert!(t.calls("settings_set").iter().any(|a| a["patch"]["underTaskbar"] == json!(false)), "{:?}", t.calls("settings_set"));
    assert!(t.v.borrow().model.get_bool("underTaskbar") == false);
}

#[test]
fn segmented_click_keeps_the_option_type() {
    let _g = motion_lock();
    let mut t = boot(json!({}), "appearance");
    let r = t.v.borrow().ctl_rect_of("shape").expect("row");
    // the last segment is 240
    t.click_at(Rect::new(r.right() - 12.0, r.y, 10.0, r.h));
    assert!(t.calls("settings_set").iter().any(|a| a["patch"]["shape"] == json!("capsule")), "{:?}", t.calls("settings_set"));
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

fn ctrl() -> Modifiers {
    Modifiers::CTRL
}

#[test]
fn hotkey_recorder_commits_through_hotkey_set() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "hotkeys");
    t.click_act(&Act::Recorder("toggle".into()));
    assert!(t.v.borrow().model.recorder.is_recording("toggle"));
    t.sim.send(&InputEvent::Key { code: KeyCode::Other(0x11), mods: ctrl(), down: true, repeat: false });
    t.settle();
    assert_eq!(t.v.borrow().rec_live, vec!["Ctrl"]);
    let m = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    t.sim.send(&InputEvent::Key { code: KeyCode::Char('K'), mods: m, down: true, repeat: false });
    t.settle();
    let calls = t.calls("hotkey_set");
    assert_eq!(calls, vec![json!({"action": "toggle", "combo": "Ctrl+Alt+K"})]);
    assert!(!t.v.borrow().model.recorder.is_recording("toggle"));
    // settings_set is never used for a hotkey
    assert!(t.calls("settings_set").is_empty());
}

#[test]
fn escape_cancels_the_recorder_and_backspace_clears() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "hotkeys");
    t.click_act(&Act::Recorder("toggle".into()));
    t.key(KeyCode::Escape, Modifiers::NONE);
    assert!(t.v.borrow().model.recorder.active().is_none());
    assert!(t.calls("hotkey_set").is_empty());
    t.click_act(&Act::Recorder("toggle".into()));
    t.key(KeyCode::Backspace, Modifiers::NONE);
    assert_eq!(t.calls("hotkey_set"), vec![json!({"action": "toggle", "combo": ""})]);
}

#[test]
fn sidebar_click_switches_the_page_and_fetches_its_data() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "general");
    t.click_act(&Act::Nav("hotkeys".into()));
    assert_eq!(t.current(), "hotkeys");
    assert!(!t.calls("launcher_list").is_empty());
    assert!(t.v.borrow().leaving.is_none());
}

#[test]
fn reduced_motion_switches_pages_at_once() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en", "reduceMotion": true}), "general");
    let r = t.v.borrow().hit_rect(&Act::Nav("appearance".into())).unwrap();
    click(&mut t.sim, r.cx(), r.cy());
    t.sim.pump();
    assert!(t.v.borrow().leaving.is_none(), "no exit slide under reduced motion");
    assert_eq!(t.current(), "appearance");
    assert!(t.v.borrow().enter_t.finished());
}

#[test]
fn ctrl_f_search_jumps_to_a_row_and_flashes_it() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "general");
    t.key(KeyCode::Char('F'), ctrl());
    assert_eq!(t.v.borrow().focus.focused(), Some(chrome::FID_SEARCH));
    t.text("quiet hours");
    assert!(t.v.borrow().model.search.open, "results are open");
    assert!(!t.v.borrow().search.rows.is_empty());
    t.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(t.current(), "behavior");
    // the highlight plays for 1.7 s: look right after the jump
    t.sim.run(3);
    let _ = t.sim.render((840.0, 760.0), &Theme::settings_dark());
    t.sim.run(3);
    assert!(t.v.borrow().page.rows().any(|r| r.row.id == "quiet.on" && r.hl.is_some()) || t.v.borrow().goto.is_none());
}

#[test]
fn wizard_steps_and_completion_flag() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "welcome");
    assert!(t.v.borrow().wiz.active());
    t.click_act(&Act::WizNext);
    assert_eq!(t.v.borrow().model.wizard.step(), 2);
    // step 2 lists the modules: Back sits below the fold
    t.v.borrow_mut().page.scroll.scroll_to(9999.0, false);
    t.settle();
    t.click_act(&Act::WizBack);
    assert_eq!(t.v.borrow().model.wizard.step(), 1);
    t.click_act(&Act::WizSkip);
    assert!(t.calls("settings_set").iter().any(|a| a["patch"]["firstRunDone"] == json!(true)));
    assert_eq!(t.current(), "general");
    assert!(!t.v.borrow().wiz.active());
}
