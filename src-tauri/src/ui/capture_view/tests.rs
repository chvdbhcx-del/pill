//! Tests of the native capture overlay: event -> model wiring, backend calls, the PNG export, the gate, and headless snapshots.
//!
//! Snapshots are rendered on WARP through `host::testing::Sim`. They are asserted on (pixels in known places) and, when
//! `PILL_CAPTURE_DUMP=<dir>` is set, written as PNGs so a human (or `tools/dev/capture-diff.mjs`) can look at them.
use super::io::{Backend, RawSink};
use super::*;
use crate::ui::host::testing::{motion_lock, Sim};
use crate::ui::view::{Clipboard, Fakes, Msg, Services};
use glasskit::color::Color;
use glasskit::geom::Rect as GRect;
use glasskit::input::{Buttons, MouseButton};
use glasskit::testkit::{Harness, Image};
use glasskit::text::Style;
use pill_ui_core::capture::{CommitKind, DrawCmd, IRect as CIRect, Tool};
use serde_json::{json, Value};
use std::sync::Mutex;

type Raw = Arc<Mutex<Vec<(String, Value, Vec<u8>)>>>;

// ---------------------------------------------------------------------------------------------------------------------------------------
// A synthetic "desktop": gradient, windows, text, so a snapshot shows whether the frame, the dim and the markup are where they belong
// ---------------------------------------------------------------------------------------------------------------------------------------

thread_local! {
    static H: Harness = Harness::new();
}

/// RGBA frame of `w x h` device px.
pub fn desktop_frame(w: u32, h: u32) -> Vec<u8> {
    H.with(|hh| {
        let img = hh.render(w, h, 1.0, Color::rgb8(20, 30, 60), |cv| {
            // vertical blue -> teal gradient in strips
            let strips = 64u32;
            for i in 0..strips {
                let t = i as f32 / strips as f32;
                let c = Color::rgb8((30.0 + 40.0 * t) as u8, (60.0 + 120.0 * t) as u8, (150.0 - 40.0 * t) as u8);
                cv.fill_rect(GRect::new(0.0, h as f32 * t, w as f32, h as f32 / strips as f32 + 1.0), c);
            }
            // two "windows" with titles and lines of text
            let (ww, wh) = (w as f32, h as f32);
            for (k, (x, y, rw, rh)) in [(0.08, 0.14, 0.42, 0.55), (0.52, 0.30, 0.40, 0.50)].into_iter().enumerate() {
                let r = GRect::new(ww * x, wh * y, ww * rw, wh * rh);
                cv.fill_rr(r, 10.0, Color::rgb8(245, 245, 248));
                cv.fill_rr(GRect::new(r.x, r.y, r.w, 30.0), 10.0, Color::rgb8(225, 228, 238));
                cv.text(GRect::new(r.x + 12.0, r.y, r.w - 24.0, 30.0), if k == 0 { "Quarterly report" } else { "Notes" }, Style::new(13.0, 600), Color::rgb8(30, 30, 40));
                for line in 0..9 {
                    let ly = r.y + 44.0 + line as f32 * 26.0;
                    if ly + 20.0 > r.bottom() {
                        break;
                    }
                    cv.text(GRect::new(r.x + 16.0, ly, r.w - 32.0, 22.0), &format!("{} The quick brown fox jumps over the lazy dog 0123456789", line + 1), Style::new(14.0, 400), Color::rgb8(40, 44, 60));
                }
            }
            cv.text(GRect::new(24.0, wh - 56.0, 420.0, 40.0), "Synthetic desktop for the capture tests", Style::new(20.0, 600), Color::rgb8(255, 255, 255));
        });
        let mut out = Vec::with_capacity(img.bgra.len());
        for p in img.bgra.chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
        out
    })
}

fn dump(name: &str, img: &Image) {
    if let Ok(dir) = std::env::var("PILL_CAPTURE_DUMP") {
        let p = std::path::Path::new(&dir).join(format!("{name}.png"));
        crate::ui::host::testing::save_png(img, p);
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------------------------------------------------------------------

/// The view behind an `Rc` so a test can read its state while the `Sim` drives it.
struct Shared(std::rc::Rc<std::cell::RefCell<CaptureView>>);

impl View for Shared {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.0.borrow_mut().event(ev, cx)
    }
    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.0.borrow_mut().message(msg, cx)
    }
    fn step(&mut self, dt: f64, cx: &mut Cx) -> bool {
        self.0.borrow_mut().step(dt, cx)
    }
    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: GRect, cx: &Cx) {
        self.0.borrow_mut().draw(cv, theme, rect, cx)
    }
    fn on_show(&mut self, cx: &mut Cx) {
        self.0.borrow_mut().on_show(cx)
    }
    fn on_hide(&mut self, cx: &mut Cx) {
        self.0.borrow_mut().on_hide(cx)
    }
    fn on_dpi(&mut self, s: f32, cx: &mut Cx) {
        self.0.borrow_mut().on_dpi(s, cx)
    }
}

struct Rig {
    v: std::rc::Rc<std::cell::RefCell<CaptureView>>,
    sim: Sim,
    fakes: Fakes,
    raw: Raw,
    vw: f32,
    vh: f32,
    scale: f32,
    frame: Arc<Vec<u8>>,
}

fn rig_with(mode: &str, vw: f32, vh: f32, scale: f32, opts: Options, ru: bool) -> Rig {
    let (fw, fh) = ((vw * scale).round() as u32, (vh * scale).round() as u32);
    let frame = Arc::new(desktop_frame(fw, fh));
    rig_frame(mode, vw, vh, scale, opts, ru, frame, 1)
}

#[allow(clippy::too_many_arguments)]
fn rig_frame(mode: &str, vw: f32, vh: f32, scale: f32, opts: Options, ru: bool, frame: Arc<Vec<u8>>, id: u64) -> Rig {
    let (fw, fh) = ((vw * scale).round() as i32, (vh * scale).round() as i32);
    let (svc, fakes) = Services::fake();
    fakes.api.reply("capture_session", Ok(json!({ "id": id, "mode": mode, "w": fw, "h": fh, "x": 0, "y": 0, "scale": scale, "focus": true })));
    let raw: Raw = Arc::new(Mutex::new(Vec::new()));
    let sink: RawSink = {
        let raw = raw.clone();
        Arc::new(move |caller, args, body| raw.lock().unwrap().push((caller.to_string(), args, body)))
    };
    let f2 = frame.clone();
    let be = Backend { frame: Box::new(move |_| Some(f2.clone())), raw: sink };
    let mut cx = Cx::headless_with(svc.clone());
    cx.scale = scale;
    cx.lang = if ru { GkLang::Ru } else { GkLang::En };
    cx.reduced_motion = opts.reduce_motion;
    let view = CaptureView::with_backend(&mut cx, "capture-0", vw as f64, vh as f64, be, opts);
    let v = std::rc::Rc::new(std::cell::RefCell::new(view));
    let mut sim = Sim::with_services(Box::new(Shared(v.clone())), svc);
    sim.cx.caller = "capture-0".into();
    sim.cx.lang = cx.lang;
    sim.cx.reduced_motion = cx.reduced_motion;
    sim.set_scale(scale);
    Rig { v, sim, fakes, raw, vw, vh, scale, frame }
}

