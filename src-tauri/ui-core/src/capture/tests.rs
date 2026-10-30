//! Tests of the capture model: geometry (incl. the capture.rs mapping tests), markup draw lists, loupe, toolbar metrics, the
//! overlay state machine driven through `FakeIo`, animations (and Reduce motion), result cards and the recording capsule.
//! Synthetic frames only: nothing here touches a screen, a window or the backend.
use super::geom::*;
use super::input::*;
use super::io::*;
use super::live::{self, CaptureDone, RecEvent, RecState, Tone};
use super::loupe::{self, Loupe};
use super::markup::*;
use super::model::*;
use super::toolbar::{self, ItemId, Toolbar, ToolbarState};
use super::anim;
use super::view::ClipInset;
use super::AnimKind;
use crate::i18n::{self, Lang, LangTr};
use serde_json::json;
use std::sync::Arc;

const RU: LangTr = LangTr { lang: Lang::Ru };
const EN: LangTr = LangTr { lang: Lang::En };

fn pt(x: f64, y: f64) -> Pt {
    Pt::new(x, y)
}

// ------------------------------------------------------------------------------------------------- harness

fn rig(vw: f64, vh: f64, fw: i32, fh: i32, mode: &str, opts: Options) -> (CaptureModel, FakeIo) {
    let mut io = FakeIo::with_session(1, mode, fw, fh);
    let mut m = CaptureModel::new("capture-0", vw, vh, opts);
    assert_eq!(m.arm(&mut io), ArmOutcome::Armed);
    (m, io)
}

fn area() -> (CaptureModel, FakeIo) {
    rig(1920.0, 1080.0, 1920, 1080, "area", Options::default())
}

fn down(m: &mut CaptureModel, io: &mut FakeIo, x: f64, y: f64) -> Reaction {
    m.on_input(Input::PointerDown { pos: pt(x, y), button: Button::Left, mods: Mods::NONE }, io)
}
fn mv(m: &mut CaptureModel, io: &mut FakeIo, x: f64, y: f64) -> Reaction {
    m.on_input(Input::PointerMove { pos: pt(x, y), mods: Mods::NONE }, io)
}
fn up(m: &mut CaptureModel, io: &mut FakeIo, x: f64, y: f64) -> Reaction {
    m.on_input(Input::PointerUp { pos: pt(x, y), button: Button::Left, mods: Mods::NONE }, io)
}
fn drag(m: &mut CaptureModel, io: &mut FakeIo, a: (f64, f64), b: (f64, f64)) {
    down(m, io, a.0, a.1);
    mv(m, io, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    mv(m, io, b.0, b.1);
    up(m, io, b.0, b.1);
}
fn key(m: &mut CaptureModel, io: &mut FakeIo, k: Key, mods: Mods) -> Reaction {
    m.on_input(Input::Key { key: k, mods }, io)
}
fn ch(m: &mut CaptureModel, io: &mut FakeIo, c: char, mods: Mods) -> Reaction {
    key(m, io, Key::Char(c), mods)
}
/// Click a toolbar item by id at its centre.
fn click_item(m: &mut CaptureModel, io: &mut FakeIo, id: &str) {
    let v = m.view(&EN);
    let bar = v.toolbar.expect("toolbar visible");
    let it = bar.items.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("no item {id}"));
    let p = pt(bar.rect.x + it.rect.x + it.rect.w / 2.0, bar.rect.y + it.rect.y + it.rect.h / 2.0);
    down(m, io, p.x, p.y);
    up(m, io, p.x, p.y);
}
fn commit_args(io: &FakeIo) -> serde_json::Value {
    io.last("capture_commit").expect("a capture_commit call").args.clone()
}

// ------------------------------------------------------------------------------------------------- geom

#[test]
fn rect_normalisation_across_dpi() {
    // the same expectations as the unit test of src-tauri/src/capture.rs
    let r = |x, y, w, h| Some(IRect { x, y, w, h });
    assert_eq!(to_frame_rect([100.0, 50.0, 300.0, 200.0], 1920.0, 1080.0, 1920, 1080), r(100, 50, 300, 200));
    assert_eq!(to_frame_rect([100.0, 50.0, 300.0, 200.0], 2560.0, 1440.0, 3840, 2160), r(150, 75, 450, 300));
    assert_eq!(to_frame_rect([400.0, 250.0, -300.0, -200.0], 1920.0, 1080.0, 1920, 1080), r(100, 50, 300, 200));
    assert_eq!(to_frame_rect([-50.0, -50.0, 5000.0, 5000.0], 1920.0, 1080.0, 1920, 1080), r(0, 0, 1920, 1080));
    assert_eq!(to_frame_rect([10.0, 10.0, 1.0, 40.0], 1920.0, 1080.0, 1920, 1080), None);
    assert_eq!(to_frame_rect([f64::NAN, 0.0, 10.0, 10.0], 1920.0, 1080.0, 1920, 1080), None);
    assert_eq!(to_frame_rect([0.0, 0.0, 10.0, 10.0], 0.0, 0.0, 1920, 1080), None);
    assert_eq!(to_frame_point(1919.9, 0.0, 1920.0, 1080.0, 1920, 1080), Some((1919, 0)));
    assert_eq!(to_frame_point(100.0, 100.0, 2560.0, 1440.0, 3840, 2160), Some((150, 150)));
    assert_eq!(to_frame_point(f64::INFINITY, 0.0, 10.0, 10.0, 10, 10), None);
}

#[test]
fn hit_test_priorities_and_tolerance() {
    let s = Rect::new(100.0, 100.0, 200.0, 100.0);
    assert_eq!(hit(&s, pt(100.0, 100.0)), Hit::Edge(Handle::Nw));
    assert_eq!(hit(&s, pt(108.0, 92.0)), Hit::Edge(Handle::Nw), "exactly 8 px away still hits");
    assert_eq!(hit(&s, pt(109.0, 100.0)), Hit::Edge(Handle::N), "9 px from the left edge is no corner any more");
    assert_eq!(hit(&s, pt(300.0, 100.0)), Hit::Edge(Handle::Ne));
    assert_eq!(hit(&s, pt(100.0, 200.0)), Hit::Edge(Handle::Sw));
    assert_eq!(hit(&s, pt(300.0, 200.0)), Hit::Edge(Handle::Se));
    assert_eq!(hit(&s, pt(200.0, 100.0)), Hit::Edge(Handle::N));
    assert_eq!(hit(&s, pt(200.0, 200.0)), Hit::Edge(Handle::S));
    assert_eq!(hit(&s, pt(100.0, 150.0)), Hit::Edge(Handle::W));
    assert_eq!(hit(&s, pt(300.0, 150.0)), Hit::Edge(Handle::E));
    assert_eq!(hit(&s, pt(200.0, 150.0)), Hit::Inside);
    assert_eq!(hit(&s, pt(50.0, 50.0)), Hit::Outside);
    assert_eq!(hit(&s, pt(200.0, 300.0)), Hit::Outside);
    // handle metadata
    assert_eq!(Handle::Nw.cursor(), Cursor::NwseResize);
    assert_eq!(Handle::Ne.cursor(), Cursor::NeswResize);
    assert_eq!(Handle::N.cursor().css(), "ns-resize");
    assert_eq!(Handle::W.cursor().css(), "ew-resize");
    assert_eq!(Handle::Se.anchor(&s), pt(300.0, 200.0));
    assert_eq!(Handle::N.anchor(&s), pt(200.0, 100.0));
    assert_eq!(Handle::W.anchor(&s), pt(100.0, 150.0));
    assert_eq!(HANDLES.len(), 8);
    let names: Vec<_> = HANDLES.iter().map(|h| h.name()).collect();
    assert_eq!(names, ["nw", "n", "ne", "e", "se", "s", "sw", "w"]);
}

#[test]
fn move_resize_flip_and_clamp() {
    let o = Rect::new(100.0, 100.0, 200.0, 100.0);
    assert_eq!(move_selection(&o, pt(150.0, 150.0), pt(170.0, 160.0), 1920.0, 1080.0), Rect::new(120.0, 110.0, 200.0, 100.0));
    assert_eq!(move_selection(&o, pt(150.0, 150.0), pt(-500.0, 5000.0), 1920.0, 1080.0), Rect::new(0.0, 980.0, 200.0, 100.0), "kept inside the viewport");
    assert_eq!(resize_selection(&o, Handle::E, pt(400.0, 999.0)), Rect::new(100.0, 100.0, 300.0, 100.0));
    assert_eq!(resize_selection(&o, Handle::Nw, pt(50.0, 60.0)), Rect::new(50.0, 60.0, 250.0, 140.0));
    // dragging the east edge past the west edge flips the box
    assert_eq!(resize_selection(&o, Handle::E, pt(40.0, 150.0)), Rect::new(40.0, 100.0, 60.0, 100.0));
    assert_eq!(Rect::from_corners(pt(5.0, 9.0), pt(1.0, 3.0)), Rect::new(1.0, 3.0, 4.0, 6.0));
    assert!(Rect::new(1.0, 1.0, 0.0, 5.0).is_empty());
}

#[test]
fn square_and_aspect_lock() {
    let a = pt(100.0, 100.0);
    assert_eq!(lock_ratio(a, pt(300.0, 150.0), 1.0), pt(300.0, 300.0), "the larger drag decides");
    assert_eq!(lock_ratio(a, pt(130.0, 400.0), 1.0), pt(400.0, 400.0));
    assert_eq!(lock_ratio(a, pt(0.0, 50.0), 1.0), pt(0.0, 0.0), "up-left keeps its signs");
    assert_eq!(lock_ratio(a, pt(260.0, 120.0), 16.0 / 9.0), pt(260.0, 100.0 + 160.0 * 9.0 / 16.0));
    assert_eq!(lock_ratio(a, pt(300.0, 300.0), 0.0), pt(300.0, 300.0), "bad ratio = free");
    let o = Rect::new(100.0, 100.0, 200.0, 100.0);
    let r = resize_locked(&o, Handle::Se, pt(400.0, 220.0), 1.0);
    assert_eq!((r.w, r.h), (300.0, 300.0));
    assert_eq!(resize_locked(&o, Handle::E, pt(400.0, 220.0), 1.0), resize_selection(&o, Handle::E, pt(400.0, 220.0)), "edges stay free");
    let nw = resize_locked(&o, Handle::Nw, pt(50.0, 90.0), 1.0);
    assert_eq!((nw.w, nw.h), (250.0, 250.0));
    assert_eq!((nw.right(), nw.bottom()), (300.0, 200.0), "the opposite corner is the anchor");
}

