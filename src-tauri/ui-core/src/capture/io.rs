//! Backend calls and I/O traits of the capture overlay. The model never touches the OS: it emits [`BackendCall`]s (the same
//! command names and camelCase arguments the web page passes to `invoke`) through the [`CaptureIo`] trait, and tests use
//! [`FakeIo`].
//!
//! Commands used (see docs/native-ui-contract.md):
//! `capture_session {label}`, `capture_frame {label}`, `capture_ready {label}`, `capture_cancel`, `capture_commit {label, vw, vh,
//! action, rect | x, y}` (JSON, or a raw PNG body with the JSON in header `x-pill-args` when markup was drawn), `capture_start
//! {mode}`, `capture_stop`, `capture_settings`, `capture_action {action, value}`, `shelf_add {paths}`. `capture_pin` is a
//! *native-only* name for the pin-to-screen extension and is not in the web backend yet (see [`BackendCall::is_registered`]).
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A command invocation: `cmd` is the Rust command name, `args` the JSON object of arguments (camelCase keys), `Null` for none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BackendCall {
    pub cmd: String,
    pub args: Value,
}

/// Commands that exist in the current backend (what the web UI can call today).
pub const REGISTERED_CMDS: &[&str] = &[
    "capture_start",
    "capture_commit",
    "capture_cancel",
    "capture_stop",
    "capture_settings",
    "capture_session",
    "capture_frame",
    "capture_ready",
    "capture_action",
    "shelf_add",
];

/// Header carrying the JSON arguments when `capture_commit` has a raw PNG body.
pub const ARGS_HEADER: &str = "x-pill-args";

impl BackendCall {
    pub fn new(cmd: &str, args: Value) -> BackendCall {
        BackendCall { cmd: cmd.to_string(), args }
    }
    /// True when the web backend has this command; false for native-only extensions that need a Rust-side command first.
    pub fn is_registered(&self) -> bool {
        REGISTERED_CMDS.contains(&self.cmd.as_str())
    }
    pub fn session(label: &str) -> BackendCall {
        BackendCall::new("capture_session", json!({ "label": label }))
    }
    pub fn frame(label: &str) -> BackendCall {
        BackendCall::new("capture_frame", json!({ "label": label }))
    }
    pub fn ready(label: &str) -> BackendCall {
        BackendCall::new("capture_ready", json!({ "label": label }))
    }
    pub fn cancel() -> BackendCall {
        BackendCall::new("capture_cancel", Value::Null)
    }
    pub fn stop() -> BackendCall {
        BackendCall::new("capture_stop", Value::Null)
    }
    pub fn settings() -> BackendCall {
        BackendCall::new("capture_settings", Value::Null)
    }
    /// Start a capture: `area | screen | text | qr | color | record | record_screen`.
    pub fn start(mode: &str) -> BackendCall {
        BackendCall::new("capture_start", json!({ "mode": mode }))
    }
    pub fn commit(args: &CommitArgs) -> BackendCall {
        BackendCall::new("capture_commit", args.to_json())
    }
    /// Result-card button: `open | reveal | copy_image | copy_file | copy_path | copy_text | open_url`.
    pub fn action(action: &str, value: &str) -> BackendCall {
        BackendCall::new("capture_action", json!({ "action": action, "value": value }))
    }
    /// Put saved files on the shelf (`shelf_add`).
    pub fn shelf_add(paths: &[String]) -> BackendCall {
        BackendCall::new("shelf_add", json!({ "paths": paths }))
    }
    /// Pin an image to the screen (native extension; not in the web backend).
    pub fn pin(path: &str) -> BackendCall {
        BackendCall::new("capture_pin", json!({ "path": path }))
    }
}

/// What a commit asks the backend to do (`extra.action` of `commit()` in capture.js).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommitKind {
    /// Copy to the clipboard (Ctrl+C).
    Copy,
    /// Save to the capture folder (Ctrl+S).
    Save,
    /// Copy and save ("Done", Enter, double click); the backend maps it through `capture.shotAction`.
    Both,
    /// Record the selected area as video.
    Record,
    /// Run the mode's job on the selection: OCR (`text`), QR decode (`qr`), record (`record`).
    Run,
    /// Colour picker click / Enter.
    Pick,
}

impl CommitKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CommitKind::Copy => "copy",
            CommitKind::Save => "save",
            CommitKind::Both => "both",
            CommitKind::Record => "record",
            CommitKind::Run => "run",
            CommitKind::Pick => "pick",
        }
    }
}

/// Arguments of `capture_commit`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitArgs {
    pub label: String,
    /// Viewport size in css px: the backend derives the per-axis scale from it and the frame size.
    pub vw: f64,
    pub vh: f64,
    pub action: CommitKind,
    /// Selection `[x, y, w, h]` in css px (absent for a whole-screen record / the colour picker).
    pub rect: Option<[f64; 4]>,
    /// Picked position in css px (colour picker only).
    pub point: Option<super::geom::Pt>,
}

