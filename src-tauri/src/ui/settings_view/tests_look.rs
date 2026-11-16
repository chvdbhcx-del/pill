//! Share your look on the Look page: a look is previewed before it is written, Cancel leaves no trace, Apply writes one patch and the toast's
//! Undo puts every value back; a stranger's look with a forbidden key never gets as far as a preview.
use super::tests2::{boot, T};
use super::ui::Act;
use super::look::LookAct;
use crate::ui::host::testing::{motion_lock, motion_on};
use glasskit::theme::Theme;
use pill_ui_core::settings::look::{allowed, LookError, SAMPLES};
use serde_json::{json, Value};

/// The page paints only what is in view: scroll down until the hit is on screen, then click it.
fn click_in_view(t: &mut T, a: &Act) {
    for step in 0..60 {
        t.v.borrow_mut().page.scroll.scroll_to(step as f32 * 90.0, false);
        t.settle();
        let (hit, clip) = {
            let v = t.v.borrow();
            (v.hit_rect(a), v.ui.page_clip)
        };
        if let Some(r) = hit.filter(|r| r.y > clip.y + 4.0 && r.bottom() < clip.bottom() - 4.0) {
            t.click_at(r);
            return;
        }
    }
    panic!("{a:?} never came into view");
}

fn patches(t: &T) -> Vec<Value> {
    t.calls("settings_set").iter().map(|a| a["patch"].clone()).collect()
}

fn row_msg(t: &T) -> String {
    t.v.borrow().page.rows().find(|r| r.row.id == "look.share").and_then(|r| r.msg.as_ref().map(|m| m.0.text.clone())).unwrap_or_default()
}

#[test]
fn a_built_in_look_is_pending_and_in_the_preview_values_but_nothing_is_written_until_apply() {
    let _m = motion_on();
    let mut t = boot(json!({}), "look");
    click_in_view(&mut t, &Act::Look(LookAct::Sample(0)));
    {
        let mut v = t.v.borrow_mut();
        let plan = v.data.look.plan.clone().expect("Midnight is pending");
        assert_eq!(plan.name, "Midnight");
        assert!(plan.changes > 0);
        // the window's own values are untouched, the preview island is told the look
        assert_eq!(v.model.get("island.color"), json!(""));
        let base = v.model.values().clone();
        let shown = v.data.look.overlay(&base);
        assert_eq!(shown.get("island.color"), Some(&json!("#0b1b3d")));
        assert_eq!(shown.get("pet"), Some(&json!("ghost")));
        assert!(base.get("pet") != Some(&json!("ghost")));
    }
    assert!(patches(&t).is_empty(), "nothing is written by a preview: {:?}", patches(&t));
    assert!(t.v.borrow().data.look.banner.target() > 0.5, "the banner opens");
    // Cancel: the preview gets the old values back once, and still nothing was written
    click_in_view(&mut t, &Act::Look(LookAct::Cancel));
    {
        let mut v = t.v.borrow_mut();
        assert!(v.data.look.plan.is_none());
        let base = v.model.values().clone();
        let back = v.data.look.overlay(&base);
        assert!(matches!(back, std::borrow::Cow::Owned(_)), "one revert pass");
        assert_eq!(back.get("island.color"), Some(&json!("")));
        let again = v.data.look.overlay(&base);
        assert!(matches!(again, std::borrow::Cow::Borrowed(_)), "and then no cost at all");
    }
    assert!(patches(&t).is_empty());
    assert_eq!(t.v.borrow().data.look.banner.target(), 0.0);
}

