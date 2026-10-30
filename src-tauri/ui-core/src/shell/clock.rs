//! Collapsed clock row: time / date formatting, weather-icon side logic, battery chip, clock-row slots (`setClockSlots`), and the
//! re-render decision of `renderClock` (full re-render + swap fx vs in-place digit roll).
//!
//! Row layout (three columns, the time stays on the capsule axis): `[ left: weather icon | slot, date ]  [ time ]  [ right: battery, temp | slot ]`.
//! The weather icon goes to the side AWAY from the pet: with `CharSide::Right` (characters.js says the pet stands on the left) it moves to
//! the right column, otherwise it stays left; a column that holds a `setClockSlots` item keeps that item instead. Time text: 24 h `HH:MM[:SS]`, 12 h `H:MM[:SS]` without the AM/PM suffix
//! (the capsule stays narrow). Dates use the same short forms as `Intl.DateTimeFormat(ru-RU | en-GB, weekday/day/month short)` with `, .` removed.

use serde_json::Value;

use crate::shell::io::Local;
use crate::shell::settings::{js_round, Lang, ShellSettings};

/// `document.documentElement.dataset.charSide`: the side the pet / eyes stand on (set by characters.js).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CharSide {
    #[default]
    Unset,
    Left,
    Right,
}

/// `timeText`: 24 h -> `HH:MM`, 12 h -> `H:MM` (no AM/PM), `:SS` when seconds are shown.
pub fn time_text(l: &Local, clock24: bool, show_seconds: bool) -> String {
    let h = if clock24 {
        format!("{:02}", l.hour)
    } else {
        let h12 = l.hour % 12;
        format!("{}", if h12 == 0 { 12 } else { h12 })
    };
    let mut s = format!("{}:{:02}", h, l.minute);
    if show_seconds {
        s.push_str(&format!(":{:02}", l.second));
    }
    s
}

const RU_WD: [&str; 7] = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];
const RU_MO: [&str; 12] = ["янв", "февр", "мар", "апр", "мая", "июн", "июл", "авг", "сент", "окт", "нояб", "дек"];
const EN_WD: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const EN_MO: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sept", "Oct", "Nov", "Dec"];

/// `dateText`: "Fri 30 Oct" (en-GB) / "пт 30 окт" (ru-RU).
pub fn date_text(l: &Local, lang: Lang) -> String {
    let (wd, mo) = match lang {
        Lang::Ru => (RU_WD, RU_MO),
        Lang::En => (EN_WD, EN_MO),
    };
    format!("{} {} {}", wd[l.weekday as usize % 7], l.day, mo[(l.month as usize + 11) % 12])
}

/// Current conditions (event `weather:current` / `weather_get().now`).
#[derive(Debug, Clone, PartialEq)]
pub struct Weather {
    pub temp: f64,
    pub icon: Option<String>,
    pub code: Option<String>,
    pub text: Option<String>,
}

impl Weather {
    /// `setWx(n)`: `n.temp != null && isFinite(+n.temp)`, code = `n.code || n.symbol`.
    pub fn from_json(n: &Value) -> Option<Weather> {
        let temp = n.get("temp")?;
        if temp.is_null() {
            return None;
        }
        let t = crate::shell::settings::plus(temp);
        if !t.is_finite() {
            return None;
        }
        let s = |k: &str| n.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string());
        Some(Weather { temp: t, icon: s("icon"), code: s("code").or_else(|| s("symbol")), text: s("text") })
    }
}

/// weather icon ids from net.rs `icon_for`: clear-day clear-night partly-day partly-night cloud fog rain sleet snow thunder.
pub const WX_IDS: [&str; 10] = ["clear-day", "clear-night", "partly-day", "partly-night", "cloud", "fog", "rain", "sleet", "snow", "thunder"];

/// `WXI`: (icon name, extra class) of the monochrome fallback icon.
pub fn wx_glyph(id: &str) -> (&'static str, &'static str) {
    match id {
        "clear-day" => ("sun", "lp-sun"),
        "clear-night" => ("moon", ""),
        "partly-day" => ("weather", ""),
        "partly-night" => ("cloudmoon", ""),
        "cloud" | "fog" => ("cloud", ""),
        "rain" => ("rain", ""),
        "sleet" | "snow" => ("snow", ""),
        "thunder" => ("bolt", ""),
        _ => ("cloud", ""),
    }
}

