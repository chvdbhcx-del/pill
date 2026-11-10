//! The weather tiles of the Weather tab (like the iPhone Weather app): wind, humidity, pressure, sun, precipitation and UV index, plus one
//! sentence that sums the day up. Everything is derived from the MET Norway forecast the tab already has (instant details of the
//! locationforecast: wind, relative humidity, pressure at sea level, clear-sky UV index; the sunrise / sunset of `net.rs`), nothing is fetched here.
//!
//! A tile carries its number apart from its text (`num` + `decimals` + `lead` + `unit`) so the renderer can ease the value from the old
//! reading to the new one and still format it the way the tab does ([`fmt_num`]).
use super::weather::{Day, Now};
use super::Tr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wind,
    Humidity,
    Pressure,
    Sun,
    Precip,
    Uv,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WxTile {
    pub kind: Kind,
    pub title: String,
    /// Printed before / after the number (`"≈ "`, `" m/s"`).
    pub lead: String,
    pub unit: String,
    /// The number to ease; `None` = the tile shows `text` as it is (sun times).
    pub num: Option<f64>,
    pub decimals: u8,
    pub text: String,
    /// One line under the value.
    pub sub: String,
    /// 0..1 for the little picture of the tile (gauge, bar, sun position, drop level).
    pub frac: f32,
    /// Wind only: the direction the wind blows towards, in degrees clockwise from north.
    pub toward: f32,
}

/// What the builders need from the tab around the raw numbers (formatting, units, the clock).
pub struct Env<'a> {
    pub tr: &'a dyn Tr,
    pub metric: bool,
    /// Degrees C to the tab's temperature text (`"3°"`).
    pub temp: &'a dyn Fn(f64) -> String,
    /// Wind speed with its unit (`"5 m/s"`), and the compass word it comes from (`"SW"`).
    pub wind: String,
    pub dir: String,
    /// Today's sunrise / sunset in ms and as clock texts, the current time in ms (the first hourly slot); a polar text when the sun does not set.
    pub rise: Option<(i64, String)>,
    pub set: Option<(i64, String)>,
    pub now_ms: Option<i64>,
    pub sun_text: Option<String>,
    /// Today's precipitation as the tab prints it (`"3,3 mm"`, empty when dry) and the next hour's.
    pub today_mm: String,
    pub hour_mm: String,
}

/// A number with `decimals` places, a decimal comma in Russian.
pub fn fmt_num(x: f64, decimals: u8, ru: bool) -> String {
    let s = format!("{:.*}", decimals as usize, x);
    if ru {
        s.replace('.', ",")
    } else {
        s
    }
}

/// Dew point in degrees C (Magnus formula) from the air temperature and the relative humidity in percent.
pub fn dew_point(t: f64, rh: f64) -> f64 {
    let (a, b) = (17.62, 243.12);
    let g = (rh.clamp(1.0, 100.0) / 100.0).ln() + a * t / (b + t);
    b * g / (a - g)
}

fn uv_word(tr: &dyn Tr, uv: f64) -> String {
    if uv < 3.0 {
        tr.t("Низкий", "Low")
    } else if uv < 6.0 {
        tr.t("Умеренный", "Moderate")
    } else if uv < 8.0 {
        tr.t("Высокий", "High")
    } else if uv < 11.0 {
        tr.t("Очень высокий", "Very high")
    } else {
        tr.t("Экстремальный", "Extreme")
    }
}

fn pressure_word(tr: &dyn Tr, hpa: f64) -> String {
    if hpa < 1000.0 {
        tr.t("Пониженное", "Low")
    } else if hpa > 1025.0 {
        tr.t("Повышенное", "High")
    } else {
        tr.t("Нормальное", "Normal")
    }
}

