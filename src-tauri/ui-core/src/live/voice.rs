//! Voice to text, pill side (port of voice-live.js; Rust side voice.rs).
//! Events from Rust: `voice:state {state, startedAt?, text?, code?, message?}`, `voice:level {rms, level, test}`.
//!
//! * recording: row `voice` (priority 96, coral outline, 214 px): pulsing coral dot, `m:ss`, 5-bar level meter, red Stop button; edge `attention` for 1.6 s.
//! * transcribing: spinner row "Transcribing..." + edge `thinking`.
//! * done: green card "Text on the clipboard" (first 80 chars; Copy / Dismiss) + edge `done` for 2.2 s.
//! * error: amber / red card (never contains a key); `busy` is silent.
//!
//! The state event is re-broadcast in-app (`voice:state`, and `voice:done {text}`) for the chat microphone and the bar.
//! PRIVACY: the transcript is shown only as text and is not stored here.

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const VOICE_PRIORITY: i32 = 96;
pub const VOICE_TICK_MS: u32 = 250;
pub const CHIP_W: f32 = 214.0;
/// The centre bar follows the level most.
pub const BARS: [f32; 5] = [0.55, 0.8, 1.0, 0.8, 0.55];
const METER: Rgba = Rgba::rgb(0xff, 0x74, 0x68);

#[derive(Debug)]
pub struct VoiceLive {
    state: String,
    started_at: i64,
    lvl: f64,
    shown: f64,
    hidden: bool,
    last_clock: String,
}

impl Default for VoiceLive {
    fn default() -> Self {
        VoiceLive { state: "idle".into(), started_at: 0, lvl: 0.0, shown: 0.0, hidden: false, last_clock: String::new() }
    }
}

/// An error code of voice.rs: (tone, title, hint, offers the Settings button).
struct ErrInfo {
    tone: Tone,
    title: (&'static str, &'static str),
    hint: Option<(&'static str, &'static str)>,
    settings: bool,
}

fn err_info(code: &str) -> ErrInfo {
    let e = |tone, title, hint, settings| ErrInfo { tone, title, hint, settings };
    match code {
        "off" => e(Tone::Amber, ("Голос в текст выключен", "Voice to text is off"), Some(("Включи его в настройках", "Turn it on in Settings")), true),
        "no_key" => e(Tone::Amber, ("Нет ключа API", "No API key"), None, true),
        "no_endpoint" => e(Tone::Amber, ("Не указан адрес сервиса", "No service address"), None, true),
        "mic" => e(Tone::Red, ("Нет доступа к микрофону", "Cannot use the microphone"), None, false),
        "too_short" => e(Tone::Amber, ("Слишком короткая запись", "Recording too short"), Some(("Скажи что-нибудь подольше", "Say something longer")), false),
        "silent" => e(
            Tone::Amber,
            ("Микрофон ничего не услышал", "The microphone heard nothing"),
            Some(("Проверь микрофон по умолчанию и его громкость", "Check the default microphone and its volume")),
            false,
        ),
        "empty" => e(Tone::Amber, ("Речь не распознана", "No speech recognised"), None, false),
        "net" => e(Tone::Red, ("Нет связи с сервисом", "Cannot reach the service"), None, true),
        "http" => e(Tone::Red, ("Сервис вернул ошибку", "The service returned an error"), None, true),
        _ => e(Tone::Red, ("Диктовка не удалась", "Dictation failed"), None, false),
    }
}

impl VoiceLive {
    pub fn state(&self) -> &str {
        &self.state
    }

    fn clock(&self, now: i64) -> String {
        clock_ms(now.saturating_sub(self.started_at))
    }

    /// Bar heights (scaleY 0.14..1, two decimals) for the current smoothed level.
    pub fn meter_bars(shown: f64) -> Vec<f32> {
        BARS.iter()
            .map(|b| {
                let v = clamp_f(shown * f64::from(*b) + 0.06, 0.14, 1.0);
                to_fixed(v, 2).parse::<f32>().unwrap_or(0.14)
            })
            .collect()
    }

