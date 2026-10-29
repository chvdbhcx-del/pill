//! Home tab: player card (sessions, seek bar interpolation, cover / accent / gradient), volume row (eased knob, 1 % system
//! writes, mute, wheel, keys), output-device menu, the 2x4 tile grid, the launcher strip ([`super::launcher`]) and the
//! collapsed-island music widget. Port of `src/tab-home.js`.
//!
//! Backend: `media_state {known:null}`, `media_sessions`, `media_select {id}`, `media_control {action}`, `media_seek {ms}`,
//! `media_volume_get|set {v}`, `media_mute {muted}`, `media_devices`, `media_set_default_device {id}`,
//! `media_pause_updates|resume_updates`, `settings_get`. Events: `media:changed`, `live:volume`, `settings:changed`,
//! `pill:show|hide`, plus the launcher's (`launcher:changed`, `tauri://drag-*`).
//! Settings: `musicPill` (title | mini), `module.media`, `module.launcher`, `live.music`.
//!
//! Time: the model keeps its own monotonic clock advanced by [`Home::tick`]; the seek position is `base + (now - at)` of that clock,
//! so the renderer only has to call `tick(dt)` once per frame while [`Home::animating`] is true.

use super::anim::Anim;
use super::cover::{CoverCache, CoverColors};
use super::launcher::{self, Launcher};
use super::util::fmt_ms;
use super::{bool_of, i64_of, str_of, BackendCall, Cfg, Fx, FxQueue, LiveBody, LiveSpec, MusicMode, Tr};
use serde_json::{json, Value};

/// Delay between a media event and the `media_sessions` refresh while the tab is open.
pub const SESSIONS_DELAY_MS: f64 = 400.0;
/// A reading of the system volume that arrives within this window after our own change is ignored.
pub const VOL_OWN_WINDOW_MS: f64 = 600.0;

/// `media.rs::MediaState`
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    pub has_session: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub session_id: String,
    pub app: String,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub thumb_id: u64,
    pub thumbnail: Option<String>,
    pub accent: Option<String>,
}

impl Media {
    pub fn from_json(v: &Value) -> Media {
        Media {
            has_session: bool_of(v, "has_session"),
            playing: bool_of(v, "playing"),
            title: str_of(v, "title").to_string(),
            artist: str_of(v, "artist").to_string(),
            album: str_of(v, "album").to_string(),
            session_id: str_of(v, "session_id").to_string(),
            app: str_of(v, "app").to_string(),
            position_ms: i64_of(v, "position_ms"),
            duration_ms: i64_of(v, "duration_ms"),
            thumb_id: v.get("thumb_id").and_then(Value::as_u64).unwrap_or(0),
            thumbnail: v.get("thumbnail").and_then(Value::as_str).map(str::to_string),
            accent: v.get("accent").and_then(Value::as_str).map(str::to_string),
        }
    }
}

/// `media.rs::SessionInfo`
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub selected: bool,
}

/// `media.rs::Device`
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctl {
    Prev,
    PlayPause,
    Next,
}

