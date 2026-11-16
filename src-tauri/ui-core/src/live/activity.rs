//! Local live activities: any program on this PC may ask the island to show one ongoing, time-bound thing (a build, an export, a
//! download of its own) through a fixed one-line JSON message (docs/live-activities.md). This file is the pure part: the schema
//! and its hard limits ([`parse_line`]), the registry with its rules ([`Registry`]: at most [`MAX_ACTIVE`] shown, a short queue, a
//! dismissed id stays locked until it ends, the inactivity time-out) and the provider that draws the registry on the island.
//! The pipe, the clock thread and the command line live in `src/liveact.rs`; they only call into this file.
//!
//! Three representations, like iOS Live Activities:
//! * **expanded**: an event card (title, subtitle, progress bar, the caller's buttons), shown when an activity starts or is promoted
//!   from the queue;
//! * **compact**: one capsule row `liveact` with the oldest shown activity (icon, title, progress bar, chip) and a dismiss button;
//! * **minimal**: the other shown activities as icon + chip after a separator in that same row.
//!
//! Rules borrowed from Android 16 Live Updates / iOS: only fixed fields (no views, no images, no markup), hard length limits,
//! at most three at once, a user dismissal sticks until the activity ends, every activity ends by itself after `ttl_s`.
//! Opt-in: nothing is drawn unless `live.local` is on (the pipe is not even open then).

use std::any::Any;
use std::collections::{HashMap, VecDeque};

