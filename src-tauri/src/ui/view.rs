//! The `View` contract: what a native Pill surface (bar, settings, capture overlay, panel tab, island, pets...) implements, and the `Cx` it is
//! handed. This file knows nothing about Win32: `ui::host` (the window host) drives views, `host::testing` drives them headlessly, and both
//! look the same to the view.
//!
//! # Threading rules (read once)
//! * A view lives on ONE thread, the host's UI thread, and is only ever touched there: `event`, `step`, `draw`, `on_*` all run on it. A view
//!   does not have to be `Send` (it is built on the UI thread by a factory closure, see `host::NativeWindow`).
//! * Everything that happens elsewhere (a backend event, the reply of a command, a worker finished) reaches the view as a [`Msg`] in
//!   [`View::message`]. The way there is the [`MsgQueue`]: any thread pushes through a [`Waker`], the UI thread drains it between frames.
//!   A `Waker` is `Send + Sync + Clone`: hand it to your worker threads ([`Cx::waker`]).
//! * `draw` must not block, allocate heavily or call the backend. Cache what you can in `step`/`event`; the frame budget at 240 Hz is 4 ms.
//!
//! # How a view reaches the backend
//! * Fast, in-memory commands: [`Cx::call`] (blocking, UI thread). Anything that may take a millisecond or more: [`Cx::call_async`], the
//!   reply comes back as `Msg::Reply { ticket, .. }` on the same view. Both go through the [`Api`] trait by command name (the same names as
//!   `docs/native-ui-contract.md`) and carry the window's caller label ([`Cx::caller`]) for logging / permission checks.
//! * Backend events (`ui::bus`): [`Cx::subscribe`] by event name; they arrive as `Msg::Event`. Subscriptions die with the window.
//!
//! # Frames
//! The host renders only when a view asks: an input event that returns `Response { redraw: true, .. }` (or calls [`Cx::request_frame`]), a
//! `Msg`, or a `step` that returned `true` on the previous frame. While frames are wanted they run at the display's refresh rate (vsync), never
//! slower (an optional per-window fps cap is honoured only when the caller set one). When nothing asks, the window costs zero CPU and GPU.
use glasskit::canvas::Canvas;
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, Response};
use glasskit::lang::Lang;
use glasskit::theme::Theme;
use serde_json::Value;
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

// ---------------------------------------------------------------------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------------------------------------------------------------------

/// A native surface. Only `draw` is required; every other method has a do-nothing default so a static view is ten lines.
pub trait View {
    /// The size (logical px) the view wants for its window, `None` = "whatever the window has". The host asks before every frame; a changed
    /// answer resizes the window (Overlay and Popup keep their anchor: the top centre; Normal windows only use it for their first size;
    /// Fullscreen ignores it). Return a stable frame and animate inside it: resizing a swap chain per frame is slow.
    fn size_hint(&self) -> Option<(f32, f32)> {
        None
    }

    /// One input event in logical px, client coordinates. Return what the view did with it (see `glasskit::input::Response`): `redraw`
    /// schedules a frame, `capture` keeps the mouse for a drag, `cursor` picks the mouse cursor over the event position.
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let _ = (ev, cx);
        Response::NONE
    }

    /// A file / text drag is over the window or was dropped (OLE `IDropTarget`). On `Enter` / `Over` return `Response::consumed()` to accept
    /// (copy cursor), anything else rejects. Only windows that opted in with `accept_drops` receive these.
    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        let _ = (ev, cx);
        Response::NONE
    }

    /// Cursor proximity (Overlay windows only): hover in / out, the zone stream, the top-edge dwell, the gaze feed. See [`ProximityEvent`].
    fn on_proximity(&mut self, ev: &ProximityEvent, cx: &mut Cx) -> Response {
        let _ = (ev, cx);
        Response::NONE
    }

    /// A message from outside the UI thread (backend event, command reply, `Custom` from your own worker). See [`Msg`].
    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        let _ = (msg, cx);
        Response::NONE
    }

    /// Advance animations by `dt_ms` of real time since the previous frame of this window (after an idle gap: one nominal frame, never the
    /// gap itself). Return `true` while something is still moving: the host then renders the next frame too. Return `false` when settled.
    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        let _ = (dt_ms, cx);
        false
    }

    /// Paint into `rect` (the whole client area, logical px, origin 0,0) on a canvas whose transform already is the DPI scale. The canvas is
    /// cleared transparent before the call; premultiplied alpha is respected, so Overlay / Popup windows show the desktop through it.
    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &Cx);

    /// The window became visible (first show, or shown again after `on_hide`). Subscribe, refresh data, start the enter animation here.
    fn on_show(&mut self, cx: &mut Cx) {
        let _ = cx;
    }

    /// The window was hidden (blur-hide, tray, fullscreen guard). Stop work that is only useful on screen.
    fn on_hide(&mut self, cx: &mut Cx) {
        let _ = cx;
    }

    /// The window moved to a monitor with another DPI (`scale` = device px per logical px) or the user changed the DPI. `cx.scale` is already
    /// the new value; drop caches that depend on device pixels (text layouts are cached by the canvas and re-made on their own).
    fn on_dpi(&mut self, scale: f32, cx: &mut Cx) {
        let _ = (scale, cx);
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Small value types
// ---------------------------------------------------------------------------------------------------------------------------------------

/// Identity of a native window (never reused within a run). `WindowId(0)` is "no window" (headless contexts).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);

/// Handle of a `Cx::call_async`; the matching `Msg::Reply` carries the same ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(pub u64);

/// The mouse cursor over the window. `glasskit::input::Cursor` plus the shapes only windows (not controls) ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Arrow,
    Text,
    Hand,
    ResizeEw,
    ResizeNs,
    /// Capture overlay: the selection crosshair.
    Crosshair,
    /// Four-way move (dragging a card around).
    Move,
    No,
    Wait,
}

