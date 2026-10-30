//! The native window host: ONE dedicated UI thread that owns every native Pill window, runs the Win32 message loop and renders views.
//! See `docs/native-host.md` for the long form; this is the map.
//!
//! * `ui::view` defines WHAT a surface is (`View`, `Cx`). This module defines WHERE it lives: [`NativeWindow`] builds a [`WindowSpec`] of one
//!   of four [`WindowKind`]s and [`HostHandle::spawn`] / [`HostHandle::register`] put a view into it.
//! * Everything is created, driven and destroyed on the UI thread. Any other thread talks to it through [`HostHandle`] (commands) and
//!   [`Waker`](crate::ui::view::Waker) (messages to a view); both are lock-light mailboxes drained by the loop, never direct calls.
//! * Rendering: DirectComposition flip-model swap chain per window, vsync-paced at the display's refresh rate, only while a view asks for
//!   frames (`present.rs`, `sched.rs`). Idle windows cost nothing.
//! * Pure, unit-tested pieces: `sched` (frame scheduler), `geom` (DPI, hit region, anchoring), `proximity` (the island's cursor logic),
//!   `focus` (blur / watchdog policies), `ui::view` (queue, Cx). `testing` renders and drives views headlessly on WARP.
//!
//! # Lifecycle and `ExitRequested`
//! `main.rs` already answers `RunEvent::ExitRequested { code: None }` with `prevent_exit()`, so closing the last window never ends the
//! process; tray "Quit" calls `app.exit(0)` (a `Some(code)` request) which does end it. Native windows are invisible to Tauri: they neither
//! keep the app alive nor block its exit. The host thread is a daemon: on `app.exit` call [`shutdown`] first (destroys the windows and
//! stops the loop, joins within 2 s) or just let the process end. [`HostHandle::open_windows`] says how many native windows are alive, for
//! a future "exit when the last window is gone" rule.
pub mod focus;
pub mod geom;
pub mod pillwin;
pub mod proximity;
pub mod sched;
pub mod testing;

mod dropt;
mod present;
mod win32;

#[cfg(test)]
mod demo;

#[allow(unused_imports)] // re-exported for the views that arrive later
pub use geom::{HitRegion, IRect};

use crate::ui::view::{Api, Cx, Msg, MsgQueue, NullApi, Services, View, Waker, WindowId};
use glasskit::lang::Lang;
use glasskit::theme::Theme;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

// ---------------------------------------------------------------------------------------------------------------------------------------
// What a window is
// ---------------------------------------------------------------------------------------------------------------------------------------

/// The four shapes of native window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    /// The island: per-pixel-alpha, topmost, never activated, tool window (no taskbar button). Click-through outside the view's hit region
    /// (see `proximity`). Takes the keyboard only when the view asks ([`Cx::set_focus`]).
    Overlay,
    /// The command bar: per-pixel-alpha topmost popup, no taskbar button, activatable on demand: it takes the foreground when shown and can
    /// hide itself when the user clicks away (`hide_on_blur`).
    Popup,
    /// Settings: an opaque top-level window. Chrome decision: the view draws its own title bar (as the HTML page does today) over a
    /// borderless-but-system-managed window: `WS_THICKFRAME` stays, so Aero Snap, DWM shadow, rounded corners on Win11 and the maximize /
    /// minimize animations work; the host answers `WM_NCCALCSIZE` (client = whole window) and `WM_NCHITTEST` (resize edges + the view's
    /// caption region -> `HTCAPTION`). Minimum size and remembered geometry are host features.
    Normal,
    /// One window that covers exactly one monitor (capture): topmost, opaque, crosshair cursor, takes the keyboard when shown, optional
    /// watchdog that closes it when it stays out of the foreground.
    Fullscreen,
}

/// Which monitor. `Index` counts in the order of [`HostHandle::monitors`]: 0 = the primary, then the others left to right.
#[derive(Clone, Debug, PartialEq)]
pub enum MonitorSel {
    Primary,
    Index(usize),
    /// The monitor under the mouse cursor at the moment of the call (re-evaluated at every show).
    UnderCursor,
    /// The monitor with exactly this rectangle (what [`HostHandle::monitors`] returned).
    Rect(IRect),
}

