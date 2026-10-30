//! The state machine of one capture overlay window (`capture-N`): a line-by-line port of src/capture.js with its I/O moved behind
//! [`CaptureIo`] and its DOM replaced by [`CaptureView`].
//!
//! Selection phases: `Idle` (armed, no selection) -> `Dragging` (new / move / resize / draw) -> `Adjusting` (handles, toolbar,
//! markup) -> `Committed` (a `capture_commit` / `capture_cancel` went out; inputs are ignored until the next `Begin`).
//! One model per monitor; the backend ends the session for all of them when any one commits or cancels, and every window then
//! receives `capture:end` ([`Input::End`]).
use super::anim::{AnimKind, AnimSet, AnimTarget};
use super::geom::{self, Cursor, Handle, Hit, IRect, Pt, Rect, Scale, HANDLES, MIN_SELECTION};
use super::input::{Action, AfterSave, Button, Input, Key, Mode, Mods, Options, Phase, Reaction};
use super::io::{BackendCall, CaptureIo, CommitArgs, CommitKind, FlattenRequest, SessionInfo};
use super::loupe::Loupe;
use super::markup::{History, Op, OpKind, Tool, COLORS};
use super::toolbar::{self, ItemId, Toolbar, ToolbarState};
use super::view::{CaptureView, ClipInset, Ghost, HandleVm, HintVm, OpVm, SelectionVm, TextVm};
use crate::i18n::Tr;
use std::sync::Arc;

/// Result of [`CaptureModel::arm`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmOutcome {
    /// Session loaded, frame received, `capture_ready` sent.
    Armed,
    /// `capture_session` answered nothing for this window.
    NoSession,
    /// The same session id as the previous one (a repeated `capture:begin`).
    Duplicate,
    /// The frame is not `w*h*4` bytes: the session was cancelled.
    FrameMismatch,
    /// `arm` re-entered while a previous one is still running.
    Busy,
}

#[derive(Clone, Debug)]
enum Drag {
    New { p: Pt },
    Move { o: Rect, p: Pt },
    Resize { h: Handle, o: Rect },
    Draw,
    MoveOp { id: u32, last: Pt, moved: bool },
}

#[derive(Clone, Debug)]
struct TextEdit {
    pos: Pt,
    color: String,
    text: String,
    caret: usize,
}

/// One overlay.
pub struct CaptureModel {
    label: String,
    opts: Options,
    // session
    last_id: u64,
    id: u64,
    mode: Mode,
    fw: i32,
    fh: i32,
    vw: f64,
    vh: f64,
    frame: Option<Arc<Vec<u8>>>,
    arming: bool,
    committed: bool,
    busy: bool,
    // selection + markup
    sel: Option<Rect>,
    tool: Tool,
    color: usize,
    history: History,
    draft: Option<Op>,
    drag: Option<Drag>,
    text: Option<TextEdit>,
    selected_op: Option<u32>,
    after_save: Option<AfterSave>,
    // chrome
    toolbar: Toolbar,
    bar_visible: bool,
    bar_origin: Pt,
    bar_press: Option<ItemId>,
    hover: Option<String>,
    pressed: Option<String>,
    hint_visible: bool,
    pointer: Pt,
    // colour mode
    loupe: Option<Loupe>,
    loupe_from: Option<Pt>,
    // animation
    anims: AnimSet,
    now: u64,
    ghost_sel: Option<Rect>,
    ghost_bar: Option<Pt>,
}

fn parse_session(v: &serde_json::Value) -> Option<SessionInfo> {
    SessionInfo::from_value(v).filter(|i| i.id != 0 || i.w > 0)
}

impl CaptureModel {
    /// A model for the overlay window `label` whose viewport is `vw x vh` css px.
    pub fn new(label: &str, vw: f64, vh: f64, opts: Options) -> CaptureModel {
        let toolbar = Toolbar::new(opts.extended_tools, opts.ext_actions);
        CaptureModel {
            label: label.to_string(),
            opts,
            last_id: 0,
            id: 0,
            mode: Mode::Area,
            fw: 0,
            fh: 0,
            vw,
            vh,
            frame: None,
            arming: false,
            committed: false,
            busy: false,
            sel: None,
            tool: Tool::Select,
            color: 0,
            history: History::new(),
            draft: None,
            drag: None,
            text: None,
            selected_op: None,
            after_save: None,
            toolbar,
            bar_visible: false,
            bar_origin: Pt::default(),
            bar_press: None,
            hover: None,
            pressed: None,
            hint_visible: false,
            pointer: Pt::default(),
            loupe: None,
            loupe_from: None,
            anims: AnimSet::new(),
            now: 0,
            ghost_sel: None,
            ghost_bar: None,
        }
    }

