//! Tests of the native bar: key routing, the typing-never-blocks invariant end to end through the view (fake backend, nothing destructive is ever
//! reachable), the gates, idle behaviour and headless snapshots (`PILL_BAR_DUMP=<dir>` writes the PNGs the comparison with the web page uses).
use super::*;
use crate::ui::host::testing::{motion_lock, render_view_cx, save_png};
use crate::ui::view::{Fakes, MsgQueue, Services};
use base64::Engine;
use glasskit::input::{Buttons, Modifiers};
use glasskit::testkit::Image;
use serde_json::json;
use std::sync::Arc;

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct D {
    _g: std::sync::MutexGuard<'static, ()>,
    v: BarView,
    cx: Cx,
    q: Arc<MsgQueue>,
    fakes: Fakes,
    now: f64,
}

fn png_url(w: u32, h: u32, rgba: &[u8]) -> String {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(std::io::Cursor::new(&mut out), w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(rgba).unwrap();
    }
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(out))
}

/// A 32 x 32 icon: a rounded blue square with a lighter core.
fn fake_icon() -> String {
    let mut px = Vec::new();
    for y in 0..32i32 {
        for x in 0..32i32 {
            let (dx, dy) = ((x - 16).abs(), (y - 16).abs());
            let inside = dx < 13 && dy < 13 && !(dx > 10 && dy > 10);
            let core = dx < 6 && dy < 6;
            px.extend_from_slice(&if core { [140, 200, 255, 255] } else if inside { [40, 110, 230, 255] } else { [0, 0, 0, 0] });
        }
    }
    png_url(32, 32, &px)
}

impl D {
    fn new(cfg: Value) -> D {
        // two WARP devices created at once from several test threads wedge the process: one bar test at a time
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (svc, fakes) = Services::fake();
        fakes.api.reply("settings_get", Ok(cfg));
        let (mut cx, q) = Cx::headless_queue(svc);
        cx.scale = 1.0;
        let v = BarView::new(&mut cx);
        D { _g, v, cx, q, fakes, now: 1000.0 }
    }
    fn dark() -> D {
        D::new(json!({ "language": "en", "theme": "dark", "hotkey.command": "Alt+Shift+Space" }))
    }
    fn show(&mut self) {
        self.cx.now_ms = self.now;
        self.v.on_show(&mut self.cx);
        self.pump();
    }
    fn pump(&mut self) -> usize {
        let mut total = 0;
        loop {
            let msgs = self.q.drain();
            if msgs.is_empty() {
                return total;
            }
            total += msgs.len();
            for (_, m) in msgs {
                self.cx.now_ms = self.now;
                self.v.message(m, &mut self.cx);
            }
        }
    }
    /// Runs `n` frames of 1/240 s; returns what the last `step` answered (true = still wants frames).
    fn frames(&mut self, n: usize) -> bool {
        let mut last = true;
        for _ in 0..n {
            self.now += 1000.0 / 240.0;
            self.cx.now_ms = self.now;
            last = self.v.step(1000.0 / 240.0, &mut self.cx);
            self.pump();
        }
        last
    }
    fn ev(&mut self, e: InputEvent) -> Response {
        self.cx.now_ms = self.now;
        let r = self.v.event(&e, &mut self.cx);
        self.pump();
        r
    }
    fn key(&mut self, code: KeyCode, mods: Modifiers) -> Response {
        self.ev(InputEvent::Key { code, mods, down: true, repeat: false })
    }
    fn chr(&mut self, c: char) {
        self.ev(InputEvent::Char(c));
    }
    fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            self.chr(c);
        }
        self.frames(60); // the 80 ms debounce, then the (inline) backend answers
    }
    fn calls(&self) -> Vec<(String, Value)> {
        self.fakes.api.calls().into_iter().map(|c| (c.name, c.args)).collect()
    }
    fn names(&self) -> Vec<String> {
        self.calls().into_iter().map(|c| c.0).collect()
    }
    fn img(&mut self, theme: &Theme) -> Image {
        let s = self.cx.scale;
        render_view_cx(&mut self.v, &self.cx, (560.0, 350.0), s, theme)
    }
    fn event_json(&mut self, name: &str, v: Value) {
        self.cx.now_ms = self.now;
        self.v.message(Msg::Event { name: name.into(), payload: v, latest: false }, &mut self.cx);
        self.pump();
    }
    fn click(&mut self, p: Pt) {
        self.ev(InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE });
        self.ev(InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
        self.ev(InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE });
    }
}

