use super::*;
use crate::ui::host::testing::{motion_lock, render_view_cx};
use crate::ui::tabs::a::snap::shoot;
use crate::ui::tabs::fx::TabFx;
use crate::ui::view::{Fakes, MsgQueue, Services};
use glasskit::input::{Buttons, Modifiers, MouseButton};
use serde_json::json;
use std::sync::Arc;

struct Calm(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
impl Drop for Calm {
    fn drop(&mut self) {
        glasskit::ease::force_reduced(None);
    }
}
fn motion(reduced: bool) -> Calm {
    let g = motion_lock();
    glasskit::ease::force_reduced(Some(reduced));
    Calm(g)
}

const BASE_T: i64 = 1_793_368_800_000; // 2026-10-30 14:00 UTC

fn hours(sym: &str, icon: &str) -> Value {
    Value::Array((0..8).map(|i| json!({ "t": BASE_T + i * 3_600_000, "temp": 4.0 + (i as f64) * 0.7, "symbol": sym, "icon": icon, "precip": if icon == "rain" && i % 2 == 0 { 0.4 } else { 0.0 } })).collect())
}

fn days(icon: &str) -> Value {
    let ic = [icon, "cloud", "partly-day", "rain", "snow", "clear-day", "fog"];
    Value::Array((0..7).map(|i| json!({ "t": BASE_T + i * 86_400_000, "dow": (5 + i) % 7, "min": -3.0 + i as f64, "max": 2.0 + (i as f64) * 1.5, "symbol": "x", "icon": ic[i as usize], "precip": 0.0 })).collect())
}

fn forecast(sym: &str, icon: &str, temp: f64) -> Value {
    json!({ "status": "ok", "city": "Oslo", "lat": 59.9, "lon": 10.7, "fetched": 1, "stale": false, "warning": null, "tz": 60,
        "now": { "temp": temp, "feels": temp - 2.5, "symbol": sym, "icon": icon, "wind": 5.2, "dir": 225.0, "humidity": 81.0, "cloud": 90.0, "precip": 0.4, "min": temp - 3.0, "max": temp + 3.0 },
        "hourly": hours(sym, icon), "daily": days(icon),
        "sun": { "state": "normal", "rise": BASE_T - 32_400_000, "set": BASE_T + 3_600_000 * 0 + 1_800_000 } })
}

struct Rig {
    v: WeatherView,
    cx: Cx,
    q: Arc<MsgQueue>,
    f: Fakes,
    fx: Vec<TabFx>,
}

impl Rig {
    fn open(settings: Value, weather: Option<Value>) -> Rig {
        let (svc, f) = Services::fake();
        f.api.reply("settings_get", Ok(settings));
        if let Some(w) = weather {
            f.api.reply("weather_get", Ok(w));
        }
        let (cx, q) = Cx::headless_queue(svc);
        let mut r = Rig { v: WeatherView::new(), cx, q, f, fx: vec![] };
        r.v.on_show(&mut r.cx);
        r.pump();
        r.frame();
        r
    }
    fn on(w: Value) -> Rig {
        Rig::open(json!({ "net.weather": true, "clock24": true }), Some(w))
    }
    fn pump(&mut self) {
        loop {
            let msgs = self.q.drain();
            if msgs.is_empty() {
                break;
            }
            for (_, m) in msgs {
                match m {
                    Msg::Custom(b) if b.is::<TabFx>() => self.fx.push(*b.downcast::<TabFx>().unwrap()),
                    m => {
                        self.v.message(m, &mut self.cx);
                    }
                }
            }
        }
    }
    /// One draw (the layout reads the rect of the last draw; the fields take their edits in it) and frames until calm.
    fn frame(&mut self) {
        let _ = render_view_cx(&mut self.v, &self.cx, (super::super::W, H), 1.0, &Theme::pill_dark());
        for _ in 0..2000 {
            self.cx.now_ms += 4.0;
            if !self.v.step(4.0, &mut self.cx) {
                break;
            }
        }
        self.pump();
    }
    fn calls(&self, name: &str) -> Vec<Value> {
        self.f.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
    }
    fn send(&mut self, ev: InputEvent) {
        self.v.event(&ev, &mut self.cx);
        self.pump();
    }
    fn click(&mut self, p: Pt) {
        self.send(InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE });
        self.send(InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
        self.send(InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE });
        self.frame();
    }
    fn type_text(&mut self, s: &str) {
        for ch in s.chars() {
            self.send(InputEvent::Char(ch));
        }
        self.frame();
    }
}

