//! The "Advanced" section at the end of a page: closed by default and never stored, opens on a spring, takes no events and no Tab stops
//! while closed, opens by itself for a search hit inside it, and an old page name still leads to its rows.
use super::tests2::{boot, T};
use super::ui::Act;
use super::*;
use crate::ui::host::testing::{motion_lock, motion_on};
use glasskit::input::{KeyCode, Modifiers};
use pill_ui_core::settings::schema::schema;
use serde_json::json;

fn paint(t: &mut T) {
    let _ = t.sim.render((840.0, 760.0), &Theme::settings_dark());
}

fn adv_on(t: &T) -> bool {
    t.v.borrow().page.adv.as_ref().expect("an Advanced section").on
}

/// The section is shown at the end of the page: scroll down so that the header is on screen.
fn scroll_to_end(t: &mut T) {
    t.v.borrow_mut().page.scroll.scroll_to(99999.0, false);
    t.settle();
}

fn pages_with_advanced() -> Vec<&'static str> {
    schema().pages.iter().filter(|p| p.advanced_groups().count() > 0).map(|p| p.id).collect()
}

#[test]
fn every_page_with_an_advanced_section_starts_with_it_closed() {
    let _g = motion_lock();
    let ids = pages_with_advanced();
    assert!(ids.len() >= 8, "most pages keep their rare rows behind the section: {ids:?}");
    for id in ids {
        let t = boot(json!({"language": "en"}), id);
        let v = t.v.borrow();
        let a = v.page.adv.as_ref().unwrap_or_else(|| panic!("{id}: no header"));
        assert!(!a.on && a.t.at() == 0.0 && !a.sub.is_empty(), "{id} starts closed and says what is inside: {:?}", a.sub);
        assert!(v.page.rows().any(|r| r.adv), "{id}");
        assert!(v.page.rows().filter(|r| r.adv).all(|r| r.ak == 0.0), "{id}: nothing in it takes events");
        let order = v.focus_order();
        assert!(order.contains(&a.fid), "{id}: the header is a Tab stop");
        assert!(!v.page.rows().any(|r| r.adv && order.contains(&r.fid)), "{id}: Tab skips the folded rows");
    }
}

#[test]
fn pages_without_the_section_have_no_header() {
    let _g = motion_lock();
    let t = boot(json!({"language": "en"}), "pet");
    assert!(t.v.borrow().page.adv.is_none() && !t.v.borrow().page.rows().any(|r| r.adv));
    let t = boot(json!({"language": "en"}), "ai");
    assert!(t.v.borrow().page.adv.is_none());
}

#[test]
fn the_header_opens_and_closes_the_section_on_a_spring_and_the_window_idles() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "clipboard");
    paint(&mut t);
    let closed_h = t.v.borrow().page.content_h;
    t.v.borrow_mut().set_advanced(true, false);
    t.sim.frame.request();
    let mut seen = Vec::new();
    for _ in 0..600 {
        if !t.sim.step_frame() {
            break;
        }
        paint(&mut t);
        seen.push(t.v.borrow().page.adv.as_ref().unwrap().t.at());
    }
    assert!(seen.iter().any(|x| *x > 0.05 && *x < 0.95), "it opens through the in-between values, not at once: {:?}", &seen[..seen.len().min(10)]);
    assert!(seen.iter().all(|x| *x < 1.3), "bounded overshoot");
    assert!(!t.sim.frame.wants_frame(), "the window idles once the section has settled");
    {
        let v = t.v.borrow();
        assert!((v.page.adv.as_ref().unwrap().t.at() - 1.0).abs() < 0.01);
        assert!(v.page.content_h > closed_h + 200.0, "the page grew: {} -> {}", closed_h, v.page.content_h);
        assert!(v.page.rows().filter(|r| r.adv).all(|r| (r.ak - 1.0).abs() < 0.01));
        assert!(v.focus_order().contains(&v.page.rows().find(|r| r.adv).unwrap().fid), "its rows are Tab stops while it is open");
    }
    t.v.borrow_mut().set_advanced(false, false);
    t.sim.frame.request();
    for _ in 0..600 {
        if !t.sim.step_frame() {
            break;
        }
        paint(&mut t);
    }
    assert!(!t.sim.frame.wants_frame());
    let v = t.v.borrow();
    assert!(v.page.adv.as_ref().unwrap().t.at().abs() < 0.01 && (v.page.content_h - closed_h).abs() < 1.0, "back to where it was");
    assert!(v.page.rows().filter(|r| r.adv).all(|r| r.ak < 0.01));
}

