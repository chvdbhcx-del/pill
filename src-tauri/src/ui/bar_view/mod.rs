//! The native command bar (Wave "bar" of the WebView -> native rewrite): `BarView` is the `View` of the popup window `bar`. It replaces
//! `bar.html` + `bar.js` + `uninstall-ui.js` + `md-lite.js` and draws with glasskit; all behaviour lives in the pure model
//! `pill_ui_core::bar` (`Bar::on_event` / `tick` / `view`), this file only translates:
//!
//! * host `InputEvent`s / `Msg`s -> `BarEvent`s, and the model's `Action`s -> backend calls (`Cx::call_async`, answered with `BarEvent::Reply`),
//!   the clipboard, the caret, scrolling and animation cues (`anim.rs`);
//! * the model's `BarView` view-model -> pixels (`paint.rs` for the search row, list, answer, menu, wave; `panel.rs` for the uninstall panel and
//!   its status strip);
//! * window glue (`native.rs`): the popup spec, the `command.rs` toggle / show / hide entry points and the gates.
//!
//! Threading and pacing follow `docs/native-host.md`: one UI thread, `draw` never calls the backend, frames run at the display rate while
//! anything moves (the liquid outline never rests while the bar is open and motion is allowed) and not at all when settled.
//! Reduced motion: the model drops its cues and holds the wave; `anim.rs` plays nothing; glasskit tweens jump.
pub mod anim;
pub mod glyphs;
pub mod icons;
pub mod keys;
mod native;
mod paint;
mod panel;
#[cfg(test)]
mod tests;

#[allow(unused_imports)] // register: wired by main.rs of the island agent; native_active: gates
pub use native::{bar_hide, bar_is_visible, bar_toggle, register};

use self::anim::AnimPlayer;
use self::icons::{IconSet, ShellIcons};
use crate::ui::view::{Cx, Msg, Ticket, View};
use glasskit::canvas::Canvas;
use glasskit::edit::{EditEvent, TextInput};
use glasskit::ease::{curves, Tween};
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Buttons, Cursor, InputEvent, KeyCode, Modifiers, MouseButton, Response};
use glasskit::richtext::{Hit, MarkdownView};
use glasskit::scroll::{Align, Axis, RowSource, RowState, ScrollList, ScrollState, Scrollbar};
use glasskit::theme::Theme;
use pill_ui_core::bar::anim::Target as AnimTarget;
use pill_ui_core::bar::keys::{KeyEvent, RunMode};
use pill_ui_core::bar::layout as L;
use pill_ui_core::bar::uninstall::view::{Control, PanelView};
use pill_ui_core::bar::{Action, Bar, BarEvent, BarView as Vm};
use serde_json::Value;
use std::collections::HashMap;

pub const BAR_LABEL: &str = "bar";

// ---------------------------------------------------------------------------------------------------------------------------------------
// small shared types
// ---------------------------------------------------------------------------------------------------------------------------------------

/// What the keyboard belongs to: the search box, or one of the uninstall panel's controls (index into `keys::panel_focusables`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Input,
    Panel(Option<usize>),
}

/// A thing the pointer can be over. Recorded by `draw` (window coordinates) and used by `event`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Tgt {
    Mic,
    Msg,
    Strip,
    Input,
    Answer,
    Body,
    RowBtn(usize, RunMode),
    Row(usize),
    Ctx(usize),
    Ctl(Control),
}

impl Tgt {
    /// A stable key for the hover / press tweens.
    fn key(&self) -> u64 {
        match *self {
            Tgt::Mic => 1,
            Tgt::Msg => 2,
            Tgt::Strip => 3,
            Tgt::Input => 4,
            Tgt::Answer => 5,
            Tgt::Body => 6,
            Tgt::RowBtn(r, m) => 0x1000 + (r as u64) * 16 + m as u64,
            Tgt::Row(r) => 0x100_000 + r as u64,
            Tgt::Ctx(i) => 0x200_000 + i as u64,
            Tgt::Ctl(c) => 0x300_000 + control_id(c),
        }
    }
}

pub(crate) fn control_id(c: Control) -> u64 {
    match c {
        Control::Start => 1,
        Control::ToggleSilent => 2,
        Control::Cancel => 3,
        Control::Close => 4,
        Control::CheckAgain => 5,
        Control::StopWait => 6,
        Control::Hide => 7,
        Control::StopScan => 8,
        Control::SelectAllCertain => 9,
        Control::DeleteSelected => 10,
        Control::AdminScan => 11,
        Control::AdminDelete => 12,
        Control::OpenBackups => 13,
        Control::ToggleGroup(g) => 100 + g as u64,
        Control::ToggleItem(i) => 1000 + i as u64,
    }
}

/// Hover and press tweens by target key: they ease in on entering, back out on leaving (the web's `transition: background / transform`).
#[derive(Default)]
pub(crate) struct Fx {
    hover: HashMap<u64, Tween<f32>>,
    press: HashMap<u64, Tween<f32>>,
    hot: Option<u64>,
    down: Option<u64>,
}

