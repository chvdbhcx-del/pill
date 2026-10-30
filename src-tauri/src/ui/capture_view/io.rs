//! `CaptureIo` for a native overlay: the seam between the pure ui-core model and the app.
//!
//! The model calls the backend synchronously, the UI thread must not wait for anything slow. So: the in-memory commands the arm sequence
//! needs (`capture_session`, `capture_ready`) are answered at once through `Cx::call`; everything else (`capture_commit`,
//! `capture_cancel`, `shelf_add`...) is handed to `Cx::call_async`. A commit with markup is a *job*: the PNG flatten (a GPU render and an
//! encode) runs on a worker thread and only then is the raw body sent, so a 4K export never blocks input or animation.
use crate::ui::view::Cx;
use pill_ui_core::capture::io::FlattenRequest;
use pill_ui_core::capture::{BackendCall, CaptureIo};
use serde_json::Value;
use std::sync::Arc;

/// Body marker the model gets from `flatten_png`: the real PNG is made later, on a worker (see [`RawJob`]).
pub const PENDING_PNG: [u8; 1] = [0];

/// Delivers a raw-body command: `(caller, args, body)`. The real one is `ui::api::call_raw_async("capture_commit", ..)`.
pub type RawSink = Arc<dyn Fn(&str, Value, Vec<u8>) + Send + Sync>;
/// `capture_frame(label)`: the session's own `Arc` (no copy).
pub type FrameSource = Box<dyn Fn(&str) -> Option<Arc<Vec<u8>>>>;

/// Everything the view reaches outside itself through, injectable for tests.
pub struct Backend {
    pub frame: FrameSource,
    pub raw: RawSink,
}

impl Backend {
    pub fn real() -> Backend {
        Backend {
            frame: Box::new(crate::ui::api::capture_frame),
            raw: Arc::new(|caller, args, body| {
                crate::ui::api::call_raw_async("capture_commit", args, caller, body, |r| {
                    if let Err(e) = r {
                        eprintln!("capture: commit failed: {e}");
                    }
                })
            }),
        }
    }
}

/// A markup commit waiting for its PNG.
pub struct RawJob {
    pub args: Value,
    pub req: FlattenRequest,
}

/// What `ViewIo` leaves behind for the view: flags set during one model call and the jobs to start after it.
#[derive(Default)]
pub struct IoState {
    /// The frozen frame as the backend shares it (kept past the model's `clear_all`: the overlay keeps painting it while it fades).
    pub frame: Option<Arc<Vec<u8>>>,
    /// `capture_session.focus`: this overlay is the one under the cursor and takes the keyboard.
    pub focus: bool,
    /// The arm sequence reached `yield_to_paint`: the first frame can be drawn, the window may be shown.
    pub yielded: bool,
    pub now: u64,
    flatten: Option<FlattenRequest>,
    pub jobs: Vec<RawJob>,
    /// The commands handed to `call_async`, in order (for logs and tests).
    pub sent: Vec<String>,
}

impl IoState {
    pub fn begin_call(&mut self, now: u64) {
        self.now = now;
        self.yielded = false;
    }
}

/// The model's I/O for the duration of one call.
pub struct ViewIo<'a> {
    pub cx: &'a mut Cx,
    pub st: &'a mut IoState,
    pub be: &'a Backend,
}

impl CaptureIo for ViewIo<'_> {
    fn call(&mut self, call: &BackendCall) -> Option<Value> {
        match call.cmd.as_str() {
            "capture_session" | "capture_ready" => {
                let r = self.cx.call(&call.cmd, call.args.clone()).ok().filter(|v| !v.is_null());
                if call.cmd == "capture_session" {
                    self.st.focus = r.as_ref().and_then(|v| v.get("focus")).and_then(Value::as_bool).unwrap_or(false);
                }
                r
            }
            _ => {
                self.st.sent.push(call.cmd.clone());
                self.cx.call_async(&call.cmd, call.args.clone());
                None
            }
        }
    }

    fn call_raw(&mut self, call: &BackendCall, body: &[u8]) -> Option<Value> {
        match (body == PENDING_PNG, self.st.flatten.take()) {
            (true, Some(req)) => self.st.jobs.push(RawJob { args: call.args.clone(), req }),
            // a body that is not ours (never happens from the model): send it as it is, off-thread
            _ => {
                let (sink, caller, args, body) = (self.be.raw.clone(), self.cx.caller.clone(), call.args.clone(), body.to_vec());
                std::thread::spawn(move || sink(&caller, args, body));
            }
        }
        self.st.sent.push(format!("{}+raw", call.cmd));
        None
    }

    fn frame(&mut self, label: &str) -> Option<Vec<u8>> {
        let f = (self.be.frame)(label)?;
        self.st.frame = Some(f.clone());
        Some(f.as_ref().clone())
    }

    fn flatten_png(&mut self, req: &FlattenRequest) -> Option<Vec<u8>> {
        self.st.flatten = Some(req.clone());
        Some(PENDING_PNG.to_vec())
    }

    fn yield_to_paint(&mut self) {
        self.st.yielded = true;
    }

    fn now_ms(&self) -> u64 {
        self.st.now
    }
}

/// An I/O that does nothing, for model calls that cannot reach the backend (a resize).
pub struct NoIo(pub u64);

impl CaptureIo for NoIo {
    fn call(&mut self, _: &BackendCall) -> Option<Value> {
        None
    }
    fn call_raw(&mut self, _: &BackendCall, _: &[u8]) -> Option<Value> {
        None
    }
    fn frame(&mut self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn flatten_png(&mut self, _: &FlattenRequest) -> Option<Vec<u8>> {
        None
    }
    fn now_ms(&self) -> u64 {
        self.0
    }
}
