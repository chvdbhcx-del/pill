//! Screen-reader / UI Automation support for the host's windows. The windows are drawn with Direct2D, so UIA would see an empty
//! pane. Each window answers `WM_GETOBJECT` with one pane named by `View::accessible()` (the island: "Pill, 12:30").
//!
//! UIA calls the provider from its own threads while the view lives on the UI thread, so every read goes through a synchronous
//! `SendMessageTimeoutW(WM_A11Y_REFRESH)`: the UI thread asks the view and stores the text and the control list in `TEXT` / `CTRL`, the
//! provider reads them. If the UI thread is busy or hung the last snapshot is used.
//!
//! Controls: a view that lists `View::controls()` (buttons, tabs, sliders, list items; see `ui::a11y::nav`) gets them as the pane's children,
//! built from the same hit rectangles the view paints and clicks with. Each child answers name, role, state, bounds and the Invoke / Toggle /
//! SelectionItem / RangeValue patterns from the snapshot; an action goes back to the UI thread as `WM_A11Y_ACT` and lands in the view's own
//! `control_act` (the code a key press runs). Nothing is computed unless a UIA client is listening (`UiaClientsAreListening`).
#![allow(non_upper_case_globals)] // the UIA constants of the windows crate keep their Win32 names

use crate::ui::a11y::nav::{Act, Control, Role, Value as CValue};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::{implement, Error, Interface, Result, BSTR, VARIANT};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::VT_I4;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_APP};

/// `WM_GETOBJECT`
pub const WM_GETOBJECT: u32 = 0x003D;
/// UI thread: store `View::accessible()` and `View::controls()` of the window (sent by the provider, answered with 1)
pub const WM_A11Y_REFRESH: u32 = WM_APP + 0x2A1;
/// UI thread: run the action parked in `ACT` on the window's view (sent by a pattern provider, answered with 1 when the view took it)
pub const WM_A11Y_ACT: u32 = WM_APP + 0x2A2;

static TEXT: Mutex<Option<HashMap<isize, String>>> = Mutex::new(None);
static CTRL: Mutex<Option<HashMap<isize, Snap>>> = Mutex::new(None);
static ACT: Mutex<Option<(u32, Act)>> = Mutex::new(None);

/// One control as the provider reads it: the control and its bounds in screen px.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub c: Control,
    /// left, top, width, height (screen px)
    pub screen: (f64, f64, f64, f64),
}

#[derive(Clone, Debug, Default)]
struct Snap {
    items: Vec<Item>,
    /// the control that has the keyboard focus (keyboard mode), for `HasKeyboardFocus` / `GetFocus`
    focus: Option<u32>,
}

/// Client logical px -> screen px: `origin` is the client area's top-left on screen, `scale` the DPI factor.
pub fn screen_rect(origin: (i32, i32), scale: f32, r: glasskit::geom::Rect) -> (f64, f64, f64, f64) {
    let s = scale as f64;
    (origin.0 as f64 + r.x as f64 * s, origin.1 as f64 + r.y as f64 * s, r.w as f64 * s, r.h as f64 * s)
}

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

/// UI thread: remember the window's controls; returns the control that just got the keyboard focus (a change worth an event).
pub fn store_controls(hwnd: HWND, items: Vec<Item>, focus: Option<u32>) -> Option<u32> {
    let Ok(mut g) = CTRL.lock() else { return None };
    let m = g.get_or_insert_with(HashMap::new);
    let old = m.get(&(hwnd.0 as isize)).and_then(|s| s.focus);
    m.insert(hwnd.0 as isize, Snap { items, focus });
    if focus != old {
        focus
    } else {
        None
    }
}

/// UI thread: the window is gone.
pub fn forget(hwnd: HWND) {
    store(hwnd, None);
    if let Ok(mut g) = CTRL.lock() {
        if let Some(m) = g.as_mut() {
            m.remove(&(hwnd.0 as isize));
        }
    }
}

fn read(hwnd: isize) -> Option<String> {
    TEXT.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&hwnd).cloned()))
}

fn snap(hwnd: isize) -> Snap {
    CTRL.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&hwnd).cloned())).unwrap_or_default()
}

fn item(hwnd: isize, id: u32) -> Option<Item> {
    snap(hwnd).items.into_iter().find(|i| i.c.id == id)
}

/// Is any UI Automation client (a screen reader, Accessibility Insights...) connected? Keeps the host from building control lists for nobody.
pub fn listening() -> bool {
    unsafe { UiaClientsAreListening().as_bool() }
}

