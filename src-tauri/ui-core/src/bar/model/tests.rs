//! Behaviour tests of the bar model: query handling and debounce, selection after list updates, the keyboard model, run outcomes, the context
//! menu, AI answers (streaming accumulation, dialogue), dictation, lifecycle (`Shown` / `Hidden`, blur guard interplay) and the uninstall panel
//! inside the bar. All backend traffic is faked; nothing here runs a real command.

use super::*;
use crate::bar::anim::Kind as AK;
use crate::bar::uninstall::state::Phase;
use serde_json::json;

struct H {
    b: Bar,
    t: f64,
}

type Call = (Option<u64>, &'static str, Value);

fn calls(a: &[Action]) -> Vec<Call> {
    a.iter().filter_map(|x| if let Action::Call { ticket, call } = x { Some((*ticket, call.cmd, call.args.clone())) } else { None }).collect()
}
fn cmds(a: &[Action]) -> Vec<&'static str> {
    calls(a).into_iter().map(|c| c.1).collect()
}
fn ticket(a: &[Action], cmd: &str) -> u64 {
    calls(a).into_iter().find(|c| c.1 == cmd).and_then(|c| c.0).unwrap_or_else(|| panic!("no ticketed {cmd} in {a:?}"))
}
fn has_anim(a: &[Action], k: AK) -> bool {
    a.iter().any(|x| matches!(x, Action::Anim(an) if an.kind == k))
}
fn any_anim(a: &[Action]) -> bool {
    a.iter().any(|x| matches!(x, Action::Anim(_)))
}
fn copied(a: &[Action]) -> Vec<String> {
    a.iter().filter_map(|x| if let Action::Copy(t) = x { Some(t.clone()) } else { None }).collect()
}

fn res(kind: &str, id: &str, title: &str, sub: &str, payload: &str) -> Value {
    json!({ "id": id, "kind": kind, "title": title, "subtitle": sub, "payload": payload })
}
fn app(n: usize) -> Value {
    res("app", &format!("app:{n}"), &format!("App {n}"), "Application", &format!("C:\\Apps\\app{n}.lnk"))
}

impl H {
    fn new() -> H {
        H { b: Bar::new("en-US"), t: 1000.0 }
    }
    fn ev(&mut self, e: BarEvent) -> Vec<Action> {
        self.t += 1.0;
        self.b.on_event(e, self.t)
    }
    fn tick(&mut self, ms: f64) -> Vec<Action> {
        self.t += ms;
        self.b.tick(self.t)
    }
    fn key(&mut self, k: KeyEvent) -> Vec<Action> {
        self.ev(BarEvent::Key(k))
    }
    fn reply(&mut self, ticket: u64, v: Value) -> Vec<Action> {
        self.ev(BarEvent::Reply { ticket, value: Some(v) })
    }
    fn fail(&mut self, ticket: u64) -> Vec<Action> {
        self.ev(BarEvent::Reply { ticket, value: None })
    }
    fn type_text(&mut self, text: &str) -> Vec<Action> {
        let n = text.chars().count();
        self.ev(BarEvent::Input { text: text.into(), caret: n })
    }
    /// Types, waits out the debounce and answers the resulting query with `rows`.
    fn search(&mut self, text: &str, rows: Vec<Value>) -> Vec<Action> {
        self.type_text(text);
        let a = self.tick(80.0);
        let t = ticket(&a, "cmd_query");
        self.reply(t, Value::Array(rows))
    }
}

/// Opens the bar the way the host does and answers its start-up calls.
fn opened(h: &mut H, cfg: Value, rows: Vec<Value>) {
    let a = h.ev(BarEvent::Shown);
    let ts = ticket(&a, "settings_get");
    let tq = ticket(&a, "cmd_query");
    let tu = ticket(&a, "uninstall_state");
    h.reply(ts, cfg);
    h.reply(tq, Value::Array(rows));
    h.reply(tu, Value::Null);
}
fn open_default(h: &mut H) {
    opened(h, json!({ "language": "en" }), vec![]);
}
fn ai_cfg() -> Value {
    json!({ "language": "en", "net.ai": true, "ai.baseUrl": "http://localhost:11434", "ai.model": "qwen" })
}

// ---------------------------------------------------------------- lifecycle

#[test]
fn shown_resets_and_asks_the_backend_for_everything() {
    let mut h = H::new();
    let a = h.ev(BarEvent::Shown);
    assert_eq!(cmds(&a), vec!["settings_get", "cmd_query", "uninstall_state"]);
    assert_eq!(calls(&a)[1].2, json!({ "text": "" }));
    assert!(a.contains(&Action::SetInput { text: String::new(), caret: 0 }) && a.contains(&Action::FocusInput));
    assert!(has_anim(&a, AK::Enter), "the bar pops in");
    let v = h.b.view();
    assert!(v.visible && !v.parked);
    assert_eq!(v.placeholder, s::PH.1);
    assert_eq!(v.hotkey, "Alt+Shift+Space");
}

#[test]
fn hidden_clears_everything_and_cancels_a_pending_ask() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![app(1)]);
    h.type_text("? why");
    h.tick(80.0);
    let a = h.key(KeyEvent::new("Enter"));
    assert_eq!(cmds(&a), vec!["cmd_ask"]);
    let a = h.ev(BarEvent::Hidden);
    assert_eq!(cmds(&a), vec!["ai_transform_cancel"], "an ask in flight is cancelled when the bar closes");
    let v = h.b.view();
    assert!(!v.visible && v.parked);
    assert!(v.rows.is_empty() && v.answer.is_none() && v.input.is_empty() && v.context.is_none() && v.message.is_none());
    assert!(!h.b.dialog_open());
}

#[test]
fn a_reply_that_arrives_after_hide_is_dropped() {
    let mut h = H::new();
    let a = h.ev(BarEvent::Shown);
    let tq = ticket(&a, "cmd_query");
    h.ev(BarEvent::Hidden);
    h.reply(tq, json!([app(1)]));
    assert!(h.b.view().rows.is_empty(), "the query was for a window that is gone");
}

#[test]
fn a_late_bar_shown_replay_just_resets_again() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("hello", vec![app(1)]);
    let a = h.ev(BarEvent::Shown);
    assert_eq!(h.b.view().input, "");
    assert_eq!(cmds(&a)[0], "settings_get");
    assert_eq!(cmds(&a)[1], "cmd_query", "and the query runs again");
}

#[test]
fn settings_choose_language_theme_and_motion() {
    let mut h = H::new();
    opened(&mut h, json!({ "language": "ru", "theme": "light", "reduceMotion": true, "hotkey.command": "Ctrl+Space" }), vec![]);
    let v = h.b.view();
    assert_eq!(v.placeholder, s::PH.0);
    assert_eq!((v.theme.as_str(), v.hotkey.as_str(), v.reduce_motion), ("light", "Ctrl+Space", true));
    assert_eq!(h.b.lang(), Lang::Ru);
    // a null answer (failed invoke) keeps what was loaded
    let a = h.ev(BarEvent::Shown);
    let ts = ticket(&a, "settings_get");
    h.fail(ts);
    assert_eq!(h.b.lang(), Lang::Ru);
}