#[test]
fn nudge_is_one_physical_pixel_on_any_dpi() {
    let s = Rect::new(100.0, 100.0, 200.0, 100.0);
    let hi = Scale::new(3840, 2160, 2560.0, 1440.0); // 150 %
    let n = nudge(&s, 1, 0, false, hi, 2560.0, 1440.0);
    assert!((n.x - (100.0 + 1.0 / 1.5)).abs() < 1e-9);
    let big = nudge(&s, 0, 1, true, Scale::IDENTITY, 1920.0, 1080.0);
    assert_eq!(big.y, 110.0);
    assert_eq!(nudge(&s, -1, 0, true, Scale::IDENTITY, 1920.0, 1080.0).x, 90.0);
    assert_eq!(nudge(&Rect::new(0.0, 0.0, 10.0, 10.0), -1, -1, false, Scale::IDENTITY, 100.0, 100.0), Rect::new(0.0, 0.0, 10.0, 10.0), "stays inside");
    let g = nudge_size(&s, 1, 1, true, Scale::IDENTITY, 1920.0, 1080.0);
    assert_eq!((g.w, g.h), (210.0, 110.0));
    let tiny = nudge_size(&Rect::new(10.0, 10.0, 2.0, 2.0), -1, -1, true, Scale::IDENTITY, 100.0, 100.0);
    assert_eq!((tiny.w, tiny.h), (1.0, 1.0), "never below one pixel");
    let wide = nudge_size(&Rect::new(1900.0, 10.0, 15.0, 10.0), 1, 0, true, Scale::IDENTITY, 1920.0, 1080.0);
    assert_eq!(wide.right(), 1920.0, "cannot leave the viewport");
}

#[test]
fn snap_pulls_edges_onto_guides() {
    let s = Snap { threshold: 6.0, guides_x: vec![500.0], guides_y: vec![] };
    let r = s.snap_move(&Rect::new(4.0, 300.0, 100.0, 50.0), 1920.0, 1080.0);
    assert_eq!(r.x, 0.0, "left edge onto the viewport edge");
    assert_eq!(r.y, 300.0);
    let r = s.snap_move(&Rect::new(397.0, 1030.0, 100.0, 47.0), 1920.0, 1080.0);
    assert_eq!(r.right(), 500.0, "right edge onto the extra guide");
    assert_eq!(r.bottom(), 1080.0);
    assert_eq!(s.snap_point(pt(503.0, 1076.0), 1920.0, 1080.0), pt(500.0, 1080.0));
    assert_eq!(s.snap_point(pt(520.0, 500.0), 1920.0, 1080.0), pt(520.0, 500.0), "outside the threshold nothing moves");
}

#[test]
fn export_rect_rounds_and_cuts_at_the_frame() {
    let sc = Scale::new(3840, 2160, 2560.0, 1440.0);
    assert_eq!(export_rect(&Rect::new(100.0, 50.0, 300.0, 200.0), sc, 3840, 2160), IRect { x: 150, y: 75, w: 450, h: 300 });
    assert_eq!(export_rect(&Rect::new(2500.0, 1400.0, 100.0, 100.0), sc, 3840, 2160), IRect { x: 3750, y: 2100, w: 90, h: 60 });
    assert_eq!(export_rect(&Rect::new(10.0, 10.0, 0.1, 0.1), Scale::IDENTITY, 100, 100), IRect { x: 10, y: 10, w: 1, h: 1 });
    assert_eq!(Scale::new(10, 10, 0.0, 0.0), Scale { rx: 10.0, ry: 10.0 }, "viewport clamps to 1 like Math.max(1, innerWidth)");
}

#[test]
fn monitors_with_mixed_dpi_and_negative_origin() {
    // a 4K 150 % screen to the left of the primary 1080p 100 % one
    let d = Desktop {
        monitors: vec![MonitorGeom::new(0, 0, 0, 1920, 1080, 1.0), MonitorGeom::new(1, -3840, -540, 3840, 2160, 1.5)],
    };
    assert_eq!(d.bounds(), Some(IRect { x: -3840, y: -540, w: 5760, h: 2160 }));
    assert_eq!(d.monitors[0].label, "capture-0");
    assert_eq!(Desktop::index_of("capture-1"), Some(1));
    assert_eq!(Desktop::index_of("pill"), None);
    assert_eq!(d.monitors[1].viewport(), (2560.0, 1440.0));
    assert_eq!(d.monitor_at(pt(-1.0, 0.0)).map(|m| m.label.as_str()), Some("capture-1"));
    assert_eq!(d.monitor_at(pt(0.0, 0.0)).map(|m| m.label.as_str()), Some("capture-0"));
    assert!(d.monitor_at(pt(-100.0, 1700.0)).is_none(), "below the left screen, left of the primary: a gap");
    assert_eq!(d.nearest(pt(-100.0, 1700.0)).map(|m| m.label.as_str()), Some("capture-1"));
    let m1 = d.by_label("capture-1").unwrap();
    let phys = pt(-3000.0, 60.0);
    let v = m1.phys_to_view(phys);
    assert_eq!(v, pt(560.0, 400.0));
    assert_eq!(m1.view_to_phys(v), phys);
    // a selection made on the 150 % screen -> frame px -> desktop px for the recorder
    let css = [100.0, 50.0, 300.0, 200.0];
    let f = to_frame_rect(css, 2560.0, 1440.0, 3840, 2160).unwrap();
    assert_eq!(m1.frame_to_screen(f), IRect { x: -3840 + 150, y: -540 + 75, w: 450, h: 300 });
    assert!(Desktop::default().bounds().is_none());
}

// ------------------------------------------------------------------------------------------------- markup