use serde_json::{json, Map, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;

pub const MAX_ACTIVE: usize = 3;
pub const MAX_QUEUED: usize = 5;
pub const MAX_LINE: usize = 2048;
pub const MAX_ID: usize = 32;
pub const MAX_TITLE: usize = 40;
pub const MAX_SUB: usize = 80;
pub const MAX_CHIP: usize = 7;
pub const MAX_ICON_EMOJI: usize = 8;
pub const MAX_ACTIONS: usize = 2;
pub const MAX_ACTION_ID: usize = 16;
pub const MAX_ACTION_LABEL: usize = 16;
pub const DEFAULT_TTL_S: u32 = 600;
pub const MAX_TTL_S: u32 = 3600;
/// The event the backend sends with the whole state after every change; its payload is [`Registry::snapshot`].
pub const STATE_EVENT: &str = "liveact:state";
pub const ROW_ID: &str = "liveact";
pub const PRIORITY: i32 = 56; // above the download ring (55), below the running timer (60)
const CARD_MS: u32 = 8000;
const CARD_MS_WITH_ACTIONS: u32 = 14000;

const FIELDS: [&str; 10] = ["v", "op", "id", "title", "subtitle", "icon", "progress", "chip", "ttl_s", "actions"];

/// Built-in glyph names a caller may use as `icon` (the line icons of the island).
const GLYPHS: [IconName; 36] = {
    use IconName::*;
    [Area, Bell, Battery, Bolt, Calendar, Check, Clipboard, Copy, Download, File, Folder, Keyboard, Mail, Mic, Monitor, Mute, Next, Note2, Pause, Play, Plus, Prev, Qr, Quote, QuoteOff, ScanText, Shelf, Sparkles, Terminal, Timer, Trash, Usb, Video, Volume, Warn, X]
};

// ---------------------------------------------------------------- schema

#[derive(Clone, Debug, PartialEq)]
pub struct Reject {
    pub code: &'static str,
    pub detail: String,
}

impl Reject {
    fn new(code: &'static str, detail: impl Into<String>) -> Reject {
        Reject { code, detail: detail.into() }
    }
    /// The one-line reply a client gets.
    pub fn reply(&self) -> String {
        json!({ "ok": false, "error": self.code, "detail": self.detail }).to_string()
    }
}

/// An update field: absent = keep, `null` or empty = clear, a value = set.
#[derive(Clone, Debug, PartialEq)]
pub enum Field<T> {
    Keep,
    Clear,
    Set(T),
}

impl<T> Field<T> {
    fn apply(self, cur: &mut Option<T>) {
        match self {
            Field::Keep => {}
            Field::Clear => *cur = None,
            Field::Set(v) => *cur = Some(v),
        }
    }
    fn into_opt(self) -> Option<T> {
        match self {
            Field::Set(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionSpec {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub id: String,
    pub title: Option<String>,
    pub subtitle: Field<String>,
    pub icon: Field<String>,
    pub progress: Field<f32>,
    pub chip: Field<String>,
    pub ttl_s: Option<u32>,
    /// `Some(vec![])` clears the buttons
    pub actions: Option<Vec<ActionSpec>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Start(Post),
    Update(Post),
    End { id: String },
}

impl Op {
    pub fn id(&self) -> &str {
        match self {
            Op::Start(p) | Op::Update(p) => &p.id,
            Op::End { id } => id,
        }
    }
}

/// Control and bidi / zero-width characters out, whitespace collapsed (what a caller sends is shown as plain text only).
fn clean(s: &str) -> String {
    let flat: String = s
        .chars()
        .filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn id_ok(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ':'))
}

fn bad(field: &str, why: impl std::fmt::Display) -> Reject {
    Reject::new("bad-field", format!("{field}: {why}"))
}

fn text_field(o: &Map<String, Value>, key: &str, max: usize) -> Result<Field<String>, Reject> {
    match o.get(key) {
        None => Ok(Field::Keep),
        Some(Value::Null) => Ok(Field::Clear),
        Some(Value::String(s)) => {
            let c = clean(s);
            if c.is_empty() {
                Ok(Field::Clear)
            } else if c.chars().count() > max {
                Err(bad(key, format!("at most {max} characters")))
            } else {
                Ok(Field::Set(c))
            }
        }
        Some(_) => Err(bad(key, "must be a string")),
    }
}

/// A glyph name or a short emoji; anything else (plain letters, a path, a URL) is refused: no images, no custom drawing.
pub fn icon_ok(s: &str) -> Option<String> {
    let lower = s.to_ascii_lowercase();
    if GLYPHS.iter().any(|g| g.as_str() == lower) {
        return Some(lower);
    }
    (s.chars().count() <= MAX_ICON_EMOJI && s.chars().all(|c| !c.is_ascii())).then(|| s.to_string())
}

/// What an icon string draws as.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    Glyph(IconName),
    Emoji(String),
}

pub fn icon_of(s: Option<&str>) -> Icon {
    match s {
        None => Icon::Glyph(IconName::Sparkles),
        Some(s) => GLYPHS.iter().find(|g| g.as_str() == s).map(|g| Icon::Glyph(*g)).unwrap_or_else(|| Icon::Emoji(s.to_string())),
    }
}

fn parse_actions(v: &Value) -> Result<Vec<ActionSpec>, Reject> {
    let a = match v {
        Value::Null => return Ok(Vec::new()),
        Value::Array(a) => a,
        _ => return Err(bad("actions", "must be a list")),
    };
    if a.len() > MAX_ACTIONS {
        return Err(bad("actions", format!("at most {MAX_ACTIONS}")));
    }
    let mut out: Vec<ActionSpec> = Vec::new();
    for e in a {
        let o = e.as_object().ok_or_else(|| bad("actions", "each is {id, label}"))?;
        if o.keys().any(|k| k != "id" && k != "label") {
            return Err(bad("actions", "only id and label"));
        }
        let id = o.get("id").and_then(Value::as_str).filter(|s| id_ok(s, MAX_ACTION_ID)).ok_or_else(|| bad("actions.id", format!("1..{MAX_ACTION_ID} of A-Z a-z 0-9 _ . - :")))?;
        let label = clean(o.get("label").and_then(Value::as_str).unwrap_or(""));
        if label.is_empty() || label.chars().count() > MAX_ACTION_LABEL {
            return Err(bad("actions.label", format!("1..{MAX_ACTION_LABEL} characters")));
        }
        if out.iter().any(|x| x.id == id) {
            return Err(bad("actions.id", "duplicate"));
        }
        out.push(ActionSpec { id: id.to_string(), label });
    }
    Ok(out)
}

/// One line of the pipe -> an operation, or the reason it is refused.
pub fn parse_line(line: &str) -> Result<Op, Reject> {
    if line.len() > MAX_LINE {
        return Err(Reject::new("too-long", format!("a message is at most {MAX_LINE} bytes")));
    }
    let v: Value = serde_json::from_str(line.trim()).map_err(|e| Reject::new("bad-json", e.to_string()))?;
    parse_value(&v)
}

pub fn parse_value(v: &Value) -> Result<Op, Reject> {
    let o = v.as_object().ok_or_else(|| Reject::new("bad-json", "a message is one JSON object"))?;
    if let Some(k) = o.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(Reject::new("unknown-field", format!("`{k}` is not part of the schema")));
    }
    if o.get("v").is_some_and(|x| x != &json!(1)) {
        return Err(bad("v", "only 1"));
    }
    let op = o.get("op").and_then(Value::as_str).ok_or_else(|| bad("op", "start | update | end"))?;
    let id = o.get("id").and_then(Value::as_str).filter(|s| id_ok(s, MAX_ID)).ok_or_else(|| bad("id", format!("1..{MAX_ID} of A-Z a-z 0-9 _ . - :")))?.to_string();
    if op == "end" {
        if o.keys().any(|k| !["v", "op", "id"].contains(&k.as_str())) {
            return Err(bad("end", "takes only the id"));
        }
        return Ok(Op::End { id });
    }
    if op != "start" && op != "update" {
        return Err(Reject::new("bad-op", "op is start, update or end"));
    }
    let title = match o.get("title") {
        None => None,
        Some(Value::String(s)) => {
            let c = clean(s);
            if c.is_empty() || c.chars().count() > MAX_TITLE {
                return Err(bad("title", format!("1..{MAX_TITLE} characters")));
            }
            Some(c)
        }
        Some(_) => return Err(bad("title", "must be a string")),
    };
    let icon = match text_field(o, "icon", MAX_ICON_EMOJI.max(16))? {
        Field::Set(s) => Field::Set(icon_ok(&s).ok_or_else(|| bad("icon", "a glyph name (bolt, timer, download, ...) or an emoji"))?),
        f => f,
    };
    let progress = match o.get("progress") {
        None => Field::Keep,
        Some(Value::Null) => Field::Clear,
        Some(x) => match x.as_f64() {
            Some(p) if p.is_finite() && (0.0..=1.0).contains(&p) => Field::Set(p as f32),
            _ => return Err(bad("progress", "a number from 0 to 1")),
        },
    };
    let ttl_s = match o.get("ttl_s") {
        None | Some(Value::Null) => None,
        Some(x) => match x.as_u64() {
            Some(t) if (1..=MAX_TTL_S as u64).contains(&t) => Some(t as u32),
            _ => return Err(bad("ttl_s", format!("1..{MAX_TTL_S} seconds"))),
        },
    };
    let post = Post {
        id,
        title,
        subtitle: text_field(o, "subtitle", MAX_SUB)?,
        icon,
        progress,
        chip: text_field(o, "chip", MAX_CHIP)?,
        ttl_s,
        actions: o.get("actions").map(parse_actions).transpose()?,
    };
    if op == "start" {
        if post.title.is_none() {
            return Err(bad("title", "required to start"));
        }
        Ok(Op::Start(post))
    } else {
        Ok(Op::Update(post))
    }
}

// ---------------------------------------------------------------- registry

#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<String>,
    pub progress: Option<f32>,
    pub chip: Option<String>,
    pub ttl_s: u32,
    pub actions: Vec<ActionSpec>,
    /// epoch ms: the time-out moves on with every start / update (an inactivity limit: a client that died leaves no card behind)
    pub deadline: i64,
}

impl Activity {
    fn from_post(p: Post, now: i64) -> Activity {
        let ttl_s = p.ttl_s.unwrap_or(DEFAULT_TTL_S);
        Activity {
            id: p.id,
            title: p.title.unwrap_or_default(),
            subtitle: p.subtitle.into_opt(),
            icon: p.icon.into_opt(),
            progress: p.progress.into_opt(),
            chip: p.chip.into_opt(),
            ttl_s,
            actions: p.actions.unwrap_or_default(),
            deadline: now + ttl_s as i64 * 1000,
        }
    }

    fn patch(&mut self, p: Post, now: i64) {
        if let Some(t) = p.title {
            self.title = t;
        }
        p.subtitle.apply(&mut self.subtitle);
        p.icon.apply(&mut self.icon);
        p.progress.apply(&mut self.progress);
        p.chip.apply(&mut self.chip);
        if let Some(t) = p.ttl_s {
            self.ttl_s = t;
        }
        if let Some(a) = p.actions {
            self.actions = a;
        }
        self.deadline = now + self.ttl_s as i64 * 1000;
    }

    fn json(&self) -> Value {
        json!({
            "id": self.id, "title": self.title, "sub": self.subtitle, "icon": self.icon, "progress": self.progress, "chip": self.chip,
            "actions": self.actions.iter().map(|a| json!({ "id": a.id, "label": a.label })).collect::<Vec<_>>(),
        })
    }
}

/// What an accepted operation did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Started,
    Queued,
    Updated,
    Ended,
    /// `end` of an id that is not there (an `end` is always safe to send twice)
    Gone,
}