impl Ctl {
    fn action(self) -> &'static str {
        match self {
            Ctl::Prev => "prev",
            Ctl::PlayPause => "play_pause",
            Ctl::Next => "next",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolKey {
    Left,
    Down,
    Right,
    Up,
    PageDown,
    PageUp,
    Home,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// prev / play-pause / next buttons of the card
    Ctl(Ctl),
    /// A button of the collapsed-island "mini" widget (same commands, handled before the pill sees the click)
    LiveButton(Ctl),
    /// A tile of the grid by id (`area`, `screen`, `ocr`, `qr`, `picker`, `plain`, `record`, `command`)
    Tile(String),
    /// The player's source label: cycles through the players when there are several
    SourceClick,
    /// Seek slider moved by the user (`input`): value in ms
    SeekDrag(f64),
    /// Seek slider released (`change`)
    SeekCommit(f64),
    /// Volume pointer down / move / up; the value is 0..100 under the pointer (see [`vol_at`])
    VolPointerDown(f32),
    VolPointerMove(f32),
    VolPointerUp,
    VolKey(VolKey),
    /// Wheel over the volume row. `lines` = delta in lines (else pixels, 100 per notch); wheel up (negative) = louder
    VolWheel { delta_y: f32, lines: bool },
    MuteClick,
    DevToggle,
    DevPick(String),
    /// Outside click on the device menu
    DevClose,
    Launcher(launcher::Action),
}

/// `[id, icon, ru, en, works]`
pub const TILES: [(&str, &str, &str, &str); 8] = [
    ("area", "area", "Область", "Area"),
    ("screen", "monitor", "Весь экран", "Full screen"),
    ("ocr", "scantext", "Текст с экрана", "Screen text"),
    ("qr", "qr", "QR-код", "QR code"),
    ("picker", "pipette", "Пипетка", "Color picker"),
    ("plain", "eraser", "Чистый текст", "Plain text"),
    ("record", "video", "Запись экрана", "Record screen"),
    ("command", "search", "Команда", "Command"),
];

/// What a tile does when clicked (`WORKS`); `plain` is "soon".
fn tile_fx(id: &str) -> Option<Fx> {
    Some(match id {
        "command" => Fx::OpenCommandBar,
        "area" => Fx::StartCapture { mode: "area" },
        "screen" => Fx::StartCapture { mode: "screen" },
        "ocr" => Fx::StartCapture { mode: "text" },
        "qr" => Fx::StartCapture { mode: "qr" },
        "picker" => Fx::StartCapture { mode: "color" },
        "record" => Fx::StartCapture { mode: "record_screen" },
        _ => return None,
    })
}

/// Pointer x to 0..100 over the slider's thumb travel (`volAt`): the 12 px thumb leaves 6 px inset at each end, in scaled px.
pub fn vol_at(client_x: f32, left: f32, width: f32, offset_width: f32) -> f32 {
    let ow = if offset_width != 0.0 { offset_width } else if width != 0.0 { width } else { 1.0 };
    let h = 6.0 * width / ow;
    (((client_x - left - h) / (width - 2.0 * h).max(1.0)) * 100.0).clamp(0.0, 100.0)
}

fn vol_round(x: f64) -> i64 {
    x.clamp(0.0, 100.0).round() as i64
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seek {
    pub enabled: bool,
    pub max_ms: i64,
    pub value_ms: f64,
    /// Fill 0..100 for the painted track.
    pub fill_pct: f64,
    pub elapsed: String,
    /// `-m:ss` remaining, or `--:--` when the duration is unknown.
    pub remaining: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// `module.media` is on.
    pub visible: bool,
    /// No session: only the "nothing is playing" empty state shows.
    pub none: bool,
    pub empty_text: String,
    pub accent: String,
    pub cover_url: Option<String>,
    pub cover_gradient: Option<CoverColors>,
    pub source: String,
    pub source_multi: bool,
    pub source_title: String,
    pub title: String,
    pub artist_line: String,
    pub playing: bool,
    /// `play` or `pause` (what the main button shows)
    pub play_glyph: &'static str,
    pub seek: Seek,
    pub seek_label: String,
    pub prev_title: String,
    pub play_title: String,
    pub next_title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VolumeView {
    /// Eased knob / fill position 0..100.
    pub shown: f64,
    /// The number next to the slider.
    pub number: i64,
    pub muted: bool,
    /// `mute` when muted or zero, else `volume`
    pub glyph: &'static str,
    pub mute_title: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceItem {
    pub id: String,
    pub name: String,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DevicesView {
    pub open: bool,
    pub button_text: String,
    pub button_title: String,
    pub items: Vec<DeviceItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TileView {
    pub id: &'static str,
    pub icon: &'static str,
    pub label: String,
    /// Not wired yet: dimmed, tooltip "Soon", no action.
    pub soon: bool,
    pub tooltip: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub player: Player,
    pub tiles: Vec<TileView>,
    pub volume: VolumeView,
    pub devices: DevicesView,
    pub launcher: launcher::View,
}

pub struct Home {
    cfg: Cfg,
    mono: f64,
    st: Option<Media>,
    art_id: u64,
    art_url: Option<String>,
    base_pos: i64,
    base_at: f64,
    vol: (i64, bool),
    shown: bool,
    hidden: bool,
    dragging: Option<f64>,
    mode: MusicMode,
    live_key: String,
    sessions: Vec<Session>,
    sess_at: Option<f64>,
    cover_fetch: bool,
    was_media: bool,
    // volume
    vol_goal: f64,
    vol_shown: f64,
    vol_animating: bool,
    vol_init: bool,
    vol_hold: bool,
    vol_user_at: f64,
    vol_sent: i64,
    vol_want: i64,
    vol_busy: bool,
    wheel_acc: f32,
    // devices
    devs: Vec<Device>,
    dev_open: bool,
    dev_opening: bool,
    after_pick: bool,
    // cover gradient
    covers: CoverCache,
    cov_pending: String,
    cov_request: Option<(String, String)>,
    cov_now: Option<CoverColors>,
    // render edge state (what the "DOM" last showed), for animation cues
    last_title: String,
    last_artist: String,
    last_pl: Option<bool>,
    art_key: Option<String>,
    launcher: Launcher,
    fx: FxQueue,
}

impl Default for Home {
    fn default() -> Self {
        Home::new()
    }
}

impl Home {
    pub fn new() -> Home {
        Home {
            cfg: Cfg::new(),
            mono: 0.0,
            st: None,
            art_id: 0,
            art_url: None,
            base_pos: 0,
            base_at: 0.0,
            vol: (0, false),
            shown: false,
            hidden: false,
            dragging: None,
            mode: MusicMode::Title,
            live_key: String::new(),
            sessions: vec![],
            sess_at: None,
            cover_fetch: false,
            was_media: true,
            vol_goal: 0.0,
            vol_shown: 0.0,
            vol_animating: false,
            vol_init: false,
            vol_hold: false,
            vol_user_at: -1e9,
            vol_sent: -1,
            vol_want: -1,
            vol_busy: false,
            wheel_acc: 0.0,
            devs: vec![],
            dev_open: false,
            dev_opening: false,
            after_pick: false,
            covers: CoverCache::default(),
            cov_pending: String::new(),
            cov_request: None,
            cov_now: None,
            last_title: String::new(),
            last_artist: String::new(),
            last_pl: None,
            art_key: None,
            launcher: Launcher::new(),
            fx: FxQueue::default(),
        }
    }

    /// Calls to run once at startup (`media_state` for the first state, `settings_get` for the mode).
    pub fn init_calls(&mut self) -> Vec<BackendCall> {
        vec![BackendCall::new("media_state", json!({ "known": null })), BackendCall::bare("settings_get")]
    }

    pub fn launcher_mut(&mut self) -> &mut Launcher {
        &mut self.launcher
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.cfg.os_reduce = on;
        self.launcher.configure(self.cfg.not_false("module.launcher"), self.cfg.reduce_motion());
    }
    pub fn take_fx(&mut self) -> Vec<Fx> {
        let mut v = self.fx.take();
        v.extend(self.launcher.take_fx());
        v
    }
    /// True while the renderer must keep calling `tick` every frame: the seek bar of a playing track or a volume glide.
    pub fn animating(&self) -> bool {
        (self.shown && !self.hidden && self.st.as_ref().map_or(false, |s| s.has_session && s.playing)) || self.vol_animating
    }
    /// Escape closes the device menu first; the island must not collapse while it is open.
    pub fn wants_escape(&self) -> bool {
        self.dev_open
    }
    pub fn media(&self) -> Option<&Media> {
        self.st.as_ref()
    }
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    fn reduce(&self) -> bool {
        self.cfg.reduce_motion()
    }
    fn media_on(&self) -> bool {
        self.cfg.not_false("module.media")
    }
    fn accent(&self) -> String {
        self.st.as_ref().and_then(|s| s.accent.clone()).unwrap_or_default()
    }
    fn app_name(&self) -> String {
        let a = self.st.as_ref().map(|s| s.app.as_str()).unwrap_or("");
        let l = a.to_ascii_lowercase();
        let a = if l.ends_with(".exe") { &a[..a.len() - 4] } else { a };
        a.trim().to_string()
    }

    // ---------------------------------------------------------------- settings
    fn load_mode(&mut self, payload: Option<&Value>) -> Vec<BackendCall> {
        match payload {
            Some(p) if p.is_object() => self.apply_cfg(p),
            _ => return vec![BackendCall::bare("settings_get")],
        }
    }

    fn apply_cfg(&mut self, p: &Value) -> Vec<BackendCall> {
        self.cfg.apply(p);
        self.mode = if self.cfg.str("musicPill") == "mini" { MusicMode::Mini } else { MusicMode::Title };
        self.launcher.configure(self.cfg.not_false("module.launcher"), self.reduce());
        let media = self.media_on();
        let mut calls = vec![];
        if media != self.was_media {
            self.was_media = media;
            if media {
                calls.push(BackendCall::new("media_state", json!({ "known": null })));
            } else {
                self.st = None;
                self.render();
            }
        }
        self.update_live();
        calls
    }

    // ---------------------------------------------------------------- media intake
    fn apply(&mut self, s: &Value) {
        if !s.is_object() || !self.media_on() {
            return;
        }
        let m = Media::from_json(s);
        if let Some(url) = &m.thumbnail {
            self.art_id = m.thumb_id;
            self.art_url = Some(url.clone());
        } else if m.has_session && m.thumb_id != self.art_id {
            // the cover changed but the event omitted it (we had missed it): fetch once
            self.art_id = m.thumb_id;
            self.art_url = None;
            self.cover_fetch = true;
        }
        self.base_pos = m.position_ms;
        self.base_at = self.mono;
        self.st = Some(m);
        self.render();
        self.update_live();
        if self.shown {
            self.sess_at = Some(self.mono + SESSIONS_DELAY_MS);
        }
    }

    fn pos(&self) -> f64 {
        let Some(s) = &self.st else { return 0.0 };
        let p = if s.playing { self.base_pos as f64 + (self.mono - self.base_at) } else { self.base_pos as f64 };
        if s.duration_ms > 0 {
            p.min(s.duration_ms as f64)
        } else {
            p
        }
    }

    /// Current playback position in ms (interpolated).
    pub fn position_ms(&self) -> f64 {
        self.pos()
    }

    // ---------------------------------------------------------------- render edge: cues and cover
    fn render(&mut self) {
        let has = self.st.as_ref().map_or(false, |s| s.has_session);
        self.paint_cover();
        if !has {
            return;
        }
        let reduce = self.reduce();
        // cover: the new image fades in over the old one; nothing the first time
        let key = self.art_url.clone().unwrap_or_default();
        if self.art_key.as_deref() != Some(key.as_str()) {
            let first = self.art_key.is_none();
            self.art_key = Some(key);
            if !first {
                self.fx.anim(reduce, "art", Anim::cover_fade());
            }
        }
        let s = self.st.clone().unwrap();
        let title = if !s.title.is_empty() { s.title.clone() } else if !self.app_name().is_empty() { self.app_name() } else { "\u{0}unknown".to_string() };
        let artist = [s.artist.as_str(), s.album.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" - ");
        if title != self.last_title {
            if !self.last_title.is_empty() {
                self.fx.anim(reduce, "title", Anim::text_swap());
            }
            self.last_title = title;
        }
        if artist != self.last_artist {
            if !self.last_artist.is_empty() {
                self.fx.anim(reduce, "artist", Anim::text_swap());
            }
            self.last_artist = artist;
        }
        if self.last_pl != Some(s.playing) {
            if self.last_pl.is_some() {
                self.fx.anim(reduce, "play", Anim::glyph_swap());
            }
            self.last_pl = Some(s.playing);
        }
    }

    fn cov_key(&self) -> String {
        format!("{}:{}", self.art_id, self.art_url.as_ref().map_or(0, |u| u.chars().count()))
    }

    fn paint_cover(&mut self) {
        if self.hidden {
            return; // hidden window: no work; pill:show repaints
        }
        let has = self.st.as_ref().map_or(false, |s| s.has_session);
        let Some(url) = self.art_url.clone().filter(|_| has) else {
            self.cov_now = None;
            return;
        };
        let key = self.cov_key();
        if let Some(c) = self.covers.get(&key) {
            self.cov_now = c.clone();
            return;
        }
        if self.cov_pending == key {
            return;
        }
        self.cov_pending = key.clone();
        self.cov_request = Some((key, url));
    }

    /// The renderer should decode this cover (data URL) and answer with [`Home::set_cover_colors`]. Returned once per cover.
    pub fn take_cover_request(&mut self) -> Option<(String, String)> {
        self.cov_request.take()
    }

    /// Result of `cover::pick_colors` for a request key (`None` = grey / decode failure: the card stays neutral).
    pub fn set_cover_colors(&mut self, key: &str, colors: Option<CoverColors>) {
        if self.cov_pending == key {
            self.cov_pending.clear();
        }
        self.covers.put(key.to_string(), colors.clone());
        if self.cov_key() == key {
            self.cov_now = colors;
        }
    }

    // ---------------------------------------------------------------- collapsed island widget
    fn update_live(&mut self) {
        let off = !self.cfg.live_on("music") || !self.media_on() || !self.st.as_ref().map_or(false, |s| s.has_session);
        if off {
            self.fx.push(Fx::Unreact { key: "media" });
            self.clear_live();
            return;
        }
        let s = self.st.clone().unwrap();
        let accent = self.accent();
        if s.playing {
            self.fx.push(Fx::React { key: "media", kind: "music", color: Some(accent.clone()), ttl_ms: None });
        } else {
            self.fx.push(Fx::Unreact { key: "media" });
        }
        if !s.playing {
            self.clear_live(); // paused: the capsule goes back to the clock + weather
            return;
        }
        let mode = if self.mode == MusicMode::Mini { "mini" } else { "title" };
        let key = [mode, "true", &s.title, &s.artist, &self.art_id.to_string(), &accent].join("|");
        if key == self.live_key {
            return;
        }
        self.live_key = key;
        let eq = if accent.is_empty() { "var(--accent)".to_string() } else { accent.clone() };
        let name = if s.title.is_empty() { self.app_name() } else { s.title.clone() };
        self.fx.push(Fx::Live {
            key: "media",
            spec: Some(LiveSpec {
                priority: 50,
                ttl_ms: None,
                tall: false,
                accent: eq.clone(),
                body: LiveBody::Music { mode: self.mode, title: name, playing: true, cover: self.art_url.clone(), eq_color: eq },
            }),
        });
    }

    fn clear_live(&mut self) {
        if !self.live_key.is_empty() {
            self.live_key.clear();
            self.fx.push(Fx::Live { key: "media", spec: None });
        }
    }

    // ---------------------------------------------------------------- volume
    fn vol_busy_now(&self) -> bool {
        self.vol_hold || self.vol_busy || self.mono - self.vol_user_at < VOL_OWN_WINDOW_MS
    }

    /// Show the state of `vol` (the icon at once, knob / fill / number glide). `force` = a change of our own (mute click).
    fn draw_vol(&mut self, force: bool) {
        if force || !self.vol_busy_now() {
            let g = if self.vol.1 { 0.0 } else { self.vol.0 as f64 };
            self.vol_go(g);
        }
    }

    /// Move the shown value to `g`: eased (25 ms time constant while dragged, 55 ms otherwise); at once for the very first reading,
    /// while the panel is hidden and with reduced motion.
    fn vol_go(&mut self, g: f64) {
        self.vol_goal = g;
        if !self.vol_init || !self.shown || self.hidden || self.reduce() {
            self.vol_init = true;
            self.vol_shown = g;
            self.vol_animating = false;
            return;
        }
        self.vol_animating = self.vol_shown != self.vol_goal;
    }

    fn vol_step(&mut self, dt_ms: f64) {
        if !self.vol_animating {
            return;
        }
        let dt = (dt_ms / 1000.0).min(0.05);
        self.vol_shown += (self.vol_goal - self.vol_shown) * (1.0 - (-dt / if self.vol_hold { 0.025 } else { 0.055 }).exp());
        if (self.vol_goal - self.vol_shown).abs() < 0.05 {
            self.vol_shown = self.vol_goal;
        }
        self.vol_animating = self.vol_shown != self.vol_goal;
    }

    /// The user moved the volume (drag, click, wheel, keys): goal + system write. Muted + louder = unmute.
    fn vol_user(&mut self, v: f64) -> Vec<BackendCall> {
        let v = v.clamp(0.0, 100.0);
        let was_muted = self.vol.1;
        self.vol.1 = false;
        self.vol.0 = vol_round(v);
        self.vol_user_at = self.mono;
        self.vol_go(v);
        let mut calls = vec![];
        if was_muted {
            calls.push(BackendCall::new("media_mute", json!({ "muted": false })));
        }
        self.vol_want = self.vol.0;
        if !self.vol_busy {
            calls.extend(self.vol_flush());
        }
        calls
    }

    /// One write in flight, the newest value follows it: every 1 % the pointer passes reaches the system, with no pile-up.
    fn vol_flush(&mut self) -> Vec<BackendCall> {
        if self.vol_want != self.vol_sent {
            self.vol_busy = true;
            self.vol_sent = self.vol_want;
            return vec![BackendCall::new("media_volume_set", json!({ "v": self.vol_sent }))];
        }
        self.vol_busy = false;
        vec![]
    }

    // ---------------------------------------------------------------- public driving API
    /// The tab became visible (`onShow`).
    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.shown = true;
        self.launcher.set_shown(true);
        vec![
            BackendCall::bare("media_sessions"),
            BackendCall::bare("media_volume_get"),
            BackendCall::bare("media_devices"),
            BackendCall::bare("launcher_list"),
        ]
    }

    /// The tab was left (`onHide`).
    pub fn on_hide(&mut self) {
        self.shown = false;
        self.launcher.set_shown(false);
        self.dev_open = false;
        self.dev_opening = false;
    }

    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        match name {
            "media:changed" => {
                self.apply(payload);
                vec![]
            }
            "live:volume" => {
                if !payload.is_object() || !self.shown || self.vol_busy_now() {
                    return vec![];
                }
                self.vol = (vol_round(payload.get("level").and_then(Value::as_f64).unwrap_or(0.0)), bool_of(payload, "muted"));
                self.vol_sent = -1;
                self.draw_vol(false);
                vec![]
            }
            "pill:hide" => {
                self.hidden = true;
                vec![BackendCall::bare("media_pause_updates")]
            }
            "pill:show" => {
                self.hidden = false;
                self.paint_cover();
                vec![BackendCall::bare("media_resume_updates"), BackendCall::bare("settings_get")]
            }
            "settings:changed" => self.load_mode(Some(payload)),
            _ => self.launcher.on_event(name, payload),
        }
    }

    /// Answers of the backend calls this model issued.
    pub fn on_result(&mut self, cmd: &str, args: &Value, res: &Value, tr: &dyn Tr) -> Vec<BackendCall> {
        match cmd {
            "settings_get" => self.load_mode(Some(res)).into_iter().collect(),
            "media_state" => {
                if self.cover_fetch {
                    self.cover_fetch = false;
                    if let Some(url) = res.get("thumbnail").and_then(Value::as_str) {
                        self.art_id = res.get("thumb_id").and_then(Value::as_u64).unwrap_or(0);
                        self.art_url = Some(url.to_string());
                        self.render();
                        self.update_live();
                    }
                } else {
                    self.apply(res);
                }
                vec![]
            }
            "media_sessions" => {
                self.sessions = res
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|v| Session {
                                id: str_of(v, "id").to_string(),
                                app: str_of(v, "app").to_string(),
                                title: str_of(v, "title").to_string(),
                                artist: str_of(v, "artist").to_string(),
                                playing: bool_of(v, "playing"),
                                selected: bool_of(v, "selected"),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                vec![]
            }
            "media_volume_get" => {
                if res.is_object() {
                    self.vol = (i64_of(res, "volume"), bool_of(res, "muted"));
                    self.vol_sent = -1;
                    self.draw_vol(false);
                }
                vec![]
            }
            "media_volume_set" => self.vol_flush(),
            "media_devices" => {
                self.devs = res
                    .as_array()
                    .map(|a| a.iter().map(|v| Device { id: str_of(v, "id").to_string(), name: str_of(v, "name").to_string(), default: bool_of(v, "default") }).collect())
                    .unwrap_or_default();
                if self.dev_opening {
                    self.dev_opening = false;
                    self.dev_open = !self.devs.is_empty();
                }
                if self.after_pick {
                    self.after_pick = false;
                    return vec![BackendCall::bare("media_volume_get")]; // the slider now belongs to the new default device
                }
                vec![]
            }
            "media_set_default_device" => {
                if res.as_str() == Some("settings") {
                    self.fx.toast(
                        "dev-settings",
                        "volume",
                        tr.t(
                            "Windows не даёт менять устройство из приложения: открыл настройки звука",
                            "Windows does not let apps switch the output device: opened the sound settings",
                        ),
                        4500,
                    );
                }
                self.after_pick = true;
                let _ = args;
                vec![BackendCall::bare("media_devices")]
            }
            _ => self.launcher.on_result(cmd, args, res, tr),
        }
    }

    pub fn on_action(&mut self, a: Action, tr: &dyn Tr) -> Vec<BackendCall> {
        match a {
            Action::Ctl(c) | Action::LiveButton(c) => vec![BackendCall::new("media_control", json!({ "action": c.action() }))],
            Action::Tile(id) => {
                if let Some(fx) = tile_fx(&id) {
                    self.fx.push(fx);
                }
                vec![]
            }
            Action::SourceClick => {
                if self.sessions.len() < 2 {
                    return vec![];
                }
                let i = self.sessions.iter().position(|s| s.selected).map_or(-1, |i| i as i64);
                let next = &self.sessions[((i + 1) as usize) % self.sessions.len()];
                vec![BackendCall::new("media_select", json!({ "id": next.id }))]
            }
            Action::SeekDrag(v) => {
                self.dragging = Some(v);
                vec![]
            }
            Action::SeekCommit(v) => {
                self.dragging = None;
                self.base_pos = v as i64;
                self.base_at = self.mono;
                vec![BackendCall::new("media_seek", json!({ "ms": v.round() as i64 }))]
            }
            Action::VolPointerDown(v) => {
                self.vol_hold = true;
                self.vol_user(v as f64)
            }
            Action::VolPointerMove(v) => {
                if self.vol_hold {
                    self.vol_user(v as f64)
                } else {
                    vec![]
                }
            }
            Action::VolPointerUp => {
                self.vol_hold = false;
                vec![]
            }
            Action::VolKey(k) => {
                let d = match k {
                    VolKey::Left | VolKey::Down => -1.0,
                    VolKey::Right | VolKey::Up => 1.0,
                    VolKey::PageDown => -10.0,
                    VolKey::PageUp => 10.0,
                    VolKey::Home | VolKey::End => 0.0,
                };
                let g = match k {
                    VolKey::Home => 0.0,
                    VolKey::End => 100.0,
                    _ => vol_round(self.vol_goal) as f64 + d,
                };
                self.vol_user(g)
            }
            Action::VolWheel { delta_y, lines } => {
                if delta_y == 0.0 {
                    return vec![];
                }
                // pixels: 100 per notch; lines: 3 per notch; 2 % per notch; wheel up = louder
                self.wheel_acc -= (if lines { delta_y / 3.0 } else { delta_y / 100.0 }) * 2.0;
                let steps = self.wheel_acc.trunc();
                if steps == 0.0 {
                    return vec![];
                }
                self.wheel_acc -= steps;
                self.vol_user(vol_round(self.vol_goal) as f64 + steps as f64)
            }
            Action::MuteClick => {
                self.vol.1 = !self.vol.1;
                self.vol_user_at = self.mono;
                self.draw_vol(true);
                vec![BackendCall::new("media_mute", json!({ "muted": self.vol.1 }))]
            }
            Action::DevToggle => {
                if self.dev_open {
                    self.dev_open = false;
                    return vec![];
                }
                self.dev_opening = true;
                vec![BackendCall::bare("media_devices")]
            }
            Action::DevPick(id) => {
                self.dev_open = false;
                vec![BackendCall::new("media_set_default_device", json!({ "id": id }))]
            }
            Action::DevClose => {
                self.dev_open = false;
                vec![]
            }
            Action::Launcher(a) => self.launcher.on_action(a, tr),
        }
    }

    /// Advance the model's clock: volume glide, delayed sessions refresh, launcher timers.
    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        self.mono += dt_ms;
        self.vol_step(dt_ms);
        let mut calls = self.launcher.tick(dt_ms);
        if let Some(at) = self.sess_at {
            if self.mono >= at {
                self.sess_at = None;
                calls.push(BackendCall::bare("media_sessions"));
            }
        }
        if self.cov_request.is_none() && self.cov_pending.is_empty() {
            self.paint_cover();
        }
        calls
    }

    // ---------------------------------------------------------------- view
    pub fn view(&self, tr: &dyn Tr) -> View {
        let st = self.st.as_ref();
        let has = st.map_or(false, |s| s.has_session);
        let dur = st.map_or(0, |s| s.duration_ms);
        let p = self.pos();
        let value = self.dragging.unwrap_or(p);
        let multi = self.sessions.len() > 1;
        let playing = st.map_or(false, |s| s.playing);
        let app = self.app_name();
        let title = match st {
            Some(s) if !s.title.is_empty() => s.title.clone(),
            _ if !app.is_empty() => app.clone(),
            _ => tr.t("Неизвестно", "Unknown"),
        };
        let artist_line = st.map(|s| [s.artist.as_str(), s.album.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" - ")).unwrap_or_default();
        let seek = Seek {
            enabled: dur > 0,
            max_ms: dur,
            value_ms: value,
            fill_pct: if dur > 0 { (value / dur as f64 * 100.0).clamp(0.0, 100.0) } else { 0.0 },
            elapsed: fmt_ms(value as i64),
            remaining: if dur > 0 { format!("-{}", fmt_ms((dur as f64 - p) as i64)) } else { "--:--".to_string() },
        };
        let v_num = vol_round(self.vol_shown);
        let default_name = self.devs.iter().find(|d| d.default).map(|d| d.name.clone()).unwrap_or_default();
        View {
            player: Player {
                visible: self.media_on(),
                none: !has,
                empty_text: tr.t("Сейчас ничего не играет", "Nothing is playing"),
                accent: self.accent(),
                cover_url: self.art_url.clone(),
                cover_gradient: if has { self.cov_now.clone() } else { None },
                source: app,
                source_multi: multi,
                source_title: if multi { tr.t("Сменить плеер", "Switch player") } else { String::new() },
                title,
                artist_line,
                playing,
                play_glyph: if playing { "pause" } else { "play" },
                seek,
                seek_label: tr.t("Перемотка", "Seek"),
                prev_title: tr.t("Назад", "Previous"),
                play_title: tr.t("Пауза / пуск", "Play / pause"),
                next_title: tr.t("Вперёд", "Next"),
            },
            tiles: TILES
                .iter()
                .map(|(id, icon, ru, en)| {
                    let soon = tile_fx(id).is_none();
                    TileView { id, icon, label: tr.t(ru, en), soon, tooltip: if soon { Some(tr.t("Скоро", "Soon")) } else { None } }
                })
                .collect(),
            volume: VolumeView {
                shown: self.vol_shown,
                number: v_num,
                muted: self.vol.1,
                glyph: if self.vol.1 || self.vol.0 == 0 { "mute" } else { "volume" },
                mute_title: tr.t("Без звука", "Mute"),
                label: tr.t("Громкость", "Volume"),
            },
            devices: DevicesView {
                open: self.dev_open,
                button_text: if default_name.is_empty() { tr.t("По умолчанию", "Default") } else { default_name.clone() },
                button_title: if default_name.is_empty() { tr.t("Устройство вывода", "Output device") } else { default_name },
                items: self.devs.iter().map(|d| DeviceItem { id: d.id.clone(), name: d.name.clone(), current: d.default }).collect(),
            },
            launcher: self.launcher.view(tr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    fn media(playing: bool, pos: i64, dur: i64) -> Value {
        json!({ "has_session": true, "playing": playing, "title": "Song", "artist": "Band", "album": "LP", "session_id": "s1", "app": "Spotify.exe",
                "position_ms": pos, "duration_ms": dur, "thumb_id": 1, "thumbnail": "data:image/png;base64,AAAA", "accent": "#aa3355" })
    }

    fn open() -> Home {
        let mut h = Home::new();
        h.on_show();
        h.on_event("media:changed", &media(true, 10_000, 200_000));
        h.take_fx();
        h
    }

    #[test]
    fn init_asks_for_state_and_settings() {
        let mut h = Home::new();
        let c = h.init_calls();
        assert_eq!(c[0], BackendCall::new("media_state", json!({ "known": null })));
        assert_eq!(c[1].cmd, "settings_get");
    }

    #[test]
    fn show_requests_the_four_loads() {
        let mut h = Home::new();
        let cmds: Vec<_> = h.on_show().iter().map(|c| c.cmd).collect();
        assert_eq!(cmds, ["media_sessions", "media_volume_get", "media_devices", "launcher_list"]);
    }

    #[test]
    fn seek_bar_interpolates_while_playing_and_clamps_to_duration() {
        let mut h = open();
        assert_eq!(h.position_ms(), 10_000.0);
        h.tick(2500.0);
        assert_eq!(h.position_ms(), 12_500.0);
        let v = h.view(&EnTr).player.seek;
        assert_eq!((v.elapsed.as_str(), v.remaining.as_str()), ("0:12", "-3:07"));
        assert!((v.fill_pct - 6.25).abs() < 1e-9);
        h.tick(1_000_000.0);
        assert_eq!(h.position_ms(), 200_000.0);
        assert_eq!(h.view(&EnTr).player.seek.remaining, "-0:00");
    }

    #[test]
    fn paused_position_stays_put_and_unknown_duration_disables_seek() {
        let mut h = open();
        h.on_event("media:changed", &media(false, 30_000, 0));
        h.tick(5000.0);
        assert_eq!(h.position_ms(), 30_000.0);
        let s = h.view(&EnTr).player.seek;
        assert!(!s.enabled);
        assert_eq!(s.remaining, "--:--");
        assert_eq!(s.fill_pct, 0.0);
    }

    #[test]
    fn new_media_event_rebases_the_clock() {
        let mut h = open();
        h.tick(4000.0);
        h.on_event("media:changed", &media(true, 50_000, 200_000));
        assert_eq!(h.position_ms(), 50_000.0);
        h.tick(1000.0);
        assert_eq!(h.position_ms(), 51_000.0);
    }

    #[test]
    fn seek_drag_shows_the_pointer_value_and_commit_sends_and_rebases() {
        let mut h = open();
        h.on_action(Action::SeekDrag(90_000.0), &EnTr);
        h.tick(3000.0);
        let s = h.view(&EnTr).player.seek;
        assert_eq!(s.elapsed, "1:30");
        assert_eq!(s.value_ms, 90_000.0);
        let c = h.on_action(Action::SeekCommit(90_400.4), &EnTr);
        assert_eq!(c, vec![BackendCall::new("media_seek", json!({ "ms": 90_400 }))]);
        assert_eq!(h.position_ms(), 90_400.0);
        h.tick(1000.0);
        assert_eq!(h.position_ms(), 91_400.0);
    }

    #[test]
    fn controls_map_to_media_control() {
        let mut h = open();
        for (c, a) in [(Ctl::Prev, "prev"), (Ctl::PlayPause, "play_pause"), (Ctl::Next, "next")] {
            assert_eq!(h.on_action(Action::Ctl(c), &EnTr), vec![BackendCall::new("media_control", json!({ "action": a }))]);
            assert_eq!(h.on_action(Action::LiveButton(c), &EnTr)[0].args["action"], a);
        }
    }

    #[test]
    fn title_fallbacks_and_artist_line() {
        let mut h = open();
        assert_eq!(h.view(&RuTr).player.source, "Spotify");
        assert_eq!(h.view(&RuTr).player.artist_line, "Band - LP");
        let mut m = media(true, 0, 1000);
        m["title"] = json!("");
        m["album"] = json!("");
        h.on_event("media:changed", &m);
        assert_eq!(h.view(&RuTr).player.title, "Spotify");
        assert_eq!(h.view(&RuTr).player.artist_line, "Band");
        m["app"] = json!("");
        h.on_event("media:changed", &m);
        assert_eq!(h.view(&RuTr).player.title, "Неизвестно");
        assert_eq!(h.view(&EnTr).player.title, "Unknown");
    }

    #[test]
    fn no_session_is_the_empty_state() {
        let mut h = open();
        h.on_event("media:changed", &json!({ "has_session": false, "playing": false }));
        let v = h.view(&EnTr);
        assert!(v.player.none);
        assert_eq!(v.player.empty_text, "Nothing is playing");
        assert!(v.player.cover_gradient.is_none());
    }

    #[test]
    fn text_and_glyph_cues_fire_only_on_real_change() {
        let mut h = open();
        h.on_event("media:changed", &media(true, 20_000, 200_000)); // same text, same state
        assert!(h.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
        let mut m = media(false, 20_000, 200_000);
        m["title"] = json!("Other");
        h.on_event("media:changed", &m);
        let targets: Vec<_> = h.take_fx().into_iter().filter_map(|f| if let Fx::Anim { target, .. } = f { Some(target) } else { None }).collect();
        assert_eq!(targets, ["title", "play"]);
    }

    #[test]
    fn first_render_has_no_cues_and_reduced_motion_none_at_all() {
        let mut h = Home::new();
        h.on_event("media:changed", &media(true, 0, 1000));
        assert!(h.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
        h.on_event("settings:changed", &json!({ "reduceMotion": true }));
        let mut m = media(false, 0, 1000);
        m["title"] = json!("Zzz");
        m["thumbnail"] = json!("data:other");
        m["thumb_id"] = json!(2);
        h.on_event("media:changed", &m);
        assert!(h.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
    }

    #[test]
    fn cover_change_crossfades() {
        let mut h = open();
        let mut m = media(true, 0, 1000);
        m["thumb_id"] = json!(2);
        m["thumbnail"] = json!("data:new");
        h.on_event("media:changed", &m);
        let fx = h.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "art" && anim.dur_ms == 250)));
    }

    #[test]
    fn missed_cover_is_fetched_once() {
        let mut h = open();
        let mut m = media(true, 0, 1000);
        m["thumb_id"] = json!(5);
        m["thumbnail"] = Value::Null;
        h.on_event("media:changed", &m);
        assert_eq!(h.view(&EnTr).player.cover_url, None);
        // the fetch result carries the cover and must not replace the track state
        let before = h.media().cloned();
        h.on_result("media_state", &json!({ "known": null }), &json!({ "thumb_id": 5, "thumbnail": "data:fetched", "title": "IGNORED" }), &EnTr);
        assert_eq!(h.view(&EnTr).player.cover_url.as_deref(), Some("data:fetched"));
        assert_eq!(h.media().cloned(), before);
    }

    #[test]
    fn sessions_refresh_400ms_after_an_event_while_shown() {
        let mut h = open();
        h.on_event("media:changed", &media(true, 0, 1000));
        assert!(h.tick(399.0).is_empty());
        assert_eq!(h.tick(2.0), vec![BackendCall::bare("media_sessions")]);
        assert!(h.tick(1000.0).is_empty());
        h.on_hide();
        h.on_event("media:changed", &media(true, 0, 1000));
        assert!(h.tick(1000.0).is_empty());
    }

    #[test]
    fn source_label_cycles_players() {
        let mut h = open();
        let ss = json!([{ "id": "a", "app": "A", "title": "", "artist": "", "playing": true, "selected": false },
                        { "id": "b", "app": "B", "title": "", "artist": "", "playing": false, "selected": true }]);
        assert!(h.on_action(Action::SourceClick, &EnTr).is_empty()); // one player: nothing to switch
        h.on_result("media_sessions", &Value::Null, &ss, &EnTr);
        assert!(h.view(&EnTr).player.source_multi);
        assert_eq!(h.view(&RuTr).player.source_title, "Сменить плеер");
        assert_eq!(h.on_action(Action::SourceClick, &EnTr), vec![BackendCall::new("media_select", json!({ "id": "a" }))]);
        h.on_result("media_sessions", &Value::Null, &Value::Null, &EnTr);
        assert!(!h.view(&EnTr).player.source_multi);
    }

    #[test]
    fn tiles_map_to_effects_and_plain_is_soon() {
        let mut h = Home::new();
        for (id, fx) in [
            ("area", Fx::StartCapture { mode: "area" }),
            ("screen", Fx::StartCapture { mode: "screen" }),
            ("ocr", Fx::StartCapture { mode: "text" }),
            ("qr", Fx::StartCapture { mode: "qr" }),
            ("picker", Fx::StartCapture { mode: "color" }),
            ("record", Fx::StartCapture { mode: "record_screen" }),
            ("command", Fx::OpenCommandBar),
        ] {
            h.on_action(Action::Tile(id.into()), &EnTr);
            assert_eq!(h.take_fx(), vec![fx], "{id}");
        }
        h.on_action(Action::Tile("plain".into()), &EnTr);
        assert!(h.take_fx().is_empty());
        let tiles = h.view(&RuTr).tiles;
        assert_eq!(tiles.len(), 8);
        assert!(tiles.iter().find(|t| t.id == "plain").unwrap().soon);
        assert_eq!(tiles.iter().find(|t| t.id == "plain").unwrap().tooltip.as_deref(), Some("Скоро"));
        assert_eq!(tiles[0].label, "Область");
        assert_eq!(tiles.iter().filter(|t| t.soon).count(), 1);
    }

    // ---------------- volume ----------------

    fn with_volume(v: i64, muted: bool) -> Home {
        let mut h = open();
        h.on_result("media_volume_get", &Value::Null, &json!({ "volume": v, "muted": muted }), &EnTr);
        h
    }

    #[test]
    fn first_volume_reading_jumps_then_changes_glide() {
        let mut h = with_volume(40, false);
        assert_eq!(h.view(&EnTr).volume.shown, 40.0);
        h.tick(1000.0); // let the own-change window pass
        h.on_event("live:volume", &json!({ "level": 80, "muted": false }));
        assert_eq!(h.view(&EnTr).volume.shown, 40.0); // goal moved, knob not yet
        assert!(h.animating());
        h.tick(16.0);
        let s1 = h.view(&EnTr).volume.shown;
        assert!(s1 > 40.0 && s1 < 80.0);
        for _ in 0..40 {
            h.tick(16.0);
        }
        assert_eq!(h.view(&EnTr).volume.shown, 80.0);
        assert_eq!(h.view(&EnTr).volume.number, 80);
    }

    #[test]
    fn glide_time_constants_55_and_25_ms() {
        let mut h = with_volume(0, false);
        h.tick(1000.0);
        h.on_event("live:volume", &json!({ "level": 100, "muted": false }));
        h.tick(55.0); // dt capped to 0.05 s: 1 - exp(-0.05 / 0.055)
        let want = 100.0 * (1.0 - (-0.05f64 / 0.055).exp());
        assert!((h.view(&EnTr).volume.shown - want).abs() < 1e-9);
    }

    #[test]
    fn reduced_motion_or_hidden_panel_snaps() {
        let mut h = with_volume(10, false);
        h.on_event("settings:changed", &json!({ "reduceMotion": true }));
        h.tick(1000.0);
        h.on_event("live:volume", &json!({ "level": 90, "muted": false }));
        assert_eq!(h.view(&EnTr).volume.shown, 90.0);
        assert!(!h.animating() || h.view(&EnTr).volume.shown == 90.0);
    }

    #[test]
    fn user_volume_writes_are_serialized_and_coalesced() {
        let mut h = with_volume(50, false);
        h.on_action(Action::VolPointerDown(10.0), &EnTr);
        let c = h.on_action(Action::VolPointerMove(11.0), &EnTr); // a write is in flight: nothing new yet
        assert!(c.is_empty());
        let c = h.on_action(Action::VolPointerMove(12.4), &EnTr);
        assert!(c.is_empty());
        // the first write was sent by the pointer-down
        let next = h.on_result("media_volume_set", &json!({ "v": 10 }), &Value::Null, &EnTr);
        assert_eq!(next, vec![BackendCall::new("media_volume_set", json!({ "v": 12 }))]); // newest value only, 11 skipped
        assert!(h.on_result("media_volume_set", &json!({ "v": 12 }), &Value::Null, &EnTr).is_empty());
        // idle again: the next change sends at once
        h.on_action(Action::VolPointerUp, &EnTr);
        assert_eq!(h.on_action(Action::VolKey(VolKey::Up), &EnTr), vec![BackendCall::new("media_volume_set", json!({ "v": 13 }))]);
    }

    #[test]
    fn first_pointer_down_sends_immediately() {
        let mut h = with_volume(50, false);
        let c = h.on_action(Action::VolPointerDown(33.0), &EnTr);
        assert_eq!(c, vec![BackendCall::new("media_volume_set", json!({ "v": 33 }))]);
        assert_eq!(h.view(&EnTr).volume.number, 50); // the knob has not moved yet: it glides
    }

    #[test]
    fn dragging_up_from_muted_unmutes_first() {
        let mut h = with_volume(50, true);
        assert_eq!(h.view(&EnTr).volume.glyph, "mute");
        let c = h.on_action(Action::VolPointerDown(30.0), &EnTr);
        assert_eq!(c[0], BackendCall::new("media_mute", json!({ "muted": false })));
        assert_eq!(c[1], BackendCall::new("media_volume_set", json!({ "v": 30 })));
        assert!(!h.view(&EnTr).volume.muted);
        assert_eq!(h.view(&EnTr).volume.glyph, "volume");
    }

    #[test]
    fn mute_click_toggles_glides_to_zero_and_writes() {
        let mut h = with_volume(60, false);
        let c = h.on_action(Action::MuteClick, &EnTr);
        assert_eq!(c, vec![BackendCall::new("media_mute", json!({ "muted": true }))]);
        assert!(h.view(&EnTr).volume.muted);
        for _ in 0..60 {
            h.tick(16.0);
        }
        assert_eq!(h.view(&EnTr).volume.shown, 0.0);
        let c = h.on_action(Action::MuteClick, &EnTr);
        assert_eq!(c[0].args, json!({ "muted": false }));
    }

    #[test]
    fn outside_readings_wait_while_busy() {
        let mut h = with_volume(50, false);
        h.on_action(Action::VolPointerDown(20.0), &EnTr);
        h.on_event("live:volume", &json!({ "level": 99, "muted": false })); // ignored: our write is in flight
        h.on_result("media_volume_set", &Value::Null, &Value::Null, &EnTr);
        h.on_event("live:volume", &json!({ "level": 99, "muted": false })); // ignored: 600 ms window
        h.on_action(Action::VolPointerUp, &EnTr);
        for _ in 0..30 {
            h.tick(16.0);
        }
        assert!(h.view(&EnTr).volume.shown < 30.0);
        h.tick(200.0);
        h.on_event("live:volume", &json!({ "level": 99, "muted": false }));
        assert!(h.animating());
    }

    #[test]
    fn keys_step_and_jump() {
        let mut h = with_volume(50, false);
        assert_eq!(h.on_action(Action::VolKey(VolKey::PageUp), &EnTr)[0].args, json!({ "v": 60 }));
        h.on_result("media_volume_set", &Value::Null, &Value::Null, &EnTr);
        assert_eq!(h.on_action(Action::VolKey(VolKey::End), &EnTr)[0].args, json!({ "v": 100 }));
        h.on_result("media_volume_set", &Value::Null, &Value::Null, &EnTr);
        assert!(h.on_action(Action::VolKey(VolKey::Right), &EnTr).is_empty()); // clamped: nothing new to write
        assert_eq!(h.on_action(Action::VolKey(VolKey::Home), &EnTr)[0].args, json!({ "v": 0 }));
    }

    #[test]
    fn wheel_accumulates_two_percent_per_notch() {
        let mut h = with_volume(50, false);
        // one pixel notch up (-100) = +2 %
        assert_eq!(h.on_action(Action::VolWheel { delta_y: -100.0, lines: false }, &EnTr)[0].args, json!({ "v": 52 }));
        h.on_result("media_volume_set", &Value::Null, &Value::Null, &EnTr);
        // touchpad deltas add up: 20 px = 0.4 %, nothing until the sum passes 1
        assert!(h.on_action(Action::VolWheel { delta_y: -20.0, lines: false }, &EnTr).is_empty());
        assert!(h.on_action(Action::VolWheel { delta_y: -20.0, lines: false }, &EnTr).is_empty());
        let c = h.on_action(Action::VolWheel { delta_y: -20.0, lines: false }, &EnTr); // 1.2 % total
        assert_eq!(c[0].args, json!({ "v": 53 }));
        h.on_result("media_volume_set", &Value::Null, &Value::Null, &EnTr);
        // lines mode: 3 lines per notch, wheel down = quieter (0.2 % was left over: -2 + 0.2 truncates to -1)
        assert_eq!(h.on_action(Action::VolWheel { delta_y: 3.0, lines: true }, &EnTr)[0].args, json!({ "v": 52 }));
        assert!(h.on_action(Action::VolWheel { delta_y: 0.0, lines: false }, &EnTr).is_empty());
    }

    #[test]
    fn pointer_maps_to_thumb_travel() {
        // 200 px wide track, 12 px thumb: 6 px inset
        assert_eq!(vol_at(100.0, 0.0, 200.0, 200.0), 50.0);
        assert_eq!(vol_at(0.0, 0.0, 200.0, 200.0), 0.0);
        assert_eq!(vol_at(500.0, 0.0, 200.0, 200.0), 100.0);
        // zoomed panel: offsetWidth 100, on-screen width 200 -> 12 px inset on screen
        assert_eq!(vol_at(100.0, 0.0, 200.0, 100.0), 50.0);
        assert_eq!(vol_at(5.0, 0.0, 0.0, 0.0), 100.0); // degenerate geometry behaves like the JS (max(1, ..) divisor)
    }

    // ---------------- devices ----------------

    fn devices() -> Value {
        json!([{ "id": "d1", "name": "Speakers", "default": true }, { "id": "d2", "name": "Headset", "default": false }])
    }

    #[test]
    fn device_menu_opens_after_the_list_arrives() {
        let mut h = open();
        assert_eq!(h.view(&RuTr).devices.button_text, "По умолчанию");
        assert_eq!(h.on_action(Action::DevToggle, &EnTr), vec![BackendCall::bare("media_devices")]);
        assert!(!h.view(&EnTr).devices.open);
        h.on_result("media_devices", &Value::Null, &devices(), &EnTr);
        let d = h.view(&EnTr).devices;
        assert!(d.open);
        assert_eq!(d.button_text, "Speakers");
        assert_eq!(d.items.iter().filter(|i| i.current).count(), 1);
        assert!(h.wants_escape());
        h.on_action(Action::DevToggle, &EnTr);
        assert!(!h.view(&EnTr).devices.open && !h.wants_escape());
    }

    #[test]
    fn empty_device_list_keeps_the_menu_closed() {
        let mut h = open();
        h.on_action(Action::DevToggle, &EnTr);
        h.on_result("media_devices", &Value::Null, &json!([]), &EnTr);
        assert!(!h.view(&EnTr).devices.open);
    }

    #[test]
    fn picking_a_device_reloads_devices_then_volume_and_may_toast() {
        let mut h = open();
        h.on_action(Action::DevToggle, &EnTr);
        h.on_result("media_devices", &Value::Null, &devices(), &EnTr);
        let c = h.on_action(Action::DevPick("d2".into()), &EnTr);
        assert_eq!(c, vec![BackendCall::new("media_set_default_device", json!({ "id": "d2" }))]);
        assert!(!h.view(&EnTr).devices.open);
        let c = h.on_result("media_set_default_device", &json!({ "id": "d2" }), &json!("settings"), &EnTr);
        assert_eq!(c, vec![BackendCall::bare("media_devices")]);
        assert!(matches!(&h.take_fx()[..], [Fx::Toast { id: "dev-settings", ttl_ms: 4500, .. }]));
        let c = h.on_result("media_devices", &Value::Null, &devices(), &EnTr);
        assert_eq!(c, vec![BackendCall::bare("media_volume_get")]);
        // a plain devices refresh afterwards does not reload the volume
        assert!(h.on_result("media_devices", &Value::Null, &devices(), &EnTr).is_empty());
    }

    #[test]
    fn leaving_the_tab_closes_the_menu() {
        let mut h = open();
        h.on_action(Action::DevToggle, &EnTr);
        h.on_result("media_devices", &Value::Null, &devices(), &EnTr);
        h.on_hide();
        assert!(!h.view(&EnTr).devices.open);
        h.on_action(Action::DevClose, &EnTr);
    }

    // ---------------- settings and modules ----------------

    #[test]
    fn module_media_off_clears_state_and_back_on_refetches() {
        let mut h = open();
        let c = h.on_event("settings:changed", &json!({ "module.media": false }));
        assert!(c.is_empty());
        assert!(h.media().is_none());
        assert!(!h.view(&EnTr).player.visible);
        h.on_event("media:changed", &media(true, 0, 1000)); // ignored while off
        assert!(h.media().is_none());
        let c = h.on_event("settings:changed", &json!({ "module.media": true }));
        assert_eq!(c, vec![BackendCall::new("media_state", json!({ "known": null }))]);
        assert!(h.on_event("settings:changed", &json!({ "module.media": true })).is_empty());
    }

    #[test]
    fn settings_result_feeds_the_same_path_and_pill_show_pauses_nothing() {
        let mut h = Home::new();
        assert_eq!(h.on_event("pill:hide", &Value::Null), vec![BackendCall::bare("media_pause_updates")]);
        let c = h.on_event("pill:show", &Value::Null);
        assert_eq!(c.iter().map(|c| c.cmd).collect::<Vec<_>>(), ["media_resume_updates", "settings_get"]);
        h.on_result("settings_get", &Value::Null, &json!({ "musicPill": "mini", "module.launcher": false }), &EnTr);
        assert!(!h.view(&EnTr).launcher.visible);
    }

    // ---------------- collapsed widget ----------------

    #[test]
    fn music_widget_follows_playback_and_dedupes() {
        let mut h = Home::new();
        h.on_event("media:changed", &media(true, 0, 1000));
        let fx = h.take_fx();
        assert!(fx.contains(&Fx::React { key: "media", kind: "music", color: Some("#aa3355".into()), ttl_ms: None }));
        let spec = fx.iter().find_map(|f| if let Fx::Live { spec: Some(s), .. } = f { Some(s.clone()) } else { None }).unwrap();
        assert_eq!(spec.priority, 50);
        assert_eq!(spec.accent, "#aa3355");
        assert_eq!(spec.body, LiveBody::Music { mode: MusicMode::Title, title: "Song".into(), playing: true, cover: Some("data:image/png;base64,AAAA".into()), eq_color: "#aa3355".into() });
        // identical event: no new widget
        h.on_event("media:changed", &media(true, 5000, 1000));
        assert!(h.take_fx().iter().all(|f| !matches!(f, Fx::Live { .. })));
        // paused: the capsule goes back to the clock
        h.on_event("media:changed", &media(false, 5000, 1000));
        let fx = h.take_fx();
        assert!(fx.contains(&Fx::Unreact { key: "media" }));
        assert!(fx.contains(&Fx::Live { key: "media", spec: None }));
        // resumed: shown again
        h.on_event("media:changed", &media(true, 5000, 1000));
        assert!(h.take_fx().iter().any(|f| matches!(f, Fx::Live { spec: Some(_), .. })));
    }

    #[test]
    fn mini_mode_and_default_accent() {
        let mut h = Home::new();
        h.on_event("settings:changed", &json!({ "musicPill": "mini" }));
        let mut m = media(true, 0, 1000);
        m["accent"] = Value::Null;
        m["title"] = json!("");
        h.on_event("media:changed", &m);
        let spec = h.take_fx().into_iter().find_map(|f| if let Fx::Live { spec: Some(s), .. } = f { Some(s) } else { None }).unwrap();
        assert_eq!(spec.accent, "var(--accent)");
        match spec.body {
            LiveBody::Music { mode, title, .. } => assert_eq!((mode, title.as_str()), (MusicMode::Mini, "Spotify")),
            b => panic!("{b:?}"),
        }
    }

    #[test]
    fn live_music_switch_or_module_off_hides_the_widget() {
        let mut h = Home::new();
        h.on_event("media:changed", &media(true, 0, 1000));
        h.take_fx();
        h.on_event("settings:changed", &json!({ "live.music": false }));
        let fx = h.take_fx();
        assert!(fx.contains(&Fx::Unreact { key: "media" }) && fx.contains(&Fx::Live { key: "media", spec: None }));
    }

    // ---------------- cover gradient plumbing ----------------

    #[test]
    fn cover_requests_once_per_cover_and_caches() {
        let mut h = Home::new();
        h.on_event("media:changed", &media(true, 0, 1000));
        let (key, url) = h.take_cover_request().unwrap();
        assert_eq!(url, "data:image/png;base64,AAAA");
        assert_eq!(key, "1:26");
        assert!(h.take_cover_request().is_none());
        h.tick(16.0); // pending: no second request
        assert!(h.take_cover_request().is_none());
        let c = CoverColors { a: "hsl(1, 50%, 40%)".into(), b: "hsl(26, 50%, 26%)".into() };
        h.set_cover_colors(&key, Some(c.clone()));
        assert_eq!(h.view(&EnTr).player.cover_gradient, Some(c.clone()));
        // same cover again: served from the cache
        h.on_event("media:changed", &media(true, 0, 1000));
        assert!(h.take_cover_request().is_none());
        assert_eq!(h.view(&EnTr).player.cover_gradient, Some(c));
        // a grey cover caches None
        let mut m = media(true, 0, 1000);
        m["thumb_id"] = json!(9);
        m["thumbnail"] = json!("data:grey");
        h.on_event("media:changed", &m);
        let (k2, _) = h.take_cover_request().unwrap();
        h.set_cover_colors(&k2, None);
        assert_eq!(h.view(&EnTr).player.cover_gradient, None);
    }

    #[test]
    fn hidden_window_does_no_cover_work_until_shown() {
        let mut h = Home::new();
        h.on_event("pill:hide", &Value::Null);
        h.on_event("media:changed", &media(true, 0, 1000));
        assert!(h.take_cover_request().is_none());
        h.on_event("pill:show", &Value::Null);
        assert!(h.take_cover_request().is_some());
    }

    #[test]
    fn animating_only_while_visible_and_playing() {
        let mut h = Home::new();
        assert!(!h.animating());
        h.on_event("media:changed", &media(true, 0, 1000));
        assert!(!h.animating()); // tab not shown
        h.on_show();
        assert!(h.animating());
        h.on_event("pill:hide", &Value::Null);
        assert!(!h.animating());
        h.on_event("pill:show", &Value::Null);
        h.on_event("media:changed", &media(false, 0, 1000));
        assert!(!h.animating());
    }

    #[test]
    fn launcher_events_and_results_route_through() {
        let mut h = open();
        h.on_event("launcher:changed", &json!({ "items": [{ "id": 1, "title": "A", "target": "x", "missing": false, "is_dir": false }] }));
        assert_eq!(h.view(&EnTr).launcher.tiles.len(), 1);
        let c = h.on_action(Action::Launcher(launcher::Action::Open(1)), &EnTr);
        assert_eq!(c[0].cmd, "launcher_open");
        h.on_result("launcher_open", &json!({ "id": 1 }), &json!(true), &EnTr);
        assert!(h.take_fx().contains(&Fx::Collapse { via: None }));
    }
}
