// Tests of the PC link (docs/pc-link.md): real servers ("Pills", each with its own data folder and a fake clipboard) joined through the fake relay of
// relay_tests.rs. Nothing leaves the machine.
use super::pc::{new_token, parse_token, show};
use super::relay_tests::{wait_for, FakeRelay};
use super::tests::Rig;
use serde_json::{json, Value};
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

fn pill(relay: &FakeRelay, name: &str) -> Rig {
    let r = Rig::new(true);
    let mut c = r.cfg.lock().unwrap();
    (c.relay_url, c.pc_name) = (relay.url(), name.into());
    drop(c);
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

/// `a` shows a token and returns it.
fn token(a: &Rig) -> String {
    a.sh().pc_pair("start", None).unwrap();
    status(a)["pair"]["token"].as_str().expect("the token is shown").to_string()
}

/// `a` shows a token, `b` enters it, `a` allows; returns when both stored the other and are connected.
fn pair(a: &Rig, b: &Rig) {
    b.sh().pc_pair("join", Some(&token(a))).unwrap();
    wait_for("the request reaches the shown side", || state(a) == "request");
    assert_eq!(status(a)["pair"]["peer"]["name"], b.cfg.lock().unwrap().pc_name, "the card names the PC that asks");
    a.sh().pc_pair("allow", None).unwrap();
    wait_for("both sides paired and online", || [a, b].iter().all(|r| state(r) == "done" && peers(r).iter().any(|p| p["online"] == true)));
}

/// Waits until the Pill's clipboard shows `text`.
fn applied(r: &Rig, text: &str) {
    wait_for("the text arrives", || r.clip.text.lock().unwrap().as_deref() == Some(text));
}

fn quiet() {
    std::thread::sleep(Duration::from_millis(400));
}

fn writes(rs: &[&Rig]) -> Vec<usize> {
    rs.iter().map(|r| r.clip.writes.load(SeqCst)).collect()
}

fn stop(rigs: Vec<Rig>, relay: FakeRelay) {
    rigs.into_iter().for_each(Rig::stop);
    relay.stop();
}

#[test]
fn the_token_is_long_typo_proof_and_forgiving() {
    let t = new_token();
    assert_eq!(t.len(), 28);
    let shown = show(&t);
    assert!(shown.starts_with("PILL-") && shown.matches('-').count() == 7, "{shown}");
    for typed in [shown.clone(), shown.to_lowercase(), t.clone(), format!("  {}\n", shown.replace('-', " "))] {
        assert_eq!(parse_token(&typed).as_deref(), Some(t.as_str()), "{typed:?}");
    }
    // one wrong character, a short one and rubbish are refused before the relay is asked for a room
    let other = if t.starts_with('0') { '1' } else { '0' };
    assert!(parse_token(&format!("{other}{}", &t[1..])).is_none() && parse_token(&t[..27]).is_none() && parse_token("PILL-hello").is_none());
}

#[test]
fn two_pills_pair_and_share_the_clipboard_both_ways_but_nothing_before() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    a.sh().clip_sync();
    assert!(!a.clip.watching.load(SeqCst) && a.sh().clip_local("geheim") == 0, "no PC paired: no clipboard listener, nothing is sent");
    pair(&a, &b);
    // both sides hold the same keys; the one that entered the token is the relay's `phone` end; neither the token nor the pairing room is on disk
    let (da, db) = (a.sh().registry().get(&id_of(&b)).unwrap(), b.sh().registry().get(&id_of(&a)).unwrap());
    assert!(da.keys.is_some() && da.keys == db.keys && da.kind == "pc" && !da.joiner && db.joiner);
    let disk = std::fs::read_to_string(a.dir.join("data").join("devices.json")).unwrap();
    assert!(!disk.contains("pairing") && !disk.contains("PILL"), "{disk}");
    // on by default: a copy on A lands on B at once (and B does not send it back), then the other way
    wait_for("the listener", || a.clip.watching.load(SeqCst) && b.clip.watching.load(SeqCst));
    assert_eq!(a.sh().clip_local("первый текст ЛИМОН"), 1);
    applied(&b, "первый текст ЛИМОН");
    b.wait("pc:clip", |v| v["name"] == "PC-A");
    assert_eq!(b.sh().clip_local("первый текст ЛИМОН"), 0, "what was applied is never echoed");
    assert_eq!(b.sh().clip_local("ответ"), 1);
    applied(&a, "ответ");
    for f in relay.st.tap.lock().unwrap().iter() {
        let hay = String::from_utf8_lossy(f);
        assert!(!hay.contains("ЛИМОН") && !hay.contains("clip") && !hay.contains("PC-A"), "plaintext leaked to the relay");
    }
    // each direction can be switched off per peer
    let (ida, idb) = (id_of(&a), id_of(&b));
    a.sh().pc_peer(&idb, "clipTo", false).unwrap();
    assert_eq!(a.sh().clip_local("nicht"), 0);
    a.sh().pc_peer(&idb, "clipTo", true).unwrap();
    b.sh().pc_peer(&ida, "clipFrom", false).unwrap();
    assert_eq!(a.sh().clip_local("taub"), 1, "it is sent, the other side does not apply it");
    quiet();
    assert_eq!(b.clip.text.lock().unwrap().as_deref(), Some("первый текст ЛИМОН"));
    stop(vec![a, b], relay);
}

