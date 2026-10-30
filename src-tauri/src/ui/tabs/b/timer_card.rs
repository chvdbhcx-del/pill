//! The right-hand card of the Note tab: the timer ring with Start / Custom time / presets, and the reminders list with its input.
//!
//! A pure renderer of `timer::TimerView` (the model lives in the Note tab's `NoteTab.card`): it paints, answers pointer / key input, and
//! hands the user's gestures back as `timer::Action`s through [`TimerCardView::drain`]. Layout and look follow `tab-note.css`
//! (`.tn-card .tn-top .tn-ring .tn-start .tn-ctime .tn-presets .tn-rhead .tn-r .tn-rempty .tn-rmsg .tn-rin`).
//!
//! * Text input (the custom-time field and the reminder field) is deferred to the next draw, see `kit::Field`.
//! * Animation: the ring arc eases over .26 s (linear, like `stroke-dashoffset` in the CSS), the arc fades in/out over .2 s, reminder rows
//!   rise in (6 px + fade), collapse when done / deleted, slide to their new place on a reorder (FLIP) and fade when they vanish
//!   without a cue ([`RowAnimator`]), the message line fades and rises. Reduced motion jumps every tween to its end state.
//! * The caret blinks, so a focused field keeps `step` returning true (frames at the display rate, never throttled).
use super::kit::{Clicks, Cues, Field, Icons};
use glasskit::canvas::Canvas;
use glasskit::ease::{curves, dur, Ease, Tween};
use glasskit::edit::EditEvent;
use glasskit::geom::{Pt, Rect};
use glasskit::input::{InputEvent, MouseButton, Response};
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::{brighten, Theme};
use glasskit::Color;
use pill_ui_core::panel::b::timer::{self, ReminderRow, TimerView};
use pill_ui_core::panel::b::{Anim, AnimCue};

/// Card width in the Note tab (`grid-template-columns: 1fr 232px`).
pub const CARD_W: f32 = 232.0;
const PAD: f32 = 12.0;
const RING: f32 = 68.0;
/// Height of a reminder row (6 + 16 + 16 + 6) and the gap between rows.
pub const ROW_H: f32 = 44.0;
pub const ROW_GAP: f32 = 2.0;
/// `.tn-start` label colour on the amber fill.
const ON_AMBER: Color = Color::rgb8(0x1b, 0x12, 0x00);
const LINEAR: Ease = Ease::bezier(0.0, 0.0, 1.0, 1.0);

// ---------------------------------------------------------------------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Font size of the ring clock: steps with the text length like `.tn-time[data-len]` (16 / 14 / 13 / 12), so the box never reflows.
pub fn clock_size(len: usize) -> f32 {
    match len {
        0..=4 => 16.0,
        5 => 14.0,
        6 => 13.0,
        _ => 12.0,
    }
}

/// The card's pieces for a card rectangle; `msg_h` is the height of the message text (0 = no message line).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub ring: Rect,
    pub start: Rect,
    pub ctime: Rect,
    pub presets: [Rect; 5],
    pub unit: Rect,
    pub hr_y: f32,
    pub head: Rect,
    pub list: Rect,
    pub msg: Rect,
    pub input: Rect,
}

pub fn layout(card: Rect, msg_h: f32) -> Layout {
    let inner = card.inset(PAD);
    let ring = Rect::new(inner.x, inner.y, RING, RING);
    let sx = ring.right() + PAD;
    let sw = (inner.right() - sx).max(0.0);
    let start = Rect::new(sx, inner.y, sw, 36.0);
    let ctime = Rect::new(sx, inner.y + 44.0, sw, 24.0);
    let py = inner.y + RING + PAD;
    let presets = std::array::from_fn(|i| Rect::new(inner.x + i as f32 * 36.0, py, 32.0, 24.0));
    let unit = Rect::new(inner.x + 5.0 * 36.0, py, (inner.w - 5.0 * 36.0).max(0.0), 24.0);
    let hr_y = py + 24.0 + PAD;
    let head = Rect::new(inner.x + 6.0, hr_y + 1.0 + 8.0, (inner.w - 6.0).max(0.0), 16.0);
    let input = Rect::new(inner.x, (inner.bottom() - 32.0).max(head.bottom()), inner.w, 32.0);
    let (msg, list_bottom) = if msg_h > 0.0 {
        let m = Rect::new(inner.x + 2.0, input.y - 8.0 - 6.0 - msg_h, (inner.w - 4.0).max(0.0), msg_h);
        (m, m.y - 4.0)
    } else {
        (Rect::new(inner.x, input.y - 8.0, inner.w, 0.0), input.y - 8.0)
    };
    let list = Rect::new(inner.x, head.bottom() + 4.0, inner.w, (list_bottom - head.bottom() - 4.0).max(0.0));
    Layout { ring, start, ctime, presets, unit, hr_y, head, list, msg, input }
}

/// Where one reminder row is painted (content coordinates of the list).
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub id: i64,
    pub y: f32,
    /// the visible height (`ROW_H` while normal, shrinking to 0 while it collapses)
    pub h: f32,
    pub alpha: f32,
    /// extra downward offset of an entering row (6 px at the start)
    pub rise: f32,
}

struct RowSt {
    id: i64,
    /// FLIP offset: where the row was minus where it is now, easing to 0
    off: Tween<f32>,
    enter: Tween<f32>,
    /// 1 = full height, 0 = collapsed
    col: Tween<f32>,
}

struct Ghost {
    y: f32,
    fade: Tween<f32>,
}

/// Row layout with FLIP motion: rows that stay slide to their new place, a new row rises in, a collapsing row shrinks, a row that vanished
/// without a collapse cue fades where it was. Pure (no drawing): tested below.
pub struct RowAnimator {
    rows: Vec<RowSt>,
    ghosts: Vec<Ghost>,
    known: bool,
    reduced: bool,
}

impl Default for RowAnimator {
    fn default() -> Self {
        RowAnimator { rows: Vec::new(), ghosts: Vec::new(), known: false, reduced: false }
    }
}

impl RowAnimator {
    pub fn new() -> RowAnimator {
        RowAnimator::default()
    }

