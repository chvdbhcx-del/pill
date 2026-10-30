//! Behaviour tests of the uninstall panel controller: the typing-never-blocks invariant, rehydration from the backend after a restart, the keyboard
//! capture rules, the checklist round trip, step transitions and reduced motion. All backend traffic is faked: nothing here touches a real system.

use super::*;
use crate::bar::anim::Kind as AK;
use crate::bar::protocol::Kind;
use serde_json::json;

fn ui(reduce: bool) -> UninstallUi {
    UninstallUi::new(Lang::En, reduce)
}
fn calls(outs: &[Out]) -> Vec<(&'static str, Value)> {
    outs.iter().filter_map(|o| if let Out::Call { call, .. } = o { Some((call.cmd, call.args.clone())) } else { None }).collect()
}
fn cmds(outs: &[Out]) -> Vec<&'static str> {
    calls(outs).into_iter().map(|c| c.0).collect()
}
fn has_anim(outs: &[Out], k: AK) -> bool {
    outs.iter().any(|o| matches!(o, Out::Anim(a) if a.kind == k))
}
fn any_anim(outs: &[Out]) -> bool {
    outs.iter().any(|o| matches!(o, Out::Anim(_)))
}
fn row() -> Res {
    Res::new(Kind::Uninstall, "uninstall:3", "qBittorrent", "The qBittorrent project · 4.6.5", "3")
}
fn info() -> Value {
    json!({ "ok": true, "name": "qBittorrent", "publisher": "The qBittorrent project", "version": "4.6.5", "location": "C:\\Program Files\\qBittorrent", "sizeKb": 51200, "store": false, "canSilent": true, "risky": false })
}
fn st(phase: &str, extra: Value) -> Value {
    let mut v = json!({ "phase": phase, "name": "qBittorrent" });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
}
fn item(id: usize, path: &str, tier: &str, admin: bool, size: u64, ticked: bool) -> Value {
    json!({ "id": id, "kind": "folder", "path": path, "size": size, "ticked": ticked, "note": "install folder", "admin": admin, "tier": tier })
}
fn review() -> Value {
    st("review", json!({ "items": [item(0, "C:\\a", "certain", false, 1000, true), item(1, "C:\\b", "certain", false, 2000, true), item(2, "C:\\c", "probable", false, 500, false), item(3, "HKLM\\d", "certain", true, 0, false)], "manual": [], "cut": false, "needAdmin": 1 }))
}
/// Opens the confirm card the way the bar does (call, then the faked reply) and settles every transition.
fn confirmed(u: &mut UninstallUi, t: &mut f64) {
    let o = u.open(&row(), *t);
    assert_eq!(cmds(&o), vec!["uninstall_info"]);
    u.on_reply(ReplyTag::Info, Some(&info()), *t);
    settle(u, t);
}
fn settle(u: &mut UninstallUi, t: &mut f64) {
    for _ in 0..10 {
        *t += 100.0;
        u.tick(*t);
    }
}

#[test]
fn confirm_card_round_trip() {
    let mut u = ui(false);
    let mut t = 0.0;
    let o = u.open(&row(), t);
    assert_eq!(calls(&o), vec![("uninstall_info", json!({ "id": 3, "name": "qBittorrent" }))]);
    assert!(matches!(&o[0], Out::Call { reply: Some(ReplyTag::Info), .. }));
    let o = u.on_reply(ReplyTag::Info, Some(&info()), t);
    assert!(u.takeover());
    assert_eq!(u.state().phase, Phase::Confirm);
    assert!(o.contains(&Out::FocusDefault), "the confirm card gives its default button the keyboard");
    assert!(has_anim(&o, AK::Enter));
    settle(&mut u, &mut t);
    let v = u.view();
    let p = v.panel.unwrap();
    assert_eq!(p.phase, "confirm");
    assert!(p.parts.contains(&Part::Heading("qBittorrent".into())));
    assert!(p.parts.contains(&Part::Sub("The qBittorrent project \u{b7} 4.6.5 \u{b7} 50 MB".into())));
    assert!(p.parts.iter().any(|x| matches!(x, Part::Sub(s) if s.starts_with("Folder: C:\\Program Files"))));
    assert!(p.parts.iter().any(|x| matches!(x, Part::Check { control: Control::ToggleSilent, checked: false, .. })));
    let Some(Part::Buttons(b)) = p.parts.last() else { panic!("buttons last") };
    assert_eq!(b[0].control, Control::Start);
    assert_eq!(b[0].style, ButtonStyle::Danger);
    assert!(b[1].default_focus && b[1].control == Control::Cancel, "Cancel is the default so Enter never starts a removal by accident");
    assert!(v.list_hidden);
    // silent tick, then start
    u.press(Control::ToggleSilent, t);
    let o = u.press(Control::Start, t);
    assert_eq!(calls(&o), vec![("uninstall_start", json!({ "silent": true }))]);
}

