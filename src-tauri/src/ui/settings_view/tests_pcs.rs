//! Settings > Phone > My other PCs: the pairing card in every state, the paste field, Allow / Deny, the per-PC switches and Unpair (a second click),
//! all against canned backend answers. `PILL_SNAP_DIR=<dir>` also writes the PNGs of the card states.
use super::pal::settings_light;
use super::tests::snap;
use super::tests2::{boot_with, T};
use super::tests_phone::click_act;
use super::ui::Act;
use crate::ui::host::testing::{motion_lock, motion_on};
use crate::ui::view::Clipboard;
use glasskit::theme::Theme;
use serde_json::{json, Value};

const TOKEN: &str = "PILL-7K2M-9XQD-4TBA-WE6H-CY3Z-N4AB-C2DF";

fn status(pair: Value, peers: Value) -> Value {
    json!({ "enabled": true, "name": "DESK", "peers": peers, "pair": pair })
}

fn peer(id: &str, name: &str, online: bool) -> Value {
    json!({ "id": id, "name": name, "os": "Windows", "online": online, "route": if online { "relay" } else { "none" }, "lastSeen": super::cw_phone::wall_ms() - 300_000, "clipTo": true, "clipFrom": online })
}

fn showing(state: &str) -> Value {
    json!({ "mode": "show", "state": state, "token": TOKEN, "expires": i64::MAX / 4, "peer": { "name": "LAPTOP-X", "os": "Windows" } })
}

fn boot_pcs(s: Value) -> T {
    boot_with(json!({}), "phone", |f| {
        f.api.reply("pc_status", Ok(s.clone())).reply("pc_pair", Ok(s.clone())).reply("pc_peer", Ok(s.clone())).reply("phone_status", Ok(json!({ "running": false, "devices": [] })));
    })
}

/// Scrolls the PCs section to the top of the window (the page is long).
fn to_section(t: &mut T, a: &Act) {
    for step in 0..60 {
        t.v.borrow_mut().page.scroll.scroll_to(step as f32 * 60.0, false);
        t.settle();
        let (hit, clip) = (t.v.borrow().hit_rect(a), t.v.borrow().ui.page_clip);
        if hit.is_some_and(|r| r.bottom() < clip.bottom() - 200.0) {
            return;
        }
    }
    panic!("{a:?} never came into view");
}

#[test]
fn pcs_snapshot_every_state_in_both_themes() {
    let _g = motion_lock();
    let peers = json!([peer("p1", "LAPTOP-X", true), peer("p2", "OFFICE", false)]);
    let states = [
        ("idle", status(Value::Null, json!([])), Act::PcPair("start")),
        ("done", status(json!({ "mode": "show", "state": "done", "peer": { "name": "LAPTOP-X" } }), peers.clone()), Act::PcPeer("p1".into(), "unpair", true)),
        ("token", status(showing("waiting"), json!([])), Act::PcPair("cancel")),
        ("request", status(showing("request"), json!([])), Act::PcPair("allow")),
        ("waiting", status(json!({ "mode": "enter", "state": "waiting" }), json!([])), Act::PcPair("cancel")),
        ("denied", status(json!({ "mode": "enter", "state": "denied" }), peers), Act::PcPeer("p1".into(), "unpair", true)),
    ];
    for (name, s, anchor) in states {
        let mut t = boot_pcs(s);
        to_section(&mut t, &anchor);
        for (tn, th) in [("dark", Theme::settings_dark()), ("light", settings_light())] {
            let img = snap(&mut t.sim, &format!("pcs-{name}-{tn}"), &th, (840.0, 760.0));
            assert!(img.count(None, |p| p[3] == 255) > 100_000, "{name} {tn}");
        }
    }
}

#[test]
fn show_a_token_copy_it_and_answer_a_request() {
    let _g = motion_lock();
    let mut t = boot_pcs(status(Value::Null, json!([])));
    to_section(&mut t, &Act::PcPair("start"));
    t.fakes.api.reply("pc_pair", Ok(status(showing("waiting"), json!([]))));
    click_act(&mut t, &Act::PcPair("start"));
    to_section(&mut t, &Act::PcCopy);
    click_act(&mut t, &Act::PcCopy);
    assert_eq!(t.fakes.clipboard.get_text().as_deref(), Some(TOKEN));
    assert!(t.v.borrow().hit_rect(&Act::PcPair("cancel")).is_some(), "the card has a Cancel");
    t.fakes.api.reply("pc_pair", Ok(status(showing("request"), json!([]))));
    click_act(&mut t, &Act::PcPair("cancel"));
    for op in ["allow", "deny"] {
        click_act(&mut t, &Act::PcPair(op));
    }
    assert_eq!(t.calls("pc_pair").iter().map(|c| c["op"].as_str().unwrap().to_string()).collect::<Vec<_>>(), ["start", "cancel", "allow", "deny"]);
}

#[test]
fn a_pasted_token_is_sent_and_a_refused_one_says_so() {
    let _g = motion_lock();
    let mut t = boot_pcs(status(Value::Null, json!([])));
    to_section(&mut t, &Act::PcPair("join"));
    t.v.borrow_mut().data.fields.get_mut("token").expect("the field is drawn").set_text("  PILL-7K2M-9XQD ");
    t.fakes.api.reply("pc_pair", Err("that is not a Pill token".into()));
    click_act(&mut t, &Act::PcPair("join"));
    assert_eq!(t.calls("pc_pair"), vec![json!({ "op": "join", "token": "  PILL-7K2M-9XQD " })]);
    assert!(t.v.borrow().data.pcs.bad_token);
    t.fakes.api.reply("pc_pair", Ok(status(json!({ "mode": "enter", "state": "waiting" }), json!([]))));
    click_act(&mut t, &Act::PcPair("join"));
    assert!(!t.v.borrow().data.pcs.bad_token && t.v.borrow().data.fields["token"].text().is_empty(), "accepted: the field is emptied");
}

#[test]
fn the_switches_of_a_pc_and_unpair_with_a_second_click() {
    let _m = motion_on();
    let mut t = boot_pcs(status(Value::Null, json!([peer("p1", "LAPTOP-X", true)])));
    let unpair = Act::PcPeer("p1".into(), "unpair", true);
    to_section(&mut t, &unpair);
    click_act(&mut t, &Act::PcPeer("p1".into(), "clipTo", false));
    assert_eq!(t.calls("pc_peer"), vec![json!({ "id": "p1", "op": "clipTo", "on": false })]);
    click_act(&mut t, &unpair);
    assert!(t.calls("pc_peer").len() == 1 && t.v.borrow().data.pcs.armed.is_some(), "the first click only arms it");
    t.fakes.api.reply("pc_peer", Ok(status(Value::Null, json!([])))).reply("pc_status", Ok(status(Value::Null, json!([]))));
    click_act(&mut t, &unpair);
    assert_eq!(t.calls("pc_peer").last(), Some(&json!({ "id": "p1", "op": "unpair", "on": true })));
    assert!(t.v.borrow().data.pcs.rows.iter().all(|r| !r.present), "the row has left or is leaving");
}
