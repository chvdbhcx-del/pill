//! Wiring tests of the Phone tab over `Services::fake()`: calls issued, events handled, the reply flow, the drop zone, idle = no frames.
use super::*;
use crate::ui::host::testing::{motion_on, render_view_cx};
use crate::ui::view::{Fakes, MsgQueue, Services};
use glasskit::geom::Pt;
use glasskit::input::{Buttons, Modifiers, MouseButton};
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) const WALL: i64 = 1_800_000_000_000;

pub(super) struct Rig {
    pub(super) v: PhoneView,
    pub(super) cx: Cx,
    q: Arc<MsgQueue>,
    pub(super) fakes: Fakes,
}

fn mv(p: Pt) -> InputEvent {
    InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE }
}
fn down(p: Pt) -> InputEvent {
    InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }
}
fn up(p: Pt) -> InputEvent {
    InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE }
}

pub(super) fn status(running: bool, devices: Value) -> Value {
    json!({ "enabled": running, "running": running, "ip": "192.168.1.5", "port": 41717, "pcName": "PC", "devices": devices, "connected": 0, "error": null })
}

pub(super) fn notif(key: &str, title: &str, can_reply: bool) -> Value {
    json!({ "key": key, "device": "d1", "pkg": "org.t", "app": "Telegram", "title": title, "text": "привет, как дела?", "when": WALL - 300_000, "ongoing": false, "replay": false, "canReply": can_reply, "icon": null, "avatar": null, "fresh": true })
}

/// A shown tab whose backend answers are canned; the mount replies are already delivered.
pub(super) fn rig(running: bool, notifs: Value) -> Rig {
    let (svc, fakes) = Services::fake();
    fakes.api.reply("phone_status", Ok(status(running, json!([{ "id": "d1", "name": "Pixel 8", "connected": true, "battery": 87, "charging": true, "network": "wifi", "ssid": "Home", "dnd": false }]))));
    fakes.api.reply("phone_notifs", Ok(notifs));
    fakes.api.reply("phone_files", Ok(json!([])));
    fakes.api.reply("phone_pair_code", Ok(json!({ "code": "AB3D9XKQ", "url": "http://192.168.1.5:41717/p?c=AB3D9XKQ", "expires": WALL + 300_000 })));
    let (mut cx, q) = Cx::headless_queue(svc);
    let mut v = PhoneView::new().with_wall(WALL);
    v.on_show(&mut cx);
    let mut r = Rig { v, cx, q, fakes };
    r.pump();
    r.frames(2);
    r
}

impl Rig {
    pub(super) fn pump(&mut self) {
        for _ in 0..4 {
            let msgs = self.q.drain();
            if msgs.is_empty() {
                break;
            }
            for (_, m) in msgs {
                self.v.message(m, &mut self.cx);
            }
        }
    }
    pub(super) fn frames(&mut self, n: usize) -> bool {
        let mut busy = false;
        for _ in 0..n {
            self.cx.now_ms += 4.0;
            busy = self.v.step(4.0, &mut self.cx);
            self.shoot();
        }
        busy
    }
    /// Frames until the tab is idle (at most 600).
    pub(super) fn settle(&mut self) {
        let mut n = 0;
        while self.frames(1) && n < 600 {
            n += 1;
        }
    }
    pub(super) fn shoot(&mut self) {
        render_view_cx(&mut self.v, &self.cx, SIZE, 1.0, &Theme::pill_dark());
    }
    pub(super) fn feed(&mut self, name: &str, payload: Value) {
        self.v.message(Msg::Event { name: name.into(), payload, latest: false }, &mut self.cx);
        self.pump();
    }
    pub(super) fn calls(&self) -> Vec<(String, Value)> {
        self.fakes.api.calls().into_iter().map(|c| (c.name, c.args)).collect()
    }
    pub(super) fn click(&mut self, k: K) {
        self.shoot();
        let r = self.v.clicks.rect_of(&k).unwrap_or_else(|| panic!("no click target {k:?}"));
        let p = r.center();
        self.v.event(&mv(p), &mut self.cx);
        self.v.event(&down(p), &mut self.cx);
        self.v.event(&up(p), &mut self.cx);
        self.pump();
    }
}

#[test]
fn showing_the_tab_reads_everything_once_and_subscribes() {
    let r = rig(true, json!([]));
    let names: Vec<String> = r.calls().into_iter().map(|c| c.0).collect();
    for want in ["phone_status", "phone_notifs", "phone_files", "phone_pair_code"] {
        assert_eq!(names.iter().filter(|n| *n == want).count(), 1, "{want} in {names:?}");
    }
    assert_eq!(r.v.model.code().map(|c| c.code.as_str()), Some("AB3D9XKQ"));
}

