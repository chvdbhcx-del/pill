// Clipboard history (local, capped, skips sensitive formats). Owner: clipboard agent.
// Capture: message-only window + AddClipboardFormatListener, 150 ms debounce, privacy filters run BEFORE content is read.
// Content is never logged. Items live in store.rs table clip_items; images are PNG files under store::dir()/clips/.
use crate::ui::bus;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Security::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

const DEFAULT_DENY: &[&str] = &["keepass", "keepassxc", "1password", "bitwarden", "lastpass", "dashlane", "enpass", "mstsc"];
const MAX_TEXT: usize = 256 * 1024;
const MAX_IMAGE: usize = 20 * 1024 * 1024;
const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;
const T_DEBOUNCE: usize = 1;
const COLS: &str = "id,kind,substr(text,1,300),length(text),files_json,size_bytes,pinned,source_exe,created_at,used_at";

static APP: OnceLock<AppHandle> = OnceLock::new();
static PAUSED: AtomicBool = AtomicBool::new(false);
static IGNORE_SEQ: AtomicU32 = AtomicU32::new(0);
static OWNER: Mutex<Option<String>> = Mutex::new(None); // exe stem of the clipboard owner, resolved at update time

// ---------- settings (local helper: swap for settings::get_* once that module exists) ----------
fn setting(key: &str) -> Option<Value> {
    let v: Value = serde_json::from_slice(&std::fs::read(crate::store::dir().join("settings.json")).ok()?).ok()?;
    if let Some(x) = v.get(key) {
        return Some(x.clone());
    }
    key.split('.').try_fold(&v, |c, k| c.get(k)).cloned()
}
fn cfg_bool(key: &str, d: bool) -> bool {
    setting(key).and_then(|v| v.as_bool()).unwrap_or(d)
}
fn cfg_i64(key: &str, d: i64) -> i64 {
    setting(key).and_then(|v| v.as_i64()).unwrap_or(d)
}
fn deny_list() -> Vec<String> {
    match setting("clipboard.denyList").and_then(|v| v.as_array().cloned()) {
        Some(a) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_lowercase())).collect(),
        None => DEFAULT_DENY.iter().map(|s| s.to_string()).collect(),
    }
}

// ---------- pure helpers (unit-tested) ----------
fn sha_hex(kind: &str, data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes());
    h.update([0]);
    h.update(data);
    format!("{:x}", h.finalize())
}

fn luhn(digits: &str) -> bool {
    let (mut sum, mut alt) = (0, false);
    for c in digits.chars().rev() {
        let mut d = c.to_digit(10).unwrap_or(0);
        if alt {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
        alt = !alt;
    }
    sum % 10 == 0
}

/// Heuristic for single-line secrets: known token prefixes, Luhn-valid 16-digit numbers, long mixed high-entropy tokens.
/// ponytail: naive entropy threshold, will miss short or low-entropy secrets and may hit random IDs.
fn looks_secret(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.contains(char::is_whitespace) && !t.chars().all(|c| c.is_ascii_digit() || c == ' ' || c == '-') {
        return false;
    }
    if ["sk-", "ghp_", "gho_", "github_pat_", "AKIA", "ASIA", "xoxb-", "xoxp-", "AIza"].iter().any(|p| t.starts_with(p)) && t.len() >= 16 {
        return true;
    }
    let digits: String = t.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 16 && t.chars().all(|c| c.is_ascii_digit() || c == ' ' || c == '-') && luhn(&digits) {
        return true;
    }
    if t.len() < 20 || t.len() > 256 || !t.chars().all(|c| c.is_ascii_alphanumeric() || "-_+/=.".contains(c)) {
        return false;
    }
    let (lo, up, dg) = (t.chars().any(|c| c.is_ascii_lowercase()), t.chars().any(|c| c.is_ascii_uppercase()), t.chars().any(|c| c.is_ascii_digit()));
    let mut freq = [0f64; 128];
    t.bytes().for_each(|b| freq[(b & 127) as usize] += 1.0);
    let n = t.len() as f64;
    let entropy: f64 = freq.iter().filter(|&&f| f > 0.0).map(|f| -(f / n) * (f / n).log2()).sum();
    lo && up && dg && entropy >= 4.0
}

fn like_escape(q: &str) -> String {
    let mut o = String::from("%");
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            o.push('\\');
        }
        o.push(c);
    }
    o.push('%');
    o
}