#[test]
fn tools_and_palette_constants() {
    assert_eq!(COLORS, ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff"]);
    let ids: Vec<_> = CORE_TOOLS.iter().map(|t| t.id()).collect();
    assert_eq!(ids, ["select", "rect", "arrow", "pen", "blur", "text"]);
    for t in CORE_TOOLS.iter().chain(EXTRA_TOOLS.iter()) {
        assert_eq!(Tool::from_id(t.id()), Some(*t));
    }
    assert_eq!(Tool::from_id("lasso"), None);
    assert!(Tool::Rect.is_core() && !Tool::Step.is_core());
    assert!(Tool::Pen.drags() && !Tool::Text.drags() && !Tool::Select.drags());
}

#[test]
fn draw_commands_follow_drawop() {
    let sc = Scale { rx: 1.5, ry: 1.5 };
    let rect = Op { id: 1, color: "#ff3b30".into(), kind: OpKind::Rect { a: pt(50.0, 80.0), b: pt(10.0, 20.0) } };
    assert_eq!(rect.draw_cmds(800, 600, sc), vec![DrawCmd::StrokeRect { rect: Rect::new(10.0, 20.0, 40.0, 60.0), color: "#ff3b30".into(), width: 4.5 }]);
    // arrow: shaft + two wings, head length max(14*rx, lw*4) = 21 capped at 0.6 * length
    let arrow = Op { id: 2, color: "#fff".into(), kind: OpKind::Arrow { a: pt(0.0, 0.0), b: pt(100.0, 0.0) } };
    let DrawCmd::Segments { segs, width, .. } = &arrow.draw_cmds(800, 600, sc)[0] else { panic!("segments") };
    assert_eq!(*width, 4.5);
    assert_eq!(segs.len(), 3);
    assert_eq!(segs[0], (pt(0.0, 0.0), pt(100.0, 0.0)));
    let w = segs[1].1;
    assert!((w.x - (100.0 - 21.0 * 0.45f64.cos())).abs() < 1e-9 && (w.y - (21.0 * 0.45f64.sin())).abs() < 1e-9, "wing {w:?}");
    let short = Op { id: 3, color: "#fff".into(), kind: OpKind::Arrow { a: pt(0.0, 0.0), b: pt(10.0, 0.0) } };
    let DrawCmd::Segments { segs, .. } = &short.draw_cmds(800, 600, sc)[0] else { panic!() };
    let head = ((segs[1].0.x - segs[1].1.x).powi(2) + (segs[1].0.y - segs[1].1.y).powi(2)).sqrt();
    assert!((head - 6.0).abs() < 1e-9, "head capped at 60 % of a short arrow, got {head}");
    // pen: a single point stays a (zero-length) polyline so a click leaves a dot
    let dot = Op { id: 4, color: "#000".into(), kind: OpKind::Pen { pts: vec![pt(5.0, 5.0)] } };
    assert!(matches!(&dot.draw_cmds(10, 10, Scale::IDENTITY)[0], DrawCmd::Polyline { pts, width, alpha, .. } if pts.len() == 1 && *width == 3.0 && *alpha == 1.0));
    // pixelate: block = max(6, round(10*rx)); skipped when <= 1 px; clipped to the frame
    let blur = Op { id: 5, color: "#000".into(), kind: OpKind::Blur { a: pt(10.0, 10.0), b: pt(110.0, 60.0) } };
    assert_eq!(blur.draw_cmds(800, 600, sc), vec![DrawCmd::Pixelate { rect: IRect { x: 10, y: 10, w: 100, h: 50 }, block: 15 }]);
    assert_eq!(blur.draw_cmds(800, 600, Scale::IDENTITY), vec![DrawCmd::Pixelate { rect: IRect { x: 10, y: 10, w: 100, h: 50 }, block: 10 }]);
    assert!(matches!(Op { kind: OpKind::Blur { a: pt(0.0, 0.0), b: pt(1.0, 40.0) }, ..blur.clone() }.draw_cmds(800, 600, sc).as_slice(), []));
    let edge = Op { kind: OpKind::Blur { a: pt(780.0, 10.0), b: pt(900.0, 60.0) }, ..blur.clone() };
    assert_eq!(edge.draw_cmds(800, 600, Scale::IDENTITY), vec![DrawCmd::Pixelate { rect: IRect { x: 780, y: 10, w: 20, h: 50 }, block: 10 }]);
    assert!(DrawCmd::Pixelate { rect: IRect::default(), block: 6 } != blur.draw_cmds(800, 600, Scale::IDENTITY)[0]);
    // text: font 16*rx, weight 600, outline max(3, 3*rx) in rgba(0,0,0,.85)
    let text = Op { id: 6, color: "#34c759".into(), kind: OpKind::Text { s: "hi".into(), a: pt(3.0, 4.0) } };
    let DrawCmd::Text { font_px, weight, outline, outline_width, .. } = &text.draw_cmds(800, 600, sc)[0] else { panic!() };
    assert_eq!((*font_px, *weight, *outline, *outline_width), (24.0, 600, "rgba(0,0,0,.85)", 4.5));
    let DrawCmd::Text { outline_width, .. } = &text.draw_cmds(800, 600, Scale::IDENTITY)[0] else { panic!() };
    assert_eq!(*outline_width, 3.0);
    // extensions
    let hl = Op { id: 7, color: "#ffcc00".into(), kind: OpKind::Highlighter { pts: vec![pt(0.0, 0.0), pt(9.0, 0.0)] } };
    assert!(matches!(&hl.draw_cmds(10, 10, Scale::IDENTITY)[0], DrawCmd::Polyline { width, alpha, .. } if *width == 15.0 && (*alpha - 0.35).abs() < 1e-9));
    let step = Op { id: 8, color: "#ff3b30".into(), kind: OpKind::Step { n: 3, c: pt(40.0, 40.0) } };
    assert!(matches!(&step.draw_cmds(100, 100, Scale::IDENTITY)[0], DrawCmd::Disc { radius, label, .. } if *radius == 12.0 && label == "3"));
    let ell = Op { id: 9, color: "#000".into(), kind: OpKind::Ellipse { a: pt(0.0, 0.0), b: pt(20.0, 10.0) } };
    assert!(matches!(ell.draw_cmds(100, 100, Scale::IDENTITY)[0], DrawCmd::StrokeEllipse { .. }));
}

#[test]
fn drafts_boundary_is_strictly_more_than_four() {
    let mut a = Op::draft(1, Tool::Arrow, "#000", pt(0.0, 0.0)).unwrap();
    a.extend(pt(4.0, 0.0));
    assert!(!a.is_substantial(), "length 4 is dropped (`> 4`)");
    a.extend(pt(4.1, 0.0));
    assert!(a.is_substantial());
    let pen = Op::draft(2, Tool::Pen, "#000", pt(0.0, 0.0)).unwrap();
    assert!(pen.is_substantial(), "a pen click is kept as a dot");
    let mut pen = pen;
    pen.extend(pt(5.0, 5.0));
    assert!(matches!(&pen.kind, OpKind::Pen { pts } if pts.len() == 2));
}

#[test]
fn history_undo_redo_and_edits() {
    let mut h = History::new();
    assert!(!h.can_undo() && !h.undo() && !h.redo());
    let mk = |h: &mut History| Op { id: h.fresh_id(), color: "#000".into(), kind: OpKind::Rect { a: pt(0.0, 0.0), b: pt(50.0, 50.0) } };
    let (a, b) = (mk(&mut h), mk(&mut h));
    let (ia, ib) = (a.id, b.id);
    h.push(a);
    h.push(b);
    assert_eq!(h.len(), 2);
    assert!(h.undo());
    assert_eq!(h.ops().iter().map(|o| o.id).collect::<Vec<_>>(), vec![ia], "undo pops the last op");
    assert!(h.can_redo());
    assert!(h.redo());
    assert_eq!(h.len(), 2);
    assert!(h.undo());
    let c = mk(&mut h);
    h.push(c);
    assert!(!h.can_redo(), "a new op clears the redo stack");
    // moving and deleting are undoable steps
    assert!(h.move_op(ia, 5.0, 7.0));
    assert!(matches!(&h.get(ia).unwrap().kind, OpKind::Rect { a, .. } if *a == pt(5.0, 7.0)));
    assert!(h.undo());
    assert!(matches!(&h.get(ia).unwrap().kind, OpKind::Rect { a, .. } if *a == pt(0.0, 0.0)));
    assert!(!h.move_op(999, 1.0, 1.0));
    assert!(h.remove(ia));
    assert!(h.get(ia).is_none());
    assert!(h.undo());
    assert!(h.get(ia).is_some());
    let _ = ib;
    h.clear();
    assert!(h.is_empty() && !h.can_undo());
    // history is bounded
    for _ in 0..(HISTORY_CAP + 20) {
        let o = mk(&mut h);
        h.push(o);
    }
    let mut n = 0;
    while h.undo() {
        n += 1;
    }
    assert_eq!(n, HISTORY_CAP);
}

#[test]
fn op_hit_testing_and_step_numbers() {
    let mut h = History::new();
    let sc = Scale::IDENTITY;
    let id1 = h.fresh_id();
    h.push(Op { id: id1, color: "#000".into(), kind: OpKind::Rect { a: pt(100.0, 100.0), b: pt(200.0, 200.0) } });
    let id2 = h.fresh_id();
    h.push(Op { id: id2, color: "#000".into(), kind: OpKind::Arrow { a: pt(0.0, 0.0), b: pt(100.0, 0.0) } });
    let id3 = h.fresh_id();
    h.push(Op { id: id3, color: "#000".into(), kind: OpKind::Blur { a: pt(300.0, 300.0), b: pt(400.0, 400.0) } });
    assert_eq!(h.hit_test(pt(100.0, 150.0), 2.0, sc), Some(id1), "on the rectangle outline");
    assert_eq!(h.hit_test(pt(150.0, 150.0), 2.0, sc), None, "inside an unfilled rectangle is a miss");
    assert_eq!(h.hit_test(pt(50.0, 3.0), 2.0, sc), Some(id2));
    assert_eq!(h.hit_test(pt(50.0, 20.0), 2.0, sc), None);
    assert_eq!(h.hit_test(pt(350.0, 350.0), 0.0, sc), Some(id3), "blur boxes are solid");
    let id4 = h.fresh_id();
    h.push(Op { id: id4, color: "#000".into(), kind: OpKind::Step { n: h.next_step(), c: pt(500.0, 500.0) } });
    assert_eq!(h.next_step(), 2);
    assert_eq!(h.hit_test(pt(508.0, 500.0), 0.0, sc), Some(id4));
    assert_eq!(h.hit_test(pt(520.0, 500.0), 0.0, sc), None);
    let pen = Op { id: 99, color: "#000".into(), kind: OpKind::Pen { pts: vec![pt(0.0, 0.0), pt(10.0, 10.0), pt(20.0, 0.0)] } };
    assert!(pen.hit(pt(10.0, 11.0), 1.0, sc) && !pen.hit(pt(10.0, 30.0), 1.0, sc));
    assert!(pen.bounds(sc) == Rect::new(0.0, 0.0, 20.0, 10.0));
    let ell = Op { id: 100, color: "#000".into(), kind: OpKind::Ellipse { a: pt(0.0, 0.0), b: pt(100.0, 60.0) } };
    assert!(ell.hit(pt(0.0, 30.0), 1.0, sc) && ell.hit(pt(50.0, 0.0), 1.0, sc) && !ell.hit(pt(50.0, 30.0), 1.0, sc));
    let mut moved = pen.clone();
    moved.move_by(3.0, 4.0);
    assert!(matches!(&moved.kind, OpKind::Pen { pts } if pts[0] == pt(3.0, 4.0)));
}

// ------------------------------------------------------------------------------------------------- loupe

fn frame_with(w: i32, h: i32, px: &[((i32, i32), [u8; 3])]) -> Arc<Vec<u8>> {
    let mut v = vec![0x20u8; (w * h * 4) as usize];
    for ((x, y), c) in px {
        let o = ((y * w + x) * 4) as usize;
        v[o..o + 3].copy_from_slice(c);
    }
    Arc::new(v)
}

#[test]
fn loupe_sampling_and_formatting() {
    assert_eq!(loupe::hex_of([255, 0, 128]), "#FF0080");
    assert_eq!(loupe::hex_of([1, 2, 3]), "#010203");
    assert_eq!(loupe::rgb_text([255, 0, 128]), "rgb(255, 0, 128)");
    assert_eq!(loupe::parse_hex("#ff423a"), Some([255, 66, 58]));
    assert_eq!(loupe::parse_hex("FF423A"), Some([255, 66, 58]));
    assert_eq!(loupe::parse_hex("#ff423"), None);
    assert_eq!(loupe::parse_hex("#gg0000"), None);
    assert_eq!(loupe::hsl_text([255, 0, 0]), "hsl(0, 100%, 50%)");
    assert_eq!(loupe::hsl_text([0, 0, 255]), "hsl(240, 100%, 50%)");
    assert_eq!(loupe::hsl_text([128, 128, 128]), "hsl(0, 0%, 50%)");
    assert_eq!((loupe::GRID, loupe::RADIUS), (15, 7));
    assert_eq!(loupe::CELL, 9.0);

    let mut l = Loupe::new(100, 50, frame_with(100, 50, &[((10, 20), [255, 0, 128])]));
    assert_eq!((l.cx, l.cy), (50, 25), "starts at the frame centre");
    l.set_pos(10.9, 20.2);
    assert_eq!((l.cx, l.cy), (10, 20), "floor");
    assert_eq!(l.current(), Some([255, 0, 128]));
    l.set_pos(-5.0, 9999.0);
    assert_eq!((l.cx, l.cy), (0, 49), "clamped into the frame");
    assert_eq!(l.at(100, 0), None);
    assert_eq!(l.at(-1, 0), None);
    l.set_pos(10.0, 20.0);
    l.step(1, 0, false);
    assert_eq!(l.cx, 11);
    l.step(-1, 0, true);
    assert_eq!(l.cx, 1, "shift moves 10 px");
    l.clear();
    assert!(l.current().is_none() && l.view(Scale::IDENTITY, 100.0, 50.0).is_none());
}

#[test]
fn loupe_view_grid_and_placement() {
    let mut l = Loupe::new(100, 50, frame_with(100, 50, &[((0, 0), [255, 0, 0])]));
    l.set_pos(0.0, 0.0);
    let v = l.view(Scale::IDENTITY, 100.0, 50.0).unwrap();
    assert_eq!(v.cells.len(), 225);
    assert_eq!(v.hex, "#FF0000");
    assert_eq!(v.rgb, "rgb(255, 0, 0)");
    assert_eq!(v.keys, "x 0  y 0", "two spaces between the coordinates, as the page prints it");
    let centre = v.cells[7 * 15 + 7];
    assert_eq!((centre.i, centre.j, centre.rgb, centre.inside), (7, 7, [255, 0, 0], true));
    let corner = v.cells[0];
    assert!(!corner.inside && corner.rgb == loupe::OUTSIDE, "cells outside the frame are #111");
    assert_eq!(v.centre_box, (7.0 * 9.0 + 1.0, 7.0 * 9.0 + 1.0, 7.0, 7.0));
    // 100x50 viewport: the default 147x215 box does not fit right/below, so it flips to the left/above of the sample
    assert_eq!(v.box_pos, pt(0.5 - 26.0 - 147.0, 0.5 - 26.0 - 215.0));
    assert_eq!(v.marker, pt(0.5, 0.5));
    let big = Loupe::new(1920, 1080, frame_with(1920, 1080, &[]));
    let vv = big.view(Scale::IDENTITY, 1920.0, 1080.0).unwrap();
    assert_eq!(vv.box_pos, pt(960.5 + 26.0, 540.5 + 26.0), "fits: right and below by 26 px");
    let mut m = big.clone();
    m.box_size = Some((100.0, 100.0));
    m.set_pos(1900.0, 1070.0);
    assert_eq!(m.lens_pos(Scale::IDENTITY, 1920.0, 1080.0), pt(1900.5 - 26.0 - 100.0, 1070.5 - 26.0 - 100.0));
    // 150 %: marker/box live in css px
    let hi = Loupe::new(3840, 2160, frame_with(3840, 2160, &[]));
    let sc = Scale::new(3840, 2160, 2560.0, 1440.0);
    assert_eq!(hi.css_centre(sc), pt(1920.5 / 1.5, 1080.5 / 1.5));
}

// ------------------------------------------------------------------------------------------------- toolbar

#[test]
fn toolbar_metrics_and_order() {
    let t = Toolbar::new(false, false);
    assert_eq!((t.width, t.height), (577.0, 44.0));
    let ids: Vec<String> = t.items().filter(|i| **i != ItemId::Sep).map(|i| i.id()).collect();
    assert_eq!(
        ids,
        ["select", "rect", "arrow", "pen", "blur", "text", "#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff", "undo", "copy", "save", "record", "both", "cancel"]
    );
    assert_eq!(t.items().filter(|i| **i == ItemId::Sep).count(), 3);
    let ext = Toolbar::new(true, true);
    assert!(ext.width > t.width);
    let eids: Vec<String> = ext.items().filter(|i| **i != ItemId::Sep).map(|i| i.id()).collect();
    for want in ["highlighter", "ellipse", "step", "redo", "pin", "shelf"] {
        assert!(eids.contains(&want.to_string()), "{want}");
    }
    // hit-testing relative to the bar origin
    let first = t.view(pt(0.0, 0.0), true, &ToolbarState::default(), &EN);
    let sel = &first.items[0];
    assert_eq!(sel.rect, Rect::new(6.0, 6.0, 32.0, 32.0));
    assert_eq!(t.item_at(pt(10.0, 10.0)), Some(&ItemId::Tool(Tool::Select)));
    assert_eq!(t.item_at(pt(2.0, 2.0)), None, "border");
    let sep = first.items.iter().find(|i| i.item == ItemId::Sep).unwrap();
    assert_eq!((sep.rect.w, sep.rect.h), (1.0, 20.0));
    let sw = first.items.iter().find(|i| i.id == "#0a84ff").unwrap();
    assert_eq!((sw.rect.w, sw.rect.h), (22.0, 32.0));
    assert_eq!(first.corner_radius, 14.0);
}

#[test]
fn toolbar_state_tips_and_shortcuts() {
    let t = Toolbar::new(false, false);
    let st = ToolbarState { tool: Some(Tool::Pen), color: 2, can_undo: false, ..ToolbarState::default() };
    let v = t.view(pt(0.0, 0.0), true, &st, &RU);
    let get = |id: &str| v.items.iter().find(|i| i.id == id).unwrap();
    assert!(get("pen").on && !get("rect").on);
    assert!(get("#34c759").on && !get("#ff3b30").on);
    assert!(get("undo").disabled, "undo is disabled without ops");
    assert!(!get("copy").disabled);
    assert_eq!(get("undo").tip, "Отменить (Ctrl+Z)");
    assert_eq!(get("both").tip, "Готово: копировать и сохранить (Enter)");
    assert_eq!(get("cancel").shortcut, Some("Esc"));
    assert_eq!(get("copy").shortcut, Some("Ctrl+C"));
    assert_eq!(get("record").icon, "video");
    assert_eq!(get("both").icon, "check");
    assert_eq!(get("both").variant, toolbar::Variant::Go);
    assert_eq!(get("cancel").variant, toolbar::Variant::Cancel);
    let busy = t.view(pt(0.0, 0.0), true, &ToolbarState { busy: true, can_undo: true, ..ToolbarState::default() }, &EN);
    assert!(busy.items.iter().find(|i| i.id == "copy").unwrap().disabled && busy.items.iter().find(|i| i.id == "undo").unwrap().disabled);
    assert!(!busy.items.iter().find(|i| i.id == "rect").unwrap().disabled);
    let hov = t.view(pt(0.0, 0.0), true, &ToolbarState { hover: Some("save".into()), pressed: Some("save".into()), ..ToolbarState::default() }, &EN);
    let s = hov.items.iter().find(|i| i.id == "save").unwrap();
    assert!(s.hover && s.pressed);
    // every web tip exists in the extracted i18n catalogue (so the ru and en strings are exactly the web ones)
    for it in t.items().filter(|i| !matches!(i, ItemId::Sep | ItemId::Swatch(_))) {
        let (ru, en) = (toolbar::tip(it, &RU), toolbar::tip(it, &EN));
        assert!(i18n::known(&ru, &en), "toolbar tip not in the web catalogue: {ru} / {en}");
    }
    assert_eq!(toolbar::tip(&ItemId::Swatch(0), &EN), "#ff3b30");
}

#[test]
fn toolbar_position_rules() {
    let (bw, bh) = (577.0, 44.0);
    // under the selection, right-aligned, clamped to the left margin
    assert_eq!(toolbar::position(&Rect::new(100.0, 100.0, 400.0, 300.0), bw, bh, 1920.0, 1080.0), pt(8.0, 410.0));
    // wide selection: right edge aligned
    assert_eq!(toolbar::position(&Rect::new(100.0, 100.0, 1000.0, 300.0), bw, bh, 1920.0, 1080.0), pt(1100.0 - 577.0, 410.0));
    // near the right edge: clamped to vw - w - 8
    assert_eq!(toolbar::position(&Rect::new(1515.0, 100.0, 400.0, 300.0), bw, bh, 1920.0, 1080.0).x, 1920.0 - 577.0 - 8.0);
    // no room below: above the selection
    assert_eq!(toolbar::position(&Rect::new(100.0, 700.0, 400.0, 350.0), bw, bh, 1920.0, 1080.0).y, 700.0 - 44.0 - 10.0);
    // no room above or below (full height selection): tucked inside the bottom edge
    assert_eq!(toolbar::position(&Rect::new(0.0, 0.0, 1920.0, 1080.0), bw, bh, 1920.0, 1080.0).y, 1080.0 - 44.0 - 10.0);
    assert!(toolbar::shortcuts().iter().any(|s| s.keys == "Ctrl+Z" && s.web));
    assert!(toolbar::shortcuts().iter().any(|s| s.keys == "Delete" && !s.web));
}

// ------------------------------------------------------------------------------------------------- session

#[test]
fn arm_protocol_and_hint() {
    let mut io = FakeIo::with_session(7, "area", 1920, 1080);
    let mut m = CaptureModel::new("capture-0", 1920.0, 1080.0, Options::default());
    assert_eq!(m.phase(), Phase::Idle);
    let r = m.on_input(Input::Begin, &mut io);
    assert!(r.redraw);
    assert_eq!(io.cmds(), ["capture_session", "capture_frame", "capture_ready"], "session, then the frame, then ready");
    assert_eq!(io.last("capture_session").unwrap().args, json!({ "label": "capture-0" }));
    assert_eq!((io.yielded, io.refocused), (1, 1));
    assert_eq!(m.mode(), Mode::Area);
    assert_eq!(m.session_id(), 7);
    assert_eq!(m.frame().unwrap().len(), 1920 * 1080 * 4);
    let v = m.view(&EN);
    assert_eq!(v.hint.as_ref().unwrap().text, "Drag to select an area · Esc to cancel");
    assert!(v.hint.unwrap().visible && v.dim_full);
    assert_eq!(m.view(&RU).hint.unwrap().text, "Выдели область · Esc — отмена");
    // a repeated capture:begin for the same session id is ignored
    let before = io.calls.len();
    assert_eq!(m.arm(&mut io), ArmOutcome::Duplicate);
    assert_eq!(io.calls.len(), before + 1, "only capture_session is asked again");
    // a new id re-arms
    io.session = Some(json!({ "id": 8, "mode": "text", "w": 1920, "h": 1080, "x": 0, "y": 0, "scale": 1.0, "focus": false }));
    assert_eq!(m.arm(&mut io), ArmOutcome::Armed);
    assert_eq!(m.mode(), Mode::Text);
}

#[test]
fn arm_failures() {
    let mut io = FakeIo::new();
    let mut m = CaptureModel::new("capture-1", 800.0, 600.0, Options::default());
    assert_eq!(m.arm(&mut io), ArmOutcome::NoSession);
    assert_eq!(io.cmds(), ["capture_session"]);
    // frame of the wrong size cancels the session
    let mut io = FakeIo::with_session(3, "area", 800, 600);
    io.frame = Some(vec![0; 10]);
    assert_eq!(m.arm(&mut io), ArmOutcome::FrameMismatch);
    assert_eq!(io.cmds(), ["capture_session", "capture_frame", "capture_cancel"]);
    assert!(m.frame().is_none());
    io.frame = None;
    let mut m2 = CaptureModel::new("capture-1", 800.0, 600.0, Options::default());
    assert_eq!(m2.arm(&mut io), ArmOutcome::FrameMismatch, "a missing frame is a mismatch too");
}

// ------------------------------------------------------------------------------------------------- selection machine

#[test]
fn selection_lifecycle_idle_dragging_adjusting_committed() {
    let (mut m, mut io) = area();
    assert_eq!(m.phase(), Phase::Idle);
    down(&mut m, &mut io, 100.0, 100.0);
    assert_eq!(m.phase(), Phase::Dragging);
    mv(&mut m, &mut io, 300.0, 250.0);
    mv(&mut m, &mut io, 500.0, 400.0);
    assert_eq!(m.selection(), Some(Rect::new(100.0, 100.0, 400.0, 300.0)));
    assert!(m.view(&EN).toolbar.is_none(), "no toolbar while dragging");
    up(&mut m, &mut io, 500.0, 400.0);
    assert_eq!(m.phase(), Phase::Adjusting);
    let v = m.view(&EN);
    let sel = v.selection.unwrap();
    assert_eq!(sel.size_label, "400 \u{d7} 300");
    assert!(!sel.size_inside);
    assert_eq!(sel.handles.len(), 8);
    assert_eq!(sel.clip, ClipInset { top: 100.0, right: 1420.0, bottom: 680.0, left: 100.0 });
    let bar = v.toolbar.unwrap();
    assert_eq!((bar.rect.x, bar.rect.y, bar.rect.w, bar.rect.h), (8.0, 410.0, 577.0, 44.0));
    assert!(!v.dim_full, "the cut-out replaces the flat dim");
    // the toolbar and the handles animate in
    let kinds: Vec<_> = v.anims.iter().map(|a| a.spec.kind).collect();
    assert!(kinds.contains(&AnimKind::ToolbarEnter) && kinds.contains(&AnimKind::HandlesPop));
    // Enter commits with "both"
    let r = key(&mut m, &mut io, Key::Enter, Mods::NONE);
    assert!(r.prevent_default);
    assert_eq!(r.actions, vec![Action::Commit(CommitKind::Both)]);
    assert_eq!(m.phase(), Phase::Committed);
    let a = commit_args(&io);
    assert_eq!(a["action"], "both");
    assert_eq!(a["rect"], json!([100.0, 100.0, 400.0, 300.0]));
    assert_eq!((a["vw"].as_f64(), a["vh"].as_f64()), (Some(1920.0), Some(1080.0)));
    assert_eq!(a["label"], "capture-0");
    assert!(io.raw_bodies.is_empty(), "no markup = plain JSON");
    // a committed overlay ignores input until the next Begin
    let n = io.calls.len();
    down(&mut m, &mut io, 5.0, 5.0);
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    assert_eq!(io.calls.len(), n);
}

#[test]
fn tiny_drag_is_discarded() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (103.0, 130.0));
    assert!(m.selection().is_none(), "width 3 < 4");
    assert_eq!(m.phase(), Phase::Idle);
    drag(&mut m, &mut io, (100.0, 100.0), (104.0, 104.0));
    assert!(m.selection().is_some(), "4 x 4 is the minimum");
    // dragging up-left normalises
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (500.0, 400.0), (100.0, 100.0));
    assert_eq!(m.selection(), Some(Rect::new(100.0, 100.0, 400.0, 300.0)));
}

