// Tests of the PC link (docs/pc-link.md): several real servers ("Pills", each in its own data folder with a fake clipboard) joined through the fake relay of
// relay_tests.rs. Nothing leaves the machine; every test stops its Pills and the relay.
use super::pc::{new_token, parse_token, show};
use super::relay_tests::{wait_for, FakeRelay};
use super::tests::Rig;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn pill(relay: &FakeRelay, name: &str) -> Rig {
    let r = Rig::new(true);
    {
        let mut c = r.cfg.lock().unwrap();
        c.relay_url = relay.url();
        c.pc_name = name.into();
    }
    r
}

fn status(r: &Rig) -> Value {
    r.sh().pc_status()
}

fn state(r: &Rig) -> String {
    status(r)["pair"]["state"].as_str().unwrap_or("none").to_string()
}

fn peers(r: &Rig) -> Vec<Value> {
    status(r)["peers"].as_array().cloned().unwrap_or_default()
}

fn id_of(r: &Rig) -> String {
    r.sh().registry().me()
}

/// `a` shows a token, `b` enters it, `a` allows; returns when both have stored the other and the two are connected.
fn pair(a: &Rig, b: &Rig) {
    a.sh().pc_pair("start", None).unwrap();
    let token = status(a)["pair"]["token"].as_str().expect("the token is shown").to_string();
    b.sh().pc_pair("join", Some(&token)).unwrap();
    wait_for("the request reaches the shown side", || state(a) == "request");
    assert_eq!(status(a)["pair"]["peer"]["name"], b.cfg.lock().unwrap().pc_name, "the card names the PC that asks");
    a.sh().pc_pair("allow", None).unwrap();
    wait_for("both sides paired and online", || peers(a).iter().any(|p| p["online"] == true) && peers(b).iter().any(|p| p["online"] == true) && state(a) == "done" && state(b) == "done");
}

/// Waits until the other Pill's clipboard shows `text`.
fn applied(r: &Rig, text: &str) {
    wait_for("the text arrives", || r.clip.text.lock().unwrap().as_deref() == Some(text));
}

fn quiet() {
    std::thread::sleep(Duration::from_millis(400));
}

fn stop(rigs: Vec<Rig>, relay: FakeRelay) {
    rigs.into_iter().for_each(Rig::stop);
    relay.stop();
}

#[test]
fn the_token_is_long_typo_proof_and_forgiving() {
    let t = new_token();
    assert_eq!(t.len(), 28);
    assert_ne!(t, new_token());
    let shown = show(&t);
    assert!(shown.starts_with("PILL-") && shown.matches('-').count() == 7, "{shown}");
    for typed in [shown.clone(), shown.to_lowercase(), t.clone(), format!("  {}\n", shown.replace('-', " "))] {
        assert_eq!(parse_token(&typed).as_deref(), Some(t.as_str()), "{typed:?}");
    }
    // one wrong character, a short one and rubbish are refused before the relay is asked for a room
    let other = if t.starts_with('0') { '1' } else { '0' };
    assert!(parse_token(&format!("{other}{}", &t[1..])).is_none());
    assert!(parse_token(&t[..27]).is_none() && parse_token("").is_none() && parse_token("PILL-hello").is_none());
}

#[test]
fn two_pills_pair_and_share_the_clipboard_both_ways() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    pair(&a, &b);
    // both sides hold the same keys; the one that entered the token is the relay's `phone` end
    let (da, db) = (a.sh().registry().get(&id_of(&b)).unwrap(), b.sh().registry().get(&id_of(&a)).unwrap());
    assert!(da.keys.is_some() && da.keys == db.keys && da.kind == "pc" && !da.joiner && db.joiner);
    assert_eq!((peers(&a)[0]["name"].clone(), peers(&b)[0]["name"].clone()), (json!("PC-B"), json!("PC-A")));
    // the token and the pairing room are not on disk
    let disk = std::fs::read_to_string(a.dir.join("data").join("devices.json")).unwrap();
    assert!(!disk.contains("pairing") && !disk.contains("PILL"), "{disk}");
    // on by default: a copy on A lands on B at once (and B does not send it back), then the other way
    wait_for("the listener", || a.clip.watching.load(Ordering::SeqCst) && b.clip.watching.load(Ordering::SeqCst));
    assert_eq!(a.sh().clip_local("первый текст ЛИМОН"), 1);
    applied(&b, "первый текст ЛИМОН");
    b.wait("pc:clip", |v| v["name"] == "PC-A");
    assert_eq!(b.sh().clip_local("первый текст ЛИМОН"), 0, "what was applied is never echoed");
    assert_eq!(b.sh().clip_local("ответ"), 1);
    applied(&a, "ответ");
    // the relay saw ciphertext only
    for f in relay.st.tap.lock().unwrap().iter() {
        let hay = String::from_utf8_lossy(f);
        assert!(!hay.contains("ЛИМОН") && !hay.contains("clip") && !hay.contains("PC-A"), "plaintext leaked to the relay");
    }
    // each direction can be switched off per peer
    let idb = id_of(&b);
    a.sh().pc_peer(&idb, "clipTo", false).unwrap();
    assert_eq!(a.sh().clip_local("nicht"), 0);
    let ida = id_of(&a);
    a.sh().pc_peer(&idb, "clipTo", true).unwrap();
    b.sh().pc_peer(&ida, "clipFrom", false).unwrap();
    assert_eq!(a.sh().clip_local("taub"), 1, "it is sent, the other side does not apply it");
    quiet();
    assert_eq!(b.clip.text.lock().unwrap().as_deref(), Some("первый текст ЛИМОН"));
    stop(vec![a, b], relay);
}