/// The tiles for the numbers that exist (a forecast without pressure or UV simply has no such tile).
pub fn tiles(n: &Now, today: Option<&Day>, e: &Env) -> Vec<WxTile> {
    let tr = e.tr;
    let mut out = Vec::new();
    let tile = |kind, title: String| WxTile { kind, title, lead: String::new(), unit: String::new(), num: None, decimals: 0, text: String::new(), sub: String::new(), frac: 0.0, toward: 0.0 };

    let (ws, unit) = if e.metric { (n.wind, tr.t(" м/с", " m/s")) } else { (n.wind * 2.23694, " mph".to_string()) };
    let mut w = tile(Kind::Wind, tr.t("Ветер", "Wind"));
    w.num = Some(ws.round());
    w.unit = unit;
    w.sub = tr.t(&format!("С направления: {}", e.dir), &format!("From {}", e.dir));
    w.frac = (n.wind / 20.0).clamp(0.0, 1.0) as f32;
    w.toward = ((n.dir + 180.0).rem_euclid(360.0)) as f32;
    out.push(w);

    let mut h = tile(Kind::Humidity, tr.t("Влажность", "Humidity"));
    h.num = Some(n.humidity.round());
    h.unit = "%".into();
    h.sub = tr.t(&format!("Точка росы {}", (e.temp)(dew_point(n.temp, n.humidity))), &format!("Dew point {}", (e.temp)(dew_point(n.temp, n.humidity))));
    h.frac = (n.humidity / 100.0).clamp(0.0, 1.0) as f32;
    out.push(h);

    if n.pressure > 100.0 {
        let mut p = tile(Kind::Pressure, tr.t("Давление", "Pressure"));
        if e.metric {
            p.num = Some(n.pressure.round());
            p.unit = tr.t(" гПа", " hPa");
        } else {
            p.num = Some(n.pressure * 0.029530);
            p.decimals = 2;
            p.unit = " inHg".into();
        }
        p.sub = pressure_word(tr, n.pressure);
        p.frac = ((n.pressure - 970.0) / 80.0).clamp(0.0, 1.0) as f32;
        out.push(p);
    }

    let mut s = tile(Kind::Sun, tr.t("Солнце", "Sun"));
    match (&e.rise, &e.set, e.now_ms) {
        (Some((r, rt)), Some((st, stt)), now) => {
            let now = now.unwrap_or(*r);
            let (title, text, other) = if now < *r {
                (tr.t("Восход", "Sunrise"), rt.clone(), tr.t(&format!("Закат: {stt}"), &format!("Sunset: {stt}")))
            } else {
                (tr.t("Закат", "Sunset"), stt.clone(), tr.t(&format!("Восход: {rt}"), &format!("Sunrise: {rt}")))
            };
            s.title = title;
            s.text = text;
            s.sub = other;
            s.frac = ((now - r) as f64 / (st - r).max(1) as f64).clamp(0.0, 1.0) as f32;
        }
        _ => {
            s.text = e.sun_text.clone().unwrap_or_default();
            s.frac = 0.5;
        }
    }
    out.push(s);

    let mut p = tile(Kind::Precip, tr.t("Осадки", "Precipitation"));
    let mm = today.map_or(0.0, |d| d.precip);
    if e.metric {
        p.num = Some((mm * 10.0).round() / 10.0);
        p.decimals = 1;
        p.unit = tr.t(" мм", " mm");
    } else {
        p.num = Some(mm / 25.4);
        p.decimals = 2;
        p.unit = " in".into();
    }
    p.sub = if e.hour_mm.is_empty() { tr.t("В ближайший час сухо", "Dry for the next hour") } else { tr.t(&format!("В ближайший час: {}", e.hour_mm), &format!("Next hour: {}", e.hour_mm)) };
    p.frac = (mm / 10.0).clamp(0.0, 1.0) as f32;
    out.push(p);

    if let Some(uv) = n.uv {
        let mut u = tile(Kind::Uv, tr.t("УФ-индекс", "UV index"));
        u.num = Some(uv.round());
        let peak = n.uv_max.map(|m| tr.t(&format!(" · максимум {}", m.round()), &format!(" · peak {}", m.round()))).unwrap_or_default();
        u.sub = format!("{}{}", uv_word(tr, uv), peak);
        u.frac = (uv / 11.0).clamp(0.0, 1.0) as f32;
        out.push(u);
    }
    out
}

