//! The marked-up PNG of a commit: the crop of the frozen frame under the selection with every markup op drawn on top, at frame resolution.
//!
//! Done off the UI thread on a Direct2D device of its own (software WARP: no contention with the UI thread's GPU device): upload the crop, run the SAME `Tools::draw_cmd`
//! the overlay uses, read the pixels back, encode with WIC into a memory stream. The bytes go to `capture_commit` as the raw body
//! (`x-pill-args` = the JSON arguments), exactly what `capture.js` did with `canvas.toBlob`.
use super::io::{RawJob, RawSink};
use super::tools::{crop_rgba_to_bgra, FrameSrc, Tools};
use crate::ui::view::Waker;
use glasskit::canvas::translation;
use glasskit::gfx::{col, rf, Gfx};
use pill_ui_core::capture::io::FlattenRequest;
use std::sync::Arc;
use windows::core::Result;
use windows::Win32::Foundation::{HGLOBAL, RECT};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
use windows::Win32::System::Com::{CoInitializeEx, IStream, COINIT_MULTITHREADED, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};

/// Posted to the view when a flatten worker finished: `true` = the PNG was sent, `false` = it could not be made (the plain commit went out).
pub struct Done(pub bool);

/// Starts the worker of a markup commit. When the PNG cannot be made the commit still goes out, with an empty body: the backend then
/// crops its own copy of the frame (no markup) instead of losing the shot.
pub fn spawn_commit(job: RawJob, frame: Option<Arc<Vec<u8>>>, fsize: (i32, i32), caller: String, sink: RawSink, waker: Waker, _session: u64) {
    std::thread::spawn(move || {
        let png = frame.and_then(|f| match std::panic::catch_unwind(|| render_png(&f, fsize.0, fsize.1, &job.req)) {
            Ok(Ok(p)) => Some(p),
            Ok(Err(e)) => {
                eprintln!("capture: markup export failed: {e}");
                None
            }
            Err(_) => None,
        });
        let ok = png.is_some();
        sink(&caller, job.args, png.unwrap_or_default());
        waker.post_custom(Done(ok));
    });
}

/// The PNG for `req` on a fresh software (WARP) device.
pub fn render_png(rgba: &[u8], fw: i32, fh: i32, req: &FlattenRequest) -> Result<Vec<u8>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED); // WIC wants COM on this thread
    }
    // Always the software device: the export is a few draws over one crop, it never competes with the UI thread's GPU device, and a
    // second hardware device created on a worker next to WARP test devices hung the process (see the test below).
    let gfx = Gfx::warp()?;
    render_png_on(&gfx, rgba, fw, fh, req)
}

/// Same on a device of the caller's choice (the tests use WARP).
pub fn render_png_on(gfx: &Gfx, rgba: &[u8], fw: i32, fh: i32, req: &FlattenRequest) -> Result<Vec<u8>> {
    let (bgra, w, h) = render_bgra(gfx, rgba, fw, fh, req)?;
    encode_png(gfx, w, h, &bgra)
}

/// The flattened region as premultiplied BGRA (+ its size).
pub fn render_bgra(gfx: &Gfx, rgba: &[u8], fw: i32, fh: i32, req: &FlattenRequest) -> Result<(Vec<u8>, u32, u32)> {
    let s = req.src;
    let src = pill_ui_core::capture::IRect { x: s.x.clamp(0, fw - 1), y: s.y.clamp(0, fh - 1), w: 0, h: 0 };
    let src = pill_ui_core::capture::IRect { w: s.w.clamp(1, fw - src.x), h: s.h.clamp(1, fh - src.y), ..src };
    if rgba.len() < fw as usize * fh as usize * 4 {
        return Err(windows::core::Error::empty());
    }
    let (w, h) = (src.w as u32, src.h as u32);
    let crop = gfx.from_bgra(w, h, &crop_rgba_to_bgra(rgba, fw, src))?;
    let tools = Tools::new(gfx)?;
    let target = gfx.target(w, h)?;
    unsafe {
        gfx.dc.SetTarget(&target);
        gfx.dc.BeginDraw();
        gfx.dc.Clear(Some(&col(0.0, 0.0, 0.0, 0.0)));
        gfx.dc.SetTransform(&glasskit::gfx::mat(1.0, 0.0, 0.0));
        gfx.dc.DrawBitmap(&crop, Some(&rf(0.0, 0.0, w as f32, h as f32)), 1.0, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
        // the ops live in frame px: move the crop's origin to (0, 0)
        gfx.dc.SetTransform(&translation(-src.x as f32, -src.y as f32));
        let fs = FrameSrc { rgba, fw, fh };
        for c in &req.cmds {
            tools.draw_cmd(gfx, c, fs, 1.0);
        }
        gfx.dc.SetTransform(&glasskit::gfx::mat(1.0, 0.0, 0.0));
        let r = gfx.dc.EndDraw(None, None);
        gfx.dc.SetTarget(None::<&ID2D1Image>);
        r?;
    }
    let cpu = gfx.cpu_read(w, h)?;
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    gfx.read_rect(&target, &cpu, RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 }, |_, row| px.extend_from_slice(&row[..(w * 4) as usize]))?;
    // the frame is opaque, so premultiplied == straight; force alpha anyway (a clear colour must never leak into the export)
    for p in px.chunks_exact_mut(4) {
        p[3] = 255;
    }
    Ok((px, w, h))
}

/// BGRA -> PNG bytes through WIC (an in-memory stream).
pub fn encode_png(gfx: &Gfx, w: u32, h: u32, bgra: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let stream: IStream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
        let enc = gfx.wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut fr: Option<IWICBitmapFrameEncode> = None;
        enc.CreateNewFrame(&mut fr, std::ptr::null_mut())?;
        let fr = fr.ok_or_else(windows::core::Error::empty)?;
        fr.Initialize(None)?;
        fr.SetSize(w, h)?;
        let mut fmt = GUID_WICPixelFormat32bppBGRA;
        fr.SetPixelFormat(&mut fmt)?;
        fr.WritePixels(h, w * 4, bgra)?;
        fr.Commit()?;
        enc.Commit()?;
        let mut st = STATSTG::default();
        stream.Stat(&mut st, STATFLAG_NONAME)?;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut out = vec![0u8; st.cbSize as usize];
        let mut read = 0u32;
        stream.Read(out.as_mut_ptr() as *mut _, out.len() as u32, Some(&mut read)).ok()?;
        out.truncate(read as usize);
        Ok(out)
    }
}
