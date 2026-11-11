// The current Windows virtual desktop, for the chip beside the pill's time (Settings > Live events > Virtual desktop).
// Source: your own user branch HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops (read only): the id list
// `VirtualDesktopIDs`, the current id (`CurrentVirtualDesktop`, per session first) and the desktop names `Desktops\{id}\Name`.
// Nothing runs unless the chip (`live.virtualDesktop`) or the switch flash (`live.virtualDesktopFlash`) is on. Then one thread sleeps
// on RegNotifyChangeKeyValue (no timer: zero work while nothing changes; a 60 s wake only re-checks) and emits
// `vdesk:now {index, total, name}` when the answer differs. Switching (`vdesk_switch`) sends Win+Ctrl+Left/Right with SendInput, no COM.
// No telemetry, nothing leaves the PC.
use crate::ui::bus;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET,
    RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY};

const VD: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops";

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
}

// ---------- pure parts (unit-tested) ----------

type Guid = [u8; 16];

/// `VirtualDesktopIDs`: the 16-byte ids one after another.
fn parse_ids(b: &[u8]) -> Vec<Guid> {
    b.chunks_exact(16).map(|c| c.try_into().unwrap_or_default()).collect()
}

/// The registry spelling of an id (a GUID struct in memory order: three little-endian fields, then 8 bytes): `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`.
fn guid_text(g: &Guid) -> String {
    let (d1, d2, d3) = (u32::from_le_bytes([g[0], g[1], g[2], g[3]]), u16::from_le_bytes([g[4], g[5]]), u16::from_le_bytes([g[6], g[7]]));
    format!("{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}", g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15])
}

/// 1-based place of the current desktop among all; an unknown current id (only one desktop was ever used) is the first.
fn place(ids: &[Guid], cur: Option<Guid>) -> (usize, usize) {
    let total = ids.len().max(1);
    (cur.and_then(|c| ids.iter().position(|i| *i == c)).map_or(1, |i| i + 1), total)
}

/// The Win+Ctrl+arrow presses for a switch: `dir` > 0 is the next desktop, < 0 the previous one. Returns (right arrow?, presses).
/// `wrap` goes round from the last to the first (a click on the chip); the wheel stops at the ends. None = nothing to do.
fn plan(index: usize, total: usize, dir: i64, wrap: bool) -> Option<(bool, usize)> {
    if total < 2 || dir == 0 {
        return None;
    }
    let (index, forward) = (index.clamp(1, total), dir > 0);
    match (forward, index) {
        (true, i) if i < total => Some((true, 1)),
        (false, i) if i > 1 => Some((false, 1)),
        (true, _) => wrap.then_some((false, total - 1)),
        (false, _) => wrap.then_some((true, total - 1)),
    }
}

// ---------- registry ----------

fn reg_get(sub: &str, name: &str, flags: windows::Win32::System::Registry::REG_ROUTINE_FLAGS) -> Option<Vec<u8>> {
    let (sub, name) = (HSTRING::from(sub), HSTRING::from(name));
    let mut n = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, &sub, &name, flags, None, None, Some(&mut n)).ok().ok()? };
    let mut buf = vec![0u8; n as usize];
    unsafe { RegGetValueW(HKEY_CURRENT_USER, &sub, &name, flags, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut n)).ok().ok()? };
    buf.truncate(n as usize);
    Some(buf)
}

fn reg_text(sub: &str, name: &str) -> Option<String> {
    let b = reg_get(sub, name, RRF_RT_REG_SZ)?;
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|c| *c != 0).collect();
    Some(String::from_utf16_lossy(&w)).filter(|s| !s.trim().is_empty())
}

fn session() -> u32 {
    let mut s = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut s) };
    s
}

/// The current desktop: `{index, total, name}` (name null while the desktop has none, Windows then calls it "Desktop N"). None without the key.
pub fn current() -> Option<Value> {
    let ids = reg_get(VD, "VirtualDesktopIDs", RRF_RT_REG_BINARY).map(|b| parse_ids(&b)).unwrap_or_default();
    // the session's own value is the live one; the global one is the older place
    let cur = reg_get(&format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{}\VirtualDesktops", session()), "CurrentVirtualDesktop", RRF_RT_REG_BINARY)
        .or_else(|| reg_get(VD, "CurrentVirtualDesktop", RRF_RT_REG_BINARY))
        .and_then(|b| b.get(..16).and_then(|c| c.try_into().ok()));
    if ids.is_empty() && cur.is_none() {
        return None;
    }
    let (index, total) = place(&ids, cur);
    let name = ids.get(index - 1).and_then(|g| reg_text(&format!(r"{VD}\Desktops\{}", guid_text(g)), "Name"));
    Some(json!({ "index": index, "total": total, "name": name }))
}

/// Command `vdesk_state`: the answer the island asks for once at start and when the setting is switched on.
pub fn vdesk_state() -> Value {
    current().unwrap_or(Value::Null)
}

// ---------- the poller ----------

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Needed by the chip (Settings > Live events > Virtual desktop) or by the flash on a switch (on by default).
fn wanted() -> bool {
    crate::settings::get_str("live.virtualDesktop", "off") != "off" || crate::settings::get_bool("live.virtualDesktopFlash", true)
}

/// One registry key whose changes signal `ev` (RegNotifyChangeKeyValue, subtree); closed on drop.
struct Watch {
    key: HKEY,
    ev: HANDLE,
}

impl Watch {
    fn open(sub: &str) -> Option<Watch> {
        let mut key = HKEY::default();
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(sub), 0, KEY_NOTIFY, &mut key).ok().ok()? };
        // auto-reset event: it is armed again after every wake
        let Ok(ev) = (unsafe { CreateEventW(None, false, false, None) }) else {
            unsafe {
                let _ = RegCloseKey(key);
            }
            return None;
        };
        let w = Watch { key, ev };
        w.arm().then_some(w)
    }

    fn arm(&self) -> bool {
        unsafe { RegNotifyChangeKeyValue(self.key, true, REG_NOTIFY_CHANGE_LAST_SET, self.ev, true).ok().is_ok() }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.key);
            let _ = CloseHandle(self.ev);
        }
    }
}