#[test]
fn os_language_is_used_until_settings_answer() {
    let mut h = H { b: Bar::new("ru-RU"), t: 0.0 };
    assert_eq!(h.b.view().placeholder, s::PH.0);
    let a = h.ev(BarEvent::Shown);
    let ts = ticket(&a, "settings_get");
    h.reply(ts, json!({ "language": "en" }));
    assert_eq!(h.b.view().placeholder, s::PH.1, "the setting wins over the OS language");
    h.reply(ts, json!({ "language": "ru" })); // an answered ticket is spent: ignored
    assert_eq!(h.b.lang(), Lang::En);
}

// ---------------------------------------------------------------- typing, debounce, stale replies

#[test]
fn typing_is_debounced_by_80_ms_and_only_the_last_query_runs() {
    let mut h = H::new();
    open_default(&mut h);
    h.type_text("n");
    assert!(cmds(&h.tick(40.0)).is_empty());
    h.type_text("no");
    assert!(cmds(&h.tick(79.0)).is_empty(), "the second keystroke restarted the wait");
    let a = h.tick(2.0);
    assert_eq!(calls(&a).len(), 1);
    assert_eq!(calls(&a)[0].1, "cmd_query");
    assert_eq!(calls(&a)[0].2, json!({ "text": "no" }));
}

#[test]
fn leading_blanks_go_and_a_trailing_one_is_kept() {
    let mut h = H::new();
    open_default(&mut h);
    h.type_text("   c ");
    let a = h.tick(80.0);
    assert_eq!(calls(&a)[0].2, json!({ "text": "c " }), "`c ` already selects the clipboard filter");
}

#[test]
fn a_stale_query_answer_is_dropped() {
    let mut h = H::new();
    open_default(&mut h);
    h.type_text("a");
    let a1 = h.tick(80.0);
    let t1 = ticket(&a1, "cmd_query");
    h.type_text("ab");
    let a2 = h.tick(80.0);
    let t2 = ticket(&a2, "cmd_query");
    h.reply(t2, json!([app(2)]));
    h.reply(t1, json!([app(1)]));
    let v = h.b.view();
    assert_eq!(v.rows.len(), 1);
    assert_eq!(v.rows[0].id, "app:2", "the older answer arrived last and lost");
}

#[test]
fn a_failed_query_leaves_the_list_alone() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("x", vec![app(1)]);
    h.type_text("xy");
    let a = h.tick(80.0);
    h.fail(ticket(&a, "cmd_query"));
    assert_eq!(h.b.view().rows.len(), 1);
}

#[test]
fn cyrillic_queries_highlight_cyrillic_titles() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("n заметки про лето", vec![res("note", "note:1", "Заметки про Лето и осень", "…", "1")]);
    let v = h.b.view();
    let marks: Vec<&str> = v.rows[0].title.iter().filter(|s| s.mark).map(|s| s.text.as_str()).collect();
    assert_eq!(marks, vec!["Заметки", "про", "Лето"]);
    assert_eq!(v.rows[0].label, "Notes");
}

#[test]
fn shell_and_calc_rows_are_not_highlighted() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("> dir", vec![res("shell", "shell:dir", "Run: dir", "cmd /c, hidden window", "dir")]);
    let r = &h.b.view().rows[0];
    assert!(r.title.iter().all(|s| !s.mark));
    assert_eq!(r.title[0].text, "Run: dir");
}

#[test]
fn empty_results_show_the_tip_only_for_an_empty_query() {
    let mut h = H::new();
    let a = h.ev(BarEvent::Shown);
    let tq = ticket(&a, "cmd_query");
    h.reply(tq, json!([]));
    assert_eq!(h.b.view().tip.as_deref(), Some(s::TIP.1));
    h.search("zzz", vec![]);
    assert_eq!(h.b.view().tip, None, "a query that finds nothing shows no tip");
}

#[test]
fn recent_items_of_an_empty_box_get_a_section_label() {
    let mut h = H::new();
    let a = h.ev(BarEvent::Shown);
    h.reply(ticket(&a, "cmd_query"), json!([app(1), app(2)]));
    assert_eq!(h.b.view().recent_label.as_deref(), Some("Recent"));
    assert_eq!(h.b.view().tip, None);
    h.search("a", vec![app(1)]);
    assert_eq!(h.b.view().recent_label, None, "a typed query is not the recent list");
    h.ev(BarEvent::Hidden);
    assert_eq!(h.b.view().recent_label, None);
}

#[test]
fn hint_rows_are_listed_separately_and_selection_ignores_them() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("f x", vec![app(1), res("hint", "hint:es", "Files need Everything", "Opens your default browser", "")]);
    let v = h.b.view();
    assert_eq!(v.rows.len(), 1);
    assert_eq!(v.hints, vec!["Files need Everything - Opens your default browser".to_string()]);
}

// ---------------------------------------------------------------- selection

fn many(h: &mut H) {
    open_default(h);
    h.search("a", vec![res("calc", "calc:1", "= 1", "x  -  Enter copies", "1"), app(1), app(2), app(3), res("folder", "folder:1", "F", "C:\\F", "C:\\F"), res("file", "file:1", "f", "C:\\f", "C:\\f"), res("web", "web:a", "Search Google for \"a\"", "", "u")]);
}

#[test]
fn selection_resets_to_the_first_row_after_every_list_update() {
    let mut h = H::new();
    many(&mut h);
    h.key(KeyEvent::new("ArrowDown"));
    h.key(KeyEvent::new("ArrowDown"));
    assert_eq!(h.b.selected(), 2);
    h.search("ab", vec![app(9), app(8)]);
    assert_eq!(h.b.selected(), 0);
    assert_eq!(h.b.view().selection_from, None, "a fresh render snaps the highlight");
    assert!(h.b.view().rows[0].selected);
}

#[test]
fn arrows_wrap_and_the_highlight_glides() {
    let mut h = H::new();
    many(&mut h);
    let a = h.key(KeyEvent::new("ArrowUp"));
    assert_eq!(h.b.selected(), 6, "up from the first row wraps to the last");
    assert!(has_anim(&a, AK::Glide));
    assert_eq!(h.b.view().selection_from, Some(0));
    assert!(a.contains(&Action::ScrollToRow(6)));
    h.key(KeyEvent::new("ArrowDown"));
    assert_eq!(h.b.selected(), 0);
}