/// UI thread: tell the clients the keyboard focus moved to control `id` of `hwnd`.
pub fn raise_focus(hwnd: HWND, id: u32) {
    let p: IRawElementProviderSimple = Child { hwnd: hwnd.0 as isize, id }.into();
    unsafe {
        let _ = UiaRaiseAutomationEvent(&p, UIA_AutomationFocusChangedEventId);
    }
}

/// UI thread: take the action a pattern provider parked (see `WM_A11Y_ACT`).
pub fn take_act() -> Option<(u32, Act)> {
    ACT.lock().ok().and_then(|mut g| g.take())
}

/// Provider thread: ask the UI thread to run `act` on control `id`.
fn post_act(hwnd: isize, id: u32, act: Act) -> Result<()> {
    if let Ok(mut g) = ACT.lock() {
        *g = Some((id, act));
    }
    let mut res = 0usize;
    unsafe {
        let _ = SendMessageTimeoutW(HWND(hwnd as *mut _), WM_A11Y_ACT, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 1500, Some(&mut res));
    }
    if let Ok(mut g) = ACT.lock() {
        *g = None;
    }
    if res == 1 {
        Ok(())
    } else {
        Err(Error::from_hresult(windows::Win32::Foundation::E_FAIL))
    }
}

fn refresh(hwnd: isize) {
    let mut res = 0usize;
    unsafe {
        let _ = SendMessageTimeoutW(HWND(hwnd as *mut _), WM_A11Y_REFRESH, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 250, Some(&mut res));
    }
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

fn bool_(b: bool) -> BOOL {
    BOOL(b as i32)
}

// ---------------------------------------------------------------------------------------------------------------------------------
// The pane (the window)
// ---------------------------------------------------------------------------------------------------------------------------------

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IRawElementProviderFragmentRoot)]
struct Pane {
    hwnd: isize,
}

impl Pane_Impl {
    /// Ask the UI thread for fresh text (best effort), then read it.
    fn name(&self) -> String {
        refresh(self.hwnd);
        read(self.hwnd).unwrap_or_else(|| "Pill".to_string())
    }