// ------------------------------------------------------------------------------------------------------------------ wiring

#[test]
fn opt_in_off_fetches_nothing_and_enable_patches_settings_then_forces_a_refresh() {
    let mut r = Rig::open(json!({ "net.weather": false }), None);
    assert!(r.calls("weather_get").is_empty(), "nothing is fetched before the user opts in");
    assert_eq!(r.v.kind, Kind::Off);
    r.f.api.reply("settings_set", Ok(json!({ "net.weather": true })));
    r.f.api.reply("weather_get", Ok(forecast("rain", "rain", 4.4)));
    r.f.api.reply("settings_get", Ok(json!({ "net.weather": true, "clock24": true }))); // what the re-read answers after the patch
    let enable = r.v.card()[4];
    r.click(enable.center());
    assert_eq!(r.calls("settings_set"), vec![json!({ "patch": { "net.weather": true } })]);
    r.frame();
    assert!(r.calls("weather_get").iter().any(|a| a["force"] == true), "{:?}", r.calls("weather_get"));
}

#[test]
fn the_contact_typed_next_to_enable_travels_with_the_opt_in() {
    let mut r = Rig::open(json!({ "net.weather": false }), None);
    let field = r.v.card()[3];
    r.click(field.center());
    r.type_text("me@x.org");
    let enable = r.v.card()[4];
    r.click(enable.center());
    assert_eq!(r.calls("settings_set"), vec![json!({ "patch": { "net.weather": true, "net.contact": "me@x.org" } })]);
}

