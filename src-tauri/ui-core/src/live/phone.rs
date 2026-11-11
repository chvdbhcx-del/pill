//! Live cards of the Pill Phone link (docs/phone-protocol.md): a mirrored phone notification as a card on the folded island (the Telegram card of the
//! product video: avatar, "<sender> · <app>", the text, "now"; hovering it grows the island to Hide / Mark read and a Reply field), the reply result,
//! and the files that came from / went to the phone.
//!
//! * `phone:notif {kind: posted, notif}` -> card `phone:<key>` (ttl 7 s) unless the notification is ongoing, a replay after a reconnect, a repeat of the same
//!   words (`fresh` false), from a muted app (`phone.mutedApps`), or `live.phone` is off. Quiet hours drop it like any non-urgent card.
//! * `phone:notif {kind: reply, key, ok}` -> the SAME card id, so it replaces the card on screen: "Reply sent" with a check, then the clock returns.
//! * `phone:files {kind: done, dir: in}` -> "Received from phone: name" (button: Show in folder); `{kind: sent}` -> "Sent to phone";
//!   `{kind: offer, status: pending}` -> "The phone wants to send: name" with Accept / Decline.
//! * `phone:clip {device, name}` (a text from a phone was put on the PC clipboard) -> "Clipboard from <name>" for 2 s. The event never carries the text.
//! * A badge with the number of listed notifications sits on the Phone tab.
//!
//! PRIVACY: message text only goes into the card data; nothing here logs or stores it.

use std::any::Any;
use std::collections::HashSet;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::{Action, ActionKind, CardAction, CardBody, CardIcon, CardSpec, IconName, LiveCmd, PhoneCard, Tone};
use super::util::{cut, truthy};

const TTL_MS: u32 = 7000;
const REPLY_TTL_MS: u32 = 2200;
const FILE_TTL_MS: u32 = 6000;
const CLIP_TTL_MS: u32 = 2000;

pub fn card_id(key: &str) -> String {
    format!("phone:{key}")
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().to_string()
}

#[derive(Debug, Default)]
pub struct PhoneLive {
    /// keys of the notifications on the phone tab's list (ongoing and replayed ones are not counted)
    listed: HashSet<String>,
}

impl PhoneLive {
    fn badge(&self) -> LiveCmd {
        LiveCmd::TabBadge { tab: "phone".into(), count: self.listed.len().min(99) as u32 }
    }

    fn muted(env: &dyn Env, pkg: &str) -> bool {
        !pkg.is_empty() && env.setting("phone.mutedApps").and_then(|v| v.as_array().map(|a| a.iter().any(|x| x.as_str() == Some(pkg)))).unwrap_or(false)
    }

    fn posted(&mut self, env: &dyn Env, n: &Value) -> Vec<LiveCmd> {
        let key = s(&n["key"]);
        if key.is_empty() {
            return Vec::new();
        }
        let ongoing = truthy(n.get("ongoing"));
        let replay = truthy(n.get("replay"));
        if !ongoing {
            self.listed.insert(key.clone());
        }
        let mut out = vec![self.badge()];
        if !env.live_on("phone") || ongoing || replay || !truthy(n.get("fresh")) || Self::muted(env, &s(&n["pkg"])) {
            return out;
        }
        let (title, text, app) = (cut(&s(&n["title"]), 80), cut(&s(&n["text"]), 200), cut(&s(&n["app"]), 40));
        if title.is_empty() && text.is_empty() {
            return out;
        }
        let avatar = n["avatar"].as_str().or(n["icon"].as_str()).filter(|a| !a.is_empty()).map(str::to_string);
        let card = PhoneCard {
            key: key.clone(),
            sender: if title.is_empty() { app.clone() } else { title },
            app,
            text,
            when: env.tr("сейчас", "now"),
            avatar,
            can_reply: truthy(n.get("canReply")),
            hide: env.tr("Скрыть", "Hide"),
            mark_read: env.tr("Прочитано", "Mark read"),
            reply_hint: env.tr("Ответить…", "Reply…"),
        };
        let mut spec = CardSpec::new(card_id(&key), TTL_MS, CardBody::Phone(card));
        spec.actions = vec![
            CardAction::new(env.tr("Скрыть", "Hide"), ActionKind::Normal, Action::invoke("phone_notif_action", json!({ "key": key, "action": "dismiss" }))),
            CardAction::new(env.tr("Прочитано", "Mark read"), ActionKind::Normal, Action::invoke("phone_notif_action", json!({ "key": key, "action": "read" }))),
        ];
        out.push(LiveCmd::Toast(spec));
        out
    }