#[test]
fn reduced_motion_opens_it_at_once() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en", "reduceMotion": true}), "clipboard");
    t.v.borrow_mut().set_advanced(true, false);
    let v = t.v.borrow();
    let a = v.page.adv.as_ref().unwrap();
    assert!(a.on && a.t.at() == 1.0 && !a.t.moving());
}

#[test]
fn a_click_on_the_header_toggles_it_and_a_folded_row_does_not_react() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "screenshots");
    scroll_to_end(&mut t);
    assert!(!adv_on(&t));
    t.click_act(&Act::Advanced);
    assert!(adv_on(&t));
    scroll_to_end(&mut t);
    // an Advanced row works like any other: a switch of the section, clicked, writes its setting
    let r = t.v.borrow().ctl_rect_of("capture.revealAfterSave").expect("row");
    t.click_at(r);
    assert!(t.calls("settings_set").iter().any(|a| a["patch"]["capture.revealAfterSave"] == json!(true)), "{:?}", t.calls("settings_set"));
    t.click_act(&Act::Advanced);
    assert!(!adv_on(&t));
}

#[test]
fn the_state_is_not_stored_and_a_new_window_starts_closed_again() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "clipboard");
    scroll_to_end(&mut t);
    t.click_act(&Act::Advanced);
    assert!(adv_on(&t));
    assert!(t.calls("settings_set").is_empty(), "opening the section writes nothing: {:?}", t.calls("settings_set"));
    // this window remembers it while it lives: away and back, it is still open
    t.v.borrow_mut().show_page("hotkeys", &mut t.sim.cx);
    t.settle();
    t.v.borrow_mut().show_page("clipboard", &mut t.sim.cx);
    t.settle();
    assert!(adv_on(&t));
    assert!(t.calls("settings_set").is_empty());
    // a window of its own starts closed
    let t2 = boot(json!({"language": "en"}), "clipboard");
    assert!(!adv_on(&t2));
}

#[test]
fn the_keyboard_reaches_the_header_and_enter_toggles_it() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "clipboard");
    let fid = t.v.borrow().page.adv.as_ref().unwrap().fid;
    t.v.borrow_mut().set_focus_id(Some(fid), true);
    t.key(KeyCode::Enter, Modifiers::NONE);
    assert!(adv_on(&t));
    // a row inside holds the focus; closing moves the focus to the header instead of leaving it on a folded row
    let inner = t.v.borrow().page.rows().find(|r| r.adv && matches!(r.ctl, page::Ctl::Field(_))).map(|r| r.fid).expect("a field inside");
    t.v.borrow_mut().set_focus_id(Some(inner), true);
    t.v.borrow_mut().set_advanced(false, false);
    assert_eq!(t.v.borrow().focus.focused(), Some(fid));
}

#[test]
fn a_search_hit_inside_a_closed_section_opens_it_scrolls_to_the_row_and_flashes_it() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "general");
    t.key(KeyCode::Char('F'), Modifiers::CTRL);
    t.text("video encoder");
    assert!(t.v.borrow().search.rows.iter().any(|r| r.title == "Video encoder"), "the row is found: {:?}", t.v.borrow().search.rows.iter().map(|r| r.title.clone()).collect::<Vec<_>>());
    t.key(KeyCode::Enter, Modifiers::NONE);
    assert_eq!(t.current(), "capture", "it opens the right page");
    t.settle();
    t.sim.run(3);
    paint(&mut t);
    t.sim.run(3);
    let v = t.v.borrow();
    assert!(v.page.adv.as_ref().unwrap().on, "and the section the row is in");
    let row = v.page.rows().find(|r| r.row.id == "capture.encoder").expect("row");
    assert!((row.ak - 1.0).abs() < 0.01 && row.rect.h > 20.0, "it is laid out, not folded: {:?}", row.rect);
    assert!(row.hl.is_some() || v.goto.is_none(), "the flash plays or has played");
    assert!(v.goto.is_none(), "the request is done");
    let top = v.page.scroll.offset();
    assert!(row.rect.y >= top - 1.0 && row.rect.bottom() <= top + v.content_rect().h + 1.0, "the row is on screen ({:?} at scroll {top})", row.rect);
}

#[test]
fn a_search_hit_on_the_page_that_is_already_open_also_opens_the_section() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "clipboard");
    assert!(!adv_on(&t));
    t.key(KeyCode::Char('F'), Modifiers::CTRL);
    t.text("video encoder");
    t.key(KeyCode::Enter, Modifiers::NONE);
    t.settle();
    assert!(adv_on(&t));
    assert!(t.v.borrow().goto.is_none());
}