    fn child(&self, id: u32) -> Result<IRawElementProviderFragment> {
        Child { hwnd: self.hwnd, id }.into_fragment()
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
    fn Navigate(&self, dir: NavigateDirection) -> Result<IRawElementProviderFragment> {
        match dir {
            NavigateDirection_FirstChild | NavigateDirection_LastChild => {
                refresh(self.hwnd);
                let items = snap(self.hwnd).items;
                match if dir == NavigateDirection_FirstChild { items.first() } else { items.last() } {
                    Some(i) => self.child(i.c.id),
                    None => no_element(),
                }
            }
            _ => no_element(),
        }
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
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        refresh(self.hwnd);
        // the smallest control under the point (a tab inside a strip beats the strip's pane)
        let hit = snap(self.hwnd)
            .items
            .into_iter()
            .filter(|i| x >= i.screen.0 && x < i.screen.0 + i.screen.2 && y >= i.screen.1 && y < i.screen.1 + i.screen.3)
            .min_by(|a, b| (a.screen.2 * a.screen.3).total_cmp(&(b.screen.2 * b.screen.3)));
        match hit {
            Some(i) => self.child(i.c.id),
            None => no_element(),
        }
    }
    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        match snap(self.hwnd).focus {
            Some(id) => self.child(id),
            None => no_element(),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// A control (a child of the pane)
// ---------------------------------------------------------------------------------------------------------------------------------

/// The UIA control type of a role.
pub fn control_type(r: Role) -> UIA_CONTROLTYPE_ID {
    match r {
        Role::Button => UIA_ButtonControlTypeId,
        Role::Tab => UIA_TabItemControlTypeId,
        Role::Slider => UIA_SliderControlTypeId,
        Role::ListItem => UIA_ListItemControlTypeId,
        Role::CheckBox => UIA_CheckBoxControlTypeId,
    }
}

/// The patterns a control supports: (invoke, toggle, selection item, range value).
pub fn patterns(c: &Control) -> (bool, bool, bool, bool) {
    let toggle = matches!(c.value, CValue::Toggled(_)) || c.role == Role::CheckBox;
    let select = matches!(c.role, Role::Tab | Role::ListItem);
    let range = c.role == Role::Slider && matches!(c.value, CValue::Range { .. });
    (c.role == Role::Button && !toggle, toggle, select, range)
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment)]
struct Child {
    hwnd: isize,
    id: u32,
}

impl Child {
    fn into_fragment(self) -> Result<IRawElementProviderFragment> {
        Ok(IRawElementProviderFragment::from(self))
    }
}

impl Child_Impl {
    fn me(&self) -> Option<Item> {
        item(self.hwnd, self.id)
    }

    fn sibling(&self, step: isize) -> Result<IRawElementProviderFragment> {
        refresh(self.hwnd);
        let items = snap(self.hwnd).items;
        let at = items.iter().position(|i| i.c.id == self.id).and_then(|i| i.checked_add_signed(step)).and_then(|i| items.get(i));
        match at {
            Some(i) => Child { hwnd: self.hwnd, id: i.c.id }.into_fragment(),
            None => no_element(),
        }
    }
}

impl IRawElementProviderSimple_Impl for Child_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }
    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<windows::core::IUnknown> {
        let Some(it) = self.me() else { return no_element() };
        let (invoke, toggle, select, range) = patterns(&it.c);
        let (h, id) = (self.hwnd, self.id);
        match pattern {
            UIA_InvokePatternId if invoke => IInvokeProvider::from(InvokeP { hwnd: h, id }).cast(),
            UIA_TogglePatternId if toggle => IToggleProvider::from(ToggleP { hwnd: h, id }).cast(),
            UIA_SelectionItemPatternId if select => ISelectionItemProvider::from(SelectP { hwnd: h, id }).cast(),
            UIA_RangeValuePatternId if range => IRangeValueProvider::from(RangeP { hwnd: h, id }).cast(),
            _ => no_element(),
        }
    }
    fn GetPropertyValue(&self, prop: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let Some(it) = self.me() else { return Ok(VARIANT::default()) };
        let focus = snap(self.hwnd).focus == Some(self.id);
        Ok(match prop {
            UIA_NamePropertyId => VARIANT::from(BSTR::from(it.c.name.as_str())),
            UIA_ControlTypePropertyId => VARIANT::from(control_type(it.c.role).0),
            UIA_AutomationIdPropertyId => VARIANT::from(BSTR::from(format!("pill.{}", it.c.id))),
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => VARIANT::from(true),
            UIA_IsEnabledPropertyId => VARIANT::from(it.c.enabled),
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(it.c.enabled),
            UIA_HasKeyboardFocusPropertyId => VARIANT::from(focus),
            UIA_FrameworkIdPropertyId => VARIANT::from(BSTR::from("Pill")),
            _ => VARIANT::default(),
        })
    }
    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        no_element()
    }
}

impl IRawElementProviderFragment_Impl for Child_Impl {
    fn Navigate(&self, dir: NavigateDirection) -> Result<IRawElementProviderFragment> {
        match dir {
            NavigateDirection_Parent => Ok(IRawElementProviderFragment::from(Pane { hwnd: self.hwnd })),
            NavigateDirection_NextSibling => self.sibling(1),
            NavigateDirection_PreviousSibling => self.sibling(-1),
            _ => no_element(),
        }
    }
    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        unsafe {
            let sa = SafeArrayCreateVector(VT_I4, 0, 2);
            for (i, v) in [UiaAppendRuntimeId as i32, self.id as i32].iter().enumerate() {
                SafeArrayPutElement(sa, &(i as i32), v as *const i32 as *const _)?;
            }
            Ok(sa)
        }
    }
    fn BoundingRectangle(&self) -> Result<UiaRect> {
        refresh(self.hwnd);
        match self.me() {
            Some(i) => Ok(UiaRect { left: i.screen.0, top: i.screen.1, width: i.screen.2, height: i.screen.3 }),
            None => Ok(UiaRect::default()),
        }
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

// ---------------------------------------------------------------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------------------------------------------------------------

#[implement(IInvokeProvider)]
struct InvokeP {
    hwnd: isize,
    id: u32,
}
impl IInvokeProvider_Impl for InvokeP_Impl {
    fn Invoke(&self) -> Result<()> {
        post_act(self.hwnd, self.id, Act::Invoke)
    }
}

#[implement(IToggleProvider)]
struct ToggleP {
    hwnd: isize,
    id: u32,
}
impl IToggleProvider_Impl for ToggleP_Impl {
    fn Toggle(&self) -> Result<()> {
        post_act(self.hwnd, self.id, Act::Toggle)
    }
    fn ToggleState(&self) -> Result<ToggleState> {
        Ok(match item(self.hwnd, self.id).map(|i| i.c.value) {
            Some(CValue::Toggled(true)) => ToggleState_On,
            _ => ToggleState_Off,
        })
    }
}

#[implement(ISelectionItemProvider)]
struct SelectP {
    hwnd: isize,
    id: u32,
}
impl ISelectionItemProvider_Impl for SelectP_Impl {
    fn Select(&self) -> Result<()> {
        post_act(self.hwnd, self.id, Act::Select)
    }
    fn AddToSelection(&self) -> Result<()> {
        post_act(self.hwnd, self.id, Act::Select) // single selection: adding is selecting
    }
    fn RemoveFromSelection(&self) -> Result<()> {
        Err(Error::from_hresult(windows::Win32::Foundation::E_NOTIMPL))
    }
    fn IsSelected(&self) -> Result<BOOL> {
        Ok(bool_(item(self.hwnd, self.id).is_some_and(|i| i.c.value == CValue::Selected(true))))
    }
    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        Ok(Pane { hwnd: self.hwnd }.into())
    }
}