    pub fn set_reduced(&mut self, on: bool) {
        self.reduced = on;
        if on {
            for r in &mut self.rows {
                r.off.finish();
                r.enter.finish();
            }
            self.ghosts.clear();
        }
    }

    pub fn ids(&self) -> Vec<i64> {
        self.rows.iter().map(|r| r.id).collect()
    }

    /// Row tops in the list's content coordinates, without the FLIP offsets.
    fn base_ys(&self) -> Vec<f32> {
        let mut y = 0.0;
        self.rows
            .iter()
            .map(|r| {
                let at = y;
                y += (ROW_H + ROW_GAP) * r.col.value().clamp(0.0, 1.0);
                at
            })
            .collect()
    }

    pub fn places(&self) -> Vec<Place> {
        let base = self.base_ys();
        self.rows
            .iter()
            .zip(base)
            .map(|(r, b)| {
                let col = r.col.value().clamp(0.0, 1.0);
                let e = r.enter.value().clamp(0.0, 1.0);
                Place { id: r.id, y: b + r.off.value(), h: ROW_H * col, alpha: col * e, rise: 6.0 * (1.0 - e) }
            })
            .collect()
    }

    /// (y, alpha) of the rows that are fading out.
    pub fn ghosts(&self) -> Vec<(f32, f32)> {
        self.ghosts.iter().map(|g| (g.y, g.fade.value().clamp(0.0, 1.0))).collect()
    }

    pub fn content_height(&self) -> f32 {
        let total: f32 = self.rows.iter().map(|r| (ROW_H + ROW_GAP) * r.col.value().clamp(0.0, 1.0)).sum();
        (total - ROW_GAP).max(0.0)
    }

    /// The list changed (call from the draw when `ids` differs from [`RowAnimator::ids`]).
    pub fn sync(&mut self, ids: &[i64]) {
        let old_pos: Vec<(i64, f32, f32)> = self.places().iter().map(|p| (p.id, p.y, p.alpha)).collect();
        let was_empty = self.rows.is_empty();
        let animate = self.known && !self.reduced;
        let mut old: Vec<RowSt> = std::mem::take(&mut self.rows);
        for id in ids {
            let st = match old.iter().position(|r| r.id == *id) {
                Some(i) => old.remove(i),
                None => {
                    let mut enter = Tween::at_rest(1.0);
                    // when the whole list appears the list-level cue animates it, not every row
                    if animate && !was_empty {
                        enter = Tween::new(0.0, 1.0, dur::MED, curves::EASE_OUT);
                    }
                    RowSt { id: *id, off: Tween::at_rest(0.0), enter, col: Tween::at_rest(1.0) }
                }
            };
            self.rows.push(st);
        }
        // rows that left: collapsed ones are gone already, the others fade where they were
        if animate {
            for r in old {
                if let Some((_, y, a)) = old_pos.iter().find(|(id, _, _)| *id == r.id) {
                    if *a > 0.3 {
                        self.ghosts.push(Ghost { y: *y, fade: Tween::new(*a, 0.0, dur::EXIT, curves::EASE_IN) });
                    }
                }
            }
        }
        if animate {
            let base = self.base_ys();
            for (r, b) in self.rows.iter_mut().zip(base) {
                if let Some((_, y, _)) = old_pos.iter().find(|(id, _, _)| *id == r.id) {
                    let delta = *y - b;
                    if delta.abs() > 0.5 {
                        r.off = Tween::new(delta, 0.0, dur::SETTLE, curves::SPRING_SETTLE);
                    }
                }
            }
        }
        self.known = true;
    }

    /// `rem:<id>` cue: the row shrinks away (the model removes it when the backend answers).
    pub fn collapse(&mut self, id: i64) {
        let reduced = self.reduced;
        if let Some(r) = self.rows.iter_mut().find(|r| r.id == id) {
            if reduced {
                r.col.jump(0.0);
            } else {
                r.col.retarget(0.0, dur::MED, curves::EASE_IN_OUT);
            }
        }
    }

    pub fn step(&mut self, dt: f32) -> bool {
        let mut busy = false;
        for r in &mut self.rows {
            busy |= r.off.step(dt);
            busy |= r.enter.step(dt);
            busy |= r.col.step(dt);
        }
        for g in &mut self.ghosts {
            busy |= g.fade.step(dt);
        }
        self.ghosts.retain(|g| !g.fade.finished());
        busy
    }
}

/// Eased state of the ring: the arc length (`left`, linear over .26 s) and the "running" amount (arc opacity, track colour; .2 s).
pub struct RingAnim {
    arc: Tween<f32>,
    run: Tween<f32>,
    reduced: bool,
}

impl RingAnim {
    pub fn new() -> RingAnim {
        RingAnim { arc: Tween::at_rest(1.0), run: Tween::at_rest(0.0), reduced: false }
    }

    /// Follow the model: `left` is the fraction still filled (only while running).
    pub fn set(&mut self, running: bool, left: f32) {
        let run_to = running as u8 as f32;
        let left = left.clamp(0.0, 1.0);
        if self.reduced {
            self.run.jump(run_to);
            self.arc.jump(left);
            return;
        }
        self.run.retarget(run_to, dur::MED, curves::EASE_OUT);
        if running && self.run.value() < 0.01 {
            // the arc fades in at its current length, it does not sweep from the previous run
            self.arc.jump(left);
        } else {
            self.arc.retarget(left, 0.26, LINEAR);
        }
    }

    pub fn step(&mut self, dt: f32) -> bool {
        let a = self.arc.step(dt);
        let b = self.run.step(dt);
        a | b
    }

    /// (arc fraction, running amount)
    pub fn value(&self) -> (f32, f32) {
        (self.arc.value().clamp(0.0, 1.0), self.run.value().clamp(0.0, 1.0))
    }
}

