// Device registry, one-time pairing codes, the bad-request limiter and the random source of the phone link.
// Tokens are never stored: only their SHA-256 sits in %LOCALAPPDATA%\Pill\phone\devices.json. Nothing here logs a token or a code.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Mutex;

pub const CODE_TTL_MS: i64 = 5 * 60 * 1000;
/// Bad tokens / codes from one address per minute before the whole server answers 429 to it.
pub const BAD_PER_MIN: u32 = 20;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ23456789";

pub fn random_bytes(n: usize) -> Vec<u8> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut b = vec![0u8; n];
    let st = unsafe { BCryptGenRandom(None, &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    assert!(st.0 == 0, "the system random generator failed");
    b
}

/// 43 url-safe characters (256 random bits).
pub fn new_token() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(32))
}

/// 8 characters from A-Z2-9, uniform (rejection sampling).
pub fn new_code() -> String {
    let m = CODE_ALPHABET.len();
    let mut out = String::new();
    while out.len() < 8 {
        for b in random_bytes(16) {
            if (b as usize) < 256 / m * m && out.len() < 8 {
                out.push(CODE_ALPHABET[b as usize % m] as char);
            }
        }
    }
    out
}

/// A file or transfer id: 128 random bits as 32 hex digits.
pub fn new_id() -> String {
    random_bytes(16).iter().map(|x| format!("{x:02x}")).collect()
}

pub fn hash_token(t: &str) -> String {
    Sha256::digest(t.as_bytes()).iter().map(|x| format!("{x:02x}")).collect()
}

pub fn valid_code_shape(c: &str) -> bool {
    c.len() == 8 && c.bytes().all(|b| CODE_ALPHABET.contains(&b))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub os: String,
    #[serde(rename = "appVersion")]
    pub app_version: String,
    #[serde(rename = "tokenHash")]
    pub token_hash: String,
    #[serde(rename = "firstSeen")]
    pub first_seen: i64,
    #[serde(rename = "lastSeen")]
    pub last_seen: i64,
    /// protocol v2 keys (docs/phone-relay.md), DPAPI-protected in the file; `None` for a phone paired before v2 (LAN only)
    #[serde(default, skip_serializing_if = "Option::is_none", with = "super::v2::stored")]
    pub keys: Option<super::v2::Keys>,
}

impl Device {
    /// What the UI may see (never the hash).
    pub fn public(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "model": self.model, "os": self.os, "appVersion": self.app_version, "firstSeen": self.first_seen, "lastSeen": self.last_seen })
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    devices: Vec<Device>,
    /// package -> app name, every app that ever sent a notification (so the muted list can show names)
    #[serde(default)]
    apps: BTreeMap<String, String>,
}

pub struct Registry {
    file: PathBuf,
    data: Mutex<Data>,
}

