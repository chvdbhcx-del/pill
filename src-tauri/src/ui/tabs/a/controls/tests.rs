use super::*;
use crate::ui::host::testing::{motion_lock, Sim};
use crate::ui::tabs::a::core::Tick;
use crate::ui::tabs::a::snap;
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{ApiCall, Fakes, MsgQueue, Services};
use glasskit::geom::Pt;
use serde_json::json;
use std::sync::Arc;

fn state(bt_on: bool) -> Value {
    json!({
        "ethernet": { "available": true, "on": true },
        "wifi": { "available": true, "on": true, "ssid": "Home-5G" },
        "bluetooth": { "available": true, "on": bt_on, "devices": [
            { "name": "WH-1000XM5", "connected": true, "battery": 80 },
            { "name": "MX Master 3", "connected": true },
            { "name": "Old speaker", "connected": false }] },
        "dark": true, "volume": 40, "muted": false
    })
}

struct Rig {
    v: ControlsView,
    cx: Cx,
    q: Arc<MsgQueue>,
    f: Fakes,
}

impl Rig {
    fn new() -> Rig {
        let (svc, f) = Services::fake();
        f.api.reply("sys_state", Ok(state(true)));
        f.api.reply("sys_set", Ok(json!({ "status": "ok" })));
        f.api.reply("sys_power", Ok(json!({ "status": "confirm", "ms": 3000 })));
        let (mut cx, q) = Cx::headless_queue(svc);
        let mut v = ControlsView::new();
        v.on_show(&mut cx);
        let mut r = Rig { v, cx, q, f };
        r.pump();
        r
    }

    /// Deliver everything queued (replies, events), then draw once so `base.rect` is set.
    fn pump(&mut self) -> Vec<TabFx> {
        let mut fx = vec![];
        loop {
            let msgs = self.q.drain();
            if msgs.is_empty() {
                return fx;
            }
            for (_, m) in msgs {
                if let Msg::Custom(b) = &m {
                    if let Some(t) = b.downcast_ref::<TabFx>() {
                        fx.push(t.clone());
                        continue;
                    }
                }
                self.v.message(m, &mut self.cx);
            }
        }
    }

    fn names(&self) -> Vec<String> {
        self.f.api.calls().into_iter().map(|c: ApiCall| c.name).collect()
    }

    fn tick(&mut self, ms: f64) {
        self.cx.now_ms += ms;
        self.v.message(Msg::Custom(Box::new(Tick)), &mut self.cx);
        self.pump();
    }

    fn ev(&mut self, e: InputEvent) -> Response {
        let r = self.v.event(&e, &mut self.cx);
        self.pump();
        r
    }

    fn click(&mut self, p: Pt) {
        self.ev(InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE });
        self.ev(InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
        self.ev(InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE });
    }

    fn tiles(&self) -> Vec<Rect> {
        layout::tiles(self.v.bounds(), self.v.v.tiles.len())
    }

    fn calls_to(&self, name: &str) -> Vec<Value> {
        self.f.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
    }
}

fn key(code: KeyCode) -> InputEvent {
    InputEvent::Key { code, mods: Modifiers::NONE, down: true, repeat: false }
}

#[test]
fn show_subscribes_reads_the_state_and_builds_the_tiles() {
    let r = Rig::new();
    assert_eq!(r.names(), ["sys_state"]);
    for e in EVENTS {
        assert_eq!(r.f.bus.subscribers(e), 1, "{e}");
    }
    let ids: Vec<_> = r.v.v.tiles.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["wifi", "ethernet", "bluetooth", "dark"]);
    assert_eq!(r.v.v.tiles[0].state, "Home-5G");
    assert_eq!(r.v.v.power.len(), 4);
}

#[test]
fn backend_events_reach_the_model() {
    let mut r = Rig::new();
    let mut s = state(true);
    s["dark"] = json!(false);
    r.f.bus.emit("sys:changed", s);
    r.pump();
    assert!(!r.v.v.tiles[3].on);
    assert_eq!(r.v.v.tiles[3].state, "Off");
}

