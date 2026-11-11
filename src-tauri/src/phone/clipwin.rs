// The real clipboard for the phone clipboard sync (clip.rs): a message-only window with AddClipboardFormatListener (event driven, no polling),
// created only while a phone that announced `clip` is connected and the switches allow it, and destroyed when that stops. Debounce 150 ms.
// On a change: the exclusion formats are checked BEFORE anything is read, then only CF_UNICODETEXT is read. Text is never logged.
use super::clip::{ClipIo, Formats, MAX_BYTES};
use crate::clipboard::{deny_list, exe_stem, global_bytes, open_clipboard, set_global, utf16_bytes};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CF_UNICODETEXT: u32 = 13;
const T_DEBOUNCE: usize = 1;

type Cb = Arc<dyn Fn(String) + Send + Sync>;
static CB: Mutex<Option<Cb>> = Mutex::new(None);
/// The clipboard sequence number right after Pill's own write: that change is not a user copy.
static IGNORE_SEQ: AtomicU32 = AtomicU32::new(0);
/// exe stem of the owner of the last change, resolved when the change arrives (the owner may be gone when the debounce fires)
static OWNER: Mutex<Option<String>> = Mutex::new(None);

pub struct WinClip {
    /// the listener window (as an address) while it exists
    hwnd: Mutex<Option<usize>>,
}

impl WinClip {
    /// `on_change` gets the text of a change that passed every filter (called on the listener thread).
    pub fn new(on_change: Cb) -> WinClip {
        *CB.lock().unwrap_or_else(|e| e.into_inner()) = Some(on_change);
        WinClip { hwnd: Mutex::new(None) }
    }
}

impl ClipIo for WinClip {
    fn set_text(&self, text: &str) -> bool {
        unsafe {
            if !open_clipboard() {
                return false;
            }
            let _ = EmptyClipboard();
            let ok = set_global(CF_UNICODETEXT, &utf16_bytes(text, false));
            let _ = CloseClipboard();
            IGNORE_SEQ.store(GetClipboardSequenceNumber(), Ordering::Relaxed);
            ok
        }
    }

    fn watch(&self, on: bool) {
        let mut g = self.hwnd.lock().unwrap_or_else(|e| e.into_inner());
        match (on, g.is_some()) {
            (true, false) => {
                let (tx, rx) = std::sync::mpsc::channel();
                if std::thread::Builder::new().name("phone-clip".into()).spawn(move || listener(tx)).is_ok() {
                    *g = rx.recv_timeout(std::time::Duration::from_secs(3)).ok().filter(|h| *h != 0);
                }
            }
            (false, true) => {
                if let Some(h) = g.take() {
                    unsafe {
                        let _ = PostMessageW(HWND(h as *mut _), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Reads the formats first, then (only when they allow it) the text. None = nothing to send.
unsafe fn read_text() -> Option<String> {
    if !open_clipboard() {
        return None;
    }
    let r = (|| {
        let (mut f, mut formats, mut text) = (0u32, Formats::default(), false);
        loop {
            f = EnumClipboardFormats(f);
            if f == 0 {
                break;
            }
            if f == CF_UNICODETEXT {
                text = true;
            } else if f >= 0xC000 {
                let mut b = [0u16; 128];
                let n = GetClipboardFormatNameW(f, &mut b).max(0) as usize;
                formats.note(&String::from_utf16_lossy(&b[..n]), || global_bytes(f, 16).and_then(|v| Some(u32::from_le_bytes(v.get(0..4)?.try_into().ok()?))));
            }
        }
        if !text || !formats.allows() {
            return None;
        }
        let raw = global_bytes(CF_UNICODETEXT, MAX_BYTES * 2 + 4)?; // UTF-16 is at most twice the UTF-8 size for text that fits
        let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        Some(String::from_utf16_lossy(&u))
    })();
    let _ = CloseClipboard();
    r
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            if GetClipboardSequenceNumber() != IGNORE_SEQ.load(Ordering::Relaxed) {
                if let Ok(mut o) = OWNER.lock() {
                    *o = GetClipboardOwner().ok().and_then(|h| exe_stem(h));
                }
                SetTimer(hwnd, T_DEBOUNCE, 150, None);
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == T_DEBOUNCE => {
            let _ = KillTimer(hwnd, T_DEBOUNCE);
            let owner = OWNER.lock().ok().and_then(|mut o| o.take());
            let denied = owner.is_some_and(|s| deny_list().iter().any(|d| !d.is_empty() && s.contains(d.as_str()))); // password managers by name
            if !denied {
                if let Some(t) = read_text() {
                    let cb = CB.lock().ok().and_then(|c| c.clone());
                    if let Some(cb) = cb {
                        cb(t);
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = RemoveClipboardFormatListener(hwnd);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn listener(tx: std::sync::mpsc::Sender<usize>) {
    unsafe {
        let Ok(hm) = GetModuleHandleW(PCWSTR::null()) else { return };
        let hi = HINSTANCE(hm.0);
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: hi, lpszClassName: w!("PillPhoneClipListener"), ..Default::default() };
        RegisterClassW(&wc); // a second start finds the class already there: fine
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PillPhoneClipListener"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, HMENU::default(), hi, None) else { return };
        if AddClipboardFormatListener(hwnd).is_err() {
            let _ = DestroyWindow(hwnd);
            return;
        }
        let _ = tx.send(hwnd.0 as usize);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