/// `wxIconHtml`'s id resolution: a known `icon` id, else derived from `code || text`.
pub fn wx_icon_id(w: &Weather) -> &'static str {
    if let Some(i) = &w.icon {
        if let Some(k) = WX_IDS.iter().find(|k| **k == i.as_str()) {
            return k;
        }
    }
    let c = w.code.clone().or_else(|| w.text.clone()).unwrap_or_default().to_lowercase();
    let has = |ws: &[&str]| ws.iter().any(|x| c.contains(x));
    if has(&["thunder"]) {
        "thunder"
    } else if has(&["snow", "sleet"]) {
        "snow"
    } else if has(&["rain", "drizzle", "shower"]) {
        "rain"
    } else if has(&["fog"]) {
        "fog"
    } else if has(&["clear", "sun"]) {
        "clear-day"
    } else if has(&["part", "fair"]) {
        "partly-day"
    } else {
        "cloud"
    }
}

/// `battery_state` result.
#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    pub has_battery: bool,
    pub percent: f64,
    pub charging: bool,
}
impl Battery {
    pub fn from_json(v: &Value) -> Option<Battery> {
        let o = v.as_object()?;
        Some(Battery {
            has_battery: o.get("has_battery").map(crate::shell::settings::truthy).unwrap_or(false),
            percent: o.get("percent").map(crate::shell::settings::plus).filter(|x| x.is_finite()).unwrap_or(0.0),
            charging: o.get("charging").map(crate::shell::settings::truthy).unwrap_or(false),
        })
    }
}

/// Content a weather lane puts into a side column (`setClockSlots` took html; the native port takes data).
#[derive(Debug, Clone, PartialEq)]
pub enum SlotItem {
    WxIcon(String),
    Temp(i32),
    Text(String),
    Spec(Value),
}

/// `Pill.setClockSlots({ left, right })`; `None` = empty column slot.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClockSlots {
    pub left: Option<SlotItem>,
    pub right: Option<SlotItem>,
}

/// One element of a side column.
#[derive(Debug, Clone, PartialEq)]
pub enum ClockItem {
    /// coloured forecast icon (`id` is one of `WX_IDS`; `glyph` / `cls` the monochrome fallback)
    WxIcon { id: String, glyph: &'static str, cls: &'static str },
    Date(String),
    /// `low`: <= 15 % and not charging (red), `charging` (green)
    Battery { percent: f64, low: bool, charging: bool },
    /// `Math.round(wx.temp)` + a degree sign
    Temp(i32),
    Slot(SlotItem),
}

impl ClockItem {
    /// First CSS class of the DOM node (`n.className.split(' ')[0]`), the unit of the shape comparison.
    pub fn tag(&self) -> &'static str {
        match self {
            ClockItem::WxIcon { .. } => "lp-ico",
            ClockItem::Date(_) => "lp-date",
            ClockItem::Battery { .. } => "lp-batt",
            ClockItem::Temp(_) => "lp-aux",
            ClockItem::Slot(SlotItem::WxIcon(_)) => "lp-ico",
            ClockItem::Slot(SlotItem::Temp(_)) => "lp-aux",
            ClockItem::Slot(_) => "slot",
        }
    }
}

/// The clock row without the time digits (`sig` of the JS minus `%T%`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClockRow {
    pub left: Vec<ClockItem>,
    /// `S.showTime`
    pub show_time: bool,
    pub right: Vec<ClockItem>,
    /// the legs' eyes stand in their own empty side column (`.eic`)
    pub eic: bool,
    /// the pet's own side column is empty and collapses (`.pcl` / `.pcr`)
    pub pcl: bool,
    pub pcr: bool,
}

impl ClockRow {
    /// Tags of the shape-relevant items in DOM order (left column, then right).
    pub fn shape(&self) -> Vec<&'static str> {
        self.left.iter().chain(self.right.iter()).map(|i| i.tag()).collect()
    }
}

