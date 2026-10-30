//! The seam between `pill.rs` and the island window. `pill.rs` drives the island Overlay only through this trait
//! (placement, show / hide, capture affinity). [`NativePillWin`] is the implementation (commands to the host thread).
//!
//! All sizes and positions are PHYSICAL screen px, exactly as `pill.rs::place` computes them.
use super::{HostHandle, IRect};
use crate::ui::view::WindowId;

pub trait PillWin: Send + Sync {
    /// The window handle (`HWND` as isize); 0 until the window exists.
    fn hwnd(&self) -> isize;
    /// `[left, top, right, bottom]` of the monitor the window is on (`MONITORINFO::rcMonitor`); `[0; 4]` if unknown.
    fn monitor_rect(&self) -> [i32; 4];
    /// Device px per logical px of that monitor (`dpi / 96`).
    fn scale(&self) -> f64;
    /// Resize keeping the top-left corner.
    fn set_size(&self, w: i32, h: i32);
    /// Move and resize (topmost, no activation): what `place()` does with `SetWindowPos`.
    fn set_bounds(&self, x: i32, y: i32, w: i32, h: i32);
    /// `ShowWindow(SW_SHOWNOACTIVATE)` / `SW_HIDE`.
    fn set_visible(&self, visible: bool);
    /// Click-through on / off (`pill.rs::set_ignore`). The native window normally decides this itself from the cursor (`proximity`); once
    /// the old cursor loop calls this, its answer wins until [`PillWin::release_ignore`].
    fn set_ignore(&self, ignore: bool);
    /// Hand click-through back to the window's own proximity logic.
    fn release_ignore(&self);
}

/// The native Overlay seen through [`PillWin`]. Commands are asynchronous (queued to the UI thread); the queries read the OS directly.
pub struct NativePillWin {
    host: HostHandle,
    id: WindowId,
    rect: std::sync::Mutex<IRect>,
}

impl NativePillWin {
    pub fn new(host: HostHandle, id: WindowId) -> NativePillWin {
        NativePillWin { host, id, rect: std::sync::Mutex::new(IRect::default()) }
    }

    fn last(&self) -> IRect {
        *self.rect.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl PillWin for NativePillWin {
    fn hwnd(&self) -> isize {
        self.host.hwnd(self.id).unwrap_or(0)
    }

    fn monitor_rect(&self) -> [i32; 4] {
        super::win32::monitor_of_window(self.hwnd()).map_or([0; 4], |m| [m.rect.l, m.rect.t, m.rect.r, m.rect.b])
    }

    fn scale(&self) -> f64 {
        super::win32::monitor_of_window(self.hwnd()).map_or(1.0, |m| m.scale as f64)
    }

    fn set_size(&self, w: i32, h: i32) {
        let cur = self.last();
        self.set_bounds(cur.l, cur.t, w, h);
    }

    fn set_bounds(&self, x: i32, y: i32, w: i32, h: i32) {
        let r = IRect::from_xywh(x, y, w, h);
        *self.rect.lock().unwrap_or_else(|e| e.into_inner()) = r;
        self.host.set_bounds(self.id, r);
    }

    fn set_visible(&self, visible: bool) {
        if visible {
            self.host.show(self.id)
        } else {
            self.host.hide(self.id)
        }
    }

    fn set_ignore(&self, ignore: bool) {
        self.host.force_ignore(self.id, Some(ignore));
    }

    fn release_ignore(&self) {
        self.host.force_ignore(self.id, None);
    }
}