impl Done {
    pub fn reply(self) -> String {
        let state = match self {
            Done::Started => "started",
            Done::Queued => "queued",
            Done::Updated => "updated",
            Done::Ended => "ended",
            Done::Gone => "gone",
        };
        json!({ "ok": true, "state": state }).to_string()
    }
}

/// Why an activity left the screen on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Ttl,
    Disabled,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Ttl => "ttl",
            Reason::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Default)]
pub struct Registry {
    active: Vec<Activity>,
    queue: VecDeque<Activity>,
    /// dismissed by the user: id -> the time the activity would have timed out; start / update of it is refused until `end` or that time
    locked: HashMap<String, i64>,
    /// ids that just entered the screen (start, promotion): the next snapshot asks for their expanded card
    expand: Vec<String>,
}

impl Registry {
    pub fn new() -> Registry {
        Registry::default()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty() && self.queue.is_empty() && self.locked.is_empty()
    }
    pub fn active(&self) -> &[Activity] {
        &self.active
    }
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
    pub fn is_locked(&self, id: &str) -> bool {
        self.locked.contains_key(id)
    }
    pub fn get(&self, id: &str) -> Option<&Activity> {
        self.active.iter().chain(self.queue.iter()).find(|a| a.id == id)
    }
    /// Does the activity show a button with this id? (an action click is only passed on when it does)
    pub fn has_action(&self, id: &str, action: &str) -> bool {
        self.get(id).is_some_and(|a| a.actions.iter().any(|x| x.id == action))
    }

    fn promote(&mut self) {
        while self.active.len() < MAX_ACTIVE {
            let Some(a) = self.queue.pop_front() else { break };
            self.expand.push(a.id.clone());
            self.active.push(a);
        }
    }

    pub fn apply(&mut self, op: Op, now: i64) -> Result<Done, Reject> {
        match op {
            Op::Start(p) => {
                if self.is_locked(&p.id) {
                    return Err(locked());
                }
                let a = Activity::from_post(p, now);
                if let Some(slot) = self.active.iter_mut().find(|x| x.id == a.id) {
                    *slot = a;
                    return Ok(Done::Updated);
                }
                if let Some(slot) = self.queue.iter_mut().find(|x| x.id == a.id) {
                    *slot = a;
                    return Ok(Done::Queued);
                }
                if self.active.len() < MAX_ACTIVE {
                    self.expand.push(a.id.clone());
                    self.active.push(a);
                    Ok(Done::Started)
                } else if self.queue.len() < MAX_QUEUED {
                    self.queue.push_back(a);
                    Ok(Done::Queued)
                } else {
                    Err(Reject::new("full", format!("{MAX_ACTIVE} activities are shown and {MAX_QUEUED} wait; end one first")))
                }
            }
            Op::Update(p) => {
                if self.is_locked(&p.id) {
                    return Err(locked());
                }
                match self.active.iter_mut().chain(self.queue.iter_mut()).find(|x| x.id == p.id) {
                    Some(a) => {
                        a.patch(p, now);
                        Ok(Done::Updated)
                    }
                    None => Err(Reject::new("unknown-id", "no such activity (it ended or timed out); start it again")),
                }
            }
            Op::End { id } => {
                if self.locked.remove(&id).is_some() {
                    return Ok(Done::Ended);
                }
                let before = self.active.len() + self.queue.len();
                self.active.retain(|a| a.id != id);
                self.queue.retain(|a| a.id != id);
                self.promote();
                Ok(if before == self.active.len() + self.queue.len() { Done::Gone } else { Done::Ended })
            }
        }
    }