#[test]
fn a_stale_or_busy_info_answer_just_reruns_the_search() {
    let mut u = ui(false);
    u.open(&row(), 0.0);
    let o = u.on_reply(ReplyTag::Info, Some(&json!({ "ok": false, "error": "stale" })), 0.0);
    assert!(o.contains(&Out::SearchAgain));
    assert!(!u.takeover());
    let o = u.on_reply(ReplyTag::Info, None, 0.0);
    assert!(o.contains(&Out::SearchAgain), "a failed invoke is a null");
}

#[test]
fn store_apps_and_risky_components_show_their_notes() {
    let mut u = ui(true);
    u.open(&row(), 0.0);
    let mut i = info();
    i["store"] = json!(true);
    i["risky"] = json!(true);
    i["canSilent"] = json!(false);
    u.on_reply(ReplyTag::Info, Some(&i), 0.0);
    let p = u.view().panel.unwrap();
    assert!(p.parts.contains(&Part::Sub("Microsoft Store app".into())));
    assert!(p.parts.contains(&Part::Warn { text: s::RISKY.1.into(), shake: false }));
    assert!(!p.parts.iter().any(|x| matches!(x, Part::Check { .. })));
}

#[test]
fn cancel_on_the_card_drops_everything_and_returns_to_search() {
    let mut u = ui(false);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    let o = u.press(Control::Cancel, t);
    assert_eq!(cmds(&o), vec!["uninstall_cancel"]);
    assert!(has_anim(&o, AK::Collapse));
    assert!(!u.takeover() && u.state().phase == Phase::Idle);
    let o2 = u.tick(t + 300.0);
    assert!(o2.contains(&Out::FocusInput) && o2.contains(&Out::SearchAgain), "after the collapse the box is focused and the query reruns");
    assert!(u.view().panel.is_none() && !u.view().list_hidden);
}

#[test]
fn typing_never_blocks_a_running_job() {
    let mut u = ui(false);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("running", json!({})), t);
    settle(&mut u, &mut t);
    u.on_state(&st("scanning", json!({ "step": "files and folders", "found": 5 })), t);
    settle(&mut u, &mut t);
    assert!(u.view().panel.is_some());
    // a keystroke in the search box: the panel steps aside at once (takeover is false), the job is untouched
    let o = u.on_input(t);
    assert!(!u.takeover(), "the very next event after the keystroke already has the box free");
    assert!(u.has_job());
    assert!(!cmds(&o).contains(&"uninstall_cancel"), "typing must not cancel the job");
    assert!(has_anim(&o, AK::Collapse));
    t += 300.0;
    u.tick(t);
    let v = u.view();
    assert!(v.panel.is_none() && !v.list_hidden, "the result list is back");
    let strip = v.strip.expect("the strip shows the job");
    assert_eq!((strip.kind, strip.phase), (StripKind::Busy, Phase::Scanning));
    assert_eq!(strip.icon, StripIcon::Spinner);
    assert_eq!(strip.name, "qBittorrent");
    assert!(strip.bar);
    // the job moves on while the user searches elsewhere: the panel does not come back by itself
    u.on_state(&st("scanning", json!({ "step": "registry (user)", "found": 40 })), t);
    assert!(!u.takeover());
    let o = u.on_state(&review(), t);
    assert!(!u.takeover() && !o.iter().any(|x| matches!(x, Out::FocusDefault)));
    t += 600.0;
    u.tick(t);
    let strip = u.view().strip.unwrap();
    assert_eq!((strip.kind, strip.phase), (StripKind::Ready, Phase::Checklist));
    assert_eq!(strip.count, Some(4), "the strip reports the number of leftovers to review");
    assert_eq!(strip.icon, StripIcon::Check);
}