/// Where a window goes when it is created / shown.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// Centred horizontally at the top of a monitor (island, bar). `below_taskbar`: start at the work-area top instead of the monitor top.
    /// `margin_top` is logical px below that; `offset_pct` (-50..50) shifts the centre by that percentage of the monitor width.
    TopCenter { monitor: MonitorSel, offset_pct: f32, below_taskbar: bool, margin_top: f32 },
    /// Centred on a monitor.
    Center(MonitorSel),
    /// Exactly here (screen px, top-left).
    At { x: i32, y: i32 },
    /// A remembered geometry (Settings); falls back to centring when it is no longer reachable on any monitor.
    Restore(Geometry),
    /// Cover the whole monitor (Fullscreen).
    Cover(MonitorSel),
}

/// Remembered placement of a Normal window: position in screen px, size in LOGICAL px (so it survives a DPI change), maximized flag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideReason {
    /// `hide` / `toggle` / `Cx::hide_self`
    Requested,
    /// `hide_on_blur`: the user clicked away.
    Blur,
    /// The focus watchdog gave up on a Fullscreen window.
    FocusLost,
}

/// What the window tells its owner (`WindowSpec::on_event`). Runs ON the UI thread: keep it to a flag flip or a channel send.
#[derive(Clone, Debug, PartialEq)]
pub enum WinEvent {
    Created,
    Shown,
    Hidden(HideReason),
    /// The window was destroyed (`close`, `Cx::close_self`, the user closing it).
    Closed,
    /// A Normal window moved / resized (debounced 700 ms) or was maximized: store it for [`Placement::Restore`].
    Geometry(Geometry),
    /// The window lost activation (before `hide_on_blur` acts).
    Blur,
}

pub type EventHook = Arc<dyn Fn(WindowId, WinEvent) + Send + Sync>;

/// Builds the view on the UI thread. Gets the context of its window and the creation arguments (`Value::Null` for `spawn`).
pub type Factory = Box<dyn FnOnce(&mut Cx, Value) -> Box<dyn View> + Send>;
type RegFactory = Arc<dyn Fn(&mut Cx, Value) -> Box<dyn View> + Send + Sync>;

/// Everything the host needs to make a window. Build it with [`NativeWindow`].
#[derive(Clone)]
pub struct WindowSpec {
    pub kind: WindowKind,
    /// Becomes `Cx::caller` and the key of singleton windows.
    pub label: String,
    /// Logical px. Also the first answer when the view has no `size_hint`.
    pub size: (f32, f32),
    pub min_size: Option<(f32, f32)>,
    pub placement: Placement,
    /// Show right after creation.
    pub visible: bool,
    /// Take the foreground when shown (Popup default, Fullscreen default).
    pub activate_on_show: bool,
    /// Popup: hide when activation is lost (400 ms grace after a show, see `focus::BlurGuard`).
    pub hide_on_blur: bool,
    /// Fullscreen: close the window when it has been out of the foreground this long (ms).
    pub focus_watchdog_ms: Option<f64>,
    /// Register as an OLE drop target; `View::on_drop` is called.
    pub accept_drops: bool,
    /// `WDA_EXCLUDEFROMCAPTURE`: the window is not part of screenshots / recordings.
    pub exclude_from_capture: bool,
    pub topmost: bool,
    pub resizable: bool,
    /// Explicit frame-rate cap; `None` = the display's refresh rate (the default, and what everything should use).
    pub fps_cap: Option<u32>,
    pub title: String,
    pub theme: Theme,
    /// Background the canvas is cleared to before `draw`: transparent for Overlay / Popup, opaque for Normal / Fullscreen.
    pub clear: [f32; 4],
    /// At most one window with this label; opening it again focuses the old one and sends `Msg::Open(args)`.
    pub singleton: bool,
    pub on_event: Option<EventHook>,
}

impl std::fmt::Debug for WindowSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowSpec").field("kind", &self.kind).field("label", &self.label).field("size", &self.size).field("placement", &self.placement).finish()
    }
}

/// Builder for a [`WindowSpec`]. Start from the constructor of the kind; the defaults are the island / bar / settings / capture window shapes.
///
/// ```ignore
/// let id = NativeWindow::popup("bar")
///     .size(560.0, 350.0)
///     .on_event(|_, e| if e == WinEvent::Blur { /* ... */ })
///     .spawn(&host::start(), |cx| Box::new(BarView::new(cx)));
/// ```
#[derive(Clone)]
pub struct NativeWindow {
    spec: WindowSpec,
}

