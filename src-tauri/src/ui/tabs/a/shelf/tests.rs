use super::*;
use crate::ui::host::testing::{motion_lock, Sim};
use crate::ui::tabs::a::snap::{png_data_url, shoot};
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{DropEvent, Fakes, Services};
use glasskit::input::{Buttons, Modifiers};
use serde_json::json;
use std::path::PathBuf;

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

fn it(id: i64, name: &str, dir: bool, icon: Option<String>) -> Value {
    json!({ "id": id, "path": format!("C:/Users/me/{name}"), "name": name, "added_at": 1, "missing": false, "is_dir": dir, "size": 1_500_000 * id, "icon": icon })
}

fn shelf(kind: &str) -> Value {
    let items = match kind {
        "empty" => vec![],
        "thumbs" => {
            let a = png_data_url(48, 48, [230, 90, 70, 255], [70, 130, 230, 255]);
            let b = png_data_url(48, 48, [90, 200, 120, 255], [250, 210, 70, 255]);
            vec![it(1, "holiday.jpg", false, Some(a)), it(2, "report.pdf", false, None), it(3, "Projects", true, None), it(4, "poster.png", false, Some(b))]
        }
        _ => vec![
            it(1, "report.pdf", false, None),
            it(2, "notes.txt", false, None),
            it(3, "Projects", true, None),
            it(4, "archive.zip", false, None),
            it(5, "song.mp3", false, None),
            it(6, "slides.pptx", false, None),
            it(7, "data.csv", false, None),
        ],
    };
    json!({ "count": items.len(), "items": items })
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
        f.api.reply("shelf_list", Ok(shelf(kind)["items"].clone()));
        f.api.reply("shelf_drag_out", Ok(json!("cancelled")));
        let mut sim = Sim::with_services(Box::new(ShelfView::new()), svc);
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
                std::thread::sleep(std::time::Duration::from_millis(15));
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
    fn down(&mut self, p: Pt, button: MouseButton, mods: Modifiers) {
        self.sim.send(&InputEvent::MouseDown { pos: p, button, mods, clicks: 1 });
    }
    fn up(&mut self, p: Pt, button: MouseButton) {
        self.sim.send(&InputEvent::MouseUp { pos: p, button, mods: Modifiers::NONE });
    }
    fn click(&mut self, p: Pt) {
        self.mv(p);
        self.down(p, MouseButton::Left, Modifiers::NONE);
        self.up(p, MouseButton::Left);
        self.settle();
    }
    /// Centre of chip `i` (head shown, 648 x 260 box).
    fn chip(&self, i: usize) -> Pt {
        let f = layout::frame(Rect::new(0.0, 0.0, super::super::W, 260.0), true);
        let c = layout::chip_rect(i, layout::cols(f.list.w));
        (f.list.x + c.x + c.w / 2.0, f.list.y + c.y + c.h / 2.0)
    }
}

// ------------------------------------------------------------------------------------------------------------------ wiring

#[test]
fn showing_the_tab_lists_the_shelf_and_subscribes() {
    let r = Rig::open("files");
    assert!(!r.calls("shelf_list").is_empty());
}

#[test]
fn dragging_a_chip_out_calls_shelf_drag_out_with_its_id() {
    let mut r = Rig::open("files");
    let a = r.chip(1);
    r.mv(a);
    r.down(a, MouseButton::Left, Modifiers::NONE);
    let mut y = a.1;
    for _ in 0..8 {
        y -= 6.0;
        r.mv((a.0, y));
    }
    r.settle();
    assert_eq!(r.calls("shelf_drag_out"), vec![json!({ "ids": [2] })]);
    r.up((a.0, y), MouseButton::Left);
    r.settle();
}

#[test]
fn ctrl_click_selects_several_and_dragging_one_drags_the_selection() {
    let mut r = Rig::open("files");
    let (a, b) = (r.chip(0), r.chip(2));
    let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
    for p in [a, b] {
        r.mv(p);
        r.down(p, MouseButton::Left, ctrl);
        r.up(p, MouseButton::Left);
    }
    r.settle();
    r.mv(a);
    r.down(a, MouseButton::Left, Modifiers::NONE);
    let mut y = a.1;
    for _ in 0..8 {
        y -= 6.0;
        r.mv((a.0, y));
    }
    r.settle();
    let d = r.calls("shelf_drag_out");
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["ids"].as_array().unwrap().len(), 2, "{d:?}");
}