#[test]
fn page_keys_clamp_and_ctrl_home_end_jump() {
    let mut h = H::new();
    many(&mut h);
    h.key(KeyEvent::new("PageDown"));
    assert_eq!(h.b.selected(), 6);
    h.key(KeyEvent::new("PageDown"));
    assert_eq!(h.b.selected(), 6, "no wrap on pages");
    h.key(KeyEvent::new("PageUp"));
    assert_eq!(h.b.selected(), 0);
    h.key(KeyEvent::new("End").ctrl());
    assert_eq!(h.b.selected(), 6);
    h.key(KeyEvent::new("Home").ctrl());
    assert_eq!(h.b.selected(), 0);
    h.key(KeyEvent::new("Home"));
    assert_eq!(h.b.selected(), 0, "plain Home is the text field's");
}

#[test]
fn tab_walks_the_groups_and_shift_tab_goes_back() {
    let mut h = H::new();
    many(&mut h);
    h.key(KeyEvent::new("Tab"));
    assert_eq!(h.b.selected(), 1, "Result -> Applications");
    h.key(KeyEvent::new("Tab"));
    assert_eq!(h.b.selected(), 4, "Applications -> Folders");
    h.key(KeyEvent::new("Tab").shift());
    assert_eq!(h.b.selected(), 1);
}

#[test]
fn hover_selects_without_wrapping_and_empty_lists_ignore_keys() {
    let mut h = H::new();
    many(&mut h);
    h.ev(BarEvent::RowHover(3));
    assert_eq!(h.b.selected(), 3);
    h.ev(BarEvent::RowHover(99));
    assert_eq!(h.b.selected(), 3);
    let mut e = H::new();
    open_default(&mut e);
    for k in ["ArrowDown", "ArrowUp", "PageDown", "Tab"] {
        let a = e.key(KeyEvent::new(k));
        assert!(!a.iter().any(|x| matches!(x, Action::ScrollToRow(_))), "{k}");
    }
    assert!(cmds(&e.key(KeyEvent::new("Enter"))).is_empty(), "Enter on nothing runs nothing");
}

#[test]
fn reduced_motion_selection_does_not_animate() {
    let mut h = H::new();
    opened(&mut h, json!({ "language": "en", "reduceMotion": true }), vec![]);
    h.search("a", vec![app(1), app(2)]);
    let a = h.key(KeyEvent::new("ArrowDown"));
    assert_eq!(h.b.selected(), 1);
    assert!(!any_anim(&a));
    assert_eq!(h.b.view().selection_from, None);
}

// ---------------------------------------------------------------- list animations

#[test]
fn first_render_staggers_rows_and_later_ones_diff() {
    let mut h = H::new();
    let a = h.ev(BarEvent::Shown);
    let tq = ticket(&a, "cmd_query");
    let a = h.reply(tq, json!([app(1), app(2), app(3)]));
    let enters: Vec<u32> = a.iter().filter_map(|x| if let Action::Anim(an) = x { Some(an.delay_ms) } else { None }).collect();
    assert_eq!(enters, vec![0, 28, 56], "the stagger plays once per opening");
    let a = h.search("a", vec![app(2), app(4)]);
    assert!(has_anim(&a, AK::Slide), "the survivor slides");
    assert!(has_anim(&a, AK::Exit), "gone rows fade out");
    let ins = a.iter().filter(|x| matches!(x, Action::Anim(an) if an.kind == AK::Enter && an.delay_ms == 0)).count();
    assert_eq!(ins, 1, "the new row drops in without a stagger");
}

// ---------------------------------------------------------------- icons

#[test]
fn shell_icons_are_requested_once_per_row_id() {
    let mut h = H::new();
    open_default(&mut h);
    let a = h.search("a", vec![app(1), res("calc", "calc:1", "= 1", "", "1"), app(2)]);
    let icons: Vec<_> = calls(&a).into_iter().filter(|c| c.1 == "cmd_icon").collect();
    assert_eq!(icons.len(), 2, "only app / file / folder rows ask");
    assert_eq!(icons[0].2, json!({ "id": "app:1" }));
    let t = icons[0].0.unwrap();
    h.reply(t, json!("data:image/png;base64,AAAA"));
    assert_eq!(h.b.view().rows[0].shell_icon.as_deref(), Some("data:image/png;base64,AAAA"));
    assert_eq!(h.b.view().rows[1].shell_icon, None);
    let a = h.search("ab", vec![app(1), app(3)]);
    let again: Vec<_> = calls(&a).into_iter().filter(|c| c.1 == "cmd_icon").collect();
    assert_eq!(again.len(), 1, "app:1 is cached, only app:3 asks");
    assert_eq!(again[0].2, json!({ "id": "app:3" }));
}

#[test]
fn an_icon_the_backend_does_not_have_leaves_the_glyph() {
    let mut h = H::new();
    open_default(&mut h);
    let a = h.search("a", vec![app(1)]);
    h.reply(ticket(&a, "cmd_icon"), Value::Null);
    assert_eq!(h.b.view().rows[0].shell_icon, None);
    assert_eq!(h.b.view().rows[0].icon, "apps");
}

// ---------------------------------------------------------------- running rows

#[test]
fn enter_variants_send_the_right_mode() {
    let mut h = H::new();
    many(&mut h);
    h.ev(BarEvent::RowHover(1)); // an app row
    let run = |h: &mut H, k: KeyEvent| calls(&h.key(k)).into_iter().find(|c| c.1 == "cmd_run").map(|c| c.2["mode"].as_str().unwrap().to_string());
    assert_eq!(run(&mut h, KeyEvent::new("Enter")).as_deref(), Some("open"));
    assert_eq!(run(&mut h, KeyEvent::new("Enter").ctrl()).as_deref(), Some("folder"));
    assert_eq!(run(&mut h, KeyEvent::new("Enter").ctrl().shift()).as_deref(), Some("admin"));
    assert_eq!(run(&mut h, KeyEvent::new("Enter").shift()).as_deref(), Some("copy"), "sent as is; the backend treats an unknown mode as open");
    assert_eq!(run(&mut h, KeyEvent::new("C").with_code("KeyC").ctrl().shift()).as_deref(), Some("path"));
    assert_eq!(run(&mut h, KeyEvent::new("p").with_code("KeyP").ctrl()).as_deref(), Some("pin"));
    assert_eq!(calls(&h.key(KeyEvent::new("Enter")))[0].2["id"], "app:1");
}

#[test]
fn modes_only_mean_something_for_their_row_kinds() {
    let mut h = H::new();
    many(&mut h);
    h.ev(BarEvent::RowHover(6)); // a web row
    let a = h.key(KeyEvent::new("Enter").ctrl());
    assert_eq!(calls(&a)[0].2["mode"], "open", "reveal / admin are for files");
    assert!(cmds(&h.key(KeyEvent::new("p").with_code("KeyP").ctrl())).is_empty(), "web rows cannot be pinned");
    h.ev(BarEvent::RowHover(4)); // a folder
    assert_eq!(calls(&h.key(KeyEvent::new("Enter").ctrl()))[0].2["mode"], "folder");
    assert_eq!(calls(&h.key(KeyEvent::new("p").with_code("KeyP").ctrl()))[0].2["mode"], "pin", "folders pin");
}