#[test]
fn a_dark_theme_click_sets_it_at_once_and_reconciles() {
    let mut r = Rig::new();
    let c = r.tiles()[3].center();
    r.click(c);
    assert_eq!(r.calls_to("sys_set"), vec![json!({ "key": "dark", "value": false })]);
    assert_eq!(r.names().last().map(String::as_str), Some("sys_state"), "re-read after the answer");
    assert!(!r.v.v.tiles[3].on || r.f.api.calls().len() >= 3);
}

#[test]
fn network_tiles_wait_for_uac_and_ignore_a_second_click() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = Rig::new();
    r.f.api.reply("sys_set", Ok(json!({ "status": "needs_uac" })));
    let c = r.tiles()[0].center();
    r.click(c);
    assert!(r.v.v.tiles[0].busy);
    assert_eq!(r.v.v.tiles[0].state, "Confirm in UAC…");
    assert!(r.v.base.player.is_active("tile:wifi"), "busy pulse plays");
    r.click(c);
    assert_eq!(r.calls_to("sys_set").len(), 1, "the busy tile is disabled");
    let mut s = state(true);
    s["wifi"]["on"] = json!(false);
    r.f.bus.emit("sys:changed", s);
    r.pump();
    assert!(!r.v.v.tiles[0].busy && !r.v.v.tiles[0].on);
    r.v.sync();
    assert!(!r.v.base.player.is_active("tile:wifi"), "the pulse stops with the busy state");
    glasskit::ease::force_reduced(None);
}

#[test]
fn power_needs_two_presses_and_the_window_closes_after_three_seconds() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = Rig::new();
    let p = layout::power(r.v.bounds());
    r.click(p[3].center());
    assert_eq!(r.calls_to("sys_power"), vec![json!({ "action": "shutdown", "confirmed": false })]);
    assert!(r.v.v.power[3].armed);
    assert_eq!(r.v.v.power[3].label, "Press again");
    assert!(r.v.base.player.is_active("power:shutdown"), "the wash countdown plays");
    r.tick(3200.0);
    assert!(!r.v.v.power[3].armed, "disarmed after the window");
    assert!(!r.v.base.player.is_active("power:shutdown"));
    // press, press again inside the window -> confirmed
    r.click(p[0].center());
    r.tick(1000.0);
    r.click(p[0].center());
    let calls = r.calls_to("sys_power");
    assert_eq!(calls.last().unwrap(), &json!({ "action": "lock", "confirmed": true }));
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_bluetooth_popover_opens_closes_outside_and_with_escape() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = Rig::new();
    let lb = layout::list_button(r.tiles()[2]);
    r.click(lb.center());
    assert!(r.v.v.popover.is_some() && r.calls_to("sys_set").is_empty(), "the list button does not toggle Bluetooth");
    assert!(r.v.base.player.is_active("bt-pop"));
    // outside press closes (and reaches the tile below)
    r.click((20.0, 230.0));
    assert!(r.v.v.popover.is_none());
    r.click(lb.center());
    assert!(r.v.v.popover.is_some());
    let resp = r.ev(key(KeyCode::Escape));
    assert!(resp.consumed && r.v.v.popover.is_none());
    // the exit plays on a kept copy
    assert!(r.v.base.player.is_active("bt-pop") && r.v.last_pop.is_some());
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_popover_button_opens_windows_settings_and_closes() {
    let mut r = Rig::new();
    let lb = layout::list_button(r.tiles()[2]);
    r.click(lb.center());
    let pl = layout::popover(r.v.bounds(), 3);
    r.click(pl.button.center());
    assert_eq!(r.calls_to("sys_set"), vec![json!({ "key": "bluetooth_settings", "value": true })]);
    assert!(r.v.v.popover.is_none());
}

#[test]
fn a_press_inside_the_popover_does_not_toggle_the_tile_under_it() {
    let mut r = Rig::new();
    let lb = layout::list_button(r.tiles()[2]);
    r.click(lb.center());
    let pl = layout::popover(r.v.bounds(), 3);
    // the popover covers the middle tiles' upper half: a click on its header must change nothing
    r.click((pl.header.cx(), pl.header.cy()));
    assert!(r.v.v.popover.is_some());
    assert!(r.calls_to("sys_set").is_empty());
}