/// Everything `renderClock` reads to build the row.
pub struct ClockInputs<'a> {
    pub settings: &'a ShellSettings,
    pub lang: Lang,
    pub local: Local,
    pub wx: Option<&'a Weather>,
    pub battery: Option<&'a Battery>,
    pub slots: &'a ClockSlots,
    pub char_side: CharSide,
    /// `dataset.char === 'legs'`
    pub char_legs: bool,
}

/// Build the row: `[weather icon] time date [battery] temp` with the icon on the side away from the pet.
pub fn build_row(i: &ClockInputs) -> ClockRow {
    let s = i.settings;
    let show_icon = i.wx.is_some() && s.show_wx_icon; // the icon and the number are separate switches
    let show_deg = i.wx.is_some() && s.show_temp;
    let away_right = i.char_side == CharSide::Right; // the pet is on the left: the icon goes to the right, away from it
    let icon = || {
        let id = wx_icon_id(i.wx.unwrap()).to_string();
        let (glyph, cls) = wx_glyph(&id);
        ClockItem::WxIcon { id, glyph, cls }
    };
    let mut left = Vec::new();
    match &i.slots.left {
        Some(sl) => left.push(ClockItem::Slot(sl.clone())),
        None if !away_right && show_icon => left.push(icon()),
        None => {}
    }
    if s.show_date {
        left.push(ClockItem::Date(date_text(&i.local, i.lang)));
    }
    let mut right = Vec::new();
    if i.slots.right.is_none() && away_right && show_icon {
        right.push(icon());
    }
    if let Some(b) = i.battery {
        if b.has_battery && s.show_battery {
            right.push(ClockItem::Battery { percent: b.percent, low: b.percent <= 15.0 && !b.charging, charging: b.charging });
        }
    }
    match &i.slots.right {
        Some(sl) => right.push(ClockItem::Slot(sl.clone())),
        None if show_deg => right.push(ClockItem::Temp(js_round(i.wx.unwrap().temp) as i32)),
        None => {}
    }
    // the legs' eyes normally stand in a 30 px slot after the row; when their own side column is empty they stand in that column
    let eye_col_left = i.char_side == CharSide::Left;
    let eic = i.char_legs && if eye_col_left { left.is_empty() } else { right.is_empty() };
    // the pet's own side column, when empty, collapses (no hole between the pet and the time)
    let pet_left = s.has_pet() && i.char_side == CharSide::Right; // pc === '.lp-l'
    let pet_right = s.has_pet() && i.char_side != CharSide::Right; // pc === '.lp-r'
    let pcl = pet_left && left.is_empty();
    let pcr = pet_right && right.is_empty();
    ClockRow { left, show_time: s.show_time, right, eic, pcl, pcr }
}

/// One digit that changed in place (the digit rolls up when `animate`).
#[derive(Debug, Clone, PartialEq)]
pub struct DigitRoll {
    pub index: usize,
    pub from: char,
    pub to: char,
}

/// What the renderer has to do after `ClockState::render`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClockUpdate {
    /// Full re-render of the row. `swap`: crossfade (old content ghosts out, new blurs in). `entering`: side-column items that pop in
    /// (`PillMotion.enter y0 scale .6 dur .45`) as (is_right, index).
    Full { swap: bool, reshaped: bool, entering: Vec<(bool, usize)> },
    /// Same row, only the digits changed.
    Time { rolls: Vec<DigitRoll> },
    /// Same row, same time: nothing to do.
    Unchanged,
}

/// Remembered state of `renderClock`: `clockSig`, `clockShape`, `clockTime`.
#[derive(Debug, Clone, Default)]
pub struct ClockState {
    sig: Option<ClockRow>,
    shape: Vec<&'static str>,
    time: Option<String>,
}

impl ClockState {
    /// `invalidateLive()` (the clock part): the next render is a full one.
    pub fn invalidate(&mut self) {
        self.sig = None;
    }
    /// `clockSig = ''; clockTime = null` (a different live owner took the row).
    pub fn reset(&mut self) {
        self.sig = None;
        self.time = None;
    }
    pub fn time(&self) -> Option<&str> {
        self.time.as_deref()
    }
    pub fn row(&self) -> Option<&ClockRow> {
        self.sig.as_ref()
    }

