//! What the Phone sub-tab knows about the phone beyond battery / network / DND: the optional `status` fields of docs/phone-protocol.md (v1.1),
//! the session counters the PC keeps, and the pure text helpers the card uses (durations, charge line, network line, dates, plurals). No drawing
//! and no clock here: every function takes the wall time it needs, so the tests are exact.
use super::phone::size_text;
use crate::panel::a::util::{clock_label, local, Wall};
use crate::panel::b::{Lang, Tr};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charge {
    None,
    Charging,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ringer {
    Sound,
    Vibrate,
    Silent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetKind {
    Wifi,
    Mobile,
    None,
}

/// The optional part of a `status`, as the PC sanitised it (`info` of a device). Every field may be unknown.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Info {
    pub charge: Option<Charge>,
    /// `ac` | `usb` | `wireless`
    pub plug: Option<String>,
    pub temp_c: Option<f32>,
    pub health: Option<String>,
    pub full_in_min: Option<u32>,
    /// (free, total) bytes
    pub storage: Option<(u64, u64)>,
    /// (free, total) MB
    pub ram: Option<(u64, u64)>,
    pub mbps: Option<u32>,
    pub bars: Option<u8>,
    pub carrier: Option<String>,
    pub gen: Option<String>,
    pub net_type: Option<NetKind>,
    pub airplane: Option<bool>,
    pub ringer: Option<Ringer>,
    pub screen: Option<bool>,
    pub tz: Option<String>,
    pub utc_min: Option<i32>,
}

/// Counters of the current PC session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Stats {
    pub notifs: u64,
    pub replies: u64,
    pub files_in: u64,
    pub files_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

fn u(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
}
fn st(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}
fn pair(v: &Value) -> Option<(u64, u64)> {
    let (f, t) = (u(&v["free"])?, u(&v["total"])?);
    (t > 0).then_some((f.min(t), t))
}

impl Info {
    pub fn parse(v: &Value) -> Info {
        let n = &v["net"];
        Info {
            charge: match v["charge"].as_str() {
                Some("none") => Some(Charge::None),
                Some("charging") => Some(Charge::Charging),
                Some("full") => Some(Charge::Full),
                _ => None,
            },
            plug: st(&v["plug"]),
            temp_c: v["tempC"].as_f64().map(|t| t as f32),
            health: st(&v["health"]),
            full_in_min: u(&v["fullInMin"]).map(|m| m.min(u32::MAX as u64) as u32),
            storage: pair(&v["storage"]),
            ram: pair(&v["ram"]),
            mbps: u(&n["mbps"]).map(|m| m as u32),
            bars: u(&n["bars"]).map(|b| b.min(4) as u8),
            carrier: st(&n["carrier"]),
            gen: st(&n["gen"]).filter(|g| g != "?"),
            net_type: match n["type"].as_str() {
                Some("wifi") => Some(NetKind::Wifi),
                Some("mobile") => Some(NetKind::Mobile),
                Some("none") => Some(NetKind::None),
                _ => None,
            },
            airplane: v["airplane"].as_bool(),
            ringer: match v["ringer"].as_str() {
                Some("sound") => Some(Ringer::Sound),
                Some("vibrate") => Some(Ringer::Vibrate),
                Some("silent") => Some(Ringer::Silent),
                _ => None,
            },
            screen: v["screen"].as_bool(),
            tz: st(&v["tz"]),
            utc_min: v["utcMin"].as_i64().map(|m| m as i32),
        }
    }

    /// Fraction of the storage in use (0..1), if known.
    pub fn storage_used(&self) -> Option<f32> {
        self.storage.map(|(f, t)| 1.0 - f as f32 / t as f32)
    }
    pub fn ram_used(&self) -> Option<f32> {
        self.ram.map(|(f, t)| 1.0 - f as f32 / t as f32)
    }
}

impl Stats {
    pub fn parse(v: &Value) -> Stats {
        Stats { notifs: u(&v["notifs"]).unwrap_or(0), replies: u(&v["replies"]).unwrap_or(0), files_in: u(&v["filesIn"]).unwrap_or(0), files_out: u(&v["filesOut"]).unwrap_or(0), bytes_in: u(&v["bytesIn"]).unwrap_or(0), bytes_out: u(&v["bytesOut"]).unwrap_or(0) }
    }
}

// ------------------------------------------------------------------------------------------------------------------------------ texts

/// Russian plural form picker: `forms` = one / few / many.
pub fn plural<'a>(n: u64, forms: [&'a str; 3]) -> &'a str {
    if n % 10 == 1 && n % 100 != 11 {
        forms[0]
    } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
        forms[1]
    } else {
        forms[2]
    }
}