#[test]
fn link_off_shows_the_switch_and_turning_it_on_patches_the_setting() {
    let mut r = rig(false, json!([]));
    assert!(r.v.model.empty_text().is_some());
    r.shoot();
    let c = r.v.switch_rect.center();
    assert!(c.0 > 0.0);
    r.v.event(&mv(c), &mut r.cx);
    r.v.event(&down(c), &mut r.cx);
    r.v.event(&up(c), &mut r.cx);
    assert!(r.calls().contains(&("settings_set".into(), json!({ "patch": { "phone.enabled": true } }))));
}

#[test]
fn hide_sends_the_command_and_the_row_collapses_out_then_the_tab_goes_idle() {
    let _m = motion_on();
    let mut r = rig(true, json!([notif("a", "Вадим", true), notif("b", "Анна", false)]));
    assert_eq!(r.v.model.rows().len(), 2);
    r.click(K::Hide("a".into()));
    assert!(r.calls().contains(&("phone_notif_action".into(), json!({ "key": "a", "action": "dismiss" }))));
    assert!(r.frames(3), "the collapse is moving");
    assert!(r.v.rows["a"].open.at() < 1.0 && r.v.rows["a"].open.at() > 0.0, "half-way: {}", r.v.rows["a"].open.at());
    let mut n = 0;
    while r.frames(1) && n < 600 {
        n += 1;
    }
    assert_eq!(r.v.model.rows().iter().map(|x| x.key.as_str()).collect::<Vec<_>>(), ["b"], "reaped after the exit motion");
    assert!(!r.v.rows.contains_key("a"));
    assert!(!r.frames(2), "settled: no frames wanted");
}

#[test]
fn mark_read_has_its_own_command() {
    let mut r = rig(true, json!([notif("a", "Вадим", false)]));
    r.click(K::Read("a".into()));
    assert!(r.calls().contains(&("phone_notif_action".into(), json!({ "key": "a", "action": "read" }))));
}

#[test]
fn a_notification_that_arrives_grows_in_but_the_first_list_is_simply_there() {
    let _m = motion_on();
    let mut r = rig(true, json!([notif("a", "Вадим", true)]));
    assert_eq!(r.v.rows["a"].open.at(), 1.0, "came with the list: no animation");
    r.feed("phone:notif", json!({ "kind": "posted", "notif": notif("n", "Новое", true) }));
    r.frames(1);
    assert_eq!(r.v.model.rows()[0].key, "n", "newest first");
    let o = r.v.rows["n"].open.at();
    assert!(o > 0.0 && o < 1.0, "growing in: {o}");
    while r.frames(1) {}
    assert_eq!(r.v.rows["n"].open.at(), 1.0);
}

#[test]
fn reduced_motion_lands_rows_at_once() {
    let mut r = rig(true, json!([]));
    r.cx.reduced_motion = true;
    r.feed("phone:notif", json!({ "kind": "posted", "notif": notif("n", "Новое", true) }));
    r.frames(1);
    assert_eq!(r.v.rows["n"].open.at(), 1.0);
    r.click(K::Hide("n".into()));
    r.frames(2);
    assert!(r.v.model.rows().is_empty(), "gone at once");
}

#[test]
fn the_reply_field_sends_on_enter_shows_sent_and_the_row_leaves() {
    let _m = motion_on();
    let mut r = rig(true, json!([notif("a", "Вадим", true)]));
    r.click(K::Reply("a".into()));
    assert_eq!(r.v.reply_key.as_deref(), Some("a"));
    assert!(r.v.field.is_focused());
    r.frames(200); // the strip opens (the focused caret keeps frames coming, so no waiting for idle)
    r.shoot();
    for ch in "хорошо".chars() {
        r.v.event(&InputEvent::Char(ch), &mut r.cx);
    }
    r.shoot();
    r.v.event(&InputEvent::Key { code: KeyCode::Enter, mods: Modifiers::NONE, down: true, repeat: false }, &mut r.cx);
    r.shoot();
    r.frames(2);
    r.pump();
    let sent = r.calls().into_iter().find(|c| c.1.get("action") == Some(&json!("reply"))).expect("reply call");
    assert_eq!(sent.1, json!({ "key": "a", "action": "reply", "text": "хорошо" }));
    // the phone confirms: "Sent", then the row leaves
    r.feed("phone:notif", json!({ "kind": "reply", "key": "a", "ok": true }));
    r.frames(2);
    assert_eq!(r.v.model.rows()[0].reply, ReplyState::Sent);
    assert!(r.v.reply_key.is_none(), "the field closes: nothing left to type");
    r.cx.now_ms += 1500.0;
    r.frames(2);
    let mut n = 0;
    while r.frames(1) && n < 600 {
        n += 1;
    }
    assert!(r.v.model.rows().is_empty(), "the row left after Sent");
}