#[test]
fn clipboard_rows_copy_instead_of_copying_a_path() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("c x", vec![res("clip", "clip:5", "some text", "now", "5")]);
    let a = h.key(KeyEvent::new("C").with_code("KeyC").ctrl().shift());
    assert_eq!(calls(&a)[0].2, json!({ "id": "clip:5", "mode": "copy" }), "the path of a clipboard item is its plain copy");
    let a = h.key(KeyEvent::new("Enter").shift());
    assert_eq!(calls(&a)[0].2["mode"], "copy");
}

#[test]
fn calc_rows_copy_before_the_window_hides() {
    let mut h = H::new();
    many(&mut h);
    let a = h.key(KeyEvent::new("Enter"));
    assert_eq!(copied(&a), vec!["1".to_string()]);
    assert_eq!(calls(&a)[0].2, json!({ "id": "calc:1", "mode": "open" }));
    let Action::Copy(_) = a[0] else { panic!("the copy comes before the call: {a:?}") };
}

#[test]
fn the_confirm_outcome_arms_the_row_for_three_seconds() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("lock", vec![res("system", "sys:lock", "Lock", "Press Enter twice to confirm", "lock")]);
    let a = h.key(KeyEvent::new("Enter"));
    let t = ticket(&a, "cmd_run");
    h.reply(t, json!({ "status": "confirm", "message": "Press Enter again within 3 s" }));
    let r = &h.b.view().rows[0];
    assert!(r.confirm);
    assert_eq!(r.label, s::AGAIN.1);
    h.tick(2999.0);
    assert!(h.b.view().rows[0].confirm);
    h.tick(2.0);
    let r = &h.b.view().rows[0];
    assert!(!r.confirm);
    assert_eq!(r.label, "Pill", "the label is back");
}

#[test]
fn copy_outcome_copies_and_closes_the_bar() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("c x", vec![res("clip", "clip:5", "t", "n", "5")]);
    let t = ticket(&h.key(KeyEvent::new("Enter").shift()), "cmd_run");
    let a = h.reply(t, json!({ "status": "copy", "message": "the clipboard text" }));
    assert_eq!(copied(&a), vec!["the clipboard text".to_string()]);
    assert_eq!(cmds(&a), vec!["cmd_toggle"]);
}

#[test]
fn updated_outcome_changes_the_subtitle_or_the_message() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("s dark", vec![res("setting", "set:dark", "Dark mode", "Off", "dark")]);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_run");
    h.reply(t, json!({ "status": "updated", "message": "On" }));
    let v = h.b.view();
    assert_eq!(v.rows[0].sub, "On");
    assert!(v.message.is_none(), "the row has a subtitle: it shows the state");
    // a row without a subtitle puts the state in the message line
    h.search("act", vec![res("action", "act:x", "Do it", "", "x")]);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_run");
    h.reply(t, json!({ "status": "updated", "message": "Done" }));
    assert_eq!(h.b.view().message.unwrap().text, "Done");
}

#[test]
fn error_outcome_shows_an_error_message() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1)]);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_run");
    let a = h.reply(t, json!({ "status": "error", "message": "Path no longer exists" }));
    let m = h.b.view().message.unwrap();
    assert_eq!((m.text.as_str(), m.ok, m.link.clone()), ("Path no longer exists", false, None));
    assert!(has_anim(&a, AK::Enter));
    // a new query clears it
    h.search("ab", vec![app(1)]);
    assert!(h.b.view().message.is_none());
}

#[test]
fn ok_outcome_does_nothing_the_host_hides_the_window() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1)]);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_run");
    let a = h.reply(t, json!({ "status": "ok", "message": "" }));
    assert!(a.is_empty());
}

#[test]
fn pinning_confirms_with_a_pop_a_flash_and_a_message_that_fades() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1)]);
    let t = ticket(&h.key(KeyEvent::new("p").with_code("KeyP").ctrl()), "cmd_run");
    let a = h.reply(t, json!({ "status": "updated", "message": "pinned" }));
    let m = h.b.view().message.unwrap();
    assert_eq!((m.text.as_str(), m.ok), ("Pinned to Pill", true));
    assert!(has_anim(&a, AK::Pop) && has_anim(&a, AK::Flash) && has_anim(&a, AK::Enter));
    h.tick(2599.0);
    assert!(h.b.view().message.is_some());
    let a = h.tick(2.0);
    assert!(has_anim(&a, AK::Exit), "the message slides out");
    h.tick(230.0);
    assert!(h.b.view().message.is_none());
    // already pinned
    let t = ticket(&h.key(KeyEvent::new("p").with_code("KeyP").ctrl()), "cmd_run");
    h.reply(t, json!({ "status": "updated", "message": "pinnedAlready" }));
    assert_eq!(h.b.view().message.unwrap().text, "Already pinned");
}

#[test]
fn pin_feedback_text_follows_the_language() {
    let mut h = H::new();
    opened(&mut h, json!({ "language": "ru" }), vec![]);
    h.search("a", vec![app(1)]);
    let t = ticket(&h.key(KeyEvent::new("з").with_code("KeyP").ctrl()), "cmd_run");
    h.reply(t, json!({ "status": "updated", "message": "pinned" }));
    assert_eq!(h.b.view().message.unwrap().text, "Закреплено в Pill");
}

// ---------------------------------------------------------------- context menu

#[test]
fn right_click_opens_the_row_menu_inside_the_window() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1), res("calc", "calc:1", "= 1", "", "1")]);
    let a = h.ev(BarEvent::RowContext { index: 0, x: 500.0, y: 300.0 });
    let c = h.b.view().context.unwrap();
    assert_eq!(c.row, 0);
    assert_eq!(c.lines.iter().map(|l| l.mode).collect::<Vec<_>>(), vec![RunMode::Open, RunMode::Folder, RunMode::Path, RunMode::Admin, RunMode::Pin, RunMode::Uninstall]);
    assert!(c.x + 250.0 <= WIN_W - 6.0 + 0.01 && c.y + 6.0 * 34.0 + 12.0 <= WIN_H - 6.0 + 0.01, "kept inside the window: {c:?}");
    assert!(has_anim(&a, AK::Enter));
    // a non-path row has no menu and closes an open one
    h.ev(BarEvent::RowContext { index: 1, x: 10.0, y: 10.0 });
    assert!(h.b.view().context.is_none());
}

