// Screen capture lane (L8): area / full-screen screenshots, screen text (OCR), QR, colour picker, screen recording.
// Owner: capture agent. Pages: capture.html + capture.js + capture.css (overlay, one window `capture-<n>` per monitor,
// created lazily and closed after 90 s idle) and capture-live.js (recording capsule + result cards in the pill window).
//
// Flow: hotkey / tile -> capture_start(mode) -> every monitor is frozen with GDI BitBlt (physical px, any DPI mix,
// negative coordinates fine) -> an overlay window per monitor shows the frozen frame (WDA_EXCLUDEFROMCAPTURE, always) ->
// the page calls capture_commit(rect, action) -> Rust crops its OWN copy of the frame (the page never supplies pixels
// except a flattened markup PNG), then copies / saves / runs OCR / decodes QR / starts ffmpeg.
// Commands: capture_start, capture_commit, capture_cancel, capture_stop, capture_settings (+ page helpers
// capture_session, capture_frame, capture_ready, capture_action).
// Events (to window "pill"): rec:state {recording, startedAt, mode, path?}, capture:done {kind, path?, text?, ...}.
// PRIVACY: nothing leaves the machine. Pixels go to the user's folder / clipboard only, OCR text only to the clipboard,
// a QR link is opened only when the user clicks "Open" on the card, the page can only open files inside the capture folders.
// Settings read through settings::get_*: capture.folder, capture.videoFolder, capture.fps, capture.encoder (auto|x264|nvenc),
// capture.ffmpeg, capture.format (mp4|gif), capture.shotAction (both|copy|save), capture.revealAfterSave, hotkey.shotArea/shotFull/ocr/qr/pipette/record.
use crate::ui::bus::{self, Target};
use crate::ui::capture_view::windows as cv_windows;
use crate::{settings, store};
use serde::Deserialize;
use serde_json::{json, Value};
use std::ffi::c_void;
use std::io::{Cursor, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Webview, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, ShellExecuteW, FOLDERID_Pictures, FOLDERID_Videos, KF_FLAG_DEFAULT};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
    SetWindowDisplayAffinity, SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNORMAL,
    WDA_EXCLUDEFROMCAPTURE,
};

const NO_WINDOW: u32 = 0x0800_0000; // CREATE_NO_WINDOW
const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const IDLE_CLOSE: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------------------------------------------
// Pure logic (unit-tested below)
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Selection rectangle from the overlay page (css px `[x, y, w, h]` inside a `vw` x `vh` viewport, w/h may be negative
/// when the user dragged up/left) -> physical px inside a frame of `fw` x `fh`. Clamped; None when it is empty (< 2 px).
/// The scale is derived per axis from the frame size, so a monitor at 150 % next to one at 100 % needs no special case.
pub fn to_frame_rect(css: [f64; 4], vw: f64, vh: f64, fw: i32, fh: i32) -> Option<Rect> {
    if vw < 1.0 || vh < 1.0 || css.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let (mut x, mut y, mut w, mut h) = (css[0], css[1], css[2], css[3]);
    if w < 0.0 {
        x += w;
        w = -w;
    }
    if h < 0.0 {
        y += h;
        h = -h;
    }
    let (sx, sy) = (fw as f64 / vw, fh as f64 / vh);
    let x0 = ((x * sx).round() as i64).clamp(0, fw as i64) as i32;
    let y0 = ((y * sy).round() as i64).clamp(0, fh as i64) as i32;
    let x1 = (((x + w) * sx).round() as i64).clamp(0, fw as i64) as i32;
    let y1 = (((y + h) * sy).round() as i64).clamp(0, fh as i64) as i32;
    (x1 - x0 >= 2 && y1 - y0 >= 2).then_some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

/// True when `mode` was already requested less than `win` ago (a hotkey that fires twice for one press); otherwise records it.
fn is_repeat<M: PartialEq + Copy>(last: &mut Option<(M, Instant)>, mode: M, now: Instant, win: Duration) -> bool {
    if last.is_some_and(|(m, t)| m == mode && now.saturating_duration_since(t) < win) {
        return true;
    }
    *last = Some((mode, now));
    false
}

/// Tracks how long the overlay has been out of the foreground; true once that lasted `limit` without a break.
fn lost_too_long(lost_since: &mut Option<Instant>, fg_ok: bool, now: Instant, limit: Duration) -> bool {
    if fg_ok {
        *lost_since = None;
        return false;
    }
    now.saturating_duration_since(*lost_since.get_or_insert(now)) >= limit
}

/// A point in css px -> frame px (clamped to the frame).
pub fn to_frame_point(x: f64, y: f64, vw: f64, vh: f64, fw: i32, fh: i32) -> Option<(i32, i32)> {
    if vw < 1.0 || vh < 1.0 || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let px = ((x * fw as f64 / vw).floor() as i64).clamp(0, fw as i64 - 1) as i32;
    let py = ((y * fh as f64 / vh).floor() as i64).clamp(0, fh as i64 - 1) as i32;
    Some((px, py))
}

/// `pill_2026-10-23_20-37-29.png`
pub fn file_name(ext: &str, t: [u16; 6]) -> String {
    format!("pill_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.{ext}", t[0], t[1], t[2], t[3], t[4], t[5])
}

/// `name.ext`, or `name_1.ext`, `name_2.ext` ... when the file already exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let (stem, ext) = (p.file_stem().and_then(|s| s.to_str()).unwrap_or("pill"), p.extension().and_then(|s| s.to_str()).unwrap_or(""));
    (1..1000)
        .map(|i| dir.join(if ext.is_empty() { format!("{stem}_{i}") } else { format!("{stem}_{i}.{ext}") }))
        .find(|c| !c.exists())
        .unwrap_or(first)
}

pub fn hex_of(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

#[cfg_attr(not(test), allow(dead_code))] // the overlay sends pixel positions, not colours; kept for the settings page / tests
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn is_http_url(s: &str) -> bool {
    let l = s.trim().to_ascii_lowercase();
    (l.starts_with("http://") || l.starts_with("https://")) && l.len() < 2048 && !l.chars().any(|c| c.is_control() || c == ' ')
}

fn crop_rgba(src: &[u8], sw: i32, r: Rect) -> Vec<u8> {
    let mut out = Vec::with_capacity((r.w * r.h * 4) as usize);
    for row in r.y..r.y + r.h {
        let s = ((row * sw + r.x) * 4) as usize;
        out.extend_from_slice(&src[s..s + (r.w * 4) as usize]);
    }
    out
}

fn pixel(rgba: &[u8], w: i32, x: i32, y: i32) -> [u8; 3] {
    let o = ((y * w + x) * 4) as usize;
    [rgba[o], rgba[o + 1], rgba[o + 2]]
}

fn luma(p: &[u8]) -> u8 {
    ((p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8) as u8
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Enc {
    X264,
    Nvenc,
}

pub struct RecOpts {
    pub rect: Rect, // absolute virtual-desktop physical px
    pub fps: u32,
    pub enc: Enc,
    pub out: String,
}

/// gdigrab -> H.264 MP4. Even size (yuv420p), fragmented MP4 so a killed ffmpeg still leaves a playable file.
pub fn ffmpeg_args(o: &RecOpts) -> Vec<String> {
    let (w, h) = ((o.rect.w & !1).max(16), (o.rect.h & !1).max(16));
    let fps = o.fps.clamp(5, 60);
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-f", "gdigrab", "-framerate"].iter().map(|s| s.to_string()).collect();
    a.extend([fps.to_string(), "-draw_mouse".into(), "1".into(), "-offset_x".into(), o.rect.x.to_string(), "-offset_y".into(), o.rect.y.to_string()]);
    a.extend(["-video_size".into(), format!("{w}x{h}"), "-i".into(), "desktop".into()]);
    a.extend(encoder_args(o.enc));
    a.extend(["-g".into(), (fps * 2).to_string(), "-movflags".into(), "+frag_keyframe+empty_moov+default_base_moof".into(), o.out.clone()]);
    a
}

fn encoder_args(e: Enc) -> Vec<String> {
    let v: &[&str] = match e {
        Enc::X264 => &["-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p"],
        Enc::Nvenc => &["-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr", "-cq", "23", "-b:v", "0", "-pix_fmt", "yuv420p"],
    };
    v.iter().map(|s| s.to_string()).collect()
}

/// RGBA -> PNG bytes (fast compression: a 4K frame must not take seconds).
fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(Cursor::new(&mut out), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.set_compression(png::Compression::Fast);
    e.write_header().ok()?.write_image_data(rgba).ok()?;
    Some(out)
}

/// PNG bytes (RGB or RGBA, 8 bit; what a canvas produces) -> (w, h, RGBA).
fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.width == 0 || info.height == 0 || info.width > 16384 || info.height > 16384 {
        return None;
    }
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some((info.width, info.height, rgba))
}

/// CF_DIB payload: BITMAPINFOHEADER + bottom-up 32 bpp BI_RGB rows (alpha forced to 255).
fn dib_bytes(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut d = vec![0u8; 40];
    d[0] = 40;
    d[4..8].copy_from_slice(&w.to_le_bytes());
    d[8..12].copy_from_slice(&h.to_le_bytes());
    d[12] = 1;
    d[14] = 32;
    d.reserve(rgba.len());
    for row in rgba.chunks_exact(w as usize * 4).rev() {
        d.extend(row.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]));
    }
    d
}

fn utf16_bytes(s: &str, nul2: bool) -> Vec<u8> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    if nul2 {
        v.push(0);
    }
    v.iter().flat_map(|u| u.to_le_bytes()).collect()
}