fn le32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

/// CF_DIB/CF_DIBV5 blob -> (w, h, RGBA). Only 24/32 bpp BI_RGB/BI_BITFIELDS (the usual screenshot/copy-image case).
/// ponytail: palette and 16 bpp bitmaps are skipped; upgrade via GetDIBits if they ever matter.
fn dib_to_rgba(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let hs = le32(d, 0)? as usize;
    let (w, h) = (le32(d, 4)? as i32, le32(d, 8)? as i32);
    let bpp = u16::from_le_bytes(d.get(14..16)?.try_into().ok()?);
    let comp = le32(d, 16)?;
    if hs < 40 || w <= 0 || h == 0 || w > 16384 || h.abs() > 16384 || !(bpp == 24 || bpp == 32) || !(comp == 0 || comp == 3) {
        return None;
    }
    let mut off = hs + le32(d, 32)? as usize * 4;
    if hs == 40 && comp == 3 {
        off += 12;
    }
    let (wu, hu, bytes) = (w as usize, h.unsigned_abs() as usize, bpp as usize / 8);
    let stride = (wu * bytes + 3) & !3;
    let px = d.get(off..off + stride * hu)?;
    let mut out = vec![0u8; wu * hu * 4];
    for y in 0..hu {
        let src_y = if h > 0 { hu - 1 - y } else { y };
        let row = &px[src_y * stride..];
        for x in 0..wu {
            let (s, o) = (x * bytes, (y * wu + x) * 4);
            out[o..o + 3].copy_from_slice(&[row[s + 2], row[s + 1], row[s]]);
            out[o + 3] = if bytes == 4 { row[s + 3] } else { 255 };
        }
    }
    if bytes == 4 && out.chunks_exact(4).all(|p| p[3] == 0) {
        out.chunks_exact_mut(4).for_each(|p| p[3] = 255); // CF_DIB 32 bpp often leaves alpha unused
    }
    Some((w as u32, hu as u32, out))
}

fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Option<()> {
    let mut e = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path).ok()?), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().ok()?.write_image_data(rgba).ok()
}

fn read_png(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let mut r = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?)).read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    (info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight).then(|| (info.width, info.height, buf[..info.buffer_size()].to_vec()))
}

fn thumb_path(p: &str) -> String {
    p.trim_end_matches(".png").to_string() + "_t.png"
}

// ---------- Win32 helpers ----------
unsafe fn exe_stem(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buf = [0u16; 520];
    let mut len = buf.len() as u32;
    let r = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
    let _ = CloseHandle(h);
    r.ok()?;
    let p = String::from_utf16_lossy(&buf[..len as usize]);
    Some(std::path::Path::new(&p).file_stem()?.to_string_lossy().to_lowercase())
}

unsafe fn open_clipboard() -> bool {
    for _ in 0..10 {
        if OpenClipboard(HWND::default()).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    false
}

unsafe fn global_bytes(fmt: u32, max: usize) -> Option<Vec<u8>> {
    let h = HGLOBAL(GetClipboardData(fmt).ok()?.0);
    let n = GlobalSize(h);
    if n == 0 || n > max {
        return None;
    }
    let p = GlobalLock(h) as *const u8;
    if p.is_null() {
        return None;
    }
    let v = std::slice::from_raw_parts(p, n).to_vec();
    let _ = GlobalUnlock(h);
    Some(v)
}

unsafe fn set_global(fmt: u32, data: &[u8]) -> bool {
    let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) else { return false };
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        return false;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
    let _ = GlobalUnlock(h);
    SetClipboardData(fmt, HANDLE(h.0)).is_ok() // on success the system owns the memory
}

