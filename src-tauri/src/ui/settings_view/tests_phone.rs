//! Settings > Phone: the switch, the QR area, the paired phones (Revoke needs a second click) and the app chips. All against canned backend answers.
use super::pal::settings_light;
use super::tests2::{boot_with, T};
use super::ui::Act;
use crate::ui::host::testing::{motion_lock, motion_on};
use glasskit::theme::Theme;
use serde_json::{json, Value};

const URL: &str = "http://192.168.1.5:41717/p?c=AB3D9XKQ";

fn status() -> Value {
    json!({
        "enabled": true, "running": true, "ip": "192.168.1.5", "port": 41717, "pcName": "DESK", "connected": 1,
        "devices": [{"id": "dev-1", "name": "Pixel 8", "model": "Pixel 8", "os": "Android 15", "lastSeen": 1, "connected": true, "battery": 87, "charging": false}]
    })
}

fn boot_phone(settings: Value) -> T {
    boot_with(settings, "phone", |f| {
        f.api
            .reply("phone_status", Ok(status()))
            .reply("phone_pair_code", Ok(json!({"code": "AB3D9XKQ", "url": URL, "expires": i64::MAX / 2})))
            .reply("phone_apps", Ok(json!({"apps": {"org.telegram.messenger": "Telegram", "com.spam": "Spam"}, "muted": ["com.spam"]})))
            .reply("phone_revoke", Ok(json!(true)));
    })
}

/// Scrolls until the hit of `a` is on screen, then clicks it.
fn click_act(t: &mut T, a: &Act) {
    for step in 0..40 {
        t.v.borrow_mut().page.scroll.scroll_to(step as f32 * 60.0, false);
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

#[test]
fn the_page_renders_off_and_on_in_both_themes() {
    let _g = motion_lock();
    for settings in [json!({}), json!({"phone.enabled": true})] {
        let mut t = boot_phone(settings);
        for th in [Theme::settings_dark(), settings_light()] {
            let _ = t.sim.render((840.0, 760.0), &th);
        }
    }
}

#[test]
fn off_asks_for_the_status_but_never_for_a_pairing_code() {
    let _g = motion_lock();
    let t = boot_phone(json!({}));
    assert!(!t.calls("phone_status").is_empty());
    assert!(t.calls("phone_pair_code").is_empty(), "it errors while the link is off, so it is never asked");
    assert!(t.v.borrow().data.phone.code.is_none());
}

#[test]
fn on_shows_the_code_the_phone_and_the_apps() {
    let _g = motion_lock();
    let t = boot_phone(json!({"phone.enabled": true}));
    assert_eq!(t.calls("phone_pair_code").len(), 1);
    let v = t.v.borrow();
    assert_eq!(v.data.phone.code.as_ref().map(|c| c.url.as_str()), Some(URL));
    assert_eq!(v.data.phone.rows.len(), 1);
    assert_eq!(v.data.phone.apps.iter().map(|a| a.1.as_str()).collect::<Vec<_>>(), ["Spam", "Telegram"]);
}

#[test]
fn the_switch_writes_phone_enabled() {
    let _g = motion_lock();
    let mut t = boot_phone(json!({}));
    let r = t.v.borrow().ctl_rect_of("phone.enabled").expect("row");
    t.click_at(r);
    assert!(patches(&t).iter().any(|p| p["phone.enabled"] == json!(true)), "{:?}", patches(&t));
    // flipping it on asks for the code at once (the backend answers canned here)
    assert!(!t.calls("phone_pair_code").is_empty());
}

#[test]
fn new_code_asks_for_a_renewed_one() {
    let _g = motion_lock();
    let mut t = boot_phone(json!({"phone.enabled": true}));
    click_act(&mut t, &Act::PhoneRenew);
    assert!(t.calls("phone_pair_code").iter().any(|a| a["renew"] == json!(true)), "{:?}", t.calls("phone_pair_code"));
}

#[test]
fn revoke_needs_a_second_click() {
    let _m = motion_on();
    let mut t = boot_phone(json!({"phone.enabled": true}));
    let a = Act::PhoneRevoke("dev-1".into());
    click_act(&mut t, &a);
    assert!(t.calls("phone_revoke").is_empty(), "the first click only arms it");
    assert!(t.v.borrow().data.phone.armed.is_some());
    t.fakes.api.reply("phone_status", Ok(json!({"running": true, "devices": []}))); // what the server says once the phone is revoked
    click_act(&mut t, &a);
    assert_eq!(t.calls("phone_revoke"), vec![json!({"id": "dev-1"})]);
    assert!(t.v.borrow().data.phone.armed.is_none());
    assert!(t.v.borrow().data.phone.rows.iter().all(|r| !r.present), "the row has left or is leaving");
}

#[test]
fn an_app_chip_mutes_and_unmutes() {
    let _g = motion_lock();
    let mut t = boot_phone(json!({"phone.enabled": true, "phone.mutedApps": ["com.spam"]}));
    click_act(&mut t, &Act::PhoneMute("org.telegram.messenger".into()));
    assert!(patches(&t).iter().any(|p| p["phone.mutedApps"] == json!(["com.spam", "org.telegram.messenger"])), "{:?}", patches(&t));
    click_act(&mut t, &Act::PhoneMute("com.spam".into()));
    assert!(patches(&t).iter().any(|p| p["phone.mutedApps"] == json!(["org.telegram.messenger"])), "{:?}", patches(&t));
}