#[test]
fn typing_in_a_confirm_card_drops_it_without_telling_the_backend() {
    let mut u = ui(false);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    let o = u.on_input(t);
    assert!(cmds(&o).is_empty(), "a confirm card is not a job: there is nothing to cancel");
    assert_eq!(u.state().phase, Phase::Idle);
    assert!(u.view().strip.is_none());
}

#[test]
fn esc_hides_a_running_job_but_dismisses_a_summary_and_tells_the_backend() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("scanning", json!({ "found": 1 })), t);
    let (consumed, o) = u.on_key(&KeyEvent::new("Escape"), t);
    assert!(consumed);
    assert!(cmds(&o).is_empty() && u.has_job() && !u.takeover());
    assert!(o.contains(&Out::FocusInput));
    // Esc on a finished summary: nothing is left to come back to
    let mut u2 = ui(true);
    confirmed(&mut u2, &mut t);
    u2.on_state(&st("done", json!({ "files": 3, "keys": 1, "other": 0, "failed": [], "items": [] })), t);
    let (_, o) = u2.on_key(&KeyEvent::new("Escape"), t);
    assert_eq!(cmds(&o), vec!["uninstall_cancel"]);
    assert!(!u2.has_job());
}

#[test]
fn rehydration_after_a_restart_rebuilds_strip_and_panel_from_the_backend() {
    // the bar window was torn down and rebuilt: a fresh controller knows nothing
    let mut u = ui(true);
    let o = u.on_bar_shown(0.0);
    assert_eq!(calls(&o), vec![("uninstall_state", Value::Null)]);
    assert!(matches!(&o[0], Out::Call { reply: Some(ReplyTag::State), .. }));
    // backend: the scan is still running
    u.on_reply(ReplyTag::State, Some(&st("scanning", json!({ "step": "services", "found": 9 }))), 0.0);
    assert!(!u.takeover(), "a recovered job shows as a strip, never forces the panel up");
    let strip = u.view().strip.unwrap();
    assert_eq!((strip.kind, strip.phase, strip.count), (StripKind::Busy, Phase::Scanning, Some(9)));
    // the user clicks the strip: the panel is drawn from the last state
    let o = u.on_focus(1.0);
    assert!(u.takeover());
    let p = u.view().panel.unwrap();
    assert_eq!(p.phase, "scanning");
    assert!(p.parts.contains(&Part::ScanLine { label: "found: ".into(), found: 9, stage: "services".into() }));
    assert!(u.view().strip.is_none(), "the strip yields to the panel");
    assert!(!o.is_empty());
    // the same phase read again changes nothing; null (no job) is ignored
    let before = u.state().clone();
    u.on_reply(ReplyTag::State, Some(&st("scanning", json!({ "step": "services", "found": 9 }))), 2.0);
    assert_eq!(u.state().phase, before.phase);
    u.on_reply(ReplyTag::State, Some(&Value::Null), 2.0);
    assert!(u.has_job());
}

#[test]
fn rehydration_honours_the_focus_flag_from_the_island_card() {
    let mut u = ui(true);
    u.on_bar_shown(0.0);
    u.on_reply(ReplyTag::State, Some(&st("review", json!({ "items": [item(0, "C:\\a", "certain", false, 10, true)], "focus": true }))), 0.0);
    assert!(u.takeover(), "uninstall_open_panel set `focus`: the panel is up");
    let p = u.view().panel.unwrap();
    assert!(p.parts.iter().any(|x| matches!(x, Part::Group(g) if g.rows.len() == 1)));
}

