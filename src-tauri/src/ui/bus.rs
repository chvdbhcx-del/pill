// In-process event bus: the one place every backend -> UI event goes through.
//   emit(name, payload)     hands the payload to the subscribers of that event name
//   subscribe(name, f) -> id  a native UI part listens to an event; f runs on the emitting thread, so it must be quick
// The payload is any `Serialize` value (`serde_json::Value` included): it is written to JSON once, by serde_json, and parsed back into a `Value`
// only when somebody subscribed to that event name.
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

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

/// Route the payload to the subscribers of `bus`.
pub fn emit_with<S: Serialize>(bus: &Bus, name: &str, payload: S) -> Result<(), serde_json::Error> {
    if bus.count.load(Ordering::Acquire) == 0 {
        return Ok(()); // nobody listens: not even serialised
    }
    bus.route(name, &serde_json::to_string(&payload)?);
    Ok(())
}

static BUS: Bus = Bus::new();

/// Send a backend event to the native UI.
pub fn emit<S: Serialize>(name: &str, payload: S) -> Result<(), serde_json::Error> {
    emit_with(&BUS, name, payload)
}

/// Listen to events from native code. `f` gets the payload as JSON; it runs on the thread that emitted, so keep it short.
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

    #[test]
    fn subscribers_get_only_their_event() {
        let bus = Bus::new();
        let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
        let (s1, s2) = (seen.clone(), seen.clone());
        let a = bus.subscribe("media:changed", move |v| s1.lock().unwrap().push(v.clone()));
        bus.subscribe("media:changed", move |v| s2.lock().unwrap().push(json!({ "second": v })));
        emit_with(&bus, "other:event", 1).unwrap();
        emit_with(&bus, "media:changed", json!({ "title": "x" })).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![json!({ "title": "x" }), json!({ "second": { "title": "x" } })]);

        bus.unsubscribe(a);
        seen.lock().unwrap().clear();
        emit_with(&bus, "media:changed", 5).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![json!({ "second": 5 })]);
    }

    #[test]
    fn a_handler_may_emit_again() {
        let bus = Arc::new(Bus::new());
        let b2 = bus.clone();
        let names: Arc<Mutex<Vec<String>>> = Arc::default();
        let n2 = names.clone();
        bus.subscribe("a", move |_| {
            emit_with(&b2, "b", 2).unwrap();
        });
        bus.subscribe("b", move |_| n2.lock().unwrap().push("b".into()));
        emit_with(&bus, "a", 1).unwrap();
        assert_eq!(*names.lock().unwrap(), ["b"]);
    }
}
