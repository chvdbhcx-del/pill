//! Weather tab: current conditions, 8 hourly slots, 7-day list with temperature range bars; off / choose-a-city / error states.
//! Port of `src/tab-weather.js` (the page never fetches anything itself: every call goes through Rust and the feature is opt-in).
//!
//! Backend: `settings_get`, `settings_set {patch}` (`net.weather`, `net.contact`, `weather.units`), `weather_get {force, tz}`
//! (status `off | nocity | ok | error`), `weather_search_city {query, lang}`, `weather_set_city {name, lat, lon}`.
//! Settings: `net.weather` (opt-in), `net.contact`, `weather.units` (`metric` | `imperial`), `clock24`.
//! Refresh: on show, the refresh button (forced), and every 5 minutes while the tab is visible (Rust serves its 30 min disk cache).

use super::anim::Anim;
use super::{bool_of, str_of, BackendCall, Cfg, Fx, FxQueue, Tr};
use serde_json::{json, Value};

/// Background refresh interval while the tab is visible.
pub const REFRESH_MS: f64 = 5.0 * 60.0 * 1000.0;
/// Shortest city query that is sent.
pub const MIN_QUERY: usize = 2;
/// How long the previous condition's gradient stays under the new one while it fades.
pub const GRADIENT_FADE_MS: f64 = 800.0;

/// JS `Math.round` (halves go up).
fn js_round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

#[derive(Clone, Debug, PartialEq)]
pub struct Now {
    pub temp: f64,
    pub feels: f64,
    pub symbol: String,
    pub icon: String,
    pub wind: f64,
    pub dir: f64,
    pub humidity: f64,
    pub precip: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hour {
    pub t: i64,
    pub temp: f64,
    pub icon: String,
    pub precip: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub dow: usize,
    pub min: f64,
    pub max: f64,
    pub icon: String,
    pub precip: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sun {
    pub state: String,
    pub rise: Option<i64>,
    pub set: Option<i64>,
}

/// An `ok` answer of `weather_get`.
#[derive(Clone, Debug, PartialEq)]
pub struct Forecast {
    pub city: String,
    pub now: Now,
    pub hourly: Vec<Hour>,
    pub daily: Vec<Day>,
    pub sun: Option<Sun>,
    pub stale: bool,
    pub warning: String,
}

fn f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

impl Forecast {
    pub fn from_json(v: &Value) -> Option<Forecast> {
        let n = v.get("now")?;
        Some(Forecast {
            city: str_of(v, "city").to_string(),
            now: Now {
                temp: f(n, "temp"),
                feels: f(n, "feels"),
                symbol: str_of(n, "symbol").to_string(),
                icon: str_of(n, "icon").to_string(),
                wind: f(n, "wind"),
                dir: f(n, "dir"),
                humidity: f(n, "humidity"),
                precip: f(n, "precip"),
                min: f(n, "min"),
                max: f(n, "max"),
            },
            hourly: v
                .get("hourly")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|h| Hour { t: h.get("t").and_then(Value::as_i64).unwrap_or(0), temp: f(h, "temp"), icon: str_of(h, "icon").to_string(), precip: f(h, "precip") }).collect())
                .unwrap_or_default(),
            daily: v
                .get("daily")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|d| Day { dow: f(d, "dow") as usize % 7, min: f(d, "min"), max: f(d, "max"), icon: str_of(d, "icon").to_string(), precip: f(d, "precip") }).collect())
                .unwrap_or_default(),
            sun: v.get("sun").filter(|s| s.is_object()).map(|s| Sun { state: str_of(s, "state").to_string(), rise: s.get("rise").and_then(Value::as_i64), set: s.get("set").and_then(Value::as_i64) }),
            stale: bool_of(v, "stale"),
            warning: str_of(v, "warning").to_string(),
        })
    }
}

/// Condition text for a MET symbol code (`cond`).
pub fn condition(tr: &dyn Tr, sym: &str) -> String {
    let (light, heavy, sh) = (sym.starts_with("light"), sym.starts_with("heavy"), sym.contains("showers"));
    let l = |ru: &str, en: &str| tr.t(ru, en);
    if sym.contains("thunder") {
        return l("Гроза", "Thunderstorm");
    }
    if sym.contains("sleet") {
        return if light { l("Небольшой мокрый снег", "Light sleet") } else { l("Дождь со снегом", "Sleet") };
    }
    if sym.contains("snow") {
        return if light {
            l("Небольшой снег", "Light snow")
        } else if heavy {
            l("Сильный снегопад", "Heavy snow")
        } else if sh {
            l("Снегопад", "Snow showers")
        } else {
            l("Снег", "Snow")
        };
    }
    if sym.contains("rain") {
        return if sh {
            if light {
                l("Небольшие ливни", "Light showers")
            } else if heavy {
                l("Сильные ливни", "Heavy showers")
            } else {
                l("Ливни", "Showers")
            }
        } else if light {
            l("Небольшой дождь", "Light rain")
        } else if heavy {
            l("Сильный дождь", "Heavy rain")
        } else {
            l("Дождь", "Rain")
        };
    }
    match sym {
        "clearsky" => l("Ясно", "Clear"),
        "fair" => l("Малооблачно", "Mostly clear"),
        "partlycloudy" => l("Переменная облачность", "Partly cloudy"),
        "fog" => l("Туман", "Fog"),
        _ => l("Пасмурно", "Cloudy"),
    }
}