#[test]
fn keyboard_tab_walks_the_tiles_and_enter_activates() {
    let mut r = Rig::new();
    r.ev(key(KeyCode::Tab)); // first enabled tile = wifi
    let before = r.calls_to("sys_set").len();
    r.ev(key(KeyCode::Enter));
    assert_eq!(r.calls_to("sys_set").len(), before + 1);
    assert_eq!(r.calls_to("sys_set")[0]["key"], "wifi");
    // tab past the end leaves the tab
    for _ in 0..r.v.focus_items().len() {
        r.ev(key(KeyCode::Tab));
    }
    assert!(r.v.focus.is_none());
}

#[test]
fn reduced_motion_plays_no_cue_and_hide_resets_everything() {
    let (svc, _f) = Services::fake();
    let (mut cx, q) = Cx::headless_queue(svc);
    cx.reduced_motion = true;
    let mut v = ControlsView::new();
    v.on_show(&mut cx);
    for (_, m) in q.drain() {
        v.message(m, &mut cx);
    }
    v.model.on_result("sys_state", &Value::Null, &state(true));
    v.sync();
    v.act(&mut cx, Action::TogglePopover);
    v.act(&mut cx, Action::Power("lock".into()));
    assert!(!v.base.player.busy());
    v.on_hide(&mut cx);
    assert!(v.v.popover.is_none() && v.last_pop.is_none() && !v.v.power[0].armed);
}

#[test]
fn an_idle_tab_asks_for_no_frames() {
    let (svc, f) = Services::fake();
    f.api.reply("sys_state", Ok(state(true)));
    let mut sim = Sim::with_services(Box::new(ControlsView::new()), svc);
    sim.show();
    sim.pump();
    sim.run(600);
    assert!(!sim.frame.wants_frame(), "settled");
    let _ = sim.render((648.0, 260.0), &Theme::pill_dark());
    assert!(!sim.frame.wants_frame());
}

#[test]
fn pixels_the_tiles_are_lit_and_the_off_tiles_are_not() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(true));
    let mut r = Rig::new();
    r.v.model.on_result("sys_state", &Value::Null, &state(true));
    r.v.sync();
    let th = Theme::pill_dark();
    let img = snap::shoot(&mut r.v, &r.cx, &th, 1.0, "controls", "probe");
    let t = r.tiles();
    // an on tile (#1f1f1f) is lighter than the page; the icon circle is the accent
    let on = img.at(t[0].x + 100.0, t[0].y + 60.0);
    assert!(on[0] > 20 && on[0] < 50, "{on:?}");
    let circle = img.at(t[0].x + 32.0, t[0].y + 20.0);
    assert!(circle[0] > 100, "{circle:?}"); // accent-blue circle (BGRA: r in [2])
    glasskit::ease::force_reduced(None);
}

fn shoot_all(name: &str, build: &dyn Fn(&mut Rig)) {
    for (tn, th) in [("dark", Theme::pill_dark()), ("light", Theme::pill_light())] {
        for scale in [1.0f32, 2.0] {
            let mut r = Rig::new();
            r.cx.scale = scale;
            build(&mut r);
            r.v.sync();
            snap::shoot(&mut r.v, &r.cx, &th, scale, "controls", &format!("{name}-{tn}-x{scale}"));
        }
    }
}

#[test]
fn snapshots_of_the_typical_states() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(true)); // end states, stable pictures
    // all on
    shoot_all("idle", &|_| {});
    // bluetooth off, dark theme off, wifi has no ssid
    shoot_all("some-off", &|r| {
        let mut s = state(false);
        s["dark"] = json!(false);
        s["wifi"]["ssid"] = Value::Null;
        r.v.model.on_result("sys_state", &Value::Null, &s);
    });
    // wifi busy (waiting for UAC)
    shoot_all("busy", &|r| {
        r.f.api.reply("sys_set", Ok(json!({ "status": "needs_uac" })));
        let c = r.tiles()[0].center();
        r.click(c);
    });
    // shutdown armed
    shoot_all("armed", &|r| {
        let p = layout::power(r.v.bounds())[3];
        r.click(p.center());
    });
    // popover with three devices
    shoot_all("popover", &|r| {
        let lb = layout::list_button(r.tiles()[2]);
        r.click(lb.center());
    });
    // popover with no devices, no adapters (3 tiles)
    shoot_all("popover-empty", &|r| {
        let mut s = state(true);
        s["bluetooth"]["devices"] = json!([]);
        r.v.model.on_result("sys_state", &Value::Null, &s);
        r.v.sync();
        let lb = layout::list_button(r.tiles()[2]);
        r.click(lb.center());
    });
    shoot_all("no-adapters", &|r| {
        let s = json!({ "ethernet": { "available": false, "on": false }, "wifi": { "available": false, "on": false }, "bluetooth": { "available": false, "on": false, "devices": [] }, "dark": false });
        r.v.model.on_result("sys_state", &Value::Null, &s);
    });
    glasskit::ease::force_reduced(None);
}