/// CF_HDROP payload for one file (DROPFILES header, wide chars).
fn hdrop_bytes(path: &str) -> Vec<u8> {
    let mut d = vec![0u8; 20];
    d[0] = 20;
    d[16] = 1;
    d.extend(utf16_bytes(path, true));
    d
}

/// Decode a QR code from an RGBA image (pure Rust, rqrr). Retries inverted (light-on-dark codes in dark themes).
pub fn decode_qr(w: usize, h: usize, rgba: &[u8]) -> Option<String> {
    if w < 21 || h < 21 || rgba.len() < w * h * 4 {
        return None;
    }
    let gray: Vec<u8> = rgba.chunks_exact(4).take(w * h).map(luma).collect();
    for invert in [false, true] {
        let mut img = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| {
            let v = gray[y * w + x];
            if invert {
                255 - v
            } else {
                v
            }
        });
        for g in img.detect_grids() {
            if let Ok((_meta, text)) = g.decode() {
                if !text.is_empty() {
                    return Some(text);
                }
            }
        }
    }
    None
}

/// Light text on a dark background reads badly in Windows OCR: invert when the area is mostly dark.
fn maybe_invert(rgba: &mut [u8]) {
    let n = (rgba.len() / 4).max(1);
    let sum: u64 = rgba.chunks_exact(4).step_by(7).map(|p| luma(p) as u64).sum();
    let mean = sum / ((n / 7).max(1) as u64);
    if mean < 110 {
        for p in rgba.chunks_exact_mut(4) {
            p[0] = 255 - p[0];
            p[1] = 255 - p[1];
            p[2] = 255 - p[2];
        }
    }
}

fn upscale2(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4 * 4];
    for y in 0..h * 2 {
        for x in 0..w * 2 {
            let (s, o) = (((y / 2) * w + x / 2) * 4, (y * w * 2 + x) * 4);
            out[o..o + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Windows glue
// ---------------------------------------------------------------------------------------------------------------

/// Windows.Media.Ocr on an RGBA image. The engine comes from the user's profile languages, English as the fallback; no network.
/// `lang`: "" / "auto" = the user's profile languages, otherwise a BCP-47 tag such as "en-US" or "ru" (setting capture.ocrLang).
pub fn ocr(w: u32, h: u32, rgba: &[u8], lang: &str) -> Result<String, String> {
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;
    let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let wanted = lang.trim();
    let explicit = (!wanted.is_empty() && !wanted.eq_ignore_ascii_case("auto"))
        .then(|| Language::CreateLanguage(&HSTRING::from(wanted)).ok().and_then(|l| OcrEngine::TryCreateFromLanguage(&l).ok()))
        .flatten();
    let engine = explicit
        .or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok())
        .or_else(|| {
            let l = Language::CreateLanguage(&HSTRING::from("en-US")).ok()?;
            OcrEngine::TryCreateFromLanguage(&l).ok()
        })
        .ok_or_else(|| "no OCR language pack installed (Settings > Time & language > Language > add a language with text recognition)".to_string())?;
    let (mut w, mut h) = (w as usize, h as usize);
    let mut px = rgba.to_vec();
    maybe_invert(&mut px);
    let max = OcrEngine::MaxImageDimension().map_err(e("max size"))? as usize;
    if h < 40 && w * 2 <= max && h * 2 <= max {
        px = upscale2(&px, w, h);
        w *= 2;
        h *= 2;
    }
    if w > max || h > max {
        return Err(format!("area too large for text recognition (max {max} px)"));
    }
    let bgra: Vec<u8> = px.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
    let wr = DataWriter::new().map_err(e("writer"))?;
    wr.WriteBytes(&bgra).map_err(e("write"))?;
    let buf = wr.DetachBuffer().map_err(e("buffer"))?;
    let bmp = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(&buf, BitmapPixelFormat::Bgra8, w as i32, h as i32, BitmapAlphaMode::Ignore)
        .map_err(e("bitmap"))?;
    let res = engine.RecognizeAsync(&bmp).map_err(e("recognize"))?.get().map_err(e("recognize"))?;
    let mut lines = Vec::new();
    for l in res.Lines().map_err(e("lines"))? {
        lines.push(l.Text().map_err(e("text"))?.to_string());
    }
    Ok(lines.join("\n").trim().to_string())
}

/// Freeze one monitor: GDI BitBlt of the virtual-desktop rectangle (physical px) -> RGBA.
fn grab(m: &Mon) -> Option<Vec<u8>> {
    let (w, h) = (m.w, m.h);
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(HWND::default());
        let dc = CreateCompatibleDC(screen);
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let out = match CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0) {
            Ok(bmp) if !bits.is_null() => {
                let old = SelectObject(dc, HGDIOBJ(bmp.0));
                let ok = BitBlt(dc, 0, 0, w, h, screen, m.x, m.y, SRCCOPY | CAPTUREBLT).is_ok();
                let px = if ok {
                    let src = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                    Some(src.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect::<Vec<u8>>())
                } else {
                    None
                };
                SelectObject(dc, old);
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                px
            }
            _ => None,
        };
        let _ = DeleteDC(dc);
        ReleaseDC(HWND::default(), screen);
        out
    }
}

unsafe fn set_global(fmt: u32, data: &[u8]) -> bool {
    let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) else { return false };
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        return false;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
    let _ = GlobalUnlock(h);
    SetClipboardData(fmt, HANDLE(HGLOBAL(h.0).0)).is_ok() // on success the system owns the memory
}

/// Replace the clipboard content with these formats. Returns false when it could not be opened or nothing was stored.
fn clip_put(items: &[(u32, Vec<u8>)]) -> bool {
    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(HWND::default()).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        if !opened {
            return false;
        }
        let _ = EmptyClipboard();
        let mut any = false;
        for (fmt, data) in items {
            any |= set_global(*fmt, data);
        }
        let _ = CloseClipboard();
        any
    }
}

fn clip_image(w: u32, h: u32, rgba: &[u8], png: &[u8]) -> bool {
    let png_fmt = unsafe { RegisterClipboardFormatW(w!("PNG")) };
    clip_put(&[(CF_DIB, dib_bytes(w, h, rgba)), (png_fmt, png.to_vec())])
}