#[test]
fn context_lines_run_with_their_mode_and_the_menu_closes() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1)]);
    h.ev(BarEvent::RowContext { index: 0, x: 20.0, y: 20.0 });
    let a = h.ev(BarEvent::ContextPick(RunMode::Folder));
    assert_eq!(calls(&a)[0].2, json!({ "id": "app:1", "mode": "folder" }));
    assert!(h.b.view().context.is_none());
}

#[test]
fn the_delete_line_of_the_menu_puts_the_uninstall_trigger_in_the_box() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("vs", vec![res("app", "app:vs", "Visual   Studio Code", "Application", "C:\\x.lnk")]);
    h.ev(BarEvent::RowContext { index: 0, x: 20.0, y: 20.0 });
    let a = h.ev(BarEvent::ContextPick(RunMode::Uninstall));
    assert!(a.contains(&Action::SetInput { text: "u Visual Studio Code".into(), caret: 20 }));
    assert_eq!(h.b.input(), "u Visual Studio Code");
    assert_eq!(calls(&a)[0].2, json!({ "text": "u Visual Studio Code" }), "the search runs at once, nothing is uninstalled");
    assert!(!cmds(&a).iter().any(|c| c.starts_with("uninstall")));
}

#[test]
fn esc_closes_the_menu_first_then_the_bar() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1)]);
    h.ev(BarEvent::RowContext { index: 0, x: 20.0, y: 20.0 });
    assert!(cmds(&h.key(KeyEvent::new("Escape"))).is_empty());
    assert!(h.b.view().context.is_none());
    assert_eq!(cmds(&h.key(KeyEvent::new("Escape"))), vec!["cmd_toggle"]);
}

#[test]
fn clicks_outside_close_the_menu_and_the_bar() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1)]);
    h.ev(BarEvent::RowContext { index: 0, x: 20.0, y: 20.0 });
    assert!(cmds(&h.ev(BarEvent::MouseDown { on_context: true, on_body: false })).is_empty());
    assert!(h.b.view().context.is_some(), "a click inside the menu keeps it for its own handler");
    assert!(cmds(&h.ev(BarEvent::MouseDown { on_context: false, on_body: true })).is_empty());
    assert!(h.b.view().context.is_none());
    assert_eq!(cmds(&h.ev(BarEvent::MouseDown { on_context: false, on_body: false })), vec!["cmd_toggle"], "the transparent margin counts as outside");
    h.ev(BarEvent::RowContext { index: 0, x: 20.0, y: 20.0 });
    h.ev(BarEvent::BackgroundContext);
    assert!(h.b.view().context.is_none());
}

#[test]
fn row_buttons_and_clicks_run_with_their_mode() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![app(1), app(2)]);
    let a = h.ev(BarEvent::RowClick { index: 1, button: Some(RunMode::Admin), ctrl: false, shift: false });
    assert_eq!(calls(&a)[0].2, json!({ "id": "app:2", "mode": "admin" }));
    let a = h.ev(BarEvent::RowClick { index: 0, button: None, ctrl: true, shift: false });
    assert_eq!(calls(&a)[0].2, json!({ "id": "app:1", "mode": "folder" }));
    let a = h.ev(BarEvent::RowClick { index: 7, button: None, ctrl: false, shift: false });
    assert!(a.is_empty(), "a click on a row that is gone does nothing");
}

#[test]
fn the_window_focus_returns_the_caret_and_the_message_link_opens_settings() {
    let mut h = H::new();
    open_default(&mut h);
    assert_eq!(h.ev(BarEvent::Focus), vec![Action::FocusInput]);
    h.type_text("? x");
    h.tick(80.0);
    h.key(KeyEvent::new("Enter")); // AI is off in this config
    let a = h.ev(BarEvent::MessageClick);
    assert!(a.is_empty(), "no link on that message");
}

// ---------------------------------------------------------------- AI

#[test]
fn ask_prefix_shows_the_ask_row_without_asking_the_backend() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? why is the sky blue");
    let a = h.tick(80.0);
    assert!(cmds(&a).is_empty(), "no cmd_query for a question");
    let v = h.b.view();
    assert_eq!(v.rows.len(), 1);
    assert_eq!((v.rows[0].kind.clone(), v.rows[0].title[0].text.as_str(), v.rows[0].sub.as_str()), (Kind::Ask, "Ask AI: why is the sky blue", s::ASK_GO.1));
    h.type_text("ии ");
    h.tick(80.0);
    let v = h.b.view();
    assert!(v.rows.is_empty());
    assert_eq!(v.tip.as_deref(), Some(s::ASK_TIP.1), "the prefix alone shows how to ask");
}

#[test]
fn ask_is_opt_in_enter_opens_the_settings_page_when_ai_is_off() {
    let mut h = H::new();
    opened(&mut h, json!({ "language": "en", "net.ai": false, "ai.baseUrl": "x", "ai.model": "m" }), vec![]);
    h.type_text("? hi");
    h.tick(80.0);
    assert_eq!(h.b.view().rows[0].sub, s::ASK_OFF.1);
    let a = h.key(KeyEvent::new("Enter"));
    assert_eq!(calls(&a), vec![(None, "settings_open", json!({ "page": "ai" }))]);
    assert!(h.b.view().answer.is_none(), "nothing was sent anywhere");
}

#[test]
fn ask_status_names_the_missing_piece() {
    let mut h = H::new();
    opened(&mut h, json!({ "language": "en", "net.ai": true, "ai.baseUrl": "", "ai.model": "m" }), vec![]);
    h.type_text("ai hi");
    h.tick(80.0);
    assert_eq!(h.b.view().rows[0].sub, s::ASK_NO_URL.1);
}

#[test]
fn a_streamed_answer_accumulates_then_the_final_text_replaces_it() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? hello");
    h.tick(80.0);
    let a = h.key(KeyEvent::new("Enter"));
    let c = &calls(&a)[0];
    assert_eq!((c.1, c.2.clone()), ("cmd_ask", json!({ "question": "hello" })));
    let t = c.0.unwrap();
    let v = h.b.view();
    assert!(v.rows.is_empty(), "the list steps away while the model thinks");
    let ans = v.answer.unwrap();
    assert!(ans.thinking && ans.thinking_label == "Thinking…" && ans.text.is_empty());
    // chunks accumulate
    h.ev(BarEvent::AskChunk { id: 1, delta: "Hel".into() });
    h.ev(BarEvent::AskChunk { id: 1, delta: "lo ".into() });
    h.ev(BarEvent::AskChunk { id: 99, delta: "IGNORED".into() });
    h.ev(BarEvent::AskChunk { id: 1, delta: "there".into() });
    assert_eq!(h.b.view().answer.unwrap().text, "Hello there");
    assert!(h.b.view().answer.unwrap().thinking);
    // the final reply
    h.reply(t, json!({ "ok": true, "text": "Hello there!" }));
    let v = h.b.view();
    let ans = v.answer.unwrap();
    assert!(!ans.thinking);
    assert_eq!(ans.text, "Hello there!");
    assert_eq!(v.input, "", "the box is cleared for the follow-up");
    assert!(v.dialog_hint.is_some());
    assert_eq!(v.placeholder, s::DLG_PH.1);
    assert_eq!(v.rows.iter().map(|r| r.kind.clone()).collect::<Vec<_>>(), vec![Kind::AskCopy, Kind::NewConv]);
    assert_eq!(v.rows[0].title[0].text, "Copy the answer");
}

