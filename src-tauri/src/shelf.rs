// File shelf (drop files, keep, drag out). Owner: shelf agent. Part of the Pill replica (user request: "replica of the Island app via the replica skills").
// Files stay where they are; the DB only keeps path + name. Native drag-out = OLE DoDragDrop with a shell CF_HDROP data object.
use crate::ui::bus::{self, Target};
use base64::Engine;
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle};
use windows::core::{implement, HSTRING, PCWSTR};
use windows::Win32::Foundation::{BOOL, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HGLOBAL, HWND, S_OK, SIZE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoInitializeEx, IBindCtx, IDataObject, COINIT_APARTMENTTHREADED};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILFree, IShellItemImageFactory, SHCreateDataObject, SHCreateItemFromParsingName, SHParseDisplayName, ShellExecuteW,
    SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

#[derive(Serialize, Clone)]
pub struct Item {
    id: i64,
    path: String,
    name: String,
    added_at: i64,
    missing: bool,
    is_dir: bool,
    /// File size in bytes (None for folders and missing files).
    size: Option<u64>,
    icon: Option<String>,
}

/// Shell icon / thumbnail of any existing path as a PNG data URL (disk-cached); shared with the Home launcher.
pub(crate) fn icon_for(path: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    icon_data_url(path, &meta)
}

pub fn init(_app: &tauri::App) {}

/// canonicalize() gives `\\?\C:\x`; the shell and the UI want `C:\x`.
fn clean(p: &Path) -> Option<String> {
    let s = std::fs::canonicalize(p).ok()?.to_string_lossy().into_owned();
    Some(if let Some(r) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{r}")
    } else {
        s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
    })
}

fn path_of(id: i64) -> Option<String> {
    store_one("select path from shelf_items where id = ?1", id)
}

fn store_one(sql: &str, id: i64) -> Option<String> {
    crate::store::with(|c| c.query_row(sql, [id], |r| r.get::<_, String>(0)))
}

fn items() -> Vec<Item> {
    let rows = crate::store::with(|c| {
        let mut st = c.prepare("select id, path, name, added_at from shelf_items order by added_at desc, id desc")?;
        let v = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>();
        v
    })
    .unwrap_or_default();
    rows.into_iter()
        .map(|(id, path, name, added_at)| {
            let meta = std::fs::metadata(&path).ok();
            Item {
                id,
                missing: meta.is_none(),
                is_dir: meta.as_ref().map_or(false, |m| m.is_dir()),
                size: meta.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
                icon: meta.as_ref().and_then(|m| icon_data_url(&path, m)),
                path,
                name,
                added_at,
            }
        })
        .collect()
}

fn changed(app: &AppHandle) -> Vec<Item> {
    let v = items();
    let _ = bus::emit(&app, Target::Label("pill"), "shelf:changed", serde_json::json!({ "count": v.len(), "items": &v }));
    v
}

