// "Report a problem" (GateGuard note: new file of the Pill app, used by main.rs + the Updates page in settings.js; user request:
// a folder in Downloads with version info, the settings export and Pill's own log tail, opened in Explorer).
// PRIVACY: only the version, Windows build, locale, monitor count, the names of the enabled modules, the settings export without
// secrets/paths (settings_backup::export_map, minus the personal keys listed in `personal`) and the tail of voice.log (one line per
// voice take: reason and durations, never audio or text). No clipboard, notes, mail, file lists or document paths. Nothing is sent anywhere.
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};

const LOG_LINES: usize = 500;
const LOG_BYTES: u64 = 256 * 1024; // read no more than this from the end of the log
const NO_WINDOW: u32 = 0x0800_0000; // CREATE_NO_WINDOW

/// `Pill-report-yyyyMMdd-HHmmss`
fn folder_name(y: u16, mo: u16, d: u16, h: u16, mi: u16, s: u16) -> String {
    format!("Pill-report-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// The last `n` lines of `text`.
fn tail_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Settings that name a person or a place (mail account, city, AI endpoints, app names) stay out of the report.
fn personal(key: &str) -> bool {
    ["mail.", "weather.", "ai."].iter().any(|p| key.starts_with(p)) || key == "hideInApps" || key == "showInApps"
}

/// Ids of the keys `<prefix>x` that are switched on, sorted.
fn enabled_ids(cfg: &serde_json::Map<String, Value>, prefix: &str) -> Vec<String> {
    let mut v: Vec<String> = cfg.iter().filter(|(k, v)| k.starts_with(prefix) && v.as_bool() == Some(true)).map(|(k, _)| k[prefix.len()..].to_string()).collect();
    v.sort();
    v
}

/// Replaces the user's home folder (either slash) with `%USERPROFILE%`.
fn scrub_home(text: &str, home: &str) -> String {
    if home.len() < 4 {
        return text.to_string();
    }
    text.replace(home, "%USERPROFILE%").replace(&home.replace('\\', "/"), "%USERPROFILE%")
}

fn info_text(version: &str, windows: &str, locale: &str, monitors: i32, modules: &[String], live: &[String], log: &str) -> String {
    format!(
        "Pill {version}\nWindows: {windows}\nLocale: {locale}\nMonitors: {monitors}\nModules on: {}\nLive events on: {}\nLog: {log}\n",
        modules.join(", "),
        live.join(", ")
    )
}

fn reg(name: PCWSTR, flags: windows::Win32::System::Registry::REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
    let mut buf = [0u8; 256];
    let mut len = buf.len() as u32;
    let sub = w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, sub, name, flags, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    (r == ERROR_SUCCESS).then(|| buf[..len as usize].to_vec())
}

fn reg_sz(name: PCWSTR) -> String {
    let Some(b) = reg(name, RRF_RT_REG_SZ) else { return String::new() };
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&u)
}

fn windows_build() -> String {
    let ubr = reg(w!("UBR"), RRF_RT_REG_DWORD).filter(|b| b.len() >= 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (build, ver) = (reg_sz(w!("CurrentBuildNumber")), reg_sz(w!("DisplayVersion")));
    format!("build {build}{} {ver}", ubr.map(|u| format!(".{u}")).unwrap_or_default()).trim().to_string()
}

fn locale() -> String {
    let mut b = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(&mut b) };
    String::from_utf16_lossy(&b[..(n.max(1) as usize - 1)])
}

/// The tail of Pill's own log (voice.log), if there is one.
fn log_tail() -> Option<String> {
    let mut f = std::fs::File::open(crate::store::dir().join("voice.log")).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(LOG_BYTES))).ok()?;
    let mut raw = Vec::new();
    f.read_to_end(&mut raw).ok()?;
    Some(tail_lines(&String::from_utf8_lossy(&raw), LOG_LINES))
}