#[test]
fn rehydration_with_an_empty_backend_leaves_the_bar_alone() {
    let mut u = ui(true);
    u.on_bar_shown(0.0);
    u.on_reply(ReplyTag::State, Some(&Value::Null), 0.0);
    u.on_reply(ReplyTag::State, Some(&json!({})), 0.0);
    assert!(!u.has_job() && u.view().strip.is_none() && u.view().panel.is_none());
}

#[test]
fn ctrl_u_brings_the_panel_back_exactly_where_it_was() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&review(), t);
    u.press(Control::ToggleItem(2), t); // tick a probable item
    u.on_input(t);
    assert!(!u.takeover());
    let (consumed, _) = u.on_key(&KeyEvent::new("u").with_code("KeyU").ctrl(), t);
    assert!(consumed && u.takeover());
    let p = u.view().panel.unwrap();
    let ticked: Vec<usize> = p.parts.iter().filter_map(|x| if let Part::Group(g) = x { Some(g) } else { None }).flat_map(|g| g.rows.iter().filter(|r| r.checked).map(|r| r.pick)).collect();
    assert_eq!(ticked, vec![0, 1, 2], "the user's tick survived the trip");
    // Ctrl+U with no job does nothing but is still consumed (nothing else in the bar uses it)
    let mut idle = ui(true);
    let (c, o) = idle.on_key(&KeyEvent::new("u").with_code("KeyU").ctrl(), 0.0);
    assert!(c && o.is_empty());
}

#[test]
fn key_capture_rules_while_the_panel_is_up() {
    let mut u = ui(true);
    let mut t = 0.0;
    // no panel: nothing is captured
    assert!(!u.on_key(&KeyEvent::new("Enter"), t).0);
    confirmed(&mut u, &mut t);
    // Enter / arrows / Tab / F4 / Ctrl+Shift belong to the panel
    for k in ["Enter", "ArrowUp", "ArrowDown", "Tab", "F4"] {
        assert!(u.on_key(&KeyEvent::new(k), t).0, "{k}");
    }
    assert!(u.on_key(&KeyEvent::new("c").with_code("KeyC").ctrl().shift(), t).0);
    assert!(u.takeover());
    // a plain letter while a panel button has the keyboard: the panel leaves, the key falls through to the bar
    let (consumed, o) = u.on_key(&KeyEvent::new("щ"), t);
    assert!(!consumed && !u.takeover());
    assert!(o.contains(&Out::FocusInput));
    // typing inside the search box itself is the input handler's job, not the key capture's
    let mut u2 = ui(true);
    confirmed(&mut u2, &mut t);
    let (consumed, _) = u2.on_key(&KeyEvent::new("a").in_search(), t);
    assert!(!consumed && u2.takeover(), "the input event steps the panel aside, not the keydown");
    u2.on_input(t);
    assert!(!u2.takeover());
    // Backspace outside a field is typing; inside one it edits itself
    let mut u3 = ui(true);
    confirmed(&mut u3, &mut t);
    u3.on_key(&KeyEvent::new("Backspace").in_field(), t);
    assert!(u3.takeover());
    u3.on_key(&KeyEvent::new("Backspace"), t);
    assert!(!u3.takeover());
}

#[test]
fn scan_count_glides_and_the_stage_word_swaps_in_two_steps() {
    let mut u = ui(false);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("scanning", json!({ "step": "files and folders", "found": 0 })), t);
    settle(&mut u, &mut t);
    let found = |u: &UninstallUi| match u.view().panel.unwrap().parts.iter().find_map(|p| if let Part::ScanLine { found, stage, .. } = p { Some((*found, stage.clone())) } else { None }) {
        Some(x) => x,
        None => panic!("no scan line"),
    };
    assert_eq!(found(&u), (0, "files and folders".into()));
    t += 10.0;
    let o = u.on_state(&st("scanning", json!({ "step": "registry (user)", "found": 200 })), t);
    assert!(has_anim(&o, AK::Exit), "the old stage word leaves");
    assert!(u.animating());
    t += 50.0;
    u.tick(t);
    let (mid, stage) = found(&u);
    assert!(mid > 0 && mid < 200, "the count is gliding: {mid}");
    assert_eq!(stage, "files and folders", "the stage word has not been swapped yet");
    t += 100.0;
    let o = u.tick(t);
    assert_eq!(found(&u).1, "registry (user)");
    assert!(has_anim(&o, AK::Enter), "the new stage word enters");
    t += 600.0;
    u.tick(t);
    assert_eq!(found(&u).0, 200);
    assert!(!u.animating());
}