    // ------------------------------------------------------------------------------------------- accessors
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn session_id(&self) -> u64 {
        self.id
    }
    pub fn selection(&self) -> Option<Rect> {
        self.sel
    }
    pub fn tool(&self) -> Tool {
        self.tool
    }
    pub fn color(&self) -> &'static str {
        COLORS[self.color % COLORS.len()]
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn ops(&self) -> &[Op] {
        self.history.ops()
    }
    pub fn options(&self) -> &Options {
        &self.opts
    }
    pub fn viewport(&self) -> (f64, f64) {
        (self.vw, self.vh)
    }
    pub fn frame_size(&self) -> (i32, i32) {
        (self.fw, self.fh)
    }
    /// The frozen RGBA frame (`w*h*4`), kept until the session ends so the renderer can paint it once.
    pub fn frame(&self) -> Option<&Arc<Vec<u8>>> {
        self.frame.as_ref()
    }
    /// css -> frame scale per axis (`rx()` / `ry()`).
    pub fn scale(&self) -> Scale {
        if self.fw > 0 && self.fh > 0 {
            Scale::new(self.fw, self.fh, self.vw, self.vh)
        } else {
            Scale::IDENTITY
        }
    }
    pub fn phase(&self) -> Phase {
        if self.committed {
            Phase::Committed
        } else if self.drag.is_some() {
            Phase::Dragging
        } else if self.sel.is_some() {
            Phase::Adjusting
        } else {
            Phase::Idle
        }
    }
    /// Follow-up for the host after a pin / shelf commit: act on the saved path when `capture:done` arrives.
    pub fn take_after_save(&mut self) -> Option<AfterSave> {
        self.after_save.take()
    }
    /// Text currently typed in the text tool, if open.
    pub fn text_input(&self) -> Option<&str> {
        self.text.as_ref().map(|t| t.text.as_str())
    }
    pub fn animating(&self) -> bool {
        self.anims.is_running()
    }

    fn anim(&mut self, kind: AnimKind, target: AnimTarget) {
        self.anims.start(kind, target, self.now, self.opts.reduce_motion);
    }

    // ------------------------------------------------------------------------------------------- session
    /// `arm()` of the page: fetch the session and the frame, validate the size, show the hint (and the loupe in colour mode),
    /// let the first frame paint, tell the backend this window is ready, take the keyboard focus.
    pub fn arm(&mut self, io: &mut dyn CaptureIo) -> ArmOutcome {
        if self.arming {
            return ArmOutcome::Busy;
        }
        self.arming = true;
        self.now = io.now_ms();
        let r = self.arm_inner(io);
        self.arming = false;
        r
    }

    fn arm_inner(&mut self, io: &mut dyn CaptureIo) -> ArmOutcome {
        let Some(info) = io.call(&BackendCall::session(&self.label)).as_ref().and_then(parse_session) else {
            return ArmOutcome::NoSession;
        };
        if info.id == self.last_id {
            return ArmOutcome::Duplicate;
        }
        self.clear_all();
        self.committed = false;
        self.last_id = info.id;
        self.id = info.id;
        self.mode = Mode::parse(&info.mode).unwrap_or(Mode::Area);
        self.fw = info.w;
        self.fh = info.h;
        let bytes = io.frame(&self.label).unwrap_or_default();
        if bytes.len() as i64 != self.fw as i64 * self.fh as i64 * 4 {
            io.call(&BackendCall::cancel());
            self.id = 0;
            return ArmOutcome::FrameMismatch;
        }
        let frame = Arc::new(bytes);
        if self.mode == Mode::Color {
            self.loupe = Some(Loupe::new(self.fw, self.fh, frame.clone()));
            self.anim(AnimKind::LoupeShow, AnimTarget::Loupe);
        }
        self.frame = Some(frame);
        self.hint_visible = self.mode.has_hint();
        io.yield_to_paint();
        io.call(&BackendCall::ready(&self.label));
        io.refocus();
        ArmOutcome::Armed
    }

    /// `clearAll()`: forget the session (the page keeps mode / frame size and the colour choice).
    fn clear_all(&mut self) {
        self.drag = None;
        self.text = None;
        self.id = 0;
        self.ghost_sel = self.ghost_sel.or(self.sel);
        self.sel = None;
        self.history.clear();
        self.draft = None;
        self.frame = None;
        self.loupe = None;
        self.loupe_from = None;
        self.busy = false;
        self.tool = Tool::Select;
        self.selected_op = None;
        self.bar_press = None;
        self.hover = None;
        self.pressed = None;
        if self.bar_visible {
            self.ghost_bar = Some(self.bar_origin);
        }
        self.bar_visible = false;
    }

