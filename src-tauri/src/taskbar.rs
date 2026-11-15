// The Windows taskbar as the island inside it needs it (`edge.position` = taskbar): where its notification area is (the island never stands on
// the tray), and keeping the island above the bar. The taskbar is itself topmost and puts itself on top of the other topmost windows whenever it
// is clicked, so the island (a topmost overlay positioned over the bar, never a child of `Shell_TrayWnd`) raises itself back:
//  * at once, from an `EVENT_SYSTEM_FOREGROUND` hook that wakes only when the foreground window changes (zero idle cost, no timer);
//  * and from the guard poll (500 ms, already running) as a safety net for a raise that gave no focus event.
// Neither activates the island or takes the focus: `SWP_NOACTIVATE`, no move, no resize.
use crate::ui::host::geom::IRect;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::core::w;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetClassNameW, GetWindow, GetWindowRect, IsWindowVisible, SetWindowPos, EVENT_SYSTEM_FOREGROUND, GW_HWNDPREV, HWND_TOPMOST,
    SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, WINEVENT_OUTOFCONTEXT,
};

/// The window classes of the primary and of a secondary monitor's taskbar.
pub fn is_taskbar_class(c: &str) -> bool {
    matches!(c, "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// Is a taskbar among the windows above the island? `above` = the classes of the windows in front of it, nearest first. Pure.
pub fn covered_by_taskbar<'a>(above: impl IntoIterator<Item = &'a str>) -> bool {
    above.into_iter().any(is_taskbar_class)
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn rect_of(h: HWND) -> Option<IRect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok()?;
    Some(IRect::new(r.left, r.top, r.right, r.bottom))
}

/// Every taskbar window: the primary one and the secondary ones (one per other monitor).
fn taskbars() -> Vec<HWND> {
    let mut v = Vec::new();
    unsafe {
        if let Ok(h) = FindWindowW(w!("Shell_TrayWnd"), None) {
            v.push(h);
        }
        let mut prev = HWND::default();
        while let Ok(h) = FindWindowExW(None, prev, w!("Shell_SecondaryTrayWnd"), None) {
            if h.0.is_null() || v.len() > 16 {
                break;
            }
            v.push(h);
            prev = h;
        }
    }
    v
}

/// The notification area (`TrayNotifyWnd`: the tray icons and the clock) of the taskbar that lies along the bottom of `mon` (physical px).
/// `None` when that bar has none (a secondary bar of some Windows builds) or it is not visible: the island then has the whole bar.
pub fn tray_rect(mon: IRect) -> Option<IRect> {
    for bar in taskbars() {
        let Some(r) = rect_of(bar) else { continue };
        let on_this_monitor = r.l >= mon.l - 2 && r.r <= mon.r + 2 && (r.b - mon.b).abs() <= 2;
        if !on_this_monitor {
            continue;
        }
        let tray = unsafe { FindWindowExW(bar, None, w!("TrayNotifyWnd"), None) }.ok().filter(|t| !t.0.is_null() && unsafe { IsWindowVisible(*t) }.as_bool());
        return tray.and_then(rect_of);
    }
    None
}

/// Puts the island back above a taskbar that got in front of it. True when it had to. No activation, no move.
pub fn raise_over_taskbar(island: HWND) -> bool {
    if island.0.is_null() {
        return false;
    }
    let mut classes = Vec::new();
    let mut w = island;
    for _ in 0..32 {
        match unsafe { GetWindow(w, GW_HWNDPREV) } {
            Ok(p) if !p.0.is_null() => {
                classes.push(class_of(p));
                w = p;
            }
            _ => break,
        }
    }
    let covered = covered_by_taskbar(classes.iter().map(String::as_str));
    if covered {
        // async: the island belongs to the host thread, never wait for it from the thread that runs the hook
        unsafe {
            let _ = SetWindowPos(island, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS);
        }
    }
    covered
}

static ISLAND: AtomicIsize = AtomicIsize::new(0);
static FOLLOW: AtomicBool = AtomicBool::new(false);
static HOOKED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_foreground(_hook: HWINEVENTHOOK, _event: u32, hwnd: HWND, _object: i32, _child: i32, _thread: u32, _ms: u32) {
    if FOLLOW.load(Ordering::Relaxed) && is_taskbar_class(&class_of(hwnd)) {
        raise_over_taskbar(HWND(ISLAND.load(Ordering::Relaxed) as *mut c_void));
    }
}

/// The island stands in the taskbar (`on`) or not any more. The hook is installed once, from the thread that calls this (the UI thread: the
/// out-of-context callback is delivered through its message loop) and does nothing while `on` is false.
pub fn follow(island: HWND, on: bool) {
    FOLLOW.store(on && !island.0.is_null(), Ordering::Relaxed);
    if !on || island.0.is_null() {
        return;
    }
    ISLAND.store(island.0 as isize, Ordering::Relaxed);
    if !HOOKED.swap(true, Ordering::Relaxed) {
        let h = unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT) };
        if h.0.is_null() {
            HOOKED.store(false, Ordering::Relaxed); // the guard poll still keeps the island on top
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_taskbar_classes_cover_the_island() {
        assert!(is_taskbar_class("Shell_TrayWnd") && is_taskbar_class("Shell_SecondaryTrayWnd"));
        for c in ["Progman", "WorkerW", "TrayNotifyWnd", "Chrome_WidgetWin_1", "", "shell_traywnd"] {
            assert!(!is_taskbar_class(c), "{c:?}");
        }
    }

    #[test]
    fn a_taskbar_anywhere_above_the_island_means_it_is_covered() {
        assert!(!covered_by_taskbar([]), "the island is at the top");
        assert!(!covered_by_taskbar(["Chrome_WidgetWin_1", "Notepad"]), "other topmost windows are none of our business");
        assert!(covered_by_taskbar(["Shell_TrayWnd"]));
        assert!(covered_by_taskbar(["Notepad", "Shell_SecondaryTrayWnd", "Chrome_WidgetWin_1"]));
    }

    #[test]
    fn there_is_nothing_to_raise_without_a_window() {
        assert!(!raise_over_taskbar(HWND::default()));
        follow(HWND::default(), true); // no window: the hook is not installed, nothing is followed
        assert!(!FOLLOW.load(Ordering::Relaxed) && !HOOKED.load(Ordering::Relaxed));
    }
}
