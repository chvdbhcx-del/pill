// In-process event bus: the one place every backend -> UI event goes through.
//   emit(app, target, name, payload)   forwards to the webviews exactly like `app.emit_to(label, ..)` / `app.emit(..)` did (same event name, same
//                                      JSON text, same target), then hands the payload to native subscribers
//   subscribe(name, f) -> id           a native (non-WebView) UI part listens to an event; f runs on the emitting thread, so it must be quick
// The payload is any `Serialize` value (`serde_json::Value` included): typed payloads are written to JSON once, by serde_json, exactly as tauri
// did, so hot events (audio bands, cursor, gaze) stay byte-identical and are not copied through a `Value`. The Value is built only when somebody
// subscribed to that event name.
// The forwarding to Tauri is the `Sink` trait object; tests (and, later, a native-only run) hand `emit_with` their own sink and `Bus`.
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Emitter};

/// Who gets the event: every window (`app.emit`) or the window / webview with this label (`app.emit_to`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    All,
    Label(&'a str),
}

/// Where an event goes besides the subscribers: production = the Tauri webviews.
pub trait Sink {
    fn send(&self, target: Target, name: &str, json: String) -> tauri::Result<()>;
}

struct TauriSink<'a>(&'a AppHandle);

impl Sink for TauriSink<'_> {
    fn send(&self, target: Target, name: &str, json: String) -> tauri::Result<()> {
        match target {
            Target::All => self.0.emit_str(name, json),
            Target::Label(l) => self.0.emit_str_to(l, name, json),
        }
    }
}

type Handler = Arc<dyn Fn(&Value) + Send + Sync>;

pub struct Bus {
    subs: RwLock<Vec<(u64, String, Handler)>>,
    count: AtomicUsize, // lets emit skip the lock (and the Value) when nobody subscribed
    next: AtomicU64,
}

impl Bus {
    pub const fn new() -> Self {
        Bus { subs: RwLock::new(Vec::new()), count: AtomicUsize::new(0), next: AtomicU64::new(1) }
    }

    pub fn subscribe(&self, name: &str, f: impl Fn(&Value) + Send + Sync + 'static) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut g = self.subs.write().unwrap_or_else(|e| e.into_inner());
        g.push((id, name.to_string(), Arc::new(f)));
        self.count.store(g.len(), Ordering::Release);
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        let mut g = self.subs.write().unwrap_or_else(|e| e.into_inner());
        g.retain(|s| s.0 != id);
        self.count.store(g.len(), Ordering::Release);
    }

    /// Calls the subscribers of `name` with the parsed payload. The handlers run after the lock is released (a handler may subscribe or emit).
    fn route(&self, name: &str, json: &str) {
        if self.count.load(Ordering::Acquire) == 0 {
            return;
        }
        let hs: Vec<Handler> = {
            let g = self.subs.read().unwrap_or_else(|e| e.into_inner());
            g.iter().filter(|s| s.1 == name).map(|s| s.2.clone()).collect()
        };
        if hs.is_empty() {
            return;
        }
        let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
        for h in hs {
            h(&v);
        }
    }
}

/// Forward to `sink`, then to the subscribers of `bus`. The result is the sink's (a failing webview must not hide an event from native parts).
pub fn emit_with<S: Serialize>(sink: &dyn Sink, bus: &Bus, target: Target, name: &str, payload: S) -> tauri::Result<()> {
    let json = serde_json::to_string(&payload)?;
    if bus.count.load(Ordering::Acquire) == 0 {
        return sink.send(target, name, json);
    }
    let r = sink.send(target, name, json.clone());
    bus.route(name, &json);
    r
}

static BUS: Bus = Bus::new();

/// The replacement of `app.emit_to(label, name, payload)` (`Target::Label`) and `app.emit(name, payload)` (`Target::All`).
pub fn emit<S: Serialize>(app: &AppHandle, target: Target, name: &str, payload: S) -> tauri::Result<()> {
    emit_with(&TauriSink(app), &BUS, target, name, payload)
}