    /// Close this overlay after a commit / cancel: clear and refuse input until the next `Begin`.
    fn finish(&mut self) {
        self.clear_all();
        self.committed = true;
    }

    // ------------------------------------------------------------------------------------------- input
    /// Feed one event. Returns what happened for the host (redraw, swallow the key, which actions ran).
    pub fn on_input(&mut self, ev: Input, io: &mut dyn CaptureIo) -> Reaction {
        self.now = io.now_ms();
        let mut r = Reaction::default();
        match ev {
            Input::Begin => {
                self.arm(io);
                r.redraw = true;
            }
            Input::End => {
                self.finish();
                self.committed = false; // `capture:end` is a reset, the next Begin arms again
                r.redraw = true;
            }
            _ if self.committed => {}
            Input::PointerDown { pos, button, mods } => self.pointer_down(pos, button, mods, io, &mut r),
            Input::PointerMove { pos, mods } => self.pointer_move(pos, mods, &mut r),
            Input::PointerUp { pos, button: _, mods: _ } => self.pointer_up(pos, io, &mut r),
            Input::DoubleClick { pos } => {
                let p = geom::clamp_pt(pos, self.vw, self.vh);
                if self.mode == Mode::Area && self.tool == Tool::Select && self.sel.is_some_and(|s| s.contains(p)) && !self.over_bar(p) {
                    self.do_action(Action::Commit(CommitKind::Both), io, &mut r);
                }
            }
            Input::ContextMenu => {
                r.prevent_default = true;
                self.do_action(Action::Cancel, io, &mut r);
            }
            Input::Key { key, mods } => self.key(key, mods, io, &mut r),
            Input::Text(s) => {
                if self.text.is_some() {
                    self.text_insert(&s);
                    r.redraw = true;
                }
            }
            Input::Resize { vw, vh } => {
                self.vw = vw;
                self.vh = vh;
                if self.mode == Mode::Area && self.bar_visible {
                    self.place_bar();
                }
                r.redraw = true;
            }
        }
        r
    }

    fn over_bar(&self, p: Pt) -> bool {
        self.bar_visible && self.bar_rect().contains(p)
    }

    fn bar_rect(&self) -> Rect {
        Rect::new(self.bar_origin.x, self.bar_origin.y, self.toolbar.width, self.toolbar.height)
    }

    fn bar_item_at(&self, p: Pt) -> Option<ItemId> {
        if !self.over_bar(p) {
            return None;
        }
        self.toolbar.item_at(Pt::new(p.x - self.bar_origin.x, p.y - self.bar_origin.y)).cloned()
    }

    fn item_disabled(&self, it: &ItemId) -> bool {
        match it {
            ItemId::Undo => !self.history.can_undo() || self.busy,
            ItemId::Redo => !self.history.can_redo() || self.busy,
            ItemId::Swatch(_) | ItemId::Tool(_) | ItemId::Sep => false,
            _ => self.busy,
        }
    }

    fn pointer_down(&mut self, pos: Pt, button: Button, _mods: Mods, io: &mut dyn CaptureIo, r: &mut Reaction) {
        if button == Button::Right {
            r.prevent_default = true; // right click cancels, like Esc
            self.do_action(Action::Cancel, io, r);
            return;
        }
        io.refocus(); // a click always gives the page the keyboard
        if button != Button::Left {
            return;
        }
        let p = geom::clamp_pt(pos, self.vw, self.vh);
        self.pointer = p;
        if let Some(it) = self.bar_item_at(p) {
            // toolbar buttons act on pointer-up over the same button (a click); the press blurs the text box, which commits it
            self.commit_text();
            if !self.item_disabled(&it) {
                self.pressed = Some(it.id());
                self.bar_press = Some(it);
                self.anim(AnimKind::ButtonPress, AnimTarget::ToolbarItem(self.pressed.clone().unwrap_or_default()));
            }
            r.redraw = true;
            return;
        }
        if self.over_bar(p) {
            return; // padding of the bar: swallowed
        }
        self.commit_text(); // the text input loses focus
        if self.hint_visible {
            self.hint_visible = false;
            self.anim(AnimKind::HintFade, AnimTarget::Hint);
        }
        if self.mode == Mode::Color {
            self.set_sample_css(pos);
            self.pick_commit(io, r);
            return;
        }
        if self.mode == Mode::Area {
            if let Some(sel) = self.sel {
                match geom::hit(&sel, p) {
                    Hit::Inside if self.tool != Tool::Select => {
                        self.start_draw(p);
                        r.redraw = true;
                        return;
                    }
                    Hit::Edge(h) => {
                        self.drag = Some(Drag::Resize { h, o: sel });
                        r.redraw = true;
                        return;
                    }
                    Hit::Inside => {
                        if self.opts.edit_ops {
                            let fp = self.scale().to_frame(p);
                            let tol = 4.0 * self.scale().rx;
                            let hitid = self.history.hit_test(fp, tol, self.scale());
                            self.selected_op = hitid;
                            if let Some(id) = hitid {
                                self.history.begin_edit();
                                self.drag = Some(Drag::MoveOp { id, last: fp, moved: false });
                                r.redraw = true;
                                return;
                            }
                        }
                        self.drag = Some(Drag::Move { o: sel, p });
                        r.redraw = true;
                        return;
                    }
                    Hit::Outside => {}
                }
            }
        }
        // a fresh selection
        self.history.clear();
        self.draft = None;
        self.selected_op = None;
        self.text = None;
        self.hide_bar();
        self.drag = Some(Drag::New { p });
        self.sel = Some(Rect::new(p.x, p.y, 0.0, 0.0));
        r.redraw = true;
    }

