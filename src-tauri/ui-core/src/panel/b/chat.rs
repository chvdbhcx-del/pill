//! Chat tab (src/tab-chat.js + the protocol of chat.rs): sessions, messages, streaming, composer, attachments, history popover,
//! AI-settings popover (`chat_settings.rs`), off / not-configured states.
//!
//! Protocol (unchanged): `chat_send {thread, text, model:null}` -> `{ok, thread, run, user?, error?}`; `chat_retry {thread, model}`;
//! `chat_stop {thread}`; `chat_threads`, `chat_messages {thread}`, `chat_new`, `chat_delete {thread}`, `chat_open_link {url}`;
//! events `chat:delta {thread, run, text}` and `chat:done {thread, run, ok, stopped, error, id}`. A delta/done that arrives before
//! `chat_send` has returned its `run` id is parked (`early`) and replayed afterwards. After `chat:done` the saved thread is
//! reloaded (`chat_messages`), which replaces the streamed view with the stored answer; a failed run then shows the error banner
//! with Retry.
//!
//! Streaming: network chunks go to a backlog (`acc`); every frame (`tick(dt)`) reveals ~1/6 of what is left, time-scaled so 60
//! and 144 Hz feel the same (`1 - (5/6)^(dt/16.7)`), at least one char, dt clamped to 1..100 ms, the whole backlog at once with
//! Reduce motion. Reveal is by `char`, and never ends inside an emoji sequence (ZWJ / variation selector / skin tone / flag pair /
//! combining mark), so the text the renderer shows is always valid and never shows half an emoji. `Utf8Assembler` turns raw byte
//! chunks (if the host streams bytes) into complete chars.
//! Scroll: the renderer reports the user's scroll position (`Action::Scrolled`); a growing answer keeps following only while the
//! user is within 70 px of the bottom (`Effect::ScrollToBottom`), a (re)loaded thread always jumps to the bottom.
//! Native additions: input history (Up/Down in the composer recalls what was sent), `Action::CopyMessage` flashes a check for 1.2 s.
//! Animations (data): message enter (y 6, 180 ms), typing dots, streaming caret, error/file banner expand/collapse, history row
//! collapse, press feedback, ghosts of a message that is fading out (retry).