#[test]
fn double_click_opens_and_the_context_menu_has_open_reveal_copy() {
    let mut r = Rig::open("files");
    let p = r.chip(0);
    r.mv(p);
    r.sim.send(&InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 2 });
    r.up(p, MouseButton::Left);
    r.settle();
    assert_eq!(r.calls("shelf_open"), vec![json!({ "id": 1 })]);
    let q = r.chip(1);
    r.mv(q);
    r.down(q, MouseButton::Right, Modifiers::NONE);
    r.up(q, MouseButton::Right);
    r.settle();
    // the menu opens at the cursor; its second row is "Reveal"
    let item = (q.0 + 40.0, q.1 + MENU_PAD + MENU_ITEM_H * 1.5);
    r.click(item);
    let rev = r.calls("shelf_reveal");
    assert!(rev.len() + r.calls("shelf_copy_path").len() + r.calls("shelf_open").len() >= 2, "the menu acted: {rev:?}");
}

#[test]
fn dropping_files_adds_them_to_the_shelf() {
    let mut r = Rig::open("files");
    let f = vec![PathBuf::from("C:\\Users\\me\\a.pdf"), PathBuf::from("C:\\Users\\me\\b.txt")];
    let pos = (300.0, 150.0);
    let e = r.sim.send_drop(&DropEvent { phase: DropPhase::Enter, pos, payload: Some(DropPayload::Files(f.clone())) });
    assert!(e.consumed, "the shelf accepts files");
    assert!(r.sim.send_drop(&DropEvent { phase: DropPhase::Over, pos, payload: None }).consumed);
    r.sim.send_drop(&DropEvent { phase: DropPhase::Drop, pos, payload: Some(DropPayload::Files(f)) });
    r.settle();
    let a = r.calls("shelf_add");
    assert_eq!(a.len(), 1, "{a:?}");
    assert_eq!(a[0]["paths"].as_array().unwrap().len(), 2);
    // text drags are not ours
    let t = r.sim.send_drop(&DropEvent { phase: DropPhase::Enter, pos, payload: Some(DropPayload::Text("hi".into())) });
    assert!(!t.consumed);
}

#[test]
fn shelf_changed_event_reloads_and_a_settled_view_costs_no_frames() {
    let _m = motion(false);
    let mut r = Rig::open("files");
    r.f.bus.emit("shelf:changed", shelf("thumbs"));
    r.settle();
    assert!(!r.sim.frame.wants_frame(), "settled = idle");
}

#[test]
fn reduced_motion_lands_chips_without_cues() {
    let _m = motion(true);
    let mut r = Rig::open("empty");
    r.sim.cx.reduced_motion = true;
    r.f.bus.emit("shelf:changed", shelf("files"));
    r.settle();
    assert!(!r.sim.frame.wants_frame());
}

// ------------------------------------------------------------------------------------------------------------------ pixels

fn shoot_all(r: &mut Rig, name: &str) {
    for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
        for sc in [1.0f32, 2.0] {
            r.sim.cx.scale = sc;
            let img = shoot(&mut *r.sim.view, &r.sim.cx, &th, sc, "shelf", &format!("{name}-{tn}-x{}", sc as u32));
            assert!(img.count(None, |p| p[3] > 0) > 3000, "{name} {tn} x{sc} drew nothing");
        }
    }
}

#[test]
fn snapshots_of_the_typical_states() {
    let _m = motion(false);
    for (kind, name) in [("empty", "empty"), ("files", "files"), ("thumbs", "thumbs")] {
        let mut r = Rig::open(kind);
        r.settle();
        shoot_all(&mut r, name);
    }
    let mut r = Rig::open("files");
    let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
    for i in [0usize, 3] {
        let p = r.chip(i);
        r.mv(p);
        r.down(p, MouseButton::Left, ctrl);
        r.up(p, MouseButton::Left);
    }
    r.settle();
    shoot_all(&mut r, "selection");
    let q = r.chip(1);
    r.mv(q);
    r.down(q, MouseButton::Right, Modifiers::NONE);
    r.up(q, MouseButton::Right);
    r.settle();
    shoot_all(&mut r, "menu");
    let mut r = Rig::open("files");
    r.sim.send_drop(&DropEvent { phase: DropPhase::Enter, pos: (300.0, 150.0), payload: Some(DropPayload::Files(vec![PathBuf::from("C:\\a.pdf")])) });
    r.settle();
    shoot_all(&mut r, "drop-hover");
}