    fn pointer_move(&mut self, pos: Pt, mods: Mods, r: &mut Reaction) {
        let new_hover = self.bar_item_at(pos).map(|i| i.id());
        if new_hover != self.hover {
            self.hover = new_hover;
            r.redraw = true;
        }
        if self.mode == Mode::Color {
            self.set_sample_css(pos);
            r.redraw = true;
            return;
        }
        let p = geom::clamp_pt(pos, self.vw, self.vh);
        self.pointer = p;
        let Some(drag) = self.drag.clone() else {
            r.redraw = true; // cursor may change
            return;
        };
        let snap = self.opts.snap.clone();
        let ratio = if self.opts.shift_square && mods.shift { Some(1.0) } else { self.opts.aspect };
        match drag {
            Drag::New { p: a } => {
                let mut q = p;
                if let Some(s) = &snap {
                    q = s.snap_point(q, self.vw, self.vh);
                }
                if let Some(k) = ratio {
                    q = geom::clamp_pt(geom::lock_ratio(a, q, k), self.vw, self.vh);
                }
                self.sel = Some(Rect::from_corners(a, q));
            }
            Drag::Move { o, p: g } => {
                let mut s = geom::move_selection(&o, g, p, self.vw, self.vh);
                if let Some(sn) = &snap {
                    s = sn.snap_move(&s, self.vw, self.vh).clamp_move(self.vw, self.vh);
                }
                self.sel = Some(s);
            }
            Drag::Resize { h, o } => {
                let mut q = p;
                if let Some(s) = &snap {
                    q = s.snap_point(q, self.vw, self.vh);
                }
                let s = match ratio {
                    Some(k) => geom::resize_locked(&o, h, q, k),
                    None => geom::resize_selection(&o, h, q),
                };
                self.sel = Some(s);
            }
            Drag::Draw => self.extend_draw(p),
            Drag::MoveOp { id, last, moved } => {
                let fp = self.scale().to_frame(p);
                let (dx, dy) = (fp.x - last.x, fp.y - last.y);
                if dx != 0.0 || dy != 0.0 {
                    self.history.move_op_live(id, dx, dy);
                    self.drag = Some(Drag::MoveOp { id, last: fp, moved: moved || dx.abs() + dy.abs() > 0.0 });
                }
            }
        }
        r.redraw = true;
    }

    fn pointer_up(&mut self, pos: Pt, io: &mut dyn CaptureIo, r: &mut Reaction) {
        if let Some(pressed) = self.bar_press.take() {
            self.pressed = None;
            let over = self.bar_item_at(pos);
            if over.as_ref() == Some(&pressed) && !self.item_disabled(&pressed) {
                self.activate(&pressed, io, r);
            }
            r.redraw = true;
            return;
        }
        let Some(d) = self.drag.take() else { return };
        r.redraw = true;
        match d {
            Drag::Draw => self.finish_draw(),
            Drag::New { .. } => {
                let valid = self.sel.is_some_and(|s| s.w >= MIN_SELECTION && s.h >= MIN_SELECTION);
                if !valid {
                    self.sel = None;
                    return;
                }
                if self.mode == Mode::Area {
                    self.show_bar();
                    self.anim(AnimKind::HandlesPop, AnimTarget::Handles);
                } else {
                    self.do_action(Action::Commit(CommitKind::Run), io, r);
                }
            }
            Drag::MoveOp { moved, .. } => {
                if !moved {
                    self.history.cancel_edit();
                }
            }
            Drag::Move { .. } | Drag::Resize { .. } => self.show_bar(),
        }
    }

