// Small system indicators for the island. Owner: focus lane. Every part is opt-in and OFF by default; with its switch off nothing runs.
//  * `live.capsLock` / `live.numLock`: a low-level keyboard hook on its own thread (wakes per key press, does nothing else while idle). It looks at
//    ONE thing: whether the key pressed is Caps Lock or Num Lock. Nothing else about a key is read, kept or sent anywhere. 30 ms after such a
//    key the toggle state is read (`GetKeyState`) and `ind:lock {key, on}` is emitted when it differs from the last one.
//  * `live.btBattery`: a DeviceWatcher over the Bluetooth devices the user has PAIRED (classic and LE), asking Windows for the connection flag
//    and the battery level it already knows. Event-driven, no scanning of the radio, no timer. `ind:bt {kind: connect | low, name, percent?}`.
// Brightness is not offered: Windows has no brightness event without polling or a WMI sink that only laptop panels feed (see docs/native-island.md).
use crate::ui::bus;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const LOW_PCT: u8 = 15;
/// The battery level is reported a moment after the connection; the chip waits this long for it.
const SETTLE_MS: u64 = 3500;

// ---------- pure parts (unit-tested) ----------

/// What a Bluetooth device's changed properties mean for the island.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// It was disconnected and now is: the chip follows after [`SETTLE_MS`] (see [`Dev::settle`]).
    Connected,
    /// The battery fell to [`LOW_PCT`] or less while connected (once until it has recovered above `LOW_PCT + 5` or reconnected).
    Low(u8),
}

#[derive(Clone, Debug, Default)]
pub struct Dev {
    pub connected: bool,
    pub pct: Option<u8>,
    low_sent: bool,
}

impl Dev {
    /// Feeds the properties of one Added / Updated event (`None` = not in this update). Returns what to announce right away.
    pub fn update(&mut self, connected: Option<bool>, pct: Option<u8>) -> Vec<Change> {
        let mut out = Vec::new();
        match connected {
            Some(true) if !self.connected => {
                self.connected = true;
                self.low_sent = false;
                out.push(Change::Connected);
            }
            Some(false) if self.connected => {
                *self = Dev::default(); // a stale level of a device that is away must not greet it on the next connection
            }
            _ => {}
        }
        if let Some(p) = pct.map(|p| p.min(100)) {
            self.pct = Some(p);
            if p > LOW_PCT + 5 {
                self.low_sent = false;
            } else if self.connected && p <= LOW_PCT && !self.low_sent && !out.contains(&Change::Connected) {
                self.low_sent = true;
                out.push(Change::Low(p));
            }
        }
        out
    }

    /// The wait after a connection is over: (level to show, low warning due). A device that connects already low gets the card here.
    pub fn settle(&mut self) -> Option<(Option<u8>, bool)> {
        if !self.connected {
            return None; // gone again within the wait
        }
        let low = self.pct.is_some_and(|p| p <= LOW_PCT) && !self.low_sent;
        self.low_sent |= low;
        Some((self.pct, low))
    }
}

/// Which lock key a virtual-key code is (`VK_CAPITAL` 0x14, `VK_NUMLOCK` 0x90).
pub fn lock_key(vk: u32) -> Option<&'static str> {
    match vk {
        0x14 => Some("caps"),
        0x90 => Some("num"),
        _ => None,
    }
}

// ---------- locks ----------

static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
static PENDING: AtomicBool = AtomicBool::new(false);
/// Last announced toggle state: bit 0 caps, bit 1 num; bit 7 = known.
static LOCKS: AtomicU32 = AtomicU32::new(0);

fn locks_wanted() -> bool {
    crate::settings::get_bool("live.capsLock", false) || crate::settings::get_bool("live.numLock", false)
}

fn toggled(vk: i32) -> bool {
    unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(vk) & 1 != 0 }
}

