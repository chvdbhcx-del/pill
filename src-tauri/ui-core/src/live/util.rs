//! Small pure helpers shared by the providers: JavaScript-flavoured value coercion (the payloads were
//! consumed with `p.x | 0`, `+p.x || 0`, `String(p.x || '')` and template strings) and the text
//! formatters (clock, sizes, clipping, plurals). String lengths and slices count UTF-16 units like the JS did.

use serde_json::Value;

// ---------------------------------------------------------------- coercion

/// JS truthiness of a payload field (missing = falsy).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// JS `Number(v)`: NaN for anything that is not a number-like value.
pub fn to_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Some(_) => f64::NAN,
    }
}

/// JS `+v || 0`.
pub fn num_or0(v: Option<&Value>) -> f64 {
    let n = to_number(v);
    if n.is_finite() { n } else { 0.0 }
}

/// JS `v | 0` (ToInt32).
pub fn int32(v: Option<&Value>) -> i32 {
    let n = to_number(v);
    if !n.is_finite() {
        return 0;
    }
    (n.trunc() as i64) as i32
}

/// JS `String(v ?? '')`.
pub fn text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => display(Some(other)),
    }
}

/// JS `String(v || '')`: like [`text`] but falsy values (0, false, "") also give "".
pub fn text_or_empty(v: Option<&Value>) -> String {
    if truthy(v) { text(v) } else { String::new() }
}

