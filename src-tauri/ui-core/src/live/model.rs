//! The data vocabulary of the live layer: what a provider hands to the renderer.
//!
//! Everything here is plain data (no behaviour, no I/O). A provider turns a backend event
//! payload into [`LiveCmd`]s; the renderer draws [`RowSpec`] (the collapsed capsule row) and
//! [`CardSpec`] (the event card that hangs from the capsule) on Direct2D and runs the [`Action`]s
//! the buttons carry.
//!
//! # Row semantics
//! * `id` is the slot (`kbd`, `copy`, `volume`, `download`, `media`, ...). Setting an id again
//!   replaces the row but keeps its place in the board (a `Map.set` in the JS).
//! * Highest `priority` wins the capsule; on a tie the row that was inserted first wins.
//! * `ttl_ms == None` keeps the row until it is cleared; `Some(ms)` expires `ms` after the *last* set.
//! * `key`: the same key as the row on screen means "edit in place" (animate `Anim::Glide` values,
//!   no enter animation, no re-measure flash); a new key is a fresh row (play `anim.enter`).
//! * `accent` colours the liquid outline while the row is on top.
//! * `width` is the row's minimum width in css px (`--lpw`), `tall` lets the row grow to its own height.
//!
//! # Animation (standing rule: every UI feature animates; Reduce motion respected)
//! [`Anim`] is data. The renderer plays `enter` when a row/card appears (new key), `update` when the
//! same key is re-set, `leave` when it is cleared or expires. With Reduce motion on the renderer
//! swaps in the end state at once (all `Anim` values and the looping `pulse_ms` / `Spinner` /
//! `Eq` motions are skipped). Nothing in this layer lowers a tick or refresh rate.

use serde_json::Value;

use super::sysmon::MetersView;

// ---------------------------------------------------------------- colour

/// 8-bit RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba(r, g, b, 255)
    }

    /// `#rgb`, `#rrggbb` or `#rrggbbaa` (the leading `#` is required, as in the media accent).
    pub fn from_hex(s: &str) -> Option<Rgba> {
        let h = s.trim().strip_prefix('#')?;
        if !h.is_ascii() {
            return None;
        }
        let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        match h.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|v| v * 17);
                Some(Rgba(d(0)?, d(1)?, d(2)?, 255))
            }
            6 => Some(Rgba(p(0)?, p(2)?, p(4)?, 255)),
            8 => Some(Rgba(p(0)?, p(2)?, p(4)?, p(6)?)),
            _ => None,
        }
    }

    /// `#rrggbb` (alpha dropped when opaque, appended otherwise).
    pub fn to_hex(self) -> String {
        if self.3 == 255 {
            format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.0, self.1, self.2, self.3)
        }
    }
}

/// Fixed colours of the live rows (the fallbacks the CSS `var(--x, #hex)` carried).
pub mod palette {
    use super::Rgba;
    pub const AMBER: Rgba = Rgba::rgb(0xff, 0xb3, 0x40); // var(--amber, #ffb340)
    pub const GREEN: Rgba = Rgba::rgb(0x58, 0xdd, 0xab); // var(--green, #58ddab)
    pub const OBS_REC: Rgba = Rgba::rgb(0xff, 0x42, 0x3a); // OBS red dot and outline
    pub const OBS_LIVE: Rgba = Rgba::rgb(0xff, 0x6a, 0x3a); // OBS "LIVE" dot
    pub const OBS_LABEL: Rgba = Rgba::rgb(0xff, 0x6b, 0x63); // "REC" / "LIVE" text
    pub const VOICE: Rgba = Rgba::rgb(0xff, 0x5a, 0x4e); // dictation dot, level bars and outline
    pub const STOP_FILL: Rgba = Rgba(0xff, 0x43, 0x3f, 51); // red Stop button, rgba(255,67,63,.20)
    pub const STOP_FILL_HOVER: Rgba = Rgba(0xff, 0x43, 0x3f, 77); // .30
    pub const STOP_SQUARE: Rgba = Rgba::rgb(0xff, 0x43, 0x3f);
    pub const STOP_TEXT: Rgba = Rgba::rgb(0xf7, 0x6f, 0x6b);
    pub const REC_DOT: Rgba = Rgba::rgb(0xff, 0x43, 0x3f); // screen recording dot
    pub const PRIV_MIC: Rgba = Rgba::rgb(0xff, 0x9f, 0x0a); // microphone dot
    pub const PRIV_CAM: Rgba = Rgba::rgb(0x30, 0xd1, 0x58); // camera dot
    /// `var(--accent)` fallback; the renderer should prefer the theme's accent for [`super::Accent::Theme`].
    pub const THEME_ACCENT_FALLBACK: Rgba = Rgba::rgb(0x8f, 0x96, 0xff);
}

