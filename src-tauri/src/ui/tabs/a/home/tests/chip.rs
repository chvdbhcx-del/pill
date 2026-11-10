//! The computer / player chip of the volume strip (target.rs) through the whole view.
use super::*;

#[test]
fn the_chip_switches_the_bar_to_the_player_and_tags_the_volume_calls() {
    let _m = motion_on_for_chip();
    let mut r = Rig::playing();
    assert!(r.calls("media_volume_set").is_empty());
    r.click(ctr(target::rect()));
    assert_eq!(r.calls("settings_set").last().unwrap(), &json!({ "patch": { "audio.volumeTarget": "player" } }), "the switch is a setting");
    assert_eq!(r.calls("media_volume_get").last().unwrap(), &json!({ "target": "player" }), "the bar re-reads the other volume");
    let v = layout::VOL;
    r.click((v.x + v.w * 0.5, v.cy()));
    let sets = r.calls("media_volume_set");
    assert!(!sets.is_empty() && sets.iter().all(|a| a["target"] == "player"), "{sets:?}");
    r.click(ctr(layout::MUTE));
    assert_eq!(r.calls("media_mute").last().unwrap()["target"], "player");
    // and back
    r.click(ctr(target::rect()));
    assert_eq!(r.calls("settings_set").last().unwrap(), &json!({ "patch": { "audio.volumeTarget": "master" } }));
    let n = r.calls("media_volume_set").len();
    r.click((v.x + v.w * 0.2, v.cy()));
    let sets = r.calls("media_volume_set");
    assert!(sets.len() > n && sets.last().unwrap().get("target").is_none(), "computer mode sends plain calls");
}

#[test]
fn player_mode_ignores_the_master_volume_event() {
    let _m = motion_on_for_chip();
    let mut r = Rig::playing();
    r.click(ctr(target::rect()));
    let probe = (layout::VOL.x + layout::VOL.w * 0.3, layout::VOL.cy()); // inside the fill at 42 %, outside it at 5 %
    let before = r.sim.render((super::super::super::W, 260.0), &Theme::pill_dark()).at(probe.0, probe.1);
    r.f.bus.emit("live:volume", json!({ "level": 5, "muted": false }));
    r.settle();
    let after = r.sim.render((super::super::super::W, 260.0), &Theme::pill_dark()).at(probe.0, probe.1);
    assert_eq!(before, after, "live:volume is the master volume, not the player's");
    // computer mode still follows it
    r.click(ctr(target::rect()));
    r.f.bus.emit("live:volume", json!({ "level": 5, "muted": false }));
    r.settle();
    let low = r.sim.render((super::super::super::W, 260.0), &Theme::pill_dark()).at(probe.0, probe.1);
    assert_ne!(before, low, "the bar follows the master volume again");
}

#[test]
fn the_setting_from_the_settings_page_reaches_the_chip() {
    let _m = motion_on_for_chip();
    let mut r = Rig::playing();
    r.f.bus.emit("settings:changed", json!({ "audio.volumeTarget": "player" })); // the backend broadcasts the whole settings object
    r.settle();
    assert!(r.calls("media_volume_get").iter().any(|a| a["target"] == "player"), "the bar re-reads the player volume");
}

#[test]
fn the_chip_glyph_flows_then_the_tab_idles() {
    let _m = motion_on_for_chip();
    let mut r = Rig::playing();
    r.settle();
    r.click(ctr(target::rect()));
    let frames = r.sim.run(3000);
    assert!(frames < 3000, "the cross-fade settles: {frames}");
    assert!(!r.sim.frame.wants_frame(), "idle once the glyph arrived");
}

fn motion_on_for_chip() -> crate::ui::host::testing::MotionOn {
    crate::ui::host::testing::motion_on()
}