fn clip_text(s: &str) -> bool {
    clip_put(&[(CF_UNICODETEXT, utf16_bytes(s, false))])
}

fn clip_file(path: &str) -> bool {
    clip_put(&[(CF_HDROP, hdrop_bytes(path))])
}

fn local_time() -> [u16; 6] {
    let t = unsafe { GetLocalTime() };
    [t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond]
}

fn known_folder(id: &windows::core::GUID, fallback: &str) -> PathBuf {
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, HANDLE::default()) {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            if let Some(s) = s.filter(|s| !s.is_empty()) {
                return PathBuf::from(s);
            }
        }
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join(fallback)
}

fn shot_dir() -> PathBuf {
    let f = settings::get_str("capture.folder", "");
    if f.trim().is_empty() {
        known_folder(&FOLDERID_Pictures, "Pictures").join("Screenshots").join("Pill")
    } else {
        PathBuf::from(f.trim())
    }
}

fn video_dir() -> PathBuf {
    let f = settings::get_str("capture.videoFolder", "");
    if f.trim().is_empty() {
        known_folder(&FOLDERID_Videos, "Videos").join("Pill")
    } else {
        PathBuf::from(f.trim())
    }
}

fn shell_open(target: &str) -> bool {
    let wide: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
    let r = unsafe { ShellExecuteW(HWND::default(), w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    r.0 as isize > 32
}

fn reveal(path: &Path) {
    let _ = Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
}

/// A path the page may ask us to open: it must exist and live inside one of the capture folders.
fn allowed_path(p: &str) -> Option<PathBuf> {
    let c = std::fs::canonicalize(p).ok()?;
    [shot_dir(), video_dir()].iter().filter_map(|d| std::fs::canonicalize(d).ok()).any(|d| c.starts_with(&d)).then_some(c)
}

fn plain(p: &Path) -> String {
    let s = p.display().to_string();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string() // canonicalize adds the verbatim prefix; the clipboard and Explorer do not want it
}

unsafe fn foreground(hwnd: HWND) {
    let fg = GetForegroundWindow();
    let cur = GetCurrentThreadId();
    let ft = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
    if ft != 0 && ft != cur {
        let _ = AttachThreadInput(ft, cur, true);
    }
    let _ = SetForegroundWindow(hwnd);
    let _ = BringWindowToTop(hwnd);
    if ft != 0 && ft != cur {
        let _ = AttachThreadInput(ft, cur, false);
    }
}

fn win_hwnd(w: &WebviewWindow) -> Option<HWND> {
    w.hwnd().ok().map(|h| HWND(h.0 as isize as *mut c_void))
}

fn now_ms() -> i64 {
    store::now_ms()
}

// ---------------------------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Mon {
    label: String,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scale: f64,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Area,
    Screen,
    Text,
    Qr,
    Color,
    Record,
    RecordScreen,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "area" => Mode::Area,
            "screen" => Mode::Screen,
            "text" => Mode::Text,
            "qr" => Mode::Qr,
            "color" | "picker" => Mode::Color,
            "record" => Mode::Record,
            "record_screen" => Mode::RecordScreen,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Mode::Area => "area",
            Mode::Screen => "screen",
            Mode::Text => "text",
            Mode::Qr => "qr",
            Mode::Color => "color",
            Mode::Record => "record",
            Mode::RecordScreen => "record_screen",
        }
    }
}

struct Shot {
    mon: Mon,
    rgba: Arc<Vec<u8>>,
    ready: bool,
}

struct Session {
    id: u64,
    mode: Mode,
    focus: String,
    shots: Vec<Shot>,
}

struct Rec {
    id: u64,
    child: Child,
    ffmpeg: PathBuf,
    gif: bool, // capture.format = gif: the finished MP4 is converted to a GIF in finalize()
    path: PathBuf,
    started: i64,
    job: isize,
    err: Arc<Mutex<String>>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static REC: Mutex<Option<Rec>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);
static LAST_END: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_START: Mutex<Option<(Mode, Instant)>> = Mutex::new(None);
static BEGIN: Mutex<()> = Mutex::new(());
static NVENC: OnceLock<bool> = OnceLock::new();

fn session() -> MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}
fn rec_slot() -> MutexGuard<'static, Option<Rec>> {
    REC.lock().unwrap_or_else(|e| e.into_inner())
}

fn emit(app: &AppHandle, ev: &str, payload: Value) {
    let _ = bus::emit(&app, Target::Label("pill"), ev, payload);
}

fn fail(app: &AppHandle, msg: impl Into<String>) {
    let msg = msg.into();
    eprintln!("capture: {msg}");
    emit(app, "capture:done", json!({ "kind": "error", "message": msg }));
}

fn monitors(app: &AppHandle) -> Vec<Mon> {
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, m)| Mon {
            label: format!("capture-{i}"),
            x: m.position().x,
            y: m.position().y,
            w: m.size().width as i32,
            h: m.size().height as i32,
            scale: m.scale_factor(),
        })
        .collect()
}

fn cursor_mon(app: &AppHandle, mons: &[Mon]) -> usize {
    let Ok(p) = app.cursor_position() else { return 0 };
    let (px, py) = (p.x as i32, p.y as i32);
    mons.iter().position(|m| px >= m.x && px < m.x + m.w && py >= m.y && py < m.y + m.h).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------------------------
// Overlay windows
// ---------------------------------------------------------------------------------------------------------------

fn ensure_window(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    let w = WebviewWindowBuilder::new(app, label, WebviewUrl::App("capture.html".into()))
        .title("pill-capture")
        .decorations(false)
        .transparent(false)
        .background_color(tauri::window::Color(0, 0, 0, 255)) // no white flash before the first paint
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .visible(false)
        .focused(false)
        .inner_size(400.0, 300.0)
        .build()
        .map_err(|e| eprintln!("capture: cannot create {label}: {e}"))
        .ok()?;
    if let Some(h) = win_hwnd(&w) {
        unsafe {
            // The overlay itself is never part of any capture or recording, whatever showInCapture says.
            let _ = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE);
        }
    }
    Some(w)
}

fn place(w: &WebviewWindow, m: &Mon, show: bool) {
    if let Some(h) = win_hwnd(w) {
        let flags = if show { SWP_NOACTIVATE | SWP_SHOWWINDOW } else { SWP_NOACTIVATE };
        unsafe {
            let _ = SetWindowPos(h, HWND_TOPMOST, m.x, m.y, m.w, m.h, flags);
        }
    }
}

/// The native overlay path is on: setting `ui.native.capture` and no `--web-ui` override (see ui/capture_view/windows.rs).
fn native_on() -> bool {
    cv_windows::enabled()
}

/// Native counterpart of the WebView branch of `open_overlays`: one host window per monitor; reused ones are re-armed with one
/// `capture:begin` (every native view listens to the bus, and each arms itself for its own label), new ones arm themselves at creation.
fn open_native(app: &AppHandle, id: u64) {
    let list: Vec<Mon> = session().as_ref().filter(|s| s.id == id).map(|s| s.shots.iter().map(|x| x.mon.clone()).collect()).unwrap_or_default();
    let mut reused = false;
    for m in &list {
        reused |= cv_windows::open(app, &m.label, m.x, m.y, m.w, m.h, m.scale);
    }
    if reused {
        let _ = bus::emit(app, Target::Label(list.first().map_or("capture-0", |m| m.label.as_str())), "capture:begin", json!({ "id": id }));
    }
}