#[test]
fn nothing_is_sent_before_a_pairing_exists() {
    let relay = FakeRelay::start();
    let a = pill(&relay, "PC-A");
    a.sh().clip_sync();
    assert!(!a.clip.watching.load(Ordering::SeqCst), "no clipboard listener without a paired PC");
    assert_eq!(a.sh().clip_local("geheim"), 0);
    // waiting with a token shown connects the pairing room, but no clipboard text can go anywhere
    a.sh().pc_pair("start", None).unwrap();
    assert_eq!(a.sh().clip_local("geheim"), 0);
    quiet();
    assert!(relay.st.tap.lock().unwrap().is_empty());
    stop(vec![a], relay);
}

#[test]
fn a_wrong_token_never_pairs_and_neither_does_an_expired_one() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    a.sh().pc_pair("start", None).unwrap();
    b.sh().pc_pair("join", Some(&new_token())).unwrap(); // a valid token, but not the one A shows
    assert!(b.sh().pc_pair("join", Some("PILL-1234")).is_err(), "not a token at all");
    quiet();
    assert_eq!((state(&a).as_str(), peers(&a).len(), peers(&b).len()), ("waiting", 0, 0), "A never saw a request");
    b.sh().pc_pair("cancel", None).unwrap();
    // an expired token: the request arrives, finds the token dead and nobody is asked
    a.sh().pair_open(true, new_token(), 1);
    let token = status(&a)["pair"]["token"].as_str().unwrap().to_string();
    std::thread::sleep(Duration::from_millis(5));
    b.sh().pc_pair("join", Some(&token)).unwrap();
    wait_for("the token dies", || state(&a) == "failed");
    assert!(status(&a)["pair"]["token"].is_null() && peers(&a).is_empty() && peers(&b).is_empty());
    stop(vec![a, b], relay);
}

#[test]
fn denying_stores_nothing_on_either_side() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    a.sh().pc_pair("start", None).unwrap();
    let token = status(&a)["pair"]["token"].as_str().unwrap().to_string();
    b.sh().pc_pair("join", Some(&token)).unwrap();
    wait_for("the request", || state(&a) == "request");
    assert!(peers(&a).is_empty(), "nothing is stored before Allow");
    a.sh().pc_pair("deny", None).unwrap();
    wait_for("the other side hears it", || state(&b) == "denied");
    assert!(peers(&a).is_empty() && peers(&b).is_empty());
    assert!(a.sh().pc_pair("allow", None).is_err(), "nothing left to allow");
    // the token was single-use: entering it again reaches nobody
    b.sh().pc_pair("join", Some(&token)).unwrap();
    quiet();
    assert_eq!(state(&a), "denied");
    stop(vec![a, b], relay);
}

#[test]
fn unpairing_reaches_the_other_side_at_once_or_when_it_comes_back() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    pair(&a, &b);
    let (ida, idb) = (id_of(&a), id_of(&b));
    // online: both records go
    a.sh().pc_peer(&idb, "unpair", true).unwrap();
    wait_for("both forgot", || peers(&a).is_empty() && peers(&b).is_empty());
    assert!(a.sh().registry().get(&idb).is_none() && b.sh().registry().get(&ida).is_none());
    // pair again, then B goes away and A unpairs: A keeps a tombstone and delivers it when B returns
    pair(&a, &b);
    b.cfg.lock().unwrap().pc_on = false;
    b.sh().relay_sync();
    wait_for("B is offline", || peers(&a).iter().all(|p| p["online"] == false));
    a.sh().pc_peer(&idb, "unpair", true).unwrap();
    assert!(peers(&a).is_empty(), "gone from the list");
    assert!(a.sh().registry().get(&idb).is_some_and(|d| d.revoked_at.is_some()), "kept as a tombstone");
    assert!(std::fs::read_to_string(a.dir.join("data").join("devices.json")).unwrap().contains("revokedAt"), "and it survives a restart");
    assert_eq!(a.sh().clip_local("nobody"), 0);
    b.cfg.lock().unwrap().pc_on = true;
    b.sh().relay_sync();
    wait_for("the tombstone is delivered", || a.sh().registry().get(&idb).is_none() && b.sh().registry().get(&ida).is_none());
    stop(vec![a, b], relay);
}

