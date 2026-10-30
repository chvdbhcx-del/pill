//! Notification centre tab: the history of what the island showed (downloads, USB, battery, calendar, mail, agents, OBS, Steam,
//! Windows notifications ...). Port of `src/tab-notifs.js`.
//!
//! Capture: the shell calls [`Notifs::on_toast`] for every card it shows (the JS wrapped `Pill.toast`); the producer's card id
//! prefix decides the kind. Storage is `notifhist.rs` (SQLite, 200 entries / 14 days): `notif_history_list|add|clear|mark_read|remove`.
//! Settings: `module.notifs` (tab on/off), `history.keep` (record at all, default on), `history.mail` (keep mail / Windows
//! notification bodies; default off = title only, enforced in Rust too), `clock24`.
//! PRIVACY: local only; OCR / QR texts of the capture cards are never stored (title only).

use super::anim::Anim;
use super::util::{clock_label, datetime_label, day_month_label, day_start, Wall};
use super::{i64_of, str_of, BackendCall, Cfg, Fx, FxQueue, Tr};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const MAX: usize = 200;
pub const MAX_AGE_MS: i64 = 14 * 24 * 3600 * 1000;
/// A producer that repeats itself within this window is recorded once.
pub const DEDUPE_MS: i64 = 4000;
/// "Click again" window of the clear button.
pub const CLEAR_ARM_MS: f64 = 3000.0;

const KINDS: [(&str, &str); 15] = [
    ("notif:", "notif"),
    ("mail", "mail"),
    ("filedone", "download"),
    ("download:", "download"),
    ("steam-done", "steam"),
    ("usb:", "usb"),
    ("battery", "battery"),
    ("cal:", "calendar"),
    ("agent", "agent"),
    ("obs", "obs"),
    ("update", "update"),
    ("timer-done", "reminder"),
    ("remind-", "reminder"),
    ("voice", "voice"),
    ("cap", "capture"),
];

/// Icon of a kind when the producer named none.
pub fn kind_icon(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "notif" => "bell",
        "mail" => "mail",
        "download" | "steam" => "download",
        "usb" => "usb",
        "battery" => "battery",
        "calendar" => "calendar",
        "agent" => "terminal",
        "obs" | "capture" => "video",
        "update" => "refresh",
        "reminder" => "bell",
        "voice" => "mic",
        _ => return None,
    })
}

/// Feedback toasts of the island's own buttons are not events.
pub fn is_skipped(id: &str) -> bool {
    ["clip-", "launcher-", "dev-settings", "dict-"].iter().any(|p| id.starts_with(p))
}

pub fn kind_of(id: &str) -> String {
    if let Some((_, k)) = KINDS.iter().find(|(p, _)| id.starts_with(p)) {
        return k.to_string();
    }
    let head = id.split(':').next().unwrap_or("");
    let k: String = head.chars().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).take(16).collect();
    if k.is_empty() {
        "event".to_string()
    } else {
        k
    }
}

fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A card the island showed, as the shell reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToastRec {
    pub id: String,
    pub title: String,
    pub sub: String,
    /// A named glyph; markup (starting with `<`) is not a name.
    pub icon: String,
    pub tone: String,
    /// Plain text of an html card whose title / body were not split (used when `title` is empty).
    pub text: String,
}

/// `notifhist.rs::Entry`
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub ts: i64,
    pub kind: String,
    pub icon: String,
    pub tone: String,
    pub title: String,
    pub sub: String,
    pub read: bool,
}