impl Default for RingAnim {
    fn default() -> Self {
        RingAnim::new()
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum K {
    Start,
    Custom,
    Preset(u32),
    Row(i64),
    Done(i64),
    Snooze(i64),
    Del(i64),
}

const ICONS: [(&str, &str); 7] = [
    ("tn_play", r#"<path d="M7 4.5v15l12-7.5z" fill="currentColor"/>"#),
    ("tn_stop", r#"<rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor"/>"#),
    ("tn_pencil", r#"<path d="M4 20l1-4L16 5l3 3L8 19z"/>"#),
    ("tn_check", r#"<path d="M5 12.5l4.5 4.5L19 7.5"/>"#),
    ("tn_clock", r#"<circle cx="12" cy="12" r="8"/><path d="M12 8v4l3 2"/>"#),
    ("tn_x", r#"<path d="M6 6l12 12M18 6 6 18"/>"#),
    ("tn_bell", r#"<path d="M6 16v-5a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>"#),
];

pub struct TimerCardView {
    rect: Rect,
    own: Icons,
    clicks: Clicks<K>,
    cues: Cues,
    sv: ScrollView,
    rows: RowAnimator,
    ring: RingAnim,
    /// custom-time field and reminder field
    cf: Field,
    rf: Field,
    out: Vec<timer::Action>,
    list_rect: Rect,
    // custom-time state
    custom_seen: bool,
    committed: bool,
    // reminder-field state
    last_sent: String,
    want_focus_r: bool,
    // message line
    msg: Option<(String, bool)>,
    msg_t: Tween<f32>,
    msg_closing: bool,
    reduced: bool,
    /// start button state swap (0 = Start, 1 = Stop), eased .2 s
    stop_t: Tween<f32>,
    /// last ring slot seen by `draw`
    empty_hint_len: usize,
}

impl TimerCardView {
    pub fn new(th: &Theme) -> Self {
        let mut own = Icons::new();
        for (n, m) in ICONS {
            own.add(n, m);
        }
        let mut cf = Field::new(false, th);
        cf.ti.style = th.small(600);
        let mut rf = Field::new(false, th);
        rf.ti.style = th.small(500);
        TimerCardView {
            rect: Rect::default(),
            own,
            clicks: Clicks::new(),
            cues: Cues::new(),
            sv: ScrollView::new(),
            rows: RowAnimator::new(),
            ring: RingAnim::new(),
            cf,
            rf,
            out: Vec::new(),
            list_rect: Rect::default(),
            custom_seen: false,
            committed: false,
            last_sent: String::new(),
            want_focus_r: false,
            msg: None,
            msg_t: Tween::at_rest(0.0),
            msg_closing: false,
            reduced: false,
            stop_t: Tween::at_rest(0.0),
            empty_hint_len: 0,
        }
    }

    pub fn set_reduced(&mut self, on: bool) {
        self.reduced = on;
        self.clicks.reduced = on;
        self.cues.reduced = on;
        self.rows.set_reduced(on);
        self.ring.reduced = on;
        self.cf.ti.reduced = Some(on);
        self.rf.ti.reduced = Some(on);
        self.sv.state.reduced = Some(on);
        if on {
            self.msg_t.finish();
            self.stop_t.finish();
        }
    }

    pub fn focus_reminder_input(&mut self) {
        self.want_focus_r = true;
    }

    pub fn wants(&self, ev: &InputEvent) -> bool {
        match ev {
            InputEvent::MouseDown { pos, .. } | InputEvent::MouseMove { pos, .. } | InputEvent::MouseUp { pos, .. } | InputEvent::Wheel { pos, .. } => {
                self.rect.contains(pos.0, pos.1) || self.cf.wants(ev) || self.rf.wants(ev)
            }
            InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_) => self.cf.wants(ev) || self.rf.wants(ev),
            _ => false,
        }
    }

    pub fn play(&mut self, cue: AnimCue) {
        if let Some(id) = cue.target.strip_prefix("rem:").and_then(|s| s.parse::<i64>().ok()) {
            if matches!(cue.kind, Anim::Collapse) {
                self.rows.collapse(id);
            }
            return;
        }
        match cue.target.as_str() {
            "timer-start" | "reminders-list" | "reminders-empty" => self.cues.play(cue),
            t if t.starts_with("preset-") => self.cues.play(cue),
            _ => {} // the message line and the row flip are driven from the view data itself
        }
    }

    fn custom_active(&self) -> bool {
        self.custom_seen && !self.committed
    }

    fn commit_custom(&mut self, apply: bool) {
        if !self.custom_active() {
            return;
        }
        self.committed = true;
        let text = self.cf.ti.text();
        self.cf.ti.set_focused(false);
        self.out.push(timer::Action::CustomCommit { text, apply });
    }

    /// The user's gestures since the last call.
    pub fn drain(&mut self) -> Vec<timer::Action> {
        for e in self.cf.take_events() {
            match e {
                EditEvent::Submit => self.commit_custom(true),
                EditEvent::Cancel => self.commit_custom(false),
                EditEvent::FocusChanged(false) => self.commit_custom(true),
                _ => {}
            }
        }
        for e in self.rf.take_events() {
            match e {
                EditEvent::Changed => {
                    let t = self.rf.ti.text();
                    self.last_sent = t.clone();
                    self.out.push(timer::Action::ReminderInput(t));
                }
                EditEvent::Submit => self.out.push(timer::Action::AddReminder),
                EditEvent::Cancel => self.rf.ti.set_focused(false),
                _ => {}
            }
        }
        std::mem::take(&mut self.out)
    }

    fn on_click(&mut self, k: K) {
        use timer::Action as A;
        match k {
            K::Start => self.out.push(A::StartStop),
            K::Custom => self.out.push(A::CustomOpen),
            K::Preset(m) => self.out.push(A::Preset(m)),
            K::Done(id) => self.out.push(A::ReminderDone(id)),
            K::Snooze(id) => self.out.push(A::ReminderSnooze(id)),
            K::Del(id) => self.out.push(A::ReminderDelete(id)),
            K::Row(_) => {}
        }
    }

    pub fn event(&mut self, ev: &InputEvent) -> Response {
        if let InputEvent::MouseDown { pos, button: MouseButton::Left, .. } = ev {
            if self.custom_active() && !self.cf.rect.contains(pos.0, pos.1) {
                self.commit_custom(true);
            }
            if self.rf.is_focused() && !self.rf.rect.contains(pos.0, pos.1) {
                self.rf.ti.set_focused(false);
            }
        }
        if self.custom_active() && self.cf.wants(ev) {
            return self.cf.queue(ev);
        }
        if self.rf.wants(ev) {
            return self.rf.queue(ev);
        }
        let in_card = ev.pos().map_or(true, |p: Pt| self.rect.contains(p.0, p.1));
        let mut r = Response::NONE;
        if in_card || matches!(ev, InputEvent::MouseLeave) || matches!(ev, InputEvent::MouseUp { .. }) {
            let (cr, click) = self.clicks.event(ev);
            r = cr;
            if let Some(k) = click {
                self.on_click(k);
            }
        }
        if !r.consumed && self.list_rect.h > 0.0 {
            r = r.merge(self.sv.event(ev, self.list_rect));
        }
        r
    }

    pub fn step(&mut self, dt_ms: f64) -> bool {
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.clicks.step(dt);
        busy |= self.cues.step(dt_ms as f32);
        busy |= self.rows.step(dt);
        busy |= self.ring.step(dt);
        busy |= self.sv.step(dt);
        busy |= self.msg_t.step(dt);
        busy |= self.stop_t.step(dt);
        if self.msg_closing && self.msg_t.finished() {
            self.msg = None;
            self.msg_closing = false;
        }
        busy |= self.cf.ti.step(dt);
        busy |= self.rf.ti.step(dt);
        // a focused field blinks its caret: keep frames coming at the display rate while the user can see it
        busy || ((self.rf.is_focused() || self.cf.is_focused()) && !self.reduced) || self.cf.has_input() || self.rf.has_input()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, v: &TimerView, _ic: &Icons, _now_ms: f64) {
        self.rect = rect;
        // ---- deferred input, then follow the model ----
        let req_c = self.cf.pump(cv, self.cf.rect);
        let req_r = self.rf.pump(cv, self.rf.rect);
        if req_c {
            self.rf.ti.set_focused(false);
            self.cf.ti.set_focused(true);
        } else if req_r {
            self.cf.ti.set_focused(false);
            self.rf.ti.set_focused(true);
        }
        if v.custom_editing && !self.custom_seen {
            self.committed = false;
            self.cf.ti.set_text("");
            self.rf.ti.set_focused(false);
            self.cf.ti.set_focused(true);
        }
        if !v.custom_editing {
            self.committed = false;
        }
        self.custom_seen = v.custom_editing;
        if self.want_focus_r {
            self.want_focus_r = false;
            self.cf.ti.set_focused(false);
            self.rf.ti.set_focused(true);
            let n = self.rf.ti.model.len();
            self.rf.ti.set_selection(n, n);
        }
        if v.input != self.rf.ti.text() && v.input != self.last_sent {
            self.rf.ti.set_text(&v.input);
            self.last_sent = v.input.clone();
        }
        let ids: Vec<i64> = v.reminders.iter().map(|r| r.id).collect();
        if ids != self.rows.ids() {
            self.rows.sync(&ids);
        }
        self.ring.set(v.running, v.left as f32);
        self.stop_t.retarget(v.running as u8 as f32, if self.reduced { 0.0 } else { dur::MED }, curves::EASE_OUT);
        if self.reduced {
            self.stop_t.finish();
        }
        self.sync_msg(v);
        self.empty_hint_len = v.empty_hint.len();

        // ---- layout ----
        let msg_h = match &self.msg {
            Some((t, _)) if !t.is_empty() => cv.measure(t, Style::new(11.0, 400).wrap(), (rect.w - 2.0 * PAD - 4.0).max(10.0)).1.max(16.0).ceil(),
            _ => 0.0,
        };
        let l = layout(rect, msg_h);
        self.list_rect = l.list;
        self.cf.rect = l.ctime;
        self.rf.rect = l.input;
        self.clicks.begin();

        cv.fill_rr(rect, 16.0, th.card);

        self.draw_top(cv, th, v, &l);
        self.draw_presets(cv, th, v, &l);
        super::kit::hline(cv, th, rect.x + PAD, rect.right() - PAD, l.hr_y);
        self.draw_head(cv, th, v, &l);
        self.draw_list(cv, th, v, &l);
        self.draw_msg(cv, th, &l);
        // the reminder field
        let mut t2 = *th;
        t2.field = th.raised;
        self.rf.ti.placeholder = v.input_placeholder.clone();
        self.rf.ti.draw(cv, l.input, &t2);
    }

    fn sync_msg(&mut self, v: &TimerView) {
        match &v.msg {
            Some(m) => {
                let fresh = self.msg.is_none() || self.msg_closing;
                let changed = self.msg.as_ref() != Some(m);
                if changed || self.msg_closing {
                    self.msg = Some(m.clone());
                    self.msg_closing = false;
                    if fresh {
                        if self.reduced {
                            self.msg_t.jump(1.0);
                        } else {
                            self.msg_t = Tween::new(0.0, 1.0, dur::MED, curves::EASE_OUT);
                        }
                    }
                }
            }
            None => {
                if self.msg.is_some() && !self.msg_closing {
                    self.msg_closing = true;
                    if self.reduced {
                        self.msg_t.jump(0.0);
                    } else {
                        self.msg_t = Tween::new(self.msg_t.value(), 0.0, 0.18, curves::EASE_IN);
                    }
                }
            }
        }
    }

    fn draw_top(&mut self, cv: &Canvas, th: &Theme, v: &TimerView, l: &Layout) {
        // ring: r 34 + stroke 5 in an 80 box drawn at 74 px
        let k = 74.0 / 80.0;
        let c = l.ring.center();
        let (arc, run) = self.ring.value();
        let amber = th.amber;
        cv.stroke_circle(c, 34.0 * k, 5.0 * k, amber.alpha(0.45 + (0.18 - 0.45) * run));
        if run > 0.01 && arc > 0.0005 {
            cv.arc(c, 34.0 * k, 5.0 * k, 0.0, 360.0 * arc, amber.fade(run));
        }
        let size = clock_size(v.clock_len);
        cv.text(l.ring, &v.clock, Style::new(size, 600).center().mono(), th.text);

        // start / stop
        self.clicks.add(K::Start, l.start);
        let hov = self.clicks.hover_amt(&K::Start);
        let pr = self.clicks.press_scale(&K::Start, 0.96) * self.cues.xf("timer-start").scale;
        let stop = self.stop_t.value().clamp(0.0, 1.0);
        let idle_bg = th.amber.mix(brighten(th.amber, 1.08), hov);
        let run_bg = th.raised2.mix(th.raised, 0.0);
        let bg = idle_bg.mix(run_bg, stop);
        let fg = ON_AMBER.mix(th.text, stop);
        cv.with_scale_about(l.start.center(), pr, |cv| {
            cv.fill_rr(l.start, 12.0, bg);
            let st = Style::new(13.0, 600);
            let tw = cv.text_width(&v.start_label, st);
            let x0 = l.start.cx() - (12.0 + 8.0 + tw) / 2.0;
            let icon = if v.running { "tn_stop" } else { "tn_play" };
            self.own.draw_sw(cv, icon, Rect::new(x0, l.start.cy() - 6.0, 12.0, 12.0), fg, 1.7);
            cv.text(Rect::new(x0 + 20.0, l.start.y, tw + 2.0, l.start.h), &v.start_label, st, fg);
        });

        // custom time: a button, or the field while editing
        if v.custom_editing {
            let mut t2 = *th;
            t2.field = th.raised;
            self.cf.ti.placeholder = v.custom_placeholder.clone();
            self.cf.ti.draw(cv, l.ctime, &t2);
        } else {
            self.clicks.add(K::Custom, l.ctime);
            let hov = self.clicks.hover_amt(&K::Custom);
            let pr = self.clicks.press_scale(&K::Custom, 0.96);
            let fg = th.text2.mix(th.text, hov);
            cv.with_scale_about(l.ctime.center(), pr, |cv| {
                cv.fill_rr(l.ctime, 12.0, th.raised2);
                let st = Style::new(11.0, 600);
                let tw = cv.text_width(&v.custom_label, st);
                let x0 = l.ctime.cx() - (12.0 + 6.0 + tw) / 2.0;
                self.own.draw(cv, "tn_pencil", Rect::new(x0, l.ctime.cy() - 6.0, 12.0, 12.0), fg);
                cv.text(Rect::new(x0 + 18.0, l.ctime.y, tw + 2.0, l.ctime.h), &v.custom_label, st, fg);
            });
        }
    }

    fn draw_presets(&mut self, cv: &Canvas, th: &Theme, v: &TimerView, l: &Layout) {
        for (i, (m, on)) in v.presets.iter().enumerate().take(5) {
            let r = l.presets[i];
            let k = K::Preset(*m);
            self.clicks.add(k, r);
            let hov = self.clicks.hover_amt(&k);
            let pr = self.clicks.press_scale(&k, 0.96) * self.cues.xf(&format!("preset-{m}")).scale;
            cv.with_scale_about(r.center(), pr, |cv| {
                cv.fill_rr(r, 12.0, if *on { th.raised2 } else { th.raised });
                let fg = if *on { th.text } else { th.text2.mix(th.text, hov) };
                cv.text(r, &m.to_string(), Style::new(11.0, 600).center().mono(), fg);
            });
        }
        cv.text(l.unit, &v.unit, Style::new(11.0, 400).right(), th.dim);
    }

    fn draw_head(&mut self, cv: &Canvas, th: &Theme, v: &TimerView, l: &Layout) {
        let h = l.head;
        self.own.draw(cv, "tn_bell", Rect::new(h.x, h.cy() - 7.0, 14.0, 14.0), th.dim);
        let st = Style::new(12.0, 600);
        let tw = cv.text_width(&v.reminders_head, st);
        cv.text(Rect::new(h.x + 22.0, h.y, tw + 2.0, h.h), &v.reminders_head, st, th.text2);
        if v.reminders_count > 0 {
            let s = v.reminders_count.to_string();
            let bs = Style::new(10.0, 400).mono();
            let bw = cv.text_width(&s, bs) + 12.0;
            let br = Rect::new(h.x + 22.0 + tw + 8.0, h.cy() - 8.0, bw, 16.0);
            cv.fill_rr(br, 8.0, th.raised2);
            cv.text(br, &s, bs.center(), th.text2);
        }
    }

    fn draw_list(&mut self, cv: &Canvas, th: &Theme, v: &TimerView, l: &Layout) {
        let view = l.list;
        if view.h <= 0.0 {
            return;
        }
        if v.reminders.is_empty() && self.rows.ghosts().is_empty() {
            // the four-line hint, centred in the list area
            let st = Style::new(11.0, 400).center();
            let lines = &v.empty_hint;
            let total = lines.len() as f32 * 16.0;
            let y0 = view.y + ((view.h - total) / 2.0).max(0.0);
            self.cues.apply(cv, "reminders-empty", view.center(), |cv| {
                cv.with_clip(view, |cv| {
                    for (i, s) in lines.iter().enumerate() {
                        cv.text(Rect::new(view.x, y0 + i as f32 * 16.0, view.w, 16.0), s, st, th.faint);
                    }
                });
            });
            self.sv.set_content_height(0.0, view.h);
            return;
        }
        let places = self.rows.places();
        let content_h = self.rows.content_height();
        self.sv.set_content_height(content_h, view.h);
        let off_y = view.y - self.sv.offset();
        // click targets in window coordinates, clipped to the list view
        for p in &places {
            if p.alpha < 0.5 || p.h < ROW_H * 0.5 {
                continue;
            }
            let rr = Rect::new(view.x, off_y + p.y, view.w, p.h);
            let vis = inter(rr, view);
            if vis.w <= 0.0 || vis.h <= 0.0 {
                continue;
            }
            self.clicks.add(K::Row(p.id), vis);
            let act = self.act_amt(p.id);
            if act > 0.05 {
                for (j, kind) in [K::Done(p.id), K::Snooze(p.id), K::Del(p.id)].into_iter().enumerate() {
                    let b = act_rect(rr, j);
                    let vb = inter(b, view);
                    if vb.w > 0.0 && vb.h > 0.0 {
                        self.clicks.add(kind, vb);
                    }
                }
            }
        }
        let rows: Vec<&ReminderRow> = v.reminders.iter().collect();
        let clicks = &self.clicks;
        let own = &self.own;
        let ghosts = self.rows.ghosts();
        let cues = &self.cues;
        self.sv.draw(cv, view, th, |cv, content| {
            cues.apply(cv, "reminders-list", content.center(), |cv| {
                for p in &places {
                    let Some(row) = rows.iter().find(|r| r.id == p.id) else { continue };
                    if p.h <= 0.5 || p.alpha <= 0.0 {
                        continue;
                    }
                    let full = Rect::new(0.0, p.y, content.w, ROW_H);
                    cv.with_clip(Rect::new(0.0, p.y, content.w, p.h.max(0.0)), |cv| {
                        cv.with_opacity(p.alpha, |cv| {
                            cv.with_offset(0.0, p.rise, |cv| paint_row(cv, th, own, clicks, row, full));
                        });
                    });
                }
                for (y, a) in &ghosts {
                    cv.with_opacity(*a, |cv| cv.fill_rr(Rect::new(0.0, *y, content.w, ROW_H), 10.0, th.hover.fade(0.5)));
                }
            });
        });
    }

    /// Reveal amount of a row's action buttons: the row or one of its buttons is hovered.
    fn act_amt(&self, id: i64) -> f32 {
        let c = &self.clicks;
        (c.hover_amt(&K::Row(id)) + c.hover_amt(&K::Done(id)) + c.hover_amt(&K::Snooze(id)) + c.hover_amt(&K::Del(id))).min(1.0)
    }

    fn draw_msg(&mut self, cv: &Canvas, th: &Theme, l: &Layout) {
        let Some((text, bad)) = &self.msg else { return };
        let e = self.msg_t.value().clamp(0.0, 1.0);
        if l.msg.h <= 0.0 {
            return;
        }
        cv.with_opacity(e, |cv| {
            cv.with_offset(0.0, 8.0 * (1.0 - e), |cv| {
                cv.text(l.msg, text, Style::new(11.0, 400).wrap(), if *bad { th.red } else { th.text2 });
            });
        });
    }
}

fn inter(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// The j-th (0 done, 1 snooze, 2 delete) 24 px action button of a row: right-aligned 2 px from the edge.
fn act_rect(row: Rect, j: usize) -> Rect {
    let x = row.right() - 2.0 - 72.0 + j as f32 * 24.0;
    Rect::new(x, row.cy() - 12.0, 24.0, 24.0)
}

fn paint_row(cv: &Canvas, th: &Theme, ic: &Icons, clicks: &Clicks<K>, row: &ReminderRow, r: Rect) {
    // the click rects are in window coordinates; the hover amount is all the row needs
    let hov = clicks.hover_amt(&K::Row(row.id));
    let act = (hov + clicks.hover_amt(&K::Done(row.id)) + clicks.hover_amt(&K::Snooze(row.id)) + clicks.hover_amt(&K::Del(row.id))).min(1.0);
    cv.fill_rr(r, 10.0, th.raised.fade(act));
    let col = if row.overdue { th.red } else { th.dim };
    ic.draw(cv, "tn_bell", Rect::new(r.x + 6.0, r.cy() - 7.0, 14.0, 14.0), col);
    let tx = r.x + 6.0 + 14.0 + 8.0;
    let tw = (r.right() - 4.0 - tx).max(0.0);
    cv.text(Rect::new(tx, r.y + 6.0, tw, 16.0), &row.text, Style::new(12.0, 600).ellipsis(), th.text);
    let ms = Style::new(11.0, 400).mono();
    let when_w = cv.text_width(&row.when, ms).min(tw);
    cv.text(Rect::new(tx, r.y + 22.0, when_w + 1.0, 16.0), &row.when, ms.ellipsis(), col);
    if !row.repeat.is_empty() {
        let bs = Style::new(10.0, 600);
        let bw = cv.text_width(&row.repeat, bs) + 12.0;
        let bx = tx + when_w + 6.0;
        let br = Rect::new(bx, r.y + 22.0, bw.min((r.right() - 4.0 - bx).max(0.0)), 16.0);
        if br.w > 8.0 {
            cv.fill_rr(br, 8.0, th.amber_bg);
            cv.text(br.inset_xy(6.0, 0.0), &row.repeat, bs.ellipsis(), th.amber);
        }
    }
    if act > 0.01 {
        cv.with_opacity(act, |cv| {
            // the strip fades in from the left over 14 px so long text does not collide with the buttons
            let strip = Rect::new(r.right() - 2.0 - 72.0 - 14.0, r.y, 14.0 + 72.0 + 2.0, r.h);
            for i in 0..7 {
                let a = (i as f32 + 1.0) / 7.0;
                cv.fill_rect(Rect::new(strip.x + i as f32 * 2.0, strip.y + 2.0, 2.0, strip.h - 4.0), th.raised.alpha(a));
            }
            cv.fill_rr(Rect::new(strip.x + 14.0, strip.y, strip.w - 14.0, strip.h), 10.0, th.raised);
            for (j, (k, name)) in [(K::Done(row.id), "tn_check"), (K::Snooze(row.id), "tn_clock"), (K::Del(row.id), "tn_x")].into_iter().enumerate() {
                let b = act_rect(r, j);
                let h = clicks.hover_amt(&k);
                cv.with_scale_about(b.center(), clicks.press_scale(&k, 0.9), |cv| {
                    cv.fill_rr(b, 8.0, th.raised2.fade(h));
                    ic.draw(cv, name, b.centered(14.0, 14.0), th.text2.mix(th.text, h));
                });
            }
        });
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(a: &mut RowAnimator) {
        for _ in 0..200 {
            if !a.step(1.0 / 60.0) {
                return;
            }
        }
        panic!("never settled");
    }

    #[test]
    fn clock_size_steps_with_the_length() {
        assert_eq!([clock_size(4), clock_size(5), clock_size(6), clock_size(7), clock_size(8)], [16.0, 14.0, 13.0, 12.0, 12.0]);
    }

    #[test]
    fn layout_fits_the_card_at_both_body_heights() {
        for h in [260.0, 346.0] {
            for mh in [0.0, 16.0, 48.0] {
                let l = layout(Rect::new(0.0, 0.0, CARD_W, h), mh);
                assert_eq!(l.ring.h, 68.0);
                assert_eq!(l.start.h + 8.0 + l.ctime.h, 68.0, "36 + 8 + 24 = the ring");
                assert!(l.input.bottom() <= h - PAD + 0.01 && l.input.h == 32.0);
                assert!(l.list.h >= 0.0 && l.list.y >= l.head.bottom());
                assert!(l.list.h == 0.0 || l.list.bottom() <= if mh > 0.0 { l.msg.y } else { l.input.y });
                assert!(l.presets[4].right() <= l.unit.x + 0.01 && l.unit.right() <= CARD_W - PAD + 0.01);
            }
        }
        // the 260 px body has room for about one reminder row
        let l = layout(Rect::new(0.0, 0.0, CARD_W, 260.0), 0.0);
        assert!(l.list.h >= 44.0, "{}", l.list.h);
    }

    #[test]
    fn first_sync_is_quiet_then_new_rows_rise_in() {
        let _g = crate::ui::host::testing::motion_lock(); // other tests flip the global reduced-motion override
        let mut a = RowAnimator::new();
        a.sync(&[1, 2]);
        assert!(a.places().iter().all(|p| p.alpha == 1.0 && p.rise == 0.0), "the first draw of a visit has no enter animation");
        assert!(!a.step(0.016));
        a.sync(&[1, 2, 3]);
        let p = a.places();
        assert_eq!(p[2].id, 3);
        assert!(p[2].alpha < 0.05 && p[2].rise > 5.5, "{:?}", p[2]);
        assert_eq!((p[0].y, p[1].y), (0.0, 46.0), "rows that stay do not move");
        assert!(a.step(0.1));
        let mid = a.places()[2].clone();
        assert!(mid.alpha > 0.1 && mid.alpha < 1.0);
        settle(&mut a);
        assert_eq!(a.places()[2].alpha, 1.0);
        assert_eq!(a.content_height(), 3.0 * 44.0 + 2.0 * 2.0);
    }

    #[test]
    fn whole_list_appearing_is_left_to_the_list_cue() {
        let _g = crate::ui::host::testing::motion_lock(); // other tests flip the global reduced-motion override
        let mut a = RowAnimator::new();
        a.sync(&[]);
        a.sync(&[7]);
        assert_eq!(a.places()[0].alpha, 1.0);
    }

    #[test]
    fn rows_that_stay_slide_to_their_new_place_and_vanished_ones_fade() {
        let _g = crate::ui::host::testing::motion_lock(); // other tests flip the global reduced-motion override
        let mut a = RowAnimator::new();
        a.sync(&[1, 2, 3]);
        a.sync(&[2, 3]); // 1 vanished without a collapse
        let p = a.places();
        assert_eq!(p[0].id, 2);
        assert!((p[0].y - 46.0).abs() < 0.01, "row 2 starts where it was: {}", p[0].y);
        assert_eq!(a.ghosts().len(), 1);
        assert!(a.step(0.1));
        assert!(a.places()[0].y < 46.0 && a.places()[0].y > 0.0);
        settle(&mut a);
        assert_eq!((a.places()[0].y, a.places()[1].y), (0.0, 46.0));
        assert!(a.ghosts().is_empty());
        // a reorder slides both
        a.sync(&[3, 2]);
        let p = a.places();
        assert!((p[0].y - 46.0).abs() < 0.01 && p[1].y.abs() < 0.01);
        settle(&mut a);
        assert_eq!((a.places()[0].y, a.places()[1].y), (0.0, 46.0));
    }

    #[test]
    fn collapse_shrinks_the_row_and_pulls_the_rest_up() {
        let _g = crate::ui::host::testing::motion_lock(); // other tests flip the global reduced-motion override
        let mut a = RowAnimator::new();
        a.sync(&[1, 2]);
        a.collapse(1);
        assert!(a.step(0.1));
        let p = a.places();
        assert!(p[0].h < 44.0 && p[1].y < 46.0);
        settle(&mut a);
        let p = a.places();
        assert_eq!((p[0].h, p[1].y), (0.0, 0.0));
        // the model then removes it: no ghost, nothing moves
        a.sync(&[2]);
        assert!(a.ghosts().is_empty());
        assert_eq!(a.places()[0].y, 0.0);
    }

    #[test]
    fn reduced_motion_jumps_everything() {
        let _g = crate::ui::host::testing::motion_lock(); // other tests flip the global reduced-motion override
        let mut a = RowAnimator::new();
        a.set_reduced(true);
        a.sync(&[1, 2]);
        a.sync(&[2, 1, 3]);
        assert!(a.places().iter().all(|p| p.alpha == 1.0 && p.rise == 0.0));
        assert!(!a.step(0.016) && a.ghosts().is_empty());
        assert_eq!(a.places().iter().map(|p| p.y).collect::<Vec<_>>(), vec![0.0, 46.0, 92.0]);
        a.collapse(2);
        assert_eq!(a.places()[0].h, 0.0);
    }

    #[test]
    fn ring_arc_is_linear_over_a_quarter_second_and_fades_in_without_a_sweep() {
        let _g = crate::ui::host::testing::motion_lock(); // other tests flip the global reduced-motion override
        let mut r = RingAnim::new();
        r.set(false, 1.0);
        assert_eq!(r.value(), (1.0, 0.0));
        r.set(true, 0.5);
        assert_eq!(r.value().0, 0.5, "starting: the arc is at the run's length at once, only its opacity fades");
        r.step(0.1);
        assert!(r.value().1 > 0.1 && r.value().1 < 1.0);
        for _ in 0..30 {
            r.step(1.0 / 60.0);
        }
        r.set(true, 0.4);
        r.step(0.13);
        let (a, _) = r.value();
        assert!(a < 0.5 && a > 0.4 && (a - 0.45).abs() < 0.02, "halfway after 130 ms: {a}");
        for _ in 0..30 {
            r.step(1.0 / 60.0);
        }
        assert!((r.value().0 - 0.4).abs() < 1e-4);
        let mut q = RingAnim::new();
        q.reduced = true;
        q.set(true, 0.3);
        assert_eq!(q.value(), (0.3, 1.0));
    }

    // ---- the card as a View: input -> actions, and the snapshots ----
    use crate::ui::host::testing::{motion_lock, render_view_cx};
    use crate::ui::tabs::b::kit::snap;
    use crate::ui::view::{Cx, View};
    use glasskit::input::{Buttons, KeyCode, Modifiers};

    struct Rig {
        card: TimerCardView,
        v: TimerView,
        ic: Icons,
    }

    impl View for Rig {
        fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, _cx: &Cx) {
            self.card.draw(cv, th, rect, &self.v, &self.ic, 0.0);
        }
    }

    fn row(id: i64, text: &str, when: &str, repeat: &str, overdue: bool) -> ReminderRow {
        ReminderRow { id, text: text.into(), when: when.into(), repeat: repeat.into(), overdue }
    }

    fn view(running: bool, rows: Vec<ReminderRow>) -> TimerView {
        TimerView {
            clock: if running { "12:30".into() } else { "5:00".into() },
            clock_len: 5,
            running,
            left: if running { 0.5 } else { 1.0 },
            start_label: if running { "Stop".into() } else { "Start".into() },
            presets: vec![(5, !running), (10, false), (15, false), (25, false), (45, false)],
            unit: "min".into(),
            custom_label: "Custom time".into(),
            custom_editing: false,
            custom_placeholder: "12, 1:30, 90s".into(),
            reminders_head: "Reminders".into(),
            reminders_count: rows.len(),
            empty_hint: vec!["Write below what and when:".into(), "\"call mum tomorrow at 9\",".into(), "\"pills every day at 23:00\",".into(), "\"turn off the stove in 20 minutes\".".into()],
            reminders: rows,
            msg: None,
            input: String::new(),
            input_placeholder: "Call mum tomorrow at 9".into(),
        }
    }

    fn rig(v: TimerView, th: &Theme) -> Rig {
        let mut card = TimerCardView::new(th);
        card.set_reduced(true);
        Rig { card, v, ic: Icons::new() }
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

    fn click(r: &mut Rig, p: Pt, cx: &Cx) {
        let th = Theme::pill_dark();
        for ev in [mv(p.0, p.1), down(p.0, p.1), up(p.0, p.1)] {
            r.card.event(&ev);
            render_view_cx(r, cx, (CARD_W, 346.0), 1.0, &th);
        }
    }

    #[test]
    fn clicks_become_actions() {
        let _g = motion_lock();
        let th = Theme::pill_dark();
        let cx = Cx::headless();
        let mut r = rig(view(false, vec![row(7, "Call", "09:00", "", false)]), &th);
        render_view_cx(&mut r, &cx, (CARD_W, 346.0), 1.0, &th);
        let l = layout(Rect::new(0.0, 0.0, CARD_W, 346.0), 0.0);
        click(&mut r, l.start.center(), &cx);
        click(&mut r, l.ctime.center(), &cx);
        click(&mut r, l.presets[2].center(), &cx);
        assert_eq!(r.card.drain(), vec![timer::Action::StartStop, timer::Action::CustomOpen, timer::Action::Preset(15)]);
        assert!(r.card.drain().is_empty(), "drain empties");
    }

    #[test]
    fn typing_in_the_reminder_field_reports_input_and_enter_adds() {
        let _g = motion_lock();
        let th = Theme::pill_dark();
        let cx = Cx::headless();
        let mut r = rig(view(false, vec![]), &th);
        render_view_cx(&mut r, &cx, (CARD_W, 346.0), 1.0, &th);
        r.card.focus_reminder_input();
        render_view_cx(&mut r, &cx, (CARD_W, 346.0), 1.0, &th);
        for ev in [InputEvent::Char('h'), InputEvent::Char('i')] {
            r.card.event(&ev);
        }
        render_view_cx(&mut r, &cx, (CARD_W, 346.0), 1.0, &th);
        let a = r.card.drain();
        assert_eq!(a.last(), Some(&timer::Action::ReminderInput("hi".into())), "{a:?}");
        r.card.event(&InputEvent::Key { code: KeyCode::Enter, mods: Modifiers::NONE, down: true, repeat: false });
        render_view_cx(&mut r, &cx, (CARD_W, 346.0), 1.0, &th);
        assert_eq!(r.card.drain(), vec![timer::Action::AddReminder]);
    }

    #[test]
    fn snapshots() {
        let _g = motion_lock();
        let mut cx = Cx::headless();
        for dark in [true, false] {
            let th = snap::theme(dark);
            for scale in [1.0, 2.0] {
                cx.scale = scale;
                let two = vec![row(1, "\u{41f}\u{43e}\u{437}\u{432}\u{43e}\u{43d}\u{438}\u{442}\u{44c} \u{43c}\u{430}\u{43c}\u{435}", "tomorrow 9:00 AM", "daily", false), row(2, "Turn off the stove", "in 18 min", "", false)];
                for (name, v) in [("timer-idle", view(false, vec![])), ("timer-running-ring", view(true, vec![])), ("reminders-2", view(false, two.clone())), ("reminders-empty", view(false, vec![]))] {
                    let mut r = rig(v, &th);
                    let img = snap::shoot(&mut r, &cx, name, (CARD_W, 346.0), scale, dark);
                    assert!(img.count(None, |p| p[3] > 0) > 1000, "{name} drew something");
                }
                let mut v = view(false, vec![]);
                v.custom_editing = true;
                let mut r = rig(v, &th);
                snap::shoot(&mut r, &cx, "custom-editing", (CARD_W, 346.0), scale, dark);
            }
        }
    }
}