    fn key(&mut self, key: Key, mods: Mods, io: &mut dyn CaptureIo, r: &mut Reaction) {
        // the text input swallows its own keys
        if self.text.is_some() {
            r.prevent_default = true;
            match key {
                Key::Enter => self.commit_text(),
                Key::Escape => self.text = None,
                Key::Backspace => self.text_backspace(),
                Key::Delete => self.text_delete(),
                Key::Left => self.text_move(-1),
                Key::Right => self.text_move(1),
                Key::Home => self.text_caret(0),
                Key::End => self.text_caret(usize::MAX),
                Key::Char(c) if !mods.command() => self.text_insert(&c.to_string()),
                _ => r.prevent_default = false,
            }
            r.redraw = true;
            return;
        }
        if key == Key::Escape {
            r.prevent_default = true;
            self.do_action(Action::Cancel, io, r);
            return;
        }
        if self.mode == Mode::Color {
            let (dx, dy) = match key {
                Key::Left => (-1, 0),
                Key::Right => (1, 0),
                Key::Up => (0, -1),
                Key::Down => (0, 1),
                Key::Enter => {
                    self.pick_commit(io, r);
                    return;
                }
                _ => return,
            };
            let (sc, vw, vh) = (self.scale(), self.vw, self.vh);
            if let Some(l) = self.loupe.as_mut() {
                let before = l.lens_pos(sc, vw, vh);
                l.step(dx, dy, mods.shift);
                self.loupe_from = Some(before);
            }
            self.anim(AnimKind::LoupeFollow, AnimTarget::Loupe);
            r.prevent_default = true;
            r.redraw = true;
            return;
        }
        if key == Key::Enter {
            r.prevent_default = true;
            let k = if self.mode == Mode::Area { CommitKind::Both } else { CommitKind::Run };
            self.do_action(Action::Commit(k), io, r);
            return;
        }
        if self.mode != Mode::Area {
            return;
        }
        if mods.command() {
            if let Key::Char(c) = key {
                match c.to_ascii_lowercase() {
                    'z' if mods.shift && self.opts.extended_tools => {
                        r.prevent_default = true;
                        self.do_action(Action::Redo, io, r);
                    }
                    'z' => {
                        r.prevent_default = true;
                        self.do_action(Action::Undo, io, r);
                    }
                    'y' if self.opts.extended_tools => {
                        r.prevent_default = true;
                        self.do_action(Action::Redo, io, r);
                    }
                    'c' if self.sel.is_some() => {
                        r.prevent_default = true;
                        self.do_action(Action::Commit(CommitKind::Copy), io, r);
                    }
                    's' if self.sel.is_some() => {
                        r.prevent_default = true;
                        self.do_action(Action::Commit(CommitKind::Save), io, r);
                    }
                    _ => {}
                }
                return;
            }
        }
        if self.sel.is_none() {
            return;
        }
        if self.opts.nudge {
            let dir = match key {
                Key::Left => Some((-1, 0)),
                Key::Right => Some((1, 0)),
                Key::Up => Some((0, -1)),
                Key::Down => Some((0, 1)),
                _ => None,
            };
            if let Some((dx, dy)) = dir {
                r.prevent_default = true;
                let a = if mods.ctrl { Action::NudgeSize { dw: dx, dh: dy, big: mods.shift } } else { Action::Nudge { dx, dy, big: mods.shift } };
                self.do_action(a, io, r);
                return;
            }
        }
        if self.opts.edit_ops && matches!(key, Key::Delete | Key::Backspace) && self.selected_op.is_some() {
            r.prevent_default = true;
            self.do_action(Action::DeleteOp, io, r);
        }
    }

    fn set_sample_css(&mut self, pos: Pt) {
        let sc = self.scale();
        let (vw, vh) = (self.vw, self.vh);
        if let Some(l) = self.loupe.as_mut() {
            let before = l.lens_pos(sc, vw, vh);
            l.set_pos(pos.x * sc.rx, pos.y * sc.ry);
            self.loupe_from = Some(before);
        }
        self.anim(AnimKind::LoupeFollow, AnimTarget::Loupe);
    }

    // ------------------------------------------------------------------------------------------- toolbar
    fn activate(&mut self, it: &ItemId, io: &mut dyn CaptureIo, r: &mut Reaction) {
        let a = match it {
            ItemId::Tool(t) => Action::SetTool(*t),
            ItemId::Swatch(i) => Action::SetColor(*i),
            ItemId::Undo => Action::Undo,
            ItemId::Redo => Action::Redo,
            ItemId::Copy => Action::Commit(CommitKind::Copy),
            ItemId::Save => Action::Commit(CommitKind::Save),
            ItemId::Record => Action::Commit(CommitKind::Record),
            ItemId::Both => Action::Commit(CommitKind::Both),
            ItemId::Cancel => Action::Cancel,
            ItemId::Pin => Action::PinToScreen,
            ItemId::Shelf => Action::AddToShelf,
            ItemId::Sep => return,
        };
        self.do_action(a, io, r);
    }