#[test]
fn a_wrong_token_never_pairs_and_neither_does_an_expired_one() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    token(&a);
    b.sh().pc_pair("join", Some(&new_token())).unwrap(); // a valid token, but not the one A shows
    assert!(b.sh().pc_pair("join", Some("PILL-1234")).is_err(), "not a token at all");
    quiet();
    assert_eq!((state(&a).as_str(), peers(&a).len(), peers(&b).len()), ("waiting", 0, 0), "A never saw a request");
    // an expired token: the request arrives, finds the token dead and nobody is asked
    a.sh().pair_open(true, new_token(), 1);
    let t = status(&a)["pair"]["token"].as_str().unwrap().to_string();
    std::thread::sleep(Duration::from_millis(5));
    b.sh().pc_pair("join", Some(&t)).unwrap();
    wait_for("the token dies", || state(&a) == "failed");
    assert!(status(&a)["pair"]["token"].is_null() && peers(&a).is_empty() && peers(&b).is_empty());
    stop(vec![a, b], relay);
}

#[test]
fn denying_stores_nothing_and_the_token_is_single_use() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    let t = token(&a);
    b.sh().pc_pair("join", Some(&t)).unwrap();
    wait_for("the request", || state(&a) == "request");
    assert!(peers(&a).is_empty(), "nothing is stored before Allow");
    a.sh().pc_pair("deny", None).unwrap();
    wait_for("the other side hears it", || state(&b) == "denied");
    assert!(peers(&a).is_empty() && peers(&b).is_empty() && a.sh().pc_pair("allow", None).is_err(), "nothing left to allow");
    b.sh().pc_pair("join", Some(&t)).unwrap(); // entering it again reaches nobody
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
    a.sh().pc_peer(&idb, "unpair", true).unwrap();
    wait_for("both forgot", || a.sh().registry().get(&idb).is_none() && b.sh().registry().get(&ida).is_none());
    // pair again, then B goes away and A unpairs: A keeps a tombstone and delivers it when B returns
    pair(&a, &b);
    b.cfg.lock().unwrap().pc_on = false;
    b.sh().relay_sync();
    wait_for("B is offline", || peers(&a).iter().all(|p| p["online"] == false));
    a.sh().pc_peer(&idb, "unpair", true).unwrap();
    assert!(peers(&a).is_empty() && a.sh().registry().get(&idb).is_some_and(|d| d.revoked_at.is_some()), "hidden, kept as a tombstone");
    assert!(std::fs::read_to_string(a.dir.join("data").join("devices.json")).unwrap().contains("revokedAt"), "which survives a restart");
    assert_eq!(a.sh().clip_local("nobody"), 0);
    b.cfg.lock().unwrap().pc_on = true;
    b.sh().relay_sync();
    wait_for("the tombstone is delivered", || a.sh().registry().get(&idb).is_none() && b.sh().registry().get(&ida).is_none());
    stop(vec![a, b], relay);
}

