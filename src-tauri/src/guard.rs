// Hide the pill over fullscreen / exclusive-fullscreen / chosen apps. Owner: pill agent (lane L1).
// Event driven: the foreground hook of taskbar.rs (foreground change, and the foreground window moving / resizing) and a display change wake
// the guard thread, which re-checks at once (hide the moment a window covers the monitor). A slow poll stays as the safety net (and re-places
// the window when monitor / DPI / taskbar changed). Show only after the verdict has been clear for `SHOW_DEBOUNCE_MS` (QUNS lags after alt-tab).
// Hiding itself (SW_HIDE + pill:hide) lives in pill.rs.
use crate::pill::{relayout, set_hidden_bg, HIDE_GUARD};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
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

/// How long the verdict must stay "clear" before the island comes back (the notification state and the foreground lag a moment after alt-tab).
const SHOW_DEBOUNCE_MS: u64 = 400;
/// The safety-net poll: events do the real work; this catches a change that raised no event (and re-places the window).
const POLL_MS: u64 = 1000;
/// While the island is hidden by the guard, look again this soon at the latest.
const HIDDEN_POLL_MS: u64 = 500;
/// After handling an event wake-up, wait this long (a window being dragged fires location events at every pixel): a burst becomes one look.
const COOLDOWN_MS: u64 = 50;

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Stay,
    Hide,
    Show,
}

/// One observation of the guard verdict (`bad` = the island should be hidden) at `now_ms`. Hide at once; show once the verdict has been clear for
/// `SHOW_DEBOUNCE_MS` (`clear_since` remembers when the clear run began).
fn step(hidden: bool, bad: bool, clear_since: &mut Option<u64>, now_ms: u64) -> Step {
    if bad {
        *clear_since = None;
        return if hidden { Step::Stay } else { Step::Hide };
    }
    let since = *clear_since.get_or_insert(now_ms);
    if hidden && now_ms.saturating_sub(since) >= SHOW_DEBOUNCE_MS {
        Step::Show
    } else {
        Step::Stay
    }
}

/// How long the guard thread may sleep before it must look again.
fn wait_ms(hidden: bool, clear_since: Option<u64>, now_ms: u64) -> u64 {
    match (hidden, clear_since) {
        (true, Some(since)) => (since + SHOW_DEBOUNCE_MS).saturating_sub(now_ms).clamp(10, HIDDEN_POLL_MS),
        (true, None) => HIDDEN_POLL_MS,
        _ => POLL_MS,
    }
}

/// The `QUNS_*` states that mean something is in front of the desktop. Exclusive Direct3D fullscreen is not drawn over at all (the island would be
/// invisible anyway), so hiding then only saves the pointless frames; "busy" and presentation mode are the user's own request not to be disturbed.
fn quns_hides(busy: bool, d3d_full: bool, presentation: bool) -> bool {
    busy || d3d_full || presentation
}

/// Exclusive Direct3D fullscreen, presentation mode, "busy" (QUERY_USER_NOTIFICATION_STATE).
fn quns_busy() -> bool {
    unsafe { SHQueryUserNotificationState().is_ok_and(|s| quns_hides(s == QUNS_BUSY, s == QUNS_RUNNING_D3D_FULL_SCREEN, s == QUNS_PRESENTATION_MODE)) }
}

/// What woke the guard thread since it last looked: `taskbar::EV_*` bits, plus `EV_DISPLAY`.
static PENDING: AtomicU8 = AtomicU8::new(0);
const EV_DISPLAY: u8 = 4;
static THREAD: OnceLock<std::thread::Thread> = OnceLock::new();

fn on_event(ev: u8) {
    PENDING.fetch_or(ev, Ordering::Relaxed);
    if let Some(t) = THREAD.get() {
        t.unpark();
    }
}