fn clip(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

impl Registry {
    pub fn load(dir: PathBuf) -> Registry {
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("devices.json");
        let data = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Registry { file, data: Mutex::new(data) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Data> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, d: &Data) {
        let tmp = self.file.with_extension("json.tmp");
        if serde_json::to_vec_pretty(d).ok().is_some_and(|b| std::fs::write(&tmp, b).is_ok()) {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    pub fn list(&self) -> Vec<Device> {
        self.lock().devices.clone()
    }

    pub fn get(&self, id: &str) -> Option<Device> {
        self.lock().devices.iter().find(|d| d.id == id).cloned()
    }

    /// The device that owns this token.
    pub fn by_token(&self, token: &str) -> Option<Device> {
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        let h = hash_token(token);
        self.lock().devices.iter().find(|d| d.token_hash == h).cloned()
    }

    /// Adds a device, or replaces the token (and the details) of the same `device_id`; the old token dies.
    pub fn pair(&self, device_id: &str, name: &str, model: &str, os: &str, app_version: &str, token: &str, now: i64, keys: Option<super::v2::Keys>) -> Device {
        let mut g = self.lock();
        let first = g.devices.iter().find(|d| d.id == device_id).map_or(now, |d| d.first_seen);
        g.devices.retain(|d| d.id != device_id);
        let dev = Device { id: clip(device_id, 64), name: clip(name, 60), model: clip(model, 60), os: clip(os, 40), app_version: clip(app_version, 20), token_hash: hash_token(token), first_seen: first, last_seen: now, keys };
        g.devices.push(dev.clone());
        self.save(&g);
        dev
    }

    pub fn touch(&self, id: &str, now: i64, persist: bool) {
        let mut g = self.lock();
        if let Some(d) = g.devices.iter_mut().find(|d| d.id == id) {
            d.last_seen = now;
        }
        if persist {
            self.save(&g);
        }
    }

    pub fn revoke(&self, id: &str) -> bool {
        let mut g = self.lock();
        let n = g.devices.len();
        g.devices.retain(|d| d.id != id);
        let gone = g.devices.len() != n;
        if gone {
            self.save(&g);
        }
        gone
    }

    /// Remembers an app seen in a notification; true when it was new (the file is rewritten only then).
    pub fn note_app(&self, pkg: &str, name: &str) -> bool {
        let (pkg, name) = (clip(pkg, 120), clip(name, 60));
        if pkg.is_empty() {
            return false;
        }
        let mut g = self.lock();
        if g.apps.get(&pkg).map(String::as_str) == Some(name.as_str()) {
            return false;
        }
        g.apps.insert(pkg, name);
        self.save(&g);
        true
    }

    pub fn apps(&self) -> BTreeMap<String, String> {
        self.lock().apps.clone()
    }
}

/// The one live pairing code.
#[derive(Default)]
pub struct Codes {
    cur: Mutex<Option<(String, i64)>>,
}

impl Codes {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(String, i64)>> {
        self.cur.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The current code with its expiry, a new one when there is none, it ran out or `renew` is set.
    pub fn current(&self, now: i64, renew: bool) -> (String, i64) {
        let mut g = self.lock();
        match &*g {
            Some((c, exp)) if !renew && *exp > now => (c.clone(), *exp),
            _ => {
                let n = (new_code(), now + CODE_TTL_MS);
                *g = Some(n.clone());
                n
            }
        }
    }
    /// Consumes the code when it is right and fresh (one use only).
    pub fn take(&self, code: &str, now: i64) -> bool {
        let mut g = self.lock();
        match &*g {
            Some((c, exp)) if *exp > now && c.eq_ignore_ascii_case(code.trim()) => {
                *g = None;
                true
            }
            _ => false,
        }
    }
    pub fn clear(&self) {
        *self.lock() = None;
    }
}

/// Counts bad tokens / codes per address in one-minute windows.
#[derive(Default)]
pub struct Limiter {
    m: Mutex<HashMap<IpAddr, (i64, u32)>>,
}

impl Limiter {
    pub fn blocked(&self, ip: IpAddr, now: i64) -> bool {
        let g = self.m.lock().unwrap_or_else(|e| e.into_inner());
        g.get(&ip).is_some_and(|(start, n)| now - start < 60_000 && *n >= BAD_PER_MIN)
    }
    pub fn bad(&self, ip: IpAddr, now: i64) {
        let mut g = self.m.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() > 256 {
            g.retain(|_, (s, _)| now - *s < 60_000);
        }
        let e = g.entry(ip).or_insert((now, 0));
        if now - e.0 >= 60_000 {
            *e = (now, 0);
        }
        e.1 += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pill-phone-test-{}-{}", name, new_id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn tokens_and_codes_have_the_protocol_shape() {
        let t = new_token();
        assert_eq!(t.len(), 43);
        assert!(t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_ne!(t, new_token());
        for _ in 0..50 {
            assert!(valid_code_shape(&new_code()));
        }
        assert!(!valid_code_shape("AB3D9XK") && !valid_code_shape("AB3D9XK0") && !valid_code_shape("ab3d9xkq"));
    }

    #[test]
    fn registry_stores_only_hashes_and_replaces_by_device_id() {
        let dir = tmp("reg");
        let r = Registry::load(dir.clone());
        let t1 = new_token();
        r.pair("dev-1", "Pixel 8", "Pixel 8", "Android 15", "1.0.0", &t1, 1000, None);
        let text = std::fs::read_to_string(dir.join("devices.json")).unwrap();
        assert!(!text.contains(&t1), "the token itself must never be written");
        assert!(text.contains(&hash_token(&t1)));
        assert_eq!(r.by_token(&t1).unwrap().id, "dev-1");
        assert!(r.by_token("nope").is_none() && r.by_token("").is_none());
        // pairing again replaces the token, keeps first_seen
        let t2 = new_token();
        let d = r.pair("dev-1", "Pixel 8 Pro", "Pixel 8", "Android 15", "1.0.1", &t2, 5000, None);
        assert_eq!((d.first_seen, d.name.as_str()), (1000, "Pixel 8 Pro"));
        assert!(r.by_token(&t1).is_none() && r.by_token(&t2).is_some());
        assert_eq!(r.list().len(), 1);
        // survives a restart; revoke kills it for good
        let r2 = Registry::load(dir.clone());
        assert!(r2.by_token(&t2).is_some());
        assert!(r2.revoke("dev-1") && !r2.revoke("dev-1"));
        assert!(Registry::load(dir.clone()).by_token(&t2).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn apps_are_remembered_once() {
        let dir = tmp("apps");
        let r = Registry::load(dir.clone());
        assert!(r.note_app("org.telegram.messenger", "Telegram"));
        assert!(!r.note_app("org.telegram.messenger", "Telegram"));
        assert_eq!(Registry::load(dir.clone()).apps()["org.telegram.messenger"], "Telegram");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_code_is_single_use_and_expires() {
        let c = Codes::default();
        let (code, exp) = c.current(1_000, false);
        assert_eq!(exp, 1_000 + CODE_TTL_MS);
        assert_eq!(c.current(2_000, false).0, code, "same code until it is used or expires");
        assert_ne!(c.current(2_000, true).0, code, "renew makes a new one");
        let (code, _) = c.current(2_000, false);
        assert!(c.take(&code.to_lowercase(), 3_000));
        assert!(!c.take(&code, 3_000), "second use fails");
        let (code2, exp2) = c.current(10_000, false);
        assert!(!c.take(&code2, exp2), "expired");
    }

    #[test]
    fn limiter_blocks_after_twenty_bad_requests_per_minute() {
        let l = Limiter::default();
        let ip: IpAddr = "192.168.1.9".parse().unwrap();
        for i in 0..BAD_PER_MIN {
            assert!(!l.blocked(ip, i as i64), "request {i}");
            l.bad(ip, i as i64);
        }
        assert!(l.blocked(ip, 1_000));
        assert!(!l.blocked("192.168.1.10".parse().unwrap(), 1_000), "per address");
        assert!(!l.blocked(ip, 61_000), "the window passes");
    }
}