#[tauri::command(async)]
pub fn shelf_add(app: AppHandle, paths: Vec<String>) -> usize {
    if !crate::settings::get_bool("module.shelf", true) {
        return 0; // Settings > Modules: the shelf is off, drops are not stored
    }
    let mut added = 0;
    for p in paths {
        let Some(path) = clean(Path::new(&p)) else { continue };
        let name = Path::new(&path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        let ins = crate::store::with(|c| {
            let dup: i64 = c.query_row("select count(*) from shelf_items where path = ?1 collate nocase", [&path], |r| r.get(0))?;
            if dup > 0 {
                return Ok(0);
            }
            c.execute("insert into shelf_items (path, name, added_at) values (?1, ?2, ?3)", rusqlite::params![path, name, crate::store::now_ms()])
        });
        added += ins.unwrap_or(0);
    }
    if added > 0 {
        changed(&app);
    }
    added
}

#[tauri::command(async)]
pub fn shelf_list() -> Vec<Item> {
    items()
}

#[tauri::command(async)]
pub fn shelf_remove(app: AppHandle, id: i64) {
    crate::store::with(|c| c.execute("delete from shelf_items where id = ?1", [id]));
    changed(&app);
}

#[tauri::command(async)]
pub fn shelf_clear(app: AppHandle) {
    crate::store::with(|c| c.execute("delete from shelf_items", []));
    changed(&app);
}

#[tauri::command(async)]
pub fn shelf_open(id: i64) -> bool {
    let Some(p) = path_of(id).filter(|p| Path::new(p).exists()) else { return false };
    unsafe {
        // ShellExecute args, no shell string; values > 32 mean success.
        ShellExecuteW(HWND::default(), &HSTRING::from("open"), &HSTRING::from(p.as_str()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL).0 as isize > 32
    }
}

#[tauri::command(async)]
pub fn shelf_reveal(id: i64) -> bool {
    use std::os::windows::process::CommandExt;
    let Some(p) = path_of(id).filter(|p| Path::new(p).exists()) else { return false };
    // One argv element handed to explorer.exe (no cmd.exe, no shell string); CREATE_NO_WINDOW.
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{p}\""))
        .creation_flags(0x0800_0000)
        .spawn()
        .is_ok()
}

#[tauri::command(async)]
pub fn shelf_copy_path(id: i64) -> bool {
    let Some(p) = path_of(id) else { return false };
    set_clipboard_text(&p)
}

/// Plain CF_UNICODETEXT write (the pill never has focus, so navigator.clipboard is not an option).
fn set_clipboard_text(s: &str) -> bool {
    let w: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(HWND::default()).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        if !opened {
            return false;
        }
        let ok = (|| -> Option<()> {
            EmptyClipboard().ok()?;
            let h: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2).ok()?;
            let dst = GlobalLock(h) as *mut u16;
            if dst.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(w.as_ptr(), dst, w.len());
            let _ = GlobalUnlock(h);
            SetClipboardData(13, windows::Win32::Foundation::HANDLE(h.0)).ok()?; // CF_UNICODETEXT; the system owns h now
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

// ---------------------------------------------------------------- native drag-out

#[implement(IDropSource)]
struct Src;

impl IDropSource_Impl for Src_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> windows::core::HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Starts a native drag of the shelf files (blocks until drop/cancel; call while the left button is down).
/// Only COPY and LINK are offered, so the source files are never moved or deleted.
/// Returns "copied" | "cancelled" ("moved" is never returned by design).
#[tauri::command(async)]
pub fn shelf_drag_out(app: AppHandle, ids: Vec<i64>) -> String {
    let paths: Vec<String> = ids.into_iter().filter_map(path_of).filter(|p| Path::new(p).exists()).collect();
    if paths.is_empty() {
        return "cancelled".into();
    }
    let (tx, rx) = std::sync::mpsc::channel();
    // OLE drag needs an STA thread with a message loop: the main (UI) thread is the only one that has it.
    if app.run_on_main_thread(move || { let _ = tx.send(unsafe { do_drag(&paths) }); }).is_err() {
        return "cancelled".into();
    }
    rx.recv().unwrap_or("cancelled").into()
}

unsafe fn do_drag(paths: &[String]) -> &'static str {
    let _ = OleInitialize(None); // no-op / harmless error if the webview already initialised this thread
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    for p in paths {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p.as_str()), None::<&IBindCtx>, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    let abs: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
    // Shell data object: CF_HDROP + shell ID list + drag image support; absolute PIDLs with a null parent.
    let data: windows::core::Result<IDataObject> = if abs.is_empty() { Err(windows::core::Error::empty()) } else { SHCreateDataObject(None, Some(&abs), None::<&IDataObject>) };
    let mut res = "cancelled";
    if let Ok(data) = data {
        let src: IDropSource = Src.into();
        let mut effect = DROPEFFECT_NONE;
        let hr = DoDragDrop(&data, &src, DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0), &mut effect);
        if hr == DRAGDROP_S_DROP && effect != DROPEFFECT_NONE {
            res = "copied";
        }
    }
    for p in pidls {
        ILFree(Some(p as *const ITEMIDLIST));
    }
    res
}