/// Reads both toggle states, announces the ones that changed (and are switched on).
fn announce_locks() {
    let now = (toggled(0x14) as u32) | ((toggled(0x90) as u32) << 1) | 0x80;
    let old = LOCKS.swap(now, Ordering::SeqCst);
    if old & 0x80 == 0 {
        return; // first sight: only a baseline
    }
    for (bit, key, sw) in [(1u32, "caps", "live.capsLock"), (2, "num", "live.numLock")] {
        if (old ^ now) & bit != 0 && crate::settings::get_bool(sw, false) {
            let _ = bus::emit("ind:lock", json!({ "key": key, "on": now & bit != 0 }));
        }
    }
}

#[cfg(windows)]
mod hook {
    use super::*;
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    unsafe extern "system" fn ll(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if code >= 0 && matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
            let vk = (*(lp.0 as *const KBDLLHOOKSTRUCT)).vkCode;
            if lock_key(vk).is_some() && !PENDING.swap(true, Ordering::SeqCst) {
                // the key has not toggled yet inside the hook: read it a moment later, off the hook thread (a slow hook is dropped by Windows)
                let _ = std::thread::Builder::new().name("lock-key".into()).spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    PENDING.store(false, Ordering::SeqCst);
                    announce_locks();
                });
            }
        }
        CallNextHookEx(HHOOK::default(), code, wp, lp)
    }

    pub fn start() {
        let _ = std::thread::Builder::new().name("lock-hook".into()).spawn(|| unsafe {
            let hinst = HINSTANCE(GetModuleHandleW(None).unwrap_or_default().0);
            let Ok(h) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll), hinst, 0) else {
                HOOK_THREAD.store(0, Ordering::SeqCst);
                return;
            };
            HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
            let mut m = MSG::default();
            while GetMessageW(&mut m, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            let _ = UnhookWindowsHookEx(h);
            HOOK_THREAD.store(0, Ordering::SeqCst);
        });
    }

    pub fn stop() {
        let t = HOOK_THREAD.load(Ordering::SeqCst);
        if t != 0 && t != u32::MAX {
            unsafe {
                let _ = PostThreadMessageW(t, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Installs the hook while a lock switch is on, removes it when both are off.
fn ensure_locks() {
    if cfg!(test) {
        return;
    }
    if locks_wanted() {
        // the thread id is set by the thread itself: a second call before that must not start a second hook
        if HOOK_THREAD.compare_exchange(0, u32::MAX, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            LOCKS.store(0, Ordering::SeqCst);
            hook::start();
            // baseline before the first key press
            let _ = std::thread::Builder::new().name("lock-base".into()).spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                announce_locks();
            });
        }
    } else {
        hook::stop();
    }
}

// ---------- Bluetooth ----------

/// The 12 hex digits of a device's address, lower case, from an association-endpoint id ("Bluetooth#Bluetooth<adapter>-04:57:91:5a:9d:ff") or a
/// device-node id ("BTHENUM\Dev_0457915A9DFF\..."); both name the same device, which is how the connection (endpoint) and the battery (node) meet.
pub fn mac_of(id: &str) -> Option<String> {
    let hex = if let Some(i) = id.to_ascii_lowercase().find("dev_") {
        id.get(i + 4..i + 16)?.to_string()
    } else {
        id.rsplit('-').next()?.replace(':', "")
    };
    (hex.len() == 12 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

mod bt {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use windows::core::{IInspectable, Interface, HSTRING};
    use windows::Devices::Bluetooth::{BluetoothDevice, BluetoothLEDevice};
    use windows::Devices::Enumeration::Pnp::{PnpObject, PnpObjectType, PnpObjectUpdate, PnpObjectWatcher};
    use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationUpdate, DeviceWatcher};
    use windows::Foundation::Collections::{IIterable, IMapView};
    use windows::Foundation::{IReference, TypedEventHandler};

    /// The battery level (percent, a byte) Windows keeps on the PnP node of a Bluetooth device (classic hands-free and the LE battery service).
    const BATTERY_KEY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";
    const CONNECTED_KEY: &str = "System.Devices.Aep.IsConnected";

    #[derive(Default)]
    struct State {
        /// address -> (name, state)
        devs: HashMap<String, (String, Dev)>,
        /// endpoint enumerations that have finished; the first enumeration is the baseline, nothing is announced for devices connected before
        done: u8,
    }

    struct Watchers {
        aep: Vec<DeviceWatcher>,
        pnp: Vec<PnpObjectWatcher>,
    }

    static WATCHERS: Mutex<Watchers> = Mutex::new(Watchers { aep: Vec::new(), pnp: Vec::new() });

    fn prop<T: windows::core::RuntimeType + 'static>(m: &IMapView<HSTRING, IInspectable>, key: &str) -> Option<T> {
        m.Lookup(&HSTRING::from(key)).ok()?.cast::<IReference<T>>().ok()?.Value().ok()
    }

    /// A level as Windows may type it (a byte; some drivers write a wider integer).
    fn level(m: &IMapView<HSTRING, IInspectable>) -> Option<u8> {
        prop::<u8>(m, BATTERY_KEY)
            .or_else(|| prop::<i32>(m, BATTERY_KEY).map(|v| v.clamp(0, 100) as u8))
            .or_else(|| prop::<u32>(m, BATTERY_KEY).map(|v| v.min(100) as u8))
    }

    fn apply(st: &Arc<Mutex<State>>, mac: String, name: Option<String>, connected: Option<bool>, pct: Option<u8>) {
        let (changes, name, ready) = {
            let mut g = st.lock().unwrap_or_else(|e| e.into_inner());
            let ready = g.done >= 2;
            let e = g.devs.entry(mac.clone()).or_insert_with(|| (String::new(), Dev::default()));
            if let Some(n) = name.filter(|n| !n.is_empty()) {
                e.0 = n;
            }
            let ch = e.1.update(connected, pct);
            (ch, e.0.clone(), ready)
        };
        if !ready || name.is_empty() {
            return;
        }
        for c in changes {
            match c {
                Change::Low(p) => {
                    let _ = bus::emit("ind:bt", json!({ "kind": "low", "name": name, "percent": p }));
                }
                Change::Connected => {
                    let (st, mac, name) = (st.clone(), mac.clone(), name.clone());
                    let _ = std::thread::Builder::new().name("bt-connect".into()).spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(SETTLE_MS));
                        let r = st.lock().unwrap_or_else(|e| e.into_inner()).devs.get_mut(&mac).and_then(|e| e.1.settle());
                        if let Some((pct, low)) = r {
                            let _ = bus::emit("ind:bt", json!({ "kind": "connect", "name": name, "percent": pct }));
                            if let (true, Some(p)) = (low, pct) {
                                let _ = bus::emit("ind:bt", json!({ "kind": "low", "name": name, "percent": p }));
                            }
                        }
                    });
                }
            }
        }
    }

    /// The paired devices of one kind: their name and connection flag. Paired only: the pairing-state selectors read the PnP store, they do
    /// not scan the radio.
    fn watch_endpoints(aqs: &HSTRING, st: &Arc<Mutex<State>>) -> windows::core::Result<DeviceWatcher> {
        let want: IIterable<HSTRING> = IIterable::try_from(vec![HSTRING::from(CONNECTED_KEY)])?;
        let w = DeviceInformation::CreateWatcherAqsFilterAndAdditionalProperties(aqs, &want)?;
        let s = st.clone();
        w.Added(&TypedEventHandler::<DeviceWatcher, DeviceInformation>::new(move |_, i| {
            if let Some(i) = i.as_ref() {
                if let (Ok(id), Ok(p)) = (i.Id(), i.Properties()) {
                    if let Some(mac) = mac_of(&id.to_string()) {
                        apply(&s, mac, i.Name().ok().map(|n| n.to_string()), prop::<bool>(&p, CONNECTED_KEY), None);
                    }
                }
            }
            Ok(())
        }))?;
        let s = st.clone();
        w.Updated(&TypedEventHandler::<DeviceWatcher, DeviceInformationUpdate>::new(move |_, u| {
            if let Some(u) = u.as_ref() {
                if let (Ok(id), Ok(p)) = (u.Id(), u.Properties()) {
                    if let Some(mac) = mac_of(&id.to_string()) {
                        apply(&s, mac, None, prop::<bool>(&p, CONNECTED_KEY), None);
                    }
                }
            }
            Ok(())
        }))?;
        let s = st.clone();
        w.EnumerationCompleted(&TypedEventHandler::<DeviceWatcher, IInspectable>::new(move |_, _| {
            s.lock().unwrap_or_else(|e| e.into_inner()).done += 1;
            Ok(())
        }))?;
        w.Start()?;
        Ok(w)
    }

    /// The device nodes whose id starts with `prefix` (BTHENUM = classic, BTHLE = LE): the battery level and its changes.
    fn watch_nodes(prefix: &str, st: &Arc<Mutex<State>>) -> windows::core::Result<PnpObjectWatcher> {
        let want: IIterable<HSTRING> = IIterable::try_from(vec![HSTRING::from(BATTERY_KEY)])?;
        let filter = HSTRING::from(format!("System.Devices.DeviceInstanceId:~<\"{prefix}\""));
        let w = PnpObject::CreateWatcherAqsFilter(PnpObjectType::Device, &want, &filter)?;
        let s = st.clone();
        w.Added(&TypedEventHandler::<PnpObjectWatcher, PnpObject>::new(move |_, n| {
            if let Some(n) = n.as_ref() {
                if let (Ok(id), Ok(p)) = (n.Id(), n.Properties()) {
                    if let Some(mac) = mac_of(&id.to_string()) {
                        apply(&s, mac, None, None, level(&p));
                    }
                }
            }
            Ok(())
        }))?;
        let s = st.clone();
        w.Updated(&TypedEventHandler::<PnpObjectWatcher, PnpObjectUpdate>::new(move |_, u| {
            if let Some(u) = u.as_ref() {
                if let (Ok(id), Ok(p)) = (u.Id(), u.Properties()) {
                    if let Some(mac) = mac_of(&id.to_string()) {
                        apply(&s, mac, None, None, level(&p));
                    }
                }
            }
            Ok(())
        }))?;
        w.Start()?;
        Ok(w)
    }

    pub fn start() {
        let mut g = WATCHERS.lock().unwrap_or_else(|e| e.into_inner());
        if !g.aep.is_empty() {
            return;
        }
        let st = Arc::new(Mutex::new(State::default()));
        for sel in [BluetoothDevice::GetDeviceSelectorFromPairingState(true), BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)] {
            match sel.and_then(|s| watch_endpoints(&s, &st)) {
                Ok(w) => g.aep.push(w),
                Err(_) => st.lock().unwrap_or_else(|e| e.into_inner()).done += 1, // a kind that cannot be watched must not hold the baseline back
            }
        }
        for prefix in ["BTHENUM", "BTHLE"] {
            if let Ok(w) = watch_nodes(prefix, &st) {
                g.pnp.push(w);
            }
        }
    }

    pub fn stop() {
        let mut g = WATCHERS.lock().unwrap_or_else(|e| e.into_inner());
        for w in g.aep.drain(..) {
            let _ = w.Stop();
        }
        for w in g.pnp.drain(..) {
            let _ = w.Stop();
        }
    }
}

