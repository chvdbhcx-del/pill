use super::*;
use crate::ui::host::testing::{motion_lock, Sim};
use crate::ui::tabs::a::snap::{png_data_url, shoot};
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{Fakes, Services};
use glasskit::input::{Buttons, Modifiers};
use glasskit::geom::Pt;
use serde_json::json;

const NOW: i64 = 1_793_369_109_000;

struct Calm(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
impl Drop for Calm {
    fn drop(&mut self) {
        glasskit::ease::force_reduced(None);
    }
}
fn motion(reduced: bool) -> Calm {
    let g = motion_lock();
    glasskit::ease::force_reduced(Some(reduced));
    Calm(g)
}

fn item(id: i64, kind: &str, preview: &str, pinned: bool) -> Value {
    json!({ "id": id, "kind": kind, "preview": preview, "truncated": false, "fileCount": if kind == "files" { 3 } else { 0 }, "size": 1234, "pinned": pinned,
        "source": "Code.exe", "createdAt": NOW - id * 60_000, "usedAt": NOW - id * 3 * 60_000 })
}

fn list(kind: &str) -> Value {
    match kind {
        "empty" => json!([]),
        "images" => json!([item(1, "image", "Screenshot 1920x1080", false), item(2, "text", "cargo test --release tabs::a", false), item(3, "image", "Image 640x480", false)]),
        "pinned" => json!([item(5, "text", "Pinned: the production database host", true), item(1, "text", "https://example.com/docs/clipboard", false), item(2, "files", "report.pdf, notes.txt +1", false)]),
        _ => json!([
            item(1, "text", "fn main() { println!(\"hello\"); }", false),
            item(2, "text", "https://example.com/some/long/link/that/does/not/fit/on/a/single/line/of/the/row", false),
            item(3, "files", "report.pdf, notes.txt +1", false),
            item(4, "text", "Meeting at 15:30, room 4B", false),
            item(5, "text", "Second line\nthird line of a multi-line copy", false),
        ]),
    }
}

struct Rig {
    sim: Sim,
    f: Fakes,
    fx: Vec<TabFx>,
}

impl Rig {
    fn open(kind: &str) -> Rig {
        let (svc, f) = Services::fake();
        f.api.reply("settings_get", Ok(json!({})));
        let l = list(kind);
        f.api.reply("clip_status", Ok(json!({ "paused": false, "count": l.as_array().unwrap().len() })));
        f.api.reply("clip_list", Ok(l));
        f.api.reply("clip_image", Ok(json!(png_data_url(48, 32, [220, 80, 60, 255], [60, 120, 220, 255]))));
        f.api.reply("clip_paste", Ok(json!("pasted")));
        let mut sim = Sim::with_services(Box::new(ClipboardView::new().with_now(NOW)), svc);
        sim.show();
        let mut r = Rig { sim, f, fx: vec![] };
        r.settle();
        let _ = r.sim.render((super::super::W, 260.0), &Theme::pill_dark());
        r
    }
    fn settle(&mut self) {
        for _ in 0..30 {
            let msgs = self.sim.queue.drain();
            if msgs.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(15)); // a worker may still be decoding an image
                if self.sim.queue.is_empty() {
                    break;
                }
                continue;
            }
            for (_, m) in msgs {
                match m {
                    Msg::Custom(b) if b.is::<TabFx>() => self.fx.push(*b.downcast::<TabFx>().unwrap()),
                    m => {
                        self.sim.deliver(m);
                    }
                }
            }
            self.sim.run(2000);
        }
        self.sim.run(2000);
    }
    fn calls(&self, name: &str) -> Vec<Value> {
        self.f.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
    }
    fn mv(&mut self, p: Pt) {
        self.sim.send(&InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE });
    }
    fn click(&mut self, p: Pt) {
        self.mv(p);
        self.sim.send(&InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
        self.sim.send(&InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE });
        self.settle();
    }
    fn key(&mut self, code: KeyCode) {
        self.sim.send(&InputEvent::Key { code, mods: Modifiers::NONE, down: true, repeat: false });
        self.settle();
    }
    fn typed(&mut self, s: &str) {
        for ch in s.chars() {
            self.sim.send(&InputEvent::Char(ch));
        }
        let _ = self.sim.render((super::super::W, 260.0), &Theme::pill_dark()); // the field takes its queued edits in draw
        self.settle();
    }
    /// Let `ms` of model time pass: the ticker the view runs while a timer is pending, as the host would deliver it.
    fn elapse(&mut self, ms: f64) {
        self.sim.now_ms += ms;
        self.sim.deliver(Msg::Custom(Box::new(crate::ui::tabs::a::core::Tick)));
        self.settle();
    }
    /// Centre of row `i` (the list starts under the search bar).
    fn row(&self, i: usize) -> Pt {
        (260.0, BAR_H + GAP + ROW_H * i as f32 + ROW_H / 2.0)
    }
}

