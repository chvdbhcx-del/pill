//! Settings > Modules > What is on Home, seen from the Home view: the parts that are off disappear, the others glide into the freed place,
//! clicks land on the shifted parts, the tab reports the height its parts need, and "recently copied" reads the clipboard history.
use super::*;
use pill_ui_core::panel::a::home_mods as hmod;

fn off(keys: &[&str]) -> Value {
    // `settings:changed` carries the whole settings: keep the original card style these tests click
    let mut m = serde_json::Map::new();
    m.insert("home.player".into(), json!("compact"));
    for k in keys {
        m.insert((*k).to_string(), json!(false));
    }
    Value::Object(m)
}

fn change(r: &mut Rig, patch: Value) {
    r.f.bus.emit("settings:changed", patch);
    r.settle();
}

fn tiles_off_except(keep: &[&str]) -> Vec<&'static str> {
    hmod::PARTS.iter().filter(|p| p.key.starts_with("home.tile.") && !keep.contains(&p.key)).map(|p| p.key).collect()
}

/// A tile reached the island (capture or command bar), as opposed to the player's own live-row effects.
fn tile_fx(r: &Rig) -> bool {
    r.fx.iter().any(|f| matches!(f, TabFx::StartCapture { .. } | TabFx::OpenCommandBar))
}

fn height(r: &Rig) -> f32 {
    r.sim.view.size_hint().unwrap().1
}

#[test]
fn everything_on_is_the_classic_tab() {
    let r = Rig::playing();
    assert_eq!(r.sim.view.size_hint(), Some((648.0, 260.0)));
}

#[test]
fn only_the_player_shrinks_the_tab_and_the_card_moves_to_the_middle() {
    let mut r = Rig::playing();
    let mut gone = tiles_off_except(&[]);
    gone.extend(["module.launcher", "home.volume"]);
    change(&mut r, off(&gone));
    assert_eq!(height(&r), 154.0);
    let _ = r.sim.render((super::super::super::W, 154.0), &Theme::pill_dark());
    // the classic play button is no longer there, the shifted one is
    r.click(ctr(layout::MAIN));
    assert!(r.calls("media_control").is_empty(), "the old place is empty: {:?}", r.calls("media_control"));
    r.click((layout::MAIN.cx() + 118.0, layout::MAIN.cy()));
    assert_eq!(r.calls("media_control").len(), 1);
    // and the tiles are gone for good
    r.click(ctr(layout::tile(0)));
    assert!(!tile_fx(&r), "{:?}", r.fx);
}

#[test]
fn player_and_one_tile_give_the_tile_the_side_column() {
    let mut r = Rig::playing();
    let mut gone = tiles_off_except(&["home.tile.command"]);
    gone.extend(["module.launcher", "home.volume"]);
    change(&mut r, off(&gone));
    assert_eq!(height(&r), 154.0);
    // the Command tile now fills the side column: its old 110 px box is not what it was, the middle of the column is
    r.click((540.0, 77.0));
    assert!(r.fx.iter().any(|f| matches!(f, TabFx::OpenCommandBar)), "{:?}", r.fx);
}

#[test]
fn a_relayout_glides_and_the_tab_idles_afterwards() {
    let _m = motion(false);
    let mut r = Rig::playing();
    let before = r.sim.render((super::super::super::W, 260.0), &Theme::pill_dark());
    r.f.bus.emit("settings:changed", off(&tiles_off_except(&["home.tile.area", "home.tile.command"])));
    r.sim.pump();
    assert!(r.sim.frame.wants_frame(), "a relayout asks for frames");
    // half way: neither the old picture nor the final one
    for _ in 0..14 {
        r.sim.step_frame();
    }
    let mid = r.sim.render((super::super::super::W, 260.0), &Theme::pill_dark());
    let n = r.sim.run(3000);
    assert!(n > 15 && n < 3000, "a spring over many frames, then rest: {n}");
    assert!(!r.sim.frame.wants_frame(), "idle once everything settled");
    let after = r.sim.render((super::super::super::W, 260.0), &Theme::pill_dark());
    assert!(before.diff(&mid, 8) > 200, "something moved");
    assert!(mid.diff(&after, 8) > 50, "and it was not there yet half way");
}

