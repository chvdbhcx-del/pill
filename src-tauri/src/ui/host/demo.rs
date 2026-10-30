//! A trivial view (rect + text + an animated spring) used to prove the host by eye and the harness headlessly.
//!
//! * Headless tests (run by default): rendering on WARP, the spring settling and the window going idle, reduced motion, input -> effects.
//! * `#[ignore]`d tests that open REAL windows: only the lead runs them by hand, see `docs/native-host.md`:
//!   `cargo test --release -- --ignored --nocapture host_demo`
use super::testing::{self, motion_lock, Sim};
use super::{HostHandle, MonitorSel, NativeWindow, WinEvent};
use crate::ui::view::{Cx, DropEvent, DropPayload, DropPhase, Msg, ProximityEvent, View, WindowCmd};
use glasskit::canvas::Canvas;
use glasskit::ease::{force_reduced, Spring};
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Buttons, InputEvent, KeyCode, Modifiers, MouseButton, Response};
use glasskit::text::Style;
use glasskit::theme::Theme;

/// What the demo says about its window in the title line.
pub struct DemoView {
    kind: &'static str,
    /// x of the dot, chasing the pointer
    dot: Spring,
    mouse: Option<Pt>,
    drawn: u64,
    hover: bool,
    typed: String,
    dropped: String,
    zone: Option<Pt>,
    edge_hits: u32,
    drag_from: Option<Pt>,
    sel: Option<(Pt, Pt)>,
}

impl DemoView {
    pub fn new(kind: &'static str) -> DemoView {
        DemoView { kind, dot: Spring::new(40.0, 420.0, 0.7), mouse: None, drawn: 0, hover: false, typed: String::new(), dropped: String::new(), zone: None, edge_hits: 0, drag_from: None, sel: None }
    }

    /// The part of the window that takes clicks (the rest of an Overlay is click-through).
    fn body(&self, r: Rect) -> Rect {
        match self.kind {
            "Overlay" => Rect::new(r.cx() - 190.0, 8.0, 380.0, 64.0),
            "Fullscreen" => r,
            _ => r.inset(12.0),
        }
    }
}

