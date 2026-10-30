//! The UI thread: window classes, the message loop, window creation per kind, input translation, the frame loop, the cursor poll of the
//! click-through island, focus handling, device-lost recovery. Everything here runs on the host thread; the rest of the app only sees
//! `HostHandle`. The pure decisions (frame scheduling, proximity, focus policies, geometry) live in their own tested modules.
//!
//! # Re-entrancy rule
//! Win32 sends messages synchronously from inside calls we make (`SetWindowPos` -> `WM_SIZE`, `SetForegroundWindow` -> `WM_ACTIVATE`,
//! `CreateWindowEx` -> `WM_NCCALCSIZE`...). The `Host` state is a `RefCell`; a message that arrives while it is borrowed is
//! (a) answered from the lock-free `WinShared` table when it needs an answer (`WM_NCHITTEST`, `WM_SETCURSOR`, `WM_NCCALCSIZE`,
//! `WM_GETMINMAXINFO`, `WM_MOUSEACTIVATE`), or (b) queued in `DEFERRED` and replayed as soon as the borrow ends. Nothing is ever lost and
//! nothing blocks.
use super::dropt;
use super::focus::{BlurGuard, FocusWatch};
use super::geom::{self, Edge, HitRegion, IRect};
use super::present::{PresentStatus, Presenter};
use super::proximity::{Probe, Proximity};
use super::sched::FrameState;
use super::{Cmd, Factory, Geometry, HideReason, HostHandle, MonitorInfo, MonitorSel, Placement, Shared, WinEvent, WinInfo, WindowKind, WindowSpec};
use crate::ui::view::{CursorShape, Cx, Effects, Msg, MsgQueue, OpenRequest, Waker, WindowCmd, WindowId};
use glasskit::canvas::Canvas;
use glasskit::fonts::FontSet;
use glasskit::geom::Rect;
use glasskit::gfx::Gfx;
use glasskit::input::win32::Translator;
use glasskit::input::{InputEvent, Response};
use glasskit::lang::Lang;
use glasskit::text::Res;
use glasskit::theme::Theme;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize, RevokeDragDrop};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::Ime::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_APP_WAKE: u32 = WM_APP + 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const T_MODAL: usize = 1;
const CLASS: PCWSTR = w!("PillNativeWindow");
const MSG_CLASS: PCWSTR = w!("PillNativeHostMsg");

// ---------------------------------------------------------------------------------------------------------------------------------------
// Monitors
// ---------------------------------------------------------------------------------------------------------------------------------------

fn irect(r: RECT) -> IRect {
    IRect::new(r.left, r.top, r.right, r.bottom)
}

fn hw(h: isize) -> HWND {
    HWND(h as *mut c_void)
}

fn hv(h: HWND) -> isize {
    h.0 as isize
}

fn hz_of_hmon(hm: HMONITOR) -> u32 {
    unsafe {
        let mut mi = MONITORINFOEXW::default();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(hm, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return 60;
        }
        let mut dm = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        if EnumDisplaySettingsW(PCWSTR(mi.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm).as_bool() && dm.dmDisplayFrequency > 1 {
            return dm.dmDisplayFrequency.clamp(24, 1000);
        }
    }
    60
}

fn info_of_hmon(hm: HMONITOR) -> Option<MonitorInfo> {
    unsafe {
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(hm, &mut mi).as_bool() {
            return None;
        }
        let (mut x, mut y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hm, MDT_EFFECTIVE_DPI, &mut x, &mut y);
        Some(MonitorInfo { rect: irect(mi.rcMonitor), work: irect(mi.rcWork), primary: mi.dwFlags & 1 != 0, scale: geom::scale_from_dpi(x), refresh_hz: hz_of_hmon(hm) })
    }
}

/// Every monitor: the primary first, then left to right (this order is what `MonitorSel::Index` means).
pub(crate) fn monitors() -> Vec<MonitorInfo> {
    // a plain EnumDisplayMonitors (microseconds): glasskit's `monitors::enumerate` also reads EDID / registry names, which a popup that
    // re-places itself at every show cannot afford
    unsafe extern "system" fn collect(hm: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        (*(lp.0 as *mut Vec<HMONITOR>)).push(hm);
        BOOL(1)
    }
    let mut hms: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut hms as *mut Vec<HMONITOR> as isize));
    }
    let mut v: Vec<MonitorInfo> = hms.into_iter().filter_map(info_of_hmon).collect();
    if v.is_empty() {
        unsafe {
            let hm = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            v.extend(info_of_hmon(hm));
        }
    }
    v.sort_by_key(|m| (!m.primary, m.rect.l, m.rect.t));
    v
}

pub(crate) fn monitor_of_window(hwnd: isize) -> Option<MonitorInfo> {
    if hwnd == 0 {
        return None;
    }
    unsafe { info_of_hmon(MonitorFromWindow(hw(hwnd), MONITOR_DEFAULTTONEAREST)) }
}

fn cursor_pos() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

fn fallback_monitor() -> MonitorInfo {
    let r = IRect::new(0, 0, 1920, 1080);
    MonitorInfo { rect: r, work: r, primary: true, scale: 1.0, refresh_hz: 60 }
}

pub(crate) fn pick_monitor(sel: &MonitorSel, mons: &[MonitorInfo], cursor: (i32, i32)) -> MonitorInfo {
    let primary = || mons.iter().find(|m| m.primary).or(mons.first()).cloned().unwrap_or_else(fallback_monitor);
    match sel {
        MonitorSel::Primary => primary(),
        MonitorSel::Index(i) => mons.get(*i).cloned().unwrap_or_else(primary),
        MonitorSel::UnderCursor => geom::monitor_at(&mons.iter().map(|m| m.rect).collect::<Vec<_>>(), cursor.0, cursor.1).map(|i| mons[i].clone()).unwrap_or_else(primary),
        MonitorSel::Rect(r) => mons.iter().find(|m| m.rect == *r).cloned().unwrap_or_else(primary),
    }
}

pub(crate) struct Resolved {
    pub rect: IRect,
    pub mon: MonitorInfo,
}