#[test]
fn reduced_motion_relays_out_at_once() {
    let _m = motion(true);
    let mut r = Rig::playing();
    r.f.bus.emit("settings:changed", off(&tiles_off_except(&[])));
    r.sim.pump();
    r.sim.run(5);
    assert!(!r.sim.frame.wants_frame());
    r.click(ctr(layout::tile(0)));
    assert!(!tile_fx(&r), "{:?}", r.fx);
}

#[test]
fn recently_copied_reads_the_history_and_a_click_copies_an_entry_back() {
    let mut r = Rig::playing();
    r.f.api.reply("clip_list", Ok(json!([
        { "id": 11, "kind": "text", "preview": "older", "fileCount": 0, "pinned": false, "usedAt": 100 },
        { "id": 12, "kind": "text", "preview": "newest\nline", "fileCount": 0, "pinned": false, "usedAt": 300 },
    ])));
    assert!(r.calls("clip_list").is_empty(), "nothing is read while the card is off");
    change(&mut r, json!({ "home.recent": true }));
    assert_eq!(r.calls("clip_list").len(), 1);
    assert_eq!(height(&r), 330.0, "the card makes the tab taller");
    // the card sits under the player: first chip is the newest entry
    let plan = hmod::plan(&hmod::On(std::array::from_fn(|i| hmod::PARTS[i].default || i == 1)));
    let rc = plan.rects[1].unwrap();
    let chip = super::super::recent::Recent::chip_rect(Rect::new(rc.x, rc.y, rc.w, rc.h), 0);
    r.click(ctr(chip));
    assert_eq!(r.calls("clip_paste"), vec![json!({ "id": 12, "copyOnly": true, "plain": false })]);
    // a new entry in the history is read again
    let n = r.calls("clip_list").len();
    r.f.bus.emit("clip:added", json!({}));
    r.settle();
    assert_eq!(r.calls("clip_list").len(), n + 1);
}

#[test]
fn nothing_but_shortcuts_is_a_strip_and_still_takes_clicks() {
    let mut r = Rig::playing();
    let mut gone = tiles_off_except(&[]);
    gone.extend(["module.media", "home.volume"]);
    change(&mut r, off(&gone));
    assert_eq!(height(&r), 52.0);
    // the strip moved from y 208 to 0
    r.click((layout::app(1).cx(), 26.0));
    assert_eq!(r.calls("launcher_open"), vec![json!({ "id": 2 })]);
}

#[test]
fn a_module_that_is_off_takes_no_clicks_even_while_it_fades() {
    let _m = motion(false);
    let mut r = Rig::playing();
    r.f.bus.emit("settings:changed", off(&["home.tile.area"]));
    r.sim.pump();
    r.sim.step_frame();
    // the Area tile is on its way out: a click on it does nothing
    r.mv(ctr(layout::tile(0)));
    r.down(ctr(layout::tile(0)), MouseButton::Left);
    r.up(ctr(layout::tile(0)), MouseButton::Left);
    assert!(r.fx.iter().all(|f| !matches!(f, TabFx::StartCapture { .. })), "{:?}", r.fx);
}

// ------------------------------------------------------------------------------------------------------------------ player styles x parts

fn mirror() -> player::Geo {
    player::Geo::new(pill_ui_core::panel::a::home_player::PlayerStyle::Mirror, false)
}

/// Writes the PNGs of a case when `PILL_TABS_DUMP` is set (a GPU render: not in the default run, parallel devices crash).
fn shot(r: &mut Rig, name: &str) {
    if std::env::var_os("PILL_TABS_DUMP").is_none() {
        return;
    }
    std::thread::sleep(std::time::Duration::from_millis(150)); // the cover decodes on a worker
    r.settle();
    for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
        shoot(&mut *r.sim.view, &r.sim.cx, &th, 1.0, "home", &format!("merge-{name}-{tn}"));
    }
}

