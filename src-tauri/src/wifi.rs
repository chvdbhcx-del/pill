// Wi-Fi for the Controls tab: list the networks, connect (a saved profile directly, a new network with a typed password), disconnect.
// Native WLAN API (wlanapi.dll), so nothing depends on the language of netsh's output. A scan is only requested while the list is open
// (the tab asks `wifi_scan` every few seconds then, never in the background).
// Privacy: the password is typed by the user, goes into a profile XML kept in memory for the one `WlanSetProfile` call (Windows then keeps
// the profile, encrypted, like for any network it connects to) and is wiped afterwards. It is never logged, never stored by Pill and
// never leaves the PC. A profile that was created for a connection that then failed is deleted again.
// Commands: wifi_scan, wifi_connect {ssid, password?}, wifi_disconnect. Results: `status` ok | failed | password | unsupported | error.
use serde_json::{json, Value};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{BOOL, HANDLE};
use windows::Win32::NetworkManagement::WiFi::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sec {
    Open,
    WpaPsk,
    Wpa2Psk,
    Wpa3Sae,
    /// Enterprise (needs a user name / certificate), WEP, shared key: Windows' own settings do these.
    Unsupported,
}

/// `dot11DefaultAuthAlgorithm` / `bSecurityEnabled` of an available network.
pub fn sec_of(auth: i32, enabled: bool) -> Sec {
    if !enabled {
        return Sec::Open;
    }
    match auth {
        4 => Sec::WpaPsk,
        7 => Sec::Wpa2Psk,
        9 => Sec::Wpa3Sae,
        _ => Sec::Unsupported,
    }
}

/// 0..=4 signal bars from the 0..=100 quality.
pub fn bars(quality: u32) -> u8 {
    match quality {
        0 => 0,
        1..=25 => 1,
        26..=50 => 2,
        51..=75 => 3,
        _ => 4,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Net {
    pub ssid: String,
    pub quality: u32,
    pub sec: Sec,
    pub connected: bool,
    /// Windows already has a profile (a saved password) for it.
    pub known: bool,
    pub profile: String,
}

/// Drops hidden networks, merges the entries of one SSID (several access points) and sorts: connected, saved, then by signal.
pub fn tidy(raw: Vec<Net>) -> Vec<Net> {
    let mut out: Vec<Net> = Vec::new();
    for n in raw.into_iter().filter(|n| !n.ssid.is_empty()) {
        match out.iter_mut().find(|o| o.ssid == n.ssid) {
            Some(o) => {
                o.quality = o.quality.max(n.quality);
                o.connected |= n.connected;
                if n.known && !o.known {
                    o.known = true;
                    o.profile = n.profile;
                }
            }
            None => out.push(n),
        }
    }
    out.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.known.cmp(&a.known)).then(b.quality.cmp(&a.quality)).then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase())));
    out
}

fn net_json(n: &Net) -> Value {
    json!({ "ssid": n.ssid, "bars": bars(n.quality), "secured": n.sec != Sec::Open, "supported": n.sec != Sec::Unsupported, "connected": n.connected, "known": n.known })
}

// ---- profiles ----

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Is `pw` acceptable for `sec`? WPA / WPA2: 8..=63 characters or 64 hex digits; WPA3: 1..=128 characters; open networks take none.
pub fn password_ok(sec: Sec, pw: &str) -> bool {
    let n = pw.chars().count();
    match sec {
        Sec::Open => true,
        Sec::WpaPsk | Sec::Wpa2Psk => (8..=63).contains(&n) && pw.is_ascii() || (n == 64 && pw.chars().all(|c| c.is_ascii_hexdigit())),
        Sec::Wpa3Sae => (1..=128).contains(&n),
        Sec::Unsupported => false,
    }
}