const STOPS: [(f64, [f64; 3]); 7] = [
    (-10.0, [90.0, 160.0, 255.0]),
    (0.0, [110.0, 208.0, 224.0]),
    (8.0, [127.0, 217.0, 192.0]),
    (14.0, [185.0, 220.0, 140.0]),
    (20.0, [230.0, 215.0, 122.0]),
    (26.0, [245.0, 177.0, 95.0]),
    (34.0, [255.0, 123.0, 90.0]),
];

/// Temperature (always degrees C) to the colour of the range bars.
pub fn temp_color(c: f64) -> [u8; 3] {
    let px = |a: [f64; 3]| [a[0] as u8, a[1] as u8, a[2] as u8];
    if c <= STOPS[0].0 {
        return px(STOPS[0].1);
    }
    for i in 1..STOPS.len() {
        if c <= STOPS[i].0 {
            let (a, ca) = STOPS[i - 1];
            let (b, cb) = STOPS[i];
            let fr = (c - a) / (b - a);
            let mix = |k: usize| js_round(ca[k] + (cb[k] - ca[k]) * fr) as u8;
            return [mix(0), mix(1), mix(2)];
        }
    }
    px(STOPS[STOPS.len() - 1].1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Loading,
    Off,
    City,
    Error,
    Ok,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// "Enable" on the off card, with the optional MET contact typed next to it.
    Enable { contact: String },
    /// Header city button, "Change city" on the error card.
    ChangeCity,
    /// Search button / Enter in the city box.
    Search { query: String },
    /// A search result by index.
    PickCity(usize),
    /// "Cancel" on the city card (shown only when there is a forecast to go back to).
    CancelCity,
    Refresh,
    Retry,
    TurnOff,
    /// "°C" / "°F" in the footer: `metric` or `imperial`.
    SetUnits(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CityResult {
    pub name: String,
    pub detail: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stat {
    pub icon: &'static str,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HourView {
    pub label: String,
    pub icon: String,
    pub temp: String,
    pub precip: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayView {
    pub name: String,
    pub today: bool,
    pub icon: String,
    pub lo: String,
    pub hi: String,
    pub precip: String,
    /// Range bar: left offset and width in % of the track, colours of both ends.
    pub bar_left: f64,
    pub bar_width: f64,
    pub color_lo: [u8; 3],
    pub color_hi: [u8; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct OkView {
    /// Condition icon name of the card gradient (`wx-g-<icon>`).
    pub gradient: String,
    /// The previous condition's gradient, still fading out under the new one.
    pub ghost_gradient: Option<String>,
    pub city: String,
    pub city_title: String,
    pub refresh_title: String,
    pub spinning: bool,
    pub icon: String,
    pub temp: String,
    pub condition: String,
    pub feels_line: String,
    pub stats: Vec<Stat>,
    /// Sunrise / sunset clock texts, or the polar text when the sun does not rise / set.
    pub sun_rise: Option<String>,
    pub sun_set: Option<String>,
    pub sun_text: Option<String>,
    pub hours: Vec<HourView>,
    pub days: Vec<DayView>,
    pub metric: bool,
    pub source: String,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OffView {
    pub title: String,
    pub text: String,
    pub contact: String,
    pub contact_placeholder: String,
    pub contact_tooltip: String,
    pub enable: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CityView {
    pub title: String,
    pub text: String,
    pub placeholder: String,
    pub prefill: String,
    pub search: String,
    pub searching: bool,
    pub msg: String,
    pub results: Vec<CityResult>,
    pub cancel: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorView {
    pub title: String,
    pub text: String,
    pub retry: String,
    pub change_city: String,
    pub turn_off: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Loading { text: String },
    Off(OffView),
    City(CityView),
    Error(ErrorView),
    Ok(Box<OkView>),
}

pub struct Weather {
    cfg: Cfg,
    mode: Mode,
    data: Option<Forecast>,
    visible: bool,
    busy: bool,
    pending_force: bool,
    searching: bool,
    edit_city: bool,
    last_err: String,
    prev_g: String,
    ghost: Option<(String, f64)>,
    spinning: bool,
    msg: String,
    results: Vec<CityResult>,
    tz_min: i64,
    ru: bool,
    since_refresh: f64,
    fx: FxQueue,
}

impl Default for Weather {
    fn default() -> Self {
        Weather::new()
    }
}

impl Weather {
    pub fn new() -> Weather {
        Weather {
            cfg: Cfg::new(),
            mode: Mode::Loading,
            data: None,
            visible: false,
            busy: false,
            pending_force: false,
            searching: false,
            edit_city: false,
            last_err: String::new(),
            prev_g: String::new(),
            ghost: None,
            spinning: false,
            msg: String::new(),
            results: vec![],
            tz_min: 0,
            ru: false,
            since_refresh: 0.0,
            fx: FxQueue::default(),
        }
    }

    /// The PC's UTC offset in minutes (JS `-getTimezoneOffset()`); sent as `tz` and used for the local clock texts.
    pub fn set_tz_min(&mut self, tz: i64) {
        self.tz_min = tz;
    }
    /// UI language: the city search is sent with `lang: "ru"` or `"en"` (`ruLang()`).
    pub fn set_language_ru(&mut self, ru: bool) {
        self.ru = ru;
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.cfg.os_reduce = on;
    }
    pub fn take_fx(&mut self) -> Vec<Fx> {
        self.fx.take()
    }
    pub fn forecast(&self) -> Option<&Forecast> {
        self.data.as_ref()
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    fn metric(&self) -> bool {
        let u = self.cfg.str("weather.units");
        (if u.is_empty() { "metric" } else { u }) != "imperial"
    }

    /// `refresh(force)`: reload settings, then the forecast. No-op while a refresh runs.
    pub fn refresh(&mut self, force: bool) -> Vec<BackendCall> {
        if self.busy {
            return vec![];
        }
        self.busy = true;
        self.pending_force = force;
        vec![BackendCall::bare("settings_get")]
    }

    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.visible = true;
        self.since_refresh = 0.0;
        self.refresh(false)
    }

    pub fn on_hide(&mut self) {
        self.visible = false;
    }

    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        if name != "settings:changed" || !payload.is_object() {
            return vec![];
        }
        let was = self.cfg.get("net.weather").cloned();
        self.cfg.apply(payload);
        if self.visible && (was.as_ref() != payload.get("net.weather") || self.data.is_none()) {
            return self.refresh(false);
        }
        vec![]
    }

    pub fn on_result(&mut self, cmd: &str, args: &Value, res: &Value) -> Vec<BackendCall> {
        match cmd {
            "settings_get" => {
                if res.is_object() {
                    self.cfg.apply(res);
                }
                if !self.cfg.truthy("net.weather") {
                    self.data = None;
                    self.busy = false;
                    self.set_mode(Mode::Off);
                    return vec![];
                }
                if self.data.is_none() {
                    self.set_mode(Mode::Loading);
                }
                vec![BackendCall::new("weather_get", json!({ "force": self.pending_force, "tz": self.tz_min }))]
            }
            "weather_get" => {
                self.busy = false;
                self.spinning = false;
                self.on_forecast(res);
                vec![]
            }
            "settings_set" => {
                if res.is_object() {
                    self.cfg.apply(res);
                }
                // the follow-up depends on which patch it was
                let p = args.get("patch").cloned().unwrap_or(Value::Null);
                if p.get("net.weather") == Some(&json!(true)) {
                    return self.refresh(true);
                }
                if p.get("net.weather") == Some(&json!(false)) {
                    return self.refresh(false);
                }
                vec![]
            }
            "weather_search_city" => {
                self.searching = false;
                if res.get("status").and_then(Value::as_str) != Some("ok") {
                    self.msg = res.get("message").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| "\u{0}search-failed".to_string());
                    return vec![];
                }
                self.results = res
                    .get("results")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|x| CityResult { name: str_of(x, "name").to_string(), detail: str_of(x, "detail").to_string(), lat: f(x, "lat"), lon: f(x, "lon") }).collect())
                    .unwrap_or_default();
                self.msg = if self.results.is_empty() { "\u{0}nothing".to_string() } else { String::new() };
                vec![]
            }
            "weather_set_city" => {
                self.edit_city = false;
                self.data = None;
                self.set_mode(Mode::Loading);
                self.refresh(true)
            }
            _ => vec![],
        }
    }

    fn on_forecast(&mut self, r: &Value) {
        if r.is_null() {
            self.last_err = "\u{0}no-answer".to_string();
            return self.set_mode(Mode::Error);
        }
        match r.get("status").and_then(Value::as_str) {
            Some("ok") => match Forecast::from_json(r) {
                Some(d) => {
                    self.data = Some(d);
                    self.last_err.clear();
                    let m = if self.edit_city { Mode::City } else { Mode::Ok };
                    self.set_mode(m);
                }
                None => {
                    self.last_err = "error".to_string();
                    let m = if self.data.is_some() { Mode::Ok } else { Mode::Error };
                    self.set_mode(m);
                }
            },
            Some("nocity") => {
                self.data = None;
                self.set_mode(Mode::City);
            }
            Some("off") => {
                self.data = None;
                self.set_mode(Mode::Off);
            }
            _ => {
                self.last_err = r.get("message").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("error").to_string();
                let m = if self.data.is_some() { Mode::Ok } else { Mode::Error };
                self.set_mode(m);
            }
        }
    }

    fn set_mode(&mut self, m: Mode) {
        self.mode = m;
        if m == Mode::City {
            self.msg.clear();
            self.results.clear();
            self.searching = false;
        }
        if m == Mode::Ok {
            if let Some(d) = &self.data {
                let icon = d.now.icon.clone();
                if !self.prev_g.is_empty() && self.prev_g != icon && !self.cfg.reduce_motion() {
                    self.ghost = Some((self.prev_g.clone(), GRADIENT_FADE_MS));
                    self.fx.anim(false, "gradient", Anim::gradient_fade());
                }
                self.prev_g = icon;
            }
        }
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Enable { contact } => {
                let mut p = serde_json::Map::new();
                p.insert("net.weather".into(), json!(true));
                if !contact.trim().is_empty() {
                    p.insert("net.contact".into(), json!(contact.trim()));
                }
                vec![BackendCall::new("settings_set", json!({ "patch": p }))]
            }
            Action::ChangeCity => {
                self.edit_city = true;
                self.set_mode(Mode::City);
                vec![]
            }
            Action::CancelCity => {
                self.edit_city = false;
                self.set_mode(Mode::Ok);
                vec![]
            }
            Action::Search { query } => {
                let q = query.trim().to_string();
                if q.chars().count() < MIN_QUERY || self.searching {
                    return vec![];
                }
                self.searching = true;
                self.msg = "\u{0}searching".to_string();
                self.results.clear();
                vec![BackendCall::new("weather_search_city", json!({ "query": q, "lang": self.lang_arg() }))]
            }
            Action::PickCity(i) => match self.results.get(i) {
                Some(r) => vec![BackendCall::new("weather_set_city", json!({ "name": r.name, "lat": r.lat, "lon": r.lon }))],
                None => vec![],
            },
            Action::Refresh => {
                self.spinning = true;
                self.fx.anim(self.cfg.reduce_motion(), "refresh", Anim::spin());
                self.refresh(true)
            }
            Action::Retry => self.refresh(true),
            Action::TurnOff => vec![BackendCall::new("settings_set", json!({ "patch": { "net.weather": false } }))],
            Action::SetUnits(u) => vec![BackendCall::new("settings_set", json!({ "patch": { "weather.units": u } }))],
        }
    }

    fn lang_arg(&self) -> &'static str {
        if self.ru { "ru" } else { "en" }
    }

    /// Timers: the 5 minute refresh while visible, the gradient ghost.
    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        if let Some((_, left)) = self.ghost.as_mut() {
            *left -= dt_ms;
            if *left <= 0.0 {
                self.ghost = None;
            }
        }
        if !self.visible {
            return vec![];
        }
        self.since_refresh += dt_ms;
        if self.since_refresh >= REFRESH_MS {
            self.since_refresh = 0.0;
            return self.refresh(false);
        }
        vec![]
    }

    // ---------------------------------------------------------------- formatting
    fn temp(&self, c: f64) -> String {
        format!("{}°", js_round(if self.metric() { c } else { c * 9.0 / 5.0 + 32.0 }))
    }
    fn wind_text(&self, tr: &dyn Tr, ms: f64) -> String {
        if self.metric() {
            format!("{} {}", js_round(ms), tr.t("м/с", "m/s"))
        } else {
            format!("{} mph", js_round(ms * 2.23694))
        }
    }
    fn mm(&self, tr: &dyn Tr, v: f64) -> String {
        if v >= 0.1 {
            let s = format!("{:.1}", (v * 10.0).round() / 10.0); // toFixed(1): halves go up
            format!("{} {}", if tr.is_ru() { s.replace('.', ",") } else { s }, tr.t("мм", "mm"))
        } else {
            String::new()
        }
    }
    fn local_hm(&self, ms: i64) -> (u32, u32) {
        let l = super::util::local(super::Wall::new(ms, self.tz_min));
        (l.hour, l.min)
    }
    fn hm(&self, ms: i64) -> String {
        let (h, m) = self.local_hm(ms);
        if self.cfg.clock24() {
            format!("{:02}:{:02}", h, m)
        } else {
            format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, m, if h < 12 { "AM" } else { "PM" })
        }
    }
    fn hour_label(&self, ms: i64) -> String {
        let (h, _) = self.local_hm(ms);
        if self.cfg.clock24() {
            format!("{:02}:00", h)
        } else {
            format!("{} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" })
        }
    }

    fn msg_text(&self, tr: &dyn Tr) -> String {
        match self.msg.as_str() {
            "\u{0}search-failed" => tr.t("Поиск не удался", "Search failed"),
            "\u{0}nothing" => tr.t("Ничего не найдено", "Nothing found"),
            "\u{0}searching" => tr.t("Ищу…", "Searching…"),
            m => m.to_string(),
        }
    }

    pub fn view(&self, tr: &dyn Tr) -> View {
        match self.mode {
            Mode::Loading => View::Loading { text: tr.t("Загрузка…", "Loading…") },
            Mode::Off => View::Off(OffView {
                title: tr.t("Погода выключена", "Weather is off"),
                text: tr.t(
                    "Прогноз берётся у норвежского метеоинститута (api.met.no), города ищутся через OpenStreetMap Nominatim. Пока вы не включите, ничего не отправляется.",
                    "The forecast comes from the Norwegian Meteorological Institute (api.met.no) and cities are searched through OpenStreetMap Nominatim. Nothing is sent until you turn it on.",
                ),
                contact: self.cfg.str("net.contact").to_string(),
                contact_placeholder: tr.t("Контакт для MET (e-mail или сайт), по желанию", "Contact for MET (e-mail or site), optional"),
                contact_tooltip: tr.t("MET Norway просит указывать контакт в User-Agent. Отправляется только им.", "MET Norway asks for a contact in the User-Agent. Sent to them only."),
                enable: tr.t("Включить", "Enable"),
            }),
            Mode::City => View::City(CityView {
                title: tr.t("Выберите город", "Choose a city"),
                text: tr.t("Введите название и нажмите Enter: поиск идёт только по запросу.", "Type a name and press Enter: the search runs only on request."),
                placeholder: tr.t("Город", "City"),
                prefill: if self.edit_city { self.data.as_ref().map(|d| d.city.clone()).unwrap_or_default() } else { String::new() },
                search: tr.t("Найти", "Search"),
                searching: self.searching,
                msg: self.msg_text(tr),
                results: self.results.clone(),
                cancel: if self.data.is_some() { Some(tr.t("Отмена", "Cancel")) } else { None },
            }),
            Mode::Error => View::Error(ErrorView {
                title: tr.t("Не удалось получить прогноз", "Could not load the forecast"),
                text: if self.last_err == "\u{0}no-answer" { tr.t("Нет ответа от приложения", "No answer from the app") } else { self.last_err.clone() },
                retry: tr.t("Повторить", "Retry"),
                change_city: tr.t("Другой город", "Change city"),
                turn_off: tr.t("Выключить", "Turn off"),
            }),
            Mode::Ok => match &self.data {
                Some(d) => View::Ok(Box::new(self.ok_view(tr, d))),
                None => View::Loading { text: tr.t("Загрузка…", "Loading…") },
            },
        }
    }

    fn ok_view(&self, tr: &dyn Tr, d: &Forecast) -> OkView {
        let n = &d.now;
        let dirs: [&str; 8] = if tr.is_ru() { ["С", "СВ", "В", "ЮВ", "Ю", "ЮЗ", "З", "СЗ"] } else { ["N", "NE", "E", "SE", "S", "SW", "W", "NW"] };
        let dir = dirs[(js_round(n.dir / 45.0).rem_euclid(8)) as usize];
        let precip = self.mm(tr, n.precip);
        let dows: [&str; 7] = if tr.is_ru() { ["Вс", "Пн", "Вт", "Ср", "Чт", "Пт", "Сб"] } else { ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] };
        let gmin = d.daily.iter().map(|x| x.min).fold(f64::INFINITY, f64::min);
        let gmax = d.daily.iter().map(|x| x.max).fold(f64::NEG_INFINITY, f64::max);
        let span = (gmax - gmin).max(1.0);
        let (mut rise, mut set, mut text) = (None, None, None);
        match &d.sun {
            Some(s) if s.rise.is_some() => {
                rise = s.rise.map(|t| self.hm(t));
                set = s.set.map(|t| self.hm(t));
            }
            s => text = Some(if s.as_ref().map_or(false, |s| s.state == "polar_day") { tr.t("Полярный день", "Midnight sun") } else { tr.t("Полярная ночь", "Polar night") }),
        }
        let warn = if d.stale { tr.t("Нет сети, показан сохранённый прогноз · ", "Offline, showing the saved forecast · ") } else { String::new() };
        OkView {
            gradient: n.icon.clone(),
            ghost_gradient: self.ghost.as_ref().map(|(g, _)| g.clone()),
            city: d.city.clone(),
            city_title: tr.t("Сменить город", "Change city"),
            refresh_title: tr.t("Обновить", "Refresh"),
            spinning: self.spinning,
            icon: n.icon.clone(),
            temp: self.temp(n.temp),
            condition: condition(tr, &n.symbol),
            feels_line: format!("{}{} · {}…{}", tr.t("Ощущается как ", "Feels like "), self.temp(n.feels), self.temp(n.min), self.temp(n.max)),
            stats: vec![
                Stat { icon: "wind", text: format!("{} {}", self.wind_text(tr, n.wind), dir) },
                Stat { icon: "drop", text: format!("{}%", js_round(n.humidity)) },
                Stat { icon: "umbrella", text: if precip.is_empty() { format!("0 {}", tr.t("мм", "mm")) } else { precip } },
            ],
            sun_rise: rise,
            sun_set: set,
            sun_text: text,
            hours: d
                .hourly
                .iter()
                .enumerate()
                .map(|(i, h)| HourView {
                    label: if i == 0 { tr.t("Сейчас", "Now") } else { self.hour_label(h.t) },
                    icon: h.icon.clone(),
                    temp: self.temp(h.temp),
                    precip: self.mm(tr, h.precip),
                })
                .collect(),
            days: d
                .daily
                .iter()
                .enumerate()
                .map(|(i, x)| DayView {
                    name: if i == 0 { tr.t("Сегодня", "Today") } else { dows[x.dow].to_string() },
                    today: i == 0,
                    icon: x.icon.clone(),
                    lo: self.temp(x.min),
                    hi: self.temp(x.max),
                    precip: self.mm(tr, x.precip),
                    bar_left: (x.min - gmin) / span * 100.0,
                    bar_width: ((x.max - x.min) / span * 100.0).max(4.0),
                    color_lo: temp_color(x.min),
                    color_hi: temp_color(x.max),
                })
                .collect(),
            metric: self.metric(),
            source: format!("{}{}", warn, tr.t("Данные: MET Norway", "Data: MET Norway")),
            tooltip: d.warning.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    fn ok_payload() -> Value {
        json!({
            "status": "ok", "city": "Oslo", "lat": 59.9, "lon": 10.7, "fetched": 1, "stale": false, "warning": null, "tz": 60,
            "now": { "temp": 4.4, "feels": 1.6, "symbol": "lightrain", "icon": "rain", "wind": 5.2, "dir": 225.0, "humidity": 81.0, "cloud": 90.0, "precip": 0.4, "min": 2.1, "max": 7.0 },
            "hourly": [
                { "t": 1_793_368_800_000i64, "temp": 4.4, "symbol": "lightrain", "icon": "rain", "precip": 0.4 },
                { "t": 1_793_372_400_000i64, "temp": 5.0, "symbol": "cloudy", "icon": "cloud", "precip": 0.0 }],
            "daily": [
                { "t": 0, "dow": 5, "min": 2.0, "max": 7.0, "symbol": "rain", "icon": "rain", "precip": 3.25 },
                { "t": 0, "dow": 6, "min": -3.0, "max": 1.0, "symbol": "snow", "icon": "snow", "precip": 0.0 }],
            "sun": { "state": "normal", "rise": 1_793_338_200_000i64, "set": 1_793_370_600_000i64 }
        })
    }

    fn started(on: bool) -> Weather {
        let mut w = Weather::new();
        w.on_event("settings:changed", &json!({ "net.weather": on, "clock24": true }));
        w.on_show();
        w
    }

    /// Drive show -> settings_get -> weather_get with an answer.
    fn loaded() -> Weather {
        let mut w = Weather::new();
        w.set_tz_min(0);
        let c = w.on_show();
        assert_eq!(c, vec![BackendCall::bare("settings_get")]);
        let c = w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true, "clock24": true }));
        assert_eq!(c, vec![BackendCall::new("weather_get", json!({ "force": false, "tz": 0 }))]);
        w.on_result("weather_get", &Value::Null, &ok_payload());
        w
    }

    #[test]
    fn rounding_matches_js() {
        assert_eq!(js_round(2.5), 3);
        assert_eq!(js_round(-2.5), -2);
        assert_eq!(js_round(-0.4), 0);
        assert_eq!(js_round(4.4), 4);
    }

    #[test]
    fn opt_in_off_shows_the_enable_card_and_sends_nothing() {
        let mut w = Weather::new();
        w.on_show();
        let c = w.on_result("settings_get", &Value::Null, &json!({ "net.weather": false }));
        assert!(c.is_empty());
        assert!(!w.is_busy());
        match w.view(&EnTr) {
            View::Off(o) => assert_eq!(o.enable, "Enable"),
            v => panic!("{v:?}"),
        }
        assert!(matches!(w.view(&RuTr), View::Off(_)));
    }

    #[test]
    fn enable_patches_settings_with_contact_then_forces_a_refresh() {
        let mut w = Weather::new();
        let c = w.on_action(Action::Enable { contact: "  me@x.org ".into() });
        assert_eq!(c, vec![BackendCall::new("settings_set", json!({ "patch": { "net.weather": true, "net.contact": "me@x.org" } }))]);
        let c = w.on_action(Action::Enable { contact: "   ".into() });
        assert_eq!(c[0].args, json!({ "patch": { "net.weather": true } }));
        let c = w.on_result("settings_set", &c[0].args, &json!({ "net.weather": true }));
        assert_eq!(c, vec![BackendCall::bare("settings_get")]);
        let c = w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        assert_eq!(c[0].args["force"], true);
    }

    #[test]
    fn ok_view_formats_metric() {
        let w = loaded();
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert_eq!(v.temp, "4°");
        assert_eq!(v.condition, "Light rain");
        assert_eq!(v.feels_line, "Feels like 2° · 2°…7°");
        assert_eq!(v.stats[0].text, "5 m/s SW"); // 225 deg = SW
        assert_eq!(v.stats[1].text, "81%");
        assert_eq!(v.stats[2].text, "0.4 mm");
        assert_eq!((v.sun_rise.as_deref(), v.sun_set.as_deref()), (Some("05:30"), Some("14:30")));
        assert_eq!(v.hours[0].label, "Now");
        assert_eq!(v.hours[1].label, "15:00");
        assert_eq!(v.hours[0].precip, "0.4 mm");
        assert_eq!(v.hours[1].precip, "");
        assert_eq!(v.days[0].name, "Today");
        assert_eq!(v.days[1].name, "Sat");
        assert_eq!(v.source, "Data: MET Norway");
        assert!(v.metric);
    }

    #[test]
    fn russian_texts_and_decimal_comma() {
        let w = loaded();
        let View::Ok(v) = w.view(&RuTr) else { panic!() };
        assert_eq!(v.condition, "Небольшой дождь");
        assert_eq!(v.stats[0].text, "5 м/с ЮЗ");
        assert_eq!(v.stats[2].text, "0,4 мм");
        assert_eq!(v.days[0].precip, "3,3 мм");
        assert_eq!(v.days[1].name, "Сб");
        assert_eq!(v.feels_line, "Ощущается как 2° · 2°…7°");
    }

    #[test]
    fn imperial_units_and_12h_clock() {
        let mut w = loaded();
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true, "weather.units": "imperial", "clock24": false }));
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert_eq!(v.temp, "40°"); // 4.4 C
        assert_eq!(v.stats[0].text, "12 mph SW");
        assert_eq!(v.sun_rise.as_deref(), Some("5:30 AM"));
        assert_eq!(v.sun_set.as_deref(), Some("2:30 PM"));
        assert_eq!(v.hours[1].label, "3 PM");
        assert!(!v.metric);
        assert_eq!(v.days[1].lo, "27°"); // -3 C
    }

    #[test]
    fn units_toggle_sends_a_settings_patch() {
        let mut w = loaded();
        assert_eq!(w.on_action(Action::SetUnits("imperial")), vec![BackendCall::new("settings_set", json!({ "patch": { "weather.units": "imperial" } }))]);
        assert!(w.on_result("settings_set", &json!({ "patch": { "weather.units": "imperial" } }), &json!({ "net.weather": true, "weather.units": "imperial" })).is_empty());
    }

    #[test]
    fn range_bars_scale_to_the_whole_week() {
        let w = loaded();
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        // week spans -3 .. 7 = 10 degrees
        assert_eq!(v.days[0].bar_left, 50.0);
        assert_eq!(v.days[0].bar_width, 50.0);
        assert_eq!(v.days[1].bar_left, 0.0);
        assert_eq!(v.days[1].bar_width, 40.0);
        assert_eq!(v.days[1].color_lo, temp_color(-3.0));
        // a one-degree day still gets a visible bar (min width 4 %)
        let mut p = ok_payload();
        p["daily"] = json!([{ "dow": 1, "min": 0.0, "max": 0.1, "icon": "cloud", "precip": 0.0 }, { "dow": 2, "min": 0.0, "max": 20.0, "icon": "cloud", "precip": 0.0 }]);
        let mut w2 = loaded();
        w2.on_result("weather_get", &Value::Null, &p);
        let View::Ok(v2) = w2.view(&EnTr) else { panic!() };
        assert_eq!(v2.days[0].bar_width, 4.0);
    }

    #[test]
    fn temperature_colours_interpolate_between_stops() {
        assert_eq!(temp_color(-50.0), [90, 160, 255]);
        assert_eq!(temp_color(0.0), [110, 208, 224]);
        assert_eq!(temp_color(4.0), [119, 213, 208]);
        assert_eq!(temp_color(34.0), [255, 123, 90]);
        assert_eq!(temp_color(99.0), [255, 123, 90]);
    }

    #[test]
    fn condition_texts_cover_the_symbol_families() {
        let c = |s: &str| condition(&EnTr, s);
        assert_eq!(c("heavyrainandthunder"), "Thunderstorm");
        assert_eq!(c("lightsleet"), "Light sleet");
        assert_eq!(c("sleetshowers"), "Sleet");
        assert_eq!(c("heavysnow"), "Heavy snow");
        assert_eq!(c("snowshowers_day"), "Snow showers");
        assert_eq!(c("lightrainshowers_night"), "Light showers");
        assert_eq!(c("heavyrain"), "Heavy rain");
        assert_eq!(c("rain"), "Rain");
        assert_eq!(c("clearsky"), "Clear");
        assert_eq!(c("fair"), "Mostly clear");
        assert_eq!(c("partlycloudy"), "Partly cloudy");
        assert_eq!(c("fog"), "Fog");
        assert_eq!(c("cloudy"), "Cloudy");
        assert_eq!(condition(&RuTr, "heavysnow"), "Сильный снегопад");
    }

    #[test]
    fn polar_sun_states() {
        let mut p = ok_payload();
        p["sun"] = json!({ "state": "polar_day", "rise": null, "set": null });
        let mut w = loaded();
        w.on_result("weather_get", &Value::Null, &p);
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert_eq!(v.sun_text.as_deref(), Some("Midnight sun"));
        p["sun"] = json!({ "state": "polar_night", "rise": null, "set": null });
        w.on_result("weather_get", &Value::Null, &p);
        let View::Ok(v) = w.view(&RuTr) else { panic!() };
        assert_eq!(v.sun_text.as_deref(), Some("Полярная ночь"));
        assert!(v.sun_rise.is_none());
    }

    #[test]
    fn stale_forecast_warns() {
        let mut p = ok_payload();
        p["stale"] = json!(true);
        p["warning"] = json!("no network");
        let mut w = loaded();
        w.on_result("weather_get", &Value::Null, &p);
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert_eq!(v.source, "Offline, showing the saved forecast · Data: MET Norway");
        assert_eq!(v.tooltip, "no network");
    }

    #[test]
    fn nocity_leads_to_the_city_card_and_search_flow() {
        let mut w = Weather::new();
        w.on_show();
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &json!({ "status": "nocity" }));
        let View::City(c) = w.view(&EnTr) else { panic!() };
        assert!(c.cancel.is_none());
        assert_eq!(c.prefill, "");
        // too short or already searching: nothing
        assert!(w.on_action(Action::Search { query: " a ".into() }).is_empty());
        let call = w.on_action(Action::Search { query: " Oslo ".into() });
        assert_eq!(call, vec![BackendCall::new("weather_search_city", json!({ "query": "Oslo", "lang": "en" }))]);
        assert!(w.on_action(Action::Search { query: "Oslo".into() }).is_empty());
        let View::City(c) = w.view(&EnTr) else { panic!() };
        assert!(c.searching);
        assert_eq!(c.msg, "Searching…");
        let found = json!({ "status": "ok", "results": [{ "name": "Oslo", "detail": "Norway", "lat": 59.91, "lon": 10.75 }] });
        w.on_result("weather_search_city", &Value::Null, &found);
        let View::City(c) = w.view(&EnTr) else { panic!() };
        assert!(!c.searching);
        assert_eq!((c.msg.as_str(), c.results.len()), ("", 1));
        assert_eq!(w.on_action(Action::PickCity(5)), vec![]);
        let pick = w.on_action(Action::PickCity(0));
        assert_eq!(pick, vec![BackendCall::new("weather_set_city", json!({ "name": "Oslo", "lat": 59.91, "lon": 10.75 }))]);
        let c = w.on_result("weather_set_city", &Value::Null, &json!(true));
        assert_eq!(c, vec![BackendCall::bare("settings_get")]);
        assert!(matches!(w.view(&EnTr), View::Loading { .. }));
    }

    #[test]
    fn search_messages() {
        let mut w = Weather::new();
        w.on_show();
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &json!({ "status": "nocity" }));
        w.on_action(Action::Search { query: "zzzz".into() });
        w.on_result("weather_search_city", &Value::Null, &json!({ "status": "ok", "results": [] }));
        let View::City(c) = w.view(&RuTr) else { panic!() };
        assert_eq!(c.msg, "Ничего не найдено");
        w.on_action(Action::Search { query: "zzzz".into() });
        w.on_result("weather_search_city", &Value::Null, &json!({ "status": "error", "message": "HTTP 503" }));
        let View::City(c) = w.view(&RuTr) else { panic!() };
        assert_eq!(c.msg, "HTTP 503");
        w.on_action(Action::Search { query: "zzzz".into() });
        w.on_result("weather_search_city", &Value::Null, &Value::Null);
        let View::City(c) = w.view(&EnTr) else { panic!() };
        assert_eq!(c.msg, "Search failed");
    }

    #[test]
    fn changing_city_prefills_and_cancel_returns() {
        let mut w = loaded();
        w.on_action(Action::ChangeCity);
        let View::City(c) = w.view(&EnTr) else { panic!() };
        assert_eq!(c.prefill, "Oslo");
        assert_eq!(c.cancel.as_deref(), Some("Cancel"));
        // a refresh arriving while editing keeps the city card
        w.refresh(false);
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &ok_payload());
        assert!(matches!(w.view(&EnTr), View::City(_)));
        w.on_action(Action::CancelCity);
        assert!(matches!(w.view(&EnTr), View::Ok(_)));
    }

    #[test]
    fn errors_keep_the_old_forecast_or_show_the_card() {
        let mut w = Weather::new();
        w.on_show();
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &Value::Null);
        let View::Error(e) = w.view(&RuTr) else { panic!() };
        assert_eq!(e.text, "Нет ответа от приложения");
        assert!(!w.is_busy());
        w.refresh(true);
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &json!({ "status": "error", "message": "HTTP 500" }));
        let View::Error(e) = w.view(&EnTr) else { panic!() };
        assert_eq!(e.text, "HTTP 500");
        // with data on screen an error leaves the forecast in place
        let mut w = loaded();
        w.refresh(true);
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &json!({ "status": "error" }));
        assert!(matches!(w.view(&EnTr), View::Ok(_)));
    }

    #[test]
    fn turn_off_and_retry() {
        let mut w = loaded();
        assert_eq!(w.on_action(Action::TurnOff), vec![BackendCall::new("settings_set", json!({ "patch": { "net.weather": false } }))]);
        let c = w.on_result("settings_set", &json!({ "patch": { "net.weather": false } }), &json!({ "net.weather": false }));
        assert_eq!(c, vec![BackendCall::bare("settings_get")]);
        assert_eq!(w.on_action(Action::Retry), vec![]); // busy: ignored (the refresh above is running)
    }

    #[test]
    fn refresh_is_single_flight_and_spins_until_done() {
        let mut w = loaded();
        let c = w.on_action(Action::Refresh);
        assert_eq!(c, vec![BackendCall::bare("settings_get")]);
        assert!(w.on_action(Action::Refresh).is_empty());
        assert!(w.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "refresh")));
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert!(v.spinning);
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &ok_payload());
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert!(!v.spinning);
    }

    #[test]
    fn five_minute_timer_runs_only_while_visible() {
        let mut w = loaded();
        w.take_fx();
        assert!(w.tick(299_000.0).is_empty());
        assert_eq!(w.tick(1_500.0), vec![BackendCall::bare("settings_get")]);
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &ok_payload());
        w.on_hide();
        assert!(w.tick(10_000_000.0).is_empty());
        // showing again restarts the interval and refreshes at once
        assert_eq!(w.on_show().len(), 1);
    }

    #[test]
    fn settings_change_triggers_refresh_only_when_visible_and_relevant() {
        let mut w = started(true);
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &ok_payload());
        assert!(w.on_event("settings:changed", &json!({ "net.weather": true, "size": 120 })).is_empty()); // unchanged
        assert_eq!(w.on_event("settings:changed", &json!({ "net.weather": false })).len(), 1);
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": false }));
        w.on_hide();
        assert!(w.on_event("settings:changed", &json!({ "net.weather": true })).is_empty()); // hidden
    }

    #[test]
    fn condition_change_keeps_the_old_gradient_for_800ms() {
        let mut w = loaded();
        w.take_fx();
        let mut p = ok_payload();
        p["now"]["icon"] = json!("clear-day");
        w.on_result("weather_get", &Value::Null, &p);
        let fx = w.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "gradient" && anim.dur_ms == 800)));
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert_eq!((v.gradient.as_str(), v.ghost_gradient.as_deref()), ("clear-day", Some("rain")));
        w.tick(801.0);
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert!(v.ghost_gradient.is_none());
        // same condition again: no cross-fade
        w.on_result("weather_get", &Value::Null, &p);
        assert!(w.take_fx().is_empty());
    }

    #[test]
    fn reduced_motion_skips_the_gradient_fade() {
        let mut w = loaded();
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true, "reduceMotion": true }));
        w.take_fx();
        let mut p = ok_payload();
        p["now"]["icon"] = json!("snow");
        w.on_result("weather_get", &Value::Null, &p);
        assert!(w.take_fx().is_empty());
        let View::Ok(v) = w.view(&EnTr) else { panic!() };
        assert!(v.ghost_gradient.is_none());
    }

    #[test]
    fn russian_ui_searches_in_russian() {
        let mut w = Weather::new();
        w.set_language_ru(true);
        w.on_show();
        w.on_result("settings_get", &Value::Null, &json!({ "net.weather": true }));
        w.on_result("weather_get", &Value::Null, &json!({ "status": "nocity" }));
        let c = w.on_action(Action::Search { query: "Москва".into() });
        assert_eq!(c[0].args["lang"], "ru");
    }
}
