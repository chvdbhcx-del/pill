//! OLE drag-and-drop target for native windows (the shelf takes files and text dropped onto the island). A window that sets
//! `accept_drops` registers one `IDropTarget`; its callbacks run on the UI thread (the OLE STA) from inside the message loop, translate the
//! `IDataObject` into a [`DropPayload`] and call `View::on_drop` through the host. A view accepts a drag by returning
//! `Response::consumed()` from `Enter` / `Over` (copy cursor); anything else shows the "not allowed" cursor.
//!
//! A click-through window is invisible to OLE hit testing, which is why the island's proximity logic makes it hit-testable while a drag
//! that started elsewhere is over the drop zone (`proximity::Drag`).
use super::win32::with_host;
use crate::ui::view::{DropPayload, DropPhase, WindowId};
use std::cell::RefCell;
use std::path::PathBuf;
use windows::core::{implement, Result};
use windows::Win32::Foundation::{HWND, POINTL};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

const CF_UNICODETEXT: u16 = 13;
const CF_HDROP: u16 = 15;

#[implement(IDropTarget)]
struct Target {
    id: WindowId,
    /// what the drag carried at `DragEnter`, for the `DragOver` / `Drop` steps that get no data object of their own
    last: RefCell<Option<DropPayload>>,
}

/// Registers `hwnd` as a drop target. False when OLE refuses (the window then simply gets no drops).
pub(super) fn register(hwnd: HWND, id: WindowId) -> bool {
    let target: IDropTarget = Target { id, last: RefCell::new(None) }.into();
    unsafe { RegisterDragDrop(hwnd, &target).is_ok() }
}

fn effect(accept: bool) -> DROPEFFECT {
    if accept {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, pdataobj: Option<&IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        let payload = pdataobj.and_then(|d| unsafe { read_payload(d) });
        *self.last.borrow_mut() = payload.clone();
        let ok = with_host(|h| h.on_drop(self.id, DropPhase::Enter, (pt.x, pt.y), payload)).unwrap_or(false);
        unsafe { *pdweffect = effect(ok) };
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        let ok = with_host(|h| h.on_drop(self.id, DropPhase::Over, (pt.x, pt.y), None)).unwrap_or(false);
        unsafe { *pdweffect = effect(ok) };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.last.borrow_mut().take();
        let _ = with_host(|h| h.on_drop(self.id, DropPhase::Leave, (0, 0), None));
        Ok(())
    }

    fn Drop(&self, pdataobj: Option<&IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, pdweffect: *mut DROPEFFECT) -> Result<()> {
        let payload = pdataobj.and_then(|d| unsafe { read_payload(d) }).or_else(|| self.last.borrow_mut().take());
        let ok = with_host(|h| h.on_drop(self.id, DropPhase::Drop, (pt.x, pt.y), payload)).unwrap_or(false);
        unsafe { *pdweffect = effect(ok) };
        Ok(())
    }
}

/// Files win over text (a file dragged from Explorer also offers its name as text).
unsafe fn read_payload(data: &IDataObject) -> Option<DropPayload> {
    if let Some(files) = read_files(data) {
        return Some(DropPayload::Files(files));
    }
    read_text(data).map(DropPayload::Text)
}

fn format(cf: u16) -> FORMATETC {
    FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0 as u32, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

unsafe fn read_files(data: &IDataObject) -> Option<Vec<PathBuf>> {
    let mut med = data.GetData(&format(CF_HDROP)).ok()?;
    let hdrop = HDROP(med.u.hGlobal.0 as _);
    let n = DragQueryFileW(hdrop, u32::MAX, None);
    let mut out = Vec::new();
    for i in 0..n {
        let len = DragQueryFileW(hdrop, i, None) as usize;
        let mut buf = vec![0u16; len + 1];
        let got = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
        out.push(PathBuf::from(String::from_utf16_lossy(&buf[..got.min(len)])));
    }
    ReleaseStgMedium(&mut med);
    (!out.is_empty()).then_some(out)
}

unsafe fn read_text(data: &IDataObject) -> Option<String> {
    let mut med = data.GetData(&format(CF_UNICODETEXT)).ok()?;
    let h = med.u.hGlobal;
    let p = GlobalLock(h) as *const u16;
    let text = if p.is_null() {
        None
    } else {
        let mut n = 0;
        while *p.add(n) != 0 && n < 1 << 20 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        let _ = GlobalUnlock(h);
        Some(s)
    };
    ReleaseStgMedium(&mut med);
    text.filter(|t| !t.is_empty())
}