#[test]
fn step_changes_are_two_phase_and_reduce_motion_applies_at_once() {
    // with motion: the old step leaves for 140 ms
    let mut u = ui(false);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    let o = u.on_state(&st("running", json!({})), t);
    assert!(has_anim(&o, AK::Exit));
    assert_eq!(u.view().panel.unwrap().phase, "confirm", "the confirm card is still on screen while it leaves");
    u.tick(t + 100.0);
    assert_eq!(u.view().panel.unwrap().phase, "confirm");
    let o = u.tick(t + 150.0);
    assert_eq!(u.view().panel.unwrap().phase, "running");
    assert!(has_anim(&o, AK::Enter));
    // reduced motion: immediate and silent
    let mut r = ui(true);
    let mut t2 = 0.0;
    confirmed(&mut r, &mut t2);
    let mut all = r.on_state(&st("running", json!({})), t2);
    assert_eq!(r.view().panel.unwrap().phase, "running");
    all.extend(r.on_state(&st("scanning", json!({ "found": 10 })), t2));
    all.extend(r.on_input(t2));
    all.extend(r.on_state(&review(), t2));
    all.extend(r.on_focus(t2));
    assert!(!any_anim(&all), "no animation under reduced motion");
    assert!(!r.animating());
    assert!(r.view().strip.is_none());
}

#[test]
fn the_result_list_steps_aside_first_only_when_it_has_rows() {
    let mut u = ui(false);
    u.list_has_rows = true;
    let o = u.open(&row(), 0.0);
    assert!(!any_anim(&o));
    let o = u.on_reply(ReplyTag::Info, Some(&info()), 0.0);
    assert!(has_anim(&o, AK::Exit));
    assert!(!u.view().list_hidden && u.view().panel.is_none(), "the card waits for the list to leave");
    u.tick(130.0);
    assert!(u.view().list_hidden && u.view().panel.is_some());
    let mut empty = ui(false);
    empty.list_has_rows = false;
    empty.open(&row(), 0.0);
    empty.on_reply(ReplyTag::Info, Some(&info()), 0.0);
    assert!(empty.view().panel.is_some(), "an empty list needs no exit");
}

#[test]
fn checklist_view_groups_counts_and_buttons() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&review(), t);
    let p = u.view().panel.unwrap();
    assert_eq!(p.phase, "review");
    assert_eq!(p.parts[0], Part::Heading("Leftovers: qBittorrent".into()));
    let groups: Vec<&GroupView> = p.parts.iter().filter_map(|x| if let Part::Group(g) = x { Some(g) } else { None }).collect();
    assert_eq!(groups.iter().map(|g| (g.title.as_str(), g.count, g.open)).collect::<Vec<_>>(), vec![("Certain", 2, true), ("Probable", 1, false), ("Needs administrator", 1, true)]);
    assert_eq!(groups[0].size_text, " \u{b7} 3 KB");
    assert_eq!(groups[0].rows[0].meta, "Folder \u{b7} 1000 B \u{b7} install folder");
    assert!(p.parts.contains(&Part::Note(s::SAFE.1.into())));
    let Some(Part::Footer(f)) = p.parts.last() else { panic!("footer last") };
    assert_eq!(f.selected.as_deref(), Some("Selected: 2 \u{b7} 3 KB"));
    let labels: Vec<_> = f.buttons.iter().map(|b| (b.control, b.label.as_str(), b.enabled)).collect();
    assert_eq!(labels, vec![(Control::SelectAllCertain, "All certain", true), (Control::DeleteSelected, "Delete selected (2)", true), (Control::Close, "Close", true)]);
    let adm = p.parts.iter().find_map(|x| if let Part::Buttons(b) = x { Some(b) } else { None }).unwrap();
    assert_eq!(adm[0].control, Control::AdminScan);
    assert_eq!((adm[1].control, adm[1].enabled, adm[1].icon), (Control::AdminDelete, false, Some("shield")));
    assert_eq!(adm[1].label, " Remove system leftovers (administrator) (0)");
}