impl From<Cursor> for CursorShape {
    fn from(c: Cursor) -> CursorShape {
        match c {
            Cursor::Arrow => CursorShape::Arrow,
            Cursor::Text => CursorShape::Text,
            Cursor::Hand => CursorShape::Hand,
            Cursor::ResizeEw => CursorShape::ResizeEw,
            Cursor::ResizeNs => CursorShape::ResizeNs,
            Cursor::No => CursorShape::No,
        }
    }
}

/// What a drag carries. `Files` are existing paths (CF_HDROP), `Text` is CF_UNICODETEXT (a dragged selection, a URL).
#[derive(Clone, Debug, PartialEq)]
pub enum DropPayload {
    Files(Vec<PathBuf>),
    Text(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropPhase {
    /// A drag entered the window; the payload is known.
    Enter,
    /// Moving over the window (no payload; `pos` is current).
    Over,
    /// The drag left without dropping.
    Leave,
    /// Released over the window: this is the drop, the payload is complete.
    Drop,
}

/// A drag-and-drop step delivered to [`View::on_drop`]. `pos` is logical px in client coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct DropEvent {
    pub phase: DropPhase,
    pub pos: Pt,
    /// `Some` for `Enter` and `Drop`.
    pub payload: Option<DropPayload>,
}

/// Cursor-proximity events of an Overlay window (the click-through island). Positions are logical px relative to the window's top-left.
/// They reproduce what `pill.rs::cursor_loop` emits as `pill:hover`, `pill:leave`, `pill:cursor`, `pill:gaze` and the top-edge reveal.
#[derive(Clone, Debug, PartialEq)]
pub enum ProximityEvent {
    /// The cursor entered the hit region (plus its pad). Fires once per visit.
    Hover,
    /// The cursor left the hit region and stayed out for the leave delay (250 ms).
    Leave,
    /// The cursor moved inside the window frame or within the zone margin (120 px) of the hit region; `None` once when it leaves the zone.
    Zone(Option<Pt>),
    /// The gaze feed ([`Cx::set_gaze`]): the cursor anywhere on any monitor, relative to the window, physical px (may be negative).
    Gaze(Pt),
    /// The cursor dwelt on the monitor's top edge above the hit region for 120 ms with no button held (auto-hide reveal).
    TopEdge,
}

/// A request from a view to open another registered window (`host::Registry`). `args` reaches the new view's factory, or, when the window
/// is a singleton that is already open, the open view as `Msg::Open(args)` (this is how "open Settings on page `ai`" focuses the old one).
#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    pub name: String,
    pub args: Value,
}

/// Window-level commands a view can issue (custom title bars, close buttons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowCmd {
    Minimize,
    ToggleMaximize,
    /// Starts the system move loop (use on mouse-down over a custom caption that is not in the caption region).
    DragMove,
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Messages from other threads
// ---------------------------------------------------------------------------------------------------------------------------------------

/// What the UI thread hands to [`View::message`].
pub enum Msg {
    /// A backend event the view subscribed to ([`Cx::subscribe`]); `payload` is the JSON the backend emitted.
    Event { name: String, payload: Value, latest: bool },
    /// The result of [`Cx::call_async`].
    Reply { ticket: Ticket, result: Result<Value, String> },
    /// A singleton window was asked to open again ([`OpenRequest`]) while open: bring your page / state up to date.
    Open(Value),
    /// Anything a worker of your own wants to tell the view; downcast it.
    Custom(Box<dyn Any + Send>),
}

impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Msg::Event { name, payload, latest } => write!(f, "Event({name}, {payload}, latest={latest})"),
            Msg::Reply { ticket, result } => write!(f, "Reply({}, {result:?})", ticket.0),
            Msg::Open(v) => write!(f, "Open({v})"),
            Msg::Custom(_) => write!(f, "Custom(..)"),
        }
    }
}

/// Longest the queue grows before the oldest message is dropped (a stuck UI thread must not eat the process's memory).
pub const QUEUE_CAP: usize = 8192;

/// The thread-safe mailbox between everything else and the UI thread. Pushing never blocks and never touches a view. The `wake` callback
/// (the host posts a window message) runs only when the queue turns from empty-and-quiet to non-empty, so a storm of events costs one wake.
pub struct MsgQueue {
    q: Mutex<VecDeque<(WindowId, Msg)>>,
    wake: Box<dyn Fn() + Send + Sync>,
    pending: AtomicBool,
    dropped: AtomicU64,
}

impl MsgQueue {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> MsgQueue {
        MsgQueue { q: Mutex::new(VecDeque::new()), wake: Box::new(wake), pending: AtomicBool::new(false), dropped: AtomicU64::new(0) }
    }

    /// A queue nobody drains (headless contexts, tests that only inspect `len`).
    pub fn detached() -> MsgQueue {
        MsgQueue::new(|| {})
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<(WindowId, Msg)>> {
        self.q.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `msg` for `id`. An `Event` with `latest: true` replaces a still-queued event of the same name for the same window (hot feeds:
    /// audio bands, cursor) so a slow frame sees the newest value instead of a backlog.
    pub fn push(&self, id: WindowId, msg: Msg) {
        {
            let mut q = self.lock();
            let key = match &msg {
                Msg::Event { name, latest: true, .. } => Some(name.clone()),
                _ => None,
            };
            if let Some(name) = key {
                if let Some(slot) = q.iter_mut().rev().find(|(i, m)| *i == id && matches!(m, Msg::Event { name: n, latest: true, .. } if *n == name)) {
                    slot.1 = msg;
                    drop(q);
                    self.kick();
                    return;
                }
            }
            if q.len() >= QUEUE_CAP {
                q.pop_front();
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            q.push_back((id, msg));
        }
        self.kick();
    }

    fn kick(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            (self.wake)();
        }
    }

    /// Take everything queued, oldest first. Re-arms the wake-up BEFORE taking, so a push racing with the drain always produces a wake.
    pub fn drain(&self) -> Vec<(WindowId, Msg)> {
        self.pending.store(false, Ordering::Release);
        self.lock().drain(..).collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many messages were dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// A cheap, `Send + Sync + Clone` handle that posts messages to ONE window's view from any thread. This is the only way off-thread code
/// talks to a view.
#[derive(Clone)]
pub struct Waker {
    id: WindowId,
    q: Arc<MsgQueue>,
}

impl Waker {
    pub fn new(id: WindowId, q: Arc<MsgQueue>) -> Waker {
        Waker { id, q }
    }

    pub fn window(&self) -> WindowId {
        self.id
    }

    pub fn post(&self, msg: Msg) {
        self.q.push(self.id, msg);
    }

    /// Shorthand for `post(Msg::Custom(Box::new(v)))`.
    pub fn post_custom<T: Any + Send>(&self, v: T) {
        self.post(Msg::Custom(Box::new(v)));
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Backend access: Api, bus, clipboard, spawn
// ---------------------------------------------------------------------------------------------------------------------------------------

/// The backend as a view sees it: a command by name with JSON arguments, answered with JSON or an error text (the `Result` convention of
/// `#[tauri::command]`). The real implementation is the `ui::api` facade (written separately, wired with `host::set_api`); tests use
/// [`RecordingApi`], a headless run without a backend uses [`NullApi`].
pub trait Api: Send + Sync {
    fn call(&self, caller: &str, name: &str, args: Value) -> Result<Value, String>;
}

/// Answers every command with an error: the host before `set_api`, and views under test that must not reach a backend.
pub struct NullApi;

impl Api for NullApi {
    fn call(&self, _caller: &str, name: &str, _args: Value) -> Result<Value, String> {
        Err(format!("no backend wired for `{name}`"))
    }
}

/// One recorded call of a [`RecordingApi`].
#[derive(Clone, Debug, PartialEq)]
pub struct ApiCall {
    pub caller: String,
    pub name: String,
    pub args: Value,
}

/// A fake backend for tests: records every call and answers from a table (`Ok(Null)` for unknown commands).
#[derive(Default)]
pub struct RecordingApi {
    calls: Mutex<Vec<ApiCall>>,
    replies: Mutex<HashMap<String, Result<Value, String>>>,
}

impl RecordingApi {
    pub fn new() -> RecordingApi {
        RecordingApi::default()
    }

    /// Sets the answer of command `name` (replaces an earlier one).
    pub fn reply(&self, name: &str, result: Result<Value, String>) -> &RecordingApi {
        self.replies.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), result);
        self
    }

    pub fn calls(&self) -> Vec<ApiCall> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Api for RecordingApi {
    fn call(&self, caller: &str, name: &str, args: Value) -> Result<Value, String> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(ApiCall { caller: caller.into(), name: name.into(), args });
        self.replies.lock().unwrap_or_else(|e| e.into_inner()).get(name).cloned().unwrap_or(Ok(Value::Null))
    }
}

/// Listening to backend events. The real one is `ui::bus` (handlers run on the emitting thread: they only post to a [`Waker`]).
pub trait BusApi: Send + Sync {
    fn subscribe(&self, name: &str, f: Box<dyn Fn(&Value) + Send + Sync>) -> u64;
    fn unsubscribe(&self, id: u64);
}

/// `ui::bus` as a [`BusApi`].
pub struct GlobalBus;

impl BusApi for GlobalBus {
    fn subscribe(&self, name: &str, f: Box<dyn Fn(&Value) + Send + Sync>) -> u64 {
        crate::ui::bus::subscribe(name, move |v| f(v))
    }
    fn unsubscribe(&self, id: u64) {
        crate::ui::bus::unsubscribe(id)
    }
}

type BusHandler = (u64, String, Arc<dyn Fn(&Value) + Send + Sync>);

/// A bus for tests: `emit` calls the subscribed handlers on the calling thread.
#[derive(Default)]
pub struct FakeBus {
    subs: Mutex<Vec<BusHandler>>,
    next: AtomicU64,
}

impl FakeBus {
    pub fn new() -> FakeBus {
        FakeBus::default()
    }

    pub fn emit(&self, name: &str, payload: Value) {
        let hs: Vec<_> = self.subs.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|s| s.1 == name).map(|s| s.2.clone()).collect();
        for h in hs {
            h(&payload);
        }
    }

    pub fn subscribers(&self, name: &str) -> usize {
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|s| s.1 == name).count()
    }
}

impl BusApi for FakeBus {
    fn subscribe(&self, name: &str, f: Box<dyn Fn(&Value) + Send + Sync>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).push((id, name.to_string(), Arc::from(f)));
        id
    }
    fn unsubscribe(&self, id: u64) {
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| s.0 != id);
    }
}

/// Text clipboard (the only format views need; files go through drag-and-drop).
pub trait Clipboard: Send + Sync {
    fn get_text(&self) -> Option<String>;
    fn set_text(&self, text: &str);
}

/// Clipboard in memory, for tests.
#[derive(Default)]
pub struct MemClipboard(Mutex<Option<String>>);

impl Clipboard for MemClipboard {
    fn get_text(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn set_text(&self, text: &str) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
    }
}

/// Runs a job off the UI thread (`Cx::call_async`).
pub trait Spawn: Send + Sync {
    fn spawn(&self, job: Box<dyn FnOnce() + Send>);
}

/// The shared worker pool of glasskit (bounded, survives a hung job).
pub struct PoolSpawn;

impl Spawn for PoolSpawn {
    fn spawn(&self, job: Box<dyn FnOnce() + Send>) {
        if !glasskit::jobs::pool::global().submit(job) {
            eprintln!("ui: worker pool full, a backend call was dropped");
        }
    }
}

/// Runs the job right away on the calling thread: tests get deterministic replies.
pub struct InlineSpawn;

impl Spawn for InlineSpawn {
    fn spawn(&self, job: Box<dyn FnOnce() + Send>) {
        job()
    }
}

/// Everything a `Cx` reaches outside the UI thread through, as trait objects so a test swaps each one.
#[derive(Clone)]
pub struct Services {
    pub api: Arc<dyn Api>,
    pub bus: Arc<dyn BusApi>,
    pub clipboard: Arc<dyn Clipboard>,
    pub spawn: Arc<dyn Spawn>,
}

/// The fakes behind [`Services::fake`], kept so a test can inspect them.
pub struct Fakes {
    pub api: Arc<RecordingApi>,
    pub bus: Arc<FakeBus>,
    pub clipboard: Arc<MemClipboard>,
}

impl Services {
    /// No backend: `NullApi`, an empty `FakeBus`, an in-memory clipboard, inline jobs.
    pub fn null() -> Services {
        Services::fake().0
    }