/// Windows (re)positioned hidden; each page reports `capture_ready` once its frame is painted, and only then is it shown.
/// With the native overlay on (`ui.native.capture`, not `--web-ui`) the windows are host `Fullscreen` windows instead (ui/capture_view).
fn open_overlays(app: &AppHandle, id: u64) {
    if native_on() {
        open_native(app, id);
    } else {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || {
            let list: Vec<Mon> = session().as_ref().filter(|s| s.id == id).map(|s| s.shots.iter().map(|x| x.mon.clone()).collect()).unwrap_or_default();
            for m in &list {
                let existed = a.get_webview_window(&m.label).is_some();
                if let Some(w) = ensure_window(&a, &m.label) {
                    place(&w, m, false);
                    if existed {
                        let _ = bus::emit(&a, Target::Label(m.label.as_str()), "capture:begin", json!({ "id": id }));
                    }
                }
            }
        });
    }
    // Watchdog: a page that never reports (script error, WebView2 missing) must not leave the session half open.
    let a = app.clone();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut lost = None;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let (alive, shown) = match session().as_ref() {
                Some(s) if s.id == id => (true, s.shots.iter().any(|x| x.ready)),
                _ => (false, false),
            };
            if !alive {
                return;
            }
            if !shown && t0.elapsed() >= Duration::from_secs(6) {
                end_session(&a);
                fail(&a, "the capture overlay did not start");
                return;
            }
            // Safety: a topmost full-screen overlay that is no longer the foreground window (focus stolen, alt-tab, keyboard
            // never arrived) is useless and traps the screen: end it after 1.5 s. Failsafe: 600 s at most.
            let fg = unsafe { GetForegroundWindow() };
            let fg_ok = !shown || if native_on() { cv_windows::foreground_ok() } else { a.webview_windows().iter().any(|(l, w)| l.starts_with("capture-") && win_hwnd(w) == Some(fg)) };
            if lost_too_long(&mut lost, fg_ok, Instant::now(), Duration::from_millis(1500)) || t0.elapsed() >= Duration::from_secs(600) {
                end_session(&a);
                return;
            }
        }
    });
}

fn hide_overlays(app: &AppHandle) {
    if native_on() {
        // Native overlays hide themselves when they get `capture:end` (after their exit animations); the 700 ms backstop below hides
        // whatever did not, unless a new session has started meanwhile.
        if let Some(l) = cv_windows::labels().first() {
            let _ = bus::emit(&app, Target::Label(l.as_str()), "capture:end", json!({}));
        }
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(700));
            if session().is_none() {
                cv_windows::hide_all();
            }
        });
    } else {
        for (label, w) in app.webview_windows() {
            if label.starts_with("capture-") {
                let _ = w.hide();
                // The overlay is shown with SetWindowPos, so the toolkit's own "visible" flag is stale and hide() alone does nothing.
                if let Some(h) = win_hwnd(&w) {
                    unsafe {
                        let _ = ShowWindow(h, SW_HIDE);
                    }
                }
                let _ = bus::emit(&app, Target::Label(label.as_str()), "capture:end", json!({}));
            }
        }
    }
    *LAST_END.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    // Close the windows after a while without use: idle RAM stays low.
    let a = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(IDLE_CLOSE + Duration::from_secs(1));
        let idle = LAST_END.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() >= IDLE_CLOSE);
        if idle && session().is_none() {
            if native_on() {
                cv_windows::close_all();
            }
            for (label, w) in a.webview_windows() {
                if label.starts_with("capture-") {
                    let _ = w.destroy();
                }
            }
        }
    });
}

fn end_session(app: &AppHandle) -> Option<Session> {
    let s = session().take();
    hide_overlays(app);
    s
}

// ---------------------------------------------------------------------------------------------------------------
// Flow
// ---------------------------------------------------------------------------------------------------------------

pub fn start(app: &AppHandle, mode: &str) {
    let Some(mode) = Mode::parse(mode) else {
        eprintln!("capture: unknown mode '{mode}'");
        return;
    };
    // One physical press must act once, whatever fires twice (plugin + fallback, key repeat, a synthetic double event).
    if is_repeat(&mut LAST_START.lock().unwrap_or_else(|e| e.into_inner()), mode, Instant::now(), Duration::from_millis(300)) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || begin(&app, mode));
}

fn begin(app: &AppHandle, mode: Mode) {
    let _one_at_a_time = BEGIN.lock().unwrap_or_else(|e| e.into_inner()); // two begin() calls must not interleave (both seeing "no session")
    if matches!(mode, Mode::Record | Mode::RecordScreen) && rec_slot().is_some() {
        stop(app); // the record hotkey / tile is a toggle
        return;
    }
    let was = session().as_ref().map(|s| s.mode);
    if was.is_some() {
        end_session(app);
        if was == Some(mode) {
            return; // same hotkey again closes the overlay
        }
    }
    if matches!(mode, Mode::Area | Mode::Screen) {
        // The user's own screenshot tool does the job; the built-in overlay below is the fallback when it is not installed.
        if let Some(tool) = find_flameshot() {
            let args: &[&str] = if matches!(mode, Mode::Area) { &["gui"] } else { &["full", "-c"] };
            let started = Command::new(tool).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(NO_WINDOW).spawn();
            if started.is_ok() {
                return;
            }
        }
    }
    let mons = monitors(app);
    if mons.is_empty() {
        return fail(app, "no monitor found");
    }
    let cur = cursor_mon(app, &mons);
    match mode {
        Mode::Screen => match grab(&mons[cur]) {
            Some(rgba) => finish_image(app, mons[cur].w as u32, mons[cur].h as u32, rgba, None, "both"),
            None => fail(app, "could not read the screen"),
        },
        Mode::RecordScreen => {
            let m = &mons[cur];
            start_recording(app, Rect { x: m.x, y: m.y, w: m.w, h: m.h }, "screen");
        }
        _ => {
            let mut shots = Vec::new();
            for m in &mons {
                match grab(m) {
                    Some(px) => shots.push(Shot { mon: m.clone(), rgba: Arc::new(px), ready: false }),
                    None => return fail(app, "could not read the screen"),
                }
            }
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            *session() = Some(Session { id, mode, focus: mons[cur].label.clone(), shots });
            open_overlays(app, id);
        }
    }
}

/// Crop / flatten result -> clipboard and/or file, then the result card. `action`: copy | save | both.
fn finish_image(app: &AppHandle, w: u32, h: u32, rgba: Vec<u8>, png: Option<Vec<u8>>, action: &str) {
    let Some(png) = png.or_else(|| png_bytes(w, h, &rgba)) else { return fail(app, "could not encode the image") };
    // "both" = Enter / double click / full-screen shot: capture.shotAction decides what that means (both | copy | save)
    let action = match (action, settings::get_str("capture.shotAction", "both").as_str()) {
        ("both", "copy") => "copy",
        ("both", "save") => "save",
        (a, _) => a,
    };
    let (copy, save) = (action != "save", action != "copy");
    let mut path = None;
    if save {
        let dir = shot_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return fail(app, format!("cannot create {}: {e}", dir.display()));
        }
        let p = unique_path(&dir, &file_name("png", local_time()));
        if let Err(e) = std::fs::write(&p, &png) {
            return fail(app, format!("cannot write {}: {e}", p.display()));
        }
        path = Some(plain(&p));
        if settings::get_bool("capture.revealAfterSave", false) {
            reveal(&p);
        }
    }
    let copied = copy && clip_image(w, h, &rgba, &png);
    emit(app, "capture:done", json!({ "kind": "screenshot", "path": path, "copied": copied, "w": w, "h": h }));
}

fn finish_text(app: &AppHandle, w: u32, h: u32, rgba: Vec<u8>) {
    match ocr(w, h, &rgba, &settings::get_str("capture.ocrLang", "auto")) {
        Ok(t) if t.is_empty() => emit(app, "capture:done", json!({ "kind": "text", "found": false })),
        Ok(t) => {
            let copied = clip_text(&t);
            let head: String = t.chars().take(400).collect();
            emit(app, "capture:done", json!({ "kind": "text", "found": true, "text": head, "chars": t.chars().count(), "copied": copied }));
        }
        Err(e) => fail(app, format!("text recognition failed: {e}")),
    }
}