impl View for DemoView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        None
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        match ev {
            InputEvent::MouseMove { pos, .. } => {
                self.mouse = Some(*pos);
                self.dot.set_target(pos.0);
                if let Some(a) = self.drag_from {
                    self.sel = Some((a, *pos));
                }
                if self.kind == "Fullscreen" {
                    cx.set_cursor(crate::ui::view::CursorShape::Crosshair);
                }
                return Response { redraw: true, capture: self.drag_from.is_some(), ..Response::NONE };
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => {
                self.drag_from = Some(*pos);
                self.sel = Some((*pos, *pos));
                if self.kind == "Overlay" {
                    cx.set_focus(true); // an Overlay takes the keyboard only when asked
                }
                if self.kind == "Normal" && pos.1 < 36.0 && pos.0 > 760.0 {
                    cx.close_self();
                }
                return Response { redraw: true, capture: true, ..Response::NONE };
            }
            InputEvent::MouseUp { .. } => {
                self.drag_from = None;
                return Response::redraw();
            }
            InputEvent::MouseLeave => {
                self.mouse = None;
                return Response::redraw();
            }
            InputEvent::Key { code: KeyCode::Escape, down: true, .. } => {
                cx.close_self();
                return Response::consumed();
            }
            InputEvent::Key { code: KeyCode::Char('M'), down: true, mods, .. } if mods.ctrl => {
                cx.window_cmd(WindowCmd::Minimize);
            }
            InputEvent::Char(c) => {
                self.typed.push(*c);
                if self.typed.chars().count() > 40 {
                    self.typed.remove(0);
                }
                return Response::redraw();
            }
            _ => {}
        }
        Response::NONE
    }

    fn on_drop(&mut self, ev: &DropEvent, _cx: &mut Cx) -> Response {
        match (&ev.phase, &ev.payload) {
            (DropPhase::Drop, Some(DropPayload::Files(f))) => self.dropped = format!("{} file(s): {}", f.len(), f[0].display()),
            (DropPhase::Drop, Some(DropPayload::Text(t))) => self.dropped = format!("text: {t}"),
            _ => {}
        }
        Response { consumed: true, redraw: true, ..Response::NONE }
    }

    fn on_proximity(&mut self, ev: &ProximityEvent, cx: &mut Cx) -> Response {
        match ev {
            ProximityEvent::Hover => self.hover = true,
            ProximityEvent::Leave => self.hover = false,
            ProximityEvent::Zone(z) => self.zone = *z,
            ProximityEvent::TopEdge => self.edge_hits += 1,
            ProximityEvent::Gaze(_) => {}
        }
        // the click-through island: only the body is clickable, and a drop zone makes a file drag land
        cx.set_hit_region(vec![self.body(Rect::new(0.0, 0.0, 720.0, 120.0))]);
        cx.set_drop_zone(Some(Rect::new(110.0, 0.0, 500.0, 90.0)));
        Response::redraw()
    }

    fn message(&mut self, _msg: Msg, _cx: &mut Cx) -> Response {
        Response::redraw()
    }

    fn step(&mut self, dt_ms: f64, _cx: &mut Cx) -> bool {
        self.dot.step((dt_ms / 1000.0) as f32)
    }

    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx) {
        self.drawn += 1;
        let body = self.body(rect);
        let fill = if self.hover { theme.raised } else { theme.card };
        cv.fill_rr(body, 18.0, fill);
        cv.stroke_rr(body, 18.0, 1.0, theme.border);
        let (title, rest) = body.cut_top(28.0);
        cv.text(title.inset_xy(14.0, 0.0), &format!("{} - {:.2}x - {:.0} Hz{}", self.kind, cx.scale, cx.refresh_hz, if cx.reduced_motion { " - reduced motion" } else { "" }), theme.base(), theme.text);
        let info = format!(
            "frames drawn {}   pointer {}   typed \"{}\"   {}",
            self.drawn,
            self.mouse.map_or("-".to_string(), |p| format!("{:.0},{:.0}", p.0, p.1)),
            self.typed,
            self.dropped
        );
        cv.text(rest.inset_xy(14.0, 0.0).cut_top(24.0).0, &info, theme.caption(), theme.dim);
        if self.kind == "Overlay" {
            let z = self.zone.map_or("zone -".to_string(), |p| format!("zone {:.0},{:.0}", p.0, p.1));
            cv.text(rest.inset_xy(14.0, 0.0).cut_bottom(20.0).1, &format!("{z}   top edge x{}", self.edge_hits), Style::CAPTION, theme.dim);
        }
        // the animated dot: a spring chasing the pointer
        let cy = body.bottom() - 16.0;
        cv.fill_circle((self.dot.x.clamp(body.x + 12.0, body.right() - 12.0), cy), 7.0, theme.accent);
        if self.kind == "Normal" {
            cv.text(Rect::new(rect.w - 90.0, 0.0, 60.0, 36.0), "close", Style::BODY, theme.dim);
        }
        if let Some((a, b)) = self.sel {
            let r = Rect::new(a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs());
            cv.fill_rect(r, theme.accent.fade(0.25));
            cv.stroke_rr(r, 0.0, 1.0, theme.accent);
        }
    }
}

fn mm(x: f32, y: f32) -> InputEvent {
    InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
}

#[test]
fn renders_headlessly_at_two_scales() {
    for scale in [1.0, 2.0] {
        let mut v = DemoView::new("Popup");
        let img = testing::render_view(&mut v, (400.0, 200.0), scale, &Theme::pill_dark());
        if let Some(dir) = std::env::var_os("PILL_HOST_DUMP") {
            testing::save_png(&img, std::path::Path::new(&dir).join(format!("demo-popup-{scale}x.png")));
        }
        assert_eq!((img.w, img.h), ((400.0 * scale) as u32, (200.0 * scale) as u32));
        assert_eq!(img.at(2.0, 2.0)[3], 0, "the corner outside the body stays transparent (per-pixel alpha)");
        assert!(img.at(200.0, 100.0)[3] > 200, "the card is opaque-ish");
        assert!(img.count(None, |p| p[3] > 0) > 1000);
    }
}

