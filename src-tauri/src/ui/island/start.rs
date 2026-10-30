//! Bringing the native island up (flag `ui.native.pill`, never with `--web-ui`): host thread, fonts, the backend facade, the Overlay window,
//! and the hand-over to `pill.rs` (placement / visibility through the `PillWin` seam).
use super::fonts::FONTS;
use super::view::IslandView;
use crate::ui::host::pillwin::NativePillWin;
use crate::ui::host::{self, NativeWindow};
use crate::ui::view::Api;
use serde_json::Value;
use std::sync::Arc;
use tauri::AppHandle;

/// The real backend for native views: `ui::api::call` (the same functions the `#[tauri::command]` wrappers call).
pub struct FacadeApi;

impl Api for FacadeApi {
    fn call(&self, caller: &str, name: &str, args: Value) -> Result<Value, String> {
        crate::ui::api::call(name, args, caller)
    }
}

/// True when the island must be the native Overlay: the flag is on and the WebView UI is not forced.
pub fn enabled() -> bool {
    crate::ui::flags::native_pill()
}

/// Creates the Overlay (call instead of `windows::build_pill`, before `pill::init`).
pub fn start(app: &AppHandle) {
    crate::ui::api::init(app);
    host::set_fonts(&FONTS);
    host::set_api(Arc::new(FacadeApi));
    let h = host::start();
    let id = NativeWindow::overlay("pill").spawn(&h, |cx| Box::new(IslandView::new(cx)));
    crate::pill::attach_native(Box::new(NativePillWin::new(h.clone(), id)));
    let app = app.clone();
    std::thread::Builder::new()
        .name("island-ready".into())
        .spawn(move || {
            for _ in 0..250 {
                if h.hwnd(id).is_some() {
                    crate::pill::native_ready(&app);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            eprintln!("pill: the native island window was not created");
        })
        .ok();
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_flag_is_off_by_default_so_the_web_island_is_built() {
        // no settings.json key `ui.native.pill` in the test environment
        assert!(!super::enabled());
    }
}
