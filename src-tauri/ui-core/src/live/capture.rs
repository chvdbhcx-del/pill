//! Capture lane, pill side (port of capture-live.js; Rust side capture.rs): the recording capsule, the Home
//! "Record" tile turning into "Stop 0:NN", and the result cards (screenshot, screen text, QR, colour, recording, errors).
//!
//! * `rec:state {recording, startedAt, mode, fps}` -> row `rec` (priority 100, no ttl): red dot pulsing 2.0 s, white `m:ss` timer
//!   that changes once a second, grey MP4/GIF tag, area icon, red Stop button 56x24 (258 px wide capsule). Ticks at 250 ms.
//! * `capture:done {kind, ...}` -> one card per kind (id `cap`, so a new result replaces the previous card).
//!
//! Settings: Settings > Live events > Screen recording (`live.recording`) hides the capsule only; `capture.format` picks the MP4/GIF tag.

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const REC_PRIORITY: i32 = 100;
pub const REC_TICK_MS: u32 = 250;
pub const CAPSULE_W: f32 = 258.0;
/// The tiles collapse the pill first so it is not part of the frozen frame, then ask Rust to start.
pub const START_DELAY_MS: u32 = 450;

fn act(action: &str, value: &str) -> Action {
    Action::invoke("capture_action", json!({ "action": action, "value": value }))
}

/// What a Home capture tile does (`PillCapture.start`).
#[derive(Clone, Debug, PartialEq)]
pub struct StartPlan {
    pub collapse_first: bool,
    pub delay_ms: u32,
    pub action: Action,
}

/// `record_screen` while recording toggles at once; everything else collapses the pill, waits 450 ms, then starts.
pub fn start_plan(mode: &str, recording: bool) -> StartPlan {
    let action = Action::invoke("capture_start", json!({ "mode": mode }));
    if mode == "record_screen" && recording {
        StartPlan { collapse_first: false, delay_ms: 0, action }
    } else {
        StartPlan { collapse_first: true, delay_ms: START_DELAY_MS, action }
    }
}

// ================================================================ recording capsule

#[derive(Debug, Default)]
pub struct RecLive {
    started_at: Option<i64>,
    hidden: bool,
    shown: String,
}

impl RecLive {
    pub fn recording(&self) -> bool {
        self.started_at.is_some()
    }

    fn clock(&self, now: i64) -> String {
        clock_hms(now.saturating_sub(self.started_at.unwrap_or(now)))
    }

    fn capsule(&self, env: &dyn Env, now: i64) -> RowSpec {
        let gif = env.setting_str("capture.format").as_deref() == Some("gif");
        RowSpec::new("rec", REC_PRIORITY)
            .key("rec")
            .width(CAPSULE_W)
            .tip(env.tr("Идёт запись экрана", "Recording the screen"))
            .anim(Anim::Pop, Anim::None, Anim::Fade)
            .with(Elem::Dot { color: palette::REC_DOT, size: 10.0, halo: false, pulse_ms: Some(2000) })
            .with(Elem::Clock { text: self.clock(now), since: self.started_at.unwrap_or(now), style: TextStyle::Clock })
            .with(Elem::Flex)
            .with(Elem::Badge { text: if gif { "GIF" } else { "MP4" }.to_string() })
            .with(Elem::icon_c(IconName::Area, 12.0, ColorRole::Dim))
            .with(Elem::Button {
                action: Action::invoke0("capture_stop"),
                label: Some(env.tr("Стоп", "Stop")),
                icon: None,
                style: ButtonStyle::Stop { width: 56.0 },
                tip: env.tr("Остановить запись (Ctrl+Alt+R)", "Stop recording (Ctrl+Alt+R)"),
            })
    }

    fn tile(&self, env: &dyn Env, now: i64) -> LiveCmd {
        LiveCmd::RecTile(Some(RecTile { label: env.tr("Стоп", "Stop"), clock: self.clock(now) }))
    }