/// "2 h 14 min", "14 min", "less than a minute", "1 d 3 h".
pub fn duration_text(ms: i64, l: Lang) -> String {
    let m = ms.max(0) / 60_000;
    let (d, h, mi) = (m / 1440, m / 60 % 24, m % 60);
    let (ud, uh, um) = (l.t("д", "d"), l.t("ч", "h"), l.t("мин", "min"));
    if d > 0 {
        format!("{d} {ud} {h} {uh}")
    } else if h > 0 {
        format!("{h} {uh} {mi} {um}")
    } else if m > 0 {
        format!("{m} {um}")
    } else {
        l.t("меньше минуты", "less than a minute").to_string()
    }
}

/// "5 min ago" / "3 h ago", "just now" below a minute; `None` when the time is unknown.
pub fn ago_text(wall: i64, then: i64, l: Lang) -> Option<String> {
    if then <= 0 {
        return None;
    }
    let m = (wall - then).max(0) / 60_000;
    Some(if m == 0 {
        l.t("только что", "just now").to_string()
    } else {
        let s = super::phone::ago(wall - then, l);
        format!("{s} {}", l.t("назад", "ago"))
    })
}

const MONTHS_RU: [&str; 12] = ["янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "30 окт 2026" / "30 Oct 2026" in the PC's time zone; `None` when unknown.
pub fn date_text(ms: i64, tz_min: i64, l: Lang) -> Option<String> {
    if ms <= 0 {
        return None;
    }
    let t = local(Wall::new(ms, tz_min));
    let m = if l == Lang::Ru { MONTHS_RU } else { MONTHS_EN };
    Some(format!("{} {} {}", t.day, m[t.month as usize - 1], t.year))
}

/// The phone's own clock "14:32" from its UTC offset.
pub fn phone_clock(wall: i64, utc_min: i32) -> String {
    clock_label(Wall::new(wall, utc_min as i64), false)
}

/// "Charging · AC · full in 42 min" / "Full" / "Not charging"; falls back to the plain `charging` flag of an old app.
pub fn charge_text(charge: Option<Charge>, charging: bool, i: &Info, l: Lang) -> String {
    let c = charge.unwrap_or(if charging { Charge::Charging } else { Charge::None });
    match c {
        Charge::Full => l.t("Заряжен", "Fully charged").to_string(),
        Charge::None => l.t("Не заряжается", "Not charging").to_string(),
        Charge::Charging => {
            let mut s = l.t("Заряжается", "Charging").to_string();
            match i.plug.as_deref() {
                Some("ac") => s += l.t(" · от сети", " · AC"),
                Some("usb") => s += " · USB",
                Some("wireless") => s += l.t(" · без провода", " · wireless"),
                _ => {}
            }
            if let Some(m) = i.full_in_min.filter(|m| *m > 0) {
                s += &format!(" · {} {}", l.t("до полной", "full in"), duration_text(m as i64 * 60_000, l));
            }
            s
        }
    }
}

pub fn health_text(h: &str, l: Lang) -> &'static str {
    match h {
        "good" => l.t("хорошее", "good"),
        "overheat" => l.t("перегрев", "overheated"),
        "cold" => l.t("холодная", "cold"),
        "dead" => l.t("изношена", "worn out"),
        "over_voltage" => l.t("перенапряжение", "over voltage"),
        "failure" => l.t("сбой", "failure"),
        _ => l.t("неизвестно", "unknown"),
    }
}