    /// Recording / fake services with inline job execution, plus handles to inspect them.
    pub fn fake() -> (Services, Fakes) {
        let f = Fakes { api: Arc::new(RecordingApi::new()), bus: Arc::new(FakeBus::new()), clipboard: Arc::new(MemClipboard::default()) };
        let s = Services { api: f.api.clone(), bus: f.bus.clone(), clipboard: f.clipboard.clone(), spawn: Arc::new(InlineSpawn) };
        (s, f)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Cx
// ---------------------------------------------------------------------------------------------------------------------------------------

/// What a view asked the host to do during one call. The host reads (and clears) it after every `event` / `step` / `message` / `on_*`.
/// Plain data, so tests assert on it without a window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effects {
    /// Render a frame even if `step` does not keep animating.
    pub frame: bool,
    pub cursor: Option<CursorShape>,
    /// `Some(rects)` = clickable parts (Overlay: everything else is click-through); `Some(vec![])` = nothing clickable.
    pub hit_region: Option<Vec<Rect>>,
    /// Back to "the whole window is clickable".
    pub hit_all: bool,
    /// Normal windows: the parts that drag the window (HTCAPTION). Buttons inside must not be covered.
    pub caption_region: Option<Vec<Rect>>,
    /// Overlay: the rectangle where a file drag makes the window hit-testable so the drop can land (`None` clears it).
    pub drop_zone: Option<Option<Rect>>,
    /// Overlay: the whole window lets clicks through while folded (the `passThrough` setting).
    pub pass_through: Option<bool>,
    /// Overlay: stream `ProximityEvent::Gaze`.
    pub gaze: Option<bool>,
    /// Take (`true`) or give back (`false`) keyboard focus; an Overlay is no-activate until asked.
    pub focus: Option<bool>,
    pub show: bool,
    pub hide: bool,
    pub close: bool,
    pub title: Option<String>,
    pub window_cmd: Option<WindowCmd>,
    pub opens: Vec<OpenRequest>,
    /// Where the IME candidate window should sit (logical px, client coordinates): the text caret.
    pub ime_rect: Option<Rect>,
    /// `Some(None)` removes the cap, `Some(Some(n))` caps this window at `n` frames per second. Never set by the framework on its own.
    pub fps_cap: Option<Option<u32>>,
}

/// The per-window context passed to every view call. Fields below are written by the host before each call; views read them.
pub struct Cx {
    /// Milliseconds on the host's monotonic clock at the start of this frame / event.
    pub now_ms: f64,
    /// Device px per logical px of the monitor the window is on.
    pub scale: f32,
    pub lang: Lang,
    /// Animations should jump to their end state: the OS "show animations" setting is off or the app setting `reduceMotion` is on.
    pub reduced_motion: bool,
    /// Refresh rate (Hz) of the monitor the window is on; frames are paced to it. 60 when unknown.
    pub refresh_hz: f32,
    /// The window's label (`"bar"`, `"settings"`, ...), sent with every backend call.
    pub caller: String,
    pub window: WindowId,
    svc: Services,
    waker: Waker,
    fx: Effects,
    subs: HashMap<String, u64>,
}

static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

impl Cx {
    pub fn new(window: WindowId, caller: &str, svc: Services, waker: Waker) -> Cx {
        Cx { now_ms: 0.0, scale: 1.0, lang: Lang::En, reduced_motion: false, refresh_hz: 60.0, caller: caller.to_string(), window, svc, waker, fx: Effects::default(), subs: HashMap::new() }
    }

