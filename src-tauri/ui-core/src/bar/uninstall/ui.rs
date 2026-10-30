//! The uninstall panel controller (port of the behaviour of src/uninstall-ui.js): confirm card -> the app's own uninstaller runs -> deep leftover scan
//! -> checklist in two groups (Certain pre-ticked, Probable not) -> delete selected (Recycle Bin, .reg backups) -> optional elevated step -> summary.
//!
//! The job lives in the backend; this is a view of it. `state.rs` decides what is on screen (takeover) and what the status strip shows; this file
//! turns backend events and user presses into `Out` requests (backend calls, focus moves, animations) and into the `UninstallView` the renderer
//! draws. Typing never blocks: any non-panel keystroke or input steps the panel aside, the job goes on, the strip takes over. After a restart the
//! backend's `uninstall_state` is read on `bar:shown` and the panel / strip is rebuilt from it.
//!
//! Time is passed in (`now_ms`, any monotonic clock): step changes are two-phase like the JS (the old step leaves for 140 ms, the new one settles
//! in), and `tick(now)` completes them and advances the number glides. Under reduced motion every phase applies at once and no animation is emitted.

use super::checklist::{Checklist, Snapshot};
use super::payload::{self, Info, ItemDone, Review, Scan};
use super::state::{self as st, Ev, Phase, State, StripKind};
use super::strings as s;
use super::view::*;
use crate::bar::anim::{self, Anims, Glide};
use crate::bar::keys::{is_ctrl_u, is_typing, KeyEvent};
use crate::bar::lang::{Lang, Tr};
use crate::bar::protocol::{BackendCall, Res};
use serde_json::{json, Value};

/// Step page shown in the box.
enum Page {
    Confirm { info: Info, silent: bool },
    Running { name: String },
    Scanning(ScanPage),
    /// the plain "Deleting the selection…" page (no checklist behind it)
    Deleting { name: String },
    Elevating { name: String, mode: String },
    Still { name: String },
    Error { name: String, message: String },
    /// the checklist lives in `UninstallUi::checklist`; `summary` = a finished job's head above it
    Checklist { summary: bool },
    Done { name: String, summary: payload::Summary },
}

struct ScanPage {
    name: String,
    found: Glide,
    stage: String,
    next_stage: Option<(String, f64)>,
}

enum AwayThen {
    None,
    Focus,
    FocusAndSearch,
}

enum Pending {
    /// the old step is leaving; at `due` the new one is applied
    Step { page: Page, due: f64 },
    /// the box collapses until `due`, then the list is back
    Away { then: AwayThen, due: f64 },
}

struct StripModel {
    shown: bool,
    phase_now: Option<Phase>,
    kind: Option<StripKind>,
    name: String,
    message: String,
    icon: StripIcon,
    count: Glide,
    has_count: bool,
}

/// Timing constants of the step transitions (ms), from the JS `M.exit` / `M.collapse` durations.
pub const LIST_EXIT_MS: f64 = 120.0;
pub const STEP_EXIT_MS: f64 = 140.0;
pub const AWAY_MS: f64 = 260.0;
pub const STAGE_EXIT_MS: f64 = 100.0;

pub struct UninstallUi {
    lang: Lang,
    anims: Anims,
    outs: Vec<Out>,
    st: State,
    /// generation of the last accepted backend state (ticks of a checklist are restored only for the same one)
    seq: u64,
    snap: Option<Snapshot>,
    checklist: Option<Checklist>,
    /// the backend phase word of the last `draw` (`phase` in the JS); empty when the panel left
    drawn: String,
    box_visible: bool,
    list_hidden: bool,
    shown: Option<Page>,
    pending: Option<Pending>,
    /// the renderer tells whether the normal result list has rows (the list steps aside with an animation only then)
    pub list_has_rows: bool,
    strip: StripModel,
    now: f64,
}

fn parse_int(s: &str) -> Option<usize> {
    let t: String = s.trim_start().chars().take_while(char::is_ascii_digit).collect();
    t.parse().ok()
}

impl UninstallUi {
    pub fn new(lang: Lang, reduce_motion: bool) -> UninstallUi {
        UninstallUi {
            lang,
            anims: Anims::new(reduce_motion),
            outs: Vec::new(),
            st: st::init(),
            seq: 0,
            snap: None,
            checklist: None,
            drawn: String::new(),
            box_visible: false,
            list_hidden: false,
            shown: None,
            pending: None,
            list_has_rows: false,
            strip: StripModel { shown: false, phase_now: None, kind: None, name: String::new(), message: String::new(), icon: StripIcon::Check, count: Glide::new(0.0), has_count: false },
            now: 0.0,
        }
    }

