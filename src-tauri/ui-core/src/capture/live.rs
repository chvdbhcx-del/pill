//! Pill-side half of the capture lane (port of src/capture-live.js): the recording capsule and its timer, the Home "Record" tile
//! that turns into "Stop 0:NN", the result cards for `capture:done` events, and the tile start plan (collapse first, then ask Rust).
//!
//! Events in: `rec:state {recording, startedAt, mode, fps}` and `capture:done {kind, ...}` (shapes in docs/native-ui-contract.md).
//! Calls out: [`BackendCall`]s (`capture_action`, `capture_stop`, `capture_start`, `shelf_add`). Strings are exact copies of the
//! web ones (ru + en); TTLs, tones and icons too.
use super::io::BackendCall;
use crate::i18n::Tr;
use serde::{Deserialize, Serialize};

// ------------------------------------------------------------------------------------------------- text helpers

/// `m:ss` (no leading zero on minutes), `h:mm:ss` after an hour (`clock` in capture-live.js).
pub fn clock(ms: i64) -> String {
    let s = (ms.max(0)) / 1000;
    let (h, m, r) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{r:02}")
    } else {
        format!("{m}:{r:02}")
    }
}

/// Recording length from whole seconds (`dur`).
pub fn dur(sec: i64) -> String {
    clock(sec * 1000)
}

/// `cut(s, n)`: collapse whitespace, trim, and shorten to `n` chars with an ellipsis.
pub fn cut(s: &str, n: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > n {
        let mut o: String = t.chars().take(n.saturating_sub(1)).collect();
        o.push('\u{2026}');
        o
    } else {
        t
    }
}

/// Last path segment, either slash (`base`).
pub fn base(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

// ------------------------------------------------------------------------------------------------- capture:done cards

/// Payload of `capture:done`. Unknown fields are ignored, missing ones default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CaptureDone {
    pub kind: String,
    pub path: Option<String>,
    pub copied: bool,
    pub w: u32,
    pub h: u32,
    pub found: bool,
    pub text: String,
    pub chars: usize,
    pub is_url: bool,
    pub hex: String,
    pub rgb: Vec<u8>,
    pub history: Vec<String>,
    pub seconds: i64,
    pub message: String,
}

impl CaptureDone {
    pub fn from_value(v: &serde_json::Value) -> Option<CaptureDone> {
        serde_json::from_value::<CaptureDone>(v.clone()).ok().filter(|d| !d.kind.is_empty())
    }
}

/// Colour of the card's accent (`tone` of `P.toast`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tone {
    Green,
    Amber,
    Accent,
    Red,
    /// A custom-HTML card (the colour card draws itself).
    Plain,
}

/// A button on a card. `primary` is the white button (`kind: 'white'`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardAction {
    pub label: String,
    pub primary: bool,
    pub call: BackendCall,
}

/// Body of the colour card: big swatch, hex, rgb line and the recent-colour swatches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorCard {
    pub hex: String,
    pub rgb_line: String,
    pub history_title: String,
    /// Up to 9 recent colours other than `hex`; clicking one copies it ([`history_click`]).
    pub history: Vec<String>,
    /// A swatch shows its "did" feedback for this long after a click (`.did`, 400 ms).
    pub swatch_flash_ms: u32,
}

/// A toast the pill shows for a `capture:done` event.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CardVm {
    /// Toast id: a new capture result replaces the previous one.
    pub id: &'static str,
    pub icon: Option<&'static str>,
    pub tone: Tone,
    pub ttl_ms: u32,
    pub title: String,
    pub sub: String,
    pub actions: Vec<CardAction>,
    pub color: Option<ColorCard>,
    /// `P.react('capture', 'file', { ttlMs: 3000 })`: the pill's pet reacts to a saved file.
    pub react_file: bool,
}

fn act(label: String, primary: bool, action: &str, value: &str) -> CardAction {
    CardAction { label, primary, call: BackendCall::action(action, value) }
}