fn build_report() -> Result<PathBuf, String> {
    let t = unsafe { GetLocalTime() };
    let dir = crate::settings_backup::downloads_dir().join(folder_name(t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let cfg = crate::settings::merged();
    let tail = log_tail();
    let info = info_text(
        env!("CARGO_PKG_VERSION"),
        &windows_build(),
        &locale(),
        unsafe { GetSystemMetrics(SM_CMONITORS) },
        &enabled_ids(&cfg, "module."),
        &enabled_ids(&cfg, "live."),
        if tail.is_some() { "voice.log (last lines)" } else { "none (Pill has not written a log on this PC)" },
    );
    std::fs::write(dir.join("info.txt"), scrub_home(&info, &home)).map_err(|e| e.to_string())?;
    let mut export = crate::settings_backup::export_map();
    export.retain(|k, _| !personal(k));
    let json = serde_json::to_string_pretty(&export).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("settings.json"), scrub_home(&json, &home)).map_err(|e| e.to_string())?;
    if let Some(l) = tail {
        std::fs::write(dir.join("voice.log"), scrub_home(&l, &home)).map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

/// -> { ok, path?, log?, error? }. Creates the folder in Downloads and opens it in Explorer.
pub fn report_problem() -> Value {
    match build_report() {
        Ok(dir) => {
            let _ = std::process::Command::new("explorer.exe").arg(&dir).creation_flags(NO_WINDOW).spawn(); // one argv element, no shell
            json!({ "ok": true, "path": dir.to_string_lossy(), "log": dir.join("voice.log").exists() })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// "Report a bug": a prefilled GitHub issue
// ---------------------------------------------------------------------------------------------------------------------------------------

const ISSUE_URL: &str = "https://github.com/chvdbhcx-del/pill/issues/new";
/// The placement settings an issue about the island's position needs. All of them are enums or numbers: no paths, names or titles.
const PLACEMENT_KEYS: [&str; 8] = ["monitor", "edge.position", "edge.align", "offsetX", "underTaskbar", "size", "media.widget", "media.widget.edge"];

/// "100%, 150%" for the monitors' DPI scales.
fn scales_text(scales: &[f32]) -> String {
    scales.iter().map(|s| format!("{}%", (s * 100.0).round() as i32)).collect::<Vec<_>>().join(", ")
}

/// `key=value` for every placement setting, in a fixed order. Numbers, booleans and short words only (anything else is shown as `?`).
fn placement_text(cfg: &serde_json::Map<String, Value>) -> String {
    PLACEMENT_KEYS
        .iter()
        .map(|k| {
            let v = match cfg.get(*k) {
                Some(Value::Bool(b)) => b.to_string(),
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::String(s)) if s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') => s.clone(),
                Some(_) => "?".into(),
                None => "default".into(),
            };
            format!("{k}={v}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The issue's title and body. Pure: everything in it is passed in, and `ru` only picks the language of the hint (the template stays English
/// so the maintainer can read it).
fn issue_text(version: &str, windows: &str, locale: &str, scales: &[f32], placement: &str, ru: bool) -> (String, String) {
    let hint = if ru {
        "Приложите папку отчёта (Настройки > Обновления > Сообщить о проблеме), заархивировав её в zip: перетащите zip в это поле."
    } else {
        "Attach the report folder from Settings > Updates > Report a problem, zipped: drag the zip into this box."
    };
    let body = format!(
        "**What happened**\n\n\n**What did you expect**\n\n\n**Steps to reproduce**\n1. \n\n---\nPill {version} | {windows} | locale {locale}\nMonitors: {} ({})\nPlacement: {placement}\n\n<!-- {hint} Nothing was sent automatically. -->\n",
        scales.len(),
        scales_text(scales)
    );
    ("Bug: ".to_string(), body)
}

fn issue_url(title: &str, body: &str) -> String {
    format!("{ISSUE_URL}?title={}&body={}", crate::net::urlenc(title), crate::net::urlenc(body))
}

/// The confirmation text: exactly the title and body that will be opened in the browser.
fn confirm_text(title: &str, body: &str, ru: bool) -> String {
    let head = if ru {
        "Pill откроет в браузере страницу нового обращения на GitHub с этим текстом. Ничего не отправляется, пока вы сами не нажмёте Submit.\nВ нём только версии, число мониторов и масштаб, настройки положения. Нет имён, путей, заголовков окон и содержимого буфера. Zip с диагностикой приложите вручную.\n\n"
    } else {
        "Pill will open the GitHub new-issue page in your browser with this text. Nothing is sent until you press Submit yourself.\nIt holds only versions, the monitor count and scale, and the placement settings. No names, paths, window titles or clipboard content. Attach the diagnostics zip by hand.\n\n"
    };
    format!("{head}Title: {title}\n\n{body}")
}

fn confirm(text: &str, ru: bool) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDOK, MB_ICONINFORMATION, MB_OKCANCEL, MB_SETFOREGROUND, MB_TOPMOST};
    let caption = if ru { "Pill: сообщить об ошибке" } else { "Pill: report a bug" };
    let (t, c): (Vec<u16>, Vec<u16>) = (text.encode_utf16().chain(Some(0)).collect(), caption.encode_utf16().chain(Some(0)).collect());
    unsafe { MessageBoxW(windows::Win32::Foundation::HWND::default(), PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OKCANCEL | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND) == IDOK }
}

fn open_in_browser(url: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe { ShellExecuteW(windows::Win32::Foundation::HWND::default(), &HSTRING::from("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL).0 as isize > 32 }
}

/// -> { ok, opened } or { ok: false, error }. Shows the exact text in a confirmation box (OK / Cancel; blocks: worker threads only), then opens the
/// prefilled new-issue page in the default browser. `opened: false` = the user cancelled.
pub fn report_issue() -> Value {
    let ru = crate::ui::capture_view::windows::is_russian();
    let scales: Vec<f32> = crate::ui::host::handle().map(|h| h.monitors().iter().map(|m| m.scale).collect()).unwrap_or_default();
    let (title, body) = issue_text(env!("CARGO_PKG_VERSION"), &windows_build(), &locale(), &scales, &placement_text(&crate::settings::merged()), ru);
    if !confirm(&confirm_text(&title, &body, ru), ru) {
        return json!({ "ok": true, "opened": false });
    }
    if open_in_browser(&issue_url(&title, &body)) {
        json!({ "ok": true, "opened": true })
    } else {
        json!({ "ok": false, "error": "could not open the browser" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_sortable() {
        assert_eq!(folder_name(2026, 10, 9, 8, 5, 3), "Pill-report-20261009-080503");
    }

    #[test]
    fn log_tail_is_trimmed() {
        let text: String = (1..=800).map(|i| format!("line {i}\n")).collect();
        let t = tail_lines(&text, LOG_LINES);
        assert_eq!(t.lines().count(), LOG_LINES);
        assert!(t.starts_with("line 301") && t.ends_with("line 800"));
        assert_eq!(tail_lines("a\nb", 500), "a\nb");
        assert_eq!(tail_lines("", 5), "");
    }

    #[test]
    fn info_has_no_home_path() {
        let home = "C:\\Users\\alice";
        let info = info_text("0.3.8", "build 26300.1 25H2", "ru-RU", 2, &["media".into(), "shelf".into()], &["usb".into()], "none");
        assert!(info.contains("Pill 0.3.8") && info.contains("Monitors: 2") && info.contains("media, shelf"));
        assert!(!info.contains(home) && !info.to_lowercase().contains("users"));
        assert_eq!(scrub_home("open C:\\Users\\alice\\x and C:/Users/alice/y", home), "open %USERPROFILE%\\x and %USERPROFILE%/y");
        assert_eq!(scrub_home("abc", ""), "abc");
    }

    #[test]
    fn system_facts_are_readable() {
        let (b, l) = (windows_build(), locale());
        eprintln!("{b} | {l}");
        assert!(b.starts_with("build ") && b.len() > 8 && l.len() >= 2);
    }

    #[test]
    fn personal_keys_and_enabled_ids() {
        assert!(personal("mail.user") && personal("weather.city") && personal("ai.baseUrl") && personal("hideInApps") && personal("showInApps"));
        assert!(!personal("module.media") && !personal("theme"));
        let Value::Object(m) = json!({ "module.b": true, "module.a": true, "module.c": false, "live.usb": true, "size": 100 }) else { unreachable!() };
        assert_eq!(enabled_ids(&m, "module."), ["a", "b"]);
    }

    #[test]
    fn the_issue_template_has_versions_and_placement_but_no_personal_data() {
        let Value::Object(cfg) = json!({ "monitor": "2", "edge.position": "taskbar", "edge.align": "left", "offsetX": 12, "underTaskbar": true, "size": 110,
            "media.widget": false, "media.widget.edge": "C:\\Users\\alice\\x", "hideInApps": ["secret.exe"], "mail.user": "a@b.c" }) else { unreachable!() };
        let p = placement_text(&cfg);
        assert_eq!(p, "monitor=2, edge.position=taskbar, edge.align=left, offsetX=12, underTaskbar=true, size=110, media.widget=false, media.widget.edge=?");
        assert!(!p.contains("alice") && !p.contains("secret") && !p.contains("a@b.c"));
        let (t, b) = issue_text("0.9.0", "build 26300.1 25H2", "ru-RU", &[1.0, 1.5], &p, false);
        assert!(t.starts_with("Bug") && b.contains("Pill 0.9.0 | build 26300.1 25H2") && b.contains("Monitors: 2 (100%, 150%)") && b.contains("Placement: monitor=2"));
        assert_eq!(scales_text(&[1.25]), "125%");
    }

    #[test]
    fn the_issue_url_is_the_repo_with_an_encoded_title_and_body_and_the_confirmation_shows_the_same_text() {
        let u = issue_url("Bug: a b", "x\ny&z");
        assert_eq!(u, "https://github.com/chvdbhcx-del/pill/issues/new?title=Bug%3A%20a%20b&body=x%0Ay%26z");
        let (t, b) = issue_text("0.9.0", "w", "en-US", &[], "monitor=primary", true);
        let c = confirm_text(&t, &b, true);
        assert!(c.contains(&b) && c.contains(&t));
        assert!(issue_url(&t, &b).len() < 2000, "browsers and ShellExecute cap the URL");
    }
}