/// Outline / element accent: a concrete colour or "whatever the theme accent is".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Rgb(Rgba),
    Theme,
}

impl Accent {
    pub const AMBER: Accent = Accent::Rgb(palette::AMBER);
    pub const GREEN: Accent = Accent::Rgb(palette::GREEN);
}

/// Card tone (`data-tone`): tints the icon and the liquid outline while the card is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Green,
    Amber,
    Red,
    Accent,
}

/// Role colour of a glyph (resolved against the theme by the renderer).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorRole {
    Text,
    Dim,
    Accent,
    Amber,
    Green,
    Red,
    Rgb(Rgba),
}

// ---------------------------------------------------------------- animation

/// Animation kind. `Glide(ms)` is a value tween of that many milliseconds (bar/ring/height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anim {
    /// No motion (the end state at once).
    None,
    /// Fade + scale from .92 with the spring (.22 s fade, .4 s scale).
    Pop,
    /// Slide in from / out to the capsule edge.
    Slide,
    /// Opacity only (.22 s).
    Fade,
    /// Tween the changed values over this many ms.
    Glide(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowAnim {
    pub enter: Anim,
    pub update: Anim,
    pub leave: Anim,
}

impl RowAnim {
    pub const fn new(enter: Anim, update: Anim, leave: Anim) -> Self {
        RowAnim { enter, update, leave }
    }
}

impl Default for RowAnim {
    fn default() -> Self {
        RowAnim::new(Anim::Fade, Anim::None, Anim::Fade)
    }
}

// ---------------------------------------------------------------- vocabulary

/// Glyph names. [`IconName::as_str`] is the name the web UI's `Pill.icon()` used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconName {
    Area,
    Bell,
    Battery,
    Bolt,
    Calendar,
    Check,
    Clipboard,
    Copy,
    Download,
    File,
    Folder,
    Keyboard,
    Mail,
    Mic,
    Monitor,
    Mute,
    Next,
    Note2,
    Pause,
    Play,
    Plus,
    Prev,
    Qr,
    ScanText,
    Shelf,
    Sparkles,
    Terminal,
    Timer,
    Trash,
    Usb,
    Video,
    Volume,
    Warn,
    X,
}

impl IconName {
    pub fn as_str(self) -> &'static str {
        use IconName::*;
        match self {
            Area => "area",
            Bell => "bell",
            Battery => "battery",
            Bolt => "bolt",
            Calendar => "calendar",
            Check => "check",
            Clipboard => "clipboard",
            Copy => "copy",
            Download => "download",
            File => "file",
            Folder => "folder",
            Keyboard => "keyboard",
            Mail => "mail",
            Mic => "mic",
            Monitor => "monitor",
            Mute => "mute",
            Next => "next",
            Note2 => "note2",
            Pause => "pause",
            Play => "play",
            Plus => "plus",
            Prev => "prev",
            Qr => "qr",
            ScanText => "scantext",
            Shelf => "shelf",
            Sparkles => "sparkles",
            Terminal => "terminal",
            Timer => "timer",
            Trash => "trash",
            Usb => "usb",
            Video => "video",
            Volume => "volume",
            Warn => "warn",
            X => "x",
        }
    }
}