/// The monitor layout / resolution changed (`WM_DISPLAYCHANGE`): re-check now. Cheap and safe from any thread.
pub fn display_changed() {
    on_event(EV_DISPLAY);
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
/// resting on the island's edge (the top, or the bottom when the island hangs from there) for 150 ms brings the island over the app. Runs only in that state.
fn edge_wait(app: &tauri::AppHandle) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let t0 = std::time::Instant::now();
    let mut dwell = crate::fspeek::EdgeDwell::default();
    while t0.elapsed() < std::time::Duration::from_millis(500) {
        std::thread::park_timeout(Duration::from_millis(60));
        if PENDING.load(Ordering::Relaxed) & (crate::taskbar::EV_FOREGROUND | EV_DISPLAY) != 0 {
            return; // the foreground changed: the main loop decides again at once
        }
        let Some((mon, xr, bottom)) = crate::pill::edge_geometry() else { continue };
        let mut p = POINT::default();
        if unsafe { GetCursorPos(&mut p) }.is_err() {
            continue;
        }
        let held = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0;
        if dwell.step(t0.elapsed().as_millis() as u64, crate::fspeek::on_edge((p.x, p.y), mon, xr, held, bottom)) {
            crate::pill::peek(app, "edge");
            return;
        }
    }
}

pub fn init(app: &tauri::App) {
    let app = app.handle().clone();
    crate::taskbar::set_listener(on_event);
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut clear_since: Option<u64> = None;
        let _ = THREAD.set(std::thread::current());
        loop {
            let woke = PENDING.swap(0, Ordering::Relaxed);
            let now = t0.elapsed().as_millis() as u64;
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
            match step(hidden, bad, &mut clear_since, now) {
                Step::Hide => {
                    hidden = true;
                    set_hidden_bg(&app, HIDE_GUARD, true, "guard");
                }
                Step::Show => {
                    hidden = false;
                    set_hidden_bg(&app, HIDE_GUARD, false, "guard");
                }
                Step::Stay => {}
            }
            // a mere location event (a window being moved) needs the verdict only: the re-place / keep-alive work waits for the poll or a real change
            if !hidden && woke != crate::taskbar::EV_LOCATION {
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
                if woke != 0 {
                    std::thread::sleep(Duration::from_millis(COOLDOWN_MS));
                }
                if PENDING.load(Ordering::Relaxed) == 0 {
                    std::thread::park_timeout(Duration::from_millis(wait_ms(hidden, clear_since, t0.elapsed().as_millis() as u64)));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_entry_matches_the_exe_with_or_without_the_suffix_in_any_case() {
        let l = vec!["Photoshop.exe".to_string(), " blender ".to_string()];
        assert!(listed("photoshop.exe", &l) && listed("blender.exe", &l));
        assert!(!listed("chrome.exe", &l) && !listed("photoshop2.exe", &l));
        assert!(!listed("chrome.exe", &[]));
    }

    #[test]
    fn hides_at_once_and_shows_only_after_the_debounce() {
        let mut c = None;
        assert_eq!(step(false, false, &mut c, 0), Step::Stay, "nothing to hide, nothing hidden");
        assert_eq!(step(false, true, &mut c, 10), Step::Hide, "a fullscreen window: hide in the same look");
        assert_eq!(step(true, true, &mut c, 20), Step::Stay);
        assert_eq!(step(true, false, &mut c, 1000), Step::Stay, "the first clear look only starts the debounce");
        assert_eq!(step(true, false, &mut c, 1000 + SHOW_DEBOUNCE_MS - 1), Step::Stay);
        assert_eq!(step(true, false, &mut c, 1000 + SHOW_DEBOUNCE_MS), Step::Show);
    }

    #[test]
    fn a_blip_of_busy_restarts_the_debounce() {
        let mut c = None;
        assert_eq!(step(true, false, &mut c, 100), Step::Stay);
        assert_eq!(step(true, true, &mut c, 300), Step::Stay, "alt-tab lag: the state went busy again");
        assert_eq!(c, None);
        assert_eq!(step(true, false, &mut c, 450), Step::Stay);
        assert_eq!(step(true, false, &mut c, 450 + SHOW_DEBOUNCE_MS), Step::Show);
    }

    #[test]
    fn the_thread_sleeps_until_the_debounce_ends_and_otherwise_polls_slowly() {
        assert_eq!(wait_ms(false, None, 5), POLL_MS, "visible and clear: only the slow safety net");
        assert_eq!(wait_ms(true, None, 5), HIDDEN_POLL_MS);
        assert_eq!(wait_ms(true, Some(1000), 1100), SHOW_DEBOUNCE_MS - 100);
        assert_eq!(wait_ms(true, Some(1000), 5000), 10, "already due: look again now, never a zero wait");
    }

    #[test]
    fn every_blocking_notification_state_hides() {
        assert!(quns_hides(true, false, false) && quns_hides(false, true, false) && quns_hides(false, false, true));
        assert!(!quns_hides(false, false, false));
    }
}