fn finish_qr(app: &AppHandle, w: u32, h: u32, rgba: Vec<u8>) {
    match decode_qr(w as usize, h as usize, &rgba) {
        Some(t) => {
            let head: String = t.chars().take(1000).collect();
            emit(app, "capture:done", json!({ "kind": "qr", "found": true, "text": head, "isUrl": is_http_url(&t) }));
        }
        None => emit(app, "capture:done", json!({ "kind": "qr", "found": false })),
    }
}

fn color_history() -> Vec<String> {
    store::with(|c| {
        let mut st = c.prepare("select hex from color_history group by hex order by max(id) desc limit 10")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
    })
    .unwrap_or_default()
}

fn finish_color(app: &AppHandle, rgb: [u8; 3]) {
    let hex = hex_of(rgb[0], rgb[1], rgb[2]);
    store::with(|c| {
        c.execute("insert into color_history(hex, picked_at) values (?1, ?2)", rusqlite::params![hex, now_ms()])?;
        c.execute("delete from color_history where id not in (select id from color_history order by id desc limit 200)", [])
    });
    let copied = clip_text(&hex);
    emit(app, "capture:done", json!({ "kind": "color", "hex": hex, "rgb": rgb, "copied": copied, "history": color_history() }));
}

// ---------------------------------------------------------------------------------------------------------------
// Recording (ffmpeg gdigrab)
// ---------------------------------------------------------------------------------------------------------------

/// The installed Flameshot (setting `capture.flameshot` wins, then the default install folder).
fn find_flameshot() -> Option<PathBuf> {
    let set = settings::get_str("capture.flameshot", "");
    let default = PathBuf::from(r"C:\Program Files\Flameshot\bin\flameshot.exe");
    [PathBuf::from(set.trim()), default].into_iter().find(|p| p.is_file())
}

fn find_ffmpeg() -> Option<PathBuf> {
    let set = settings::get_str("capture.ffmpeg", "");
    if !set.trim().is_empty() && Path::new(set.trim()).is_file() {
        return Some(PathBuf::from(set.trim()));
    }
    let mut found: Option<PathBuf> = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("ffmpeg.exe"))
        .find(|p| p.is_file());
    // A chocolatey shim forwards stdin and cannot be put under our job object: use the real binary behind it.
    if let Some(p) = &found {
        let s = p.display().to_string().to_ascii_lowercase();
        if s.contains("chocolatey") && s.contains(r"\bin\") {
            if let Some(real) = p.parent().and_then(|b| b.parent()).map(|root| root.join("lib")).and_then(|lib| real_in_choco_lib(&lib)) {
                found = Some(real);
            }
        }
    }
    if found.is_none() {
        let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default();
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
        found = [
            PathBuf::from(r"C:\ffmpeg\bin\ffmpeg.exe"),
            pf.join(r"ffmpeg\bin\ffmpeg.exe"),
            home.join(r"scoop\shims\ffmpeg.exe"),
            local.join(r"Microsoft\WinGet\Links\ffmpeg.exe"),
            PathBuf::from(r"C:\ProgramData\chocolatey\bin\ffmpeg.exe"),
        ]
        .into_iter()
        .find(|p| p.is_file());
    }
    found
}

fn real_in_choco_lib(lib: &Path) -> Option<PathBuf> {
    for pkg in std::fs::read_dir(lib).ok()?.flatten() {
        if !pkg.file_name().to_string_lossy().to_ascii_lowercase().starts_with("ffmpeg") {
            continue;
        }
        let tools = pkg.path().join("tools");
        let mut stack = vec![(tools, 0u8)];
        while let Some((d, depth)) = stack.pop() {
            for e in std::fs::read_dir(&d).ok()?.flatten() {
                let p = e.path();
                if p.is_dir() && depth < 3 {
                    stack.push((p, depth + 1));
                } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("ffmpeg.exe")) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Does `h264_nvenc` really work on this PC (a card + driver, not just an entry in the encoder list)? Probed once.
fn nvenc_works(ffmpeg: &Path) -> bool {
    *NVENC.get_or_init(|| {
        let mut a: Vec<String> =
            ["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=256x256:r=10:d=0.3"].iter().map(|s| s.to_string()).collect();
        a.extend(encoder_args(Enc::Nvenc));
        a.extend(["-f", "null", "-"].iter().map(|s| s.to_string()));
        Command::new(ffmpeg).args(a).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(NO_WINDOW).status().is_ok_and(|s| s.success())
    })
}

fn pick_encoder(ffmpeg: &Path) -> Enc {
    match settings::get_str("capture.encoder", "auto").as_str() {
        "x264" => Enc::X264,
        "nvenc" => Enc::Nvenc,
        _ => {
            if nvenc_works(ffmpeg) {
                Enc::Nvenc
            } else {
                Enc::X264
            }
        }
    }
}

/// Kill-on-close job: if the pill dies, ffmpeg dies with it instead of recording forever.
pub(crate) fn job_for(child: &Child) -> isize {
    unsafe {
        let Ok(job) = CreateJobObjectW(None, PCWSTR::null()) else { return 0 };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        let _ = AssignProcessToJobObject(job, HANDLE(child.as_raw_handle()));
        job.0 as isize
    }
}

fn start_recording(app: &AppHandle, rect: Rect, kind: &str) {
    let Some(ffmpeg) = find_ffmpeg() else {
        return fail(
            app,
            "ffmpeg was not found: install it (for example `choco install ffmpeg`) or set capture.ffmpeg to the path of ffmpeg.exe in settings.json. Nothing is downloaded automatically.",
        );
    };
    let dir = video_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(app, format!("cannot create {}: {e}", dir.display()));
    }
    let path = unique_path(&dir, &file_name("mp4", local_time()));
    let gif = settings::get_str("capture.format", "mp4") == "gif";
    // a GIF is made at 15 fps anyway: recording faster only costs CPU
    let fps = if gif { 15 } else { settings::get_f64("capture.fps", 30.0).round() as u32 };
    let opts = RecOpts { rect, fps, enc: pick_encoder(&ffmpeg), out: path.display().to_string() };
    let mut child = match Command::new(&ffmpeg)
        .args(ffmpeg_args(&opts))
        .env("__COMPAT_LAYER", "HighDpiAware") // gdigrab must see physical pixels on scaled displays
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return fail(app, format!("cannot start ffmpeg ({}): {e}", ffmpeg.display())),
    };
    let err = Arc::new(Mutex::new(String::new()));
    if let Some(mut se) = child.stderr.take() {
        let e2 = err.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 1024];
            while let Ok(n) = se.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut g = e2.lock().unwrap_or_else(|e| e.into_inner());
                if g.len() < 2000 {
                    g.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
        });
    }
    // Give it a moment: a bad option or a missing desktop dies immediately, and that must be a message, not silence.
    std::thread::sleep(Duration::from_millis(700));
    if let Ok(Some(st)) = child.try_wait() {
        std::thread::sleep(Duration::from_millis(100));
        let tail = err.lock().unwrap_or_else(|e| e.into_inner()).trim().to_string();
        return fail(app, format!("ffmpeg stopped right away ({st}): {tail}"));
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let started = now_ms();
    let job = job_for(&child);
    *rec_slot() = Some(Rec { id, child, ffmpeg: ffmpeg.clone(), gif, path, started, job, err });
    emit(app, "rec:state", json!({ "recording": true, "startedAt": started, "mode": kind, "fps": opts.fps }));
    // Watch for ffmpeg ending by itself (disk full, display change).
    let a = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut g = rec_slot();
        match g.as_mut() {
            Some(r) if r.id == id => {
                if r.gif && now_ms() - r.started >= GIF_MAX_MS {
                    drop(g);
                    stop(&a); // a GIF is for short clips: it ends by itself after a minute
                    break;
                }
                if matches!(r.child.try_wait(), Ok(Some(_))) {
                    let r = g.take().unwrap();
                    drop(g);
                    finalize(&a, r, false);
                    break;
                }
            }
            _ => break,
        }
    });
}

pub fn stop(app: &AppHandle) {
    let Some(r) = rec_slot().take() else { return };
    let a = app.clone();
    std::thread::spawn(move || finalize(&a, r, true));
}