#[test]
fn delete_sends_only_user_ids_and_rests_the_buttons() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&review(), t);
    let o = u.press(Control::DeleteSelected, t);
    assert_eq!(calls(&o), vec![("uninstall_delete", json!({ "ids": [0, 1] }))]);
    let p = u.view().panel.unwrap();
    assert!(p.busy);
    assert!(matches!(&p.parts[0], Part::DeleteProgress { text } if text == "Deleting the selection\u{2026} 0 / ?"));
    let Some(Part::Footer(f)) = p.parts.last() else { panic!() };
    assert!(f.buttons.iter().all(|b| !b.enabled), "every button rests while deleting");
    // a second press while busy sends nothing
    assert!(cmds(&u.press(Control::DeleteSelected, t)).is_empty());
    assert!(cmds(&u.press(Control::Close, t)).is_empty(), "no Close mid-deletion");
    // the backend confirms the phase and reports items
    u.on_state(&st("deleting", json!({ "total": 2 })), t);
    assert!(u.view().panel.unwrap().busy, "the checklist stays under the overlay");
    u.on_item(&json!({ "shown": "C:\\a", "ok": true, "n": 1, "total": 2 }), t);
    u.on_item(&json!({ "shown": "C:\\b", "ok": false, "error": "in use", "n": 2, "total": 2 }), t);
    let p = u.view().panel.unwrap();
    assert!(matches!(&p.parts[0], Part::DeleteProgress { text } if text == "Deleting the selection\u{2026} 2 / 2"));
    let g = p.parts.iter().find_map(|x| if let Part::Group(g) = x { Some(g) } else { None }).unwrap();
    assert!(g.rows[0].gone);
    assert_eq!((g.rows[1].failed, g.rows[1].checked, g.rows[1].meta.as_str()), (true, false, "in use"));
    assert_eq!(g.count, 1);
}

#[test]
fn done_with_leftovers_keeps_the_checklist_under_a_summary() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&review(), t);
    u.press(Control::DeleteSelected, t);
    let done = st(
        "done",
        json!({ "items": [item(2, "C:\\c", "probable", false, 500, false), item(3, "HKLM\\d", "certain", true, 0, false)], "files": 2, "keys": 1, "other": 0, "failed": ["C:\\b: in use"], "needAdmin": 1,
            "backup": "C:\\bk\\a.reg", "log": "C:\\bk\\a.log" }),
    );
    u.on_state(&done, t);
    let p = u.view().panel.unwrap();
    assert_eq!(p.phase, "done");
    assert!(!p.busy, "a fresh page is not busy");
    let Part::DoneHead { title, line, need_admin, failed_head, failed } = &p.parts[0] else { panic!("summary first: {:?}", p.parts[0]) };
    assert_eq!((title.as_str(), line.as_str()), ("qBittorrent", "Removed 2 files and folders, 1 registry keys"));
    assert_eq!(need_admin.as_deref(), Some("Need administrator: 1"));
    assert_eq!(failed_head.as_deref(), Some("Could not delete:"));
    assert_eq!(failed, &vec!["C:\\b: in use".to_string()]);
    assert!(p.parts.contains(&Part::Extra { text: "Registry backup: C:\\bk\\a.reg".into() }));
    assert!(p.parts.iter().any(|x| matches!(x, Part::Link(b) if b.control == Control::OpenBackups)));
    let o = u.press(Control::OpenBackups, t);
    assert_eq!(cmds(&o), vec!["uninstall_open_backups"]);
}

