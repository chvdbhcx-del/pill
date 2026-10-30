//! The native rich-text editor of the Note tab: a [`Doc`] (see `note_doc.rs`) drawn with one glasskit `RichLayout` per paragraph, a caret and a
//! selection, undo / redo, the web editor's formatting commands, and a scroll view.
//!
//! # What is supported (the web editor is a contenteditable with exactly this whitelist: `b i s mark ul ol li div br a input[checkbox]`)
//! * Inline: bold, italic, strikethrough, highlight (`mark`), toggled on a selection or as a pending style at the caret (Ctrl+B / Ctrl+I /
//!   toolbar). Links that are already in a note are kept, drawn underlined in the accent colour and opened with Ctrl+click
//!   (`chat_open_link`); they cannot be created (neither can the web editor create them, only paste them).
//! * Lines: plain, bullet, numbered (nesting levels are kept and drawn), checklist (click the box to tick, Enter continues the list, Enter on an
//!   empty item ends it, Backspace at the start of a list line turns it into a plain line). Shift+Enter is a soft line break (`<br>`).
//! * Editing: typing (also Cyrillic / AltGr / composed text through `InputEvent::Char`), Backspace / Delete (Ctrl: by word), Enter, Ctrl+A / C / X /
//!   V (plain text), Ctrl+Z / Y / Shift+Z (200 snapshots, typing coalesced), mouse: click, drag, shift-click, double-click word, triple-click
//!   line, drag past the edge scrolls; keys: arrows (Ctrl: word, Shift: extend), Home / End (Ctrl: document), PageUp / PageDown.
//! * NOT supported: underline, headings, colours, images, tables (the sanitizer removes them in the web too), pasting rich text (paste is plain
//!   text), IME composition string (the finished text arrives as `Char`), drag-and-drop of text, spell-check.
//!
//! # Threading of input
//! `View::event` has no `Canvas` but hit-testing needs one, so [`Editor::queue`] stores the event and the next [`Editor::draw`] applies it before
//! painting (the typed character is on screen in the same frame). Results go to [`Editor::take_out`].
use super::note_doc::*;
use glasskit::canvas::Canvas;
use glasskit::ease::{curves, Tween};
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, KeyCode, Modifiers, MouseButton, Response};
use glasskit::richtext::{RichLayout, RichText, SpanStyle};
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use crate::ui::view::Cx;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// Inner padding of the editor box (`.tn-ed { padding: 16px }`).
pub const PAD: f32 = 16.0;
/// Line height (`font: 500 14px / 20px`).
pub const LH: f32 = 20.0;
const BASE: Style = Style::new(14.0, 500);
const UNDO_CAP: usize = 200;
/// Seconds the caret keeps blinking after the last input; then it stays solid so an idle focused editor costs no frames.
const BLINK_WINDOW: f32 = 12.0;
const BLINK_HALF: f32 = 0.53;
const CHECK_INDENT: f32 = 23.0;
const BOX: f32 = 15.0;

/// What the editor reports to the view.
#[derive(Clone, Debug, PartialEq)]
pub enum EdOut {
    /// The user changed the document (read [`Editor::html`]).
    Changed,
    /// The editor lost focus.
    Blur,
    OpenLink(String),
    /// Formatting at the caret: bold, italic, strike, mark, bullet, numbered.
    Caret([bool; 6]),
}

#[derive(Clone)]
struct Snap {
    doc: Doc,
    caret: Pos,
    anchor: Pos,
}

struct Geo {
    y: f32,
    h: f32,
    indent: f32,
    lay: Option<Rc<RichLayout>>,
}

#[derive(Clone, Copy)]
enum Drag {
    Chars,
    Words(Pos, Pos),
    Lines(usize),
    Bar,
}

pub struct Editor {
    pub doc: Doc,
    caret: Pos,
    anchor: Pos,
    /// formatting for the next typed char at a collapsed caret (`None` = continue the char before the caret)
    pend: Option<u8>,
    want_x: Option<f32>,
    cache: HashMap<u64, Rc<RichLayout>>,
    geo: Vec<Geo>,
    geo_dirty: bool,
    content_h: f32,
    rect: Rect,
    sv: ScrollView,
    inbox: Vec<InputEvent>,
    focused: bool,
    blink: f32,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    coalesce: bool,
    drag: Option<Drag>,
    out: Vec<EdOut>,
    reveal: bool,
    last_caret: [bool; 6],
    ime: Option<Rect>,
    ph_t: Tween<f32>,
    ph_on: bool,
    pop: Option<(usize, Tween<f32>)>,
    dark: bool,
    pub reduced: bool,
}

fn u16_of(text: &[char], col: usize) -> usize {
    text[..col.min(text.len())].iter().map(|c| c.len_utf16()).sum()
}

fn col_of(text: &[char], u: usize) -> usize {
    let mut n = 0;
    for (i, c) in text.iter().enumerate() {
        if n >= u {
            return i;
        }
        n += c.len_utf16();
    }
    text.len()
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new()
    }
}

