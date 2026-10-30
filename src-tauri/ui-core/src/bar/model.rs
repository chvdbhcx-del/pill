//! The command bar model: everything bar.js does between a key press and a drawn frame, as `on_event` + `tick` -> `Vec<Action>` and `view()`.
//!
//! Flow (mirrors bar.js):
//!   * typing: every input kicks the wave, the uninstall panel (if up) steps aside first, then an 80 ms debounce runs `search()`;
//!   * `search()`: ends a dialogue, clears the answer, then `? question` shows the ask row, otherwise `cmd_query { text }`; a stale reply
//!     (the query moved on, the bar closed) is dropped by the `seq` counter;
//!   * `render()`: the rows replace the list, selection goes to row 0, shell icons are requested per app / file / folder row (`cmd_icon`);
//!   * keys: Esc, arrows, PageUp/Down, Ctrl+Home/End, Tab groups, F4 dictation, Ctrl+P pin, Ctrl+Shift+C copy path, Enter with Ctrl / Shift
//!     variants, Ctrl+U (uninstall panel); right click opens the per-row menu; clicks on the transparent margin close the bar;
//!   * `run(row, mode)`: `cmd_run`, then the outcome (copy, updated, confirm, error) shapes the row / message; `Uninstall` rows open the
//!     uninstall panel, the row's trash button / «Удалить…» put `u <name>` in the box;
//!   * AI: `ask` rows call `cmd_ask`, the answer opens a dialogue (follow-ups, copy, new conversation), `AskChunk` events accumulate streamed text;
//!   * lifecycle: `Shown` resets everything (and reads settings and the uninstall job from the backend), `Hidden` clears state, cancels a pending
//!     ask, closes the menu and stops the wave. The window blur guard lives in Rust (command.rs); the UI side is `Focus` -> caret into the box.
//!
//! The renderer performs `Action::Call`s and answers with `BarEvent::Reply { ticket, value }` (`None` = the invoke failed, the JS wrappers' `null`).

use super::anim::{self, Anim, Anims};
use super::ask::{self, Answer, Dialog, Finished};
use super::keys::{intent_of, Intent, KeyEvent, RunMode};
use super::lang::{Lang, Tr};
use super::protocol::{AskResult, BackendCall, Config, Kind, Outcome, Res, Status, VoicePayload};
use super::query::{self, Input};
use super::results::{self, AskState, IconCache, RowView};
use super::strings as s;
use super::uninstall::ui::UninstallUi;
use super::uninstall::view::{Control, Out, ReplyTag, UninstallView};
use super::wave::Wave;
use serde_json::Value;
use std::collections::HashMap;

/// Window size of the bar (command.rs `WIN_W` x `WIN_H`), used to keep the context menu inside it.
pub const WIN_W: f32 = 560.0;
pub const WIN_H: f32 = 350.0;
/// Context menu metrics (bar.css `.ctx`): min width 250, padding 6, lines 34 px tall.
const MENU_W: f32 = 250.0;
const MENU_PAD: f32 = 6.0;
const MENU_LINE: f32 = 34.0;

/// How long a dangerous row stays armed after the first Enter (`CONFIRM_MS`).
pub const CONFIRM_MS: f64 = 3000.0;
/// «Скопировано» stays on the label / code button this long.
pub const COPIED_MS: f64 = 1800.0;
/// The pin confirmation message fades out after this long, and its exit animation takes 220 ms.
pub const PIN_MSG_MS: f64 = 2600.0;
const PIN_MSG_EXIT_MS: f64 = 220.0;

/// What the renderer has to do. Calls are answered with `BarEvent::Reply`.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// run a backend command; `ticket: None` = nobody waits for the answer
    Call { ticket: Option<u64>, call: BackendCall },
    /// write text to the clipboard
    Copy(String),
    /// caret into the search box
    FocusInput,
    /// keyboard to the default button of the uninstall step
    FocusPanelDefault,
    /// replace the box text and put the caret at `caret` (char index)
    SetInput { text: String, caret: usize },
    /// bring row i into view
    ScrollToRow(usize),
    /// play an animation
    Anim(Anim),
}