impl Fx {
    pub fn set_hover(&mut self, k: Option<u64>) -> bool {
        if self.hot == k {
            return false;
        }
        if let Some(o) = self.hot {
            if let Some(t) = self.hover.get_mut(&o) {
                t.retarget(0.0, 0.22, curves::EASE_OUT);
            }
        }
        if let Some(n) = k {
            let t = self.hover.entry(n).or_insert_with(|| Tween::at_rest(0.0));
            t.retarget(1.0, 0.12, curves::EASE_OUT);
        }
        self.hot = k;
        true
    }
    pub fn set_down(&mut self, k: Option<u64>) {
        if self.down == k {
            return;
        }
        if let Some(o) = self.down {
            if let Some(t) = self.press.get_mut(&o) {
                t.retarget(0.0, 0.38, curves::SPRING_POP);
            }
        }
        if let Some(n) = k {
            let t = self.press.entry(n).or_insert_with(|| Tween::at_rest(0.0));
            t.retarget(1.0, 0.08, curves::EASE_OUT);
        }
        self.down = k;
    }
    pub fn hov(&self, k: u64) -> f32 {
        self.hover.get(&k).map_or(0.0, |t| t.value())
    }
    pub fn prs(&self, k: u64) -> f32 {
        self.press.get(&k).map_or(0.0, |t| t.value())
    }
    pub fn hot(&self) -> Option<u64> {
        self.hot
    }
    pub fn step(&mut self, dt: f32) -> bool {
        let mut busy = false;
        for t in self.hover.values_mut().chain(self.press.values_mut()) {
            busy |= t.step(dt);
        }
        if self.hover.len() > 256 {
            self.hover.retain(|k, t| Some(*k) == self.hot || !t.finished() || t.value() > 0.0);
        }
        busy
    }
}

/// One scrolling box (answer, uninstall panel).
pub(crate) struct Scroll {
    pub st: ScrollState,
    pub bar: Scrollbar,
}

impl Scroll {
    fn new() -> Scroll {
        Scroll { st: ScrollState::new(), bar: Scrollbar::new(Axis::Vertical) }
    }
}

/// An entry of the result list: the empty-state tip, a result row, or an amber hint.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Entry {
    Tip(String),
    Row(usize),
    Hint(String),
}

pub(crate) const TIP_PAD: (f32, f32) = (14.0, 18.0);

/// Rough line count of wrapped UI text before it can be measured (the list asks heights without a canvas).
fn est_lines(text: &str, size: f32, width: f32) -> f32 {
    let w = (text.chars().count() as f32 * size * 0.52).max(1.0);
    (w / width.max(1.0)).ceil().max(1.0)
}

struct Rows<'a> {
    entries: &'a [Entry],
}

impl RowSource for Rows<'_> {
    fn len(&self) -> usize {
        self.entries.len()
    }
    fn height(&self, i: usize, width: f32) -> f32 {
        match &self.entries[i] {
            Entry::Row(_) => L::ROW_H,
            Entry::Tip(t) => 2.0 * TIP_PAD.0 + 20.0 * est_lines(t, 13.0, width - 2.0 * TIP_PAD.1),
            Entry::Hint(t) => 4.0 + 8.0 + 16.0 + 18.0 * est_lines(t, 12.0, width - 36.0),
        }
    }
    fn draw_row(&self, _: &Canvas, _: usize, _: Rect, _: RowState) {}
}

/// The rectangles of one frame, in window coordinates.
#[derive(Clone, Debug, Default)]
pub(crate) struct Lay {
    pub body: Rect,
    pub input: Rect,
    pub mic: Rect,
    pub kbd: Rect,
    pub ans: Option<Rect>,
    pub dlg: Option<Rect>,
    pub panel: Option<Rect>,
    /// the rows viewport (x = 6 .. 554)
    pub list: Option<Rect>,
    pub strip: Option<Rect>,
    pub msg: Option<Rect>,
}

/// What canvas-measured sizes the last `draw` found (the content heights flex-box layout needs).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Measured {
    pub ans_content: f32,
    pub panel_content: f32,
    pub kbd_w: f32,
    pub msg_h: f32,
}

/// Flex-shrink: `bases` = (size, shrinkable). Everything fits -> unchanged; otherwise the overflow is taken from the shrinkable ones in
/// proportion to their size (CSS `flex: 0 1 auto` with `min-height: 0`). `fixed` = the space of the items that do not shrink.
pub(crate) fn flow(fixed: f32, bases: &[(f32, bool)], avail: f32) -> Vec<f32> {
    let mut out: Vec<f32> = bases.iter().map(|b| b.0).collect();
    let total = fixed + out.iter().sum::<f32>();
    if total <= avail {
        return out;
    }
    let mut over = total - avail;
    let mut frozen = vec![false; out.len()];
    for _ in 0..4 {
        let sum: f32 = bases.iter().enumerate().filter(|(i, b)| b.1 && !frozen[*i]).map(|(_, b)| b.0).sum();
        if sum <= 0.0 || over <= 0.0 {
            break;
        }
        let mut taken = 0.0;
        for (i, b) in bases.iter().enumerate() {
            if b.1 && !frozen[i] {
                let cut = over * b.0 / sum;
                if out[i] - cut < 0.0 {
                    taken += out[i];
                    out[i] = 0.0;
                    frozen[i] = true;
                } else {
                    out[i] -= cut;
                    taken += cut;
                }
            }
        }
        over -= taken;
        if over < 0.01 {
            break;
        }
    }
    out
}