impl Entry {
    pub fn from_json(v: &Value) -> Entry {
        Entry {
            id: i64_of(v, "id"),
            ts: i64_of(v, "ts"),
            kind: str_of(v, "kind").to_string(),
            icon: str_of(v, "icon").to_string(),
            tone: str_of(v, "tone").to_string(),
            title: str_of(v, "title").to_string(),
            sub: str_of(v, "sub").to_string(),
            read: v.get("read").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The clear button (first click arms, second within 3 s clears).
    Clear,
    /// The x of a row.
    Remove(i64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: i64,
    pub kind: String,
    pub icon: String,
    pub tone: String,
    pub title: String,
    pub sub: String,
    /// Unread when the tab was opened: its dot stays until the tab is left.
    pub unread: bool,
    pub time: String,
    pub time_tooltip: String,
    pub remove_title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Day(String),
    Row(Row),
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub count: String,
    pub empty: bool,
    pub off: bool,
    pub clear_visible: bool,
    pub clear_armed: bool,
    pub clear_label: String,
    pub items: Vec<Item>,
    pub empty_title: String,
    pub empty_text: String,
    pub footer: String,
}

pub struct Notifs {
    cfg: Cfg,
    mono: f64,
    items: Vec<Entry>,
    shown: bool,
    clear_arm: Option<f64>,
    fresh: HashSet<i64>,
    loaded: bool,
    pending: Vec<Value>,
    recent: HashMap<String, i64>,
    last_badge: Option<String>,
    fx: FxQueue,
}

impl Default for Notifs {
    fn default() -> Self {
        Notifs::new()
    }
}

impl Notifs {
    pub fn new() -> Notifs {
        Notifs {
            cfg: Cfg::new(),
            mono: 0.0,
            items: vec![],
            shown: false,
            clear_arm: None,
            fresh: HashSet::new(),
            loaded: false,
            pending: vec![],
            recent: HashMap::new(),
            last_badge: None,
            fx: FxQueue::default(),
        }
    }

    /// Calls to run at startup (the stored history).
    pub fn init_calls(&self) -> Vec<BackendCall> {
        vec![BackendCall::bare("notif_history_list")]
    }
    pub fn set_settings(&mut self, v: &Value) {
        self.cfg.apply(v);
        self.badge();
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.cfg.os_reduce = on;
    }
    pub fn take_fx(&mut self) -> Vec<Fx> {
        self.fx.take()
    }
    pub fn entries(&self) -> &[Entry] {
        &self.items
    }
    pub fn unread(&self) -> usize {
        self.items.iter().filter(|e| !e.read).count()
    }
    /// Recording happens only with the module on and `history.keep` not off.
    pub fn enabled(&self) -> bool {
        self.cfg.module_on("module.notifs") && self.cfg.not_false("history.keep")
    }
    fn reduce(&self) -> bool {
        self.cfg.reduce_motion()
    }

    fn badge(&mut self) {
        let n = self.unread();
        let text = if n == 0 { String::new() } else if n > 99 { "99+".to_string() } else { n.to_string() };
        if self.last_badge.as_deref() != Some(text.as_str()) {
            self.last_badge = Some(text.clone());
            self.fx.push(Fx::Badge { tab: "notifs", text });
        }
    }

    /// `entryOf`: the stored form of a card, or `None` when it is not recorded.
    pub fn entry_of(&self, o: &ToastRec) -> Option<Value> {
        if is_skipped(&o.id) {
            return None;
        }
        let kind = kind_of(&o.id);
        let mut title = flat(&o.title);
        let mut sub = flat(&o.sub);
        if title.is_empty() && !o.text.is_empty() {
            title = flat(&o.text).chars().take(200).collect();
            sub.clear();
        }
        if title.is_empty() {
            return None;
        }
        let named = if o.icon.is_empty() || o.icon.starts_with('<') { "" } else { o.icon.as_str() };
        let icon = if !named.is_empty() { named.to_string() } else { kind_icon(&kind).unwrap_or("bell").to_string() };
        // the same rule runs in notifhist.rs: the body does not even cross the bridge
        if (kind == "mail" || kind == "notif") && self.cfg.get("history.mail") != Some(&Value::Bool(true)) {
            sub.clear();
        }
        if kind == "capture" && (icon == "scantext" || icon == "qr") {
            sub.clear(); // the recognised text itself stays out of the history
        }
        Some(json!({ "kind": kind, "icon": icon, "tone": o.tone, "title": title, "sub": sub }))
    }

    /// A card was shown by the island. `now_ms` is the wall clock (dedupe window). Returns the `notif_history_add` call, if any.
    pub fn on_toast(&mut self, o: &ToastRec, now_ms: i64) -> Vec<BackendCall> {
        if !self.enabled() {
            return vec![];
        }
        let Some(e) = self.entry_of(o) else { return vec![] };
        let key = format!("{}|{}|{}", o.id, str_of(&e, "title"), str_of(&e, "sub"));
        if now_ms - self.recent.get(&key).copied().unwrap_or(0) < DEDUPE_MS {
            return vec![];
        }
        self.recent.insert(key, now_ms);
        if self.recent.len() > 50 {
            self.recent.retain(|_, v| now_ms - *v <= DEDUPE_MS);
        }
        if !self.loaded {
            self.pending.push(e); // adds wait for the stored history, so nothing is lost or doubled
            return vec![];
        }
        vec![self.add_call(e)]
    }

    fn add_call(&self, e: Value) -> BackendCall {
        BackendCall::new("notif_history_add", json!({ "entry": e, "read": self.shown }))
    }

    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.shown = true;
        self.fresh.clear();
        let ids: Vec<i64> = self.items.iter().filter(|e| !e.read).map(|e| e.id).collect();
        self.fresh.extend(ids.iter().copied());
        let mut calls = vec![];
        if !ids.is_empty() {
            for e in self.items.iter_mut() {
                e.read = true;
            }
            calls.push(BackendCall::new("notif_history_mark_read", json!({ "ids": ids })));
        }
        // staggered entry of the rows (and the unread dots popping a moment later)
        let reduce = self.reduce();
        for (i, e) in self.items.iter().enumerate() {
            self.fx.anim(reduce, format!("row:{}", e.id), Anim::row_stagger(i));
            if self.fresh.contains(&e.id) {
                self.fx.anim(reduce, format!("dot:{}", e.id), Anim::pop(0.0, 400).delayed(120));
            }
        }
        self.badge();
        calls
    }

    pub fn on_hide(&mut self) {
        self.shown = false;
        self.clear_arm = None;
        self.fresh.clear();
    }

    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        if name == "settings:changed" && payload.is_object() {
            self.cfg.apply(payload);
            self.badge();
        }
        vec![]
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Clear => {
                if self.clear_arm.is_none() {
                    self.clear_arm = Some(self.mono + CLEAR_ARM_MS); // two clicks within 3 s, like the clipboard tab
                    return vec![];
                }
                self.clear_arm = None;
                let reduce = self.reduce();
                for (i, e) in self.items.iter().enumerate() {
                    self.fx.anim(reduce, format!("row:{}", e.id), Anim::exit(-4.0, 1.0).delayed(i.min(8) as u32 * 20));
                }
                vec![BackendCall::bare("notif_history_clear")]
            }
            Action::Remove(id) => {
                self.items.retain(|e| e.id != id);
                self.fresh.remove(&id);
                self.badge();
                let reduce = self.reduce();
                self.fx.anim(reduce, format!("row:{id}"), Anim::collapse());
                if self.items.is_empty() {
                    self.fx.anim(reduce, "empty", Anim::enter(6.0).delayed(Anim::collapse().dur_ms));
                }
                vec![BackendCall::new("notif_history_remove", json!({ "id": id }))]
            }
        }
    }