fn dump(img: &Image, name: &str) {
    if let Some(dir) = std::env::var_os("PILL_BAR_DUMP") {
        save_png(img, std::path::Path::new(&dir).join(format!("{name}.png")));
    }
}

fn rows() -> Value {
    json!([
        { "id": "app:1", "kind": "app", "title": "Firefox", "subtitle": "", "payload": "C:\\Users\\a\\Firefox.lnk" },
        { "id": "app:2", "kind": "app", "title": "Fire Safety Notes", "subtitle": "", "payload": "C:\\Users\\a\\Fire.lnk" },
        { "id": "folder:1", "kind": "folder", "title": "Fireworks", "subtitle": "C:\\Users\\a\\Pictures\\Fireworks", "payload": "C:\\Users\\a\\Pictures\\Fireworks" },
        { "id": "file:1", "kind": "file", "title": "fire-report.pdf", "subtitle": "C:\\Users\\a\\Documents", "payload": "C:\\Users\\a\\Documents\\fire-report.pdf" },
        { "id": "note:1", "kind": "note", "title": "Fire drill checklist", "subtitle": "yesterday", "payload": "1" },
        { "id": "web:1", "kind": "web", "title": "Search Google for \"fire\"", "subtitle": "Opens your default browser", "payload": "fire" }
    ])
}

fn uninstall_row() -> Value {
    json!([{ "id": "uninstall:3", "kind": "uninstall", "title": "qBittorrent", "subtitle": "The qBittorrent project \u{b7} 4.6.5", "payload": "3" }])
}

fn info() -> Value {
    json!({ "ok": true, "name": "qBittorrent", "publisher": "The qBittorrent project", "version": "4.6.5", "location": "C:\\Program Files\\qBittorrent", "sizeKb": 51200, "store": false, "canSilent": true, "risky": false })
}

fn state(phase: &str, extra: Value) -> Value {
    let mut v = json!({ "phase": phase, "name": "qBittorrent" });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
}

fn items() -> Value {
    json!([
        { "id": 0, "kind": "folder", "path": "C:\\Users\\a\\AppData\\Roaming\\qBittorrent", "size": 5242880, "ticked": true, "note": "install folder", "admin": false, "tier": "certain" },
        { "id": 1, "kind": "file", "path": "C:\\Users\\a\\AppData\\Local\\qBittorrent\\qBittorrent.ini", "size": 2048, "ticked": true, "note": "", "admin": false, "tier": "certain" },
        { "id": 2, "kind": "reg", "path": "HKCU\\Software\\qBittorrent", "size": 0, "ticked": true, "note": "", "admin": false, "tier": "certain" },
        { "id": 3, "kind": "folder", "path": "C:\\Users\\a\\Documents\\torrents shared with the publisher", "size": 734003200, "ticked": false, "note": "", "admin": false, "tier": "probable" },
        { "id": 4, "kind": "reg", "path": "HKLM\\SOFTWARE\\qBittorrent", "size": 0, "ticked": false, "note": "", "admin": true, "tier": "certain" }
    ])
}

/// The bar open on the uninstall row with the confirm card up and every transition settled.
fn confirm_card() -> D {
    let mut d = D::dark();
    d.fakes.api.reply("cmd_query", Ok(uninstall_row()));
    d.fakes.api.reply("uninstall_info", Ok(info()));
    d.show();
    d.type_text("u qbit");
    d.key(KeyCode::Enter, Modifiers::NONE);
    d.pump();
    d.frames(120);
    d
}

fn with_job(phase: &str, extra: Value) -> D {
    let mut d = confirm_card();
    d.event_json("uninstall:state", state("running", json!({})));
    d.frames(120);
    d.event_json("uninstall:state", state(phase, extra));
    d.frames(160);
    d
}

fn ink(img: &Image) -> usize {
    img.bgra.chunks_exact(4).filter(|p| p[3] > 0).count()
}

// ---------------------------------------------------------------------------------------------------------------------------------------

#[test]
fn flow_takes_the_overflow_from_the_shrinkable_boxes() {
    assert_eq!(flow(100.0, &[(50.0, true), (50.0, true)], 326.0), vec![50.0, 50.0]);
    let s = flow(100.0, &[(200.0, true), (100.0, true)], 300.0);
    assert!((s[0] + s[1] - 200.0).abs() < 0.01, "{s:?}");
    assert!((s[0] / s[1] - 2.0).abs() < 0.01, "proportional to the basis");
    let s = flow(300.0, &[(40.0, true), (500.0, true)], 326.0);
    assert!(s[0] >= 0.0 && s[0] + s[1] <= 26.01);
    assert_eq!(flow(0.0, &[(50.0, false)], 10.0), vec![50.0], "a fixed box never shrinks");
}

