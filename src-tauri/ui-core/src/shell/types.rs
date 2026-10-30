//! Public input/output types of the shell model: [`Event`] in, [`Notice`] out (to sibling modules), [`View`] for the renderer.
//!
//! Data flow per frame: the host feeds events (`Shell::handle`) and calls `Shell::tick(dt_ms, now_ms)` once per display frame, then draws
//! `Shell::view()`. Commands for the backend (`pill_rect`, `pill_set_state`, ...) leave through the `Backend` trait as they happen;
//! cross-module signals (reaction changed, tab shown, card button pressed, language changed) are queued as [`Notice`]s
//! (`Shell::take_notices`). All geometry in [`View`] is in window css px at the current scale unless a field says "100 %".

use serde_json::Value;

use crate::shell::cards::CardView;
use crate::shell::clock::{CharSide, ClockRow, DigitRoll};
use crate::shell::color::{Palette, Rgba, Theme};
use crate::shell::liquid::{Bands, EdgeView};
use crate::shell::motion::AnimSpec;
use crate::shell::react::Preset;
use crate::shell::settings::{Lang, Shape};

/// Window / environment facts the renderer owns (the `window.innerWidth`, `devicePixelRatio`, media queries of the JS).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Env {
    /// width of the pill window (css px)
    pub inner_w: f64,
    pub dpr: f64,
    /// `prefers-color-scheme: light`
    pub os_prefers_light: bool,
    /// `prefers-reduced-motion: reduce`
    pub os_reduce_motion: bool,
    /// `document.hidden`: the window is occluded / minimised; frames are paused
    pub occluded: bool,
}

impl Default for Env {
    fn default() -> Self {
        Env { inner_w: 720.0, dpr: 1.0, os_prefers_light: false, os_reduce_motion: false, occluded: false }
    }
}

/// Rect in window css px.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Everything that can happen TO the shell (backend events, pointer/keyboard, renderer measurements).
#[derive(Debug, Clone)]
pub enum Event {
    /// `settings_get` result at startup (also applies `lastTab`, starts weather / battery polling).
    Start(Value),
    /// `settings:changed` payload (the full settings object).
    SettingsChanged(Value),
    /// `pill:show` / `pill:hide`
    Show,
    Hide,
    /// `pill:expand { tab, section }`
    Expand { tab: Option<String>, section: Option<String> },
    /// `pill:open-tab { tab, section }` (only `settings` is handled here)
    OpenTab { tab: Option<String>, section: Option<String> },
    /// `pill:collapse`
    Collapse,
    /// `pill:hover` / `pill:leave` (from the backend cursor poller)
    Hover,
    Leave,
    /// `pill:cursor` / mousemove: window-relative css px; `None` = cursor gone
    Cursor(Option<(f64, f64)>),
    /// `audio:bands`
    AudioBands(Bands),
    /// `weather:current`
    WeatherCurrent(Value),
    /// results of fire-and-forget commands
    BatteryState(Value),
    WeatherGet(Value),
    /// `settings_open` finished (`true` = the window opened)
    SettingsOpened(bool),
    /// `pill_desktop_fg` answer
    DesktopFg(bool),
    /// a text field gained / lost keyboard focus
    TextFocus(bool),
    /// Escape key
    Escape,
    /// mouse down on the stage outside the body
    BackgroundClick,
    /// the pointer left the window
    MouseLeave,
    /// the global hotkey (backend toggles through `pill_toggle`)
    Hotkey,
    /// header buttons
    SettingsButton,
    PinButton,
    DictButton,
    /// tab button pressed
    TabClicked(String),
    /// a card action button was pressed (index into the showing card's actions)
    CardAction(usize),
    /// renderer facts
    Env(Env),
    /// measured size of the live row content (screen css px incl. scale), from the layout pass / ResizeObserver
    LiveMeasured { w: f64, h: f64 },
    /// characters.js: the side the pet / eyes stand on and whether the legs style is active
    Character { side: CharSide, legs: bool },
}

/// Cross-module signals produced by the shell.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// `Pill.onReaction` observers: the effective reaction changed (or a fresh event of the same preset)
    Reaction { name: Preset, color: String },
    /// `pill:live-changed`: the clock row was rebuilt (characters re-measure where the eyes stand)
    LiveChanged,
    /// tab lifecycle (`onShow` / `onHide` of the tab module)
    TabShown(String),
    TabHidden(String),
    /// a tab was registered: the panel module should mount its view
    TabMounted(String),
    /// a card action button was pressed
    CardAction { card_id: Option<String>, action_id: String },
    /// the language changed (the JS reloaded the page)
    LangChanged(Lang),
    /// blur the focused element when the island collapses
    Blur,
    /// the dictation header button was pressed and `set_dictate_available(true)`
    Dictate,
}