struct AnswerMd {
    view: Option<MarkdownView>,
    src: String,
    width: f32,
    dark: bool,
    labels: (String, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    /// selecting text in the answer
    Answer,
}

pub struct BarView {
    pub(crate) bar: Bar,
    pub(crate) vm: Vm,
    pub(crate) input: TextInput,
    pub(crate) list: ScrollList,
    pub(crate) entries: Vec<Entry>,
    pub(crate) icons: IconSet,
    pub(crate) shell: ShellIcons,
    pub(crate) anim: AnimPlayer,
    pub(crate) fx: Fx,
    md: AnswerMd,
    pub(crate) ans_scroll: Scroll,
    pub(crate) uni_scroll: Scroll,
    pub(crate) focus: Focus,
    tickets: HashMap<u64, u64>,
    pub(crate) lay: Lay,
    pub(crate) m: Measured,
    pub(crate) hits: Vec<(Rect, Tgt)>,
    pub(crate) now: f64,
    busy_until: f64,
    /// the highlight under the rows: its top in list content coordinates
    pub(crate) ind: Tween<f32>,
    ind_row: usize,
    /// seconds since the selected row changed (the row's buttons and enter icon come in one after another)
    pub(crate) sel_age: f32,
    sel_seen: usize,
    pub(crate) row_off: HashMap<usize, Tween<f32>>,
    prev_ids: Vec<String>,
    last_text: String,
    last_sel: (usize, usize),
    pub(crate) light_os: bool,
    drag: Drag,
    pub(crate) reduced: bool,
    pointer: Option<Pt>,
    /// panel controls in focus order, from the last view
    pub(crate) focusables: Vec<keys::Focusable>,
    want_default_focus: bool,
    pending_scroll: Option<usize>,
    os_sent: bool,
    /// the keyboard focus ring of a panel control shows only after Tab / arrows (the web's :focus-visible)
    pub(crate) ring: bool,
    pub(crate) panel_frame: Option<panel::PanelFrame>,
    pub(crate) chev: HashMap<usize, Tween<f32>>,
}

impl BarView {
    pub fn new(cx: &mut Cx) -> BarView {
        let theme = Theme::pill_dark();
        let mut input = TextInput::new(false, &theme);
        input.style = glasskit::text::Style::new(15.0, 400);
        input.pad = (4.0, 0.0);
        input.radius = 0.0; // the (clipped-away) focus ring of the field has square corners
        input.model.set_max_len(Some(500));
        let mut bar = Bar::new(os_locale(cx));
        bar.on_event(BarEvent::BodyWidth(L::WINDOW_W), 0.0);
        let vm = bar.view();
        for n in ["voice:state", "uninstall:state", "uninstall:item", "uninstall:focus", "ask:chunk"] {
            cx.subscribe(n);
        }
        BarView {
            bar,
            vm,
            input,
            list: {
                let mut l = ScrollList::new(L::ROW_H);
                l.keyboard_select = false;
                l
            },
            entries: Vec::new(),
            icons: IconSet::new(),
            shell: ShellIcons::new(),
            anim: AnimPlayer::new(),
            fx: Fx::default(),
            md: AnswerMd { view: None, src: String::new(), width: 0.0, dark: true, labels: (String::new(), String::new()) },
            ans_scroll: Scroll::new(),
            uni_scroll: Scroll::new(),
            focus: Focus::Input,
            tickets: HashMap::new(),
            lay: Lay::default(),
            m: Measured::default(),
            hits: Vec::new(),
            now: 0.0,
            busy_until: 0.0,
            ind: Tween::at_rest(0.0),
            ind_row: 0,
            sel_age: 10.0,
            sel_seen: 0,
            row_off: HashMap::new(),
            prev_ids: Vec::new(),
            last_text: String::new(),
            last_sel: (0, 0),
            light_os: false,
            drag: Drag::None,
            reduced: false,
            pointer: None,
            focusables: Vec::new(),
            want_default_focus: false,
            pending_scroll: None,
            os_sent: false,
            ring: false,
            panel_frame: None,
            chev: HashMap::new(),
        }
    }

    /// The model, for tests.
    pub fn model(&self) -> &Bar {
        &self.bar
    }

    // ---- model plumbing ----

    /// Feeds one event to the model and carries out what it asks for.
    pub(crate) fn feed(&mut self, ev: BarEvent, cx: &mut Cx) {
        let now = cx.now_ms;
        self.now = now;
        self.busy_until = now + 3300.0; // debounce / confirm / copied timers live in the model: keep frames coming until they have run
        let acts = self.bar.on_event(ev, now);
        self.exec(acts, cx);
        self.refresh();
    }

    pub(crate) fn exec(&mut self, acts: Vec<Action>, cx: &mut Cx) {
        for a in acts {
            match a {
                Action::Call { ticket, call } => {
                    let t: Ticket = cx.call_async(call.cmd, call.args);
                    if let Some(id) = ticket {
                        self.tickets.insert(t.0, id);
                    }
                }
                Action::Copy(text) => cx.set_clipboard_text(&text),
                Action::FocusInput => self.focus_input(),
                Action::FocusPanelDefault => {
                    self.want_default_focus = true;
                    self.ring = false;
                    self.focus = Focus::Panel(None);
                    self.input.set_focused(false);
                }
                Action::SetInput { text, caret } => {
                    self.input.set_text(&text);
                    self.input.set_selection(caret, caret);
                    self.last_text = text;
                    self.last_sel = (caret, caret);
                }
                Action::ScrollToRow(i) => self.pending_scroll = Some(i),
                Action::Anim(a) => match a.target {
                    AnimTarget::Selection { .. } => {} // the highlight glides by itself (`sync_selection`)
                    _ => self.anim.play(&a),
                },
            }
        }
    }

    pub(crate) fn focus_input(&mut self) {
        self.focus = Focus::Input;
        self.input.set_focused(true);
    }