#[test]
fn move_resize_and_cursor() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    mv(&mut m, &mut io, 300.0, 250.0);
    assert_eq!(m.view(&EN).cursor, Cursor::Move);
    mv(&mut m, &mut io, 500.0, 400.0);
    assert_eq!(m.view(&EN).cursor, Cursor::NwseResize);
    mv(&mut m, &mut io, 300.0, 100.0);
    assert_eq!(m.view(&EN).cursor, Cursor::NsResize);
    mv(&mut m, &mut io, 900.0, 900.0);
    assert_eq!(m.view(&EN).cursor, Cursor::Default);
    // move the body
    drag(&mut m, &mut io, (300.0, 250.0), (340.0, 270.0));
    assert_eq!(m.selection(), Some(Rect::new(140.0, 120.0, 400.0, 300.0)));
    assert_eq!(m.phase(), Phase::Adjusting);
    // resize by the south-east handle
    drag(&mut m, &mut io, (540.0, 420.0), (640.0, 520.0));
    assert_eq!(m.selection(), Some(Rect::new(140.0, 120.0, 500.0, 400.0)));
    // the toolbar follows (still visible, no re-animation needed)
    assert!(m.view(&EN).toolbar.is_some());
    // a press outside the selection starts a new one and hides the bar
    down(&mut m, &mut io, 1000.0, 800.0);
    assert!(m.view(&EN).toolbar.is_none());
    assert!(m.view(&EN).anims.iter().any(|a| a.spec.kind == AnimKind::ToolbarLeave));
    up(&mut m, &mut io, 1000.0, 800.0);
    assert!(m.selection().is_none());
}