/// Named text styles (font size / weight / colour live in the renderer's theme).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    /// 14/600, +.04em tracking, tabular digits (`lv-code`: "RU", "42", "GPU 0 · 88°C").
    Code,
    /// 14/600 plain text (`lv-txt`, "Copied").
    Label,
    /// 13/500 dim (`lv-dim`, sizes, "+2").
    Dim,
    /// 13/500 text colour (agents status).
    Status,
    /// 13/500 amber (agents "needs your input").
    StatusAmber,
    /// 13/600 tabular text colour (OBS timer, recording timer).
    Clock,
    /// 12/600 +.04em, `OBS_LABEL` red ("REC", "LIVE").
    RecLabel,
    /// 13/700 tabular, 28 px min width (dictation timer).
    VoiceTimer,
    /// 13/600 ellipsis (media title).
    Name,
    /// 13/600 ellipsis (Steam game).
    Game,
    /// 13/600 accent (Steam percent).
    Percent,
    /// 11/500 dim (Steam speed).
    Small,
    /// 14/600 ellipsis (download file name).
    FileName,
    /// Volume number slot: 14/600 tabular, 3 chars wide, right aligned.
    VolNum,
    /// Same, dim (muted).
    VolNumMuted,
    /// 13 count (shelf chip).
    Count,
    /// 13/700 (timer chip digits).
    Metric,
}

/// What a button / card action does. Backend commands are the command names of docs/native-ui-contract.md.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Noop,
    /// Call a backend command with these arguments (camelCase keys as on the JS side).
    Invoke { cmd: String, args: Value },
    /// Put this text on the clipboard.
    CopyText(String),
    /// "Again" on the finished-timer card: start the timer with the last selected length.
    TimerAgain,
}