    pub fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
        // the strip's message is worded when the phase changes: re-word it for the new language
        if let Some(p) = self.strip.phase_now {
            self.strip.message = self.strip_words(p).to_string();
        }
    }
    pub fn set_reduce_motion(&mut self, reduce: bool) {
        self.anims.set_reduce(reduce);
        if let Some(c) = self.checklist.as_mut() {
            c.reduce = reduce;
        }
    }
    fn reduce(&self) -> bool {
        self.anims.reduced()
    }
    fn t(&self, x: s::S) -> &'static str {
        x.tr(self.lang)
    }

    pub fn state(&self) -> &State {
        &self.st
    }
    pub fn takeover(&self) -> bool {
        self.st.takeover
    }
    pub fn panel_visible(&self) -> bool {
        self.box_visible
    }
    /// The backend job exists (the bar window must not be torn down, `job_active` in Rust).
    pub fn has_job(&self) -> bool {
        self.st.job.is_some()
    }

    // ---- output plumbing ----

    fn call(&mut self, call: BackendCall) {
        self.outs.push(Out::Call { call, reply: None });
    }
    fn flush(&mut self) -> Vec<Out> {
        for a in self.anims.drain() {
            self.outs.push(Out::Anim(a));
        }
        std::mem::take(&mut self.outs)
    }

    // ---- steps: the old one leaves, the new one settles in ----

    fn go(&mut self, page: Page) {
        let now = self.now;
        if !self.box_visible {
            if !self.reduce() && self.list_has_rows && !self.list_hidden {
                // the result list steps aside first
                self.anims.push(anim::list_step_aside());
                self.pending = Some(Pending::Step { page, due: now + LIST_EXIT_MS });
            } else {
                self.list_hidden = true;
                self.apply(page);
            }
        } else if self.shown.is_some() && !self.reduce() {
            self.anims.push(anim::step_exit());
            self.pending = Some(Pending::Step { page, due: now + STEP_EXIT_MS });
        } else {
            self.apply(page);
        }
    }

    fn apply(&mut self, page: Page) {
        self.pending = None;
        self.shown = Some(page);
        if !self.box_visible {
            self.box_visible = true;
            self.list_hidden = true;
            self.anims.push(anim::panel_box_enter());
        }
        self.anims.push(anim::step_enter());
        if let Some(v) = self.panel_view() {
            for i in 0..v.parts.len() {
                self.anims.push(anim::part_enter(i));
            }
        }
        // a button gets the keyboard only when the step waits for a decision; a running step leaves the search box alone
        if matches!(self.drawn.as_str(), "confirm" | "checklist" | "done" | "still" | "error") {
            self.outs.push(Out::FocusDefault);
        } else {
            self.outs.push(Out::FocusInput);
        }
    }

    /// The panel leaves the screen; `then` runs when the list is back.
    fn away(&mut self, then: AwayThen) {
        self.pending = None;
        if !self.box_visible || self.reduce() {
            self.finish_away(then);
        } else {
            self.anims.push(anim::panel_collapse());
            self.pending = Some(Pending::Away { then, due: self.now + AWAY_MS });
        }
    }

    fn finish_away(&mut self, then: AwayThen) {
        self.pending = None;
        self.box_visible = false;
        self.shown = None;
        self.list_hidden = false;
        self.anims.push(anim::list_back());
        match then {
            AwayThen::None => {}
            AwayThen::Focus => self.outs.push(Out::FocusInput),
            AwayThen::FocusAndSearch => {
                self.outs.push(Out::FocusInput);
                self.outs.push(Out::SearchAgain);
            }
        }
        self.update_strip();
    }

    /// The explicit Close / Cancel: the backend job is dropped too (a running uninstaller is only no longer waited for).
    fn close(&mut self) {
        self.drawn.clear();
        self.checklist = None;
        self.snap = None;
        self.st = st::reduce(&self.st, &Ev::Close);
        self.call(BackendCall::bare("uninstall_cancel"));
        self.away(AwayThen::FocusAndSearch);
    }

    /// Typing / Esc / the Hide button: the panel steps aside and the job goes on in the backend; the strip takes over.
    fn leave(&mut self, ev: Ev) {
        if !self.st.takeover {
            return;
        }
        if let (Some(cl), Some(_)) = (self.checklist.as_ref(), self.st.last.as_ref()) {
            self.snap = Some(cl.snapshot(self.seq));
        }
        let was = self.st.job.is_some();
        let typing = ev == Ev::Typing;
        self.st = st::reduce(&self.st, &ev);
        if self.st.job.is_none() && was {
            self.call(BackendCall::bare("uninstall_cancel")); // a dismissed summary / error: nothing is left to come back to
        }
        self.drawn.clear();
        self.away(if typing { AwayThen::None } else { AwayThen::Focus });
    }

    /// The panel comes back where it was: redrawn from the last backend state, the ticks restored.
    fn focus_panel(&mut self) {
        if self.st.job.is_none() {
            return;
        }
        self.st = st::reduce(&self.st, &Ev::Focus);
        if let Some(last) = self.st.last.clone() {
            self.draw(&last);
        }
        self.update_strip();
    }

    // ---- the status strip under the results ----

    fn strip_words(&self, phase: Phase) -> &'static str {
        let k = match phase {
            Phase::Running => s::S_RUNNING,
            Phase::Scanning => s::S_SCANNING,
            Phase::Deleting => s::S_DELETING,
            Phase::Admin => s::S_ADMIN,
            Phase::Checklist => s::S_REVIEW,
            Phase::Done => s::S_DONE,
            Phase::Still => s::S_STILL,
            _ => s::S_ERROR,
        };
        self.t(k)
    }

    fn update_strip(&mut self) {
        let reduce = self.reduce();
        let now = self.now;
        let Some(d) = st::strip(&self.st) else {
            if self.strip.shown {
                self.strip.shown = false;
                if !reduce {
                    self.anims.push(anim::strip_exit());
                }
            }
            return;
        };
        let last = self.st.last.clone().unwrap_or(Value::Null);
        let n: Option<u64> = match d.phase {
            Phase::Scanning => Some(last.get("found").and_then(Value::as_u64).unwrap_or(0)),
            Phase::Checklist => Some(last.get("items").and_then(Value::as_array).map_or(0, |a| a.len() as u64)),
            _ => None,
        };
        self.strip.name = d.name.clone();
        if self.strip.phase_now != Some(d.phase) {
            let was = self.strip.phase_now.is_some();
            self.strip.phase_now = Some(d.phase);
            self.strip.message = self.strip_words(d.phase).to_string();
            self.strip.icon = if d.kind == StripKind::Busy {
                StripIcon::Spinner
            } else if matches!(d.phase, Phase::Error | Phase::Still) {
                StripIcon::Warn
            } else {
                StripIcon::Check
            };
            self.strip.kind = Some(d.kind);
            if was && !reduce {
                // the strip changes meaning: the label slides, a finished step pops
                self.anims.push(anim::strip_label_slide());
                if d.kind == StripKind::Ready {
                    self.anims.push(anim::strip_pop());
                }
            }
        }
        match n {
            None => self.strip.has_count = false,
            Some(n) => {
                if !self.strip.has_count {
                    self.strip.count = Glide::new(0.0);
                }
                self.strip.has_count = true;
                self.strip.count.set(n as f64, now, reduce);
            }
        }
        if !self.strip.shown {
            self.strip.shown = true;
            if !reduce {
                self.anims.push(anim::strip_enter());
            }
        }
    }

    // ---- entry points ----

    /// The user picked an `uninstall` row (Enter, click, the row's trash button via `u <name>`).
    pub fn open(&mut self, row: &Res, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        if self.st.job.is_some() {
            // `u <name>` while a job exists (this app or another one): the same panel, not a second card
            self.focus_panel();
            return self.flush();
        }
        let id = parse_int(&row.payload).unwrap_or(0);
        self.outs.push(Out::Call { call: BackendCall::new("uninstall_info", json!({ "id": id, "name": row.title })), reply: Some(ReplyTag::Info) });
        self.flush()
    }

    /// The answer to a call that carried a `ReplyTag`.
    pub fn on_reply(&mut self, tag: ReplyTag, value: Option<&Value>, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        match tag {
            ReplyTag::Info => {
                let info = value.map(Info::from_value);
                match info {
                    Some(i) if i.ok => {
                        self.st = st::reduce(&self.st, &Ev::Open { name: i.name.clone() });
                        self.drawn = "confirm".into();
                        self.checklist = None;
                        self.snap = None;
                        self.go(Page::Confirm { info: i, silent: false });
                    }
                    _ => self.outs.push(Out::SearchAgain),
                }
            }
            ReplyTag::State => {
                if let Some(v) = value {
                    let word = v.get("phase").and_then(Value::as_str).unwrap_or("");
                    let focus = v.get("focus").and_then(Value::as_bool) == Some(true);
                    if !word.is_empty() && (focus || word != self.drawn || self.st.job.is_none()) {
                        self.ingest(v);
                    }
                }
                self.update_strip();
            }
        }
        self.flush()
    }

    /// `bar:shown`: whatever happened meanwhile is read from the backend (`uninstall_state`).
    pub fn on_bar_shown(&mut self, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        self.outs.push(Out::Call { call: BackendCall::bare("uninstall_state"), reply: Some(ReplyTag::State) });
        self.flush()
    }

    /// `uninstall:state`.
    pub fn on_state(&mut self, payload: &Value, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        self.ingest(payload);
        self.flush()
    }

    /// `uninstall:focus` (the island card's button while the bar is open) or a click on the strip / Ctrl+U.
    pub fn on_focus(&mut self, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        self.focus_panel();
        self.flush()
    }

    /// `uninstall:item`: one deletion result.
    pub fn on_item(&mut self, payload: &Value, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        if payload.is_null() || !self.st.takeover {
            return self.flush();
        }
        let d = ItemDone::from_value(payload);
        let mut anims = std::mem::take(&mut self.anims);
        if let Some(c) = self.checklist.as_mut() {
            c.item_done(&d, now_ms, &mut anims);
        }
        self.anims = anims;
        self.flush()
    }

    /// A backend state arrives: the machine moves, the panel is redrawn only when it is on screen, the strip follows otherwise.
    fn ingest(&mut self, p: &Value) {
        if p.get("phase").and_then(Value::as_str).is_none() {
            return;
        }
        if Phase::from_server(p.get("phase").and_then(Value::as_str).unwrap_or("")).is_some() {
            self.seq += 1;
        }
        let focus = p.get("focus").and_then(Value::as_bool) == Some(true);
        self.st = st::reduce(&self.st, &Ev::Server(p.clone()));
        if focus {
            self.focus_panel();
            return;
        }
        if self.st.takeover {
            self.draw(p);
        } else {
            self.update_strip();
        }
    }

    fn name_of(&self, p: &Value) -> String {
        p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| self.st.name.clone())
    }

    fn draw(&mut self, p: &Value) {
        let word = p.get("phase").and_then(Value::as_str).unwrap_or("").to_string();
        let prev = std::mem::replace(&mut self.drawn, word.clone());
        let name = self.name_of(p);
        let now = self.now;
        let reduce = self.reduce();
        match word.as_str() {
            "running" => self.go(Page::Running { name }),
            "scanning" => {
                let sc = Scan::from_value(p);
                let stage = s::stage(&sc.step, self.lang);
                if prev == "scanning" {
                    if let Some(Page::Scanning(sp)) = self.shown.as_mut() {
                        sp.found.set(sc.found as f64, now, reduce);
                        if sp.stage != stage {
                            if reduce {
                                sp.stage = stage;
                                sp.next_stage = None;
                            } else {
                                self.anims.push(anim::stage_exit());
                                sp.next_stage = Some((stage, now + STAGE_EXIT_MS));
                            }
                        }
                        return;
                    }
                }
                let mut found = Glide::new(0.0);
                found.set(sc.found as f64, now, reduce);
                self.go(Page::Scanning(ScanPage { name, found, stage, next_stage: None }));
            }
            "deleting" => {
                if matches!(self.shown, Some(Page::Checklist { .. })) && self.checklist.is_some() {
                    let mut anims = std::mem::take(&mut self.anims);
                    if let Some(c) = self.checklist.as_mut() {
                        c.begin_busy(&mut anims);
                    }
                    self.anims = anims;
                } else {
                    self.go(Page::Deleting { name });
                }
            }
            "elevating" => self.go(Page::Elevating { name, mode: payload::elevating_mode(p) }),
            "still" => self.go(Page::Still { name }),
            "review" => {
                let page = self.build_checklist(Review::from_value(p), false);
                self.go(page);
            }
            "error" => self.go(Page::Error { name, message: p.get("message").and_then(Value::as_str).unwrap_or("").to_string() }),
            "done" => {
                let r = Review::from_value(p);
                if !r.items.is_empty() {
                    let page = self.build_checklist(r, true);
                    self.go(page);
                } else {
                    self.checklist = None;
                    self.go(Page::Done { name, summary: r.summary });
                }
            }
            _ => {}
        }
    }

    fn build_checklist(&mut self, review: Review, summary: bool) -> Page {
        let carry = self.checklist.as_ref().is_some_and(|c| c.admin_scanned);
        let cl = Checklist::build(review, carry, self.snap.as_ref(), self.seq, self.now, self.reduce());
        self.checklist = Some(cl);
        Page::Checklist { summary }
    }

    // ---- user input ----

    /// A button / tick / header of the panel was pressed.
    pub fn press(&mut self, c: Control, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        let busy = self.checklist.as_ref().is_some_and(|c| c.busy) && matches!(self.shown, Some(Page::Checklist { .. }));
        match c {
            Control::Start => {
                if let Some(Page::Confirm { silent, .. }) = &self.shown {
                    let silent = *silent;
                    self.call(BackendCall::new("uninstall_start", json!({ "silent": silent })));
                }
            }
            Control::ToggleSilent => {
                if let Some(Page::Confirm { silent, .. }) = self.shown.as_mut() {
                    *silent = !*silent;
                }
            }
            Control::Cancel | Control::Close => {
                if !busy {
                    self.close();
                }
            }
            Control::CheckAgain => self.call(BackendCall::bare("uninstall_scan")),
            Control::StopWait | Control::StopScan => self.call(BackendCall::bare("uninstall_cancel")),
            Control::Hide => self.leave(Ev::Esc),
            Control::SelectAllCertain => {
                if !busy {
                    if let Some(cl) = self.checklist.as_mut() {
                        cl.select_all_certain(now_ms);
                    }
                }
            }
            Control::DeleteSelected => {
                let mut anims = std::mem::take(&mut self.anims);
                let ids = self.checklist.as_ref().map(|c| c.user_ids()).unwrap_or_default();
                if !ids.is_empty() && !busy {
                    if let Some(cl) = self.checklist.as_mut() {
                        cl.begin_busy(&mut anims);
                    }
                    self.outs.push(Out::Call { call: BackendCall::new("uninstall_delete", json!({ "ids": ids })), reply: None });
                }
                self.anims = anims;
            }
            Control::AdminScan => {
                if !busy {
                    self.call(BackendCall::new("uninstall_admin", json!({ "mode": "scan", "ids": [] })));
                }
            }
            Control::AdminDelete => {
                let ids = self.checklist.as_ref().map(|c| c.admin_ids()).unwrap_or_default();
                if !ids.is_empty() && !busy {
                    self.call(BackendCall::new("uninstall_admin", json!({ "mode": "delete", "ids": ids })));
                }
            }
            Control::OpenBackups => self.call(BackendCall::bare("uninstall_open_backups")),
            Control::ToggleGroup(g) => {
                let mut anims = std::mem::take(&mut self.anims);
                if let Some(cl) = self.checklist.as_mut() {
                    cl.toggle_group(g, &mut anims);
                }
                self.anims = anims;
            }
            Control::ToggleItem(i) => {
                if let Some(cl) = self.checklist.as_mut() {
                    cl.toggle(i, now_ms);
                }
            }
        }
        self.flush()
    }

    /// The search box's `input` event, seen first (capture phase): the panel steps aside, the bar then runs its normal query.
    pub fn on_input(&mut self, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        if self.st.takeover {
            self.leave(Ev::Typing);
        }
        self.flush()
    }

    /// The window-level `keydown` in the capture phase. Returns whether the bar must NOT see the key (`stopPropagation`).
    pub fn on_key(&mut self, e: &KeyEvent, now_ms: f64) -> (bool, Vec<Out>) {
        self.now = now_ms;
        // Ctrl+U: the panel of the running job comes back (nothing else in the bar uses it)
        if is_ctrl_u(e) {
            self.focus_panel();
            return (true, self.flush());
        }
        if !self.box_visible {
            return (false, self.flush());
        }
        if e.key == "Escape" {
            self.leave(Ev::Esc); // only the view: the backend job goes on (Cancel / Close drop it)
            return (true, self.flush());
        }
        // a letter typed while a panel button has the keyboard is typing for the search box: leave the panel, put the caret there, the key lands in it
        if !e.target_is_search && is_typing(Some(e)) {
            self.leave(Ev::Typing);
            self.outs.push(Out::FocusInput);
            return (false, self.flush());
        }
        let own = matches!(e.key.as_str(), "Enter" | "ArrowUp" | "ArrowDown" | "Tab" | "F4") || (e.ctrl && e.shift);
        (own, self.flush())
    }

    /// Advance transitions and glides.
    pub fn tick(&mut self, now_ms: f64) -> Vec<Out> {
        self.now = now_ms;
        match self.pending.take() {
            Some(Pending::Step { page, due }) => {
                if now_ms >= due {
                    self.apply(page);
                } else {
                    self.pending = Some(Pending::Step { page, due });
                }
            }
            Some(Pending::Away { then, due }) => {
                if now_ms >= due {
                    self.finish_away(then);
                } else {
                    self.pending = Some(Pending::Away { then, due });
                }
            }
            None => {}
        }
        if let Some(Page::Scanning(sp)) = self.shown.as_mut() {
            sp.found.value(now_ms);
            if let Some((next, due)) = sp.next_stage.take() {
                if now_ms >= due {
                    sp.stage = next;
                    self.anims.push(anim::stage_enter());
                } else {
                    sp.next_stage = Some((next, due));
                }
            }
        }
        if let Some(c) = self.checklist.as_mut() {
            c.tick(now_ms);
        }
        self.strip.count.value(now_ms);
        self.flush()
    }

    /// True while something still moves (a transition is pending or a number glides): keep ticking at the full frame rate.
    pub fn animating(&self) -> bool {
        self.pending.is_some()
            || self.strip.count.running()
            || matches!(&self.shown, Some(Page::Scanning(sp)) if sp.found.running() || sp.next_stage.is_some())
            || self.checklist.as_ref().is_some_and(|c| {
                c.sel_count.running() || c.sel_size.running() || c.groups.iter().any(|g| g.count.running() || g.size.running()) || c.progress.as_ref().is_some_and(|p| p.n.running())
            })
    }

    // ---- view ----

    pub fn view(&self) -> UninstallView {
        UninstallView { takeover: self.st.takeover, panel: if self.box_visible { self.panel_view() } else { None }, list_hidden: self.list_hidden, strip: self.strip_view(), animating: self.animating() }
    }

    fn strip_view(&self) -> Option<StripView> {
        if !self.strip.shown {
            return None;
        }
        let (kind, phase) = (self.strip.kind?, self.strip.phase_now?);
        Some(StripView {
            name: self.strip.name.clone(),
            message: self.strip.message.clone(),
            count: self.strip.has_count.then(|| self.strip.count.shown().round().max(0.0) as u64),
            icon: self.strip.icon,
            kind,
            phase,
            tooltip: self.t(s::STRIP_TIP).to_string(),
            shortcut: "Ctrl+U",
            bar: kind == StripKind::Busy,
        })
    }

    fn panel_view(&self) -> Option<PanelView> {
        let page = self.shown.as_ref()?;
        let t = |x: s::S| self.t(x).to_string();
        let mut parts: Vec<Part> = Vec::new();
        let mut busy = false;
        let phase = match page {
            Page::Confirm { info, silent } => {
                let meta = [info.publisher.clone(), info.version.clone(), if info.size_kb > 0 { s::size(info.size_kb * 1024) } else { String::new() }].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" \u{b7} ");
                parts.push(Part::Heading(info.name.clone()));
                parts.push(Part::Sub(if info.store { t(s::STORE) } else { meta }));
                if !info.location.is_empty() {
                    parts.push(Part::Sub(format!("{}{}", self.t(s::FOLDER), info.location)));
                }
                if info.risky {
                    parts.push(Part::Warn { text: t(s::RISKY), shake: false });
                }
                if info.can_silent {
                    parts.push(Part::Check { control: Control::ToggleSilent, label: format!(" {}", self.t(s::SILENT)), checked: *silent });
                }
                parts.push(Part::Note(t(s::HOW)));
                parts.push(Part::Buttons(vec![Button::new(Control::Start, self.t(s::UNINSTALL)).danger(), Button::new(Control::Cancel, self.t(s::CANCEL)).dflt()]));
                "confirm"
            }
            Page::Running { name } => {
                parts.extend([Part::Heading(name.clone()), Part::Progress(ProgressKind::Indeterminate), Part::Busy(t(s::RUNNING)), Part::Note(t(s::WAITING))]);
                parts.push(Part::Buttons(vec![
                    Button::new(Control::CheckAgain, self.t(s::CHECK_AGAIN)),
                    Button::new(Control::StopWait, self.t(s::STOP_WAIT)),
                    Button::new(Control::Hide, self.t(s::HIDE)).dflt(),
                ]));
                "running"
            }
            Page::Scanning(sp) => {
                parts.extend([Part::Heading(sp.name.clone()), Part::Progress(ProgressKind::Indeterminate), Part::Busy(t(s::SCANNING))]);
                parts.push(Part::ScanLine { label: t(s::FOUND), found: sp.found.shown().round().max(0.0) as u64, stage: sp.stage.clone() });
                parts.push(Part::Buttons(vec![Button::new(Control::StopScan, self.t(s::STOP_SCAN)), Button::new(Control::Hide, self.t(s::HIDE)).dflt()]));
                "scanning"
            }
            Page::Deleting { name } => {
                parts.extend([Part::Heading(name.clone()), Part::Progress(ProgressKind::Indeterminate), Part::Busy(t(s::DELETING))]);
                "deleting"
            }
            Page::Elevating { name, mode } => {
                parts.extend([Part::Shield, Part::Heading(name.clone()), Part::Progress(ProgressKind::Elevating), Part::Busy(t(if mode == "delete" { s::ELEV_DEL } else { s::ELEV_SCAN }))]);
                "elevating"
            }
            Page::Still { name } => {
                parts.extend([Part::Heading(name.clone()), Part::Warn { text: t(s::STILL), shake: false }]);
                parts.push(Part::Buttons(vec![Button::new(Control::CheckAgain, self.t(s::CHECK_AGAIN)).dflt(), Button::new(Control::Close, self.t(s::CLOSE))]));
                "still"
            }
            Page::Error { name, message } => {
                parts.extend([Part::Heading(name.clone()), Part::Warn { text: format!("{}{}", self.t(s::ERR), message), shake: true }]);
                parts.push(Part::Buttons(vec![Button::new(Control::Close, self.t(s::CLOSE)).dflt()]));
                "error"
            }
            Page::Done { name, summary } => {
                self.push_summary(&mut parts, name, summary);
                parts.push(Part::Buttons(vec![Button::new(Control::Close, self.t(s::CLOSE)).dflt()]));
                "done"
            }
            Page::Checklist { summary } => {
                let cl = self.checklist.as_ref()?;
                busy = cl.busy;
                self.push_checklist(&mut parts, cl, *summary);
                if *summary {
                    "done"
                } else {
                    "review"
                }
            }
        };
        Some(PanelView { phase: phase.to_string(), parts, busy })
    }

    fn push_summary(&self, parts: &mut Vec<Part>, name: &str, sm: &payload::Summary) {
        let cnt = [(sm.files, s::FILES_W), (sm.keys, s::KEYS_W), (sm.other, s::OTHER_W)]
            .iter()
            .filter(|x| x.0 > 0)
            .map(|x| format!("{} {}", x.0, self.t(x.1)))
            .collect::<Vec<_>>()
            .join(", ");
        let line = if sm.any() { format!("{}{}", self.t(s::REMOVED), if cnt.is_empty() { "0".to_string() } else { cnt }) } else { self.t(s::NONE).to_string() };
        parts.push(Part::DoneHead {
            title: name.to_string(),
            line,
            need_admin: (sm.need_admin > 0).then(|| format!("{}{}", self.t(s::NEED_ADMIN), sm.need_admin)),
            failed_head: (!sm.failed.is_empty()).then(|| self.t(s::FAILED).to_string()),
            failed: sm.failed.clone(),
        });
        if !sm.backup.is_empty() {
            parts.push(Part::Extra { text: format!("{}{}", self.t(s::BACKUP), sm.backup) });
        }
        if !sm.log.is_empty() {
            parts.push(Part::Extra { text: format!("{}{}", self.t(s::LOG), sm.log) });
        }
        if !sm.backup.is_empty() || !sm.log.is_empty() {
            parts.push(Part::Link(Button::new(Control::OpenBackups, self.t(s::OPEN_BK))));
        }
    }

    fn push_checklist(&self, parts: &mut Vec<Part>, cl: &Checklist, summary: bool) {
        let r = &cl.review;
        let name = if r.name.is_empty() { self.st.name.as_str() } else { r.name.as_str() };
        if summary {
            self.push_summary(parts, name, &r.summary);
        } else {
            parts.push(Part::Heading(format!("{}{}", self.t(s::LEFTOVERS), name)));
        }
        if !r.admin_error.is_empty() {
            let why = if r.admin_error == "cancelled" { self.t(s::CANCELLED).to_string() } else { r.admin_error.clone() };
            parts.push(Part::Warn { text: format!("{}{}", self.t(s::ADMIN_ERR), why), shake: false });
        }
        for (gi, g) in cl.groups.iter().enumerate() {
            parts.push(Part::Group(GroupView {
                index: gi,
                title: self.t(g.key.title()).to_string(),
                tip: g.key.tip().map(|t| self.t(t).to_string()).unwrap_or_default(),
                open: g.open,
                hidden: g.hidden,
                count: g.count.shown().round().max(0.0) as u64,
                size_text: s::size_suffix(g.size.shown().round().max(0.0) as u64),
                rows: g
                    .picks
                    .iter()
                    .map(|&pi| {
                        let p = &cl.picks[pi];
                        let meta = match &p.fail {
                            Some(e) => e.clone(),
                            None => [s::kind_label(&p.item.kind, self.lang), if p.item.size > 0 { s::size(p.item.size) } else { String::new() }, s::note(&p.item.note, self.lang)]
                                .into_iter()
                                .filter(|x| !x.is_empty())
                                .collect::<Vec<_>>()
                                .join(" \u{b7} "),
                        };
                        ItemRowView { pick: pi, checked: p.checked, path: p.item.path.clone(), meta, failed: p.fail.is_some(), gone: p.gone }
                    })
                    .collect(),
            }));
        }
        if r.items.is_empty() {
            parts.push(Part::Sub(self.t(s::NONE).to_string()));
        }
        if !r.manual.is_empty() {
            parts.push(Part::SubHead(self.t(s::MANUAL).to_string()));
            for m in &r.manual {
                parts.push(Part::Path { text: m.clone(), fail: false });
            }
        }
        if r.cut {
            parts.push(Part::Warn { text: self.t(s::CUT).to_string(), shake: false });
        }
        if r.hidden_others > 0 {
            parts.push(Part::Note(format!("{}{}", r.hidden_others, self.t(s::HIDDEN_OTHERS))));
        }
        if !r.items.is_empty() {
            parts.push(Part::Note(self.t(s::SAFE).to_string()));
        }
        let n_user = cl.picked_user().len();
        let n_admin = cl.picked_admin().len();
        let mut adm_row = Vec::new();
        if !cl.admin_scanned {
            adm_row.push(Button::new(Control::AdminScan, self.t(s::ADMIN_SCAN)).ghost().disabled(cl.busy));
        }
        if cl.has_admin() {
            adm_row.push(Button::new(Control::AdminDelete, &format!(" {} ({})", self.t(s::ADMIN_DEL), n_admin)).danger().icon("shield").disabled(n_admin == 0 || cl.busy));
        }
        if !adm_row.is_empty() {
            parts.push(Part::Buttons(adm_row));
        }
        if let Some(p) = &cl.progress {
            parts.insert(0, Part::DeleteProgress { text: format!("{} {} / {}", self.t(s::DELETING), p.n.shown().round().max(0.0) as u64, if p.total > 0 { p.total.to_string() } else { "?".to_string() }) });
        }
        let mut foot = Vec::new();
        if cl.has_certain() {
            foot.push(Button::new(Control::SelectAllCertain, self.t(s::SEL_ALL)).ghost().tip(self.t(s::SEL_ALL_TIP)).disabled(cl.busy));
        }
        if cl.has_user() {
            foot.push(Button::new(Control::DeleteSelected, &format!("{} ({})", self.t(s::DEL), n_user)).danger().disabled(n_user == 0 || cl.busy));
        }
        foot.push(Button::new(Control::Close, self.t(s::CLOSE)).dflt().disabled(cl.busy));
        parts.push(Part::Footer(FooterView {
            selected: cl.has_user().then(|| format!("{}{}{}", self.t(s::SELECTED), cl.sel_count.shown().round().max(0.0) as u64, s::size_suffix(cl.sel_size.shown().round().max(0.0) as u64))),
            buttons: foot,
        }));
    }
}

#[cfg(test)]
mod tests;
