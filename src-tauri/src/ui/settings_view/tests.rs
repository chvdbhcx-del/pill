//! Headless tests of the native settings window: control -> model wiring, navigation, search, wizard, recorder, gating, and PNG snapshots
//! (`PILL_SNAP_DIR=<dir> cargo test --release settings_view` writes them; compare with `tools/dev/settings-diff.mjs`).
use super::*;
use crate::ui::host::testing::{motion_lock, save_png, Sim};
use crate::ui::view::{Fakes, Services};
use glasskit::input::{Buttons, KeyCode, Modifiers, MouseButton};
use serde_json::{json, Value};

struct Nil;
impl View for Nil {
    fn draw(&mut self, _cv: &Canvas, _t: &Theme, _r: Rect, _cx: &Cx) {}
}

pub(super) fn boot(settings: Value, args: Value) -> (Sim, Fakes) {
    let (svc, fakes) = Services::fake();
    fakes.api.reply("settings_get", Ok(settings)).reply("settings_info", Ok(json!({"version": "0.5.3", "dir": "C:/Users/x/AppData/Pill"}))).reply("autostart_get", Ok(json!(false))).reply("pill_monitors", Ok(json!(["DISPLAY1", "DISPLAY2"])));
    let mut sim = Sim::with_services(Box::new(Nil), svc);
    sim.view = Box::new(SettingsView::new(&mut sim.cx, &args));
    sim.pump();
    sim.show();
    sim.run(400);
    (sim, fakes)
}

pub(super) fn snap(sim: &mut Sim, name: &str, theme: &Theme, size: (f32, f32)) -> glasskit::testkit::Image {
    let img = sim.render(size, theme);
    if let Ok(dir) = std::env::var("PILL_SNAP_DIR") {
        save_png(&img, std::path::Path::new(&dir).join(format!("{name}.png")));
    }
    img
}

fn mv(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
}
fn down(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseDown { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }
}
fn up(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseUp { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE }
}

pub(super) fn click(sim: &mut Sim, x: f32, y: f32) {
    sim.send(&mv(x, y));
    sim.send(&down(x, y));
    sim.send(&up(x, y));
    sim.pump();
    sim.run(400);
}

#[test]
fn first_page_renders() {
    let _g = motion_lock();
    let (mut sim, _f) = boot(json!({}), json!({"page": "general"}));
    let th = Theme::settings_dark();
    let img = snap(&mut sim, "general-dark-1x", &th, (840.0, 760.0));
    // the sidebar background and a card both appear
    assert!(img.count(None, |p| p[3] == 255) > 100_000);
}