    fn reply_result(&self, env: &dyn Env, p: &Value) -> Vec<LiveCmd> {
        if !env.live_on("phone") {
            return Vec::new();
        }
        let key = s(&p["key"]);
        if key.is_empty() {
            return Vec::new();
        }
        let ok = truthy(p.get("ok"));
        let mut c = if ok {
            CardSpec::text(card_id(&key), REPLY_TTL_MS, env.tr("Ответ отправлен", "Reply sent"), None).tone(Tone::Green)
        } else {
            let why = match s(&p["error"]).as_str() {
                "expired" => env.tr("Уведомление уже недоступно на телефоне", "The notification is gone on the phone"),
                "no_action" => env.tr("В этом уведомлении нельзя ответить", "This notification cannot be answered"),
                _ => env.tr("Телефон не принял ответ", "The phone did not take the reply"),
            };
            CardSpec::text(card_id(&key), TTL_MS, env.tr("Не удалось ответить", "Reply failed"), Some(why)).icon(IconName::Warn).tone(Tone::Amber)
        };
        if ok {
            c.icon = Some(CardIcon::Check);
        }
        vec![LiveCmd::Toast(c)]
    }

    fn clip(&self, env: &dyn Env, p: &Value) -> Vec<LiveCmd> {
        if !env.live_on("phone") {
            return Vec::new();
        }
        let name = cut(&s(&p["name"]), 40);
        let name = if name.is_empty() { env.tr("телефона", "phone") } else { name };
        // one id: a second text while the card is still up replaces it instead of stacking
        let title = format!("{} {}", env.tr("Буфер от", "Clipboard from"), name);
        vec![LiveCmd::Toast(CardSpec::text("phone-clip", CLIP_TTL_MS, title, None).icon(IconName::Clipboard))]
    }

    fn file(&self, env: &dyn Env, p: &Value) -> Vec<LiveCmd> {
        if !env.live_on("phone") {
            return Vec::new();
        }
        let kind = s(&p["kind"]);
        let t = &p["transfer"];
        let (id, name, dir, status) = (s(&t["id"]), cut(&s(&t["name"]), 60), s(&t["dir"]), s(&t["status"]));
        let name = if name.is_empty() { env.tr("файл", "file") } else { name };
        let mut c = match (kind.as_str(), dir.as_str()) {
            ("done", "in") => {
                let mut c = CardSpec::text(format!("phone-file:{id}"), FILE_TTL_MS, env.tr("Получено с телефона", "Received from phone"), Some(name)).icon(IconName::Download).tone(Tone::Green);
                c.actions = vec![CardAction::new(env.tr("Показать", "Show"), ActionKind::Normal, Action::invoke("phone_open_file", json!({ "id": id, "reveal": true })))];
                c
            }
            ("sent", "out") => CardSpec::text(format!("phone-file:{id}"), 4500, env.tr("Отправлено на телефон", "Sent to phone"), Some(name)).icon(IconName::Check).tone(Tone::Green),
            ("offer", "in") if status == "pending" => {
                let mut c = CardSpec::text(format!("phone-file:{id}"), 15_000, env.tr("Телефон хочет отправить файл", "The phone wants to send a file"), Some(name)).icon(IconName::File).tone(Tone::Accent);
                c.actions = vec![
                    CardAction::new(env.tr("Отклонить", "Decline"), ActionKind::Normal, Action::invoke("phone_file_decide", json!({ "id": id, "accept": false }))),
                    CardAction::new(env.tr("Принять", "Accept"), ActionKind::White, Action::invoke("phone_file_decide", json!({ "id": id, "accept": true }))),
                ];
                c
            }
            _ => return Vec::new(),
        };
        vec![LiveCmd::Toast(c)]
    }
}

