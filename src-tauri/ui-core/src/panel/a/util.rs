//! Small formatting helpers shared by panel set A: `m:ss` clocks, ru/en plurals, local calendar maths
//! (no chrono: the PC's UTC offset in minutes arrives with every call, exactly as `weather_get` takes `tz`).

/// Wall clock handed to view builders: epoch milliseconds plus the PC's UTC offset in minutes (JS `-getTimezoneOffset()`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wall {
    pub ms: i64,
    pub tz_min: i64,
}

impl Wall {
    pub fn new(ms: i64, tz_min: i64) -> Wall {
        Wall { ms, tz_min }
    }
}

/// tab-home `fmt`: `m:ss`, never negative.
pub fn fmt_ms(ms: i64) -> String {
    let s = (ms.max(0)) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Russian plural form of "file" (`файл`, `файла`, `файлов`) or English (`file`, `files`), without a leading space.
pub fn files_word(ru: bool, n: u64) -> &'static str {
    if ru {
        if n % 10 == 1 && n % 100 != 11 {
            "файл"
        } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
            "файла"
        } else {
            "файлов"
        }
    } else if n == 1 {
        "file"
    } else {
        "files"
    }
}

/// Broken-down local time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub min: u32,
    pub sec: u32,
    /// 0 = Sunday (JS `getDay`)
    pub dow: u32,
}

/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's civil_from_days).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn local(w: Wall) -> Local {
    let t = (w.ms + w.tz_min * 60_000).div_euclid(1000);
    let days = t.div_euclid(86_400);
    let sod = t.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Local { year, month, day, hour: (sod / 3600) as u32, min: ((sod / 60) % 60) as u32, sec: (sod % 60) as u32, dow: (days + 4).rem_euclid(7) as u32 }
}

/// Epoch ms of the local midnight of the day containing `w`.
pub fn day_start(w: Wall) -> i64 {
    let t = w.ms + w.tz_min * 60_000;
    t.div_euclid(86_400_000) * 86_400_000 - w.tz_min * 60_000
}

const MONTHS_RU: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// `toLocaleTimeString(lang, {hour:'2-digit', minute:'2-digit', hour12})`: `14:05` or `02:05 PM`.
pub fn clock_label(w: Wall, hour12: bool) -> String {
    let l = local(w);
    if hour12 {
        let h = l.hour % 12;
        format!("{:02}:{:02} {}", if h == 0 { 12 } else { h }, l.min, if l.hour < 12 { "AM" } else { "PM" })
    } else {
        format!("{:02}:{:02}", l.hour, l.min)
    }
}

/// `toLocaleDateString(lang, {day:'numeric', month:'long'})`: `30 октября` / `October 30`.
pub fn day_month_label(w: Wall, ru: bool) -> String {
    let l = local(w);
    if ru {
        format!("{} {}", l.day, MONTHS_RU[l.month as usize - 1])
    } else {
        format!("{} {}", MONTHS_EN[l.month as usize - 1], l.day)
    }
}

/// `toLocaleString(lang)` for tooltips: `30.10.2026, 14:05:09` / `10/30/2026, 2:05:09 PM`.
pub fn datetime_label(w: Wall, ru: bool) -> String {
    let l = local(w);
    if ru {
        format!("{:02}.{:02}.{}, {:02}:{:02}:{:02}", l.day, l.month, l.year, l.hour, l.min, l.sec)
    } else {
        let h = l.hour % 12;
        format!("{}/{}/{}, {}:{:02}:{:02} {}", l.month, l.day, l.year, if h == 0 { 12 } else { h }, l.min, l.sec, if l.hour < 12 { "AM" } else { "PM" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-30 14:05:09 UTC
    const T: i64 = 1_793_369_109_000;

    #[test]
    fn seek_clock_formats() {
        assert_eq!(fmt_ms(0), "0:00");
        assert_eq!(fmt_ms(61_999), "1:01");
        assert_eq!(fmt_ms(-5000), "0:00");
        assert_eq!(fmt_ms(3_599_000), "59:59");
        assert_eq!(fmt_ms(3_600_000), "60:00");
    }

    #[test]
    fn russian_plurals() {
        let w: Vec<_> = [1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 100, 101, 111].iter().map(|&n| files_word(true, n)).collect();
        assert_eq!(w, ["файл", "файла", "файла", "файлов", "файлов", "файлов", "файлов", "файл", "файла", "файлов", "файлов", "файл", "файлов"]);
        assert_eq!((files_word(false, 1), files_word(false, 0), files_word(false, 3)), ("file", "files", "files"));
    }

    #[test]
    fn local_time_with_offset() {
        let l = local(Wall::new(T, 0));
        assert_eq!((l.year, l.month, l.day, l.hour, l.min, l.sec, l.dow), (2026, 10, 30, 14, 5, 9, 5)); // a Friday
        let l = local(Wall::new(T, 180));
        assert_eq!((l.day, l.hour), (30, 17));
        let l = local(Wall::new(T, 600)); // crosses midnight
        assert_eq!((l.day, l.hour, l.dow), (31, 0, 6));
    }

    #[test]
    fn day_start_is_local_midnight() {
        let w = Wall::new(T, 180);
        let s = day_start(w);
        let l = local(Wall::new(s, 180));
        assert_eq!((l.hour, l.min, l.sec, l.day), (0, 0, 0, 30));
        assert_eq!(day_start(Wall::new(s - 1, 180)), s - 86_400_000);
        assert_eq!(day_start(Wall::new(s, 180)), s);
    }

    #[test]
    fn labels() {
        assert_eq!(clock_label(Wall::new(T, 0), false), "14:05");
        assert_eq!(clock_label(Wall::new(T, 0), true), "02:05 PM");
        assert_eq!(clock_label(Wall::new(T - 14 * 3_600_000, 0), true), "12:05 AM");
        assert_eq!(day_month_label(Wall::new(T, 0), true), "30 октября");
        assert_eq!(day_month_label(Wall::new(T, 0), false), "October 30");
        assert_eq!(datetime_label(Wall::new(T, 0), true), "30.10.2026, 14:05:09");
        assert_eq!(datetime_label(Wall::new(T, 0), false), "10/30/2026, 2:05:09 PM");
    }
}
