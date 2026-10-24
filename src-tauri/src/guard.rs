// Hide the pill over fullscreen / exclusive-fullscreen / chosen apps. Owner: pill agent (lane L1).
// One 500 ms poll: hide at once, show after 2 clear polls (QUNS lags after alt-tab). While visible the same poll
// re-places the window when monitor / DPI / taskbar changed. Hiding itself (SW_HIDE + pill:hide) lives in pill.rs.
use crate::pill::{relayout, set_hidden_bg, HIDE_GUARD};
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

/// Page query before it slides the pill away (setting `autoHideKeepDesktop`).
#[tauri::command]
pub fn pill_desktop_fg() -> bool {
    let fg = unsafe { GetForegroundWindow() };
    !fg.0.is_null() && is_desktop(fg)
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

fn in_hide_list(fg: HWND) -> bool {
    let list = crate::settings::get_list("hideInApps");
    if list.is_empty() {
        return false;
    }
    let Some(exe) = exe_name(fg) else { return false };
    let stem = exe.strip_suffix(".exe").unwrap_or(&exe).to_string();
    list.iter().any(|e| {
        let e = e.trim().to_lowercase();
        e == exe || e == stem
    })
}

pub fn init(app: &tauri::App) {
    let app = app.handle().clone();
    std::thread::spawn(move || {
        let (mut hidden, mut clear) = (false, 0u32);
        loop {
            let fg = unsafe { GetForegroundWindow() };
            // `hideInFullscreen` (default on): off = fullscreen windows / D3D fullscreen never hide the pill, only the hide list does
            let fs = crate::settings::get_bool("hideInFullscreen", true);
            let bad = !fg.0.is_null() && ((fs && foreground_is_fullscreen(fg)) || in_hide_list(fg)) || (fs && quns_busy());
            // `autoHideKeepDesktop`: the desktop in front brings an auto-hidden pill back (the page also stops sliding it away there)
            if !fg.0.is_null() && crate::pill::auto_hidden() && crate::settings::get_bool("autoHideKeepDesktop", false) && is_desktop(fg) {
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
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}
