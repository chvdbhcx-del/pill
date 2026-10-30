//! Window glue of the native bar: the popup spec, the lazily created window, and the entry points `command.rs` calls when the native bar is on
//! (`bar_toggle`, `bar_hide`, `bar_is_visible`). Nothing here is used while the flag is off: `command.rs` asks `bar_is_native()` first and keeps
//! its WebView code path untouched.
use super::{BarView, BAR_LABEL};
use crate::ui::flags;
use crate::ui::host::{self, HostHandle, NativeWindow, WinEvent};
use crate::ui::view::{Api, WindowId};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static BAR_ID: AtomicU64 = AtomicU64::new(0);
static CREATE: Mutex<()> = Mutex::new(());

/// The one rule of the gate: native only when the setting is on AND the WebView UI was not forced from the command line.
pub fn native_active_from(setting: bool, forced: bool) -> bool {
    setting && !forced
}

/// Is the command bar native in this run? (`ui.native.bar` and no `--web-ui`.)
pub fn native_active() -> bool {
    flags::native_bar()
}

/// Same as [`native_active`]; the name `command.rs` and `ui/lazy.rs` use.
pub fn bar_is_native() -> bool {
    native_active()
}

/// The `ui::api` facade as the host's backend (idempotent: the last `set_api` wins and they all are this).
struct Facade;

impl Api for Facade {
    fn call(&self, caller: &str, name: &str, args: Value) -> Result<Value, String> {
        crate::ui::api::call(name, args, caller)
    }
}

fn wire(app: &tauri::AppHandle) {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        crate::ui::api::init(app);
        host::set_api(Arc::new(Facade));
    });
}

/// The window builder: `Popup` kind, 560 x 350, top centre of the monitor under the cursor, hidden, takes the foreground when shown, hides when
/// it loses it (host `BlurGuard`, 400 ms grace), cleans the backend's in-memory rows whenever it goes away.
pub fn spec() -> NativeWindow {
    NativeWindow::popup(BAR_LABEL).on_event(|id, e| match e {
        WinEvent::Hidden(_) => crate::command::bar_hidden_cleanup(),
        WinEvent::Closed => {
            let _ = BAR_ID.compare_exchange(id.0, 0, Ordering::SeqCst, Ordering::SeqCst);
        }
        _ => {}
    })
}

/// Registers the window under the name `bar` (so `Cx::open_window("bar", ..)` and `HostHandle::open` find it).
pub fn register(host: &HostHandle) {
    spec().register(host, BAR_LABEL, |cx, _| Box::new(BarView::new(cx)));
}

/// The id of the bar window, creating it (hidden) on first use. Safe from any thread.
fn ensure(host: &HostHandle) -> WindowId {
    let _g = CREATE.lock().unwrap_or_else(|e| e.into_inner());
    let cur = BAR_ID.load(Ordering::SeqCst);
    if cur != 0 {
        return WindowId(cur);
    }
    let id = spec().spawn(host, |cx| Box::new(BarView::new(cx)));
    BAR_ID.store(id.0, Ordering::SeqCst);
    id
}

/// `cmd_toggle` for the native bar: show (under the island when it is showing) or hide. The host swallows the open request that follows
/// a blur-hide within 300 ms (the click that closed the bar must not re-open it).
pub fn bar_toggle(app: &tauri::AppHandle) {
    wire(app);
    let h = host::start();
    let id = ensure(&h);
    match below_island(&h) {
        Some(y) => h.toggle_below(id, y),
        None => h.toggle(id),
    }
}

pub fn bar_hide() {
    if let Some(h) = host::handle() {
        let id = BAR_ID.load(Ordering::SeqCst);
        if id != 0 {
            h.hide(WindowId(id));
        }
    }
}

pub fn bar_is_visible() -> bool {
    let id = BAR_ID.load(Ordering::SeqCst);
    id != 0 && host::handle().is_some_and(|h| h.is_visible(WindowId(id)))
}

/// Physical y below the island body on the monitor under the cursor (+ 8 logical px), like `command::place`.
fn below_island(h: &HostHandle) -> Option<i32> {
    let (cx, cy) = cursor();
    let mons = h.monitors();
    let m = mons.iter().find(|m| cx >= m.rect.l && cx < m.rect.r && cy >= m.rect.t && cy < m.rect.b).or_else(|| mons.iter().find(|m| m.primary))?;
    crate::pill::body_bottom(m.rect.l, m.rect.t).map(|b| b + (8.0 * m.scale).round() as i32)
}

fn cursor() -> (i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// `AppsUseLightTheme` (HKCU Personalize): the OS preference `theme = auto` follows (`prefers-color-scheme` of the web page).
pub fn os_prefers_light() -> bool {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut v: u32 = 0;
    let mut n = 4u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut n)) };
    r.is_ok() && v != 0
}
