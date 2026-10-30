//! Bringing the native island up: host thread, fonts, the backend facade, the Overlay window,
//! and the hand-over to `pill.rs` (placement / visibility through the `PillWin` seam).
use super::view::IslandView;
use crate::ui::host::pillwin::NativePillWin;
use crate::ui::host::NativeWindow;
use tauri::AppHandle;

/// Creates the Overlay (before `pill::init`).
pub fn start(app: &AppHandle) {
    let h = crate::ui::native::ensure_host(app);
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
