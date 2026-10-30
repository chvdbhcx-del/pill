//! Headless tests of the native settings window: control -> model wiring, navigation, search, wizard, recorder, gating, and PNG snapshots
//! (`PILL_SNAP_DIR=<dir> cargo test --release settings_view` writes them).
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

fn aisetup_status() -> Value {
    json!({
        "done": false, "dir": "C:/Users/x/AppData/Pill/ai", "busy": null,
        "whisper": {"engine": {"tag": "v1.7.5", "size": 4000000, "host": "github.com", "installed": false, "verified": true},
            "models": [{"id": "base", "size": 148000000, "host": "huggingface.co", "installed": false}, {"id": "small", "size": 488000000, "host": "huggingface.co", "installed": false}, {"id": "turbo", "size": 574000000, "host": "huggingface.co", "installed": false}],
            "enabled": false, "running": null, "model": "small", "endpoint": "", "voiceOn": false},
        "ollama": {"installed": true, "up": true, "installer": {"host": "ollama.com", "size": 1200000000, "ready": false}, "catalog": [{"name": "qwen3:4b", "size": 2500000000.0}, {"name": "gemma3:1b", "size": 815000000.0}]},
        "servers": [{"id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": true, "models": ["qwen3:4b"]}],
        "ai": {"on": false, "baseUrl": "", "model": ""}
    })
}

pub(super) fn all_page_ids() -> Vec<&'static str> {
    pill_ui_core::settings::schema::schema().pages.iter().map(|p| p.id).collect()
}

pub(super) fn boot_page(id: &str) -> (Sim, Fakes) {
    let (svc, fakes) = Services::fake();
    fakes.api.reply("settings_get", Ok(json!({}))).reply("settings_info", Ok(json!({"version": "0.5.3", "dir": "C:/x"}))).reply("aisetup_status", Ok(aisetup_status()));
    let mut sim = Sim::with_services(Box::new(Nil), svc);
    sim.view = Box::new(SettingsView::new(&mut sim.cx, &json!({ "page": id })));
    sim.pump();
    sim.show();
    sim.pump();
    sim.run(400);
    (sim, fakes)
}

#[test]
fn every_page_renders_in_both_themes() {
    let _g = motion_lock();
    for (tn, th) in [("dark", Theme::settings_dark()), ("light", settings_light())] {
        for id in all_page_ids() {
            let (mut sim, _f) = boot_page(id);
            let img = snap(&mut sim, &format!("{id}-{tn}-1x"), &th, (840.0, 760.0));
            assert!(img.count(None, |p| p[3] == 255) > 100_000, "{id} {tn}");
        }
    }
}
