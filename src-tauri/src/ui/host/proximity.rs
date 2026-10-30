//! Cursor proximity for the click-through island: the native counterpart of `pill.rs::cursor_loop` + `set_ignore`, as a pure state machine.
//!
//! Why it exists: a window with `WS_EX_TRANSPARENT` receives no mouse messages, so something must read the cursor and take the style off the
//! moment the pointer reaches the island. (`WM_NCHITTEST -> HTTRANSPARENT` only passes clicks to windows of the SAME thread, so the style is
//! the cross-process mechanism; the host answers `WM_NCHITTEST` as well for completeness.) The host polls the cursor and feeds a [`Probe`]
//! to [`Proximity::step`]; the answer says whether the window must be click-through right now, which [`ProximityEvent`]s to deliver to the
//! view, and when to poll next.
//!
//! What is reproduced from pill.rs (numbers included):
//! * `ignore` = the cursor is NOT within 4 px of a hit rect (and no file drag is engaged), or pass-through mode is on;
//! * `Hover` once per visit, `Leave` only after the cursor stayed out for 250 ms; a button held since before the cursor arrived is a drag
//!   (armed) and never opens the island; a drag that ended on the island suppresses hover until the cursor leaves it;
//! * `Zone` stream while the cursor is in the window frame or within 120 logical px of the hit bounds (`None` once on leaving);
//! * `TopEdge` after 120 ms pinned on the monitor's top row inside the island's x range with no button held;
//! * drag-to-shelf: a left-button drag that started elsewhere and enters the drop zone makes the window hit-testable (hysteresis +16 px) so
//!   OLE can hand it the files;
//! * `Gaze`: the cursor in physical px relative to the window, only when asked and only when it moved;
//! * poll cadence: display period (floored at 6 ms) while near, 30 ms when the cursor rests, 100 ms far (50 ms / one frame with gaze).
use super::geom::{rect_to_phys, HitRegion, IRect};
use crate::ui::view::ProximityEvent;
use glasskit::geom::Rect;

/// Slack around a hit rect (physical px) so the edge pixels stay grabbable.
const PAD: i32 = 4;
/// Logical px around the hit bounds where the `Zone` stream runs.
const ZONE: f32 = 120.0;
const LEAVE_MS: f64 = 250.0;
const EDGE_MS: f64 = 120.0;
/// Hysteresis of an engaged drop zone, logical px.
const DRAG_HYST: f32 = 16.0;
/// Fastest poll period (never spin faster than ~166 Hz).
const MIN_PERIOD_MS: f64 = 6.0;

/// Drag-to-shelf state (the same machine as `pill.rs::Drag`).
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Drag {
    /// the button is down and the cursor has been outside the zone since (the press did not start on the zone)
    pub armed: bool,
    /// armed and inside the zone: the window is hit-testable
    pub engaged: bool,
}

/// One step of the drag machine. `near` = cursor in the zone (with hysteresis once engaged), `eligible` = a drop zone is set and the island
/// is not in pass-through mode.
pub fn drag_step(d: Drag, held: bool, near: bool, eligible: bool) -> Drag {
    if !held || !eligible {
        return Drag::default();
    }
    let armed = d.armed || !near;
    Drag { armed, engaged: armed && near }
}

/// What the host reads from the OS each poll.
#[derive(Clone, Copy, Debug)]
pub struct Probe {
    pub now_ms: f64,
    /// screen position, physical px
    pub cursor: (i32, i32),
    /// the window's rectangle on screen, physical px
    pub window: IRect,
    /// top row of the monitor the window is on
    pub monitor_top: i32,
    /// the left mouse button is down (`GetAsyncKeyState(VK_LBUTTON)`)
    pub button_down: bool,
    /// the window is shown (a hidden island polls slowly and forgets its state)
    pub visible: bool,
}

/// What a poll decided.
#[derive(Clone, Debug, PartialEq)]
pub struct PollOut {
    /// Make the window click-through (`WS_EX_TRANSPARENT`) until the next poll.
    pub ignore: bool,
    pub events: Vec<ProximityEvent>,
    /// Poll again after this many milliseconds.
    pub next_poll_ms: f64,
}

pub struct Proximity {
    scale: f32,
    hz: u32,
    hit: HitRegion,
    pass_through: bool,
    drop_zone: Option<Rect>,
    gaze: bool,
    inside: bool,
    left_at: Option<f64>,
    edge_at: Option<f64>,
    zone_on: bool,
    last: (i32, i32),
    gaze_last: (i32, i32),
    drag: Drag,
    no_hover: bool,
    seen: Option<((i32, i32), bool, IRect, bool)>,
    still: u32,
}