/// Build the card for a result. `ext_actions` adds the "To shelf" button on saved files (native extension).
pub fn card(p: &CaptureDone, tr: &dyn Tr, ext_actions: bool) -> Option<CardVm> {
    let mut c = CardVm { id: "cap", icon: None, tone: Tone::Plain, ttl_ms: 0, title: String::new(), sub: String::new(), actions: Vec::new(), color: None, react_file: false };
    match p.kind.as_str() {
        "screenshot" => {
            let path = p.path.as_deref().filter(|s| !s.is_empty());
            c.icon = Some("monitor");
            c.tone = Tone::Green;
            c.ttl_ms = 7000;
            if let Some(path) = path {
                c.react_file = true;
                c.actions.push(act(tr.t("Открыть", "Open"), true, "open", path));
                c.actions.push(act(tr.t("В папке", "Show in folder"), false, "reveal", path));
                c.actions.push(act(tr.t("Копировать", "Copy"), false, "copy_image", path));
                if ext_actions {
                    c.actions.push(CardAction { label: tr.t("На полку", "Add to shelf"), primary: false, call: BackendCall::shelf_add(&[path.to_string()]) });
                }
            }
            c.title = if path.is_some() { tr.t("Скриншот сохранён", "Screenshot saved") } else { tr.t("Скриншот скопирован", "Screenshot copied") };
            c.sub = match path {
                Some(path) => base(path),
                None => format!("{}\u{d7}{}", p.w, p.h),
            };
        }
        "text" => {
            c.icon = Some("scantext");
            if p.found {
                c.tone = Tone::Green;
                c.ttl_ms = 6000;
                c.title = tr.t("Текст скопирован", "Text copied");
                c.sub = cut(&p.text, 72) + &if p.chars > 72 { format!(" \u{b7} {}", p.chars) } else { String::new() };
            } else {
                c.tone = Tone::Amber;
                c.ttl_ms = 4500;
                c.title = tr.t("Текст не найден", "No text found");
                c.sub = tr.t("Попробуй выделить область покрупнее", "Try a larger area");
            }
        }
        "qr" => {
            c.icon = Some("qr");
            if !p.found {
                c.tone = Tone::Amber;
                c.ttl_ms = 4500;
                c.title = tr.t("QR-код не найден", "No QR code found");
                c.sub = tr.t("Выдели код целиком, с белой рамкой", "Select the whole code, with its white border");
            } else {
                c.tone = Tone::Accent;
                c.ttl_ms = 12000;
                c.title = tr.t("QR-код", "QR code");
                c.sub = cut(&p.text, 90);
                if p.is_url {
                    c.actions.push(act(tr.t("Открыть ссылку", "Open link"), true, "open_url", &p.text));
                }
                c.actions.push(act(tr.t("Копировать", "Copy"), false, "copy_text", &p.text));
            }
        }
        "color" => {
            c.ttl_ms = 9000;
            let rgb = p.rgb.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ");
            c.color = Some(ColorCard {
                hex: p.hex.clone(),
                rgb_line: format!("rgb({rgb}) \u{b7} {}", tr.t("скопировано", "copied")),
                history_title: tr.t("Недавние цвета: клик копирует", "Recent colours: click to copy"),
                history: p.history.iter().filter(|h| **h != p.hex).take(9).cloned().collect(),
                swatch_flash_ms: 400,
            });
        }
        "record" => {
            let path = p.path.as_deref().unwrap_or("");
            c.icon = Some("video");
            c.tone = Tone::Green;
            c.ttl_ms = 9000;
            c.react_file = !path.is_empty();
            c.title = tr.t("Запись сохранена", "Recording saved");
            c.sub = format!("{} \u{b7} {}", base(path), dur(p.seconds));
            c.actions.push(act(tr.t("Открыть", "Open"), true, "open", path));
            c.actions.push(act(tr.t("В папке", "Show in folder"), false, "reveal", path));
        }
        "error" => {
            c.icon = Some("x");
            c.tone = Tone::Red;
            c.ttl_ms = 10000;
            c.title = tr.t("Не получилось", "Capture failed");
            c.sub = cut(&p.message, 180);
        }
        _ => return None,
    }
    Some(c)
}

/// A click on a colour-history swatch of the colour card.
pub fn history_click(hex: &str) -> BackendCall {
    BackendCall::action("copy_text", hex)
}

// ------------------------------------------------------------------------------------------------- recording

/// Poll interval of the timer text (the page ticks every 250 ms and skips ticks while the island is hidden).
pub const TIMER_POLL_MS: u64 = 250;
/// Delay between collapsing the pill and asking Rust to start (so the pill is not in the frozen frame).
pub const START_DELAY_MS: u64 = 450;
/// Capsule geometry / look from the video notes.
pub const CAPSULE_WIDTH: f64 = 258.0;
pub const DOT_SIZE: f64 = 10.0;
/// The red dot pulses opacity 1 <-> .5 over this period (static under Reduce motion).
pub const DOT_PULSE_MS: u32 = 2000;
pub const DOT_PULSE_LOW: f64 = 0.5;
pub const STOP_SIZE: (f64, f64) = (56.0, 24.0);
/// Stop button fill rgba(255,67,63,.20), .30 on hover.
pub const STOP_ALPHA: f64 = 0.20;
pub const STOP_ALPHA_HOVER: f64 = 0.30;
/// Live-row priority of the capsule (`P.setLive('rec', { priority: 100 })`).
pub const LIVE_PRIORITY: i32 = 100;