impl Provider for PhoneLive {
    fn name(&self) -> &'static str {
        "phone"
    }
    fn events(&self) -> &'static [&'static str] {
        &["phone:notif", "phone:files", "phone:state", "phone:clip"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        match name {
            "phone:notif" => match s(&p["kind"]).as_str() {
                "posted" => self.posted(env, &p["notif"]),
                "reply" => {
                    let mut out = self.reply_result(env, p);
                    if truthy(p.get("ok")) && self.listed.remove(&s(&p["key"])) {
                        out.push(self.badge());
                    }
                    out
                }
                "removed" | "read" => {
                    if self.listed.remove(&s(&p["key"])) {
                        vec![self.badge()]
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            },
            "phone:files" => self.file(env, p),
            "phone:clip" => self.clip(env, p),
            // the link went off or the phone left: nothing of it is listed any more
            "phone:state" if p["connected"].as_u64() == Some(0) || p["running"] == json!(false) => {
                if self.listed.is_empty() {
                    Vec::new()
                } else {
                    self.listed.clear();
                    vec![self.badge()]
                }
            }
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    const T0: i64 = 1_793_363_640_000;

    fn notif(extra: Value) -> Value {
        let mut n = json!({ "key": "0|tg|1", "pkg": "org.telegram.messenger", "app": "Telegram", "title": "Вадим", "text": "привет, как дела?", "ongoing": false, "replay": false, "fresh": true, "canReply": true, "avatar": "iVBORw0KGgo=" });
        n.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json!({ "kind": "posted", "notif": n })
    }

    fn toast(cmds: &[LiveCmd]) -> Option<&CardSpec> {
        cmds.iter().find_map(|c| if let LiveCmd::Toast(c) = c { Some(c) } else { None })
    }

    #[test]
    fn a_message_becomes_the_telegram_card() {
        let mut p = PhoneLive::default();
        let cmds = p.on_event(&MemEnv::en(), "phone:notif", &notif(json!({})), T0);
        let c = toast(&cmds).expect("a card");
        assert_eq!((c.id.as_str(), c.ttl_ms, c.tone), ("phone:0|tg|1", 7000, Tone::Neutral));
        let CardBody::Phone(ph) = &c.body else { panic!("{:?}", c.body) };
        assert_eq!((ph.sender.as_str(), ph.app.as_str(), ph.text.as_str(), ph.when.as_str(), ph.can_reply), ("Вадим", "Telegram", "привет, как дела?", "now", true));
        assert_eq!(ph.avatar.as_deref(), Some("iVBORw0KGgo="));
        let labels: Vec<_> = c.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, ["Hide", "Mark read"]);
        assert_eq!(c.actions[0].action, Action::invoke("phone_notif_action", json!({ "key": "0|tg|1", "action": "dismiss" })));
        assert_eq!(c.actions[1].action, Action::invoke("phone_notif_action", json!({ "key": "0|tg|1", "action": "read" })));
        assert!(cmds.contains(&LiveCmd::TabBadge { tab: "phone".into(), count: 1 }));
        // Russian
        let ru = p.on_event(&MemEnv::default(), "phone:notif", &notif(json!({ "key": "k2" })), T0);
        let CardBody::Phone(ph) = &toast(&ru).unwrap().body else { panic!() };
        assert_eq!((ph.when.as_str(), ph.hide.as_str()), ("сейчас", "Скрыть"));
    }

    #[test]
    fn a_text_from_the_phone_shows_a_two_second_card_without_the_text() {
        let mut p = PhoneLive::default();
        let cmds = p.on_event(&MemEnv::en(), "phone:clip", &json!({ "device": "d1", "name": "Pixel 8", "text": "must never be read" }), T0);
        let c = toast(&cmds).expect("a card");
        assert_eq!((c.id.as_str(), c.ttl_ms), ("phone-clip", 2000));
        let CardBody::Text { title, sub } = &c.body else { panic!("{:?}", c.body) };
        assert_eq!((title.as_str(), sub.as_deref()), ("Clipboard from Pixel 8", None));
        assert_eq!(c.icon, Some(CardIcon::Name(IconName::Clipboard)));
        let ru = p.on_event(&MemEnv::default(), "phone:clip", &json!({ "name": "Pixel 8" }), T0);
        let CardBody::Text { title, .. } = &toast(&ru).unwrap().body else { panic!() };
        assert_eq!(title, "Буфер от Pixel 8");
        let mut off = MemEnv::en();
        off.set("live.phone", json!(false));
        assert!(toast(&p.on_event(&off, "phone:clip", &json!({ "name": "x" }), T0)).is_none());
    }

    #[test]
    fn what_must_not_pop_up() {
        let mut p = PhoneLive::default();
        let env = MemEnv::en();
        for extra in [json!({ "ongoing": true }), json!({ "replay": true }), json!({ "fresh": false }), json!({ "title": "", "text": "" }), json!({ "key": "" })] {
            assert!(toast(&p.on_event(&env, "phone:notif", &notif(extra.clone()), T0)).is_none(), "{extra}");
        }
        let mut off = MemEnv::en();
        off.set("live.phone", json!(false));
        assert!(toast(&p.on_event(&off, "phone:notif", &notif(json!({})), T0)).is_none());
        let mut muted = MemEnv::en();
        muted.set("phone.mutedApps", json!(["org.telegram.messenger"]));
        assert!(toast(&p.on_event(&muted, "phone:notif", &notif(json!({})), T0)).is_none());
        muted.set("phone.mutedApps", json!(["com.other"]));
        assert!(toast(&p.on_event(&muted, "phone:notif", &notif(json!({})), T0)).is_some());
    }

    #[test]
    fn quiet_hours_drop_the_card_but_not_the_badge() {
        let mut hub = crate::live::Hub::new(vec![Box::new(PhoneLive::default())]);
        let mut env = MemEnv::en();
        env.quiet = true;
        let out = hub.dispatch(&env, "phone:notif", &notif(json!({})), T0);
        assert!(toast(&out).is_none(), "{out:?}");
        assert!(out.iter().any(|c| matches!(c, LiveCmd::TabBadge { .. })));
        env.quiet = false;
        assert!(toast(&hub.dispatch(&env, "phone:notif", &notif(json!({ "key": "k9" })), T0)).is_some());
    }

    #[test]
    fn long_texts_are_cut_and_a_missing_title_falls_back_to_the_app() {
        let mut p = PhoneLive::default();
        let cmds = p.on_event(&MemEnv::en(), "phone:notif", &notif(json!({ "title": "", "text": "x".repeat(900) })), T0);
        let CardBody::Phone(ph) = &toast(&cmds).unwrap().body else { panic!() };
        assert_eq!(ph.sender, "Telegram");
        assert!(ph.text.chars().count() <= 200);
    }

    #[test]
    fn the_reply_result_replaces_the_card_with_a_check_or_a_warning() {
        let mut p = PhoneLive::default();
        let env = MemEnv::en();
        p.on_event(&env, "phone:notif", &notif(json!({})), T0);
        let ok = p.on_event(&env, "phone:notif", &json!({ "kind": "reply", "key": "0|tg|1", "ok": true }), T0);
        let c = toast(&ok).unwrap();
        assert_eq!(c.id, "phone:0|tg|1", "same id: it replaces the message card");
        assert!(matches!(&c.body, CardBody::Text { title, .. } if title == "Reply sent"));
        assert_eq!((c.icon.clone(), c.tone, c.ttl_ms), (Some(CardIcon::Check), Tone::Green, 2200));
        assert!(ok.contains(&LiveCmd::TabBadge { tab: "phone".into(), count: 0 }));
        let bad = p.on_event(&env, "phone:notif", &json!({ "kind": "reply", "key": "0|tg|1", "ok": false, "error": "expired" }), T0);
        let c = toast(&bad).unwrap();
        assert!(matches!(&c.body, CardBody::Text { title, sub: Some(s) } if title == "Reply failed" && s.contains("gone")));
        assert_eq!(c.tone, Tone::Amber);
    }

    #[test]
    fn files_in_and_out() {
        let mut p = PhoneLive::default();
        let env = MemEnv::en();
        let done = p.on_event(&env, "phone:files", &json!({ "kind": "done", "transfer": { "id": "f1", "name": "photo.jpg", "dir": "in", "status": "done" } }), T0);
        let c = toast(&done).unwrap();
        assert!(matches!(&c.body, CardBody::Text { title, sub: Some(n) } if title == "Received from phone" && n == "photo.jpg"));
        assert_eq!(c.actions[0].action, Action::invoke("phone_open_file", json!({ "id": "f1", "reveal": true })));
        let sent = p.on_event(&env, "phone:files", &json!({ "kind": "sent", "transfer": { "id": "f2", "name": "r.docx", "dir": "out", "status": "done" } }), T0);
        assert!(matches!(&toast(&sent).unwrap().body, CardBody::Text { title, .. } if title == "Sent to phone"));
        let offer = p.on_event(&env, "phone:files", &json!({ "kind": "offer", "transfer": { "id": "f3", "name": "big.zip", "dir": "in", "status": "pending" } }), T0);
        let c = toast(&offer).unwrap();
        assert_eq!(c.actions.len(), 2);
        assert_eq!(c.actions[1].action, Action::invoke("phone_file_decide", json!({ "id": "f3", "accept": true })));
        // an accepted offer, progress and failures are silent
        for (k, st) in [("offer", "accepted"), ("progress", "active"), ("failed", "failed"), ("start", "active")] {
            assert!(p.on_event(&env, "phone:files", &json!({ "kind": k, "transfer": { "id": "x", "name": "n", "dir": "in", "status": st } }), T0).is_empty(), "{k}");
        }
    }

    #[test]
    fn the_badge_follows_the_list_and_clears_when_the_link_goes() {
        let mut p = PhoneLive::default();
        let env = MemEnv::en();
        p.on_event(&env, "phone:notif", &notif(json!({ "key": "a" })), T0);
        let c = p.on_event(&env, "phone:notif", &notif(json!({ "key": "b" })), T0);
        assert!(c.contains(&LiveCmd::TabBadge { tab: "phone".into(), count: 2 }));
        p.on_event(&env, "phone:notif", &notif(json!({ "key": "m", "ongoing": true })), T0);
        let r = p.on_event(&env, "phone:notif", &json!({ "kind": "removed", "key": "a" }), T0);
        assert_eq!(r, vec![LiveCmd::TabBadge { tab: "phone".into(), count: 1 }]);
        assert!(p.on_event(&env, "phone:notif", &json!({ "kind": "removed", "key": "zzz" }), T0).is_empty());
        let r = p.on_event(&env, "phone:state", &json!({ "running": true, "connected": 0 }), T0);
        assert_eq!(r, vec![LiveCmd::TabBadge { tab: "phone".into(), count: 0 }]);
        assert!(p.on_event(&env, "phone:state", &json!({ "running": true, "connected": 1 }), T0).is_empty());
    }
}