#[test]
fn old_page_names_open_the_page_that_holds_their_rows_and_scroll_to_the_section() {
    let _g = motion_lock();
    for (old, page, row) in [
        ("behavior", "general", "hideInFullscreen"),
        ("hiding", "general", "hideInFullscreen"),
        ("quiet", "general", "quiet.on"),
        ("search", "general", "module.fileSearch"),
        ("weather", "weather", "net.weather"),
        ("voice", "voice", "voice.enabled"),
        ("aisetup", "ai", "aisetup.intro"),
        ("character", "edge", "character.liquid"),
        ("sound", "media", "live.volume"),
        ("modules", "tabs", "modules.tabs"),
        ("live", "notifications", "module.notifs"),
        ("start", "general", "language"),
        ("general", "general", "language"),
        ("appearance", "look", "shape"),
        // the single-function pages of 0.8.8: each is a section of one of the fifteen pages now (the page may be long: the row is scrolled to)
        ("pet", "edge", "pet.side"),
        ("clock", "look", "showTime"),
        ("animation", "look", "reduceMotion"),
        ("volume", "media", "live.volume"),
        ("screenshots", "capture", "capture.shotAction"),
        ("recording", "capture", "live.recording"),
        ("notes", "tools", "module.note"),
        ("launcher", "tools", "module.launcher"),
        ("shelf", "tools", "module.shelf"),
        ("text", "tools", "module.text"),
        ("controls", "tools", "module.controls"),
        ("vpn", "tools", "module.vpn"),
        ("downloads", "notifications", "live.downloads"),
        ("battery", "notifications", "live.battery"),
        ("keyboard", "notifications", "live.keyboardLayout"),
        ("desktops", "notifications", "live.virtualDesktopFlash"),
        ("usb", "notifications", "live.usb"),
        ("obs", "calendar", "live.obs"),
        ("mail", "calendar", "live.mail"),
        ("agents", "ai", "live.agents"),
        ("localai", "ai", "aisetup.intro"),
    ] {
        let mut t = boot(json!({"language": "en"}), old);
        t.settle();
        assert_eq!(t.current(), page, "{old}");
        let v = t.v.borrow();
        assert!(v.goto.is_none(), "{old}: the scroll was done");
        assert!(v.page.rows().all(|r| r.hl.is_none()), "{old}: an old name flashes nothing");
        let r = v.page.rows().find(|r| r.row.id == row).unwrap_or_else(|| panic!("{old}: {row}"));
        let top = v.page.scroll.offset();
        if old == "pet" {
            // the pet gallery is a long section: the window lands on its start (the gallery), the row under it is further down
            assert!(top > 300.0 && r.rect.y > top, "pet: scrolled to the gallery ({top}), its row is below ({:?})", r.rect);
            continue;
        }
        assert!(r.rect.y >= top - 1.0 && r.rect.bottom() <= top + v.content_rect().h + 1.0, "{old}: {row} is on screen ({:?} at scroll {top})", r.rect);
    }
}

#[test]
fn an_old_name_asked_for_while_the_window_is_open_scrolls_too() {
    let _g = motion_lock();
    let mut t = boot(json!({"language": "en"}), "hotkeys");
    t.v.borrow_mut().show_page("sound", &mut t.sim.cx);
    t.settle();
    assert_eq!(t.current(), "media", "the old Sound page is Media and sound now");
    let v = t.v.borrow();
    assert!(v.goto.is_none(), "the scroll was done");
    let r = v.page.rows().find(|r| r.row.id == "live.volume").expect("the first row of the old page");
    assert!(r.rect.y >= v.page.scroll.offset() - 1.0 && r.rect.bottom() <= v.page.scroll.offset() + v.content_rect().h + 1.0, "it is on screen");
}

/// `PILL_SNAP_DIR=<dir> cargo test tests_adv` also writes the closed and the open section of three pages as PNGs.
#[test]
fn the_section_renders_closed_and_open() {
    let _g = motion_lock();
    for id in ["general", "clipboard", "notifications"] {
        let mut t = boot(json!({"language": "en"}), id);
        scroll_to_end(&mut t);
        let th = Theme::settings_dark();
        let closed = super::tests::snap(&mut t.sim, &format!("adv-{id}-closed"), &th, (840.0, 760.0));
        assert!(closed.count(None, |p| p[3] == 255) > 100_000, "{id}");
        t.v.borrow_mut().set_advanced(true, true);
        scroll_to_end(&mut t);
        scroll_to_end(&mut t); // the first one clamped to the closed page; this one knows how tall it is now
        let open = super::tests::snap(&mut t.sim, &format!("adv-{id}-open"), &th, (840.0, 760.0));
        assert!(open.count(None, |p| p[3] == 255) > 100_000, "{id}");
    }
}