#[test]
fn plain_done_and_the_no_leftovers_message() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("done", json!({ "items": [], "files": 0, "keys": 0, "other": 0, "failed": [], "log": "" })), t);
    let p = u.view().panel.unwrap();
    let Part::DoneHead { line, .. } = &p.parts[0] else { panic!() };
    assert_eq!(line, "No leftovers found. The app is gone.");
    assert!(!p.parts.iter().any(|x| matches!(x, Part::Link(_))), "no backup, no log: no link");
    let Some(Part::Buttons(b)) = p.parts.last() else { panic!() };
    assert!(b[0].default_focus && b[0].control == Control::Close);
    let o = u.press(Control::Close, t);
    assert_eq!(cmds(&o), vec!["uninstall_cancel"]);
}

#[test]
fn admin_steps() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&review(), t);
    let o = u.press(Control::AdminScan, t);
    assert_eq!(calls(&o), vec![("uninstall_admin", json!({ "mode": "scan", "ids": [] }))]);
    // nothing ticked on the admin side yet: the delete button does nothing
    assert!(cmds(&u.press(Control::AdminDelete, t)).is_empty());
    u.on_state(&st("elevating", json!({ "mode": "scan" })), t);
    let p = u.view().panel.unwrap();
    assert_eq!(p.parts[0], Part::Shield);
    assert!(p.parts.contains(&Part::Busy(s::ELEV_SCAN.1.into())));
    assert_eq!(u.view().panel.unwrap().phase, "elevating");
    // the elevated scan came back: the administrator scan is not offered again, and a declined prompt is explained
    let mut r = review();
    r["adminScanned"] = json!(true);
    r["adminError"] = json!("cancelled");
    u.on_state(&r, t);
    let p = u.view().panel.unwrap();
    assert!(p.parts.contains(&Part::Warn { text: "Administrator step did not run: the Windows prompt was declined".into(), shake: false }));
    assert!(p.parts.iter().filter_map(|x| if let Part::Buttons(b) = x { Some(b) } else { None }).all(|b| b.iter().all(|x| x.control != Control::AdminScan)));
    // tick the admin item, delete it through the elevated step
    u.press(Control::ToggleItem(3), t);
    let o = u.press(Control::AdminDelete, t);
    assert_eq!(calls(&o), vec![("uninstall_admin", json!({ "mode": "delete", "ids": [3] }))]);
    u.on_state(&st("elevating", json!({ "mode": "delete" })), t);
    assert!(u.view().panel.unwrap().parts.contains(&Part::Busy(s::ELEV_DEL.1.into())));
}

#[test]
fn admin_scanned_survives_a_second_review() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    let mut r = review();
    r["adminScanned"] = json!(true);
    u.on_state(&r, t);
    u.on_state(&review(), t); // a later review payload without the flag
    let p = u.view().panel.unwrap();
    assert!(!p.parts.iter().any(|x| matches!(x, Part::Buttons(b) if b.iter().any(|y| y.control == Control::AdminScan))));
}

#[test]
fn still_and_error_pages() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("still", json!({})), t);
    let p = u.view().panel.unwrap();
    assert!(p.parts.contains(&Part::Warn { text: s::STILL.1.into(), shake: false }));
    let Some(Part::Buttons(b)) = p.parts.last() else { panic!() };
    assert_eq!((b[0].control, b[0].default_focus), (Control::CheckAgain, true));
    assert_eq!(cmds(&u.press(Control::CheckAgain, t)), vec!["uninstall_scan"]);
    u.on_state(&st("error", json!({ "message": "internal error: boom" })), t);
    let p = u.view().panel.unwrap();
    assert!(p.parts.contains(&Part::Warn { text: "Failed: internal error: boom".into(), shake: true }));
}