#[test]
fn small_selection_near_top_puts_the_size_label_inside() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 20.0), (300.0, 200.0));
    assert!(m.view(&EN).selection.unwrap().size_inside);
}

#[test]
fn right_click_escape_and_context_menu_cancel() {
    let (mut m, mut io) = area();
    let r = m.on_input(Input::PointerDown { pos: pt(1.0, 1.0), button: Button::Right, mods: Mods::NONE }, &mut io);
    assert!(r.prevent_default && r.actions == vec![Action::Cancel]);
    assert_eq!(io.cmds().last(), Some(&"capture_cancel"));
    assert_eq!(m.phase(), Phase::Committed);
    let (mut m, mut io) = area();
    key(&mut m, &mut io, Key::Escape, Mods::NONE);
    assert_eq!(io.cmds().last(), Some(&"capture_cancel"));
    let (mut m, mut io) = area();
    m.on_input(Input::ContextMenu, &mut io);
    assert_eq!(io.cmds().last(), Some(&"capture_cancel"));
    // capture:end resets, the next Begin re-arms (new id needed)
    m.on_input(Input::End, &mut io);
    assert_eq!(m.phase(), Phase::Idle);
    io.session = Some(json!({ "id": 2, "mode": "area", "w": 1920, "h": 1080 }));
    m.on_input(Input::Begin, &mut io);
    assert_eq!(m.session_id(), 2);
}

#[test]
fn keyboard_shortcuts_in_area_mode() {
    let (mut m, mut io) = area();
    // Ctrl+C / Ctrl+S need a selection
    ch(&mut m, &mut io, 'c', Mods::CTRL);
    ch(&mut m, &mut io, 's', Mods::CTRL);
    assert!(io.last("capture_commit").is_none());
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    let r = ch(&mut m, &mut io, 'C', Mods::CTRL);
    assert_eq!(r.actions, vec![Action::Commit(CommitKind::Copy)], "case-insensitive");
    assert_eq!(commit_args(&io)["action"], "copy");
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    ch(&mut m, &mut io, 's', Mods::CTRL);
    assert_eq!(commit_args(&io)["action"], "save");
    // Cmd works like Ctrl
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    ch(&mut m, &mut io, 'c', Mods { meta: true, ..Mods::NONE });
    assert_eq!(commit_args(&io)["action"], "copy");
    // Enter without a selection commits the whole screen (no rect)
    let (mut m, mut io) = area();
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    let a = commit_args(&io);
    assert_eq!(a["action"], "both");
    assert!(a.get("rect").is_none());
}

#[test]
fn double_click_commits_only_with_the_select_tool_inside() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    m.on_input(Input::DoubleClick { pos: pt(900.0, 900.0) }, &mut io);
    assert!(io.last("capture_commit").is_none(), "outside");
    m.apply(Action::SetTool(Tool::Rect), &mut io);
    m.on_input(Input::DoubleClick { pos: pt(300.0, 250.0) }, &mut io);
    assert!(io.last("capture_commit").is_none(), "markup tool selected");
    m.apply(Action::SetTool(Tool::Select), &mut io);
    m.on_input(Input::DoubleClick { pos: pt(300.0, 250.0) }, &mut io);
    assert_eq!(commit_args(&io)["action"], "both");
}

// ------------------------------------------------------------------------------------------------- markup in the machine

#[test]
fn drawing_clamps_to_the_selection_and_undo_toggles_the_button() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    let v = m.view(&EN);
    assert!(v.toolbar.unwrap().items.iter().find(|i| i.id == "undo").unwrap().disabled);
    click_item(&mut m, &mut io, "rect");
    assert_eq!(m.tool(), Tool::Rect);
    // drag from inside to far outside: clamped to the selection edge
    drag(&mut m, &mut io, (150.0, 150.0), (900.0, 900.0));
    assert_eq!(m.ops().len(), 1);
    assert_eq!(m.ops()[0].kind, OpKind::Rect { a: pt(150.0, 150.0), b: pt(500.0, 400.0) });
    assert_eq!(m.ops()[0].color, "#ff3b30");
    assert!(!m.view(&EN).toolbar.unwrap().items.iter().find(|i| i.id == "undo").unwrap().disabled);
    assert!(m.view(&EN).anims.iter().any(|a| a.spec.kind == AnimKind::OpInsert), "new ops animate in");
    // a click without a drag leaves no rectangle
    drag(&mut m, &mut io, (200.0, 200.0), (202.0, 201.0));
    assert_eq!(m.ops().len(), 1);
    // colour swatch then pen
    click_item(&mut m, &mut io, "#0a84ff");
    assert_eq!(m.color(), "#0a84ff");
    click_item(&mut m, &mut io, "pen");
    drag(&mut m, &mut io, (200.0, 200.0), (260.0, 240.0));
    assert!(matches!(&m.ops()[1].kind, OpKind::Pen { pts } if pts.len() >= 3));
    assert_eq!(m.ops()[1].color, "#0a84ff");
    // undo via Ctrl+Z, then via the button
    ch(&mut m, &mut io, 'z', Mods::CTRL);
    assert_eq!(m.ops().len(), 1);
    click_item(&mut m, &mut io, "undo");
    assert!(m.ops().is_empty());
    click_item(&mut m, &mut io, "undo"); // disabled: no effect, no panic
    assert!(m.ops().is_empty());
    // the draft is visible while dragging
    down(&mut m, &mut io, 150.0, 150.0);
    mv(&mut m, &mut io, 190.0, 190.0);
    assert!(m.view(&EN).draft.is_some());
    up(&mut m, &mut io, 190.0, 190.0);
    assert!(m.view(&EN).draft.is_none());
}

#[test]
fn text_tool_flow() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    click_item(&mut m, &mut io, "text");
    assert_eq!(m.view(&EN).cursor, Cursor::Default);
    mv(&mut m, &mut io, 300.0, 250.0);
    assert_eq!(m.view(&EN).cursor, Cursor::Text);
    down(&mut m, &mut io, 150.0, 160.0);
    up(&mut m, &mut io, 150.0, 160.0);
    assert!(m.view(&EN).text_edit.is_some());
    m.on_input(Input::Text("héllo".into()), &mut io);
    key(&mut m, &mut io, Key::Left, Mods::NONE);
    key(&mut m, &mut io, Key::Backspace, Mods::NONE);
    ch(&mut m, &mut io, 'X', Mods::NONE);
    assert_eq!(m.text_input(), Some("hélXo"), "caret editing is char-based");
    key(&mut m, &mut io, Key::Home, Mods::NONE);
    key(&mut m, &mut io, Key::Delete, Mods::NONE);
    assert_eq!(m.text_input(), Some("élXo"));
    // keys go to the text box: Esc closes without committing, and does not cancel the capture
    key(&mut m, &mut io, Key::Escape, Mods::NONE);
    assert!(m.text_input().is_none() && m.ops().is_empty());
    assert!(io.last("capture_cancel").is_none());
    // Enter commits trimmed text at the frame position
    down(&mut m, &mut io, 150.0, 160.0);
    up(&mut m, &mut io, 150.0, 160.0);
    m.on_input(Input::Text("  Hi  ".into()), &mut io);
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    assert_eq!(m.ops().len(), 1);
    assert_eq!(m.ops()[0].kind, OpKind::Text { s: "Hi".into(), a: pt(150.0, 160.0) });
    // an empty text leaves nothing
    down(&mut m, &mut io, 200.0, 200.0);
    m.on_input(Input::Text("   ".into()), &mut io);
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    assert_eq!(m.ops().len(), 1);
    // clicking elsewhere commits what was typed (blur), then opens a new box
    down(&mut m, &mut io, 220.0, 220.0);
    m.on_input(Input::Text("again".into()), &mut io);
    down(&mut m, &mut io, 300.0, 300.0);
    assert_eq!(m.ops().len(), 2);
    assert!(m.view(&EN).text_edit.is_some());
    // switching the tool drops an unfinished box
    click_item(&mut m, &mut io, "rect");
    assert!(m.view(&EN).text_edit.is_none());
}