    fn rec_row(&self, env: &dyn Env, now: i64) -> RowSpec {
        RowSpec::new("voice", VOICE_PRIORITY)
            .key("voice-rec")
            .accent(Accent::Rgb(palette::VOICE))
            .width(CHIP_W)
            .tip(env.tr("Диктовка: говорите", "Dictating: speak now"))
            .anim(Anim::Pop, Anim::None, Anim::Fade)
            .with(Elem::Dot { color: palette::VOICE, size: 10.0, halo: true, pulse_ms: Some(2000) })
            .with(Elem::Clock { text: self.clock(now), since: self.started_at, style: TextStyle::VoiceTimer })
            .with(Elem::Levels { bars: Self::meter_bars(self.shown), color: METER })
            .with(Elem::Button {
                action: Action::invoke0("voice_stop"),
                label: Some(env.tr("Стоп", "Stop")),
                icon: None,
                style: ButtonStyle::Stop { width: 52.0 },
                tip: env.tr("Остановить и распознать (Ctrl+Alt+D)", "Stop and transcribe (Ctrl+Alt+D)"),
            })
    }

    fn busy_row(&self, env: &dyn Env) -> RowSpec {
        RowSpec::new("voice", VOICE_PRIORITY)
            .key("voice-busy")
            .width(CHIP_W)
            .anim(Anim::Fade, Anim::None, Anim::Fade)
            .with(Elem::Spinner { size: 14.0 })
            .with(Elem::text(env.tr("Распознаю…", "Transcribing…"), TextStyle::Label))
    }

    fn clear_live() -> Vec<LiveCmd> {
        vec![LiveCmd::clear("voice"), LiveCmd::unreact("voice")]
    }

    fn on_state(&mut self, env: &dyn Env, p: &Value, now: i64) -> Vec<LiveCmd> {
        if p.is_null() {
            return Vec::new();
        }
        self.state = match p.get("state") {
            Some(Value::String(s)) => s.clone(),
            other => display(other),
        };
        let mut out = vec![LiveCmd::Broadcast { name: "voice:state".into(), payload: p.clone() }];
        match self.state.as_str() {
            "recording" => {
                let t = num_or0(p.get("startedAt")).trunc() as i64;
                self.started_at = if t != 0 { t } else { now };
                self.lvl = 0.0;
                self.shown = 0.0;
                self.last_clock = self.clock(now);
                out.push(LiveCmd::set(self.rec_row(env, now)));
                out.push(LiveCmd::react("voice", Preset::Attention, Some(1600), None));
            }
            "transcribing" => {
                out.push(LiveCmd::set(self.busy_row(env)));
                out.push(LiveCmd::react("voice", Preset::Thinking, None, None));
            }
            "done" => {
                out.extend(Self::clear_live());
                out.push(LiveCmd::react("voice", Preset::Done, Some(2200), None));
                let body = text_or_empty(p.get("text"));
                let card = CardSpec::text("voice", 8000, env.tr("Текст в буфере", "Text on the clipboard"), Some(cut(&body, 80)))
                    .icon(IconName::Mic)
                    .tone(Tone::Green)
                    .actions(vec![
                        CardAction::new(env.tr("Копировать", "Copy"), ActionKind::White, Action::invoke("capture_action", json!({ "action": "copy_text", "value": body }))),
                        CardAction::close(env.tr("Закрыть", "Dismiss")),
                    ]);
                out.push(LiveCmd::Toast(card));
                out.push(LiveCmd::Broadcast { name: "voice:done".into(), payload: json!({ "text": body }) });
            }
            "error" => {
                out.extend(Self::clear_live());
                let code = text_or_empty(p.get("code"));
                if code == "busy" {
                    return out;
                }
                let e = err_info(&code);
                let mut actions = Vec::new();
                if e.settings {
                    actions.push(CardAction::new(env.tr("Настройки", "Settings"), ActionKind::White, Action::invoke("settings_open", json!({ "page": "voice" }))));
                }
                actions.push(CardAction::close(env.tr("Закрыть", "Dismiss")));
                let sub = match e.hint {
                    Some(h) if code != "no_key" && code != "no_endpoint" => env.tr(h.0, h.1),
                    _ => cut(&text_or_empty(p.get("message")), 110),
                };
                let card = CardSpec::text("voice", 7000, env.tr(e.title.0, e.title.1), Some(sub)).icon(IconName::Mic).tone(e.tone).actions(actions);
                out.push(LiveCmd::Toast(card));
            }
            // idle (cancelled) and anything else
            _ => out.extend(Self::clear_live()),
        }
        out
    }