/// Where a window of logical size `size_l` goes for `placement`. Pure (monitors and cursor are passed in).
pub(crate) fn resolve_placement(p: &Placement, size_l: (f32, f32), mons: &[MonitorInfo], cursor: (i32, i32), below: Option<i32>) -> Resolved {
    match p {
        Placement::TopCenter { monitor, offset_pct, below_taskbar, margin_top } => {
            let mon = pick_monitor(monitor, mons, cursor);
            let sz = geom::size_to_phys(size_l, mon.scale);
            let mut top = if *below_taskbar { mon.work.t.max(mon.rect.t) } else { mon.rect.t } + geom::phys(*margin_top, mon.scale);
            if let Some(b) = below {
                top = top.max(b);
            }
            let (x, y) = geom::anchor_top_center(mon.rect, top, sz, *offset_pct);
            Resolved { rect: IRect::from_xywh(x, y, sz.0, sz.1), mon }
        }
        Placement::Center(monitor) => {
            let mon = pick_monitor(monitor, mons, cursor);
            let sz = geom::size_to_phys(size_l, mon.scale);
            let (x, y) = geom::center_on(mon.rect, sz.0, sz.1);
            Resolved { rect: IRect::from_xywh(x, y, sz.0, sz.1), mon }
        }
        Placement::At { x, y } => {
            let mon = pick_monitor(&MonitorSel::UnderCursor, mons, (*x, *y));
            let sz = geom::size_to_phys(size_l, mon.scale);
            Resolved { rect: IRect::from_xywh(*x, *y, sz.0, sz.1), mon }
        }
        Placement::Restore(g) => {
            let mon = pick_monitor(&MonitorSel::UnderCursor, mons, (g.x + 60, g.y + 10));
            let sz = geom::size_to_phys((g.w, g.h), mon.scale);
            let all: Vec<IRect> = mons.iter().map(|m| m.rect).collect();
            if geom::pos_visible(g.x, g.y, sz.0, &all) {
                Resolved { rect: IRect::from_xywh(g.x, g.y, sz.0, sz.1), mon }
            } else {
                let mon = pick_monitor(&MonitorSel::Primary, mons, cursor);
                let sz = geom::size_to_phys((g.w, g.h), mon.scale);
                let (x, y) = geom::center_on(mon.rect, sz.0, sz.1);
                Resolved { rect: IRect::from_xywh(x, y, sz.0, sz.1), mon }
            }
        }
        Placement::Cover(monitor) => {
            let mon = pick_monitor(monitor, mons, cursor);
            Resolved { rect: mon.rect, mon }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// GPU resources (shared by every window of the host)
// ---------------------------------------------------------------------------------------------------------------------------------------

static FONTS: OnceLock<&'static FontSet> = OnceLock::new();
static SYSTEM_FONTS: FontSet = FontSet::system();

pub(crate) fn set_fonts(f: &'static FontSet) {
    let _ = FONTS.set(f);
}

struct Gpu {
    gfx: Gfx,
    res: Res,
}

impl Gpu {
    fn new() -> windows::core::Result<Gpu> {
        let gfx = if std::env::var_os("PILL_FORCE_WARP").is_some() { Gfx::warp()? } else { Gfx::new(None)? };
        let res = Res::new(&gfx, FONTS.get().copied().unwrap_or(&SYSTEM_FONTS))?;
        Ok(Gpu { gfx, res })
    }
}

fn force_layered() -> bool {
    std::env::var_os("PILL_FORCE_LAYERED").is_some_and(|v| v != "0")
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Lock-free per-window data for messages that need an answer while the Host is borrowed
// ---------------------------------------------------------------------------------------------------------------------------------------

struct WinShared {
    kind: WindowKind,
    resizable: bool,
    scale: Cell<f32>,
    hit: RefCell<HitRegion>,
    /// a file drag is engaged: the whole window answers hit tests
    hit_all: Cell<bool>,
    caption: RefCell<Vec<Rect>>,
    cursor: Cell<CursorShape>,
    activatable: Cell<bool>,
    min_phys: Cell<(i32, i32)>,
    maximized: Cell<bool>,
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    static DEFERRED: RefCell<Vec<(isize, u32, usize, isize)>> = const { RefCell::new(Vec::new()) };
    static SHARED: RefCell<HashMap<isize, Rc<WinShared>>> = RefCell::new(HashMap::new());
}

fn shared_of(hwnd: HWND) -> Option<Rc<WinShared>> {
    SHARED.with(|s| s.borrow().get(&hv(hwnd)).cloned())
}

/// Runs `f` on the Host if it is not borrowed (None when it is). The drop target uses this too.
pub(super) fn with_host<R>(f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|h| match h.try_borrow_mut() {
        Ok(mut g) => g.as_mut().map(f),
        Err(_) => None,
    })
}

fn load_cursor(c: CursorShape) -> HCURSOR {
    let id = match c {
        CursorShape::Arrow => IDC_ARROW,
        CursorShape::Text => IDC_IBEAM,
        CursorShape::Hand => IDC_HAND,
        CursorShape::ResizeEw => IDC_SIZEWE,
        CursorShape::ResizeNs => IDC_SIZENS,
        CursorShape::Crosshair => IDC_CROSS,
        CursorShape::Move => IDC_SIZEALL,
        CursorShape::No => IDC_NO,
        CursorShape::Wait => IDC_WAIT,
    };
    unsafe { LoadCursorW(None, id).unwrap_or_default() }
}

fn ht_of(e: Edge) -> Option<isize> {
    Some(match e {
        Edge::None => return None,
        Edge::Left => HTLEFT,
        Edge::Right => HTRIGHT,
        Edge::Top => HTTOP,
        Edge::Bottom => HTBOTTOM,
        Edge::TopLeft => HTTOPLEFT,
        Edge::TopRight => HTTOPRIGHT,
        Edge::BottomLeft => HTBOTTOMLEFT,
        Edge::BottomRight => HTBOTTOMRIGHT,
    } as isize)
}

/// Messages answered without the Host (see the re-entrancy rule).
unsafe fn quick(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_ERASEBKGND => return Some(LRESULT(1)),
        WM_PAINT => {
            // a plain ValidateRect does not stop WM_PAINT on layered / composition windows; a Begin/EndPaint pair does
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            return None; // the stateful path asks for a frame
        }
        _ => {}
    }
    let sh = shared_of(hwnd)?;
    match msg {
        WM_NCHITTEST => {
            let (sx, sy) = (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32);
            let mut r = RECT::default();
            let _ = GetWindowRect(hwnd, &mut r);
            let (cx, cy) = (sx - r.left, sy - r.top);
            let s = sh.scale.get();
            match sh.kind {
                WindowKind::Overlay => {
                    let inside = sh.hit_all.get() || sh.hit.borrow().contains(cx as f32 / s, cy as f32 / s);
                    Some(LRESULT(if inside { HTCLIENT as isize } else { -1 })) // -1 = HTTRANSPARENT
                }
                WindowKind::Normal => {
                    if !sh.maximized.get() && sh.resizable {
                        let b = (6.0 * s).round() as i32;
                        if let Some(h) = ht_of(geom::resize_edge(cx, cy, r.right - r.left, r.bottom - r.top, b)) {
                            return Some(LRESULT(h));
                        }
                    }
                    let (lx, ly) = (cx as f32 / s, cy as f32 / s);
                    if sh.caption.borrow().iter().any(|c| c.contains(lx, ly)) {
                        return Some(LRESULT(HTCAPTION as isize));
                    }
                    Some(LRESULT(HTCLIENT as isize))
                }
                _ => Some(LRESULT(HTCLIENT as isize)),
            }
        }
        WM_SETCURSOR if (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
            SetCursor(load_cursor(sh.cursor.get()));
            Some(LRESULT(1))
        }
        WM_MOUSEACTIVATE if sh.kind == WindowKind::Overlay && !sh.activatable.get() => Some(LRESULT(MA_NOACTIVATE as isize)),
        WM_NCCALCSIZE if sh.kind == WindowKind::Normal => {
            if wp.0 != 0 && IsZoomed(hwnd).as_bool() {
                // a maximized WS_THICKFRAME window overhangs the monitor by its frame: take it back
                let dpi = GetDpiForWindow(hwnd);
                let f = GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
                let p = lp.0 as *mut NCCALCSIZE_PARAMS;
                (*p).rgrc[0].left += f;
                (*p).rgrc[0].right -= f;
                (*p).rgrc[0].top += f;
                (*p).rgrc[0].bottom -= f;
            }
            Some(LRESULT(0))
        }
        WM_NCACTIVATE if sh.kind == WindowKind::Normal => Some(DefWindowProcW(hwnd, msg, wp, LPARAM(-1))), // no non-client repaint
        WM_NCPAINT if sh.kind == WindowKind::Normal => Some(LRESULT(0)),
        WM_GETMINMAXINFO => {
            let (mw, mh) = sh.min_phys.get();
            if mw > 0 {
                let mi = lp.0 as *mut MINMAXINFO;
                (*mi).ptMinTrackSize = POINT { x: mw, y: mh };
            }
            Some(LRESULT(0))
        }
        _ => None,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(r) = quick(hwnd, msg, wp, lp) {
        return r;
    }
    enum Out {
        Handled(LRESULT),
        Default,
        Busy,
    }
    let out = HOST.with(|h| match h.try_borrow_mut() {
        Ok(mut g) => match g.as_mut() {
            Some(host) => match catch_unwind(AssertUnwindSafe(|| host.on_message(hwnd, msg, wp, lp))) {
                Ok(Some(r)) => Out::Handled(r),
                Ok(None) => Out::Default,
                Err(_) => {
                    eprintln!("ui host: a panic in message {msg:#x} was caught; continuing");
                    Out::Default
                }
            },
            None => Out::Default,
        },
        Err(_) => Out::Busy,
    });
    match out {
        Out::Handled(r) => {
            drain_deferred();
            r
        }
        Out::Default => {
            drain_deferred();
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        Out::Busy => {
            DEFERRED.with(|d| d.borrow_mut().push((hv(hwnd), msg, wp.0, lp.0)));
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

/// Replays messages that arrived while the Host was borrowed. Results are discarded (the default procedure already ran for them).
fn drain_deferred() {
    for _ in 0..64 {
        let next = DEFERRED.with(|d| {
            let mut d = d.borrow_mut();
            if d.is_empty() {
                None
            } else {
                Some(d.remove(0))
            }
        });
        let Some((h, m, wp, lp)) = next else { return };
        let _ = with_host(|host| catch_unwind(AssertUnwindSafe(|| host.on_message(hw(h), m, WPARAM(wp), LPARAM(lp)))));
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Window state
// ---------------------------------------------------------------------------------------------------------------------------------------

struct Win {
    id: WindowId,
    hwnd: HWND,
    spec: WindowSpec,
    view: Box<dyn crate::ui::view::View>,
    cx: Cx,
    tr: Translator,
    frame: FrameState,
    pres: Option<Presenter>,
    sh: Rc<WinShared>,
    scale: f32,
    size_px: (i32, i32),
    size_l: (f32, f32),
    visible: bool,
    theme: Theme,
    hit: HitRegion,
    prox: Option<Proximity>,
    poll_at: f64,
    force_ignore: Option<bool>,
    ignore_applied: bool,
    prev_fg: isize,
    blur: BlurGuard,
    watch: Option<FocusWatch>,
    watch_at: f64,
    geo_at: Option<f64>,
    in_size_move: bool,
    captured: bool,
    retry_at: f64,
    /// layered fallback: earliest time of the next frame
    pace_at: f64,
    hz: u32,
    drop_registered: bool,
    layered: bool,
    shown_once: bool,
}

pub(crate) struct Host {
    shared: Arc<Shared>,
    t0: Instant,
    gpu: Option<Gpu>,
    wins: HashMap<WindowId, Win>,
    by_hwnd: HashMap<isize, WindowId>,
    msg_hwnd: HWND,
    timer: HANDLE,
    quit: bool,
    /// frame-latency handles whose signal the loop's sleep already consumed (valid for the next `render_due` only)
    ready: std::collections::HashSet<isize>,
}

fn clear_color(c: [f32; 4]) -> D2D1_COLOR_F {
    // the canvas works in premultiplied alpha
    D2D1_COLOR_F { r: c[0] * c[3], g: c[1] * c[3], b: c[2] * c[3], a: c[3] }
}

impl Host {
    fn now_ms(&self) -> f64 {
        self.t0.elapsed().as_secs_f64() * 1000.0
    }

    fn emit(&self, id: WindowId, hook: &Option<super::EventHook>, ev: WinEvent) {
        if let Some(h) = hook {
            h(id, ev);
        }
    }

    fn ensure_gpu(&mut self) -> bool {
        if self.gpu.is_none() {
            match Gpu::new() {
                Ok(g) => self.gpu = Some(g),
                Err(e) => {
                    eprintln!("ui host: no graphics device: {e}");
                    return false;
                }
            }
        }
        true
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // creation / destruction
    // ------------------------------------------------------------------------------------------------------------------------------

    fn create(&mut self, id: WindowId, spec: WindowSpec, factory: Factory, args: Value) {
        if let Err(e) = self.try_create(id, spec.clone(), factory, args) {
            eprintln!("ui host: cannot create window `{}`: {e}", spec.label);
            self.emit(id, &spec.on_event, WinEvent::Closed);
        }
    }

    fn try_create(&mut self, id: WindowId, spec: WindowSpec, factory: Factory, args: Value) -> Result<(), String> {
        if !self.ensure_gpu() {
            return Err("no graphics device".into());
        }
        let mons = monitors();
        let cursor = cursor_pos();
        let first = resolve_placement(&spec.placement, spec.size, &mons, cursor, None);
        let waker = Waker::new(id, self.shared.queue.clone());
        let mut cx = Cx::new(id, &spec.label, super::real_services(), waker);
        cx.scale = first.mon.scale;
        cx.refresh_hz = first.mon.refresh_hz as f32;
        cx.lang = if self.shared.lang_ru.load(Ordering::Relaxed) { Lang::Ru } else { Lang::En };
        cx.reduced_motion = glasskit::ease::reduced();
        cx.now_ms = self.now_ms();
        let view = factory(&mut cx, args);
        let hint = if spec.kind == WindowKind::Fullscreen { None } else { view.size_hint() };
        let size_l = hint.unwrap_or(spec.size);
        let placed = resolve_placement(&spec.placement, size_l, &mons, cursor, None);
        let (cw, ch) = (placed.rect.w().max(1), placed.rect.h().max(1));
        let want_layered = force_layered();

        let mut layered = want_layered;
        let (hwnd, pres) = loop {
            let (ex, style) = styles(&spec, layered);
            let hwnd = unsafe {
                let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
                let title: Vec<u16> = spec.title.encode_utf16().chain(Some(0)).collect();
                CreateWindowExW(ex, CLASS, PCWSTR(title.as_ptr()), style, placed.rect.l, placed.rect.t, cw, ch, None, None, hinst, None).map_err(|e| e.to_string())?
            };
            let gpu = self.gpu.as_ref().ok_or("no gpu")?;
            let p = if layered { Presenter::new_layered(&gpu.gfx, hwnd, cw as u32, ch as u32) } else { Presenter::new_comp(&gpu.gfx, hwnd, cw as u32, ch as u32) };
            match p {
                Ok(p) => break (hwnd, p),
                Err(e) if !layered => {
                    eprintln!("ui host: composition unavailable for `{}` ({e}); using the layered fallback", spec.label);
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    layered = true;
                }
                Err(e) => {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    return Err(e.to_string());
                }
            }
        };
        let scale = unsafe { geom::scale_from_dpi(GetDpiForWindow(hwnd)) };
        let dark_min = spec.min_size.map_or((0, 0), |m| geom::size_to_phys(m, scale));
        let sh = Rc::new(WinShared {
            kind: spec.kind,
            resizable: spec.resizable,
            scale: Cell::new(scale),
            hit: RefCell::new(HitRegion::All),
            hit_all: Cell::new(false),
            caption: RefCell::new(Vec::new()),
            cursor: Cell::new(if spec.kind == WindowKind::Fullscreen { CursorShape::Crosshair } else { CursorShape::Arrow }),
            activatable: Cell::new(spec.kind != WindowKind::Overlay),
            min_phys: Cell::new(dark_min),
            maximized: Cell::new(false),
        });
        SHARED.with(|s| s.borrow_mut().insert(hv(hwnd), sh.clone()));
        unsafe {
            if spec.kind == WindowKind::Overlay || layered {
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA); // a layered window is invisible without attributes
            }
            if spec.kind == WindowKind::Normal {
                let m = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 1 };
                let _ = DwmExtendFrameIntoClientArea(hwnd, &m); // the DWM shadow of a frameless window
                let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            }
            if spec.exclude_from_capture {
                let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
            }
        }
        let drop_registered = spec.accept_drops && dropt::register(hwnd, id);
        let hz = placed.mon.refresh_hz;
        let mut frame = FrameState::new(hz as f32);
        frame.set_cap(spec.fps_cap);
        let theme = spec.theme;
        let prox = (spec.kind == WindowKind::Overlay).then(|| {
            let mut p = Proximity::new(scale, hz);
            p.set_hit_region(HitRegion::All);
            p
        });
        let watch = spec.focus_watchdog_ms.map(FocusWatch::new);
        cx.scale = scale;
        cx.refresh_hz = hz as f32;
        let win = Win {
            id,
            hwnd,
            tr: Translator::new(scale),
            frame,
            pres: Some(pres),
            sh,
            scale,
            size_px: (cw, ch),
            size_l,
            visible: false,
            theme,
            hit: HitRegion::All,
            prox,
            poll_at: 0.0,
            force_ignore: None,
            ignore_applied: false,
            prev_fg: 0,
            blur: BlurGuard::default(),
            watch,
            watch_at: f64::INFINITY,
            geo_at: None,
            in_size_move: false,
            captured: false,
            retry_at: 0.0,
            pace_at: 0.0,
            hz,
            drop_registered,
            layered,
            shown_once: false,
            view,
            cx,
            spec: spec.clone(),
        };
        self.by_hwnd.insert(hv(hwnd), id);
        self.wins.insert(id, win);
        {
            let mut w = self.shared.windows.lock().unwrap_or_else(|e| e.into_inner());
            w.insert(id, WinInfo { hwnd: hv(hwnd), visible: false, comp: !layered });
        }
        self.shared.open.fetch_add(1, Ordering::Relaxed);
        self.emit(id, &spec.on_event, WinEvent::Created);
        self.apply_fx(id);
        if spec.visible {
            self.show(id, None);
        }
        Ok(())
    }

    fn close(&mut self, id: WindowId) {
        let Some(mut win) = self.wins.remove(&id) else { return };
        self.by_hwnd.remove(&hv(win.hwnd));
        SHARED.with(|s| s.borrow_mut().remove(&hv(win.hwnd)));
        win.pres = None; // release the composition target before the window goes
        unsafe {
            if win.drop_registered {
                let _ = RevokeDragDrop(win.hwnd);
            }
            if win.captured {
                let _ = ReleaseCapture();
            }
            if win.prev_fg != 0 && win.spec.kind != WindowKind::Overlay {
                // nothing to restore: a closing window gives the foreground back through the system
            }
            let _ = DestroyWindow(win.hwnd);
        }
        self.shared.windows.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        self.shared.open.fetch_sub(1, Ordering::Relaxed);
        let hook = win.spec.on_event.clone();
        drop(win); // drops the Cx: unsubscribes from the bus
        self.emit(id, &hook, WinEvent::Closed);
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // show / hide / placement
    // ------------------------------------------------------------------------------------------------------------------------------

    fn place(&mut self, id: WindowId, below: Option<i32>, flags: SET_WINDOW_POS_FLAGS) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        let size_l = if win.spec.kind == WindowKind::Fullscreen { win.spec.size } else { win.view.size_hint().unwrap_or(win.spec.size) };
        win.size_l = size_l;
        let r = resolve_placement(&win.spec.placement, size_l, &monitors(), cursor_pos(), below);
        let after = if win.spec.topmost { HWND_TOPMOST } else { HWND_TOP };
        unsafe {
            let _ = SetWindowPos(win.hwnd, after, r.rect.l, r.rect.t, r.rect.w(), r.rect.h(), flags);
        }
        // the monitor may have another DPI than the window had: adopt it before the first frame
        self.after_move(id, Some(r.mon));
        self.sync_size(id);
    }

    /// Adopt the monitor's DPI / refresh rate after a move.
    fn after_move(&mut self, id: WindowId, mon: Option<MonitorInfo>) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        let dpi_scale = unsafe { geom::scale_from_dpi(GetDpiForWindow(win.hwnd)) };
        if (dpi_scale - win.scale).abs() > 1e-3 {
            self.set_scale(id, dpi_scale);
        }
        if let Some(win) = self.wins.get_mut(&id) {
            let hz = mon.map(|m| m.refresh_hz).or_else(|| monitor_of_window(hv(win.hwnd)).map(|m| m.refresh_hz)).unwrap_or(win.hz);
            if hz != win.hz {
                win.hz = hz;
                win.frame.set_refresh_hz(hz as f32);
                if let Some(p) = win.prox.as_mut() {
                    p.set_refresh_hz(hz);
                }
            }
        }
    }

    fn set_scale(&mut self, id: WindowId, scale: f32) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        win.scale = scale;
        win.sh.scale.set(scale);
        win.tr.set_scale(scale);
        win.cx.scale = scale;
        if let Some(m) = win.spec.min_size {
            win.sh.min_phys.set(geom::size_to_phys(m, scale));
        }
        if let Some(p) = win.prox.as_mut() {
            p.set_scale(scale);
        }
        win.frame.request();
        self.call_view(id, |v, cx| {
            v.on_dpi(scale, cx);
            Response::NONE
        });
    }

    fn show(&mut self, id: WindowId, below: Option<i32>) {
        let now = self.now_ms();
        let (kind, activate, hidden) = match self.wins.get(&id) {
            Some(w) => (w.spec.kind, w.spec.activate_on_show, !w.visible),
            None => return,
        };
        if hidden && kind != WindowKind::Normal {
            // Popup / Overlay / Fullscreen follow their placement rules at every show (monitor under the cursor, taskbar, DPI);
            // a Normal window keeps the geometry it was created with / the user gave it
            self.place(id, below, SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
        // the first frame is drawn BEFORE the window appears (no flash of an empty window)
        if hidden {
            if let Some(win) = self.wins.get_mut(&id) {
                win.visible = true;
                win.blur.note_shown(now);
                win.frame.request();
            }
            self.call_view(id, |v, cx| {
                v.on_show(cx);
                Response::redraw()
            });
            self.render(id, true);
        }
        let Some(win) = self.wins.get_mut(&id) else { return };
        let hwnd = win.hwnd;
        let maximize = kind == WindowKind::Normal && !win.shown_once && matches!(win.spec.placement, Placement::Restore(g) if g.maximized);
        win.shown_once = true;
        unsafe {
            if kind == WindowKind::Normal {
                let _ = ShowWindow(hwnd, if maximize { SW_SHOWMAXIMIZED } else { SW_SHOW });
            } else {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                if win.spec.topmost {
                    let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
            }
        }
        if let Some(mut w) = self.shared.windows.lock().ok() {
            if let Some(i) = w.get_mut(&id) {
                i.visible = true;
            }
        }
        let hook = self.wins.get(&id).and_then(|w| w.spec.on_event.clone());
        if hidden {
            self.emit(id, &hook, WinEvent::Shown);
            // a present to a window that was not on screen yet may have been dropped: draw once more now that it is
            if let Some(w) = self.wins.get_mut(&id) {
                w.retry_at = 0.0;
                w.frame.request();
            }
            self.render(id, true);
        }
        if activate && kind != WindowKind::Overlay {
            self.activate(id, true);
        }
        if let Some(w) = self.wins.get_mut(&id) {
            w.poll_at = 0.0;
            w.watch_at = now + 250.0;
            if let Some(f) = w.watch.as_mut() {
                f.reset();
            }
            w.frame.request();
        }
    }

    fn hide(&mut self, id: WindowId, reason: HideReason) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        if !win.visible {
            return;
        }
        win.visible = false;
        let (hwnd, hook) = (win.hwnd, win.spec.on_event.clone());
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        if let Some(mut w) = self.shared.windows.lock().ok() {
            if let Some(i) = w.get_mut(&id) {
                i.visible = false;
            }
        }
        if reason != HideReason::Blur {
            self.activate(id, false);
        }
        self.call_view(id, |v, cx| {
            v.on_hide(cx);
            Response::NONE
        });
        self.emit(id, &hook, WinEvent::Hidden(reason));
    }

    /// `pill_activate` / `grab_keyboard`: take the foreground (borrowing the old foreground thread's input state so Windows allows it) or
    /// give it back.
    fn activate(&mut self, id: WindowId, on: bool) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        let hwnd = win.hwnd;
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            if on {
                if win.spec.kind == WindowKind::Overlay {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE.0 as isize));
                    win.sh.activatable.set(true);
                }
                let fg = GetForegroundWindow();
                if fg != hwnd {
                    win.prev_fg = hv(fg);
                }
                let (mine, theirs) = (GetCurrentThreadId(), GetWindowThreadProcessId(fg, None));
                let attached = theirs != 0 && theirs != mine && AttachThreadInput(mine, theirs, true).as_bool();
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(hwnd);
                if attached {
                    let _ = AttachThreadInput(mine, theirs, false);
                }
            } else {
                if win.spec.kind == WindowKind::Overlay {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
                    win.sh.activatable.set(false);
                }
                let prev = std::mem::take(&mut win.prev_fg);
                if prev != 0 && GetForegroundWindow() == hwnd && IsWindow(hw(prev)).as_bool() {
                    let _ = SetForegroundWindow(hw(prev));
                }
            }
        }
    }

    /// The client size changed (or may have): resize the swap chain to it.
    fn sync_size(&mut self, id: WindowId) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        let mut r = RECT::default();
        unsafe {
            if GetClientRect(win.hwnd, &mut r).is_err() {
                return;
            }
        }
        let size = ((r.right - r.left).max(1), (r.bottom - r.top).max(1));
        if size == win.size_px {
            return;
        }
        win.size_px = size;
        win.frame.request();
        let mut lost = false;
        if let (Some(p), Some(g)) = (win.pres.as_mut(), self.gpu.as_ref()) {
            if let Err(e) = p.resize(&g.gfx, size.0 as u32, size.1 as u32) {
                eprintln!("ui host: resize failed: {e}");
                lost = Presenter::is_device_lost(&e);
            }
        }
        if lost {
            self.recover_device();
        }
    }

    fn resize_logical(&mut self, id: WindowId, size_l: (f32, f32)) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        win.size_l = size_l;
        let mut cur = RECT::default();
        unsafe {
            let _ = GetWindowRect(win.hwnd, &mut cur);
        }
        let sz = geom::size_to_phys(size_l, win.scale);
        let r = geom::resize_keep_top_center(irect(cur), sz);
        unsafe {
            let _ = SetWindowPos(win.hwnd, None, r.l, r.t, r.w(), r.h(), SWP_NOZORDER | SWP_NOACTIVATE);
        }
        self.sync_size(id);
    }

    fn set_bounds(&mut self, id: WindowId, r: IRect) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        unsafe {
            let after = if win.spec.topmost { HWND_TOPMOST } else { HWND_TOP };
            let _ = SetWindowPos(win.hwnd, after, r.l, r.t, r.w(), r.h(), SWP_NOACTIVATE);
        }
        self.after_move(id, None);
        self.sync_size(id);
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // view calls and effects
    // ------------------------------------------------------------------------------------------------------------------------------

    fn prepare_cx(&mut self, id: WindowId) {
        let now = self.now_ms();
        let lang = if self.shared.lang_ru.load(Ordering::Relaxed) { Lang::Ru } else { Lang::En };
        let reduced = glasskit::ease::reduced();
        if let Some(win) = self.wins.get_mut(&id) {
            win.cx.now_ms = now;
            win.cx.lang = lang;
            win.cx.reduced_motion = reduced;
            win.cx.scale = win.scale;
            win.cx.refresh_hz = win.hz as f32;
        }
    }

    /// Calls into the view, then applies the response and the effects it asked for.
    fn call_view(&mut self, id: WindowId, f: impl FnOnce(&mut dyn crate::ui::view::View, &mut Cx) -> Response) -> Response {
        self.prepare_cx(id);
        let resp = match self.wins.get_mut(&id) {
            Some(win) => catch_unwind(AssertUnwindSafe(|| f(&mut *win.view, &mut win.cx))).unwrap_or_else(|_| {
                eprintln!("ui host: a panic in view `{}` was caught", win.spec.label);
                Response::NONE
            }),
            None => return Response::NONE,
        };
        if let Some(win) = self.wins.get_mut(&id) {
            if resp.redraw {
                win.frame.request();
            }
            if let Some(c) = resp.cursor {
                win.sh.cursor.set(c.into());
            }
        }
        self.apply_fx(id);
        resp
    }

    fn apply_fx(&mut self, id: WindowId) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        let fx: Effects = win.cx.take_effects();
        if fx == Effects::default() {
            return;
        }
        if fx.frame {
            win.frame.request();
        }
        if let Some(c) = fx.cursor {
            win.sh.cursor.set(c);
        }
        if fx.hit_all {
            win.hit = HitRegion::All;
        }
        if let Some(r) = fx.hit_region {
            win.hit = HitRegion::Rects(r);
        }
        if fx.hit_all || win.hit != *win.sh.hit.borrow() {
            *win.sh.hit.borrow_mut() = win.hit.clone();
            if let Some(p) = win.prox.as_mut() {
                p.set_hit_region(win.hit.clone());
            }
            win.poll_at = 0.0; // re-evaluate click-through now
        }
        if let Some(c) = fx.caption_region {
            *win.sh.caption.borrow_mut() = c;
        }
        if let Some(p) = win.prox.as_mut() {
            if let Some(z) = fx.drop_zone {
                p.set_drop_zone(z);
            }
            if let Some(v) = fx.pass_through {
                p.set_pass_through(v);
                win.poll_at = 0.0;
            }
            if let Some(v) = fx.gaze {
                p.set_gaze(v);
                win.poll_at = 0.0;
            }
        }
        if let Some(c) = fx.fps_cap {
            win.spec.fps_cap = c;
            win.frame.set_cap(c);
        }
        if let Some(t) = &fx.title {
            let w: Vec<u16> = t.encode_utf16().chain(Some(0)).collect();
            unsafe {
                let _ = SetWindowTextW(win.hwnd, PCWSTR(w.as_ptr()));
            }
        }
        if let Some(r) = fx.ime_rect {
            unsafe { set_ime_pos(win.hwnd, r, win.scale) };
        }
        let hwnd = win.hwnd;
        if let Some(c) = fx.window_cmd {
            unsafe {
                match c {
                    WindowCmd::Minimize => {
                        let _ = ShowWindow(hwnd, SW_MINIMIZE);
                    }
                    WindowCmd::ToggleMaximize => {
                        let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
                    }
                    WindowCmd::DragMove => {
                        let _ = ReleaseCapture();
                        let _ = PostMessageW(hwnd, WM_SYSCOMMAND, WPARAM((SC_MOVE | 2) as usize), LPARAM(0)); // 2 = HTCAPTION
                    }
                }
            }
        }
        if let Some(on) = fx.focus {
            self.activate(id, on);
        }
        for OpenRequest { name, args } in fx.opens {
            self.open_registered(&name, args);
        }
        if fx.show {
            self.show(id, None);
        }
        if fx.hide {
            self.hide(id, HideReason::Requested);
        }
        if fx.close {
            self.close(id);
        }
    }

    fn open_registered(&mut self, name: &str, args: Value) {
        let entry = self.shared.registry.lock().unwrap_or_else(|e| e.into_inner()).get(name).cloned();
        let Some((spec, factory)) = entry else {
            eprintln!("ui host: open_window(`{name}`): not registered");
            return;
        };
        if spec.singleton {
            if let Some(id) = self.wins.values().find(|w| w.spec.label == spec.label).map(|w| w.id) {
                self.show(id, None);
                self.activate(id, true);
                self.call_view(id, |v, cx| v.message(Msg::Open(args), cx));
                return;
            }
        }
        let id = WindowId(self.shared.next_id.fetch_add(1, Ordering::Relaxed));
        self.create(id, spec, Box::new(move |cx, a| factory(cx, a)), args);
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // commands and messages from other threads
    // ------------------------------------------------------------------------------------------------------------------------------

    fn drain_queue(&mut self) {
        for (id, msg) in self.shared.queue.drain() {
            if trace() {
                eprintln!("ui host: queue item for {id:?}: {}", match &msg { Msg::Custom(_) => "custom/command".to_string(), m => format!("{m:?}") });
            }
            match msg {
                Msg::Custom(b) if id == WindowId(0) => match b.downcast::<Cmd>() {
                    Ok(cmd) => self.on_cmd(*cmd),
                    Err(_) => {}
                },
                other => {
                    if self.wins.contains_key(&id) {
                        self.call_view(id, |v, cx| v.message(other, cx));
                    }
                }
            }
        }
    }

    fn on_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Create { id, spec, factory, args } => self.create(id, *spec, factory, args),
            Cmd::Open { name, args } => self.open_registered(&name, args),
            Cmd::Show { id, below } => self.show(id, below),
            Cmd::Hide(id) => self.hide(id, HideReason::Requested),
            Cmd::Toggle { id, below } => {
                let now = self.now_ms();
                match self.wins.get(&id) {
                    Some(w) if w.visible => self.hide(id, HideReason::Requested),
                    Some(w) if w.blur.reopen_blocked(now) => {} // the echo of the click that just hid it
                    Some(_) => self.show(id, below),
                    None => {}
                }
            }
            Cmd::Close(id) => self.close(id),
            Cmd::SetBounds { id, rect } => self.set_bounds(id, rect),
            Cmd::Relayout(id) => {
                if self.wins.get(&id).is_some_and(|w| w.visible) {
                    self.place(id, None, SWP_NOACTIVATE);
                }
            }
            Cmd::ForceIgnore { id, ignore } => {
                if let Some(w) = self.wins.get_mut(&id) {
                    w.force_ignore = ignore;
                    w.poll_at = 0.0;
                }
            }
            Cmd::SetTheme { id, theme } => {
                if let Some(w) = self.wins.get_mut(&id) {
                    w.theme = theme;
                    w.frame.request();
                }
            }
            Cmd::Frame(id) => {
                if let Some(w) = self.wins.get_mut(&id) {
                    w.frame.request();
                }
            }
            Cmd::Activate { id, on } => self.activate(id, on),
            Cmd::Shutdown => {
                let ids: Vec<WindowId> = self.wins.keys().copied().collect();
                for id in ids {
                    self.close(id);
                }
                self.quit = true;
            }
        }
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // rendering
    // ------------------------------------------------------------------------------------------------------------------------------

    fn recover_device(&mut self) {
        eprintln!("ui host: graphics device lost, rebuilding");
        for w in self.wins.values_mut() {
            w.pres = None;
        }
        self.gpu = None;
        if !self.ensure_gpu() {
            return;
        }
        let Some(gpu) = self.gpu.as_ref() else { return };
        for w in self.wins.values_mut() {
            let (cw, ch) = (w.size_px.0 as u32, w.size_px.1 as u32);
            let p = if w.layered { Presenter::new_layered(&gpu.gfx, w.hwnd, cw, ch) } else { Presenter::new_comp(&gpu.gfx, w.hwnd, cw, ch) };
            match p {
                Ok(p) => w.pres = Some(p),
                Err(e) => eprintln!("ui host: cannot rebuild the presenter of `{}`: {e}", w.spec.label),
            }
            w.frame.request();
        }
    }

    /// One frame of window `id`: step, size hint, draw, present. `force` renders even when the scheduler is idle (first frame at show).
    fn render(&mut self, id: WindowId, force: bool) {
        if self.gpu.is_none() && !self.ensure_gpu() {
            return;
        }
        let now = self.now_ms();
        match self.wins.get(&id) {
            Some(w) if w.visible && (force || w.frame.wants_frame()) && w.pres.is_some() => {}
            _ => return,
        }
        self.prepare_cx(id);
        // 1. step
        let keep = {
            let Some(win) = self.wins.get_mut(&id) else { return };
            let dt = win.frame.begin(now);
            let keep = catch_unwind(AssertUnwindSafe(|| win.view.step(dt, &mut win.cx))).unwrap_or_else(|_| {
                eprintln!("ui host: a panic in step of `{}` was caught", win.spec.label);
                false
            });
            keep
        };
        self.apply_fx(id);
        // 2. the view's size wish
        let hint = match self.wins.get(&id) {
            Some(w) if matches!(w.spec.kind, WindowKind::Overlay | WindowKind::Popup) => w.view.size_hint().filter(|h| *h != w.size_l),
            Some(_) => None,
            None => return,
        };
        if let Some(h) = hint {
            self.resize_logical(id, h);
        }
        // 3. draw + present
        let mut lost = false;
        let mut occluded = false;
        {
            let (Some(gpu), Some(win)) = (self.gpu.as_ref(), self.wins.get_mut(&id)) else { return };
            let Some(pres) = win.pres.as_mut() else { return };
            let (scale, theme, clear) = (win.scale, win.theme, clear_color(win.spec.clear));
            let rect = Rect::new(0.0, 0.0, win.size_px.0 as f32 / scale, win.size_px.1 as f32 / scale);
            let (view, cx) = (&mut win.view, &win.cx);
            let r = pres.draw(&gpu.gfx, scale, clear, || {
                let cv = Canvas::new(&gpu.gfx, &gpu.res, scale);
                if catch_unwind(AssertUnwindSafe(|| view.draw(&cv, &theme, rect, cx))).is_err() {
                    eprintln!("ui host: a panic in draw was caught");
                }
            });
            match r {
                Ok(()) => match pres.present(&gpu.gfx) {
                    PresentStatus::Ok => {}
                    PresentStatus::Occluded => occluded = true,
                    PresentStatus::DeviceLost => lost = true,
                    PresentStatus::Failed => {}
                },
                Err(e) => {
                    eprintln!("ui host: draw failed: {e}");
                    lost = Presenter::is_device_lost(&e) || e.code().0 as u32 == 0x8899000C; // D2DERR_RECREATE_TARGET
                }
            }
        }
        if let Some(win) = self.wins.get_mut(&id) {
            win.frame.end(keep);
            if occluded {
                win.retry_at = now + 100.0; // minimized / covered: look again in 100 ms instead of spinning
            }
        }
        if lost {
            self.recover_device();
        }
    }

    /// Renders every window whose frame is due. Composition windows are due when their swap chain's waitable object is signalled
    /// (consumed here); layered windows when their pacing deadline passed.
    fn render_due(&mut self) {
        let now = self.now_ms();
        let ids: Vec<WindowId> = self.wins.iter().filter(|(_, w)| w.visible && w.frame.wants_frame() && now >= w.retry_at).map(|(i, _)| *i).collect();
        for id in ids {
            let Some(w) = self.wins.get(&id) else { continue };
            if w.frame.wait_ms(now).is_none_or(|ms| ms > 0.0) {
                continue; // an explicit fps cap says not yet
            }
            let ready = match w.pres.as_ref().and_then(|p| p.latency_handle()) {
                // the sleep in `run_loop` already consumed this handle's signal (kept as a token), else try to consume it now
                Some(h) => self.ready.contains(&(h.0 as isize)) || unsafe { WaitForSingleObject(h, 0) == WAIT_OBJECT_0 },
                None => now >= w.pace_at, // layered: one frame per display period
            };
            if ready {
                self.render(id, false);
                if let Some(w) = self.wins.get_mut(&id) {
                    w.pace_at = now + 1000.0 / w.hz.max(1) as f64;
                }
            }
        }
        self.ready.clear();
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // timers: cursor poll, geometry memory, focus watchdog
    // ------------------------------------------------------------------------------------------------------------------------------

    fn run_timers(&mut self) {
        let now = self.now_ms();
        let ids: Vec<WindowId> = self.wins.keys().copied().collect();
        for id in ids {
            self.poll_proximity(id, now);
            self.watchdog(id, now);
            self.geometry(id, now);
        }
    }

    fn poll_proximity(&mut self, id: WindowId, now: f64) {
        let (hwnd, due) = match self.wins.get(&id) {
            Some(w) if w.prox.is_some() => (w.hwnd, now >= w.poll_at),
            _ => return,
        };
        if !due {
            return;
        }
        let (events, ignore, next) = {
            let Some(win) = self.wins.get_mut(&id) else { return };
            let mut wr = RECT::default();
            let (mut cur, mut mt) = (POINT::default(), 0);
            unsafe {
                if GetWindowRect(hwnd, &mut wr).is_err() || GetCursorPos(&mut cur).is_err() {
                    win.poll_at = now + 100.0;
                    return;
                }
                let hm = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                if GetMonitorInfoW(hm, &mut mi).as_bool() {
                    mt = mi.rcMonitor.top;
                }
            }
            let held = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 };
            let probe = Probe { now_ms: now, cursor: (cur.x, cur.y), window: irect(wr), monitor_top: mt, button_down: held, visible: win.visible };
            let out = win.prox.as_mut().map(|p| p.step(&probe)).unwrap();
            win.poll_at = now + out.next_poll_ms;
            let engaged = win.prox.as_ref().is_some_and(|p| p.engaged());
            win.sh.hit_all.set(engaged);
            (out.events, out.ignore, out.next_poll_ms)
        };
        let _ = next;
        if let Some(win) = self.wins.get_mut(&id) {
            let want = win.force_ignore.unwrap_or(ignore);
            if want != win.ignore_applied {
                win.ignore_applied = want;
                unsafe {
                    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    let tr = (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) as isize;
                    let new = if want { ex | tr } else { ex & !(WS_EX_TRANSPARENT.0 as isize) };
                    if new != ex {
                        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new);
                        if want {
                            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
                        }
                    }
                }
            }
        }
        for ev in events {
            self.call_view(id, |v, cx| v.on_proximity(&ev, cx));
        }
    }

    fn watchdog(&mut self, id: WindowId, now: f64) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        if !win.visible || win.watch.is_none() || now < win.watch_at {
            return;
        }
        win.watch_at = now + 250.0;
        let front = unsafe { GetForegroundWindow() } == win.hwnd;
        let hook = win.spec.on_event.clone();
        if win.watch.as_mut().is_some_and(|w| w.lost_too_long(front, now)) {
            self.emit(id, &hook, WinEvent::Hidden(HideReason::FocusLost));
            self.close(id);
        }
    }

    fn geometry(&mut self, id: WindowId, now: f64) {
        let Some(win) = self.wins.get_mut(&id) else { return };
        let Some(at) = win.geo_at else { return };
        if now < at {
            return;
        }
        win.geo_at = None;
        if !win.visible {
            return; // a hidden window's rectangle is not worth remembering
        }
        unsafe {
            if IsIconic(win.hwnd).as_bool() {
                return;
            }
            let maximized = IsZoomed(win.hwnd).as_bool();
            let mut r = RECT::default();
            if GetWindowRect(win.hwnd, &mut r).is_ok() && r.left > -30000 && r.top > -30000 {
                let s = win.scale;
                let g = Geometry { x: r.left, y: r.top, w: ((r.right - r.left) as f32 / s).round(), h: ((r.bottom - r.top) as f32 / s).round(), maximized };
                let hook = win.spec.on_event.clone();
                self.emit(id, &hook, WinEvent::Geometry(g));
            }
        }
    }

    /// Earliest absolute time (ms) something needs the loop to wake without input or a frame handle.
    fn next_deadline(&self) -> Option<f64> {
        let now = self.now_ms();
        let mut best: Option<f64> = None;
        let mut put = |t: f64| best = Some(best.map_or(t, |b: f64| b.min(t)));
        for w in self.wins.values() {
            if w.prox.is_some() && w.visible {
                put(w.poll_at);
            }
            if let Some(t) = w.geo_at {
                put(t);
            }
            if w.watch.is_some() && w.visible {
                put(w.watch_at);
            }
            if w.visible && w.frame.wants_frame() {
                if w.retry_at > now {
                    put(w.retry_at);
                }
                match w.frame.wait_ms(now) {
                    Some(ms) if ms > 0.0 => put(now + ms),
                    Some(_) if w.pres.as_ref().is_some_and(|p| p.latency_handle().is_none()) => put(w.pace_at), // layered pacing
                    _ => {}
                }
            }
        }
        best
    }

    fn frame_handles(&self) -> Vec<HANDLE> {
        let now = self.now_ms();
        self.wins
            .values()
            .filter(|w| w.visible && w.frame.wants_frame() && now >= w.retry_at && w.frame.wait_ms(now) == Some(0.0))
            .filter_map(|w| w.pres.as_ref().and_then(|p| p.latency_handle()))
            .take(60)
            .collect()
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // Win32 messages
    // ------------------------------------------------------------------------------------------------------------------------------

    fn on_message(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if hwnd == self.msg_hwnd {
            return None;
        }
        let id = *self.by_hwnd.get(&hv(hwnd))?;
        match msg {
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MBUTTONDBLCLK
            | WM_XBUTTONDOWN | WM_XBUTTONUP | WM_XBUTTONDBLCLK | WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WM_MOUSELEAVE | WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP | WM_CHAR
            | WM_SETFOCUS | WM_KILLFOCUS => {
                self.on_input(id, hwnd, msg, wp, lp);
                // Alt+key must still reach the system (Alt+F4, Alt+Space)
                if matches!(msg, WM_SYSKEYDOWN | WM_SYSKEYUP) {
                    None
                } else {
                    Some(LRESULT(0))
                }
            }
            WM_CAPTURECHANGED => {
                if let Some(w) = self.wins.get_mut(&id) {
                    w.captured = false;
                }
                None
            }
            WM_PAINT => {
                if let Some(w) = self.wins.get_mut(&id) {
                    w.frame.request();
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                    self.on_deactivate(id);
                }
                None
            }
            WM_SIZE => {
                let maximized = wp.0 as u32 == SIZE_MAXIMIZED;
                if let Some(w) = self.wins.get_mut(&id) {
                    w.sh.maximized.set(maximized);
                    if w.spec.kind == WindowKind::Normal {
                        w.geo_at = Some(self.t0.elapsed().as_secs_f64() * 1000.0 + 700.0);
                    }
                }
                self.sync_size(id);
                if self.wins.get(&id).is_some_and(|w| w.in_size_move) {
                    self.render(id, true); // the modal sizing loop does not run ours: repaint right here
                }
                Some(LRESULT(0))
            }
            WM_MOVE => {
                let now = self.now_ms();
                if let Some(w) = self.wins.get_mut(&id) {
                    if w.spec.kind == WindowKind::Normal {
                        w.geo_at = Some(now + 700.0);
                    }
                }
                self.after_move(id, None);
                None
            }
            WM_ENTERSIZEMOVE => {
                if let Some(w) = self.wins.get_mut(&id) {
                    w.in_size_move = true;
                    unsafe {
                        SetTimer(hwnd, T_MODAL, 4, None); // the move / size loop is modal: keep frames coming (the OS clamps to ~10 ms)
                    }
                }
                None
            }
            WM_EXITSIZEMOVE => {
                if let Some(w) = self.wins.get_mut(&id) {
                    w.in_size_move = false;
                    unsafe {
                        let _ = KillTimer(hwnd, T_MODAL);
                    }
                }
                self.after_move(id, None);
                self.sync_size(id);
                None
            }
            WM_TIMER if wp.0 == T_MODAL => {
                self.render(id, false);
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xFFFF) as u32;
                let scale = geom::scale_from_dpi(dpi);
                let suggested = unsafe { *(lp.0 as *const RECT) };
                let kind = self.wins.get(&id).map(|w| w.spec.kind)?;
                self.set_scale(id, scale);
                unsafe {
                    match kind {
                        WindowKind::Fullscreen => {
                            // stay glued to the monitor
                            if let Some(m) = monitor_of_window(hv(hwnd)) {
                                let _ = SetWindowPos(hwnd, None, m.rect.l, m.rect.t, m.rect.w(), m.rect.h(), SWP_NOZORDER | SWP_NOACTIVATE);
                            }
                        }
                        WindowKind::Overlay | WindowKind::Popup => {
                            let size_l = self.wins.get(&id).map(|w| w.size_l)?;
                            let sz = geom::size_to_phys(size_l, scale);
                            let r = geom::resize_keep_top_center(irect(suggested), sz);
                            let _ = SetWindowPos(hwnd, None, r.l, r.t, r.w(), r.h(), SWP_NOZORDER | SWP_NOACTIVATE);
                        }
                        WindowKind::Normal => {
                            let _ = SetWindowPos(hwnd, None, suggested.left, suggested.top, suggested.right - suggested.left, suggested.bottom - suggested.top, SWP_NOZORDER | SWP_NOACTIVATE);
                        }
                    }
                }
                self.after_move(id, None);
                self.sync_size(id);
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                // refresh rate / monitor layout / "show animations" may have changed
                for w in self.wins.values_mut() {
                    w.frame.request();
                }
                let ids: Vec<WindowId> = self.wins.keys().copied().collect();
                for i in ids {
                    self.after_move(i, None);
                }
                None
            }
            WM_CLOSE => {
                let kind = self.wins.get(&id).map(|w| w.spec.kind)?;
                match kind {
                    WindowKind::Normal | WindowKind::Fullscreen => self.close(id),
                    WindowKind::Popup | WindowKind::Overlay => self.hide(id, HideReason::Requested),
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn on_deactivate(&mut self, id: WindowId) {
        let now = self.now_ms();
        let (hook, hide) = match self.wins.get_mut(&id) {
            Some(w) if w.visible => {
                let hide = w.spec.hide_on_blur && w.blur.blur_hides(now);
                (w.spec.on_event.clone(), hide)
            }
            _ => return,
        };
        self.emit(id, &hook, WinEvent::Blur);
        if hide {
            self.hide(id, HideReason::Blur);
        }
    }

    fn on_input(&mut self, id: WindowId, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) {
        let ev = {
            let Some(win) = self.wins.get_mut(&id) else { return };
            if matches!(msg, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
                let mut o = POINT::default();
                unsafe {
                    let _ = ClientToScreen(hwnd, &mut o);
                }
                win.tr.set_client_origin(o.x, o.y);
            }
            let ev = win.tr.translate(msg, wp.0, lp.0, unsafe { GetMessageTime() } as u32);
            if win.tr.take_track_request() {
                let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                unsafe {
                    let _ = TrackMouseEvent(&mut t);
                }
            }
            ev
        };
        let Some(ev) = ev else { return };
        // the default cursor of the window comes back on every pointer event; the view's answer overrides it
        if matches!(ev, InputEvent::MouseMove { .. } | InputEvent::MouseDown { .. }) {
            if let Some(win) = self.wins.get(&id) {
                win.sh.cursor.set(if win.spec.kind == WindowKind::Fullscreen { CursorShape::Crosshair } else { CursorShape::Arrow });
            }
        }
        let resp = self.call_view(id, |v, cx| v.event(&ev, cx));
        let Some(win) = self.wins.get_mut(&id) else { return };
        unsafe {
            if resp.capture && !win.captured && matches!(ev, InputEvent::MouseDown { .. } | InputEvent::MouseMove { .. }) {
                SetCapture(hwnd);
                win.captured = true;
            }
            if win.captured && matches!(ev, InputEvent::MouseUp { .. }) && !resp.capture {
                let _ = ReleaseCapture();
                win.captured = false;
            }
        }
        // keep the cursor shape current without waiting for the next WM_SETCURSOR
        if matches!(ev, InputEvent::MouseMove { .. }) {
            unsafe {
                let mut p = POINT::default();
                if GetCursorPos(&mut p).is_ok() && ScreenToClient(hwnd, &mut p).as_bool() {
                    // the cursor is ours only while the pointer is over the client area; WM_SETCURSOR does the real work
                    SetCursor(load_cursor(win.sh.cursor.get()));
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------------------------------------------
    // drag and drop (called by the IDropTarget)
    // ------------------------------------------------------------------------------------------------------------------------------

    pub(super) fn on_drop(&mut self, id: WindowId, phase: crate::ui::view::DropPhase, screen: (i32, i32), payload: Option<crate::ui::view::DropPayload>) -> bool {
        let pos = match self.wins.get(&id) {
            Some(w) => {
                let mut p = POINT { x: screen.0, y: screen.1 };
                unsafe {
                    let _ = ScreenToClient(w.hwnd, &mut p);
                }
                (p.x as f32 / w.scale, p.y as f32 / w.scale)
            }
            None => return false,
        };
        let ev = crate::ui::view::DropEvent { phase, pos, payload };
        self.call_view(id, |v, cx| v.on_drop(&ev, cx)).consumed
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// styles, IME
// ---------------------------------------------------------------------------------------------------------------------------------------

fn styles(spec: &WindowSpec, layered: bool) -> (WINDOW_EX_STYLE, WINDOW_STYLE) {
    let mut ex = WINDOW_EX_STYLE(0);
    let style;
    match spec.kind {
        WindowKind::Overlay => {
            style = WS_POPUP;
            ex |= WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED;
        }
        WindowKind::Popup => {
            style = WS_POPUP;
            ex |= WS_EX_TOOLWINDOW;
        }
        WindowKind::Normal => {
            let mut s = WS_OVERLAPPEDWINDOW;
            if !spec.resizable {
                s &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
            }
            style = s;
            ex |= WS_EX_APPWINDOW;
        }
        WindowKind::Fullscreen => {
            style = WS_POPUP;
            ex |= WS_EX_TOOLWINDOW;
        }
    }
    if spec.topmost {
        ex |= WS_EX_TOPMOST;
    }
    if layered {
        ex |= WS_EX_LAYERED;
    } else {
        ex |= WS_EX_NOREDIRECTIONBITMAP;
    }
    (ex, style)
}

unsafe fn set_ime_pos(hwnd: HWND, r: Rect, scale: f32) {
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() {
        return;
    }
    let pt = POINT { x: (r.x * scale) as i32, y: ((r.y + r.h) * scale) as i32 };
    let cf = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: pt, rcArea: RECT::default() };
    let _ = ImmSetCompositionWindow(himc, &cf);
    let cand = CANDIDATEFORM { dwIndex: 0, dwStyle: CFS_CANDIDATEPOS, ptCurrentPos: pt, rcArea: RECT::default() };
    let _ = ImmSetCandidateWindow(himc, &cand);
    let _ = ImmReleaseContext(hwnd, himc);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// thread
// ---------------------------------------------------------------------------------------------------------------------------------------

pub(super) fn start() -> HostHandle {
    let msg_hwnd = Arc::new(AtomicIsize::new(0));
    let wake_target = msg_hwnd.clone();
    let queue = Arc::new(MsgQueue::new(move || {
        let h = wake_target.load(Ordering::Acquire);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(hw(h), WM_APP_WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }));
    let shared = Arc::new(Shared {
        next_id: std::sync::atomic::AtomicU64::new(1),
        queue,
        registry: Mutex::new(HashMap::new()),
        windows: Mutex::new(HashMap::new()),
        msg_hwnd,
        open: std::sync::atomic::AtomicUsize::new(0),
        lang_ru: std::sync::atomic::AtomicBool::new(false),
        thread: Mutex::new(None),
    });
    let s2 = shared.clone();
    let th = std::thread::Builder::new().name("pill-ui".into()).spawn(move || ui_thread(s2)).expect("spawn the UI thread");
    *shared.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(th);
    HostHandle(shared)
}

fn ui_thread(shared: Arc<Shared>) {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = OleInitialize(None);
        let hinst = match GetModuleHandleW(None) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("ui host: GetModuleHandle failed: {e}");
                return;
            }
        };
        let cls = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: CLASS, hCursor: HCURSOR::default(), ..Default::default() };
        RegisterClassExW(&cls);
        let mcls = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc), hInstance: hinst.into(), lpszClassName: MSG_CLASS, ..Default::default() };
        RegisterClassExW(&mcls);
        let msg_hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), MSG_CLASS, w!("pill-ui-host"), WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, None, hinst, None).unwrap_or_default();
        shared.msg_hwnd.store(hv(msg_hwnd), Ordering::Release);
        let timer = CreateWaitableTimerExW(None, PCWSTR::null(), 0x2 /* CREATE_WAITABLE_TIMER_HIGH_RESOLUTION */, 0x1F0003 /* TIMER_ALL_ACCESS */).unwrap_or_default();
        let host = Host { shared, t0: Instant::now(), gpu: None, wins: HashMap::new(), by_hwnd: HashMap::new(), msg_hwnd, timer, quit: false, ready: Default::default() };
        HOST.with(|h| *h.borrow_mut() = Some(host));
        run_loop();
        let host = HOST.with(|h| h.borrow_mut().take());
        drop(host);
        if !timer.is_invalid() {
            let _ = CloseHandle(timer);
        }
        let _ = DestroyWindow(msg_hwnd);
        OleUninitialize();
    }
}

/// `PILL_HOST_TRACE=1` prints the loop's decisions to stderr (diagnostics for the lead; off by default).
fn trace() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| std::env::var_os("PILL_HOST_TRACE").is_some_and(|v| v != "0"))
}