impl NativeWindow {
    fn base(kind: WindowKind, label: &str) -> WindowSpec {
        WindowSpec {
            kind,
            label: label.to_string(),
            size: (560.0, 350.0),
            min_size: None,
            placement: Placement::Center(MonitorSel::Primary),
            visible: true,
            activate_on_show: false,
            hide_on_blur: false,
            focus_watchdog_ms: None,
            accept_drops: false,
            exclude_from_capture: false,
            topmost: false,
            resizable: false,
            fps_cap: None,
            title: "Pill".to_string(),
            theme: Theme::pill_dark(),
            clear: [0.0, 0.0, 0.0, 0.0],
            singleton: false,
            on_event: None,
        }
    }

    /// The island: 720 x 120 at the top centre of the primary monitor (below a top taskbar), topmost, no-activate.
    pub fn overlay(label: &str) -> NativeWindow {
        NativeWindow {
            spec: WindowSpec {
                size: (720.0, 120.0),
                title: "pill-main".to_string(), // the island window's title: tools that match windows by title keep working
                placement: Placement::TopCenter { monitor: MonitorSel::Primary, offset_pct: 0.0, below_taskbar: true, margin_top: 0.0 },
                topmost: true,
                singleton: true,
                ..Self::base(WindowKind::Overlay, label)
            },
        }
    }

    /// The command bar: 560 x 350 hanging 7 px below the top of the monitor under the cursor, hidden until shown, hides on blur.
    pub fn popup(label: &str) -> NativeWindow {
        NativeWindow {
            spec: WindowSpec {
                size: (560.0, 350.0),
                title: "pill-bar".to_string(),
                placement: Placement::TopCenter { monitor: MonitorSel::UnderCursor, offset_pct: 0.0, below_taskbar: false, margin_top: 7.0 },
                visible: false,
                activate_on_show: true,
                hide_on_blur: true,
                topmost: true,
                singleton: true,
                ..Self::base(WindowKind::Popup, label)
            },
        }
    }

    /// Settings: 840 x 760, minimum 700 x 520, centred, resizable, opaque (`#111013`).
    pub fn normal(label: &str) -> NativeWindow {
        NativeWindow {
            spec: WindowSpec {
                size: (840.0, 760.0),
                min_size: Some((700.0, 520.0)),
                activate_on_show: true,
                resizable: true,
                singleton: true,
                clear: [17.0 / 255.0, 16.0 / 255.0, 19.0 / 255.0, 1.0],
                ..Self::base(WindowKind::Normal, label)
            },
        }
    }

    /// A window covering exactly `monitor`: topmost, opaque black, hidden from screen capture, closes itself 1.5 s after losing the foreground.
    pub fn fullscreen(label: &str, monitor: MonitorSel) -> NativeWindow {
        NativeWindow {
            spec: WindowSpec {
                placement: Placement::Cover(monitor),
                title: "pill-capture".to_string(),
                topmost: true,
                activate_on_show: true,
                exclude_from_capture: true,
                focus_watchdog_ms: Some(1500.0),
                clear: [0.0, 0.0, 0.0, 1.0],
                ..Self::base(WindowKind::Fullscreen, label)
            },
        }
    }

    pub fn size(mut self, w: f32, h: f32) -> Self {
        self.spec.size = (w, h);
        self
    }
    pub fn min_size(mut self, w: f32, h: f32) -> Self {
        self.spec.min_size = Some((w, h));
        self
    }
    pub fn placement(mut self, p: Placement) -> Self {
        self.spec.placement = p;
        self
    }
    /// Remember/restore: shorthand for `placement(Placement::Restore(g))`.
    pub fn restore(self, g: Geometry) -> Self {
        self.placement(Placement::Restore(g))
    }
    /// Create hidden; show with [`HostHandle::show`].
    pub fn hidden(mut self) -> Self {
        self.spec.visible = false;
        self
    }
    pub fn activate_on_show(mut self, on: bool) -> Self {
        self.spec.activate_on_show = on;
        self
    }
    pub fn hide_on_blur(mut self, on: bool) -> Self {
        self.spec.hide_on_blur = on;
        self
    }
    pub fn focus_watchdog_ms(mut self, ms: Option<f64>) -> Self {
        self.spec.focus_watchdog_ms = ms;
        self
    }
    pub fn accept_drops(mut self) -> Self {
        self.spec.accept_drops = true;
        self
    }
    pub fn exclude_from_capture(mut self, on: bool) -> Self {
        self.spec.exclude_from_capture = on;
        self
    }
    pub fn topmost(mut self, on: bool) -> Self {
        self.spec.topmost = on;
        self
    }
    pub fn resizable(mut self, on: bool) -> Self {
        self.spec.resizable = on;
        self
    }
    /// Only for a cap the USER chose. The default (none) paces to the display.
    pub fn fps_cap(mut self, fps: Option<u32>) -> Self {
        self.spec.fps_cap = fps;
        self
    }
    pub fn title(mut self, t: &str) -> Self {
        self.spec.title = t.to_string();
        self
    }
    pub fn theme(mut self, t: Theme) -> Self {
        self.spec.theme = t;
        self
    }
    /// Straight RGBA the canvas is cleared to before `draw`.
    pub fn clear(mut self, rgba: [f32; 4]) -> Self {
        self.spec.clear = rgba;
        self
    }
    pub fn singleton(mut self, on: bool) -> Self {
        self.spec.singleton = on;
        self
    }
    pub fn on_event(mut self, f: impl Fn(WindowId, WinEvent) + Send + Sync + 'static) -> Self {
        self.spec.on_event = Some(Arc::new(f));
        self
    }