#[test]
fn a_row_without_canreply_has_no_reply_target() {
    let mut r = rig(true, json!([notif("a", "Вадим", false)]));
    r.shoot();
    assert!(r.v.clicks.rect_of(&K::Reply("a".into())).is_none());
    assert!(r.v.clicks.rect_of(&K::Hide("a".into())).is_some());
}

#[test]
fn files_tab_accepts_a_pending_offer_and_shows_transfers() {
    let mut r = rig(true, json!([]));
    r.click(K::Sub(1));
    assert_eq!(r.v.model.sub(), Sub::Files);
    r.feed("phone:files", json!({ "kind": "offer", "transfer": { "id": "f1", "name": "photo.jpg", "size": 2048, "dir": "in", "status": "pending", "bytes": 0 } }));
    r.settle();
    r.click(K::Accept("f1".into()));
    assert!(r.calls().contains(&("phone_file_decide".into(), json!({ "id": "f1", "accept": true }))));
    r.feed("phone:files", json!({ "kind": "done", "transfer": { "id": "f1", "name": "photo.jpg", "size": 2048, "dir": "in", "status": "done", "bytes": 2048, "path": "C:\\dl\\photo.jpg" } }));
    r.settle();
    r.click(K::Open("f1".into()));
    r.click(K::Folder("f1".into()));
    let calls = r.calls();
    assert!(calls.contains(&("phone_open_file".into(), json!({ "id": "f1", "reveal": false }))));
    assert!(calls.contains(&("phone_open_file".into(), json!({ "id": "f1", "reveal": true }))));
}

#[test]
fn the_drop_zone_lights_up_and_a_drop_sends_the_files_to_the_phone() {
    let _m = motion_on();
    let mut r = rig(true, json!([]));
    let files = Some(DropPayload::Files(vec!["C:\\a.txt".into(), "C:\\b.png".into()]));
    // on the Notifications sub-tab the tab does not take the drop
    let ev = |phase, payload| DropEvent { phase, pos: (100.0, 100.0), payload };
    assert!(!r.v.on_drop(&ev(DropPhase::Enter, files.clone()), &mut r.cx).consumed);
    r.click(K::Sub(1));
    assert!(r.v.on_drop(&ev(DropPhase::Enter, files.clone()), &mut r.cx).consumed);
    assert!(r.v.drag);
    r.frames(6);
    assert!(r.v.drop_k.at() > 0.0, "the zone lights up on a spring");
    assert!(r.v.on_drop(&ev(DropPhase::Over, None), &mut r.cx).consumed);
    r.v.on_drop(&ev(DropPhase::Leave, None), &mut r.cx);
    assert!(!r.v.drag);
    r.v.on_drop(&ev(DropPhase::Enter, files.clone()), &mut r.cx);
    r.fakes.api.reply("phone_send_file", Err("no phone is connected".into()));
    assert!(r.v.on_drop(&ev(DropPhase::Drop, files), &mut r.cx).consumed);
    let sends: Vec<Value> = r.calls().into_iter().filter(|c| c.0 == "phone_send_file").map(|c| c.1).collect();
    assert_eq!(sends, [json!({ "path": "C:\\a.txt" }), json!({ "path": "C:\\b.png" })]);
    r.pump();
    assert_eq!(r.v.drop_err.as_deref(), Some("no phone is connected"), "a failed offer is told in the zone");
    assert!(!r.v.drag);
}

#[test]
fn the_phone_tab_draws_the_qr_and_new_code_renews_it() {
    // nobody paired: the QR card is the content
    let mut r = rig(true, json!([]));
    r.feed("phone:state", status(true, json!([])));
    r.click(K::Sub(2));
    r.frames(2);
    r.shoot();
    let (url, rows) = r.v.qr_cache.clone().expect("the QR was built from the pairing URL");
    assert_eq!(url, "http://192.168.1.5:41717/p?c=AB3D9XKQ");
    assert!(rows.len() >= 21 && rows.iter().any(|x| x.contains('#')));
    r.click(K::NewCode);
    assert!(r.calls().contains(&("phone_pair_code".into(), json!({ "renew": true }))));
}