#[test]
fn an_enabled_tab_loads_the_forecast_and_refresh_spins_and_forces() {
    let _m = motion(false);
    let mut r = Rig::on(forecast("lightrain", "rain", 4.4));
    assert_eq!(r.v.kind, Kind::Ok);
    assert_eq!(r.calls("weather_get")[0]["force"], false);
    let rf = r.v.rf_rect();
    r.send(InputEvent::MouseMove { pos: rf.center(), mods: Modifiers::NONE, buttons: Buttons::NONE });
    r.send(InputEvent::MouseDown { pos: rf.center(), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
    r.send(InputEvent::MouseUp { pos: rf.center(), button: MouseButton::Left, mods: Modifiers::NONE });
    assert!(r.calls("weather_get").iter().any(|a| a["force"] == true));
    assert!(r.v.base.player.is_active("refresh"), "the refresh icon turns");
    r.frame();
}

#[test]
fn units_toggle_patches_the_setting() {
    let mut r = Rig::on(forecast("cloudy", "cloud", 8.0));
    let u = r.v.unit_rect(1);
    r.click(u.center());
    assert_eq!(r.calls("settings_set"), vec![json!({ "patch": { "weather.units": "imperial" } })]);
}

#[test]
fn changing_the_city_searches_and_picking_a_result_sets_it() {
    let mut r = Rig::on(forecast("cloudy", "cloud", 8.0));
    let c = r.v.city_rect();
    r.click(c.center());
    assert_eq!(r.v.kind, Kind::City);
    r.f.api.reply("weather_search_city", Ok(json!({ "status": "ok", "results": [{ "name": "Oslo", "detail": "Norway", "lat": 59.91, "lon": 10.75 }, { "name": "Osló", "detail": "Spain", "lat": 40.0, "lon": -3.0 }] })));
    let (inp, go) = r.v.search_rects(r.v.card()[3]);
    r.click(inp.center());
    r.v.city.set_text(""); // the box starts with the current city
    r.type_text("Oslo");
    r.click(go.center());
    assert_eq!(r.calls("weather_search_city"), vec![json!({ "query": "Oslo", "lang": "en" })]);
    r.f.api.reply("weather_set_city", Ok(json!({ "status": "ok" })));
    let row = r.v.res_row(r.v.card()[5], 0);
    r.click(row.center());
    assert_eq!(r.calls("weather_set_city"), vec![json!({ "name": "Oslo", "lat": 59.91, "lon": 10.75 })]);
}

#[test]
fn an_error_card_offers_retry_and_turn_off() {
    let mut r = Rig::on(json!({ "status": "error", "message": "HTTP 503" }));
    assert_eq!(r.v.kind, Kind::Error);
    let rs = r.v.card();
    let b = r.v.err_buttons(*rs.last().unwrap());
    r.f.api.reply("weather_get", Ok(forecast("clearsky", "clear-day", 12.0)));
    r.click(b[0].center());
    assert!(r.calls("weather_get").len() >= 2, "retry asks again");
    assert_eq!(r.v.kind, Kind::Ok);
}

#[test]
fn a_settled_forecast_costs_no_frames_and_reduced_motion_plays_no_cues() {
    let _m = motion(true);
    let mut r = Rig::on(forecast("rain", "rain", 4.0));
    r.cx.reduced_motion = true;
    let rf = r.v.rf_rect();
    r.click(rf.center());
    assert!(!r.v.base.player.busy(), "no cue under reduced motion");
    assert!(!r.v.step(4.0, &mut r.cx), "settled = idle");
}

// ------------------------------------------------------------------------------------------------------------------ pixels

fn shoot_all(r: &mut Rig, name: &str) {
    for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
        for sc in [1.0f32, 2.0] {
            r.cx.scale = sc;
            let img = shoot(&mut r.v, &r.cx, &th, sc, "weather", &format!("{name}-{tn}-x{}", sc as u32));
            assert!(img.count(None, |p| p[3] > 0) > 4000, "{name} {tn} x{sc} drew nothing");
        }
    }
}

#[test]
fn snapshots_of_the_typical_states() {
    let _m = motion(false);
    for (name, sym, icon, t) in [
        ("clear-day", "clearsky", "clear-day", 17.0),
        ("clear-night", "clearsky", "clear-night", 6.0),
        ("cloudy", "cloudy", "cloud", 9.0),
        ("rain", "lightrain", "rain", 4.4),
        ("snow", "snow", "snow", -4.0),
        ("thunder", "heavyrainandthunder", "thunder", 21.0),
        ("fog", "fog", "fog", 3.0),
    ] {
        let mut r = Rig::on(forecast(sym, icon, t));
        r.frame();
        shoot_all(&mut r, name);
    }
    // off / loading / error / city
    let mut r = Rig::open(json!({ "net.weather": false }), None);
    shoot_all(&mut r, "off");
    let mut r = Rig::open(json!({ "net.weather": true }), Some(json!({ "status": "error", "message": "HTTP 503" })));
    shoot_all(&mut r, "error");
    let mut r = Rig::open(json!({ "net.weather": true }), Some(json!({ "status": "nocity" })));
    shoot_all(&mut r, "city");
    // loading: the commands never answer
    let (svc, _f) = Services::fake();
    let (mut cx, _q) = Cx::headless_queue(svc);
    let mut v = WeatherView::new();
    v.on_show(&mut cx);
    for (th, tn) in [(Theme::pill_dark(), "dark"), (Theme::pill_light(), "light")] {
        shoot(&mut v, &cx, &th, 1.0, "weather", &format!("loading-{tn}-x1"));
    }
}