#[derive(Clone, Debug, PartialEq)]
pub enum BarEvent {
    /// `bar:shown`
    Shown,
    /// `bar:hidden`
    Hidden,
    /// the window got focus: the caret goes back to the box
    Focus,
    /// the search box text changed (`caret` = char index of the caret)
    Input { text: String, caret: usize },
    /// selection inside the box moved (used to insert a dictated phrase at the caret)
    Caret { start: usize, end: usize },
    Key(KeyEvent),
    /// a click on row `index`; `button` = one of the row's own hover buttons
    RowClick { index: usize, button: Option<RunMode>, ctrl: bool, shift: bool },
    RowHover(usize),
    /// right click on row `index` at window position (x, y)
    RowContext { index: usize, x: f32, y: f32 },
    /// right click anywhere else (not in the search box)
    BackgroundContext,
    /// a line of the context menu
    ContextPick(RunMode),
    /// any mouse-down: inside the menu / inside the bar body or on the transparent margin around it
    MouseDown { on_context: bool, on_body: bool },
    /// the cursor over the bar body, x relative to the wave's left edge (the sag follows it)
    BodyMouse { x: f32 },
    BodyMouseLeave,
    /// the body's width changed (re-cut the wave)
    BodyWidth(f32),
    MicClick,
    /// click on the message line (opens the settings page it links to)
    MessageClick,
    /// mouse-up in the answer block (a click without a selection returns the caret to the box)
    AnswerMouseUp { selection_collapsed: bool },
    /// the copy button of code block `block` of the answer
    CopyCode { block: usize, text: String },
    /// the answer to an `Action::Call` with a ticket
    Reply { ticket: u64, value: Option<Value> },
    /// a streamed piece of an AI answer
    AskChunk { id: u64, delta: String },
    /// `voice:state`
    Voice(Value),
    /// `uninstall:state` / `uninstall:item` / `uninstall:focus`
    UninstallState(Value),
    UninstallItem(Value),
    UninstallFocus,
    /// a press in the uninstall panel
    UninstallPress(Control),
    /// the OS "reduce motion" preference
    OsReduceMotion(bool),
    /// the window became visible / hidden (`visibilitychange`)
    Visibility(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    Query { seq: u64, empty: bool },
    Icon { id: String },
    Run { id: String, mode: RunMode },
    Ask { id: u64 },
    Settings,
    VoiceState,
    VoiceStart,
    Uninstall(ReplyTag),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicState {
    Idle,
    Recording,
    Transcribing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicView {
    pub state: MicState,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageView {
    pub text: String,
    /// the message is a link: click opens this settings page
    pub link: Option<String>,
    /// a confirmation (green) instead of an error (red)
    pub ok: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnswerView {
    /// «Думаю…» with a spinner (a streamed `text` may already be shown)
    pub thinking: bool,
    pub thinking_label: String,
    /// the answer's markdown source
    pub text: String,
    pub copy_code_label: String,
    /// the code block whose copy button says «Скопировано» right now
    pub code_copied: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContextView {
    pub row: usize,
    /// top-left of the menu, kept inside the window
    pub x: f32,
    pub y: f32,
    /// animation origin: the cursor relative to the menu's corner
    pub origin: (f32, f32),
    pub lines: Vec<results::Action>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BarView {
    pub visible: bool,
    /// the body is transparent between two openings (`.body.off`)
    pub parked: bool,
    pub placeholder: String,
    pub hotkey: String,
    pub input: String,
    /// `light` | `dark` | `auto` (the renderer resolves `auto` against the OS)
    pub theme: String,
    pub reduce_motion: bool,
    pub mic: MicView,
    /// «Диалог · Esc — закончить»
    pub dialog_hint: Option<String>,
    pub answer: Option<AnswerView>,
    /// the empty-state tip / the hint about the AI prefix
    pub tip: Option<String>,
    pub rows: Vec<RowView>,
    pub hints: Vec<String>,
    pub selected: usize,
    /// the highlight glides from this row (None = it snapped)
    pub selection_from: Option<usize>,
    pub message: Option<MessageView>,
    pub context: Option<ContextView>,
    pub uninstall: UninstallView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TipKind {
    Help,
    Ask,
}

struct Confirm {
    row: usize,
    until: f64,
}

pub struct Bar {
    lang: Lang,
    os_locale: String,
    cfg: Config,
    os_reduce: bool,
    now: f64,
    anims: Anims,
    outs: Vec<Action>,
    visible: bool,
    parked: bool,
    window_visible: bool,
    // input
    input: String,
    sel_start: usize,
    sel_end: usize,
    debounce: Option<f64>,
    // results
    items: Vec<Res>,
    hints: Vec<Res>,
    words: Vec<String>,
    tip: Option<TipKind>,
    sel: usize,
    sel_from: Option<usize>,
    fresh: bool,
    seq: u64,
    sub_overrides: HashMap<usize, String>,
    confirm: Option<Confirm>,
    copied: Option<(usize, f64)>,
    icons: IconCache,
    // replies
    ticket: u64,
    pending: HashMap<u64, Pending>,
    // message line
    msg: Option<MessageView>,
    pin_msg: Option<(f64, String)>,
    msg_exit: Option<f64>,
    // AI
    dialog: Dialog,
    answer: Answer,
    code_copied: Option<(usize, f64)>,
    // context menu
    ctx: Option<ContextView>,
    // dictation
    mic: MicState,
    dict_here: bool,
    // pieces
    wave: Wave,
    uninstall: UninstallUi,
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}
fn char_slice(s: &str, a: usize, b: usize) -> String {
    s.chars().skip(a).take(b.saturating_sub(a)).collect()
}

impl Bar {
    /// `os_locale`: the OS language tag (`ru-RU`), used until `settings_get` answers.
    pub fn new(os_locale: &str) -> Bar {
        let lang = Lang::from_locale(os_locale);
        Bar {
            lang,
            os_locale: os_locale.to_string(),
            cfg: Config::default(),
            os_reduce: false,
            now: 0.0,
            anims: Anims::new(false),
            outs: Vec::new(),
            visible: false,
            parked: true,
            window_visible: true,
            input: String::new(),
            sel_start: 0,
            sel_end: 0,
            debounce: None,
            items: Vec::new(),
            hints: Vec::new(),
            words: Vec::new(),
            tip: None,
            sel: 0,
            sel_from: None,
            fresh: false,
            seq: 0,
            sub_overrides: HashMap::new(),
            confirm: None,
            copied: None,
            icons: IconCache::new(),
            ticket: 0,
            pending: HashMap::new(),
            msg: None,
            pin_msg: None,
            msg_exit: None,
            dialog: Dialog::new(),
            answer: Answer::None,
            code_copied: None,
            ctx: None,
            mic: MicState::Idle,
            dict_here: false,
            wave: Wave::new(),
            uninstall: UninstallUi::new(lang, false),
        }
    }

    // ---- small accessors ----

    pub fn lang(&self) -> Lang {
        self.lang
    }
    pub fn reduce_motion(&self) -> bool {
        self.cfg.reduce_motion || self.os_reduce
    }
    pub fn selected(&self) -> usize {
        self.sel
    }
    pub fn items(&self) -> &[Res] {
        &self.items
    }
    pub fn input(&self) -> &str {
        &self.input
    }
    pub fn is_visible(&self) -> bool {
        self.visible
    }
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_open()
    }
    pub fn uninstall(&self) -> &UninstallUi {
        &self.uninstall
    }
    /// The liquid outline for the renderer (see `wave`).
    pub fn wave(&self) -> &Wave {
        &self.wave
    }
    /// True while the renderer must keep its frame clock running (wave, glides, pending transitions): never throttle it.
    pub fn animating(&self) -> bool {
        self.wave.running() || self.uninstall.animating()
    }

    fn t(&self, x: s::S) -> &'static str {
        x.tr(self.lang)
    }
    fn ask_state(&self) -> AskState {
        AskState { net_ai: self.cfg.net_ai, base_url: self.cfg.ai_base_url.clone(), model: self.cfg.ai_model.clone() }
    }

    // ---- output plumbing ----

    fn call(&mut self, call: BackendCall, pending: Option<Pending>) {
        let ticket = pending.map(|p| {
            self.ticket += 1;
            self.pending.insert(self.ticket, p);
            self.ticket
        });
        self.outs.push(Action::Call { ticket, call });
    }
    fn take(&mut self) -> Vec<Action> {
        for a in self.anims.drain() {
            self.outs.push(Action::Anim(a));
        }
        std::mem::take(&mut self.outs)
    }
    fn push_uninstall(&mut self, outs: Vec<Out>) {
        for o in outs {
            match o {
                Out::Call { call, reply } => self.call(call, reply.map(Pending::Uninstall)),
                Out::FocusDefault => self.outs.push(Action::FocusPanelDefault),
                Out::FocusInput => self.outs.push(Action::FocusInput),
                Out::SearchAgain => self.input_changed(),
                Out::Anim(a) => self.outs.push(Action::Anim(a)),
            }
        }
    }
    fn sync_reduce(&mut self) {
        let r = self.reduce_motion();
        self.anims.set_reduce(r);
        self.uninstall.set_reduce_motion(r);
        if r {
            self.wave.stop();
        }
    }

    // ---- events ----

    pub fn on_event(&mut self, ev: BarEvent, now_ms: f64) -> Vec<Action> {
        self.now = now_ms;
        self.uninstall.list_has_rows = !self.items.is_empty();
        match ev {
            BarEvent::Shown => self.on_shown(),
            BarEvent::Hidden => self.on_hidden(),
            BarEvent::Focus => self.outs.push(Action::FocusInput),
            BarEvent::Input { text, caret } => {
                self.input = text;
                self.sel_start = caret;
                self.sel_end = caret;
                self.input_changed();
            }
            BarEvent::Caret { start, end } => {
                self.sel_start = start;
                self.sel_end = end;
            }
            BarEvent::Key(k) => self.on_key(k),
            BarEvent::RowClick { index, button, ctrl, shift } => {
                let mode = button.unwrap_or_else(|| super::keys::mode_of(ctrl, shift));
                self.run(index, mode);
            }
            BarEvent::RowHover(i) => {
                if i < self.items.len() && i != self.sel {
                    self.select(i as i64);
                }
            }
            BarEvent::RowContext { index, x, y } => self.on_row_context(index, x, y),
            BarEvent::BackgroundContext => self.close_ctx(),
            BarEvent::ContextPick(mode) => {
                if let Some(c) = self.ctx.take() {
                    self.run(c.row, mode);
                }
            }
            BarEvent::MouseDown { on_context, on_body } => {
                if !on_context {
                    self.close_ctx();
                }
                if !on_body && !on_context {
                    self.call(BackendCall::toggle(), None);
                }
            }
            BarEvent::BodyMouse { x } => self.wave.mouse(x),
            BarEvent::BodyMouseLeave => self.wave.mouse_leave(),
            BarEvent::BodyWidth(w) => {
                self.wave.set_body_width(w);
            }
            BarEvent::MicClick => self.dictate(),
            BarEvent::MessageClick => {
                if let Some(page) = self.msg.as_ref().and_then(|m| m.link.clone()) {
                    self.call(BackendCall::settings_open(&page), None);
                }
            }
            BarEvent::AnswerMouseUp { selection_collapsed } => {
                if selection_collapsed {
                    self.outs.push(Action::FocusInput);
                }
            }
            BarEvent::CopyCode { block, text } => {
                self.outs.push(Action::Copy(text));
                self.code_copied = Some((block, self.now + COPIED_MS));
                self.outs.push(Action::FocusInput);
            }
            BarEvent::Reply { ticket, value } => self.on_reply(ticket, value),
            BarEvent::AskChunk { id, delta } => {
                if self.dialog.chunk(id, &delta) {
                    self.answer = Answer::Thinking { partial: self.dialog.partial().to_string() };
                }
            }
            BarEvent::Voice(v) => self.on_voice(&VoicePayload::from_value(&v)),
            BarEvent::UninstallState(v) => {
                let o = self.uninstall.on_state(&v, self.now);
                self.push_uninstall(o);
            }
            BarEvent::UninstallItem(v) => {
                let o = self.uninstall.on_item(&v, self.now);
                self.push_uninstall(o);
            }
            BarEvent::UninstallFocus => {
                let o = self.uninstall.on_focus(self.now);
                self.push_uninstall(o);
            }
            BarEvent::UninstallPress(c) => {
                let o = self.uninstall.press(c, self.now);
                self.push_uninstall(o);
            }
            BarEvent::OsReduceMotion(r) => {
                self.os_reduce = r;
                self.sync_reduce();
            }
            BarEvent::Visibility(v) => {
                self.window_visible = v;
                if !v {
                    self.parked = true;
                    self.wave.stop();
                } else if self.visible {
                    self.wave.start(self.now, self.reduce_motion(), false);
                }
            }
        }
        self.take()
    }

    /// Advance timers, transitions and the wave. Call once per frame.
    pub fn tick(&mut self, now_ms: f64) -> Vec<Action> {
        self.now = now_ms;
        if let Some(due) = self.debounce {
            if now_ms >= due {
                self.debounce = None;
                if self.dialog.is_open() {
                    self.dlg_input();
                } else {
                    self.search();
                }
            }
        }
        if self.confirm.as_ref().is_some_and(|c| now_ms >= c.until) {
            self.confirm = None;
        }
        if self.copied.is_some_and(|c| now_ms >= c.1) {
            self.copied = None;
        }
        if self.code_copied.is_some_and(|c| now_ms >= c.1) {
            self.code_copied = None;
        }
        if let Some((due, text)) = self.pin_msg.clone() {
            if now_ms >= due {
                self.pin_msg = None;
                if self.msg.as_ref().is_some_and(|m| m.text == text) {
                    self.anims.push(anim::message_exit());
                    self.msg_exit = Some(now_ms + PIN_MSG_EXIT_MS);
                }
            }
        }
        if self.msg_exit.is_some_and(|d| now_ms >= d) {
            self.msg_exit = None;
            if self.msg.as_ref().is_some_and(|m| m.ok) {
                self.msg = None;
            }
        }
        self.wave.step(now_ms);
        let o = self.uninstall.tick(now_ms);
        self.push_uninstall(o);
        self.take()
    }

    // ---- lifecycle ----

    fn on_shown(&mut self) {
        // reset(): everything starts clean for each opening
        self.end_dialog();
        self.input.clear();
        self.sel_start = 0;
        self.sel_end = 0;
        self.outs.push(Action::SetInput { text: String::new(), caret: 0 });
        self.clear_msg();
        self.clear_ans();
        self.debounce = None;
        self.visible = true;
        self.parked = false;
        self.anims.push(anim::body_pop());
        self.fresh = true;
        self.outs.push(Action::FocusInput);
        self.call(BackendCall::settings_get(), Some(Pending::Settings));
        self.search();
        // the uninstall page's own `bar:shown` handler: read the job back from the backend
        let o = self.uninstall.on_bar_shown(self.now);
        self.push_uninstall(o);
    }

    fn on_hidden(&mut self) {
        self.debounce = None;
        self.visible = false;
        self.parked = true;
        self.wave.stop();
        self.close_ctx();
        self.seq += 1;
        self.end_dialog();
        self.clear_ans();
        self.dict_here = false;
        self.set_mic(MicState::Idle);
        self.confirm = None;
        self.copied = None;
        self.input.clear();
        self.items.clear();
        self.hints.clear();
        self.tip = None;
        self.sel = 0;
        self.sel_from = None;
        self.icons.forget_pending();
        self.sub_overrides.clear();
        self.pin_msg = None;
        self.msg_exit = None;
    }

    // ---- settings ----

    fn load_cfg(&mut self, v: Option<&Value>) {
        let Some(v) = v else { return };
        self.cfg = Config::from_value(v);
        self.lang = Lang::from_setting(self.cfg.language.as_deref(), &self.os_locale);
        self.uninstall.set_lang(self.lang);
        self.sync_reduce();
        if self.visible {
            self.wave.start(self.now, self.reduce_motion(), !self.window_visible);
        }
    }

    // ---- replies ----

    fn on_reply(&mut self, ticket: u64, value: Option<Value>) {
        let Some(p) = self.pending.remove(&ticket) else { return };
        match p {
            Pending::Settings => self.load_cfg(value.as_ref()),
            Pending::Query { seq, empty } => {
                // a stale answer (the query moved on, the bar closed) or a failed invoke is dropped
                if seq != self.seq {
                    return;
                }
                if let Some(v) = value {
                    self.render(Res::list_from(&v), empty);
                }
            }
            Pending::Icon { id } => {
                let url = value.as_ref().and_then(Value::as_str).map(str::to_string);
                self.icons.put(&id, url);
            }
            Pending::Run { id, mode } => self.after_run(&id, mode, value),
            Pending::Ask { id } => {
                let r = AskResult::from_value(value.as_ref());
                self.finish_ask(id, r);
            }
            Pending::VoiceState => {
                let Some(v) = value else { return };
                let p = VoicePayload::from_value(&v);
                if p.state != "idle" {
                    return;
                }
                if p.disabled {
                    self.mic_msg(self.t(s::VOICE_OFF).to_string(), Some("voice"));
                    return;
                }
                self.mic_msg(String::new(), None);
                self.dict_here = true; // only a take started from this bar is typed into it
                self.call(BackendCall::voice_start(), Some(Pending::VoiceStart));
            }
            Pending::VoiceStart => {
                if value.as_ref().and_then(|v| v.get("ok")).and_then(Value::as_bool) == Some(false) {
                    self.dict_here = false; // the error itself arrives as a voice:state event
                    self.set_mic(MicState::Idle);
                }
            }
            Pending::Uninstall(tag) => {
                let o = self.uninstall.on_reply(tag, value.as_ref(), self.now);
                self.push_uninstall(o);
            }
        }
    }

    // ---- input and search ----

    fn input_changed(&mut self) {
        self.wave.kick();
        // the uninstall panel's capture handler runs first: typing steps the panel aside
        let o = self.uninstall.on_input(self.now);
        self.push_uninstall(o);
        self.debounce = Some(self.now + query::DEBOUNCE_MS);
    }

    fn clear_msg(&mut self) {
        self.msg = None;
        self.pin_msg = None;
        self.msg_exit = None;
    }
    fn set_error(&mut self, text: String, link: Option<&str>) {
        self.msg = Some(MessageView { text, link: link.map(str::to_string), ok: false });
        self.anims.push(anim::message_enter());
    }

    fn end_dialog(&mut self) {
        self.dialog.end();
    }
    fn clear_ans(&mut self) {
        self.answer = Answer::None;
        if self.dialog.abandon() {
            self.call(BackendCall::ask_cancel(), None);
        }
    }

    /// `search()`: the line in the box becomes either an ask row or a `cmd_query`.
    fn search(&mut self) {
        let text = self.input.trim_start().to_string();
        self.seq += 1;
        let my = self.seq;
        self.end_dialog();
        self.clear_ans();
        self.clear_msg();
        if let Input::Ask { question } = query::classify(&text) {
            self.words.clear();
            if question.is_empty() {
                self.render(Vec::new(), false);
                self.tip = Some(TipKind::Ask);
            } else {
                self.render(vec![Res::new(Kind::Ask, "ask:q", "", "", &question)], false);
            }
            return;
        }
        // leading blanks go, a trailing one is kept: "c " already selects the clipboard filter
        self.words = query::highlight_words(&text);
        let empty = text.trim().is_empty();
        self.call(BackendCall::query(&text), Some(Pending::Query { seq: my, empty }));
    }

    /// While a dialogue is open, typing is a follow-up question (no `? ` needed); a leading `>` or `=` ends it and searches as usual.
    fn dlg_input(&mut self) {
        let raw = self.input.trim_start().to_string();
        if query::leaves_dialogue(&raw) {
            return self.search();
        }
        self.seq += 1;
        if self.dialog.abandon() {
            self.call(BackendCall::ask_cancel(), None); // a new keystroke abandons a pending follow-up; the previous answer comes back
        }
        self.clear_msg();
        if !matches!(self.answer, Answer::Shown(_)) {
            self.answer = Answer::Shown(self.dialog.last_answer().to_string());
        }
        let q = query::follow_up(&raw);
        self.dlg_rows(&q);
    }

    /// Rows under the answer while the dialogue is open: the typed follow-up (if any), copy, new conversation.
    fn dlg_rows(&mut self, text: &str) {
        self.words.clear();
        let mut rows = Vec::new();
        if !text.is_empty() {
            rows.push(Res::new(Kind::Ask, "ask:q", "", "", text));
        }
        rows.push(Res::new(Kind::AskCopy, "ask:copy", "", "", self.dialog.last_answer()));
        rows.push(Res::new(Kind::NewConv, "ask:new", "", "", ""));
        self.render(rows, false);
    }

    /// `render()`: new rows replace the list; selection goes to row 0; later renders slide survivors and drop new rows in.
    fn render(&mut self, res: Vec<Res>, empty: bool) {
        let old: Vec<String> = self.items.iter().map(|r| r.id.clone()).collect();
        self.items = res.iter().filter(|r| r.kind != Kind::Hint).cloned().collect();
        self.hints = res.into_iter().filter(|r| r.kind == Kind::Hint).collect();
        self.sel = 0;
        self.sel_from = None; // a fresh render snaps back to the rows' own background
        self.confirm = None;
        self.copied = None;
        self.sub_overrides.clear();
        self.tip = (self.items.is_empty() && empty).then_some(TipKind::Help);
        let new: Vec<String> = self.items.iter().map(|r| r.id.clone()).collect();
        if self.fresh {
            for i in 0..new.len() {
                self.anims.push(anim::row_insert(i, true)); // the stagger plays once per opening, not on every keystroke
            }
        } else {
            let d = results::diff(&old, &new);
            for &i in &d.inserted {
                self.anims.push(anim::row_insert(i, false));
            }
            for &(i, _) in &d.moved {
                self.anims.push(anim::row_slide(i));
            }
            for &o in &d.removed {
                self.anims.push(anim::row_remove(o));
            }
        }
        self.fresh = false;
        self.uninstall.list_has_rows = !self.items.is_empty();
        // app / file / folder rows show the real shell icon; the glyph stays until it loads
        let ids: Vec<String> = self.items.iter().filter(|r| r.kind.is_pathy()).map(|r| r.id.clone()).collect();
        for id in ids {
            if self.icons.want(&id) {
                self.call(BackendCall::icon(&id), Some(Pending::Icon { id }));
            }
        }
    }

    // ---- selection ----

    fn select(&mut self, i: i64) {
        let Some(n) = results::wrap(i, self.items.len()) else { return };
        let prev = self.sel;
        self.sel = n;
        if prev != n && !self.anims.reduced() {
            self.sel_from = Some(prev);
            self.anims.push(anim::selection_glide(prev, n));
        }
        self.outs.push(Action::ScrollToRow(n));
    }
    fn select_abs(&mut self, i: usize) {
        if self.items.is_empty() {
            return;
        }
        let i = i.min(self.items.len() - 1);
        let prev = self.sel;
        self.sel = i;
        if prev != i && !self.anims.reduced() {
            self.sel_from = Some(prev);
            self.anims.push(anim::selection_glide(prev, i));
        }
        self.outs.push(Action::ScrollToRow(i));
    }

    // ---- keys ----

    fn on_key(&mut self, e: KeyEvent) {
        // the uninstall panel's capture handler sees every key first (Ctrl+U, Esc, typing, its own Enter / Tab / arrows)
        let (consumed, o) = self.uninstall.on_key(&e, self.now);
        self.push_uninstall(o);
        if consumed {
            return;
        }
        match intent_of(&e) {
            Intent::Escape => {
                if self.ctx.is_some() {
                    self.close_ctx();
                } else {
                    self.call(BackendCall::toggle(), None);
                }
            }
            Intent::Move(d) => self.select(self.sel as i64 + d as i64),
            Intent::Page(d) => {
                if let Some(t) = results::page(self.sel, d, self.items.len()) {
                    self.select_abs(t);
                }
            }
            Intent::Edge { last } => {
                if !self.items.is_empty() {
                    self.select_abs(if last { self.items.len() - 1 } else { 0 });
                }
            }
            Intent::Dictate => self.dictate(),
            Intent::Group(d) => {
                if let Some(t) = results::next_group(&self.items, self.sel, d) {
                    self.select_abs(t);
                }
            }
            Intent::Pin => {
                if self.items.get(self.sel).is_some_and(|r| r.kind.pinnable()) {
                    self.run(self.sel, RunMode::Pin);
                }
            }
            Intent::CopyPath => self.run(self.sel, RunMode::Path),
            Intent::Run(m) => self.run(self.sel, m),
            Intent::Pass => {}
        }
    }

    // ---- context menu ----

    fn close_ctx(&mut self) {
        self.ctx = None;
    }
    fn on_row_context(&mut self, index: usize, x: f32, y: f32) {
        let Some(it) = self.items.get(index).cloned() else { return self.close_ctx() };
        self.select(index as i64);
        let lines = results::context_lines(&it.kind, self.lang);
        if lines.is_empty() {
            return self.close_ctx();
        }
        let h = lines.len() as f32 * MENU_LINE + 2.0 * MENU_PAD;
        let (lx, ly, ox, oy) = results::context_position(x, y, MENU_W, h, WIN_W, WIN_H);
        self.anims.push(anim::ctx_open());
        for k in 0..lines.len() {
            self.anims.push(anim::ctx_item(k));
        }
        self.ctx = Some(ContextView { row: index, x: lx, y: ly, origin: (ox, oy), lines });
    }

    // ---- running a row ----

    fn copy_text(&mut self, text: String) {
        self.outs.push(Action::Copy(text));
    }

    fn run(&mut self, index: usize, mode: RunMode) {
        let Some(it) = self.items.get(index).cloned() else { return };
        if mode == RunMode::Uninstall && it.kind == Kind::App {
            // the row's trash button / menu line: put the uninstall trigger for this app in the box; its row (with the confirm step) shows right here
            let text = query::uninstall_query_for(&it.title);
            let caret = char_len(&text);
            self.input = text.clone();
            self.sel_start = caret;
            self.sel_end = caret;
            self.outs.push(Action::SetInput { text, caret });
            self.outs.push(Action::FocusInput);
            return self.search();
        }
        match it.kind {
            Kind::Ask => return self.ask(&it),
            Kind::Uninstall => {
                let o = self.uninstall.open(&it, self.now); // confirm card first: nothing is removed by this keystroke
                return self.push_uninstall(o);
            }
            Kind::NewConv => {
                self.debounce = None;
                self.input.clear();
                self.outs.push(Action::SetInput { text: String::new(), caret: 0 });
                return self.search(); // search() ends the dialogue
            }
            Kind::AskCopy => {
                self.copy_text(ask::plain_text(&it.payload)); // plain text, markdown markers stripped
                self.copied = Some((index, self.now + COPIED_MS));
                return;
            }
            Kind::Calc | Kind::Convert => self.copy_text(it.payload.clone()), // copy before Rust hides the window
            _ => {}
        }
        let mut mode = mode;
        if !matches!(it.kind, Kind::App | Kind::File | Kind::Folder | Kind::Shelf | Kind::Clip) {
            mode = RunMode::Open; // reveal / admin / copy-only only mean something for these
        } else if it.kind == Kind::Clip && mode == RunMode::Path {
            mode = RunMode::Copy; // "copy the path" of a clipboard item is its plain copy
        }
        self.call(BackendCall::run(&it.id, mode.as_str()), Some(Pending::Run { id: it.id, mode }));
    }

    fn after_run(&mut self, id: &str, mode: RunMode, value: Option<Value>) {
        let Some(v) = value else { return };
        let mut r = Outcome::from_value(&v);
        if mode == RunMode::Pin && r.status == Status::Updated {
            // Rust answers with a key: pinned / pinnedAlready
            r.message = self.t(s::message_key(&r.message).unwrap_or(s::PINNED)).to_string();
        }
        let row = self.items.iter().position(|x| x.id == id);
        self.confirm = None;
        match r.status {
            Status::Copy => {
                // clipboard item, Shift+Enter: copy only
                self.copy_text(r.message);
                self.call(BackendCall::toggle(), None);
            }
            Status::Updated => {
                // a setting or action switched: show the new state, stay open
                let has_sub = row.and_then(|i| self.items.get(i)).map(|it| results::parts(it, self.lang, &self.ask_state())).is_some_and(|p| !p.sub.is_empty());
                match row {
                    Some(i) if has_sub && self.items[i].kind != Kind::App => {
                        self.sub_overrides.insert(i, r.message.clone());
                    }
                    _ => {
                        self.msg = Some(MessageView { text: r.message.clone(), link: None, ok: false });
                    }
                }
                if mode == RunMode::Pin {
                    self.pin_feedback(row, r.message);
                }
            }
            Status::Confirm => {
                if let Some(i) = row {
                    self.confirm = Some(Confirm { row: i, until: self.now + CONFIRM_MS });
                }
            }
            Status::Error => self.set_error(r.message, None),
            Status::Ok | Status::Other(_) => {}
        }
    }

    /// Pin confirmation: the pin icon pops with a spring and turns into place, the row flashes once, the message slides in and fades out again.
    fn pin_feedback(&mut self, row: Option<usize>, text: String) {
        if let Some(i) = row {
            self.anims.push(anim::pin_pop(i));
            self.anims.push(anim::row_flash(i));
        }
        if self.msg.as_ref().is_some_and(|m| !m.text.is_empty()) {
            if let Some(m) = self.msg.as_mut() {
                m.ok = true; // a confirmation, not an error: green instead of the error red
            }
            self.anims.push(anim::message_enter());
            self.pin_msg = Some((self.now + PIN_MSG_MS, text));
        }
    }

    // ---- AI ----

    fn ask(&mut self, it: &Res) {
        if self.dialog.asking() {
            return;
        }
        if !self.cfg.ask_ready() {
            self.call(BackendCall::settings_open("ai"), None);
            return;
        }
        let seq = self.seq;
        let (id, call) = match self.dialog.begin(&it.payload, seq) {
            Some(f) => (f.id, BackendCall::ask(&f.question, &f.history)),
            None => return,
        };
        self.msg = None;
        self.items.clear();
        self.hints.clear();
        self.tip = None;
        self.answer = Answer::Thinking { partial: String::new() };
        self.call(call, Some(Pending::Ask { id }));
    }

    fn finish_ask(&mut self, id: u64, result: AskResult) {
        let seq = self.seq;
        let question = self.dialog.inflight().map(|f| f.question.clone()).unwrap_or_default();
        match self.dialog.finish(id, &result, seq, self.lang) {
            Finished::Stale => {}
            Finished::Answered(text) => {
                self.input.clear();
                self.sel_start = 0;
                self.sel_end = 0;
                self.outs.push(Action::SetInput { text: String::new(), caret: 0 });
                self.answer = Answer::Shown(text);
                self.dlg_rows("");
            }
            Finished::Failed(err, in_dialog) => {
                self.answer = if in_dialog { Answer::Shown(self.dialog.last_answer().to_string()) } else { Answer::None };
                if !err.silent {
                    self.set_error(err.text, err.settings_page);
                }
                let again = Res::new(Kind::Ask, "ask:q", "", "", &question);
                if in_dialog {
                    self.dlg_rows(&question);
                } else {
                    self.render(vec![again], false);
                }
            }
        }
    }

    // ---- dictation ----

    fn set_mic(&mut self, st: MicState) {
        self.mic = st;
    }
    fn mic_msg(&mut self, text: String, page: Option<&str>) {
        self.msg = if text.is_empty() { None } else { Some(MessageView { text, link: page.map(str::to_string), ok: false }) };
    }
    fn dictate(&mut self) {
        match self.mic {
            MicState::Recording => self.call(BackendCall::voice_stop(), None),
            MicState::Idle => self.call(BackendCall::voice_state(), Some(Pending::VoiceState)),
            MicState::Transcribing => {}
        }
    }

    fn on_voice(&mut self, p: &VoicePayload) {
        if !self.dict_here {
            return;
        }
        match p.state.as_str() {
            "recording" => self.set_mic(MicState::Recording),
            "transcribing" => self.set_mic(MicState::Transcribing),
            "done" => {
                self.dict_here = false;
                self.set_mic(MicState::Idle);
                let text = p.text.trim();
                if text.is_empty() {
                    return;
                }
                let a = self.sel_start.min(char_len(&self.input));
                let b = self.sel_end.clamp(a, char_len(&self.input));
                let pre = char_slice(&self.input, 0, a);
                let sp = if !pre.is_empty() && !pre.ends_with(char::is_whitespace) { " " } else { "" };
                let end = a + sp.len() + char_len(text);
                let new = format!("{pre}{sp}{text}{}", char_slice(&self.input, b, char_len(&self.input)));
                self.input = new.clone();
                self.sel_start = end;
                self.sel_end = end;
                self.outs.push(Action::SetInput { text: new, caret: end });
                self.outs.push(Action::FocusInput);
                self.input_changed();
            }
            "error" => {
                if p.code == "busy" {
                    return;
                }
                self.dict_here = false;
                self.set_mic(MicState::Idle);
                match s::voice_error(&p.code) {
                    Some(s::VoiceErr::NotSetUp) => self.mic_msg(self.t(s::VOICE_OFF).to_string(), Some("voice")),
                    Some(s::VoiceErr::Text(t)) => self.mic_msg(t.tr(self.lang).to_string(), None),
                    None => {
                        let detail = if p.message.is_empty() { String::new() } else { format!(": {}", p.message.chars().take(160).collect::<String>()) };
                        self.mic_msg(format!("{}{detail}", self.t(s::VOICE_FAIL)), None);
                    }
                }
            }
            "idle" => {
                self.dict_here = false;
                self.set_mic(MicState::Idle); // cancelled (Esc)
            }
            _ => {}
        }
    }

    // ---- view ----

    pub fn view(&self) -> BarView {
        let lang = self.lang;
        let ask = self.ask_state();
        let rows: Vec<RowView> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let mut r = results::row_view(it, i, lang, &ask, &self.words);
                r.selected = i == self.sel;
                r.shell_icon = self.icons.get(&it.id).map(str::to_string);
                if let Some(o) = self.sub_overrides.get(&i) {
                    r.sub = o.clone();
                    r.show_sub = true;
                }
                if self.confirm.as_ref().is_some_and(|c| c.row == i) {
                    r.confirm = true;
                    r.label = self.t(s::AGAIN).to_string();
                }
                if self.copied.is_some_and(|c| c.0 == i) {
                    r.label = self.t(s::COPIED).to_string();
                }
                r
            })
            .collect();
        let hints = self
            .hints
            .iter()
            .map(|h| {
                let p = results::parts(h, lang, &ask);
                format!("{}{}", h.title, if p.sub.is_empty() { String::new() } else { format!(" - {}", p.sub) })
            })
            .collect();
        let answer = match &self.answer {
            Answer::None => None,
            Answer::Thinking { partial } => Some(AnswerView { thinking: true, thinking_label: self.t(s::THINK).to_string(), text: partial.clone(), copy_code_label: self.t(s::COPY_CODE).to_string(), code_copied: None }),
            Answer::Shown(t) => Some(AnswerView { thinking: false, thinking_label: String::new(), text: t.clone(), copy_code_label: self.t(s::COPY_CODE).to_string(), code_copied: self.code_copied.map(|c| c.0) }),
        };
        BarView {
            visible: self.visible,
            parked: self.parked,
            placeholder: self.t(if self.dialog.is_open() { s::DLG_PH } else { s::PH }).to_string(),
            hotkey: self.cfg.hotkey.clone(),
            input: self.input.clone(),
            theme: self.cfg.theme.clone(),
            reduce_motion: self.reduce_motion(),
            mic: MicView {
                state: self.mic,
                tooltip: self.t(match self.mic {
                    MicState::Recording => s::MIC_STOP,
                    MicState::Transcribing => s::MIC_BUSY,
                    MicState::Idle => s::MIC_TIP,
                })
                .to_string(),
            },
            dialog_hint: self.dialog.is_open().then(|| self.t(s::DLG).to_string()),
            answer,
            tip: self.tip.map(|k| self.t(if k == TipKind::Ask { s::ASK_TIP } else { s::TIP }).to_string()),
            rows,
            hints,
            selected: self.sel,
            selection_from: self.sel_from,
            message: self.msg.clone(),
            context: self.ctx.clone(),
            uninstall: self.uninstall.view(),
        }
    }
}

#[cfg(test)]
mod tests;
