//! System live events (port of live-system.js; Rust side livesys.rs): the volume HUD, the microphone /
//! camera dots, the download chip + "download complete" card, and the "new version" card.
//!
//! * `live:volume {level 0..100, muted}` -> speaker + bar + number for 1.6 s (priority 88).
//!   The mouse wheel over the collapsed capsule changes the volume by 2 % per notch ([`VolumeLive::wheel`]).
//! * `live:privacy {mic, cam, apps}` -> orange (mic) / green (camera) dots in the capsule corner, pulsing 2.4 s, no text.
//! * `live:download {state: progress|done|gone, id, name, bytes, total|null, path}` -> chip with ring + file name + size (priority 55),
//!   a green card when it is done.
//! * `update:available {version}` -> "A new Pill version is available" card.
//!
//! PRIVACY: nothing here reads sound, picture or file content; names are only ever text.
//! Switches: Settings > Live events > Volume / Microphone and camera / Downloads.

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

// ================================================================ volume

pub const VOL_MS: u32 = 1600;
pub const WHEEL_STEP: i32 = 2;
pub const VOL_PRIORITY: i32 = 88;

/// Where the current volume is read when the wheel moves before any `live:volume` arrived (`media_volume_get`).
pub trait VolumeIo {
    /// `(level 0..100, muted)`; `None` when the call failed.
    fn get(&mut self) -> Option<(i32, bool)>;
}

#[derive(Debug)]
pub struct VolumeLive {
    level: i32,
    muted: bool,
    until: i64,
    seq: u64,
}

impl Default for VolumeLive {
    fn default() -> Self {
        VolumeLive { level: -1, muted: false, until: 0, seq: 0 }
    }
}

impl VolumeLive {
    /// Same gate as livesys.rs: the Volume switch and (Media player or Controls module).
    pub fn vol_on(env: &dyn Env) -> bool {
        env.live_on("volume") && (env.module_on("module.media") || env.module_on("module.controls"))
    }

    fn icon(&self) -> IconName {
        if self.muted || self.level == 0 { IconName::Mute } else { IconName::Volume }
    }

    fn row(&self) -> RowSpec {
        let w = if self.muted { 0 } else { self.level.max(0) };
        RowSpec::new("volume", VOL_PRIORITY)
            .ttl(VOL_MS)
            .key(format!("vol{}", self.seq))
            .anim(Anim::Pop, Anim::Glide(120.0), Anim::Fade)
            .with(Elem::icon_c(self.icon(), 17.0, if self.muted { ColorRole::Dim } else { ColorRole::Text }))
            .with(Elem::Bar { value: w as f32 / 100.0, width: 56.0, height: 6.0, glide_ms: 120, muted: self.muted })
            .with(Elem::text(w.to_string(), if self.muted { TextStyle::VolNumMuted } else { TextStyle::VolNum }))
    }

    fn volume(&mut self, env: &dyn Env, p: &Value, now: i64) -> Vec<LiveCmd> {
        if p.is_null() {
            return Vec::new();
        }
        self.level = int32(p.get("level")).clamp(0, 100);
        self.muted = truthy(p.get("muted"));
        // the Home slider moves the volume while the panel is open: no HUD then
        if !Self::vol_on(env) || !env.collapsed() {
            return Vec::new();
        }
        if now >= self.until {
            self.seq += 1; // a fresh HUD (enter animation); within the 1.6 s the same chip is edited in place
        }
        self.until = now + i64::from(VOL_MS);
        vec![LiveCmd::set(self.row())]
    }

    /// The wheel is honoured only over the collapsed capsule, with the volume HUD enabled, and for a real scroll.
    pub fn wheel_allowed(env: &dyn Env, delta_y: f64) -> bool {
        env.collapsed() && Self::vol_on(env) && delta_y != 0.0
    }