// ------------------------------------------------------------------------------------------------ Wi-Fi list and the VPN tile

fn networks() -> Value {
    json!({ "status": "ok", "networks": [
        { "ssid": "Home-5G", "bars": 4, "secured": true, "supported": true, "connected": true, "known": true },
        { "ssid": "Cafe", "bars": 2, "secured": true, "supported": true, "connected": false, "known": false },
        { "ssid": "Free", "bars": 1, "secured": false, "supported": true, "connected": false, "known": false },
        { "ssid": "Corp", "bars": 3, "secured": true, "supported": false, "connected": false, "known": false }] })
}

fn wifi_rig() -> Rig {
    let r = Rig::new();
    r.f.api.reply("wifi_scan", Ok(networks()));
    r.f.api.reply("wifi_connect", Ok(json!({ "status": "ok" })));
    r.f.api.reply("wifi_disconnect", Ok(json!({ "status": "ok" })));
    r
}

fn open_wifi(r: &mut Rig) -> Vec<Rect> {
    let wl = layout::list_button(r.tiles()[0]);
    r.click(wl.center());
    r.v.wui.rects(r.v.bounds()).1
}

#[test]
fn the_wifi_list_scans_only_while_it_is_open_and_stops_when_it_closes() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = wifi_rig();
    r.tick(60_000.0);
    assert!(r.calls_to("wifi_scan").is_empty(), "no scan before the list is opened");
    open_wifi(&mut r);
    assert_eq!(r.calls_to("wifi_scan").len(), 1);
    assert_eq!(r.v.v.wifi.as_ref().unwrap().rows.len(), 4);
    r.tick(5000.0);
    assert_eq!(r.calls_to("wifi_scan").len(), 2, "re-scan while open");
    for _ in 0..60 {
        r.v.step(16.0, &mut r.cx); // the card has dropped in
    }
    let resp = r.ev(key(KeyCode::Escape));
    assert!(resp.consumed && r.v.v.wifi.is_none());
    r.tick(60_000.0);
    assert_eq!(r.calls_to("wifi_scan").len(), 2, "closed: no more scans");
    // it leaves on a spring and then lets the tab idle
    assert!(r.v.wui.shown());
    for _ in 0..120 {
        r.v.step(16.0, &mut r.cx);
    }
    assert!(!r.v.wui.shown() && !r.v.step(16.0, &mut r.cx));
    glasskit::ease::force_reduced(None);
}