    fn on_level(&mut self, env: &dyn Env, p: &Value, now: i64) -> Vec<LiveCmd> {
        if p.is_null() || truthy(p.get("test")) || self.state != "recording" {
            return Vec::new();
        }
        self.lvl = num_or0(p.get("level"));
        self.shown += (self.lvl - self.shown) * 0.6; // light smoothing on top of Rust's 25 Hz
        vec![LiveCmd::set(self.rec_row(env, now))]
    }
}

/// `Pill.dictate`: the pill header button. While dictation is switched off a click opens its settings page.
pub fn dictate_action(voice_state: &Value) -> Action {
    if !voice_state.is_null() && voice_state.get("enabled") == Some(&Value::Bool(false)) && voice_state.get("state").and_then(Value::as_str) == Some("idle") {
        Action::invoke("settings_open", json!({ "page": "voice" }))
    } else {
        Action::invoke0("voice_toggle")
    }
}

impl Provider for VoiceLive {
    fn name(&self) -> &'static str {
        "voice"
    }
    fn events(&self) -> &'static [&'static str] {
        &["voice:state", "voice:level", "voice:boot", "pill:hide", "pill:show"]
    }
    fn boot(&self) -> Vec<Query> {
        // a page reload while recording: pick the state up again
        vec![Query::new("voice_state", "voice:boot")]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        match name {
            "voice:state" => self.on_state(env, p, now),
            "voice:level" => self.on_level(env, p, now),
            "voice:boot" => match p.get("state").and_then(Value::as_str) {
                Some("recording") => self.on_state(env, &json!({ "state": "recording", "startedAt": p.get("startedAt").cloned().unwrap_or(Value::Null) }), now),
                Some("transcribing") => self.on_state(env, &json!({ "state": "transcribing" }), now),
                _ => Vec::new(),
            },
            "pill:hide" => {
                self.hidden = true;
                Vec::new()
            }
            "pill:show" => {
                self.hidden = false;
                self.tick_row(env, now)
            }
            _ => Vec::new(),
        }
    }
    fn on_tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if self.hidden {
            return Vec::new();
        }
        self.tick_row(env, now)
    }
    fn tick_ms(&self, _now: i64) -> Option<u32> {
        if self.state == "recording" && !self.hidden { Some(VOICE_TICK_MS) } else { None }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl VoiceLive {
    fn tick_row(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        if self.state != "recording" {
            return Vec::new();
        }
        let c = self.clock(now);
        if c == self.last_clock {
            return Vec::new();
        }
        self.last_clock = c;
        vec![LiveCmd::set(self.rec_row(env, now))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    const T0: i64 = 1_700_000_000_000;

    fn st(v: &mut VoiceLive, env: &MemEnv, p: Value, now: i64) -> Vec<LiveCmd> {
        v.on_event(env, "voice:state", &p, now)
    }
    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no row in {cmds:?}"))
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
    fn bars(r: &RowSpec) -> Vec<f32> {
        r.elements.iter().find_map(|e| if let Elem::Levels { bars, .. } = e { Some(bars.clone()) } else { None }).unwrap()
    }
    fn clock(r: &RowSpec) -> String {
        r.elements.iter().find_map(|e| if let Elem::Clock { text, .. } = e { Some(text.clone()) } else { None }).unwrap()
    }

    #[test]
    fn recording_chip() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        let cmds = st(&mut v, &env, json!({"state": "recording", "startedAt": T0 - 7000}), T0);
        assert!(matches!(&cmds[0], LiveCmd::Broadcast { name, .. } if name == "voice:state"));
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms, r.width), ("voice", 96, None, 214.0));
        assert_eq!(r.accent, Some(Accent::Rgb(Rgba::rgb(0xff, 0x5a, 0x4e))));
        assert_eq!(r.tip.as_deref(), Some("Диктовка: говорите"));
        assert_eq!(r.elements[0], Elem::Dot { color: palette::VOICE, size: 10.0, halo: true, pulse_ms: Some(2000) });
        assert_eq!(clock(r), "0:07");
        assert_eq!(bars(r), vec![0.14; 5]);
        match &r.elements[3] {
            Elem::Button { action, label, style, tip, .. } => {
                assert_eq!(*action, Action::invoke0("voice_stop"));
                assert_eq!(label.as_deref(), Some("Стоп"));
                assert_eq!(*style, ButtonStyle::Stop { width: 52.0 });
                assert_eq!(tip, "Остановить и распознать (Ctrl+Alt+D)");
            }
            o => panic!("{o:?}"),
        }
        assert!(cmds.contains(&LiveCmd::react("voice", Preset::Attention, Some(1600), None)));
        assert_eq!(v.tick_ms(T0), Some(250));
        assert_eq!(v.state(), "recording");
    }

    #[test]
    fn level_smoothing_and_bar_heights() {
        // shown moves 60 % of the way to the level on every event
        assert_eq!(VoiceLive::meter_bars(0.0), vec![0.14; 5]);
        assert_eq!(VoiceLive::meter_bars(1.0), vec![0.61, 0.86, 1.0, 0.86, 0.61]);
        assert_eq!(VoiceLive::meter_bars(5.0), vec![1.0; 5], "clamped to 1");
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        st(&mut v, &env, json!({"state": "recording", "startedAt": T0}), T0);
        let r1 = v.on_event(&env, "voice:level", &json!({"rms": 0.2, "level": 1.0, "test": false}), T0 + 40);
        // shown = 0.6 after one event
        assert_eq!(bars(row(&r1)), vec![0.39, 0.54, 0.66, 0.54, 0.39]);
        let r2 = v.on_event(&env, "voice:level", &json!({"level": 1.0}), T0 + 80);
        // shown = 0.6 + 0.4 * 0.6 = 0.84
        assert_eq!(bars(row(&r2)), vec![0.52, 0.73, 0.9, 0.73, 0.52]);
        assert_eq!(row(&r2).key, "voice-rec", "edited in place");
    }

    #[test]
    fn level_ignored_outside_recording_and_for_tests() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        assert!(v.on_event(&env, "voice:level", &json!({"level": 0.5}), T0).is_empty());
        st(&mut v, &env, json!({"state": "recording"}), T0);
        assert!(v.on_event(&env, "voice:level", &json!({"level": 0.5, "test": true}), T0).is_empty());
        assert!(v.on_event(&env, "voice:level", &Value::Null, T0).is_empty());
        // garbage level counts as silence
        let r = v.on_event(&env, "voice:level", &json!({"level": "loud"}), T0);
        assert_eq!(bars(row(&r)), vec![0.14; 5]);
    }

    #[test]
    fn timer_ticks_and_hide() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        st(&mut v, &env, json!({"state": "recording", "startedAt": T0}), T0);
        assert!(v.on_tick(&env, T0 + 500).is_empty());
        assert_eq!(clock(row(&v.on_tick(&env, T0 + 1000))), "0:01");
        v.on_event(&env, "pill:hide", &Value::Null, T0 + 1100);
        assert_eq!(v.tick_ms(T0), None);
        assert!(v.on_tick(&env, T0 + 9000).is_empty());
        assert_eq!(clock(row(&v.on_event(&env, "pill:show", &Value::Null, T0 + 65_000))), "1:05");
        // minutes are not rolled into hours
        assert_eq!(clock(row(&v.on_tick(&env, T0 + 3_725_000))), "62:05");
    }

    #[test]
    fn transcribing_row() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        st(&mut v, &env, json!({"state": "recording", "startedAt": T0}), T0);
        let cmds = st(&mut v, &env, json!({"state": "transcribing"}), T0 + 3000);
        let r = row(&cmds);
        assert_eq!((r.priority, r.accent, r.width), (96, None, 214.0));
        assert_eq!(r.elements[0], Elem::Spinner { size: 14.0 });
        assert_eq!(r.elements[1], Elem::text("Распознаю…", TextStyle::Label));
        assert!(cmds.contains(&LiveCmd::react("voice", Preset::Thinking, None, None)));
        assert_eq!(v.tick_ms(T0), None);
        let cmds = st(&mut v, &MemEnv::en(), json!({"state": "transcribing"}), T0 + 3000);
        assert_eq!(row(&cmds).elements[1], Elem::text("Transcribing…", TextStyle::Label));
    }

    #[test]
    fn done_card() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        st(&mut v, &env, json!({"state": "recording"}), T0);
        let cmds = st(&mut v, &env, json!({"state": "done", "text": "привет  мир\nкак дела", "chars": 19}), T0 + 5000);
        assert!(cmds.contains(&LiveCmd::clear("voice")) && cmds.contains(&LiveCmd::unreact("voice")));
        assert!(cmds.contains(&LiveCmd::react("voice", Preset::Done, Some(2200), None)));
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone, t.icon.clone()), ("voice", 8000, Tone::Green, Some(CardIcon::Name(IconName::Mic))));
        assert_eq!(ts(t), ("Текст в буфере".to_string(), Some("привет мир как дела".to_string())));
        assert_eq!(t.actions[0].label, "Копировать");
        assert_eq!(t.actions[0].action, Action::invoke("capture_action", json!({"action": "copy_text", "value": "привет  мир\nкак дела"})), "the full text is copied, not the preview");
        assert_eq!(t.actions[1].label, "Закрыть");
        assert!(cmds.contains(&LiveCmd::Broadcast { name: "voice:done".into(), payload: json!({"text": "привет  мир\nкак дела"}) }));
        assert_eq!(v.state(), "done");
        let long = "w".repeat(200);
        let cmds = st(&mut v, &MemEnv::en(), json!({"state": "done", "text": long}), T0);
        assert_eq!(ts(toast(&cmds)), ("Text on the clipboard".to_string(), Some(format!("{}…", "w".repeat(79)))));
        assert_eq!(toast(&cmds).actions[1].label, "Dismiss");
    }

    #[test]
    fn error_table() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        let mut e = |env: &MemEnv, code: &str, msg: &str| {
            let cmds = st(&mut v, env, json!({"state": "error", "code": code, "message": msg}), T0);
            toast(&cmds).clone()
        };
        let t = e(&env, "off", "x");
        assert_eq!((t.ttl_ms, t.tone), (7000, Tone::Amber));
        assert_eq!(ts(&t), ("Голос в текст выключен".to_string(), Some("Включи его в настройках".to_string())));
        assert_eq!(t.actions.len(), 2);
        assert_eq!(t.actions[0].action, Action::invoke("settings_open", json!({"page": "voice"})));
        let t = e(&env, "no_key", "Add a key in Settings");
        assert_eq!(ts(&t), ("Нет ключа API".to_string(), Some("Add a key in Settings".to_string())), "the backend message, not a hint");
        let t = e(&env, "no_endpoint", "set the URL");
        assert_eq!(ts(&t), ("Не указан адрес сервиса".to_string(), Some("set the URL".to_string())));
        let t = e(&env, "mic", "device busy");
        assert_eq!((t.tone, t.actions.len()), (Tone::Red, 1));
        assert_eq!(ts(&t), ("Нет доступа к микрофону".to_string(), Some("device busy".to_string())));
        let t = e(&env, "too_short", "m");
        assert_eq!(ts(&t), ("Слишком короткая запись".to_string(), Some("Скажи что-нибудь подольше".to_string())));
        assert_eq!(t.actions.len(), 1);
        let t = e(&env, "silent", "m");
        assert_eq!(ts(&t).1, Some("Проверь микрофон по умолчанию и его громкость".to_string()));
        let t = e(&env, "empty", "nothing");
        assert_eq!((ts(&t).0.as_str(), t.tone), ("Речь не распознана", Tone::Amber));
        let t = e(&env, "net", "dns");
        assert_eq!((t.tone, t.actions.len()), (Tone::Red, 2));
        assert_eq!(ts(&t).0, "Нет связи с сервисом");
        let t = e(&env, "http", "502");
        assert_eq!(ts(&t), ("Сервис вернул ошибку".to_string(), Some("502".to_string())));
        assert_eq!(t.actions.len(), 2);
        let t = e(&env, "weird", "boom");
        assert_eq!((t.tone, ts(&t)), (Tone::Red, ("Диктовка не удалась".to_string(), Some("boom".to_string()))));
        assert_eq!(t.actions.len(), 1);
        let en = MemEnv::en();
        let t = e(&en, "off", "");
        assert_eq!(ts(&t), ("Voice to text is off".to_string(), Some("Turn it on in Settings".to_string())));
        assert_eq!((t.actions[0].label.as_str(), t.actions[1].label.as_str()), ("Settings", "Dismiss"));
        assert_eq!(ts(&e(&en, "silent", "")).1, Some("Check the default microphone and its volume".to_string()));
    }

    #[test]
    fn error_message_is_cut_and_busy_is_silent() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        let cmds = st(&mut v, &env, json!({"state": "error", "code": "net", "message": "x ".repeat(100)}), T0);
        assert_eq!(ts(toast(&cmds)).1.unwrap().chars().count(), 110);
        let cmds = st(&mut v, &env, json!({"state": "error", "code": "busy", "message": "already"}), T0);
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::Toast(_))));
        assert!(cmds.contains(&LiveCmd::clear("voice")), "busy still clears the chip");
        let cmds = st(&mut v, &env, json!({"state": "error"}), T0);
        assert_eq!(ts(toast(&cmds)), ("Диктовка не удалась".to_string(), Some(String::new())));
    }

    #[test]
    fn idle_clears_and_broadcasts() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        st(&mut v, &env, json!({"state": "recording"}), T0);
        let cmds = st(&mut v, &env, json!({"state": "idle"}), T0 + 10);
        assert_eq!(cmds.len(), 3);
        assert!(matches!(&cmds[0], LiveCmd::Broadcast { payload, .. } if payload == &json!({"state": "idle"})));
        assert_eq!(&cmds[1..], &[LiveCmd::clear("voice"), LiveCmd::unreact("voice")]);
        assert_eq!(v.tick_ms(T0), None);
        assert!(st(&mut v, &env, Value::Null, T0).is_empty());
    }

    #[test]
    fn reload_restores_state() {
        let mut v = VoiceLive::default();
        let env = MemEnv::default();
        assert_eq!(v.boot(), vec![Query::new("voice_state", "voice:boot")]);
        assert!(v.on_event(&env, "voice:boot", &json!({"state": "idle", "enabled": true}), T0).is_empty());
        let cmds = v.on_event(&env, "voice:boot", &json!({"state": "recording", "startedAt": T0 - 30_000}), T0);
        assert_eq!(clock(row(&cmds)), "0:30");
        let cmds = v.on_event(&env, "voice:boot", &json!({"state": "transcribing"}), T0);
        assert_eq!(row(&cmds).key, "voice-busy");
        assert!(v.on_event(&env, "voice:boot", &Value::Null, T0).is_empty());
    }

    #[test]
    fn dictate_button_routing() {
        assert_eq!(dictate_action(&json!({"enabled": false, "state": "idle"})), Action::invoke("settings_open", json!({"page": "voice"})));
        assert_eq!(dictate_action(&json!({"enabled": false, "state": "recording"})), Action::invoke0("voice_toggle"));
        assert_eq!(dictate_action(&json!({"enabled": true, "state": "idle"})), Action::invoke0("voice_toggle"));
        assert_eq!(dictate_action(&Value::Null), Action::invoke0("voice_toggle"));
    }
}