/// Listen to events from native code. `f` gets the payload as JSON; it runs on the thread that emitted, so keep it short.
#[allow(dead_code)] // the native UI (later phases) subscribes; nothing does yet
pub fn subscribe(name: &str, f: impl Fn(&Value) + Send + Sync + 'static) -> u64 {
    BUS.subscribe(name, f)
}

#[allow(dead_code)]
pub fn unsubscribe(id: u64) {
    BUS.unsubscribe(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake(Mutex<Vec<(String, String, String)>>); // (target, name, json)
    impl Sink for Fake {
        fn send(&self, target: Target, name: &str, json: String) -> tauri::Result<()> {
            let t = match target {
                Target::All => "*".to_string(),
                Target::Label(l) => l.to_string(),
            };
            self.0.lock().unwrap().push((t, name.to_string(), json));
            Ok(())
        }
    }

    #[test]
    fn forwards_to_the_sink_with_target_name_and_json() {
        let (sink, bus) = (Fake::default(), Bus::new());
        emit_with(&sink, &bus, Target::Label("pill"), "pill:show", json!({ "reason": "tray" })).unwrap();
        emit_with(&sink, &bus, Target::All, "settings:changed", [1, 2]).unwrap();
        emit_with(&sink, &bus, Target::Label("bar"), "bar:shown", ()).unwrap();
        let got = sink.0.lock().unwrap().clone();
        assert_eq!(got[0], ("pill".into(), "pill:show".into(), r#"{"reason":"tray"}"#.into()));
        assert_eq!(got[1], ("*".into(), "settings:changed".into(), "[1,2]".into()));
        assert_eq!(got[2], ("bar".into(), "bar:shown".into(), "null".into()));
    }

    #[test]
    fn typed_payloads_keep_serde_json_text() {
        // an f32 must not take a detour through Value (0.123_f32 would become 0.12300000339746475)
        #[derive(Serialize)]
        struct Frame {
            rms: f32,
        }
        let (sink, bus) = (Fake::default(), Bus::new());
        emit_with(&sink, &bus, Target::Label("pill"), "audio:bands", Frame { rms: 0.123 }).unwrap();
        assert_eq!(sink.0.lock().unwrap()[0].2, r#"{"rms":0.123}"#);
    }

    #[test]
    fn subscribers_get_only_their_event() {
        let (sink, bus) = (Fake::default(), Bus::new());
        let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
        let (s1, s2) = (seen.clone(), seen.clone());
        let a = bus.subscribe("media:changed", move |v| s1.lock().unwrap().push(v.clone()));
        bus.subscribe("media:changed", move |v| s2.lock().unwrap().push(json!({ "second": v })));
        emit_with(&sink, &bus, Target::Label("pill"), "other:event", 1).unwrap();
        emit_with(&sink, &bus, Target::Label("pill"), "media:changed", json!({ "title": "x" })).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![json!({ "title": "x" }), json!({ "second": { "title": "x" } })]);
        assert_eq!(sink.0.lock().unwrap().len(), 2, "the webviews still get both events");

        bus.unsubscribe(a);
        seen.lock().unwrap().clear();
        emit_with(&sink, &bus, Target::All, "media:changed", 5).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![json!({ "second": 5 })]);
    }

    #[test]
    fn a_handler_may_emit_again() {
        let (sink, bus) = (Arc::new(Fake::default()), Arc::new(Bus::new()));
        let (s2, b2) = (sink.clone(), bus.clone());
        bus.subscribe("a", move |_| {
            emit_with(&*s2, &b2, Target::All, "b", 2).unwrap();
        });
        emit_with(&*sink, &bus, Target::All, "a", 1).unwrap();
        let names: Vec<String> = sink.0.lock().unwrap().iter().map(|e| e.1.clone()).collect();
        assert_eq!(names, ["a", "b"]);
    }
}