#[test]
fn commit_with_markup_flattens_and_sends_a_raw_body() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    click_item(&mut m, &mut io, "arrow");
    drag(&mut m, &mut io, (150.0, 150.0), (350.0, 300.0));
    click_item(&mut m, &mut io, "copy");
    assert_eq!(m.phase(), Phase::Committed);
    assert_eq!(io.flattened.len(), 1);
    assert_eq!(io.flattened[0].src, IRect { x: 100, y: 100, w: 400, h: 300 });
    assert_eq!(io.flattened[0].cmds.len(), 1);
    assert_eq!(io.raw_bodies.len(), 1);
    assert_eq!(io.raw_bodies[0].1, vec![0x89, b'P', b'N', b'G']);
    let a = commit_args(&io);
    assert_eq!(a["action"], "copy");
    assert_eq!(a["rect"], json!([100.0, 100.0, 400.0, 300.0]));
    // the args also travel as the x-pill-args header
    let args = CommitArgs { label: "capture-0".into(), vw: 1920.0, vh: 1080.0, action: CommitKind::Copy, rect: Some([100.0, 100.0, 400.0, 300.0]), point: None };
    assert_eq!(serde_json::from_str::<serde_json::Value>(&args.header_json()).unwrap(), a);
    assert_eq!(ARGS_HEADER, "x-pill-args");
    // the copy flashes over the selection and the toolbar fades out
    let v = m.view(&EN);
    assert!(v.anims.iter().any(|a| a.spec.kind == AnimKind::CopyFlash));
    assert!(v.ghost.is_some() && v.selection.is_none() && v.toolbar.is_none());
}

#[test]
fn record_never_sends_markup_and_failed_flatten_falls_back_to_json() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    click_item(&mut m, &mut io, "rect");
    drag(&mut m, &mut io, (150.0, 150.0), (350.0, 300.0));
    click_item(&mut m, &mut io, "record");
    assert!(io.flattened.is_empty() && io.raw_bodies.is_empty());
    assert_eq!(commit_args(&io)["action"], "record");
    let (mut m, mut io) = area();
    io.png = None;
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    click_item(&mut m, &mut io, "rect");
    drag(&mut m, &mut io, (150.0, 150.0), (350.0, 300.0));
    click_item(&mut m, &mut io, "save");
    assert!(io.raw_bodies.is_empty());
    assert_eq!(commit_args(&io)["action"], "save");
}

#[test]
fn toolbar_clicks_need_press_and_release_on_the_same_button() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    let bar = m.view(&EN).toolbar.unwrap();
    let c = |id: &str| {
        let it = bar.items.iter().find(|i| i.id == id).unwrap();
        pt(bar.rect.x + it.rect.x + 16.0, bar.rect.y + it.rect.y + 16.0)
    };
    let (copy, save) = (c("copy"), c("save"));
    down(&mut m, &mut io, copy.x, copy.y);
    assert!(m.view(&EN).toolbar.unwrap().items.iter().find(|i| i.id == "copy").unwrap().pressed);
    up(&mut m, &mut io, save.x, save.y);
    assert!(io.last("capture_commit").is_none(), "released on another button");
    assert!(m.selection().is_some(), "the press on the bar never touched the selection");
    mv(&mut m, &mut io, c("rect").x, c("rect").y);
    assert!(m.view(&EN).toolbar.unwrap().items.iter().find(|i| i.id == "rect").unwrap().hover);
    let cancel = c("cancel");
    down(&mut m, &mut io, cancel.x, cancel.y);
    up(&mut m, &mut io, cancel.x, cancel.y);
    assert_eq!(io.cmds().last(), Some(&"capture_cancel"));
}

// ------------------------------------------------------------------------------------------------- modes

#[test]
fn text_qr_and_record_modes_run_on_pointer_up() {
    for (mode, hint_en) in [("text", "Select the text to read · Esc to cancel"), ("qr", "Select a QR code · Esc to cancel"), ("record", "Select the area to record · Enter for the whole screen · Esc to cancel")] {
        let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, mode, Options::default());
        assert_eq!(m.view(&EN).hint.unwrap().text, hint_en);
        drag(&mut m, &mut io, (10.0, 10.0), (210.0, 110.0));
        let a = commit_args(&io);
        assert_eq!(a["action"], "run", "{mode}");
        assert_eq!(a["rect"], json!([10.0, 10.0, 200.0, 100.0]));
        assert_eq!(m.phase(), Phase::Committed);
    }
    // no handles and no toolbar outside area mode
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "text", Options::default());
    down(&mut m, &mut io, 10.0, 10.0);
    mv(&mut m, &mut io, 200.0, 100.0);
    let v = m.view(&EN);
    assert!(v.selection.unwrap().handles.is_empty() && v.toolbar.is_none());
    // Enter in record mode with no selection = the whole screen
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "record", Options::default());
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    let a = commit_args(&io);
    assert_eq!(a["action"], "run");
    assert!(a.get("rect").is_none());
    // a tiny selection in text mode is dropped, not run
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "qr", Options::default());
    drag(&mut m, &mut io, (10.0, 10.0), (12.0, 12.0));
    assert!(io.last("capture_commit").is_none());
    // Ctrl+C is an area-mode shortcut only
    ch(&mut m, &mut io, 'c', Mods::CTRL);
    assert!(io.last("capture_commit").is_none());
}

fn color_rig(vw: f64, vh: f64, fw: i32, fh: i32) -> (CaptureModel, FakeIo) {
    let mut io = FakeIo::with_session(1, "color", fw, fh);
    let mut f = vec![0x30u8; (fw * fh * 4) as usize];
    let o = ((20 * fw + 10) * 4) as usize;
    f[o..o + 3].copy_from_slice(&[255, 0, 128]);
    io.frame = Some(f);
    let mut m = CaptureModel::new("capture-0", vw, vh, Options::default());
    assert_eq!(m.arm(&mut io), ArmOutcome::Armed);
    (m, io)
}

#[test]
fn colour_picker_follows_pointer_and_arrows() {
    let (mut m, mut io) = color_rig(1920.0, 1080.0, 1920, 1080);
    let v = m.view(&EN);
    assert!(v.cursor_hidden && !v.dim_full && v.toolbar.is_none());
    assert_eq!(v.hint.unwrap().text, "Click to copy the colour · arrows move 1 pixel · Esc to cancel");
    assert_eq!(v.loupe.as_ref().unwrap().sample, (960, 540), "starts at the centre");
    assert!(v.anims.iter().any(|a| a.spec.kind == AnimKind::LoupeShow));
    mv(&mut m, &mut io, 10.7, 20.2);
    let v = m.view(&EN);
    let l = v.loupe.unwrap();
    assert_eq!((l.sample, l.hex.as_str(), l.rgb.as_str()), ((10, 20), "#FF0080", "rgb(255, 0, 128)"));
    assert!(v.loupe_from.is_some() && v.anims.iter().any(|a| a.spec.kind == AnimKind::LoupeFollow));
    key(&mut m, &mut io, Key::Right, Mods::NONE);
    key(&mut m, &mut io, Key::Down, Mods::SHIFT);
    assert_eq!(m.view(&EN).loupe.unwrap().sample, (11, 30));
    key(&mut m, &mut io, Key::Up, Mods::NONE);
    key(&mut m, &mut io, Key::Left, Mods::NONE);
    assert_eq!(m.view(&EN).loupe.unwrap().sample, (10, 29));
    // Enter picks the sample centre
    mv(&mut m, &mut io, 10.7, 20.2);
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    let a = commit_args(&io);
    assert_eq!(a["action"], "pick");
    assert_eq!((a["x"].as_f64(), a["y"].as_f64()), (Some(10.5), Some(20.5)));
    assert!(a.get("rect").is_none());
    assert_eq!(m.phase(), Phase::Committed);
}

#[test]
fn colour_picker_click_on_a_150_percent_screen() {
    let (mut m, mut io) = color_rig(2560.0, 1440.0, 3840, 2160);
    down(&mut m, &mut io, 100.0, 100.0);
    let a = commit_args(&io);
    assert_eq!(a["action"], "pick");
    assert_eq!((a["vw"].as_f64(), a["vh"].as_f64()), (Some(2560.0), Some(1440.0)));
    // sample (150,150) -> centre (150.5)/1.5 css px; the backend turns it back into frame px with to_frame_point
    let (x, y) = (a["x"].as_f64().unwrap(), a["y"].as_f64().unwrap());
    assert!((x - 150.5 / 1.5).abs() < 1e-9 && (y - 150.5 / 1.5).abs() < 1e-9);
    assert_eq!(to_frame_point(x, y, 2560.0, 1440.0, 3840, 2160), Some((150, 150)));
    // Esc cancels
    let (mut m, mut io) = color_rig(1920.0, 1080.0, 1920, 1080);
    key(&mut m, &mut io, Key::Escape, Mods::NONE);
    assert_eq!(io.cmds().last(), Some(&"capture_cancel"));
}

