//! Screen-reader / UI Automation support for the host's windows. The windows are drawn with Direct2D, so UIA would see an empty
//! pane. Each window answers `WM_GETOBJECT` with one read-only element (a pane) named by `View::accessible()` (the island: "Pill, 12:30").
//!
//! UIA calls the provider from its own threads while the view lives on the UI thread, so every read of the name goes through a
//! synchronous `SendMessageTimeoutW(WM_A11Y_REFRESH)`: the UI thread asks the view and stores the text in `TEXT`, the provider reads
//! it. If the UI thread is busy or hung the last text is used.
#![allow(non_upper_case_globals)] // the UIA constants of the windows crate keep their Win32 names

use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::{implement, Error, Result, BSTR, VARIANT};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_APP};

/// `WM_GETOBJECT`
pub const WM_GETOBJECT: u32 = 0x003D;
/// UI thread: store `View::accessible()` of the window in `TEXT` (sent by the provider, answered with 1)
pub const WM_A11Y_REFRESH: u32 = WM_APP + 0x2A1;

static TEXT: Mutex<Option<HashMap<isize, String>>> = Mutex::new(None);

/// UI thread: remember what the window's view says about itself.
pub fn store(hwnd: HWND, text: Option<String>) {
    if let Ok(mut g) = TEXT.lock() {
        let m = g.get_or_insert_with(HashMap::new);
        match text {
            Some(t) => m.insert(hwnd.0 as isize, t),
            None => m.remove(&(hwnd.0 as isize)),
        };
    }
}

/// UI thread: the window is gone.
pub fn forget(hwnd: HWND) {
    store(hwnd, None);
}

fn read(hwnd: isize) -> Option<String> {
    TEXT.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&hwnd).cloned()))
}

/// UI thread: answer `WM_GETOBJECT` for the UIA root object; None for every other request. `text` is the view's current text
/// (stored at once, so the first read is already right).
pub unsafe fn get_object(hwnd: HWND, wp: WPARAM, lp: LPARAM, text: Option<String>) -> Option<LRESULT> {
    if lp.0 as i32 != UiaRootObjectId {
        return None;
    }
    store(hwnd, text);
    let p: IRawElementProviderSimple = Pane { hwnd: hwnd.0 as isize }.into();
    Some(UiaReturnRawElementProvider(hwnd, wp, lp, &p))
}

fn no_element<T>() -> Result<T> {
    Err(Error::empty()) // S_OK + null: "there is none"
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IRawElementProviderFragmentRoot)]
struct Pane {
    hwnd: isize,
}

impl Pane_Impl {
    /// Ask the UI thread for fresh text (best effort), then read it.
    fn name(&self) -> String {
        let mut res = 0usize;
        unsafe {
            let _ = SendMessageTimeoutW(HWND(self.hwnd as *mut _), WM_A11Y_REFRESH, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 250, Some(&mut res));
        }
        read(self.hwnd).unwrap_or_else(|| "Pill".to_string())
    }
}

impl IRawElementProviderSimple_Impl for Pane_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }
    fn GetPatternProvider(&self, _: UIA_PATTERN_ID) -> Result<windows::core::IUnknown> {
        no_element()
    }
    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        Ok(match id {
            UIA_NamePropertyId => VARIANT::from(BSTR::from(self.name())),
            UIA_ControlTypePropertyId => VARIANT::from(UIA_PaneControlTypeId.0),
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId | UIA_IsEnabledPropertyId => VARIANT::from(true),
            UIA_IsKeyboardFocusablePropertyId | UIA_HasKeyboardFocusPropertyId => VARIANT::from(false),
            UIA_FrameworkIdPropertyId => VARIANT::from(BSTR::from("Pill")),
            _ => VARIANT::default(),
        })
    }
    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        unsafe { UiaHostProviderFromHwnd(HWND(self.hwnd as *mut _)) }
    }
}

impl IRawElementProviderFragment_Impl for Pane_Impl {
    fn Navigate(&self, _: NavigateDirection) -> Result<IRawElementProviderFragment> {
        no_element()
    }
    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut()) // the HWND provider supplies the id of the root
    }
    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(HWND(self.hwnd as *mut _), &mut r)? };
        Ok(UiaRect { left: r.left as f64, top: r.top as f64, width: (r.right - r.left) as f64, height: (r.bottom - r.top) as f64 })
    }
    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }
    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }
    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(Pane { hwnd: self.hwnd }.into())
    }
}

impl IRawElementProviderFragmentRoot_Impl for Pane_Impl {
    fn ElementProviderFromPoint(&self, _: f64, _: f64) -> Result<IRawElementProviderFragment> {
        no_element()
    }
    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        no_element()
    }
}

/// The text the island announces: `Pill` alone, or `Pill, <what is showing>`.
pub fn announce(what: &str) -> String {
    let w = what.trim();
    if w.is_empty() {
        "Pill".to_string()
    } else {
        format!("Pill, {w}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::Interface;

    #[test]
    fn announcement_is_the_name_with_what_is_showing() {
        assert_eq!(announce(""), "Pill");
        assert_eq!(announce("  "), "Pill");
        assert_eq!(announce("12:30"), "Pill, 12:30");
    }

    #[test]
    fn the_pane_reports_the_stored_text_and_a_pane_role() {
        let hwnd = HWND(0x7777 as *mut _); // no real window: the refresh message simply goes nowhere and the stored text is used
        store(hwnd, Some(announce("Agent working")));
        let p: IRawElementProviderSimple = Pane { hwnd: 0x7777 }.into();
        let name = unsafe { p.GetPropertyValue(UIA_NamePropertyId) }.unwrap();
        assert_eq!(BSTR::try_from(&name).unwrap().to_string(), "Pill, Agent working");
        let frag: IRawElementProviderFragment = p.cast().unwrap();
        assert!(unsafe { frag.Navigate(NavigateDirection_FirstChild) }.is_err(), "a single element, no children");
        forget(hwnd);
        let name = unsafe { p.GetPropertyValue(UIA_NamePropertyId) }.unwrap();
        assert_eq!(BSTR::try_from(&name).unwrap().to_string(), "Pill", "a forgotten window falls back to its plain name");
    }
}