#[test]
fn a_follow_up_sends_the_history_and_keeps_the_dialogue() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": true, "text": "answer one" }));
    // typing now is a follow-up: no `? ` needed
    h.type_text("and two");
    let a = h.tick(80.0);
    assert!(cmds(&a).is_empty(), "no cmd_query while a dialogue is open");
    let v = h.b.view();
    assert_eq!(v.rows.iter().map(|r| r.kind.clone()).collect::<Vec<_>>(), vec![Kind::Ask, Kind::AskCopy, Kind::NewConv]);
    assert_eq!(v.answer.unwrap().text, "answer one", "the previous answer stays");
    let a = h.key(KeyEvent::new("Enter"));
    let c = &calls(&a)[0];
    assert_eq!(c.2, json!({ "question": "and two", "history": [{ "role": "user", "content": "one" }, { "role": "assistant", "content": "answer one" }] }));
    h.reply(c.0.unwrap(), json!({ "ok": true, "text": "answer two" }));
    assert_eq!(h.b.view().answer.unwrap().text, "answer two");
    assert!(h.b.dialog_open());
}

#[test]
fn a_new_keystroke_abandons_a_pending_follow_up_and_the_old_answer_returns() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": true, "text": "first" }));
    h.type_text("two");
    h.tick(80.0);
    let t2 = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    assert!(h.b.view().answer.unwrap().thinking);
    h.type_text("twox");
    let a = h.tick(80.0);
    assert_eq!(cmds(&a), vec!["ai_transform_cancel"]);
    let ans = h.b.view().answer.unwrap();
    assert!(!ans.thinking && ans.text == "first");
    // the abandoned request's late reply is ignored
    h.reply(t2, json!({ "ok": true, "text": "late" }));
    assert_eq!(h.b.view().answer.unwrap().text, "first");
}

#[test]
fn leading_gt_or_eq_ends_the_dialogue_and_searches() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": true, "text": "first" }));
    h.type_text("> dir");
    let a = h.tick(80.0);
    assert_eq!(calls(&a)[0].2, json!({ "text": "> dir" }));
    assert!(!h.b.dialog_open() && h.b.view().answer.is_none());
}

#[test]
fn ask_errors_are_worded_and_keep_the_question_row() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": false, "error": "no_model" }));
    let v = h.b.view();
    let m = v.message.unwrap();
    assert_eq!((m.text.as_str(), m.link.as_deref()), (s::ASK_NO_MODEL.1, Some("ai")));
    assert_eq!(v.rows.len(), 1, "the ask row is back so Enter can retry");
    assert!(v.answer.is_none());
    // the link opens the settings page
    let a = h.ev(BarEvent::MessageClick);
    assert_eq!(calls(&a), vec![(None, "settings_open", json!({ "page": "ai" }))]);
    // a plain error shows verbatim, cut to 300 chars
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": false, "error": "x".repeat(400) }));
    assert_eq!(h.b.view().message.unwrap().text.len(), 300);
    // cancelled is silent
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": false, "error": "cancelled" }));
    assert!(h.b.view().message.is_none());
}

#[test]
fn a_failed_follow_up_keeps_the_dialogue_and_its_answer() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": true, "text": "first" }));
    h.type_text("two");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.fail(t);
    let v = h.b.view();
    assert_eq!(v.message.unwrap().text, s::ASK_FAIL.1);
    assert_eq!(v.answer.unwrap().text, "first");
    assert_eq!(v.rows.iter().map(|r| r.kind.clone()).collect::<Vec<_>>(), vec![Kind::Ask, Kind::AskCopy, Kind::NewConv]);
    assert!(h.b.dialog_open());
}

#[test]
fn copy_the_answer_copies_plain_text_and_the_label_flips_for_a_moment() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": true, "text": "**Bold** answer\n\n- a\n- b" }));
    let a = h.ev(BarEvent::RowClick { index: 0, button: None, ctrl: false, shift: false });
    assert_eq!(copied(&a), vec!["Bold answer\n\n- a\n- b".to_string()]);
    assert_eq!(h.b.view().rows[0].label, "Copied");
    h.tick(1801.0);
    assert_eq!(h.b.view().rows[0].label, "Copy");
}

#[test]
fn new_conversation_forgets_the_history_and_searches_as_usual() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": true, "text": "first" }));
    let a = h.ev(BarEvent::RowClick { index: 1, button: None, ctrl: false, shift: false });
    assert_eq!(calls(&a)[0].2, json!({ "text": "" }));
    assert!(a.contains(&Action::SetInput { text: String::new(), caret: 0 }));
    assert!(!h.b.dialog_open());
    assert_eq!(h.b.view().placeholder, s::PH.1);
}

#[test]
fn copying_a_code_block_flashes_its_button_and_returns_focus() {
    let mut h = H::new();
    opened(&mut h, ai_cfg(), vec![]);
    h.type_text("? one");
    h.tick(80.0);
    let t = ticket(&h.key(KeyEvent::new("Enter")), "cmd_ask");
    h.reply(t, json!({ "ok": true, "text": "```\nlet x = 1;\n```" }));
    let a = h.ev(BarEvent::CopyCode { block: 0, text: "let x = 1;".into() });
    assert_eq!(copied(&a), vec!["let x = 1;".to_string()]);
    assert!(a.contains(&Action::FocusInput));
    assert_eq!(h.b.view().answer.unwrap().code_copied, Some(0));
    h.tick(1801.0);
    assert_eq!(h.b.view().answer.unwrap().code_copied, None);
    assert!(h.ev(BarEvent::AnswerMouseUp { selection_collapsed: true }).contains(&Action::FocusInput));
    assert!(h.ev(BarEvent::AnswerMouseUp { selection_collapsed: false }).is_empty());
}

// ---------------------------------------------------------------- dictation