/// One sentence for the day: what it is now, the range, the wind and what falls from the sky.
pub fn summary(n: &Now, condition: &str, e: &Env) -> String {
    let tr = e.tr;
    let cond = condition.to_lowercase();
    let rain = if e.today_mm.is_empty() {
        tr.t("осадков не ожидается", "with no precipitation expected")
    } else {
        tr.t(&format!("ожидается около {} осадков", e.today_mm), &format!("with about {} of precipitation expected", e.today_mm))
    };
    let s = tr.t(
        &format!("Сейчас {cond}, сегодня от {} до {}, ветер {} ({}), {rain}.", (e.temp)(n.min), (e.temp)(n.max), e.wind, e.dir),
        &format!("{cond} now, {} to {} today, wind {} from {}, {rain}.", (e.temp)(n.min), (e.temp)(n.max), e.wind, e.dir),
    );
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    fn now() -> Now {
        Now { temp: 4.4, feels: 1.6, symbol: "lightrain".into(), icon: "rain".into(), wind: 5.2, dir: 225.0, humidity: 81.0, precip: 0.4, min: 2.1, max: 7.0, pressure: 1013.4, uv: Some(3.2), uv_max: Some(5.6) }
    }

    fn env<'a>(tr: &'a dyn Tr, metric: bool, temp: &'a dyn Fn(f64) -> String) -> Env<'a> {
        Env {
            tr,
            metric,
            temp,
            wind: "5 m/s".into(),
            dir: "SW".into(),
            rise: Some((1_000, "05:30".into())),
            set: Some((9_000, "14:30".into())),
            now_ms: Some(5_000),
            sun_text: None,
            today_mm: "3.3 mm".into(),
            hour_mm: "0.4 mm".into(),
        }
    }

    #[test]
    fn six_tiles_in_the_iphone_order_with_real_numbers() {
        let t = |c: f64| format!("{}°", c.round());
        let e = env(&EnTr, true, &t);
        let day = Day { dow: 5, min: 2.0, max: 7.0, icon: "rain".into(), precip: 3.25 };
        let v = tiles(&now(), Some(&day), &e);
        let kinds: Vec<Kind> = v.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, [Kind::Wind, Kind::Humidity, Kind::Pressure, Kind::Sun, Kind::Precip, Kind::Uv]);
        assert_eq!((v[0].num, v[0].unit.as_str(), v[0].sub.as_str()), (Some(5.0), " m/s", "From SW"));
        assert_eq!(v[0].toward, 45.0, "the arrow points where the wind goes, the opposite of where it comes from");
        assert_eq!((v[1].num, v[1].unit.as_str()), (Some(81.0), "%"));
        assert!(v[1].sub.starts_with("Dew point ") && v[1].sub.ends_with('°'));
        assert_eq!((v[2].num, v[2].sub.as_str()), (Some(1013.0), "Normal"));
        assert_eq!((v[3].title.as_str(), v[3].text.as_str(), v[3].sub.as_str()), ("Sunset", "14:30", "Sunrise: 05:30"));
        assert!((v[3].frac - 0.5).abs() < 1e-6, "the sun is half way between rise and set");
        assert_eq!((v[4].num, v[4].decimals, v[4].sub.as_str()), (Some(3.3), 1, "Next hour: 0.4 mm"));
        assert_eq!((v[5].num, v[5].sub.as_str()), (Some(3.0), "Moderate · peak 6"));
    }

    #[test]
    fn missing_pressure_and_uv_drop_their_tiles_and_the_sun_waits_for_sunrise() {
        let t = |c: f64| format!("{}°", c.round());
        let mut e = env(&EnTr, true, &t);
        e.now_ms = Some(0);
        e.hour_mm.clear();
        let mut n = now();
        n.pressure = 0.0;
        n.uv = None;
        let v = tiles(&n, None, &e);
        assert_eq!(v.len(), 4);
        let sun = v.iter().find(|x| x.kind == Kind::Sun).unwrap();
        assert_eq!((sun.title.as_str(), sun.text.as_str(), sun.frac), ("Sunrise", "05:30", 0.0));
        assert_eq!(v.iter().find(|x| x.kind == Kind::Precip).unwrap().sub, "Dry for the next hour");
        // polar day: no times, the text from the tab
        e.rise = None;
        e.set = None;
        e.sun_text = Some("Midnight sun".into());
        assert_eq!(tiles(&n, None, &e).iter().find(|x| x.kind == Kind::Sun).unwrap().text, "Midnight sun");
    }

    #[test]
    fn imperial_units() {
        let t = |c: f64| format!("{}°", (c * 9.0 / 5.0 + 32.0).round());
        let e = env(&EnTr, false, &t);
        let day = Day { dow: 5, min: 2.0, max: 7.0, icon: "rain".into(), precip: 2.54 };
        let v = tiles(&now(), Some(&day), &e);
        assert_eq!((v[0].num, v[0].unit.as_str()), (Some(12.0), " mph"));
        assert_eq!((v[2].decimals, v[2].unit.as_str()), (2, " inHg"));
        assert!((v[2].num.unwrap() - 29.93).abs() < 0.01);
        assert!((v[4].num.unwrap() - 0.1).abs() < 1e-9 && v[4].unit == " in");
    }

    #[test]
    fn the_dew_point_is_below_the_air_temperature_and_equal_when_saturated() {
        assert!((dew_point(10.0, 100.0) - 10.0).abs() < 0.05);
        assert!((dew_point(20.0, 50.0) - 9.3).abs() < 0.3);
        assert!(dew_point(4.4, 81.0) < 4.4);
    }

    #[test]
    fn numbers_print_with_a_comma_in_russian() {
        assert_eq!(fmt_num(3.26, 1, false), "3.3");
        assert_eq!(fmt_num(3.3, 1, true), "3,3");
        assert_eq!(fmt_num(81.0, 0, true), "81");
    }

    #[test]
    fn one_sentence_in_both_languages() {
        let t = |c: f64| format!("{}°", c.round());
        let e = env(&EnTr, true, &t);
        assert_eq!(summary(&now(), "Light rain", &e), "Light rain now, 2° to 7° today, wind 5 m/s from SW, with about 3.3 mm of precipitation expected.");
        let mut dry = env(&EnTr, true, &t);
        dry.today_mm.clear();
        assert!(summary(&now(), "Clear", &dry).ends_with("with no precipitation expected."));
        let mut ru = env(&RuTr, true, &t);
        ru.wind = "5 м/с".into();
        ru.dir = "ЮЗ".into();
        ru.today_mm = "3,3 мм".into();
        assert_eq!(summary(&now(), "Небольшой дождь", &ru), "Сейчас небольшой дождь, сегодня от 2° до 7°, ветер 5 м/с (ЮЗ), ожидается около 3,3 мм осадков.");
        let one = summary(&now(), "Light rain", &e);
        assert!(one.ends_with('.') && !one[..one.len() - 1].contains(". "), "one sentence: {one}");
    }

    #[test]
    fn the_uv_words_follow_the_who_bands() {
        let w = |x: f64| uv_word(&EnTr, x);
        assert_eq!((w(1.0), w(4.0), w(7.0), w(9.0), w(12.0)), ("Low".into(), "Moderate".into(), "High".into(), "Very high".into(), "Extreme".into()));
    }
}