const GIF_MAX_MS: i64 = 60_000;

/// capture.format = gif: the finished MP4 becomes `<name>.gif` in two passes (palette, then dithered frames: small file, clean
/// colours; one pass would hold the whole clip in memory) and the MP4 is deleted. On any failure the MP4 stays.
fn mp4_to_gif(ffmpeg: &Path, mp4: &Path) -> Result<PathBuf, String> {
    let (gif, pal) = (mp4.with_extension("gif"), mp4.with_extension("palette.png"));
    let run = |args: Vec<String>| Command::new(ffmpeg).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(NO_WINDOW).status().is_ok_and(|s| s.success());
    let (i, p, o) = (mp4.display().to_string(), pal.display().to_string(), gif.display().to_string());
    let scale = "fps=15,scale='min(960,iw)':-2:flags=lanczos";
    let base = |extra: Vec<&str>| -> Vec<String> { ["-hide_banner", "-loglevel", "error", "-y"].iter().map(|s| s.to_string()).chain(extra.into_iter().map(String::from)).collect() };
    let one = run(base(vec!["-i", &i, "-vf", &format!("{scale},palettegen=stats_mode=diff"), &p]));
    let two = one && run(base(vec!["-i", &i, "-i", &p, "-lavfi", &format!("{scale}[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle"), "-loop", "0", &o]));
    let _ = std::fs::remove_file(&pal);
    if two && std::fs::metadata(&gif).is_ok_and(|m| m.len() > 1024) {
        let _ = std::fs::remove_file(mp4);
        Ok(gif)
    } else {
        let _ = std::fs::remove_file(&gif);
        Err("ffmpeg could not make the GIF".into())
    }
}

/// Graceful end: 'q' on ffmpeg's stdin lets it write the MP4 trailer; killed only after 12 s.
fn finalize(app: &AppHandle, mut r: Rec, graceful: bool) {
    if graceful {
        if let Some(mut si) = r.child.stdin.take() {
            let _ = si.write_all(b"q");
            let _ = si.flush();
        }
        let t0 = Instant::now();
        while r.child.try_wait().ok().flatten().is_none() {
            if t0.elapsed() > Duration::from_secs(12) {
                let _ = r.child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = r.child.wait();
    if r.job != 0 {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(HANDLE(r.job as *mut c_void));
        }
    }
    emit(app, "rec:state", json!({ "recording": false, "startedAt": r.started }));
    let size = std::fs::metadata(&r.path).map(|m| m.len()).unwrap_or(0);
    if size < 1024 {
        let tail = r.err.lock().unwrap_or_else(|e| e.into_inner()).trim().to_string();
        return fail(app, format!("the recording is empty{}", if tail.is_empty() { String::new() } else { format!(": {tail}") }));
    }
    let mut path = r.path.clone();
    if r.gif {
        match mp4_to_gif(&r.ffmpeg, &path) {
            Ok(g) => path = g,
            Err(e) => eprintln!("capture: GIF conversion failed ({e}), the MP4 is kept"),
        }
    }
    let p = plain(&path);
    let copied = clip_file(&p);
    emit(app, "capture:done", json!({ "kind": "record", "path": p, "copied": copied, "seconds": (now_ms() - r.started) / 1000 }));
}

// ---------------------------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------------------------

#[tauri::command]
pub fn capture_start(app: AppHandle, mode: String) {
    start(&app, &mode);
}

#[tauri::command]
pub fn capture_cancel(app: AppHandle) {
    end_session(&app);
}

#[tauri::command]
pub fn capture_stop(app: AppHandle) {
    stop(&app);
}

#[tauri::command]
pub fn capture_settings() -> Value {
    let started = rec_slot().as_ref().map(|r| r.started); // one lock, taken before the json! below
    json!({
        "folder": plain(&shot_dir()),
        "videoFolder": plain(&video_dir()),
        "fps": settings::get_f64("capture.fps", 30.0),
        "encoder": settings::get_str("capture.encoder", "auto"),
        "format": settings::get_str("capture.format", "mp4"),
        "shotAction": settings::get_str("capture.shotAction", "both"),
        "revealAfterSave": settings::get_bool("capture.revealAfterSave", false),
        "ffmpeg": find_ffmpeg().map(|p| plain(&p)),
        "recording": started.is_some(),
        "recStartedAt": started,
        "colors": color_history(),
        "hotkeys": HOTKEY_IDS.iter().map(|(m, id)| json!({ "mode": m, "key": settings::get_str(&format!("hotkey.{id}"), crate::settingswin::ACTIONS.iter().find(|a| a.0 == *id).map_or("", |a| a.1)) })).collect::<Vec<_>>(),
    })
}

/// Page (overlay) asks what to show. None when there is no session for this window.
#[tauri::command]
pub fn capture_session(label: String) -> Option<Value> {
    let g = session();
    let s = g.as_ref()?;
    let sh = s.shots.iter().find(|x| x.mon.label == label)?;
    Some(json!({ "id": s.id, "mode": s.mode.name(), "w": sh.mon.w, "h": sh.mon.h, "x": sh.mon.x, "y": sh.mon.y, "scale": sh.mon.scale, "focus": s.focus == label }))
}

/// The frozen frame of one monitor: raw RGBA, w*h*4 bytes (an ArrayBuffer on the page).
#[tauri::command]
pub fn capture_frame(label: String) -> tauri::ipc::Response {
    tauri::ipc::Response::new(capture_frame_arc(&label).map(|a| a.as_ref().clone()).unwrap_or_default())
}

/// The frozen frame of one monitor, shared (no copy): what `capture_frame` sends, for the native UI (ui/api.rs).
pub fn capture_frame_arc(label: &str) -> Option<std::sync::Arc<Vec<u8>>> {
    session().as_ref().and_then(|s| s.shots.iter().find(|x| x.mon.label == label).map(|x| x.rgba.clone()))
}

/// Keyboard focus for the overlay: the OS window AND the WebView2 control inside it (a window shown with SWP_NOACTIVATE is
/// "foreground" without the web content ever getting key events, so Esc did nothing).
fn focus_overlay(w: &WebviewWindow, focus: bool) {
    if !focus {
        return;
    }
    if let Some(h) = win_hwnd(w) {
        unsafe { foreground(h) };
    }
    let _ = w.set_focus();
    let _ = Webview::set_focus(w.as_ref());
}

/// The page painted its frame: show the window now (topmost, not activating), and focus the one under the cursor.
#[tauri::command]
pub fn capture_ready(app: AppHandle, label: String) {
    let (mon, focus) = {
        let mut g = session();
        let Some(s) = g.as_mut() else { return };
        let focus = s.focus == label;
        let Some(sh) = s.shots.iter_mut().find(|x| x.mon.label == label) else { return };
        sh.ready = true;
        (sh.mon.clone(), focus)
    };
    let a = app.clone();
    let mon1 = mon.clone();
    let _ = app.run_on_main_thread(move || {
        let mon = mon1;
        let Some(w) = a.get_webview_window(&mon.label) else { return };
        place(&w, &mon, true);
        focus_overlay(&w, focus);
    });
    // A second pass: moving a window between monitors of different DPI can make the first size request be reinterpreted.
    let a = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        let a2 = a.clone();
        let _ = a.run_on_main_thread(move || {
            if session().is_none() {
                return; // closed within the 120 ms: never re-show a finished overlay
            }
            if let Some(w) = a2.get_webview_window(&mon.label) {
                place(&w, &mon, true);
                focus_overlay(&w, focus); // retry: the first SetForegroundWindow can lose to the app that owned the keyboard
            }
        });
    });
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct CommitArgs {
    label: String,
    action: String,
    rect: Option<[f64; 4]>,
    vw: f64,
    vh: f64,
    x: f64,
    y: f64,
}

/// Selection / click from an overlay page. Plain JSON args, or (markup) a raw PNG body with the JSON in header `x-pill-args`.
#[tauri::command]
pub async fn capture_commit(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let (args, png): (CommitArgs, Option<Vec<u8>>) = match request.body() {
        tauri::ipc::InvokeBody::Raw(b) => {
            let a = request
                .headers()
                .get("x-pill-args")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| serde_json::from_str(s).ok())
                .ok_or("bad capture arguments")?;
            (a, Some(b.clone()))
        }
        tauri::ipc::InvokeBody::Json(v) => (serde_json::from_value(v.clone()).map_err(|e| e.to_string())?, None),
    };
    capture_commit_impl(&app, args, png)
}

