//! Settings > Modules > "What is on Home" and "Island tabs": the chips write their keys, the schematic relays itself as chips are switched,
//! hidden tabs are a list of known ids.
use super::tests2::boot;
use super::ui::Act;
use crate::ui::host::testing::motion_on;
use pill_ui_core::panel::a::home_mods as hmod;
use serde_json::json;

/// The page paints only what is in view: scroll down until the chip is on screen, then click it.
fn click_chip(t: &mut super::tests2::T, a: &Act) {
    for step in 0..60 {
        t.v.borrow_mut().page.scroll.scroll_to(step as f32 * 120.0, false);
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
    panic!("the chip {a:?} never came into view");
}

fn patches(t: &super::tests2::T) -> Vec<serde_json::Value> {
    t.calls("settings_set").iter().map(|a| a["patch"].clone()).collect()
}

#[test]
fn a_home_chip_writes_its_key_and_a_second_click_takes_it_back() {
    let _m = motion_on();
    let mut t = boot(json!({}), "modules");
    // "recently copied" is off by default, the area tile is on
    click_chip(&mut t, &Act::HomeChip(1));
    assert!(patches(&t).iter().any(|p| p["home.recent"] == json!(true)), "{:?}", patches(&t));
    click_chip(&mut t, &Act::HomeChip(3));
    assert!(patches(&t).iter().any(|p| p["home.tile.area"] == json!(false)), "{:?}", patches(&t));
    click_chip(&mut t, &Act::HomeChip(3));
    assert!(patches(&t).iter().any(|p| p["home.tile.area"] == json!(true)));
    // the player and the shortcuts are the module switches the page already had
    click_chip(&mut t, &Act::HomeChip(0));
    assert!(patches(&t).iter().any(|p| p["module.media"] == json!(false)), "{:?}", patches(&t));
    click_chip(&mut t, &Act::HomeChip(11));
    assert!(patches(&t).iter().any(|p| p["module.launcher"] == json!(false)));
}

#[test]
fn the_schematic_relays_itself_when_chips_are_switched_and_then_idles() {
    let _m = motion_on();
    let mut t = boot(json!({}), "modules");
    {
        let v = t.v.borrow();
        assert_eq!(v.data.home.content_h(), 260.0);
    }
    // everything but the player off: the tab shrinks to the card and the card moves to the middle
    for i in 1..hmod::N {
        let on = t.v.borrow().home_on().0[i];
        if on {
            click_chip(&mut t, &Act::HomeChip(i));
        }
    }
    t.sim.frame.request();
    let n = t.sim.run(3000);
    assert!(n < 3000, "the schematic settles");
    let v = t.v.borrow();
    assert_eq!(v.data.home.content_h(), 154.0);
    assert_eq!(v.data.home.rect(0).map(|r| r.x), Some(118.0));
    assert!(v.data.home.rect(hmod::STRIP).is_none());
}

#[test]
fn tab_chips_hide_and_show_tabs_through_a_list_of_known_ids() {
    let _m = motion_on();
    let mut t = boot(json!({}), "modules");
    click_chip(&mut t, &Act::TabChip("weather"));
    assert!(patches(&t).iter().any(|p| p["tabs.hidden"] == json!(["weather"])), "{:?}", patches(&t));
    click_chip(&mut t, &Act::TabChip("chat"));
    assert!(patches(&t).iter().any(|p| p["tabs.hidden"] == json!(["weather", "chat"])), "{:?}", patches(&t));
    click_chip(&mut t, &Act::TabChip("weather"));
    assert!(patches(&t).iter().any(|p| p["tabs.hidden"] == json!(["chat"])), "{:?}", patches(&t));
    // the model refuses ids that are not tabs the strip can lose (Home never leaves it)
    let mut v = t.v.borrow_mut();
    assert!(v.model.try_set("tabs.hidden", json!(["home"])).is_err());
    assert!(v.model.try_set("tabs.hidden", json!("weather")).is_err());
    assert!(v.model.try_set("home.recent", json!("yes")).is_err());
    assert!(v.model.try_set("home.nope", json!(true)).is_err());
}