    fn start(&mut self, env: &dyn Env, started_at: &Value, now: i64) -> Vec<LiveCmd> {
        let t = num_or0(Some(started_at)).trunc() as i64;
        self.started_at = Some(if t != 0 { t } else { now });
        self.shown = self.clock(now);
        let mut out = Vec::new();
        if env.live_on("recording") {
            out.push(LiveCmd::set(self.capsule(env, now)));
        }
        out.push(self.tile(env, now));
        out
    }

    fn stop(&mut self) -> Vec<LiveCmd> {
        self.started_at = None;
        self.shown.clear();
        vec![LiveCmd::clear("rec"), LiveCmd::RecTile(None)]
    }

    fn repaint(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        let s = self.clock(now);
        if s == self.shown {
            return Vec::new();
        }
        self.shown = s;
        let mut out = Vec::new();
        if env.live_on("recording") {
            out.push(LiveCmd::set(self.capsule(env, now)));
        }
        out.push(self.tile(env, now));
        out
    }
}

impl Provider for RecLive {
    fn name(&self) -> &'static str {
        "rec"
    }
    fn events(&self) -> &'static [&'static str] {
        &["rec:state", "capture:settings", "settings:changed", "pill:hide", "pill:show"]
    }
    fn boot(&self) -> Vec<Query> {
        // a page reload while recording: restore the capsule
        vec![Query::new("capture_settings", "capture:settings")]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "rec:state" => {
                if truthy(p.get("recording")) {
                    self.start(env, p.get("startedAt").unwrap_or(&Value::Null), now)
                } else {
                    self.stop()
                }
            }
            "capture:settings" => {
                if !p.is_null() && truthy(p.get("recording")) && truthy(p.get("recStartedAt")) {
                    self.start(env, p.get("recStartedAt").unwrap_or(&Value::Null), now)
                } else {
                    Vec::new()
                }
            }
            // the switch works while a recording runs
            "settings:changed" => {
                if self.started_at.is_none() {
                    return Vec::new();
                }
                vec![if env.live_on("recording") { LiveCmd::set(self.capsule(env, now)) } else { LiveCmd::clear("rec") }]
            }
            // no ticks while the island is hidden (nobody sees the text)
            "pill:hide" => {
                self.hidden = true;
                Vec::new()
            }
            "pill:show" => {
                self.hidden = false;
                if self.started_at.is_some() { self.repaint(env, now) } else { Vec::new() }
            }
            _ => Vec::new(),
        }
    }
    fn on_tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if self.started_at.is_none() || self.hidden {
            return Vec::new();
        }
        self.repaint(env, now)
    }
    fn tick_ms(&self, _now: i64) -> Option<u32> {
        if self.started_at.is_some() && !self.hidden { Some(REC_TICK_MS) } else { None }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ================================================================ result cards

#[derive(Debug, Default)]
pub struct CaptureCards;

fn card(env: &dyn Env, ttl: u32, tone: Tone, icon: IconName, title: String, sub: Option<String>, actions: Vec<CardAction>) -> CardSpec {
    let _ = env;
    CardSpec::text("cap", ttl, title, sub).icon(icon).tone(tone).actions(actions)
}

impl CaptureCards {
    fn build(&self, env: &dyn Env, p: &Value) -> Vec<LiveCmd> {
        if p.is_null() || !truthy(p.get("kind")) {
            return Vec::new();
        }
        let tr = |ru: &str, en: &str| env.tr(ru, en);
        let path = text_or_empty(p.get("path"));
        match p.get("kind").and_then(Value::as_str).unwrap_or("") {
            "screenshot" => {
                let saved = truthy(p.get("path"));
                let mut out = Vec::new();
                if saved {
                    out.push(LiveCmd::react("capture", Preset::File, Some(3000), None));
                }
                let mut actions = Vec::new();
                if saved {
                    actions.push(CardAction::new(tr("Открыть", "Open"), ActionKind::White, act("open", &path)));
                    actions.push(CardAction::new(tr("В папке", "Show in folder"), ActionKind::Normal, act("reveal", &path)));
                    actions.push(CardAction::new(tr("Копировать", "Copy"), ActionKind::Normal, act("copy_image", &path)));
                }
                let (title, sub) = if saved {
                    (tr("Скриншот сохранён", "Screenshot saved"), base(&path))
                } else {
                    (tr("Скриншот скопирован", "Screenshot copied"), format!("{}×{}", display(p.get("w")), display(p.get("h"))))
                };
                out.push(LiveCmd::Toast(card(env, 7000, Tone::Green, IconName::Monitor, title, Some(sub), actions)));
                out
            }
            "text" => {
                let c = if truthy(p.get("found")) {
                    let body = text_or_empty(p.get("text"));
                    let more = if to_number(p.get("chars")) > 72.0 { format!(" · {}", display(p.get("chars"))) } else { String::new() };
                    card(env, 6000, Tone::Green, IconName::ScanText, tr("Текст скопирован", "Text copied"), Some(format!("{}{more}", cut(&body, 72))), Vec::new())
                } else {
                    card(
                        env,
                        4500,
                        Tone::Amber,
                        IconName::ScanText,
                        tr("Текст не найден", "No text found"),
                        Some(tr("Попробуй выделить область покрупнее", "Try a larger area")),
                        Vec::new(),
                    )
                };
                vec![LiveCmd::Toast(c)]
            }
            "qr" => {
                if !truthy(p.get("found")) {
                    return vec![LiveCmd::Toast(card(
                        env,
                        4500,
                        Tone::Amber,
                        IconName::Qr,
                        tr("QR-код не найден", "No QR code found"),
                        Some(tr("Выдели код целиком, с белой рамкой", "Select the whole code, with its white border")),
                        Vec::new(),
                    ))];
                }
                let body = text_or_empty(p.get("text"));
                let mut actions = Vec::new();
                if truthy(p.get("isUrl")) {
                    actions.push(CardAction::new(tr("Открыть ссылку", "Open link"), ActionKind::White, act("open_url", &body)));
                }
                actions.push(CardAction::new(tr("Копировать", "Copy"), ActionKind::Normal, act("copy_text", &body)));
                vec![LiveCmd::Toast(card(env, 12000, Tone::Accent, IconName::Qr, tr("QR-код", "QR code"), Some(cut(&body, 90)), actions))]
            }
            "color" => {
                let hex = text_or_empty(p.get("hex"));
                let rgb = p.get("rgb").and_then(Value::as_array).map(|a| a.iter().map(|x| display(Some(x))).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let history: Vec<Swatch> = p
                    .get("history")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|h| text(Some(h))).filter(|h| *h != hex).take(9).map(|h| Swatch { action: act("copy_text", &h), hex: h }).collect())
                    .unwrap_or_default();
                let c = CardSpec::new("cap", 9000, CardBody::Color { hex, rgb, copied: tr("скопировано", "copied"), history });
                vec![LiveCmd::Toast(c)]
            }
            "record" => {
                let mut out = Vec::new();
                if truthy(p.get("path")) {
                    out.push(LiveCmd::react("capture", Preset::File, Some(3000), None));
                }
                let secs = num_or0(p.get("seconds"));
                let sub = format!("{} · {}", base(&path), clock_hms((secs * 1000.0).trunc() as i64));
                out.push(LiveCmd::Toast(card(
                    env,
                    9000,
                    Tone::Green,
                    IconName::Video,
                    tr("Запись сохранена", "Recording saved"),
                    Some(sub),
                    vec![
                        CardAction::new(tr("Открыть", "Open"), ActionKind::White, act("open", &path)),
                        CardAction::new(tr("В папке", "Show in folder"), ActionKind::Normal, act("reveal", &path)),
                    ],
                )));
                out
            }
            "error" => vec![LiveCmd::Toast(card(env, 10000, Tone::Red, IconName::X, tr("Не получилось", "Capture failed"), Some(cut(&text_or_empty(p.get("message")), 180)), Vec::new()))],
            _ => Vec::new(),
        }
    }
}

