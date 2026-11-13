//! The settings window moves like liquid: pages, the sidebar indicator, the search popup, row messages and the wizard dots all chase
//! their targets on springs. These tests assert behaviour (heads for the target, bounded overshoot, no jumps, retargeting keeps its
//! place, idles afterwards), never a time curve.
use super::tests2::{boot, T};
use super::ui::Act;
use super::*;
use crate::ui::host::testing::motion_on;
use glasskit::input::{KeyCode, Modifiers};
use pill_ui_core::settings::flows::Msg as FMsg;
use serde_json::json;

/// Frames at 240 Hz until the window idles; panics when it never does.
fn idle_within(t: &mut T, ms: f32) -> usize {
    let max = (ms / (1000.0 / 240.0)) as usize;
    let n = t.sim.run(max);
    assert!(n < max, "still moving after {ms} ms");
    assert!(!t.sim.frame.wants_frame(), "the window must idle once everything settled");
    n
}

fn paint(t: &mut T) {
    let _ = t.sim.render((840.0, 760.0), &Theme::settings_dark());
}

#[test]
fn a_page_change_slides_out_and_rises_in_without_jumps_and_idles() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "general");
    t.v.borrow_mut().show_page("hotkeys", &mut t.sim.cx);
    t.sim.frame.request();
    let (mut last_leave, mut leave_frames, mut peak_enter) = (0.0f32, 0, 0.0f32);
    for _ in 0..400 {
        if !t.sim.step_frame() {
            break;
        }
        let v = t.v.borrow();
        if v.leaving.is_some() {
            let x = v.leave_t.at();
            assert!(x >= last_leave - 1e-4 && x - last_leave < 0.3, "the exit glides forward: {last_leave} -> {x}");
            last_leave = x;
            leave_frames += 1;
        } else {
            peak_enter = peak_enter.max(v.enter_t.at());
        }
    }
    assert!(leave_frames > 5, "the old page takes a moment to leave ({leave_frames} frames)");
    assert!(peak_enter > 0.9 && peak_enter < 1.3, "the new page rises to its place with at most a small overshoot: {peak_enter}");
    idle_within(&mut t, 1500.0);
    let v = t.v.borrow();
    assert_eq!(v.model.current, "hotkeys");
    assert!(v.leaving.is_none() && (v.enter_t.at() - 1.0).abs() < 0.01);
}

#[test]
fn a_page_cancelled_while_rising_turns_around_where_it_is() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "general");
    t.v.borrow_mut().show_page("hotkeys", &mut t.sim.cx);
    t.sim.frame.request();
    for _ in 0..600 {
        t.sim.step_frame();
        let v = t.v.borrow();
        if v.leaving.is_none() && v.enter_t.at() > 0.4 && v.enter_t.at() < 0.8 {
            break;
        }
    }
    let e = t.v.borrow().enter_t.at01();
    assert!(e > 0.4 && e < 0.8, "caught mid-rise ({e})");
    t.v.borrow_mut().show_page("look", &mut t.sim.cx);
    let (leave, moving) = {
        let v = t.v.borrow();
        (v.leave_t.at(), v.leave_t.moving())
    };
    assert!((leave - (1.0 - e)).abs() < 0.01, "the exit starts where the entrance was: {leave} vs {}", 1.0 - e);
    assert!(moving);
    t.sim.frame.request();
    idle_within(&mut t, 1500.0);
    assert_eq!(t.v.borrow().page.id, "look");
}

#[test]
fn the_sidebar_indicator_glides_to_the_new_page_with_a_bounded_overshoot() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "general");
    let from = t.v.borrow().side.ind_y.at();
    let to = chrome::Side::item_y("hotkeys");
    t.v.borrow_mut().show_page("hotkeys", &mut t.sim.cx);
    t.sim.frame.request();
    assert_eq!(t.v.borrow().side.ind_y.at(), from, "it does not jump");
    let mut peak = from;
    let mut prev = from;
    for _ in 0..400 {
        if !t.sim.step_frame() {
            break;
        }
        let y = t.v.borrow().side.ind_y.at();
        assert!((y - prev).abs() < (to - from).abs() * 0.5, "no jump between frames: {prev} -> {y}");
        prev = y;
        peak = peak.max(y);
    }
    assert!(peak > to - 1.0 && peak < to + (to - from) * 0.3, "overshoots a little, not a lot: peak {peak}, target {to}");
    assert_eq!(t.v.borrow().side.ind_y.at(), to, "settles exactly on the item");
}

#[test]
fn the_search_popup_flows_shut_keeping_its_rows_and_then_lets_the_window_idle() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "general");
    t.key(KeyCode::Char('F'), Modifiers::CTRL);
    t.text("quiet hours");
    assert!(t.v.borrow().model.search.open && !t.v.borrow().search.rows.is_empty());
    idle_within(&mut t, 2000.0);
    assert!((t.v.borrow().search.pop.at() - 1.0).abs() < 0.01, "open");
    t.v.borrow_mut().search_close_silently();
    t.sim.frame.request();
    t.sim.step_frame();
    t.sim.step_frame();
    {
        let v = t.v.borrow();
        assert!(v.search.shown(false), "still on screen while it closes");
        assert!(!v.search.rows.is_empty(), "its rows stay until it has gone");
        assert!(v.search.pop.at() < 1.0 && v.search.pop.at() > 0.0, "on its way out: {}", v.search.pop.at());
    }
    idle_within(&mut t, 1500.0);
    let v = t.v.borrow();
    assert!(!v.search.shown(false) && v.search.rows.is_empty());
}