// ---------------------------------------------------------------- icons / thumbnails

/// 64 px shell icon or thumbnail as a PNG data URL, cached on disk by path + mtime + size.
// ponytail: no memory cache and thumbnails of huge videos can be slow on first list; add a background pre-warm if that shows.
fn icon_data_url(path: &str, meta: &std::fs::Metadata) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path.to_lowercase(), mtime, meta.len()).hash(&mut h);
    let cache = crate::store::dir().join("shelf-icons").join(format!("{:016x}.png", h.finish()));
    let png = match std::fs::read(&cache) {
        Ok(b) => b,
        Err(_) => {
            let b = icon_png(path, SIIGBF_BIGGERSIZEOK)?;
            if let Some(d) = cache.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&cache, &b);
            b
        }
    };
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

/// Command-bar row icon: the plain shell ICON (never a thumbnail): an app's own icon (`.lnk`, exe or `shell:AppsFolder\...`), the type
/// icon of a file, the folder icon of a folder. `path` is a parsing name, so it need not exist on disk (UWP apps); disk-cached.
pub(crate) fn bar_icon(path: &str) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let st = std::fs::metadata(path).ok().map(|m| (m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()), m.len(), m.is_dir()));
    // file types share one icon per extension: key those by extension so a folder of 500 PNGs costs one shell call
    let key = match (&st, Path::new(path).extension()) {
        (Some((_, _, false)), Some(e)) if !matches!(e.to_string_lossy().to_lowercase().as_str(), "exe" | "lnk" | "ico" | "url" | "scr" | "msi") => format!("ext:{}", e.to_string_lossy().to_lowercase()),
        (Some((_, _, true)), _) => "folder".to_string(),
        _ => path.to_lowercase(),
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("bar", key, st.map(|s| s.0)).hash(&mut h);
    let cache = crate::store::dir().join("shelf-icons").join(format!("{:016x}.png", h.finish()));
    let png = match std::fs::read(&cache) {
        Ok(b) => b,
        Err(_) => {
            // some shell items refuse the icon-only request (protected folders, odd associations): fall back to any image the shell gives
            let b = icon_png(path, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).or_else(|| icon_png(path, SIIGBF_ICONONLY)).or_else(|| icon_png(path, SIIGBF_BIGGERSIZEOK))?;
            if let Some(d) = cache.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&cache, &b);
            b
        }
    };
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

fn icon_png(path: &str, flags: SIIGBF) -> Option<Vec<u8>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let hbmp = factory.GetImage(SIZE { cx: 64, cy: 64 }, flags).ok()?;
        let mut bm = BITMAP::default();
        GetObjectW(hbmp, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut std::ffi::c_void));
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut bi = BITMAPINFO {
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
        let mut px = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
        let dc = GetDC(HWND::default());
        let rows = GetDIBits(dc, hbmp, 0, h as u32, Some(px.as_mut_ptr() as *mut std::ffi::c_void), &mut bi, DIB_RGB_COLORS);
        ReleaseDC(HWND::default(), dc);
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        if rows == 0 || w <= 0 || h <= 0 {
            return None;
        }
        let no_alpha = px.chunks(4).all(|p| p[3] == 0);
        for p in px.chunks_mut(4) {
            p.swap(0, 2); // BGRA -> RGBA
            if no_alpha {
                p[3] = 255;
            } else if p[3] > 0 && p[3] < 255 {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255) / a).min(255) as u8; // shell bitmaps are premultiplied
                }
            }
        }
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().ok()?.write_image_data(&px).ok()?;
        Some(out)
    }
}