impl Proximity {
    pub fn new(scale: f32, refresh_hz: u32) -> Proximity {
        Proximity {
            scale,
            hz: refresh_hz,
            hit: HitRegion::All,
            pass_through: false,
            drop_zone: None,
            gaze: false,
            inside: false,
            left_at: None,
            edge_at: None,
            zone_on: false,
            last: (i32::MIN, i32::MIN),
            gaze_last: (i32::MIN, i32::MIN),
            drag: Drag::default(),
            no_hover: false,
            seen: None,
            still: 0,
        }
    }

    pub fn set_scale(&mut self, s: f32) {
        self.scale = s;
    }
    pub fn set_refresh_hz(&mut self, hz: u32) {
        self.hz = hz;
    }
    pub fn set_hit_region(&mut self, h: HitRegion) {
        self.hit = h;
        self.seen = None;
    }
    pub fn set_pass_through(&mut self, on: bool) {
        self.pass_through = on;
        self.seen = None;
    }
    pub fn set_drop_zone(&mut self, z: Option<Rect>) {
        self.drop_zone = z;
    }
    pub fn set_gaze(&mut self, on: bool) {
        self.gaze = on;
        self.gaze_last = (i32::MIN, i32::MIN);
    }
    pub fn pass_through(&self) -> bool {
        self.pass_through
    }
    pub fn hit_region(&self) -> &HitRegion {
        &self.hit
    }
    pub fn engaged(&self) -> bool {
        self.drag.engaged
    }

    fn period_ms(&self) -> f64 {
        (1000.0 / if self.hz == 0 { 60.0 } else { self.hz as f64 }).max(MIN_PERIOD_MS)
    }

    fn forget(&mut self) {
        self.inside = false;
        self.left_at = None;
        self.edge_at = None;
        self.zone_on = false;
        self.gaze_last = (i32::MIN, i32::MIN);
        self.drag = Drag::default();
        self.seen = None;
    }