#[test]
fn the_dot_animates_then_the_window_idles() {
    let _g = motion_lock();
    force_reduced(Some(false));
    let mut s = Sim::new(Box::new(DemoView::new("Popup")));
    s.show();
    assert_eq!(s.run(1000), 1, "the first frame after show settles at once: nothing is moving yet");
    assert!(!s.frame.wants_frame());
    s.send(&mm(300.0, 50.0));
    let n = s.run(2000);
    assert!(n > 20, "a spring needs many frames, got {n}");
    assert!(n < 2000, "and then it stops");
    assert_eq!(s.run(10), 0, "idle: zero frames, zero work");
    // 240 Hz: one frame is ~4.17 ms of simulated time
    assert!((s.now_ms / s.frames_run as f64 - 1000.0 / 240.0).abs() < 1e-6);
    force_reduced(None);
}

#[test]
fn reduced_motion_lands_in_one_frame() {
    let _g = motion_lock();
    force_reduced(Some(true));
    let mut s = Sim::new(Box::new(DemoView::new("Popup")));
    s.show();
    s.run(10);
    s.send(&mm(300.0, 50.0));
    assert_eq!(s.run(100), 1, "the spring jumps to its target");
    force_reduced(None);
}

#[test]
fn input_becomes_effects_and_escape_closes() {
    let mut s = Sim::new(Box::new(DemoView::new("Overlay")));
    s.send(&InputEvent::MouseDown { pos: (360.0, 30.0), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
    let fx = s.take_fx();
    assert_eq!(fx.focus, Some(true), "clicking the island asks for the keyboard");
    assert!(!fx.close);
    let r = s.send(&InputEvent::Key { code: KeyCode::Escape, mods: Modifiers::NONE, down: true, repeat: false });
    assert!(r.consumed);
    assert!(s.take_fx().close);
}

#[test]
fn proximity_and_drops_reach_the_view() {
    let mut s = Sim::new(Box::new(DemoView::new("Overlay")));
    s.send_proximity(&ProximityEvent::Hover);
    let fx = s.take_fx();
    assert_eq!(fx.hit_region.as_ref().map(|r| r.len()), Some(1), "the island declares its clickable body");
    assert!(matches!(fx.drop_zone, Some(Some(_))));
    let r = s.send_drop(&DropEvent { phase: DropPhase::Enter, pos: (300.0, 40.0), payload: Some(DropPayload::Text("x".into())) });
    assert!(r.consumed, "Enter accepted -> the copy cursor");
    s.send_drop(&DropEvent { phase: DropPhase::Drop, pos: (300.0, 40.0), payload: Some(DropPayload::Files(vec!["C:\\a.txt".into()])) });
    let img = s.render((720.0, 120.0), &Theme::pill_dark());
    assert!(img.count(None, |p| p[3] > 0) > 500);
}

#[test]
fn drive_runs_a_whole_session() {
    let _g = motion_lock();
    force_reduced(Some(false));
    let s = testing::drive(Box::new(DemoView::new("Normal")), &[mm(100.0, 100.0), mm(500.0, 100.0)], 5000);
    assert!(s.frames_run > 40);
    assert!(!s.frame.wants_frame());
    force_reduced(None);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// By hand (real windows). Never runs in CI: `cargo test --release -- --ignored --nocapture host_demo`
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Creates one window of each kind but NEVER shows them: proves window creation per kind, the DirectComposition swap chain, the command
/// queue and shutdown without anything appearing on screen. Needs a desktop session and a GPU (or WARP).
/// `cargo test --release -- --ignored --nocapture host_smoke_hidden` (add `PILL_FORCE_LAYERED=1` to exercise the fallback).
#[test]
#[ignore = "creates real (hidden, never shown) windows; needs a desktop session"]
fn host_smoke_hidden() {
    use super::WinEvent::{Closed, Created};
    use std::sync::mpsc::channel;
    use std::time::Duration;
    let host = super::start();
    let (tx, rx) = channel();
    let mk = |b: NativeWindow, label: &'static str| {
        let tx = tx.clone();
        b.hidden().on_event(move |_, e| {
            let _ = tx.send((label, e));
        })
    };
    let ids = [
        mk(NativeWindow::overlay("smoke-overlay").accept_drops(), "overlay").spawn(&host, |_| Box::new(DemoView::new("Overlay"))),
        mk(NativeWindow::popup("smoke-popup"), "popup").spawn(&host, |_| Box::new(DemoView::new("Popup"))),
        mk(NativeWindow::normal("smoke-normal"), "normal").spawn(&host, |_| Box::new(DemoView::new("Normal"))),
        mk(NativeWindow::fullscreen("smoke-full", MonitorSel::Primary), "fullscreen").spawn(&host, |_| Box::new(DemoView::new("Fullscreen"))),
    ];
    let mut created = 0;
    while created < 4 {
        let (label, ev) = rx.recv_timeout(Duration::from_secs(10)).expect("a window must be created within 10 s");
        if ev == Created {
            created += 1;
            println!("created {label}");
        } else {
            panic!("{label}: unexpected {ev:?}");
        }
    }
    assert_eq!(host.open_windows(), 4);
    for id in ids {
        assert!(host.hwnd(id).is_some(), "{id:?} has a window handle");
        println!("{id:?}: {:?} via {:?}", host.hwnd(id), host.backend(id));
        assert!(!host.is_visible(id));
        host.request_frame(id);
        host.post(id, Msg::Open(serde_json::json!({ "x": 1 })));
    }
    for id in ids {
        host.close(id);
    }
    let mut closed = 0;
    while closed < 4 {
        let (_, ev) = rx.recv_timeout(Duration::from_secs(10)).expect("closing must be reported");
        if ev == Closed {
            closed += 1;
        }
    }
    assert_eq!(host.open_windows(), 0);
    super::shutdown();
}

/// Opens one window of each kind. Esc (or the close label) closes a window; the test ends when all are closed or after
/// `PILL_DEMO_SECS` (default 120). See `docs/native-host.md` for the checklist of what to look at.
#[test]
#[ignore = "opens real windows; run by hand: cargo test --release -- --ignored --nocapture host_demo"]
fn host_demo() {
    let host: HostHandle = super::start();
    let log = |label: &'static str| move |id, e: WinEvent| println!("[{label} {id:?}] {e:?}");
    let secs: u64 = std::env::var("PILL_DEMO_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    println!("monitors: {:?}", host.monitors());
    let overlay = NativeWindow::overlay("demo-overlay").accept_drops().on_event(log("overlay")).spawn(&host, |cx| {
        let v = DemoView::new("Overlay");
        // only the 380 x 64 body takes clicks; a file drag over the top 90 px makes the window hit-testable so the drop can land
        cx.set_hit_region(vec![v.body(Rect::new(0.0, 0.0, 720.0, 120.0))]);
        cx.set_drop_zone(Some(Rect::new(110.0, 0.0, 500.0, 90.0)));
        Box::new(v)
    });
    let popup = NativeWindow::popup("demo-popup").hidden().on_event(log("popup")).spawn(&host, |_| Box::new(DemoView::new("Popup")));
    let _normal = NativeWindow::normal("demo-normal").on_event(log("normal")).spawn(&host, |cx| {
        cx.set_caption_region(vec![Rect::new(0.0, 0.0, 740.0, 36.0)]);
        Box::new(DemoView::new("Normal"))
    });
    let _full = NativeWindow::fullscreen("demo-full", MonitorSel::Primary).on_event(log("fullscreen")).spawn(&host, |_| Box::new(DemoView::new("Fullscreen")));
    std::thread::sleep(std::time::Duration::from_millis(1500));
    println!("popup: shown now; click elsewhere and it hides (blur). toggling it twice in 3 s");
    host.show(popup);
    let start = std::time::Instant::now();
    let mut toggled = false;
    while host.open_windows() > 0 && start.elapsed().as_secs() < secs {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if !toggled && start.elapsed().as_secs() > 6 {
            toggled = true;
            host.toggle(popup);
        }
    }
    println!("overlay hwnd {:?}, windows still open: {}", host.hwnd(overlay), host.open_windows());
    super::shutdown();
}

/// Measures the frame loop for real WITHOUT showing anything to the user: an Overlay is placed far outside every monitor
/// (`Placement::At { -30000, -30000 }`), animated for 2 s, then left idle. Expect about `2 * refresh` frames while animating (vsync pace,
/// no cap) and ZERO frames afterwards. If DWM reports the off-screen window as occluded the frame rate collapses to 10/s and the test says so.
/// `cargo test --release -- --ignored --nocapture host_pacing_offscreen`
#[test]
#[ignore = "creates a real window far off-screen; needs a desktop session"]
fn host_pacing_offscreen() {
    use super::Placement;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct Pacer {
        drawn: Arc<AtomicU64>,
        span: Arc<Mutex<Option<(Instant, Instant)>>>,
        left_ms: f64,
        max_dt: Arc<Mutex<f64>>,
    }
    impl View for Pacer {
        fn message(&mut self, _m: Msg, _cx: &mut Cx) -> Response {
            self.left_ms = 2000.0;
            Response::redraw()
        }
        fn step(&mut self, dt_ms: f64, _cx: &mut Cx) -> bool {
            let now = Instant::now();
            let mut s = self.span.lock().unwrap();
            *s = Some(s.map_or((now, now), |(a, _)| (a, now)));
            let mut m = self.max_dt.lock().unwrap();
            *m = m.max(dt_ms);
            self.left_ms -= dt_ms;
            self.left_ms > 0.0
        }
        fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, _cx: &Cx) {
            let n = self.drawn.fetch_add(1, Ordering::Relaxed);
            cv.fill_rr(rect.inset(4.0), 12.0, theme.card);
            cv.fill_circle(((n % 200) as f32 + 20.0, 40.0), 6.0, theme.accent);
        }
    }

    let host = super::start();
    let visible = std::env::var_os("PILL_PACING_VISIBLE").is_some(); // the lead: a 300 x 80 box appears 60 px from the primary monitor's corner for 3 s
    let hz = host.monitors().first().map_or(60, |m| m.refresh_hz);
    let (drawn, span, max_dt) = (Arc::new(AtomicU64::new(0)), Arc::new(Mutex::new(None)), Arc::new(Mutex::new(0.0)));
    let (d2, s2, m2) = (drawn.clone(), span.clone(), max_dt.clone());
    let id = NativeWindow::overlay("pacing")
        .placement(if visible { Placement::At { x: host.monitors()[0].rect.l + 60, y: host.monitors()[0].rect.t + 60 } } else { Placement::At { x: -30000, y: -30000 } })
        .size(300.0, 80.0)
        .spawn(&host, move |_| Box::new(Pacer { drawn: d2, span: s2, left_ms: 0.0, max_dt: m2 }));
    std::thread::sleep(Duration::from_millis(800));
    let before = drawn.load(Ordering::Relaxed);
    println!("display {hz} Hz, backend {:?}, first frame(s): {before}", host.backend(id));
    host.post(id, Msg::Custom(Box::new(())));
    std::thread::sleep(Duration::from_millis(2600));
    let total = drawn.load(Ordering::Relaxed);
    let (a, b) = span.lock().unwrap().expect("the animation ran");
    let secs = b.duration_since(a).as_secs_f64();
    let frames = total - before;
    println!("animated {frames} frames in {secs:.3} s = {:.1} fps (display {hz} Hz), largest dt {:.2} ms", frames as f64 / secs, *max_dt.lock().unwrap());
    std::thread::sleep(Duration::from_millis(500));
    let idle = drawn.load(Ordering::Relaxed) - total;
    println!("frames while idle (500 ms): {idle}");
    assert_eq!(idle, 0, "a settled window must not render");
    let fps = frames as f64 / secs;
    assert!(fps < hz as f64 * 2.2, "the runaway guard must hold an unpaced (off-screen) swap chain to ~2x the refresh rate, got {fps:.1} fps");
    if std::env::var_os("PILL_PACING_VISIBLE").is_some() {
        assert!(fps > hz as f64 * 0.9, "vsync pace expected, got {fps:.1} fps");
        assert!(fps < hz as f64 * 1.1, "never faster than the display either");
    }
    super::shutdown();
}