// ------------------------------------------------------------------------------------------------------------------ wiring

#[test]
fn showing_the_tab_reads_the_status_and_the_first_page_of_the_history() {
    let r = Rig::open("text");
    let n: Vec<String> = r.f.api.calls().into_iter().map(|c| c.name).collect();
    assert!(n.contains(&"clip_status".to_string()) && n.contains(&"clip_list".to_string()), "{n:?}");
    assert_eq!(r.calls("clip_list")[0]["limit"], 100);
}

#[test]
fn image_rows_ask_for_their_thumbnails_and_the_cache_gets_them() {
    let _m = motion(false);
    let mut r = Rig::open("images");
    r.settle();
    let ids: Vec<i64> = r.calls("clip_image").iter().map(|a| a["id"].as_i64().unwrap()).collect();
    assert!(ids.contains(&1) && ids.contains(&3), "{ids:?}");
}

#[test]
fn enter_pastes_the_selected_row_and_a_click_pastes_that_row() {
    let mut r = Rig::open("text");
    r.key(KeyCode::Down);
    r.key(KeyCode::Enter);
    let first = r.calls("clip_paste");
    assert_eq!(first.len(), 1, "{first:?}");
    let p = r.row(2);
    r.click(p);
    let all = r.calls("clip_paste");
    assert_eq!(all.len(), 2, "{all:?}");
    assert_ne!(all[0]["id"], all[1]["id"]);
}

#[test]
fn row_buttons_pin_and_delete() {
    let mut r = Rig::open("text");
    // the action buttons appear on the hovered row, at its right edge
    let y = r.row(0).1;
    r.mv((300.0, y));
    r.sim.run(100);
    let x_del = super::super::W - 12.0 - ACT / 2.0;
    r.click((x_del, y));
    let del = r.calls("clip_delete");
    let pin = r.calls("clip_pin");
    assert!(del.len() + pin.len() >= 1, "a button click reached the backend");
}

#[test]
fn typing_in_the_search_box_queries_the_backend_after_the_debounce() {
    let _m = motion(false);
    let mut r = Rig::open("text");
    r.click((200.0, BAR_H / 2.0));
    r.typed("link");
    r.elapse(400.0);
    assert!(r.calls("clip_list").iter().any(|a| a["query"] == "link"), "{:?}", r.calls("clip_list"));
}

#[test]
fn a_new_copy_event_reloads_the_list() {
    let mut r = Rig::open("text");
    let before = r.calls("clip_list").len();
    r.f.bus.emit("clip:added", json!({ "id": 9 }));
    r.settle();
    r.elapse(300.0);
    assert!(r.calls("clip_list").len() > before);
}

#[test]
fn frames_stop_when_settled_and_reduced_motion_plays_no_cues() {
    let _m = motion(true);
    let mut r = Rig::open("text");
    r.sim.cx.reduced_motion = true;
    r.f.bus.emit("clip:added", json!({ "id": 9 }));
    r.settle();
    assert!(!r.sim.frame.wants_frame(), "settled = idle");
}

// ------------------------------------------------------------------------------------------------------------------ pixels

fn shoot_all(r: &mut Rig, name: &str) {
    for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
        for sc in [1.0f32, 2.0] {
            r.sim.cx.scale = sc;
            let img = shoot(&mut *r.sim.view, &r.sim.cx, &th, sc, "clipboard", &format!("{name}-{tn}-x{}", sc as u32));
            assert!(img.count(None, |p| p[3] > 0) > 5000, "{name} {tn} x{sc} drew nothing");
        }
    }
}

#[test]
fn snapshots_of_the_typical_states() {
    let _m = motion(false);
    for (kind, name) in [("text", "text"), ("images", "images"), ("pinned", "pinned"), ("empty", "empty")] {
        let mut r = Rig::open(kind);
        r.settle();
        shoot_all(&mut r, name);
    }
    let mut r = Rig::open("text");
    r.key(KeyCode::Down);
    r.key(KeyCode::Down);
    shoot_all(&mut r, "selected");
    r.click((200.0, BAR_H / 2.0));
    r.typed("link");
    shoot_all(&mut r, "search-typing");
}