use super::chat_settings::{AiCfg, AiSettings, SettingsView};
use super::timer::wall;
use super::{Anim, AnimCue, BackendCall, Effect, Fx, Lang, Reply, Tags, Timer, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

const OWNER: u8 = 4;
const OWNER_SET: u8 = 5;
pub const MAX_FILES: usize = 5;
pub const MAX_FILE_BYTES: usize = 200 * 1024;
pub const NEAR_BOTTOM_PX: f32 = 70.0;
/// composer grows to this height (px) and then scrolls
pub const COMPOSER_MAX_H: f32 = 96.0;
pub const TAB_HEIGHT: f32 = 417.0;
const INPUT_HISTORY_MAX: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub id: i64,
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PickedFile {
    pub name: String,
    pub size: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// composer text changed
    Input(String),
    /// Enter in the composer (no Shift, no IME composition)
    Send,
    /// the round button: Stop while an answer streams, otherwise Send
    SendButton,
    /// an "ideas" chip
    SendText(String),
    Stop,
    NewChat,
    Retry,
    /// copy the answer with this index in `view().messages`
    CopyMessage(usize),
    /// a code block's Copy button (the markdown layouter knows the code)
    CopyText(String),
    OpenLink(String),
    Attach(Vec<PickedFile>),
    RemoveFile(usize),
    DismissError,
    OpenHistory,
    PickThread(i64),
    DeleteThread(i64),
    /// a click outside the history popover and the tools row
    ClickOutside,
    OpenSettings,
    Settings(super::chat_settings::SAction),
    /// the "Enable" button of the off screen
    EnableAi,
    Mic,
    HistoryUp,
    HistoryDown,
    /// the user scrolled the message list (px)
    Scrolled { offset: f32, viewport: f32, content: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MsgView {
    pub id: i64,
    pub role: Role,
    /// raw text (the kit's markdown layouter draws it); for the streaming answer only the revealed part
    pub text: String,
    pub streaming: bool,
    /// three-dot indicator, no text revealed yet
    pub typing: bool,
    /// copy / answer-again buttons (last stored answer, idle)
    pub actions: bool,
    /// copied check shown on the copy button
    pub copied: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub text: String,
    pub left_ms: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ErrView {
    pub text: String,
    pub retry: bool,
    pub retry_label: String,
    pub settings_label: String,
    pub dismiss_title: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HistRow {
    pub id: i64,
    pub title: String,
    pub date: String,
    pub selected: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct HistView {
    pub open: bool,
    pub new_label: String,
    pub empty_text: Option<String>,
    pub rows: Vec<HistRow>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct EmptyView {
    pub title: String,
    pub sub: String,
    pub ideas: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct OffView {
    pub title: String,
    pub body: String,
    pub enable_label: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ChatView {
    pub messages: Vec<MsgView>,
    pub ghosts: Vec<Ghost>,
    pub empty: Option<EmptyView>,
    pub off: Option<OffView>,
    pub error: Option<ErrView>,
    pub files: Vec<String>,
    pub remove_file_title: String,
    pub input: String,
    pub placeholder: String,
    pub busy: bool,
    pub send_title: String,
    pub mic_title: String,
    pub mic_recording: bool,
    pub attach_title: String,
    pub new_chat_title: String,
    pub history_title: String,
    pub settings_title: String,
    pub history: HistView,
    pub settings: SettingsView,
    pub near_bottom: bool,
    pub model: String,
}

#[derive(Debug)]
enum Pend {
    Send { from_input: bool },
    Retry,
    Load { err: Option<(String, bool)>, close_hist: bool },
    Threads,
    FirstThread,
    Other,
}

/// Raw bytes in, complete chars out; an incomplete UTF-8 tail waits for the next chunk. Invalid bytes become U+FFFD.
#[derive(Default, Debug)]
pub struct Utf8Assembler(Vec<u8>);
impl Utf8Assembler {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.0.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.0) {
                Ok(s) => {
                    out.push_str(s);
                    self.0.clear();
                    return out;
                }
                Err(e) => {
                    let n = e.valid_up_to();
                    out.push_str(std::str::from_utf8(&self.0[..n]).unwrap_or(""));
                    match e.error_len() {
                        None => {
                            self.0.drain(..n);
                            return out; // incomplete tail: wait
                        }
                        Some(l) => {
                            out.push('\u{FFFD}');
                            self.0.drain(..n + l);
                        }
                    }
                }
            }
        }
    }
    /// Stream ended: a dangling partial sequence becomes one U+FFFD.
    pub fn finish(&mut self) -> String {
        if self.0.is_empty() { String::new() } else { self.0.clear(); "\u{FFFD}".into() }
    }
}

fn joins(prev: Option<char>, next: Option<char>, ri_before: usize) -> bool {
    let (Some(n), p) = (next, prev) else { return false };
    let u = n as u32;
    u == 0x200D
        || (0xFE00..=0xFE0F).contains(&u)
        || (0x1F3FB..=0x1F3FF).contains(&u)
        || (0x0300..=0x036F).contains(&u)
        || (0x20D0..=0x20FF).contains(&u)
        || p == Some('\u{200D}')
        || (ri_before % 2 == 1 && (0x1F1E6..=0x1F1FF).contains(&u))
}
/// Move `end` (a byte offset on a char boundary) forward until it is not inside an emoji / combining sequence.
fn safe_end(acc: &str, mut end: usize) -> usize {
    while end < acc.len() {
        let prev = acc[..end].chars().next_back();
        let next = acc[end..].chars().next();
        let ri = acc[..end].chars().rev().take_while(|c| (0x1F1E6..=0x1F1FF).contains(&(*c as u32))).count();
        if !joins(prev, next, ri) {
            break;
        }
        end += next.map_or(0, |c| c.len_utf8());
    }
    end
}

pub struct ChatModel {
    pub fx: Fx,
    pub settings: AiSettings,
    lang: Lang,
    tags: Tags,
    pend: HashMap<u64, Pend>,
    tz_min: i32,
    thread: Option<i64>,
    msgs: Vec<Msg>,
    /// the user bubble of the message being sent (until the thread is reloaded)
    optimistic: Option<String>,
    run: Option<u64>,
    sending: bool,
    /// the streaming answer exists (typing dots / growing text)
    ai_body: bool,
    acc: String,
    shown: usize,
    drip: bool,
    has_last: bool,
    early: Vec<(bool, Value)>,
    files: Vec<PickedFile>,
    input: String,
    sent: Vec<String>,
    hist_ix: Option<usize>,
    err: Option<(String, bool)>,
    hist_open: bool,
    history: Vec<Value>,
    near_bottom: bool,
    loaded: bool,
    opened: bool,
    show_pending: bool,
    enabling: bool,
    mic_wait: bool,
    mic_recording: bool,
    copied: Option<usize>,
    copied_left: Timer,
    ghosts: Vec<Ghost>,
}

impl ChatModel {
    pub fn new(lang: Lang) -> Self {
        Self {
            fx: Fx::default(), settings: AiSettings::new(lang, OWNER_SET), lang, tags: Tags::new(OWNER), pend: HashMap::new(), tz_min: 0, thread: None, msgs: vec![], optimistic: None,
            run: None, sending: false, ai_body: false, acc: String::new(), shown: 0, drip: false, has_last: false, early: vec![], files: vec![], input: String::new(), sent: vec![],
            hist_ix: None, err: None, hist_open: false, history: vec![], near_bottom: true, loaded: false, opened: false, show_pending: false, enabling: false, mic_wait: false, mic_recording: false,
            copied: None, copied_left: Timer::default(), ghosts: vec![],
        }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
        self.settings.set_lang(l);
    }
    pub fn set_tz_offset_min(&mut self, m: i32) {
        self.tz_min = m;
    }
    pub fn set_reduce_motion(&mut self, v: bool) {
        self.fx.reduce_motion = v;
        self.settings.fx.reduce_motion = v;
    }
    pub fn take_effects(&mut self) -> Vec<Effect> {
        let mut v = self.fx.take_effects();
        v.extend(self.settings.fx.take_effects());
        v
    }
    pub fn take_anims(&mut self) -> Vec<AnimCue> {
        let mut v = self.fx.take_anims();
        v.extend(self.settings.fx.take_anims());
        v
    }
    pub fn thread(&self) -> Option<i64> {
        self.thread
    }
    pub fn messages(&self) -> &[Msg] {
        &self.msgs
    }
    pub fn is_streaming(&self) -> bool {
        self.run.is_some() || self.sending
    }
    pub fn cfg(&self) -> &AiCfg {
        self.settings.cfg()
    }
    /// Text revealed so far and the whole backlog (tests and the host's "copy while streaming").
    pub fn streamed(&self) -> (&str, &str) {
        (&self.acc[..self.shown], &self.acc)
    }
    fn ready(&self) -> bool {
        self.cfg().ready()
    }
    fn call(&mut self, cmd: &'static str, args: Value, p: Pend) -> BackendCall {
        let tag = self.tags.next();
        self.pend.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }

    // ---- banners ----
    fn show_err(&mut self, msg: &str, retry: bool) {
        if msg.is_empty() {
            if self.err.take().is_some() {
                self.fx.anim("chat-error", Anim::Collapse);
            }
            return;
        }
        if self.err.is_none() {
            self.fx.anim("chat-error", Anim::Expand);
        }
        self.err = Some((msg.to_string(), retry));
    }
    fn render_msgs(&mut self) {
        self.fx.eff(Effect::ScrollToBottom);
        self.near_bottom = true;
    }
    fn open_settings(&mut self, show: bool) -> Vec<BackendCall> {
        self.close_hist();
        self.settings.open(show)
    }
    fn close_hist(&mut self) {
        if self.hist_open {
            self.hist_open = false;
            self.fx.anim("chat-history", Anim::Exit { y: -6.0, scale: 0.97, ms: 160 });
        }
    }

    // ---- mode ----
    fn apply_mode(&mut self) -> Vec<BackendCall> {
        let c = self.cfg();
        if c.on && !c.ready() && !self.opened {
            self.opened = true;
            return self.open_settings(true);
        }
        vec![]
    }
    /// called after every reply of the settings model: it flags configuration changes
    fn cfg_changed(&mut self) -> Vec<BackendCall> {
        let mut calls = self.apply_mode();
        if std::mem::take(&mut self.enabling) && calls.is_empty() && !self.ready() {
            calls = self.open_settings(true);
        }
        if std::mem::take(&mut self.show_pending) && !self.loaded {
            self.loaded = true;
            if self.cfg().on {
                calls.push(self.call("chat_threads", json!({}), Pend::FirstThread));
            } else {
                self.render_msgs();
            }
        }
        calls
    }

    // ---- lifecycle ----
    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.show_pending = true;
        vec![self.settings.load_cfg()]
    }

    // ---- thread loading ----
    fn load_thread(&mut self, id: Option<i64>, err: Option<(String, bool)>, close_hist: bool) -> Vec<BackendCall> {
        if self.is_streaming() {
            if close_hist {
                self.close_hist();
            }
            return vec![];
        }
        self.thread = id;
        match id {
            Some(t) => vec![self.call("chat_messages", json!({ "thread": t }), Pend::Load { err, close_hist })],
            None => {
                self.msgs.clear();
                self.finish_load(err, close_hist);
                vec![]
            }
        }
    }
    fn finish_load(&mut self, err: Option<(String, bool)>, close_hist: bool) {
        self.optimistic = None;
        self.show_err("", false);
        self.render_msgs();
        if let Some((m, retry)) = err {
            self.show_err(&m, retry);
        }
        if close_hist {
            self.close_hist();
        }
    }

    // ---- streaming placeholder ----
    fn start_ai(&mut self) {
        self.acc.clear();
        self.shown = 0;
        self.has_last = false;
        self.drip = false;
        self.ai_body = true;
        self.fx.anim("msg:ai", Anim::Enter { y: 6.0, ms: 180 });
        self.fx.anim("msg:ai", Anim::TypingDots);
        self.fx.eff(Effect::ScrollToBottom);
    }

    fn send(&mut self, text_in: Option<String>) -> Vec<BackendCall> {
        if self.is_streaming() {
            return vec![];
        }
        let mut text = text_in.clone().unwrap_or_else(|| self.input.clone()).trim().to_string();
        if text.is_empty() && self.files.is_empty() {
            return vec![];
        }
        if !self.ready() {
            let m = if !self.cfg().on { self.lang.t("ИИ выключен.", "AI is switched off.") } else { self.lang.t("Укажите адрес API и модель.", "Set the API address and the model.") }.to_string();
            let calls = self.open_settings(true);
            self.show_err(&m, false);
            return calls;
        }
        for f in &self.files {
            text.push_str(&format!("\n\n[{}]\n```\n{}\n```", f.name, f.text));
        }
        self.show_err("", false);
        self.sending = true;
        self.early.clear();
        self.optimistic = Some(text.clone());
        self.fx.anim("msg:user", Anim::Enter { y: 6.0, ms: 180 });
        self.start_ai();
        let from_input = text_in.is_none();
        vec![self.call("chat_send", json!({ "thread": self.thread, "text": text, "model": null }), Pend::Send { from_input })]
    }

    fn retry(&mut self) -> Vec<BackendCall> {
        if self.is_streaming() || self.thread.is_none() {
            return vec![];
        }
        if !self.ready() {
            return self.open_settings(true);
        }
        self.show_err("", false);
        self.sending = true;
        self.early.clear();
        // the last answer fades out, then a new one streams
        if let Some(last) = self.msgs.last().filter(|m| m.role == Role::Assistant) {
            self.ghosts.push(Ghost { text: last.content.clone(), left_ms: 180 });
            self.fx.anim("msg:last", Anim::Exit { y: -6.0, scale: 1.0, ms: 180 });
        }
        while self.msgs.last().map_or(false, |m| m.role == Role::Assistant) {
            self.msgs.pop();
        }
        self.render_msgs();
        self.start_ai();
        let t = self.thread;
        vec![self.call("chat_retry", json!({ "thread": t, "model": null }), Pend::Retry)]
    }

    fn replay_early(&mut self) -> Vec<BackendCall> {
        let early = std::mem::take(&mut self.early);
        let mut out = vec![];
        for (is_delta, p) in early {
            out.extend(if is_delta { self.on_delta(&p) } else { self.on_done(&p) });
        }
        out
    }

    fn on_delta(&mut self, p: &Value) -> Vec<BackendCall> {
        if self.sending && self.run.is_none() {
            self.early.push((true, p.clone()));
            return vec![];
        }
        if p.is_null() || p["run"].as_u64() != self.run || !self.ai_body {
            return vec![];
        }
        self.acc.push_str(p["text"].as_str().unwrap_or(""));
        if !self.drip {
            self.drip = true;
            self.has_last = false;
        }
        vec![]
    }

    fn on_done(&mut self, p: &Value) -> Vec<BackendCall> {
        if self.sending && self.run.is_none() {
            self.early.push((false, p.clone()));
            return vec![];
        }
        if p.is_null() || p["run"].as_u64() != self.run || self.run.is_none() {
            return vec![];
        }
        // flush instantly: drop the drip, the reload below paints the full saved answer
        self.acc.clear();
        self.shown = 0;
        self.drip = false;
        self.has_last = false;
        self.run = None;
        self.ai_body = false;
        let err = if p["ok"].as_bool().unwrap_or(false) {
            None
        } else {
            let e = p["error"].as_str().filter(|e| !e.is_empty()).unwrap_or_else(|| self.lang.t("Ошибка", "Error")).to_string();
            Some((e, true))
        };
        let t = self.thread;
        self.load_thread(t, err, false)
    }

    pub fn on_event(&mut self, name: &str, p: &Value) -> Vec<BackendCall> {
        match name {
            "chat:delta" => self.on_delta(p),
            "chat:done" => self.on_done(p),
            "settings:changed" => vec![self.settings.load_cfg()],
            "voice:state" => {
                let s = p["state"].as_str().unwrap_or("");
                self.mic_recording = s == "recording";
                if s == "error" || s == "idle" {
                    self.mic_wait = false;
                }
                vec![]
            }
            "voice:done" => {
                if std::mem::take(&mut self.mic_wait) {
                    if let Some(x) = p["text"].as_str().filter(|x| !x.is_empty()) {
                        if !self.input.is_empty() && !self.input.ends_with(char::is_whitespace) {
                            self.input.push(' ');
                        }
                        self.input.push_str(x);
                        self.fx.eff(Effect::Focus("chat-input"));
                    }
                }
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Input(t) => {
                self.input = t;
                self.hist_ix = None;
                vec![]
            }
            Action::Send => self.send(None),
            Action::SendButton => {
                self.fx.anim("chat-send", Anim::Press);
                if self.is_streaming() { self.stop() } else { self.send(None) }
            }
            Action::SendText(t) => self.send(Some(t)),
            Action::Stop => self.stop(),
            Action::NewChat => {
                if self.is_streaming() {
                    return vec![];
                }
                self.thread = None;
                self.msgs.clear();
                self.show_err("", false);
                self.render_msgs();
                self.close_hist();
                vec![]
            }
            Action::Retry => self.retry(),
            Action::CopyMessage(i) => {
                let v = self.view();
                if let Some(m) = v.messages.get(i) {
                    self.fx.eff(Effect::CopyText(m.text.clone()));
                    self.copied = Some(i);
                    self.copied_left.arm(1200);
                }
                vec![]
            }
            Action::CopyText(t) => {
                self.fx.eff(Effect::CopyText(t));
                vec![]
            }
            Action::OpenLink(u) => vec![self.call("chat_open_link", json!({ "url": u }), Pend::Other)],
            Action::Attach(files) => {
                let ru = self.lang.is_ru();
                for f in files.into_iter().take(MAX_FILES) {
                    if f.size > MAX_FILE_BYTES {
                        let m = format!("{}{}", if ru { "Файл больше 200 КБ: " } else { "File is over 200 KB: " }, f.name);
                        self.show_err(&m, false);
                        continue;
                    }
                    if f.text.contains('\0') {
                        let m = format!("{}{}", if ru { "Только текстовые файлы: " } else { "Text files only: " }, f.name);
                        self.show_err(&m, false);
                        continue;
                    }
                    self.files.push(f);
                }
                vec![]
            }
            Action::RemoveFile(i) => {
                if i < self.files.len() {
                    self.files.remove(i);
                }
                vec![]
            }
            Action::DismissError => {
                self.show_err("", false);
                vec![]
            }
            Action::OpenHistory => {
                if self.hist_open {
                    self.close_hist();
                    return vec![];
                }
                self.settings.close();
                vec![self.call("chat_threads", json!({}), Pend::Threads)]
            }
            Action::PickThread(id) => self.load_thread(Some(id), None, true),
            Action::DeleteThread(id) => {
                self.fx.anim(&format!("thread:{id}"), Anim::Collapse);
                self.history.retain(|t| t["id"].as_i64() != Some(id));
                if self.thread == Some(id) {
                    self.thread = None;
                    self.msgs.clear();
                    self.render_msgs();
                }
                vec![self.call("chat_delete", json!({ "thread": id }), Pend::Other)]
            }
            Action::ClickOutside => {
                self.close_hist();
                vec![]
            }
            Action::OpenSettings => self.open_settings(false),
            Action::Settings(sa) => self.settings.on_action(sa),
            Action::EnableAi => {
                // saveCfg({'net.ai': true}); then, if the chat is still not configured, open the settings
                self.enabling = true;
                vec![self.settings.patch(json!({ "net.ai": true }))]
            }
            Action::Mic => {
                self.mic_wait = true;
                self.fx.eff(Effect::VoiceToggle);
                vec![]
            }
            Action::HistoryUp => {
                if self.sent.is_empty() {
                    return vec![];
                }
                let ix = match self.hist_ix {
                    None => self.sent.len() - 1,
                    Some(0) => 0,
                    Some(i) => i - 1,
                };
                self.hist_ix = Some(ix);
                self.input = self.sent[ix].clone();
                vec![]
            }
            Action::HistoryDown => {
                match self.hist_ix {
                    Some(i) if i + 1 < self.sent.len() => {
                        self.hist_ix = Some(i + 1);
                        self.input = self.sent[i + 1].clone();
                    }
                    Some(_) => {
                        self.hist_ix = None;
                        self.input.clear();
                    }
                    None => {}
                }
                vec![]
            }
            Action::Scrolled { offset, viewport, content } => {
                self.near_bottom = content - offset - viewport < NEAR_BOTTOM_PX;
                vec![]
            }
        }
    }

    fn stop(&mut self) -> Vec<BackendCall> {
        match self.thread {
            Some(t) => vec![self.call("chat_stop", json!({ "thread": t }), Pend::Other)],
            None => vec![],
        }
    }

    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        if Tags::owner_of(tag) == OWNER_SET {
            let mut calls = self.settings.on_reply(tag, r);
            if self.settings.take_changed() {
                calls.extend(self.cfg_changed());
            }
            return calls;
        }
        let Some(p) = self.pend.remove(&tag) else { return vec![] };
        match p {
            Pend::Send { from_input } => {
                let v = if r.ok { r.v.clone() } else { Value::Null };
                if !v["ok"].as_bool().unwrap_or(false) {
                    self.sending = false;
                    self.ai_body = false;
                    if v["user"].as_i64().is_some() || v["user"].as_bool() == Some(true) {
                        self.thread = v["thread"].as_i64();
                    }
                    let m = v["error"].as_str().filter(|e| !e.is_empty()).unwrap_or(self.lang.t("Не удалось отправить.", "Could not send.")).to_string();
                    let t = self.thread;
                    return self.load_thread(t, Some((m, false)), false);
                }
                if from_input {
                    let sent = std::mem::take(&mut self.input);
                    let sent = sent.trim().to_string();
                    if !sent.is_empty() {
                        self.sent.retain(|x| *x != sent);
                        self.sent.push(sent);
                        if self.sent.len() > INPUT_HISTORY_MAX {
                            self.sent.remove(0);
                        }
                    }
                    self.hist_ix = None;
                }
                self.files.clear();
                self.thread = v["thread"].as_i64();
                self.run = v["run"].as_u64();
                self.sending = false;
                self.replay_early()
            }
            Pend::Retry => {
                let v = if r.ok { r.v.clone() } else { Value::Null };
                if !v["ok"].as_bool().unwrap_or(false) {
                    self.sending = false;
                    self.ai_body = false;
                    let m = v["error"].as_str().filter(|e| !e.is_empty()).unwrap_or(self.lang.t("Не удалось повторить.", "Could not retry.")).to_string();
                    let t = self.thread;
                    return self.load_thread(t, Some((m, false)), false);
                }
                self.run = v["run"].as_u64();
                self.sending = false;
                self.replay_early()
            }
            Pend::Load { err, close_hist } => {
                if r.ok {
                    self.msgs = r
                        .v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|m| Msg { id: m["id"].as_i64().unwrap_or(0), role: if m["role"] == "user" { Role::User } else { Role::Assistant }, content: m["content"].as_str().unwrap_or("").to_string() })
                        .collect();
                } else {
                    self.msgs.clear();
                }
                self.finish_load(err, close_hist);
                vec![]
            }
            Pend::Threads => {
                self.history = r.v.as_array().cloned().unwrap_or_default();
                self.hist_open = true;
                self.fx.anim("chat-history", Anim::Enter { y: 6.0, ms: 220 });
                vec![]
            }
            Pend::FirstThread => {
                let first = r.v.as_array().and_then(|a| a.first()).and_then(|t| t["id"].as_i64());
                self.render_msgs();
                match first {
                    Some(id) => self.load_thread(Some(id), None, false),
                    None => vec![],
                }
            }
            Pend::Other => vec![],
        }
    }

    pub fn tick(&mut self, dt_ms: u32) {
        if self.copied_left.tick(dt_ms) {
            self.copied = None;
        }
        for g in &mut self.ghosts {
            g.left_ms = g.left_ms.saturating_sub(dt_ms);
        }
        self.ghosts.retain(|g| g.left_ms > 0);
        self.settings.fx.reduce_motion = self.fx.reduce_motion;
        if self.drip && self.ai_body {
            self.paint_ai(dt_ms as f64);
        }
    }

    /// One animation frame of the reveal.
    fn paint_ai(&mut self, dt: f64) {
        let dt = if self.has_last { dt.clamp(1.0, 100.0) } else { 16.7 };
        self.has_last = true;
        let backlog = self.acc[self.shown..].chars().count();
        if backlog > 0 {
            let n = if self.fx.reduce_motion { backlog } else { ((backlog as f64) * (1.0 - (5.0f64 / 6.0).powf(dt / 16.7))).ceil().max(1.0) as usize };
            let end = self.acc[self.shown..].char_indices().nth(n).map_or(self.acc.len(), |(i, _)| self.shown + i);
            self.shown = safe_end(&self.acc, end);
            if self.near_bottom {
                self.fx.eff(Effect::ScrollToBottom);
            }
            self.fx.anim("msg:ai", Anim::Caret);
        }
        if self.shown >= self.acc.len() {
            self.has_last = false;
            self.drip = false;
        }
    }

    pub fn view(&self) -> ChatView {
        let l = self.lang;
        let streaming = self.is_streaming();
        let last_ai = self.msgs.iter().rposition(|m| m.role == Role::Assistant);
        let mut messages: Vec<MsgView> = self
            .msgs
            .iter()
            .enumerate()
            .map(|(i, m)| MsgView {
                id: m.id,
                role: m.role,
                text: m.content.clone(),
                streaming: false,
                typing: false,
                actions: m.role == Role::Assistant && !streaming && Some(i) == last_ai && i == self.msgs.len() - 1,
                copied: self.copied == Some(i),
            })
            .collect();
        if let Some(t) = &self.optimistic {
            messages.push(MsgView { id: -1, role: Role::User, text: t.clone(), streaming: false, typing: false, actions: false, copied: false });
        }
        if self.ai_body {
            messages.push(MsgView { id: -2, role: Role::Assistant, text: self.acc[..self.shown].to_string(), streaming: true, typing: self.shown == 0, actions: false, copied: false });
        }
        let model = self.cfg().model.clone();
        let sub = if model.is_empty() {
            l.t("Выберите модель в настройках ИИ (значок справа вверху) · Shift+Enter — новая строка", "Choose a model in the AI settings (top right) · Shift+Enter new line").to_string()
        } else {
            format!("{model} {}", l.t("помнит весь разговор · Shift+Enter — новая строка", "remembers the whole conversation · Shift+Enter new line"))
        };
        let off = !self.cfg().on;
        let date = |ms: i64| {
            let w = wall(ms, self.tz_min);
            if l.is_ru() { format!("{:02}.{:02}.{}", w.day, w.month, w.year) } else { format!("{}/{}/{}", w.month, w.day, w.year) }
        };
        ChatView {
            empty: (self.msgs.is_empty() && self.optimistic.is_none() && !streaming).then(|| EmptyView {
                title: l.t("Спроси что угодно", "Ask me anything").into(),
                sub,
                ideas: vec![
                    l.t("Объясни простыми словами, как работает VPN", "Explain in simple words how a VPN works").into(),
                    l.t("Помоги написать вежливое письмо о переносе встречи", "Help me write a polite letter about rescheduling a meeting").into(),
                    l.t("Придумай пять идей, чем заняться в выходные", "Come up with five ideas for the weekend").into(),
                ],
            }),
            messages,
            ghosts: self.ghosts.clone(),
            off: off.then(|| OffView {
                title: l.t("ИИ-чат выключен", "AI chat is off").into(),
                body: l.t(
                    "Чат работает с вашим OpenAI-совместимым сервером: локальная Ollama, FreeLLMAPI или любой другой. Пока вы не включите, ничего не отправляется. История хранится только на этом компьютере.",
                    "The chat talks to your own OpenAI-compatible server: local Ollama, FreeLLMAPI or any other. Nothing is sent until you turn it on. History stays on this computer.",
                )
                .into(),
                enable_label: l.t("Включить", "Enable").into(),
            }),
            error: self.err.as_ref().map(|(m, retry)| ErrView {
                text: m.clone(),
                retry: *retry,
                retry_label: l.t("Повторить", "Retry").into(),
                settings_label: l.t("Настройки ИИ", "AI settings").into(),
                dismiss_title: l.t("Скрыть", "Dismiss").into(),
            }),
            files: self.files.iter().map(|f| f.name.clone()).collect(),
            remove_file_title: l.t("Убрать файл", "Remove the file").into(),
            input: self.input.clone(),
            placeholder: l.t("Сообщение для ИИ…", "Message the AI…").into(),
            busy: streaming,
            send_title: if streaming { l.t("Остановить", "Stop") } else { l.t("Отправить", "Send") }.into(),
            mic_title: l.t("Голосовой ввод: нажми, скажи, нажми ещё раз", "Voice input: press, speak, press again").into(),
            mic_recording: self.mic_recording,
            attach_title: l.t("Прикрепить текстовый файл", "Attach a text file").into(),
            new_chat_title: l.t("Новый чат", "New chat").into(),
            history_title: l.t("История чатов", "Chat history").into(),
            settings_title: l.t("Настройки ИИ", "AI settings").into(),
            history: HistView {
                open: self.hist_open,
                new_label: l.t("Новый чат", "New chat").into(),
                empty_text: self.history.is_empty().then(|| l.t("Пока нет чатов", "No chats yet").to_string()),
                rows: self
                    .history
                    .iter()
                    .map(|t| HistRow {
                        id: t["id"].as_i64().unwrap_or(0),
                        title: t["title"].as_str().filter(|s| !s.is_empty()).unwrap_or(l.t("Без названия", "Untitled")).to_string(),
                        date: date(t["updated_at"].as_i64().unwrap_or(0)),
                        selected: t["id"].as_i64() == self.thread,
                    })
                    .collect(),
            },
            settings: self.settings.view(),
            near_bottom: self.near_bottom,
            model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scripted backend for the chat commands.
    struct Be {
        cfg: Value,
        threads: Vec<(i64, String, Vec<(i64, &'static str, String)>)>,
        send_reply: Option<Value>,
        retry_reply: Option<Value>,
        log: Vec<(String, Value)>,
    }
    impl Be {
        fn new() -> Self {
            Be {
                cfg: json!({ "on": true, "baseUrl": "http://127.0.0.1:11434/v1", "model": "qwen3", "preset": "ollama", "hasKey": false, "urlError": null }),
                threads: vec![],
                send_reply: None,
                retry_reply: None,
                log: vec![],
            }
        }
        fn answer(&mut self, c: &BackendCall) -> Reply {
            self.log.push((c.cmd.to_string(), c.args.clone()));
            Reply::ok(match c.cmd {
                "ai_config" => self.cfg.clone(),
                "chat_threads" => Value::Array(self.threads.iter().map(|(id, t, m)| json!({ "id": id, "title": t, "created_at": 0, "updated_at": 1_761_800_000_000i64, "count": m.len() })).collect()),
                "chat_messages" => {
                    let id = c.args["thread"].as_i64().unwrap();
                    let m = self.threads.iter().find(|t| t.0 == id).map(|t| t.2.clone()).unwrap_or_default();
                    Value::Array(m.iter().map(|(i, r, c)| json!({ "id": i, "role": r, "content": c, "created_at": 0 })).collect())
                }
                "chat_send" => self.send_reply.clone().unwrap_or_else(|| json!({ "ok": true, "thread": 1, "run": 7, "user": 10 })),
                "chat_retry" => self.retry_reply.clone().unwrap_or_else(|| json!({ "ok": true, "thread": 1, "run": 8 })),
                "chat_stop" => json!(true),
                "chat_delete" => {
                    let id = c.args["thread"].as_i64().unwrap();
                    self.threads.retain(|t| t.0 != id);
                    json!(true)
                }
                "settings_set" => {
                    if let Some(on) = c.args["patch"]["net.ai"].as_bool() {
                        self.cfg["on"] = json!(on);
                    }
                    Value::Null
                }
                "ai_probe" => json!([]),
                "net_features_status" => json!({ "features": [] }),
                _ => Value::Null,
            })
        }
        fn cmds(&self) -> Vec<&str> {
            self.log.iter().map(|(c, _)| c.as_str()).collect()
        }
    }
    fn drive(m: &mut ChatModel, be: &mut Be, calls: Vec<BackendCall>) {
        let mut q = calls;
        let mut n = 0;
        while !q.is_empty() {
            n += 1;
            assert!(n < 60, "runaway call chain");
            let c = q.remove(0);
            let r = be.answer(&c);
            q.extend(m.on_reply(c.tag, r));
        }
    }
    fn ready_chat() -> (ChatModel, Be) {
        let mut m = ChatModel::new(Lang::En);
        let mut be = Be::new();
        let c = m.on_show();
        drive(&mut m, &mut be, c);
        be.log.clear();
        (m, be)
    }
    /// Start a send and hand back the call so the test can deliver the reply itself.
    fn send_text(m: &mut ChatModel, t: &str) -> BackendCall {
        m.on_action(Action::Input(t.into()));
        let mut c = m.on_action(Action::Send);
        assert_eq!(c.len(), 1, "send makes one call");
        c.remove(0)
    }
    fn delta(run: u64, text: &str) -> Value {
        json!({ "thread": 1, "run": run, "text": text })
    }
    fn done(run: u64, ok: bool, err: Option<&str>) -> Value {
        json!({ "thread": 1, "run": run, "ok": ok, "stopped": false, "error": err, "id": 11 })
    }
    fn ai_text(m: &ChatModel) -> String {
        m.view().messages.iter().find(|x| x.streaming).map(|x| x.text.clone()).unwrap_or_default()
    }
    /// run frames until the reveal catches up; returns the number of frames
    fn settle(m: &mut ChatModel) -> usize {
        let mut n = 0;
        while m.streamed().0.len() < m.streamed().1.len() {
            m.tick(16);
            n += 1;
            assert!(n < 2000);
        }
        n
    }

    // ---------------------------------------------------------------- utf8 / emoji
    #[test]
    fn utf8_assembler_waits_for_the_rest_of_a_split_emoji() {
        let bytes = "a😀б".as_bytes().to_vec();
        for cut in 1..bytes.len() {
            let mut a = Utf8Assembler::default();
            let mut s = a.push(&bytes[..cut]);
            assert!(!s.contains('\u{FFFD}'), "cut {cut}");
            s.push_str(&a.push(&bytes[cut..]));
            assert_eq!(s, "a😀б", "cut {cut}");
        }
        // one byte at a time
        let mut a = Utf8Assembler::default();
        let s: String = "мир 🇷🇺".bytes().map(|b| a.push(&[b])).collect();
        assert_eq!(s, "мир 🇷🇺");
    }
    #[test]
    fn utf8_assembler_invalid_bytes_and_dangling_tail() {
        let mut a = Utf8Assembler::default();
        assert_eq!(a.push(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
        assert_eq!(a.push(&[0xF0, 0x9F]), "");
        assert_eq!(a.finish(), "\u{FFFD}");
        assert_eq!(a.finish(), "");
        assert_eq!(a.push(&[0xE2, 0x28, 0xA1]), "\u{FFFD}(\u{FFFD}");
    }
    #[test]
    fn reveal_never_splits_an_emoji_sequence() {
        let family = "👨\u{200D}👩\u{200D}👧";
        let flag = "🇷🇺";
        let thumbs = "👍🏽";
        let heart = "❤\u{FE0F}";
        let accent = "e\u{0301}";
        let text = format!("x {family} y {flag}{flag} {thumbs} {heart} {accent}!");
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "hi");
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        // chunks of every size 1..=5 chars: after every frame the revealed text is a prefix that ends on a boundary
        m.on_event("chat:delta", &delta(7, &text));
        let seqs = [family, flag, thumbs, heart, accent];
        let mut frames = 0;
        while m.streamed().0.len() < text.len() {
            m.tick(16);
            frames += 1;
            let shown = m.streamed().0.to_string();
            assert!(text.starts_with(&shown));
            for s in seqs {
                let at = text.find(s).unwrap();
                for k in 1..s.chars().count() {
                    let inner = at + s.char_indices().nth(k).unwrap().0;
                    assert_ne!(shown.len(), inner, "revealed {shown:?} ends inside {s:?}");
                }
            }
            assert!(frames < 500);
        }
        assert_eq!(ai_text(&m), text);
    }

    // ---------------------------------------------------------------- streaming
    #[test]
    fn stream_accumulates_then_done_reloads_the_stored_answer() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "  hello  ");
        assert_eq!(c.cmd, "chat_send");
        assert_eq!(c.args, json!({ "thread": null, "text": "hello", "model": null }));
        assert!(m.is_streaming());
        let v = m.view();
        assert!(v.busy && v.send_title == "Stop" && v.empty.is_none());
        assert_eq!(v.messages.len(), 2);
        assert_eq!((v.messages[0].id, v.messages[0].text.as_str()), (-1, "hello"));
        assert!(v.messages[1].streaming && v.messages[1].typing && v.messages[1].text.is_empty());
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        assert_eq!((m.thread(), m.view().input.as_str()), (Some(1), ""), "input cleared once the send was accepted");
        for chunk in ["Hel", "lo ", "wor", "ld! 😀"] {
            m.on_event("chat:delta", &delta(7, chunk));
        }
        assert_eq!(m.streamed().1, "Hello world! 😀");
        assert_eq!(ai_text(&m), "", "nothing is revealed before the first frame");
        m.tick(16);
        let first = ai_text(&m);
        assert!(!first.is_empty() && "Hello world! 😀".starts_with(&first));
        assert!(!m.view().messages[1].typing);
        let frames = settle(&mut m);
        assert!(frames > 1, "the reveal is spread over several frames");
        assert_eq!(ai_text(&m), "Hello world! 😀");
        // the backend finishes: the saved thread is reloaded and replaces the streamed view
        be.threads.push((1, "hello".into(), vec![(10, "user", "hello".into()), (11, "assistant", "Hello world! 😀".into())]));
        let calls = m.on_event("chat:done", &done(7, true, None));
        assert_eq!(calls[0].cmd, "chat_messages");
        assert_eq!(calls[0].args, json!({ "thread": 1 }));
        assert!(!m.is_streaming(), "the stream is over as soon as done arrives");
        drive(&mut m, &mut be, calls);
        let v = m.view();
        assert_eq!(v.messages.len(), 2);
        assert!(v.messages.iter().all(|x| !x.streaming && x.id > 0));
        assert!(v.messages[1].actions && !v.messages[0].actions, "copy/retry only on the last stored answer");
        assert!(v.error.is_none() && !v.busy);
        assert_eq!(m.streamed().1, "");
    }
    #[test]
    fn drip_speed_is_time_scaled() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "x");
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        m.on_event("chat:delta", &delta(7, &"a".repeat(60)));
        m.tick(5); // first frame ignores the real dt and assumes 16.7 ms
        assert_eq!(m.streamed().0.len(), 10, "ceil(60 * (1 - 5/6))");
        m.tick(100); // a long frame reveals more: ceil(50 * (1 - (5/6)^(100/16.7)))
        let want = (10.0 + (50.0f64 * (1.0 - (5.0f64 / 6.0).powf(100.0 / 16.7))).ceil()) as usize;
        assert_eq!(m.streamed().0.len(), want);
        m.tick(100_000); // a stall is clamped to 100 ms
        assert!(m.streamed().0.len() < 60 || want >= 55);
        // always at least one char per frame
        let mut m2 = ChatModel::new(Lang::En);
        m2.acc = "ab".into();
        m2.ai_body = true;
        m2.drip = true;
        m2.paint_ai(16.7);
        assert_eq!(m2.shown, 1);
    }
    #[test]
    fn reduce_motion_shows_everything_at_once_and_emits_no_cues() {
        let (mut m, mut be) = ready_chat();
        m.set_reduce_motion(true);
        let c = send_text(&mut m, "x");
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        m.on_event("chat:delta", &delta(7, "a long answer that would otherwise drip"));
        m.tick(16);
        assert_eq!(ai_text(&m), "a long answer that would otherwise drip");
        assert!(m.take_anims().is_empty());
    }
    #[test]
    fn animation_cues_for_enter_typing_and_caret() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "x");
        let kinds: Vec<Anim> = m.take_anims().into_iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&Anim::Enter { y: 6.0, ms: 180 }) && kinds.contains(&Anim::TypingDots));
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        m.on_event("chat:delta", &delta(7, "hi"));
        m.tick(16);
        assert!(m.take_anims().iter().any(|a| a.kind == Anim::Caret));
    }
    #[test]
    fn events_before_the_send_reply_are_replayed_in_order() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "x");
        // run id unknown yet: parked
        m.on_event("chat:delta", &delta(7, "fast "));
        m.on_event("chat:delta", &delta(7, "answer"));
        assert_eq!(m.streamed().1, "");
        let r = be.answer(&c);
        let calls = m.on_reply(c.tag, r);
        assert!(calls.is_empty());
        assert_eq!(m.streamed().1, "fast answer");
        // done parked as well
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "x");
        m.on_event("chat:delta", &delta(7, "ok"));
        let d = m.on_event("chat:done", &done(7, true, None));
        assert!(d.is_empty(), "done is parked until the run id is known");
        let r = be.answer(&c);
        let calls = m.on_reply(c.tag, r);
        assert_eq!(calls[0].cmd, "chat_messages");
        assert!(!m.is_streaming());
    }
    #[test]
    fn stale_or_foreign_events_are_ignored() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "x");
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        m.on_event("chat:delta", &delta(99, "other run"));
        assert_eq!(m.streamed().1, "");
        assert!(m.on_event("chat:done", &done(99, true, None)).is_empty());
        assert!(m.is_streaming());
        m.on_event("chat:delta", &Value::Null);
        assert!(m.on_event("chat:done", &Value::Null).is_empty());
        // a delta after done does nothing
        be.threads.push((1, "x".into(), vec![]));
        let calls = m.on_event("chat:done", &done(7, true, None));
        drive(&mut m, &mut be, calls);
        m.on_event("chat:delta", &delta(7, "late"));
        assert_eq!(m.streamed().1, "");
        assert!(m.on_event("chat:done", &done(7, true, None)).is_empty(), "a second done for the same run is dropped");
    }
    #[test]
    fn stop_mid_stream_keeps_the_partial_answer() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "write a poem");
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        m.on_event("chat:delta", &delta(7, "Roses are red, "));
        m.tick(16);
        let stop = m.on_action(Action::SendButton);
        assert_eq!((stop[0].cmd, stop[0].args.clone()), ("chat_stop", json!({ "thread": 1 })));
        assert!(m.is_streaming(), "the stream ends only when chat:done arrives");
        // backend: stopped = true, no error, the partial text is saved
        be.threads.push((1, "write a poem".into(), vec![(10, "user", "write a poem".into()), (11, "assistant", "Roses are red, ".into())]));
        let calls = m.on_event("chat:done", &json!({ "thread": 1, "run": 7, "ok": true, "stopped": true, "error": null, "id": 11 }));
        drive(&mut m, &mut be, calls);
        let v = m.view();
        assert!(v.error.is_none() && !v.busy);
        assert_eq!(v.messages.last().unwrap().text, "Roses are red, ");
        assert_eq!(m.streamed().1, "", "the drip state is gone");
        // nothing more is revealed afterwards
        m.tick(16);
        assert!(m.view().messages.iter().all(|x| !x.streaming));
    }
    #[test]
    fn stop_without_a_thread_does_nothing() {
        let (mut m, _) = ready_chat();
        assert!(m.on_action(Action::Stop).is_empty());
    }

    // ---------------------------------------------------------------- errors
    #[test]
    fn failed_run_shows_a_banner_with_retry() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "x");
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        be.threads.push((1, "x".into(), vec![(10, "user", "x".into())]));
        let calls = m.on_event("chat:done", &done(7, false, Some("HTTP 401: Incorrect API key (check the API key)")));
        drive(&mut m, &mut be, calls);
        let e = m.view().error.unwrap();
        assert_eq!(e.text, "HTTP 401: Incorrect API key (check the API key)");
        assert!(e.retry && e.retry_label == "Retry" && e.settings_label == "AI settings");
        assert!(!m.is_streaming());
        // without an error text the banner says "Error"
        let c = m.on_action(Action::Retry);
        let r = be.answer(&c[0]);
        m.on_reply(c[0].tag, r);
        let calls = m.on_event("chat:done", &json!({ "thread": 1, "run": 8, "ok": false }));
        drive(&mut m, &mut be, calls);
        assert_eq!(m.view().error.unwrap().text, "Error");
        m.set_lang(Lang::Ru);
        assert_eq!(m.view().error.unwrap().retry_label, "Повторить");
        m.on_action(Action::DismissError);
        assert!(m.view().error.is_none());
    }
    #[test]
    fn send_rejected_by_the_backend_reloads_and_explains() {
        let (mut m, mut be) = ready_chat();
        be.send_reply = Some(json!({ "ok": false, "thread": 4, "error": "This chat is still answering. Stop it first.", "user": 12 }));
        be.threads.push((4, "t".into(), vec![(12, "user", "x".into())]));
        let c = send_text(&mut m, "x");
        let r = be.answer(&c);
        let calls = m.on_reply(c.tag, r);
        assert!(!m.is_streaming());
        assert_eq!(m.thread(), Some(4), "the user message was stored: keep that thread");
        drive(&mut m, &mut be, calls);
        let v = m.view();
        assert_eq!(v.error.as_ref().unwrap().text, "This chat is still answering. Stop it first.");
        assert!(!v.error.unwrap().retry);
        assert_eq!(v.messages.len(), 1);
        assert_eq!(v.input, "x", "the text stays in the composer so nothing is lost");
        // no user message stored and no thread: empty screen + banner, default text
        let (mut m, mut be) = ready_chat();
        be.send_reply = Some(json!({ "ok": false }));
        let c = send_text(&mut m, "y");
        let r = be.answer(&c);
        let calls = m.on_reply(c.tag, r);
        assert!(calls.is_empty());
        let v = m.view();
        assert_eq!(v.error.unwrap().text, "Could not send.");
        assert!(v.empty.is_some() && v.messages.is_empty());
    }
    #[test]
    fn not_ready_opens_the_settings_instead_of_sending() {
        let mut m = ChatModel::new(Lang::En);
        let mut be = Be::new();
        be.cfg["on"] = json!(false);
        let c = m.on_show();
        drive(&mut m, &mut be, c);
        assert!(m.view().off.is_some());
        m.on_action(Action::Input("hello".into()));
        let c = m.on_action(Action::Send);
        assert_eq!(c[0].cmd, "ai_config", "the settings popover loads its state");
        assert_eq!(m.view().error.unwrap().text, "AI is switched off.");
        assert!(!m.is_streaming() && m.view().input == "hello");
        let mut m = ChatModel::new(Lang::Ru);
        let mut be = Be::new();
        be.cfg["model"] = json!("");
        let c = m.on_show();
        drive(&mut m, &mut be, c);
        assert!(m.view().settings.open, "first show with AI on but no model opens the settings once");
        m.on_action(Action::Settings(super::super::chat_settings::SAction::Close));
        m.on_action(Action::Input("привет".into()));
        m.on_action(Action::Send);
        assert_eq!(m.view().error.unwrap().text, "Укажите адрес API и модель.");
    }
    #[test]
    fn empty_message_is_ignored_and_a_second_send_while_streaming_too() {
        let (mut m, mut be) = ready_chat();
        m.on_action(Action::Input("   ".into()));
        assert!(m.on_action(Action::Send).is_empty());
        let c = send_text(&mut m, "one");
        assert!(m.on_action(Action::SendText("two".into())).is_empty());
        m.on_action(Action::Input("three".into()));
        assert!(m.on_action(Action::Send).is_empty());
        assert!(m.on_action(Action::NewChat).is_empty(), "new chat is blocked while streaming");
        assert!(m.on_action(Action::Retry).is_empty());
        let _ = (c, &mut be);
    }
    #[test]
    fn enable_button_switches_ai_on_and_opens_settings_if_unconfigured() {
        let mut m = ChatModel::new(Lang::En);
        let mut be = Be::new();
        be.cfg["on"] = json!(false);
        be.cfg["model"] = json!("");
        let c = m.on_show();
        drive(&mut m, &mut be, c);
        assert!(m.view().off.is_some() && !m.view().settings.open);
        let c = m.on_action(Action::EnableAi);
        assert_eq!((c[0].cmd, c[0].args.clone()), ("settings_set", json!({ "patch": { "net.ai": true } })));
        drive(&mut m, &mut be, c);
        assert!(m.view().off.is_none());
        assert!(m.view().settings.open, "AI is on but there is no model: the settings open");
    }

    // ---------------------------------------------------------------- retry
    #[test]
    fn retry_drops_the_answer_fades_it_and_streams_a_new_one() {
        let (mut m, mut be) = ready_chat();
        be.threads.push((1, "t".into(), vec![(10, "user", "q".into()), (11, "assistant", "old answer".into())]));
        let c = m.on_action(Action::PickThread(1));
        drive(&mut m, &mut be, c);
        assert!(m.view().messages[1].actions);
        let c = m.on_action(Action::Retry);
        assert_eq!((c[0].cmd, c[0].args.clone()), ("chat_retry", json!({ "thread": 1, "model": null })));
        let v = m.view();
        assert_eq!(v.messages.len(), 2, "user message + the new streaming answer");
        assert_eq!(v.ghosts.iter().map(|g| g.text.as_str()).collect::<Vec<_>>(), vec!["old answer"]);
        m.tick(100);
        assert_eq!(m.view().ghosts.len(), 1);
        m.tick(80);
        assert!(m.view().ghosts.is_empty(), "the ghost lives 180 ms");
        let r = be.answer(&c[0]);
        m.on_reply(c[0].tag, r);
        m.on_event("chat:delta", &delta(8, "new answer"));
        settle(&mut m);
        assert_eq!(ai_text(&m), "new answer");
    }
    #[test]
    fn retry_rejected_restores_the_thread_and_says_why() {
        let (mut m, mut be) = ready_chat();
        be.threads.push((1, "t".into(), vec![(10, "user", "q".into()), (11, "assistant", "a".into())]));
        let c = m.on_action(Action::PickThread(1));
        drive(&mut m, &mut be, c);
        be.retry_reply = Some(json!({ "ok": false, "error": "Nothing to retry." }));
        let c = m.on_action(Action::Retry);
        let r = be.answer(&c[0]);
        let calls = m.on_reply(c[0].tag, r);
        drive(&mut m, &mut be, calls);
        assert_eq!(m.view().error.unwrap().text, "Nothing to retry.");
        assert_eq!(m.view().messages.len(), 2, "the stored answer is back");
        assert!(!m.is_streaming());
    }
    #[test]
    fn retry_needs_a_thread() {
        let (mut m, _) = ready_chat();
        assert!(m.on_action(Action::Retry).is_empty());
    }

    // ---------------------------------------------------------------- scroll
    #[test]
    fn follows_the_stream_only_while_near_the_bottom() {
        let (mut m, mut be) = ready_chat();
        let c = send_text(&mut m, "x");
        let r = be.answer(&c);
        m.on_reply(c.tag, r);
        m.take_effects();
        m.on_event("chat:delta", &delta(7, "one two three"));
        m.tick(16);
        assert!(m.take_effects().contains(&Effect::ScrollToBottom));
        // the user scrolled 100 px away from the bottom
        m.on_action(Action::Scrolled { offset: 400.0, viewport: 300.0, content: 800.0 });
        assert!(!m.view().near_bottom);
        m.on_event("chat:delta", &delta(7, " four five six seven"));
        m.tick(16);
        assert!(!m.take_effects().contains(&Effect::ScrollToBottom));
        // 69 px from the bottom counts as near, 70 does not
        m.on_action(Action::Scrolled { offset: 431.0, viewport: 300.0, content: 800.0 });
        assert!(m.view().near_bottom);
        m.on_action(Action::Scrolled { offset: 430.0, viewport: 300.0, content: 800.0 });
        assert!(!m.view().near_bottom);
        // a reloaded thread always jumps down
        be.threads.push((1, "x".into(), vec![(10, "user", "x".into())]));
        let calls = m.on_event("chat:done", &done(7, true, None));
        drive(&mut m, &mut be, calls);
        assert!(m.take_effects().contains(&Effect::ScrollToBottom) && m.view().near_bottom);
    }

    // ---------------------------------------------------------------- composer
    #[test]
    fn attachments_are_limited_validated_and_appended_as_fenced_blocks() {
        let (mut m, mut be) = ready_chat();
        let f = |n: &str, size: usize, t: &str| PickedFile { name: n.into(), size, text: t.into() };
        m.on_action(Action::Attach(vec![f("big.txt", 200 * 1024 + 1, "x"), f("bin.dat", 10, "a\0b")]));
        assert_eq!(m.view().error.unwrap().text, "Text files only: bin.dat", "the last message wins, like the JS banner");
        assert!(m.view().files.is_empty());
        m.on_action(Action::Attach(vec![f("big.txt", 200 * 1024 + 1, "x")]));
        assert_eq!(m.view().error.unwrap().text, "File is over 200 KB: big.txt");
        m.on_action(Action::Attach((0..8).map(|i| f(&format!("f{i}.md"), 5, "body")).collect()));
        assert_eq!(m.view().files.len(), 5, "at most five files per pick");
        m.on_action(Action::RemoveFile(0));
        assert_eq!(m.view().files, vec!["f1.md", "f2.md", "f3.md", "f4.md"]);
        m.on_action(Action::Attach(vec![f("a.rs", 3, "fn main(){}")]));
        m.on_action(Action::RemoveFile(0));
        m.on_action(Action::RemoveFile(0));
        m.on_action(Action::RemoveFile(0));
        m.on_action(Action::RemoveFile(0));
        assert_eq!(m.view().files, vec!["a.rs"]);
        m.on_action(Action::Input("see file".into()));
        let c = m.on_action(Action::Send);
        assert_eq!(c[0].args["text"], json!("see file\n\n[a.rs]\n```\nfn main(){}\n```"));
        assert!(m.view().files.len() == 1, "files stay until the send is accepted");
        let r = be.answer(&c[0]);
        m.on_reply(c[0].tag, r);
        assert!(m.view().files.is_empty());
        // only files, no text: still sends
        let (mut m, _) = ready_chat();
        m.on_action(Action::Attach(vec![f("a.txt", 1, "z")]));
        assert_eq!(m.on_action(Action::Send)[0].args["text"], json!("\n\n[a.txt]\n```\nz\n```"));
    }
    #[test]
    fn idea_chips_send_without_touching_the_composer() {
        let (mut m, mut be) = ready_chat();
        m.on_action(Action::Input("draft".into()));
        let ideas = m.view().empty.unwrap().ideas;
        assert_eq!(ideas.len(), 3);
        let c = m.on_action(Action::SendText(ideas[0].clone()));
        assert_eq!(c[0].args["text"], json!(ideas[0]));
        let r = be.answer(&c[0]);
        m.on_reply(c[0].tag, r);
        assert_eq!(m.view().input, "draft");
    }
    #[test]
    fn input_history_recalls_sent_messages() {
        let (mut m, mut be) = ready_chat();
        for t in ["first", "second"] {
            let c = send_text(&mut m, t);
            let r = be.answer(&c);
            m.on_reply(c.tag, r);
            be.threads.push((1, "t".into(), vec![]));
            let calls = m.on_event("chat:done", &done(7, true, None));
            drive(&mut m, &mut be, calls);
        }
        assert!(m.on_action(Action::HistoryDown).is_empty());
        m.on_action(Action::HistoryUp);
        assert_eq!(m.view().input, "second");
        m.on_action(Action::HistoryUp);
        assert_eq!(m.view().input, "first");
        m.on_action(Action::HistoryUp);
        assert_eq!(m.view().input, "first");
        m.on_action(Action::HistoryDown);
        assert_eq!(m.view().input, "second");
        m.on_action(Action::HistoryDown);
        assert_eq!(m.view().input, "");
        m.on_action(Action::HistoryUp);
        m.on_action(Action::Input("typed".into()));
        m.on_action(Action::HistoryUp);
        assert_eq!(m.view().input, "second", "typing resets the recall position");
    }
    #[test]
    fn copy_message_flashes_a_check() {
        let (mut m, mut be) = ready_chat();
        be.threads.push((1, "t".into(), vec![(10, "user", "q".into()), (11, "assistant", "**bold** answer".into())]));
        let c = m.on_action(Action::PickThread(1));
        drive(&mut m, &mut be, c);
        m.take_effects();
        m.on_action(Action::CopyMessage(1));
        assert_eq!(m.take_effects(), vec![Effect::CopyText("**bold** answer".into())], "raw markdown text is copied");
        assert!(m.view().messages[1].copied);
        m.tick(1199);
        assert!(m.view().messages[1].copied);
        m.tick(1);
        assert!(!m.view().messages[1].copied);
        m.on_action(Action::CopyText("let x = 1;".into()));
        assert_eq!(m.take_effects(), vec![Effect::CopyText("let x = 1;".into())]);
    }
    #[test]
    fn voice_text_is_appended_only_after_the_mic_button() {
        let (mut m, _) = ready_chat();
        m.on_event("voice:done", &json!({ "text": "ignored" }));
        assert_eq!(m.view().input, "");
        m.on_action(Action::Input("hello".into()));
        m.on_action(Action::Mic);
        assert!(m.take_effects().contains(&Effect::VoiceToggle));
        m.on_event("voice:state", &json!({ "state": "recording" }));
        assert!(m.view().mic_recording);
        m.on_event("voice:done", &json!({ "text": "world" }));
        assert_eq!(m.view().input, "hello world");
        m.on_event("voice:state", &json!({ "state": "idle" }));
        assert!(!m.view().mic_recording);
        m.on_event("voice:done", &json!({ "text": "again" }));
        assert_eq!(m.view().input, "hello world", "one dictation per press");
    }
    #[test]
    fn open_link_is_a_backend_call() {
        let (mut m, _) = ready_chat();
        let c = m.on_action(Action::OpenLink("https://example.com/a".into()));
        assert_eq!((c[0].cmd, c[0].args.clone()), ("chat_open_link", json!({ "url": "https://example.com/a" })));
    }

    // ---------------------------------------------------------------- threads / history
    #[test]
    fn first_show_opens_the_newest_thread_once() {
        let mut m = ChatModel::new(Lang::En);
        let mut be = Be::new();
        be.threads.push((9, "Newest".into(), vec![(1, "user", "hi".into()), (2, "assistant", "hello".into())]));
        let c = m.on_show();
        drive(&mut m, &mut be, c);
        assert_eq!(m.thread(), Some(9));
        assert_eq!(m.view().messages.len(), 2);
        assert_eq!(be.cmds(), vec!["ai_config", "chat_threads", "chat_messages"]);
        be.log.clear();
        let c = m.on_show();
        drive(&mut m, &mut be, c);
        assert_eq!(be.cmds(), vec!["ai_config"], "later shows only refresh the config");
        assert_eq!(m.view().model, "qwen3");
    }
    #[test]
    fn off_screen_skips_the_thread_load() {
        let mut m = ChatModel::new(Lang::En);
        let mut be = Be::new();
        be.cfg["on"] = json!(false);
        let c = m.on_show();
        drive(&mut m, &mut be, c);
        assert_eq!(be.cmds(), vec!["ai_config"]);
        assert_eq!(m.view().off.unwrap().title, "AI chat is off");
    }
    #[test]
    fn history_popover_lists_picks_and_deletes() {
        let (mut m, mut be) = ready_chat();
        be.threads.push((1, "Plans".into(), vec![(1, "user", "a".into())]));
        be.threads.push((2, "".into(), vec![(2, "user", "b".into())]));
        let c = m.on_action(Action::OpenHistory);
        assert_eq!(c[0].cmd, "chat_threads");
        drive(&mut m, &mut be, c);
        let h = m.view().history;
        assert!(h.open && h.empty_text.is_none() && h.rows.len() == 2);
        assert_eq!((h.rows[0].title.as_str(), h.rows[0].date.as_str()), ("Plans", "10/30/2025"));
        assert_eq!(h.rows[1].title, "Untitled");
        m.set_lang(Lang::Ru);
        assert_eq!(m.view().history.rows[0].date, "30.10.2025");
        assert_eq!(m.view().history.rows[1].title, "Без названия");
        // pick: loads and closes
        let c = m.on_action(Action::PickThread(2));
        drive(&mut m, &mut be, c);
        let v = m.view();
        assert!(!v.history.open && v.messages[0].text == "b");
        assert!(v.history.rows[1].selected);
        // reopen, delete the open thread: the chat empties and the row collapses
        let c = m.on_action(Action::OpenHistory);
        drive(&mut m, &mut be, c);
        m.take_anims();
        let c = m.on_action(Action::DeleteThread(2));
        assert_eq!(c[0].cmd, "chat_delete");
        assert!(m.take_anims().iter().any(|a| a.target == "thread:2" && a.kind == Anim::Collapse));
        drive(&mut m, &mut be, c);
        assert!(m.view().messages.is_empty() && m.thread().is_none() && m.view().history.open);
        assert_eq!(m.view().history.rows.len(), 1);
        let c = m.on_action(Action::DeleteThread(1));
        drive(&mut m, &mut be, c);
        assert_eq!(m.view().history.empty_text.as_deref(), Some("Пока нет чатов"));
        // toggle closes; click outside closes
        assert!(m.on_action(Action::OpenHistory).is_empty());
        assert!(!m.view().history.open);
        let c = m.on_action(Action::OpenHistory);
        drive(&mut m, &mut be, c);
        m.on_action(Action::ClickOutside);
        assert!(!m.view().history.open);
    }
    #[test]
    fn history_and_settings_popovers_exclude_each_other() {
        let (mut m, mut be) = ready_chat();
        let c = m.on_action(Action::OpenHistory);
        drive(&mut m, &mut be, c);
        assert!(m.view().history.open);
        let c = m.on_action(Action::OpenSettings);
        drive(&mut m, &mut be, c);
        assert!(m.view().settings.open && !m.view().history.open);
        let c = m.on_action(Action::OpenHistory);
        drive(&mut m, &mut be, c);
        assert!(m.view().history.open);
        assert!(!m.view().settings.open);
    }
    #[test]
    fn new_chat_clears_and_closes_history() {
        let (mut m, mut be) = ready_chat();
        be.threads.push((1, "t".into(), vec![(1, "user", "a".into())]));
        let c = m.on_action(Action::PickThread(1));
        drive(&mut m, &mut be, c);
        let c = m.on_action(Action::OpenHistory);
        drive(&mut m, &mut be, c);
        m.on_action(Action::NewChat);
        assert!(m.thread().is_none() && m.view().messages.is_empty() && !m.view().history.open);
        assert!(m.view().empty.is_some());
        // the next send starts a new thread (thread: null)
        let c = send_text(&mut m, "fresh");
        assert_eq!(c.args["thread"], Value::Null);
    }
    #[test]
    fn model_name_shows_in_the_empty_screen_subtitle() {
        let (mut m, _) = ready_chat();
        assert_eq!(m.view().empty.unwrap().sub, "qwen3 remembers the whole conversation · Shift+Enter new line");
        m.set_lang(Lang::Ru);
        assert!(m.view().empty.unwrap().sub.starts_with("qwen3 помнит весь разговор"));
        let m = ChatModel::new(Lang::En);
        assert!(m.view().empty.unwrap().sub.starts_with("Choose a model in the AI settings"));
    }
    #[test]
    fn settings_changed_event_reloads_the_config() {
        let (mut m, mut be) = ready_chat();
        be.cfg["model"] = json!("llama3");
        let c = m.on_event("settings:changed", &Value::Null);
        drive(&mut m, &mut be, c);
        assert_eq!(m.cfg().model, "llama3");
        assert_eq!(m.view().model, "llama3");
    }
    #[test]
    fn settings_edits_flow_through_the_chat() {
        use super::super::chat_settings::SAction;
        let (mut m, mut be) = ready_chat();
        let c = m.on_action(Action::OpenSettings);
        drive(&mut m, &mut be, c);
        let c = m.on_action(Action::Settings(SAction::ModelChanged("phi4".into())));
        drive(&mut m, &mut be, c);
        let c = m.on_action(Action::Settings(SAction::TurnOff));
        drive(&mut m, &mut be, c);
        assert!(m.view().off.is_some() && !m.view().settings.open);
    }
}