    /// Re-reads the view-model and everything derived from it.
    pub(crate) fn refresh(&mut self) {
        self.vm = self.bar.view();
        self.focusables = self.vm.uninstall.panel.as_ref().map(keys::panel_focusables).unwrap_or_default();
        if self.want_default_focus && !self.focusables.is_empty() {
            self.want_default_focus = false;
            self.focus = Focus::Panel(keys::default_focus(&self.focusables));
        }
        if matches!(self.focus, Focus::Panel(_)) && self.vm.uninstall.panel.is_none() {
            self.focus_input();
        }
        self.sync_entries();
        self.sync_selection();
        self.placeholder();
        let rm = self.bar.reduce_motion();
        if rm != self.reduced {
            self.reduced = rm;
            self.anim.set_reduced(rm);
        }
    }

    fn placeholder(&mut self) {
        if self.input.placeholder != self.vm.placeholder {
            self.input.placeholder = self.vm.placeholder.clone();
        }
    }

    fn sync_entries(&mut self) {
        let vm = &self.vm;
        let mut e = Vec::with_capacity(vm.rows.len() + 2);
        if let Some(t) = &vm.tip {
            e.push(Entry::Tip(t.clone()));
        }
        e.extend((0..vm.rows.len()).map(Entry::Row));
        e.extend(vm.hints.iter().map(|h| Entry::Hint(h.clone())));
        let ids: Vec<String> = vm.rows.iter().map(|r| r.id.clone()).collect();
        if ids != self.prev_ids {
            // survivors that moved slide from their old place (FLIP); new rows come in through the model's own Enter cues
            let d = pill_ui_core::bar::results::diff(&self.prev_ids, &ids);
            if !self.reduced {
                for (new, old) in d.moved {
                    let from = (old as f32 - new as f32) * L::ROW_H;
                    self.row_off.insert(new, Tween::new(from, 0.0, 0.4, curves::SPRING_SETTLE));
                }
            }
            self.list.invalidate_all();
            self.prev_ids = ids;
        }
        if e != self.entries {
            if e.len() != self.entries.len() || e.iter().zip(&self.entries).any(|(a, b)| a != b) {
                self.list.invalidate_all();
            }
            self.entries = e;
        }
    }

    /// Top of result row `i` in list content coordinates.
    pub(crate) fn row_top(&self, i: usize) -> f32 {
        let mut y = 0.0;
        if let Some(Entry::Tip(_)) = self.entries.first() {
            y += self.list.row_height(0);
        }
        y + i as f32 * L::ROW_H
    }

    fn sync_selection(&mut self) {
        let sel = self.vm.selected;
        let target = self.row_top(sel);
        if sel != self.sel_seen {
            self.sel_seen = sel;
            self.sel_age = 0.0;
        }
        if sel != self.ind_row || self.ind.target() != target {
            let glide = self.vm.selection_from.is_some() && !self.reduced && sel != self.ind_row;
            if glide {
                self.ind.retarget(target, 0.38, curves::SPRING_SETTLE);
            } else {
                self.ind.jump(target);
            }
            self.ind_row = sel;
        }
    }

    /// Hands the pending `ScrollToRow` to the list (needs the rows viewport, so it waits for the first layout).
    fn do_scroll(&mut self) {
        let (Some(i), Some(view)) = (self.pending_scroll, self.lay.list) else { return };
        self.pending_scroll = None;
        let entries = std::mem::take(&mut self.entries);
        let src = Rows { entries: &entries };
        self.list.update(&src, view);
        if let Some(idx) = entries.iter().position(|e| *e == Entry::Row(i)) {
            self.list.scroll_to_index(&src, idx, !self.reduced, Align::Nearest);
        }
        self.entries = entries;
    }

    // ---- text box ----

    /// Pushes text / caret changes of the box into the model (`Input` when the text changed, `Caret` when only the selection moved).
    fn sync_text(&mut self, cx: &mut Cx) {
        let text = self.input.text();
        let (a, b) = self.input.selection();
        let sel = (a.min(b), a.max(b));
        if text != self.last_text {
            self.last_text = text.clone();
            self.last_sel = sel;
            self.feed(BarEvent::Input { text, caret: b }, cx);
        } else if sel != self.last_sel {
            self.last_sel = sel;
            self.feed(BarEvent::Caret { start: sel.0, end: sel.1 }, cx);
        }
    }

    fn text_event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let rect = self.input_rect();
        let r = with_measure(|cv| self.input.event(ev, rect, cv)).unwrap_or(Response::NONE);
        for e in self.input.take_events() {
            match e {
                EditEvent::FocusChanged(true) => self.focus = Focus::Input,
                EditEvent::ContextMenu(_) | EditEvent::Submit | EditEvent::Cancel | EditEvent::Changed | EditEvent::FocusChanged(false) => {}
            }
        }
        self.sync_text(cx);
        r
    }

    /// The rectangle the text box is laid out in: the 22 px field plus 4 px on every side (glasskit draws a ring at its edge; the clip hides it).
    pub(crate) fn input_rect(&self) -> Rect {
        let f = if self.lay.input.w > 0.0 { self.lay.input } else { default_input_rect(self.m.kbd_w) };
        f.inset(-4.0)
    }

    // ---- hit testing ----

    pub(crate) fn hit(&self, p: Pt) -> Option<Tgt> {
        self.hits.iter().rev().find(|(r, _)| r.contains(p.0, p.1)).map(|(_, t)| *t)
    }

    fn panel_open(&self) -> bool {
        self.vm.uninstall.panel.is_some()
    }

    /// The panel the user sees (for the key rules).
    pub(crate) fn panel(&self) -> Option<&PanelView> {
        self.vm.uninstall.panel.as_ref()
    }
}

fn os_locale(cx: &Cx) -> &'static str {
    match cx.lang {
        glasskit::lang::Lang::Ru => "ru-RU",
        _ => "en-US",
    }
}

