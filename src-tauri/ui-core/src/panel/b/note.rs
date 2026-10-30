//! Note tab (src/tab-note.js): note chips + rich-text editor with autosave on the left, the timer/reminder card on the right.
//!
//! Backend (notes.rs): `note_list {folder_id}`, `note_get {id}`, `note_save {id, body_html, title}` (id null creates),
//! `note_delete {id}`. The body is whitelisted HTML (`note_html::sanitize`); the renderer's editor reports its HTML with
//! `Action::EditorChanged`, and formatting buttons come back as `Effect::EditorExec("bold" | "insertUnorderedList" | ...)` /
//! `Effect::SetEditorHtml`. The editor then reports its new HTML, which is what starts the 400 ms autosave debounce.
//!
//! Behaviour kept from the JS: autosave 400 ms after the last edit, save on blur / tab hide / page hide, switching or creating
//! a note saves first, deleting needs two presses within 2.5 s, the last note is replaced by an empty one, the open note id is
//! remembered in `pill.note.cur`, title = first non-blank line (max 60 chars).
//! Deliberate fix: when saving fails (`note_save` error) switching to another note is *not* performed, so unsaved text is not
//! thrown away (the JS dropped it). The chip row can be filtered by title (`Action::SetFilter`); pinned notes keep the backend order
//! (pinned first) and carry `pinned: true` (the backend has no pin command yet).