#[test]
fn apply_writes_one_patch_with_look_keys_only_and_the_toast_undoes_it() {
    let _m = motion_on();
    let mut t = boot(json!({ "theme": "light", "size": 120, "module.media": false }), "look");
    click_in_view(&mut t, &Act::Look(LookAct::Sample(2)));
    click_in_view(&mut t, &Act::Look(LookAct::Apply));
    let ps = patches(&t);
    assert_eq!(ps.len(), 1, "{ps:?}");
    let patch = ps[0].as_object().unwrap();
    assert!(patch.keys().all(|k| allowed(k)), "{patch:?}");
    assert_eq!((patch["theme"].clone(), patch["island.colorCustom"].clone(), patch["character"].clone()), (json!("dark"), json!("#7a2bff"), json!("glow")));
    {
        let v = t.v.borrow();
        assert_eq!(v.model.get("theme"), json!("dark"));
        assert_eq!(v.model.get("module.media"), json!(false), "what is not a look is not touched");
        assert!(v.data.look.plan.is_none() && v.data.look.undo.is_some());
        assert!(v.data.look.toast.target() > 0.5, "the toast comes up");
    }
    // Undo from the toast
    t.click_act(&Act::Look(LookAct::Undo));
    let ps = patches(&t);
    assert_eq!(ps.len(), 2, "{ps:?}");
    assert_eq!((ps[1]["theme"].clone(), ps[1]["size"].clone()), (json!("light"), json!(120)));
    let v = t.v.borrow();
    assert_eq!((v.model.get("theme"), v.model.get("size"), v.model.get("character")), (json!("light"), json!(120), json!("liquid")));
    assert!(v.data.look.undo.is_none());
    assert_eq!(v.data.look.toast.target(), 0.0);
}

#[test]
fn a_pasted_look_with_a_forbidden_key_or_garbage_is_refused_with_a_message() {
    let _m = motion_on();
    let mut t = boot(json!({}), "look");
    let evil = r#"{"kind":"pill-look","version":1,"name":"x","values":{"theme":"dark","module.media":false,"hotkey.toggle":"Ctrl+Q"}}"#;
    let lang = t.v.borrow().lang();
    for (text, want) in [(evil, None), ("hello", Some(LookError::NotALook)), ("pill-look:AAAA", Some(LookError::NotALook)), ("", Some(LookError::Empty))] {
        t.v.borrow_mut().look_stage(text, None);
        t.settle();
        assert!(t.v.borrow().data.look.plan.is_none(), "{text}");
        let m = row_msg(&t);
        match want {
            Some(e) => assert_eq!(m, e.message(lang), "{text:?}"),
            None => assert!(m.contains("module.media") || m.contains("hotkey.toggle"), "the stranger's key is named: {m:?}"),
        }
    }
    assert!(patches(&t).is_empty());
    // a look that changes nothing says so instead of opening a preview
    let own = pill_ui_core::settings::look::to_json(&pill_ui_core::settings::look::export(&t.v.borrow().model, "same"));
    t.v.borrow_mut().look_stage(&own, None);
    t.settle();
    assert!(t.v.borrow().data.look.plan.is_none());
    assert!(row_msg(&t).contains("nothing would change") || row_msg(&t).contains("уже стоят"), "{}", row_msg(&t));
}

#[test]
fn the_look_page_with_a_pending_look_and_with_the_toast_renders() {
    let _g = motion_lock();
    let mut t = boot(json!({}), "look");
    click_in_view(&mut t, &Act::Look(LookAct::Sample(1)));
    t.v.borrow_mut().page.scroll.scroll_to(100_000.0, false);
    t.settle();
    let th = Theme::settings_dark();
    let img = super::tests::snap(&mut t.sim, "look-share-pending", &th, (840.0, 760.0));
    assert!(img.count(None, |p| p[3] == 255) > 100_000);
    click_in_view(&mut t, &Act::Look(LookAct::Apply));
    let img = super::tests::snap(&mut t.sim, "look-share-toast", &th, (840.0, 760.0));
    assert!(img.count(None, |p| p[3] == 255) > 100_000);
    assert!(t.v.borrow().hit_rect(&Act::Look(LookAct::Undo)).is_some(), "the toast's Undo is on screen");
    assert!(SAMPLES.len() == 4);
}