    /// The user closed it. It leaves the screen and the id stays locked until `end` or its time-out. False when it was not shown.
    pub fn dismiss(&mut self, id: &str) -> bool {
        let Some(i) = self.active.iter().position(|a| a.id == id) else { return false };
        let a = self.active.remove(i);
        self.locked.insert(a.id, a.deadline);
        self.promote();
        true
    }

    /// Everything whose time is up, removed (the queue moves on). Returns the ids that were on the list (shown or waiting).
    pub fn expire(&mut self, now: i64) -> Vec<String> {
        let mut gone: Vec<String> = self.active.iter().chain(self.queue.iter()).filter(|a| a.deadline <= now).map(|a| a.id.clone()).collect();
        self.active.retain(|a| a.deadline > now);
        self.queue.retain(|a| a.deadline > now);
        self.locked.retain(|id, d| {
            let keep = *d > now;
            if !keep {
                gone.push(id.clone());
            }
            keep
        });
        self.promote();
        gone
    }

    /// Ends everything (the switch went off). Returns the ids that were there.
    pub fn clear(&mut self) -> Vec<String> {
        let mut ids: Vec<String> = self.active.iter().chain(self.queue.iter()).map(|a| a.id.clone()).collect();
        ids.extend(self.locked.keys().cloned());
        *self = Registry::default();
        ids
    }

    /// The earliest time something needs attention (`None` = nothing to wait for).
    pub fn next_deadline(&self) -> Option<i64> {
        self.active.iter().chain(self.queue.iter()).map(|a| a.deadline).chain(self.locked.values().copied()).min()
    }

    /// The state the island draws. `expand` is handed out once.
    pub fn snapshot(&mut self) -> Value {
        let shown: Vec<&str> = self.active.iter().map(|a| a.id.as_str()).collect();
        let expand: Vec<String> = std::mem::take(&mut self.expand).into_iter().filter(|id| shown.contains(&id.as_str())).collect();
        json!({ "acts": self.active.iter().map(Activity::json).collect::<Vec<_>>(), "queued": self.queue.len(), "expand": expand })
    }
}

fn locked() -> Reject {
    Reject::new("locked", "the user dismissed this activity; it stays closed until you end it (or it times out)")
}

/// The test activity of Settings > Live activities (and the preview): invented content, two buttons.
pub fn sample_start(ru: bool) -> Op {
    Op::Start(Post {
        id: "pill-sample".into(),
        title: Some(if ru { "Пример: экспорт видео" } else { "Sample: exporting video" }.into()),
        subtitle: Field::Set(if ru { "Так выглядит живая активность" } else { "This is how a live activity looks" }.into()),
        icon: Field::Set("video".into()),
        progress: Field::Set(0.4),
        chip: Field::Set("40%".into()),
        ttl_s: Some(30),
        actions: Some(vec![ActionSpec { id: "stop".into(), label: if ru { "Стоп" } else { "Stop" }.into() }, ActionSpec { id: "open".into(), label: if ru { "Открыть" } else { "Open" }.into() }]),
    })
}

/// A flood guard for one connection: a token bucket (`BURST` messages at once, `PER_S` per second after that).
#[derive(Debug)]
pub struct RateLimit {
    tokens: f64,
    last: i64,
}

