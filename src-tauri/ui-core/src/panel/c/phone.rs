//! The Phone tab model: the mirrored phone notifications (Hide / Mark read / Reply), the files going to and from the phone, and the link state
//! with the pairing code. Pure data in, `BackendCall`s out: the view feeds it bus events (`phone:state`, `phone:notif`, `phone:files`), command
//! replies and user gestures, and draws what it exposes.
//!
//! Commands used: `phone_status`, `phone_notifs`, `phone_files`, `phone_pair_code {renew}`, `phone_notif_action {key, action, text}`,
//! `phone_file_decide {id, accept}`, `phone_open_file {id, reveal}`, `settings_set {patch: {"phone.enabled": true}}`.
//!
//! Rows: every notification the PC lists is shown; `ongoing` (media, recording) and `replay` (re-sent after a reconnect) ones are flagged so the
//! view can dim them. A row that goes away (Hide, Mark read, removed on the phone, a sent reply) is not deleted at once: it gets `leaving` and the
//! view calls [`PhoneModel::reap`] when its exit motion has finished. A row that is being replied to (`Sending` / `Sent`) ignores a removal until
//! the "Sent" confirmation has been shown ([`SENT_HOLD_MS`]).
use super::phone_info::{Info, Stats};
use crate::panel::b::{BackendCall, Lang, Reply, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

/// How long "Sent" stays on a row before it leaves.
pub const SENT_HOLD_MS: f64 = 1200.0;
/// A reply the phone never confirmed fails after this long.
pub const REPLY_TIMEOUT_MS: f64 = 20_000.0;
const MAX_FILES: usize = 100;
const MAX_NOTIFS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Sub {
    #[default]
    Notifs,
    Files,
    Phone,
}

impl Sub {
    pub const ALL: [Sub; 3] = [Sub::Notifs, Sub::Files, Sub::Phone];
    pub fn title(self, l: Lang) -> &'static str {
        match self {
            Sub::Notifs => l.t("Уведомления", "Notifications"),
            Sub::Files => l.t("Файлы", "Files"),
            Sub::Phone => l.t("Телефон", "Phone"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReplyState {
    Closed,
    Open,
    Sending,
    Sent,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notif {
    pub key: String,
    pub pkg: String,
    pub app: String,
    pub title: String,
    pub text: String,
    pub when: i64,
    pub ongoing: bool,
    pub replay: bool,
    pub can_reply: bool,
    /// base64 PNGs (96 px), as sent by the phone
    pub icon: Option<String>,
    pub avatar: Option<String>,
    pub reply: ReplyState,
    pub leaving: bool,
    since: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileRow {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub incoming: bool,
    pub status: String,
    pub bytes: u64,
    pub path: Option<String>,
}

impl FileRow {
    pub fn progress(&self) -> Option<f32> {
        (self.status == "active" && self.size > 0).then(|| (self.bytes as f32 / self.size as f32).clamp(0.0, 1.0))
    }
    pub fn pending(&self) -> bool {
        self.incoming && self.status == "pending"
    }
    pub fn has_file(&self) -> bool {
        self.status == "done" && self.path.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Device {
    pub name: String,
    pub connected: bool,
    pub battery: Option<i64>,
    pub charging: bool,
    pub network: String,
    pub ssid: Option<String>,
    pub dnd: bool,
    /// from pairing
    pub model: String,
    pub os: String,
    pub app_version: String,
    pub first_seen: i64,
    pub last_seen: i64,
    /// the address the PC saw the phone at (kept after it left), start of the current connection (ms, 0 = never)
    pub ip: String,
    pub connected_at: i64,
    /// the optional rich status and the session counters (`phone_info`)
    pub info: Info,
    pub stats: Stats,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Link {
    /// the "Phone link" switch
    pub enabled: bool,
    pub running: bool,
    /// `no_private_ip` | `port_busy` while the server could not start
    pub error: Option<String>,
    pub ip: String,
    pub port: u16,
    pub pc_name: String,
    pub devices: Vec<Device>,
    pub paired: usize,
}

impl Link {
    /// The phone to show: a connected one, else the first paired.
    pub fn phone(&self) -> Option<&Device> {
        self.devices.iter().find(|d| d.connected).or_else(|| self.devices.first())
    }
    pub fn connected(&self) -> bool {
        self.devices.iter().any(|d| d.connected)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Code {
    pub code: String,
    pub url: String,
    pub expires: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(Sub),
    Hide(String),
    MarkRead(String),
    ReplyOpen(String),
    ReplyClose(String),
    ReplySend(String, String),
    Accept(String),
    Decline(String),
    OpenFile(String),
    Reveal(String),
    NewCode,
    Enable,
}

#[derive(Clone, Debug)]
enum Pending {
    Status,
    Notifs,
    Files,
    Code,
    Reply(String),
    Other,
}

pub struct PhoneModel {
    lang: Lang,
    sub: Sub,
    rows: Vec<Notif>,
    files: Vec<FileRow>,
    link: Link,
    code: Option<Code>,
    code_asked: bool,
    next_tag: u64,
    pending: HashMap<u64, Pending>,
    /// epoch ms of the wall clock, set by the view (`set_wall`)
    wall: i64,
    now: f64,
    mounted: bool,
    loaded: bool,
}

/// "now", "5 min", "3 h", "2 d": how long ago, in the user's language.
pub fn ago(diff_ms: i64, l: Lang) -> String {
    let m = diff_ms.max(0) / 60_000;
    match m {
        0 => l.t("сейчас", "now").to_string(),
        1..=59 => format!("{m} {}", l.t("мин", "min")),
        60..=1439 => format!("{} {}", m / 60, l.t("ч", "h")),
        _ => format!("{} {}", m / 1440, l.t("д", "d")),
    }
}

/// "1.4 MB" style size.
pub fn size_text(b: u64, l: Lang) -> String {
    let (units_ru, units_en) = (["Б", "КБ", "МБ", "ГБ"], ["B", "KB", "MB", "GB"]);
    let u = if l == Lang::Ru { units_ru } else { units_en };
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 3 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} {}", u[0])
    } else {
        format!("{v:.1} {}", u[i])
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn opt_s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_string)
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

impl PhoneModel {
    pub fn new(lang: Lang) -> PhoneModel {
        PhoneModel { lang, sub: Sub::Notifs, rows: vec![], files: vec![], link: Link::default(), code: None, code_asked: false, next_tag: 1, pending: HashMap::new(), wall: 0, now: 0.0, mounted: false, loaded: false }
    }

    // ------------------------------------------------------------------------------------------------ read side
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
    }
    pub fn lang(&self) -> Lang {
        self.lang
    }
    pub fn sub(&self) -> Sub {
        self.sub
    }
    /// The first list read has been answered: rows that exist from now on are news, the ones that came with it are not.
    pub fn loaded(&self) -> bool {
        self.loaded
    }
    pub fn rows(&self) -> &[Notif] {
        &self.rows
    }
    pub fn files(&self) -> &[FileRow] {
        &self.files
    }
    pub fn link(&self) -> &Link {
        &self.link
    }
    pub fn code(&self) -> Option<&Code> {
        self.code.as_ref()
    }
    /// Number of notifications that count for a badge (not leaving, not ongoing).
    pub fn unread(&self) -> usize {
        self.rows.iter().filter(|r| !r.leaving && !r.ongoing).count()
    }
    /// The state the tab shows when there is no list: link off, link on but nobody paired, paired but offline, or an empty list.
    pub fn empty_text(&self) -> Option<String> {
        let l = self.lang;
        if !self.link.enabled && !self.link.running {
            return Some(l.t("Связь с телефоном выключена", "Phone link is off").to_string());
        }
        if !self.link.running {
            return Some(match self.link.error.as_deref() {
                Some("no_private_ip") => l.t("Нет домашней сети: подключись к Wi-Fi", "No home network: connect to Wi-Fi"),
                Some(_) => l.t("Порт занят другой программой", "The port is used by another program"),
                None => l.t("Запускаю связь…", "Starting the link…"),
            }
            .to_string());
        }
        None
    }
    pub fn time_label(&self, when: i64) -> String {
        ago(self.wall - when, self.lang)
    }

    // ------------------------------------------------------------------------------------------------ plumbing
    pub fn set_wall(&mut self, ms: i64) {
        self.wall = ms;
    }

    fn call(&mut self, cmd: &'static str, args: Value, p: Pending) -> BackendCall {
        let tag = self.next_tag;
        self.next_tag += 1;
        self.pending.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }

    /// First open of the tab: everything is read once; after that the events keep it current.
    pub fn mount(&mut self) -> Vec<BackendCall> {
        if self.mounted {
            return vec![];
        }
        self.mounted = true;
        vec![self.call("phone_status", Value::Null, Pending::Status), self.call("phone_notifs", Value::Null, Pending::Notifs), self.call("phone_files", Value::Null, Pending::Files)]
    }

    /// The tab became visible: ask for a new pairing code when the old one ran out (nothing wakes the view at the expiry itself).
    pub fn on_show(&mut self, wall_ms: i64) -> Vec<BackendCall> {
        self.wall = wall_ms;
        self.refresh_code()
    }

    fn refresh_code(&mut self) -> Vec<BackendCall> {
        if !self.link.running || self.code_asked {
            return vec![];
        }
        let stale = self.code.as_ref().map_or(true, |c| c.expires <= self.wall);
        if !stale {
            return vec![];
        }
        self.code_asked = true;
        vec![self.call("phone_pair_code", json!({ "renew": self.code.is_some() }), Pending::Code)]
    }

    // ------------------------------------------------------------------------------------------------ events
    pub fn on_event(&mut self, name: &str, p: &Value) -> Vec<BackendCall> {
        match name {
            "phone:state" => self.apply_status(p),
            "phone:notif" => {
                self.on_notif(p);
                vec![]
            }
            "phone:files" => {
                if let Some(t) = p.get("transfer").filter(|t| t.is_object()) {
                    self.upsert_file(t);
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn apply_status(&mut self, v: &Value) -> Vec<BackendCall> {
        let was_running = self.link.running;
        let paired_before = self.link.paired;
        let devices: Vec<Device> = v
            .get("devices")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|d| Device {
                        name: s(d, "name"),
                        connected: b(d, "connected"),
                        battery: d.get("battery").and_then(Value::as_i64),
                        charging: b(d, "charging"),
                        network: s(d, "network"),
                        ssid: opt_s(d, "ssid"),
                        dnd: b(d, "dnd"),
                        model: s(d, "model"),
                        os: s(d, "os"),
                        app_version: s(d, "appVersion"),
                        first_seen: d.get("firstSeen").and_then(Value::as_i64).unwrap_or(0),
                        last_seen: d.get("lastSeen").and_then(Value::as_i64).unwrap_or(0),
                        ip: s(d, "ip"),
                        connected_at: d.get("connectedAt").and_then(Value::as_i64).unwrap_or(0),
                        info: Info::parse(d.get("info").unwrap_or(&Value::Null)),
                        stats: Stats::parse(d.get("stats").unwrap_or(&Value::Null)),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let running = b(v, "running");
        self.link = Link {
            enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(running || self.link.enabled),
            running,
            error: opt_s(v, "error"),
            ip: s(v, "ip"),
            port: v.get("port").and_then(Value::as_u64).unwrap_or(0) as u16,
            pc_name: s(v, "pcName"),
            paired: devices.len(),
            devices,
        };
        if !running {
            self.code = None;
            self.code_asked = false;
            if was_running {
                self.rows.clear(); // the PC forgets the list when the link stops
            }
            return vec![];
        }
        // a pairing used the code: the QR gets a fresh one
        if self.link.paired > paired_before && self.code.is_some() && !self.code_asked {
            self.code = None;
        }
        self.refresh_code()
    }

    fn on_notif(&mut self, p: &Value) {
        let key = s(p, "key");
        match p.get("kind").and_then(Value::as_str).unwrap_or("") {
            "posted" => {
                let Some(n) = p.get("notif") else { return };
                let row = Notif {
                    key: s(n, "key"),
                    pkg: s(n, "pkg"),
                    app: s(n, "app"),
                    title: s(n, "title"),
                    text: s(n, "text"),
                    when: n.get("when").and_then(Value::as_i64).unwrap_or(self.wall),
                    ongoing: b(n, "ongoing"),
                    replay: b(n, "replay"),
                    can_reply: b(n, "canReply"),
                    icon: opt_s(n, "icon"),
                    avatar: opt_s(n, "avatar"),
                    reply: ReplyState::Closed,
                    leaving: false,
                    since: 0.0,
                };
                if row.key.is_empty() {
                    return;
                }
                match self.rows.iter().position(|r| r.key == row.key) {
                    Some(i) if !b(n, "fresh") => {
                        let old = &self.rows[i];
                        let keep = Notif { reply: old.reply.clone(), since: old.since, ..row };
                        self.rows[i] = keep;
                    }
                    found => {
                        let (reply, since) = found.map(|i| (self.rows[i].reply.clone(), self.rows[i].since)).unwrap_or((ReplyState::Closed, 0.0));
                        if let Some(i) = found {
                            self.rows.remove(i);
                        }
                        self.rows.insert(0, Notif { reply, since, ..row });
                    }
                }
                self.rows.truncate(MAX_NOTIFS);
            }
            "removed" | "read" => self.leave(&key, false),
            "reply" => {
                let ok = b(p, "ok");
                let err = s(p, "error");
                let now = self.now;
                if let Some(r) = self.rows.iter_mut().find(|r| r.key == key) {
                    if ok {
                        r.reply = ReplyState::Sent;
                        r.since = now;
                    } else {
                        r.reply = ReplyState::Failed(if err.is_empty() { "failed".into() } else { err });
                    }
                }
            }
            _ => {}
        }
    }

    /// Marks a row as leaving. `force` is the user's own Hide / Mark read; a removal that comes from outside waits for a reply in progress.
    fn leave(&mut self, key: &str, force: bool) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.key == key) {
            if force || !matches!(r.reply, ReplyState::Sending | ReplyState::Sent) {
                r.leaving = true;
            }
        }
    }

    /// The view finished the exit motion of `key`: forget the row.
    pub fn reap(&mut self, key: &str) {
        self.rows.retain(|r| !(r.key == key && r.leaving));
    }

    fn upsert_file(&mut self, t: &Value) {
        let f = FileRow {
            id: s(t, "id"),
            name: s(t, "name"),
            size: t.get("size").and_then(Value::as_u64).unwrap_or(0),
            incoming: t.get("dir").and_then(Value::as_str) == Some("in"),
            status: s(t, "status"),
            bytes: t.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            path: opt_s(t, "path"),
        };
        if f.id.is_empty() {
            return;
        }
        match self.files.iter().position(|x| x.id == f.id) {
            Some(i) => self.files[i] = f,
            None => {
                self.files.insert(0, f);
                self.files.truncate(MAX_FILES);
            }
        }
    }

    // ------------------------------------------------------------------------------------------------ replies
    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        let Some(p) = self.pending.remove(&tag) else { return vec![] };
        match p {
            Pending::Status => {
                if r.ok {
                    return self.apply_status(&r.v);
                }
            }
            Pending::Notifs => {
                self.loaded = true;
                if let Some(a) = r.v.as_array().filter(|_| r.ok) {
                    let old: HashMap<String, Notif> = std::mem::take(&mut self.rows).into_iter().map(|n| (n.key.clone(), n)).collect();
                    for n in a.iter().rev() {
                        self.on_notif(&json!({ "kind": "posted", "notif": n }));
                    }
                    // newest first: the list comes newest first, so inserting in reverse restores it; keep reply state of rows we knew
                    for row in &mut self.rows {
                        if let Some(o) = old.get(&row.key) {
                            row.reply = o.reply.clone();
                            row.since = o.since;
                        }
                    }
                }
            }
            Pending::Files => {
                if let Some(a) = r.v.as_array().filter(|_| r.ok) {
                    self.files.clear();
                    for t in a.iter().rev() {
                        self.upsert_file(t);
                    }
                }
            }
            Pending::Code => {
                self.code_asked = false;
                if r.ok {
                    self.code = Some(Code { code: s(&r.v, "code"), url: s(&r.v, "url"), expires: r.v.get("expires").and_then(Value::as_i64).unwrap_or(0) });
                }
            }
            Pending::Reply(key) => {
                if !r.ok {
                    if let Some(row) = self.rows.iter_mut().find(|x| x.key == key) {
                        row.reply = ReplyState::Failed(r.err.clone());
                    }
                }
            }
            Pending::Other => {}
        }
        vec![]
    }

    // ------------------------------------------------------------------------------------------------ gestures
    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Select(s) => {
                self.sub = s;
                if s == Sub::Phone {
                    return self.refresh_code();
                }
                vec![]
            }
            Action::Hide(key) => self.row_action(&key, "dismiss"),
            Action::MarkRead(key) => self.row_action(&key, "read"),
            Action::ReplyOpen(key) => {
                if let Some(r) = self.rows.iter_mut().find(|r| r.key == key && r.can_reply) {
                    if matches!(r.reply, ReplyState::Closed | ReplyState::Failed(_)) {
                        r.reply = ReplyState::Open;
                    }
                }
                vec![]
            }
            Action::ReplyClose(key) => {
                if let Some(r) = self.rows.iter_mut().find(|r| r.key == key) {
                    if matches!(r.reply, ReplyState::Open | ReplyState::Failed(_)) {
                        r.reply = ReplyState::Closed;
                    }
                }
                vec![]
            }
            Action::ReplySend(key, text) => {
                let text = text.trim().to_string();
                let now = self.now;
                let ok = self.rows.iter_mut().find(|r| r.key == key).filter(|r| r.can_reply && !text.is_empty() && matches!(r.reply, ReplyState::Open | ReplyState::Failed(_))).map(|r| {
                    r.reply = ReplyState::Sending;
                    r.since = now;
                });
                if ok.is_none() {
                    return vec![];
                }
                vec![self.call("phone_notif_action", json!({ "key": key, "action": "reply", "text": text }), Pending::Reply(key))]
            }
            Action::Accept(id) => vec![self.call("phone_file_decide", json!({ "id": id, "accept": true }), Pending::Other)],
            Action::Decline(id) => vec![self.call("phone_file_decide", json!({ "id": id, "accept": false }), Pending::Other)],
            Action::OpenFile(id) => vec![self.call("phone_open_file", json!({ "id": id, "reveal": false }), Pending::Other)],
            Action::Reveal(id) => vec![self.call("phone_open_file", json!({ "id": id, "reveal": true }), Pending::Other)],
            Action::NewCode => {
                if !self.link.running || self.code_asked {
                    return vec![];
                }
                self.code_asked = true;
                vec![self.call("phone_pair_code", json!({ "renew": true }), Pending::Code)]
            }
            Action::Enable => {
                self.link.enabled = true; // the switch shows "starting" until phone:state says more
                vec![self.call("settings_set", json!({ "patch": { "phone.enabled": true } }), Pending::Other)]
            }
        }
    }

    fn row_action(&mut self, key: &str, action: &str) -> Vec<BackendCall> {
        if !self.rows.iter().any(|r| r.key == key && !r.leaving) {
            return vec![];
        }
        self.leave(key, true);
        vec![self.call("phone_notif_action", json!({ "key": key, "action": action }), Pending::Other)]
    }

    /// Advances the "Sent" hold and the reply timeout. Returns true while a timer is still running (the view keeps asking for frames).
    pub fn tick(&mut self, now_ms: f64) -> bool {
        self.now = now_ms;
        let mut running = false;
        for r in &mut self.rows {
            match r.reply {
                ReplyState::Sent if !r.leaving => {
                    if now_ms - r.since >= SENT_HOLD_MS {
                        r.leaving = true;
                    } else {
                        running = true;
                    }
                }
                ReplyState::Sending => {
                    if now_ms - r.since >= REPLY_TIMEOUT_MS {
                        r.reply = ReplyState::Failed("timeout".into());
                    } else {
                        running = true;
                    }
                }
                _ => {}
            }
        }
        running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posted(key: &str, extra: Value) -> Value {
        let mut n = json!({ "key": key, "pkg": "org.t", "app": "Telegram", "title": "Вадим", "text": "привет", "when": 1000, "ongoing": false, "replay": false, "canReply": true, "icon": null, "avatar": null, "fresh": true });
        if let (Some(a), Some(e)) = (n.as_object_mut(), extra.as_object()) {
            a.extend(e.clone());
        }
        json!({ "kind": "posted", "notif": n })
    }

    /// A running link whose first code call is still open.
    fn raw() -> (PhoneModel, Vec<BackendCall>) {
        let mut m = PhoneModel::new(Lang::En);
        let c = m.on_event("phone:state", &json!({ "running": true, "enabled": true, "ip": "192.168.1.5", "port": 41717, "pcName": "PC", "devices": [], "connected": 0 }));
        (m, c)
    }

    /// A running link with a code that never runs out.
    fn model() -> PhoneModel {
        let (mut m, c) = raw();
        m.on_reply(c[0].tag, Reply::ok(json!({ "code": "ZZZZZZZZ", "url": "u", "expires": i64::MAX / 2 })));
        m
    }

    #[test]
    fn mount_reads_status_notifs_and_files_once() {
        let mut m = PhoneModel::new(Lang::En);
        let c = m.mount();
        assert_eq!(c.iter().map(|c| c.cmd).collect::<Vec<_>>(), ["phone_status", "phone_notifs", "phone_files"]);
        assert!(m.mount().is_empty());
    }

    #[test]
    fn status_reply_starts_the_link_and_asks_for_a_code_once() {
        let mut m = PhoneModel::new(Lang::En);
        let c = m.mount();
        assert_eq!(m.empty_text().as_deref(), Some("Phone link is off"));
        let calls = m.on_reply(c[0].tag, Reply::ok(json!({ "enabled": true, "running": true, "ip": "10.0.0.2", "port": 41717, "pcName": "PC", "devices": [] })));
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].cmd, calls[0].args.clone()), ("phone_pair_code", json!({ "renew": false })));
        assert!(m.refresh_code().is_empty(), "already asked");
        m.on_reply(calls[0].tag, Reply::ok(json!({ "code": "AB3D9XKQ", "url": "http://10.0.0.2:41717/p?c=AB3D9XKQ", "expires": 5000 })));
        assert_eq!(m.code().unwrap().code, "AB3D9XKQ");
        assert!(m.empty_text().is_none());
    }

    #[test]
    fn empty_states_follow_the_link() {
        let mut m = PhoneModel::new(Lang::Ru);
        assert_eq!(m.empty_text().as_deref(), Some("Связь с телефоном выключена"));
        m.on_event("phone:state", &json!({ "enabled": true, "running": false, "error": "no_private_ip" }));
        assert!(m.empty_text().unwrap().contains("Wi-Fi"));
        m.on_event("phone:state", &json!({ "enabled": true, "running": false, "error": "port_busy" }));
        assert!(m.empty_text().unwrap().contains("Порт"));
        assert_eq!(m.on_action(Action::Enable)[0].args, json!({ "patch": { "phone.enabled": true } }));
    }

    #[test]
    fn a_stale_code_is_renewed_when_the_tab_is_shown() {
        let (mut m, c) = raw();
        assert_eq!(c.len(), 1);
        assert!(m.on_show(100).is_empty(), "already asked");
        m.on_reply(c[0].tag, Reply::ok(json!({ "code": "AAAAAAAA", "url": "u", "expires": 1000 })));
        assert!(m.on_show(500).is_empty());
        let c = m.on_show(2000);
        assert_eq!(c[0].args, json!({ "renew": true }));
        // the Phone sub-tab also checks
        m.on_reply(c[0].tag, Reply::ok(json!({ "code": "BBBBBBBB", "url": "u", "expires": 90_000 })));
        assert!(m.on_action(Action::Select(Sub::Phone)).is_empty());
        assert_eq!(m.sub(), Sub::Phone);
    }

    #[test]
    fn a_pairing_replaces_the_code() {
        let (mut m, c) = raw();
        m.on_reply(c[0].tag, Reply::ok(json!({ "code": "AAAAAAAA", "url": "u", "expires": 90_000 })));
        let calls = m.on_event("phone:state", &json!({ "running": true, "devices": [{ "name": "Pixel", "connected": true, "battery": 80 }] }));
        assert_eq!(calls.len(), 1, "the used code is replaced: {calls:?}");
        assert!(m.code().is_none());
        assert_eq!(m.link().phone().unwrap().battery, Some(80));
    }

    #[test]
    fn a_device_carries_its_identity_info_and_counters() {
        let mut m = PhoneModel::new(Lang::En);
        m.on_event("phone:state", &json!({ "running": true, "devices": [{ "name": "Pixel", "connected": true, "battery": 80, "model": "Pixel 8", "os": "Android 15", "appVersion": "1.1.0",
            "firstSeen": 5, "lastSeen": 9, "ip": "192.168.1.7", "connectedAt": 7, "info": { "charge": "full", "ringer": "silent" }, "stats": { "notifs": 4, "filesOut": 1, "bytesOut": 9 } }] }));
        let d = m.link().phone().unwrap();
        assert_eq!((d.model.as_str(), d.os.as_str(), d.app_version.as_str(), d.first_seen, d.last_seen, d.ip.as_str(), d.connected_at), ("Pixel 8", "Android 15", "1.1.0", 5, 9, "192.168.1.7", 7));
        assert_eq!((d.info.charge, d.stats.notifs, d.stats.files_out, d.stats.bytes_out), (Some(crate::panel::c::phone_info::Charge::Full), 4, 1, 9));
        // an old PC / phone: none of it, still fine
        m.on_event("phone:state", &json!({ "running": true, "devices": [{ "name": "Old", "connected": false }] }));
        let d = m.link().phone().unwrap();
        assert_eq!((d.info.clone(), d.stats, d.ip.as_str()), (Info::default(), Stats::default(), ""));
    }

    #[test]
    fn notifications_are_newest_first_and_a_repost_updates_in_place() {
        let mut m = model();
        m.on_event("phone:notif", &posted("a", json!({})));
        m.on_event("phone:notif", &posted("b", json!({ "title": "Анна" })));
        assert_eq!(m.rows().iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        m.on_event("phone:notif", &posted("a", json!({ "fresh": false, "text": "same" })));
        assert_eq!(m.rows()[1].text, "same", "not fresh: edited where it stands");
        m.on_event("phone:notif", &posted("a", json!({ "fresh": true, "text": "new words" })));
        assert_eq!(m.rows()[0].key, "a", "fresh: comes to the top");
        assert_eq!(m.rows().len(), 2);
        assert_eq!(m.unread(), 2);
    }

    #[test]
    fn ongoing_and_replay_are_listed_but_flagged() {
        let mut m = model();
        m.on_event("phone:notif", &posted("m", json!({ "ongoing": true, "canReply": false })));
        m.on_event("phone:notif", &posted("r", json!({ "replay": true })));
        assert!(m.rows().iter().find(|r| r.key == "m").unwrap().ongoing);
        assert!(m.rows().iter().find(|r| r.key == "r").unwrap().replay);
        assert_eq!(m.unread(), 1, "ongoing ones do not count");
        assert!(m.on_action(Action::ReplyOpen("m".into())).is_empty());
        assert_eq!(m.rows().iter().find(|r| r.key == "m").unwrap().reply, ReplyState::Closed, "no reply field without canReply");
    }

    #[test]
    fn hide_and_mark_read_send_the_command_and_the_row_leaves() {
        let mut m = model();
        m.on_event("phone:notif", &posted("a", json!({})));
        m.on_event("phone:notif", &posted("b", json!({})));
        let c = m.on_action(Action::Hide("a".into()));
        assert_eq!((c[0].cmd, c[0].args.clone()), ("phone_notif_action", json!({ "key": "a", "action": "dismiss" })));
        assert!(m.rows().iter().find(|r| r.key == "a").unwrap().leaving);
        assert!(m.on_action(Action::Hide("a".into())).is_empty(), "once");
        let c = m.on_action(Action::MarkRead("b".into()));
        assert_eq!(c[0].args["action"], "read");
        assert_eq!(m.unread(), 0);
        m.reap("a");
        assert_eq!(m.rows().len(), 1);
        m.on_event("phone:notif", &json!({ "kind": "removed", "key": "b" }));
        m.reap("b");
        assert!(m.rows().is_empty());
        m.reap("zzz");
    }

    #[test]
    fn the_reply_flow_ends_in_sent_then_the_row_leaves() {
        let mut m = model();
        m.on_event("phone:notif", &posted("a", json!({})));
        assert!(m.on_action(Action::ReplySend("a".into(), "hi".into())).is_empty(), "the field is not open");
        m.on_action(Action::ReplyOpen("a".into()));
        assert_eq!(m.rows()[0].reply, ReplyState::Open);
        assert!(m.on_action(Action::ReplySend("a".into(), "   ".into())).is_empty(), "no empty replies");
        m.tick(1000.0);
        let c = m.on_action(Action::ReplySend("a".into(), " хорошо ".into()));
        assert_eq!((c[0].cmd, c[0].args.clone()), ("phone_notif_action", json!({ "key": "a", "action": "reply", "text": "хорошо" })));
        assert_eq!(m.rows()[0].reply, ReplyState::Sending);
        m.on_reply(c[0].tag, Reply::ok(json!({ "ok": true })));
        assert_eq!(m.rows()[0].reply, ReplyState::Sending, "relayed, not yet confirmed by the phone");
        // the phone removes the notification at the same time: deferred while the reply is on its way
        m.on_event("phone:notif", &json!({ "kind": "removed", "key": "a" }));
        assert!(!m.rows()[0].leaving);
        m.on_event("phone:notif", &json!({ "kind": "reply", "key": "a", "ok": true }));
        assert_eq!(m.rows()[0].reply, ReplyState::Sent);
        assert!(m.tick(1000.0 + SENT_HOLD_MS - 100.0));
        assert!(!m.rows()[0].leaving);
        assert!(!m.tick(1000.0 + SENT_HOLD_MS + 1.0) || m.rows()[0].leaving);
        assert!(m.rows()[0].leaving);
        m.reap("a");
        assert!(m.rows().is_empty());
    }

    #[test]
    fn a_failed_reply_keeps_the_row_and_can_be_retried() {
        let mut m = model();
        m.on_event("phone:notif", &posted("a", json!({})));
        m.on_action(Action::ReplyOpen("a".into()));
        let c = m.on_action(Action::ReplySend("a".into(), "x".into()));
        m.on_reply(c[0].tag, Reply::err("the phone is not connected"));
        assert!(matches!(m.rows()[0].reply, ReplyState::Failed(_)));
        let c = m.on_action(Action::ReplySend("a".into(), "x".into()));
        assert_eq!(c.len(), 1, "retry from the failed state");
        m.on_event("phone:notif", &json!({ "kind": "reply", "key": "a", "ok": false, "error": "expired" }));
        assert_eq!(m.rows()[0].reply, ReplyState::Failed("expired".into()));
        m.on_action(Action::ReplyClose("a".into()));
        assert_eq!(m.rows()[0].reply, ReplyState::Closed);
        assert!(!m.rows()[0].leaving);
    }

    #[test]
    fn a_reply_nobody_confirms_times_out() {
        let mut m = model();
        m.on_event("phone:notif", &posted("a", json!({})));
        m.on_action(Action::ReplyOpen("a".into()));
        m.tick(10.0);
        m.on_action(Action::ReplySend("a".into(), "x".into()));
        assert!(m.tick(5000.0));
        assert!(!m.tick(10.0 + REPLY_TIMEOUT_MS + 1.0));
        assert_eq!(m.rows()[0].reply, ReplyState::Failed("timeout".into()));
    }

    #[test]
    fn the_notifs_reply_restores_the_list_newest_first_and_keeps_open_replies() {
        let mut m = model();
        m.on_event("phone:notif", &posted("a", json!({})));
        m.on_action(Action::ReplyOpen("a".into()));
        let mut mm = PhoneModel::new(Lang::En);
        let c = mm.mount();
        mm.on_reply(c[1].tag, Reply::ok(json!([{ "key": "n2", "title": "two", "fresh": true }, { "key": "n1", "title": "one", "fresh": true }])));
        assert_eq!(mm.rows().iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["n2", "n1"]);
        let c = m.mount();
        m.on_reply(c[1].tag, Reply::ok(json!([{ "key": "a", "title": "Вадим", "canReply": true }])));
        assert_eq!(m.rows()[0].reply, ReplyState::Open);
    }

    #[test]
    fn files_are_tracked_by_id_and_decided() {
        let mut m = model();
        let t = |st: &str, bytes: u64| json!({ "kind": "offer", "transfer": { "id": "f1", "name": "a.jpg", "size": 100, "dir": "in", "status": st, "bytes": bytes, "path": null } });
        m.on_event("phone:files", &t("pending", 0));
        m.on_event("phone:files", &json!({ "kind": "offered", "transfer": { "id": "f2", "name": "r.docx", "size": 10, "dir": "out", "status": "accepted", "bytes": 0 } }));
        assert_eq!(m.files().iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["f2", "f1"]);
        assert!(m.files()[1].pending() && !m.files()[0].incoming);
        assert_eq!(m.on_action(Action::Accept("f1".into()))[0].args, json!({ "id": "f1", "accept": true }));
        assert_eq!(m.on_action(Action::Decline("f1".into()))[0].args, json!({ "id": "f1", "accept": false }));
        m.on_event("phone:files", &t("active", 25));
        assert_eq!(m.files()[1].progress(), Some(0.25));
        m.on_event("phone:files", &json!({ "kind": "done", "transfer": { "id": "f1", "name": "a.jpg", "size": 100, "dir": "in", "status": "done", "bytes": 100, "path": "C:\\dl\\a.jpg" } }));
        assert!(m.files()[1].has_file() && m.files()[1].progress().is_none());
        assert_eq!(m.on_action(Action::OpenFile("f1".into()))[0].args, json!({ "id": "f1", "reveal": false }));
        assert_eq!(m.on_action(Action::Reveal("f1".into()))[0].args, json!({ "id": "f1", "reveal": true }));
        assert_eq!(m.files().len(), 2);
    }

    #[test]
    fn stopping_the_link_clears_the_list() {
        let mut m = model();
        m.on_event("phone:notif", &posted("a", json!({})));
        m.on_event("phone:state", &json!({ "running": false, "enabled": false }));
        assert!(m.rows().is_empty());
        assert!(m.empty_text().is_some());
    }

    #[test]
    fn labels() {
        assert_eq!(ago(10_000, Lang::En), "now");
        assert_eq!(ago(5 * 60_000, Lang::En), "5 min");
        assert_eq!(ago(3 * 3_600_000, Lang::Ru), "3 ч");
        assert_eq!(ago(-5, Lang::En), "now");
        assert_eq!(ago(2 * 86_400_000, Lang::En), "2 d");
        assert_eq!(size_text(512, Lang::En), "512 B");
        assert_eq!(size_text(1536 * 1024, Lang::En), "1.5 MB");
        assert_eq!(size_text(3 * 1024, Lang::Ru), "3.0 КБ");
        assert_eq!(Sub::Files.title(Lang::Ru), "Файлы");
    }
}