impl Provider for CaptureCards {
    fn name(&self) -> &'static str {
        "capture-cards"
    }
    fn events(&self) -> &'static [&'static str] {
        &["capture:done"]
    }
    fn on_event(&mut self, env: &dyn Env, _name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        self.build(env, p)
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
    fn tile(cmds: &[LiveCmd]) -> Option<RecTile> {
        cmds.iter().find_map(|c| if let LiveCmd::RecTile(t) = c { Some(t.clone()) } else { None }).flatten()
    }
    fn toast(cmds: &[LiveCmd]) -> &CardSpec {
        cmds.iter().find_map(|c| if let LiveCmd::Toast(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no toast in {cmds:?}"))
    }
    fn ts(c: &CardSpec) -> (String, Option<String>) {
        match &c.body {
            CardBody::Text { title, sub } => (title.clone(), sub.clone()),
            o => panic!("{o:?}"),
        }
    }
    fn clock_of(r: &RowSpec) -> String {
        r.elements.iter().find_map(|e| if let Elem::Clock { text, .. } = e { Some(text.clone()) } else { None }).unwrap()
    }
    fn done(env: &MemEnv, v: Value) -> Vec<LiveCmd> {
        CaptureCards.on_event(env, "capture:done", &v, T0)
    }

    // ---------------- capsule
    #[test]
    fn capsule_layout() {
        let mut r = RecLive::default();
        let env = MemEnv::default();
        let cmds = r.on_event(&env, "rec:state", &json!({"recording": true, "startedAt": T0 - 65_000, "mode": "screen", "fps": 30}), T0);
        let row = row(&cmds);
        assert_eq!((row.id.as_str(), row.priority, row.ttl_ms, row.width), ("rec", 100, None, 258.0));
        assert_eq!(row.elements.len(), 6);
        assert_eq!(row.elements[0], Elem::Dot { color: palette::REC_DOT, size: 10.0, halo: false, pulse_ms: Some(2000) });
        assert_eq!(clock_of(row), "1:05");
        assert_eq!(row.elements[2], Elem::Flex);
        assert_eq!(row.elements[3], Elem::Badge { text: "MP4".into() });
        assert!(matches!(row.elements[4], Elem::Icon { name: IconName::Area, .. }));
        match &row.elements[5] {
            Elem::Button { action, label, style, tip, .. } => {
                assert_eq!(*action, Action::invoke0("capture_stop"));
                assert_eq!(label.as_deref(), Some("Стоп"));
                assert_eq!(*style, ButtonStyle::Stop { width: 56.0 });
                assert_eq!(tip, "Остановить запись (Ctrl+Alt+R)");
            }
            o => panic!("{o:?}"),
        }
        assert_eq!(row.tip.as_deref(), Some("Идёт запись экрана"));
        assert_eq!(tile(&cmds), Some(RecTile { label: "Стоп".into(), clock: "1:05".into() }));
        assert_eq!(r.tick_ms(T0), Some(250));
    }

    #[test]
    fn capsule_gif_and_english() {
        let mut r = RecLive::default();
        let mut env = MemEnv::en();
        env.set("capture.format", json!("gif"));
        let cmds = r.on_event(&env, "rec:state", &json!({"recording": true, "startedAt": T0}), T0);
        let row = row(&cmds);
        assert_eq!(row.elements[3], Elem::Badge { text: "GIF".into() });
        assert_eq!(row.tip.as_deref(), Some("Recording the screen"));
        assert!(matches!(&row.elements[5], Elem::Button { label: Some(l), tip, .. } if l == "Stop" && tip == "Stop recording (Ctrl+Alt+R)"));
        assert_eq!(tile(&cmds).unwrap().label, "Stop");
    }

    #[test]
    fn capsule_missing_start_uses_now_and_long_clock() {
        let mut r = RecLive::default();
        let env = MemEnv::default();
        let cmds = r.on_event(&env, "rec:state", &json!({"recording": true}), T0);
        assert_eq!(clock_of(row(&cmds)), "0:00");
        let tick = r.on_tick(&env, T0 + 3_661_000);
        assert_eq!(clock_of(row(&tick)), "1:01:01");
    }

    #[test]
    fn capsule_ticks_once_per_displayed_second_and_keeps_its_key() {
        let mut r = RecLive::default();
        let env = MemEnv::default();
        r.on_event(&env, "rec:state", &json!({"recording": true, "startedAt": T0}), T0);
        assert!(r.on_tick(&env, T0 + 250).is_empty());
        assert!(r.on_tick(&env, T0 + 750).is_empty());
        let t = r.on_tick(&env, T0 + 1000);
        assert_eq!(clock_of(row(&t)), "0:01");
        assert_eq!(row(&t).key, "rec");
        assert_eq!(tile(&t).unwrap().clock, "0:01");
    }

    #[test]
    fn capsule_hidden_pauses_ticks_and_catches_up() {
        let mut r = RecLive::default();
        let env = MemEnv::default();
        r.on_event(&env, "rec:state", &json!({"recording": true, "startedAt": T0}), T0);
        r.on_event(&env, "pill:hide", &Value::Null, T0 + 10);
        assert_eq!(r.tick_ms(T0 + 20), None);
        assert!(r.on_tick(&env, T0 + 9000).is_empty());
        let t = r.on_event(&env, "pill:show", &Value::Null, T0 + 12_000);
        assert_eq!(clock_of(row(&t)), "0:12");
        assert_eq!(r.tick_ms(T0 + 12_000), Some(250));
    }

    #[test]
    fn stop_clears_row_and_tile() {
        let mut r = RecLive::default();
        let env = MemEnv::default();
        r.on_event(&env, "rec:state", &json!({"recording": true, "startedAt": T0}), T0);
        let cmds = r.on_event(&env, "rec:state", &json!({"recording": false, "startedAt": T0}), T0 + 5000);
        assert_eq!(cmds, vec![LiveCmd::clear("rec"), LiveCmd::RecTile(None)]);
        assert!(!r.recording());
        assert_eq!(r.tick_ms(T0), None);
        assert!(r.on_tick(&env, T0 + 6000).is_empty());
        assert_eq!(r.on_event(&env, "rec:state", &Value::Null, T0), vec![LiveCmd::clear("rec"), LiveCmd::RecTile(None)]);
    }

    #[test]
    fn switch_hides_the_capsule_but_not_the_tile() {
        let mut r = RecLive::default();
        let mut env = MemEnv::default();
        env.set("live.recording", json!(false));
        let cmds = r.on_event(&env, "rec:state", &json!({"recording": true, "startedAt": T0}), T0);
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::SetRow(_))));
        assert!(tile(&cmds).is_some());
        // switching it on while the recording runs shows the capsule
        env.unset("live.recording");
        let cmds = r.on_event(&env, "settings:changed", &Value::Null, T0 + 3000);
        assert_eq!(clock_of(row(&cmds)), "0:03");
        env.set("live.recording", json!(false));
        assert_eq!(r.on_event(&env, "settings:changed", &Value::Null, T0 + 4000), vec![LiveCmd::clear("rec")]);
        // no recording: a settings change is silent
        let mut idle = RecLive::default();
        assert!(idle.on_event(&env, "settings:changed", &Value::Null, T0).is_empty());
    }

    #[test]
    fn restore_after_reload() {
        let mut r = RecLive::default();
        let env = MemEnv::default();
        assert_eq!(r.boot(), vec![Query::new("capture_settings", "capture:settings")]);
        assert!(r.on_event(&env, "capture:settings", &json!({"recording": false, "recStartedAt": 0}), T0).is_empty());
        assert!(r.on_event(&env, "capture:settings", &json!({"recording": true}), T0).is_empty(), "no start time: nothing to restore");
        assert!(r.on_event(&env, "capture:settings", &Value::Null, T0).is_empty());
        let cmds = r.on_event(&env, "capture:settings", &json!({"recording": true, "recStartedAt": T0 - 90_000}), T0);
        assert_eq!(clock_of(row(&cmds)), "1:30");
    }

    #[test]
    fn start_plans() {
        let p = start_plan("area", false);
        assert_eq!((p.collapse_first, p.delay_ms), (true, 450));
        assert_eq!(p.action, Action::invoke("capture_start", json!({"mode": "area"})));
        assert_eq!(start_plan("record_screen", false).delay_ms, 450);
        let p = start_plan("record_screen", true);
        assert_eq!((p.collapse_first, p.delay_ms), (false, 0));
    }

    // ---------------- cards
    #[test]
    fn screenshot_saved() {
        let env = MemEnv::default();
        let cmds = done(&env, json!({"kind": "screenshot", "path": "C:\\Pictures\\shot 1.png", "copied": true, "w": 1920, "h": 1080}));
        assert_eq!(cmds[0], LiveCmd::react("capture", Preset::File, Some(3000), None));
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone, t.icon.clone()), ("cap", 7000, Tone::Green, Some(CardIcon::Name(IconName::Monitor))));
        assert_eq!(ts(t), ("Скриншот сохранён".to_string(), Some("shot 1.png".to_string())));
        let acts: Vec<(&str, ActionKind)> = t.actions.iter().map(|a| (a.label.as_str(), a.kind)).collect();
        assert_eq!(acts, [("Открыть", ActionKind::White), ("В папке", ActionKind::Normal), ("Копировать", ActionKind::Normal)]);
        assert_eq!(t.actions[0].action, Action::invoke("capture_action", json!({"action": "open", "value": "C:\\Pictures\\shot 1.png"})));
        assert_eq!(t.actions[1].action, Action::invoke("capture_action", json!({"action": "reveal", "value": "C:\\Pictures\\shot 1.png"})));
        assert_eq!(t.actions[2].action, Action::invoke("capture_action", json!({"action": "copy_image", "value": "C:\\Pictures\\shot 1.png"})));
    }

    #[test]
    fn screenshot_copied_only() {
        let env = MemEnv::en();
        let cmds = done(&env, json!({"kind": "screenshot", "copied": true, "w": 800, "h": 600}));
        assert_eq!(cmds.len(), 1, "no reaction without a saved file");
        let t = toast(&cmds);
        assert_eq!(ts(t), ("Screenshot copied".to_string(), Some("800×600".to_string())));
        assert!(t.actions.is_empty());
        // missing size prints like a JS template literal
        assert_eq!(ts(toast(&done(&env, json!({"kind": "screenshot"})))).1, Some("undefined×undefined".to_string()));
    }

    #[test]
    fn screen_text_cards() {
        let env = MemEnv::default();
        let t = toast(&done(&env, json!({"kind": "text", "found": true, "text": "  hello\n  world ", "chars": 11, "copied": true}))).clone();
        assert_eq!((t.ttl_ms, t.tone, t.icon.clone()), (6000, Tone::Green, Some(CardIcon::Name(IconName::ScanText))));
        assert_eq!(ts(&t), ("Текст скопирован".to_string(), Some("hello world".to_string())));
        let long = "а".repeat(100);
        let t = toast(&done(&env, json!({"kind": "text", "found": true, "text": long, "chars": 100}))).clone();
        assert_eq!(ts(&t).1, Some(format!("{}… · 100", "а".repeat(71))));
        let t = toast(&done(&env, json!({"kind": "text", "found": false}))).clone();
        assert_eq!((t.ttl_ms, t.tone), (4500, Tone::Amber));
        assert_eq!(ts(&t), ("Текст не найден".to_string(), Some("Попробуй выделить область покрупнее".to_string())));
        let t = toast(&done(&MemEnv::en(), json!({"kind": "text", "found": false}))).clone();
        assert_eq!(ts(&t), ("No text found".to_string(), Some("Try a larger area".to_string())));
        assert_eq!(ts(toast(&done(&MemEnv::en(), json!({"kind": "text", "found": true, "text": "x"})))).0, "Text copied");
    }

    #[test]
    fn qr_cards() {
        let env = MemEnv::default();
        let t = toast(&done(&env, json!({"kind": "qr", "found": true, "text": "https://example.com/a", "isUrl": true}))).clone();
        assert_eq!((t.ttl_ms, t.tone, t.icon.clone()), (12000, Tone::Accent, Some(CardIcon::Name(IconName::Qr))));
        assert_eq!(ts(&t), ("QR-код".to_string(), Some("https://example.com/a".to_string())));
        assert_eq!(t.actions.len(), 2);
        assert_eq!(t.actions[0].label, "Открыть ссылку");
        assert_eq!(t.actions[0].action, Action::invoke("capture_action", json!({"action": "open_url", "value": "https://example.com/a"})));
        assert_eq!(t.actions[1].action, Action::invoke("capture_action", json!({"action": "copy_text", "value": "https://example.com/a"})));
        let t = toast(&done(&env, json!({"kind": "qr", "found": true, "text": "plain text", "isUrl": false}))).clone();
        assert_eq!(t.actions.len(), 1);
        let t = toast(&done(&MemEnv::en(), json!({"kind": "qr", "found": false}))).clone();
        assert_eq!((t.ttl_ms, t.tone), (4500, Tone::Amber));
        assert_eq!(ts(&t), ("No QR code found".to_string(), Some("Select the whole code, with its white border".to_string())));
        let t = toast(&done(&env, json!({"kind": "qr", "found": true, "text": "z".repeat(120)}))).clone();
        assert_eq!(ts(&t).1.unwrap().chars().count(), 90);
    }

    #[test]
    fn colour_card_filters_history() {
        let env = MemEnv::default();
        let cmds = done(&env, json!({"kind": "color", "hex": "#FF8800", "rgb": [255, 136, 0], "history": ["#FF8800", "#111111", "#222222", "#333333", "#444444", "#555555", "#666666", "#777777", "#888888", "#999999", "#aaaaaa"]}));
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.icon.clone(), t.tone), ("cap", 9000, None, Tone::Neutral));
        match &t.body {
            CardBody::Color { hex, rgb, copied, history } => {
                assert_eq!((hex.as_str(), rgb.as_str(), copied.as_str()), ("#FF8800", "255, 136, 0", "скопировано"));
                assert_eq!(history.len(), 9);
                assert_eq!(history[0].hex, "#111111");
                assert_eq!(history[0].action, Action::invoke("capture_action", json!({"action": "copy_text", "value": "#111111"})));
            }
            o => panic!("{o:?}"),
        }
        match &toast(&done(&MemEnv::en(), json!({"kind": "color", "hex": "#000"}))).body {
            CardBody::Color { rgb, copied, history, .. } => assert_eq!((rgb.as_str(), copied.as_str(), history.len()), ("", "copied", 0)),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn record_and_error_cards() {
        let env = MemEnv::default();
        let cmds = done(&env, json!({"kind": "record", "path": "D:\\Videos\\rec.mp4", "seconds": 75}));
        assert_eq!(cmds[0], LiveCmd::react("capture", Preset::File, Some(3000), None));
        let t = toast(&cmds);
        assert_eq!((t.ttl_ms, t.tone, t.icon.clone()), (9000, Tone::Green, Some(CardIcon::Name(IconName::Video))));
        assert_eq!(ts(t), ("Запись сохранена".to_string(), Some("rec.mp4 · 1:15".to_string())));
        assert_eq!(t.actions[0].label, "Открыть");
        assert_eq!(t.actions[1].action, Action::invoke("capture_action", json!({"action": "reveal", "value": "D:\\Videos\\rec.mp4"})));
        // no path: no reaction, empty name, zero length
        let cmds = done(&MemEnv::en(), json!({"kind": "record"}));
        assert_eq!(cmds.len(), 1);
        assert_eq!(ts(toast(&cmds)), ("Recording saved".to_string(), Some(" · 0:00".to_string())));
        assert_eq!(ts(toast(&done(&env, json!({"kind": "record", "path": "a.mp4", "seconds": 3725.9})))).1, Some("a.mp4 · 1:02:05".to_string()));
        let t = toast(&done(&env, json!({"kind": "error", "message": "no  encoder\nfound"}))).clone();
        assert_eq!((t.ttl_ms, t.tone, t.icon.clone()), (10000, Tone::Red, Some(CardIcon::Name(IconName::X))));
        assert_eq!(ts(&t), ("Не получилось".to_string(), Some("no encoder found".to_string())));
        assert_eq!(ts(toast(&done(&MemEnv::en(), json!({"kind": "error", "message": "m".repeat(200)})))).0, "Capture failed");
        assert_eq!(ts(toast(&done(&env, json!({"kind": "error"})))).1, Some(String::new()));
    }

    #[test]
    fn unknown_kinds_are_ignored() {
        let env = MemEnv::default();
        assert!(done(&env, Value::Null).is_empty());
        assert!(done(&env, json!({})).is_empty());
        assert!(done(&env, json!({"kind": "hologram"})).is_empty());
    }
}