    /// One wheel notch: `dir` +1 (up, louder) / -1. Returns the backend calls to make.
    pub fn wheel(&mut self, dir: i32, io: &mut dyn VolumeIo) -> Vec<LiveCmd> {
        if self.level < 0 {
            let Some((level, muted)) = io.get() else { return Vec::new() };
            self.level = level.clamp(0, 100);
            self.muted = muted;
        }
        let next = (self.level + dir * WHEEL_STEP).clamp(0, 100);
        let mut out = Vec::new();
        if self.muted && dir > 0 {
            self.muted = false;
            out.push(LiveCmd::Run(Action::invoke("media_mute", json!({ "muted": false }))));
        }
        self.level = next;
        out.push(LiveCmd::Run(Action::invoke("media_volume_set", json!({ "v": next }))));
        out
    }

    pub fn level(&self) -> i32 {
        self.level
    }
    pub fn muted(&self) -> bool {
        self.muted
    }
}

impl Provider for VolumeLive {
    fn name(&self) -> &'static str {
        "volume"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:volume", "settings:changed"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, payload: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "live:volume" => self.volume(env, payload, now),
            "settings:changed" if !Self::vol_on(env) => {
                self.until = 0;
                vec![LiveCmd::clear("volume")]
            }
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ microphone / camera

#[derive(Debug, Default)]
pub struct PrivacyLive {
    mic: bool,
    cam: bool,
    apps: Vec<String>,
    dots_key: String,
}

impl PrivacyLive {
    fn draw(&mut self, env: &dyn Env) -> Vec<LiveCmd> {
        let ok = env.live_on("micCam");
        let (mic, cam) = (ok && self.mic, ok && self.cam);
        let key = format!("{}{}", if cam { "c" } else { "" }, if mic { "m" } else { "" });
        if key == self.dots_key {
            return Vec::new();
        }
        self.dots_key = key;
        vec![LiveCmd::PrivacyDots { cam, mic }]
    }