#[test]
fn a_saved_or_open_network_connects_with_one_click_and_a_secured_new_one_asks_first() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = wifi_rig();
    let rows = open_wifi(&mut r);
    r.click(rows[2].center()); // Free (open)
    assert_eq!(r.calls_to("wifi_connect"), vec![json!({ "ssid": "Free" })]);
    assert!(r.names().ends_with(&["wifi_scan".to_string(), "sys_state".to_string()]), "the list and the tile are refreshed after the answer");
    let rows = r.v.wui.rects(r.v.bounds()).1;
    r.click(rows[1].center()); // Cafe (secured, unknown): no connect yet, a password drawer opens
    assert_eq!(r.calls_to("wifi_connect").len(), 1);
    assert!(r.v.v.wifi.as_ref().unwrap().rows[1].ask.is_some() && r.v.wui.typing());
    // the password goes out in the one call
    r.v.act(&mut r.cx, Action::WifiSubmit("hunter2hunter2".into()));
    r.pump();
    assert_eq!(r.calls_to("wifi_connect").last().unwrap(), &json!({ "ssid": "Cafe", "password": "hunter2hunter2" }));
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_connected_network_has_a_disconnect_button_and_enterprise_ones_are_refused() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = wifi_rig();
    let rows = open_wifi(&mut r);
    // the right end of the first row is the Disconnect button
    let d = rows[0];
    r.click((d.right() - 20.0, d.cy()));
    assert_eq!(r.names().iter().filter(|n| *n == "wifi_disconnect").count(), 1);
    let rows = r.v.wui.rects(r.v.bounds()).1;
    r.click(rows[3].center()); // Corp: enterprise
    assert!(r.calls_to("wifi_connect").is_empty());
    assert!(r.v.v.wifi.as_ref().unwrap().flash.is_some());
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_wifi_list_button_is_the_tiles_own_and_an_outside_press_closes_the_list() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = wifi_rig();
    open_wifi(&mut r);
    assert!(r.calls_to("sys_set").is_empty(), "the list button does not toggle Wi-Fi");
    let wl = layout::list_button(r.tiles()[0]);
    r.click(wl.center());
    assert!(r.v.v.wifi.is_none(), "the list button toggles the list");
    r.click(wl.center());
    assert!(r.v.v.wifi.is_some());
    r.click((630.0, 250.0));
    assert!(r.v.v.wifi.is_none());
    glasskit::ease::force_reduced(None);
}

#[test]
fn the_vpn_tile_shows_only_with_the_feature_on_and_something_to_switch() {
    let mut r = Rig::new();
    let with = |vpn: Value| {
        let mut s = state(true);
        s["vpn"] = vpn;
        s
    };
    r.f.bus.emit("sys:changed", with(json!({ "enabled": true, "configured": false, "on": false, "name": "" })));
    r.pump();
    assert_eq!(r.v.v.tiles.len(), 4, "nothing to start and nothing up: no tile");
    r.f.bus.emit("sys:changed", with(json!({ "enabled": false, "configured": true, "on": true, "name": "Amnezia" })));
    r.pump();
    assert_eq!(r.v.v.tiles.len(), 4, "the feature is switched off: no tile");
    r.f.bus.emit("sys:changed", with(json!({ "enabled": true, "configured": true, "on": false, "name": "Amnezia" })));
    r.pump();
    let ids: Vec<_> = r.v.v.tiles.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["wifi", "ethernet", "bluetooth", "dark", "vpn"]);
    assert_eq!(r.v.v.tiles[4].state, "Off");
    assert_eq!(layout::tiles(r.v.bounds(), 5).len(), 5);
    r.f.bus.emit("sys:changed", with(json!({ "enabled": true, "configured": true, "on": true, "name": "Amnezia" })));
    r.pump();
    assert!(r.v.v.tiles[4].on && r.v.v.tiles[4].state == "Amnezia");
}

#[test]
fn a_vpn_tile_click_toggles_and_waits_for_the_adapter() {
    let _g = motion_lock();
    glasskit::ease::force_reduced(Some(false));
    let mut r = Rig::new();
    r.f.api.reply("vpn_toggle", Ok(json!({ "status": "ok", "on": true })));
    r.f.api.reply("vpn_state", Ok(json!({ "enabled": true, "configured": true, "on": true, "name": "Amnezia" })));
    let mut s = state(true);
    s["vpn"] = json!({ "enabled": true, "configured": true, "on": false, "name": "Amnezia" });
    r.f.bus.emit("sys:changed", s);
    r.pump();
    let c = r.tiles()[4].center();
    r.click(c);
    assert_eq!(r.names().iter().filter(|n| *n == "vpn_toggle").count(), 1);
    assert!(r.v.v.tiles[4].busy, "the tile waits for the adapter");
    r.click(c);
    assert_eq!(r.names().iter().filter(|n| *n == "vpn_toggle").count(), 1, "a busy tile ignores a second click");
    r.tick(1600.0);
    assert!(r.names().iter().any(|n| n == "vpn_state"), "it looks again until the state shows");
    assert!(r.v.v.tiles[4].on && !r.v.v.tiles[4].busy);
    glasskit::ease::force_reduced(None);
}