#[test]
fn running_page_buttons_and_hide() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("running", json!({})), t);
    let p = u.view().panel.unwrap();
    let Some(Part::Buttons(b)) = p.parts.last() else { panic!() };
    assert_eq!(b.iter().map(|x| x.control).collect::<Vec<_>>(), vec![Control::CheckAgain, Control::StopWait, Control::Hide]);
    assert!(b[2].default_focus);
    assert_eq!(cmds(&u.press(Control::StopWait, t)), vec!["uninstall_cancel"]);
    assert!(u.takeover(), "stop waiting is a backend request, the panel stays");
    u.press(Control::Hide, t);
    assert!(!u.takeover() && u.has_job());
    assert_eq!(u.view().strip.unwrap().kind, StripKind::Busy);
}

#[test]
fn strip_phase_change_slides_the_label_and_pops_when_finished() {
    let mut u = ui(false);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("scanning", json!({ "found": 3 })), t);
    settle(&mut u, &mut t);
    u.on_input(t);
    settle(&mut u, &mut t);
    let o = u.on_state(&st("scanning", json!({ "found": 4 })), t);
    assert!(!has_anim(&o, AK::Pop), "same phase: only the number moves");
    let o = u.on_state(&review(), t);
    assert!(has_anim(&o, AK::Enter) && has_anim(&o, AK::Pop), "a finished step: the label slides and the strip pops");
    let o = u.on_key(&KeyEvent::new("u").with_code("KeyU").ctrl(), t).1;
    assert!(has_anim(&o, AK::Exit), "the strip leaves when the panel returns");
    assert!(u.view().strip.is_none());
}

#[test]
fn strip_texts_per_phase_and_language() {
    let mut u = UninstallUi::new(Lang::Ru, true);
    let mut t = 0.0;
    u.open(&row(), t);
    u.on_reply(ReplyTag::Info, Some(&info()), t);
    u.on_state(&st("scanning", json!({ "found": 12 })), t);
    u.on_input(t);
    t += 1.0;
    let s1 = u.view().strip.unwrap();
    assert_eq!((s1.message.as_str(), s1.count, s1.tooltip.as_str(), s1.shortcut), (": ищу остатки, найдено ", Some(12), "Открыть панель удаления (Ctrl+U)", "Ctrl+U"));
    u.on_state(&st("error", json!({ "message": "x" })), t);
    let s2 = u.view().strip.unwrap();
    assert_eq!((s2.icon, s2.message.as_str(), s2.count), (StripIcon::Warn, ": ошибка", None));
    u.set_lang(Lang::En);
    assert_eq!(u.view().strip.unwrap().message, ": failed");
}

#[test]
fn open_with_a_running_job_brings_the_existing_panel_back() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&st("scanning", json!({ "found": 2 })), t);
    u.on_input(t);
    let o = u.open(&row(), t);
    assert!(cmds(&o).is_empty(), "no second uninstall_info");
    assert!(u.takeover());
    assert_eq!(u.view().panel.unwrap().phase, "scanning");
}

#[test]
fn uninstall_events_for_the_panel_that_is_away_do_not_redraw_it() {
    let mut u = ui(true);
    let mut t = 0.0;
    confirmed(&mut u, &mut t);
    u.on_state(&review(), t);
    u.on_input(t);
    // a stray item event while away is ignored (the next state resynchronises)
    let o = u.on_item(&json!({ "shown": "C:\\a", "ok": true, "n": 1, "total": 1 }), t);
    assert!(o.is_empty());
    assert!(u.view().panel.is_none());
}

#[test]
fn state_events_without_a_known_phase_are_ignored() {
    let mut u = ui(true);
    u.on_state(&json!({ "phase": "bogus", "name": "x" }), 0.0);
    u.on_state(&json!({ "name": "x" }), 0.0);
    u.on_state(&Value::Null, 0.0);
    assert!(!u.has_job() && u.view().strip.is_none());
}

#[test]
fn parse_int_reads_the_leading_digits_of_the_row_payload() {
    assert_eq!(parse_int("12"), Some(12));
    assert_eq!(parse_int("7!"), Some(7));
    assert_eq!(parse_int(" 5"), Some(5));
    assert_eq!(parse_int("x"), None);
    assert_eq!(parse_int(""), None);
}