#[test]
fn dictation_types_the_transcript_at_the_caret() {
    let mut h = H::new();
    open_default(&mut h);
    h.type_text("hello");
    h.tick(80.0);
    let a = h.ev(BarEvent::MicClick);
    let t = ticket(&a, "voice_state");
    let a = h.reply(t, json!({ "state": "idle", "enabled": true }));
    assert_eq!(cmds(&a), vec!["voice_start"]);
    h.ev(BarEvent::Voice(json!({ "state": "recording" })));
    let v = h.b.view();
    assert_eq!(v.mic.state, MicState::Recording);
    assert_eq!(v.mic.tooltip, s::MIC_STOP.1);
    // F4 while recording stops it
    assert_eq!(cmds(&h.key(KeyEvent::new("F4"))), vec!["voice_stop"]);
    h.ev(BarEvent::Voice(json!({ "state": "transcribing" })));
    assert_eq!(h.b.view().mic.tooltip, s::MIC_BUSY.1);
    assert!(cmds(&h.key(KeyEvent::new("F4"))).is_empty(), "busy: F4 does nothing");
    let a = h.ev(BarEvent::Voice(json!({ "state": "done", "text": "  world  " })));
    assert!(a.contains(&Action::SetInput { text: "hello world".into(), caret: 11 }));
    assert_eq!(h.b.input(), "hello world");
    assert_eq!(h.b.view().mic.state, MicState::Idle);
    let a = h.tick(80.0);
    assert_eq!(calls(&a)[0].2, json!({ "text": "hello world" }), "the transcript is searched like typed text");
}

#[test]
fn a_transcript_replaces_the_selection_and_needs_no_extra_space() {
    let mut h = H::new();
    open_default(&mut h);
    h.type_text("open  notes");
    h.ev(BarEvent::Caret { start: 5, end: 11 });
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    h.reply(t, json!({ "state": "idle" }));
    h.ev(BarEvent::Voice(json!({ "state": "done", "text": "мои заметки" })));
    assert_eq!(h.b.input(), "open  мои заметки".replace("open  ", "open "), "pre = `open ` ends with a blank: no extra space");
}

#[test]
fn only_a_take_started_here_is_typed_into_the_bar() {
    let mut h = H::new();
    open_default(&mut h);
    h.ev(BarEvent::Voice(json!({ "state": "done", "text": "from the text tab" })));
    assert_eq!(h.b.input(), "");
    assert_eq!(h.b.view().mic.state, MicState::Idle);
}

#[test]
fn dictation_off_and_errors_are_worded() {
    let mut h = H::new();
    open_default(&mut h);
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    let a = h.reply(t, json!({ "state": "idle", "enabled": false }));
    assert!(cmds(&a).is_empty());
    let m = h.b.view().message.unwrap();
    assert_eq!((m.text.as_str(), m.link.as_deref()), (s::VOICE_OFF.1, Some("voice")));
    // a busy recorder: nothing starts
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    assert!(cmds(&h.reply(t, json!({ "state": "recording" }))).is_empty());
    // a take that fails to start
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    let a = h.reply(t, json!({ "state": "idle" }));
    h.reply(ticket(&a, "voice_start"), json!({ "ok": false }));
    h.ev(BarEvent::Voice(json!({ "state": "error", "code": "mic" })));
    assert!(h.b.view().message.is_none(), "dict_here was cleared: the late error event is not ours");
    // error codes from our own take
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    h.reply(t, json!({ "state": "idle" }));
    h.ev(BarEvent::Voice(json!({ "state": "error", "code": "too_short" })));
    assert_eq!(h.b.view().message.unwrap().text, "Recording too short: say something longer");
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    h.reply(t, json!({ "state": "idle" }));
    h.ev(BarEvent::Voice(json!({ "state": "error", "code": "weird", "message": "boom" })));
    assert_eq!(h.b.view().message.unwrap().text, "Dictation failed: boom");
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    h.reply(t, json!({ "state": "idle" }));
    h.ev(BarEvent::Voice(json!({ "state": "error", "code": "no_key" })));
    assert_eq!(h.b.view().message.unwrap().link.as_deref(), Some("voice"));
    let t = ticket(&h.ev(BarEvent::MicClick), "voice_state");
    h.reply(t, json!({ "state": "idle" }));
    h.ev(BarEvent::Voice(json!({ "state": "recording" })));
    h.ev(BarEvent::Voice(json!({ "state": "error", "code": "busy" })));
    assert_eq!(h.b.view().mic.state, MicState::Recording, "a busy error keeps our take");
    h.ev(BarEvent::Voice(json!({ "state": "idle" })));
    assert_eq!(h.b.view().mic.state, MicState::Idle, "Esc elsewhere cancelled it");
}

// ---------------------------------------------------------------- uninstall inside the bar

fn uninstall_row() -> Value {
    res("uninstall", "uninstall:3", "qBittorrent", "The qBittorrent project · 4.6.5", "3")
}
fn job(phase: &str, extra: Value) -> Value {
    let mut v = json!({ "phase": phase, "name": "qBittorrent" });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
}
fn open_card(h: &mut H) {
    h.search("u qbit", vec![uninstall_row()]);
    let a = h.key(KeyEvent::new("Enter"));
    assert_eq!(calls(&a)[0].1, "uninstall_info");
    h.reply(ticket(&a, "uninstall_info"), json!({ "ok": true, "name": "qBittorrent", "publisher": "P", "version": "1", "location": "", "sizeKb": 0, "store": false, "canSilent": false, "risky": false }));
    h.tick(500.0);
}

#[test]
fn an_uninstall_row_opens_the_confirm_card_and_nothing_else() {
    let mut h = H::new();
    open_default(&mut h);
    let a = h.search("u qbit", vec![uninstall_row()]);
    assert!(!cmds(&a).iter().any(|c| c.starts_with("uninstall")));
    open_card(&mut h);
    let v = h.b.view();
    assert!(v.uninstall.takeover && v.uninstall.panel.is_some() && v.uninstall.list_hidden);
    assert_eq!(h.b.uninstall().state().phase, Phase::Confirm);
}

#[test]
fn while_the_panel_is_up_enter_belongs_to_the_panel_not_the_list() {
    let mut h = H::new();
    open_default(&mut h);
    open_card(&mut h);
    let a = h.key(KeyEvent::new("Enter"));
    assert!(!cmds(&a).contains(&"cmd_run"), "the bar's Enter handler never saw the key");
    let a = h.key(KeyEvent::new("ArrowDown"));
    assert!(!a.iter().any(|x| matches!(x, Action::ScrollToRow(_))));
}

#[test]
fn typing_never_blocks_the_search_while_a_job_runs() {
    let mut h = H::new();
    open_default(&mut h);
    open_card(&mut h);
    h.ev(BarEvent::UninstallState(job("running", json!({}))));
    h.tick(300.0);
    h.ev(BarEvent::UninstallState(job("scanning", json!({ "found": 3, "step": "services" }))));
    h.tick(300.0);
    assert!(h.b.view().uninstall.takeover);
    // the user types something else: the panel steps aside in the same event, the search runs after its debounce
    let a = h.type_text("calc");
    assert!(!h.b.view().uninstall.takeover);
    assert!(has_anim(&a, AK::Collapse));
    assert!(!cmds(&a).contains(&"uninstall_cancel"), "the job is not cancelled by typing");
    let a = h.tick(300.0);
    let q = calls(&a).into_iter().find(|c| c.1 == "cmd_query").expect("the query ran");
    assert_eq!(q.2, json!({ "text": "calc" }));
    let v = h.b.view();
    assert!(!v.uninstall.list_hidden, "the result list is back");
    assert_eq!(v.uninstall.strip.unwrap().phase, Phase::Scanning);
    // results of the new query show up under the strip
    h.reply(q.0.unwrap(), json!([res("calc", "calc:4", "= 4", "2+2  -  Enter copies", "4")]));
    let v = h.b.view();
    assert_eq!(v.rows.len(), 1);
    assert!(v.uninstall.strip.is_some() && v.uninstall.panel.is_none());
}