    /// One poll.
    pub fn step(&mut self, p: &Probe) -> PollOut {
        let mut events = Vec::new();
        if !p.visible {
            self.forget();
            return PollOut { ignore: self.pass_through, events, next_poll_ms: 100.0 };
        }
        let (cx, cy) = p.cursor;
        let wr = p.window;
        let s = self.scale;
        // hit rects on screen
        let rects: Vec<IRect> = match &self.hit {
            HitRegion::All => vec![wr],
            HitRegion::Rects(rs) => rs.iter().map(|r| rect_to_phys(*r, s).offset(wr.l, wr.t)).collect(),
        };
        let in_rect = rects.iter().any(|r| r.inflate(PAD).contains(cx, cy));
        let bounds = rects.iter().copied().reduce(|a, b| a.union(&b));
        let held = p.button_down;
        // drag-to-shelf
        let eligible = self.drop_zone.is_some() && !self.pass_through;
        let near_drop = self.drop_zone.is_some_and(|z| {
            let m = if self.drag.engaged { (DRAG_HYST * s).round() as i32 } else { 0 };
            rect_to_phys(z, s).offset(wr.l, wr.t).inflate(m).contains(cx, cy)
        });
        let nd = drag_step(self.drag, held, near_drop, eligible);
        if self.drag.armed && !nd.armed && in_rect {
            self.no_hover = true; // the drag ended on the island (a drop): no hover-open until the cursor leaves it
        }
        self.no_hover &= in_rect;
        self.drag = nd;
        // click-through: with `All` the window is always clickable (nothing to poll for), otherwise only near the hit rects
        let ignore = if self.pass_through {
            true
        } else if matches!(self.hit, HitRegion::All) {
            false
        } else {
            !(in_rect || self.drag.engaged)
        };
        // top-edge dwell
        let x_in = bounds.is_some_and(|b| cx >= b.l - PAD && cx < b.r + PAD);
        let on_edge = x_in && cy <= p.monitor_top + 1 && !held;
        self.edge_at = if on_edge { self.edge_at.or(Some(p.now_ms)) } else { None };
        if self.edge_at.is_some_and(|t| p.now_ms - t >= EDGE_MS) {
            self.edge_at = None;
            events.push(ProximityEvent::TopEdge);
        }
        // hover / leave
        if !self.pass_through && in_rect {
            self.left_at = None;
            if !self.inside && !self.drag.armed && !self.no_hover {
                self.inside = true;
                events.push(ProximityEvent::Hover);
            }
        } else if self.inside {
            match self.left_at {
                None => self.left_at = Some(p.now_ms),
                Some(t) if p.now_ms - t >= LEAVE_MS => {
                    self.inside = false;
                    self.left_at = None;
                    events.push(ProximityEvent::Leave);
                }
                _ => {}
            }
        }
        // zone stream
        let in_frame = wr.contains(cx, cy);
        let zm = (ZONE * s).round() as i32;
        let zone = in_frame || bounds.is_some_and(|b| b.inflate(zm).contains(cx, cy));
        if zone {
            if !self.zone_on || p.cursor != self.last {
                self.zone_on = true;
                self.last = p.cursor;
                events.push(ProximityEvent::Zone(Some(((cx - wr.l) as f32 / s, (cy - wr.t) as f32 / s))));
            }
        } else if self.zone_on {
            self.zone_on = false;
            events.push(ProximityEvent::Zone(None));
        }
        // gaze
        let mut moved = false;
        if self.gaze {
            if p.cursor != self.gaze_last {
                self.gaze_last = p.cursor;
                moved = true;
                events.push(ProximityEvent::Gaze(((cx - wr.l) as f32, (cy - wr.t) as f32)));
            }
        } else {
            self.gaze_last = (i32::MIN, i32::MIN);
        }
        // cadence
        let key = Some((p.cursor, self.inside, wr, ignore));
        self.still = if key == self.seen { self.still.saturating_add(1) } else { 0 };
        self.seen = key;
        let near = self.inside || zone;
        let next = if near {
            if self.still >= 3 {
                30.0
            } else {
                self.period_ms()
            }
        } else if self.gaze {
            if moved {
                self.period_ms().max(16.0)
            } else {
                50.0
            }
        } else {
            100.0
        };
        PollOut { ignore, events, next_poll_ms: next }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 720 x 120 window at the top centre of a 2560 x 1440 monitor, the island body 140 x 36 in its middle (+ a 14 px liquid strip).
    const WIN: IRect = IRect::new(920, 0, 1640, 120);
    fn body() -> HitRegion {
        HitRegion::Rects(vec![Rect::new(290.0, 7.0, 140.0, 36.0), Rect::new(300.0, 43.0, 120.0, 14.0)])
    }
    fn probe(now: f64, x: i32, y: i32) -> Probe {
        Probe { now_ms: now, cursor: (x, y), window: WIN, monitor_top: 0, button_down: false, visible: true }
    }
    fn prox() -> Proximity {
        let mut p = Proximity::new(1.0, 240);
        p.set_hit_region(body());
        p
    }
    fn ev(o: &PollOut) -> Vec<ProximityEvent> {
        o.events.iter().filter(|e| !matches!(e, ProximityEvent::Zone(_))).cloned().collect()
    }

    #[test]
    fn click_through_everywhere_but_on_the_island() {
        let mut p = prox();
        assert!(p.step(&probe(0.0, 100, 500)).ignore, "far away");
        assert!(p.step(&probe(10.0, 1000, 100)).ignore, "inside the window frame but outside the body");
        assert!(!p.step(&probe(20.0, 1280, 20)).ignore, "on the body");
        assert!(!p.step(&probe(30.0, 920 + 290 - 4, 20)).ignore, "4 px of slack");
        assert!(p.step(&probe(40.0, 920 + 290 - 5, 20)).ignore);
        assert!(!p.step(&probe(50.0, 1280, 56 + 3)).ignore, "the liquid strip and its slack");
    }

    #[test]
    fn region_all_is_never_click_through_and_pass_through_always_is() {
        let mut p = Proximity::new(1.0, 60);
        assert!(!p.step(&probe(0.0, 5, 5)).ignore);
        p.set_pass_through(true);
        let o = p.step(&probe(10.0, 1280, 20));
        assert!(o.ignore && !ev(&o).contains(&ProximityEvent::Hover), "pass-through: no hover either");
        p.set_pass_through(false);
        assert!(!p.step(&probe(20.0, 5, 5)).ignore);
    }

    #[test]
    fn hover_once_leave_after_250ms() {
        let mut p = prox();
        assert_eq!(ev(&p.step(&probe(0.0, 1280, 20))), vec![ProximityEvent::Hover]);
        assert_eq!(ev(&p.step(&probe(10.0, 1281, 20))), vec![], "no second hover");
        // leaves: nothing for 250 ms
        assert_eq!(ev(&p.step(&probe(100.0, 1280, 300))), vec![]);
        assert_eq!(ev(&p.step(&probe(340.0, 1280, 300))), vec![], "249 ms");
        assert_eq!(ev(&p.step(&probe(350.0, 1280, 300))), vec![ProximityEvent::Leave]);
        // coming back within the delay cancels the leave
        let mut p = prox();
        p.step(&probe(0.0, 1280, 20));
        p.step(&probe(100.0, 1280, 300));
        p.step(&probe(200.0, 1280, 20));
        assert_eq!(ev(&p.step(&probe(600.0, 1280, 20))), vec![]);
    }

    #[test]
    fn zone_stream_sends_changes_only_and_ends_with_none() {
        let mut p = prox();
        let z = |o: &PollOut| o.events.iter().filter_map(|e| if let ProximityEvent::Zone(z) = e { Some(*z) } else { None }).collect::<Vec<_>>();
        assert_eq!(z(&p.step(&probe(0.0, 100, 900))), vec![], "far");
        assert_eq!(z(&p.step(&probe(10.0, 1280, 150))), vec![Some((360.0, 150.0))], "below the window but within 120 px of the body");
        assert_eq!(z(&p.step(&probe(20.0, 1280, 150))), vec![], "unchanged position: silence");
        assert_eq!(z(&p.step(&probe(30.0, 1290, 150))), vec![Some((370.0, 150.0))]);
        assert_eq!(z(&p.step(&probe(40.0, 1290, 900))), vec![None], "one None on leaving");
        assert_eq!(z(&p.step(&probe(50.0, 1290, 901))), vec![], "then silence");
        assert_eq!(z(&p.step(&probe(60.0, 1000, 100))), vec![Some((80.0, 100.0))], "inside the window frame counts");
    }

    #[test]
    fn top_edge_needs_120ms_on_the_top_row_and_no_button() {
        let mut p = prox();
        let edge = |o: &PollOut| o.events.contains(&ProximityEvent::TopEdge);
        assert!(!edge(&p.step(&probe(0.0, 1280, 0))));
        assert!(!edge(&p.step(&probe(119.0, 1280, 0))));
        assert!(edge(&p.step(&probe(120.0, 1280, 0))));
        assert!(!edge(&p.step(&probe(125.0, 1280, 0))), "the dwell restarts after firing");
        assert!(edge(&p.step(&probe(250.0, 1280, 0))));
        // outside the island's x range
        let mut p = prox();
        p.step(&probe(0.0, 100, 0));
        assert!(!edge(&p.step(&probe(500.0, 100, 0))));
        // a held button (dragging a window to the top) does not count
        let mut p = prox();
        let mut pr = probe(0.0, 1280, 0);
        pr.button_down = true;
        p.step(&pr);
        pr.now_ms = 500.0;
        assert!(!edge(&p.step(&pr)));
        // moving away resets the dwell
        let mut p = prox();
        p.step(&probe(0.0, 1280, 0));
        p.step(&probe(100.0, 1280, 200));
        assert!(!edge(&p.step(&probe(150.0, 1280, 0))));
    }

    #[test]
    fn a_file_drag_into_the_drop_zone_makes_the_window_hit_testable() {
        let mut p = prox();
        p.set_drop_zone(Some(Rect::new(230.0, 0.0, 260.0, 90.0)));
        let mut pr = probe(0.0, 100, 500);
        pr.button_down = true;
        assert!(p.step(&pr).ignore, "armed on the way, still click-through far away");
        pr.cursor = (1280, 70);
        pr.now_ms = 10.0;
        let o = p.step(&pr);
        assert!(!o.ignore && p.engaged(), "engaged inside the zone although the body is not under the cursor");
        assert!(!ev(&o).contains(&ProximityEvent::Hover), "a drag never opens the island");
        // hysteresis keeps it engaged just outside
        pr.cursor = (920 + 230 - 10, 70);
        pr.now_ms = 20.0;
        assert!(!p.step(&pr).ignore);
        pr.cursor = (920 + 230 - 20, 70);
        pr.now_ms = 30.0;
        assert!(p.step(&pr).ignore && !p.engaged());
        // release resets
        pr.cursor = (1280, 70);
        p.step(&pr);
        pr.button_down = false;
        pr.now_ms = 40.0;
        assert!(p.step(&pr).ignore && !p.engaged());
    }

    #[test]
    fn a_press_that_starts_on_the_zone_is_not_a_file_drag() {
        let mut p = prox();
        p.set_drop_zone(Some(Rect::new(230.0, 0.0, 260.0, 90.0)));
        let mut pr = probe(0.0, 1280, 70);
        pr.button_down = true;
        for i in 0..5 {
            pr.now_ms = i as f64;
            assert!(!p.step(&pr).events.contains(&ProximityEvent::Hover) || i == 0);
            assert!(!p.engaged());
        }
    }

    #[test]
    fn a_drop_on_the_island_does_not_pop_it_open() {
        let mut p = prox();
        p.set_drop_zone(Some(Rect::new(230.0, 0.0, 260.0, 90.0)));
        let mut pr = probe(0.0, 100, 500);
        pr.button_down = true;
        p.step(&pr);
        pr.cursor = (1280, 20);
        pr.now_ms = 10.0;
        p.step(&pr);
        pr.button_down = false;
        pr.now_ms = 20.0;
        assert!(!ev(&p.step(&pr)).contains(&ProximityEvent::Hover), "released on the island: no hover");
        pr.cursor = (1281, 21);
        pr.now_ms = 30.0;
        assert!(!ev(&p.step(&pr)).contains(&ProximityEvent::Hover), "still none while the cursor stays");
        pr.cursor = (100, 500);
        pr.now_ms = 40.0;
        p.step(&pr);
        pr.cursor = (1280, 20);
        pr.now_ms = 50.0;
        assert!(ev(&p.step(&pr)).contains(&ProximityEvent::Hover), "after leaving and coming back it hovers again");
    }

    #[test]
    fn gaze_streams_physical_positions_only_when_asked_and_moved() {
        let mut p = prox();
        assert_eq!(ev(&p.step(&probe(0.0, 3000, 900))), vec![]);
        p.set_gaze(true);
        let o = p.step(&probe(10.0, 3000, 900));
        assert_eq!(ev(&o), vec![ProximityEvent::Gaze((2080.0, 900.0))], "any distance, any monitor, relative to the window");
        assert_eq!(o.next_poll_ms, 6.0f64.max(1000.0 / 240.0).max(16.0), "moving: at most 60 Hz when far");
        let o = p.step(&probe(20.0, 3000, 900));
        assert_eq!(ev(&o), vec![]);
        assert_eq!(o.next_poll_ms, 50.0, "resting: relaxed");
        p.set_gaze(false);
        assert_eq!(ev(&p.step(&probe(30.0, 3001, 900))), vec![]);
    }

    #[test]
    fn poll_cadence() {
        let mut p = prox();
        assert_eq!(p.step(&probe(0.0, 100, 900)).next_poll_ms, 100.0, "far");
        let o = p.step(&probe(10.0, 1280, 20));
        assert_eq!(o.next_poll_ms, 6.0, "near: one display period (4.2 ms at 240 Hz) floored at 6 ms");
        let mut last = 0.0;
        for i in 0..5 {
            last = p.step(&probe(20.0 + i as f64, 1280, 20)).next_poll_ms;
        }
        assert_eq!(last, 30.0, "cursor resting on the island: relaxed poll");
        p.set_refresh_hz(60);
        assert_eq!(p.step(&probe(100.0, 1285, 20)).next_poll_ms, 1000.0 / 60.0);
        let mut hidden = probe(110.0, 1280, 20);
        hidden.visible = false;
        let o = p.step(&hidden);
        assert_eq!(o.next_poll_ms, 100.0);
        assert!(o.events.is_empty());
    }

    #[test]
    fn a_hidden_window_forgets_hover_state() {
        let mut p = prox();
        p.step(&probe(0.0, 1280, 20));
        let mut hidden = probe(10.0, 1280, 20);
        hidden.visible = false;
        p.step(&hidden);
        assert!(ev(&p.step(&probe(20.0, 1280, 20))).contains(&ProximityEvent::Hover), "shown again: a fresh visit");
    }

    #[test]
    fn dpi_scales_the_pad_zone_and_rects() {
        // 200 %: logical rects double on screen; the window is 1440 wide
        let win = IRect::new(1840, 0, 3280, 240);
        let mut p = Proximity::new(2.0, 144);
        p.set_hit_region(body());
        let mk = |x, y| Probe { now_ms: 0.0, cursor: (x, y), window: win, monitor_top: 0, button_down: false, visible: true };
        assert!(!p.step(&mk(1840 + 600, 40)).ignore, "(300,20) logical is on the body");
        assert!(p.step(&mk(1840 + 560 - 5, 40)).ignore, "left of the body by more than the 4 px slack");
    }
}
