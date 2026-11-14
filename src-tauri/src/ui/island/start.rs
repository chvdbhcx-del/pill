//! Bringing the native island up: host thread, fonts, the backend facade, the Overlay window,
//! and the hand-over to `pill.rs` (placement / visibility through the `PillWin` seam).
use super::view::IslandView;
use crate::ui::host::pillwin::NativePillWin;
use crate::ui::host::geom::Dock;
use crate::ui::host::{MonitorSel, NativeWindow, Placement};
use tauri::AppHandle;

/// Creates the Overlay (before `pill::init`).
pub fn start(app: &AppHandle) {
    let h = crate::ui::native::ensure_host(app);
    let mut win = NativeWindow::overlay("pill");
    if crate::pill::dock() == Dock::Bottom {
        // born at the bottom edge: `pill::native_ready` applies the real monitor / offset right after, this only avoids a flash at the top
        win = win.placement(Placement::BottomCenter { monitor: MonitorSel::Primary, offset_pct: 0.0, above_taskbar: true, margin_bottom: 0.0 });
    }
    let id = win.spawn(&h, |cx| Box::new(IslandView::new(cx)));
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