/// The body of `capture_commit` (also called by the native UI through ui/api.rs): `png` is the marked-up image of the Area mode, if any.
pub(crate) fn capture_commit_impl(app: &AppHandle, args: CommitArgs, png: Option<Vec<u8>>) -> Result<(), String> {
    let sess = end_session(app).ok_or("no capture in progress")?;
    let sh = sess.shots.iter().find(|x| x.mon.label == args.label).ok_or("unknown monitor")?;
    let (fw, fh) = (sh.mon.w, sh.mon.h);
    let whole = Rect { x: 0, y: 0, w: fw, h: fh };
    let sel = match args.rect {
        Some(r) => to_frame_rect(r, args.vw, args.vh, fw, fh),
        None => Some(whole),
    };
    let mon = sh.mon.clone();
    let frame = sh.rgba.clone();
    // Heavy work off the UI/IPC path; the overlay is already hidden.
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(r) = sel.filter(|_| sess.mode != Mode::Color).or(if sess.mode == Mode::Color { Some(whole) } else { None }) else {
            return fail(&a, "the selection is empty");
        };
        match sess.mode {
            Mode::Area => {
                let act = if matches!(args.action.as_str(), "copy" | "save" | "record") { args.action.as_str() } else { "both" };
                if act == "record" {
                    return start_recording(&a, Rect { x: mon.x + r.x, y: mon.y + r.y, w: r.w, h: r.h }, "area");
                }
                match png.and_then(|b| decode_png(&b).map(|d| (d, b))) {
                    Some(((w, h, rgba), bytes)) => finish_image(&a, w, h, rgba, Some(bytes), act),
                    None => finish_image(&a, r.w as u32, r.h as u32, crop_rgba(&frame, fw, r), None, act),
                }
            }
            Mode::Text => finish_text(&a, r.w as u32, r.h as u32, crop_rgba(&frame, fw, r)),
            Mode::Qr => finish_qr(&a, r.w as u32, r.h as u32, crop_rgba(&frame, fw, r)),
            Mode::Record => start_recording(&a, Rect { x: mon.x + r.x, y: mon.y + r.y, w: r.w, h: r.h }, if args.rect.is_some() { "area" } else { "screen" }),
            Mode::Color => match to_frame_point(args.x, args.y, args.vw, args.vh, fw, fh) {
                Some((x, y)) => finish_color(&a, pixel(&frame, fw, x, y)),
                None => fail(&a, "bad pixel position"),
            },
            Mode::Screen | Mode::RecordScreen => {}
        }
    });
    Ok(())
}

