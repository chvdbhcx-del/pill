//! How pixels reach the screen. Two back ends behind one `Presenter`:
//!
//! * **Composition** (default): a DXGI flip-model swap chain created `ForComposition` (premultiplied alpha, `FLIP_SEQUENTIAL`, two buffers)
//!   attached to a DirectComposition visual of an `WS_EX_NOREDIRECTIONBITMAP` window. DWM blends it with the desktop, no CPU readback, and
//!   the swap chain's frame-latency waitable object (maximum latency 1) paces the UI thread to the display's vblank: one signal per refresh
//!   at ANY rate (60, 144, 240 Hz), no timers, no cap.
//! * **Layered** (`PILL_FORCE_LAYERED=1`, or when composition cannot be set up): the frame is drawn to a Direct2D target, read back and handed
//!   to `UpdateLayeredWindow` through a DIB section (the approach `mswidgets` ships). Slower and not vsynced; paced by a timer at the
//!   display period.
//!
//! Drawing goes through [`Presenter::draw`], which sets the target, clears, applies the DPI transform and calls your closure (build a
//! `Canvas` in it). Direct2D is single-threaded: call from the UI thread only.
use glasskit::gfx::{bmp_props, mat, Gfx};
use windows::core::{Interface, Result};
use windows::Win32::Foundation::{COLORREF, E_FAIL, HANDLE, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

/// `DXGI_STATUS_OCCLUDED`: the window is minimized / fully covered; presenting is a no-op and the frame loop must not spin.
const STATUS_OCCLUDED: i32 = 0x087A0001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentStatus {
    Ok,
    /// Nothing is visible: slow the loop down and retry later.
    Occluded,
    /// The GPU device was removed or reset: rebuild `Gfx` and every presenter.
    DeviceLost,
    Failed,
}

const SWAP_FLAGS: i32 = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0;

struct Comp {
    swap: IDXGISwapChain1,
    latency: HANDLE,
    // kept alive for the lifetime of the swap chain attachment
    _dev: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
}

struct Layered {
    hwnd: HWND,
    mem_dc: HDC,
    dib: HBITMAP,
    bits: *mut u8,
    tgt: ID2D1Bitmap1,
    cpu: ID2D1Bitmap1,
}

enum Mode {
    Comp(Comp),
    Layered(Layered),
}

pub struct Presenter {
    mode: Mode,
    pub w: u32,
    pub h: u32,
}

fn device_lost(e: &windows::core::Error) -> bool {
    e.code() == DXGI_ERROR_DEVICE_REMOVED.into() || e.code() == DXGI_ERROR_DEVICE_RESET.into() || e.code() == DXGI_ERROR_DEVICE_HUNG.into()
}

impl Presenter {
    /// Composition presenter for `hwnd` (which must have `WS_EX_NOREDIRECTIONBITMAP`).
    pub fn new_comp(gfx: &Gfx, hwnd: HWND, w: u32, h: u32) -> Result<Presenter> {
        unsafe {
            let (w, h) = (w.max(1), h.max(1));
            let dxgi: IDXGIDevice = gfx.d3d.cast()?;
            let factory: IDXGIFactory2 = dxgi.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                Flags: SWAP_FLAGS as u32,
            };
            let swap = factory.CreateSwapChainForComposition(&gfx.d3d, &desc, None::<&IDXGIOutput>)?;
            let swap2: IDXGISwapChain2 = swap.cast()?;
            swap2.SetMaximumFrameLatency(1)?;
            let latency = swap2.GetFrameLatencyWaitableObject();
            if latency.is_invalid() {
                return Err(E_FAIL.into());
            }
            let dev: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dev.CreateTargetForHwnd(hwnd, true)?;
            let visual = dev.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dev.Commit()?;
            Ok(Presenter { mode: Mode::Comp(Comp { swap, latency, _dev: dev, _target: target, _visual: visual }), w, h })
        }
    }

    /// Layered-window presenter (readback + `UpdateLayeredWindow`); `hwnd` must have `WS_EX_LAYERED`.
    pub fn new_layered(gfx: &Gfx, hwnd: HWND, w: u32, h: u32) -> Result<Presenter> {
        let (w, h) = (w.max(1), h.max(1));
        let l = Layered::new(gfx, hwnd, w, h)?;
        Ok(Presenter { mode: Mode::Layered(l), w, h })
    }

    pub fn is_comp(&self) -> bool {
        matches!(self.mode, Mode::Comp(_))
    }

    /// The swap chain's frame-latency waitable object: signalled when the next frame may be started (one per vblank). `None` for layered.
    pub fn latency_handle(&self) -> Option<HANDLE> {
        match &self.mode {
            Mode::Comp(c) => Some(c.latency),
            Mode::Layered(_) => None,
        }
    }

    /// New back-buffer size in device px. A no-op when unchanged.
    pub fn resize(&mut self, gfx: &Gfx, w: u32, h: u32) -> Result<()> {
        let (w, h) = (w.max(1), h.max(1));
        if (w, h) == (self.w, self.h) {
            return Ok(());
        }
        unsafe {
            match &mut self.mode {
                Mode::Comp(c) => {
                    gfx.dc.SetTarget(None::<&ID2D1Image>); // the swap chain cannot resize while a bitmap still points at its buffers
                    c.swap.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(SWAP_FLAGS))?;
                }
                Mode::Layered(l) => *l = Layered::new(gfx, l.hwnd, w, h)?,
            }
        }
        self.w = w;
        self.h = h;
        Ok(())
    }

    /// Draws one frame: target = the back buffer, cleared to `clear`, transform = `scale`; `f` paints. Does not present.
    pub fn draw(&mut self, gfx: &Gfx, scale: f32, clear: D2D1_COLOR_F, f: impl FnOnce()) -> Result<()> {
        unsafe {
            let bmp: ID2D1Bitmap1 = match &self.mode {
                Mode::Comp(c) => {
                    let surface: IDXGISurface = c.swap.GetBuffer(0)?;
                    let props = bmp_props(D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_ALPHA_MODE_PREMULTIPLIED);
                    gfx.dc.CreateBitmapFromDxgiSurface(&surface, Some(&props))?
                }
                Mode::Layered(l) => l.tgt.clone(),
            };
            gfx.dc.SetTarget(&bmp);
            gfx.dc.BeginDraw();
            gfx.dc.Clear(Some(&clear));
            gfx.dc.SetTransform(&mat(scale, 0.0, 0.0));
            f();
            gfx.dc.SetTransform(&mat(1.0, 0.0, 0.0));
            let r = gfx.dc.EndDraw(None, None);
            gfx.dc.SetTarget(None::<&ID2D1Image>);
            r
        }
    }

    /// Hands the drawn frame to the screen. Composition: `Present(1)` (vsync). Layered: readback + `UpdateLayeredWindow`.
    pub fn present(&mut self, gfx: &Gfx) -> PresentStatus {
        unsafe {
            match &mut self.mode {
                Mode::Comp(c) => {
                    let hr = c.swap.Present(1, DXGI_PRESENT(0));
                    if hr.0 == STATUS_OCCLUDED {
                        PresentStatus::Occluded
                    } else if hr.is_ok() {
                        PresentStatus::Ok
                    } else if hr == DXGI_ERROR_DEVICE_REMOVED || hr == DXGI_ERROR_DEVICE_RESET || hr == DXGI_ERROR_DEVICE_HUNG {
                        PresentStatus::DeviceLost
                    } else {
                        eprintln!("ui host: Present failed: {hr:?}");
                        PresentStatus::Failed
                    }
                }
                Mode::Layered(l) => match l.present(gfx, self.w, self.h) {
                    Ok(()) => PresentStatus::Ok,
                    Err(e) if device_lost(&e) => PresentStatus::DeviceLost,
                    Err(e) => {
                        eprintln!("ui host: layered present failed: {e}");
                        PresentStatus::Failed
                    }
                },
            }
        }
    }

    /// Whether an error from `draw` / `resize` means the device is gone.
    pub fn is_device_lost(e: &windows::core::Error) -> bool {
        device_lost(e)
    }
}