/// `off(..)` plus the mirror style (`settings:changed` and `settings_get` carry the whole settings).
fn mirror_cfg(keys: &[&str]) -> Value {
    let mut c = off(keys);
    c["home.player"] = json!("mirror");
    c
}

#[test]
fn the_mirror_card_and_the_volume_row_answer_where_their_slots_are() {
    let _m = motion(true);
    let g = mirror();
    // only the player: the card sits in the middle (x + 118), the mirror's own buttons and seek bar move with it
    let mut gone = tiles_off_except(&[]);
    gone.extend(["module.launcher", "home.volume"]);
    let mut r = Rig::with_cfg(Some(true), true, 4, mirror_cfg(&gone), None);
    assert_eq!(height(&r), 154.0);
    shot(&mut r, "only-player");
    r.click(ctr(g.main));
    assert!(r.calls("media_control").is_empty(), "the old place is empty");
    r.click((g.main.cx() + 118.0, g.main.cy()));
    assert_eq!(r.calls("media_control").len(), 1, "the mirror's play button, shifted");
    r.click((g.seek.x + g.seek.w / 2.0 + 118.0, g.seek.cy()));
    let ms = r.calls("media_seek")[0]["ms"].as_i64().unwrap();
    assert!((ms - 122_000).abs() < 8_000, "half of 244 s, got {ms}");
    // player + area tile + volume: the chip switches the target, the bar sets the volume, the tile fills the side column
    let mut gone = tiles_off_except(&["home.tile.area"]);
    gone.push("module.launcher");
    let mut r = Rig::with_cfg(Some(true), true, 4, mirror_cfg(&gone), None);
    assert_eq!(height(&r), 200.0);
    shot(&mut r, "player-area-volume");
    r.click(ctr(target::rect()));
    assert_eq!(r.calls("settings_set").len(), 1, "the computer / player chip");
    r.click((layout::VOL.x + layout::VOL.w * 0.5, layout::VOL.cy()));
    assert!(!r.calls("media_volume_set").is_empty(), "the bar");
    r.click((540.0, 100.0));
    assert!(tile_fx(&r), "{:?}", r.fx);
    // everything on with "recently copied": the volume row sits under the card and the card (a 4-line stack), 8 + 62 + 8 lower
    let mut r = Rig::with_cfg(Some(true), true, 5, json!({ "home.recent": true, "home.player": "mirror" }), None);
    shot(&mut r, "everything");
    r.click(ctr(g.next));
    assert_eq!(r.calls("media_control").len(), 1, "the card is first in the stack");
    r.click((layout::VOL.x + layout::VOL.w * 0.5, layout::VOL.cy() + 70.0));
    assert!(!r.calls("media_volume_set").is_empty(), "the bar follows its slot");
}

#[test]
fn the_device_list_stays_inside_the_tab_when_the_volume_row_is_at_the_top() {
    let _m = motion(true);
    // volume + shortcuts only: the row is the top of the tab, so the list hangs below it
    let mut gone = tiles_off_except(&[]);
    gone.push("module.media");
    let mut r = Rig::with_cfg(Some(true), false, 4, mirror_cfg(&gone), None);
    assert_eq!(height(&r), 38.0 + 8.0 + 52.0);
    r.click((layout::DEV.cx() + 118.0, 19.0)); // no tiles: the column is centred, x + 118
    shot(&mut r, "volume-top-menu");
    // the list is cut to the 54 px under the row (one row, the rest scrolls); that row answers a click
    r.click((layout::VOL_ROW.right() + 118.0 - 60.0, 38.0 + 6.0 + 5.0 + layout::MENU_ROW / 2.0));
    assert_eq!(r.calls("media_set_default_device").len(), 1);
    r.click((layout::VOL_ROW.right() + 118.0 - 60.0, 90.0));
    assert_eq!(r.calls("media_set_default_device").len(), 1, "the list is closed now; the strip under it is not a device");
}
