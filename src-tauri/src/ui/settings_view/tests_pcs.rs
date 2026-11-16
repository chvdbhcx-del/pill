//! Settings > Phone > My other PCs: the pairing card in every state, the paste field, Allow / Deny, the per-PC switches and Unpair (a second click).
//! All against canned backend answers. `PILL_SNAP_DIR=<dir>` also writes the PNGs of the card states.
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

fn peer() -> Value {
    json!({ "id": "p1", "name": "LAPTOP-X", "os": "Windows", "online": true, "route": "relay", "lastSeen": super::cw_phone::wall_ms() - 300_000, "clipTo": true, "clipFrom": true })
}

fn showing(state: &str) -> Value {
    json!({ "mode": "show", "state": state, "token": TOKEN, "expires": i64::MAX / 4, "peer": { "name": "LAPTOP-X", "os": "Windows" } })
}

fn boot_pcs(s: Value) -> T {
    boot_with(json!({}), "phone", |f| {
        f.api.reply("pc_status", Ok(s.clone())).reply("pc_pair", Ok(s.clone())).reply("pc_peer", Ok(s.clone())).reply("phone_status", Ok(json!({ "running": false, "devices": [] })));
    })
}

/// Scrolls the PCs section to the top of the window (the page is long) and draws a frame.
fn to_section(t: &mut T, a: &Act) {
    for step in 0..60 {
        t.v.borrow_mut().page.scroll.scroll_to(step as f32 * 60.0, false);
        t.settle();
        let (hit, clip) = {
            let v = t.v.borrow();
            (v.hit_rect(a), v.ui.page_clip)
        };
        if hit.is_some_and(|r| r.bottom() < clip.bottom() - 200.0) {
            return;
        }
    }
    panic!("{a:?} never came into view");
}

#[test]
fn pcs_snapshot_every_state_in_both_themes() {
    let _g = motion_lock();
    let states = [
        ("idle", status(Value::Null, json!([]))),
        ("done", status(json!({ "mode": "show", "state": "done", "peer": { "name": "LAPTOP-X" } }), json!([peer()]))),
        ("token", status(showing("waiting"), json!([]))),
        ("request", status(showing("request"), json!([]))),
        ("waiting", status(json!({ "mode": "enter", "state": "waiting" }), json!([]))),
        ("denied", status(json!({ "mode": "enter", "state": "denied" }), json!([peer(), json!({ "id": "p2", "name": "OFFICE", "os": "Windows", "online": false, "route": "none", "lastSeen": 1, "clipTo": true, "clipFrom": false })]))),
    ];
    for (name, s) in states {
        let mut t = boot_pcs(s);
        let anchor = match name {
            "idle" => Act::PcPair("start"),
            "request" => Act::PcPair("allow"),
            "done" | "denied" => Act::PcPeer("p1".into(), "unpair", true),
            _ => Act::PcPair("cancel"),
        };
        to_section(&mut t, &anchor);
        for (tn, th) in [("dark", Theme::settings_dark()), ("light", settings_light())] {
            let img = snap(&mut t.sim, &format!("pcs-{name}-{tn}"), &th, (840.0, 760.0));
            assert!(img.count(None, |p| p[3] == 255) > 100_000, "{name} {tn}");
        }
    }
}

#[test]
fn show_a_token_then_copy_it() {
    let _g = motion_lock();
    let mut t = boot_pcs(status(Value::Null, json!([])));
    to_section(&mut t, &Act::PcPair("start"));
    t.fakes.api.reply("pc_pair", Ok(status(showing("waiting"), json!([]))));
    click_act(&mut t, &Act::PcPair("start"));
    assert_eq!(t.calls("pc_pair"), vec![json!({ "op": "start", "token": null })]);
    to_section(&mut t, &Act::PcCopy);
    click_act(&mut t, &Act::PcCopy);
    assert_eq!(t.fakes.clipboard.get_text().as_deref(), Some(TOKEN));
    assert!(t.v.borrow().hit_rect(&Act::PcPair("cancel")).is_some(), "the card has a Cancel");
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
fn allow_and_deny_answer_the_request() {
    let _g = motion_lock();
    let mut t = boot_pcs(status(showing("request"), json!([])));
    to_section(&mut t, &Act::PcPair("allow"));
    click_act(&mut t, &Act::PcPair("allow"));
    click_act(&mut t, &Act::PcPair("deny"));
    assert_eq!(t.calls("pc_pair"), vec![json!({ "op": "allow", "token": null }), json!({ "op": "deny", "token": null })]);
}

#[test]
fn the_switches_of_a_pc_and_unpair_with_a_second_click() {
    let _m = motion_on();
    let mut t = boot_pcs(status(Value::Null, json!([peer()])));
    to_section(&mut t, &Act::PcPeer("p1".into(), "unpair", true));
    click_act(&mut t, &Act::PcPeer("p1".into(), "clipTo", false));
    assert_eq!(t.calls("pc_peer"), vec![json!({ "id": "p1", "op": "clipTo", "on": false })]);
    let a = Act::PcPeer("p1".into(), "unpair", true);
    click_act(&mut t, &a);
    assert_eq!(t.calls("pc_peer").len(), 1, "the first click only arms it");
    assert!(t.v.borrow().data.pcs.armed.is_some());
    t.fakes.api.reply("pc_peer", Ok(status(Value::Null, json!([])))).reply("pc_status", Ok(status(Value::Null, json!([]))));
    click_act(&mut t, &a);
    assert_eq!(t.calls("pc_peer").last(), Some(&json!({ "id": "p1", "op": "unpair", "on": true })));
    assert!(t.v.borrow().data.pcs.rows.iter().all(|r| !r.present), "the row has left or is leaving");
}