    /// A context without a window: `NullApi`-style fake services, a queue nobody drains. For unit tests of a view.
    pub fn headless() -> Cx {
        Cx::headless_with(Services::null())
    }

    /// A headless context over services of your choice (`Services::fake()` to inspect calls). Returns the queue so a test can drain what
    /// the view's subscriptions and `call_async` replies posted.
    pub fn headless_with(svc: Services) -> Cx {
        Cx::new(WindowId(0), "test", svc, Waker::new(WindowId(0), Arc::new(MsgQueue::detached())))
    }

    /// The same, with the queue exposed (to drain `Msg::Reply` / `Msg::Event` in a test).
    pub fn headless_queue(svc: Services) -> (Cx, Arc<MsgQueue>) {
        let q = Arc::new(MsgQueue::detached());
        (Cx::new(WindowId(0), "test", svc, Waker::new(WindowId(0), q.clone())), q)
    }

    // ---- backend ----

    /// Calls a backend command and waits for the answer. UI thread: only for commands that are in-memory fast (a settings read).
    pub fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        self.svc.api.call(&self.caller, name, args)
    }

    /// Calls a backend command on a worker; the answer arrives as `Msg::Reply { ticket, .. }` (a panic in the command becomes an `Err`).
    pub fn call_async(&mut self, name: &str, args: Value) -> Ticket {
        let ticket = Ticket(NEXT_TICKET.fetch_add(1, Ordering::Relaxed));
        let (api, waker, caller, name) = (self.svc.api.clone(), self.waker.clone(), self.caller.clone(), name.to_string());
        self.svc.spawn.spawn(Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| api.call(&caller, &name, args))).unwrap_or_else(|_| Err(format!("`{name}` panicked")));
            waker.post(Msg::Reply { ticket, result });
        }));
        ticket
    }

    /// Delivers backend event `name` as `Msg::Event` (idempotent per name). Unsubscribed automatically when the window closes.
    pub fn subscribe(&mut self, name: &str) {
        self.sub(name, false);
    }

    /// Like `subscribe`, but a burst collapses to the newest payload per frame (audio bands, cursor positions).
    pub fn subscribe_latest(&mut self, name: &str) {
        self.sub(name, true);
    }

    fn sub(&mut self, name: &str, latest: bool) {
        if self.subs.contains_key(name) {
            return;
        }
        let (w, n) = (self.waker.clone(), name.to_string());
        let id = self.svc.bus.subscribe(name, Box::new(move |v| w.post(Msg::Event { name: n.clone(), payload: v.clone(), latest })));
        self.subs.insert(name.to_string(), id);
    }

    pub fn unsubscribe(&mut self, name: &str) {
        if let Some(id) = self.subs.remove(name) {
            self.svc.bus.unsubscribe(id);
        }
    }

    /// A `Send + Sync + Clone` handle for your own worker threads: whatever they post comes back in [`View::message`].
    pub fn waker(&self) -> Waker {
        self.waker.clone()
    }

    /// Posts `msg` to this view's own queue: it arrives in [`View::message`] after the current call (and wakes a frame).
    pub fn post(&self, msg: Msg) {
        self.waker.post(msg);
    }

    pub fn clipboard_text(&self) -> Option<String> {
        self.svc.clipboard.get_text()
    }

    pub fn set_clipboard_text(&self, text: &str) {
        self.svc.clipboard.set_text(text)
    }

    // ---- window requests (applied by the host after the call returns) ----

    /// Render a frame soon even if nothing is animating.
    pub fn request_frame(&mut self) {
        self.fx.frame = true;
    }

    /// The cursor for the current pointer position (sticky until the next event; `Response::cursor` does the same per event).
    pub fn set_cursor(&mut self, c: CursorShape) {
        self.fx.cursor = Some(c);
    }

    /// Clickable parts of the window in logical px (Overlay: the rest is click-through, see `host` docs for the proximity engine).
    pub fn set_hit_region(&mut self, rects: Vec<Rect>) {
        self.fx.hit_all = false;
        self.fx.hit_region = Some(rects);
    }

    /// The whole window is clickable again (the default).
    pub fn clear_hit_region(&mut self) {
        self.fx.hit_region = None;
        self.fx.hit_all = true;
    }

    pub fn set_caption_region(&mut self, rects: Vec<Rect>) {
        self.fx.caption_region = Some(rects);
    }

    pub fn set_drop_zone(&mut self, zone: Option<Rect>) {
        self.fx.drop_zone = Some(zone);
    }

    pub fn set_pass_through(&mut self, on: bool) {
        self.fx.pass_through = Some(on);
    }

    pub fn set_gaze(&mut self, on: bool) {
        self.fx.gaze = Some(on);
    }

    /// Ask for keyboard focus (`true`) or give it back to the previous foreground window (`false`).
    pub fn set_focus(&mut self, on: bool) {
        self.fx.focus = Some(on);
    }

    pub fn show_self(&mut self) {
        self.fx.show = true;
        self.fx.hide = false;
    }

    pub fn hide_self(&mut self) {
        self.fx.hide = true;
        self.fx.show = false;
    }

    /// Closes (destroys) this window after the current call.
    pub fn close_self(&mut self) {
        self.fx.close = true;
    }

    pub fn set_title(&mut self, title: &str) {
        self.fx.title = Some(title.to_string());
    }

    pub fn window_cmd(&mut self, c: WindowCmd) {
        self.fx.window_cmd = Some(c);
    }

    /// Opens (or re-opens, for singletons) a registered window. Unknown names are logged and ignored.
    pub fn open_window(&mut self, name: &str, args: Value) {
        self.fx.opens.push(OpenRequest { name: name.to_string(), args });
    }

    /// Where the text caret is, so the IME candidate list appears next to it.
    pub fn set_ime_rect(&mut self, r: Rect) {
        self.fx.ime_rect = Some(r);
    }

    /// Caps this window's frame rate. Only for callers that really want less than the display rate (a battery mode the USER chose).
    pub fn set_fps_cap(&mut self, cap: Option<u32>) {
        self.fx.fps_cap = Some(cap);
    }

    /// Takes (and clears) what the view asked for. Called by the host after every view call; tests read it to assert behaviour.
    pub fn take_effects(&mut self) -> Effects {
        std::mem::take(&mut self.fx)
    }

    /// Peek without clearing.
    pub fn effects(&self) -> &Effects {
        &self.fx
    }
}