#[implement(IRangeValueProvider)]
struct RangeP {
    hwnd: isize,
    id: u32,
}
impl RangeP_Impl {
    fn range(&self) -> (f64, f64, f64, f64) {
        match item(self.hwnd, self.id).map(|i| i.c.value) {
            Some(CValue::Range { min, max, cur, step }) => (min, max, cur, step),
            _ => (0.0, 0.0, 0.0, 0.0),
        }
    }
}
impl IRangeValueProvider_Impl for RangeP_Impl {
    fn SetValue(&self, val: f64) -> Result<()> {
        let (min, max, ..) = self.range();
        post_act(self.hwnd, self.id, Act::Set(val.clamp(min, max)))
    }
    fn Value(&self) -> Result<f64> {
        Ok(self.range().2)
    }
    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(bool_(false))
    }
    fn Maximum(&self) -> Result<f64> {
        Ok(self.range().1)
    }
    fn Minimum(&self) -> Result<f64> {
        Ok(self.range().0)
    }
    fn LargeChange(&self) -> Result<f64> {
        let (min, max, _, step) = self.range();
        Ok((step * 5.0).min((max - min).max(step)))
    }
    fn SmallChange(&self) -> Result<f64> {
        Ok(self.range().3)
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
    use glasskit::geom::Rect;

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
        assert!(unsafe { frag.Navigate(NavigateDirection_FirstChild) }.is_err(), "no controls listed: a single element, no children");
        forget(hwnd);
        let name = unsafe { p.GetPropertyValue(UIA_NamePropertyId) }.unwrap();
        assert_eq!(BSTR::try_from(&name).unwrap().to_string(), "Pill", "a forgotten window falls back to its plain name");
    }

    fn items() -> Vec<Item> {
        let at = |id: u32, name: &str, role: Role, v: CValue, x: f32| {
            let c = Control::new(id, name, role, Rect::new(x, 10.0, 40.0, 20.0)).with(v);
            Item { screen: screen_rect((100, 200), 1.5, c.rect), c }
        };
        vec![
            at(1, "Home", Role::Tab, CValue::Selected(true), 0.0),
            at(2, "Pin", Role::Button, CValue::Toggled(true), 50.0),
            at(3, "Volume", Role::Slider, CValue::Range { min: 0.0, max: 100.0, cur: 40.0, step: 5.0 }, 100.0),
            at(4, "Settings", Role::Button, CValue::None, 150.0),
        ]
    }

    fn prop(p: &IRawElementProviderSimple, id: UIA_PROPERTY_ID) -> VARIANT {
        unsafe { p.GetPropertyValue(id) }.unwrap()
    }

    #[test]
    fn the_controls_become_children_with_roles_values_and_bounds() {
        let hwnd = HWND(0x7778 as *mut _);
        assert_eq!(store_controls(hwnd, items(), Some(1)), Some(1), "the first focus is news");
        assert_eq!(store_controls(hwnd, items(), Some(1)), None, "the same focus is not");
        let pane: IRawElementProviderFragment = IRawElementProviderSimple::from(Pane { hwnd: 0x7778 }).cast().unwrap();
        let first = unsafe { pane.Navigate(NavigateDirection_FirstChild) }.unwrap();
        let first_s: IRawElementProviderSimple = first.cast().unwrap();
        assert_eq!(BSTR::try_from(&prop(&first_s, UIA_NamePropertyId)).unwrap().to_string(), "Home");
        assert_eq!(i32::try_from(&prop(&first_s, UIA_ControlTypePropertyId)).unwrap(), UIA_TabItemControlTypeId.0);
        assert!(bool::try_from(&prop(&first_s, UIA_HasKeyboardFocusPropertyId)).unwrap());
        let r = unsafe { first.BoundingRectangle() }.unwrap();
        assert_eq!((r.left, r.top, r.width, r.height), (100.0, 215.0, 60.0, 30.0), "screen px: origin + logical * 1.5");
        // siblings walk the list in order, both ways; the parent is the pane
        let second = unsafe { first.Navigate(NavigateDirection_NextSibling) }.unwrap();
        let back = unsafe { second.Navigate(NavigateDirection_PreviousSibling) }.unwrap();
        let back_s: IRawElementProviderSimple = back.cast().unwrap();
        assert_eq!(BSTR::try_from(&prop(&back_s, UIA_NamePropertyId)).unwrap().to_string(), "Home");
        assert!(unsafe { first.Navigate(NavigateDirection_PreviousSibling) }.is_err());
        assert!(unsafe { first.Navigate(NavigateDirection_Parent) }.is_ok());
        let last = unsafe { pane.Navigate(NavigateDirection_LastChild) }.unwrap();
        assert!(unsafe { last.Navigate(NavigateDirection_NextSibling) }.is_err());
        // a point finds the control under it
        let root: IRawElementProviderFragmentRoot = IRawElementProviderSimple::from(Pane { hwnd: 0x7778 }).cast().unwrap();
        let hit: IRawElementProviderSimple = unsafe { root.ElementProviderFromPoint(100.0 + 80.0 * 1.5 + 3.0, 215.0 + 3.0) }.unwrap().cast().unwrap();
        assert_eq!(BSTR::try_from(&prop(&hit, UIA_NamePropertyId)).unwrap().to_string(), "Pin");
        assert!(unsafe { root.ElementProviderFromPoint(5.0, 5.0) }.is_err());
        let f: IRawElementProviderSimple = unsafe { root.GetFocus() }.unwrap().cast().unwrap();
        assert_eq!(BSTR::try_from(&prop(&f, UIA_NamePropertyId)).unwrap().to_string(), "Home");
        forget(hwnd);
    }

    #[test]
    fn each_role_offers_its_own_patterns() {
        let it = items();
        let p = |i: usize| patterns(&it[i].c);
        assert_eq!(p(0), (false, false, true, false), "a tab selects");
        assert_eq!(p(1), (false, true, false, false), "a switch toggles");
        assert_eq!(p(2), (false, false, false, true), "a slider has a range");
        assert_eq!(p(3), (true, false, false, false), "a button invokes");
        assert_eq!(control_type(Role::ListItem), UIA_ListItemControlTypeId);
        let hwnd = HWND(0x7779 as *mut _);
        store_controls(hwnd, items(), None);
        let get = |id: u32, pat: UIA_PATTERN_ID| unsafe { IRawElementProviderSimple::from(Child { hwnd: 0x7779, id }).GetPatternProvider(pat) };
        assert!(get(4, UIA_InvokePatternId).is_ok() && get(4, UIA_TogglePatternId).is_err());
        assert!(get(2, UIA_TogglePatternId).is_ok() && get(2, UIA_InvokePatternId).is_err());
        assert!(get(1, UIA_SelectionItemPatternId).is_ok());
        let range: IRangeValueProvider = get(3, UIA_RangeValuePatternId).unwrap().cast().unwrap();
        unsafe {
            assert_eq!((range.Minimum().unwrap(), range.Maximum().unwrap(), range.Value().unwrap(), range.SmallChange().unwrap()), (0.0, 100.0, 40.0, 5.0));
        }
        let toggle: IToggleProvider = get(2, UIA_TogglePatternId).unwrap().cast().unwrap();
        assert_eq!(unsafe { toggle.ToggleState() }.unwrap(), ToggleState_On);
        let sel: ISelectionItemProvider = get(1, UIA_SelectionItemPatternId).unwrap().cast().unwrap();
        assert!(unsafe { sel.IsSelected() }.unwrap().as_bool());
        // an action needs a window to run on: without one it reports failure instead of pretending
        let inv: IInvokeProvider = get(4, UIA_InvokePatternId).unwrap().cast().unwrap();
        assert!(unsafe { inv.Invoke() }.is_err());
        forget(hwnd);
    }

    #[test]
    fn screen_rect_scales_and_offsets() {
        assert_eq!(screen_rect((-10, 20), 2.0, Rect::new(1.0, 2.0, 3.0, 4.0)), (-8.0, 24.0, 6.0, 8.0));
    }
}