#[test]
fn a_letter_pressed_in_a_panel_button_goes_to_the_search_box() {
    let mut h = H::new();
    open_default(&mut h);
    open_card(&mut h);
    let a = h.key(KeyEvent::new("q"));
    assert!(a.contains(&Action::FocusInput));
    assert!(!h.b.view().uninstall.takeover);
}

#[test]
fn ctrl_u_returns_to_the_panel_and_esc_hides_it_without_closing_the_bar() {
    let mut h = H::new();
    open_default(&mut h);
    open_card(&mut h);
    h.ev(BarEvent::UninstallState(job("scanning", json!({ "found": 1 }))));
    h.tick(300.0);
    let a = h.key(KeyEvent::new("Escape"));
    assert!(!cmds(&a).contains(&"cmd_toggle"), "Esc inside the panel only steps it aside");
    assert!(!h.b.view().uninstall.takeover);
    h.tick(300.0);
    h.key(KeyEvent::new("u").with_code("KeyU").ctrl());
    assert!(h.b.view().uninstall.takeover);
    h.tick(200.0); // the result list steps aside first, then the panel is on screen
    // Esc steps the panel aside again; with it away, Esc closes the bar as usual
    h.key(KeyEvent::new("Escape"));
    h.tick(300.0);
    assert_eq!(cmds(&h.key(KeyEvent::new("Escape"))), vec!["cmd_toggle"]);
}

#[test]
fn a_reopened_bar_rehydrates_the_job_from_the_backend() {
    let mut h = H::new();
    let a = h.ev(BarEvent::Shown);
    let tu = ticket(&a, "uninstall_state");
    h.reply(tu, job("scanning", json!({ "found": 12, "step": "registry (user)" })));
    h.tick(200.0);
    let v = h.b.view();
    assert!(!v.uninstall.takeover, "the panel is not forced open over the search box");
    let strip = v.uninstall.strip.unwrap();
    assert_eq!((strip.name.as_str(), strip.phase), ("qBittorrent", Phase::Scanning));
    // the island card's button (uninstall:focus) brings the panel back
    h.ev(BarEvent::UninstallFocus);
    assert!(h.b.view().uninstall.takeover);
    assert_eq!(h.b.view().uninstall.panel.unwrap().phase, "scanning");
}

#[test]
fn panel_presses_go_through_the_bar() {
    let mut h = H::new();
    open_default(&mut h);
    open_card(&mut h);
    let a = h.ev(BarEvent::UninstallPress(Control::Start));
    assert_eq!(calls(&a), vec![(None, "uninstall_start", json!({ "silent": false }))]);
    let a = h.ev(BarEvent::UninstallItem(json!({ "shown": "x", "ok": true, "n": 1, "total": 1 })));
    assert!(a.is_empty() || !cmds(&a).contains(&"uninstall_delete"));
}

#[test]
fn a_confirm_card_closed_with_cancel_reruns_the_search() {
    let mut h = H::new();
    open_default(&mut h);
    open_card(&mut h);
    let a = h.ev(BarEvent::UninstallPress(Control::Cancel));
    assert_eq!(cmds(&a), vec!["uninstall_cancel"]);
    h.tick(300.0);
    let a = h.tick(80.0);
    assert_eq!(calls(&a)[0].1, "cmd_query", "the box content is searched again once the panel is gone");
}

// ---------------------------------------------------------------- motion and the wave

#[test]
fn reduced_motion_session_emits_no_animation_at_all() {
    let mut h = H::new();
    opened(&mut h, json!({ "language": "en", "reduceMotion": true, "net.ai": true, "ai.baseUrl": "u", "ai.model": "m" }), vec![]);
    let mut all = Vec::new();
    all.extend(h.search("a", vec![app(1), app(2)]));
    all.extend(h.key(KeyEvent::new("ArrowDown")));
    all.extend(h.ev(BarEvent::RowContext { index: 0, x: 5.0, y: 5.0 }));
    all.extend(h.ev(BarEvent::ContextPick(RunMode::Pin)));
    all.extend(h.search("b", vec![app(2), app(3)]));
    assert!(!any_anim(&all), "{all:?}");
    assert!(!h.b.animating(), "and the wave rests (drawn once)");
}

#[test]
fn os_reduce_motion_also_counts() {
    let mut h = H::new();
    h.ev(BarEvent::OsReduceMotion(true));
    opened(&mut h, json!({ "language": "en" }), vec![]);
    assert!(h.b.view().reduce_motion);
    assert!(!any_anim(&h.search("a", vec![app(1)])));
}

#[test]
fn the_wave_runs_while_shown_is_kicked_by_typing_and_stops_on_hide() {
    let mut h = H::new();
    open_default(&mut h);
    assert!(h.b.wave().running() && h.b.animating());
    h.ev(BarEvent::BodyWidth(560.0));
    h.ev(BarEvent::BodyMouse { x: 100.0 });
    let before = h.b.wave().points();
    h.type_text("x");
    h.tick(16.0);
    h.tick(16.0);
    assert_ne!(h.b.wave().points(), before);
    h.ev(BarEvent::Hidden);
    assert!(!h.b.wave().running());
    // a window that is not visible does not animate
    open_default(&mut h);
    h.ev(BarEvent::Visibility(false));
    assert!(!h.b.wave().running());
    assert!(h.b.view().parked);
    h.ev(BarEvent::Visibility(true));
    assert!(h.b.wave().running());
}

#[test]
fn language_switch_relabels_rows_without_a_new_query() {
    let mut h = H::new();
    open_default(&mut h);
    h.search("a", vec![res("system", "sys:lock", "Lock", "Press Enter twice to confirm", "lock")]);
    assert_eq!(h.b.view().rows[0].title[0].text, "Lock the computer");
    let a = h.ev(BarEvent::Shown);
    h.reply(ticket(&a, "settings_get"), json!({ "language": "ru" }));
    h.reply(ticket(&a, "cmd_query"), json!([res("system", "sys:lock", "Lock", "Press Enter twice to confirm", "lock")]));
    assert_eq!(h.b.view().rows[0].title[0].text, "Заблокировать компьютер");
}