/// "31.5 °C · health good" (parts that are unknown are left out), `None` when both are.
pub fn battery_detail(i: &Info, l: Lang) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(t) = i.temp_c {
        parts.push(format!("{t:.1} °C"));
    }
    if let Some(h) = i.health.as_deref().filter(|h| *h != "unknown") {
        parts.push(format!("{} {}", l.t("состояние", "health"), health_text(h, l)));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What the network tile shows.
#[derive(Clone, Debug, PartialEq)]
pub struct NetView {
    pub kind: NetKind,
    pub title: String,
    pub detail: Option<String>,
    pub bars: Option<u8>,
}

/// The network tile from the old fields (`network`, `ssid`) plus the optional details; airplane mode wins over a stale connection type.
pub fn net_view(network: &str, ssid: Option<&str>, i: &Info, l: Lang) -> NetView {
    let kind = match network {
        "wifi" => NetKind::Wifi,
        "mobile" => NetKind::Mobile,
        _ => i.net_type.filter(|_| network.is_empty()).unwrap_or(NetKind::None),
    };
    match kind {
        _ if i.airplane == Some(true) => NetView { kind: NetKind::None, title: l.t("Режим полёта", "Airplane mode").to_string(), detail: None, bars: Some(0) },
        NetKind::Wifi => NetView { kind, title: ssid.unwrap_or("Wi-Fi").to_string(), detail: i.mbps.filter(|m| *m > 0).map(|m| format!("{m} {}", l.t("Мбит/с", "Mbps"))), bars: i.bars },
        NetKind::Mobile => {
            let title = i.carrier.clone().unwrap_or_else(|| l.t("Мобильная сеть", "Mobile data").to_string());
            NetView { kind, title, detail: i.gen.clone(), bars: i.bars }
        }
        NetKind::None => NetView { kind, title: l.t("Нет сети", "No network").to_string(), detail: None, bars: Some(0) },
    }
}

pub fn ringer_text(r: Ringer, l: Lang) -> &'static str {
    match r {
        Ringer::Sound => l.t("Звук", "Sound"),
        Ringer::Vibrate => l.t("Вибро", "Vibrate"),
        Ringer::Silent => l.t("Без звука", "Silent"),
    }
}

/// "12 notifications" (the number is drawn separately, animated; this is the word after it).
pub fn notifs_word(n: u64, l: Lang) -> &'static str {
    if l == Lang::Ru {
        plural(n, ["уведомление", "уведомления", "уведомлений"])
    } else if n == 1 {
        "notification"
    } else {
        "notifications"
    }
}

pub fn replies_word(n: u64, l: Lang) -> &'static str {
    if l == Lang::Ru {
        plural(n, ["ответ", "ответа", "ответов"])
    } else if n == 1 {
        "reply"
    } else {
        "replies"
    }
}