#[test]
fn an_older_copy_never_overwrites_a_newer_one() {
    let relay = FakeRelay::start();
    let (a, b) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"));
    pair(&a, &b);
    let ida = id_of(&a);
    wait_for("the listener", || b.clip.watching.load(SeqCst));
    assert_eq!(b.sh().clip_local("B copied last"), 1);
    // A's copy has an older stamp and reaches B afterwards: B keeps its own, nothing is written
    let (conn, dev) = (b.sh().channels(&ida).remove(0), b.sh().registry().get(&ida).unwrap());
    let clip = |seq: u64, text: &str| json!({ "t": "clip", "id": "x", "text": text, "ts": 1, "seq": seq });
    b.sh().on_value(&dev, &conn, &clip(5, "A copied first"));
    assert_eq!(writes(&[&b]), [0]);
    // a newer one wins; the same stamp again is a duplicate
    for _ in 0..2 {
        b.sh().on_value(&dev, &conn, &clip(1 << 50, "A copied later"));
    }
    assert_eq!((writes(&[&b]), b.clip.text.lock().unwrap().clone().as_deref()), (vec![1], Some("A copied later")));
    stop(vec![a, b], relay);
}

#[test]
fn three_pills_in_a_full_mesh_do_not_ping_pong() {
    let relay = FakeRelay::start();
    let (a, b, c) = (pill(&relay, "PC-A"), pill(&relay, "PC-B"), pill(&relay, "PC-C"));
    pair(&a, &b);
    pair(&a, &c);
    pair(&b, &c);
    let all = [&a, &b, &c];
    wait_for("everyone is online to everyone, listening", || all.iter().all(|r| peers(r).len() == 2 && peers(r).iter().all(|p| p["online"] == true) && r.clip.watching.load(SeqCst)));
    quiet();
    let before = relay.st.tap.lock().unwrap().len();
    assert_eq!(a.sh().clip_local("rundum"), 2);
    applied(&b, "rundum");
    applied(&c, "rundum");
    quiet();
    assert_eq!(relay.st.tap.lock().unwrap().len() - before, 2, "one frame per peer and nothing forwarded");
    assert_eq!(writes(&all), [0, 1, 1]);
    assert_eq!(c.sh().clip_local("zurück"), 2);
    applied(&a, "zurück");
    applied(&b, "zurück");
    quiet();
    assert_eq!(writes(&all), [1, 2, 1]);
    stop(vec![a, b, c], relay);
}

#[test]
fn an_old_devices_file_loads_unchanged_and_its_phones_stay_phones() {
    let dir = std::env::temp_dir().join(format!("pill-pc-old-{}", super::reg::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("devices.json"), r#"{"devices":[{"id":"dev-1","name":"Pixel 8","model":"Pixel 8","os":"Android 15","appVersion":"1.0.0","tokenHash":"abc","firstSeen":1,"lastSeen":2}],"apps":{"org.x":"X"}}"#).unwrap();
    let r = super::reg::Registry::load(dir.clone());
    let d = r.get("dev-1").unwrap();
    assert!((d.kind.as_str(), d.joiner, d.clip_to, d.clip_from, d.revoked_at) == ("", false, false, false, None) && d.keys.is_none());
    r.touch("dev-1", 9, true); // saved again: a phone record gains no field an older Pill would trip over
    let text = std::fs::read_to_string(dir.join("devices.json")).unwrap();
    assert!(!text.contains("kind") && !text.contains("joiner") && !text.contains("clipTo") && !text.contains("revokedAt"), "{text}");
    let _ = std::fs::remove_dir_all(dir);
}