#[test]
fn mixed_dpi_overlay_reports_frame_pixels() {
    let (mut m, mut io) = rig(2560.0, 1440.0, 3840, 2160, "area", Options::default());
    assert!((m.scale().rx - 1.5).abs() < 1e-12);
    drag(&mut m, &mut io, (100.0, 50.0), (400.0, 250.0));
    assert_eq!(m.view(&EN).selection.unwrap().size_label, "450 \u{d7} 300", "labelled in physical pixels");
    click_item(&mut m, &mut io, "rect");
    drag(&mut m, &mut io, (150.0, 100.0), (300.0, 200.0));
    // ops are stored in frame px
    assert_eq!(m.ops()[0].kind, OpKind::Rect { a: pt(225.0, 150.0), b: pt(450.0, 300.0) });
    let req = m.flatten_request();
    assert_eq!(req.src, IRect { x: 150, y: 75, w: 450, h: 300 });
    let DrawCmd::StrokeRect { width, .. } = &req.cmds[0] else { panic!() };
    assert_eq!(*width, 4.5, "3 css px at 150 %");
    // the backend gets css px + the viewport and derives the same crop
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    let a = commit_args(&io);
    let rect: Vec<f64> = a["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert_eq!(to_frame_rect([rect[0], rect[1], rect[2], rect[3]], 2560.0, 1440.0, 3840, 2160), Some(IRect { x: 150, y: 75, w: 450, h: 300 }));
}

#[test]
fn resize_repositions_the_bar() {
    let (mut m, mut io) = area();
    drag(&mut m, &mut io, (100.0, 700.0), (1000.0, 1050.0));
    let before = m.view(&EN).toolbar.unwrap().rect;
    assert_eq!(before.y, 700.0 - 44.0 - 10.0, "no room below: above the selection");
    m.on_input(Input::Resize { vw: 1920.0, vh: 1500.0 }, &mut io);
    let after = m.view(&EN).toolbar.unwrap().rect;
    assert_eq!(after.y, 1060.0);
    assert_eq!(m.viewport(), (1920.0, 1500.0));
}

// ------------------------------------------------------------------------------------------------- extensions

#[test]
fn extensions_are_off_in_web_parity_mode() {
    let mut o = Options::web_parity();
    assert!(!o.nudge && !o.shift_square && !o.extended_tools && o.snap.is_none() && !o.edit_ops);
    o.reduce_motion = false;
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", o);
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    key(&mut m, &mut io, Key::Right, Mods::NONE);
    assert_eq!(m.selection().unwrap().x, 100.0, "arrows do nothing in area mode on the web page");
    m.apply(Action::SetTool(Tool::Step), &mut io);
    assert_eq!(m.tool(), Tool::Select, "extended tools are refused");
    // Shift is ignored while dragging
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", Options::web_parity());
    down(&mut m, &mut io, 100.0, 100.0);
    m.on_input(Input::PointerMove { pos: pt(400.0, 200.0), mods: Mods::SHIFT }, &mut io);
    assert_eq!(m.selection().unwrap().w, 300.0);
    assert_eq!(m.selection().unwrap().h, 100.0);
}

#[test]
fn shift_square_nudge_and_snap() {
    let (mut m, mut io) = area();
    down(&mut m, &mut io, 100.0, 100.0);
    m.on_input(Input::PointerMove { pos: pt(400.0, 200.0), mods: Mods::SHIFT }, &mut io);
    assert_eq!(m.selection(), Some(Rect::new(100.0, 100.0, 300.0, 300.0)));
    up(&mut m, &mut io, 400.0, 200.0);
    // nudge by 1 frame px, Shift = 10, Ctrl = resize
    key(&mut m, &mut io, Key::Right, Mods::NONE);
    key(&mut m, &mut io, Key::Down, Mods::SHIFT);
    assert_eq!(m.selection(), Some(Rect::new(101.0, 110.0, 300.0, 300.0)));
    key(&mut m, &mut io, Key::Right, Mods::CTRL);
    assert_eq!(m.selection().unwrap().w, 301.0);
    let bar = m.view(&EN).toolbar.unwrap().rect;
    assert_eq!(bar.y, 110.0 + 300.0 + 10.0, "the bar follows the nudge");
    // aspect lock (16:9) while dragging
    let o = Options { aspect: Some(16.0 / 9.0), ..Options::default() };
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", o);
    drag(&mut m, &mut io, (100.0, 100.0), (420.0, 130.0));
    let s = m.selection().unwrap();
    assert!((s.w / s.h - 16.0 / 9.0).abs() < 1e-9);
    // snapping to the viewport edge while moving
    let o = Options { snap: Some(Snap::default()), ..Options::default() };
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", o);
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    drag(&mut m, &mut io, (300.0, 250.0), (205.0, 250.0));
    assert_eq!(m.selection().unwrap().x, 0.0, "x = 100 - 95 = 5 snaps onto the viewport edge");
}

#[test]
fn editable_ops_step_numbers_redo_and_pin() {
    let o = Options { extended_tools: true, edit_ops: true, ext_actions: true, ..Options::default() };
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", o);
    drag(&mut m, &mut io, (100.0, 100.0), (700.0, 500.0));
    click_item(&mut m, &mut io, "step");
    down(&mut m, &mut io, 200.0, 200.0);
    up(&mut m, &mut io, 200.0, 200.0);
    down(&mut m, &mut io, 300.0, 300.0);
    up(&mut m, &mut io, 300.0, 300.0);
    assert!(matches!(&m.ops()[0].kind, OpKind::Step { n: 1, .. }) && matches!(&m.ops()[1].kind, OpKind::Step { n: 2, .. }));
    // undo, redo (Ctrl+Y and the button)
    ch(&mut m, &mut io, 'z', Mods::CTRL);
    assert_eq!(m.ops().len(), 1);
    ch(&mut m, &mut io, 'y', Mods::CTRL);
    assert_eq!(m.ops().len(), 2);
    ch(&mut m, &mut io, 'z', Mods::CTRL);
    assert_eq!(m.ops().len(), 1);
    ch(&mut m, &mut io, 'Z', Mods { ctrl: true, shift: true, ..Mods::NONE });
    assert_eq!(m.ops().len(), 2, "Ctrl+Shift+Z is redo");
}

#[test]
fn editable_ops_move_and_delete() {
    let o = Options { edit_ops: true, ..Options::default() };
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", o);
    drag(&mut m, &mut io, (100.0, 100.0), (700.0, 500.0));
    click_item(&mut m, &mut io, "rect");
    drag(&mut m, &mut io, (200.0, 200.0), (300.0, 300.0));
    click_item(&mut m, &mut io, "select");
    // grab the rectangle's outline and drag it
    drag(&mut m, &mut io, (200.0, 250.0), (230.0, 270.0));
    assert_eq!(m.ops()[0].kind, OpKind::Rect { a: pt(230.0, 220.0), b: pt(330.0, 320.0) });
    assert_eq!(m.selection(), Some(Rect::new(100.0, 100.0, 600.0, 400.0)), "the selection stayed put");
    assert!(m.view(&EN).ops[0].selected);
    key(&mut m, &mut io, Key::Delete, Mods::NONE);
    assert!(m.ops().is_empty());
    ch(&mut m, &mut io, 'z', Mods::CTRL);
    assert_eq!(m.ops().len(), 1, "delete is undoable");
    // a click on the outline without moving leaves no undo step
    drag(&mut m, &mut io, (230.0, 270.0), (230.0, 270.0));
    ch(&mut m, &mut io, 'z', Mods::CTRL);
    assert_eq!(m.ops()[0].kind, OpKind::Rect { a: pt(200.0, 200.0), b: pt(300.0, 300.0) }, "the next undo reverts the real move, not a phantom one");
    ch(&mut m, &mut io, 'z', Mods::CTRL);
    assert!(m.ops().is_empty());
}

#[test]
fn pin_and_shelf_save_first() {
    let o = Options { ext_actions: true, ..Options::default() };
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", o);
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    click_item(&mut m, &mut io, "pin");
    assert_eq!(commit_args(&io)["action"], "save");
    let after = m.take_after_save().unwrap();
    assert_eq!(after, AfterSave::Pin);
    let c = after.call("C:\\x\\a.png");
    assert_eq!(c.cmd, "capture_pin");
    assert!(!c.is_registered(), "native-only: needs a backend command");
    assert_eq!(AfterSave::Shelf.call("p.png"), BackendCall::shelf_add(&["p.png".to_string()]));
    assert!(BackendCall::shelf_add(&[]).is_registered());
    assert!(m.take_after_save().is_none());
}

// ------------------------------------------------------------------------------------------------- animation

#[test]
fn animation_specs_and_reduce_motion() {
    use anim::*;
    for k in [AnimKind::HandlesPop, AnimKind::ToolbarEnter, AnimKind::ToolbarLeave, AnimKind::LoupeShow, AnimKind::LoupeFollow, AnimKind::OpInsert, AnimKind::CopyFlash, AnimKind::HintFade, AnimKind::ButtonPress, AnimKind::SwatchPop] {
        let s = spec(k, false);
        assert!(s.duration_ms > 0, "{k:?} must animate");
        let r = spec(k, true);
        assert_eq!((r.duration_ms, r.fade_ms), (0, 0), "{k:?} is instant under Reduce motion");
    }
    let bar = spec(AnimKind::ToolbarEnter, false);
    assert_eq!((bar.duration_ms, bar.fade_ms, bar.dy_from), (400, 120, 4.0), "bar-fade .12 s + bar-rise .4 s of capture.css");
    assert_eq!(spec(AnimKind::ButtonPress, false).scale_from, 0.92);
    assert_eq!(spec(AnimKind::LoupeShow, false).scale_from, 0.94);
    // curves
    assert!((cubic_bezier(0.22, 1.0, 0.36, 1.0, 0.0)).abs() < 1e-9 && (cubic_bezier(0.22, 1.0, 0.36, 1.0, 1.0) - 1.0).abs() < 1e-9);
    let spring_peak = (0..=100).map(|i| Ease::Spring.apply(i as f64 / 100.0)).fold(0.0, f64::max);
    assert!(spring_peak > 1.02, "the spring overshoots, got {spring_peak}");
    let mut prev = 0.0;
    for i in 0..=100 {
        let v = Ease::Out.apply(i as f64 / 100.0);
        assert!(v >= prev - 1e-9, "ease-out is monotonic");
        prev = v;
    }
    // sampling
    let a = Anim { id: 1, spec: spec(AnimKind::ToolbarEnter, false), target: AnimTarget::Toolbar, start_ms: 1000 };
    let f0 = a.frame(1000);
    assert_eq!((f0.opacity, f0.dy, f0.done), (0.0, 4.0, false));
    let f1 = a.frame(1060);
    assert!(f1.opacity > 0.0 && f1.opacity < 1.0);
    let end = a.frame(1400);
    assert_eq!((end.opacity, end.dy, end.done), (1.0, 0.0, true));
    let instant = Anim { id: 2, spec: spec(AnimKind::ToolbarEnter, true), target: AnimTarget::Toolbar, start_ms: 5 };
    assert!(instant.frame(5).done && instant.frame(5).opacity == 1.0);
    // set semantics
    let mut set = AnimSet::new();
    set.start(AnimKind::ButtonPress, AnimTarget::ToolbarItem("copy".into()), 0, false);
    set.start(AnimKind::ButtonPress, AnimTarget::ToolbarItem("copy".into()), 40, false);
    assert_eq!(set.active().len(), 1, "mashing a button restarts its press");
    set.start(AnimKind::ButtonPress, AnimTarget::ToolbarItem("save".into()), 40, false);
    assert_eq!(set.active().len(), 2);
    assert!(set.tick(100));
    assert!(!set.tick(1000) && !set.is_running());
}

#[test]
fn reduce_motion_makes_model_animations_instant_and_tick_retires_them() {
    let o = Options { reduce_motion: true, ..Options::default() };
    let (mut m, mut io) = rig(1920.0, 1080.0, 1920, 1080, "area", o);
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    let v = m.view(&EN);
    assert!(v.anims.iter().all(|a| a.spec.duration_ms == 0));
    assert!(!m.tick(io.now), "instant animations are retired by the first tick");
    // with motion on, the animations run until their time is up, then tick reports idle
    let (mut m, mut io) = area();
    io.now = 1000;
    drag(&mut m, &mut io, (100.0, 100.0), (500.0, 400.0));
    assert!(m.animating() && m.tick(1100));
    assert!(!m.tick(1500));
    assert!(m.view(&EN).anims.is_empty());
    // the exit ghost lives exactly as long as its animations
    key(&mut m, &mut io, Key::Enter, Mods::NONE);
    assert!(m.view(&EN).ghost.is_some());
    m.tick(io.now + 1);
    assert!(m.view(&EN).ghost.is_some());
    assert!(!m.tick(io.now + 2000));
    assert!(m.view(&EN).ghost.is_none());
}

#[test]
fn hint_fades_on_the_first_press() {
    let (mut m, mut io) = area();
    assert!(m.view(&EN).hint.unwrap().visible);
    down(&mut m, &mut io, 10.0, 10.0);
    let v = m.view(&EN);
    assert!(!v.hint.unwrap().visible);
    assert!(v.anims.iter().any(|a| a.spec.kind == AnimKind::HintFade));
}

// ------------------------------------------------------------------------------------------------- io + live

#[test]
fn backend_call_shapes_match_the_web_page() {
    assert_eq!(BackendCall::session("capture-2").args, json!({ "label": "capture-2" }));
    assert_eq!(BackendCall::ready("capture-2").cmd, "capture_ready");
    assert_eq!(BackendCall::cancel(), BackendCall { cmd: "capture_cancel".into(), args: serde_json::Value::Null });
    assert_eq!(BackendCall::start("record_screen").args, json!({ "mode": "record_screen" }));
    assert_eq!(BackendCall::action("open_url", "https://x.y").args, json!({ "action": "open_url", "value": "https://x.y" }));
    for c in [BackendCall::session("a"), BackendCall::frame("a"), BackendCall::ready("a"), BackendCall::cancel(), BackendCall::stop(), BackendCall::settings(), BackendCall::start("area"), BackendCall::action("open", "p"), BackendCall::shelf_add(&[])] {
        assert!(c.is_registered(), "{} must exist in the backend", c.cmd);
    }
    let pick = CommitArgs { label: "capture-0".into(), vw: 10.0, vh: 20.0, action: CommitKind::Pick, rect: None, point: Some(Pt::new(1.5, 2.5)) };
    assert_eq!(pick.to_json(), json!({ "label": "capture-0", "vw": 10.0, "vh": 20.0, "action": "pick", "x": 1.5, "y": 2.5 }));
    for (k, s) in [(CommitKind::Copy, "copy"), (CommitKind::Save, "save"), (CommitKind::Both, "both"), (CommitKind::Record, "record"), (CommitKind::Run, "run"), (CommitKind::Pick, "pick")] {
        assert_eq!(k.as_str(), s);
    }
    let info = SessionInfo::from_value(&json!({ "id": 4, "mode": "qr", "w": 10, "h": 20, "x": -5, "y": 0, "scale": 1.25, "focus": true, "extra": 1 })).unwrap();
    assert_eq!((info.id, info.x, info.scale, info.focus), (4, -5, 1.25, true));
    assert_eq!(Mode::parse("picker"), Some(Mode::Color));
    assert_eq!(Mode::parse("nope"), None);
    assert!(!Mode::Screen.has_hint() && Mode::Area.has_hint());
    assert_eq!(Mode::Color.id(), "color");
}

#[test]
fn clock_cut_and_base() {
    assert_eq!(live::clock(0), "0:00");
    assert_eq!(live::clock(999), "0:00");
    assert_eq!(live::clock(65_000), "1:05");
    assert_eq!(live::clock(3_600_000), "1:00:00");
    assert_eq!(live::clock(3_725_000), "1:02:05");
    assert_eq!(live::clock(-5), "0:00");
    assert_eq!(live::dur(125), "2:05");
    assert_eq!(live::cut("  a \n b\t c ", 10), "a b c");
    assert_eq!(live::cut("abcdefghij", 5), "abcd\u{2026}");
    assert_eq!(live::cut("abcde", 5), "abcde");
    assert_eq!(live::base("C:\\Users\\x\\pill_1.png"), "pill_1.png");
    assert_eq!(live::base("/tmp/a/b.mp4"), "b.mp4");
    assert_eq!(live::base(""), "");
}

fn done(v: serde_json::Value) -> CaptureDone {
    CaptureDone::from_value(&v).unwrap()
}

#[test]
fn result_cards_for_every_kind() {
    let c = live::card(&done(json!({ "kind": "screenshot", "path": "C:\\p\\pill_1.png", "copied": true, "w": 100, "h": 50 })), &EN, false).unwrap();
    assert_eq!((c.title.as_str(), c.sub.as_str(), c.ttl_ms, c.tone, c.icon), ("Screenshot saved", "pill_1.png", 7000, Tone::Green, Some("monitor")));
    assert!(c.react_file);
    assert_eq!(c.actions.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["Open", "Show in folder", "Copy"]);
    assert!(c.actions[0].primary && !c.actions[1].primary);
    assert_eq!(c.actions[0].call, BackendCall::action("open", "C:\\p\\pill_1.png"));
    assert_eq!(c.actions[2].call, BackendCall::action("copy_image", "C:\\p\\pill_1.png"));
    let with_shelf = live::card(&done(json!({ "kind": "screenshot", "path": "p.png" })), &RU, true).unwrap();
    assert_eq!(with_shelf.actions.last().unwrap().label, "На полку");
    let copied = live::card(&done(json!({ "kind": "screenshot", "copied": true, "w": 640, "h": 480 })), &RU, true).unwrap();
    assert_eq!((copied.title.as_str(), copied.sub.as_str()), ("Скриншот скопирован", "640\u{d7}480"));
    assert!(copied.actions.is_empty() && !copied.react_file);

    let t = live::card(&done(json!({ "kind": "text", "found": true, "text": "x".repeat(100), "chars": 100 })), &EN, false).unwrap();
    assert_eq!((t.title.as_str(), t.ttl_ms, t.icon), ("Text copied", 6000, Some("scantext")));
    assert!(t.sub.ends_with("\u{2026} \u{b7} 100") && t.sub.chars().count() == 72 + 6);
    let short = live::card(&done(json!({ "kind": "text", "found": true, "text": "hello", "chars": 5 })), &EN, false).unwrap();
    assert_eq!(short.sub, "hello");
    let nt = live::card(&done(json!({ "kind": "text", "found": false })), &RU, false).unwrap();
    assert_eq!((nt.title.as_str(), nt.sub.as_str(), nt.tone, nt.ttl_ms), ("Текст не найден", "Попробуй выделить область покрупнее", Tone::Amber, 4500));

    let q = live::card(&done(json!({ "kind": "qr", "found": true, "text": "https://a.b/c", "isUrl": true })), &EN, false).unwrap();
    assert_eq!((q.title.as_str(), q.ttl_ms, q.tone), ("QR code", 12000, Tone::Accent));
    assert_eq!(q.actions[0].label, "Open link");
    assert_eq!(q.actions[0].call, BackendCall::action("open_url", "https://a.b/c"));
    assert_eq!(q.actions[1].call, BackendCall::action("copy_text", "https://a.b/c"));
    let plain = live::card(&done(json!({ "kind": "qr", "found": true, "text": "wifi" })), &EN, false).unwrap();
    assert_eq!(plain.actions.len(), 1, "no Open link for plain text");
    let nq = live::card(&done(json!({ "kind": "qr", "found": false })), &EN, false).unwrap();
    assert_eq!((nq.title.as_str(), nq.sub.as_str()), ("No QR code found", "Select the whole code, with its white border"));

    let col = live::card(&done(json!({ "kind": "color", "hex": "#FF0080", "rgb": [255, 0, 128], "history": ["#FF0080", "#111111", "#222222", "#333333", "#444444", "#555555", "#666666", "#777777", "#888888", "#999999"] })), &EN, false).unwrap();
    let cc = col.color.unwrap();
    assert_eq!((col.ttl_ms, cc.rgb_line.as_str(), cc.swatch_flash_ms), (9000, "rgb(255, 0, 128) \u{b7} copied", 400));
    assert_eq!(cc.history.len(), 9, "the picked colour is left out, then capped at 9");
    assert!(!cc.history.contains(&"#FF0080".to_string()));
    assert_eq!(live::history_click("#111111"), BackendCall::action("copy_text", "#111111"));

    let r = live::card(&done(json!({ "kind": "record", "path": "C:\\v\\pill.mp4", "seconds": 75 })), &EN, false).unwrap();
    assert_eq!((r.title.as_str(), r.sub.as_str(), r.ttl_ms), ("Recording saved", "pill.mp4 \u{b7} 1:15", 9000));
    assert!(r.react_file);
    let e = live::card(&done(json!({ "kind": "error", "message": "no   monitor\nfound" })), &RU, false).unwrap();
    assert_eq!((e.title.as_str(), e.sub.as_str(), e.tone, e.ttl_ms), ("Не получилось", "no monitor found", Tone::Red, 10000));
    assert!(live::card(&done(json!({ "kind": "mystery" })), &EN, false).is_none());
    assert!(CaptureDone::from_value(&json!({})).is_none());
    // every card string is a real web string
    for (ru, en) in [("Скриншот сохранён", "Screenshot saved"), ("Текст скопирован", "Text copied"), ("QR-код", "QR code"), ("Открыть ссылку", "Open link"), ("Запись сохранена", "Recording saved"), ("Идёт запись экрана", "Recording the screen"), ("Остановить запись (Ctrl+Alt+R)", "Stop recording (Ctrl+Alt+R)")] {
        assert!(i18n::known(ru, en), "{ru} / {en}");
    }
}

#[test]
fn recording_capsule_timer_and_start_plan() {
    let mut s = RecState::new();
    assert!(!s.recording() && s.tick(0).is_none() && s.capsule(0, &EN, false).is_none());
    let appeared = s.on_event(&RecEvent { recording: true, started_at: 10_000, mode: "screen".into(), fps: 30.0 }, 10_000);
    assert!(appeared && s.recording());
    assert_eq!(s.tick(10_100), Some("0:00".to_string()));
    assert_eq!(s.tick(10_900), None, "text unchanged within the same second");
    assert_eq!(s.tick(11_000), Some("0:01".to_string()));
    s.hidden = true;
    assert_eq!(s.tick(20_000), None, "no ticks while the island is hidden");
    s.hidden = false;
    let cap = s.capsule(75_000 + 10_000, &RU, false).unwrap();
    assert_eq!((cap.timer.as_str(), cap.format, cap.stop_label.as_str(), cap.width, cap.dot_pulse_ms, cap.priority), ("1:15", "MP4", "Стоп", 258.0, 2000, 100));
    assert_eq!(cap.stop, BackendCall::stop());
    assert_eq!((cap.dot_size, cap.stop_size, cap.stop_alpha, cap.stop_alpha_hover), (10.0, (56.0, 24.0), 0.20, 0.30));
    s.gif = true;
    assert_eq!(s.capsule(10_000, &EN, true).map(|c| (c.format, c.dot_pulse_ms)), Some(("GIF", 0)), "Reduce motion: a still dot");
    assert_eq!(s.tile(12_000, &EN), live::TileVm { recording: true, title: "Stop".into(), timer: "0:02".into() });
    assert!(s.on_event(&RecEvent { recording: false, started_at: 10_000, ..RecEvent::default() }, 30_000));
    assert!(!s.recording() && s.timer(30_000).is_empty());
    s.restore(true, Some(5_000), 9_000);
    assert_eq!(s.timer(9_000), "0:04");
    assert_eq!((live::TIMER_POLL_MS, live::START_DELAY_MS), (250, 450));
    // tile start plan: collapse, wait 450 ms, then ask; record_screen while recording toggles at once
    let p = live::start_plan("area", false);
    assert_eq!((p.collapse_first, p.delay_ms, p.call), (true, 450, BackendCall::start("area")));
    let p = live::start_plan("record_screen", false);
    assert!(p.collapse_first);
    let p = live::start_plan("record_screen", true);
    assert_eq!((p.collapse_first, p.delay_ms), (false, 0));
}
