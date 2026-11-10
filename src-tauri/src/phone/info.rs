// What the PC keeps about a phone beyond battery / network / DND: the optional `status` fields of docs/phone-protocol.md ("Changes by the phone
// side", v1.1) and the per-session counters. Everything the phone sends is untrusted: only known keys survive, enums are checked against a
// whitelist, numbers are clamped, strings are clipped and stripped of control characters. A phone that sends none of it (older app) yields `{}`.
use serde_json::{json, Map, Value};

const CHARGE: [&str; 3] = ["none", "charging", "full"];
const PLUG: [&str; 3] = ["ac", "usb", "wireless"];
const HEALTH: [&str; 7] = ["good", "overheat", "cold", "dead", "over_voltage", "failure", "unknown"];
const RINGER: [&str; 3] = ["sound", "vibrate", "silent"];
const GEN: [&str; 5] = ["2G", "3G", "4G", "5G", "?"];
const NET: [&str; 3] = ["wifi", "mobile", "none"];

fn clip(v: &Value, max: usize) -> Option<String> {
    let s: String = v.as_str()?.chars().filter(|c| !c.is_control()).take(max).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn pick(v: &Value, allowed: &[&str]) -> Option<Value> {
    let s = v.as_str()?;
    allowed.iter().find(|a| a.eq_ignore_ascii_case(s)).map(|a| json!(a))
}

fn int(v: &Value, lo: i64, hi: i64) -> Option<i64> {
    // JSON numbers may arrive as 31.0 or 3.1e1: accept any finite number, round it
    let f = v.as_f64().filter(|f| f.is_finite())?;
    Some((f.round() as i64).clamp(lo, hi))
}

fn pair(v: &Value, a: &str, b: &str, hi: i64) -> Option<Value> {
    let (x, y) = (int(&v[a], 0, hi)?, int(&v[b], 0, hi)?);
    (y > 0).then(|| json!({ a: x.min(y), b: y }))
}

fn net(n: &Value) -> Option<Value> {
    if !n.is_object() {
        return None;
    }
    let mut o = Map::new();
    if let Some(v) = pick(&n["type"], &NET) {
        o.insert("type".into(), v);
    }
    if let Some(v) = int(&n["mbps"], 0, 100_000) {
        o.insert("mbps".into(), json!(v));
    }
    if let Some(v) = int(&n["bars"], 0, 4) {
        o.insert("bars".into(), json!(v));
    }
    if let Some(v) = clip(&n["carrier"], 40) {
        o.insert("carrier".into(), json!(v));
    }
    if let Some(v) = pick(&n["gen"], &GEN) {
        o.insert("gen".into(), v);
    }
    (!o.is_empty()).then(|| Value::Object(o))
}

/// The sanitised optional fields of a `status` message as one JSON object (absent fields stay absent).
pub fn clean(m: &Value) -> Value {
    let mut o = Map::new();
    let mut put = |k: &str, v: Option<Value>| {
        if let Some(v) = v {
            o.insert(k.into(), v);
        }
    };
    put("charge", pick(&m["charge"], &CHARGE));
    put("plug", pick(&m["plug"], &PLUG));
    put("tempC", m["tempC"].as_f64().filter(|t| t.is_finite() && (-40.0..=120.0).contains(t)).map(|t| json!((t * 10.0).round() / 10.0)));
    put("health", pick(&m["health"], &HEALTH));
    put("fullInMin", int(&m["fullInMin"], 0, 7 * 24 * 60).map(|v| json!(v)));
    put("storage", pair(&m["storage"], "free", "total", i64::MAX / 4));
    put("ram", pair(&m["ram"], "free", "total", 4_000_000));
    put("net", net(&m["net"]));
    put("airplane", m["airplane"].as_bool().map(|b| json!(b)));
    put("ringer", pick(&m["ringer"], &RINGER));
    put("screen", m["screen"].as_bool().map(|b| json!(b)));
    put("tz", clip(&m["tz"], 40).map(|s| json!(s)));
    put("utcMin", int(&m["utcMin"], -14 * 60, 14 * 60).map(|v| json!(v)));
    Value::Object(o)
}

/// Counters of one phone for as long as the PC server runs (memory only).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// notifications mirrored (new ones, not replays or progress ticks)
    pub notifs: u64,
    /// replies the phone confirmed
    pub replies: u64,
    pub files_in: u64,
    pub files_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

impl Stats {
    pub fn json(&self) -> Value {
        json!({ "notifs": self.notifs, "replies": self.replies, "filesIn": self.files_in, "filesOut": self.files_out, "bytesIn": self.bytes_in, "bytesOut": self.bytes_out })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_status_survives_and_junk_is_dropped() {
        let c = clean(&json!({
            "t": "status", "battery": 80, "charge": "CHARGING", "plug": "usb", "tempC": 31.46, "health": "good", "fullInMin": 42.0,
            "storage": { "free": 50, "total": 128, "extra": 1 }, "ram": { "free": 3000, "total": 8000 },
            "net": { "type": "wifi", "mbps": 866, "bars": 3, "carrier": null, "gen": "5g", "x": 1 },
            "airplane": false, "ringer": "vibrate", "screen": true, "tz": "Europe/Moscow\u{7}", "utcMin": 180, "evil": "<script>"
        }));
        assert_eq!(
            c,
            json!({ "charge": "charging", "plug": "usb", "tempC": 31.5, "health": "good", "fullInMin": 42, "storage": { "free": 50, "total": 128 }, "ram": { "free": 3000, "total": 8000 },
                    "net": { "type": "wifi", "mbps": 866, "bars": 3, "gen": "5G" }, "airplane": false, "ringer": "vibrate", "screen": true, "tz": "Europe/Moscow", "utcMin": 180 })
        );
    }

    #[test]
    fn old_phones_and_nonsense_give_an_empty_object() {
        assert_eq!(clean(&json!({ "t": "status", "battery": 5, "charging": true })), json!({}));
        let c = clean(&json!({ "charge": "exploding", "plug": 3, "tempC": 900, "health": "great", "storage": { "free": 5, "total": 0 }, "ram": 7, "net": "wifi", "ringer": "loud", "utcMin": 99999 }));
        assert_eq!(c, json!({ "utcMin": 840 }), "an out-of-range offset is clamped, the rest is dropped: {c}");
    }

    #[test]
    fn free_never_exceeds_total_and_bars_are_clamped() {
        let c = clean(&json!({ "storage": { "free": 200, "total": 100 }, "net": { "bars": 9 } }));
        assert_eq!((c["storage"]["free"].as_i64(), c["net"]["bars"].as_i64()), (Some(100), Some(4)));
    }

    #[test]
    fn stats_json_uses_camel_case() {
        let s = Stats { notifs: 3, replies: 1, files_in: 2, files_out: 1, bytes_in: 10, bytes_out: 5 };
        assert_eq!(s.json(), json!({ "notifs": 3, "replies": 1, "filesIn": 2, "filesOut": 1, "bytesIn": 10, "bytesOut": 5 }));
    }
}