    /// `renderClock` decision. `live_is_clock`: `liveKey === 'clock'` before this call. `animate`: digit roll allowed (`!showSeconds`,
    /// motion on, visible).
    pub fn render(&mut self, live_is_clock: bool, row: ClockRow, text: &str, animate: bool) -> ClockUpdate {
        if self.sig.as_ref() != Some(&row) || self.time.is_none() {
            let shape = row.shape();
            let reshaped = live_is_clock && self.time.is_some() && shape != self.shape;
            let had: Vec<&str> = self.shape.clone();
            let swap = !live_is_clock || reshaped;
            let mut entering = Vec::new();
            if reshaped {
                for (right, col) in [(false, &row.left), (true, &row.right)] {
                    for (idx, it) in col.iter().enumerate() {
                        if !had.contains(&it.tag()) {
                            entering.push((right, idx));
                        }
                    }
                }
            }
            self.shape = shape;
            self.time = if row.show_time { Some(text.to_string()) } else { None };
            self.sig = Some(row);
            return ClockUpdate::Full { swap, reshaped, entering };
        }
        // same row: only the digits
        let old = self.time.clone().unwrap_or_default();
        if old == text {
            return ClockUpdate::Unchanged;
        }
        let (o, n): (Vec<char>, Vec<char>) = (old.chars().collect(), text.chars().collect());
        self.time = Some(text.to_string());
        if o.len() != n.len() {
            // digit spans are rebuilt (no roll)
            return ClockUpdate::Time { rolls: Vec::new() };
        }
        let rolls = if animate {
            (0..n.len()).filter(|&i| o[i] != n[i]).map(|i| DigitRoll { index: i, from: o[i], to: n[i] }).collect()
        } else {
            Vec::new()
        };
        ClockUpdate::Time { rolls }
    }
}

