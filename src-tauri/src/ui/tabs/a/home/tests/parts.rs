//! Settings > Modules > What is on Home, seen from the Home view: the parts that are off disappear, the others glide into the freed place,
//! clicks land on the shifted parts, the tab reports the height its parts need, and "recently copied" reads the clipboard history.
use super::*;
use pill_ui_core::panel::a::home_mods as hmod;

fn off(keys: &[&str]) -> Value {
    let mut m = serde_json::Map::new();
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