/// Safety net: even with no signal the answer is re-read this often (an Explorer restart, a build that writes elsewhere).
const RECHECK_MS: u32 = 60_000;

fn spawn_poller() {
    let _ = std::thread::Builder::new().name("vdesk".into()).spawn(|| {
        let mut last: Option<Value> = None;
        let mut watches: Vec<Watch> = Vec::new();
        loop {
            if !wanted() {
                RUNNING.store(false, Ordering::SeqCst);
                if !wanted() || RUNNING.swap(true, Ordering::SeqCst) {
                    return; // switched off for good (or another poller took over); the watches close on drop
                }
                continue; // switched back on in the gap: keep going
            }
            // armed BEFORE reading, so a change between the read and the wait still wakes us
            if watches.len() < 2 {
                watches = [VD.to_string(), format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\{}\VirtualDesktops", session())].iter().filter_map(|k| Watch::open(k)).collect();
            }
            let now = current();
            if now != last {
                if let Some(v) = &now {
                    let _ = bus::emit("vdesk:now", v);
                }
                last = now;
            }
            if watches.is_empty() {
                std::thread::sleep(Duration::from_secs(5)); // no key to watch (yet): a slow look now and then
                continue;
            }
            let evs: Vec<HANDLE> = watches.iter().map(|w| w.ev).collect();
            let r = unsafe { WaitForMultipleObjects(&evs, false, RECHECK_MS) };
            let i = r.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
            if i < watches.len() {
                std::thread::sleep(Duration::from_millis(40)); // one switch writes a few values: read once, after them
                if !watches[i].arm() {
                    watches.clear(); // reopened at the top
                }
            }
        }
    });
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } } }
}

/// Command `vdesk_switch`: Win+Ctrl+Left / Right (the documented shortcut, no COM). `dir` > 0 next, < 0 previous; `wrap` goes round at the ends.
/// True when keys were sent (false: one desktop, already at the end, or the keys were refused).
pub fn vdesk_switch(dir: i64, wrap: Option<bool>) -> bool {
    let Some(v) = current() else { return false };
    let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(1) as usize;
    let Some((right, presses)) = plan(n("index"), n("total"), dir, wrap.unwrap_or(false)) else { return false };
    let arrow = if right { 0x27 } else { 0x25 };
    let mut seq = vec![key(0x5B, false), key(0x11, false)];
    for _ in 0..presses {
        seq.extend([key(arrow, false), key(arrow, true)]);
    }
    seq.extend([key(0x11, true), key(0x5B, true)]);
    unsafe { SendInput(&seq, std::mem::size_of::<INPUT>() as i32) == seq.len() as u32 }
}

/// Starts the poller when the setting is on (and it is not running yet).
pub fn ensure() {
    if wanted() && !RUNNING.swap(true, Ordering::SeqCst) {
        spawn_poller();
    }
}

pub fn init(_app: &tauri::App) {
    ensure();
}

/// settings.rs calls this after every patch.
pub fn on_settings(patch: &Map<String, Value>) {
    if patch.contains_key("live.virtualDesktop") || patch.contains_key("live.virtualDesktopFlash") {
        ensure();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_split_into_sixteen_bytes_and_a_torn_tail_is_dropped() {
        let mut b: Vec<u8> = (0..32).collect();
        b.extend([9, 9, 9]);
        let ids = parse_ids(&b);
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[1][0], 16);
    }

    #[test]
    fn the_id_is_spelled_like_the_registry_key() {
        let g: Guid = [0x78, 0x56, 0x34, 0x12, 0xbc, 0x9a, 0xf0, 0xde, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0xab];
        assert_eq!(guid_text(&g), "{12345678-9ABC-DEF0-0102-0304050607AB}");
    }

    #[test]
    fn switching_steps_one_desktop_and_wraps_only_when_asked() {
        assert_eq!(plan(2, 3, 1, false), Some((true, 1)));
        assert_eq!(plan(2, 3, -1, false), Some((false, 1)));
        assert_eq!(plan(3, 3, 1, false), None, "the wheel stops at the end");
        assert_eq!(plan(1, 3, -1, false), None);
        assert_eq!(plan(3, 3, 1, true), Some((false, 2)), "a click on the last goes back to the first: two lefts");
        assert_eq!(plan(1, 4, -1, true), Some((true, 3)));
        assert_eq!(plan(1, 1, 1, true), None, "one desktop");
        assert_eq!(plan(2, 3, 0, true), None);
        assert_eq!(plan(9, 3, 1, false), None, "an index past the end counts as the last");
    }

    #[test]
    fn the_place_is_one_based_and_defaults_to_the_first() {
        let (a, b, c): (Guid, Guid, Guid) = ([1; 16], [2; 16], [3; 16]);
        assert_eq!(place(&[a, b, c], Some(b)), (2, 3));
        assert_eq!(place(&[a, b, c], Some([9; 16])), (1, 3));
        assert_eq!(place(&[], None), (1, 1));
    }
}
