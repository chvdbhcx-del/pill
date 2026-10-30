//! Live events from OBS, Steam, Windows notifications and the uninstaller (port of live-ext.js;
//! Rust side obs.rs, steam.rs, notif.rs, uninstall.rs).
//!
//! * `live:obs {recording, streaming, recMs, streamMs, file?}` -> red REC / LIVE chip (priority 90, no ttl, red outline) with timers;
//!   the card "Recording saved" when `file` arrives. Opt-in: `live.obs` is OFF by default.
//! * `live:steam {state, game, appid, percent, speed, etaSec}` -> ring chip with the game and percent (priority 45, below the music pill);
//!   a card + edge pulse on `done`. The percent shown is whatever the backend sends (the estimator is a separate task).
//! * `live:notification {app, title, body, id}` -> card: app letter (a bell when the app is unknown), title <= 2 lines, body <= 3 lines.
//! * `uninstall:ready {name, kind, count}` -> a finished uninstall step while the command bar is closed.
//!
//! PRIVACY: every string from outside (file names, game names, notification texts) is data for text elements only.

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

// ================================================================ OBS

pub const OBS_PRIORITY: i32 = 90;
pub const OBS_TICK_MS: u32 = 500;

#[derive(Debug, Default)]
pub struct ObsLive {
    /// Epoch ms the recording / stream started, 0 = not active.
    rec: i64,
    live: i64,
    hidden: bool,
    /// A row is on the board (the timer is armed).
    armed: bool,
    shown: (String, String),
}

impl ObsLive {
    fn texts(&self, now: i64) -> (String, String) {
        (
            if self.rec != 0 { clock_hms(now.saturating_sub(self.rec)) } else { String::new() },
            if self.live != 0 { clock_hms(now.saturating_sub(self.live)) } else { String::new() },
        )
    }

    fn row(&self, env: &dyn Env, now: i64) -> RowSpec {
        let (rt, lt) = self.texts(now);
        let mut r = RowSpec::new("obs", OBS_PRIORITY)
            .key(format!("obs{}:{}", self.rec, self.live))
            .accent(Accent::Rgb(palette::OBS_REC))
            .tip(env.tr("OBS записывает или ведёт трансляцию", "OBS is recording or streaming"))
            .anim(Anim::Fade, Anim::None, Anim::Fade);
        let part = |label: &str, dot: Rgba, since: i64, t: String| {
            vec![
                Elem::Dot { color: dot, size: 10.0, halo: true, pulse_ms: Some(2000) },
                Elem::text(label, TextStyle::RecLabel),
                Elem::Clock { text: t, since, style: TextStyle::Clock },
            ]
        };
        if self.rec != 0 {
            r.elements.extend(part("REC", palette::OBS_REC, self.rec, rt));
        }
        if self.live != 0 {
            if self.rec != 0 {
                r.elements.push(Elem::Sep);
            }
            r.elements.extend(part("LIVE", palette::OBS_LIVE, self.live, lt));
        }
        r
    }

    fn render(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if !env.live_on("obs") || (self.rec == 0 && self.live == 0) {
            let was = self.armed;
            self.armed = false;
            self.shown = Default::default();
            return if was { vec![LiveCmd::clear("obs")] } else { Vec::new() };
        }
        self.armed = true;
        self.shown = self.texts(now);
        vec![LiveCmd::set(self.row(env, now))]
    }
}