    pub fn into_spec(self) -> WindowSpec {
        self.spec
    }

    /// Creates the window on the UI thread; `factory` builds the view there. Returns at once with the id; the window appears a moment later.
    pub fn spawn(self, host: &HostHandle, factory: impl FnOnce(&mut Cx) -> Box<dyn View> + Send + 'static) -> WindowId {
        host.spawn_with(self.spec, Value::Null, Box::new(move |cx, _| factory(cx)))
    }

    /// Registers the window under `name` for [`Cx::open_window`] / [`HostHandle::open`]; nothing is created yet.
    pub fn register(self, host: &HostHandle, name: &str, factory: impl Fn(&mut Cx, Value) -> Box<dyn View> + Send + Sync + 'static) {
        host.0.registry.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), (self.spec, Arc::new(factory)));
    }
}

/// One monitor as the host sees it (physical px).
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorInfo {
    pub rect: IRect,
    pub work: IRect,
    pub primary: bool,
    pub scale: f32,
    pub refresh_hz: u32,
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Commands (any thread -> UI thread)
// ---------------------------------------------------------------------------------------------------------------------------------------

/// A command for the UI thread; travels through the same `MsgQueue` as view messages (so `spawn` then `post` keep their order).
pub(crate) enum Cmd {
    Create { id: WindowId, spec: Box<WindowSpec>, factory: Factory, args: Value },
    Open { name: String, args: Value },
    Show { id: WindowId, below: Option<i32> },
    Hide(WindowId),
    Toggle { id: WindowId, below: Option<i32> },
    Close(WindowId),
    SetBounds { id: WindowId, rect: IRect },
    Relayout(WindowId),
    ForceIgnore { id: WindowId, ignore: Option<bool> },
    SetTheme { id: WindowId, theme: Theme },
    Frame(WindowId),
    Activate { id: WindowId, on: bool },
    Shutdown,
}

#[derive(Default)]
pub(crate) struct WinInfo {
    pub hwnd: isize,
    pub visible: bool,
    /// DirectComposition (true) or the layered-window fallback (false)
    pub comp: bool,
}

pub(crate) struct Shared {
    pub next_id: AtomicU64,
    pub queue: Arc<MsgQueue>,
    pub registry: Mutex<HashMap<String, (WindowSpec, RegFactory)>>,
    pub windows: Mutex<HashMap<WindowId, WinInfo>>,
    pub msg_hwnd: Arc<AtomicIsize>,
    pub open: AtomicUsize,
    pub lang_ru: std::sync::atomic::AtomicBool,
    pub thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// A cheap, thread-safe handle to the host. Clone it freely.
#[derive(Clone)]
pub struct HostHandle(pub(crate) Arc<Shared>);

static HOST: OnceLock<HostHandle> = OnceLock::new();
static API: RwLock<Option<Arc<dyn Api>>> = RwLock::new(None);

/// Wires the backend facade (`ui::api`): from now on `Cx::call` / `call_async` of every view reach it. Callable at any time, from any thread.
pub fn set_api(api: Arc<dyn Api>) {
    *API.write().unwrap_or_else(|e| e.into_inner()) = Some(api);
}

struct GlobalApi;

impl Api for GlobalApi {
    fn call(&self, caller: &str, name: &str, args: Value) -> Result<Value, String> {
        let api = API.read().unwrap_or_else(|e| e.into_inner()).clone();
        match api {
            Some(a) => a.call(caller, name, args),
            None => NullApi.call(caller, name, args),
        }
    }
}

struct Win32Clipboard;

impl crate::ui::view::Clipboard for Win32Clipboard {
    fn get_text(&self) -> Option<String> {
        use glasskit::edit::clip::Clipboard;
        glasskit::edit::clip::WinClipboard.get_text()
    }
    fn set_text(&self, text: &str) {
        use glasskit::edit::clip::Clipboard;
        glasskit::edit::clip::WinClipboard.set_text(text)
    }
}

pub(crate) fn real_services() -> Services {
    Services {
        api: Arc::new(GlobalApi),
        bus: Arc::new(crate::ui::view::GlobalBus),
        clipboard: Arc::new(Win32Clipboard),
        spawn: Arc::new(crate::ui::view::PoolSpawn),
    }
}

/// Starts the host thread on first call (idempotent) and returns its handle.
pub fn start() -> HostHandle {
    HOST.get_or_init(win32::start).clone()
}

/// The handle if the host was started.
pub fn handle() -> Option<HostHandle> {
    HOST.get().cloned()
}

/// Destroys every native window and stops the UI thread (waits up to 2 s). Safe to call when the host never started.
pub fn shutdown() {
    if let Some(h) = HOST.get() {
        h.send(Cmd::Shutdown);
        let t = h.0.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(t) = t {
            let start = std::time::Instant::now();
            while !t.is_finished() && start.elapsed() < std::time::Duration::from_secs(2) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }
}

/// The app setting `reduceMotion`. Motion is reduced when EITHER this is on OR Windows' "show animations" is off (OS value read live), so
/// turning the app setting off hands the decision back to the OS. Views see the result as `Cx::reduced_motion`; every glasskit `Spring` /
/// `Tween` follows the same flag and jumps to its end state.
pub fn set_reduce_motion_setting(on: bool) {
    glasskit::ease::force_reduced(on.then_some(true));
}

/// Bundled fonts for all native text (default: the system fonts). Call before the first window is created.
pub fn set_fonts(f: &'static glasskit::fonts::FontSet) {
    win32::set_fonts(f);
}

impl HostHandle {
    pub(crate) fn send(&self, cmd: Cmd) {
        self.0.queue.push(WindowId(0), Msg::Custom(Box::new(cmd)));
    }

    /// Creates a window from a ready spec (the builder's `spawn` is the usual way).
    pub fn spawn_with(&self, spec: WindowSpec, args: Value, factory: Factory) -> WindowId {
        let id = WindowId(self.0.next_id.fetch_add(1, Ordering::Relaxed));
        self.send(Cmd::Create { id, spec: Box::new(spec), factory, args });
        id
    }

    /// Opens a registered window (see [`NativeWindow::register`]); a singleton that is already open is shown, focused and told
    /// `Msg::Open(args)`.
    pub fn open(&self, name: &str, args: Value) {
        self.send(Cmd::Open { name: name.to_string(), args });
    }

    /// Show (and, per spec, activate) the window; Popup / Overlay are re-placed first (monitor under the cursor, taskbar, ...).
    pub fn show(&self, id: WindowId) {
        self.send(Cmd::Show { id, below: None });
    }

    /// Like `show`, but a Popup is placed no higher than physical screen y `below` (the command bar hangs under the island).
    pub fn show_below(&self, id: WindowId, below: i32) {
        self.send(Cmd::Show { id, below: Some(below) });
    }

    pub fn hide(&self, id: WindowId) {
        self.send(Cmd::Hide(id));
    }

    /// Hide when visible, else show. A show right after a blur-hide is swallowed (the click that hid the bar must not re-open it).
    pub fn toggle(&self, id: WindowId) {
        self.send(Cmd::Toggle { id, below: None });
    }

    pub fn toggle_below(&self, id: WindowId, below: i32) {
        self.send(Cmd::Toggle { id, below: Some(below) });
    }

    pub fn close(&self, id: WindowId) {
        self.send(Cmd::Close(id));
    }

    /// Moves / resizes to an exact rectangle (screen, physical px).
    pub fn set_bounds(&self, id: WindowId, rect: IRect) {
        self.send(Cmd::SetBounds { id, rect });
    }

    /// Re-applies the placement rules (monitor, taskbar, DPI): the island's equivalent of `pill::place`.
    pub fn relayout(&self, id: WindowId) {
        self.send(Cmd::Relayout(id));
    }

    /// Forces the Overlay click-through state (`Some`), or hands it back to the proximity engine (`None`).
    pub fn force_ignore(&self, id: WindowId, ignore: Option<bool>) {
        self.send(Cmd::ForceIgnore { id, ignore });
    }

    pub fn set_theme(&self, id: WindowId, theme: Theme) {
        self.send(Cmd::SetTheme { id, theme });
    }

    /// Asks the window to render a frame (e.g. after external state changed).
    pub fn request_frame(&self, id: WindowId) {
        self.send(Cmd::Frame(id));
    }

    /// Take (`true`) or release (`false`) the keyboard foreground for the window.
    pub fn activate(&self, id: WindowId, on: bool) {
        self.send(Cmd::Activate { id, on });
    }

    /// A `Send + Sync` handle posting messages to the view of `id`.
    pub fn waker(&self, id: WindowId) -> Waker {
        Waker::new(id, self.0.queue.clone())
    }

    pub fn post(&self, id: WindowId, msg: Msg) {
        self.waker(id).post(msg);
    }

    /// UI language of every view's `Cx::lang`.
    pub fn set_lang(&self, lang: Lang) {
        self.0.lang_ru.store(lang == Lang::Ru, Ordering::Relaxed);
    }

    /// The window handle once the window exists (`None` before creation or after close).
    pub fn hwnd(&self, id: WindowId) -> Option<isize> {
        self.0.windows.lock().unwrap_or_else(|e| e.into_inner()).get(&id).map(|w| w.hwnd).filter(|h| *h != 0)
    }

    /// `"composition"` or `"layered"`: how this window puts pixels on screen (the lead's diagnostic for `PILL_FORCE_LAYERED`).
    pub fn backend(&self, id: WindowId) -> Option<&'static str> {
        self.0.windows.lock().unwrap_or_else(|e| e.into_inner()).get(&id).map(|w| if w.comp { "composition" } else { "layered" })
    }

    pub fn is_visible(&self, id: WindowId) -> bool {
        self.0.windows.lock().unwrap_or_else(|e| e.into_inner()).get(&id).is_some_and(|w| w.visible)
    }

    /// How many native windows are alive.
    pub fn open_windows(&self) -> usize {
        self.0.open.load(Ordering::Relaxed)
    }

    /// The monitors right now (any thread).
    pub fn monitors(&self) -> Vec<MonitorInfo> {
        win32::monitors()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_defaults_match_the_window_shapes() {
        let o = NativeWindow::overlay("pill").into_spec();
        assert_eq!((o.kind, o.size, o.topmost, o.activate_on_show, o.visible), (WindowKind::Overlay, (720.0, 120.0), true, false, true));
        assert!(matches!(o.placement, Placement::TopCenter { below_taskbar: true, .. }));
        assert_eq!(o.clear[3], 0.0, "the island is transparent");
        let p = NativeWindow::popup("bar").into_spec();
        assert_eq!((p.size, p.visible, p.activate_on_show, p.hide_on_blur), ((560.0, 350.0), false, true, true));
        assert!(matches!(p.placement, Placement::TopCenter { monitor: MonitorSel::UnderCursor, margin_top, .. } if margin_top == 7.0));
        let n = NativeWindow::normal("settings").into_spec();
        assert_eq!((n.size, n.min_size, n.resizable), ((840.0, 760.0), Some((700.0, 520.0)), true));
        assert_eq!(n.clear[3], 1.0, "settings are opaque");
        let f = NativeWindow::fullscreen("capture-0", MonitorSel::Index(0)).into_spec();
        assert!(f.exclude_from_capture && f.topmost && f.focus_watchdog_ms == Some(1500.0));
        assert_eq!(f.placement, Placement::Cover(MonitorSel::Index(0)));
    }

    #[test]
    fn builder_chaining_overrides() {
        let s = NativeWindow::normal("s").size(900.0, 700.0).hidden().accept_drops().fps_cap(Some(30)).title("T").restore(Geometry { x: 1, y: 2, w: 3.0, h: 4.0, maximized: true }).into_spec();
        assert_eq!((s.size, s.visible, s.accept_drops, s.fps_cap), ((900.0, 700.0), false, true, Some(30)));
        assert_eq!(s.title, "T");
        assert!(matches!(s.placement, Placement::Restore(g) if g.maximized));
        assert_eq!(NativeWindow::overlay("x").fps_cap(None).into_spec().fps_cap, None, "no cap by default");
    }

    #[test]
    fn the_global_api_is_null_until_wired() {
        assert!(GlobalApi.call("t", "x", Value::Null).is_err());
    }
}
