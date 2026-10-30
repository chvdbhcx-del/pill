//! More live events as cards: finished downloads, removable drives, battery, next calendar event, new mail
//! (port of live-plus.js; the producers live in src-tauri/src/liveplus.rs and mail.rs).
//!
//! * `live:file-done {name, path, more}` -> "Downloaded: <name>" card; a click on the card reveals the file. Delayed by 1.5 s so a browser
//!   download that livesys.rs announces itself (`live:download` done) does not get a second card.
//! * `live:usb {letter, added}` -> "Drive E: connected" / "Drive E: disconnected".
//! * `live:battery {kind: low|charging|full, percent}` -> "Battery 20%" (amber, 10 % and less red, urgent) / "Charging" / "Fully charged".
//! * `live:calendar {title, minutes}` -> "In 10 min: <title>". Opt-in: needs `live.calendar === true`.
//! * `live:mail {from, subject, more}` -> "New mail: <sender> - <subject>". Opt-in: needs `live.mail === true`.
//!
//! Switches: Settings > Live events (`live.dlDone`, `live.usb`, `live.battery`, `live.calendar`, `live.mail`).
//! PRIVACY: names and titles are only ever text. Cards only: none of these owns a capsule row.

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

fn close(env: &dyn Env) -> CardAction {
    CardAction::close(env.tr("Закрыть", "Close"))
}

fn and_more(env: &dyn Env, more: i32) -> String {
    env.tr(&format!("и ещё {more}"), &format!("and {more} more"))
}

// ================================================================ finished downloads

/// livesys.rs reports a browser download within ~1 s of the rename; look again after that.
pub const FILE_DONE_DELAY_MS: i64 = 1500;
pub const ANNOUNCED_KEEP_MS: i64 = 60_000;

#[derive(Debug, Clone)]
struct Pending {
    name: String,
    path: String,
    more: i32,
    due: i64,
}

#[derive(Debug, Default)]
pub struct FileDoneLive {
    pending: Vec<Pending>,
    /// lower-case name -> time a `live:download` "done" announced it
    announced: Vec<(String, i64)>,
}

impl FileDoneLive {
    fn card(&self, env: &dyn Env, p: &Pending) -> LiveCmd {
        let sub = if p.more > 0 { and_more(env, p.more) } else { env.tr("Нажмите, чтобы показать в папке", "Click to show in folder") };
        let mut card = CardSpec::text(
            "filedone",
            8000,
            format!("{}{}", env.tr("Загружено: ", "Downloaded: "), clip(&p.name, 40)),
            Some(sub),
        )
        .icon(IconName::File)
        .tone(Tone::Green)
        .actions(vec![
            // Rust accepts only files inside the Downloads folder
            CardAction::new(env.tr("Показать в папке", "Show in folder"), ActionKind::White, Action::invoke("livesys_reveal", json!({ "path": p.path }))),
            close(env),
        ]);
        card.click_first = true; // the whole card is a button
        LiveCmd::Toast(card)
    }
}