#[test]
fn an_older_copy_never_overwrites_a_newer_one_and_a_tie_goes_to_one_side_only() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    pair(&a, &b);
    let (ida, idb) = (id_of(&a), id_of(&b));
    wait_for("the listener", || b.clip.watching.load(Ordering::SeqCst));
    assert_eq!(b.sh().clip_local("B copied last"), 1);
    // A's copy was made before B's (an older stamp) and reaches B afterwards: B keeps its own, nothing is written
    let (conn, dev) = (b.sh().channels(&ida).remove(0), b.sh().registry().get(&ida).unwrap());
    b.sh().on_value(&dev, &conn, &json!({ "t": "clip", "id": "x1", "text": "A copied first", "ts": 1, "seq": 5 }));
    assert_eq!(b.clip.writes.load(Ordering::SeqCst), 0);
    // a newer one wins; the same stamp again is a duplicate
    let newer = json!({ "t": "clip", "id": "x2", "text": "A copied later", "ts": 1, "seq": 1u64 << 50 });
    b.sh().on_value(&dev, &conn, &newer);
    b.sh().on_value(&dev, &conn, &newer);
    assert_eq!((b.clip.writes.load(Ordering::SeqCst), b.clip.text.lock().unwrap().clone().as_deref()), (1, Some("A copied later")));
    // a tie: the larger id wins on both sides, so exactly one of the two applies the other's text
    let tie = (1u64 << 50) + 10;
    let (sa, sb) = (a.sh().clipst.stamp(&ida, tie as i64), b.sh().clipst.stamp(&idb, tie as i64));
    assert_eq!(sa, sb);
    let a_takes = a.sh().clipst.take_stamped("from b", sb, &idb);
    let b_takes = b.sh().clipst.take_stamped("from a", sa, &ida);
    assert!(a_takes != b_takes, "exactly one side applies, they cannot swap");
    stop(vec![a, b], relay);
}

#[test]
fn three_pills_in_a_full_mesh_do_not_ping_pong() {
    let relay = FakeRelay::start();
    let (a, b, c) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"), pill(&relay, "PC-C"));
    pair(&a, &b);
    pair(&a, &c);
    pair(&b, &c);
    wait_for("everyone is online to everyone", || [&a, &b, &c].iter().all(|r| peers(r).len() == 2 && peers(r).iter().all(|p| p["online"] == true)));
    wait_for("the listeners", || [&a, &b, &c].iter().all(|r| r.clip.watching.load(Ordering::SeqCst)));
    quiet();
    let before = relay.st.tap.lock().unwrap().len();
    assert_eq!(a.sh().clip_local("rundum"), 2);
    applied(&b, "rundum");
    applied(&c, "rundum");
    quiet();
    assert_eq!(relay.st.tap.lock().unwrap().len() - before, 2, "one frame per peer and nothing forwarded");
    assert_eq!((a.clip.writes.load(Ordering::SeqCst), b.clip.writes.load(Ordering::SeqCst), c.clip.writes.load(Ordering::SeqCst)), (0, 1, 1));
    // a copy on C reaches both of the others, once
    assert_eq!(c.sh().clip_local("zurück"), 2);
    applied(&a, "zurück");
    applied(&b, "zurück");
    quiet();
    assert_eq!((a.clip.writes.load(Ordering::SeqCst), b.clip.writes.load(Ordering::SeqCst), c.clip.writes.load(Ordering::SeqCst)), (1, 2, 1));
    stop(vec![a, b, c], relay);
}

#[test]
fn what_a_password_manager_marks_is_never_read_and_the_phone_default_is_unchanged() {
    // the watcher drops such a copy before `clip_local` is called (the same single callback serves phones and PCs)
    let mut f = super::clip::Formats::default();
    f.note("ExcludeClipboardContentFromMonitorProcessing", || None);
    assert!(!f.allows());
    assert!(!super::clip::ClipCfg::default().on, "phone.clipboard stays off by default");
}

#[test]
fn an_old_devices_file_loads_unchanged_and_its_phones_stay_phones() {
    let dir = std::env::temp_dir().join(format!("pill-pc-old-{}", super::reg::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let old = r#"{"devices":[{"id":"dev-1","name":"Pixel 8","model":"Pixel 8","os":"Android 15","appVersion":"1.0.0","tokenHash":"abc","firstSeen":1,"lastSeen":2}],"apps":{"org.x":"X"}}"#;
    std::fs::write(dir.join("devices.json"), old).unwrap();
    let r = super::reg::Registry::load(dir.clone());
    let d = r.get("dev-1").unwrap();
    assert!((d.kind.as_str(), d.joiner, d.clip_to, d.clip_from, d.revoked_at) == ("", false, false, false, None) && d.keys.is_none());
    assert_eq!(r.apps()["org.x"], "X");
    r.touch("dev-1", 9, true); // saved again: a phone record gains no new field an older Pill would trip over
    let text = std::fs::read_to_string(dir.join("devices.json")).unwrap();
    assert!(!text.contains("kind") && !text.contains("joiner") && !text.contains("clipTo") && !text.contains("revokedAt"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}