#[test]
fn an_idle_tab_asks_for_no_frames() {
    let _m = motion_on();
    let mut r = rig(true, json!([notif("a", "Вадим", true)]));
    let mut n = 0;
    while r.frames(1) && n < 600 {
        n += 1;
    }
    assert!(n < 600, "settles");
    assert!(!r.v.step(16.0, &mut r.cx));
}

#[test]
fn the_sub_tab_indicator_glides_to_the_chosen_chip() {
    let _m = motion_on();
    let mut r = rig(true, json!([]));
    let x0 = r.v.seg_x.at();
    r.click(K::Sub(2));
    r.frames(3);
    let mid = r.v.seg_x.at();
    assert!(mid > x0, "on its way: {x0} -> {mid}");
    let mut n = 0;
    while r.frames(1) && n < 600 {
        n += 1;
    }
    assert!(n < 600, "settles: seg {:?} bat {:?} drop {} conn {} qr {} sw", (r.v.seg_x.moving(), r.v.seg_w.moving()), r.v.bat.moving(), r.v.drop_k.moving(), r.v.conn_k.moving(), r.v.qr_k.moving());
    assert!((r.v.seg_x.at() - r.v.seg_cache[2].x).abs() < 0.5);
}

#[test]
fn click_targets_scrolled_out_of_sight_are_not_registered() {
    let mut clicks: Clicks<K> = Clicks::new();
    let body = Rect::new(0.0, 40.0, 600.0, 200.0);
    super::super::phone_draw::reg(&mut clicks, K::NewCode, Rect::new(0.0, 500.0, 50.0, 28.0), body, 0.0);
    assert!(clicks.rect_of(&K::NewCode).is_none());
    super::super::phone_draw::reg(&mut clicks, K::NewCode, Rect::new(0.0, 100.0, 50.0, 28.0), body, 90.0);
    let r = clicks.rect_of(&K::NewCode).unwrap();
    assert_eq!((r.y, r.h), (50.0, 28.0));
}

pub(super) fn rich(connected: bool, battery: i64, notifs: u64) -> Value {
    json!({ "id": "d1", "name": "Pixel 8", "model": "Pixel 8", "os": "Android 15", "appVersion": "1.1.0", "firstSeen": WALL - 20 * 86_400_000, "lastSeen": WALL - 300_000,
            "connected": connected, "battery": battery, "charging": true, "network": "wifi", "ssid": "Home-5G", "dnd": true, "ip": "192.168.1.23", "connectedAt": WALL - 134 * 60_000,
            "info": { "charge": "charging", "plug": "ac", "tempC": 31.5, "health": "good", "fullInMin": 42, "storage": { "free": 52_000_000_000u64, "total": 128_000_000_000u64 },
                      "ram": { "free": 3100, "total": 8000 }, "net": { "type": "wifi", "mbps": 866, "bars": 3 }, "airplane": false, "ringer": "vibrate", "screen": true, "tz": "Europe/Moscow", "utcMin": 180 },
            "stats": { "notifs": notifs, "replies": 2, "filesIn": 1, "filesOut": 3, "bytesIn": 2_400_000, "bytesOut": 15_000_000 } })
}

pub(super) fn state_with(d: Value) -> Value {
    status(true, json!([d]))
}

#[test]
fn the_phone_card_counts_numbers_and_glides_bars_then_idles() {
    let _m = motion_on();
    let mut r = rig(true, json!([]));
    r.click(K::Sub(2));
    r.feed("phone:state", state_with(rich(true, 87, 3)));
    r.settle();
    let d = r.v.model.link().phone().unwrap().clone();
    assert_eq!((d.model.as_str(), d.ip.as_str(), d.stats.files_out, d.info.ringer), ("Pixel 8", "192.168.1.23", 3, Some(pill_ui_core::panel::c::phone_info::Ringer::Vibrate)));
    assert_eq!(r.v.card.n_notifs.at().round(), 3.0);
    assert!(!r.frames(2), "settled: no frames wanted");
    // one more notification and a new battery level: the counter and the bar move, they do not jump
    r.feed("phone:state", state_with(rich(true, 40, 12)));
    r.frames(3);
    let (n, b) = (r.v.card.n_notifs.at(), r.v.bat.at());
    assert!(n > 3.0 && n < 12.0, "counting: {n}");
    assert!(b > 0.4 && b < 0.87, "gliding: {b}");
    assert!(r.frames(1), "still moving");
    r.settle();
    assert_eq!((r.v.card.n_notifs.at().round(), (r.v.bat.at() * 100.0).round()), (12.0, 40.0));
    assert!(!r.frames(2));
}

