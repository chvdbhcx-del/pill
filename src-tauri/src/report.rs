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
    ["mail.", "weather.", "ai."].iter().any(|p| key.starts_with(p)) || key == "hideInApps"
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
#[tauri::command]
pub fn report_problem() -> Value {
    match build_report() {
        Ok(dir) => {
            let _ = std::process::Command::new("explorer.exe").arg(&dir).creation_flags(NO_WINDOW).spawn(); // one argv element, no shell
            json!({ "ok": true, "path": dir.to_string_lossy(), "log": dir.join("voice.log").exists() })
        }
        Err(e) => json!({ "ok": false, "error": e }),
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
        assert!(personal("mail.user") && personal("weather.city") && personal("ai.baseUrl") && personal("hideInApps"));
        assert!(!personal("module.media") && !personal("theme"));
        let Value::Object(m) = json!({ "module.b": true, "module.a": true, "module.c": false, "live.usb": true, "size": 100 }) else { unreachable!() };
        assert_eq!(enabled_ids(&m, "module."), ["a", "b"]);
    }
}
