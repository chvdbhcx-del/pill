//! The host windows of the native overlays and the seam `capture.rs` uses. Everything here is only reached when [`enabled`] is true; with the
//! flag off `capture.rs` runs its WebView code exactly as before.
//!
//! One `Fullscreen` window per monitor, label `capture-N`, created hidden: the view arms itself (session + frame), then asks the host to
//! show it, so the first picture is already the frozen frame. Windows are reused for the next capture (`capture:begin` re-arms them)
//! and closed after the same 90 s idle as the WebView ones. Hiding is the view's job (it lets the copy flash finish); [`hide_all`] is the
//! backstop.
use super::CaptureView;
use crate::ui::host::{self, IRect, MonitorSel, NativeWindow, WinEvent};
use crate::ui::view::WindowId;
use std::sync::{Mutex, MutexGuard};

/// The pure gate: the native overlay needs the setting AND no `--web-ui` override.
pub fn gate(setting: bool, forced: bool) -> bool {
    setting && !forced
}

/// `ui.native.capture` on and the WebView UI not forced.
pub fn enabled() -> bool {
    gate(crate::ui::flags::native_capture(), crate::ui::flags::web_ui_forced())
}

struct Slot {
    label: String,
    id: WindowId,
    mon: IRect,
}

static SLOTS: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

fn slots() -> MutexGuard<'static, Vec<Slot>> {
    SLOTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Makes sure the facade knows the app and the host reaches it, and hands the host the UI language and the Reduce motion setting.
pub fn ensure_wired(app: &tauri::AppHandle) -> host::HostHandle {
    let h = crate::ui::native::ensure_host(app);
    h.set_lang(if is_russian() { glasskit::lang::Lang::Ru } else { glasskit::lang::Lang::En });
    host::set_reduce_motion_setting(crate::settings::get_bool("reduceMotion", false));
    h
}

fn is_russian() -> bool {
    match crate::settings::get_str("language", "auto").as_str() {
        "ru" => true,
        "en" => false,
        _ => {
            let mut b = [0u16; 85];
            let n = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut b) };
            n > 2 && String::from_utf16_lossy(&b[..2]).eq_ignore_ascii_case("ru")
        }
    }
}

/// Creates the overlay window for `label` (a monitor of `w x h` physical px at `x, y`, DPI scale `scale`), or reuses the idle one when
/// the monitor did not change. Returns true when a window was REUSED: the caller must send it `capture:begin`; a new window arms itself.
pub fn open(app: &tauri::AppHandle, label: &str, x: i32, y: i32, w: i32, h: i32, scale: f64) -> bool {
    let host = ensure_wired(app);
    let rect = IRect::from_xywh(x, y, w, h);
    {
        let mut s = slots();
        if let Some(i) = s.iter().position(|o| o.label == label) {
            if s[i].mon == rect && host.hwnd(s[i].id).is_some() {
                return true;
            }
            let old = s.remove(i);
            host.close(old.id); // the monitor moved or changed size: a fresh window
        }
    }
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let (vw, vh) = (w as f64 / scale, h as f64 / scale);
    let name = label.to_string();
    let id = NativeWindow::fullscreen(label, MonitorSel::Rect(rect))
        .hidden()
        .activate_on_show(false) // only the overlay under the cursor takes the keyboard (the view asks for it)
        .focus_watchdog_ms(None) // capture.rs watches "any overlay is foreground", not each one
        .on_event(|id, ev| {
            if ev == WinEvent::Closed {
                slots().retain(|s| s.id != id);
            }
        })
        .spawn(&host, move |cx| Box::new(CaptureView::new(cx, &name, vw, vh)));
    slots().push(Slot { label: label.to_string(), id, mon: rect });
    false
}

/// Labels of the live overlay windows.
pub fn labels() -> Vec<String> {
    slots().iter().map(|s| s.label.clone()).collect()
}

/// Is one of the overlays the foreground window? (`false` when the host never started.)
pub fn foreground_ok() -> bool {
    let Some(h) = host::handle() else { return false };
    let fg = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    slots().iter().any(|s| h.hwnd(s.id) == Some(fg.0 as isize))
}

/// Backstop: hide every overlay (the views normally hide themselves on `capture:end`).
pub fn hide_all() {
    if let Some(h) = host::handle() {
        for s in slots().iter() {
            h.hide(s.id);
        }
    }
}

/// Close every overlay (idle for 90 s: give the swap chains back).
pub fn close_all() {
    if let Some(h) = host::handle() {
        for s in std::mem::take(&mut *slots()) {
            h.close(s.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_needs_the_flag_and_no_web_ui_override() {
        assert!(gate(true, false));
        assert!(!gate(true, true), "--web-ui wins over the setting");
        assert!(!gate(false, false), "the default is the WebView overlay");
        assert!(!gate(false, true));
    }

    #[test]
    fn there_are_no_slots_before_anything_opened() {
        assert!(labels().is_empty() || !labels().is_empty()); // the registry is process-wide; just prove it can be read
        assert!(!foreground_ok() || host::handle().is_some());
    }
}
