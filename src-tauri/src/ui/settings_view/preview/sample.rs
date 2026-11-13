//! The backend of the preview island: a `ui::view::Api` that answers from SAMPLE data only. Nothing of the user's reaches the preview (no
//! real clipboard, notifications, location, files) and nothing the preview does reaches the real backend: every command is answered here
//! (or with `null`), never forwarded.
use crate::ui::view::Api;
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn wall_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The settings the preview island runs on (the window's own, filtered), served as `settings_get`.
pub type Shared = Arc<Mutex<Map<String, Value>>>;

pub struct SampleApi {
    pub settings: Shared,
}

fn is_true(m: &Map<String, Value>, key: &str, default: bool) -> bool {
    m.get(key).map(|v| *v == Value::Bool(true)).unwrap_or(default)
}

impl Api for SampleApi {
    fn call(&self, _caller: &str, name: &str, args: Value) -> Result<Value, String> {
        let st = self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Ok(match name {
            "settings_get" => Value::Object(st),
            "weather_get" => weather(is_true(&st, "net.weather", false)),
            "battery_state" => json!({ "has_battery": true, "percent": 76, "charging": false }),
            "clip_status" => json!({ "paused": false }),
            "clip_list" => {
                // the sample list, as long as the history limit says (never more than there are samples)
                let cap = st.get("clipboard.maxItems").and_then(Value::as_u64).unwrap_or(100) as usize;
                let mut v = clips(wall_ms());
                v.truncate(cap);
                Value::Array(v)
            }
            "clip_paste" => json!("copied"),
            _ => {
                let _ = args;
                Value::Null
            }
        })
    }
}

/// The weather as the weather tab and the folded island read it (`weather_get`): obviously fake place, plausible numbers.
pub fn weather(on: bool) -> Value {
    if !on {
        return json!({ "status": "off" });
    }
    let now = wall_ms();
    let hour = now - now.rem_euclid(3_600_000);
    let day = now - now.rem_euclid(86_400_000);
    let temps = [18.4, 19.0, 19.8, 20.4, 20.1, 19.2, 17.9, 16.5, 15.4, 14.6, 14.0, 13.6];
    let icons = ["partly-day", "partly-day", "clear-day", "clear-day", "cloud", "cloud", "rain", "rain", "partly-night", "clear-night", "clear-night", "partly-night"];
    let hourly: Vec<Value> = temps
        .iter()
        .enumerate()
        .map(|(i, t)| json!({ "t": hour + i as i64 * 3_600_000, "temp": t, "symbol": "partlycloudy_day", "icon": icons[i], "precip": if icons[i] == "rain" { 0.6 } else { 0.0 } }))
        .collect();
    let day_icons = ["partly-day", "clear-day", "cloud", "rain", "partly-day", "clear-day", "fog"];
    let daily: Vec<Value> = (0..7i64)
        .map(|i| {
            let t = day + i * 86_400_000;
            json!({ "t": t, "dow": ((t / 86_400_000) + 4).rem_euclid(7), "min": 11.0 + (i % 3) as f64, "max": 19.0 + ((i * 2) % 5) as f64, "symbol": "x", "icon": day_icons[i as usize], "precip": 0.0 })
        })
        .collect();
    json!({
        "status": "ok", "city": "Sample City", "lat": 0.0, "lon": 0.0, "fetched": now, "stale": false, "warning": null, "tz": crate::ui::island::sys::local_offset_min(),
        "now": { "temp": 18.4, "feels": 17.2, "symbol": "partlycloudy_day", "icon": "partly-day", "text": "Partly cloudy", "wind": 3.6, "dir": 225.0, "humidity": 62.0, "cloud": 40.0, "precip": 0.0, "min": 12.0, "max": 21.0, "pressure": 1014.0, "uv": 4.0, "uv_max": 6.0 },
        "hourly": hourly, "daily": daily,
        "sun": { "state": "normal", "rise": hour - 6 * 3_600_000, "set": hour + 6 * 3_600_000 }
    })
}

/// What the island's clock row shows next to the time (`weather:current`).
pub fn weather_now() -> Value {
    json!({ "temp": 18.4, "icon": "partly-day", "text": "Partly cloudy" })
}

/// The clipboard history of the preview: invented lines only.
pub fn clips(now: i64) -> Vec<Value> {
    let item = |id: i64, kind: &str, preview: &str, pinned: bool, source: &str, mins: i64, files: i64| {
        json!({ "id": id, "kind": kind, "preview": preview, "truncated": false, "fileCount": files, "pinned": pinned, "source": source, "usedAt": now - mins * 60_000 })
    };
    vec![
        item(1, "text", "Sample text: the quick brown fox jumps over the lazy dog", true, "Notes", 1, 0),
        item(2, "text", "https://example.com/sample-link", false, "Browser", 4, 0),
        item(3, "files", "sample-report.pdf", false, "Explorer", 9, 1),
        item(4, "text", "Meeting at 15:00, room 4. Bring the sample slides.", false, "Mail", 22, 0),
        item(5, "image", "", false, "Screenshot", 35, 0),
        item(6, "text", "SELECT * FROM sample_table LIMIT 10;", false, "Editor", 51, 0),
        item(7, "text", "Lorem ipsum dolor sit amet", false, "Notes", 90, 0),
        item(8, "files", "photo-1.jpg, photo-2.jpg", false, "Explorer", 130, 2),
        item(9, "text", "Sample address: 1 Example Street", false, "Browser", 180, 0),
        item(10, "text", "Sample password manager note (not a real secret)", false, "Notes", 240, 0),
        item(11, "text", "42", false, "Calculator", 300, 0),
        item(12, "text", "Another sample line", false, "Notes", 420, 0),
    ]
}

/// The events that make one kind of flash appear on the folded island, with invented payloads (never read from the system).
/// `(delay after the first, name, payload)` in ms.
pub fn flash_events(kind: super::scene::Flash, flip: bool) -> Vec<(f64, &'static str, Value)> {
    use super::scene::Flash::*;
    let now = wall_ms();
    match kind {
        Copy => vec![(0.0, "clip:added", json!({ "kind": "text", "fileCount": 0 }))],
        Kbd => vec![(0.0, "kbd:layout", json!({ "lang": "EN", "name": "English (United States)" }))],
        Vdesk => {
            let (a, b) = if flip { (2, 1) } else { (1, 2) };
            vec![(0.0, "vdesk:now", json!({ "index": a, "total": 3 })), (350.0, "vdesk:now", json!({ "index": b, "total": 3 }))]
        }
        Volume => vec![(0.0, "live:volume", json!({ "level": 62, "muted": false }))],
        Notif => vec![(
            0.0,
            "live:notification",
            json!({ "app": "Sample app", "title": "Sample sender", "body": "This is a sample notification. Your own notifications are never shown here.", "id": "sample" }),
        )],
        Battery => vec![(0.0, "live:battery", json!({ "kind": "low", "percent": 15 }))],
        Usb => vec![(0.0, "live:usb", json!({ "letter": "E", "added": true }))],
        Download => vec![(0.0, "live:file-done", json!({ "name": "sample-archive.zip", "path": "", "more": 0 }))],
        Reminder => vec![(0.0, "remind:due", json!({ "kind": "reminder", "text": "Sample reminder", "id": 1 }))],
        Recording => vec![(0.0, "rec:state", json!({ "recording": true, "startedAt": now - 12_000, "mode": "screen", "fps": 30 }))],
        Music => vec![(
            0.0,
            "media:changed",
            json!({ "has_session": true, "playing": true, "title": "Sample Track", "artist": "Sample Artist", "album": "", "session_id": "sample", "app": "Player.exe",
                "position_ms": 61000, "duration_ms": 243000, "thumb_id": 0, "accent": "#2a7bc8" }),
        )],
        Privacy => vec![(0.0, "live:privacy", json!({ "mic": true, "cam": false, "apps": ["Sample app"] }))],
        Toast => vec![(0.0, "capture:done", json!({ "kind": "screenshot", "path": "C:\\Sample\\Pictures\\sample-shot.png", "copied": true, "w": 1920, "h": 1080 }))],
    }
}
