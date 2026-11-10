// The current Windows virtual desktop, for the chip beside the pill's time (Settings > Live events > Virtual desktop).
// Source: your own user branch HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops (read only): the id list
// `VirtualDesktopIDs`, the current id (`CurrentVirtualDesktop`, per session first) and the desktop names `Desktops\{id}\Name`.
// Nothing is polled while the setting is "off": the poller thread exists only while it is on, ticks once a second and emits
// `vdesk:now {index, total, name}` only when something changed. No telemetry, nothing leaves the PC.
use crate::ui::bus;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows::core::HSTRING;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_SZ};

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

fn wanted() -> bool {
    crate::settings::get_str("live.virtualDesktop", "off") != "off"
}

fn spawn_poller() {
    let _ = std::thread::Builder::new().name("vdesk".into()).spawn(|| {
        let mut last: Option<Value> = None;
        loop {
            if !wanted() {
                RUNNING.store(false, Ordering::SeqCst);
                if !wanted() || RUNNING.swap(true, Ordering::SeqCst) {
                    return; // switched off for good (or another poller took over)
                }
                continue; // switched back on in the gap: keep going
            }
            if !crate::pill::is_hidden() {
                let now = current();
                if now != last {
                    if let Some(v) = &now {
                        let _ = bus::emit("vdesk:now", v);
                    }
                    last = now;
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
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
    if patch.contains_key("live.virtualDesktop") {
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
    fn the_place_is_one_based_and_defaults_to_the_first() {
        let (a, b, c): (Guid, Guid, Guid) = ([1; 16], [2; 16], [3; 16]);
        assert_eq!(place(&[a, b, c], Some(b)), (2, 3));
        assert_eq!(place(&[a, b, c], Some([9; 16])), (1, 3));
        assert_eq!(place(&[], None), (1, 1));
    }
}