#[test]
fn a_row_message_opens_its_room_smoothly_and_closes_it_again() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "general");
    paint(&mut t);
    let id = t.v.borrow().page.rows().find(|r| matches!(r.ctl, page::Ctl::Toggle(_))).map(|r| r.row.id).expect("a toggle row");
    let h0 = |t: &T| t.v.borrow().page.rows().find(|r| r.row.id == id).map(|r| r.mh.at()).unwrap();
    assert_eq!(h0(&t), 0.0);
    t.v.borrow_mut().row_message(id, FMsg::plain("Saved. This takes effect right away."));
    t.sim.frame.request();
    let mut seen = Vec::new();
    for _ in 0..400 {
        if !t.sim.step_frame() {
            break;
        }
        paint(&mut t);
        seen.push(h0(&t));
    }
    let need = t.v.borrow().page.rows().find(|r| r.row.id == id).map(|r| r.mh.target()).unwrap();
    assert!(need > 10.0, "the lines need room ({need})");
    assert!(seen.iter().any(|h| *h > 1.0 && *h < need - 1.0), "it passes through the in-between heights: {:?}", &seen[..seen.len().min(12)]);
    assert!(seen.iter().all(|h| *h < need * 1.3), "bounded overshoot");
    assert!((h0(&t) - need).abs() < 0.1, "ends exactly open");
    idle_within(&mut t, 300.0);
    t.v.borrow_mut().row_message(id, FMsg::plain(""));
    t.sim.frame.request();
    for _ in 0..400 {
        if !t.sim.step_frame() {
            break;
        }
        paint(&mut t);
    }
    assert!(h0(&t).abs() < 0.1, "closed again");
    let v = t.v.borrow();
    assert!(v.page.rows().find(|r| r.row.id == id).unwrap().msg.is_none(), "the faded message is dropped once it has gone");
}

#[test]
fn the_wizard_dot_pops_to_the_new_step_and_settles() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "welcome");
    idle_within(&mut t, 2000.0);
    let r = t.v.borrow().hit_rect(&Act::WizNext).unwrap();
    super::tests::click(&mut t.sim, r.cx(), r.cy());
    // the click already ran the frames: the second dot is now the current one and has landed
    let v = t.v.borrow();
    assert_eq!(v.model.wizard.step(), 2);
    assert!(!v.wiz.dots[1].moving() && (v.wiz.dots[1].at() - 1.5).abs() < 0.01);
    assert!((v.wiz.dots[0].at() - 1.0).abs() < 0.01);
}

#[test]
fn reduce_motion_lands_everything_at_once() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en", "reduceMotion": true}), "general");
    t.v.borrow_mut().show_page("hotkeys", &mut t.sim.cx);
    let v = t.v.borrow();
    assert!(v.leaving.is_none() && !v.enter_t.moving() && !v.side.ind_y.moving());
    assert_eq!(v.side.ind_y.at(), chrome::Side::item_y("hotkeys"));
}

#[test]
fn the_window_ends_fully_opaque_after_it_is_shown_on_every_page() {
    let _m = motion_on();
    for id in super::tests::all_page_ids() {
        let mut t = boot(json!({"language": "en"}), id);
        t.sim.frame.request();
        t.sim.run(480); // two seconds at 240 Hz: some pages keep a live clock going, so only the window opacity is asserted
        let k = t.v.borrow().win_t.at();
        assert!((k - 1.0).abs() < 0.01, "{id}: the window stayed at {k} opacity");
    }
}

#[test]
fn a_fade_that_never_finished_cannot_leave_the_window_washed_out() {
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "updates");
    t.sim.run(480);
    // the steps stopped reaching the fade half way: the spring says 0.4 and nobody advances it
    let now = t.sim.cx.now_ms;
    {
        let mut v = t.v.borrow_mut();
        v.win_t.jump(0.4);
        v.shown_ms = now; // just shown
    }
    paint(&mut t);
    assert!((t.v.borrow().win_t.at() - 0.4).abs() < 1e-3, "inside the first 1.2 s after show nothing is forced");
    t.sim.cx.now_ms += 5_000.0;
    paint(&mut t);
    assert!((t.v.borrow().win_t.at() - 1.0).abs() < 1e-3, "an old enough window is fully opaque");
}

#[test]
fn the_updates_page_does_not_ask_for_its_info_over_and_over_and_ends_fully_opaque() {
    // every answer to `settings_info` used to rebuild the page, which asked for it again: the page's fade-in restarted at 0.5 forever
    let _m = motion_on();
    let mut t = boot(json!({"language": "en"}), "updates");
    for _ in 0..40 {
        t.sim.pump();
        t.sim.run(12);
    }
    let asked = t.fakes.api.calls().iter().filter(|c| c.name == "settings_info").count();
    assert!(asked <= 3, "settings_info was asked {asked} times");
    let e = t.v.borrow().enter_t.at();
    assert!((e - 1.0).abs() < 0.01, "the page stayed at {e} opacity");
}