fn utf16_bytes(s: &str, nul2: bool) -> Vec<u8> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    if nul2 {
        v.push(0);
    }
    v.iter().flat_map(|u| u.to_le_bytes()).collect()
}

// ---------- capture ----------
enum Content {
    Text(String),
    Files(Vec<String>),
    Dib(Vec<u8>),
}

/// Runs the privacy filters, then reads one representation. Returns None when the change must not be stored.
unsafe fn read_clipboard(source: &Option<String>) -> Option<Content> {
    if !open_clipboard() {
        return None;
    }
    let r = (|| {
        let (mut text, mut files, mut dib) = (false, false, None);
        let mut f = 0;
        loop {
            f = EnumClipboardFormats(f);
            if f == 0 {
                break;
            }
            match f {
                CF_UNICODETEXT => text = true,
                CF_HDROP => files = true,
                CF_DIBV5 => dib = Some(CF_DIBV5),
                CF_DIB if dib.is_none() => dib = Some(CF_DIB),
                x if x >= 0xC000 => {
                    let mut b = [0u16; 128];
                    let n = GetClipboardFormatNameW(x, &mut b).max(0) as usize;
                    match String::from_utf16_lossy(&b[..n]).to_lowercase().as_str() {
                        "excludeclipboardcontentfrommonitorprocessing" | "clipboard viewer ignore" => return None,
                        "canincludeinclipboardhistory" | "canuploadtocloudclipboard" => {
                            if global_bytes(x, 16).and_then(|v| le32(&v, 0)) == Some(0) {
                                return None;
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if let Some(s) = source {
            if deny_list().iter().any(|d| !d.is_empty() && s.contains(d.as_str())) {
                return None;
            }
        }
        if files {
            let h = HDROP(GetClipboardData(CF_HDROP).ok()?.0);
            let n = DragQueryFileW(h, u32::MAX, None).min(200);
            let v: Vec<String> = (0..n)
                .filter_map(|i| {
                    let len = DragQueryFileW(h, i, None) as usize;
                    let mut b = vec![0u16; len + 1];
                    DragQueryFileW(h, i, Some(&mut b));
                    Some(String::from_utf16_lossy(&b[..len]))
                })
                .collect();
            return (!v.is_empty()).then_some(Content::Files(v));
        }
        if text {
            let raw = global_bytes(CF_UNICODETEXT, MAX_TEXT * 4)?;
            let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            let s = String::from_utf16_lossy(&u);
            return (!s.trim().is_empty() && s.len() <= MAX_TEXT).then_some(Content::Text(s));
        }
        global_bytes(dib?, MAX_IMAGE).map(Content::Dib)
    })();
    let _ = CloseClipboard();
    r
}

fn capture(_app: &AppHandle) {
    if PAUSED.load(Ordering::Relaxed) || !crate::settings::get_bool("module.clipboard", true) || unsafe { GetClipboardSequenceNumber() } == IGNORE_SEQ.load(Ordering::Relaxed) {
        return;
    }
    let source = OWNER.lock().ok().and_then(|mut o| o.take());
    let Some(content) = (unsafe { read_clipboard(&source) }) else { return };
    let (kind, text, files_json, hash, size, dib) = match content {
        Content::Text(s) => {
            if cfg_bool("clipboard.skipSecrets", true) && looks_secret(&s) {
                return;
            }
            let h = sha_hex("t", s.as_bytes());
            let n = s.len();
            ("text", Some(s), None, h, n, None)
        }
        Content::Files(v) => {
            let names = v.iter().map(|p| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect::<Vec<_>>().join("\n");
            ("files", Some(names), Some(json!(v).to_string()), sha_hex("f", v.join("\n").as_bytes()), v.len(), None)
        }
        Content::Dib(d) => ("image", None, None, sha_hex("i", &d), d.len(), Some(d)),
    };
    let now = crate::store::now_ms();
    // Dedupe: an existing row just moves to the top (no re-encoding of images).
    let existing = crate::store::with(|c| {
        let n = c.execute("update clip_items set used_at=?1, source_exe=coalesce(?2,source_exe) where hash=?3", params![now, source, hash])?;
        c.query_row("select id from clip_items where hash=?1", [&hash], |r| r.get::<_, i64>(0)).map(|id| (n, id))
    });
    let id = match existing {
        Some((n, id)) if n > 0 => id,
        _ => {
            let mut image_path = None;
            if let Some(d) = dib {
                let Some((w, h, rgba)) = dib_to_rgba(&d) else { return };
                let dir = crate::store::dir().join("clips");
                let _ = std::fs::create_dir_all(&dir);
                let p: PathBuf = dir.join(format!("{hash}.png"));
                if write_png(&p, w, h, &rgba).is_none() {
                    return;
                }
                let k = (w.max(h) as usize).div_ceil(160).max(1);
                let (tw, th) = (w as usize / k, h as usize / k);
                let t: Vec<u8> = (0..th).flat_map(|y| (0..tw).flat_map(move |x| (0..4).map(move |c| (y, x, c)))).map(|(y, x, c)| rgba[((y * k) * w as usize + x * k) * 4 + c]).collect();
                let _ = write_png(std::path::Path::new(&thumb_path(&p.to_string_lossy())), tw.max(1) as u32, th.max(1) as u32, &t);
                image_path = Some(p.to_string_lossy().into_owned());
            }
            let Some(id) = crate::store::with(|c| {
                c.execute(
                    "insert into clip_items(kind,text,image_path,files_json,hash,source_exe,size_bytes,pinned,created_at,used_at) values(?1,?2,?3,?4,?5,?6,?7,0,?8,?8)",
                    params![kind, text, image_path, files_json, hash, source, size as i64, now],
                )?;
                Ok(c.last_insert_rowid())
            }) else {
                return;
            };
            id
        }
    };
    prune();
    if let Some(item) = crate::store::with(|c| item_by_id(c, id)) {
        let _ = bus::emit("clip:added", item);
    }
}

// ---------- DB helpers ----------
fn row_to_json(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let files: Option<String> = r.get(4)?;
    Ok(json!({
        "id": r.get::<_, i64>(0)?, "kind": r.get::<_, String>(1)?,
        "preview": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        "truncated": r.get::<_, Option<i64>>(3)?.unwrap_or(0) > 300,
        "fileCount": files.and_then(|f| serde_json::from_str::<Vec<String>>(&f).ok()).map_or(0, |v| v.len()),
        "size": r.get::<_, i64>(5)?, "pinned": r.get::<_, i64>(6)? == 1,
        "source": r.get::<_, Option<String>>(7)?, "createdAt": r.get::<_, i64>(8)?, "usedAt": r.get::<_, i64>(9)?,
    }))
}

fn item_by_id(c: &Connection, id: i64) -> rusqlite::Result<Value> {
    c.query_row(&format!("select {COLS} from clip_items where id=?1"), [id], row_to_json)
}

fn remove_files(paths: Vec<String>) {
    for p in paths {
        let _ = std::fs::remove_file(thumb_path(&p));
        let _ = std::fs::remove_file(p);
    }
}

/// Delete rows matching `cond` (never pinned unless cond says so) and return their image paths.
fn purge(c: &Connection, cond: &str, p: &[&dyn rusqlite::ToSql]) -> rusqlite::Result<Vec<String>> {
    let paths = c.prepare(&format!("select image_path from clip_items where {cond} and image_path is not null"))?.query_map(p, |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    c.execute(&format!("delete from clip_items where {cond}"), p)?;
    Ok(paths)
}

/// Retention + count cap + image-bytes cap, unpinned rows only. Cheap enough to run after every insert.
fn prune() {
    let (days, max_items, max_img) = (cfg_i64("clipboard.retentionDays", 30), cfg_i64("clipboard.maxItems", 1000), cfg_i64("clipboard.maxImageMb", 512) * 1024 * 1024);
    let gone = crate::store::with(|c| {
        let mut gone = vec![];
        if days > 0 {
            gone.extend(purge(c, "pinned=0 and used_at < ?1", &[&(crate::store::now_ms() - days * 86_400_000)])?);
        }
        gone.extend(purge(c, "pinned=0 and id not in (select id from clip_items where pinned=0 order by used_at desc limit ?1)", &[&max_items])?);
        let mut total: i64 = c.query_row("select coalesce(sum(size_bytes),0) from clip_items where kind='image' and pinned=0", [], |r| r.get(0))?;
        let old: Vec<(i64, i64)> = c.prepare("select id,size_bytes from clip_items where kind='image' and pinned=0 order by used_at asc")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (id, sz) in old {
            if total <= max_img {
                break;
            }
            gone.extend(purge(c, "id=?1", &[&id])?);
            total -= sz;
        }
        Ok(gone)
    });
    remove_files(gone.unwrap_or_default());
}

// ---------- listener thread ----------
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            if !crate::settings::get_bool("module.clipboard", true) {
                return LRESULT(0); // module off: the history is not written, nothing even wakes up
            }
            // Owner window may be gone by the time the debounce fires: resolve it now.
            if let Ok(mut o) = OWNER.lock() {
                *o = GetClipboardOwner().ok().and_then(|h| exe_stem(h));
            }
            SetTimer(hwnd, T_DEBOUNCE, 150, None);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == T_DEBOUNCE => {
            let _ = KillTimer(hwnd, T_DEBOUNCE);
            if let Some(app) = APP.get() {
                capture(app);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn listener() {
    unsafe {
        let Ok(hm) = GetModuleHandleW(PCWSTR::null()) else { return };
        let hi = HINSTANCE(hm.0);
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hi, lpszClassName: w!("PillClipListener"), ..Default::default() };
        RegisterClassW(&wc);
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PillClipListener"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, HMENU::default(), hi, None) else { return };
        if AddClipboardFormatListener(hwnd).is_err() {
            eprintln!("clipboard: AddClipboardFormatListener failed");
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let _ = APP.set(h.clone());
    // the Ctrl+Alt+V hotkey (clip:open) is registered, and re-registered live, by hotkeys.rs
    std::thread::spawn(listener);
    std::thread::spawn(|| loop {
        prune();
        std::thread::sleep(std::time::Duration::from_secs(86_400));
    });
}

// ---------- commands ----------
pub async fn clip_list(query: Option<String>, kind: Option<String>, limit: Option<i64>, offset: Option<i64>) -> Vec<Value> {
    let q = query.filter(|q| !q.trim().is_empty()).map(|q| like_escape(q.trim()));
    crate::store::with(|c| {
        let sql = format!("select {COLS} from clip_items where (?1 is null or text like ?1 escape '\\') and (?2 is null or kind=?2) order by pinned desc, used_at desc limit ?3 offset ?4");
        c.prepare(&sql)?.query_map(params![q, kind, limit.unwrap_or(50).clamp(1, 500), offset.unwrap_or(0).max(0)], row_to_json)?.collect()
    })
    .unwrap_or_default()
}

pub async fn clip_image(id: i64) -> Option<String> {
    use base64::Engine;
    let path: String = crate::store::with(|c| c.query_row("select image_path from clip_items where id=?1 and image_path is not null", [id], |r| r.get(0)))?;
    let bytes = std::fs::read(thumb_path(&path)).or_else(|_| std::fs::read(&path)).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

pub async fn clip_pin(id: i64, pinned: bool) {
    crate::store::with(|c| c.execute("update clip_items set pinned=?1 where id=?2", params![pinned as i64, id]));
}

pub async fn clip_delete(id: i64) {
    if let Some(p) = crate::store::with(|c| purge(c, "id=?1", &[&id])) {
        remove_files(p);
    }
}

pub async fn clip_wipe() {
    // history rows (pinned too) + image files; secure_delete zeroes the freed pages, the checkpoint empties the WAL copy
    crate::store::with(|c| {
        c.execute_batch("pragma secure_delete=ON; delete from clip_items; pragma wal_checkpoint(TRUNCATE); vacuum; pragma secure_delete=OFF;")
    });
    let d = crate::store::dir().join("clips");
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::create_dir_all(&d);
    // the live system clipboard and the Win+V history go too, so the last copied item cannot be pasted or re-captured
    unsafe {
        if open_clipboard() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
            IGNORE_SEQ.store(GetClipboardSequenceNumber(), Ordering::Relaxed);
        }
    }
    let _ = windows::ApplicationModel::DataTransfer::Clipboard::ClearHistory(); // unavailable on some SKUs: ignore
}

pub async fn clip_pause(paused: bool) {
    PAUSED.store(paused, Ordering::Relaxed);
}

pub async fn clip_status() -> Value {
    let (n, b) = crate::store::with(|c| c.query_row("select count(*), coalesce(sum(case when kind='image' then size_bytes end),0) from clip_items", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))).unwrap_or((0, 0));
    json!({ "paused": PAUSED.load(Ordering::Relaxed), "count": n, "imageBytes": b })
}

enum Payload {
    Text(String),
    Files(Vec<String>),
    Image(String),
}

/// True when the foreground window is (or might be) elevated, so UIPI would eat our Ctrl+V.
unsafe fn foreground_blocked() -> bool {
    for _ in 0..4 {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return true;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == std::process::id() {
            std::thread::sleep(std::time::Duration::from_millis(60)); // pill_activate(false) is still handing focus back
            continue;
        }
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return true };
        let mut tok = HANDLE::default();
        let mut elevated = true; // unknown = assume blocked
        if OpenProcessToken(p, TOKEN_QUERY, &mut tok).is_ok() {
            let (mut te, mut n) = (TOKEN_ELEVATION::default(), 0u32);
            if GetTokenInformation(tok, TokenElevation, Some(&mut te as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut n).is_ok() {
                elevated = te.TokenIsElevated != 0;
            }
            let _ = CloseHandle(tok);
        }
        let _ = CloseHandle(p);
        return elevated;
    }
    true
}

unsafe fn put_on_clipboard(p: &Payload) -> bool {
    if !open_clipboard() {
        return false;
    }
    let _ = EmptyClipboard();
    let ok = match p {
        Payload::Text(s) => set_global(CF_UNICODETEXT, &utf16_bytes(s, false)),
        Payload::Files(v) => {
            let mut d = vec![0u8; 20]; // DROPFILES: pFiles=20, pt=0, fNC=0, fWide=1
            d[0] = 20;
            d[16] = 1;
            d.extend(utf16_bytes(&v.join("\0"), true));
            set_global(CF_HDROP, &d)
        }
        Payload::Image(path) => match read_png(path) {
            Some((w, h, rgba)) => {
                let mut d = vec![0u8; 40];
                d[0] = 40;
                d[4..8].copy_from_slice(&w.to_le_bytes());
                d[8..12].copy_from_slice(&h.to_le_bytes());
                d[12] = 1;
                d[14] = 32;
                for row in rgba.chunks_exact(w as usize * 4).rev() {
                    d.extend(row.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]));
                }
                let a = set_global(CF_DIB, &d);
                let png = std::fs::read(path).map(|b| set_global(RegisterClipboardFormatW(w!("PNG")), &b)).unwrap_or(false);
                a || png
            }
            None => false,
        },
    };
    let _ = CloseClipboard();
    IGNORE_SEQ.store(GetClipboardSequenceNumber(), Ordering::Relaxed); // our own write must not re-enter the history
    ok
}

unsafe fn send_ctrl_v() -> bool {
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
    };
    let seq = [key(0x11, false), key(0x56, false), key(0x56, true), key(0x11, true)];
    SendInput(&seq, std::mem::size_of::<INPUT>() as i32) == 4
}

/// Returns "pasted", "copied_only" (target elevated / Ctrl+V failed), "copied" (copy_only was asked for) or "gone" (row deleted meanwhile / unreadable).
/// `copy_only` (Shift+click) never sends Ctrl+V; `plain` (the "T" button) puts file lists on the clipboard as plain text (paths, one per line).
/// Both are Option so that an invoke without them keeps the old behaviour (the command bar passes None, None).
pub async fn clip_paste(id: i64, copy_only: Option<bool>, plain: Option<bool>) -> &'static str {
    // One closure = one lock on the store: the row cannot be deleted between reading it and bumping used_at.
    let row = crate::store::with(|c| {
        let r = c.query_row("select kind,text,image_path,files_json from clip_items where id=?1", [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?)))?;
        c.execute("update clip_items set used_at=?1 where id=?2", params![crate::store::now_ms(), id])?;
        Ok(r)
    });
    let Some((kind, text, img, files)) = row else { return "gone" };
    let payload = match kind.as_str() {
        "text" => text.map(Payload::Text),
        "image" => img.map(Payload::Image),
        _ => files
            .and_then(|f| serde_json::from_str::<Vec<String>>(&f).ok())
            .map(|v| if plain.unwrap_or(false) { Payload::Text(v.join("\r\n")) } else { Payload::Files(v) }),
    };
    let Some(payload) = payload else { return "gone" };
    unsafe {
        if !put_on_clipboard(&payload) {
            return "gone";
        }
        if copy_only.unwrap_or(false) {
            return "copied";
        }
        if foreground_blocked() || !send_ctrl_v() {
            return "copied_only";
        }
    }
    "pasted"
}

/// Puts arbitrary text (an AI result) on the clipboard without recording it in the history, and pastes it unless `copy_only`.
/// Same return values as clip_paste: "pasted" | "copied_only" | "copied" | "gone".
pub async fn clip_paste_text(text: String, copy_only: Option<bool>) -> &'static str {
    let text: String = text.chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return "gone";
    }
    unsafe {
        if !put_on_clipboard(&Payload::Text(text)) {
            return "gone";
        }
        if copy_only.unwrap_or(false) {
            return "copied";
        }
        if foreground_blocked() || !send_ctrl_v() {
            return "copied_only";
        }
    }
    "pasted"
}

/// Hotkey "paste without formatting": the text on the clipboard is put back as plain text only (no fonts, colours, links),
/// then Ctrl+V goes to the window in front once the hotkey's own modifiers are released. Anything but text is left alone.
pub fn paste_plain() {
    std::thread::spawn(|| unsafe {
        if !open_clipboard() {
            return;
        }
        let text = global_bytes(CF_UNICODETEXT, MAX_TEXT * 4).map(|raw| {
            let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            String::from_utf16_lossy(&u)
        });
        let _ = CloseClipboard();
        let Some(t) = text.filter(|t| !t.is_empty()) else { return };
        if !put_on_clipboard(&Payload::Text(t)) || foreground_blocked() {
            return;
        }
        // Ctrl+Alt+Shift are still down while the hotkey fires: sending Ctrl+V now would arrive as Ctrl+Alt+Shift+V
        for _ in 0..40 {
            if [0x10u16, 0x11, 0x12, 0x5B, 0x5C].iter().all(|vk| (GetAsyncKeyState(*vk as i32) as u16 & 0x8000) == 0) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        send_ctrl_v();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_parts() {
        assert!(looks_secret("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(looks_secret("4111 1111 1111 1111"));
        assert!(!looks_secret("4111 1111 1111 1112"));
        assert!(looks_secret("aB3dE6gH9jK2mN5pQ8sT1vW4yZ7cF0hJ"));
        assert!(!looks_secret("hello world, this is a normal sentence"));
        assert!(!looks_secret("https://example.com/some/long/path/index.html"));
        assert_eq!(sha_hex("t", b"abc"), sha_hex("t", b"abc"));
        assert_ne!(sha_hex("t", b"abc"), sha_hex("f", b"abc")); // kind is part of the dedupe key
        assert_eq!(like_escape("50%_"), "%50\\%\\_%");
    }
}