    fn show_bar(&mut self) {
        if self.mode != Mode::Area || self.sel.is_none() {
            return;
        }
        let was = self.bar_visible;
        self.place_bar();
        self.bar_visible = true;
        if !was {
            self.anims.stop(AnimKind::ToolbarLeave);
            self.anim(AnimKind::ToolbarEnter, AnimTarget::Toolbar);
        }
    }

    fn place_bar(&mut self) {
        if let Some(s) = self.sel {
            self.bar_origin = toolbar::position(&s, self.toolbar.width, self.toolbar.height, self.vw, self.vh);
        }
    }

    fn hide_bar(&mut self) {
        if self.bar_visible {
            self.bar_visible = false;
            self.ghost_bar = Some(self.bar_origin);
            self.ghost_sel = self.sel;
            self.anim(AnimKind::ToolbarLeave, AnimTarget::Toolbar);
        }
    }

    // ------------------------------------------------------------------------------------------- markup
    fn current_color(&self) -> String {
        COLORS[self.color % COLORS.len()].to_string()
    }

    fn start_draw(&mut self, p: Pt) {
        let fp = self.scale().to_frame(p);
        match self.tool {
            Tool::Text => self.open_text(p),
            Tool::Step => {
                let n = self.history.next_step();
                let id = self.history.fresh_id();
                self.history.push(Op { id, color: self.current_color(), kind: OpKind::Step { n, c: fp } });
                self.anim(AnimKind::OpInsert, AnimTarget::Op(id));
            }
            Tool::Select => {}
            t => {
                let id = self.history.fresh_id();
                self.draft = Op::draft(id, t, &self.current_color(), fp);
                if self.draft.is_some() {
                    self.drag = Some(Drag::Draw);
                }
            }
        }
    }

    fn extend_draw(&mut self, p: Pt) {
        let (Some(s), true) = (self.sel, self.draft.is_some()) else { return };
        let q = Pt::new(p.x.max(s.x).min(s.right()), p.y.max(s.y).min(s.bottom()));
        let fq = self.scale().to_frame(q);
        if let Some(d) = self.draft.as_mut() {
            d.extend(fq);
        }
    }

    fn finish_draw(&mut self) {
        if let Some(d) = self.draft.take() {
            if d.is_substantial() {
                let id = d.id;
                self.history.push(d);
                self.anim(AnimKind::OpInsert, AnimTarget::Op(id));
            }
        }
    }

    fn open_text(&mut self, p: Pt) {
        self.text = Some(TextEdit { pos: p, color: self.current_color(), text: String::new(), caret: 0 });
    }

    fn commit_text(&mut self) {
        let Some(t) = self.text.take() else { return };
        let s = t.text.trim().to_string();
        if !s.is_empty() {
            let id = self.history.fresh_id();
            let at = self.scale().to_frame(t.pos);
            self.history.push(Op { id, color: t.color, kind: OpKind::Text { s, a: at } });
            self.anim(AnimKind::OpInsert, AnimTarget::Op(id));
        }
    }

    fn byte_of(s: &str, ch: usize) -> usize {
        s.char_indices().nth(ch).map_or(s.len(), |(i, _)| i)
    }

    fn text_insert(&mut self, s: &str) {
        if let Some(t) = self.text.as_mut() {
            let clean: String = s.chars().filter(|c| !c.is_control()).collect();
            let at = Self::byte_of(&t.text, t.caret);
            t.text.insert_str(at, &clean);
            t.caret += clean.chars().count();
        }
    }

    fn text_backspace(&mut self) {
        if let Some(t) = self.text.as_mut() {
            if t.caret > 0 {
                let at = Self::byte_of(&t.text, t.caret - 1);
                t.text.remove(at);
                t.caret -= 1;
            }
        }
    }

    fn text_delete(&mut self) {
        if let Some(t) = self.text.as_mut() {
            if t.caret < t.text.chars().count() {
                let at = Self::byte_of(&t.text, t.caret);
                t.text.remove(at);
            }
        }
    }

    fn text_move(&mut self, d: i32) {
        if let Some(t) = self.text.as_mut() {
            let n = t.text.chars().count() as i32;
            t.caret = (t.caret as i32 + d).clamp(0, n) as usize;
        }
    }