impl Provider for FileDoneLive {
    fn name(&self) -> &'static str {
        "file-done"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:file-done", "live:download"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        if p.is_null() {
            return Vec::new();
        }
        match name {
            "live:download" => {
                if p.get("state").and_then(Value::as_str) == Some("done") && truthy(p.get("name")) {
                    let k = text(p.get("name")).to_lowercase();
                    match self.announced.iter_mut().find(|(n, _)| *n == k) {
                        Some(e) => e.1 = now,
                        None => self.announced.push((k, now)),
                    }
                }
            }
            "live:file-done" => {
                if !truthy(p.get("name")) || !env.live_on("dlDone") {
                    return Vec::new();
                }
                self.pending.push(Pending { name: text(p.get("name")), path: text_or_empty(p.get("path")), more: int32(p.get("more")), due: now + FILE_DONE_DELAY_MS });
            }
            _ => {}
        }
        Vec::new()
    }
    fn on_tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        let (due, rest): (Vec<Pending>, Vec<Pending>) = self.pending.drain(..).partition(|p| p.due <= now);
        self.pending = rest;
        let mut out = Vec::new();
        for p in due {
            self.announced.retain(|(_, t)| now - *t <= ANNOUNCED_KEEP_MS);
            if self.announced.iter().any(|(n, _)| *n == p.name.to_lowercase()) || !env.live_on("dlDone") {
                continue;
            }
            out.push(self.card(env, &p));
        }
        out
    }
    fn tick_ms(&self, _now: i64) -> Option<u32> {
        if self.pending.is_empty() { None } else { Some(250) }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ USB drives

#[derive(Debug, Default)]
pub struct UsbLive;

impl Provider for UsbLive {
    fn name(&self) -> &'static str {
        "usb"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:usb"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if p.is_null() || !truthy(p.get("letter")) || !env.live_on("usb") {
            return Vec::new();
        }
        let l = format!("{}:", u16_slice(&text(p.get("letter")), 0, 1).to_uppercase());
        let added = truthy(p.get("added"));
        let title = if added { env.tr(&format!("Подключён диск {l}"), &format!("Drive {l} connected")) } else { env.tr(&format!("Диск {l} отключён"), &format!("Drive {l} disconnected")) };
        let card = CardSpec::text(format!("usb:{l}"), 4000, title, None)
            .icon(IconName::Usb)
            .tone(if added { Tone::Accent } else { Tone::Neutral })
            .actions(vec![close(env)]);
        vec![LiveCmd::Toast(card)]
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ battery

#[derive(Debug, Default)]
pub struct BatteryLive;

impl Provider for BatteryLive {
    fn name(&self) -> &'static str {
        "battery"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:battery"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if p.is_null() || !env.live_on("battery") {
            return Vec::new();
        }
        let pct = int32(p.get("percent")).clamp(0, 100);
        let kind = p.get("kind").and_then(Value::as_str).unwrap_or("");
        let (title, sub, tone, icon) = match kind {
            "low" => (
                env.tr(&format!("Заряд {pct} %"), &format!("Battery {pct}%")),
                env.tr("Подключите зарядное устройство", "Plug in the charger"),
                if pct <= 10 { Tone::Red } else { Tone::Amber },
                IconName::Battery,
            ),
            "charging" => (env.tr("Заряжается", "Charging"), format!("{pct} %"), Tone::Green, IconName::Bolt),
            "full" => (env.tr("Заряжено", "Fully charged"), env.tr("Можно отключить зарядку", "You can unplug the charger"), Tone::Green, IconName::Battery),
            _ => return Vec::new(),
        };
        let low = kind == "low";
        let mut card = CardSpec::text("battery", if low { 8000 } else { 5000 }, title, Some(sub)).icon(icon).tone(tone).actions(vec![close(env)]);
        card.urgent = low;
        vec![LiveCmd::Toast(card)]
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ calendar

#[derive(Debug, Default)]
pub struct CalendarLive;

impl Provider for CalendarLive {
    fn name(&self) -> &'static str {
        "calendar"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:calendar"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        // opt-in: without an explicit true the producer is off (liveOn treats a missing key as on)
        if p.is_null() || !truthy(p.get("title")) || !env.setting_true("live.calendar") {
            return Vec::new();
        }
        let m = int32(p.get("minutes")).max(1);
        let title = text(p.get("title"));
        let card = CardSpec::text(
            format!("cal:{title}"),
            12000,
            format!("{}{title}", env.tr(&format!("Через {m} мин: "), &format!("In {m} min: "))),
            None,
        )
        .icon(IconName::Calendar)
        .tone(Tone::Accent)
        .actions(vec![close(env)]);
        vec![LiveCmd::Toast(card)]
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ mail

#[derive(Debug, Default)]
pub struct MailLive;

impl Provider for MailLive {
    fn name(&self) -> &'static str {
        "mail"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:mail"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        // opt-in like the calendar: without an explicit true nothing is shown
        if p.is_null() || !env.setting_true("live.mail") {
            return Vec::new();
        }
        let more = int32(p.get("more"));
        let from = if truthy(p.get("from")) { text(p.get("from")) } else { env.tr("неизвестный отправитель", "unknown sender") };
        let subject = if truthy(p.get("subject")) { text(p.get("subject")) } else { env.tr("(без темы)", "(no subject)") };
        let card = CardSpec::text(
            "mail",
            10000,
            format!("{}{} — {}", env.tr("Новое письмо: ", "New mail: "), clip(&from, 40), clip(&subject, 60)),
            if more > 0 { Some(and_more(env, more)) } else { None },
        )
        .icon(IconName::Mail)
        .tone(Tone::Accent)
        .actions(vec![close(env)]);
        vec![LiveCmd::Toast(card)]
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

    fn toast(cmds: &[LiveCmd]) -> &CardSpec {
        cmds.iter().find_map(|c| if let LiveCmd::Toast(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no toast in {cmds:?}"))
    }
    fn title_sub(c: &CardSpec) -> (String, Option<String>) {
        match &c.body {
            CardBody::Text { title, sub } => (title.clone(), sub.clone()),
            o => panic!("{o:?}"),
        }
    }

    // ---------------- file done
    fn fd(name: &str, more: i64) -> Value {
        json!({"name": name, "path": format!("C:\\Users\\me\\Downloads\\{name}"), "more": more})
    }

    #[test]
    fn file_done_card_appears_after_the_delay() {
        let mut f = FileDoneLive::default();
        let env = MemEnv::default();
        assert!(f.on_event(&env, "live:file-done", &fd("a.zip", 0), T0).is_empty());
        assert_eq!(f.tick_ms(T0), Some(250));
        assert!(f.on_tick(&env, T0 + 1499).is_empty());
        let cmds = f.on_tick(&env, T0 + 1500);
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone, t.click_first), ("filedone", 8000, Tone::Green, true));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::File)));
        assert_eq!(title_sub(t), ("Загружено: a.zip".to_string(), Some("Нажмите, чтобы показать в папке".to_string())));
        assert_eq!(t.actions[0].action, Action::invoke("livesys_reveal", json!({"path": "C:\\Users\\me\\Downloads\\a.zip"})));
        assert_eq!(f.tick_ms(T0 + 1500), None);
        assert!(f.on_tick(&env, T0 + 9000).is_empty(), "fires once");
    }

    #[test]
    fn file_done_more_and_en() {
        let mut f = FileDoneLive::default();
        f.on_event(&MemEnv::en(), "live:file-done", &fd("b.iso", 3), T0);
        let cmds = f.on_tick(&MemEnv::en(), T0 + 2000);
        assert_eq!(title_sub(toast(&cmds)), ("Downloaded: b.iso".to_string(), Some("and 3 more".to_string())));
        let mut f = FileDoneLive::default();
        f.on_event(&MemEnv::default(), "live:file-done", &fd("b.iso", 3), T0);
        assert_eq!(title_sub(toast(&f.on_tick(&MemEnv::default(), T0 + 2000))).1, Some("и ещё 3".to_string()));
    }

    #[test]
    fn file_done_long_name_is_clipped_to_forty() {
        let mut f = FileDoneLive::default();
        let env = MemEnv::default();
        let long = format!("{}.zip", "x".repeat(60));
        f.on_event(&env, "live:file-done", &fd(&long, 0), T0);
        let cmds = f.on_tick(&env, T0 + 1500);
        let t = title_sub(toast(&cmds)).0;
        assert_eq!(u16_len(&t), u16_len("Загружено: ") + 40);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn file_done_is_suppressed_when_the_browser_download_was_announced() {
        let mut f = FileDoneLive::default();
        let env = MemEnv::default();
        f.on_event(&env, "live:file-done", &fd("Report.PDF", 0), T0);
        // livesys.rs announced the same file (case-insensitively) 700 ms later
        f.on_event(&env, "live:download", &json!({"state": "done", "id": "r", "name": "report.pdf", "bytes": 5, "path": "p"}), T0 + 700);
        assert!(f.on_tick(&env, T0 + 1500).is_empty());
        // an old announcement (> 60 s) no longer suppresses
        f.on_event(&env, "live:file-done", &fd("Report.PDF", 0), T0 + 100_000);
        assert!(!f.on_tick(&env, T0 + 101_600).is_empty());
    }

    #[test]
    fn file_done_gates() {
        let mut f = FileDoneLive::default();
        let mut env = MemEnv::default();
        assert!(f.on_event(&env, "live:file-done", &Value::Null, T0).is_empty());
        assert!(f.on_event(&env, "live:file-done", &json!({"path": "x"}), T0).is_empty());
        assert_eq!(f.tick_ms(T0), None);
        env.set("live.dlDone", json!(false));
        f.on_event(&env, "live:file-done", &fd("a", 0), T0);
        assert_eq!(f.tick_ms(T0), None, "switch off: nothing scheduled");
        // switched off between the event and the delayed card
        env.unset("live.dlDone");
        f.on_event(&env, "live:file-done", &fd("a", 0), T0);
        env.set("live.dlDone", json!(false));
        assert!(f.on_tick(&env, T0 + 1600).is_empty());
    }

    // ---------------- usb
    #[test]
    fn usb_added_and_removed() {
        let mut u = UsbLive;
        let env = MemEnv::default();
        let t = toast(&u.on_event(&env, "live:usb", &json!({"letter": "e", "added": true}), T0)).clone();
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("usb:E:", 4000, Tone::Accent));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Usb)));
        assert_eq!(title_sub(&t), ("Подключён диск E:".to_string(), None));
        let t = toast(&u.on_event(&env, "live:usb", &json!({"letter": "F", "added": false}), T0)).clone();
        assert_eq!((t.tone, title_sub(&t).0), (Tone::Neutral, "Диск F: отключён".to_string()));
        let en = MemEnv::en();
        assert_eq!(title_sub(toast(&u.on_event(&en, "live:usb", &json!({"letter": "g", "added": true}), T0))).0, "Drive G: connected");
        assert_eq!(title_sub(toast(&u.on_event(&en, "live:usb", &json!({"letter": "g", "added": false}), T0))).0, "Drive G: disconnected");
    }

    #[test]
    fn usb_takes_only_the_first_character_and_gates() {
        let mut u = UsbLive;
        let env = MemEnv::default();
        assert_eq!(toast(&u.on_event(&env, "live:usb", &json!({"letter": "xyz", "added": 1}), T0)).id, "usb:X:");
        assert_eq!(toast(&u.on_event(&env, "live:usb", &json!({"letter": "ж", "added": true}), T0)).id, "usb:Ж:");
        assert!(u.on_event(&env, "live:usb", &json!({"added": true}), T0).is_empty());
        assert!(u.on_event(&env, "live:usb", &json!({"letter": "", "added": true}), T0).is_empty());
        assert!(u.on_event(&env, "live:usb", &Value::Null, T0).is_empty());
        let mut off = MemEnv::default();
        off.set("live.usb", json!(false));
        assert!(u.on_event(&off, "live:usb", &json!({"letter": "e", "added": true}), T0).is_empty());
    }

    // ---------------- battery
    #[test]
    fn battery_low_tones_and_urgency() {
        let mut b = BatteryLive;
        let env = MemEnv::default();
        let t = toast(&b.on_event(&env, "live:battery", &json!({"kind": "low", "percent": 20}), T0)).clone();
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone, t.urgent), ("battery", 8000, Tone::Amber, true));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Battery)));
        assert_eq!(title_sub(&t), ("Заряд 20 %".to_string(), Some("Подключите зарядное устройство".to_string())));
        assert!(t.quiet_exempt(), "a low battery gets through quiet hours");
        let t = toast(&b.on_event(&env, "live:battery", &json!({"kind": "low", "percent": 10}), T0)).clone();
        assert_eq!(t.tone, Tone::Red);
        let t = toast(&b.on_event(&env, "live:battery", &json!({"kind": "low", "percent": 11}), T0)).clone();
        assert_eq!(t.tone, Tone::Amber);
        let t = toast(&b.on_event(&MemEnv::en(), "live:battery", &json!({"kind": "low", "percent": 5}), T0)).clone();
        assert_eq!(title_sub(&t), ("Battery 5%".to_string(), Some("Plug in the charger".to_string())));
        assert_eq!(t.tone, Tone::Red);
    }

    #[test]
    fn battery_charging_and_full() {
        let mut b = BatteryLive;
        let env = MemEnv::default();
        let t = toast(&b.on_event(&env, "live:battery", &json!({"kind": "charging", "percent": 37}), T0)).clone();
        assert_eq!((t.ttl_ms, t.tone, t.urgent, t.icon.clone()), (5000, Tone::Green, false, Some(CardIcon::Name(IconName::Bolt))));
        assert_eq!(title_sub(&t), ("Заряжается".to_string(), Some("37 %".to_string())));
        assert!(!t.quiet_exempt());
        let t = toast(&b.on_event(&MemEnv::en(), "live:battery", &json!({"kind": "full", "percent": 100}), T0)).clone();
        assert_eq!(title_sub(&t), ("Fully charged".to_string(), Some("You can unplug the charger".to_string())));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Battery)));
    }

    #[test]
    fn battery_edge_cases() {
        let mut b = BatteryLive;
        let env = MemEnv::default();
        assert!(b.on_event(&env, "live:battery", &json!({"kind": "weird", "percent": 5}), T0).is_empty());
        assert!(b.on_event(&env, "live:battery", &json!({"percent": 5}), T0).is_empty());
        assert!(b.on_event(&env, "live:battery", &Value::Null, T0).is_empty());
        // percent clamped, missing = 0 = red
        let t = toast(&b.on_event(&env, "live:battery", &json!({"kind": "low", "percent": 250}), T0)).clone();
        assert_eq!(title_sub(&t).0, "Заряд 100 %");
        let t = toast(&b.on_event(&env, "live:battery", &json!({"kind": "low"}), T0)).clone();
        assert_eq!((title_sub(&t).0, t.tone), ("Заряд 0 %".to_string(), Tone::Red));
        let mut off = MemEnv::default();
        off.set("live.battery", json!(false));
        assert!(b.on_event(&off, "live:battery", &json!({"kind": "low", "percent": 5}), T0).is_empty());
    }

    // ---------------- calendar
    #[test]
    fn calendar_is_opt_in() {
        let mut c = CalendarLive;
        let mut env = MemEnv::default();
        let ev = json!({"title": "Standup", "minutes": 10});
        assert!(c.on_event(&env, "live:calendar", &ev, T0).is_empty(), "missing key = off");
        env.set("live.calendar", json!(false));
        assert!(c.on_event(&env, "live:calendar", &ev, T0).is_empty());
        env.set("live.calendar", json!(true));
        let t = toast(&c.on_event(&env, "live:calendar", &ev, T0)).clone();
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("cal:Standup", 12000, Tone::Accent));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Calendar)));
        assert_eq!(title_sub(&t), ("Через 10 мин: Standup".to_string(), None));
        let mut en = MemEnv::en();
        en.set("live.calendar", json!(true));
        assert_eq!(title_sub(toast(&c.on_event(&en, "live:calendar", &ev, T0))).0, "In 10 min: Standup");
    }

    #[test]
    fn calendar_minutes_floor_and_title_is_not_clipped() {
        let mut c = CalendarLive;
        let mut env = MemEnv::default();
        env.set("live.calendar", json!(true));
        let long = "Т".repeat(80);
        let t = toast(&c.on_event(&env, "live:calendar", &json!({"title": long, "minutes": 0}), T0)).clone();
        assert_eq!(title_sub(&t).0, format!("Через 1 мин: {long}"));
        let t = toast(&c.on_event(&env, "live:calendar", &json!({"title": "x", "minutes": -5}), T0)).clone();
        assert_eq!(title_sub(&t).0, "Через 1 мин: x");
        let t = toast(&c.on_event(&env, "live:calendar", &json!({"title": "x"}), T0)).clone();
        assert_eq!(title_sub(&t).0, "Через 1 мин: x");
        assert!(c.on_event(&env, "live:calendar", &json!({"minutes": 5}), T0).is_empty());
        assert!(c.on_event(&env, "live:calendar", &json!({"title": "", "minutes": 5}), T0).is_empty());
        assert!(c.on_event(&env, "live:calendar", &Value::Null, T0).is_empty());
    }

    // ---------------- mail
    #[test]
    fn mail_is_opt_in_and_formats() {
        let mut m = MailLive;
        let mut env = MemEnv::default();
        let ev = json!({"from": "Anna", "subject": "Hello", "more": 0});
        assert!(m.on_event(&env, "live:mail", &ev, T0).is_empty());
        env.set("live.mail", json!(true));
        let t = toast(&m.on_event(&env, "live:mail", &ev, T0)).clone();
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("mail", 10000, Tone::Accent));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Mail)));
        assert_eq!(title_sub(&t), ("Новое письмо: Anna — Hello".to_string(), None));
        let ev2 = json!({"from": "Anna", "subject": "Hello", "more": 4});
        assert_eq!(title_sub(toast(&m.on_event(&env, "live:mail", &ev2, T0))).1, Some("и ещё 4".to_string()));
        let mut en = MemEnv::en();
        en.set("live.mail", json!(true));
        assert_eq!(title_sub(toast(&m.on_event(&en, "live:mail", &ev2, T0))), ("New mail: Anna — Hello".to_string(), Some("and 4 more".to_string())));
    }

    #[test]
    fn mail_fallbacks_and_clipping() {
        let mut m = MailLive;
        let mut env = MemEnv::default();
        env.set("live.mail", json!(true));
        let t = toast(&m.on_event(&env, "live:mail", &json!({}), T0)).clone();
        assert_eq!(title_sub(&t).0, "Новое письмо: неизвестный отправитель — (без темы)");
        let mut en = MemEnv::en();
        en.set("live.mail", json!(true));
        assert_eq!(title_sub(toast(&m.on_event(&en, "live:mail", &json!({"from": "", "subject": null}), T0))).0, "New mail: unknown sender — (no subject)");
        let from = "f".repeat(50);
        let subj = "s".repeat(70);
        let t = toast(&m.on_event(&env, "live:mail", &json!({"from": from, "subject": subj, "more": -2}), T0)).clone();
        assert_eq!(title_sub(&t), (format!("Новое письмо: {}… — {}…", "f".repeat(39), "s".repeat(59)), None));
        assert!(m.on_event(&env, "live:mail", &Value::Null, T0).is_empty());
    }
}