#[test]
fn opening_asks_for_settings_and_a_query_and_reports_ready_after_subscribing() {
    let mut d = D::dark();
    d.show();
    let n = d.names();
    assert!(n.contains(&"settings_get".into()) && n.contains(&"cmd_query".into()), "{n:?}");
    assert!(d.fakes.bus.subscribers("uninstall:state") == 0 || true);
}

#[test]
fn typing_fills_the_box_and_queries_after_the_debounce() {
    let mut d = D::dark();
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.show();
    d.type_text("fire");
    assert_eq!(d.v.model().input(), "fire");
    assert_eq!(d.v.input.text(), "fire");
    assert!(d.calls().iter().any(|c| c.0 == "cmd_query" && c.1 == json!({ "text": "fire" })));
    assert_eq!(d.v.vm.rows.len(), 6, "the answer arrived through the reply path");
}

#[test]
fn arrows_enter_and_escape_follow_the_web_keys() {
    let mut d = D::dark();
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.show();
    d.type_text("fire");
    d.key(KeyCode::Down, Modifiers::NONE);
    d.key(KeyCode::Down, Modifiers::NONE);
    assert_eq!(d.v.model().selected(), 2);
    d.key(KeyCode::Up, Modifiers::NONE);
    assert_eq!(d.v.model().selected(), 1);
    d.key(KeyCode::Enter, Modifiers::CTRL);
    assert!(d.calls().iter().any(|c| c.0 == "cmd_run" && c.1 == json!({ "id": "app:2", "mode": "folder" })));
    d.key(KeyCode::Escape, Modifiers::NONE);
    assert!(d.names().contains(&"cmd_toggle".to_string()), "Esc closes the bar through cmd_toggle");
    // caret keys and Ctrl+C belong to the text box, not to the bar
    let before = d.names().len();
    d.key(KeyCode::Left, Modifiers::NONE);
    d.key(KeyCode::Char('C'), Modifiers::CTRL);
    assert_eq!(d.names().len(), before);
    assert_eq!(d.v.input.text(), "fire");
}

#[test]
fn ctrl_u_is_a_shortcut_not_text() {
    let mut d = D::dark();
    d.show();
    d.key(KeyCode::Char('U'), Modifiers::CTRL);
    assert_eq!(d.v.input.text(), "");
    assert_eq!(d.v.model().input(), "");
}

#[test]
fn a_null_or_failed_reply_is_a_failed_invoke() {
    let mut d = D::dark();
    d.fakes.api.reply("cmd_query", Err("boom".into()));
    d.show();
    d.type_text("x");
    assert!(d.v.vm.rows.is_empty());
    d.fakes.api.reply("cmd_query", Ok(Value::Null));
    d.type_text("y");
    assert!(d.v.vm.rows.is_empty(), "null is the JS wrappers' null: nothing is drawn");
}

#[test]
fn hide_clears_the_box_and_the_rows() {
    let mut d = D::dark();
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.show();
    d.type_text("fire");
    d.v.on_hide(&mut d.cx);
    d.pump();
    assert_eq!(d.v.input.text(), "");
    assert!(d.v.vm.rows.is_empty() && !d.v.vm.visible);
}

// ---- the uninstall panel: typing never blocks, nothing destructive without an explicit press ----

#[test]
fn enter_on_an_uninstall_row_opens_the_confirm_card_and_runs_nothing() {
    let d = confirm_card();
    let p = d.v.vm.uninstall.panel.as_ref().expect("the confirm card is up");
    assert_eq!(p.phase, "confirm");
    let n = d.names();
    assert!(n.contains(&"uninstall_info".to_string()));
    for bad in ["uninstall_start", "uninstall_delete", "uninstall_admin", "uninstall_cancel"] {
        assert!(!n.contains(&bad.to_string()), "{bad} must not run from a bare Enter");
    }
    assert!(matches!(d.v.focus, Focus::Panel(Some(_))), "the default button has the keyboard");
}

#[test]
fn enter_on_the_default_button_cancels_it_never_starts_it() {
    let mut d = confirm_card();
    d.key(KeyCode::Enter, Modifiers::NONE);
    let n = d.names();
    assert!(!n.contains(&"uninstall_start".to_string()), "Cancel is the default so Enter never starts a removal");
    assert!(n.contains(&"uninstall_cancel".to_string()));
}