/// Payload of `rec:state`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RecEvent {
    pub recording: bool,
    pub started_at: i64,
    pub mode: String,
    pub fps: f64,
}

/// What the capsule shows.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CapsuleVm {
    pub timer: String,
    /// `MP4`, or `GIF` when `capture.format` is gif.
    pub format: &'static str,
    pub title: String,
    pub stop_label: String,
    pub stop_tip: String,
    pub width: f64,
    pub dot_size: f64,
    /// 0 under Reduce motion (static dot).
    pub dot_pulse_ms: u32,
    pub dot_pulse_low: f64,
    pub stop_size: (f64, f64),
    pub stop_alpha: f64,
    pub stop_alpha_hover: f64,
    pub priority: i32,
    /// The Stop button's call.
    pub stop: BackendCall,
}

/// Home "Record screen" tile while recording: a red square, "Stop" and the running time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileVm {
    pub recording: bool,
    pub title: String,
    pub timer: String,
}

/// Recording state of the pill side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecState {
    started_at: Option<i64>,
    /// `capture.format == gif`.
    pub gif: bool,
    /// The island is hidden: no ticks (nobody sees the text).
    pub hidden: bool,
    last_text: String,
}

impl RecState {
    pub fn new() -> RecState {
        RecState::default()
    }
    pub fn recording(&self) -> bool {
        self.started_at.is_some()
    }
    /// Apply a `rec:state` event (`onRec`); returns true when the capsule appeared or went away.
    pub fn on_event(&mut self, e: &RecEvent, now_ms: i64) -> bool {
        let was = self.recording();
        if e.recording {
            self.started_at = Some(if e.started_at > 0 { e.started_at } else { now_ms });
        } else {
            self.started_at = None;
            self.last_text.clear();
        }
        was != self.recording()
    }
    /// Restore after a reload from `capture_settings {recording, recStartedAt}`.
    pub fn restore(&mut self, recording: bool, started_at: Option<i64>, now_ms: i64) {
        if recording {
            if let Some(t) = started_at.filter(|t| *t > 0) {
                self.started_at = Some(t);
            } else {
                self.started_at = Some(now_ms);
            }
        }
    }
    /// The timer text now (`""` when not recording).
    pub fn timer(&self, now_ms: i64) -> String {
        self.started_at.map_or(String::new(), |s| clock(now_ms - s))
    }
    /// `tick()`: `Some(new text)` only when the text changed (once a second), `None` otherwise or while hidden / idle.
    pub fn tick(&mut self, now_ms: i64) -> Option<String> {
        if self.hidden || !self.recording() {
            return None;
        }
        let t = self.timer(now_ms);
        if t != self.last_text {
            self.last_text = t.clone();
            Some(t)
        } else {
            None
        }
    }
    pub fn capsule(&self, now_ms: i64, tr: &dyn Tr, reduce_motion: bool) -> Option<CapsuleVm> {
        self.started_at?;
        Some(CapsuleVm {
            timer: self.timer(now_ms),
            format: if self.gif { "GIF" } else { "MP4" },
            title: tr.t("Идёт запись экрана", "Recording the screen"),
            stop_label: tr.t("Стоп", "Stop"),
            stop_tip: tr.t("Остановить запись (Ctrl+Alt+R)", "Stop recording (Ctrl+Alt+R)"),
            width: CAPSULE_WIDTH,
            dot_size: DOT_SIZE,
            dot_pulse_ms: if reduce_motion { 0 } else { DOT_PULSE_MS },
            dot_pulse_low: DOT_PULSE_LOW,
            stop_size: STOP_SIZE,
            stop_alpha: STOP_ALPHA,
            stop_alpha_hover: STOP_ALPHA_HOVER,
            priority: LIVE_PRIORITY,
            stop: BackendCall::stop(),
        })
    }
    pub fn tile(&self, now_ms: i64, tr: &dyn Tr) -> TileVm {
        TileVm { recording: self.recording(), title: tr.t("Стоп", "Stop"), timer: self.timer(now_ms) }
    }
}

/// How a Home tile starts a capture (`PillCapture.start`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StartPlan {
    /// Collapse the pill first so it is not part of the frozen frame.
    pub collapse_first: bool,
    /// Wait this long after collapsing before the call.
    pub delay_ms: u64,
    pub call: BackendCall,
}

/// `record_screen` while recording toggles at once; everything else collapses the pill and waits 450 ms.
pub fn start_plan(mode: &str, recording: bool) -> StartPlan {
    let immediate = mode == "record_screen" && recording;
    StartPlan { collapse_first: !immediate, delay_ms: if immediate { 0 } else { START_DELAY_MS }, call: BackendCall::start(mode) }
}