    pub fn on_result(&mut self, cmd: &str, _args: &Value, res: &Value) -> Vec<BackendCall> {
        match cmd {
            "notif_history_list" => {
                self.items = res.as_array().map(|a| a.iter().map(Entry::from_json).collect()).unwrap_or_default();
                self.loaded = true;
                self.badge();
                let pending = std::mem::take(&mut self.pending);
                pending.into_iter().map(|e| self.add_call(e)).collect()
            }
            "notif_history_add" => {
                if !res.is_object() {
                    return vec![];
                }
                let s = Entry::from_json(res);
                if self.items.iter().any(|x| x.id == s.id) {
                    return vec![];
                }
                let id = s.id;
                self.items.insert(0, s);
                self.items.truncate(MAX);
                if self.shown {
                    self.fresh.insert(id);
                    self.fx.anim(self.reduce(), "list", Anim::flip(true));
                } else {
                    self.badge();
                }
                vec![]
            }
            "notif_history_clear" => {
                self.items.clear();
                self.fresh.clear();
                self.fx.anim(self.reduce(), "empty", Anim::enter(6.0));
                self.badge();
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        self.mono += dt_ms;
        if self.clear_arm.map_or(false, |at| at <= self.mono) {
            self.clear_arm = None;
        }
        vec![]
    }

    // ---------------------------------------------------------------- view
    fn rel(&self, tr: &dyn Tr, ts: i64, now: Wall) -> String {
        let s = (now.ms - ts) as f64 / 1000.0;
        if s < 45.0 {
            return tr.t("только что", "just now");
        }
        let m = (s / 60.0 + 0.5).floor() as i64;
        if m < 60 {
            return tr.t(&format!("{m} мин назад"), &format!("{m} min ago"));
        }
        let h = m / 60;
        if h < 24 {
            return tr.t(&format!("{h} ч назад"), &format!("{h} h ago"));
        }
        clock_label(Wall::new(ts, now.tz_min), !self.cfg.clock24())
    }

    fn day_label(&self, tr: &dyn Tr, d0: i64, now: Wall) -> String {
        let today = day_start(now);
        if d0 == today {
            return tr.t("Сегодня", "Today");
        }
        if d0 == day_start(Wall::new(today - 1, now.tz_min)) {
            return tr.t("Вчера", "Yesterday");
        }
        day_month_label(Wall::new(d0, now.tz_min), tr.is_ru())
    }

    /// Entries newer than 14 days.
    fn alive(&self, now: Wall) -> Vec<&Entry> {
        self.items.iter().filter(|e| e.ts >= now.ms - MAX_AGE_MS).collect()
    }

    pub fn view(&self, tr: &dyn Tr, now: Wall) -> View {
        let off = !self.cfg.not_false("history.keep");
        let alive = self.alive(now);
        let n = alive.len();
        let mut items = vec![];
        let mut day: Option<i64> = None;
        for e in alive {
            let d0 = day_start(Wall::new(e.ts, now.tz_min));
            if day != Some(d0) {
                day = Some(d0);
                items.push(Item::Day(self.day_label(tr, d0, now)));
            }
            items.push(Item::Row(Row {
                id: e.id,
                kind: e.kind.clone(),
                icon: if e.icon.is_empty() { kind_icon(&e.kind).unwrap_or("bell").to_string() } else { e.icon.clone() },
                tone: e.tone.clone(),
                title: e.title.clone(),
                sub: e.sub.clone(),
                unread: self.fresh.contains(&e.id),
                time: self.rel(tr, e.ts, now),
                time_tooltip: datetime_label(Wall::new(e.ts, now.tz_min), tr.is_ru()),
                remove_title: tr.t("Удалить", "Remove"),
            }));
        }
        View {
            count: if off {
                tr.t("История выключена", "History is off")
            } else if n > 0 {
                format!("{}{}", tr.t("Всего: ", "Total: "), n)
            } else {
                tr.t("Уведомления", "Notifications")
            },
            empty: n == 0,
            off,
            clear_visible: n > 0,
            clear_armed: self.clear_arm.is_some(),
            clear_label: if self.clear_arm.is_some() { tr.t("Нажми ещё раз", "Click again") } else { tr.t("Очистить историю", "Clear history") },
            items,
            empty_title: tr.t("Пока ничего нет", "Nothing here yet"),
            empty_text: tr.t("Здесь останутся загрузки, события и агенты, о которых сообщал остров", "Downloads, events and agents the island told you about will stay here"),
            footer: if off {
                tr.t("Запись выключена: Настройки → Модули → «Хранить историю».", "Recording is off: Settings → Modules → “Keep history”.")
            } else {
                tr.t("Только на этом ПК · 14 дней, до 200 записей", "Stored on this PC only · 14 days, up to 200 entries")
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    const NOW: i64 = 1_793_369_109_000; // 2026-10-30 14:05:09 UTC
    const W: Wall = Wall { ms: NOW, tz_min: 0 };

    fn entry(id: i64, ts: i64, read: bool) -> Value {
        json!({ "id": id, "ts": ts, "kind": "download", "icon": "download", "tone": "", "title": format!("File {id}"), "sub": "done", "read": read })
    }

    fn loaded(list: Value) -> Notifs {
        let mut n = Notifs::new();
        n.on_result("notif_history_list", &Value::Null, &list);
        n.take_fx();
        n
    }

    fn rec(id: &str, title: &str, sub: &str) -> ToastRec {
        ToastRec { id: id.into(), title: title.into(), sub: sub.into(), ..Default::default() }
    }

    #[test]
    fn kinds_by_id_prefix() {
        for (id, k) in [
            ("notif:abc", "notif"), ("mail-1", "mail"), ("filedone", "download"), ("download:7", "download"), ("steam-done", "steam"),
            ("usb:E", "usb"), ("battery-low", "battery"), ("cal:1", "calendar"), ("agent-done", "agent"), ("obs-rec", "obs"),
            ("update-x", "update"), ("timer-done", "reminder"), ("remind-3", "reminder"), ("voice-err", "voice"), ("cap-shot", "capture"),
            ("Weird_ID:9", "weirdid"), ("", "event"), ("123:4", "event"), ("averyveryverylongkindname:1", "averyveryverylon"),
        ] {
            assert_eq!(kind_of(id), k, "{id}");
        }
        assert_eq!(kind_icon("steam"), Some("download"));
        assert_eq!(kind_icon("capture"), Some("video"));
        assert_eq!(kind_icon("weirdid"), None);
    }

    #[test]
    fn own_feedback_toasts_are_never_recorded() {
        let n = loaded(json!([]));
        for id in ["clip-paste", "launcher-open", "dev-settings", "dict-soon"] {
            assert!(n.entry_of(&rec(id, "x", "")).is_none(), "{id}");
        }
        assert!(n.entry_of(&rec("download:1", "", "")).is_none()); // no title
    }

    #[test]
    fn entry_flattens_whitespace_and_picks_icons() {
        let n = loaded(json!([]));
        let mut r = rec("download:1", "  a \n b  ", " c\t d ");
        let e = n.entry_of(&r).unwrap();
        assert_eq!(e, json!({ "kind": "download", "icon": "download", "tone": "", "title": "a b", "sub": "c d" }));
        r.icon = "folder".into();
        r.tone = "ok".into();
        let e = n.entry_of(&r).unwrap();
        assert_eq!((e["icon"].as_str(), e["tone"].as_str()), (Some("folder"), Some("ok")));
        r.icon = "<svg/>".into(); // markup is not a name
        assert_eq!(n.entry_of(&r).unwrap()["icon"], "download");
        assert_eq!(n.entry_of(&rec("unknown:1", "t", "")).unwrap()["icon"], "bell");
    }

    #[test]
    fn html_cards_fall_back_to_their_text() {
        let n = loaded(json!([]));
        let r = ToastRec { id: "notif:1".into(), text: format!("  {}  ", "z".repeat(300)), ..Default::default() };
        let e = n.entry_of(&r).unwrap();
        assert_eq!(e["title"].as_str().unwrap().len(), 200);
        assert_eq!(e["sub"], "");
    }

    #[test]
    fn mail_and_notification_bodies_need_history_mail() {
        let mut n = loaded(json!([]));
        assert_eq!(n.entry_of(&rec("mail-1", "From Bob", "secret body")).unwrap()["sub"], "");
        assert_eq!(n.entry_of(&rec("notif:2", "Chat", "hi there")).unwrap()["sub"], "");
        assert_eq!(n.entry_of(&rec("download:3", "File", "kept")).unwrap()["sub"], "kept");
        n.set_settings(&json!({ "history.mail": true }));
        assert_eq!(n.entry_of(&rec("mail-1", "From Bob", "secret body")).unwrap()["sub"], "secret body");
    }

    #[test]
    fn capture_cards_never_store_recognised_text() {
        let n = loaded(json!([]));
        let mut r = rec("cap-text", "Text copied", "my password is hunter2");
        r.icon = "scantext".into();
        assert_eq!(n.entry_of(&r).unwrap()["sub"], "");
        r.icon = "qr".into();
        assert_eq!(n.entry_of(&r).unwrap()["sub"], "");
        r.icon = "video".into();
        assert_eq!(n.entry_of(&r).unwrap()["sub"], "my password is hunter2");
    }

    #[test]
    fn recording_respects_module_and_history_switches() {
        let mut n = loaded(json!([]));
        assert_eq!(n.on_toast(&rec("download:1", "A", ""), NOW).len(), 1);
        n.set_settings(&json!({ "history.keep": false }));
        assert!(n.on_toast(&rec("download:2", "B", ""), NOW).is_empty());
        n.set_settings(&json!({ "module.notifs": false }));
        assert!(n.on_toast(&rec("download:3", "C", ""), NOW).is_empty());
        n.set_settings(&json!({}));
        assert_eq!(n.on_toast(&rec("download:4", "D", ""), NOW).len(), 1);
    }

    #[test]
    fn repeats_within_four_seconds_are_recorded_once() {
        let mut n = loaded(json!([]));
        let r = rec("download:1", "A", "x");
        assert_eq!(n.on_toast(&r, NOW).len(), 1);
        assert!(n.on_toast(&r, NOW + 3999).is_empty());
        assert_eq!(n.on_toast(&r, NOW + 4000).len(), 1);
        assert_eq!(n.on_toast(&rec("download:1", "A", "y"), NOW + 4001).len(), 1); // different body = different key
    }

    #[test]
    fn adds_wait_for_the_stored_history_then_flush_in_order() {
        let mut n = Notifs::new();
        assert!(n.on_toast(&rec("download:1", "A", ""), NOW).is_empty());
        assert!(n.on_toast(&rec("download:2", "B", ""), NOW).is_empty());
        let calls = n.on_result("notif_history_list", &Value::Null, &json!([]));
        assert_eq!(calls.iter().map(|c| c.args["entry"]["title"].as_str().unwrap()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(calls[0].args["read"], false);
        // after loading, adds go straight out and carry whether the tab is open
        n.on_show();
        assert_eq!(n.on_toast(&rec("download:3", "C", ""), NOW + 10).first().unwrap().args["read"], true);
    }

    #[test]
    fn new_entry_while_hidden_raises_the_badge_while_shown_it_flips() {
        let mut n = loaded(json!([entry(1, NOW - 1000, true)]));
        n.on_result("notif_history_add", &Value::Null, &entry(2, NOW, false));
        assert_eq!(n.unread(), 1);
        assert_eq!(n.take_fx(), vec![Fx::Badge { tab: "notifs", text: "1".into() }]);
        // duplicates by id are ignored
        n.on_result("notif_history_add", &Value::Null, &entry(2, NOW, false));
        assert_eq!(n.entries().len(), 2);
        assert!(n.on_result("notif_history_add", &Value::Null, &Value::Null).is_empty());
        n.on_show();
        n.take_fx();
        n.on_result("notif_history_add", &Value::Null, &entry(3, NOW, true));
        let fx = n.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "list" && anim.dur_ms == 400)));
        assert!(!fx.iter().any(|f| matches!(f, Fx::Badge { .. })));
        let v = n.view(&EnTr, W);
        let Item::Row(r) = &v.items[1] else { panic!() };
        assert!(r.unread && r.id == 3);
    }

    #[test]
    fn badge_caps_at_99_plus_and_hides_at_zero() {
        let list: Vec<Value> = (1..=120).map(|i| entry(i, NOW - i, false)).collect();
        let mut n = Notifs::new();
        n.on_result("notif_history_list", &Value::Null, &Value::Array(list));
        assert_eq!(n.take_fx(), vec![Fx::Badge { tab: "notifs", text: "99+".into() }]);
        let calls = n.on_show();
        assert_eq!(calls[0].cmd, "notif_history_mark_read");
        assert_eq!(calls[0].args["ids"].as_array().unwrap().len(), 120);
        assert_eq!(n.unread(), 0);
        assert!(n.take_fx().iter().any(|f| matches!(f, Fx::Badge { text, .. } if text.is_empty())));
        // nothing unread: no mark-read call
        n.on_hide();
        assert!(n.on_show().is_empty());
    }

    #[test]
    fn dots_stay_until_the_tab_is_left() {
        let mut n = loaded(json!([entry(1, NOW - 10_000, false), entry(2, NOW - 20_000, true)]));
        n.on_show();
        let v = n.view(&EnTr, W);
        let unread: Vec<_> = v.items.iter().filter_map(|i| if let Item::Row(r) = i { Some(r.unread) } else { None }).collect();
        assert_eq!(unread, [true, false]);
        n.on_hide();
        let v = n.view(&EnTr, W);
        assert!(v.items.iter().all(|i| !matches!(i, Item::Row(r) if r.unread)));
    }

    #[test]
    fn show_staggers_rows_and_pops_dots() {
        let mut n = loaded(json!([entry(1, NOW - 10_000, false), entry(2, NOW - 20_000, true)]));
        n.on_show();
        let fx = n.take_fx();
        let delay = |t: &str| fx.iter().find_map(|f| if let Fx::Anim { target, anim } = f { (target == t).then_some(anim.delay_ms) } else { None });
        assert_eq!(delay("row:1"), Some(0));
        assert_eq!(delay("row:2"), Some(28));
        assert_eq!(delay("dot:1"), Some(120));
        assert_eq!(delay("dot:2"), None);
    }

    #[test]
    fn day_groups_and_relative_times() {
        let list = json!([
            entry(1, NOW - 10_000, true),            // just now
            entry(2, NOW - 5 * 60_000, true),        // 5 min
            entry(3, NOW - 3 * 3_600_000, true),     // 3 h
            entry(4, NOW - 26 * 3_600_000, true),    // yesterday at noon (14:05 - 26 h = 12:05 on the 29th)
            entry(5, NOW - 40 * 3_600_000, true),    // the 28th
        ]);
        let n = loaded(list);
        let v = n.view(&EnTr, W);
        let days: Vec<_> = v.items.iter().filter_map(|i| if let Item::Day(d) = i { Some(d.as_str()) } else { None }).collect();
        assert_eq!(days, ["Today", "Yesterday", "October 28"]);
        let times: Vec<_> = v.items.iter().filter_map(|i| if let Item::Row(r) = i { Some(r.time.as_str()) } else { None }).collect();
        assert_eq!(times, ["just now", "5 min ago", "3 h ago", "12:05", "22:05"]);
        let ru = n.view(&RuTr, W);
        assert!(matches!(&ru.items[0], Item::Day(d) if d == "Сегодня"));
        assert!(ru.items.iter().any(|i| matches!(i, Item::Row(r) if r.time == "5 мин назад")));
        assert!(ru.items.iter().any(|i| matches!(i, Item::Day(d) if d == "28 октября")));
        let Item::Row(r) = &v.items[1] else { panic!() };
        assert_eq!(r.time_tooltip, "10/30/2026, 2:04:59 PM");
    }

    #[test]
    fn relative_time_boundaries() {
        let n = loaded(json!([]));
        let r = |s: i64| n.rel(&EnTr, NOW - s * 1000, W);
        assert_eq!(r(44), "just now");
        assert_eq!(r(45), "1 min ago");
        assert_eq!(r(59 * 60 + 29), "59 min ago");
        assert_eq!(r(59 * 60 + 30), "1 h ago"); // rounds to 60 min
        assert_eq!(r(23 * 3600 + 29 * 60), "23 h ago");
    }

    #[test]
    fn entries_older_than_14_days_are_hidden() {
        let n = loaded(json!([entry(1, NOW - 1000, true), entry(2, NOW - MAX_AGE_MS - 1, true)]));
        let v = n.view(&EnTr, W);
        assert_eq!(v.count, "Total: 1");
        assert_eq!(v.items.iter().filter(|i| matches!(i, Item::Row(_))).count(), 1);
    }

    #[test]
    fn header_states_and_footer() {
        let mut n = loaded(json!([]));
        let v = n.view(&EnTr, W);
        assert!(v.empty && !v.clear_visible);
        assert_eq!(v.count, "Notifications");
        assert_eq!(v.footer, "Stored on this PC only · 14 days, up to 200 entries");
        n.set_settings(&json!({ "history.keep": false }));
        let v = n.view(&RuTr, W);
        assert!(v.off);
        assert_eq!(v.count, "История выключена");
        assert!(v.footer.starts_with("Запись выключена"));
    }

    #[test]
    fn clear_needs_two_clicks_within_three_seconds() {
        let mut n = loaded(json!([entry(1, NOW - 1000, true), entry(2, NOW - 2000, true)]));
        assert!(n.on_action(Action::Clear).is_empty());
        let v = n.view(&EnTr, W);
        assert!(v.clear_armed);
        assert_eq!(v.clear_label, "Click again");
        n.tick(3001.0);
        assert!(!n.view(&EnTr, W).clear_armed);
        assert_eq!(n.view(&RuTr, W).clear_label, "Очистить историю");
        n.on_action(Action::Clear);
        n.tick(1000.0);
        let c = n.on_action(Action::Clear);
        assert_eq!(c, vec![BackendCall::bare("notif_history_clear")]);
        let fx = n.take_fx();
        assert_eq!(fx.iter().filter(|f| matches!(f, Fx::Anim { anim, .. } if anim.dur_ms == 160)).count(), 2);
        assert_eq!(n.entries().len(), 2); // rows stay until the backend answers
        n.on_result("notif_history_clear", &Value::Null, &Value::Null);
        assert!(n.entries().is_empty());
        assert!(n.view(&EnTr, W).empty);
        assert!(n.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "empty")));
    }

    #[test]
    fn leaving_the_tab_disarms_clear() {
        let mut n = loaded(json!([entry(1, NOW - 1000, true)]));
        n.on_action(Action::Clear);
        n.on_hide();
        assert!(!n.view(&EnTr, W).clear_armed);
    }

    #[test]
    fn removing_one_updates_at_once_and_asks_the_backend() {
        let mut n = loaded(json!([entry(1, NOW - 1000, false), entry(2, NOW - 2000, true)]));
        let c = n.on_action(Action::Remove(1));
        assert_eq!(c, vec![BackendCall::new("notif_history_remove", json!({ "id": 1 }))]);
        assert_eq!(n.entries().len(), 1);
        let fx = n.take_fx();
        assert!(fx.contains(&Fx::Badge { tab: "notifs", text: String::new() }));
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "row:1" && anim.dur_ms == 300)));
        assert!(!fx.iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "empty")));
        n.on_action(Action::Remove(2));
        assert!(n.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "empty" && anim.delay_ms == 300)));
    }

    #[test]
    fn history_is_capped_at_200() {
        let list: Vec<Value> = (1..=200).map(|i| entry(i, NOW - i, true)).collect();
        let mut n = loaded(Value::Array(list));
        n.on_result("notif_history_add", &Value::Null, &entry(500, NOW, true));
        assert_eq!(n.entries().len(), 200);
        assert_eq!(n.entries()[0].id, 500);
        assert!(n.entries().iter().all(|e| e.id != 200));
    }

    #[test]
    fn reduced_motion_pushes_no_row_cues() {
        let mut n = loaded(json!([entry(1, NOW - 1000, false)]));
        n.set_settings(&json!({ "reduceMotion": true }));
        n.take_fx();
        n.on_show();
        n.on_action(Action::Remove(1));
        assert!(n.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
    }

    #[test]
    fn twelve_hour_clock_for_old_entries() {
        let mut n = loaded(json!([entry(1, NOW - 26 * 3_600_000, true)]));
        n.set_settings(&json!({ "clock24": false }));
        let v = n.view(&EnTr, W);
        let Item::Row(r) = &v.items[1] else { panic!() };
        assert_eq!(r.time, "12:05 PM");
    }
}