/// High-level state for tests and the renderer ("what is the capsule doing").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// the window is hidden by the backend (guard / tray): no timers, no animation
    Hidden,
    /// auto-hide: the body slid up
    AutoHidden,
    Collapsed,
    /// the pointer is on the collapsed pill (hover intent pending)
    Hover,
    /// expanded state, the springs (or the frame wait) still run
    Expanding,
    Expanded,
    /// collapsed state, the OS frame is still the expanded one while the springs run
    Collapsing,
    /// an event card owns the island
    Cards,
}

/// Direction of the tab-switch slide (`#views[data-dir]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabDir {
    #[default]
    Right,
    Left,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyView {
    /// left edge on whole device pixels, `snapPx((innerWidth - w) / 2)`
    pub left: f64,
    /// vertical offset (the `translateY` of `#body`: floating offsets, auto-hide slide), mirrored transition
    pub top: f64,
    pub width: f64,
    pub height: f64,
    /// `border-radius: top top bottom bottom`
    pub radius_top: f64,
    pub radius_bottom: f64,
    /// fill colour with the folded / open transparency as alpha
    pub fill: Rgba,
    /// concave flares at the top corners (notch shape, collapsed only); radius `14 * scale`
    pub flare_radius: f64,
    pub flare_visible: bool,
    /// light theme drop shadow (`--shadow`)
    pub shadow: bool,
    /// the body slid away (auto-hide)
    pub away: bool,
    /// `#panel` left offset inside the body
    pub panel_left: f64,
}

/// `--shadow` of the light theme: (dx, dy, blur, alpha) layers.
pub const LIGHT_SHADOW: [(f64, f64, f64, f64); 2] = [(0.0, 10.0, 30.0, 0.22), (0.0, 2.0, 8.0, 0.14)];

/// What the live row shows.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveContent {
    None,
    Clock { row: ClockRow, time: Option<String> },
    Slot { id: String, key: String, spec: Value },
    Card(CardView),
}

/// The previous content, fading out under the new one for ~90 ms (`liveGhost`).
#[derive(Debug, Clone, PartialEq)]
pub struct Ghost {
    pub content: LiveContent,
    pub age_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiveView {
    /// `#live` opacity: 0 while expanded (150 ms fade)
    pub visible: bool,
    pub content: LiveContent,
    /// natural size instead of the fixed capsule height (`.tall`)
    pub tall: bool,
    /// bumps each time the content swaps: play `motion::LIVE_SWAP` on the new content
    pub swap_epoch: u64,
    pub swap_age_ms: f64,
    pub ghost: Option<Ghost>,
    /// digits rolling in place (clock): (roll, age since start)
    pub rolls: Vec<(DigitRoll, f64)>,
    /// side-column items popping in after a reshape: (is_right, index, age); play `motion::RESHAPE_POP`
    pub entering: Vec<(bool, usize, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelView {
    pub visible: bool,
    /// tab body height in css px at 100 % (`panelEl.style.height`)
    pub height: f64,
    /// when the panel became visible (play `motion::PANEL_IN` from there)
    pub shown_age_ms: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabView {
    pub id: String,
    pub title: String,
    pub order: i32,
    pub hidden: bool,
    pub off: bool,
    pub active: bool,
    pub badge: u32,
}

/// Header cluster state.
#[derive(Debug, Clone, PartialEq)]
pub struct ChromeView {
    pub vpn: String,
    pub pinned: bool,
    /// the settings button looks pressed while the in-pill settings tab is current
    pub settings_current: bool,
}

/// Placement of the liquid-edge strip: centred on the body, top at `body_bottom - up * scale`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StripRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One complete frame description for the renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub phase: Phase,
    pub scale: f64,
    pub theme: Theme,
    pub palette: Palette,
    pub shape: Shape,
    /// reduced motion is on (setting or OS): play no animation, show end states
    pub reduced: bool,
    pub lang: Lang,
    /// OS window frame the backend should have (css px at 100 %)
    pub frame: (f64, f64),
    pub body: BodyView,
    pub live: LiveView,
    pub panel: PanelView,
    pub edge: EdgeView,
    pub strip: StripRect,
    pub tabs: Vec<TabView>,
    pub tab_dir: TabDir,
    pub current_tab: Option<String>,
    pub chrome: ChromeView,
    /// `--wave-c`: accent of the running live widget / card tone
    pub wave_color: Option<Rgba>,
    /// the interactive region last reported with `pill_rect`
    pub hit_rect: Rect,
    /// entering animation of the panel content
    pub panel_anim: AnimSpec,
    /// cap of the shared frame loop (0 = display rate)
    pub fps_cap: u32,
    /// the loop wants frames now
    pub wants_frames: bool,
}