/// The search field when nothing was laid out yet (the first events of a window that was never drawn).
pub(crate) fn default_input_rect(kbd_w: f32) -> Rect {
    let kbd_w = if kbd_w > 0.0 { kbd_w } else { 120.0 };
    let kbd_x = L::WINDOW_W - L::SEARCH_PAD.1 - kbd_w;
    let mic_x = kbd_x - 10.0 - L::MIC_SIZE;
    let x = L::SEARCH_PAD.0 + 18.0 + L::SEARCH_GAP;
    Rect::new(x, 14.0, mic_x - L::SEARCH_GAP - x, 22.0)
}

thread_local! {
    static MEASURE: std::cell::OnceCell<Option<(glasskit::gfx::Gfx, glasskit::text::Res)>> = const { std::cell::OnceCell::new() };
}

static SYSTEM_FONTS: glasskit::fonts::FontSet = glasskit::fonts::FontSet::system();

/// A private canvas for measuring text during input events (`TextInput::event` needs one; `View::event` gets none). Software device, created
/// once per UI thread; nothing is ever painted with it.
fn with_measure<R>(f: impl FnOnce(&Canvas) -> R) -> Option<R> {
    MEASURE.with(|m| {
        let (gfx, res) = m
            .get_or_init(|| {
                unsafe {
                    let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
                }
                let gfx = glasskit::gfx::Gfx::warp().ok()?;
                let res = glasskit::text::Res::new(&gfx, &SYSTEM_FONTS).ok()?;
                Some((gfx, res))
            })
            .as_ref()?;
        let cv = Canvas::new(gfx, res, 1.0);
        Some(f(&cv))
    })
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------------------------------------------------------------------

fn is_null(v: &Value) -> bool {
    v.is_null()
}

impl View for BarView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((L::WINDOW_W, L::WINDOW_H))
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.now = cx.now_ms;
        self.busy_until = self.busy_until.max(cx.now_ms + 400.0);
        let mut r = self.on_input_event(ev, cx);
        r.redraw = true;
        r
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Reply { ticket, result } => {
                if let Some(id) = self.tickets.remove(&ticket.0) {
                    // an `Err` and a `null` are both the JS wrappers' `null`
                    let value = result.ok().filter(|v| !is_null(v));
                    self.feed(BarEvent::Reply { ticket: id, value }, cx);
                }
            }
            Msg::Event { name, payload, .. } => match name.as_str() {
                "voice:state" => self.feed(BarEvent::Voice(payload), cx),
                "uninstall:state" => self.feed(BarEvent::UninstallState(payload), cx),
                "uninstall:item" => self.feed(BarEvent::UninstallItem(payload), cx),
                "uninstall:focus" => self.feed(BarEvent::UninstallFocus, cx),
                "ask:chunk" => {
                    let id = payload.get("id").and_then(Value::as_u64).unwrap_or(0);
                    let delta = payload.get("delta").and_then(Value::as_str).unwrap_or("").to_string();
                    self.feed(BarEvent::AskChunk { id, delta }, cx);
                }
                _ => {}
            },
            Msg::Open(_) | Msg::Custom(_) => {}
        }
        Response::redraw()
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        let dt = (dt_ms / 1000.0) as f32;
        self.now = cx.now_ms;
        if cx.reduced_motion != self.os_sent {
            self.os_sent = cx.reduced_motion;
            self.feed(BarEvent::OsReduceMotion(cx.reduced_motion), cx);
        }
        let acts = self.bar.tick(cx.now_ms);
        let had = !acts.is_empty();
        self.exec(acts, cx);
        self.refresh();
        let mut busy = had || self.bar.animating() || self.vm.uninstall.animating;
        busy |= self.anim.step(dt);
        busy |= self.fx.step(dt);
        busy |= self.ind.step(dt);
        for t in self.chev.values_mut() {
            busy |= t.step(dt);
        }
        self.sel_age += dt;
        busy |= self.sel_age < 0.6 && !self.reduced;
        for t in self.row_off.values_mut() {
            busy |= t.step(dt);
        }
        self.row_off.retain(|_, t| !t.finished());
        busy |= self.input.step(dt);
        let entries = std::mem::take(&mut self.entries);
        if let Some(view) = self.lay.list {
            busy |= self.list.step(dt, &Rows { entries: &entries }, view);
        }
        self.entries = entries;
        self.do_scroll();
        for s in [&mut self.ans_scroll, &mut self.uni_scroll] {
            busy |= s.st.step(dt);
            busy |= s.bar.step(dt);
        }
        if let Some(md) = self.md.view.as_mut() {
            busy |= md.step(dt);
        }
        busy |= self.loops_running();
        busy |= cx.now_ms < self.busy_until && self.vm.visible;
        busy
    }

    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx) {
        self.paint(cv, theme, rect, cx);
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.light_os = native::os_prefers_light();
        self.input.set_text("");
        self.last_text.clear();
        self.last_sel = (0, 0);
        self.anim.clear();
        self.row_off.clear();
        self.ans_scroll.st.scroll_to(0.0, false);
        self.uni_scroll.st.scroll_to(0.0, false);
        self.reduced = cx.reduced_motion;
        self.os_sent = cx.reduced_motion;
        self.feed(BarEvent::OsReduceMotion(cx.reduced_motion), cx);
        self.feed(BarEvent::BodyWidth(L::WINDOW_W), cx);
        self.feed(BarEvent::Shown, cx);
        self.feed(BarEvent::Visibility(true), cx);
        self.focus_input();
        cx.request_frame();
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.feed(BarEvent::Visibility(false), cx);
        self.feed(BarEvent::Hidden, cx);
        self.input.set_text("");
        self.last_text.clear();
        self.tickets.clear();
        self.drag = Drag::None;
        self.fx.set_down(None);
        self.anim.clear();
        self.row_off.clear();
        self.md.view = None;
        self.md.src.clear();
        self.focus_input();
    }

    fn on_dpi(&mut self, _scale: f32, cx: &mut Cx) {
        self.md.view = None; // text layouts depend on device pixels
        cx.request_frame();
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// input
// ---------------------------------------------------------------------------------------------------------------------------------------

impl BarView {
    fn on_input_event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        match ev {
            InputEvent::MouseMove { pos, buttons, .. } => self.on_move(*pos, *buttons, ev, cx),
            InputEvent::MouseLeave => {
                self.pointer = None;
                self.fx.set_hover(None);
                self.feed(BarEvent::BodyMouseLeave, cx);
                let mut r = Response::redraw();
                let entries = std::mem::take(&mut self.entries);
                if let Some(view) = self.lay.list {
                    r = r.merge(self.list.event(ev, &Rows { entries: &entries }, view));
                }
                self.entries = entries;
                r
            }
            InputEvent::MouseDown { pos, button, mods, clicks } => self.on_down(*pos, *button, *mods, *clicks, ev, cx),
            InputEvent::MouseUp { pos, button, .. } => self.on_up(*pos, *button, ev, cx),
            InputEvent::Wheel { pos, delta, precise, .. } => self.on_wheel(*pos, *delta, *precise, ev, cx),
            InputEvent::Key { code, mods, down, .. } => {
                if *down {
                    self.on_key(*code, *mods, ev, cx)
                } else {
                    Response::NONE
                }
            }
            InputEvent::Char(c) => self.on_char(*c, ev, cx),
            InputEvent::Focus(on) => {
                let r = self.text_event(ev, cx);
                if *on {
                    self.feed(BarEvent::Focus, cx);
                }
                r
            }
            _ => Response::NONE,
        }
    }

    fn on_move(&mut self, pos: Pt, buttons: Buttons, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.pointer = Some(pos);
        let tgt = self.hit(pos);
        let key = tgt.filter(|t| !matches!(t, Tgt::Body | Tgt::Answer)).map(|t| t.key());
        let mut r = Response::NONE;
        // a selection drag in the answer
        if self.drag == Drag::Answer && buttons.left() {
            if let Some(a) = self.lay.ans {
                let o = self.answer_origin(a);
                if let Some(md) = self.md.view.as_mut() {
                    if let Some(p) = md.text_pos_at((pos.0 - o.0, pos.1 - o.1)) {
                        let anchor = md.selection().map_or(p, |s| s.anchor);
                        md.set_selection(Some(glasskit::richtext::Selection { anchor, focus: p }));
                    }
                }
            }
            return Response { consumed: true, capture: true, redraw: true, cursor: Some(Cursor::Text), ..Response::NONE };
        }
        if self.fx.set_hover(key) {
            r.redraw = true;
        }
        if let Some(body) = Some(self.lay.body).filter(|b| b.contains(pos.0, pos.1)) {
            self.feed(BarEvent::BodyMouse { x: pos.0 - (body.x + pill_ui_core::bar::wave::INSET) }, cx);
        } else {
            self.feed(BarEvent::BodyMouseLeave, cx);
        }
        // the answer's copy buttons ease their hover
        if let (Some(a), Some(md)) = (self.lay.ans, self.md.view.as_mut()) {
            let o = (a.x + 18.0, a.y + 12.0 - self.ans_scroll.st.offset());
            let h = if a.contains(pos.0, pos.1) { md.hit((pos.0 - o.0, pos.1 - o.1)) } else { Hit::None };
            md.set_hover(&h);
        }
        // scrollbars and the list
        if let Some(a) = self.lay.ans {
            r = r.merge(self.ans_scroll.bar.event(ev, a, &mut self.ans_scroll.st));
        }
        if let Some(p) = self.lay.panel {
            r = r.merge(self.uni_scroll.bar.event(ev, p, &mut self.uni_scroll.st));
        }
        if let Some(view) = self.lay.list {
            let entries = std::mem::take(&mut self.entries);
            let hover_before = self.list.hovered();
            r = r.merge(self.list.event(ev, &Rows { entries: &entries }, view));
            let hov = self.list.hovered();
            self.entries = entries;
            if hov != hover_before {
                if let Some(Entry::Row(i)) = hov.and_then(|h| self.entries.get(h)).cloned() {
                    if i != self.vm.selected && !matches!(tgt, Some(Tgt::RowBtn(..))) || (i != self.vm.selected && self.hit_is_row(pos, i)) {
                        self.feed(BarEvent::RowHover(i), cx);
                    }
                }
            }
        }
        r = r.merge(self.text_event(ev, cx));
        r.cursor = Some(match tgt {
            Some(Tgt::Input) => Cursor::Text,
            Some(Tgt::Answer) => Cursor::Text,
            Some(Tgt::Mic | Tgt::Msg | Tgt::Strip | Tgt::RowBtn(..) | Tgt::Ctx(_) | Tgt::Ctl(_)) => Cursor::Hand,
            _ => Cursor::Arrow,
        });
        r.redraw = true;
        r
    }

    fn hit_is_row(&self, pos: Pt, i: usize) -> bool {
        matches!(self.hit(pos), Some(Tgt::Row(r)) if r == i)
    }

    fn answer_origin(&self, a: Rect) -> Pt {
        (a.x + 18.0, a.y + 12.0 - self.ans_scroll.st.offset())
    }

    fn on_down(&mut self, pos: Pt, button: MouseButton, mods: Modifiers, clicks: u8, ev: &InputEvent, cx: &mut Cx) -> Response {
        let tgt = self.hit(pos);
        let in_body = self.lay.body.contains(pos.0, pos.1);
        let on_ctx = matches!(tgt, Some(Tgt::Ctx(_)));
        // a context-menu line acts on the press
        if let (MouseButton::Left, Some(Tgt::Ctx(k))) = (button, tgt) {
            if let Some(mode) = self.vm.context.as_ref().and_then(|c| c.lines.get(k)).map(|l| l.mode) {
                self.feed(BarEvent::MouseDown { on_context: true, on_body: true }, cx);
                self.feed(BarEvent::ContextPick(mode), cx);
                return Response::consumed();
            }
        }
        self.feed(BarEvent::MouseDown { on_context: on_ctx, on_body: in_body }, cx);
        if !in_body && !on_ctx {
            return Response::consumed();
        }
        if button == MouseButton::Right {
            match tgt {
                Some(Tgt::Row(i)) | Some(Tgt::RowBtn(i, _)) => self.feed(BarEvent::RowContext { index: i, x: pos.0, y: pos.1 }, cx),
                Some(Tgt::Input) => {
                    let r = self.text_event(ev, cx);
                    return r;
                }
                _ => self.feed(BarEvent::BackgroundContext, cx),
            }
            return Response::consumed();
        }
        if button != MouseButton::Left {
            return Response::NONE;
        }
        self.fx.set_down(tgt.map(|t| t.key()));
        // scrollbars first (they sit on top of their boxes)
        if let Some(a) = self.lay.ans {
            let r = self.ans_scroll.bar.event(ev, a, &mut self.ans_scroll.st);
            if r.consumed || r.capture {
                return r;
            }
        }
        if let Some(p) = self.lay.panel {
            let r = self.uni_scroll.bar.event(ev, p, &mut self.uni_scroll.st);
            if r.consumed || r.capture {
                return r;
            }
        }
        match tgt {
            Some(Tgt::Mic) => {
                self.feed(BarEvent::MicClick, cx);
                Response::consumed()
            }
            Some(Tgt::Msg) => {
                self.feed(BarEvent::MessageClick, cx);
                Response::consumed()
            }
            Some(Tgt::Strip) => {
                self.feed(BarEvent::UninstallFocus, cx);
                Response::consumed()
            }
            Some(Tgt::Ctl(c)) => {
                if let Some(i) = self.focusables.iter().position(|f| f.control == c) {
                    self.ring = false;
                    self.focus = Focus::Panel(Some(i));
                    self.input.set_focused(false);
                }
                self.feed(BarEvent::UninstallPress(c), cx);
                Response::consumed()
            }
            Some(Tgt::RowBtn(i, mode)) => {
                self.feed(BarEvent::RowClick { index: i, button: Some(mode), ctrl: mods.ctrl, shift: mods.shift }, cx);
                Response::consumed()
            }
            Some(Tgt::Row(i)) => {
                self.feed(BarEvent::RowClick { index: i, button: None, ctrl: mods.ctrl, shift: mods.shift }, cx);
                Response::consumed()
            }
            Some(Tgt::Answer) => self.answer_down(pos, clicks, cx),
            Some(Tgt::Input) => self.text_event(ev, cx).merge(Response { focus: true, ..Response::NONE }),
            _ => {
                // anything else in the list area (tip, hints) or the bare body
                if let Some(view) = self.lay.list {
                    let entries = std::mem::take(&mut self.entries);
                    let r = self.list.event(ev, &Rows { entries: &entries }, view);
                    self.entries = entries;
                    let _ = self.list.take_events();
                    if r.consumed || r.capture {
                        return r;
                    }
                }
                Response::NONE
            }
        }
    }

    fn answer_down(&mut self, pos: Pt, clicks: u8, cx: &mut Cx) -> Response {
        let Some(a) = self.lay.ans else { return Response::NONE };
        let o = self.answer_origin(a);
        let local = (pos.0 - o.0, pos.1 - o.1);
        let Some(md) = self.md.view.as_mut() else { return Response::consumed() };
        match md.hit(local) {
            Hit::CopyButton(leaf) => {
                let block = md.code_frames().iter().position(|(l, _)| *l == leaf).unwrap_or(0);
                let text = md.leaf_text(leaf);
                md.mark_copied(leaf);
                self.feed(BarEvent::CopyCode { block, text }, cx);
                Response::consumed()
            }
            Hit::Text(p) => {
                if clicks >= 3 {
                    md.select_all();
                } else {
                    md.set_selection(Some(glasskit::richtext::Selection { anchor: p, focus: p }));
                    self.drag = Drag::Answer;
                }
                Response { consumed: true, capture: self.drag == Drag::Answer, redraw: true, ..Response::NONE }
            }
            _ => {
                md.set_selection(None);
                Response::consumed()
            }
        }
    }

    fn on_up(&mut self, pos: Pt, button: MouseButton, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.fx.set_down(None);
        let mut r = Response::NONE;
        if button == MouseButton::Left {
            if let Some(a) = self.lay.ans {
                r = r.merge(self.ans_scroll.bar.event(ev, a, &mut self.ans_scroll.st));
            }
            if let Some(p) = self.lay.panel {
                r = r.merge(self.uni_scroll.bar.event(ev, p, &mut self.uni_scroll.st));
            }
            if self.drag == Drag::Answer {
                self.drag = Drag::None;
                let collapsed = self.md.view.as_ref().and_then(|m| m.selection()).is_none_or(|s| s.is_empty());
                if self.lay.ans.is_some_and(|a| a.contains(pos.0, pos.1)) {
                    self.feed(BarEvent::AnswerMouseUp { selection_collapsed: collapsed }, cx);
                }
                return Response { consumed: true, redraw: true, ..r };
            }
            if let Some(view) = self.lay.list {
                let entries = std::mem::take(&mut self.entries);
                r = r.merge(self.list.event(ev, &Rows { entries: &entries }, view));
                self.entries = entries;
            }
            r = r.merge(self.text_event(ev, cx));
        }
        r
    }

    fn on_wheel(&mut self, pos: Pt, delta: Pt, precise: bool, ev: &InputEvent, _cx: &mut Cx) -> Response {
        if let Some(p) = self.lay.panel.filter(|p| p.contains(pos.0, pos.1)) {
            let moved = self.uni_scroll.st.wheel(delta.1, precise);
            if moved {
                self.uni_scroll.bar.show();
            }
            let _ = p;
            return Response { consumed: moved, redraw: moved, ..Response::NONE };
        }
        if let Some(a) = self.lay.ans.filter(|a| a.contains(pos.0, pos.1)) {
            let moved = self.ans_scroll.st.wheel(delta.1, precise);
            if moved {
                self.ans_scroll.bar.show();
            }
            let _ = a;
            return Response { consumed: moved, redraw: moved, ..Response::NONE };
        }
        if let Some(view) = self.lay.list {
            let entries = std::mem::take(&mut self.entries);
            let r = self.list.event(ev, &Rows { entries: &entries }, view);
            self.entries = entries;
            return r;
        }
        Response::NONE
    }

    fn key_target(&self) -> (bool, bool) {
        let search = self.focus == Focus::Input;
        (search, search)
    }

    fn on_key(&mut self, code: KeyCode, mods: Modifiers, ev: &InputEvent, cx: &mut Cx) -> Response {
        let Some(mut k) = keys::key_event(code, mods) else {
            // keys the bar has no name for still reach the box
            return if self.focus == Focus::Input { self.text_event(ev, cx) } else { Response::NONE };
        };
        let (in_field, in_search) = self.key_target();
        k.target_is_field = in_field;
        k.target_is_search = in_search;
        // Space / Enter / Tab on a focused panel control act on it (the browser's button behaviour)
        if let (Focus::Panel(idx), true) = (self.focus, self.panel_open()) {
            if self.panel_key(&k, idx, cx) {
                return Response::consumed();
            }
        }
        let panel_open = self.panel_open();
        let consumed = keys::bar_consumes(&k, panel_open);
        let typing = pill_ui_core::bar::keys::is_typing(Some(&k));
        // the model sees every key first (the uninstall panel's capture handler is part of it)
        let was_panel = matches!(self.focus, Focus::Panel(_));
        self.feed(BarEvent::Key(k), cx);
        if consumed {
            return Response::consumed();
        }
        if was_panel && !typing {
            return Response::NONE;
        }
        if self.focus == Focus::Input {
            return self.text_event(ev, cx);
        }
        Response::NONE
    }

    /// Keys that belong to a focused panel control. True when handled.
    fn panel_key(&mut self, k: &KeyEvent, idx: Option<usize>, cx: &mut Cx) -> bool {
        if k.ctrl || k.alt || k.meta {
            return false;
        }
        match k.key.as_str() {
            "Tab" | "ArrowDown" | "ArrowUp" => {
                let dir = if k.key == "ArrowUp" || (k.key == "Tab" && k.shift) { -1 } else { 1 };
                if let Some(n) = keys::next_focus(&self.focusables, idx, dir) {
                    self.ring = true;
                    self.focus = Focus::Panel(Some(n));
                }
                true
            }
            "Enter" | " " => {
                let i = idx.or_else(|| keys::default_focus(&self.focusables));
                if let Some(f) = i.and_then(|i| self.focusables.get(i)).cloned() {
                    if f.enabled {
                        self.feed(BarEvent::UninstallPress(f.control), cx);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn on_char(&mut self, c: char, ev: &InputEvent, cx: &mut Cx) -> Response {
        let mut k = keys::char_key(c);
        let (in_field, in_search) = self.key_target();
        k.target_is_field = in_field;
        k.target_is_search = in_search;
        // typing never waits for the panel: the model steps it aside (and asks for the caret), the character lands in the box
        if c != ' ' || self.focus == Focus::Input {
            self.feed(BarEvent::Key(k), cx);
        }
        if self.focus != Focus::Input {
            // a space on a panel control is the control's (handled on the key press); everything else was typing
            return Response::NONE;
        }
        self.text_event(ev, cx)
    }

    /// Is something looping (spinner, progress bar, recording dot) that must keep drawing? Reduced motion draws them still.
    fn loops_running(&self) -> bool {
        if self.reduced {
            return false;
        }
        self.vm.answer.as_ref().is_some_and(|a| a.thinking)
            || self.vm.uninstall.strip.as_ref().is_some_and(|s| s.bar || s.icon == pill_ui_core::bar::uninstall::view::StripIcon::Spinner)
            || self.vm.uninstall.panel.as_ref().is_some_and(|p| p.parts.iter().any(|x| matches!(x, pill_ui_core::bar::uninstall::view::Part::Progress(_) | pill_ui_core::bar::uninstall::view::Part::Shield)))
            || self.vm.mic.state == pill_ui_core::bar::model::MicState::Recording
    }
}