impl CommitArgs {
    /// The JSON the page builds: `{label, vw, vh, action, rect?, x?, y?}`.
    pub fn to_json(&self) -> Value {
        let mut v = json!({ "label": self.label, "vw": self.vw, "vh": self.vh, "action": self.action.as_str() });
        if let Some(r) = self.rect {
            v["rect"] = json!(r);
        }
        if let Some(p) = self.point {
            v["x"] = json!(p.x);
            v["y"] = json!(p.y);
        }
        v
    }
    /// Value of the `x-pill-args` header for the raw-PNG form.
    pub fn header_json(&self) -> String {
        self.to_json().to_string()
    }
}

/// Answer of `capture_session`: what the overlay should show.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionInfo {
    pub id: u64,
    pub mode: String,
    /// Frame size in physical px.
    pub w: i32,
    pub h: i32,
    /// Monitor origin in the virtual desktop, physical px.
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    /// True for the overlay under the cursor (it takes the keyboard focus).
    pub focus: bool,
}

impl SessionInfo {
    pub fn from_value(v: &Value) -> Option<SessionInfo> {
        serde_json::from_value(v.clone()).ok()
    }
}

/// What the renderer must flatten for a markup export: the cropped frame region plus the ops drawn on top.
#[derive(Clone, Debug, PartialEq)]
pub struct FlattenRequest {
    /// Crop of the frame, physical px.
    pub src: super::geom::IRect,
    pub cmds: Vec<super::markup::DrawCmd>,
}

/// The seam between the model and the world. `call` returns the JSON the backend answered, `None` on error or `null`
/// (`inv()` of the page swallows errors the same way).
pub trait CaptureIo {
    fn call(&mut self, call: &BackendCall) -> Option<Value>;
    /// A command with a raw body (the flattened PNG); the args travel in the [`ARGS_HEADER`] header.
    fn call_raw(&mut self, call: &BackendCall, body: &[u8]) -> Option<Value>;
    /// The frozen RGBA frame of this overlay (`capture_frame`), `w*h*4` bytes.
    fn frame(&mut self, label: &str) -> Option<Vec<u8>>;
    /// Render crop + ops into a PNG (the renderer owns the canvas / bitmap).
    fn flatten_png(&mut self, req: &FlattenRequest) -> Option<Vec<u8>>;
    /// Let the first frame paint before the window is shown (the page yields through a MessageChannel).
    fn yield_to_paint(&mut self) {}
    /// Give the overlay keyboard focus (`refocus()`).
    fn refocus(&mut self) {}
    /// Monotonic milliseconds for animation timestamps.
    fn now_ms(&self) -> u64;
}

/// Recording fake: scripted replies, every call logged. `frame`, `png` and `session` are what the "backend" answers.
#[derive(Debug, Default)]
pub struct FakeIo {
    pub calls: Vec<BackendCall>,
    /// Body lengths of raw calls, parallel to the `commit` entries that were raw (`(index into calls, body)`).
    pub raw_bodies: Vec<(usize, Vec<u8>)>,
    pub session: Option<Value>,
    pub frame: Option<Vec<u8>>,
    pub png: Option<Vec<u8>>,
    pub flattened: Vec<FlattenRequest>,
    pub now: u64,
    pub refocused: u32,
    pub yielded: u32,
}

impl FakeIo {
    pub fn new() -> FakeIo {
        FakeIo::default()
    }
    /// A fake that answers `capture_session` / `capture_frame` for an area overlay of `w x h` (a flat grey frame).
    pub fn with_session(id: u64, mode: &str, w: i32, h: i32) -> FakeIo {
        FakeIo {
            session: Some(json!({ "id": id, "mode": mode, "w": w, "h": h, "x": 0, "y": 0, "scale": 1.0, "focus": true })),
            frame: Some(vec![0x80; (w * h * 4) as usize]),
            png: Some(vec![0x89, b'P', b'N', b'G']),
            ..FakeIo::default()
        }
    }
    /// Commands in call order.
    pub fn cmds(&self) -> Vec<&str> {
        self.calls.iter().map(|c| c.cmd.as_str()).collect()
    }
    /// The last call with this command name.
    pub fn last(&self, cmd: &str) -> Option<&BackendCall> {
        self.calls.iter().rev().find(|c| c.cmd == cmd)
    }
}

impl CaptureIo for FakeIo {
    fn call(&mut self, call: &BackendCall) -> Option<Value> {
        self.calls.push(call.clone());
        match call.cmd.as_str() {
            "capture_session" => self.session.clone(),
            _ => None,
        }
    }
    fn call_raw(&mut self, call: &BackendCall, body: &[u8]) -> Option<Value> {
        self.calls.push(call.clone());
        self.raw_bodies.push((self.calls.len() - 1, body.to_vec()));
        None
    }
    fn frame(&mut self, label: &str) -> Option<Vec<u8>> {
        self.calls.push(BackendCall::frame(label));
        self.frame.clone()
    }
    fn flatten_png(&mut self, req: &FlattenRequest) -> Option<Vec<u8>> {
        self.flattened.push(req.clone());
        self.png.clone()
    }
    fn yield_to_paint(&mut self) {
        self.yielded += 1;
    }
    fn refocus(&mut self) {
        self.refocused += 1;
    }
    fn now_ms(&self) -> u64 {
        self.now
    }
}