fn sync_bt() {
    if cfg!(test) {
        return;
    }
    if crate::settings::get_bool("live.btBattery", false) {
        bt::start();
    } else {
        bt::stop();
    }
}

pub fn init(_app: &tauri::App) {
    ensure_locks();
    sync_bt();
}

pub fn on_settings(patch: &Map<String, Value>) {
    if patch.contains_key("live.capsLock") || patch.contains_key("live.numLock") {
        ensure_locks();
    }
    if patch.contains_key("live.btBattery") {
        sync_bt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_connection_is_announced_once_and_its_level_follows() {
        let mut d = Dev::default();
        assert_eq!(d.update(Some(true), None), [Change::Connected]);
        assert_eq!(d.update(Some(true), None), [], "the same flag again is nothing");
        assert_eq!(d.update(None, Some(80)), [], "a level alone is not announced");
        assert_eq!(d.settle(), Some((Some(80), false)));
    }

    #[test]
    fn low_battery_is_announced_once_until_it_recovers() {
        let mut d = Dev::default();
        d.update(Some(true), Some(60));
        d.settle();
        assert_eq!(d.update(None, Some(16)), []);
        assert_eq!(d.update(None, Some(15)), [Change::Low(15)], "15 % and less");
        assert_eq!(d.update(None, Some(14)), [], "not again at 14");
        assert_eq!(d.update(None, Some(18)), [], "18 is not above the recovery line (20)");
        assert_eq!(d.update(None, Some(14)), [], "so no second warning");
        d.update(None, Some(30));
        assert_eq!(d.update(None, Some(10)), [Change::Low(10)], "after a recovery it warns again");
    }

    #[test]
    fn a_device_that_connects_already_low_warns_after_the_wait_and_only_then() {
        let mut d = Dev::default();
        assert_eq!(d.update(Some(true), Some(9)), [Change::Connected], "the connection first, the chip waits for the level");
        assert_eq!(d.settle(), Some((Some(9), true)));
        assert_eq!(d.settle(), Some((Some(9), false)), "once");
        // the level arrives before the connection flag
        let mut e = Dev::default();
        assert_eq!(e.update(None, Some(9)), [], "not connected: no warning for a device that is away");
        assert_eq!(e.update(Some(true), None), [Change::Connected]);
        assert_eq!(e.settle(), Some((Some(9), true)));
    }

    #[test]
    fn a_disconnected_device_forgets_its_level() {
        let mut d = Dev::default();
        d.update(Some(true), Some(50));
        assert_eq!(d.update(Some(false), None), []);
        assert_eq!(d.settle(), None, "gone before the wait ended: no chip");
        assert_eq!(d.update(Some(true), None), [Change::Connected]);
        assert_eq!(d.settle(), Some((None, false)), "a reconnect does not show the old level");
    }

    #[test]
    fn an_endpoint_and_its_node_name_the_same_address() {
        assert_eq!(mac_of("Bluetooth#Bluetooth9c:b6:d0:11:22:33-04:57:91:5A:9D:FF").as_deref(), Some("0457915a9dff"));
        assert_eq!(mac_of("BluetoothLE#BluetoothLE9c:b6:d0:11:22:33-04:57:91:5a:9d:ff").as_deref(), Some("0457915a9dff"));
        assert_eq!(mac_of(r"BTHENUM\Dev_0457915A9DFF\a&2179a568&1&BluetoothDevice_0457915A9DFF").as_deref(), Some("0457915a9dff"));
        assert_eq!(mac_of(r"BTHLE\DEV_0457915A9DFF\8&abc&0&0457915A9DFF").as_deref(), Some("0457915a9dff"));
        assert_eq!(mac_of(r"USB\VID_0BDA&PID_B850\00e04c000001"), None);
        assert_eq!(mac_of(r"BTH\MS_RFCOMM\9&6088e3e&0&0"), None);
        assert_eq!(mac_of(r"BTHENUM\Dev_04579"), None, "a short id is nothing");
    }

    #[test]
    fn levels_are_clamped_and_lock_keys_named() {
        let mut d = Dev::default();
        d.update(Some(true), Some(250));
        assert_eq!(d.pct, Some(100));
        assert_eq!((lock_key(0x14), lock_key(0x90), lock_key(0x41), lock_key(0x91)), (Some("caps"), Some("num"), None, None), "Scroll Lock is not one of ours");
    }

    /// Live check on a PC with Bluetooth (not run by default): the four watchers start, finish their first enumeration without announcing anything
    /// and stop again. `cargo test --bin pill -- --ignored indicators::tests::live_watchers_start_and_stop`
    #[test]
    #[ignore]
    fn live_watchers_start_and_stop() {
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let s2 = seen.clone();
        let id = crate::ui::bus::subscribe("ind:bt", move |_| {
            s2.fetch_add(1, Ordering::SeqCst);
        });
        bt::start();
        std::thread::sleep(std::time::Duration::from_millis(2500));
        assert_eq!(seen.load(Ordering::SeqCst), 0, "devices that were there before are only a baseline");
        bt::stop();
        crate::ui::bus::unsubscribe(id);
    }
}