/// The profile XML Windows takes. `None` for networks this cannot describe.
pub fn profile_xml(ssid: &str, sec: Sec, password: &str) -> Option<String> {
    let (auth, enc) = match sec {
        Sec::Open => ("open", "none"),
        Sec::WpaPsk => ("WPAPSK", "AES"),
        Sec::Wpa2Psk => ("WPA2PSK", "AES"),
        Sec::Wpa3Sae => ("WPA3SAE", "AES"),
        Sec::Unsupported => return None,
    };
    if ssid.is_empty() || ssid.len() > 32 {
        return None;
    }
    let hex: String = ssid.bytes().map(|b| format!("{b:02X}")).collect();
    let name = xml_escape(ssid);
    let key = if sec == Sec::Open {
        String::new()
    } else {
        let kind = if password.len() == 64 && sec != Sec::Wpa3Sae { "networkKey" } else { "passPhrase" };
        format!("<sharedKey><keyType>{kind}</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>", xml_escape(password))
    };
    Some(format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{name}</name>\
         <SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode>\
         <MSM><security><authEncryption><authentication>{auth}</authentication><encryption>{enc}</encryption><useOneX>false</useOneX></authEncryption>{key}</security></MSM></WLANProfile>"
    ))
}

// ---- the flow, over an interface so it can be tested ----

pub trait Wlan {
    /// Asks the adapter for a fresh scan (the answer shows up in `networks` a few seconds later).
    fn scan(&self);
    fn networks(&self) -> Result<Vec<Net>, String>;
    fn set_profile(&self, xml: &str, overwrite: bool) -> Result<(), String>;
    fn connect(&self, profile: &str) -> Result<(), String>;
    fn disconnect(&self) -> Result<(), String>;
    fn delete_profile(&self, name: &str);
    /// Between two looks at the connection state while it is coming up.
    fn pause(&self);
}

const WAIT_LOOKS: usize = 24;

pub fn list(w: &dyn Wlan) -> Value {
    w.scan();
    match w.networks() {
        Ok(v) => json!({ "status": "ok", "networks": tidy(v).iter().map(net_json).collect::<Vec<_>>() }),
        Err(e) => json!({ "status": "error", "message": e }),
    }
}

pub fn connect_flow(w: &dyn Wlan, ssid: &str, password: Option<&str>) -> Value {
    let nets = match w.networks() {
        Ok(n) => tidy(n),
        Err(e) => return json!({ "status": "error", "message": e }),
    };
    let Some(n) = nets.iter().find(|n| n.ssid == ssid) else { return json!({ "status": "error", "message": "network not found" }) };
    if n.connected {
        return json!({ "status": "ok" });
    }
    let typed = password.filter(|p| !p.is_empty());
    let mut created = false;
    let profile = if !n.known || typed.is_some() {
        // a new network, or a saved one the user re-enters the password for
        if n.sec == Sec::Unsupported {
            return json!({ "status": "unsupported" });
        }
        let pw = typed.unwrap_or("");
        if n.sec != Sec::Open && !password_ok(n.sec, pw) {
            return json!({ "status": "password", "message": if typed.is_none() { "need" } else { "bad" } });
        }
        let Some(mut xml) = profile_xml(ssid, n.sec, pw) else { return json!({ "status": "unsupported" }) };
        let r = w.set_profile(&xml, n.known);
        unsafe { xml.as_bytes_mut() }.fill(0); // the key material does not stay in memory
        if let Err(e) = r {
            return json!({ "status": "error", "message": e });
        }
        created = !n.known;
        ssid.to_string()
    } else {
        n.profile.clone()
    };
    if let Err(e) = w.connect(&profile) {
        if created {
            w.delete_profile(&profile);
        }
        return json!({ "status": "error", "message": e });
    }
    for _ in 0..WAIT_LOOKS {
        w.pause();
        if w.networks().is_ok_and(|v| v.iter().any(|x| x.ssid == ssid && x.connected)) {
            return json!({ "status": "ok" });
        }
    }
    if created {
        w.delete_profile(&profile); // a wrong password must not leave a broken profile behind
    }
    json!({ "status": "failed", "known": n.known })
}

// ---- the WLAN API ----

struct Real {
    h: HANDLE,
    guid: GUID,
}

impl Drop for Real {
    fn drop(&mut self) {
        unsafe { WlanCloseHandle(self.h, None) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn text(w: &[u16]) -> String {
    String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())])
}