use super::note_html::{first_line, is_empty, plain_text, sanitize};
use super::timer::{self, TimerCard, TimerView};
use super::{clip_chars, Anim, AnimCue, BackendCall, Effect, Flash, Fx, Kv, Lang, Reply, Tags, Timer, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const KV_CUR: &str = "pill.note.cur";
pub const AUTOSAVE_MS: u32 = 400;
pub const DELETE_ARM_MS: u32 = 2500;
pub const TITLE_MAX: usize = 60;
const OWNER: u8 = 1;
const OWNER_TIMER: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveState {
    Saved,
    Saving,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Bold,
    Italic,
    Strike,
    Mark,
    BulletList,
    NumberList,
    Checklist,
}
impl Fmt {
    /// the `document.execCommand` name the editor runs ("mark" and "checklist" are inserted as html by the editor)
    pub fn command(self) -> &'static str {
        match self {
            Fmt::Bold => "bold",
            Fmt::Italic => "italic",
            Fmt::Strike => "strikeThrough",
            Fmt::Mark => "highlight",
            Fmt::BulletList => "insertUnorderedList",
            Fmt::NumberList => "insertOrderedList",
            Fmt::Checklist => "checklist",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open(i64),
    New,
    /// first press arms (2.5 s), the second deletes
    Delete,
    /// the editor contents changed (any html; it is sanitized here)
    EditorChanged(String),
    /// the editor lost focus
    Blur,
    Format(Fmt),
    /// copy the whole note as plain text
    Copy,
    /// "Remind" button: `selection` = the selected editor text, if any
    RemindFromNote { selection: Option<String> },
    /// which formatting is active at the caret (toggle highlight of the toolbar)
    CaretState { bold: bool, italic: bool, strike: bool, mark: bool, ul: bool, ol: bool },
    SetFilter(String),
    Timer(timer::Action),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub id: i64,
    pub title: String,
    pub on: bool,
    pub pinned: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ToolView {
    pub fmt: Fmt,
    pub title: String,
    pub on: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NoteView {
    pub chips: Vec<Chip>,
    pub new_title: String,
    pub placeholder: String,
    pub show_placeholder: bool,
    pub saved: SaveState,
    pub saved_title: String,
    pub tools: Vec<ToolView>,
    pub remind_label: String,
    pub remind_title: String,
    pub copy_title: String,
    pub delete_armed: bool,
    pub delete_title: String,
    pub toast: Option<(String, bool)>,
    pub filter: String,
    pub timer: TimerView,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Then {
    None,
    Open(i64),
    New,
}
#[derive(Debug)]
enum Pend {
    List,
    Get(i64),
    Save { id: i64, then: Then },
    Create,
    Delete { index: usize },
}

#[derive(Clone, Debug)]
struct Meta {
    id: i64,
    title: String,
    pinned: bool,
}

pub struct NoteTab {
    pub fx: Fx,
    pub card: TimerCard,
    lang: Lang,
    kv: Box<dyn Kv>,
    tags: Tags,
    pend: HashMap<u64, Pend>,
    notes: Vec<Meta>,
    cur: Option<i64>,
    html: String,
    dirty: bool,
    saving: bool,
    saved: SaveState,
    save_deb: Timer,
    armed: bool,
    del_arm: Timer,
    flash: Flash,
    caret: [bool; 6],
    filter: String,
    visible: bool,
    want_rem: bool,
    reveal_in: Timer,
    focus_after_open: bool,
}

impl NoteTab {
    /// `kv_note` and `kv_timer` may be the same `SharedKv`.
    pub fn new(lang: Lang, kv_note: Box<dyn Kv>, kv_timer: Box<dyn Kv>) -> Self {
        Self {
            fx: Fx::default(), card: TimerCard::new(lang, kv_timer, OWNER_TIMER), lang, kv: kv_note, tags: Tags::new(OWNER), pend: HashMap::new(), notes: vec![], cur: None,
            html: String::new(), dirty: false, saving: false, saved: SaveState::Saved, save_deb: Timer::default(), armed: false, del_arm: Timer::default(),
            flash: Flash::default(), caret: [false; 6], filter: String::new(), visible: false, want_rem: false, reveal_in: Timer::default(), focus_after_open: false,
        }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
        self.card.set_lang(l);
    }
    pub fn set_reduce_motion(&mut self, v: bool) {
        self.fx.reduce_motion = v;
        self.card.fx.reduce_motion = v;
    }
    pub fn current(&self) -> Option<i64> {
        self.cur
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn note_html(&self) -> &str {
        &self.html
    }
    pub fn take_effects(&mut self) -> Vec<Effect> {
        let mut v = self.fx.take_effects();
        v.extend(self.card.fx.take_effects());
        v
    }
    pub fn take_anims(&mut self) -> Vec<AnimCue> {
        let mut v = self.fx.take_anims();
        v.extend(self.card.fx.take_anims());
        v
    }

    fn call(&mut self, cmd: &'static str, args: Value, p: Pend) -> BackendCall {
        let tag = self.tags.next();
        self.pend.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }
    fn say(&mut self, ru: &str, en: &str, bad: bool) {
        let m = self.lang.t(ru, en).to_string();
        self.flash.say(&m, bad, Some(2000));
    }

    /// Everything the tab does when it is built: load the notes, the timer and the reminders.
    pub fn mount(&mut self) -> Vec<BackendCall> {
        let mut v = vec![self.call("note_list", json!({ "folder_id": null }), Pend::List)];
        v.push(self.card.sync());
        v.extend(self.card.on_show());
        v
    }
    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.visible = true;
        if self.want_rem {
            self.reveal_in.arm(380);
        }
        self.card.on_show()
    }
    pub fn on_hide(&mut self) -> Vec<BackendCall> {
        self.visible = false;
        self.card.on_hide();
        self.save(Then::None)
    }
    /// `pagehide` of the window: flush a pending edit
    pub fn flush(&mut self) -> Vec<BackendCall> {
        self.save(Then::None)
    }

    // ---- save / open / new / delete ----
    fn save(&mut self, then: Then) -> Vec<BackendCall> {
        self.save_deb.cancel();
        let Some(id) = self.cur.filter(|_| self.dirty) else { return self.run_then(then) };
        let html = sanitize(&self.html);
        let title = clip_chars(&first_line(&html), TITLE_MAX);
        self.dirty = false;
        self.saving = true;
        self.saved = SaveState::Saving;
        vec![self.call("note_save", json!({ "id": id, "body_html": html, "title": title }), Pend::Save { id, then })]
    }
    fn run_then(&mut self, then: Then) -> Vec<BackendCall> {
        match then {
            Then::None => vec![],
            Then::Open(id) => vec![self.call("note_get", json!({ "id": id }), Pend::Get(id))],
            Then::New => vec![self.call("note_save", json!({ "id": null, "body_html": "", "title": "" }), Pend::Create)],
        }
    }
    fn open(&mut self, id: i64) -> Vec<BackendCall> {
        if Some(id) == self.cur {
            return vec![];
        }
        self.save(Then::Open(id))
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Open(id) => self.open(id),
            Action::New => self.save(Then::New),
            Action::Delete => {
                if self.cur.is_none() {
                    return vec![];
                }
                if !self.armed {
                    self.armed = true;
                    self.del_arm.arm(DELETE_ARM_MS);
                    self.fx.anim("note-delete", Anim::Press);
                    return vec![];
                }
                self.armed = false;
                self.del_arm.cancel();
                let id = self.cur.unwrap();
                let index = self.notes.iter().position(|n| n.id == id).unwrap_or(0);
                self.save_deb.cancel();
                self.dirty = false;
                vec![self.call("note_delete", json!({ "id": id }), Pend::Delete { index })]
            }
            Action::EditorChanged(html) => {
                self.html = html;
                self.dirty = true;
                self.saved = SaveState::Saving;
                self.save_deb.arm(AUTOSAVE_MS);
                vec![]
            }
            Action::Blur => self.save(Then::None),
            Action::Format(f) => {
                self.fx.eff(Effect::EditorExec(f.command()));
                vec![]
            }
            Action::Copy => {
                let txt = plain_text(&self.html);
                let txt = txt.trim_matches(super::text::js_ws);
                if !txt.is_empty() {
                    self.fx.eff(Effect::CopyText(txt.to_string()));
                    self.say("Скопировано", "Copied", false);
                }
                vec![]
            }
            Action::RemindFromNote { selection } => {
                let picked = selection.as_deref().map(|s| s.trim_matches(super::text::js_ws)).unwrap_or("");
                let src = if picked.is_empty() { first_line(&self.html) } else { picked.to_string() };
                let mut one = String::new();
                let mut in_ws = false;
                for c in src.chars() {
                    if super::text::js_ws(c) {
                        if !in_ws {
                            one.push(' ');
                        }
                        in_ws = true;
                    } else {
                        in_ws = false;
                        one.push(c);
                    }
                }
                let txt = clip_chars(&one, 80);
                let pre = if txt.is_empty() { String::new() } else { format!("{txt} ") };
                self.card.prefill_reminder(&pre, true);
                vec![]
            }
            Action::CaretState { bold, italic, strike, mark, ul, ol } => {
                self.caret = [bold, italic, strike, mark, ul, ol];
                vec![]
            }
            Action::SetFilter(q) => {
                self.filter = q;
                vec![]
            }
            Action::Timer(t) => self.card.on_action(t),
        }
    }

    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        if Tags::owner_of(tag) == OWNER_TIMER {
            return self.card.on_reply(tag, r);
        }
        let Some(p) = self.pend.remove(&tag) else { return vec![] };
        let good = r.ok && !r.v.is_null();
        match p {
            Pend::List => {
                if !r.ok {
                    return vec![];
                }
                self.notes = r.v.as_array().into_iter().flatten().map(|n| Meta { id: n["id"].as_i64().unwrap_or(0), title: n["title"].as_str().unwrap_or("").to_string(), pinned: n["pinned"].as_bool().unwrap_or(false) }).collect();
                if self.notes.is_empty() {
                    return self.save(Then::New);
                }
                let want = self.kv.get(KV_CUR).and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0);
                self.cur = None;
                let id = if self.notes.iter().any(|n| n.id == want) { want } else { self.notes[0].id };
                self.open(id)
            }
            Pend::Get(id) => {
                if !good {
                    return vec![];
                }
                self.cur = Some(id);
                self.kv.set(KV_CUR, &id.to_string());
                self.html = sanitize(r.v["body_html"].as_str().unwrap_or(""));
                self.dirty = false;
                self.saved = SaveState::Saved;
                self.fx.eff(Effect::SetEditorHtml(self.html.clone()));
                self.fx.eff(Effect::ScrollIntoView("note-chip"));
                if std::mem::take(&mut self.focus_after_open) {
                    self.fx.eff(Effect::Focus("note-editor"));
                }
                vec![]
            }
            Pend::Save { id, then } => {
                self.saving = false;
                if !good {
                    self.dirty = true;
                    self.saved = SaveState::Error;
                    // keep the unsaved text: stay on this note instead of switching away
                    return vec![];
                }
                let title = r.v["title"].as_str().unwrap_or("").to_string();
                if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
                    if n.title != title {
                        n.title = title;
                    }
                }
                if !self.dirty {
                    self.saved = SaveState::Saved;
                }
                self.run_then(then)
            }
            Pend::Create => {
                if !good {
                    self.say("Не удалось создать заметку", "Could not create a note", true);
                    return vec![];
                }
                let id = r.v["id"].as_i64().unwrap_or(0);
                self.notes.push(Meta { id, title: String::new(), pinned: false });
                self.fx.anim(&format!("note-chip:{id}"), Anim::Enter { y: 0.0, ms: 180 });
                self.cur = None;
                self.focus_after_open = true;
                self.open(id)
            }
            Pend::Delete { index } => {
                let id = self.cur;
                self.notes.retain(|n| Some(n.id) != id);
                self.cur = None;
                if self.notes.is_empty() {
                    return self.save(Then::New);
                }
                let next = self.notes[index.min(self.notes.len() - 1)].id;
                self.open(next)
            }
        }
    }

    pub fn on_event(&mut self, name: &str, p: &Value) -> Vec<BackendCall> {
        match name {
            "pill:hide" => {
                self.card.set_pill_hidden(true);
                vec![]
            }
            "pill:show" => {
                self.card.set_pill_hidden(false);
                vec![]
            }
            // Command bar: {tab:'note', noteId?, section?}
            "pill:open-tab" => {
                if p["tab"].as_str() != Some("note") {
                    return vec![];
                }
                if p["section"].as_str() == Some("reminders") {
                    self.want_rem = true;
                    if self.visible {
                        self.reveal_in.arm(100);
                    }
                }
                let Some(id) = p["noteId"].as_i64().or_else(|| p["noteId"].as_str().and_then(|s| s.parse().ok())).filter(|i| *i != 0) else { return vec![] };
                self.kv.set(KV_CUR, &id.to_string());
                self.open(id)
            }
            _ => self.card.on_event(name, p),
        }
    }

    pub fn tick(&mut self, dt_ms: u32) -> Vec<BackendCall> {
        let mut calls = vec![];
        if self.save_deb.tick(dt_ms) {
            calls.extend(self.save(Then::None));
        }
        if self.del_arm.tick(dt_ms) {
            self.armed = false;
        }
        if self.reveal_in.tick(dt_ms) {
            self.want_rem = false;
            self.card.reveal();
        }
        self.flash.tick(dt_ms);
        calls.extend(self.card.tick(dt_ms));
        calls
    }

    pub fn view(&self) -> NoteView {
        let l = self.lang;
        let q = self.filter.trim().to_lowercase();
        let chips = self
            .notes
            .iter()
            .filter(|n| q.is_empty() || Some(n.id) == self.cur || n.title.to_lowercase().contains(&q))
            .map(|n| Chip { id: n.id, title: if n.title.is_empty() { l.t("Заметка", "Note").into() } else { n.title.clone() }, on: Some(n.id) == self.cur, pinned: n.pinned })
            .collect();
        let on = |i: usize| self.caret[i];
        let tool = |fmt: Fmt, ru: &str, en: &str, on: bool| ToolView { fmt, title: l.t(ru, en).into(), on };
        NoteView {
            chips,
            new_title: l.t("Новая заметка", "New note").into(),
            placeholder: l.t("Пиши сюда что угодно: номер заказа, список дел, кусок кода. Сохраняется само.", "Write anything: order number, to-do list, a piece of code. Saves itself.").into(),
            show_placeholder: is_empty(&self.html),
            saved: self.saved,
            saved_title: match self.saved {
                SaveState::Saved => l.t("Сохранено", "Saved"),
                SaveState::Saving => l.t("Сохраняется…", "Saving…"),
                SaveState::Error => l.t("Не удалось сохранить", "Could not save"),
            }
            .into(),
            tools: vec![
                tool(Fmt::Bold, "Жирный (Ctrl+B)", "Bold (Ctrl+B)", on(0)),
                tool(Fmt::Italic, "Курсив (Ctrl+I)", "Italic (Ctrl+I)", on(1)),
                tool(Fmt::Strike, "Зачёркнутый", "Strikethrough", on(2)),
                tool(Fmt::Mark, "Выделить маркером", "Highlight", on(3)),
                tool(Fmt::BulletList, "Список", "Bullet list", on(4)),
                tool(Fmt::NumberList, "Нумерованный список", "Numbered list", on(5)),
                tool(Fmt::Checklist, "Чек-лист", "Checklist", false),
            ],
            remind_label: l.t("Напомнить", "Remind").into(),
            remind_title: l.t("Создать напоминание из заметки", "Create a reminder from this note").into(),
            copy_title: l.t("Копировать текст", "Copy text").into(),
            delete_armed: self.armed,
            delete_title: if self.armed { l.t("Нажми ещё раз, чтобы удалить", "Click again to delete") } else { l.t("Удалить заметку", "Delete note") }.into(),
            toast: self.flash.visible().then(|| (self.flash.text.clone(), self.flash.bad)),
            filter: self.filter.clone(),
            timer: self.card.view(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::b::{MemKv, SharedKv};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// In-memory notes backend with a switch to make `note_save` fail.
    #[derive(Default)]
    struct Fake {
        notes: Vec<(i64, String, String)>, // id, title, html
        next: i64,
        fail_save: bool,
        log: Vec<String>,
    }
    type F = Rc<RefCell<Fake>>;

    fn answer(f: &F, c: &BackendCall) -> Option<Reply> {
        let mut f = f.borrow_mut();
        f.log.push(c.cmd.to_string());
        Some(match c.cmd {
            "note_list" => Reply::ok(Value::Array(f.notes.iter().map(|(id, t, _)| json!({ "id": id, "title": t, "pinned": false })).collect())),
            "note_get" => {
                let id = c.args["id"].as_i64().unwrap();
                match f.notes.iter().find(|n| n.0 == id) {
                    Some((id, t, h)) => Reply::ok(json!({ "id": id, "title": t, "body_html": h })),
                    None => Reply::ok(Value::Null),
                }
            }
            "note_save" => {
                if f.fail_save {
                    return Some(Reply::err("store"));
                }
                let (html, title) = (c.args["body_html"].as_str().unwrap().to_string(), c.args["title"].as_str().unwrap().to_string());
                match c.args["id"].as_i64() {
                    Some(id) => {
                        let n = f.notes.iter_mut().find(|n| n.0 == id).unwrap();
                        n.1 = title.clone();
                        n.2 = html;
                        Reply::ok(json!({ "id": id, "title": title }))
                    }
                    None => {
                        f.next += 1;
                        let id = f.next;
                        f.notes.push((id, title.clone(), html));
                        Reply::ok(json!({ "id": id, "title": title }))
                    }
                }
            }
            "note_delete" => {
                let id = c.args["id"].as_i64().unwrap();
                f.notes.retain(|n| n.0 != id);
                Reply::ok(Value::Null)
            }
            "timer_state" => Reply::ok(json!({ "active": false })),
            "remind_list" => Reply::ok(json!([])),
            _ => return None,
        })
    }
    /// Run calls to completion, feeding every reply back (follow-up calls included).
    fn drive(t: &mut NoteTab, f: &F, calls: Vec<BackendCall>) {
        let mut q = calls;
        let mut guard = 0;
        while !q.is_empty() {
            guard += 1;
            assert!(guard < 50, "runaway call chain");
            let c = q.remove(0);
            if let Some(r) = answer(f, &c) {
                q.extend(t.on_reply(c.tag, r));
            }
        }
    }
    fn setup(notes: &[(&str, &str)]) -> (NoteTab, F, SharedKv) {
        let f: F = Rc::new(RefCell::new(Fake::default()));
        for (t, h) in notes {
            let mut b = f.borrow_mut();
            b.next += 1;
            let id = b.next;
            b.notes.push((id, t.to_string(), h.to_string()));
        }
        let kv = SharedKv::new(MemKv::default());
        let mut t = NoteTab::new(Lang::En, Box::new(kv.clone()), Box::new(kv.clone()));
        let calls = t.mount();
        drive(&mut t, &f, calls);
        (t, f, kv)
    }
    fn saves(f: &F) -> usize {
        f.borrow().log.iter().filter(|c| *c == "note_save").count()
    }

    #[test]
    fn mount_opens_first_note_or_creates_one() {
        let (t, _, _) = setup(&[("one", "<div>one</div>"), ("two", "<div>two</div>")]);
        assert_eq!(t.current(), Some(1));
        assert_eq!(t.view().chips.len(), 2);
        assert!(t.view().chips[0].on && !t.view().chips[1].on);
        let (mut t, f, _) = setup(&[]);
        assert_eq!(t.current(), Some(1), "an empty store gets one blank note");
        assert_eq!(f.borrow().notes.len(), 1);
        assert_eq!(t.view().chips[0].title, "Note", "untitled note chip");
        assert!(t.view().show_placeholder);
        assert!(t.take_effects().contains(&Effect::Focus("note-editor")));
    }
    #[test]
    fn remembers_the_open_note() {
        let (mut t, f, kv) = setup(&[("a", ""), ("b", "")]);
        let c = t.on_action(Action::Open(2));
        drive(&mut t, &f, c);
        assert_eq!(kv.get(KV_CUR).as_deref(), Some("2"));
        let mut t2 = NoteTab::new(Lang::En, Box::new(kv.clone()), Box::new(kv));
        let c = t2.mount();
        drive(&mut t2, &f, c);
        assert_eq!(t2.current(), Some(2));
    }
    #[test]
    fn autosave_is_debounced_400ms() {
        let (mut t, f, _) = setup(&[("a", "")]);
        let base = saves(&f);
        t.on_action(Action::EditorChanged("<div>h</div>".into()));
        assert_eq!(t.view().saved, SaveState::Saving);
        assert!(t.tick(399).is_empty());
        t.on_action(Action::EditorChanged("<div>he</div>".into())); // typing again restarts the wait
        assert!(t.tick(399).is_empty());
        let c = t.tick(1);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].args["body_html"], json!("<div>he</div>"));
        assert_eq!(c[0].args["title"], json!("he"));
        drive(&mut t, &f, c);
        assert_eq!(saves(&f), base + 1);
        assert_eq!(t.view().saved, SaveState::Saved);
        assert!(!t.is_dirty());
        assert!(t.tick(5000).is_empty(), "nothing more to save");
    }
    #[test]
    fn saved_body_is_sanitized_and_title_is_clipped() {
        let (mut t, f, _) = setup(&[("a", "")]);
        let long = "я".repeat(80);
        t.on_action(Action::EditorChanged(format!("<h1 onclick=x>{long}</h1><script>bad()</script>")));
        let c = t.tick(400);
        assert_eq!(c[0].args["body_html"], json!(format!("<div>{long}</div>")));
        assert_eq!(c[0].args["title"].as_str().unwrap().chars().count(), 60);
        drive(&mut t, &f, c);
        assert_eq!(t.view().chips[0].title.chars().count(), 60, "the chip takes the title the backend returned");
    }
    #[test]
    fn switching_notes_saves_first_and_loads_the_other() {
        let (mut t, f, _) = setup(&[("a", "<div>a</div>"), ("b", "<div>b</div>")]);
        t.on_action(Action::EditorChanged("<div>a2</div>".into()));
        let c = t.on_action(Action::Open(2));
        assert_eq!(c[0].cmd, "note_save");
        drive(&mut t, &f, c);
        assert_eq!(f.borrow().notes[0].2, "<div>a2</div>");
        assert_eq!(t.current(), Some(2));
        assert_eq!(t.note_html(), "<div>b</div>");
        assert!(t.take_effects().contains(&Effect::SetEditorHtml("<div>b</div>".into())));
        assert!(t.on_action(Action::Open(2)).is_empty(), "opening the open note is a no-op");
    }
    #[test]
    fn failed_save_keeps_unsaved_text_and_does_not_switch() {
        let (mut t, f, _) = setup(&[("a", "<div>a</div>"), ("b", "")]);
        t.on_action(Action::EditorChanged("<div>precious</div>".into()));
        f.borrow_mut().fail_save = true;
        let c = t.on_action(Action::Open(2));
        drive(&mut t, &f, c);
        assert_eq!(t.current(), Some(1));
        assert_eq!(t.view().saved, SaveState::Error);
        assert!(t.is_dirty() && t.note_html() == "<div>precious</div>");
        f.borrow_mut().fail_save = false;
        let c = t.tick(0);
        assert!(c.is_empty());
        let c = t.on_action(Action::Blur);
        drive(&mut t, &f, c);
        assert_eq!(t.view().saved, SaveState::Saved);
        assert_eq!(f.borrow().notes[0].2, "<div>precious</div>");
    }
    #[test]
    fn edit_during_a_save_stays_dirty() {
        let (mut t, f, _) = setup(&[("a", "")]);
        t.on_action(Action::EditorChanged("<div>1</div>".into()));
        let c = t.tick(400);
        t.on_action(Action::EditorChanged("<div>12</div>".into())); // typed while the save is in flight
        let r = answer(&f, &c[0]).unwrap();
        t.on_reply(c[0].tag, r);
        assert!(t.is_dirty());
        assert_eq!(t.view().saved, SaveState::Saving);
        let c = t.tick(400);
        assert_eq!(c[0].args["body_html"], json!("<div>12</div>"));
    }
    #[test]
    fn new_note_saves_current_creates_and_focuses() {
        let (mut t, f, _) = setup(&[("a", "")]);
        t.on_action(Action::EditorChanged("<div>draft</div>".into()));
        let c = t.on_action(Action::New);
        drive(&mut t, &f, c);
        assert_eq!(f.borrow().notes.len(), 2);
        assert_eq!(f.borrow().notes[0].2, "<div>draft</div>");
        assert_eq!(t.current(), Some(2));
        assert_eq!(t.view().chips.len(), 2);
        assert!(t.take_effects().contains(&Effect::Focus("note-editor")));
        assert!(t.take_anims().iter().any(|a| a.target == "note-chip:2"));
    }
    #[test]
    fn delete_needs_two_presses_and_disarms_after_2500ms() {
        let (mut t, f, _) = setup(&[("a", ""), ("b", ""), ("c", "")]);
        let c = t.on_action(Action::Open(2));
        drive(&mut t, &f, c);
        assert!(t.on_action(Action::Delete).is_empty());
        assert!(t.view().delete_armed);
        assert_eq!(t.view().delete_title, "Click again to delete");
        t.tick(2499);
        assert!(t.view().delete_armed);
        t.tick(1);
        assert!(!t.view().delete_armed);
        assert_eq!(t.view().delete_title, "Delete note");
        t.on_action(Action::Delete);
        let c = t.on_action(Action::Delete);
        assert_eq!(c[0].cmd, "note_delete");
        drive(&mut t, &f, c);
        assert_eq!(f.borrow().notes.len(), 2);
        assert_eq!(t.current(), Some(3), "the note that took its place opens");
        assert_eq!(t.view().chips.len(), 2);
    }
    #[test]
    fn deleting_the_last_note_leaves_a_blank_one() {
        let (mut t, f, _) = setup(&[("only", "<div>only</div>")]);
        t.on_action(Action::Delete);
        let c = t.on_action(Action::Delete);
        drive(&mut t, &f, c);
        assert_eq!(f.borrow().notes.len(), 1);
        assert_eq!(f.borrow().notes[0].2, "");
        assert_eq!(t.note_html(), "");
    }
    #[test]
    fn deleting_discards_the_pending_autosave() {
        let (mut t, f, _) = setup(&[("a", ""), ("b", "")]);
        t.on_action(Action::EditorChanged("<div>x</div>".into()));
        t.on_action(Action::Delete);
        let c = t.on_action(Action::Delete);
        let before = saves(&f);
        drive(&mut t, &f, c);
        assert!(t.tick(1000).iter().all(|c| c.cmd != "note_save"));
        assert_eq!(saves(&f), before);
    }
    #[test]
    fn blur_hide_and_pagehide_flush() {
        let (mut t, f, _) = setup(&[("a", "")]);
        for flush in 0..3 {
            t.on_action(Action::EditorChanged(format!("<div>{flush}</div>")));
            let c = match flush {
                0 => t.on_action(Action::Blur),
                1 => t.on_hide(),
                _ => t.flush(),
            };
            assert_eq!(c.len(), 1, "flush path {flush}");
            drive(&mut t, &f, c);
        }
        assert!(t.on_hide().is_empty(), "clean note: nothing to write");
    }
    #[test]
    fn copy_remind_and_formatting() {
        let (mut t, _, _) = setup(&[("a", "")]);
        t.on_action(Action::EditorChanged("<div>Buy  milk</div><div>and eggs</div>".into()));
        t.take_effects();
        t.on_action(Action::Copy);
        assert_eq!(t.take_effects(), vec![Effect::CopyText("Buy  milk\nand eggs".into())]);
        assert_eq!(t.view().toast, Some(("Copied".into(), false)));
        t.tick(2000);
        assert_eq!(t.view().toast, None);
        t.on_action(Action::RemindFromNote { selection: None });
        assert_eq!(t.view().timer.input, "Buy milk ");
        t.on_action(Action::RemindFromNote { selection: Some("  call\n mum  ".into()) });
        assert_eq!(t.view().timer.input, "call mum ");
        t.on_action(Action::RemindFromNote { selection: Some("я".repeat(100)) });
        assert_eq!(t.view().timer.input.chars().count(), 81);
        t.take_effects();
        t.on_action(Action::Format(Fmt::Bold));
        t.on_action(Action::Format(Fmt::BulletList));
        assert_eq!(t.take_effects(), vec![Effect::EditorExec("bold"), Effect::EditorExec("insertUnorderedList")]);
        t.on_action(Action::CaretState { bold: true, italic: false, strike: false, mark: false, ul: false, ol: true });
        let v = t.view();
        assert!(v.tools[0].on && !v.tools[1].on && v.tools[5].on);
        assert_eq!(v.tools[0].title, "Bold (Ctrl+B)");
    }
    #[test]
    fn empty_note_does_not_copy() {
        let (mut t, _, _) = setup(&[("a", "")]);
        t.take_effects();
        t.on_action(Action::Copy);
        assert!(t.take_effects().is_empty());
    }
    #[test]
    fn filter_keeps_the_open_note_visible() {
        let (mut t, _, _) = setup(&[("Shopping", ""), ("Ideas", ""), ("Работа", "")]);
        t.on_action(Action::SetFilter("ide".into()));
        let ids: Vec<i64> = t.view().chips.iter().map(|c| c.id).collect();
        assert_eq!(ids, vec![1, 2], "open note #1 stays, #2 matches");
        t.on_action(Action::SetFilter("РАБ".into()));
        assert_eq!(t.view().chips.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 3]);
    }
    #[test]
    fn open_tab_event_selects_note_and_reveals_reminders() {
        let (mut t, f, kv) = setup(&[("a", ""), ("b", "")]);
        let c = t.on_event("pill:open-tab", &json!({ "tab": "note", "noteId": 2 }));
        drive(&mut t, &f, c);
        assert_eq!(t.current(), Some(2));
        assert_eq!(kv.get(KV_CUR).as_deref(), Some("2"));
        assert!(t.on_event("pill:open-tab", &json!({ "tab": "home", "noteId": 1 })).is_empty());
        // section: reminders before the tab is shown -> revealed 380 ms after it appears
        t.on_event("pill:open-tab", &json!({ "tab": "note", "section": "reminders" }));
        t.take_effects();
        let c = t.on_show();
        drive(&mut t, &f, c);
        t.tick(379);
        assert!(!t.take_effects().contains(&Effect::Focus("reminder-input")));
        t.tick(1);
        let fx = t.take_effects();
        assert!(fx.contains(&Effect::ScrollIntoView("reminders")) && fx.contains(&Effect::Focus("reminder-input")));
    }
    #[test]
    fn timer_card_is_routed_by_tag_owner() {
        let (mut t, f, _) = setup(&[("a", "")]);
        let c = t.on_action(Action::Timer(timer::Action::StartStop));
        assert_eq!(c[0].cmd, "timer_start");
        assert_eq!(Tags::owner_of(c[0].tag), 2);
        t.set_reduce_motion(true);
        let r = Reply::ok(json!({ "active": true, "id": 1, "due_at": 60_000, "total_s": 60 }));
        t.on_reply(c[0].tag, r);
        assert!(t.view().timer.running);
        drive(&mut t, &f, vec![]);
        // events not handled by the note tab go to the card
        let calls = t.on_event("remind:changed", &Value::Null);
        assert_eq!(calls.iter().map(|c| c.cmd).collect::<Vec<_>>(), vec!["remind_list", "timer_state"]);
    }
    #[test]
    fn a_vanished_note_does_not_break_open() {
        let (mut t, f, _) = setup(&[("a", ""), ("b", "")]);
        f.borrow_mut().notes.remove(1);
        let c = t.on_action(Action::Open(2));
        drive(&mut t, &f, c);
        assert_eq!(t.current(), Some(1), "note_get returned null: stay where we were");
    }
}