impl Drop for Cx {
    fn drop(&mut self) {
        for (_, id) in self.subs.drain() {
            self.svc.bus.unsubscribe(id);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn queue_wakes_once_per_burst_and_rearms_on_drain() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let q = MsgQueue::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        });
        for i in 0..100 {
            q.push(WindowId(1), Msg::Open(json!(i)));
        }
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "a burst is one wake");
        let got = q.drain();
        assert_eq!(got.len(), 100);
        assert!(matches!(&got[0].1, Msg::Open(v) if *v == json!(0)), "oldest first");
        q.push(WindowId(1), Msg::Open(json!("again")));
        assert_eq!(wakes.load(Ordering::SeqCst), 2, "drain re-arms the wake");
        assert!(q.drain().len() == 1 && q.is_empty());
    }

    #[test]
    fn latest_events_collapse_per_window_and_name() {
        let q = MsgQueue::detached();
        let ev = |n: &str, v: i32, latest| Msg::Event { name: n.into(), payload: json!(v), latest };
        q.push(WindowId(1), ev("audio:bands", 1, true));
        q.push(WindowId(1), ev("media:changed", 1, false));
        q.push(WindowId(1), ev("audio:bands", 2, true));
        q.push(WindowId(2), ev("audio:bands", 9, true));
        q.push(WindowId(1), ev("media:changed", 2, false));
        q.push(WindowId(1), ev("audio:bands", 3, true));
        let got: Vec<(u64, String, Value)> = q
            .drain()
            .into_iter()
            .map(|(id, m)| match m {
                Msg::Event { name, payload, .. } => (id.0, name, payload),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (1, "audio:bands".into(), json!(3)), // newest value, original position
                (1, "media:changed".into(), json!(1)),
                (2, "audio:bands".into(), json!(9)),
                (1, "media:changed".into(), json!(2)), // plain events are never merged
            ]
        );
    }

    #[test]
    fn queue_is_bounded_and_counts_drops() {
        let q = MsgQueue::detached();
        for i in 0..QUEUE_CAP + 5 {
            q.push(WindowId(1), Msg::Open(json!(i)));
        }
        assert_eq!(q.len(), QUEUE_CAP);
        assert_eq!(q.dropped(), 5);
        assert!(matches!(&q.drain()[0].1, Msg::Open(v) if *v == json!(5)), "the oldest went");
    }

    #[test]
    fn queue_is_safe_from_many_threads() {
        let q = Arc::new(MsgQueue::detached());
        let hs: Vec<_> = (0..4)
            .map(|t| {
                let w = Waker::new(WindowId(t), q.clone());
                std::thread::spawn(move || {
                    for i in 0..500 {
                        w.post_custom((t, i));
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(q.drain().len(), 2000);
    }

    #[test]
    fn call_goes_through_the_api_with_the_caller_label() {
        let (svc, fakes) = Services::fake();
        fakes.api.reply("settings_get", Ok(json!({ "size": 100 })));
        let mut cx = Cx::headless_with(svc);
        cx.caller = "bar".into();
        assert_eq!(cx.call("settings_get", json!({ "key": "size" })).unwrap(), json!({ "size": 100 }));
        assert_eq!(cx.call("unknown", json!(null)).unwrap(), Value::Null);
        let calls = fakes.api.calls();
        assert_eq!(calls[0], ApiCall { caller: "bar".into(), name: "settings_get".into(), args: json!({ "key": "size" }) });
        assert!(NullApi.call("x", "y", json!(null)).is_err());
    }

    #[test]
    fn call_async_replies_through_the_queue_and_survives_a_panic() {
        struct Boom;
        impl Api for Boom {
            fn call(&self, _: &str, name: &str, _: Value) -> Result<Value, String> {
                if name == "boom" {
                    panic!("backend bug");
                }
                Ok(json!(name))
            }
        }
        let (mut svc, _) = Services::fake();
        svc.api = Arc::new(Boom);
        let (mut cx, q) = Cx::headless_queue(svc);
        let (a, b) = (cx.call_async("fine", json!(null)), cx.call_async("boom", json!(null)));
        assert_ne!(a, b);
        let got = q.drain();
        assert_eq!(got.len(), 2);
        match (&got[0].1, &got[1].1) {
            (Msg::Reply { ticket: ta, result: Ok(v) }, Msg::Reply { ticket: tb, result: Err(e) }) => {
                assert_eq!((*ta, *tb), (a, b));
                assert_eq!(v, &json!("fine"));
                assert!(e.contains("panicked"), "{e}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn subscriptions_post_events_are_idempotent_and_die_with_the_cx() {
        let (svc, fakes) = Services::fake();
        let (mut cx, q) = Cx::headless_queue(svc);
        cx.subscribe("media:changed");
        cx.subscribe("media:changed");
        cx.subscribe_latest("audio:bands");
        assert_eq!(fakes.bus.subscribers("media:changed"), 1);
        fakes.bus.emit("media:changed", json!({ "title": "x" }));
        fakes.bus.emit("audio:bands", json!([1]));
        fakes.bus.emit("audio:bands", json!([2]));
        fakes.bus.emit("other", json!(0));
        assert_eq!(q.len(), 2, "latest collapsed, unrelated ignored");
        cx.unsubscribe("media:changed");
        assert_eq!(fakes.bus.subscribers("media:changed"), 0);
        drop(cx);
        assert_eq!(fakes.bus.subscribers("audio:bands"), 0, "dropping the context unsubscribes");
    }

    #[test]
    fn effects_are_recorded_and_taken_once() {
        let mut cx = Cx::headless();
        cx.request_frame();
        cx.set_cursor(CursorShape::Crosshair);
        cx.set_hit_region(vec![Rect::new(0.0, 0.0, 10.0, 10.0)]);
        cx.open_window("settings", json!({ "page": "ai" }));
        cx.set_clipboard_text("hi");
        assert_eq!(cx.clipboard_text().as_deref(), Some("hi"));
        cx.close_self();
        let fx = cx.take_effects();
        assert!(fx.frame && fx.close && !fx.hit_all);
        assert_eq!(fx.cursor, Some(CursorShape::Crosshair));
        assert_eq!(fx.hit_region.as_deref(), Some(&[Rect::new(0.0, 0.0, 10.0, 10.0)][..]));
        assert_eq!(fx.opens, vec![OpenRequest { name: "settings".into(), args: json!({ "page": "ai" }) }]);
        assert_eq!(cx.take_effects(), Effects::default(), "taken effects are cleared");
        cx.clear_hit_region();
        assert!(cx.effects().hit_all && cx.effects().hit_region.is_none());
    }

    #[test]
    fn a_view_can_post_to_itself() {
        let (cx, q) = Cx::headless_queue(Services::null());
        cx.post(Msg::Open(json!(7)));
        cx.waker().post_custom(5u32);
        let got = q.drain();
        assert_eq!(got.len(), 2);
        assert!(matches!(&got[0].1, Msg::Open(v) if *v == json!(7)));
        assert!(matches!(&got[1].1, Msg::Custom(b) if b.downcast_ref::<u32>() == Some(&5)));
    }

    #[test]
    fn cursor_conversion_covers_every_glasskit_cursor() {
        assert_eq!(CursorShape::from(Cursor::Hand), CursorShape::Hand);
        assert_eq!(CursorShape::from(Cursor::Text), CursorShape::Text);
        assert_eq!(CursorShape::from(Cursor::ResizeEw), CursorShape::ResizeEw);
        assert_eq!(CursorShape::from(Cursor::No), CursorShape::No);
    }

    struct Dot;
    impl View for Dot {
        fn draw(&mut self, _: &Canvas, _: &Theme, _: Rect, _: &Cx) {}
    }

    #[test]
    fn a_static_view_needs_only_draw() {
        let mut v = Dot;
        let mut cx = Cx::headless();
        assert_eq!(v.size_hint(), None);
        assert!(!v.step(16.0, &mut cx));
        assert_eq!(v.event(&InputEvent::MouseLeave, &mut cx), Response::NONE);
        assert_eq!(v.message(Msg::Open(json!(1)), &mut cx), Response::NONE);
    }
}