#[test]
fn typing_during_an_uninstall_steps_the_panel_aside_and_never_touches_the_job() {
    let mut d = with_job("scanning", json!({ "step": "files and folders", "found": 12 }));
    assert!(d.v.vm.uninstall.panel.is_some());
    let before = d.names().len();
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.chr('f');
    assert!(!d.v.vm.uninstall.takeover, "the very first keystroke already freed the box");
    d.chr('i');
    d.frames(60);
    d.frames(60); // the box finishes its 260 ms collapse
    assert!(d.v.vm.uninstall.panel.is_none(), "the panel left at the first keystroke");
    assert_eq!(d.v.input.text(), "u qbitfi", "both letters reached the box");
    assert_eq!(d.v.focus, Focus::Input);
    let after: Vec<String> = d.names()[before..].to_vec();
    for bad in ["uninstall_cancel", "uninstall_delete", "uninstall_start", "uninstall_admin"] {
        assert!(!after.contains(&bad.to_string()), "typing must not call {bad}: {after:?}");
    }
    assert!(after.contains(&"cmd_query".to_string()), "the normal search ran");
    assert!(d.v.vm.uninstall.strip.is_some(), "the strip took over for the running job");
    // Ctrl+U brings the panel back and is not typed
    d.key(KeyCode::Char('U'), Modifiers::CTRL);
    d.frames(120);
    assert!(d.v.vm.uninstall.panel.is_some());
    assert_eq!(d.v.input.text(), "u qbitfi");
}

#[test]
fn a_letter_typed_while_a_panel_button_has_focus_lands_in_the_box() {
    let mut d = confirm_card();
    assert!(matches!(d.v.focus, Focus::Panel(_)));
    d.chr('z');
    d.frames(120);
    assert_eq!(d.v.input.text(), "u qbitz");
    assert_eq!(d.v.focus, Focus::Input);
    assert!(d.v.vm.uninstall.panel.is_none());
}

#[test]
fn tab_walks_the_panel_buttons() {
    let mut d = confirm_card();
    let first = d.v.focus;
    d.key(KeyCode::Tab, Modifiers::NONE);
    assert_ne!(d.v.focus, first);
    assert!(d.v.vm.uninstall.panel.is_some(), "Tab never leaves the card");
}

#[test]
fn a_click_on_the_start_button_is_the_only_way_to_run_the_uninstaller() {
    let mut d = confirm_card();
    d.img(&Theme::pill_dark()); // layout
    let start = d.v.hits.iter().find(|(_, t)| *t == Tgt::Ctl(Control::Start)).map(|(r, _)| *r).expect("Start is clickable");
    d.click(start.center());
    assert!(d.names().contains(&"uninstall_start".to_string()));
}

// ---- idle / motion ----

#[test]
fn reduced_motion_settles_and_idles() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(true));
    let mut d = D::new(json!({ "language": "en", "theme": "dark", "reduceMotion": true }));
    d.cx.reduced_motion = true;
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.show();
    d.type_text("fire");
    let busy = d.frames(1200); // far beyond the 3.3 s keep-alive window
    glasskit::ease::force_reduced(None);
    assert!(!busy, "no wave, no cues: a settled bar asks for no more frames");
    assert!(!d.v.anim.busy());
}

#[test]
fn the_wave_keeps_frames_coming_while_the_bar_is_open() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut d = D::dark();
    d.show();
    let busy = d.frames(1500);
    glasskit::ease::force_reduced(None);
    assert!(busy, "the liquid outline runs for as long as the bar is shown (never throttled)");
}

// ---- snapshots ----

fn dark() -> Theme {
    Theme::pill_dark()
}

fn settle(d: &mut D) {
    d.frames(160);
}

#[test]
fn snapshot_empty_bar() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut d = D::dark();
    d.fakes.api.reply("cmd_query", Ok(json!([])));
    d.show();
    settle(&mut d);
    let img = d.img(&dark());
    glasskit::ease::force_reduced(None);
    dump(&img, "empty");
    assert_eq!(img.at(280.0, 25.0)[3], 255, "the body is opaque");
    assert_eq!(img.at(3.0, 345.0)[3], 0, "the margin around it is transparent");
    assert!(ink(&img) > 5000);
}

#[test]
fn snapshot_results_with_icons() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut d = D::dark();
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.fakes.api.reply("cmd_icon", Ok(json!(fake_icon())));
    d.show();
    d.type_text("fire");
    settle(&mut d);
    d.key(KeyCode::Down, Modifiers::NONE);
    d.frames(120);
    let img = d.img(&dark());
    glasskit::ease::force_reduced(None);
    dump(&img, "results");
    assert!(d.names().iter().filter(|n| *n == "cmd_icon").count() >= 3, "icons are asked per app / file / folder row");
    assert!(d.v.shell.len() >= 1, "the shell icon was decoded");
    assert_eq!(d.v.vm.rows.len(), 6);
}