/// Delay (ms) until the clock row must be rendered again: once a minute, once a second with seconds (`now` = `Date.now()`).
pub fn next_clock_render_ms(now_wall: i64, show_seconds: bool) -> i64 {
    if show_seconds {
        1000 - now_wall.rem_euclid(1000) + 15
    } else {
        60_000 - now_wall.rem_euclid(60_000) + 30
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(h: u32, m: u32, s: u32) -> Local {
        Local { hour: h, minute: m, second: s, ..Local::ymd_hm(2026, 10, 30, 0, 0) }
    }
    fn settings(v: Value) -> ShellSettings {
        ShellSettings::from_json(&v)
    }
    fn wx(temp: f64, icon: &str) -> Weather {
        Weather { temp, icon: Some(icon.into()), code: None, text: None }
    }
    fn row_with(s: &ShellSettings, w: Option<&Weather>, b: Option<&Battery>, slots: &ClockSlots, side: CharSide) -> ClockRow {
        build_row(&ClockInputs { settings: s, lang: Lang::En, local: at(13, 5, 0), wx: w, battery: b, slots, char_side: side, char_legs: false })
    }

    #[test]
    fn time_text_24h_and_12h() {
        assert_eq!(time_text(&at(9, 5, 7), true, false), "09:05");
        assert_eq!(time_text(&at(13, 5, 7), true, true), "13:05:07");
        assert_eq!(time_text(&at(13, 5, 7), false, false), "1:05");
        assert_eq!(time_text(&at(0, 5, 7), false, false), "12:05");
        assert_eq!(time_text(&at(12, 0, 0), false, true), "12:00:00");
        assert_eq!(time_text(&at(23, 59, 9), false, true), "11:59:09");
        assert_eq!(time_text(&at(0, 0, 0), true, false), "00:00");
    }

    #[test]
    fn date_text_ru_and_en() {
        let l = Local::ymd_hm(2026, 10, 30, 0, 0); // Friday
        assert_eq!(date_text(&l, Lang::En), "Fri 30 Oct");
        assert_eq!(date_text(&l, Lang::Ru), "пт 30 окт");
        let l = Local::ymd_hm(2026, 9, 1, 0, 0); // Tuesday
        assert_eq!(date_text(&l, Lang::En), "Tue 1 Sept");
        assert_eq!(date_text(&l, Lang::Ru), "вт 1 сент");
        assert_eq!(date_text(&Local::ymd_hm(2026, 1, 4, 0, 0), Lang::Ru), "вс 4 янв");
        assert_eq!(date_text(&Local::ymd_hm(2026, 5, 9, 0, 0), Lang::Ru), "сб 9 мая");
    }

    #[test]
    fn weather_parsing_and_icon_resolution() {
        assert_eq!(Weather::from_json(&json!({"temp": "21.5", "icon": "rain"})).unwrap().temp, 21.5);
        assert!(Weather::from_json(&json!({"temp": null})).is_none());
        assert!(Weather::from_json(&json!({"temp": "abc"})).is_none());
        assert!(Weather::from_json(&json!({})).is_none());
        assert_eq!(Weather::from_json(&json!({"temp": 3, "symbol": "snow-ish"})).unwrap().code.as_deref(), Some("snow-ish"));
        assert_eq!(wx_icon_id(&wx(1.0, "fog")), "fog");
        let by_code = |c: &str| wx_icon_id(&Weather { temp: 1.0, icon: Some("weird".into()), code: Some(c.into()), text: None });
        assert_eq!(by_code("Thunderstorm"), "thunder");
        assert_eq!(by_code("light sleet"), "snow");
        assert_eq!(by_code("Drizzle"), "rain");
        assert_eq!(by_code("shower"), "rain");
        assert_eq!(by_code("fog"), "fog");
        assert_eq!(by_code("Sunny"), "clear-day");
        assert_eq!(by_code("Partly cloudy"), "partly-day"); // "part" matches before the plain-cloud default
        assert_eq!(by_code("overcast"), "cloud");
        let by_text = Weather { temp: 1.0, icon: None, code: None, text: Some("Clear sky".into()) };
        assert_eq!(wx_icon_id(&by_text), "clear-day");
        assert_eq!(wx_glyph("clear-day"), ("sun", "lp-sun"));
        assert_eq!(wx_glyph("sleet"), ("snow", ""));
        assert_eq!(wx_glyph("thunder"), ("bolt", ""));
    }

    #[test]
    fn icon_goes_left_by_default_and_right_when_the_pet_is_on_the_right() {
        let s = settings(json!({}));
        let w = wx(16.4, "clear-day");
        let none = ClockSlots::default();
        let r = row_with(&s, Some(&w), None, &none, CharSide::Unset);
        assert!(matches!(r.left[0], ClockItem::WxIcon { .. }));
        assert_eq!(r.right, vec![ClockItem::Temp(16)]);
        // `awayRight` (charSide === 'right'): the JS comment says the pet is on the left; the icon moves to the right column
        let r = row_with(&s, Some(&w), None, &none, CharSide::Right);
        assert!(r.left.is_empty());
        assert!(matches!(r.right[0], ClockItem::WxIcon { .. }));
        assert_eq!(r.right[1], ClockItem::Temp(16));
        let r = row_with(&s, Some(&w), None, &none, CharSide::Left);
        assert!(matches!(r.left[0], ClockItem::WxIcon { .. }));
    }

    #[test]
    fn separate_switches_for_icon_and_number_and_no_weather() {
        let w = wx(-3.5, "snow");
        let none = ClockSlots::default();
        let r = row_with(&settings(json!({"showWxIcon": false})), Some(&w), None, &none, CharSide::Unset);
        assert!(r.left.is_empty());
        assert_eq!(r.right, vec![ClockItem::Temp(-3)]); // Math.round(-3.5) === -3
        let r = row_with(&settings(json!({"showTemp": false})), Some(&w), None, &none, CharSide::Unset);
        assert_eq!(r.right.len(), 0);
        assert_eq!(r.left.len(), 1);
        let r = row_with(&settings(json!({})), None, None, &none, CharSide::Unset);
        assert!(r.left.is_empty() && r.right.is_empty());
    }

    #[test]
    fn date_battery_and_slots() {
        let s = settings(json!({"showDate": true}));
        let w = wx(10.0, "cloud");
        let b = Battery { has_battery: true, percent: 12.0, charging: false };
        let r = row_with(&s, Some(&w), Some(&b), &ClockSlots::default(), CharSide::Unset);
        assert_eq!(r.left.len(), 2); // icon + date
        assert_eq!(r.left[1], ClockItem::Date("Fri 30 Oct".into()));
        assert_eq!(r.right[0], ClockItem::Battery { percent: 12.0, low: true, charging: false });
        assert_eq!(r.right[1], ClockItem::Temp(10));
        let b = Battery { has_battery: true, percent: 12.0, charging: true };
        let r = row_with(&s, None, Some(&b), &ClockSlots::default(), CharSide::Unset);
        assert_eq!(r.right[0], ClockItem::Battery { percent: 12.0, low: false, charging: true }); // charging is never "low"
        let b = Battery { has_battery: false, percent: 99.0, charging: false };
        assert!(row_with(&s, None, Some(&b), &ClockSlots::default(), CharSide::Unset).right.is_empty());
        let b = Battery { has_battery: true, percent: 50.0, charging: false };
        assert!(row_with(&settings(json!({"showBattery": false})), None, Some(&b), &ClockSlots::default(), CharSide::Unset).right.is_empty());
        // slots replace the weather items on their side; the icon then takes the other column only if that one is free
        let slots = ClockSlots { left: Some(SlotItem::Text("x".into())), right: Some(SlotItem::Temp(7)) };
        let r = row_with(&s, Some(&w), None, &slots, CharSide::Unset);
        assert_eq!(r.left[0], ClockItem::Slot(SlotItem::Text("x".into())));
        assert_eq!(r.right, vec![ClockItem::Slot(SlotItem::Temp(7))]);
        let r = row_with(&settings(json!({})), Some(&w), None, &slots, CharSide::Right);
        assert_eq!(r.right, vec![ClockItem::Slot(SlotItem::Temp(7))]); // right slot present: no icon there
    }

    #[test]
    fn eyes_and_pet_columns() {
        let s = settings(json!({"pet": "cat"}));
        let none = ClockSlots::default();
        // pet on the left (charSide right in the JS naming): its column is .lp-l; empty -> pcl
        let r = build_row(&ClockInputs { settings: &s, lang: Lang::En, local: at(1, 0, 0), wx: None, battery: None, slots: &none, char_side: CharSide::Right, char_legs: false });
        assert!(r.pcl && !r.pcr);
        let r = build_row(&ClockInputs { settings: &s, lang: Lang::En, local: at(1, 0, 0), wx: None, battery: None, slots: &none, char_side: CharSide::Unset, char_legs: false });
        assert!(r.pcr && !r.pcl);
        let r = build_row(&ClockInputs { settings: &settings(json!({})), lang: Lang::En, local: at(1, 0, 0), wx: None, battery: None, slots: &none, char_side: CharSide::Unset, char_legs: false });
        assert!(!r.pcl && !r.pcr);
        // legs: the eyes use their own empty side column
        let r = build_row(&ClockInputs { settings: &settings(json!({})), lang: Lang::En, local: at(1, 0, 0), wx: None, battery: None, slots: &none, char_side: CharSide::Left, char_legs: true });
        assert!(r.eic);
        let w = wx(5.0, "rain");
        let r = build_row(&ClockInputs { settings: &settings(json!({})), lang: Lang::En, local: at(1, 0, 0), wx: Some(&w), battery: None, slots: &none, char_side: CharSide::Left, char_legs: true });
        assert!(!r.eic); // the left column holds the weather icon
    }

    #[test]
    fn first_render_is_full_swap_then_digits_roll_in_place() {
        let s = settings(json!({}));
        let mut st = ClockState::default();
        let row = row_with(&s, None, None, &ClockSlots::default(), CharSide::Unset);
        let u = st.render(false, row.clone(), "13:05", true);
        assert_eq!(u, ClockUpdate::Full { swap: true, reshaped: false, entering: vec![] });
        assert_eq!(st.time(), Some("13:05"));
        assert_eq!(st.render(true, row.clone(), "13:05", true), ClockUpdate::Unchanged);
        let u = st.render(true, row.clone(), "13:06", true);
        assert_eq!(u, ClockUpdate::Time { rolls: vec![DigitRoll { index: 4, from: '5', to: '6' }] });
        // minute carry: two digits roll
        let u = st.render(true, row.clone(), "13:10", true);
        assert_eq!(u, ClockUpdate::Time { rolls: vec![DigitRoll { index: 3, from: '0', to: '1' }, DigitRoll { index: 4, from: '6', to: '0' }] });
        // animation off (seconds shown / reduced motion): the text just changes
        assert_eq!(st.render(true, row, "13:11", false), ClockUpdate::Time { rolls: vec![] });
    }

    #[test]
    fn length_change_rebuilds_without_roll() {
        let row = row_with(&settings(json!({})), None, None, &ClockSlots::default(), CharSide::Unset);
        let mut st = ClockState::default();
        st.render(false, row.clone(), "9:59", true);
        assert_eq!(st.render(true, row, "10:00", true), ClockUpdate::Time { rolls: vec![] });
    }

    #[test]
    fn reshape_crossfades_and_pops_only_the_new_pieces() {
        let s = settings(json!({}));
        let mut st = ClockState::default();
        let r1 = row_with(&s, None, None, &ClockSlots::default(), CharSide::Unset);
        st.render(false, r1, "13:05", true);
        // the weather arrives: icon (left) and temperature (right) are new
        let w = wx(12.0, "cloud");
        let r2 = row_with(&s, Some(&w), None, &ClockSlots::default(), CharSide::Unset);
        let u = st.render(true, r2.clone(), "13:05", true);
        assert_eq!(u, ClockUpdate::Full { swap: true, reshaped: true, entering: vec![(false, 0), (true, 0)] });
        // a changed number only (temperature 12 -> 13): full re-render but same shape: no swap, no pop
        let w2 = wx(13.0, "cloud");
        let r3 = row_with(&s, Some(&w2), None, &ClockSlots::default(), CharSide::Unset);
        let u = st.render(true, r3, "13:05", true);
        assert_eq!(u, ClockUpdate::Full { swap: false, reshaped: false, entering: vec![] });
        // the date switch is flipped on: only the date pops
        let s2 = settings(json!({"showDate": true}));
        let r4 = row_with(&s2, Some(&w2), None, &ClockSlots::default(), CharSide::Unset);
        let u = st.render(true, r4, "13:05", true);
        assert_eq!(u, ClockUpdate::Full { swap: true, reshaped: true, entering: vec![(false, 1)] });
    }

    #[test]
    fn invalidate_forces_a_full_pass_and_hidden_time_is_never_in_place() {
        let s = settings(json!({}));
        let row = row_with(&s, None, None, &ClockSlots::default(), CharSide::Unset);
        let mut st = ClockState::default();
        st.render(false, row.clone(), "13:05", true);
        st.invalidate();
        assert!(matches!(st.render(true, row, "13:05", true), ClockUpdate::Full { swap: false, .. }));
        // showTime off: no time element, so every render is a full pass (as in the JS)
        let off = row_with(&settings(json!({"showTime": false})), None, None, &ClockSlots::default(), CharSide::Unset);
        let mut st = ClockState::default();
        st.render(false, off.clone(), "13:05", true);
        assert_eq!(st.time(), None);
        assert!(matches!(st.render(true, off, "13:06", true), ClockUpdate::Full { swap: false, reshaped: false, .. }));
    }

    #[test]
    fn another_owner_resets_the_clock_state() {
        let row = row_with(&settings(json!({})), None, None, &ClockSlots::default(), CharSide::Unset);
        let mut st = ClockState::default();
        st.render(false, row.clone(), "13:05", true);
        st.reset();
        assert_eq!(st.time(), None);
        assert!(matches!(st.render(false, row, "13:05", true), ClockUpdate::Full { swap: true, .. }));
    }

    #[test]
    fn render_timer_aligns_to_the_next_minute_or_second() {
        // 12:00:20.500
        let now = 12 * 3_600_000 + 20_500;
        assert_eq!(next_clock_render_ms(now, false), 60_000 - 20_500 + 30);
        assert_eq!(next_clock_render_ms(now, true), 1000 - 500 + 15);
        assert_eq!(next_clock_render_ms(0, true), 1015);
    }

    #[test]
    fn battery_parsing() {
        let b = Battery::from_json(&json!({"has_battery": true, "percent": 87, "charging": true})).unwrap();
        assert_eq!((b.has_battery, b.percent, b.charging), (true, 87.0, true));
        assert!(Battery::from_json(&json!(null)).is_none());
        assert!(!Battery::from_json(&json!({})).unwrap().has_battery);
    }
}