    /// Names of the apps using the devices (at most 4), for a tooltip or the Settings page.
    pub fn apps(&self) -> &[String] {
        &self.apps
    }
}

impl Provider for PrivacyLive {
    fn name(&self) -> &'static str {
        "privacy"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:privacy", "settings:changed", "pill:hide", "pill:show"]
    }
    fn boot(&self) -> Vec<Query> {
        vec![Query::new("livesys_privacy", "live:privacy")]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        match name {
            "live:privacy" => {
                if p.is_null() {
                    return Vec::new();
                }
                self.mic = truthy(p.get("mic"));
                self.cam = truthy(p.get("cam"));
                self.apps = p.get("apps").and_then(Value::as_array).map(|a| a.iter().take(4).map(|x| display(Some(x))).collect()).unwrap_or_default();
                self.draw(env)
            }
            "settings:changed" => self.draw(env),
            // the Rust polls (mic/camera, Downloads) pause while the pill is hidden
            "pill:hide" => vec![LiveCmd::Run(Action::invoke("livesys_visible", json!({ "on": false })))],
            "pill:show" => vec![LiveCmd::Run(Action::invoke("livesys_visible", json!({ "on": true })))],
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ downloads

pub const DL_TTL: u32 = 3500;
pub const DL_STALE_MS: i64 = 15_000;
pub const DL_PRIORITY: i32 = 55;

#[derive(Debug, Clone)]
struct Dl {
    id: String,
    name: String,
    bytes: f64,
    total: f64,
    t: i64,
}

#[derive(Debug, Default)]
pub struct DownloadLive {
    dls: Vec<Dl>,
    shown: String,
    seq: u64,
    row_until: i64,
}

impl DownloadLive {
    fn render(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        self.dls.retain(|d| now - d.t <= DL_STALE_MS);
        if self.dls.is_empty() || !env.live_on("downloads") {
            if self.shown.is_empty() {
                return Vec::new();
            }
            self.shown.clear();
            return vec![LiveCmd::clear("download")];
        }
        let mut cur = &self.dls[0];
        for d in &self.dls {
            if d.t >= cur.t {
                cur = d;
            }
        }
        let cur = cur.clone();
        let more = if self.dls.len() > 1 { format!("+{}", self.dls.len() - 1) } else { String::new() };
        if self.shown != cur.id || now >= self.row_until {
            self.shown = cur.id.clone();
            self.seq += 1;
        }
        self.row_until = now + i64::from(DL_TTL);
        let pct = if cur.total > 0.0 { clamp_f(js_round(cur.bytes / cur.total * 100.0), 1.0, 100.0) } else { 0.0 };
        let label = if cur.name.is_empty() { env.tr("Загрузка", "Downloading") } else { cut_name(&cur.name, 28) };
        let mut row = RowSpec::new("download", DL_PRIORITY)
            .ttl(DL_TTL)
            .key(format!("dl{}", self.seq))
            .anim(Anim::Fade, Anim::Glide(400.0), Anim::Fade)
            .with(Elem::Ring { value: if pct > 0.0 { Some(pct as f32 / 100.0) } else { None }, size: 18.0, glide_ms: 400 })
            .with(Elem::text_max(label, TextStyle::FileName, 120.0))
            .with(Elem::text(fmt_size(cur.bytes, env.is_ru()), TextStyle::Dim));
        if !more.is_empty() {
            row = row.with(Elem::text(more, TextStyle::Dim));
        }
        vec![LiveCmd::set(row)]
    }

    fn download(&mut self, env: &dyn Env, p: &Value, now: i64) -> Vec<LiveCmd> {
        if p.is_null() || !truthy(p.get("state")) {
            return Vec::new();
        }
        let id = text_or_empty(p.get("id"));
        match p.get("state").and_then(Value::as_str).unwrap_or("") {
            "progress" => {
                if !env.live_on("downloads") || id.is_empty() {
                    return Vec::new();
                }
                let d = Dl { id: id.clone(), name: text_or_empty(p.get("name")), bytes: num_or0(p.get("bytes")), total: num_or0(p.get("total")), t: now };
                match self.dls.iter_mut().find(|x| x.id == id) {
                    Some(x) => *x = d,
                    None => self.dls.push(d),
                }
                self.render(env, now)
            }
            // cancelled: the temp file disappeared and no finished file appeared
            "gone" => {
                self.dls.retain(|d| d.id != id);
                self.render(env, now)
            }
            "done" => {
                self.dls.retain(|d| d.id != id);
                let mut out = self.render(env, now);
                if !env.live_on("downloads") {
                    return out;
                }
                out.push(LiveCmd::react("download", Preset::File, Some(3000), None)); // edge reaction
                let name = text_or_empty(p.get("name"));
                let path = text_or_empty(p.get("path"));
                let sub = format!("{}{}", cut_name(&name, 44), if truthy(p.get("bytes")) { format!(" · {}", fmt_size(num_or0(p.get("bytes")), env.is_ru())) } else { String::new() });
                let mut card = CardSpec::text(format!("download:{id}"), 9000, env.tr("Загрузка завершена", "Download complete"), Some(sub))
                    .icon(IconName::Check)
                    .tone(Tone::Green)
                    .actions(vec![
                        // Rust accepts only files inside Downloads
                        CardAction::new(env.tr("Показать в папке", "Show in folder"), ActionKind::White, Action::invoke("livesys_reveal", json!({ "path": path }))),
                        CardAction::close(env.tr("Закрыть", "Close")),
                    ]);
                card.enter = Anim::Slide;
                out.push(LiveCmd::Toast(card));
                out
            }
            _ => Vec::new(),
        }
    }

    /// Downloads currently tracked (id, name, bytes), oldest first.
    pub fn tracked(&self) -> Vec<(String, String, f64)> {
        self.dls.iter().map(|d| (d.id.clone(), d.name.clone(), d.bytes)).collect()
    }
}

impl Provider for DownloadLive {
    fn name(&self) -> &'static str {
        "download"
    }
    fn events(&self) -> &'static [&'static str] {
        &["live:download", "settings:changed"]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "live:download" => self.download(env, p, now),
            "settings:changed" if !env.live_on("downloads") => {
                self.dls.clear();
                self.render(env, now)
            }
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ updater

/// `update:available` (updater.rs found a newer release; only when "check on start" is on).
#[derive(Debug, Default)]
pub struct UpdateLive;

impl Provider for UpdateLive {
    fn name(&self) -> &'static str {
        "update"
    }
    fn events(&self) -> &'static [&'static str] {
        &["update:available"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        let version = if p.is_null() { String::new() } else { text_or_empty(p.get("version")) };
        let card = CardSpec::text("update", 9000, env.tr("Доступна новая версия Pill", "A new Pill version is available"), Some(version))
            .icon(IconName::Bell)
            .tone(Tone::Green)
            .actions(vec![
                CardAction::new(env.tr("Открыть", "Open"), ActionKind::White, Action::invoke("settings_open", json!({ "page": "updates" }))),
                CardAction::close(env.tr("Позже", "Later")),
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
    use crate::live::env::{MemEnv, PillState};

    const T0: i64 = 1_700_000_000_000;

    fn row_of(cmds: &[LiveCmd]) -> &RowSpec {
        match cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }) {
            Some(r) => r,
            None => panic!("no row in {cmds:?}"),
        }
    }
    fn toast_of(cmds: &[LiveCmd]) -> &CardSpec {
        match cmds.iter().find_map(|c| if let LiveCmd::Toast(t) = c { Some(t) } else { None }) {
            Some(r) => r,
            None => panic!("no toast in {cmds:?}"),
        }
    }
    fn text_at(r: &RowSpec, i: usize) -> String {
        match &r.elements[i] {
            Elem::Text { text, .. } => text.clone(),
            other => panic!("not text: {other:?}"),
        }
    }

    // ---------------- volume
    #[test]
    fn volume_hud_row() {
        let mut v = VolumeLive::default();
        let env = MemEnv::default();
        let cmds = v.volume(&env, &json!({"level": 42, "muted": false}), T0);
        let r = row_of(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("volume", 88, Some(1600)));
        assert_eq!(r.anim, RowAnim::new(Anim::Pop, Anim::Glide(120.0), Anim::Fade));
        assert_eq!(r.elements[0], Elem::icon_c(IconName::Volume, 17.0, ColorRole::Text));
        assert_eq!(r.elements[1], Elem::Bar { value: 0.42, width: 56.0, height: 6.0, glide_ms: 120, muted: false });
        assert_eq!(text_at(r, 2), "42");
    }

    #[test]
    fn volume_updates_in_place_then_rebuilds_after_expiry() {
        let mut v = VolumeLive::default();
        let env = MemEnv::default();
        let k1 = row_of(&v.volume(&env, &json!({"level": 10}), T0)).key.clone();
        let k2 = row_of(&v.volume(&env, &json!({"level": 12}), T0 + 400)).key.clone();
        assert_eq!(k1, k2, "within 1.6 s the same chip is edited");
        let k3 = row_of(&v.volume(&env, &json!({"level": 14}), T0 + 400 + 1600)).key.clone();
        assert_ne!(k2, k3, "after the HUD expired a new one pops in");
    }

    #[test]
    fn volume_muted_and_zero() {
        let mut v = VolumeLive::default();
        let env = MemEnv::default();
        let r = row_of(&v.volume(&env, &json!({"level": 60, "muted": true}), T0)).clone();
        assert_eq!(r.elements[0], Elem::icon_c(IconName::Mute, 17.0, ColorRole::Dim));
        assert_eq!(text_at(&r, 2), "0");
        assert!(matches!(r.elements[1], Elem::Bar { value, muted: true, .. } if value == 0.0));
        assert!(matches!(&r.elements[2], Elem::Text { style: TextStyle::VolNumMuted, .. }));
        // level 0 shows the mute glyph but is not "muted" styled
        let r = row_of(&v.volume(&env, &json!({"level": 0, "muted": false}), T0 + 10)).clone();
        assert_eq!(r.elements[0], Elem::icon_c(IconName::Mute, 17.0, ColorRole::Text));
        assert!(matches!(&r.elements[2], Elem::Text { style: TextStyle::VolNum, .. }));
    }

    #[test]
    fn volume_clamps_and_tolerates_garbage() {
        let mut v = VolumeLive::default();
        let env = MemEnv::default();
        assert_eq!(text_at(row_of(&v.volume(&env, &json!({"level": 250}), T0)), 2), "100");
        assert_eq!(text_at(row_of(&v.volume(&env, &json!({"level": -9}), T0)), 2), "0");
        assert_eq!(text_at(row_of(&v.volume(&env, &json!({}), T0)), 2), "0"); // missing level = 0
        assert!(v.volume(&env, &Value::Null, T0).is_empty());
        assert_eq!(text_at(row_of(&v.volume(&env, &json!({"level": "55"}), T0)), 2), "55");
    }

    #[test]
    fn volume_gates() {
        let mut v = VolumeLive::default();
        let mut env = MemEnv::default();
        env.state = PillState::Expanded;
        assert!(v.volume(&env, &json!({"level": 30}), T0).is_empty());
        assert_eq!(v.level(), 30, "the level is remembered even when no HUD is shown");
        env.state = PillState::Collapsed;
        env.set("live.volume", json!(false));
        assert!(v.volume(&env, &json!({"level": 31}), T0).is_empty());
        env.unset("live.volume");
        env.set("module.media", json!(false));
        assert!(!v.volume(&env, &json!({"level": 32}), T0).is_empty(), "controls module still on");
        env.set("module.controls", json!(false));
        assert!(v.volume(&env, &json!({"level": 33}), T0).is_empty(), "both modules off");
    }

    #[test]
    fn volume_switch_off_clears_the_chip() {
        let mut v = VolumeLive::default();
        let mut env = MemEnv::default();
        assert!(v.on_event(&env, "settings:changed", &Value::Null, T0).is_empty());
        env.set("live.volume", json!(false));
        assert_eq!(v.on_event(&env, "settings:changed", &Value::Null, T0), vec![LiveCmd::clear("volume")]);
    }

    struct FakeVol(Option<(i32, bool)>, u32);
    impl VolumeIo for FakeVol {
        fn get(&mut self) -> Option<(i32, bool)> {
            self.1 += 1;
            self.0
        }
    }

    #[test]
    fn wheel_steps_and_unmutes() {
        let mut v = VolumeLive::default();
        let mut io = FakeVol(Some((50, true)), 0);
        let cmds = v.wheel(1, &mut io);
        assert_eq!(
            cmds,
            vec![LiveCmd::Run(Action::invoke("media_mute", json!({"muted": false}))), LiveCmd::Run(Action::invoke("media_volume_set", json!({"v": 52})))]
        );
        assert_eq!(io.1, 1);
        // known level: no second read; down keeps mute state untouched
        let cmds = v.wheel(-1, &mut io);
        assert_eq!(cmds, vec![LiveCmd::Run(Action::invoke("media_volume_set", json!({"v": 50})))]);
        assert_eq!(io.1, 1);
    }

    #[test]
    fn wheel_clamps_and_survives_a_failed_read() {
        let mut v = VolumeLive::default();
        assert!(v.wheel(1, &mut FakeVol(None, 0)).is_empty());
        assert_eq!(v.level(), -1);
        let mut v = VolumeLive::default();
        v.wheel(1, &mut FakeVol(Some((99, false)), 0));
        assert_eq!(v.level(), 100);
        let mut v = VolumeLive::default();
        let c = v.wheel(-1, &mut FakeVol(Some((1, false)), 0));
        assert_eq!(c, vec![LiveCmd::Run(Action::invoke("media_volume_set", json!({"v": 0})))]);
        // a muted player stays muted when scrolling down
        let mut v = VolumeLive::default();
        v.wheel(-1, &mut FakeVol(Some((40, true)), 0));
        assert!(v.muted());
    }

    #[test]
    fn wheel_allowed_gate() {
        let mut env = MemEnv::default();
        assert!(VolumeLive::wheel_allowed(&env, -3.0));
        assert!(!VolumeLive::wheel_allowed(&env, 0.0));
        env.state = PillState::Expanded;
        assert!(!VolumeLive::wheel_allowed(&env, -3.0));
        env.state = PillState::Collapsed;
        env.set("live.volume", json!(false));
        assert!(!VolumeLive::wheel_allowed(&env, -3.0));
    }

    // ---------------- privacy
    #[test]
    fn privacy_dots_follow_the_payload_and_dedupe() {
        let mut p = PrivacyLive::default();
        let env = MemEnv::default();
        assert!(p.on_event(&env, "live:privacy", &json!({"mic": false, "cam": false, "apps": []}), T0).is_empty());
        assert_eq!(p.on_event(&env, "live:privacy", &json!({"mic": true, "cam": false, "apps": ["Zoom.exe"]}), T0), vec![LiveCmd::PrivacyDots { cam: false, mic: true }]);
        assert!(p.on_event(&env, "live:privacy", &json!({"mic": true, "cam": false, "apps": ["Zoom.exe", "x"]}), T0).is_empty());
        assert_eq!(p.on_event(&env, "live:privacy", &json!({"mic": true, "cam": true}), T0), vec![LiveCmd::PrivacyDots { cam: true, mic: true }]);
        assert_eq!(p.on_event(&env, "live:privacy", &json!({"mic": false, "cam": false}), T0), vec![LiveCmd::PrivacyDots { cam: false, mic: false }]);
        assert!(p.on_event(&env, "live:privacy", &Value::Null, T0).is_empty());
    }

    #[test]
    fn privacy_apps_capped_at_four_and_stringified() {
        let mut p = PrivacyLive::default();
        let env = MemEnv::default();
        p.on_event(&env, "live:privacy", &json!({"mic": true, "apps": ["a", "b", "c", "d", "e", 5]}), T0);
        assert_eq!(p.apps(), ["a", "b", "c", "d"]);
    }

    #[test]
    fn privacy_switch_hides_and_restores_the_dots() {
        let mut p = PrivacyLive::default();
        let mut env = MemEnv::default();
        p.on_event(&env, "live:privacy", &json!({"mic": true, "cam": true}), T0);
        env.set("live.micCam", json!(false));
        assert_eq!(p.on_event(&env, "settings:changed", &Value::Null, T0), vec![LiveCmd::PrivacyDots { cam: false, mic: false }]);
        env.unset("live.micCam");
        assert_eq!(p.on_event(&env, "settings:changed", &Value::Null, T0), vec![LiveCmd::PrivacyDots { cam: true, mic: true }]);
        assert!(p.on_event(&env, "settings:changed", &Value::Null, T0).is_empty());
    }

    #[test]
    fn hide_show_pause_the_backend_polls() {
        let mut p = PrivacyLive::default();
        let env = MemEnv::default();
        assert_eq!(p.on_event(&env, "pill:hide", &Value::Null, T0), vec![LiveCmd::Run(Action::invoke("livesys_visible", json!({"on": false})))]);
        assert_eq!(p.on_event(&env, "pill:show", &Value::Null, T0), vec![LiveCmd::Run(Action::invoke("livesys_visible", json!({"on": true})))]);
        assert_eq!(p.boot(), vec![Query::new("livesys_privacy", "live:privacy")]);
    }

    // ---------------- downloads
    fn prog(id: &str, name: &str, bytes: u64, total: Value) -> Value {
        json!({"state": "progress", "id": id, "name": name, "bytes": bytes, "total": total, "path": format!("C:\\Downloads\\{name}")})
    }

    #[test]
    fn download_chip_text_ru_en() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        let r = row_of(&d.download(&env, &prog("a", "setup.exe", 5 * 1048576, Value::Null), T0)).clone();
        assert_eq!((r.priority, r.ttl_ms), (55, Some(3500)));
        assert_eq!(r.elements[0], Elem::Ring { value: None, size: 18.0, glide_ms: 400 }); // total unknown: spinner
        assert_eq!(text_at(&r, 1), "setup.exe");
        assert_eq!(text_at(&r, 2), "5.0 МБ");
        assert_eq!(r.elements.len(), 3);
        let en = MemEnv::en();
        let mut d = DownloadLive::default();
        let r = row_of(&d.download(&en, &prog("a", "setup.exe", 2048, Value::Null), T0)).clone();
        assert_eq!(text_at(&r, 2), "2.0 KB");
    }

    #[test]
    fn download_label_fallback_and_long_names() {
        let env = MemEnv::default();
        let mut d = DownloadLive::default();
        assert_eq!(text_at(row_of(&d.download(&env, &prog("a", "", 10, Value::Null), T0)), 1), "Загрузка");
        let mut d = DownloadLive::default();
        assert_eq!(text_at(row_of(&d.download(&MemEnv::en(), &prog("a", "", 10, Value::Null), T0)), 1), "Downloading");
        let mut d = DownloadLive::default();
        let t = text_at(row_of(&d.download(&env, &prog("a", "очень-длинное-имя-файла-с-кириллицей-и-цифрами-12345.pdf", 10, Value::Null), T0)), 1);
        assert!(t.ends_with("….pdf") && u16_len(&t) == 28, "{t}");
    }

    #[test]
    fn download_ring_percent_and_more() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        d.download(&env, &prog("a", "a.zip", 10, json!(100)), T0);
        let r = row_of(&d.download(&env, &prog("b", "b.zip", 50, json!(200)), T0 + 1000)).clone();
        // the most recently updated download is shown, the other counts as "+1"
        assert_eq!(text_at(&r, 1), "b.zip");
        assert_eq!(r.elements[0], Elem::Ring { value: Some(0.25), size: 18.0, glide_ms: 400 });
        assert_eq!(text_at(&r, 3), "+1");
        // tiny progress still shows at least 1 %
        let mut d = DownloadLive::default();
        let r = row_of(&d.download(&env, &prog("a", "a", 1, json!(1_000_000)), T0)).clone();
        assert_eq!(r.elements[0], Elem::Ring { value: Some(0.01), size: 18.0, glide_ms: 400 });
    }

    #[test]
    fn download_same_id_updates_in_place_new_id_rebuilds() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        let k1 = row_of(&d.download(&env, &prog("a", "a.zip", 10, Value::Null), T0)).key.clone();
        let k2 = row_of(&d.download(&env, &prog("a", "a.zip", 20, Value::Null), T0 + 1000)).key.clone();
        assert_eq!(k1, k2);
        let k3 = row_of(&d.download(&env, &prog("b", "b.zip", 20, Value::Null), T0 + 2000)).key.clone();
        assert_ne!(k2, k3);
        // chip expired (3.5 s with no progress), same id comes back: fresh chip
        let k4 = row_of(&d.download(&env, &prog("b", "b.zip", 30, Value::Null), T0 + 2000 + 3500)).key.clone();
        assert_ne!(k3, k4);
    }