/// JS template-string interpolation `${v}`: a missing field prints "undefined".
pub fn display(v: Option<&Value>) -> String {
    match v {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => fmt_num(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().map(|x| if x.is_null() { String::new() } else { display(Some(x)) }).collect::<Vec<_>>().join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

/// JS `Number.prototype.toString` for the finite, non-exponent cases that occur here.
pub fn fmt_num(x: f64) -> String {
    if x.is_nan() {
        "NaN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else if x == 0.0 {
        "0".to_string()
    } else if x == x.trunc() && x.abs() < 1e21 {
        format!("{x:.0}")
    } else {
        format!("{x}")
    }
}

/// JS `Math.round`.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// JS `Number.prototype.toFixed(d)` (exact ties round up, unlike Rust's half-to-even).
pub fn to_fixed(x: f64, d: usize) -> String {
    if !x.is_finite() {
        return fmt_num(x);
    }
    let scale = 10f64.powi(d as i32);
    let scaled = x * scale;
    if (scaled - scaled.trunc()).abs() == 0.5 {
        let up = if x >= 0.0 { scaled.trunc() + 1.0 } else { scaled.trunc() - 1.0 };
        return format!("{:.*}", d, up / scale);
    }
    format!("{x:.d$}")
}

// ---------------------------------------------------------------- UTF-16 string helpers

pub fn u16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// JS `s.slice(from, to)` on UTF-16 units (a cut surrogate pair becomes U+FFFD).
pub fn u16_slice(s: &str, from: usize, to: usize) -> String {
    let u: Vec<u16> = s.encode_utf16().collect();
    let to = to.min(u.len());
    let from = from.min(to);
    String::from_utf16_lossy(&u[from..to])
}

/// `s.replace(/\s+/g, ' ').trim()`.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `clip(s, n)`: longer than `n` units becomes the first `n - 1` plus an ellipsis.
pub fn clip(s: &str, n: usize) -> String {
    if u16_len(s) > n { format!("{}…", u16_slice(s, 0, n.saturating_sub(1))) } else { s.to_string() }
}

/// `cut(s, n)`: whitespace collapsed first, then [`clip`].
pub fn cut(s: &str, n: usize) -> String {
    clip(&collapse_ws(s), n)
}

/// A long file name keeps its extension: `very-long-name-of-a-fi….pdf`.
pub fn cut_name(s: &str, n: usize) -> String {
    if u16_len(s) <= n {
        return s.to_string();
    }
    let u: Vec<u16> = s.encode_utf16().collect();
    let dot = u.iter().rposition(|&c| c == b'.' as u16);
    let ext: String = match dot {
        Some(i) if i > 0 && u.len() - i <= 8 => String::from_utf16_lossy(&u[i..]),
        _ => String::new(),
    };
    let keep = (n as isize - u16_len(&ext) as isize - 1).max(4) as usize;
    format!("{}…{}", u16_slice(s, 0, keep), ext)
}

/// `String(p).split(/[\\/]/).pop()`: the last path segment (empty after a trailing slash).
pub fn base(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

/// The last non-empty path segment (`split(/[\\/]/).filter(Boolean).pop() || ''`).
pub fn last_folder(path: &str) -> String {
    path.split(['\\', '/']).rfind(|s| !s.is_empty()).unwrap_or("").to_string()
}

/// Session titles are the first words of a prompt: drop markdown marks `# * ` > _ [ ]`.
pub fn plain_title(s: &str) -> String {
    let t: String = s.chars().map(|c| if "#*`>_[]".contains(c) { ' ' } else { c }).collect();
    collapse_ws(&t)
}

// ---------------------------------------------------------------- formatters

fn pad2(n: i64) -> String {
    format!("{n:02}")
}

/// `m:ss`, or `h:mm:ss` after an hour (OBS and screen recording clocks).
pub fn clock_hms(ms: i64) -> String {
    let s = (ms.max(0)) / 1000;
    let (h, m, r) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{}:{}", pad2(m), pad2(r)) } else { format!("{m}:{}", pad2(r)) }
}

/// `m:ss` with unbounded minutes (dictation timer).
pub fn clock_ms(ms: i64) -> String {
    let s = (ms.max(0)) / 1000;
    format!("{}:{}", s / 60, pad2(s % 60))
}

/// Timer clock from seconds, rounded up (`fmtClock` of the Note tab).
pub fn clock_ceil(sec: f64) -> String {
    let s = if sec.is_finite() { sec.ceil().max(0.0) as i64 } else { 0 };
    let (h, m, r) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{}:{}", pad2(m), pad2(r)) } else { format!("{m}:{}", pad2(r)) }
}

/// Byte counts: `812 B`, `4.2 MB`, `120 MB` (Russian units when `ru`).
pub fn fmt_size(n: f64, ru: bool) -> String {
    let mut n = if n.is_finite() { n.max(0.0) } else { 0.0 };
    let units: [&str; 5] = if ru { ["Б", "КБ", "МБ", "ГБ", "ТБ"] } else { ["B", "KB", "MB", "GB", "TB"] };
    let mut i = 0;
    while n >= 1024.0 && i < units.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    let num = if i == 0 || n >= 100.0 { fmt_num(js_round(n)) } else { to_fixed(n, 1) };
    format!("{num} {}", units[i])
}

/// "1 файл / 2 файла / 5 файлов" or "1 file / 2 files".
pub fn files_plural(n: i64, ru: bool) -> String {
    if ru {
        let w = if n % 10 == 1 && n % 100 != 11 {
            "файл"
        } else if (2..=4).contains(&(n % 10)) && (n % 100 < 10 || n % 100 >= 20) {
            "файла"
        } else {
            "файлов"
        };
        format!("{n} {w}")
    } else {
        format!("{n} {}", if n == 1 { "file" } else { "files" })
    }
}

/// Elapsed time in the sessions list: `42 с`, `7 мин`, `2 ч 5 мин`.
pub fn dur_short(ms: i64, ru: bool) -> String {
    let s = (ms.max(0)) / 1000;
    let m = s / 60;
    let h = m / 60;
    if s < 60 {
        if ru { format!("{s} с") } else { format!("{s}s") }
    } else if m < 60 {
        if ru { format!("{m} мин") } else { format!("{m}m") }
    } else if ru {
        format!("{h} ч {} мин", m % 60)
    } else {
        format!("{h}h {}m", m % 60)
    }
}

pub fn clamp_f(x: f64, lo: f64, hi: f64) -> f64 {
    x.max(lo).min(hi)
}

/// `[a, b, c].filter(Boolean).join(sep)`.
pub fn join_nonempty(parts: &[String], sep: &str) -> String {
    parts.iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(sep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coercions_match_js() {
        assert_eq!(int32(Some(&json!(12.9))), 12);
        assert_eq!(int32(Some(&json!(-3.7))), -3);
        assert_eq!(int32(Some(&json!("42"))), 42);
        assert_eq!(int32(Some(&json!(null))), 0);
        assert_eq!(int32(None), 0);
        assert_eq!(int32(Some(&json!("x"))), 0);
        assert_eq!(int32(Some(&json!(4294967297u64))), 1); // wraps like |0
        assert_eq!(num_or0(Some(&json!("3.5"))), 3.5);
        assert_eq!(num_or0(Some(&json!("abc"))), 0.0);
        assert_eq!(num_or0(None), 0.0);
        assert!(truthy(Some(&json!("a"))) && !truthy(Some(&json!(""))) && !truthy(Some(&json!(0))) && truthy(Some(&json!([]))));
        assert!(!truthy(None) && !truthy(Some(&json!(null))));
    }

    #[test]
    fn string_coercions() {
        assert_eq!(text(None), "");
        assert_eq!(text(Some(&json!(null))), "");
        assert_eq!(text(Some(&json!(7))), "7");
        assert_eq!(text_or_empty(Some(&json!(0))), "");
        assert_eq!(display(None), "undefined");
        assert_eq!(display(Some(&json!(null))), "null");
        assert_eq!(display(Some(&json!(33.5))), "33.5");
        assert_eq!(display(Some(&json!(33.0))), "33");
        assert_eq!(display(Some(&json!(["a", 2]))), "a,2");
    }

    #[test]
    fn to_fixed_and_round() {
        assert_eq!(to_fixed(0.25, 1), "0.3"); // JS: (0.25).toFixed(1) === "0.3"
        assert_eq!(to_fixed(4.24, 1), "4.2");
        assert_eq!(to_fixed(2.0, 1), "2.0");
        assert_eq!(to_fixed(1.75, 1), "1.8");
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.49), 0.0);
    }

    #[test]
    fn clip_cut_and_utf16() {
        assert_eq!(clip("abcdef", 6), "abcdef");
        assert_eq!(clip("abcdefg", 6), "abcde…");
        assert_eq!(cut("  a \n b\t c ", 20), "a b c");
        assert_eq!(cut("aaaa bbbb", 5), "aaaa…");
        // an emoji is two UTF-16 units: the length test counts it twice like the JS
        assert_eq!(u16_len("a😀"), 3);
        assert_eq!(clip("😀😀😀", 4), "😀\u{fffd}…"); // 6 units > 4: slice(0,3) cuts the second pair
        assert_eq!(clip("Привет, мир", 7), "Привет…");
        assert_eq!(clip("x", 0), "…"); // n = 0 must not underflow
    }

    #[test]
    fn cut_name_keeps_extension() {
        assert_eq!(cut_name("short.pdf", 28), "short.pdf");
        let long = "very-long-name-of-a-file-that-goes-on-and-on.pdf";
        let c = cut_name(long, 28);
        assert!(c.ends_with("….pdf"), "{c}");
        assert_eq!(u16_len(&c), 28);
        // no short extension: plain cut
        let c = cut_name("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 28);
        assert_eq!(c, format!("{}…", "a".repeat(27)));
        // dot-file / long "extension" is not kept
        assert_eq!(cut_name(&format!(".{}", "b".repeat(40)), 10), format!(".{}…", "b".repeat(8)));
        // tiny limits keep at least 4 characters
        assert!(cut_name("abcdefghijkl.mp4", 6).starts_with("abcd"));
    }

    #[test]
    fn paths() {
        assert_eq!(base(r"C:\Users\me\Downloads\a.zip"), "a.zip");
        assert_eq!(base("/home/me/b.txt"), "b.txt");
        assert_eq!(base(r"C:\dir\"), "");
        assert_eq!(base(""), "");
        assert_eq!(last_folder(r"C:\Users\usr\island-replica"), "island-replica");
        assert_eq!(last_folder("/home/me/app/"), "app");
        assert_eq!(last_folder(""), "");
    }

    #[test]
    fn plain_title_drops_markdown() {
        assert_eq!(plain_title("## Fix **the** `bug` > now_ [x]"), "Fix the bug now x");
        assert_eq!(plain_title(""), "");
        assert_eq!(plain_title("***"), "");
    }

    #[test]
    fn clocks() {
        assert_eq!(clock_hms(0), "0:00");
        assert_eq!(clock_hms(65_000), "1:05");
        assert_eq!(clock_hms(3_600_000), "1:00:00");
        assert_eq!(clock_hms(3_725_999), "1:02:05");
        assert_eq!(clock_hms(-5), "0:00");
        assert_eq!(clock_ms(3_725_000), "62:05");
        assert_eq!(clock_ceil(59.2), "1:00");
        assert_eq!(clock_ceil(0.0), "0:00");
        assert_eq!(clock_ceil(3600.0), "1:00:00");
        assert_eq!(clock_ceil(f64::NAN), "0:00");
    }

    #[test]
    fn sizes() {
        assert_eq!(fmt_size(0.0, false), "0 B");
        assert_eq!(fmt_size(1023.0, false), "1023 B");
        assert_eq!(fmt_size(1536.0, false), "1.5 KB");
        assert_eq!(fmt_size(5.0 * 1048576.0, true), "5.0 МБ");
        assert_eq!(fmt_size(150.0 * 1048576.0, true), "150 МБ");
        assert_eq!(fmt_size(-5.0, false), "0 B");
        assert_eq!(fmt_size(f64::NAN, false), "0 B");
        assert_eq!(fmt_size(1099511627776.0 * 2000.0, false), "2000 TB"); // stays on the last unit
    }

    #[test]
    fn plurals() {
        for (n, w) in [(1, "1 файл"), (2, "2 файла"), (4, "4 файла"), (5, "5 файлов"), (11, "11 файлов"), (12, "12 файлов"), (21, "21 файл"), (22, "22 файла"), (111, "111 файлов"), (0, "0 файлов")] {
            assert_eq!(files_plural(n, true), w);
        }
        assert_eq!(files_plural(1, false), "1 file");
        assert_eq!(files_plural(3, false), "3 files");
    }

    #[test]
    fn durations() {
        assert_eq!(dur_short(5_000, true), "5 с");
        assert_eq!(dur_short(59_999, false), "59s");
        assert_eq!(dur_short(60_000, true), "1 мин");
        assert_eq!(dur_short(125 * 60_000, true), "2 ч 5 мин");
        assert_eq!(dur_short(125 * 60_000, false), "2h 5m");
        assert_eq!(dur_short(-1, false), "0s");
    }

    #[test]
    fn joins() {
        assert_eq!(join_nonempty(&["a".into(), "".into(), "b".into()], " · "), "a · b");
        assert_eq!(join_nonempty(&[], "x"), "");
    }
}