fn run_loop() {
    unsafe {
        loop {
            // 1. every pending message (input, DPI, activation...)
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                if m.message == WM_QUIT {
                    return;
                }
                if trace() {
                    eprintln!("ui host: dispatch {:#x} to {:#x} visible={} label={:?}", m.message, m.hwnd.0 as isize, IsWindowVisible(m.hwnd).as_bool(), with_host(|h| h.by_hwnd.get(&(m.hwnd.0 as isize)).and_then(|i| h.wins.get(i)).map(|w| w.spec.label.clone())));
                }
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            // 2. commands and view messages from other threads, timers, due frames
            let (handles, timer, deadline_in) = {
                let step = HOST.with(|h| {
                    let mut g = h.try_borrow_mut().ok()?;
                    let host = g.as_mut()?;
                    catch_unwind(AssertUnwindSafe(|| {
                        host.drain_queue();
                        if host.quit {
                            return None;
                        }
                        host.run_timers();
                        host.render_due();
                        let now = host.now_ms();
                        let dl = host.next_deadline().map(|t| (t - now).max(0.0));
                        Some((host.frame_handles(), host.timer, dl))
                    }))
                    .unwrap_or_else(|_| {
                        eprintln!("ui host: a panic in the loop was caught");
                        Some((Vec::new(), HANDLE::default(), Some(50.0)))
                    })
                });
                match step {
                    Some(s) => s,
                    None => {
                        if trace() {
                            eprintln!("ui host: loop ends (quit requested or the host state is gone)");
                        }
                        return;
                    }
                }
            };
            drain_deferred();
            // 3. sleep until a frame handle, the next deadline or a message
            let mut set: Vec<HANDLE> = Vec::with_capacity(handles.len() + 1);
            if !timer.is_invalid() {
                match deadline_in {
                    Some(ms) => {
                        let due = -((ms * 10_000.0) as i64).max(1); // 100 ns units, negative = relative
                        let _ = SetWaitableTimer(timer, &due, 0, None, None, false);
                    }
                    None => {
                        let _ = CancelWaitableTimer(timer);
                    }
                }
                set.push(timer);
            }
            set.extend(handles);
            let timeout = if timer.is_invalid() { deadline_in.map_or(INFINITE, |ms| ms.ceil() as u32) } else { INFINITE };
            if trace() {
                eprintln!("ui host: sleeping on {} handle(s), timeout {} ms", set.len(), if timeout == INFINITE { -1 } else { timeout as i64 });
            }
            let r = MsgWaitForMultipleObjectsEx(Some(&set), timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            if trace() {
                eprintln!("ui host: woke with {:#x}", r.0);
            }
            // waiting consumed the signal of a frame handle: remember it so `render_due` does not wait for a second one
            let idx = r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
            let first_handle = usize::from(!timer.is_invalid());
            if idx >= first_handle && idx < set.len() {
                let h = set[idx].0 as isize;
                let _ = with_host(|host| host.ready.insert(h));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mons() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo { rect: IRect::new(0, 0, 2560, 1440), work: IRect::new(0, 48, 2560, 1440), primary: true, scale: 1.0, refresh_hz: 240 },
            MonitorInfo { rect: IRect::new(2560, 0, 4480, 1080), work: IRect::new(2560, 0, 4480, 1040), primary: false, scale: 1.5, refresh_hz: 60 },
        ]
    }

    #[test]
    fn the_island_sits_under_the_taskbar_centred_on_its_monitor() {
        let p = Placement::TopCenter { monitor: MonitorSel::Primary, offset_pct: 0.0, below_taskbar: true, margin_top: 0.0 };
        let r = resolve_placement(&p, (720.0, 120.0), &mons(), (0, 0), None);
        assert_eq!(r.rect, IRect::from_xywh(920, 48, 720, 120));
        assert_eq!(r.mon.refresh_hz, 240);
    }

    #[test]
    fn the_bar_follows_the_cursor_monitor_and_its_dpi_and_hangs_below_the_island() {
        let p = Placement::TopCenter { monitor: MonitorSel::UnderCursor, offset_pct: 0.0, below_taskbar: false, margin_top: 7.0 };
        let r = resolve_placement(&p, (560.0, 350.0), &mons(), (3000, 500), None);
        assert_eq!(r.mon.scale, 1.5);
        assert_eq!(r.rect, IRect::from_xywh(2560 + (1920 - 840) / 2, 11, 840, 525), "7 logical px = 11 physical at 150 %");
        let below = resolve_placement(&p, (560.0, 350.0), &mons(), (3000, 500), Some(120));
        assert_eq!(below.rect.t, 120, "never above the island's bottom edge");
        let off = resolve_placement(&p, (560.0, 350.0), &mons(), (100, 100), None);
        assert_eq!((off.mon.scale, off.rect.t), (1.0, 7));
    }

    #[test]
    fn restore_falls_back_to_the_centre_when_the_monitor_is_gone() {
        let g = Geometry { x: 9000, y: 100, w: 840.0, h: 760.0, maximized: false };
        let r = resolve_placement(&Placement::Restore(g), (840.0, 760.0), &mons(), (0, 0), None);
        assert_eq!(r.rect, IRect::from_xywh((2560 - 840) / 2, (1440 - 760) / 2, 840, 760));
        let g = Geometry { x: 100, y: 100, ..g };
        assert_eq!(resolve_placement(&Placement::Restore(g), (840.0, 760.0), &mons(), (0, 0), None).rect, IRect::from_xywh(100, 100, 840, 760));
    }

    #[test]
    fn fullscreen_covers_the_exact_monitor() {
        let r = resolve_placement(&Placement::Cover(MonitorSel::Index(1)), (400.0, 300.0), &mons(), (0, 0), None);
        assert_eq!(r.rect, IRect::new(2560, 0, 4480, 1080));
        let r = resolve_placement(&Placement::Cover(MonitorSel::Rect(IRect::new(2560, 0, 4480, 1080))), (1.0, 1.0), &mons(), (0, 0), None);
        assert_eq!(r.rect.w(), 1920);
        let r = resolve_placement(&Placement::Cover(MonitorSel::Index(9)), (1.0, 1.0), &mons(), (0, 0), None);
        assert_eq!(r.rect, IRect::new(0, 0, 2560, 1440), "an unknown index falls back to the primary");
    }

    #[test]
    fn the_monitor_list_is_primary_first_and_sane() {
        let m = monitors();
        assert!(!m.is_empty(), "a desktop session has at least one monitor");
        assert!(m[0].primary, "the primary comes first (MonitorSel::Index(0))");
        assert_eq!(m.iter().filter(|x| x.primary).count(), 1);
        for x in &m {
            assert!(x.rect.w() > 0 && x.rect.h() > 0 && x.scale >= 0.25 && x.refresh_hz >= 24, "{x:?}");
            assert!(x.work.w() <= x.rect.w() && x.work.h() <= x.rect.h());
        }
    }

    #[test]
    fn styles_per_kind() {
        let o = super::super::NativeWindow::overlay("p").into_spec();
        let (ex, st) = styles(&o, false);
        assert!(ex.contains(WS_EX_NOACTIVATE) && ex.contains(WS_EX_TOPMOST) && ex.contains(WS_EX_TOOLWINDOW) && ex.contains(WS_EX_NOREDIRECTIONBITMAP));
        assert!(st.contains(WS_POPUP));
        let (ex, _) = styles(&o, true);
        assert!(ex.contains(WS_EX_LAYERED) && !ex.contains(WS_EX_NOREDIRECTIONBITMAP), "the layered fallback is a plain layered window");
        let n = super::super::NativeWindow::normal("s").into_spec();
        let (ex, st) = styles(&n, false);
        assert!(st.contains(WS_THICKFRAME) && ex.contains(WS_EX_APPWINDOW) && !ex.contains(WS_EX_TOPMOST));
        let b = super::super::NativeWindow::popup("b").into_spec();
        let (ex, _) = styles(&b, false);
        assert!(!ex.contains(WS_EX_NOACTIVATE), "the bar must be able to take the keyboard");
    }
}