    fn text_caret(&mut self, to: usize) {
        if let Some(t) = self.text.as_mut() {
            t.caret = to.min(t.text.chars().count());
        }
    }

    // ------------------------------------------------------------------------------------------- actions
    fn do_action(&mut self, a: Action, io: &mut dyn CaptureIo, r: &mut Reaction) {
        r.actions.push(a.clone());
        r.redraw = true;
        self.apply(a, io);
    }

    /// Perform one action (what a toolbar click, a shortcut or the host's own UI asks for).
    pub fn apply(&mut self, a: Action, io: &mut dyn CaptureIo) {
        self.now = io.now_ms();
        if self.committed {
            return;
        }
        match a {
            Action::SetTool(t) => {
                if t.is_core() || self.opts.extended_tools {
                    self.tool = t;
                    self.text = None; // switching tools drops an unfinished text, like closeText()
                    self.selected_op = None;
                }
            }
            Action::SetColor(i) => {
                if i < COLORS.len() {
                    self.color = i;
                    self.anim(AnimKind::SwatchPop, AnimTarget::ToolbarItem(COLORS[i].to_string()));
                }
            }
            Action::Undo => {
                self.history.undo();
                self.selected_op = None;
            }
            Action::Redo => {
                self.history.redo();
            }
            Action::DeleteOp => {
                if let Some(id) = self.selected_op.take() {
                    self.history.remove(id);
                }
            }
            Action::Commit(k) => self.commit(k, io),
            Action::Cancel => {
                self.text = None;
                io.call(&BackendCall::cancel());
                self.finish();
            }
            Action::Nudge { dx, dy, big } => {
                if let Some(s) = self.sel {
                    self.sel = Some(geom::nudge(&s, dx, dy, big, self.scale(), self.vw, self.vh));
                    if self.bar_visible {
                        self.place_bar();
                    }
                }
            }
            Action::NudgeSize { dw, dh, big } => {
                if let Some(s) = self.sel {
                    self.sel = Some(geom::nudge_size(&s, dw, dh, big, self.scale(), self.vw, self.vh));
                    if self.bar_visible {
                        self.place_bar();
                    }
                }
            }
            Action::PinToScreen => {
                self.after_save = Some(AfterSave::Pin);
                self.commit(CommitKind::Save, io);
            }
            Action::AddToShelf => {
                self.after_save = Some(AfterSave::Shelf);
                self.commit(CommitKind::Save, io);
            }
        }
    }

    /// `commit(extra)` of the page: close the text box, build the arguments, flatten the markup when there is any, call the
    /// backend, clear. With markup the PNG travels as the raw body and the arguments in the `x-pill-args` header; if the
    /// renderer cannot flatten, the plain JSON form is sent and the backend crops its own copy of the frame (no markup).
    fn commit(&mut self, kind: CommitKind, io: &mut dyn CaptureIo) {
        if self.busy {
            return;
        }
        self.commit_text();
        self.busy = true;
        let args = CommitArgs {
            label: self.label.clone(),
            vw: self.vw,
            vh: self.vh,
            action: kind,
            rect: self.sel.map(|s| [s.x, s.y, s.w, s.h]),
            point: None,
        };
        let call = BackendCall::commit(&args);
        let with_markup = self.mode == Mode::Area && !self.history.is_empty() && kind != CommitKind::Record && self.sel.is_some();
        if with_markup {
            let req = self.flatten_request();
            match io.flatten_png(&req) {
                Some(png) => {
                    io.call_raw(&call, &png);
                }
                None => {
                    io.call(&call);
                }
            }
        } else {
            io.call(&call);
        }
        if matches!(kind, CommitKind::Copy | CommitKind::Both | CommitKind::Save) {
            self.ghost_sel = self.sel;
            self.anim(AnimKind::CopyFlash, AnimTarget::Selection);
        }
        if self.bar_visible {
            self.anim(AnimKind::ToolbarLeave, AnimTarget::Toolbar);
        }
        self.finish();
    }

    /// What the renderer must draw to produce the markup PNG: the frame crop of the selection plus every op.
    pub fn flatten_request(&self) -> FlattenRequest {
        let sc = self.scale();
        let src = self.sel.map(|s| geom::export_rect(&s, sc, self.fw, self.fh)).unwrap_or(IRect { x: 0, y: 0, w: self.fw, h: self.fh });
        let cmds = self.history.ops().iter().flat_map(|o| o.draw_cmds(self.fw, self.fh, sc)).collect();
        FlattenRequest { src, cmds }
    }

