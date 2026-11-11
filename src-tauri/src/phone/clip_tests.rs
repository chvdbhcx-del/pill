// End-to-end tests of the clipboard sync (protocol v1.2): the real server on 127.0.0.1 against a fake phone, the system clipboard replaced by
// `FakeClip` (tests never touch the real one).
use super::clip::{self, ClipCfg};
use super::tests::{Phone, Rig};
use serde_json::json;
use std::sync::atomic::Ordering::SeqCst;
use std::time::{Duration, Instant};

fn clip_on(r: &Rig, on: bool, to: bool, from: bool) {
    r.cfg.lock().unwrap().clip = ClipCfg { on, to_phone: to, from_phone: from };
    r.sh().clip_sync();
}

/// Everything the phone sent before has been handled by the server (a ping answered after it, on the same ordered socket).
fn settle(p: &mut Phone) {
    p.send(json!({ "t": "ping" }));
    p.recv_t("pong");
}

fn watching(r: &Rig) -> bool {
    r.clip.watching.load(SeqCst)
}

fn wait_watch(r: &Rig, want: bool) {
    let t = Instant::now();
    while watching(r) != want {
        assert!(t.elapsed() < Duration::from_secs(5), "listener state never became {want}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn connected_phone(r: &Rig) -> Phone {
    let token = r.pair("clip-dev");
    let mut p = Phone::connect(r.addr(), &token);
    p.recv_t("hello");
    p
}

#[test]
fn no_clip_before_caps_and_none_when_the_master_switch_is_off() {
    let r = Rig::new(true);
    clip_on(&r, true, true, true);
    let mut p = connected_phone(&r);
    // no caps yet: nothing is sent and no listener exists
    assert_eq!(r.sh().clip_local("before caps"), 0);
    assert!(!watching(&r), "the listener is not registered before a phone asked for the clipboard");
    p.send(json!({ "t": "caps", "clip": 1 }));
    wait_watch(&r, true);
    assert_eq!(r.sh().clip_local("after caps"), 1);
    let m = p.recv_t("clip");
    assert_eq!(m["text"], "after caps");
    assert!(m["id"].as_str().is_some_and(|s| !s.is_empty()) && m["ts"].as_i64().is_some_and(|t| t > 1_600_000_000_000), "{m}");
    // master off: no listener, nothing sent
    clip_on(&r, false, true, true);
    assert!(!watching(&r));
    assert_eq!(r.sh().clip_local("master off"), 0);
    // the master on but PC -> phone off: also no listener
    clip_on(&r, true, false, true);
    assert!(!watching(&r));
    assert_eq!(r.sh().clip_local("one way"), 0);
    // back on: works again, and nothing was queued in between
    clip_on(&r, true, true, true);
    wait_watch(&r, true);
    assert_eq!(r.sh().clip_local("again"), 1);
    assert_eq!(p.recv_t("clip")["text"], "again");
    r.stop();
}

#[test]
fn caps_zero_or_a_dropped_connection_ends_the_listener() {
    let r = Rig::new(true);
    clip_on(&r, true, true, true);
    let mut p = connected_phone(&r);
    p.send(json!({ "t": "caps", "clip": 1 }));
    wait_watch(&r, true);
    p.send(json!({ "t": "caps", "clip": 0 })); // the phone's own switch went off
    wait_watch(&r, false);
    assert_eq!(r.sh().clip_local("nobody wants it"), 0);
    p.send(json!({ "t": "caps", "clip": 1 }));
    wait_watch(&r, true);
    drop(p);
    wait_watch(&r, false); // zero cost with no phone connected
    r.stop();
}

#[test]
fn a_clip_from_the_phone_is_applied_once_and_never_echoed() {
    let r = Rig::new(true);
    clip_on(&r, true, true, true);
    let mut p = connected_phone(&r);
    p.send(json!({ "t": "caps", "clip": 1 }));
    wait_watch(&r, true);
    p.send(json!({ "t": "clip", "id": "c1", "text": "из телефона", "ts": 1 }));
    let ev = r.wait("phone:clip", |_| true);
    assert_eq!(ev["name"], "Pixel 8");
    assert!(!ev.to_string().contains("из телефона"), "the event never carries the text");
    assert_eq!(r.clip.text.lock().unwrap().as_deref(), Some("из телефона"));
    // the clipboard change this caused comes back from the watcher: it is not sent to the phone
    assert_eq!(r.sh().clip_local("из телефона"), 0, "never echoed");
    // the same text again is not applied twice
    p.send(json!({ "t": "clip", "id": "c2", "text": "из телефона", "ts": 2 }));
    settle(&mut p);
    assert_eq!(r.clip.writes.load(SeqCst), 1);
    // a different text from the PC goes out, and the phone echoing it is ignored
    assert_eq!(r.sh().clip_local("с компьютера"), 1);
    assert_eq!(p.recv_t("clip")["text"], "с компьютера");
    p.send(json!({ "t": "clip", "id": "c3", "text": "с компьютера", "ts": 3 }));
    settle(&mut p);
    assert_eq!(r.clip.writes.load(SeqCst), 1, "the echo of what the PC sent is dropped");
    r.stop();
}

#[test]
fn bad_or_forbidden_clips_from_the_phone_are_dropped_silently() {
    let r = Rig::new(true);
    clip_on(&r, true, true, true);
    let mut p = connected_phone(&r);
    p.send(json!({ "t": "caps", "clip": 1 }));
    wait_watch(&r, true);
    p.send(json!({ "t": "clip", "id": "e", "text": "", "ts": 1 })); // empty
    p.send(json!({ "t": "clip", "id": "b", "text": "x".repeat(clip::MAX_BYTES + 1), "ts": 1 })); // over 256 KiB
    p.send(json!({ "t": "clip", "id": "n", "ts": 1 })); // no text
    settle(&mut p);
    assert_eq!(r.clip.writes.load(SeqCst), 0);
    p.send(json!({ "t": "clip", "id": "ok", "text": "x".repeat(clip::MAX_BYTES), "ts": 1 })); // exactly 256 KiB
    settle(&mut p);
    assert_eq!(r.clip.writes.load(SeqCst), 1);
    // Phone -> PC off
    clip_on(&r, true, true, false);
    p.send(json!({ "t": "clip", "id": "off", "text": "not wanted", "ts": 2 }));
    settle(&mut p);
    // master off
    clip_on(&r, false, true, true);
    p.send(json!({ "t": "clip", "id": "off2", "text": "not wanted either", "ts": 3 }));
    settle(&mut p);
    assert_eq!(r.clip.writes.load(SeqCst), 1, "switched-off directions write nothing");
    assert_eq!(r.events.lock().unwrap().iter().filter(|(n, _)| n == "phone:clip").count(), 1);
    r.stop();
}

#[test]
fn a_clipboard_too_big_or_empty_on_the_pc_is_not_sent_and_only_phones_with_caps_get_it() {
    let r = Rig::new(true);
    clip_on(&r, true, true, true);
    let t1 = r.pair("clip-a");
    let t2 = r.pair("clip-b");
    let (mut a, mut b) = (Phone::connect(r.addr(), &t1), Phone::connect(r.addr(), &t2));
    a.recv_t("hello");
    b.recv_t("hello");
    a.send(json!({ "t": "caps", "clip": 1 }));
    settle(&mut a);
    assert_eq!(r.sh().clip_local(&"x".repeat(clip::MAX_BYTES + 1)), 0);
    assert_eq!(r.sh().clip_local(""), 0);
    assert_eq!(r.sh().clip_local("only a"), 1);
    assert_eq!(a.recv_t("clip")["text"], "only a");
    b.send(json!({ "t": "caps", "clip": 1 }));
    settle(&mut b);
    assert_eq!(r.sh().clip_local("both"), 2);
    assert_eq!(a.recv_t("clip")["text"], "both");
    assert_eq!(b.recv_t("clip")["text"], "both");
    r.stop();
}

#[test]
fn with_the_default_settings_the_v12_messages_are_ignored_like_any_unknown_type() {
    let r = Rig::new(true); // phone.clipboard is off by default
    let mut p = connected_phone(&r);
    p.send(json!({ "t": "caps", "clip": 1 }));
    p.send(json!({ "t": "clip", "id": "x", "text": "hello", "ts": 1 }));
    p.send(json!({ "t": "something.new", "x": 1 }));
    settle(&mut p); // no error, no answer, the socket lives on
    assert_eq!(r.clip.writes.load(SeqCst), 0);
    assert!(!watching(&r));
    assert_eq!(r.sh().clip_local("x"), 0);
    r.stop();
}