#[test]
fn the_first_values_of_an_opening_card_are_simply_there_and_reduced_motion_lands_at_once() {
    let mut r = rig(true, json!([]));
    r.cx.reduced_motion = true;
    r.click(K::Sub(2));
    r.feed("phone:state", state_with(rich(true, 87, 3)));
    r.frames(2);
    assert_eq!((r.v.card.n_notifs.at(), r.v.card.sig.at()), (3.0, 3.0));
    r.feed("phone:state", state_with(rich(true, 87, 9)));
    r.frames(2);
    assert_eq!(r.v.card.n_notifs.at(), 9.0, "reduced motion: at once");
    assert!(r.v.card.dnd.at01() > 0.99 && r.v.card.ringer.at01() > 0.99);
}

#[test]
fn offline_keeps_the_last_known_values_dimmed_and_an_old_phone_still_draws() {
    let _m = motion_on();
    let mut r = rig(true, json!([]));
    r.click(K::Sub(2));
    r.feed("phone:state", state_with(rich(true, 87, 3)));
    r.settle();
    r.feed("phone:state", state_with(rich(false, 87, 3)));
    r.settle();
    assert!(r.v.conn_k.at() < 0.01, "dimmed");
    assert_eq!((r.v.card.temp.target(), r.v.card.st_used.target() > 0.5, r.v.card.n_notifs.target()), (31.5, true, 3.0), "the last values stay");
    assert!(!r.v.model.link().phone().unwrap().connected);
    // an old phone: only the original fields
    r.feed("phone:state", state_with(json!({ "id": "d1", "name": "Old", "connected": true, "battery": 20, "charging": false, "network": "mobile", "dnd": false })));
    r.settle();
    r.shoot();
    assert!(r.v.card.detail_k.at01() < 0.01 && r.v.card.store_k.at01() < 0.01, "nothing to show for the missing fields");
}

/// Review pictures of the four states (only written when `PILL_SNAP_DIR` is set; the name keeps it out of the default run).
#[test]
fn snapshot_states() {
    let Some(dir) = std::env::var_os("PILL_SNAP_DIR").map(std::path::PathBuf::from) else { return };
    let _ = std::fs::create_dir_all(&dir);
    let _m = motion_on();
    let mut long = notif("b", "Анна Иванова", false);
    long["text"] = json!("Встречаемся в шесть у входа, не забудь зонт и документы, будет дождь");
    long["app"] = json!("WhatsApp");
    let mut r = rig(true, json!([notif("a", "Вадим", true), long]));
    r.settle();
    r.click(K::Reply("a".into()));
    r.frames(120);
    let save = |r: &mut Rig, name: &str| {
        let img = render_view_cx(&mut r.v, &r.cx, SIZE, 2.0, &Theme::pill_dark());
        crate::ui::host::testing::save_png(&img, dir.join(format!("phone-{name}.png")));
    };
    save(&mut r, "notifs");
    r.click(K::Sub(1));
    r.feed("phone:files", json!({ "kind": "offer", "transfer": { "id": "f1", "name": "photo.jpg", "size": 2_400_000, "dir": "in", "status": "active", "bytes": 1_000_000 } }));
    r.feed("phone:files", json!({ "kind": "offer", "transfer": { "id": "f2", "name": "Report.docx", "size": 20480, "dir": "out", "status": "done", "bytes": 20480, "path": "C:\\x.docx" } }));
    r.feed("phone:files", json!({ "kind": "offer", "transfer": { "id": "f3", "name": "big-video.mp4", "size": 900_000_000, "dir": "in", "status": "pending", "bytes": 0 } }));
    r.frames(400);
    save(&mut r, "files");
    r.click(K::Sub(2));
    r.frames(400);
    save(&mut r, "phone");
    r.feed("phone:state", state_with(rich(true, 87, 12)));
    r.frames(400);
    save(&mut r, "phone-rich");
    r.feed("phone:state", state_with(rich(false, 87, 12)));
    r.frames(400);
    save(&mut r, "phone-offline");
    let mut off = rig(false, json!([]));
    off.frames(100);
    let img = render_view_cx(&mut off.v, &off.cx, SIZE, 2.0, &Theme::pill_dark());
    crate::ui::host::testing::save_png(&img, dir.join("phone-off.png"));
}