    /// `pickCommit()`: send the sampled pixel centre (css px) and close.
    fn pick_commit(&mut self, io: &mut dyn CaptureIo, r: &mut Reaction) {
        if self.busy {
            return;
        }
        let Some(l) = self.loupe.as_ref() else { return };
        self.busy = true;
        let c = l.css_centre(self.scale());
        let args = CommitArgs { label: self.label.clone(), vw: self.vw, vh: self.vh, action: CommitKind::Pick, rect: None, point: Some(c) };
        r.actions.push(Action::Commit(CommitKind::Pick));
        r.redraw = true;
        io.call(&BackendCall::commit(&args));
        self.finish();
    }

    // ------------------------------------------------------------------------------------------- tick / view
    /// Advance animations. Returns true while something is still animating (keep rendering at the normal rate).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.now = now_ms;
        let running = self.anims.tick(now_ms);
        if !self.anims.has(AnimKind::ToolbarLeave) && !self.anims.has(AnimKind::CopyFlash) {
            self.ghost_sel = None;
            self.ghost_bar = None;
        }
        if !self.anims.has(AnimKind::LoupeFollow) {
            self.loupe_from = None;
        }
        running
    }

    fn cursor(&self) -> Cursor {
        if self.mode != Mode::Area {
            return Cursor::Default;
        }
        let Some(s) = self.sel else { return Cursor::Default };
        match geom::hit(&s, self.pointer) {
            Hit::Edge(h) => h.cursor(),
            Hit::Inside => match self.tool {
                Tool::Select => Cursor::Move,
                Tool::Text => Cursor::Text,
                _ => Cursor::Crosshair,
            },
            Hit::Outside => Cursor::Default,
        }
    }

    fn toolbar_state(&self) -> ToolbarState {
        ToolbarState {
            tool: Some(self.tool),
            color: self.color,
            can_undo: self.history.can_undo(),
            can_redo: self.history.can_redo(),
            busy: self.busy,
            hover: self.hover.clone(),
            pressed: self.pressed.clone(),
        }
    }

    /// Snapshot for the renderer. Cheap enough to call every frame (ops are re-expanded into draw commands each time).
    pub fn view(&self, tr: &dyn Tr) -> CaptureView {
        let sc = self.scale();
        let selection = self.sel.map(|s| SelectionVm {
            rect: s,
            size_label: format!("{} \u{d7} {}", (s.w * sc.rx).round() as i64, (s.h * sc.ry).round() as i64),
            size_inside: s.y < 30.0,
            handles: if self.mode == Mode::Area { HANDLES.iter().map(|h| HandleVm { handle: *h, at: h.anchor(&s), cursor: h.cursor() }).collect() } else { Vec::new() },
            clip: ClipInset { top: s.y, right: self.vw - s.x - s.w, bottom: self.vh - s.y - s.h, left: s.x },
        });
        let op_vm = |o: &Op| OpVm { id: o.id, cmds: o.draw_cmds(self.fw, self.fh, sc), selected: self.selected_op == Some(o.id) };
        let toolbar = if self.bar_visible { Some(self.toolbar.view(self.bar_origin, true, &self.toolbar_state(), tr)) } else { None };
        let ghost_active = self.anims.has(AnimKind::ToolbarLeave) || self.anims.has(AnimKind::CopyFlash);
        let ghost = if ghost_active && (self.ghost_sel.is_some() || self.ghost_bar.is_some()) {
            Some(Ghost {
                selection: self.ghost_sel.unwrap_or_default(),
                toolbar: self.ghost_bar.map(|o| self.toolbar.view(o, true, &self.toolbar_state(), tr)),
            })
        } else {
            None
        };
        CaptureView {
            mode: self.mode,
            phase: self.phase(),
            viewport: (self.vw, self.vh),
            scale: sc,
            hint: self.mode.hint(tr).map(|text| HintVm { text, visible: self.hint_visible && !self.committed }),
            dim_full: self.mode != Mode::Color && self.sel.is_none() && !self.committed,
            selection,
            ops: self.history.ops().iter().map(op_vm).collect(),
            draft: self.draft.as_ref().map(op_vm),
            text_edit: self.text.as_ref().map(|t| TextVm { pos: t.pos, color: t.color.clone(), text: t.text.clone(), caret: t.caret }),
            toolbar,
            loupe: self.loupe.as_ref().and_then(|l| l.view(sc, self.vw, self.vh)),
            loupe_from: self.loupe_from,
            cursor: self.cursor(),
            cursor_hidden: self.mode == Mode::Color,
            tool: self.tool,
            color: self.current_color(),
            busy: self.busy,
            anims: self.anims.active().to_vec(),
            ghost,
        }
    }
}