    #[test]
    fn download_stale_entries_dropped() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        d.download(&env, &prog("a", "a.zip", 10, Value::Null), T0);
        d.download(&env, &prog("b", "b.zip", 10, Value::Null), T0 + 14_000);
        assert_eq!(d.tracked().len(), 2);
        let cmds = d.download(&env, &prog("b", "b.zip", 20, Value::Null), T0 + 15_001);
        assert_eq!(d.tracked().len(), 1, "a was silent for 15 s");
        assert_eq!(text_at(row_of(&cmds), 1), "b.zip");
        assert_eq!(row_of(&cmds).elements.len(), 3, "no +N any more");
    }

    #[test]
    fn download_gone_clears_the_chip() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        d.download(&env, &prog("a", "a.zip", 10, Value::Null), T0);
        assert_eq!(d.download(&env, &json!({"state": "gone", "id": "a"}), T0 + 10), vec![LiveCmd::clear("download")]);
        // nothing shown any more: a second "gone" is silent
        assert!(d.download(&env, &json!({"state": "gone", "id": "a"}), T0 + 20).is_empty());
    }

    #[test]
    fn download_done_card_and_reaction() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        d.download(&env, &prog("a", "report.pdf", 10, Value::Null), T0);
        let cmds = d.download(&env, &json!({"state": "done", "id": "a", "name": "report.pdf", "bytes": 3 * 1048576, "total": null, "path": "C:\\Downloads\\report.pdf"}), T0 + 100);
        assert_eq!(cmds[0], LiveCmd::clear("download"));
        assert_eq!(cmds[1], LiveCmd::react("download", Preset::File, Some(3000), None));
        let t = toast_of(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("download:a", 9000, Tone::Green));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Check)));
        assert_eq!(t.body, CardBody::Text { title: "Загрузка завершена".into(), sub: Some("report.pdf · 3.0 МБ".into()) });
        assert_eq!(t.actions.len(), 2);
        assert_eq!(t.actions[0].label, "Показать в папке");
        assert_eq!(t.actions[0].kind, ActionKind::White);
        assert_eq!(t.actions[0].action, Action::invoke("livesys_reveal", json!({"path": "C:\\Downloads\\report.pdf"})));
        assert_eq!(t.actions[1].label, "Закрыть");
    }

    #[test]
    fn download_done_en_without_size() {
        let mut d = DownloadLive::default();
        let env = MemEnv::en();
        let cmds = d.download(&env, &json!({"state": "done", "id": "z", "name": "a.bin", "bytes": 0, "path": "p"}), T0);
        let t = toast_of(&cmds);
        assert_eq!(t.body, CardBody::Text { title: "Download complete".into(), sub: Some("a.bin".into()) });
        assert_eq!(t.actions[0].label, "Show in folder");
        assert_eq!(t.actions[1].label, "Close");
    }

    #[test]
    fn download_switch_off_means_no_card_and_no_chip() {
        let mut d = DownloadLive::default();
        let mut env = MemEnv::default();
        d.download(&env, &prog("a", "a.zip", 10, Value::Null), T0);
        env.set("live.downloads", json!(false));
        assert!(d.download(&env, &prog("a", "a.zip", 20, Value::Null), T0 + 10).is_empty());
        let cmds = d.download(&env, &json!({"state": "done", "id": "a", "name": "a.zip", "bytes": 1, "path": "p"}), T0 + 20);
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::Toast(_) | LiveCmd::React { .. })), "{cmds:?}");
        // settings:changed with the switch off clears a standing chip
        let mut d = DownloadLive::default();
        let mut env = MemEnv::default();
        d.download(&env, &prog("a", "a.zip", 10, Value::Null), T0);
        env.set("live.downloads", json!(false));
        assert_eq!(d.on_event(&env, "settings:changed", &Value::Null, T0 + 5), vec![LiveCmd::clear("download")]);
        assert!(d.tracked().is_empty());
    }

    #[test]
    fn download_ignores_malformed_events() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        assert!(d.download(&env, &Value::Null, T0).is_empty());
        assert!(d.download(&env, &json!({}), T0).is_empty());
        assert!(d.download(&env, &json!({"state": "weird", "id": "a"}), T0).is_empty());
        assert!(d.download(&env, &json!({"state": "progress"}), T0).is_empty(), "no id");
        // bytes/total as strings or missing do not break
        let r = row_of(&d.download(&env, &json!({"state": "progress", "id": "q", "bytes": "2048"}), T0)).clone();
        assert_eq!(text_at(&r, 2), "2.0 КБ");
    }

    #[test]
    fn download_unicode_name() {
        let mut d = DownloadLive::default();
        let env = MemEnv::default();
        let r = row_of(&d.download(&env, &prog("u", "отчёт 📄.docx", 1, Value::Null), T0)).clone();
        assert_eq!(text_at(&r, 1), "отчёт 📄.docx");
    }

    // ---------------- update card
    #[test]
    fn update_card() {
        let mut u = UpdateLive;
        let env = MemEnv::default();
        let cmds = u.on_event(&env, "update:available", &json!({"version": "0.4.0"}), T0);
        let t = toast_of(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone), ("update", 9000, Tone::Green));
        assert_eq!(t.icon, Some(CardIcon::Name(IconName::Bell)));
        assert_eq!(t.body, CardBody::Text { title: "Доступна новая версия Pill".into(), sub: Some("0.4.0".into()) });
        assert_eq!(t.actions[0].action, Action::invoke("settings_open", json!({"page": "updates"})));
        assert_eq!(t.actions[1].label, "Позже");
        let cmds = u.on_event(&MemEnv::en(), "update:available", &Value::Null, T0);
        let t = toast_of(&cmds);
        assert_eq!(t.body, CardBody::Text { title: "A new Pill version is available".into(), sub: Some(String::new()) });
        assert_eq!(t.actions[1].label, "Later");
    }
}