impl Provider for ObsLive {
    fn name(&self) -> &'static str {
        "obs"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:obs", "settings:changed", "pill:hide", "pill:show"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "pill:hide" => {
                self.hidden = true; // no ticks while the island is hidden (nobody sees the text)
                Vec::new()
            }
            "pill:show" => {
                self.hidden = false;
                if self.armed { self.render(env, now) } else { Vec::new() }
            }
            "settings:changed" => {
                if !env.live_on("obs") {
                    self.rec = 0;
                    self.live = 0;
                    return self.render(env, now);
                }
                Vec::new()
            }
            "live:obs" => {
                if p.is_null() {
                    return Vec::new();
                }
                let ms = |k: &str| num_or0(p.get(k)).trunc() as i64;
                // a stream event must not restart a running REC timer
                self.rec = if truthy(p.get("recording")) { if self.rec != 0 { self.rec } else { now.saturating_sub(ms("recMs")) } } else { 0 };
                self.live = if truthy(p.get("streaming")) { if self.live != 0 { self.live } else { now.saturating_sub(ms("streamMs")) } } else { 0 };
                let mut out = self.render(env, now);
                if truthy(p.get("file")) && env.live_on("obs") {
                    let f = text(p.get("file"));
                    let card = CardSpec::text("obs-saved", 12000, env.tr("Запись сохранена", "Recording saved"), Some(base(&f)))
                        .icon(IconName::Check)
                        .tone(Tone::Green)
                        .actions(vec![
                            CardAction::new(env.tr("Показать в папке", "Show in folder"), ActionKind::White, Action::invoke("obs_reveal", json!({ "path": f }))),
                            CardAction::close(env.tr("Закрыть", "Close")),
                        ]);
                    out.push(LiveCmd::Toast(card));
                }
                out
            }
            _ => Vec::new(),
        }
    }
    fn on_tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if !self.armed || self.hidden || !env.live_on("obs") {
            return Vec::new();
        }
        let t = self.texts(now);
        if t == self.shown {
            return Vec::new(); // the displayed seconds did not change
        }
        self.shown = t;
        vec![LiveCmd::set(self.row(env, now))]
    }
    fn tick_ms(&self, _now: i64) -> Option<u32> {
        if self.armed && !self.hidden { Some(OBS_TICK_MS) } else { None }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ Steam

pub const STEAM_PRIORITY: i32 = 45;

/// `812 KB/s`-style rate: MB/s, one decimal below 10.
pub fn mbs(env: &dyn Env, bytes_per_s: f64) -> String {
    let m = bytes_per_s / 1_048_576.0;
    let n = if m >= 10.0 { fmt_num(js_round(m)) } else { to_fixed(m, 1) };
    format!("{n} {}", env.tr("МБ/с", "MB/s"))
}

/// "12 мин" / "1 ч 5 мин".
pub fn eta(env: &dyn Env, s: f64) -> String {
    if s >= 3600.0 {
        format!("{}{}{}{}", (s / 3600.0).floor(), env.tr(" ч ", " h "), fmt_num(js_round((s % 3600.0) / 60.0)), env.tr(" мин", " min"))
    } else {
        format!("{}{}", fmt_num(js_round(s / 60.0).max(1.0)), env.tr(" мин", " min"))
    }
}

fn steam_chip(env: &dyn Env, p: &Value) -> RowSpec {
    let pct = clamp_f(num_or0(p.get("percent")), 0.0, 100.0);
    let state = p.get("state").and_then(Value::as_str).unwrap_or("");
    let speed = to_number(p.get("speed"));
    let eta_s = to_number(p.get("etaSec"));
    let downloading = state == "downloading";
    let right = match state {
        "paused" => env.tr("пауза", "paused"),
        "installing" => env.tr("установка", "installing"),
        _ => format!("{}%", fmt_num(pct)),
    };
    let game = text_or_empty(p.get("game"));
    let tip = join_nonempty(
        &[
            game.clone(),
            if downloading && speed > 0.0 { mbs(env, speed) } else { String::new() },
            if eta_s > 0.0 { format!("{}{}", env.tr("осталось ", "left "), eta(env, eta_s)) } else { String::new() },
        ],
        " · ",
    );
    let mut pg = vec![Elem::text(right, TextStyle::Percent)];
    if downloading && speed > 0.0 {
        pg.push(Elem::text(mbs(env, speed), TextStyle::Small));
    }
    RowSpec::new("steam", STEAM_PRIORITY)
        .tip(tip)
        .anim(Anim::Fade, Anim::Glide(600.0), Anim::Fade)
        .with(Elem::Ring { value: Some(pct as f32 / 100.0), size: 16.0, glide_ms: 600 })
        .with(Elem::text_max(if game.is_empty() { "Steam".to_string() } else { game }, TextStyle::Game, 96.0))
        .with(Elem::Group(pg))
}

#[derive(Debug, Default)]
pub struct SteamLive;

impl Provider for SteamLive {
    fn name(&self) -> &'static str {
        "steam"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:steam", "settings:changed"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if name == "settings:changed" {
            return if env.live_on("steam") { Vec::new() } else { vec![LiveCmd::clear("steam")] };
        }
        if p.is_null() || !env.live_on("steam") {
            return vec![LiveCmd::clear("steam")];
        }
        match p.get("state").and_then(Value::as_str).unwrap_or("") {
            "done" => {
                let game = text_or_empty(p.get("game"));
                let card = CardSpec::text(
                    format!("steam-done:{}", display(p.get("appid"))),
                    8000,
                    if game.is_empty() { "Steam".to_string() } else { game },
                    Some(env.tr("Загрузка Steam завершена", "Steam download finished")),
                )
                .icon(IconName::Download)
                .tone(Tone::Green)
                .actions(vec![CardAction::close(env.tr("Закрыть", "Close"))]);
                // not muted by quiet hours here (as before); the hub still drops the card itself
                vec![LiveCmd::clear("steam"), LiveCmd::EdgePulse(0.7), LiveCmd::Toast(card)]
            }
            "idle" => vec![LiveCmd::clear("steam")],
            _ => vec![LiveCmd::set(steam_chip(env, p))],
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ Windows notifications

#[derive(Debug, Default)]
pub struct NotificationLive;

impl Provider for NotificationLive {
    fn name(&self) -> &'static str {
        "notification"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:notification"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if p.is_null() || !env.live_on("notifications") {
            return Vec::new();
        }
        let app = text_or_empty(p.get("app")).trim().to_string();
        let title_raw = text_or_empty(p.get("title")).trim().to_string();
        let title = if title_raw.is_empty() { app.clone() } else { title_raw };
        let body = text_or_empty(p.get("body")).trim().to_string();
        // known app: its first letter; system notifications come without one: a generic bell instead of an arbitrary letter from the title
        let icon = match app.chars().next() {
            Some(c) => CardIcon::Letter(c.to_uppercase().collect()),
            None => CardIcon::Name(IconName::Bell),
        };
        let mut card = CardSpec::new(
            format!("notif:{}", display(p.get("id"))),
            7000,
            CardBody::Notification { title, body: if body.is_empty() { None } else { Some(body) }, app, max_title_lines: 2, max_body_lines: 3 },
        )
        .actions(vec![CardAction::close(env.tr("Закрыть", "Close"))]);
        card.icon = Some(icon);
        vec![LiveCmd::Toast(card)]
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ uninstall

/// Rust (uninstall.rs) sends this only when the command bar is closed; the bar's own status strip covers the open bar.
/// Quiet hours apply (not urgent).
#[derive(Debug, Default)]
pub struct UninstallLive;

impl Provider for UninstallLive {
    fn name(&self) -> &'static str {
        "uninstall"
    }
    fn events(&self) -> &'static [&'static str] {
        &["uninstall:ready"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        if p.is_null() {
            return Vec::new();
        }
        let name = text_or_empty(p.get("name"));
        let n = num_or0(p.get("count"));
        let kind = text_or_empty(p.get("kind"));
        let ns = fmt_num(n);
        let sub = match kind.as_str() {
            "review" => format!("{}{ns}", env.tr("Остатков к проверке: ", "Leftovers to review: ")),
            "done" => {
                if n != 0.0 {
                    format!("{}{ns}", env.tr("Готово, удалено: ", "Done, removed: "))
                } else {
                    env.tr("Остатков не найдено", "No leftovers found")
                }
            }
            "still" => env.tr("Приложение всё ещё установлено", "The app is still installed"),
            _ => env.tr("Не получилось", "It did not work"),
        };
        let bad = kind == "error" || kind == "still";
        let card = CardSpec::text(format!("uninstall:{name}"), 12000, format!("{}{name}", env.tr("Удаление: ", "Uninstall of ")), Some(sub))
            .icon(if bad { IconName::Trash } else { IconName::Check })
            .tone(if bad { Tone::Neutral } else { Tone::Green })
            .actions(vec![
                CardAction::new(env.tr("Открыть", "Open"), ActionKind::White, Action::invoke0("uninstall_open_panel")),
                CardAction::close(env.tr("Закрыть", "Close")),
            ]);
        vec![LiveCmd::Toast(card)]
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    const T0: i64 = 1_700_000_000_000;

    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no row in {cmds:?}"))
    }
    fn toast(cmds: &[LiveCmd]) -> &CardSpec {
        cmds.iter().find_map(|c| if let LiveCmd::Toast(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no toast in {cmds:?}"))
    }
    fn obs_env() -> MemEnv {
        let mut e = MemEnv::default();
        e.set("live.obs", json!(true));
        e
    }
    fn clocks(r: &RowSpec) -> Vec<String> {
        r.elements.iter().filter_map(|e| if let Elem::Clock { text, .. } = e { Some(text.clone()) } else { None }).collect()
    }

    // ---------------- OBS
    #[test]
    fn obs_is_off_by_default() {
        let mut o = ObsLive::default();
        let cmds = o.on_event(&MemEnv::default(), "live:obs", &json!({"recording": true, "streaming": false, "recMs": 0, "streamMs": 0}), T0);
        assert!(cmds.is_empty(), "{cmds:?}");
        assert_eq!(o.tick_ms(T0), None);
    }

    #[test]
    fn obs_rec_chip() {
        let mut o = ObsLive::default();
        let env = obs_env();
        let cmds = o.on_event(&env, "live:obs", &json!({"recording": true, "streaming": false, "recMs": 65_000, "streamMs": 0}), T0);
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("obs", 90, None));
        assert_eq!(r.accent, Some(Accent::Rgb(Rgba::rgb(0xff, 0x42, 0x3a))));
        assert_eq!(r.tip.as_deref(), Some("OBS записывает или ведёт трансляцию"));
        assert_eq!(r.elements.len(), 3);
        assert_eq!(r.elements[0], Elem::Dot { color: palette::OBS_REC, size: 10.0, halo: true, pulse_ms: Some(2000) });
        assert_eq!(r.elements[1], Elem::text("REC", TextStyle::RecLabel));
        assert_eq!(clocks(r), ["1:05"]);
        assert_eq!(o.tick_ms(T0), Some(500));
    }

    #[test]
    fn obs_rec_and_live_with_separator() {
        let mut o = ObsLive::default();
        let env = obs_env();
        let cmds = o.on_event(&env, "live:obs", &json!({"recording": true, "streaming": true, "recMs": 3_600_000, "streamMs": 5000}), T0);
        let r = row(&cmds);
        assert_eq!(r.elements.len(), 7);
        assert_eq!(r.elements[3], Elem::Sep);
        assert_eq!(r.elements[4], Elem::Dot { color: palette::OBS_LIVE, size: 10.0, halo: true, pulse_ms: Some(2000) });
        assert_eq!(r.elements[5], Elem::text("LIVE", TextStyle::RecLabel));
        assert_eq!(clocks(r), ["1:00:00", "0:05"]);
        // live only: no separator
        let mut o = ObsLive::default();
        let cmds = o.on_event(&env, "live:obs", &json!({"recording": false, "streaming": true, "streamMs": 0}), T0);
        assert!(!row(&cmds).elements.contains(&Elem::Sep));
    }

    #[test]
    fn obs_a_stream_event_does_not_restart_the_rec_timer() {
        let mut o = ObsLive::default();
        let env = obs_env();
        o.on_event(&env, "live:obs", &json!({"recording": true, "streaming": false, "recMs": 0}), T0);
        let cmds = o.on_event(&env, "live:obs", &json!({"recording": true, "streaming": true, "recMs": 0, "streamMs": 0}), T0 + 30_000);
        assert_eq!(clocks(row(&cmds)), ["0:30", "0:00"]);
    }

    #[test]
    fn obs_timer_ticks_only_when_the_text_changes() {
        let mut o = ObsLive::default();
        let env = obs_env();
        let first = o.on_event(&env, "live:obs", &json!({"recording": true, "recMs": 0}), T0);
        let key = row(&first).key.clone();
        assert!(o.on_tick(&env, T0 + 400).is_empty());
        let t = o.on_tick(&env, T0 + 1000);
        assert_eq!(clocks(row(&t)), ["0:01"]);
        assert_eq!(row(&t).key, key, "edited in place, not rebuilt");
        assert!(o.on_tick(&env, T0 + 1500).is_empty());
    }

    #[test]
    fn obs_hidden_island_stops_ticking_and_catches_up_on_show() {
        let mut o = ObsLive::default();
        let env = obs_env();
        o.on_event(&env, "live:obs", &json!({"recording": true, "recMs": 0}), T0);
        o.on_event(&env, "pill:hide", &Value::Null, T0 + 100);
        assert_eq!(o.tick_ms(T0 + 200), None);
        assert!(o.on_tick(&env, T0 + 5000).is_empty());
        let cmds = o.on_event(&env, "pill:show", &Value::Null, T0 + 61_000);
        assert_eq!(clocks(row(&cmds)), ["1:01"]);
        assert_eq!(o.tick_ms(T0 + 61_000), Some(500));
    }

    #[test]
    fn obs_stops_and_clears() {
        let mut o = ObsLive::default();
        let env = obs_env();
        o.on_event(&env, "live:obs", &json!({"recording": true, "recMs": 0}), T0);
        let cmds = o.on_event(&env, "live:obs", &json!({"recording": false, "streaming": false, "recMs": 0, "streamMs": 0}), T0 + 10);
        assert_eq!(cmds, vec![LiveCmd::clear("obs")]);
        assert_eq!(o.tick_ms(T0 + 10), None);
        // nothing on screen: a repeated idle event is silent
        assert!(o.on_event(&env, "live:obs", &json!({"recording": false, "streaming": false}), T0 + 20).is_empty());
    }

    #[test]
    fn obs_saved_card() {
        let mut o = ObsLive::default();
        let env = obs_env();
        let cmds = o.on_event(&env, "live:obs", &json!({"recording": false, "streaming": false, "recMs": 0, "streamMs": 0, "file": "D:\\Videos\\2026-10-30 12-00-01.mkv"}), T0);
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("obs-saved", 12000, Tone::Green));
        assert_eq!(t.body, CardBody::Text { title: "Запись сохранена".into(), sub: Some("2026-10-30 12-00-01.mkv".into()) });
        assert_eq!(t.actions[0].action, Action::invoke("obs_reveal", json!({"path": "D:\\Videos\\2026-10-30 12-00-01.mkv"})));
        let en = {
            let mut e = MemEnv::en();
            e.set("live.obs", json!(true));
            e
        };
        let cmds = ObsLive::default().on_event(&en, "live:obs", &json!({"recording": false, "file": "/v/a.mp4"}), T0);
        assert_eq!(toast(&cmds).body, CardBody::Text { title: "Recording saved".into(), sub: Some("a.mp4".into()) });
    }

    #[test]
    fn obs_switch_off_resets() {
        let mut o = ObsLive::default();
        let mut env = obs_env();
        o.on_event(&env, "live:obs", &json!({"recording": true, "recMs": 0}), T0);
        env.set("live.obs", json!(false));
        assert_eq!(o.on_event(&env, "settings:changed", &Value::Null, T0 + 5), vec![LiveCmd::clear("obs")]);
        assert_eq!(o.tick_ms(T0 + 5), None);
        // with the switch on a settings change is silent
        let env = obs_env();
        assert!(o.on_event(&env, "settings:changed", &Value::Null, T0 + 6).is_empty());
        assert!(o.on_event(&env, "live:obs", &Value::Null, T0).is_empty());
    }

    // ---------------- Steam
    fn steam(state: &str, game: &str, percent: f64, speed: u64, eta_sec: u64) -> Value {
        json!({"state": state, "game": game, "appid": "730", "percent": percent, "speed": speed, "etaSec": eta_sec})
    }

    #[test]
    fn steam_chip_downloading() {
        let mut s = SteamLive;
        let env = MemEnv::default();
        let cmds = s.on_event(&env, "live:steam", &steam("downloading", "Counter-Strike 2", 42.0, 12 * 1_048_576, 300), T0);
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("steam", 45, None));
        assert_eq!(r.elements[0], Elem::Ring { value: Some(0.42), size: 16.0, glide_ms: 600 });
        assert_eq!(r.elements[1], Elem::text_max("Counter-Strike 2", TextStyle::Game, 96.0));
        assert_eq!(r.elements[2], Elem::Group(vec![Elem::text("42%", TextStyle::Percent), Elem::text("12 МБ/с", TextStyle::Small)]));
        assert_eq!(r.tip.as_deref(), Some("Counter-Strike 2 · 12 МБ/с · осталось 5 мин"));
    }

    #[test]
    fn steam_chip_en_speed_and_eta_formats() {
        let mut s = SteamLive;
        let env = MemEnv::en();
        let r = row(&s.on_event(&env, "live:steam", &steam("downloading", "Hades", 5.0, 3 * 1_048_576 / 2, 7_500), T0)).clone();
        assert_eq!(r.elements[2], Elem::Group(vec![Elem::text("5%", TextStyle::Percent), Elem::text("1.5 MB/s", TextStyle::Small)]));
        assert_eq!(r.tip.as_deref(), Some("Hades · 1.5 MB/s · left 2 h 5 min"));
        // under a minute still says 1 min
        assert_eq!(eta(&env, 20.0), "1 min");
        assert_eq!(eta(&env, 90.0), "2 min");
        assert_eq!(eta(&env, 3600.0), "1 h 0 min");
        // the JS rounding quirk is kept: 59.6 minutes past the hour rounds to 60
        assert_eq!(eta(&env, 3600.0 + 59.6 * 60.0), "1 h 60 min");
        assert_eq!(mbs(&env, 0.25 * 1_048_576.0), "0.3 MB/s");
        assert_eq!(mbs(&env, 10.4 * 1_048_576.0), "10 MB/s");
    }

    #[test]
    fn steam_paused_installing_have_no_speed() {
        let mut s = SteamLive;
        let env = MemEnv::default();
        let r = row(&s.on_event(&env, "live:steam", &steam("paused", "Hades", 55.0, 5_000_000, 100), T0)).clone();
        assert_eq!(r.elements[2], Elem::Group(vec![Elem::text("пауза", TextStyle::Percent)]));
        assert_eq!(r.tip.as_deref(), Some("Hades · осталось 2 мин"), "speed is shown only while downloading, eta whenever > 0");
        let r = row(&s.on_event(&MemEnv::en(), "live:steam", &steam("installing", "Hades", 100.0, 0, 0), T0)).clone();
        assert_eq!(r.elements[2], Elem::Group(vec![Elem::text("installing", TextStyle::Percent)]));
        assert_eq!(r.tip.as_deref(), Some("Hades"));
    }

    #[test]
    fn steam_percent_is_clamped_and_kept_verbatim() {
        let mut s = SteamLive;
        let env = MemEnv::default();
        let mut pct = |v: Value| match &row(&s.on_event(&env, "live:steam", &v, T0)).elements[2] {
            Elem::Group(g) => match &g[0] {
                Elem::Text { text, .. } => text.clone(),
                o => panic!("{o:?}"),
            },
            o => panic!("{o:?}"),
        };
        assert_eq!(pct(json!({"state": "downloading", "percent": 140})), "100%");
        assert_eq!(pct(json!({"state": "downloading", "percent": -3})), "0%");
        assert_eq!(pct(json!({"state": "downloading"})), "0%");
        assert_eq!(pct(json!({"state": "downloading", "percent": 33.5})), "33.5%", "no rounding, as before");
        assert_eq!(pct(json!({"state": "downloading", "percent": "7"})), "7%");
    }

    #[test]
    fn steam_unknown_game_is_steam() {
        let mut s = SteamLive;
        let env = MemEnv::default();
        let r = row(&s.on_event(&env, "live:steam", &json!({"state": "downloading", "percent": 1}), T0)).clone();
        assert_eq!(r.elements[1], Elem::text_max("Steam", TextStyle::Game, 96.0));
        assert_eq!(r.tip.as_deref(), Some(""));
    }

    #[test]
    fn steam_done_idle_and_switch() {
        let mut s = SteamLive;
        let env = MemEnv::default();
        let cmds = s.on_event(&env, "live:steam", &json!({"state": "done", "game": "Hades", "appid": "1145360", "percent": 100}), T0);
        assert_eq!(cmds[0], LiveCmd::clear("steam"));
        assert_eq!(cmds[1], LiveCmd::EdgePulse(0.7));
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("steam-done:1145360", 8000, Tone::Green));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Download)));
        assert_eq!(t.body, CardBody::Text { title: "Hades".into(), sub: Some("Загрузка Steam завершена".into()) });
        assert_eq!(t.actions.len(), 1);
        assert_eq!(s.on_event(&env, "live:steam", &json!({"state": "idle"}), T0), vec![LiveCmd::clear("steam")]);
        assert_eq!(s.on_event(&env, "live:steam", &Value::Null, T0), vec![LiveCmd::clear("steam")]);
        let mut off = MemEnv::default();
        off.set("live.steam", json!(false));
        assert_eq!(s.on_event(&off, "live:steam", &steam("downloading", "x", 1.0, 0, 0), T0), vec![LiveCmd::clear("steam")]);
        assert_eq!(s.on_event(&off, "settings:changed", &Value::Null, T0), vec![LiveCmd::clear("steam")]);
        assert!(s.on_event(&env, "settings:changed", &Value::Null, T0).is_empty());
        // a done event without a game or an appid
        let cmds = s.on_event(&MemEnv::en(), "live:steam", &json!({"state": "done"}), T0);
        let t = toast(&cmds);
        assert_eq!(t.id, "steam-done:undefined");
        assert_eq!(t.body, CardBody::Text { title: "Steam".into(), sub: Some("Steam download finished".into()) });
    }

    // ---------------- notifications
    #[test]
    fn notification_card_with_app() {
        let mut n = NotificationLive;
        let env = MemEnv::default();
        let cmds = n.on_event(&env, "live:notification", &json!({"app": " telegram ", "title": " Anna ", "body": " hi there ", "id": 42}), T0);
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("notif:42", 7000, Tone::Neutral));
        assert_eq!(t.icon, Some(CardIcon::Letter("T".into())));
        assert_eq!(
            t.body,
            CardBody::Notification { title: "Anna".into(), body: Some("hi there".into()), app: "telegram".into(), max_title_lines: 2, max_body_lines: 3 }
        );
        assert_eq!(t.actions[0].label, "Закрыть");
    }

    #[test]
    fn notification_without_app_gets_a_bell_and_title_falls_back() {
        let mut n = NotificationLive;
        let env = MemEnv::en();
        let cmds = n.on_event(&env, "live:notification", &json!({"app": "", "title": "Battery low", "body": "", "id": "x1"}), T0);
        let t = toast(&cmds);
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Bell)));
        assert_eq!(t.id, "notif:x1");
        assert!(matches!(&t.body, CardBody::Notification { body: None, title, .. } if title == "Battery low"));
        assert_eq!(t.actions[0].label, "Close");
        // empty title falls back to the app name
        let cmds = n.on_event(&env, "live:notification", &json!({"app": "Mail", "title": "  ", "id": 1}), T0);
        assert!(matches!(&toast(&cmds).body, CardBody::Notification { title, .. } if title == "Mail"));
    }

    #[test]
    fn notification_unicode_letter_and_gates() {
        let mut n = NotificationLive;
        let env = MemEnv::default();
        let cmds = n.on_event(&env, "live:notification", &json!({"app": "ёлка", "title": "t", "id": 1}), T0);
        assert_eq!(toast(&cmds).icon, Some(CardIcon::Letter("Ё".into())));
        let cmds = n.on_event(&env, "live:notification", &json!({"app": "ßeta", "title": "t", "id": 1}), T0);
        assert_eq!(toast(&cmds).icon, Some(CardIcon::Letter("SS".into())), "toUpperCase of the first character, like the JS");
        let cmds = n.on_event(&env, "live:notification", &json!({"app": "😀 chat", "title": "t"}), T0);
        assert_eq!(toast(&cmds).icon, Some(CardIcon::Letter("😀".into())));
        assert_eq!(toast(&cmds).id, "notif:undefined");
        assert!(n.on_event(&env, "live:notification", &Value::Null, T0).is_empty());
        let mut off = MemEnv::default();
        off.set("live.notifications", json!(false));
        assert!(n.on_event(&off, "live:notification", &json!({"app": "a", "title": "t"}), T0).is_empty());
    }

    // ---------------- uninstall
    #[test]
    fn uninstall_cards() {
        let mut u = UninstallLive;
        let env = MemEnv::default();
        let sub = |env: &MemEnv, kind: &str, count: i64| match &toast(&u_ev(env, kind, count)).body {
            CardBody::Text { sub, .. } => sub.clone().unwrap(),
            o => panic!("{o:?}"),
        };
        fn u_ev(env: &MemEnv, kind: &str, count: i64) -> Vec<LiveCmd> {
            UninstallLive.on_event(env, "uninstall:ready", &json!({"name": "Foo", "kind": kind, "count": count}), 0)
        }
        assert_eq!(sub(&env, "review", 3), "Остатков к проверке: 3");
        assert_eq!(sub(&env, "done", 5), "Готово, удалено: 5");
        assert_eq!(sub(&env, "done", 0), "Остатков не найдено");
        assert_eq!(sub(&env, "still", 0), "Приложение всё ещё установлено");
        assert_eq!(sub(&env, "error", 0), "Не получилось");
        assert_eq!(sub(&env, "whatever", 0), "Не получилось");
        let en = MemEnv::en();
        assert_eq!(sub(&en, "review", 3), "Leftovers to review: 3");
        assert_eq!(sub(&en, "done", 5), "Done, removed: 5");
        assert_eq!(sub(&en, "done", 0), "No leftovers found");
        assert_eq!(sub(&en, "still", 0), "The app is still installed");
        assert_eq!(sub(&en, "error", 0), "It did not work");
        let t = toast(&u.on_event(&env, "uninstall:ready", &json!({"name": "Foo", "kind": "done", "count": 2}), T0)).clone();
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone, t.icon.clone()), ("uninstall:Foo", 12000, Tone::Green, Some(CardIcon::Name(IconName::Check))));
        assert!(matches!(&t.body, CardBody::Text { title, .. } if title == "Удаление: Foo"));
        assert_eq!(t.actions[0].action, Action::invoke0("uninstall_open_panel"));
        let t = toast(&u.on_event(&en, "uninstall:ready", &json!({"name": "Foo", "kind": "error"}), T0)).clone();
        assert_eq!((t.tone, t.icon.clone()), (Tone::Neutral, Some(CardIcon::Name(IconName::Trash))));
        assert!(matches!(&t.body, CardBody::Text { title, sub: Some(s) } if title == "Uninstall of Foo" && s == "It did not work"));
        let t = toast(&u.on_event(&en, "uninstall:ready", &json!({"name": "Foo", "kind": "still"}), T0)).clone();
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Trash)));
        assert!(u.on_event(&en, "uninstall:ready", &Value::Null, T0).is_empty());
        // garbage count counts as zero
        assert_eq!(sub(&env, "review", 0), "Остатков к проверке: 0");
        let t = toast(&u.on_event(&env, "uninstall:ready", &json!({"name": "F", "kind": "done", "count": "abc"}), T0)).clone();
        assert!(matches!(&t.body, CardBody::Text { sub: Some(s), .. } if s == "Остатков не найдено"));
    }
}