impl RateLimit {
    const BURST: f64 = 30.0;
    const PER_S: f64 = 15.0;
    pub fn new(now_ms: i64) -> RateLimit {
        RateLimit { tokens: Self::BURST, last: now_ms }
    }
    pub fn allow(&mut self, now_ms: i64) -> bool {
        let dt = (now_ms - self.last).max(0) as f64 / 1000.0;
        self.last = now_ms;
        self.tokens = (self.tokens + dt * Self::PER_S).min(Self::BURST);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------- the provider

struct View {
    id: String,
    title: String,
    sub: Option<String>,
    icon: Option<String>,
    progress: Option<f32>,
    chip: Option<String>,
    actions: Vec<ActionSpec>,
}

fn views(state: &Value) -> Vec<View> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    state
        .get("acts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|v| View {
                    id: s(v, "id").unwrap_or_default(),
                    title: s(v, "title").unwrap_or_default(),
                    sub: s(v, "sub"),
                    icon: s(v, "icon"),
                    progress: v.get("progress").and_then(Value::as_f64).map(|p| p.clamp(0.0, 1.0) as f32),
                    chip: s(v, "chip"),
                    actions: v
                        .get("actions")
                        .and_then(Value::as_array)
                        .map(|l| l.iter().filter_map(|a| Some(ActionSpec { id: s(a, "id")?, label: s(a, "label")? })).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn icon_elem(icon: &Option<String>, size: f32) -> Elem {
    match icon_of(icon.as_deref()) {
        Icon::Glyph(n) => Elem::icon(n, size),
        Icon::Emoji(e) => Elem::text(e, TextStyle::Label),
    }
}

/// The compact + minimal representation: one capsule row.
fn row(env: &dyn Env, acts: &[View], queued: usize) -> RowSpec {
    let p = &acts[0];
    let mut e = vec![icon_elem(&p.icon, 16.0), Elem::text_max(p.title.clone(), TextStyle::Name, 120.0)];
    if let Some(v) = p.progress {
        e.push(Elem::Bar { value: v, width: 44.0, height: 4.0, glide_ms: 300, muted: false });
    }
    if let Some(c) = &p.chip {
        e.push(Elem::text(c.clone(), TextStyle::Percent));
    }
    for m in &acts[1..] {
        e.push(Elem::Sep);
        e.push(icon_elem(&m.icon, 14.0));
        if let Some(c) = &m.chip {
            e.push(Elem::text(c.clone(), TextStyle::Small));
        }
    }
    if queued > 0 {
        e.push(Elem::text(format!("+{queued}"), TextStyle::Small));
    }
    e.push(Elem::Button { action: Action::invoke("liveact_dismiss", json!({ "id": p.id })), label: None, icon: Some(IconName::X), style: ButtonStyle::IconRound, tip: env.tr("Скрыть", "Hide") });
    // the key changes with what the row is made of (a new chip animates in); the progress value is edited in place and glides
    let key = format!("{}|{}|{}|{}|{}", p.id, acts.len(), p.progress.is_some() as u8, p.chip.is_some() as u8, queued);
    let tip = match &p.sub {
        Some(s) => format!("{} - {}", p.title, s),
        None => p.title.clone(),
    };
    RowSpec::new(ROW_ID, PRIORITY).key(key).anim(Anim::Pop, Anim::None, Anim::Fade).tip(tip).elems(e)
}

/// The expanded representation: a card.
fn card(env: &dyn Env, a: &View) -> CardSpec {
    let ms = if a.actions.is_empty() { CARD_MS } else { CARD_MS_WITH_ACTIONS };
    let mut c = CardSpec::new(format!("liveact-{}", a.id), ms, CardBody::Activity { title: a.title.clone(), sub: a.sub.clone(), progress: a.progress, chip: a.chip.clone() }).tone(Tone::Accent);
    c.icon = Some(match icon_of(a.icon.as_deref()) {
        Icon::Glyph(n) => CardIcon::Name(n),
        Icon::Emoji(e) => CardIcon::Letter(e),
    });
    c.actions = a
        .actions
        .iter()
        .enumerate()
        .map(|(i, x)| CardAction::new(x.label.clone(), if i == 0 { ActionKind::White } else { ActionKind::Normal }, Action::invoke("liveact_action", json!({ "id": a.id, "action": x.id }))))
        .collect();
    let _ = env;
    c
}

#[derive(Default)]
pub struct ActivityLive {
    last: Value,
    shown: bool,
}

impl ActivityLive {
    fn paint(&mut self, env: &dyn Env, fresh: bool) -> Vec<LiveCmd> {
        let acts = views(&self.last);
        if !env.setting_true("live.local") || acts.is_empty() {
            return if std::mem::take(&mut self.shown) { vec![LiveCmd::clear(ROW_ID)] } else { Vec::new() };
        }
        self.shown = true;
        let queued = self.last.get("queued").and_then(Value::as_u64).unwrap_or(0) as usize;
        let mut out = vec![LiveCmd::set(row(env, &acts, queued))];
        if fresh {
            let ids: Vec<&str> = self.last.get("expand").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let mut any = false;
            for a in acts.iter().filter(|a| ids.contains(&a.id.as_str())) {
                out.push(LiveCmd::Toast(card(env, a)));
                any = true;
            }
            if any {
                out.push(LiveCmd::EdgePulse(0.5)); // the shell skips it under Reduce motion
            }
        }
        out
    }
}

impl Provider for ActivityLive {
    fn name(&self) -> &'static str {
        "liveact"
    }
    fn events(&self) -> &'static [&'static str] {
        &[STATE_EVENT, "settings:changed"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if name == STATE_EVENT {
            self.last = p.clone();
            return self.paint(env, true);
        }
        // a switch changed: draw or hide what is there (nothing new to expand)
        let want = env.setting_true("live.local") && !views(&self.last).is_empty();
        if want == self.shown {
            return Vec::new();
        }
        self.paint(env, false)
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    const T0: i64 = 1_700_000_000_000;

    fn ok(line: &str) -> Op {
        parse_line(line).unwrap_or_else(|e| panic!("{line}: {e:?}"))
    }
    fn err(line: &str) -> &'static str {
        parse_line(line).expect_err(line).code
    }
    fn start(id: &str) -> Op {
        ok(&format!(r#"{{"op":"start","id":"{id}","title":"T {id}"}}"#))
    }
    fn on() -> MemEnv {
        let mut e = MemEnv::en();
        e.set("live.local", json!(true));
        e
    }

    #[test]
    fn a_full_message_parses_and_is_cleaned() {
        let Op::Start(p) = ok(r#"{"v":1,"op":"start","id":"build.1","title":"  Build \u0007 app\u202e ","subtitle":"42 of 90","icon":"Timer","progress":0.5,"chip":"50%","ttl_s":120,"actions":[{"id":"stop","label":"Stop"}]}"#) else { panic!() };
        assert_eq!(p.title.as_deref(), Some("Build app"));
        assert_eq!((p.subtitle, p.icon, p.progress, p.chip, p.ttl_s), (Field::Set("42 of 90".into()), Field::Set("timer".into()), Field::Set(0.5), Field::Set("50%".into()), Some(120)));
        assert_eq!(p.actions, Some(vec![ActionSpec { id: "stop".into(), label: "Stop".into() }]));
        assert!(matches!(ok(r#"{"op":"end","id":"x"}"#), Op::End { id } if id == "x"));
    }

    #[test]
    fn update_fields_keep_clear_or_set() {
        let Op::Update(p) = ok(r#"{"op":"update","id":"a","progress":null,"chip":"","subtitle":"x"}"#) else { panic!() };
        assert_eq!((p.title, p.progress, p.chip, p.subtitle, p.icon, p.actions), (None, Field::Clear, Field::Clear, Field::Set("x".into()), Field::Keep, None));
        let Op::Update(p) = ok(r#"{"op":"update","id":"a","actions":null}"#) else { panic!() };
        assert_eq!(p.actions, Some(vec![]));
    }

    #[test]
    fn the_schema_is_fixed_and_the_limits_are_hard() {
        assert_eq!(err("nope"), "bad-json");
        assert_eq!(err("[1]"), "bad-json");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","color":"red"}"#), "unknown-field");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","html":"<b>"}"#), "unknown-field");
        assert_eq!(err(r#"{"op":"fly","id":"a"}"#), "bad-op");
        assert_eq!(err(r#"{"op":"start","id":"a"}"#), "bad-field", "a start needs a title");
        assert_eq!(err(r#"{"op":"start","id":"a b","title":"t"}"#), "bad-field");
        assert_eq!(err(&format!(r#"{{"op":"start","id":"{}","title":"t"}}"#, "x".repeat(33))), "bad-field");
        assert_eq!(err(&format!(r#"{{"op":"start","id":"a","title":"{}"}}"#, "x".repeat(41))), "bad-field");
        assert_eq!(err(&format!(r#"{{"op":"start","id":"a","title":"t","subtitle":"{}"}}"#, "x".repeat(81))), "bad-field");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","chip":"12345678"}"#), "bad-field");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","progress":1.5}"#), "bad-field");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","progress":"half"}"#), "bad-field");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","ttl_s":0}"#), "bad-field");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","ttl_s":3601}"#), "bad-field");
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","v":2}"#), "bad-field");
        assert_eq!(err(r#"{"op":"end","id":"a","title":"t"}"#), "bad-field");
        assert_eq!(err(&format!(r#"{{"op":"start","id":"a","title":"{}"}}"#, " ".repeat(2100))), "too-long");
        assert!(matches!(ok(r#"{"op":"start","id":"a","title":"t","ttl_s":3600,"progress":1}"#), Op::Start(_)));
    }

    #[test]
    fn icons_are_glyph_names_or_emoji_only() {
        assert!(icon_ok("bolt").is_some() && icon_ok("DOWNLOAD").as_deref() == Some("download"));
        assert!(icon_ok("\u{1F4E6}").is_some());
        assert!(icon_ok("C:\\x.png").is_none() && icon_ok("http://x/y.png").is_none() && icon_ok("star").is_none());
        assert!(icon_ok("\u{1F4E6}\u{1F4E6}\u{1F4E6}\u{1F4E6}\u{1F4E6}\u{1F4E6}\u{1F4E6}\u{1F4E6}\u{1F4E6}").is_none());
        assert_eq!(err(r#"{"op":"start","id":"a","title":"t","icon":"logo.png"}"#), "bad-field");
        assert_eq!(icon_of(Some("bolt")), Icon::Glyph(IconName::Bolt));
        assert_eq!(icon_of(None), Icon::Glyph(IconName::Sparkles));
        assert_eq!(icon_of(Some("\u{1F4E6}")), Icon::Emoji("\u{1F4E6}".into()));
    }

    #[test]
    fn at_most_two_distinct_buttons_with_short_labels() {
        let a = |s: &str| err(&format!(r#"{{"op":"start","id":"a","title":"t","actions":{s}}}"#));
        assert_eq!(a(r#"[{"id":"a","label":"A"},{"id":"b","label":"B"},{"id":"c","label":"C"}]"#), "bad-field");
        assert_eq!(a(r#"[{"id":"a","label":"A"},{"id":"a","label":"B"}]"#), "bad-field");
        assert_eq!(a(r#"[{"id":"a","label":"A very long label here"}]"#), "bad-field");
        assert_eq!(a(r#"[{"id":"a","label":"A","url":"x"}]"#), "bad-field");
        assert_eq!(a(r#"[{"id":"a b","label":"A"}]"#), "bad-field");
        assert_eq!(a(r#""stop""#), "bad-field");
    }

    #[test]
    fn three_are_shown_the_rest_wait_and_a_slot_goes_to_the_queue() {
        let mut r = Registry::new();
        for id in ["a", "b", "c"] {
            assert_eq!(r.apply(start(id), T0), Ok(Done::Started));
        }
        assert_eq!(r.apply(start("d"), T0), Ok(Done::Queued));
        assert_eq!(r.apply(start("e"), T0), Ok(Done::Queued));
        let s = r.snapshot();
        assert_eq!(s["acts"].as_array().unwrap().len(), 3);
        assert_eq!((s["queued"].clone(), s["expand"].clone()), (json!(2), json!(["a", "b", "c"])));
        assert_eq!(r.snapshot()["expand"], json!([]), "the cards are asked for once");
        assert_eq!(r.apply(ok(r#"{"op":"end","id":"b"}"#), T0), Ok(Done::Ended));
        let s = r.snapshot();
        assert_eq!(s["acts"].as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap()).collect::<Vec<_>>(), ["a", "c", "d"], "the first waiting one takes the slot");
        assert_eq!(s["expand"], json!(["d"]), "and gets its card");
    }

    #[test]
    fn the_queue_is_short() {
        let mut r = Registry::new();
        for i in 0..(MAX_ACTIVE + MAX_QUEUED) {
            r.apply(start(&format!("x{i}")), T0).unwrap();
        }
        assert_eq!(r.apply(start("one-more"), T0).unwrap_err().code, "full");
        assert_eq!(r.apply(start("x0"), T0), Ok(Done::Updated), "an id that is already there is replaced, not refused");
        assert_eq!(r.queued(), MAX_QUEUED);
    }

    #[test]
    fn update_edits_in_place_and_an_unknown_id_is_an_error() {
        let mut r = Registry::new();
        r.apply(ok(r#"{"op":"start","id":"a","title":"One","progress":0.1,"chip":"10%","subtitle":"s"}"#), T0).unwrap();
        assert_eq!(r.apply(ok(r#"{"op":"update","id":"a","progress":0.7,"chip":"70%"}"#), T0 + 5), Ok(Done::Updated));
        let a = r.get("a").unwrap();
        assert_eq!((a.title.as_str(), a.progress, a.chip.as_deref(), a.subtitle.as_deref()), ("One", Some(0.7), Some("70%"), Some("s")));
        r.apply(ok(r#"{"op":"update","id":"a","progress":null,"subtitle":null}"#), T0 + 6).unwrap();
        let a = r.get("a").unwrap();
        assert_eq!((a.progress, a.subtitle.clone(), a.chip.as_deref()), (None, None, Some("70%")));
        assert_eq!(r.apply(ok(r#"{"op":"update","id":"nope","progress":0.2}"#), T0).unwrap_err().code, "unknown-id");
        assert_eq!(r.apply(ok(r#"{"op":"end","id":"nope"}"#), T0), Ok(Done::Gone));
        assert_eq!(r.snapshot()["expand"], json!(["a"]), "an update never opens the card again");
    }

    #[test]
    fn a_dismissed_activity_stays_closed_until_it_ends() {
        let mut r = Registry::new();
        r.apply(start("a"), T0).unwrap();
        r.apply(start("b"), T0).unwrap();
        assert!(r.dismiss("a"));
        assert!(!r.dismiss("a"), "not shown any more");
        assert_eq!(r.active().len(), 1);
        assert_eq!(r.apply(start("a"), T0 + 1).unwrap_err().code, "locked", "no re-posting");
        assert_eq!(r.apply(ok(r#"{"op":"update","id":"a","progress":0.9}"#), T0 + 1).unwrap_err().code, "locked");
        assert!(r.is_locked("a"));
        assert_eq!(r.apply(ok(r#"{"op":"end","id":"a"}"#), T0 + 2), Ok(Done::Ended), "the end lifts the lock");
        assert_eq!(r.apply(start("a"), T0 + 3), Ok(Done::Started));
    }

    #[test]
    fn dismissing_frees_a_slot_for_the_queue() {
        let mut r = Registry::new();
        for id in ["a", "b", "c", "d"] {
            r.apply(start(id), T0).unwrap();
        }
        r.snapshot();
        assert!(r.dismiss("b"));
        let s = r.snapshot();
        assert_eq!(s["acts"].as_array().unwrap().len(), 3);
        assert_eq!(s["expand"], json!(["d"]));
        assert_eq!(r.queued(), 0);
    }

    #[test]
    fn every_activity_ends_by_itself_and_updates_keep_it_alive() {
        let mut r = Registry::new();
        r.apply(ok(r#"{"op":"start","id":"a","title":"A","ttl_s":10}"#), T0).unwrap();
        r.apply(start("b"), T0).unwrap();
        assert_eq!(r.get("b").unwrap().ttl_s, DEFAULT_TTL_S);
        assert_eq!(r.next_deadline(), Some(T0 + 10_000));
        r.apply(ok(r#"{"op":"update","id":"a","progress":0.5}"#), T0 + 8_000).unwrap();
        assert!(r.expire(T0 + 12_000).is_empty(), "the update moved the time-out");
        assert_eq!(r.expire(T0 + 18_000), ["a"]);
        assert_eq!(r.active().len(), 1);
        assert_eq!(r.expire(T0 + 600_000), ["b"]);
        assert!(r.is_empty() && r.next_deadline().is_none());
    }

    #[test]
    fn a_dismissed_lock_times_out_with_the_activity_and_the_queue_times_out_too() {
        let mut r = Registry::new();
        r.apply(ok(r#"{"op":"start","id":"a","title":"A","ttl_s":5}"#), T0).unwrap();
        r.dismiss("a");
        assert_eq!(r.next_deadline(), Some(T0 + 5_000));
        assert_eq!(r.apply(start("a"), T0 + 4_000).unwrap_err().code, "locked");
        assert_eq!(r.expire(T0 + 5_000), ["a"]);
        assert_eq!(r.apply(start("a"), T0 + 5_001), Ok(Done::Started));
        // waiting ones expire as well, and an expiry promotes the queue
        let mut r = Registry::new();
        for id in ["a", "b", "c"] {
            r.apply(ok(&format!(r#"{{"op":"start","id":"{id}","title":"T","ttl_s":1}}"#)), T0).unwrap();
        }
        r.apply(start("q"), T0).unwrap();
        r.apply(ok(r#"{"op":"start","id":"s","title":"T","ttl_s":1}"#), T0).unwrap();
        assert_eq!(r.expire(T0 + 2_000).len(), 4);
        assert_eq!(r.active().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["q"]);
    }

    #[test]
    fn clear_forgets_everything_and_has_action_checks_the_declared_buttons() {
        let mut r = Registry::new();
        r.apply(ok(r#"{"op":"start","id":"a","title":"A","actions":[{"id":"stop","label":"Stop"}]}"#), T0).unwrap();
        assert!(r.has_action("a", "stop") && !r.has_action("a", "rm -rf") && !r.has_action("zz", "stop"));
        r.apply(start("b"), T0).unwrap();
        r.dismiss("b");
        let mut gone = r.clear();
        gone.sort();
        assert_eq!(gone, ["a", "b"]);
        assert!(r.is_empty());
    }

    #[test]
    fn the_flood_guard_allows_a_burst_then_a_steady_rate() {
        let mut l = RateLimit::new(T0);
        assert_eq!((0..40).filter(|_| l.allow(T0)).count(), 30);
        assert!(!l.allow(T0 + 10));
        assert!(l.allow(T0 + 200), "15 per second refills");
        assert!((0..100).all(|i| l.allow(T0 + 10_000 + i * 70)), "a steady 14 per second never trips it");
    }

    fn fed(p: &mut ActivityLive, env: &MemEnv, r: &mut Registry) -> Vec<LiveCmd> {
        let s = r.snapshot();
        p.on_event(env, STATE_EVENT, &s, T0)
    }

    #[test]
    fn nothing_is_drawn_while_the_switch_is_off() {
        let mut p = ActivityLive::default();
        let mut r = Registry::new();
        r.apply(sample_start(false), T0).unwrap();
        assert!(fed(&mut p, &MemEnv::en(), &mut r).is_empty());
        // switching on draws what is there (no card: nothing new)
        let cmds = p.on_event(&on(), "settings:changed", &Value::Null, T0);
        assert!(matches!(cmds.as_slice(), [LiveCmd::SetRow(_)]));
        // and off again clears it, once
        assert_eq!(p.on_event(&MemEnv::en(), "settings:changed", &Value::Null, T0), vec![LiveCmd::clear(ROW_ID)]);
        assert!(p.on_event(&MemEnv::en(), "settings:changed", &Value::Null, T0).is_empty());
    }

    #[test]
    fn compact_minimal_and_expanded() {
        let env = on();
        let mut p = ActivityLive::default();
        let mut r = Registry::new();
        r.apply(sample_start(false), T0).unwrap();
        r.apply(ok(r#"{"op":"start","id":"dl","title":"Download","icon":"download","chip":"3/9"}"#), T0).unwrap();
        r.apply(ok(r#"{"op":"start","id":"e","title":"Emoji","icon":"\ud83d\udce6"}"#), T0).unwrap();
        r.apply(start("q"), T0).unwrap();
        let cmds = fed(&mut p, &env, &mut r);
        let LiveCmd::SetRow(row) = &cmds[0] else { panic!("{cmds:?}") };
        assert_eq!((row.id.as_str(), row.priority), (ROW_ID, PRIORITY));
        // compact: icon, title, bar, chip of the oldest
        assert!(matches!(row.elements[0], Elem::Icon { name: IconName::Video, .. }));
        assert!(matches!(&row.elements[1], Elem::Text { text, max_w: Some(_), .. } if text == "Sample: exporting video"));
        assert!(matches!(row.elements[2], Elem::Bar { value, .. } if value == 0.4));
        assert!(matches!(&row.elements[3], Elem::Text { text, .. } if text == "40%"));
        // minimal: a separator, icon + chip; an emoji is text
        assert_eq!(row.elements[4], Elem::Sep);
        assert!(matches!(row.elements[5], Elem::Icon { name: IconName::Download, .. }));
        assert!(matches!(&row.elements[6], Elem::Text { text, .. } if text == "3/9"));
        assert!(matches!(&row.elements[8], Elem::Text { text, .. } if text == "\u{1F4E6}"));
        // the queue count and the dismiss button close the row
        assert!(matches!(&row.elements[row.elements.len() - 2], Elem::Text { text, .. } if text == "+1"));
        let Some(Elem::Button { action, icon, .. }) = row.elements.last() else { panic!() };
        assert_eq!((*icon, action.clone()), (Some(IconName::X), Action::invoke("liveact_dismiss", json!({ "id": "pill-sample" }))));
        // expanded: one card per new activity, buttons run the action command, then a pulse
        let cards: Vec<&CardSpec> = cmds.iter().filter_map(|c| if let LiveCmd::Toast(c) = c { Some(c) } else { None }).collect();
        assert_eq!(cards.len(), 3);
        assert_eq!(cards[0].id, "liveact-pill-sample");
        assert!(matches!(&cards[0].body, CardBody::Activity { progress: Some(p), chip: Some(c), .. } if *p == 0.4 && c == "40%"));
        assert_eq!(cards[0].actions.len(), 2);
        assert_eq!(cards[0].actions[0].action, Action::invoke("liveact_action", json!({ "id": "pill-sample", "action": "stop" })));
        assert_eq!((cards[0].actions[0].kind, cards[0].actions[1].kind), (ActionKind::White, ActionKind::Normal));
        assert_eq!(cards[0].icon, Some(CardIcon::Name(IconName::Video)));
        assert_eq!(cards[2].icon, Some(CardIcon::Letter("\u{1F4E6}".into())));
        assert_eq!(cmds.last(), Some(&LiveCmd::EdgePulse(0.5)));
        // a progress update edits the same row in place: same key, no card
        r.apply(ok(r#"{"op":"update","id":"pill-sample","progress":0.8,"chip":"80%"}"#), T0 + 1).unwrap();
        let again = fed(&mut p, &env, &mut r);
        let LiveCmd::SetRow(row2) = &again[0] else { panic!() };
        assert_eq!((row2.key.clone(), again.len()), (row.key.clone(), 1));
        // everything ended: the row goes
        r.clear();
        assert_eq!(fed(&mut p, &env, &mut r), vec![LiveCmd::clear(ROW_ID)]);
    }

    #[test]
    fn the_sample_is_valid_in_both_languages() {
        for ru in [true, false] {
            let Op::Start(p) = sample_start(ru) else { panic!() };
            let line = json!({ "op": "start", "id": p.id, "title": p.title, "subtitle": p.subtitle.into_opt(), "icon": "video", "progress": 0.4, "chip": "40%", "ttl_s": 30,
                "actions": p.actions.unwrap().iter().map(|a| json!({"id": a.id, "label": a.label})).collect::<Vec<_>>() })
            .to_string();
            assert!(parse_line(&line).is_ok(), "{line}");
        }
    }
}