fn rig(mode: &str) -> Rig {
    rig_with(mode, 960.0, 540.0, 1.0, Options::web_parity(), false)
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
fn key(code: KeyCode, mods: Modifiers) -> InputEvent {
    InputEvent::Key { code, mods, down: true, repeat: false }
}

impl Rig {
    fn model(&self) -> std::cell::Ref<'_, CaptureModel> {
        std::cell::Ref::map(self.v.borrow(), |v| &v.model)
    }
    fn view(&self) -> std::cell::Ref<'_, CaptureView> {
        self.v.borrow()
    }
    fn send(&mut self, e: InputEvent) {
        self.sim.send(&e);
        self.sim.run(2000);
    }
    /// press at a, drag to b, release
    fn drag(&mut self, a: (f32, f32), b: (f32, f32)) {
        self.send(mv(a.0, a.1));
        self.send(down(a.0, a.1));
        let steps = 6;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            self.send(mv(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
        self.send(up(b.0, b.1));
    }
    fn click(&mut self, p: (f32, f32)) {
        self.send(mv(p.0, p.1));
        self.send(down(p.0, p.1));
        self.send(up(p.0, p.1));
    }
    fn shot(&mut self, name: &str) -> Image {
        let img = self.sim.render((self.vw, self.vh), &Theme::pill_dark());
        dump(name, &img);
        img
    }
    fn calls(&self) -> Vec<String> {
        self.fakes.api.calls().into_iter().map(|c| c.name).collect()
    }
    fn commit_args(&self) -> Option<Value> {
        self.fakes.api.calls().into_iter().rev().find(|c| c.name == "capture_commit").map(|c| c.args)
    }
    /// The centre of toolbar item `id` in the current view-model.
    fn item(&self, id: &str) -> (f32, f32) {
        let v = self.view();
        let tb = v.vm.toolbar.as_ref().expect("a toolbar");
        let it = tb.items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no toolbar item {id}"));
        ((tb.rect.x + it.rect.x + it.rect.w / 2.0) as f32, (tb.rect.y + it.rect.y + it.rect.h / 2.0) as f32)
    }
    fn wait_raw(&self) -> Vec<(String, Value, Vec<u8>)> {
        for _ in 0..400 {
            if !self.raw.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        self.raw.lock().unwrap().clone()
    }
}

fn rgb(p: [u8; 4]) -> [u8; 3] {
    [p[0], p[1], p[2]]
}

fn lum(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Arming and the idle overlay
// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn arming_fetches_the_session_and_frame_and_asks_to_be_shown() {
    let mut r = rig("area");
    assert_eq!(r.calls().first().map(String::as_str), Some("capture_session"), "{:?}", r.calls());
    assert!(r.calls().contains(&"capture_ready".to_string()));
    assert_eq!(r.model().session_id(), 1);
    assert!(r.view().frame_rgba.is_some(), "the overlay keeps the frame to paint");
    assert!(r.view().io.yielded && r.view().io.focus);
    // the factory's effects (show) went to the construction cx; a re-arm shows through the sim's cx
    r.sim.deliver(Msg::Event { name: "capture:end".into(), payload: json!({}), latest: false });
    r.sim.take_fx();
    r.fakes.api.reply("capture_session", Ok(json!({ "id": 2, "mode": "area", "w": 960, "h": 540, "x": 0, "y": 0, "scale": 1.0, "focus": true })));
    r.sim.deliver(Msg::Event { name: "capture:begin".into(), payload: json!({ "id": 2 }), latest: false });
    let fx = r.sim.take_fx();
    assert!(fx.show, "armed: the window is shown");
    assert_eq!(r.model().session_id(), 2);
    r.sim.pump(); // the self-posted Focus message
    assert_eq!(r.sim.take_fx().focus, Some(true), "the overlay under the cursor takes the keyboard");
}

#[test]
fn a_window_without_a_session_stays_hidden() {
    let (svc, fakes) = Services::fake();
    fakes.api.reply("capture_session", Ok(Value::Null));
    let mut cx = Cx::headless_with(svc.clone());
    let be = Backend { frame: Box::new(|_| None), raw: Arc::new(|_, _, _| {}) };
    let v = CaptureView::with_backend(&mut cx, "capture-3", 800.0, 600.0, be, Options::web_parity());
    assert!(!v.io.yielded);
    assert!(!cx.take_effects().show, "no session: nothing to show");
}

#[test]
fn idle_overlay_shows_the_frame_dim_and_hint() {
    let _m = motion_lock();
    let mut r = rig("area");
    r.sim.run(200);
    let img = r.shot("01-idle");
    // the frame is dimmed by 38 %: a bright desktop pixel is darker than the frame's own pixel
    let f = |x: usize, y: usize| {
        let o = (y * 960 + x) * 4;
        [r.frame[o] as u32, r.frame[o + 1] as u32, r.frame[o + 2] as u32]
    };
    let p = img.at(200.0, 300.0);
    let q = f(200, 300);
    for c in 0..3 {
        let expect = (q[c] as f32 * 0.62).round() as i32;
        assert!((p[c] as i32 - expect).abs() <= 3, "channel {c}: {} vs dimmed {}", p[c], expect);
    }
    // the hint pill is centred near the top: dark pill with light text
    assert!(img.count(Some(GRect::new(300.0, 22.0, 360.0, 32.0)), |p| lum(p) > 600) > 100, "hint text is drawn");
    assert!(r.view().vm.hint.as_ref().is_some_and(|h| h.visible));
}

#[test]
fn the_dim_and_hint_fade_in_unless_motion_is_reduced() {
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = rig("area");
    let first = r.sim.render((960.0, 540.0), &Theme::pill_dark());
    r.sim.run(200);
    let settled = r.sim.render((960.0, 540.0), &Theme::pill_dark());
    assert!(first.diff(&settled, 2) > 1000, "the first frame is not yet fully dimmed");
    // reduced motion: the final state at once
    let mut r2 = rig_with("area", 960.0, 540.0, 1.0, Options { reduce_motion: true, ..Options::web_parity() }, false);
    let f2 = r2.sim.render((960.0, 540.0), &Theme::pill_dark());
    assert_eq!(f2.diff(&settled, 2), 0, "reduced motion starts at the end state");
    glasskit::ease::force_reduced(None);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Selecting, adjusting, handles
// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn dragging_draws_the_cutout_and_the_size_badge() {
    let _m = motion_lock();
    let mut r = rig("area");
    r.send(mv(200.0, 120.0));
    r.send(down(200.0, 120.0));
    r.send(mv(520.0, 380.0));
    let img = r.shot("02-selecting");
    assert_eq!(r.model().phase(), Phase::Dragging);
    // inside the selection the frame is untouched, outside it is dimmed by 46 %
    let inside = img.at(300.0, 250.0);
    let outside = img.at(700.0, 450.0);
    let o = |x: usize, y: usize| {
        let i = (y * 960 + x) * 4;
        [r.frame[i], r.frame[i + 1], r.frame[i + 2]]
    };
    let (fi, fo) = (o(300, 250), o(700, 450));
    for c in 0..3 {
        assert!((inside[c] as i32 - fi[c] as i32).abs() <= 2, "inside is the raw frame");
        assert!((outside[c] as i32 - (fo[c] as f32 * 0.54).round() as i32).abs() <= 3, "outside is dimmed 46 %");
    }
    // the 1 px light outline hugs the box from the outside
    assert!(img.at(199.5, 250.0)[0] > 180, "outline left of x=200");
    // the size badge sits above the top-left corner
    assert!(img.count(Some(GRect::new(200.0, 94.0, 110.0, 20.0)), |p| lum(p) > 600) > 20, "size label text");
    assert_eq!(r.view().vm.selection.as_ref().unwrap().size_label, "320 \u{d7} 260");
}

#[test]
fn releasing_shows_handles_the_toolbar_and_pops_the_handles() {
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = rig("area");
    r.send(mv(200.0, 120.0));
    r.send(down(200.0, 120.0));
    r.send(mv(520.0, 380.0));
    r.sim.send(&up(520.0, 380.0));
    assert_eq!(r.model().phase(), Phase::Adjusting);
    assert!(r.view().vm.toolbar.is_some());
    assert!(r.sim.step_frame(), "the pop animations ask for frames");
    // mid-animation: the handles are still small, the toolbar still faint
    let early = r.shot("03a-adjusting-early");
    r.sim.run(2000);
    let late = r.shot("03-adjusting");
    assert!(!r.sim.frame.wants_frame(), "settled: no more frames");
    assert!(early.diff(&late, 4) > 200, "the pop is visible");
    // all 8 handles are white discs with the accent ring
    let sel = r.view().vm.selection.as_ref().unwrap().clone();
    assert_eq!(sel.handles.len(), 8);
    for h in &sel.handles {
        let p = late.at(h.at.x as f32, h.at.y as f32);
        assert!(p[0] > 240 && p[1] > 240 && p[2] > 240, "handle centre is white: {p:?}");
    }
    // the toolbar: dark rounded bar under the selection, right-aligned to it
    let tb = r.view().vm.toolbar.as_ref().unwrap().rect;
    assert!((tb.x - 8.0).abs() < 1.0 && (tb.y - 390.0).abs() < 1.0, "the bar is wider than the selection: pinned to the left margin, 10 px below: {tb:?}");
    let bar = late.at((tb.x + 4.0) as f32, (tb.y + 22.0) as f32);
    assert!(lum(bar) < 120, "bar background is near black: {bar:?}");
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_toolbar_sits_above_when_there_is_no_room_below() {
    let mut r = rig("area");
    r.drag((300.0, 300.0), (600.0, 520.0));
    let tb = r.view().vm.toolbar.as_ref().unwrap().rect;
    assert!(tb.y + tb.h <= 300.0, "flipped above the selection: {tb:?}");
}

#[test]
fn escape_right_click_and_the_cancel_button_cancel() {
    for how in 0..3 {
        let mut r = rig("area");
        r.drag((200.0, 120.0), (520.0, 380.0));
        match how {
            0 => r.send(key(KeyCode::Escape, Modifiers::NONE)),
            1 => r.send(InputEvent::MouseDown { pos: (300.0, 200.0), button: MouseButton::Right, mods: Modifiers::NONE, clicks: 1 }),
            _ => {
                let c = r.item("cancel");
                r.click(c)
            }
        }
        assert!(r.calls().contains(&"capture_cancel".to_string()), "way {how}: {:?}", r.calls());
        assert_eq!(r.model().phase(), Phase::Committed);
    }
}

#[test]
fn enter_copy_save_and_double_click_commit_with_the_selection() {
    let cases: [(&str, Box<dyn Fn(&mut Rig)>); 5] = [
        ("both", Box::new(|r| r.send(key(KeyCode::Enter, Modifiers::NONE)))),
        ("copy", Box::new(|r| r.send(key(KeyCode::Char('C'), Modifiers::CTRL)))),
        ("save", Box::new(|r| r.send(key(KeyCode::Char('S'), Modifiers::CTRL)))),
        (
            "both",
            Box::new(|r| {
                r.send(mv(300.0, 200.0));
                r.send(InputEvent::MouseDown { pos: (300.0, 200.0), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
                r.send(up(300.0, 200.0));
                r.send(InputEvent::MouseDown { pos: (300.0, 200.0), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 2 });
                r.send(up(300.0, 200.0));
            }),
        ),
        (
            "record",
            Box::new(|r| {
                let c = r.item("record");
                r.click(c)
            }),
        ),
    ];
    for (action, f) in cases {
        let mut r = rig("area");
        r.drag((200.0, 120.0), (520.0, 380.0));
        f(&mut r);
        let a = r.commit_args().unwrap_or_else(|| panic!("{action}: no commit in {:?}", r.calls()));
        assert_eq!(a["action"], action);
        assert_eq!(a["label"], "capture-0");
        assert_eq!((a["vw"].as_f64(), a["vh"].as_f64()), (Some(960.0), Some(540.0)));
        assert_eq!(a["rect"], json!([200.0, 120.0, 320.0, 260.0]));
        assert!(r.raw.lock().unwrap().is_empty(), "no markup: the plain JSON commit");
    }
}

#[test]
fn a_drag_in_the_other_direction_and_resizing_work_with_the_handles() {
    let mut r = rig("area");
    r.drag((520.0, 380.0), (200.0, 120.0)); // bottom-right to top-left
    assert_eq!(r.model().selection().map(|s| (s.x, s.y, s.w, s.h)), Some((200.0, 120.0, 320.0, 260.0)));
    r.drag((520.0, 380.0), (600.0, 450.0)); // the se handle
    assert_eq!(r.model().selection().map(|s| (s.w, s.h)), Some((400.0, 330.0)));
    r.drag((300.0, 200.0), (320.0, 210.0)); // move
    assert_eq!(r.model().selection().map(|s| (s.x, s.y)), Some((220.0, 130.0)));
    assert_eq!(r.view().vm.cursor, cap::Cursor::Move, "inside with the select tool");
}

#[test]
fn the_cursor_follows_the_model_and_the_toolbar() {
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    r.sim.take_fx();
    r.send(mv(200.0, 250.0)); // the west edge
    assert_eq!(r.sim.take_fx().cursor, Some(CursorShape::ResizeEw));
    r.send(mv(360.0, 120.0));
    assert_eq!(r.sim.take_fx().cursor, Some(CursorShape::ResizeNs));
    r.send(mv(200.0, 120.0));
    assert_eq!(r.sim.take_fx().cursor, Some(CursorShape::Move), "corner: the host has no diagonal cursor yet");
    r.send(mv(300.0, 250.0));
    assert_eq!(r.sim.take_fx().cursor, Some(CursorShape::Move));
    let c = r.item("rect");
    r.send(mv(c.0, c.1));
    assert_eq!(r.sim.take_fx().cursor, Some(CursorShape::Arrow), "over the toolbar");
    r.send(mv(800.0, 500.0));
    assert_eq!(r.sim.take_fx().cursor, Some(CursorShape::Crosshair));
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Toolbar
// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn toolbar_hover_press_active_and_disabled_states_render_differently() {
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    r.sim.run(2000);
    let base = r.shot("04a-toolbar");
    // the select tool is active, undo is disabled (nothing drawn yet)
    let tb = r.view().vm.toolbar.clone().unwrap();
    let select = tb.items.iter().find(|i| i.id == "select").unwrap();
    assert!(select.on);
    assert!(tb.items.iter().find(|i| i.id == "undo").unwrap().disabled);
    // hover the rect tool: its square lightens within the transition
    let c = r.item("rect");
    r.sim.send(&mv(c.0, c.1));
    let until = r.sim.now_ms + 200.0;
    while r.sim.now_ms < until && r.sim.step_frame() {}
    let hovered = r.shot("04b-toolbar-hover");
    let spot = hovered.at(c.0 - 13.0, c.1 - 13.0);
    let before = base.at(c.0 - 13.0, c.1 - 13.0);
    assert!(lum(spot) > lum(before) + 12, "hover background: {before:?} -> {spot:?}");
    // the tip appears after the delay
    r.sim.now_ms += 700.0;
    r.sim.step_frame();
    let tipped = r.shot("04c-toolbar-tip");
    assert!(hovered.diff(&tipped, 8) > 300, "a tooltip is drawn under the bar");
    // press: scale .92 while held
    r.sim.send(&down(c.0, c.1));
    r.sim.run(2000);
    let pressed = r.shot("04d-toolbar-pressed");
    assert!(tipped.diff(&pressed, 8) > 20);
    r.sim.send(&up(c.0, c.1));
    r.sim.run(2000);
    assert_eq!(r.model().tool(), Tool::Rect, "the click chose the rectangle");
    // the colour swatches: pick the blue one
    let blue = r.item("#0a84ff");
    r.click(blue);
    assert_eq!(r.view().vm.color, "#0a84ff");
    let sw = r.shot("04e-toolbar-swatch");
    assert!(sw.diff(&pressed, 8) > 50);
    glasskit::ease::force_reduced(None);
}

#[test]
fn toolbar_clicks_do_not_start_a_new_selection() {
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    let before = r.model().selection();
    let c = r.item("arrow");
    r.click(c);
    assert_eq!(r.model().selection(), before);
    assert_eq!(r.model().tool(), Tool::Arrow);
}

#[test]
fn russian_tips_and_hint_come_from_the_language() {
    let mut r = rig_with("area", 960.0, 540.0, 1.0, Options::web_parity(), true);
    assert_eq!(r.view().vm.hint.as_ref().unwrap().text, "Выдели область · Esc — отмена");
    r.drag((200.0, 120.0), (520.0, 380.0));
    let v = r.view();
    let tb = v.vm.toolbar.as_ref().unwrap();
    assert_eq!(tb.items.iter().find(|i| i.id == "copy").unwrap().tip, "Копировать (Ctrl+C)");
    drop(v);
    r.shot("04f-russian");
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Markup
// ---------------------------------------------------------------------------------------------------------------------------------------

fn choose(r: &mut Rig, tool: &str) {
    let c = r.item(tool);
    r.click(c);
}

fn extras() -> Options {
    Options { extended_tools: true, ext_actions: true, edit_ops: true, ..Options::web_parity() }
}

#[test]
fn every_markup_tool_draws_and_stays_inside_the_selection() {
    let _m = motion_lock();
    let mut r = rig_with("area", 960.0, 540.0, 1.0, extras(), false);
    r.drag((120.0, 100.0), (760.0, 400.0));
    // rect (red), arrow, pen, blur, highlighter, ellipse, step, text
    choose(&mut r, "rect");
    r.drag((150.0, 120.0), (300.0, 200.0));
    choose(&mut r, "arrow");
    r.drag((320.0, 130.0), (460.0, 230.0));
    choose(&mut r, "pen");
    r.send(mv(480.0, 140.0));
    r.send(down(480.0, 140.0));
    for i in 0..30 {
        let t = i as f32 / 29.0;
        r.send(mv(480.0 + 120.0 * t, 140.0 + 40.0 * (t * 9.0).sin()));
    }
    r.send(up(600.0, 140.0));
    choose(&mut r, "blur");
    r.drag((150.0, 240.0), (330.0, 300.0));
    choose(&mut r, "highlighter");
    r.drag((350.0, 260.0), (520.0, 262.0));
    choose(&mut r, "ellipse");
    r.drag((540.0, 220.0), (700.0, 300.0));
    choose(&mut r, "step");
    r.click((160.0, 340.0));
    r.click((200.0, 340.0));
    choose(&mut r, "text");
    r.click((250.0, 330.0));
    for ch in "Hello, markup".chars() {
        r.send(InputEvent::Char(ch));
    }
    assert_eq!(r.model().text_input(), Some("Hello, markup"));
    let editing = r.shot("05a-markup-text-editing");
    r.send(key(KeyCode::Enter, Modifiers::NONE));
    assert!(r.model().text_input().is_none());
    // let every insert animation finish
    r.sim.now_ms += 2000.0;
    r.sim.run(2000);
    let img = r.shot("05-markup-all-tools");
    assert!(editing.diff(&img, 8) > 100);
    let ops = r.model().ops().len();
    assert_eq!(ops, 9, "rect, arrow, pen, blur, highlighter, ellipse, 2 steps, text -> {ops}");
    // red stroke of the rectangle's left edge (x = 150, y = 160) and nothing red outside the selection
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 90 && p[2] < 90;
    assert!(red(img.at(150.0, 160.0)) || red(img.at(149.5, 160.0)) || red(img.at(150.5, 160.0)), "rect edge: {:?}", img.at(150.0, 160.0));
    // the pixelated region is blocky: neighbouring pixels inside a block are identical
    assert_eq!(rgb(img.at(200.0, 270.0)), rgb(img.at(203.0, 272.0)), "pixelate: 10 px cells");
    // the step discs: red with a number
    let d = img.at(160.0, 340.0);
    assert!(d[0] > 150 || lum(d) > 300, "numbered disc: {d:?}");
    // undo removes the last op (the text)
    r.send(key(KeyCode::Char('Z'), Modifiers::CTRL));
    assert_eq!(r.model().ops().len(), ops - 1);
}

#[test]
fn drawing_is_clipped_to_the_selection() {
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    choose(&mut r, "rect");
    r.send(mv(220.0, 140.0));
    r.send(down(220.0, 140.0));
    r.send(mv(900.0, 500.0)); // far outside: clamped by the model, clipped by the view
    let img = r.shot("05b-clipped");
    r.send(up(900.0, 500.0));
    let red = |p: [u8; 4]| p[0] > 200 && p[1] < 90 && p[2] < 90;
    assert_eq!(img.count(Some(GRect::new(540.0, 0.0, 420.0, 540.0)), red), 0, "nothing red right of the selection");
    assert_eq!(img.count(Some(GRect::new(0.0, 440.0, 960.0, 100.0)), |p| red(p)), 0, "nothing red below it");
}

#[test]
fn the_select_tool_picks_and_deletes_an_op_with_extras_on() {
    let mut r = rig_with("area", 960.0, 540.0, 1.0, extras(), false);
    r.drag((120.0, 100.0), (760.0, 400.0));
    choose(&mut r, "rect");
    r.drag((200.0, 150.0), (400.0, 260.0));
    choose(&mut r, "select");
    r.click((300.0, 150.0)); // on the top edge of the rectangle
    assert!(r.view().vm.ops.iter().any(|o| o.selected));
    let img = r.shot("05c-op-selected");
    assert!(img.count(Some(GRect::new(190.0, 140.0, 220.0, 130.0)), |p| lum(p) > 700) > 30, "the dashed outline");
    r.send(key(KeyCode::Delete, Modifiers::NONE));
    assert!(r.model().ops().is_empty());
}

#[test]
fn pasting_and_typing_go_into_the_text_box_and_enter_commits_it() {
    let mut r = rig("area");
    r.drag((120.0, 100.0), (760.0, 400.0));
    choose(&mut r, "text");
    r.click((200.0, 200.0));
    r.fakes.clipboard.set_text("pasted\r\nline");
    r.send(key(KeyCode::Char('V'), Modifiers::CTRL));
    r.send(InputEvent::Char('!'));
    assert_eq!(r.model().text_input(), Some("pasted  line!"));
    r.send(key(KeyCode::Backspace, Modifiers::NONE));
    assert_eq!(r.model().text_input(), Some("pasted  line"));
    // Ctrl+C while the box is open must not copy the screenshot
    r.send(key(KeyCode::Char('C'), Modifiers::CTRL));
    assert!(r.commit_args().is_none());
    r.send(key(KeyCode::Escape, Modifiers::NONE));
    assert!(r.model().text_input().is_none(), "Esc closes the box first");
    assert!(r.commit_args().is_none() && !r.calls().contains(&"capture_cancel".to_string()));
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Commit with markup: the flattened PNG
// ---------------------------------------------------------------------------------------------------------------------------------------

fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut rd = dec.read_info().expect("a PNG");
    let mut buf = vec![0; rd.output_buffer_size().unwrap()];
    let info = rd.next_frame(&mut buf).expect("a frame");
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        other => panic!("unexpected {other:?}"),
    };
    (info.width, info.height, rgba)
}

#[test]
fn a_markup_commit_sends_the_flattened_png_as_a_raw_body() {
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    choose(&mut r, "rect");
    r.drag((240.0, 160.0), (400.0, 300.0));
    r.send(key(KeyCode::Enter, Modifiers::NONE));
    let raw = r.wait_raw();
    assert_eq!(raw.len(), 1, "one raw commit");
    let (caller, args, body) = &raw[0];
    assert_eq!(caller, "capture-0");
    assert_eq!(args["action"], "both");
    assert_eq!(args["rect"], json!([200.0, 120.0, 320.0, 260.0]));
    assert!(r.commit_args().is_none(), "the JSON form is not sent as well");
    let (w, h, px) = decode(body);
    assert_eq!((w, h), (320, 260), "the crop of the selection");
    // the rectangle's left edge in crop coordinates: x = 240 - 200 = 40, y = 40..180; stroke 3 px, red (#ff3b30)
    let at = |x: u32, y: u32| {
        let o = ((y * w + x) * 4) as usize;
        [px[o], px[o + 1], px[o + 2], px[o + 3]]
    };
    let e = at(40, 100);
    assert!(e[0] > 230 && e[1] < 90 && e[2] < 80 && e[3] == 255, "red stroke: {e:?}");
    // inside the rectangle and elsewhere: the frame's own pixel
    let f = |x: usize, y: usize| {
        let o = (y * 960 + x) * 4;
        [r.frame[o], r.frame[o + 1], r.frame[o + 2], 255]
    };
    assert_eq!(at(100, 100), f(300, 220), "inside the rect the frame is untouched");
    assert_eq!(at(5, 5), f(205, 125), "outside the op too");
}

#[test]
fn flatten_renders_every_command_kind_at_frame_resolution() {
    let (fw, fh) = (200i32, 160i32);
    let rgba: Vec<u8> = (0..fw * fh).flat_map(|_| [10u8, 20, 30, 255]).collect();
    let req = cap::io::FlattenRequest {
        src: CIRect { x: 20, y: 10, w: 100, h: 80 },
        cmds: vec![
            DrawCmd::StrokeRect { rect: cap::Rect::new(30.0, 20.0, 60.0, 40.0), color: "#ff3b30".into(), width: 4.0 },
            DrawCmd::Segments { segs: vec![(cap::Pt::new(30.0, 70.0), cap::Pt::new(100.0, 70.0))], color: "#34c759".into(), width: 4.0 },
            DrawCmd::Polyline { pts: vec![cap::Pt::new(30.0, 80.0), cap::Pt::new(100.0, 80.0)], color: "#0a84ff".into(), width: 4.0, alpha: 1.0 },
            DrawCmd::StrokeEllipse { rect: cap::Rect::new(60.0, 25.0, 50.0, 30.0), color: "#ffffff".into(), width: 3.0 },
            DrawCmd::Disc { center: cap::Pt::new(40.0, 50.0), radius: 8.0, color: "#ffcc00".into(), label: "3".into(), label_px: 12.0 },
            DrawCmd::Text { text: "Hi".into(), at: cap::Pt::new(92.0, 60.0), color: "#ffffff".into(), font_px: 16.0, weight: 600, outline: "rgba(0,0,0,.85)", outline_width: 3.0 },
        ],
    };
    unsafe {
        let _ = ::windows::Win32::System::Com::CoInitializeEx(None, ::windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let gfx = glasskit::gfx::Gfx::warp().unwrap();
    let png = flatten::render_png_on(&gfx, &rgba, fw, fh, &req).expect("png");
    assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let (w, h, px) = decode(&png);
    assert_eq!((w, h), (100, 80));
    let at = |x: u32, y: u32| {
        let o = ((y * w + x) * 4) as usize;
        [px[o], px[o + 1], px[o + 2], px[o + 3]]
    };
    assert_eq!(at(2, 2), [10, 20, 30, 255], "the untouched frame");
    assert!(at(10, 20)[0] > 230 && at(10, 20)[1] < 80, "rect edge (x=30-20, y=20+: 4 px wide): {:?}", at(10, 20));
    assert!(at(50, 60)[1] > 180 && at(50, 60)[0] < 80, "arrow shaft green: {:?}", at(50, 60));
    assert!(at(50, 70)[2] > 220 && at(50, 70)[0] < 60, "pen blue: {:?}", at(50, 70));
    assert!(at(14, 42)[0] > 200 && at(14, 42)[1] > 150, "yellow disc: {:?}", at(14, 42));
    assert!(w * h * 4 == px.len() as u32);
}

#[test]
fn pixelate_cells_average_blocks_and_are_stable() {
    let (fw, fh) = (40i32, 20i32);
    // left half black, right half white, block 20: two cells
    let rgba: Vec<u8> = (0..fh).flat_map(|_| (0..fw).flat_map(|x| if x < 20 { [0u8, 0, 0, 255] } else { [255, 255, 255, 255] })).collect();
    let (cw, ch, px) = tools::pixelate_cells(&rgba, fw, fh, CIRect { x: 0, y: 0, w: 40, h: 20 }, 20).unwrap();
    assert_eq!((cw, ch), (2, 1));
    assert_eq!(&px[..4], &[0, 0, 0, 255]);
    assert_eq!(&px[4..8], &[255, 255, 255, 255]);
    // a block that straddles the edge averages to grey
    let (cw, _, px) = tools::pixelate_cells(&rgba, fw, fh, CIRect { x: 10, y: 0, w: 20, h: 20 }, 20).unwrap();
    assert_eq!(cw, 1);
    assert!((px[0] as i32 - 127).abs() <= 24, "{}", px[0]);
    // out of range rectangles are clamped, empty ones refused
    assert!(tools::pixelate_cells(&rgba, fw, fh, CIRect { x: 100, y: 0, w: 10, h: 10 }, 6).is_none());
    assert!(tools::pixelate_cells(&rgba, fw, fh, CIRect { x: -10, y: -10, w: 20, h: 20 }, 6).is_some());
}

#[test]
fn colour_text_parsing() {
    assert_eq!(tools::parse_color("#ff3b30"), Color::rgb8(255, 59, 48));
    assert_eq!(tools::parse_color("#fff"), Color::rgb8(255, 255, 255));
    let c = tools::parse_color("rgba(0,0,0,.85)");
    assert_eq!((c.r, c.a), (0.0, 0.85));
    assert_eq!(tools::parse_color("nonsense"), Color::rgb8(0, 0, 0));
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Copy flash, exit, ghost
// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn copy_flashes_the_selection_and_the_overlay_closes_after_the_backend_ends_the_session() {
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    r.sim.run(2000);
    let calm = r.shot("06a-before-copy");
    r.sim.send(&key(KeyCode::Char('C'), Modifiers::CTRL));
    r.sim.step_frame();
    let flash = r.shot("06-copy-flash");
    let p = flash.at(490.0, 300.0);
    let q = calm.at(490.0, 300.0);
    assert!(lum(p) > lum(q) + 150, "white flash over the selection: {q:?} -> {p:?}");
    assert_eq!(r.model().phase(), Phase::Committed);
    // the backend answers with capture:end: the exit animations run, then the window hides itself
    r.sim.take_fx();
    r.sim.deliver(Msg::Event { name: "capture:end".into(), payload: json!({}), latest: false });
    assert!(!r.sim.take_fx().hide, "still animating: not hidden yet");
    r.sim.run(2000);
    assert!(r.sim.take_fx().hide, "hidden once the flash is done");
    glasskit::ease::force_reduced(None);
}

#[test]
fn cancel_closes_at_once_and_a_failed_command_does_not_trap_the_screen() {
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    r.send(key(KeyCode::Escape, Modifiers::NONE));
    r.sim.take_fx();
    r.sim.deliver(Msg::Event { name: "capture:end".into(), payload: json!({}), latest: false });
    r.sim.run(2000);
    assert!(r.sim.take_fx().hide, "no animation to wait for");
    // a commit whose backend call fails: the reply handler closes the overlay
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    r.send(key(KeyCode::Enter, Modifiers::NONE));
    r.sim.take_fx();
    r.sim.deliver(Msg::Reply { ticket: crate::ui::view::Ticket(1), result: Err("no capture in progress".into()) });
    assert!(r.sim.take_fx().hide);
    // and the stuck guard does the same when nothing ever answers
    let mut r = rig("area");
    r.drag((200.0, 120.0), (520.0, 380.0));
    r.send(key(KeyCode::Escape, Modifiers::NONE));
    r.sim.take_fx();
    r.sim.deliver(Msg::Custom(Box::new(SelfMsg::Stuck(1))));
    assert!(r.sim.take_fx().hide);
}

#[test]
fn pin_is_not_available_so_the_overlay_says_so() {
    let mut r = rig_with("area", 960.0, 540.0, 1.0, extras(), false);
    r.drag((200.0, 120.0), (520.0, 380.0));
    let c = r.item("pin");
    r.send(mv(c.0, c.1));
    r.sim.send(&down(c.0, c.1));
    r.sim.send(&up(c.0, c.1)); // no frames yet: the toast is fresh
    assert!(r.view().toast.as_ref().is_some_and(|t| t.text.contains("Pin")), "toast shown");
    // the shot is saved (the model's pin path commits a save)
    assert_eq!(r.commit_args().unwrap()["action"], "save");
    let until = r.sim.now_ms + 300.0;
    while r.sim.now_ms < until && r.sim.step_frame() {}
    let img = r.shot("06b-pin-toast");
    assert!(img.count(Some(GRect::new(330.0, 460.0, 300.0, 40.0)), |p| lum(p) > 600) > 30, "toast text");
    // after capture:end the window waits for the toast, then hides
    r.sim.take_fx();
    r.sim.deliver(Msg::Event { name: "capture:end".into(), payload: json!({}), latest: false });
    assert!(!r.sim.take_fx().hide);
}

#[test]
fn shelf_after_save_adds_the_saved_path() {
    let mut r = rig_with("area", 960.0, 540.0, 1.0, extras(), false);
    r.drag((200.0, 120.0), (520.0, 380.0));
    let c = r.item("shelf");
    r.click(c);
    r.sim.deliver(Msg::Event { name: "capture:done".into(), payload: json!({ "kind": "screenshot", "path": "C:\\shots\\a.png", "copied": true }), latest: false });
    let call = r.fakes.api.calls().into_iter().find(|c| c.name == "shelf_add").expect("shelf_add");
    assert_eq!(call.args, json!({ "paths": ["C:\\shots\\a.png"] }));
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Modes: text / qr / record select, colour picker
// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn ocr_qr_and_record_commit_on_release_without_a_toolbar() {
    for (mode, hint) in [("text", "Select the text"), ("qr", "Select a QR"), ("record", "Select the area to record")] {
        let mut r = rig(mode);
        assert!(r.view().vm.hint.as_ref().unwrap().text.starts_with(hint), "{mode}");
        r.drag((200.0, 120.0), (520.0, 380.0));
        let a = r.commit_args().unwrap_or_else(|| panic!("{mode}: {:?}", r.calls()));
        assert_eq!(a["action"], "run");
        assert!(r.view().vm.toolbar.is_none());
    }
}

#[test]
fn record_mode_enter_records_the_whole_screen() {
    let mut r = rig("record");
    r.send(key(KeyCode::Enter, Modifiers::NONE));
    let a = r.commit_args().expect("commit");
    assert_eq!(a["action"], "run");
    assert!(a.get("rect").is_none(), "no selection = the whole screen: {a}");
}

#[test]
fn colour_mode_shows_the_loupe_follows_the_pointer_and_picks() {
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = rig("color");
    assert!(r.view().vm.loupe.is_some() && r.view().vm.cursor_hidden);
    r.send(mv(300.0, 200.0));
    let l = r.view().vm.loupe.clone().unwrap();
    assert_eq!(l.sample, (300, 200));
    assert!(l.hex.starts_with('#') && l.rgb.starts_with("rgb(") && l.hsl.starts_with("hsl("));
    assert_eq!(l.cells.len(), 225);
    r.sim.run(2000);
    let img = r.shot("07-loupe");
    // the lens box: dark panel with the 15 x 15 grid; the centre cell is outlined in white
    let b = l.box_pos;
    let panel = img.at(b.x as f32 + 3.0, b.y as f32 + 100.0);
    assert!(lum(panel) < 150, "lens background: {panel:?}");
    let grid_px = img.at(b.x as f32 + 7.0 + 4.5, b.y as f32 + 7.0 + 4.5);
    let o = ((200 - 7) * 960 + (300 - 7)) * 4;
    assert!(grid_px[0].abs_diff(r.frame[o]) < 6, "first cell is the pixel 7 left and up of the sample");
    // the marker square around the sample
    assert!(img.at(300.5 - 7.0, 200.5)[0] > 200 || img.at(300.5 - 6.5, 200.5)[0] > 200, "marker edge");
    // arrows move the sample by one pixel
    r.send(key(KeyCode::Right, Modifiers::NONE));
    assert_eq!(r.view().vm.loupe.as_ref().unwrap().sample, (301, 200));
    // a click picks: the commit carries the pixel centre in css px
    r.click((300.0, 200.0));
    let a = r.commit_args().unwrap();
    assert_eq!(a["action"], "pick");
    assert_eq!((a["x"].as_f64(), a["y"].as_f64()), (Some(300.5), Some(200.5)));
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_loupe_lens_follows_with_an_ease_not_a_jump() {
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = rig("color");
    r.sim.run(2000);
    r.send(mv(100.0, 100.0));
    r.sim.run(2000);
    r.sim.send(&mv(500.0, 300.0));
    r.sim.step_frame();
    let a = r.sim.render((960.0, 540.0), &Theme::pill_dark());
    r.sim.run(2000);
    let b = r.sim.render((960.0, 540.0), &Theme::pill_dark());
    assert!(a.diff(&b, 4) > 500, "the lens is still travelling one frame after the move");
    glasskit::ease::force_reduced(None);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Multi-monitor, DPI, reduced motion
// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn two_monitors_with_mixed_dpi_and_offsets_each_arm_for_their_own_label() {
    use cap::{Desktop, MonitorGeom};
    // a 1920x1080 @100 % primary and a 2880x1620 @150 % monitor to its left and a bit higher (negative coordinates)
    let desktop = Desktop { monitors: vec![MonitorGeom::new(0, 0, 0, 1920, 1080, 1.0), MonitorGeom::new(1, -2880, -300, 2880, 1620, 1.5)] };
    assert_eq!(desktop.by_label("capture-1").unwrap().viewport(), (1920.0, 1080.0));
    let mut views = Vec::new();
    for m in &desktop.monitors {
        let (vw, vh) = m.viewport();
        // keep the test light: render at a quarter of the real size, same ratios
        let (qw, qh) = ((vw / 4.0) as f32, (vh / 4.0) as f32);
        let sc = m.scale as f32;
        let (fw, fh) = ((qw * sc).round() as u32, (qh * sc).round() as u32);
        let frame = Arc::new(desktop_frame(fw, fh));
        let r = rig_frame("area", qw, qh, sc, Options::web_parity(), false, frame, 7);
        r.fakes.api.reply("capture_session", Ok(json!({ "id": 7, "mode": "area", "w": fw, "h": fh, "x": m.x, "y": m.y, "scale": m.scale, "focus": m.label == "capture-0" })));
        views.push((m.clone(), r));
    }
    // the selection starts on monitor 1 (the offset one); monitor 0 stays dimmed and untouched
    let (_, r1) = &mut views[1];
    r1.drag((100.0, 60.0), (330.0, 190.0));
    let sel = r1.model().selection().unwrap();
    assert_eq!((sel.w, sel.h), (230.0, 130.0));
    let sc = r1.model().scale();
    assert!((sc.rx - 1.5).abs() < 0.01, "css -> frame scale of the 150 % monitor: {sc:?}");
    r1.send(key(KeyCode::Enter, Modifiers::NONE));
    let a = r1.commit_args().unwrap();
    assert_eq!(a["label"], "capture-0", "each rig is its own window: the label the rig was built with");
    assert_eq!(a["rect"], json!([100.0, 60.0, 230.0, 130.0]));
    // desktop mapping: the pointer in monitor 1's css px maps to the right physical desktop point
    let m1 = &desktop.monitors[1];
    let p = m1.view_to_phys(cap::Pt::new(100.0, 60.0));
    assert_eq!((p.x, p.y), (-2880.0 + 150.0, -300.0 + 90.0));
    assert_eq!(desktop.nearest(cap::Pt::new(-10.0, 5.0)).unwrap().label, "capture-1");
    // snapshots of both side by side
    let (_, r0) = &mut views[0];
    r0.sim.run(2000);
    let i0 = r0.shot("08a-monitor0-idle");
    let (_, r1) = &mut views[1];
    let i1 = r1.shot("08b-monitor1-after-commit");
    assert!(i0.w > 0 && i1.w > 0);
}

#[test]
fn a_hidpi_overlay_renders_at_device_resolution() {
    let _m = motion_lock();
    let mut r = rig_with("area", 480.0, 270.0, 2.0, Options::web_parity(), false);
    r.drag((100.0, 60.0), (330.0, 190.0));
    r.sim.run(2000);
    let img = r.shot("09-dpi-2x");
    assert_eq!((img.w, img.h), (960, 540));
    // handles are 9 logical = 18 device px discs
    let h = r.view().vm.selection.as_ref().unwrap().handles[0];
    assert!(img.at(h.at.x as f32, h.at.y as f32)[0] > 240);
    assert!((r.model().scale().rx - 2.0).abs() < 1e-6);
}

#[test]
fn reduced_motion_settles_without_frames_and_nothing_animates() {
    let _m = motion_lock();
    let mut r = rig_with("area", 960.0, 540.0, 1.0, Options { reduce_motion: true, ..Options::web_parity() }, false);
    r.sim.run(50);
    r.drag((200.0, 120.0), (520.0, 380.0));
    assert!(r.sim.run(600) <= 2, "the model's animations are instant");
    assert!(!r.sim.frame.wants_frame(), "idle = no frames");
    let a = r.shot("10-reduced-motion");
    let c = r.item("rect");
    r.send(mv(c.0, c.1));
    r.sim.run(600);
    let hovered = r.sim.render((960.0, 540.0), &Theme::pill_dark());
    assert!(lum(hovered.at(c.0 - 13.0, c.1 - 13.0)) > lum(a.at(c.0 - 13.0, c.1 - 13.0)), "the hover state is applied at once");
    assert!(r.view().fx.values().all(|f| f.hover.finished()));
}

#[test]
fn an_idle_overlay_costs_no_frames() {
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = rig("area");
    r.sim.run(2000);
    assert!(!r.sim.frame.wants_frame());
    let n = r.sim.frames_run;
    for _ in 0..10 {
        r.sim.step_frame();
    }
    assert_eq!(r.sim.frames_run, n, "no timer, no frames");
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_model_follows_a_resized_window() {
    let mut r = rig("area");
    let img = r.sim.render((1280.0, 720.0), &Theme::pill_dark());
    assert_eq!(r.model().viewport(), (1280.0, 720.0));
    assert!(img.w > 0);
}

#[test]
fn key_mapping_is_exact() {
    assert_eq!(map_key(KeyCode::Escape, Modifiers::NONE, false), Some(Key::Escape));
    assert_eq!(map_key(KeyCode::Char('Z'), Modifiers::CTRL, false), Some(Key::Char('z')));
    assert_eq!(map_key(KeyCode::Char('Z'), Modifiers::NONE, false), None, "plain letters are not shortcuts");
    assert_eq!(map_key(KeyCode::Char('Z'), Modifiers::CTRL, true), None, "typing text: no shortcuts");
    assert_eq!(map_key(KeyCode::Space, Modifiers::NONE, true), None);
    assert_eq!(map_key(KeyCode::Left, Modifiers::SHIFT, false), Some(Key::Left));
}

#[test]
fn commit_kinds_map_to_the_backend_words() {
    assert_eq!(CommitKind::Both.as_str(), "both");
    assert_eq!(CommitKind::Run.as_str(), "run");
    assert_eq!(CommitKind::Pick.as_str(), "pick");
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Scenes for `tools/dev/capture-diff.mjs`: the same scripts run in the web page, then both PNGs are compared
// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn web_parity_scenes() {
    let Ok(dir) = std::env::var("PILL_CAPTURE_DUMP") else { return };
    let _m = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let theme = Theme::pill_dark();
    let save = |name: &str, r: &mut Rig| {
        r.sim.now_ms += 3000.0;
        r.sim.run(2000);
        let img = r.sim.render((960.0, 540.0), &theme);
        crate::ui::host::testing::save_png(&img, std::path::Path::new(&dir).join(format!("w-{name}.png")));
    };
    let mut r = rig("area");
    // the frame itself, for the web page's stub
    let mut px = Vec::new();
    for p in r.frame.chunks_exact(4) {
        px.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    let img = Image { w: 960, h: 540, scale: 1.0, bgra: px };
    crate::ui::host::testing::save_png(&img, std::path::Path::new(&dir).join("frame.png"));
    save("idle", &mut r);
    r.drag((200.0, 120.0), (520.0, 380.0));
    save("adjusting", &mut r);
    let c = r.item("rect");
    r.send(mv(c.0, c.1));
    save("hover", &mut r);

    let mut r = rig("area");
    r.drag((120.0, 100.0), (760.0, 400.0));
    choose(&mut r, "rect");
    r.drag((150.0, 120.0), (300.0, 200.0));
    choose(&mut r, "arrow");
    r.drag((320.0, 130.0), (460.0, 230.0));
    choose(&mut r, "pen");
    r.send(mv(480.0, 140.0));
    r.send(down(480.0, 140.0));
    for i in 0..30 {
        let t = i as f32 / 29.0;
        r.send(mv(480.0 + 120.0 * t, 140.0 + 40.0 * (t * 9.0).sin()));
    }
    r.send(up(600.0, 140.0));
    choose(&mut r, "blur");
    r.drag((150.0, 240.0), (330.0, 300.0));
    choose(&mut r, "text");
    r.click((250.0, 330.0));
    for ch in "Hello, markup".chars() {
        r.send(InputEvent::Char(ch));
    }
    r.send(key(KeyCode::Enter, Modifiers::NONE));
    r.send(mv(900.0, 520.0));
    save("markup", &mut r);

    let mut r = rig("color");
    r.send(mv(300.0, 200.0));
    save("loupe", &mut r);
    glasskit::ease::force_reduced(None);
}