impl Real {
    /// The first Wi-Fi adapter.
    fn open() -> Result<Real, String> {
        unsafe {
            let (mut ver, mut h) = (0u32, HANDLE::default());
            if WlanOpenHandle(2, None, &mut ver, &mut h) != 0 {
                return Err("no Wi-Fi service".into());
            }
            let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
            let r = WlanEnumInterfaces(h, None, &mut list);
            let guid = (r == 0 && !list.is_null() && (*list).dwNumberOfItems > 0).then(|| (*list).InterfaceInfo[0].InterfaceGuid);
            if !list.is_null() {
                WlanFreeMemory(list as *const _);
            }
            match guid {
                Some(guid) => Ok(Real { h, guid }),
                None => {
                    WlanCloseHandle(h, None);
                    Err("no Wi-Fi adapter".into())
                }
            }
        }
    }
}

impl Wlan for Real {
    fn scan(&self) {
        unsafe { WlanScan(self.h, &self.guid, None, None, None) };
    }
    fn networks(&self) -> Result<Vec<Net>, String> {
        unsafe {
            let mut l: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
            let r = WlanGetAvailableNetworkList(self.h, &self.guid, 0, None, &mut l);
            if r != 0 || l.is_null() {
                return Err(format!("network list failed ({r})"));
            }
            let items = std::slice::from_raw_parts((*l).Network.as_ptr(), (*l).dwNumberOfItems as usize);
            let out = items
                .iter()
                .map(|n| {
                    let len = (n.dot11Ssid.uSSIDLength as usize).min(32);
                    Net {
                        ssid: String::from_utf8_lossy(&n.dot11Ssid.ucSSID[..len]).into_owned(),
                        quality: n.wlanSignalQuality,
                        sec: sec_of(n.dot11DefaultAuthAlgorithm.0, n.bSecurityEnabled.as_bool()),
                        connected: n.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                        known: n.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0,
                        profile: text(&n.strProfileName),
                    }
                })
                .collect();
            WlanFreeMemory(l as *const _);
            Ok(out)
        }
    }
    fn set_profile(&self, xml: &str, overwrite: bool) -> Result<(), String> {
        let mut w = wide(xml);
        let mut reason = 0u32;
        let r = unsafe { WlanSetProfile(self.h, &self.guid, WLAN_PROFILE_USER, PCWSTR(w.as_ptr()), PCWSTR::null(), BOOL::from(overwrite), None, &mut reason) };
        w.fill(0);
        if r == 0 { Ok(()) } else { Err(format!("profile rejected ({r}, reason {reason})")) }
    }
    fn connect(&self, profile: &str) -> Result<(), String> {
        let w = wide(profile);
        let p = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(w.as_ptr()),
            pDot11Ssid: std::ptr::null_mut(),
            pDesiredBssidList: std::ptr::null_mut(),
            dot11BssType: dot11_BSS_type_any,
            dwFlags: 0,
        };
        let r = unsafe { WlanConnect(self.h, &self.guid, &p, None) };
        if r == 0 { Ok(()) } else { Err(format!("connect failed ({r})")) }
    }
    fn disconnect(&self) -> Result<(), String> {
        let r = unsafe { WlanDisconnect(self.h, &self.guid, None) };
        if r == 0 { Ok(()) } else { Err(format!("disconnect failed ({r})")) }
    }
    fn delete_profile(&self, name: &str) {
        let w = wide(name);
        unsafe { WlanDeleteProfile(self.h, &self.guid, PCWSTR(w.as_ptr()), None) };
    }
    fn pause(&self) {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

fn with_wlan(f: impl FnOnce(&dyn Wlan) -> Value) -> Value {
    match Real::open() {
        Ok(r) => f(&r),
        Err(e) => json!({ "status": "error", "message": e }),
    }
}

/// Command `wifi_scan`: asks for a fresh scan and returns what is known now.
pub async fn wifi_scan() -> Value {
    tauri::async_runtime::spawn_blocking(|| with_wlan(list)).await.unwrap_or_else(|_| json!({ "status": "error", "message": "task failed" }))
}

/// Command `wifi_connect`. Takes up to ~12 s (the adapter needs that long to say no to a wrong password).
pub async fn wifi_connect(ssid: String, password: Option<String>) -> Value {
    tauri::async_runtime::spawn_blocking(move || with_wlan(|w| connect_flow(w, &ssid, password.as_deref()))).await.unwrap_or_else(|_| json!({ "status": "error", "message": "task failed" }))
}

/// Command `wifi_disconnect`.
pub async fn wifi_disconnect() -> Value {
    tauri::async_runtime::spawn_blocking(|| with_wlan(|w| w.disconnect().map(|_| json!({ "status": "ok" })).unwrap_or_else(|e| json!({ "status": "error", "message": e }))))
        .await
        .unwrap_or_else(|_| json!({ "status": "error", "message": "task failed" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn net(ssid: &str, q: u32, sec: Sec, connected: bool, known: bool) -> Net {
        Net { ssid: ssid.into(), quality: q, sec, connected, known, profile: if known { ssid.into() } else { String::new() } }
    }

    #[test]
    fn bars_and_security_classes() {
        assert_eq!([0, 1, 25, 26, 50, 51, 75, 76, 100].map(bars), [0, 1, 1, 2, 2, 3, 3, 4, 4]);
        assert_eq!(sec_of(1, false), Sec::Open);
        assert_eq!(sec_of(7, true), Sec::Wpa2Psk);
        assert_eq!(sec_of(9, true), Sec::Wpa3Sae);
        assert_eq!(sec_of(4, true), Sec::WpaPsk);
        assert_eq!(sec_of(6, true), Sec::Unsupported, "WPA2-Enterprise");
        assert_eq!(sec_of(2, true), Sec::Unsupported, "shared key / WEP");
    }

    #[test]
    fn the_list_is_merged_and_ordered() {
        let v = tidy(vec![
            net("Cafe", 40, Sec::Open, false, false),
            net("", 90, Sec::Open, false, false),
            net("Home", 30, Sec::Wpa2Psk, false, true),
            net("Home", 80, Sec::Wpa2Psk, false, false),
            net("Strong", 99, Sec::Wpa2Psk, false, false),
            net("Office", 10, Sec::Wpa2Psk, true, true),
        ]);
        assert_eq!(v.iter().map(|n| n.ssid.as_str()).collect::<Vec<_>>(), ["Office", "Home", "Strong", "Cafe"]);
        assert_eq!(v[1].quality, 80);
        assert!(v[1].known && v[1].profile == "Home");
    }

    #[test]
    fn password_rules() {
        assert!(password_ok(Sec::Wpa2Psk, "12345678") && !password_ok(Sec::Wpa2Psk, "1234567"));
        assert!(!password_ok(Sec::Wpa2Psk, &"g".repeat(64)) && !password_ok(Sec::Wpa2Psk, &"a".repeat(70)));
        assert!(password_ok(Sec::Wpa2Psk, &"ab".repeat(32)), "64 hex digits are a raw key");
        assert!(password_ok(Sec::Wpa3Sae, "x") && !password_ok(Sec::Wpa3Sae, ""));
        assert!(password_ok(Sec::Open, "") && !password_ok(Sec::Unsupported, "12345678"));
    }

    #[test]
    fn profile_xml_escapes_and_picks_the_security() {
        let x = profile_xml("A&B <1>", Sec::Wpa2Psk, "pa\"ss&1").unwrap();
        assert!(x.contains("<name>A&amp;B &lt;1&gt;</name>") && x.contains("<authentication>WPA2PSK</authentication>"));
        assert!(x.contains("<keyMaterial>pa&quot;ss&amp;1</keyMaterial>") && x.contains("<keyType>passPhrase</keyType>"));
        assert!(x.contains(&format!("<hex>{}</hex>", "A&B <1>".bytes().map(|b| format!("{b:02X}")).collect::<String>())));
        let open = profile_xml("Cafe", Sec::Open, "").unwrap();
        assert!(open.contains("<authentication>open</authentication>") && open.contains("<encryption>none</encryption>") && !open.contains("sharedKey"));
        assert!(profile_xml("X", Sec::Wpa3Sae, "pw").unwrap().contains("WPA3SAE"));
        assert!(profile_xml("X", Sec::Unsupported, "pw").is_none() && profile_xml(&"s".repeat(33), Sec::Open, "").is_none());
        assert!(profile_xml("X", Sec::Wpa2Psk, &"ab".repeat(32)).unwrap().contains("networkKey"));
    }

    struct Fake {
        nets: RefCell<Vec<Net>>,
        log: RefCell<Vec<String>>,
        /// the network that comes up after `connect` (None = the connection never completes)
        comes_up: Option<&'static str>,
        looks: Cell<usize>,
    }
    impl Fake {
        fn new(nets: Vec<Net>, comes_up: Option<&'static str>) -> Fake {
            Fake { nets: RefCell::new(nets), log: RefCell::new(vec![]), comes_up, looks: Cell::new(0) }
        }
        fn said(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }
    impl Wlan for Fake {
        fn scan(&self) {
            self.log.borrow_mut().push("scan".into());
        }
        fn networks(&self) -> Result<Vec<Net>, String> {
            Ok(self.nets.borrow().clone())
        }
        fn set_profile(&self, xml: &str, overwrite: bool) -> Result<(), String> {
            self.log.borrow_mut().push(format!("set overwrite={overwrite} secret_in_xml={}", xml.contains("hunter2hunter2")));
            Ok(())
        }
        fn connect(&self, profile: &str) -> Result<(), String> {
            self.log.borrow_mut().push(format!("connect {profile}"));
            Ok(())
        }
        fn disconnect(&self) -> Result<(), String> {
            Ok(())
        }
        fn delete_profile(&self, name: &str) {
            self.log.borrow_mut().push(format!("delete {name}"));
        }
        fn pause(&self) {
            self.looks.set(self.looks.get() + 1);
            if let (Some(up), true) = (self.comes_up, self.looks.get() == 2) {
                for n in self.nets.borrow_mut().iter_mut() {
                    n.connected = n.ssid == up;
                }
            }
        }
    }

    #[test]
    fn a_saved_network_connects_by_its_profile_without_a_password() {
        let f = Fake::new(vec![net("Home", 70, Sec::Wpa2Psk, false, true)], Some("Home"));
        assert_eq!(connect_flow(&f, "Home", None)["status"], "ok");
        assert_eq!(f.said(), ["connect Home"]);
    }

    #[test]
    fn a_new_secured_network_asks_for_a_password_first_and_never_stores_a_bad_one() {
        let f = Fake::new(vec![net("Cafe", 70, Sec::Wpa2Psk, false, false)], Some("Cafe"));
        assert_eq!(connect_flow(&f, "Cafe", None), json!({ "status": "password", "message": "need" }));
        assert_eq!(connect_flow(&f, "Cafe", Some("short")), json!({ "status": "password", "message": "bad" }));
        assert!(f.said().is_empty(), "nothing reached Windows");
        assert_eq!(connect_flow(&f, "Cafe", Some("hunter2hunter2"))["status"], "ok");
        assert_eq!(f.said(), ["set overwrite=false secret_in_xml=true", "connect Cafe"]);
    }

    #[test]
    fn an_open_network_needs_no_password() {
        let f = Fake::new(vec![net("Free", 70, Sec::Open, false, false)], Some("Free"));
        assert_eq!(connect_flow(&f, "Free", None)["status"], "ok");
    }

    #[test]
    fn a_failed_connection_removes_the_profile_it_created_but_keeps_an_old_one() {
        let f = Fake::new(vec![net("Cafe", 70, Sec::Wpa2Psk, false, false)], None);
        assert_eq!(connect_flow(&f, "Cafe", Some("hunter2hunter2"))["status"], "failed");
        assert_eq!(f.said().last().unwrap(), "delete Cafe");
        let g = Fake::new(vec![net("Home", 70, Sec::Wpa2Psk, false, true)], None);
        assert_eq!(connect_flow(&g, "Home", None)["status"], "failed");
        assert!(!g.said().iter().any(|l| l.starts_with("delete")));
        // a new password for a saved network overwrites the profile
        let h = Fake::new(vec![net("Home", 70, Sec::Wpa2Psk, false, true)], Some("Home"));
        assert_eq!(connect_flow(&h, "Home", Some("hunter2hunter2"))["status"], "ok");
        assert_eq!(h.said()[0], "set overwrite=true secret_in_xml=true");
    }

    #[test]
    fn enterprise_and_missing_networks_are_refused() {
        let f = Fake::new(vec![net("Corp", 70, Sec::Unsupported, false, false)], None);
        assert_eq!(connect_flow(&f, "Corp", Some("hunter2hunter2"))["status"], "unsupported");
        assert_eq!(connect_flow(&f, "Nope", None)["status"], "error");
        assert_eq!(list(&f)["networks"][0]["supported"], false);
    }
}
