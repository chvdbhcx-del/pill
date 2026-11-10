// Hide the pill over fullscreen / exclusive-fullscreen / chosen apps. Owner: pill agent (lane L1).
// One 500 ms poll: hide at once, show after 2 clear polls (QUNS lags after alt-tab). While visible the same poll
// re-places the window when monitor / DPI / taskbar changed. Hiding itself (SW_HIDE + pill:hide) lives in pill.rs.
use crate::pill::{relayout, set_hidden_bg, HIDE_GUARD};
use std::sync::Mutex;
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
    IsZoomed, GWL_STYLE, WS_CAPTION,
};

/// Foreground window covers its whole monitor (borderless game, fullscreen video). Ported from macshell main.rs;
/// a maximized window with a title bar is not fullscreen (matters with an auto-hide taskbar).
fn foreground_is_fullscreen(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false; // a hidden window can linger as "foreground" (e.g. a just-closed overlay): it covers nothing
        }
        let mut buf = [0u16; 256];
        let len = GetClassNameW(hwnd, &mut buf).max(0) as usize;
        if matches!(
            String::from_utf16_lossy(&buf[..len]).as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return false;
        }
        if IsZoomed(hwnd).as_bool() && GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CAPTION.0 == WS_CAPTION.0 {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return false;
        }
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
            return false;
        }
        let m = mi.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}

/// The desktop itself (Progman / WorkerW) is the foreground window: no program window in front.
fn is_desktop(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) }.max(0) as usize;
    matches!(String::from_utf16_lossy(&buf[..len]).as_str(), "Progman" | "WorkerW")
}

/// Page query before it slides the pill away: the desktop in front (when `autoHideKeepDesktop` is on) or a program of `showInApps` in front.
pub fn pill_desktop_fg() -> bool {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return false;
    }
    (crate::settings::get_bool("autoHideKeepDesktop", false) && is_desktop(fg)) || exe_name(fg).is_some_and(|e| listed(&e, &crate::settings::get_list("showInApps")))
}

/// Exclusive Direct3D fullscreen, presentation mode, "busy" (QUERY_USER_NOTIFICATION_STATE).
fn quns_busy() -> bool {
    unsafe {
        SHQueryUserNotificationState()
            .is_ok_and(|s| s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE)
    }
}

/// Lower-case file name of the foreground window's exe ("chrome.exe").
fn exe_name(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        ok.then(|| path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase())
    }
}

/// `exe` ("chrome.exe") is on the list (a process name, with or without ".exe").
fn listed(exe: &str, list: &[String]) -> bool {
    let stem = exe.strip_suffix(".exe").unwrap_or(exe);
    list.iter().any(|e| {
        let e = e.trim().to_lowercase();
        e == exe || e == stem
    })
}

/// The last foreground program that is not Pill itself: what "add the active program" in the settings adds (the settings window is
/// Pill's own, so the foreground at the click is Pill). Refreshed only when the foreground window changes.
static LAST_APP: Mutex<(isize, String)> = Mutex::new((0, String::new()));

fn note_app(fg: HWND) {
    let key = fg.0 as isize;
    if key == 0 || LAST_APP.lock().unwrap_or_else(|e| e.into_inner()).0 == key {
        return;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    if pid == std::process::id() {
        return; // our own window (settings, the island): keep the program that was in front before
    }
    let name = exe_name(fg).unwrap_or_default();
    *LAST_APP.lock().unwrap_or_else(|e| e.into_inner()) = (key, name);
}

/// Settings button "Add the active program": appends the last foreground program to `hideInApps` / `showInApps` and returns the settings.
pub fn pill_add_active_app(app: tauri::AppHandle, key: String) -> serde_json::Map<String, serde_json::Value> {
    let exe = LAST_APP.lock().unwrap_or_else(|e| e.into_inner()).1.clone();
    if matches!(key.as_str(), "hideInApps" | "showInApps") {
        if let Some(next) = crate::fspeek::with_app(&crate::settings::get_list(&key), &exe) {
            let mut patch = serde_json::Map::new();
            patch.insert(key, serde_json::json!(next));
            return crate::settings::apply_patch(&app, patch).0;
        }
    }
    crate::settings::settings_get()
}

/// Watches the cursor for up to 500 ms (60 ms ticks) while a fullscreen app hides the island and `fullscreenPeekEdge` is on:
/// resting on the top edge for 150 ms brings the island over the app. Runs only in that state.
fn edge_wait(app: &tauri::AppHandle) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let t0 = std::time::Instant::now();
    let mut dwell = crate::fspeek::EdgeDwell::default();
    while t0.elapsed() < std::time::Duration::from_millis(500) {
        std::thread::sleep(std::time::Duration::from_millis(60));
        let Some((mon, xr)) = crate::pill::edge_geometry() else { continue };
        let mut p = POINT::default();
        if unsafe { GetCursorPos(&mut p) }.is_err() {
            continue;
        }
        let held = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0;
        if dwell.step(t0.elapsed().as_millis() as u64, crate::fspeek::on_top_edge((p.x, p.y), mon, xr, held)) {
            crate::pill::peek(app, "edge");
            return;
        }
    }
}