impl Action {
    pub fn invoke(cmd: &str, args: Value) -> Action {
        Action::Invoke { cmd: cmd.to_string(), args }
    }
    pub fn invoke0(cmd: &str) -> Action {
        Action::Invoke { cmd: cmd.to_string(), args: Value::Null }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ButtonStyle {
    /// 22 px round icon button (mini player).
    IconRound,
    /// Red Stop pill: `STOP_FILL` background (hover `STOP_FILL_HOVER`), red square + `STOP_TEXT` label.
    /// `width` px x 24 px, corner radius 12.
    Stop { width: f32 },
}

/// One element of a row, laid out left to right. Sizes are css px at 100 %.
#[derive(Clone, Debug, PartialEq)]
pub enum Elem {
    Text { text: String, style: TextStyle, max_w: Option<f32> },
    /// Live clock text. `since` is the epoch ms it counts from; `text` is its value when the row was built
    /// (the provider re-sets the row with the same key on its tick, so a renderer may simply show `text`).
    Clock { text: String, since: i64, style: TextStyle },
    Icon { name: IconName, size: f32, color: ColorRole },
    /// 4-band equaliser. `playing == false` draws the bands still at scaleY .45. `None` accent = theme accent.
    Eq { accent: Option<Accent>, playing: bool },
    /// Album cover, `size` px, 7 px corners. `id` changes with the picture (cross-fade .25 s); `url` is a data URL, `None` = placeholder.
    Cover { id: u64, url: Option<String>, size: f32 },
    /// Filled dot; `halo` draws the soft ring; `pulse_ms` breathes opacity 1 <-> .5 with that period.
    Dot { color: Rgba, size: f32, halo: bool, pulse_ms: Option<u32> },
    /// Rotating ring spinner (.8 s per turn).
    Spinner { size: f32 },
    /// Circle with a tick that draws itself.
    Check { size: f32, color: ColorRole },
    /// Progress ring; `value` 0..1, `None` = indeterminate (rotates). Value changes tween over `glide_ms`.
    Ring { value: Option<f32>, size: f32, glide_ms: u32 },
    /// Horizontal bar, `value` 0..1; `muted` draws it without the tween.
    Bar { value: f32, width: f32, height: f32, glide_ms: u32, muted: bool },
    /// Dictation level meter: one bar per entry, entry = scaleY 0.14..1 (tween 70 ms), `color`.
    Levels { bars: Vec<f32>, color: Rgba },
    /// Fixed gap.
    Spacer(f32),
    /// Flexible gap (pushes what follows to the right).
    Flex,
    /// Thin vertical separator.
    Sep,
    /// Elements sharing one baseline (Steam percent + speed).
    Group(Vec<Elem>),
    /// Small tag ("MP4" / "GIF").
    Badge { text: String },
    Button { action: Action, label: Option<String>, icon: Option<IconName>, style: ButtonStyle, tip: String },
}

impl Elem {
    pub fn text(s: impl Into<String>, style: TextStyle) -> Elem {
        Elem::Text { text: s.into(), style, max_w: None }
    }
    pub fn text_max(s: impl Into<String>, style: TextStyle, max_w: f32) -> Elem {
        Elem::Text { text: s.into(), style, max_w: Some(max_w) }
    }
    pub fn icon(name: IconName, size: f32) -> Elem {
        Elem::Icon { name, size, color: ColorRole::Text }
    }
    pub fn icon_c(name: IconName, size: f32, color: ColorRole) -> Elem {
        Elem::Icon { name, size, color }
    }
}

/// One live row of the collapsed capsule.
#[derive(Clone, Debug, PartialEq)]
pub struct RowSpec {
    pub id: String,
    pub priority: i32,
    pub ttl_ms: Option<u32>,
    pub key: String,
    pub accent: Option<Accent>,
    pub tall: bool,
    /// Minimum width in css px (`--lpw`); 0 = natural.
    pub width: f32,
    /// Tooltip (the `title` attribute).
    pub tip: Option<String>,
    pub elements: Vec<Elem>,
    pub anim: RowAnim,
}

impl RowSpec {
    pub fn new(id: &str, priority: i32) -> RowSpec {
        RowSpec {
            id: id.to_string(),
            priority,
            ttl_ms: None,
            key: String::new(),
            accent: None,
            tall: false,
            width: 0.0,
            tip: None,
            elements: Vec::new(),
            anim: RowAnim::default(),
        }
    }
    pub fn ttl(mut self, ms: u32) -> Self {
        self.ttl_ms = Some(ms);
        self
    }
    pub fn key(mut self, k: impl Into<String>) -> Self {
        self.key = k.into();
        self
    }
    pub fn accent(mut self, a: Accent) -> Self {
        self.accent = Some(a);
        self
    }
    pub fn tall(mut self) -> Self {
        self.tall = true;
        self
    }
    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }
    pub fn tip(mut self, t: impl Into<String>) -> Self {
        self.tip = Some(t.into());
        self
    }
    pub fn anim(mut self, enter: Anim, update: Anim, leave: Anim) -> Self {
        self.anim = RowAnim::new(enter, update, leave);
        self
    }
    pub fn with(mut self, e: Elem) -> Self {
        self.elements.push(e);
        self
    }
    pub fn elems(mut self, v: Vec<Elem>) -> Self {
        self.elements = v;
        self
    }
}

// ---------------------------------------------------------------- cards

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Normal,
    /// The emphasised (white) button.
    White,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CardAction {
    pub label: String,
    pub kind: ActionKind,
    pub action: Action,
}

impl CardAction {
    pub fn new(label: impl Into<String>, kind: ActionKind, action: Action) -> Self {
        CardAction { label: label.into(), kind, action }
    }
    /// A plain button that only dismisses the card ("Close" / "Dismiss" / "OK").
    pub fn close(label: impl Into<String>) -> Self {
        CardAction::new(label, ActionKind::Normal, Action::Noop)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CardIcon {
    Name(IconName),
    /// Avatar letter of a notification (first letter of the app, upper case).
    Letter(String),
    /// Animated check (circle + tick that draws itself).
    Check,
}

/// A colour-history swatch: click runs `action` (copy the hex).
#[derive(Clone, Debug, PartialEq)]
pub struct Swatch {
    pub hex: String,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CardBody {
    Text { title: String, sub: Option<String> },
    /// Windows notification: title <= `max_title_lines`, body <= `max_body_lines` (the renderer drops body lines first, then title lines, when the OS frame is short).
    Notification { title: String, body: Option<String>, app: String, max_title_lines: u8, max_body_lines: u8 },
    /// Picked colour: big swatch, `hex`, `rgb(..) · copied`, up to 9 recent colours.
    Color { hex: String, rgb: String, copied: String, history: Vec<Swatch> },
}

/// An event card. A card is dismissed after `ttl_ms` or when any of its buttons is pressed (the action runs first).
#[derive(Clone, Debug, PartialEq)]
pub struct CardSpec {
    pub id: String,
    pub icon: Option<CardIcon>,
    pub tone: Tone,
    pub ttl_ms: u32,
    /// Shown during quiet hours too.
    pub urgent: bool,
    /// A click outside the buttons runs the first action (and dismisses).
    pub click_first: bool,
    pub body: CardBody,
    pub actions: Vec<CardAction>,
    pub enter: Anim,
    pub leave: Anim,
}

impl CardSpec {
    pub fn new(id: impl Into<String>, ttl_ms: u32, body: CardBody) -> CardSpec {
        CardSpec {
            id: id.into(),
            icon: None,
            tone: Tone::Neutral,
            ttl_ms,
            urgent: false,
            click_first: false,
            body,
            actions: Vec::new(),
            enter: Anim::Slide,
            leave: Anim::Fade,
        }
    }
    /// The common "icon + title + sub" card.
    pub fn text(id: impl Into<String>, ttl_ms: u32, title: impl Into<String>, sub: Option<String>) -> CardSpec {
        CardSpec::new(id, ttl_ms, CardBody::Text { title: title.into(), sub })
    }
    pub fn icon(mut self, i: IconName) -> Self {
        self.icon = Some(CardIcon::Name(i));
        self
    }
    pub fn tone(mut self, t: Tone) -> Self {
        self.tone = t;
        self
    }
    pub fn actions(mut self, a: Vec<CardAction>) -> Self {
        self.actions = a;
        self
    }
    /// Quiet hours let through anything marked urgent and the timers / reminders the user set themselves
    /// (`id` starting with `timer` or `remind`).
    pub fn quiet_exempt(&self) -> bool {
        self.urgent || self.id.starts_with("timer") || self.id.starts_with("remind")
    }
}

// ---------------------------------------------------------------- commands

/// Edge reaction presets (`Pill.react`), with the web UI's resolver priorities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Preset {
    Attention,
    Thinking,
    File,
    Done,
    Music,
    Sleep,
    Calm,
}

impl Preset {
    /// Higher wins in the resolver.
    pub fn priority(self) -> u8 {
        match self {
            Preset::Attention => 6,
            Preset::Thinking => 5,
            Preset::File => 4,
            Preset::Done => 3,
            Preset::Music => 2,
            Preset::Sleep => 1,
            Preset::Calm => 0,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::Attention => "attention",
            Preset::Thinking => "thinking",
            Preset::File => "file",
            Preset::Done => "done",
            Preset::Music => "music",
            Preset::Sleep => "sleep",
            Preset::Calm => "calm",
        }
    }
}

/// A backend question whose answer goes back into the provider as an event named `as_event`
/// (the answer payload as-is, `Null` when the call failed), `delay_ms` after the query was issued.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub cmd: &'static str,
    pub args: Value,
    pub as_event: &'static str,
    pub delay_ms: u32,
}

impl Query {
    pub fn new(cmd: &'static str, as_event: &'static str) -> Query {
        Query { cmd, args: Value::Null, as_event, delay_ms: 0 }
    }
}

/// The record tile on Home turns into a red "Stop 0:NN" tile while a recording runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecTile {
    pub label: String,
    pub clock: String,
}

/// What a provider asks the shell/renderer to do.
#[derive(Clone, Debug, PartialEq)]
pub enum LiveCmd {
    SetRow(RowSpec),
    ClearRow(String),
    /// `Pill.react(source, preset, { ttlMs, color })`.
    React { source: String, preset: Preset, ttl_ms: Option<u32>, color: Option<Rgba> },
    Unreact(String),
    Toast(CardSpec),
    /// Liquid-edge pulse of this strength (0..1). Reduce motion is the shell's call.
    EdgePulse(f32),
    /// Tab badge (`Pill.setBadge`); 0 clears it.
    TabBadge { tab: String, count: u32 },
    /// Orange microphone / green camera dots in the corner of the capsule (camera first). Both false clears them.
    PrivacyDots { cam: bool, mic: bool },
    RecTile(Option<RecTile>),
    /// System-monitor meters for the top of the Controls tab.
    SysMeters(MetersView),
    /// Run a button's action without a click (wheel volume, hide/show notifications to the backend).
    Run(Action),
    Query(Query),
    /// An in-app event for other panels (`voice:state`, `voice:done`).
    Broadcast { name: String, payload: Value },
}

impl LiveCmd {
    pub fn set(r: RowSpec) -> LiveCmd {
        LiveCmd::SetRow(r)
    }
    pub fn clear(id: &str) -> LiveCmd {
        LiveCmd::ClearRow(id.to_string())
    }
    pub fn react(source: &str, preset: Preset, ttl_ms: Option<u32>, color: Option<Rgba>) -> LiveCmd {
        LiveCmd::React { source: source.to_string(), preset, ttl_ms, color }
    }
    pub fn unreact(source: &str) -> LiveCmd {
        LiveCmd::Unreact(source.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(Rgba::from_hex("#ff423a"), Some(Rgba(255, 66, 58, 255)));
        assert_eq!(Rgba::from_hex("#f80"), Some(Rgba(255, 136, 0, 255)));
        assert_eq!(Rgba::from_hex("#11223344"), Some(Rgba(0x11, 0x22, 0x33, 0x44)));
        assert_eq!(Rgba::from_hex("ff423a"), None);
        assert_eq!(Rgba::from_hex("#ff42"), None);
        assert_eq!(Rgba::from_hex("#zzzzzz"), None);
        assert_eq!(Rgba::from_hex("#ёёёё"), None); // non-ASCII must not panic on slicing
        assert_eq!(Rgba(1, 2, 3, 255).to_hex(), "#010203");
        assert_eq!(Rgba(1, 2, 3, 4).to_hex(), "#01020304");
    }

    #[test]
    fn quiet_exempt_ids() {
        assert!(CardSpec::text("timer-done", 1, "t", None).quiet_exempt());
        assert!(CardSpec::text("remind-7", 1, "t", None).quiet_exempt());
        assert!(!CardSpec::text("uninstall:x", 1, "t", None).quiet_exempt());
        let mut c = CardSpec::text("battery", 1, "t", None);
        c.urgent = true;
        assert!(c.quiet_exempt());
    }

    #[test]
    fn preset_priorities_match_the_resolver() {
        assert!(Preset::Attention.priority() > Preset::Thinking.priority());
        assert!(Preset::Thinking.priority() > Preset::File.priority());
        assert!(Preset::File.priority() > Preset::Done.priority());
        assert!(Preset::Done.priority() > Preset::Music.priority());
        assert_eq!(Preset::Calm.priority(), 0);
        assert_eq!(Preset::Music.as_str(), "music");
    }

    #[test]
    fn row_builder_defaults() {
        let r = RowSpec::new("x", 5).ttl(10).key("k").width(20.0).with(Elem::Flex);
        assert_eq!((r.priority, r.ttl_ms, r.key.as_str(), r.width, r.elements.len()), (5, Some(10), "k", 20.0, 1));
        assert_eq!(r.anim, RowAnim::default());
        assert_eq!(IconName::ScanText.as_str(), "scantext");
    }
}