/// "2 files · 14.0 MB" tail of a file chip (the count is drawn animated before it): word and bytes.
pub fn files_tail(n: u64, bytes: u64, l: Lang) -> String {
    let w = crate::panel::a::util::files_word(l == Lang::Ru, n);
    if bytes == 0 {
        w.to_string()
    } else {
        format!("{w} · {}", size_text(bytes, l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn info_parses_what_is_there_and_nothing_else() {
        let i = Info::parse(&json!({ "charge": "charging", "plug": "usb", "tempC": 30.5, "health": "good", "fullInMin": 42, "storage": { "free": 25, "total": 100 }, "ram": { "free": 2000, "total": 8000 },
            "net": { "type": "wifi", "mbps": 866, "bars": 3, "gen": "?" }, "airplane": false, "ringer": "silent", "screen": true, "tz": "Europe/Moscow", "utcMin": 180 }));
        assert_eq!((i.charge, i.plug.as_deref(), i.full_in_min, i.storage, i.mbps, i.bars, i.gen.clone()), (Some(Charge::Charging), Some("usb"), Some(42), Some((25, 100)), Some(866), Some(3), None));
        assert_eq!((i.ringer, i.screen, i.airplane, i.utc_min, i.net_type), (Some(Ringer::Silent), Some(true), Some(false), Some(180), Some(NetKind::Wifi)));
        assert!((i.storage_used().unwrap() - 0.75).abs() < 1e-6 && (i.ram_used().unwrap() - 0.75).abs() < 1e-6);
        assert_eq!(Info::parse(&json!({})), Info::default());
        assert_eq!(Info::parse(&json!({ "storage": { "free": 1, "total": 0 } })).storage, None);
        assert_eq!(Stats::parse(&json!({ "notifs": 3, "filesIn": 2, "bytesIn": 2048 })), Stats { notifs: 3, files_in: 2, bytes_in: 2048, ..Stats::default() });
    }

    #[test]
    fn durations_and_ages() {
        assert_eq!(duration_text(134 * 60_000, Lang::En), "2 h 14 min");
        assert_eq!(duration_text(134 * 60_000, Lang::Ru), "2 ч 14 мин");
        assert_eq!(duration_text(14 * 60_000, Lang::En), "14 min");
        assert_eq!(duration_text(5_000, Lang::En), "less than a minute");
        assert_eq!(duration_text((26 * 60) * 60_000, Lang::En), "1 d 2 h");
        assert_eq!(ago_text(10 * 60_000, 5 * 60_000, Lang::En).as_deref(), Some("5 min ago"));
        assert_eq!(ago_text(10 * 60_000 + 30, 10 * 60_000, Lang::Ru).as_deref(), Some("только что"));
        assert_eq!(ago_text(10, 0, Lang::En), None);
    }

    #[test]
    fn dates_and_the_phones_own_clock() {
        // 2026-10-30 14:05:09 UTC
        let t = 1_793_369_109_000;
        assert_eq!(date_text(t, 0, Lang::En).as_deref(), Some("30 Oct 2026"));
        assert_eq!(date_text(t, 600, Lang::Ru).as_deref(), Some("31 окт 2026"), "ten hours east is already the next day");
        assert_eq!(phone_clock(t, 180), "17:05");
        assert_eq!(phone_clock(t, -300), "09:05");
    }

    #[test]
    fn charge_lines() {
        let mut i = Info { plug: Some("ac".into()), full_in_min: Some(75), ..Info::default() };
        assert_eq!(charge_text(Some(Charge::Charging), true, &i, Lang::En), "Charging · AC · full in 1 h 15 min");
        assert_eq!(charge_text(None, true, &Info::default(), Lang::En), "Charging", "an old app only has the flag");
        assert_eq!(charge_text(None, false, &Info::default(), Lang::Ru), "Не заряжается");
        assert_eq!(charge_text(Some(Charge::Full), true, &i, Lang::Ru), "Заряжен");
        i.plug = Some("wireless".into());
        assert!(charge_text(Some(Charge::Charging), true, &i, Lang::Ru).contains("без провода"));
        let d = Info { temp_c: Some(31.46), health: Some("good".into()), ..Info::default() };
        assert_eq!(battery_detail(&d, Lang::En).as_deref(), Some("31.5 °C · health good"));
        assert_eq!(battery_detail(&Info { health: Some("unknown".into()), ..Info::default() }, Lang::En), None);
    }

    #[test]
    fn the_network_tile() {
        let i = Info { mbps: Some(866), bars: Some(3), carrier: Some("Beeline".into()), gen: Some("5G".into()), ..Info::default() };
        let w = net_view("wifi", Some("Home"), &i, Lang::En);
        assert_eq!((w.kind, w.title.as_str(), w.detail.as_deref(), w.bars), (NetKind::Wifi, "Home", Some("866 Mbps"), Some(3)));
        assert_eq!(net_view("wifi", None, &Info::default(), Lang::En).title, "Wi-Fi");
        let m = net_view("mobile", None, &i, Lang::Ru);
        assert_eq!((m.title.as_str(), m.detail.as_deref()), ("Beeline", Some("5G")));
        assert_eq!(net_view("mobile", None, &Info::default(), Lang::Ru).title, "Мобильная сеть");
        assert_eq!(net_view("none", None, &Info::default(), Lang::En).title, "No network");
        let a = net_view("wifi", Some("Home"), &Info { airplane: Some(true), ..Info::default() }, Lang::En);
        assert_eq!((a.kind, a.title.as_str()), (NetKind::None, "Airplane mode"));
    }

    #[test]
    fn plurals_and_tails() {
        assert_eq!([1, 2, 5, 11, 21].map(|n| notifs_word(n, Lang::Ru)), ["уведомление", "уведомления", "уведомлений", "уведомлений", "уведомление"]);
        assert_eq!((notifs_word(1, Lang::En), notifs_word(0, Lang::En), replies_word(1, Lang::En), replies_word(2, Lang::Ru)), ("notification", "notifications", "reply", "ответа"));
        assert_eq!(files_tail(2, 14 * 1024 * 1024, Lang::En), "files · 14.0 MB");
        assert_eq!(files_tail(0, 0, Lang::Ru), "файлов");
        assert_eq!(ringer_text(Ringer::Vibrate, Lang::Ru), "Вибро");
    }
}