pub fn init(app: &tauri::App) {
    let app = app.handle().clone();
    std::thread::spawn(move || {
        let mut clear = 0u32;
        loop {
            let mut hidden = crate::pill::guard_hidden(); // a peek or the tray entry may have lifted / re-set it since the last poll
            let fg = unsafe { GetForegroundWindow() };
            note_app(fg);
            let (hide_l, show_l) = (crate::settings::get_list("hideInApps"), crate::settings::get_list("showInApps"));
            let exe = if fg.0.is_null() || (hide_l.is_empty() && show_l.is_empty()) { None } else { exe_name(fg) };
            let in_show = exe.as_deref().is_some_and(|e| listed(e, &show_l));
            // `hideInFullscreen` (default on): off = fullscreen windows / D3D fullscreen never hide the pill, only the hide list does;
            // a program of `showInApps` in front keeps the pill whatever else is true
            let fs = crate::settings::get_bool("hideInFullscreen", true);
            let facts = crate::fspeek::Facts {
                fs_setting: fs,
                fullscreen: fs && ((!fg.0.is_null() && foreground_is_fullscreen(fg)) || quns_busy()),
                in_hide_list: exe.as_deref().is_some_and(|e| listed(e, &hide_l)),
                in_show_list: in_show,
            };
            let why = crate::fspeek::why_hide(facts);
            crate::fspeek::publish_why(why);
            if crate::fspeek::peek() != crate::fspeek::PEEK_NONE && !crate::pill::is_expanded() {
                crate::pill::end_peek(&app); // the island that came over a fullscreen app has folded again
            }
            let bad = crate::fspeek::hides(why, crate::fspeek::peek());
            // `autoHideKeepDesktop`: the desktop in front brings an auto-hidden pill back (the page also stops sliding it away there);
            // a program of `showInApps` in front does the same
            if !fg.0.is_null() && crate::pill::auto_hidden() && ((crate::settings::get_bool("autoHideKeepDesktop", false) && is_desktop(fg)) || in_show) {
                set_hidden_bg(&app, crate::pill::HIDE_AUTO, false, "desktop");
            }
            if bad {
                clear = 0;
                if !hidden {
                    hidden = true;
                    set_hidden_bg(&app, HIDE_GUARD, true, "guard");
                }
            } else {
                clear += 1;
                if hidden && clear >= 2 {
                    hidden = false;
                    set_hidden_bg(&app, HIDE_GUARD, false, "guard");
                }
            }
            if !hidden {
                relayout(&app);
                crate::pill::keep_alive(&app); // another program hid / minimised / un-topmosted the island: bring it back
            }
            let watch_edge = hidden
                && why & crate::fspeek::WHY_LIST == 0
                && crate::fspeek::peek() == crate::fspeek::PEEK_NONE
                && crate::settings::get_bool("fullscreenPeek", true)
                && crate::settings::get_bool("fullscreenPeekEdge", false);
            if watch_edge {
                edge_wait(&app);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::listed;

    #[test]
    fn a_list_entry_matches_the_exe_with_or_without_the_suffix_in_any_case() {
        let l = vec!["Photoshop.exe".to_string(), " blender ".to_string()];
        assert!(listed("photoshop.exe", &l) && listed("blender.exe", &l));
        assert!(!listed("chrome.exe", &l) && !listed("photoshop2.exe", &l));
        assert!(!listed("chrome.exe", &[]));
    }
}