impl Layered {
    fn new(gfx: &Gfx, hwnd: HWND, w: u32, h: u32) -> Result<Layered> {
        unsafe {
            let screen = GetDC(None);
            let mem_dc = CreateCompatibleDC(screen);
            ReleaseDC(None, screen);
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32), // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = match CreateDIBSection(mem_dc, &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(d) => d,
                Err(e) => {
                    let _ = DeleteDC(mem_dc);
                    return Err(e);
                }
            };
            SelectObject(mem_dc, dib);
            let tgt = gfx.target(w, h)?;
            let cpu = gfx.cpu_read(w, h)?;
            Ok(Layered { hwnd, mem_dc, dib, bits: bits as *mut u8, tgt, cpu })
        }
    }

    fn present(&mut self, gfx: &Gfx, w: u32, h: u32) -> Result<()> {
        unsafe {
            let (bits, stride) = (self.bits, w as usize * 4);
            gfx.read_rect(&self.tgt, &self.cpu, RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 }, |y, row| {
                std::ptr::copy_nonoverlapping(row.as_ptr(), bits.add(y as usize * stride), row.len());
            })?;
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let (src, size) = (POINT { x: 0, y: 0 }, SIZE { cx: w as i32, cy: h as i32 });
            // position stays as the window has it (null destination point), the size follows the bitmap
            UpdateLayeredWindow(self.hwnd, None, None, Some(&size), self.mem_dc, Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA)
        }
    }
}

impl Drop for Layered {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.dib);
            let _ = DeleteDC(self.mem_dc);
        }
    }
}