impl Editor {
    pub fn new() -> Editor {
        let mut sv = ScrollView::new();
        sv.keys = false;
        Editor {
            doc: Doc::empty(),
            caret: Pos::default(),
            anchor: Pos::default(),
            pend: None,
            want_x: None,
            cache: HashMap::new(),
            geo: Vec::new(),
            geo_dirty: true,
            content_h: 0.0,
            rect: Rect::default(),
            sv,
            inbox: Vec::new(),
            focused: false,
            blink: 0.0,
            undo: Vec::new(),
            redo: Vec::new(),
            coalesce: false,
            drag: None,
            out: Vec::new(),
            reveal: false,
            last_caret: [false; 6],
            ime: None,
            ph_t: Tween::at_rest(1.0),
            ph_on: true,
            pop: None,
            dark: true,
            reduced: false,
        }
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // public surface
    // -----------------------------------------------------------------------------------------------------------------------------------

    /// Replace the document (a note was opened). Resets caret, history and scroll; reports nothing.
    pub fn set_html(&mut self, html: &str) {
        self.doc = Doc::from_html(html);
        self.caret = Pos::default();
        self.anchor = Pos::default();
        self.pend = None;
        self.undo.clear();
        self.redo.clear();
        self.coalesce = false;
        self.drag = None;
        self.geo_dirty = true;
        self.sv.state.scroll_to(0.0, false);
        self.last_caret = [false; 6];
    }

    pub fn html(&self) -> String {
        self.doc.to_html()
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_focused(&mut self, on: bool) {
        if on == self.focused {
            return;
        }
        self.focused = on;
        self.blink = 0.0;
        self.coalesce = false;
        if !on {
            self.drag = None;
            self.out.push(EdOut::Blur);
        }
    }

    pub fn take_out(&mut self) -> Vec<EdOut> {
        std::mem::take(&mut self.out)
    }

    /// The selected text (empty when collapsed).
    pub fn selected_text(&self) -> String {
        let (a, b) = self.sel();
        self.doc.text_in(a, b)
    }

    /// The caret box in window coordinates as of the last draw (for the IME candidate window).
    pub fn ime_rect(&self) -> Option<Rect> {
        self.ime
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selection(&self) -> (Pos, Pos) {
        self.sel()
    }

    pub fn caret(&self) -> Pos {
        self.caret
    }

    pub fn set_selection(&mut self, anchor: Pos, caret: Pos) {
        self.anchor = self.doc.clamp(anchor);
        self.caret = self.doc.clamp(caret);
        self.pend = None;
        self.reveal = true;
    }

    /// Does this event belong to the editor (pointer on it / dragging, keys while focused)?
    pub fn wants(&self, ev: &InputEvent) -> bool {
        match ev {
            InputEvent::MouseDown { pos, .. } | InputEvent::Wheel { pos, .. } => self.rect.contains(pos.0, pos.1),
            InputEvent::MouseMove { pos, .. } => self.drag.is_some() || self.rect.contains(pos.0, pos.1),
            InputEvent::MouseUp { .. } => self.drag.is_some(),
            InputEvent::MouseLeave => true,
            InputEvent::Key { .. } | InputEvent::Char(_) => self.focused,
            _ => false,
        }
    }

    /// Queue an input event for the next draw and answer the host now.
    pub fn queue(&mut self, ev: &InputEvent) -> Response {
        let r = self.predict(ev);
        match ev {
            InputEvent::MouseDown { button: MouseButton::Left, pos, .. } if self.rect.contains(pos.0, pos.1) => self.focused_pending(),
            _ => {}
        }
        self.inbox.push(ev.clone());
        r
    }

    fn focused_pending(&mut self) {
        if !self.focused {
            self.focused = true;
            self.blink = 0.0;
        }
    }

    pub fn predict(&self, ev: &InputEvent) -> Response {
        match ev {
            InputEvent::MouseDown { button: MouseButton::Left, pos, .. } if self.rect.contains(pos.0, pos.1) => {
                Response { consumed: true, redraw: true, capture: true, focus: true, cursor: Some(Cursor::Text), ..Response::NONE }
            }
            InputEvent::MouseMove { pos, .. } if self.drag.is_some() || self.rect.contains(pos.0, pos.1) => {
                Response { redraw: true, capture: self.drag.is_some(), cursor: Some(Cursor::Text), ..Response::NONE }
            }
            InputEvent::MouseUp { .. } if self.drag.is_some() => Response { consumed: true, redraw: true, ..Response::NONE },
            InputEvent::Wheel { .. } => Response { consumed: self.sv.state.scrollable(), redraw: true, ..Response::NONE },
            InputEvent::Key { code, mods, .. } => {
                let ours = !matches!(code, KeyCode::Tab | KeyCode::Escape) && !mods.alt && !matches!(code, KeyCode::F(_));
                Response { consumed: ours, redraw: ours, ..Response::NONE }
            }
            InputEvent::Char(_) => Response { consumed: true, redraw: true, ..Response::NONE },
            InputEvent::MouseLeave => Response::redraw(),
            _ => Response::NONE,
        }
    }

    /// Advance animations; true while frames are needed (scroll glide, placeholder fade, checkbox pop, the caret's blink window).
    pub fn step(&mut self, dt_ms: f64) -> bool {
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.sv.step(dt);
        busy |= self.ph_t.step(dt);
        if let Some((_, t)) = &mut self.pop {
            if !t.step(dt) {
                self.pop = None;
            } else {
                busy = true;
            }
        }
        if self.focused {
            self.blink += dt;
            busy |= !self.reduced && self.blink < BLINK_WINDOW;
        }
        busy || !self.inbox.is_empty()
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // history and small helpers
    // -----------------------------------------------------------------------------------------------------------------------------------

    fn sel(&self) -> (Pos, Pos) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    fn snap(&self) -> Snap {
        Snap { doc: self.doc.clone(), caret: self.caret, anchor: self.anchor }
    }

    /// Call before an edit. `typing` coalesces consecutive typed characters into one undo step.
    fn begin_edit(&mut self, typing: bool) {
        if !(typing && self.coalesce) {
            let s = self.snap();
            self.undo.push(s);
            if self.undo.len() > UNDO_CAP {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.coalesce = typing;
    }

    fn changed(&mut self) {
        if self.out.last() != Some(&EdOut::Changed) {
            self.out.push(EdOut::Changed);
        }
        self.reveal = true;
        self.blink = 0.0;
        self.geo_dirty = true;
        self.want_x = None;
    }

    fn move_to(&mut self, p: Pos, extend: bool) {
        self.caret = self.doc.clamp(p);
        if !extend {
            self.anchor = self.caret;
        }
        self.pend = None;
        self.coalesce = false;
        self.reveal = true;
        self.blink = 0.0;
    }

    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.sel();
        if a == b {
            return false;
        }
        let p = self.doc.delete(a, b);
        self.caret = p;
        self.anchor = p;
        true
    }

    fn typing_attr(&self) -> Attr {
        let (a, _) = self.sel();
        match self.pend {
            Some(f) if !self.has_selection() => Attr { f, link: 0 },
            _ if self.has_selection() => {
                let l = &self.doc.lines[a.line];
                if a.col < l.len() && a.col > 0 {
                    self.doc.attr_before(Pos::new(a.line, a.col + 1))
                } else if a.col < l.len() {
                    Attr { f: l.at[a.col].f & FMT_MASK, link: 0 }
                } else {
                    self.doc.attr_before(a)
                }
            }
            _ if self.caret.col == 0 && !self.doc.lines[self.caret.line].is_empty() => {
                // at the start of a line the browser continues the formatting of the first char
                Attr { f: self.doc.lines[self.caret.line].at[0].f & FMT_MASK, link: 0 }
            }
            _ => self.doc.attr_before(self.caret),
        }
    }

    fn type_text(&mut self, s: &str) {
        let single = s.chars().count() == 1;
        self.begin_edit(single);
        let attr = self.typing_attr();
        self.delete_selection();
        let p = if s.contains('\n') || s.contains('\r') { self.doc.insert_multiline(self.caret, s, attr) } else { self.doc.insert(self.caret, s, attr) };
        self.caret = p;
        self.anchor = p;
        self.pend = None;
        self.changed();
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // commands (also reachable from the toolbar through `exec`)
    // -----------------------------------------------------------------------------------------------------------------------------------

    /// Run a web-editor command by its `execCommand` name (what `Effect::EditorExec` carries).
    pub fn exec(&mut self, cmd: &str) {
        match cmd {
            "bold" => self.toggle_flag(B),
            "italic" => self.toggle_flag(I),
            "strikeThrough" => self.toggle_flag(S),
            "highlight" => self.toggle_flag(M),
            "insertUnorderedList" => self.toggle_list(Kind::Bullet),
            "insertOrderedList" => self.toggle_list(Kind::Number),
            "checklist" => self.checklist(),
            _ => {}
        }
        self.focused_pending();
    }

    fn toggle_flag(&mut self, bit: u8) {
        let (a, b) = self.sel();
        if a != b {
            self.begin_edit(false);
            let on = !self.doc.all_have(a, b, bit);
            self.doc.set_flag(a, b, bit, on);
            self.changed();
            return;
        }
        if bit == M {
            // inside a highlight: remove the whole highlighted run (the web toggles the `mark` element off)
            let l = &self.doc.lines[a.line];
            let at = if a.col < l.len() && l.at[a.col].f & M != 0 { Some(a.col) } else if a.col > 0 && l.at[a.col - 1].f & M != 0 { Some(a.col - 1) } else { None };
            if let Some(i) = at {
                let (mut s, mut e) = (i, i + 1);
                while s > 0 && l.at[s - 1].f & M != 0 {
                    s -= 1;
                }
                while e < l.len() && l.at[e].f & M != 0 {
                    e += 1;
                }
                self.begin_edit(false);
                self.doc.set_flag(Pos::new(a.line, s), Pos::new(a.line, e), M, false);
                self.changed();
                return;
            }
        }
        let cur = self.pend.unwrap_or(self.doc.attr_before(self.caret).f);
        self.pend = Some((cur ^ bit) & FMT_MASK);
        self.coalesce = false;
    }

    fn toggle_list(&mut self, kind: Kind) {
        let (a, b) = self.sel();
        self.begin_edit(false);
        let all = (a.line..=b.line).all(|i| self.doc.lines[i].kind == kind);
        for i in a.line..=b.line {
            let l = &mut self.doc.lines[i];
            if all {
                l.kind = Kind::Plain;
                l.level = 0;
            } else {
                if !l.kind.is_list() {
                    l.level = 0;
                }
                l.kind = kind;
            }
        }
        self.changed();
    }

    fn checklist(&mut self) {
        self.begin_edit(false);
        let line = self.caret.line;
        let l = &self.doc.lines[line];
        match l.kind {
            Kind::Check(_) => self.doc.lines[line].kind = Kind::Plain,
            Kind::Plain if self.caret.col == 0 || l.is_empty() => self.doc.lines[line].kind = Kind::Check(false),
            _ => {
                // mid-line: an inline box followed by a no-break space, like `insertHTML('<input type=checkbox>&nbsp;')`
                let p = self.doc.insert(self.caret, &BOX_OFF.to_string(), Attr { f: CHK, link: 0 });
                let p = self.doc.insert(p, "\u{a0}", Attr::default());
                self.caret = p;
                self.anchor = p;
            }
        }
        self.changed();
    }

    fn toggle_box(&mut self, line: usize) {
        if let Kind::Check(t) = self.doc.lines[line].kind {
            self.begin_edit(false);
            self.doc.lines[line].kind = Kind::Check(!t);
            if !self.reduced {
                self.pop = Some((line, Tween::new(0.0, 1.0, 0.32, curves::SPRING_POP)));
            }
            self.changed();
        }
    }

    fn do_undo(&mut self, redo: bool) {
        let from = if redo { &mut self.redo } else { &mut self.undo };
        let Some(s) = from.pop() else { return };
        let cur = Snap { doc: self.doc.clone(), caret: self.caret, anchor: self.anchor };
        if redo {
            self.undo.push(cur);
        } else {
            self.redo.push(cur);
        }
        self.doc = s.doc;
        self.caret = self.doc.clamp(s.caret);
        self.anchor = self.doc.clamp(s.anchor);
        self.coalesce = false;
        self.pend = None;
        self.changed();
    }

    fn enter(&mut self, soft: bool) {
        self.begin_edit(false);
        let attr = self.typing_attr();
        self.delete_selection();
        if soft {
            let p = self.doc.insert(self.caret, "\n", attr);
            self.caret = p;
            self.anchor = p;
        } else {
            let l = &mut self.doc.lines[self.caret.line];
            if l.is_empty() && l.kind != Kind::Plain {
                // Enter on an empty list / checklist item ends the list
                l.kind = Kind::Plain;
                l.level = 0;
            } else {
                let p = self.doc.split(self.caret);
                self.caret = p;
                self.anchor = p;
            }
        }
        self.pend = None;
        self.changed();
    }

    fn backspace(&mut self, word: bool) {
        if self.has_selection() {
            self.begin_edit(false);
            self.delete_selection();
            return self.changed();
        }
        let p = self.caret;
        if p.col == 0 {
            if self.doc.lines[p.line].kind != Kind::Plain {
                self.begin_edit(false);
                let l = &mut self.doc.lines[p.line];
                l.kind = Kind::Plain;
                l.level = 0;
                return self.changed();
            }
            if p.line == 0 {
                return;
            }
            self.begin_edit(false);
            let prev = Pos::new(p.line - 1, self.doc.lines[p.line - 1].len());
            self.caret = self.doc.delete(prev, p);
            self.anchor = self.caret;
            return self.changed();
        }
        let from = if word { self.word_left(p) } else { Pos::new(p.line, p.col - 1) };
        self.begin_edit(word || !self.coalesce);
        self.coalesce = true;
        self.caret = self.doc.delete(from, p);
        self.anchor = self.caret;
        self.changed();
    }

    fn delete_fwd(&mut self, word: bool) {
        if self.has_selection() {
            self.begin_edit(false);
            self.delete_selection();
            return self.changed();
        }
        let p = self.caret;
        let len = self.doc.lines[p.line].len();
        let to = if p.col < len {
            if word {
                self.word_right(p)
            } else {
                Pos::new(p.line, p.col + 1)
            }
        } else if p.line + 1 < self.doc.lines.len() {
            Pos::new(p.line + 1, 0)
        } else {
            return;
        };
        self.begin_edit(false);
        self.doc.delete(p, to);
        self.changed();
    }

    fn word_left(&self, p: Pos) -> Pos {
        if p.col == 0 {
            return if p.line > 0 { Pos::new(p.line - 1, self.doc.lines[p.line - 1].len()) } else { p };
        }
        let t = &self.doc.lines[p.line].text;
        let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
        let mut i = p.col;
        while i > 0 && class(t[i - 1]) == 1 {
            i -= 1;
        }
        if i > 0 {
            let k = class(t[i - 1]);
            while i > 0 && class(t[i - 1]) == k {
                i -= 1;
            }
        }
        Pos::new(p.line, i)
    }

    fn word_right(&self, p: Pos) -> Pos {
        let t = &self.doc.lines[p.line].text;
        if p.col >= t.len() {
            return if p.line + 1 < self.doc.lines.len() { Pos::new(p.line + 1, 0) } else { p };
        }
        let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
        let mut i = p.col;
        let k = class(t[i]);
        while i < t.len() && class(t[i]) == k {
            i += 1;
        }
        while i < t.len() && class(t[i]) == 1 {
            i += 1;
        }
        Pos::new(p.line, i)
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // geometry
    // -----------------------------------------------------------------------------------------------------------------------------------

    fn line_indent(&self, l: &Line) -> f32 {
        match l.kind {
            Kind::Plain => 0.0,
            Kind::Check(_) => CHECK_INDENT,
            _ => 24.0 + 22.0 * l.level as f32,
        }
    }

    fn rich(&self, l: &Line, th: &Theme) -> RichText {
        let mut rt = RichText::new().with_base(BASE).with_line_height(LH);
        let mut i = 0;
        while i < l.text.len() {
            let a = l.at[i];
            let mut j = i;
            while j < l.text.len() && l.at[j] == a {
                j += 1;
            }
            let s: String = l.text[i..j].iter().collect();
            let mut st = SpanStyle { bold: a.f & B != 0, italic: a.f & I != 0, strike: a.f & S != 0, ..Default::default() };
            if a.f & M != 0 {
                st.bg = Some(th.amber.alpha(0.38));
            }
            if let Some(url) = self.doc.link(a) {
                st.underline = true;
                st.link = Some(url.to_string());
                st.color = Some(th.accent);
            }
            if a.f & CHK != 0 {
                st.color = Some(if a.f & TICK != 0 { th.accent } else { th.dim });
            }
            rt.push_mut(&s, st);
            i = j;
        }
        rt
    }

    fn key_of(&self, l: &Line, w: f32, dark: bool) -> u64 {
        let mut h = DefaultHasher::new();
        l.text.hash(&mut h);
        for a in &l.at {
            a.f.hash(&mut h);
            self.doc.link(*a).hash(&mut h);
        }
        w.to_bits().hash(&mut h);
        dark.hash(&mut h);
        h.finish()
    }

    fn ensure_geo(&mut self, cv: &Canvas, th: &Theme) {
        if !self.geo_dirty && self.dark == th.dark && self.geo.len() == self.doc.lines.len() {
            return;
        }
        self.dark = th.dark;
        if self.cache.len() > 3000 {
            self.cache.clear();
        }
        let width = (self.rect.w - 2.0 * PAD).max(40.0);
        let mut geo = Vec::with_capacity(self.doc.lines.len());
        let mut y = 0.0;
        for (i, l) in self.doc.lines.iter().enumerate() {
            let list = l.kind.is_list();
            if list && (i == 0 || !self.doc.lines[i - 1].kind.is_list()) {
                y += 2.0;
            }
            let indent = self.line_indent(l);
            let w = width - indent;
            let key = self.key_of(l, w, th.dark);
            let lay = match self.cache.get(&key) {
                Some(r) => Some(r.clone()),
                None => RichLayout::new(cv, &self.rich(l, th), w).ok().map(|r| {
                    let r = Rc::new(r);
                    self.cache.insert(key, r.clone());
                    r
                }),
            };
            let h = lay.as_ref().map_or(LH, |r| r.height().max(LH));
            geo.push(Geo { y, h, indent, lay });
            y += h;
            if list && (i + 1 == self.doc.lines.len() || !self.doc.lines[i + 1].kind.is_list()) {
                y += 2.0;
            }
        }
        self.content_h = y + 2.0 * PAD;
        self.geo = geo;
        self.geo_dirty = false;
        self.sv.set_content_height(self.content_h, self.rect.h);
    }

    fn to_content(&self, p: Pt) -> Pt {
        (p.0 - self.rect.x - PAD, p.1 - self.rect.y + self.sv.state.offset() - PAD)
    }

    fn line_at(&self, y: f32) -> usize {
        self.geo.partition_point(|g| g.y + g.h <= y).min(self.geo.len().saturating_sub(1))
    }

    fn pos_at(&self, c: Pt) -> Pos {
        if self.geo.is_empty() {
            return Pos::default();
        }
        let li = self.line_at(c.1);
        let g = &self.geo[li];
        let l = &self.doc.lines[li];
        let Some(lay) = &g.lay else { return Pos::new(li, 0) };
        let u = lay.index_at((c.0 - g.indent, c.1 - g.y));
        Pos::new(li, col_of(&l.text, u))
    }

    /// Caret box in content coordinates: (x, y, height).
    fn caret_box(&self, p: Pos) -> (f32, f32, f32) {
        let Some(g) = self.geo.get(p.line) else { return (0.0, 0.0, LH) };
        let l = &self.doc.lines[p.line];
        match &g.lay {
            Some(lay) => {
                let r = lay.caret_rect(u16_of(&l.text, p.col));
                (g.indent + r.x, g.y + r.y, r.h.max(LH))
            }
            None => (g.indent, g.y, LH),
        }
    }

    fn vertical(&mut self, dir: f32, rows_px: f32, extend: bool) {
        let (cx, cy, ch) = self.caret_box(self.caret);
        let wx = *self.want_x.get_or_insert(cx);
        let ty = cy + ch / 2.0 + dir * rows_px;
        let total = self.content_h - 2.0 * PAD;
        let p = if ty < 0.0 {
            Pos::new(0, 0)
        } else if ty >= total {
            self.doc.end()
        } else {
            self.pos_at((wx, ty))
        };
        let keep = self.want_x;
        self.move_to(p, extend);
        self.want_x = keep;
    }

    fn row_edge(&mut self, end: bool, extend: bool) {
        let (_, cy, ch) = self.caret_box(self.caret);
        let p = self.pos_at((if end { 1.0e6 } else { -1.0e6 }, cy + ch / 2.0));
        self.move_to(p, extend);
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // input
    // -----------------------------------------------------------------------------------------------------------------------------------

    fn handle(&mut self, ev: &InputEvent, cv: &Canvas, th: &Theme, cx: &Cx) {
        self.ensure_geo(cv, th);
        match ev {
            InputEvent::MouseDown { pos, button: MouseButton::Left, mods, clicks } => {
                let r = self.sv.event(ev, self.rect);
                if r.consumed || r.capture {
                    self.drag = Some(Drag::Bar);
                    return;
                }
                if !self.rect.contains(pos.0, pos.1) {
                    return;
                }
                self.set_focused(true);
                let c = self.to_content(*pos);
                let li = self.line_at(c.1);
                if let (Some(g), Some(l)) = (self.geo.get(li), self.doc.lines.get(li)) {
                    if matches!(l.kind, Kind::Check(_)) && c.0 < BOX + 4.0 && c.1 >= g.y && c.1 < g.y + LH {
                        return self.toggle_box(li);
                    }
                    if mods.ctrl {
                        if let Some(url) = g.lay.as_ref().and_then(|lay| lay.link_at((c.0 - g.indent, c.1 - g.y))) {
                            self.out.push(EdOut::OpenLink(url.to_string()));
                            return;
                        }
                    }
                }
                let p = self.pos_at(c);
                match clicks {
                    2 => {
                        let (s, e) = self.doc.word_at(p);
                        self.anchor = s;
                        self.caret = e;
                        self.drag = Some(Drag::Words(s, e));
                    }
                    n if *n >= 3 => {
                        self.anchor = Pos::new(p.line, 0);
                        self.caret = Pos::new(p.line, self.doc.lines[p.line].len());
                        self.drag = Some(Drag::Lines(p.line));
                    }
                    _ => {
                        self.caret = p;
                        if !mods.shift {
                            self.anchor = p;
                        }
                        self.drag = Some(Drag::Chars);
                    }
                }
                self.pend = None;
                self.coalesce = false;
                self.want_x = None;
                self.blink = 0.0;
                self.reveal = true;
            }
            InputEvent::MouseMove { pos, .. } => {
                match self.drag {
                    Some(Drag::Bar) => {
                        self.sv.event(ev, self.rect);
                    }
                    Some(d) => {
                        // dragging past the edge scrolls
                        if pos.1 < self.rect.y {
                            self.sv.state.scroll_by(-14.0, false);
                        } else if pos.1 > self.rect.bottom() {
                            self.sv.state.scroll_by(14.0, false);
                        }
                        let p = self.pos_at(self.to_content(*pos));
                        match d {
                            Drag::Chars => self.caret = p,
                            Drag::Words(a0, a1) => {
                                let (ws, we) = self.doc.word_at(p);
                                if p >= a0 {
                                    self.anchor = a0;
                                    self.caret = we.max(a1);
                                } else {
                                    self.anchor = a1;
                                    self.caret = ws;
                                }
                            }
                            Drag::Lines(l0) => {
                                if p.line >= l0 {
                                    self.anchor = Pos::new(l0, 0);
                                    self.caret = Pos::new(p.line, self.doc.lines[p.line].len());
                                } else {
                                    self.anchor = Pos::new(l0, self.doc.lines[l0].len());
                                    self.caret = Pos::new(p.line, 0);
                                }
                            }
                            Drag::Bar => {}
                        }
                        self.reveal = true;
                    }
                    None => {
                        self.sv.event(ev, self.rect);
                    }
                }
            }
            InputEvent::MouseUp { .. } => {
                if matches!(self.drag, Some(Drag::Bar)) {
                    self.sv.event(ev, self.rect);
                }
                self.drag = None;
            }
            InputEvent::MouseLeave => {
                self.sv.event(ev, self.rect);
            }
            InputEvent::Wheel { .. } => {
                self.sv.event(ev, self.rect);
            }
            InputEvent::Char(c) => {
                if self.focused && !c.is_control() {
                    let s = c.to_string();
                    self.type_text(&s);
                }
            }
            InputEvent::Key { code, mods, down: true, .. } if self.focused => self.key(*code, *mods, cx),
            _ => {}
        }
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers, cx: &Cx) {
        let (shift, ctrl) = (mods.shift, mods.ctrl);
        if mods.alt && !ctrl {
            return;
        }
        match code {
            KeyCode::Char(c) if ctrl => match c.to_ascii_uppercase() {
                'A' => {
                    self.anchor = Pos::default();
                    self.caret = self.doc.end();
                    self.pend = None;
                }
                'C' => {
                    if self.has_selection() {
                        cx.set_clipboard_text(&self.selected_text());
                    }
                }
                'X' => {
                    if self.has_selection() {
                        cx.set_clipboard_text(&self.selected_text());
                        self.begin_edit(false);
                        self.delete_selection();
                        self.changed();
                    }
                }
                'V' => {
                    if let Some(t) = cx.clipboard_text().filter(|t| !t.is_empty()) {
                        self.type_text(&t);
                        self.coalesce = false;
                    }
                }
                'Z' => self.do_undo(shift),
                'Y' => self.do_undo(true),
                'B' => self.toggle_flag(B),
                'I' => self.toggle_flag(I),
                _ => {}
            },
            KeyCode::Left => {
                let (a, b) = self.sel();
                if a != b && !shift {
                    self.move_to(a, false);
                } else {
                    let p = if ctrl { self.word_left(self.caret) } else if self.caret.col > 0 { Pos::new(self.caret.line, self.caret.col - 1) } else if self.caret.line > 0 { Pos::new(self.caret.line - 1, self.doc.lines[self.caret.line - 1].len()) } else { self.caret };
                    self.move_to(p, shift);
                }
            }
            KeyCode::Right => {
                let (a, b) = self.sel();
                if a != b && !shift {
                    self.move_to(b, false);
                } else {
                    let len = self.doc.lines[self.caret.line].len();
                    let p = if ctrl { self.word_right(self.caret) } else if self.caret.col < len { Pos::new(self.caret.line, self.caret.col + 1) } else if self.caret.line + 1 < self.doc.lines.len() { Pos::new(self.caret.line + 1, 0) } else { self.caret };
                    self.move_to(p, shift);
                }
            }
            KeyCode::Up => self.vertical(-1.0, LH, shift),
            KeyCode::Down => self.vertical(1.0, LH, shift),
            KeyCode::PageUp => self.vertical(-1.0, (self.rect.h - 2.0 * PAD - LH).max(LH), shift),
            KeyCode::PageDown => self.vertical(1.0, (self.rect.h - 2.0 * PAD - LH).max(LH), shift),
            KeyCode::Home => {
                if ctrl {
                    self.move_to(Pos::default(), shift)
                } else {
                    self.row_edge(false, shift)
                }
            }
            KeyCode::End => {
                if ctrl {
                    let e = self.doc.end();
                    self.move_to(e, shift)
                } else {
                    self.row_edge(true, shift)
                }
            }
            KeyCode::Enter => self.enter(shift),
            KeyCode::Backspace => self.backspace(ctrl),
            KeyCode::Delete => {
                if shift && self.has_selection() {
                    cx.set_clipboard_text(&self.selected_text());
                    self.begin_edit(false);
                    self.delete_selection();
                    self.changed();
                } else {
                    self.delete_fwd(ctrl)
                }
            }
            _ => {}
        }
        self.blink = 0.0;
        self.reveal = true;
    }

    fn caret_flags(&self) -> [bool; 6] {
        let (a, b) = self.sel();
        let f = |bit: u8| {
            if a != b {
                self.doc.all_have(a, b, bit)
            } else {
                self.pend.unwrap_or(self.doc.attr_before(self.caret).f) & bit != 0
            }
        };
        let k = self.doc.lines[self.caret.line].kind;
        [f(B), f(I), f(S), f(M), k == Kind::Bullet, k == Kind::Number]
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // draw
    // -----------------------------------------------------------------------------------------------------------------------------------

    /// Apply the queued input, then paint the editor box into `rect`. `placeholder`: the hint shown while the note is blank (`None` = never).
    pub fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, placeholder: Option<&str>, cx: &Cx) {
        self.rect = rect;
        self.reduced = cx.reduced_motion;
        self.sv.state.reduced = Some(cx.reduced_motion);
        self.sv.bar.reduced = Some(cx.reduced_motion);
        self.geo_dirty |= self.geo.is_empty();
        self.ensure_geo(cv, th);
        let evs = std::mem::take(&mut self.inbox);
        for ev in &evs {
            self.handle(ev, cv, th, cx);
        }
        self.caret = self.doc.clamp(self.caret);
        self.anchor = self.doc.clamp(self.anchor);
        self.ensure_geo(cv, th);
        self.sv.set_content_height(self.content_h, rect.h);
        if std::mem::take(&mut self.reveal) {
            let (_, y, h) = self.caret_box(self.caret);
            let (top, bottom) = (y, y + h + 2.0 * PAD);
            let off = self.sv.state.target();
            if top < off {
                self.sv.state.scroll_to(top, true);
            } else if bottom > off + rect.h {
                self.sv.state.scroll_to(bottom - rect.h, true);
            }
        }
        let flags = self.caret_flags();
        if flags != self.last_caret {
            self.last_caret = flags;
            self.out.push(EdOut::Caret(flags));
        }
        let want_ph = placeholder.is_some() && self.doc.is_blank();
        if want_ph != self.ph_on {
            self.ph_on = want_ph;
            if self.reduced {
                self.ph_t.jump(want_ph as u8 as f32);
            } else {
                self.ph_t.retarget(want_ph as u8 as f32, 0.12, curves::EASE_OUT);
            }
        }
        self.paint(cv, th, rect, placeholder);
    }

    fn paint(&mut self, cv: &Canvas, th: &Theme, rect: Rect, placeholder: Option<&str>) {
        cv.fill_rr(rect, 16.0, th.card);
        let off = self.sv.state.offset();
        let (sa, sb) = self.sel();
        let sel_col = th.accent.alpha(if th.dark { 0.38 } else { 0.30 });
        let show_caret = self.focused && (self.reduced || self.blink >= BLINK_WINDOW || self.blink % (2.0 * BLINK_HALF) < BLINK_HALF);
        cv.with_clip_rr(rect, 16.0, |cv| {
            cv.with_offset(rect.x + PAD, rect.y + PAD - off, |cv| {
                let view_top = off - PAD - 40.0;
                let view_bot = off + rect.h + 40.0;
                for (i, g) in self.geo.iter().enumerate() {
                    if g.y + g.h < view_top || g.y > view_bot {
                        continue;
                    }
                    let l = &self.doc.lines[i];
                    // selection
                    if sa != sb && i >= sa.line && i <= sb.line {
                        let c0 = if i == sa.line { sa.col } else { 0 };
                        let c1 = if i == sb.line { sb.col } else { l.len() };
                        if let Some(lay) = &g.lay {
                            let (u0, u1) = (u16_of(&l.text, c0), u16_of(&l.text, c1));
                            for r in lay.selection_rects(u0..u1) {
                                cv.fill_rr(r.offset(g.indent, g.y), 2.0, sel_col);
                            }
                            if i < sb.line {
                                let e = lay.caret_rect(u16_of(&l.text, l.len()));
                                cv.fill_rr(Rect::new(g.indent + e.x, g.y + e.y, 6.0, e.h), 2.0, sel_col);
                            }
                        }
                    }
                    // gutter: bullet / number / checkbox
                    match l.kind {
                        Kind::Bullet => cv.fill_circle((g.indent - 12.0, g.y + LH / 2.0), 2.4, th.text),
                        Kind::Number => {
                            let n = format!("{}.", self.doc.number_of(i));
                            cv.text(Rect::new(g.indent - 40.0, g.y, 34.0, LH), &n, BASE.right(), th.text);
                        }
                        Kind::Check(t) => {
                            let scale = match &self.pop {
                                Some((pi, tw)) if *pi == i => 0.72 + 0.28 * tw.value(),
                                _ => 1.0,
                            };
                            let r = Rect::new(0.0, g.y + (LH - BOX) / 2.0 + 0.5, BOX, BOX);
                            cv.with_scale_about(r.center(), scale, |cv| draw_box(cv, th, r, t));
                        }
                        Kind::Plain => {}
                    }
                    if let Some(lay) = &g.lay {
                        lay.draw(cv, (g.indent, g.y), th.text);
                    }
                }
                if show_caret {
                    let (x, y, h) = self.caret_box(self.caret);
                    cv.fill_rect(Rect::new(x, y, 1.5, h), th.text);
                }
            });
            if let (Some(ph), k) = (placeholder, self.ph_t.value().clamp(0.0, 1.0)) {
                if k > 0.01 {
                    cv.text(Rect::new(rect.x + PAD, rect.y + PAD, rect.w - 2.0 * PAD, LH * 3.0), ph, BASE.wrap(), th.dim.fade(0.85 * k));
                }
            }
        });
        self.sv.bar.draw(cv, rect, &self.sv.state, th);
        let (x, y, h) = self.caret_box(self.caret);
        self.ime = Some(Rect::new(rect.x + PAD + x, rect.y + PAD + y - off, 1.0, h));
    }
}

/// The checklist box (`input[type=checkbox]` of tab-note.css): 15 px, radius 5, 1.5 px dim border; ticked = accent fill with a dark check.
fn draw_box(cv: &Canvas, th: &Theme, r: Rect, on: bool) {
    if on {
        cv.fill_rr(r, 5.0, th.accent);
        let (s, ox, oy) = (11.0 / 24.0, r.cx() - 5.5, r.cy() - 5.5);
        let p = |x: f32, y: f32| (ox + x * s, oy + y * s);
        let ink = Color::rgb8(0x0b, 0x0b, 0x14);
        cv.line(p(5.0, 12.5), p(9.5, 17.0), 3.5 * s, ink);
        cv.line(p(9.5, 17.0), p(19.0, 7.5), 3.5 * s, ink);
    } else {
        cv.stroke_rr(r, 5.0, 1.5, th.dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::view::Services;
    use glasskit::input::Buttons;
    use glasskit::testkit::Harness;

    thread_local! {
        static H: Harness = Harness::new();
    }

    pub fn key(code: KeyCode, mods: Modifiers) -> InputEvent {
        InputEvent::Key { code, mods, down: true, repeat: false }
    }
    pub fn typ(s: &str) -> Vec<InputEvent> {
        s.chars().map(InputEvent::Char).collect()
    }
    fn down(x: f32, y: f32, clicks: u8, mods: Modifiers) -> InputEvent {
        InputEvent::MouseDown { pos: (x, y), button: MouseButton::Left, mods, clicks }
    }
    fn mv(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE.with(MouseButton::Left) }
    }
    fn up(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseUp { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE }
    }

    pub const RECT: Rect = Rect { x: 0.0, y: 0.0, w: 420.0, h: 200.0 };

    /// Draw one frame of the editor over a headless canvas.
    pub fn frame(ed: &mut Editor, cx: &Cx) {
        let th = Theme::pill_dark();
        H.with(|h| {
            h.render(420, 200, 1.0, th.bg, |cv| ed.draw(cv, &th, RECT, Some("Write here"), cx));
        });
    }

    pub fn feed(ed: &mut Editor, evs: &[InputEvent], cx: &Cx) {
        for e in evs {
            ed.queue(e);
        }
        frame(ed, cx);
    }

    fn cx() -> Cx {
        let (svc, _f) = Services::fake();
        Cx::headless_with(svc)
    }

    fn focused(html: &str) -> Editor {
        let mut ed = Editor::new();
        ed.set_html(html);
        ed.set_focused(true);
        frame(&mut ed, &cx());
        ed
    }

    #[test]
    fn typing_builds_text_and_reports_one_change_batch() {
        let c = cx();
        let mut ed = focused("");
        feed(&mut ed, &typ("Привет, мир"), &c);
        assert_eq!(ed.html(), "Привет, мир");
        let out = ed.take_out();
        assert_eq!(out.iter().filter(|o| **o == EdOut::Changed).count(), 1);
        assert_eq!(ed.caret(), Pos::new(0, 11));
    }

    #[test]
    fn enter_backspace_and_line_merging() {
        let c = cx();
        let mut ed = focused("");
        feed(&mut ed, &typ("ab"), &c);
        feed(&mut ed, &[key(KeyCode::Enter, Modifiers::NONE)], &c);
        feed(&mut ed, &typ("cd"), &c);
        assert_eq!(ed.html(), "ab<div>cd</div>");
        feed(&mut ed, &[key(KeyCode::Home, Modifiers::NONE), key(KeyCode::Backspace, Modifiers::NONE)], &c);
        assert_eq!(ed.html(), "abcd");
        feed(&mut ed, &[key(KeyCode::Enter, Modifiers::SHIFT)], &c);
        assert_eq!(ed.doc.lines.len(), 1, "shift+enter is a soft break");
        assert!(ed.html().contains("<br>"));
    }

    #[test]
    fn selection_keys_and_replace() {
        let c = cx();
        let mut ed = focused("hello world");
        feed(&mut ed, &[key(KeyCode::Home, Modifiers::NONE), key(KeyCode::Right, Modifiers::CTRL_SHIFT)], &c);
        assert_eq!(ed.selected_text(), "hello ");
        feed(&mut ed, &typ("X"), &c);
        assert_eq!(ed.html(), "Xworld");
        feed(&mut ed, &[key(KeyCode::Char('a'), Modifiers::CTRL)], &c);
        assert_eq!(ed.selected_text(), "Xworld");
    }

    #[test]
    fn formatting_toggles_on_selection_and_as_pending_style() {
        let c = cx();
        let mut ed = focused("one two");
        ed.set_selection(Pos::new(0, 0), Pos::new(0, 3));
        ed.exec("bold");
        assert_eq!(ed.html(), "<b>one</b> two");
        ed.exec("bold");
        assert_eq!(ed.html(), "one two");
        // pending style: toggle at the caret, then type
        ed.set_selection(Pos::new(0, 7), Pos::new(0, 7));
        ed.exec("italic");
        feed(&mut ed, &typ("!!"), &c);
        assert_eq!(ed.html(), "one two<i>!!</i>");
        ed.set_selection(Pos::new(0, 4), Pos::new(0, 7));
        ed.exec("highlight");
        ed.exec("strikeThrough");
        assert!(ed.html().contains("<s><mark>two</mark></s>"), "{}", ed.html());
        // caret inside a highlight, toggling removes the whole run
        ed.set_selection(Pos::new(0, 5), Pos::new(0, 5));
        ed.exec("highlight");
        assert!(!ed.html().contains("mark"), "{}", ed.html());
        // caret state follows the caret
        feed(&mut ed, &[], &c);
        let out = ed.take_out();
        assert!(out.iter().any(|o| matches!(o, EdOut::Caret(_))));
    }

    #[test]
    fn list_commands_and_enter_rules() {
        let c = cx();
        let mut ed = focused("a<div>b</div>");
        ed.set_selection(Pos::new(0, 0), Pos::new(1, 1));
        ed.exec("insertUnorderedList");
        assert_eq!(ed.html(), "<ul><li>a</li><li>b</li></ul>");
        ed.exec("insertUnorderedList");
        assert_eq!(ed.html(), "a<div>b</div>");
        ed.exec("insertOrderedList");
        assert_eq!(ed.html(), "<ol><li>a</li><li>b</li></ol>");
        // Enter on an empty item ends the list
        ed.set_selection(Pos::new(1, 1), Pos::new(1, 1));
        feed(&mut ed, &[key(KeyCode::Enter, Modifiers::NONE)], &c);
        assert_eq!(ed.doc.lines.len(), 3);
        feed(&mut ed, &[key(KeyCode::Enter, Modifiers::NONE)], &c);
        assert_eq!(ed.html(), "<ol><li>a</li><li>b</li></ol><div><br></div>");
        // Backspace at the start of an item makes it plain
        let mut ed = focused("<ul><li>x</li></ul>");
        feed(&mut ed, &[key(KeyCode::Backspace, Modifiers::NONE)], &c);
        assert_eq!(ed.html(), "x");
    }

    #[test]
    fn checklist_continues_ends_and_toggles_by_click() {
        let c = cx();
        let mut ed = focused("");
        ed.exec("checklist");
        feed(&mut ed, &typ("milk"), &c);
        assert_eq!(ed.html(), "<input type=\"checkbox\">&nbsp;milk");
        feed(&mut ed, &[key(KeyCode::Enter, Modifiers::NONE)], &c);
        assert_eq!(ed.doc.lines[1].kind, Kind::Check(false));
        feed(&mut ed, &[key(KeyCode::Enter, Modifiers::NONE)], &c);
        assert_eq!(ed.doc.lines[1].kind, Kind::Plain, "Enter on an empty checklist line ends the list");
        // a click on the first box ticks it
        ed.take_out();
        feed(&mut ed, &[down(PAD + 6.0, PAD + 10.0, 1, Modifiers::NONE), up(PAD + 6.0, PAD + 10.0)], &c);
        assert_eq!(ed.doc.lines[0].kind, Kind::Check(true));
        assert!(ed.take_out().contains(&EdOut::Changed));
        assert!(ed.html().starts_with("<input type=\"checkbox\" checked=\"\">"));
    }

    #[test]
    fn undo_redo_and_typing_coalescing() {
        let c = cx();
        let mut ed = focused("");
        feed(&mut ed, &typ("abc"), &c);
        feed(&mut ed, &[key(KeyCode::Enter, Modifiers::NONE)], &c);
        feed(&mut ed, &typ("def"), &c);
        assert_eq!(ed.html(), "abc<div>def</div>");
        feed(&mut ed, &[key(KeyCode::Char('z'), Modifiers::CTRL)], &c);
        assert_eq!(ed.html(), "abc<div></div>".replace("<div></div>", "<div><br></div>"), "one undo removes the whole typed word");
        feed(&mut ed, &[key(KeyCode::Char('z'), Modifiers::CTRL)], &c);
        assert_eq!(ed.html(), "abc");
        feed(&mut ed, &[key(KeyCode::Char('y'), Modifiers::CTRL)], &c);
        assert_eq!(ed.html(), "abc<div><br></div>");
        feed(&mut ed, &[key(KeyCode::Char('z'), Modifiers::CTRL_SHIFT)], &c);
        assert_eq!(ed.html(), "abc<div>def</div>", "Ctrl+Shift+Z redoes");
    }

    #[test]
    fn clipboard_copy_cut_paste() {
        let (svc, _f) = Services::fake();
        let c = Cx::headless_with(svc);
        let mut ed = focused("one two");
        ed.set_selection(Pos::new(0, 0), Pos::new(0, 3));
        feed(&mut ed, &[key(KeyCode::Char('c'), Modifiers::CTRL)], &c);
        assert_eq!(c.clipboard_text().as_deref(), Some("one"));
        feed(&mut ed, &[key(KeyCode::Char('x'), Modifiers::CTRL)], &c);
        assert_eq!(ed.html(), " two");
        ed.set_selection(Pos::new(0, 4), Pos::new(0, 4));
        c.set_clipboard_text("l1\nl2 & <b>");
        feed(&mut ed, &[key(KeyCode::Char('v'), Modifiers::CTRL)], &c);
        assert_eq!(ed.html(), " twol1<div>l2 &amp; &lt;b&gt;</div>", "paste is plain text, lines become lines, markup is escaped");
    }

    #[test]
    fn mouse_click_double_triple_and_drag() {
        let c = cx();
        let mut ed = focused("alpha beta<div>second line</div>");
        let (x, y) = (PAD + 4.0, PAD + 10.0);
        feed(&mut ed, &[down(x, y, 2, Modifiers::NONE), up(x, y)], &c);
        assert_eq!(ed.selected_text(), "alpha");
        feed(&mut ed, &[down(x, y, 3, Modifiers::NONE), up(x, y)], &c);
        assert_eq!(ed.selected_text(), "alpha beta");
        feed(&mut ed, &[down(PAD + 1.0, PAD + 10.0, 1, Modifiers::NONE), mv(PAD + 200.0, PAD + 30.0), up(PAD + 200.0, PAD + 30.0)], &c);
        assert!(ed.selected_text().starts_with("alpha beta\nsecond"), "{:?}", ed.selected_text());
    }

    #[test]
    fn arrow_navigation_across_lines() {
        let c = cx();
        let mut ed = focused("ab<div>cd</div>");
        feed(&mut ed, &[key(KeyCode::Char('a'), Modifiers::CTRL), key(KeyCode::Left, Modifiers::NONE)], &c);
        assert_eq!(ed.caret(), Pos::new(0, 0));
        feed(&mut ed, &[key(KeyCode::Down, Modifiers::NONE)], &c);
        assert_eq!(ed.caret().line, 1);
        feed(&mut ed, &[key(KeyCode::Up, Modifiers::NONE)], &c);
        assert_eq!(ed.caret().line, 0);
        feed(&mut ed, &[key(KeyCode::End, Modifiers::CTRL)], &c);
        assert_eq!(ed.caret(), Pos::new(1, 2));
        feed(&mut ed, &[key(KeyCode::Left, Modifiers::NONE), key(KeyCode::Left, Modifiers::NONE), key(KeyCode::Left, Modifiers::NONE)], &c);
        assert_eq!(ed.caret(), Pos::new(0, 2), "left wraps to the end of the previous line");
    }

    #[test]
    fn untouched_note_reports_nothing() {
        let c = cx();
        let mut ed = focused("<b>x</b><div>y</div><ul><li>z</li></ul>");
        feed(&mut ed, &[key(KeyCode::Right, Modifiers::NONE), key(KeyCode::Down, Modifiers::NONE), key(KeyCode::Char('a'), Modifiers::CTRL)], &c);
        assert!(!ed.take_out().contains(&EdOut::Changed), "navigation and selection never count as an edit");
    }

    #[test]
    fn ctrl_click_on_a_link_asks_to_open_it() {
        let c = cx();
        let mut ed = focused("<a href=\"https://example.com\">example</a>");
        feed(&mut ed, &[down(PAD + 8.0, PAD + 10.0, 1, Modifiers::CTRL), up(PAD + 8.0, PAD + 10.0)], &c);
        assert!(ed.take_out().contains(&EdOut::OpenLink("https://example.com".into())));
    }

    #[test]
    fn reduced_motion_keeps_a_steady_caret_and_no_frames() {
        let mut ed = focused("x");
        for _ in 0..40 {
            ed.step(50.0);
        }
        ed.blink = 0.0;
        ed.reduced = true;
        assert!(!ed.step(16.0), "nothing animates");
        ed.reduced = false;
        assert!(ed.step(16.0), "the caret blink window asks for frames");
        for _ in 0..400 {
            ed.step(50.0);
        }
        assert!(!ed.step(16.0), "after the blink window the caret stays solid and the editor idles");
    }
}