/// Result-card buttons. Only files inside the capture folders are touched; a URL is opened only on this explicit call.
#[tauri::command]
pub fn capture_action(action: String, value: String) -> Result<(), String> {
    let file = || allowed_path(&value).ok_or_else(|| "that file is outside the capture folders or no longer exists".to_string());
    match action.as_str() {
        "open" => shell_open(&plain(&file()?)).then_some(()).ok_or_else(|| "could not open the file".into()),
        "reveal" => {
            reveal(Path::new(&plain(&file()?)));
            Ok(())
        }
        "copy_image" => {
            let p = file()?;
            let (w, h, rgba) = decode_png(&std::fs::read(&p).map_err(|e| e.to_string())?).ok_or("not a PNG image")?;
            let png = png_bytes(w, h, &rgba).ok_or("encode failed")?;
            clip_image(w, h, &rgba, &png).then_some(()).ok_or_else(|| "clipboard is busy".into())
        }
        "copy_file" => clip_file(&plain(&file()?)).then_some(()).ok_or_else(|| "clipboard is busy".into()),
        "copy_path" => clip_text(&plain(&file()?)).then_some(()).ok_or_else(|| "clipboard is busy".into()),
        "copy_text" if value.len() <= 65536 => clip_text(&value).then_some(()).ok_or_else(|| "clipboard is busy".into()),
        "open_url" if is_http_url(&value) => shell_open(value.trim()).then_some(()).ok_or_else(|| "could not open the link".into()),
        _ => Err("unsupported action".into()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Hotkeys + init
// ---------------------------------------------------------------------------------------------------------------

/// Hotkeys: `hotkey.shotArea / shotFull / ocr / qr / pipette / record` are registered (and re-registered live) by hotkeys.rs,
/// which keeps the plugin -> RegisterHotKey fallback; the 300 ms repeat guard in start() stays here. Ctrl+Alt+Q is the pill's quit key.
/// (mode, settings action id) for capture_settings().
const HOTKEY_IDS: &[(&str, &str)] = &[("area", "shotArea"), ("screen", "shotFull"), ("text", "ocr"), ("qr", "qr"), ("color", "pipette"), ("record_screen", "record")];

pub fn init(_app: &tauri::App) {}

// ---------------------------------------------------------------------------------------------------------------
// Tests: pure logic + synthetic images only (never touches the real screen)
// ---------------------------------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgb: [u8; 3]) -> Vec<u8> {
        (0..w * h).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect()
    }

    #[test]
    fn rect_normalisation_across_dpi() {
        // 100 % monitor: css == physical
        assert_eq!(to_frame_rect([100.0, 50.0, 300.0, 200.0], 1920.0, 1080.0, 1920, 1080), Some(Rect { x: 100, y: 50, w: 300, h: 200 }));
        // 150 % monitor: viewport 2560x1440 css, frame 3840x2160 physical
        assert_eq!(to_frame_rect([100.0, 50.0, 300.0, 200.0], 2560.0, 1440.0, 3840, 2160), Some(Rect { x: 150, y: 75, w: 450, h: 300 }));
        // dragged up-left: negative size is flipped
        assert_eq!(to_frame_rect([400.0, 250.0, -300.0, -200.0], 1920.0, 1080.0, 1920, 1080), Some(Rect { x: 100, y: 50, w: 300, h: 200 }));
        // clamped to the frame
        assert_eq!(to_frame_rect([-50.0, -50.0, 5000.0, 5000.0], 1920.0, 1080.0, 1920, 1080), Some(Rect { x: 0, y: 0, w: 1920, h: 1080 }));
        // empty / junk
        assert_eq!(to_frame_rect([10.0, 10.0, 1.0, 40.0], 1920.0, 1080.0, 1920, 1080), None);
        assert_eq!(to_frame_rect([f64::NAN, 0.0, 10.0, 10.0], 1920.0, 1080.0, 1920, 1080), None);
        assert_eq!(to_frame_rect([0.0, 0.0, 10.0, 10.0], 0.0, 0.0, 1920, 1080), None);
        assert_eq!(to_frame_point(1919.9, 0.0, 1920.0, 1080.0, 1920, 1080), Some((1919, 0)));
        assert_eq!(to_frame_point(100.0, 100.0, 2560.0, 1440.0, 3840, 2160), Some((150, 150)));
    }

    #[test]
    fn debounce_and_foreground_watch() {
        let t = Instant::now();
        let ms = Duration::from_millis;
        let mut last = None;
        assert!(!is_repeat(&mut last, Mode::Area, t, ms(300)));
        assert!(is_repeat(&mut last, Mode::Area, t + ms(100), ms(300)), "same hotkey twice in 100 ms = one press");
        assert!(!is_repeat(&mut last, Mode::Qr, t + ms(150), ms(300)), "another action is never a repeat");
        assert!(!is_repeat(&mut last, Mode::Qr, t + ms(600), ms(300)), "a real second press later toggles");
        let mut lost = None;
        assert!(!lost_too_long(&mut lost, false, t, ms(1500)));
        assert!(!lost_too_long(&mut lost, false, t + ms(1400), ms(1500)));
        assert!(lost_too_long(&mut lost, false, t + ms(1500), ms(1500)));
        assert!(!lost_too_long(&mut lost, true, t + ms(1600), ms(1500)), "regaining the foreground resets the clock");
        assert!(!lost_too_long(&mut lost, false, t + ms(2000), ms(1500)));
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(file_name("png", [2026, 10, 23, 8, 5, 9]), "pill_2026-10-23_08-05-09.png");
        let d = std::env::temp_dir().join(format!("pill-cap-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let first = unique_path(&d, "pill_x.png");
        assert_eq!(first, d.join("pill_x.png"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(unique_path(&d, "pill_x.png"), d.join("pill_x_1.png"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn colour_conversions() {
        assert_eq!(hex_of(255, 66, 58), "#FF423A");
        assert_eq!(parse_hex("#ff423a"), Some([255, 66, 58]));
        assert_eq!(parse_hex("FF423A"), Some([255, 66, 58]));
        assert_eq!(parse_hex("#ff423"), None);
        assert_eq!(parse_hex("#gg0000"), None);
        let mut img = solid(4, 4, [1, 2, 3]);
        img[(2 * 4 + 1) * 4..(2 * 4 + 1) * 4 + 3].copy_from_slice(&[10, 20, 30]);
        assert_eq!(pixel(&img, 4, 1, 2), [10, 20, 30]);
        assert_eq!(pixel(&img, 4, 0, 0), [1, 2, 3]);
    }

    #[test]
    fn crop_dib_png_roundtrip() {
        let mut img = solid(8, 6, [0, 0, 0]);
        for x in 2..5 {
            img[(3 * 8 + x) * 4..(3 * 8 + x) * 4 + 3].copy_from_slice(&[200, 100, 50]);
        }
        let c = crop_rgba(&img, 8, Rect { x: 2, y: 3, w: 3, h: 2 });
        assert_eq!(c.len(), 3 * 2 * 4);
        assert_eq!(&c[0..4], &[200, 100, 50, 255]);
        assert_eq!(&c[12..16], &[0, 0, 0, 255]);
        // DIB is bottom-up BGR(A)
        let d = dib_bytes(3, 2, &c);
        assert_eq!(d.len(), 40 + 3 * 2 * 4);
        assert_eq!(&d[40..44], &[0, 0, 0, 255]); // bottom row first (black)
        assert_eq!(&d[40 + 12..40 + 16], &[50, 100, 200, 255]); // then the top row, BGR order
        let png = png_bytes(3, 2, &c).unwrap();
        assert_eq!(decode_png(&png).unwrap(), (3, 2, c));
        let h = hdrop_bytes(r"C:\a.mp4");
        assert_eq!(h[0], 20);
        assert_eq!(h[16], 1);
        assert_eq!(&h[h.len() - 4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn ffmpeg_argument_builder() {
        let o = RecOpts { rect: Rect { x: -1920, y: 0, w: 1281, h: 721 }, fps: 30, enc: Enc::X264, out: r"C:\v\a.mp4".into() };
        let a = ffmpeg_args(&o);
        let has = |k: &str, v: &str| a.windows(2).any(|p| p[0] == k && p[1] == v);
        assert!(has("-f", "gdigrab"));
        assert!(has("-framerate", "30"));
        assert!(has("-offset_x", "-1920"));
        assert!(has("-offset_y", "0"));
        assert!(has("-video_size", "1280x720"), "size must be even: {a:?}");
        assert!(has("-i", "desktop"));
        assert!(has("-c:v", "libx264") && has("-pix_fmt", "yuv420p"));
        assert_eq!(a.last().unwrap(), r"C:\v\a.mp4");
        let n = ffmpeg_args(&RecOpts { enc: Enc::Nvenc, fps: 999, ..o });
        assert!(n.windows(2).any(|p| p[0] == "-c:v" && p[1] == "h264_nvenc"));
        assert!(n.windows(2).any(|p| p[0] == "-framerate" && p[1] == "60"), "fps is clamped");
    }

    #[test]
    fn finds_ffmpeg_when_installed() {
        // PC-dependent: only asserts consistency when ffmpeg is installed here (it is, via chocolatey, on the dev PC).
        match find_ffmpeg() {
            Some(p) => {
                assert!(p.is_file(), "{p:?}");
                let s = p.display().to_string().to_ascii_lowercase();
                if s.contains("chocolatey") && real_in_choco_lib(Path::new(r"C:\ProgramData\chocolatey\lib")).is_some() {
                    assert!(!s.contains(r"infmpeg.exe") || s.contains(r"	ools\"), "should prefer the real binary over the shim: {s}");
                }
                eprintln!("ffmpeg: {s}");
            }
            None => eprintln!("skipped: ffmpeg not installed"),
        }
    }

    #[test]
    fn urls() {
        assert!(is_http_url("https://example.com/a?b=1"));
        assert!(!is_http_url("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("https://a b"));
        assert!(!is_http_url("calc.exe"));
    }

    #[test]
    fn qr_decodes_a_synthetic_code() {
        let code = qrcode::QrCode::new(b"https://example.com/pill").unwrap();
        let (n, scale, quiet) = (code.width(), 6usize, 4usize);
        let side = (n + 2 * quiet) * scale;
        let mut img = solid(side, side, [255, 255, 255]);
        for y in 0..n {
            for x in 0..n {
                if code[(x, y)] == qrcode::Color::Dark {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let (px, py) = ((x + quiet) * scale + dx, (y + quiet) * scale + dy);
                            img[(py * side + px) * 4..(py * side + px) * 4 + 3].copy_from_slice(&[0, 0, 0]);
                        }
                    }
                }
            }
        }
        assert_eq!(decode_qr(side, side, &img).as_deref(), Some("https://example.com/pill"));
        // light-on-dark (inverted) code
        let inv: Vec<u8> = img.chunks_exact(4).flat_map(|p| [255 - p[0], 255 - p[1], 255 - p[2], 255]).collect();
        assert_eq!(decode_qr(side, side, &inv).as_deref(), Some("https://example.com/pill"));
        // blank image: nothing found, no panic
        assert_eq!(decode_qr(60, 60, &solid(60, 60, [255, 255, 255])), None);
    }

    #[test]
    fn ocr_reads_rendered_text() {
        use windows::Win32::Graphics::Gdi::{CreateFontW, SetBkMode, SetTextColor, TextOutW, TRANSPARENT};
        let (w, h) = (520i32, 90i32);
        let rgba = unsafe {
            let dc = CreateCompatibleDC(None);
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, HANDLE::default(), 0).unwrap();
            SelectObject(dc, HGDIOBJ(bmp.0));
            std::ptr::write_bytes(bits as *mut u8, 0xFF, (w * h * 4) as usize); // white page
            let font = CreateFontW(-44, 0, 0, 0, 400, 0, 0, 0, 0, 0, 0, 5, 0, w!("Arial"));
            SelectObject(dc, HGDIOBJ(font.0));
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, windows::Win32::Foundation::COLORREF(0));
            let text: Vec<u16> = "Hello Pill 2026".encode_utf16().collect();
            let _ = TextOutW(dc, 12, 18, &text);
            let src = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
            let out: Vec<u8> = src.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
            let _ = DeleteObject(HGDIOBJ(font.0));
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(dc);
            out
        };
        match ocr(w as u32, h as u32, &rgba, "en-US") {
            Ok(t) => assert!(t.to_lowercase().contains("hello"), "OCR returned {t:?}"),
            Err(e) if e.contains("no OCR language") => eprintln!("skipped: {e}"),
            Err(e) => panic!("{e}"),
        }
    }
}