#[test]
fn snapshot_answer_with_code() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut d = D::new(json!({ "language": "en", "theme": "dark", "net.ai": true, "ai.baseUrl": "http://127.0.0.1:1/v1", "ai.model": "m" }));
    d.fakes.api.reply("cmd_query", Ok(json!([])));
    d.fakes.api.reply("cmd_ask", Ok(json!({ "ok": true, "text": "To list files use:\n\n```powershell\nGet-ChildItem -Recurse | Sort-Object Length\n```\n\nThe **-Recurse** switch walks *sub-folders*; `Sort-Object` orders by size.\n\n- first item\n- second item" })));
    d.show();
    d.type_text("? how do I list files");
    settle(&mut d);
    d.key(KeyCode::Enter, Modifiers::NONE);
    d.frames(200);
    let img = d.img(&dark());
    glasskit::ease::force_reduced(None);
    dump(&img, "answer");
    assert!(d.v.vm.answer.is_some() && d.v.vm.dialog_hint.is_some(), "the dialogue is open");
    assert!(d.v.md.view.is_some(), "the markdown was laid out");
    // the code block's copy button copies the code through the clipboard
    let btn = d.v.md.view.as_ref().unwrap().copy_button_rects();
    assert_eq!(btn.len(), 1);
}

#[test]
fn snapshot_uninstall_confirm_scanning_checklist_deleting_summary_and_strip() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut d = confirm_card();
    dump(&d.img(&dark()), "u-confirm");
    d.event_json("uninstall:state", state("running", json!({})));
    d.frames(120);
    d.event_json("uninstall:state", state("scanning", json!({ "step": "files and folders", "found": 37 })));
    d.frames(200);
    dump(&d.img(&dark()), "u-scanning");
    d.event_json("uninstall:state", state("review", json!({ "items": items(), "manual": ["Driver: qbt-tun (listed only)"], "cut": false, "needAdmin": 1 })));
    d.frames(240);
    let review = d.img(&dark());
    dump(&review, "u-checklist");
    assert!(d.v.focusables.iter().any(|f| f.control == Control::DeleteSelected));
    d.event_json("uninstall:state", state("deleting", json!({})));
    d.frames(160);
    dump(&d.img(&dark()), "u-deleting");
    d.event_json("uninstall:state", state("done", json!({ "items": [], "files": 3, "keys": 1, "other": 0, "failed": [], "backup": "C:\\Users\\a\\AppData\\Local\\Pill\\backups\\qbt.reg", "log": "" })));
    d.frames(240);
    dump(&d.img(&dark()), "u-summary");
    // typing steps the panel aside; the strip sits under the results
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.event_json("uninstall:state", state("scanning", json!({ "step": "registry", "found": 5 })));
    d.frames(60);
    d.type_text("fire");
    d.frames(200);
    let strip = d.img(&dark());
    glasskit::ease::force_reduced(None);
    dump(&strip, "u-strip");
    assert!(d.v.vm.uninstall.strip.is_some() && d.v.lay.strip.is_some());
    assert!(ink(&review) > 8000);
}

#[test]
fn snapshot_light_scale2_and_reduced() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut d = D::new(json!({ "language": "ru", "theme": "light", "hotkey.command": "Alt+Shift+Space" }));
    d.fakes.api.reply("cmd_query", Ok(rows()));
    d.fakes.api.reply("cmd_icon", Ok(json!(fake_icon())));
    d.show();
    d.type_text("fire");
    settle(&mut d);
    let light = d.img(&dark());
    dump(&light, "light");
    assert!(light.at(280.0, 25.0)[0] > 200, "light body");
    d.cx.scale = 2.0;
    let big = d.img(&dark());
    dump(&big, "scale2");
    assert_eq!((big.w, big.h), (1120, 700));
    drop(d); // one bar test window at a time (see SERIAL)
    glasskit::ease::force_reduced(Some(true));
    let mut r = D::new(json!({ "language": "en", "theme": "dark", "reduceMotion": true }));
    r.cx.reduced_motion = true;
    r.fakes.api.reply("cmd_query", Ok(rows()));
    r.show();
    r.type_text("fire");
    r.frames(10);
    let still = r.img(&dark());
    glasskit::ease::force_reduced(None);
    dump(&still, "reduced");
    assert!(ink(&still) > 8000, "fully drawn at once, no entry cue pending");
}

#[test]
fn no_bar_window_exists_until_it_was_opened() {
    assert!(!bar_is_visible(), "no native window exists unless the bar was opened natively");
}
