// Protocol v2 crypto (docs/phone-relay.md): the key agreement made at pairing, the frame format (XChaCha20-Poly1305), replay protection and the
// DPAPI protection of the stored keys. Only RustCrypto primitives are used; nothing here does I/O (relay.rs does) and nothing is ever logged.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use x25519_dalek::{PublicKey, StaticSecret};

/// Largest plaintext envelope (the relay's frame limit is 1 MiB).
pub const MAX_PLAIN: usize = 900 * 1024;
const NONCE: usize = 24;
const TAG: usize = 16;

/// The three secrets of one paired phone (bytes 0..96 of the HKDF output).
#[derive(Clone, PartialEq, Eq)]
pub struct Keys {
    /// phone -> PC frames
    pub p2c: [u8; 32],
    /// PC -> phone frames
    pub c2p: [u8; 32],
    /// source of the room name and the join secret
    pub room_key: [u8; 32],
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys(..)") // never prints a secret
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    P2c,
    C2p,
}

impl Dir {
    fn name(self) -> &'static str {
        match self {
            Dir::P2c => "p2c",
            Dir::C2p => "c2p",
        }
    }
}

/// `okm = HKDF-SHA256(ikm = shared, salt = SHA256("pill-phone-v2" || code || deviceId), info = "keys", L = 96)`. The code is trimmed and upper-cased.
pub fn derive(shared: &[u8; 32], code: &str, device_id: &str) -> Keys {
    let mut h = Sha256::new();
    h.update(b"pill-phone-v2");
    h.update(code.trim().to_ascii_uppercase().as_bytes());
    h.update(device_id.as_bytes());
    let salt = h.finalize();
    let mut okm = [0u8; 96];
    Hkdf::<Sha256>::new(Some(&salt), shared).expand(b"keys", &mut okm).expect("96 bytes is a valid HKDF-SHA256 length");
    let part = |i: usize| -> [u8; 32] { okm[i * 32..i * 32 + 32].try_into().expect("32 bytes") };
    Keys { p2c: part(0), c2p: part(1), room_key: part(2) }
}

fn hmac(key: &[u8; 32], msg: &[u8]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(msg);
    m.finalize().into_bytes().into()
}

/// `hex(HMAC-SHA256(k_room, "room")[0..16])`: 32 lower-case hex digits.
pub fn room(k: &Keys) -> String {
    hmac(&k.room_key, b"room")[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// `base64url(HMAC-SHA256(k_room, "join"))`, no padding.
pub fn join(k: &Keys) -> String {
    B64.encode(hmac(&k.room_key, b"join"))
}

/// The PC's half of the pairing: from the phone's public key makes a fresh key pair, derives the keys and returns (PC public key, keys).
/// `None` when the phone key is not 32 bytes or gives an all-zero secret (a low-order point).
pub fn pair_pc(phone_pk: &[u8], code: &str, device_id: &str, secret: [u8; 32]) -> Option<([u8; 32], Keys)> {
    let theirs = PublicKey::from(<[u8; 32]>::try_from(phone_pk).ok()?);
    let me = StaticSecret::from(secret);
    let shared = me.diffie_hellman(&theirs);
    if !shared.was_contributory() {
        return None;
    }
    Some((PublicKey::from(&me).to_bytes(), derive(shared.as_bytes(), code, device_id)))
}

/// `"pill-v2" || room || dir` with the room as its 32 ASCII hex digits and the direction as ASCII.
pub fn aad(room: &str, dir: Dir) -> Vec<u8> {
    let mut a = b"pill-v2".to_vec();
    a.extend_from_slice(room.as_bytes());
    a.extend_from_slice(dir.name().as_bytes());
    a
}

/// `nonce(24) || ciphertext || tag`.
pub fn seal_with_nonce(key: &[u8; 32], nonce: &[u8; NONCE], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let c = XChaCha20Poly1305::new(key.into());
    let ct = c.encrypt(XNonce::from_slice(nonce), Payload { msg: plain, aad }).expect("encryption cannot fail for a bounded message");
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    out
}

pub fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let n: [u8; NONCE] = super::reg::random_bytes(NONCE).try_into().expect("24 bytes");
    seal_with_nonce(key, &n, aad, plain)
}

/// The plaintext of a frame, or `None` when it is too short or does not authenticate.
pub fn open(key: &[u8; 32], aad: &[u8], frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < NONCE + TAG {
        return None;
    }
    let (n, ct) = frame.split_at(NONCE);
    XChaCha20Poly1305::new(key.into()).decrypt(XNonce::from_slice(n), Payload { msg: ct, aad }).ok()
}

/// The envelope plaintext: `{"sid":..,"seq":..,"m":<message>}`.
pub fn envelope(sid: &str, seq: u64, m: &Value) -> Vec<u8> {
    format!("{{\"sid\":\"{sid}\",\"seq\":{seq},\"m\":{m}}}").into_bytes() // field order is part of the test vector (the sid is hex)
}

/// A new connection id: 8 random bytes as 16 hex digits.
pub fn new_sid() -> String {
    super::reg::random_bytes(8).iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses a decrypted envelope into (sid, seq, message).
pub fn parse_envelope(plain: &[u8]) -> Option<(String, u64, Value)> {
    let v: Value = serde_json::from_slice(plain).ok()?;
    let sid = v["sid"].as_str().filter(|s| !s.is_empty() && s.len() <= 32)?.to_string();
    Some((sid, v["seq"].as_u64()?, v.get("m").filter(|m| m.is_object())?.clone()))
}

/// Replay / reorder protection: per `sid` only a `seq` greater than the last accepted one passes. Remembers the 64 newest sids.
#[derive(Default)]
pub struct Replay {
    last: HashMap<String, u64>,
    order: VecDeque<String>,
}

impl Replay {
    /// Call only for an authenticated frame. True = new, and now remembered.
    pub fn accept(&mut self, sid: &str, seq: u64) -> bool {
        match self.last.get_mut(sid) {
            Some(l) if seq <= *l => return false,
            Some(l) => *l = seq,
            None => {
                self.last.insert(sid.to_string(), seq);
                self.order.push_back(sid.to_string());
                if self.order.len() > 64 {
                    if let Some(old) = self.order.pop_front() {
                        self.last.remove(&old);
                    }
                }
            }
        }
        true
    }
}

/// Counts frames that failed authentication; `limit` of them within a minute means "close the connection".
pub struct AuthFails {
    at: VecDeque<std::time::Instant>,
}

impl AuthFails {
    pub const LIMIT: usize = 50;
    pub fn new() -> AuthFails {
        AuthFails { at: VecDeque::new() }
    }
    /// Records one failure; true when the limit is reached.
    pub fn record(&mut self, now: std::time::Instant) -> bool {
        self.at.push_back(now);
        while self.at.front().is_some_and(|t| now.duration_since(*t) > std::time::Duration::from_secs(60)) {
            self.at.pop_front();
        }
        self.at.len() >= Self::LIMIT
    }
}

// ---------------------------------------------------------------- DPAPI

const ENTROPY: &[u8] = b"pill.phone.v2.keys";

fn blob(b: &[u8]) -> windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
    windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB { cbData: b.len() as u32, pbData: b.as_ptr() as *mut u8 }
}

fn take(out: windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    unsafe {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
        let _ = LocalFree(HLOCAL(out.pbData as *mut std::ffi::c_void));
        v
    }
}

/// Windows DPAPI (current user) around raw bytes.
pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    use windows::core::w;
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    let (input, ent) = (blob(plain), blob(ENTROPY));
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    unsafe { CryptProtectData(&input, w!("pill-phone"), Some(&ent), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
    Some(take(out))
}

pub fn unprotect(raw: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    let (input, ent) = (blob(raw), blob(ENTROPY));
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    unsafe { CryptUnprotectData(&input, None, Some(&ent), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.ok()?;
    Some(take(out))
}

/// serde adapter for `Option<Keys>`: stored as base64 of the DPAPI blob of the 96 key bytes. A blob this Windows user cannot open reads as "no keys".
pub mod stored {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(k: &Option<Keys>, s: S) -> Result<S::Ok, S::Error> {
        let enc = k.as_ref().and_then(|k| {
            let mut raw = Vec::with_capacity(96);
            raw.extend_from_slice(&k.p2c);
            raw.extend_from_slice(&k.c2p);
            raw.extend_from_slice(&k.room_key);
            let blob = protect(&raw);
            raw.iter_mut().for_each(|b| *b = 0);
            blob
        });
        match enc {
            Some(b) => s.serialize_some(&base64::engine::general_purpose::STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Keys>, D::Error> {
        let t: Option<String> = Option::deserialize(d)?;
        Ok(t.and_then(|t| {
            let raw = unprotect(&base64::engine::general_purpose::STANDARD.decode(t.trim()).ok()?)?;
            let raw: [u8; 96] = raw.try_into().ok()?;
            Some(Keys { p2c: raw[..32].try_into().ok()?, c2p: raw[32..64].try_into().ok()?, room_key: raw[64..].try_into().ok()? })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn seq32(from: u8) -> [u8; 32] {
        std::array::from_fn(|i| from.wrapping_add(i as u8))
    }

    /// The vector of docs/phone-relay.md ("Test vector"). If a value changes, change the document in the same commit.
    #[test]
    fn the_documented_test_vector() {
        let (phone_sk, pc_sk) = (seq32(1), seq32(33));
        let phone_pk = PublicKey::from(&StaticSecret::from(phone_sk));
        let (pc_pk, keys) = pair_pc(phone_pk.as_bytes(), "AB3D9XKQ", "dev-0001", pc_sk).unwrap();
        let shared = StaticSecret::from(phone_sk).diffie_hellman(&PublicKey::from(pc_pk));
        // the phone reaches the same keys from its side
        assert_eq!(derive(shared.as_bytes(), "ab3d9xkq", "dev-0001"), keys, "code case does not matter");
        eprintln!("phone_pk  {}\npc_pk     {}\nshared    {}\nk_p2c     {}\nk_c2p     {}\nk_room    {}\nroom      {}\njoin      {}", hex(phone_pk.as_bytes()), hex(&pc_pk), hex(shared.as_bytes()), hex(&keys.p2c), hex(&keys.c2p), hex(&keys.room_key), room(&keys), join(&keys));
        assert_eq!(hex(phone_pk.as_bytes()), "07a37cbc142093c8b755dc1b10e86cb426374ad16aa853ed0bdfc0b2b86d1c7c");
        assert_eq!(hex(&pc_pk), "5869aff450549732cbaaed5e5df9b30a6da31cb0e5742bad5ad4a1a768f1a67b");
        assert_eq!(hex(shared.as_bytes()), "a84dc7c3c8f058b1b2dc4cd1e9b5dc0a7987f88b6a9564cde3391fc421159e77");
        assert_eq!(hex(&keys.p2c), "66b82e9d3f48c6b06c992309ea4db0c4b9c2b79a0651bd9bf7a6a1d256f0f457");
        assert_eq!(hex(&keys.c2p), "3219d3731507c259220e8b0c0c31f6a17a4be6d8bf800429edeed23d13e9ecf5");
        assert_eq!(hex(&keys.room_key), "80b8bb20746f7f3be0e5628fed2472c748584c3a63317e0096bc297ea073666e");
        assert_eq!(room(&keys), "9cb6afbf89e807519b65ebc880f4fa66");
        assert_eq!(join(&keys), "wORkxOrxSanjPbcJo8r6EAT7eo9bNbaGYbfb0-uKFb0");
        // one frame with a fixed nonce
        let room_s = room(&keys);
        let a = aad(&room_s, Dir::P2c);
        assert_eq!(a, [b"pill-v2".as_slice(), room_s.as_bytes(), b"p2c"].concat());
        let nonce: [u8; 24] = std::array::from_fn(|i| 0xa0 + i as u8);
        let env = envelope("0011223344556677", 1, &json!({ "t": "ping" }));
        assert_eq!(String::from_utf8(env.clone()).unwrap(), r#"{"sid":"0011223344556677","seq":1,"m":{"t":"ping"}}"#);
        let frame = seal_with_nonce(&keys.p2c, &nonce, &a, &env);
        eprintln!("frame     {}", hex(&frame));
        assert_eq!(hex(&frame), "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7ec82a532ab29459806a5e78bdff7b6bf512a03374af0af29145e48ce4c228719e9903b94cc9e5f9f12fd7cd7e20f56eec08ed64a4960276a057f07e1b11d7ae7b685cd");
        assert_eq!(open(&keys.p2c, &a, &frame).unwrap(), env);
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    /// The independent Python vector of the phone side (docs/phone-relay.md, "Changes by the phone side (v2)"): the PC must reproduce it.
    #[test]
    fn the_phone_sides_independent_vector() {
        let phone_sk: [u8; 32] = unhex("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20").try_into().unwrap();
        let pc_sk: [u8; 32] = unhex("65666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8081828384").try_into().unwrap();
        let phone_pk = PublicKey::from(&StaticSecret::from(phone_sk));
        assert_eq!(B64.encode(phone_pk.as_bytes()), "B6N8vBQgk8i3VdwbEOhstCY3StFqqFPtC9_AsrhtHHw");
        let (pc_pk, keys) = pair_pc(phone_pk.as_bytes(), "AB3D9XKQ", "0b6f2f6e-6c7e-4c53-9d2a-1d0c5b7a9e11", pc_sk).unwrap();
        assert_eq!(B64.encode(pc_pk), "VxR2nRFr92Q2rnS8eT0sMK0ZA8WaxSc4BcfiaYtBDDY");
        assert_eq!(hex(&keys.p2c), "6f29aa994173633730fd4e75b68768a37fd7a566053ee8b9eb547892d25121af");
        assert_eq!(hex(&keys.c2p), "e305543817a96cab46c969005eabfeb66aa15dd70bcd93a3f31beb9ea743b726");
        assert_eq!(room(&keys), "2fdc4b4165a4e9db4e0f4cf160d6d9e3");
        assert_eq!(join(&keys), "NsHWn-LIc5AIRoIZMQEdcxf-AmAm9IdoAFk89Q6YKJY");
        let nonce: [u8; 24] = std::array::from_fn(|i| 0xa0 + i as u8);
        // the phone's p2c frame opens on the PC and holds the expected envelope
        let p2c = unhex("a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b789056473c7a389b1db80a44d36411304040928e6c57ea0e40e475b3d740962081779a35e543d48cde87dcdde576e3aaa0126dbf10ae6d0b796da57c3f67332161e5f95");
        let plain = open(&keys.p2c, &aad(&room(&keys), Dir::P2c), &p2c).unwrap();
        assert_eq!(String::from_utf8(plain).unwrap(), r#"{"sid":"a1b2c3d4e5f60718","seq":1,"m":{"t":"ping"}}"#);
        // the PC's c2p frame is byte for byte what the phone expects
        let env = envelope("0011223344556677", 7, &json!({ "t": "hello", "pcName": "DESKTOP-X", "serverVersion": 1, "time": 1700000000000u64 }));
        let mine = seal_with_nonce(&keys.c2p, &nonce, &aad(&room(&keys), Dir::C2p), &env);
        let theirs = "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b74b5e5c8e21e8ba96a44f519ba3d061edd5f3647fb9b3930235a33f00b36717d055836d45e5ff3a68a141700eea3f1ca489d8a7ee22a0d6a54cd1d2890388aa9196673df1405638ae86dbf8c917786aa1ff6f810006275a3dea6b165d06d0a2ce70417ef0984529d34fa0d3fa5d71a4ff9e4de3309e246fb72b505748aebd21a2";
        // json! sorts the hello keys, so compare through the plaintext rather than guessing key order
        let opened = open(&keys.c2p, &aad(&room(&keys), Dir::C2p), &unhex(theirs)).unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&opened).unwrap(), serde_json::from_slice::<Value>(&env).unwrap());
        let literal = r#"{"sid":"0011223344556677","seq":7,"m":{"t":"hello","pcName":"DESKTOP-X","serverVersion":1,"time":1700000000000}}"#;
        assert_eq!(String::from_utf8(opened).unwrap(), literal);
        assert_eq!(hex(&seal_with_nonce(&keys.c2p, &nonce, &aad(&room(&keys), Dir::C2p), literal.as_bytes())), theirs);
        let _ = mine;
    }

    #[test]
    fn frames_authenticate_direction_room_and_content() {
        let k = derive(&seq32(7), "ZZZZ2222", "d");
        let r = room(&k);
        let f = seal(&k.p2c, &aad(&r, Dir::P2c), b"hello");
        assert_eq!(open(&k.p2c, &aad(&r, Dir::P2c), &f).unwrap(), b"hello");
        assert!(open(&k.p2c, &aad(&r, Dir::C2p), &f).is_none(), "a frame of the other direction is refused (reflection)");
        assert!(open(&k.c2p, &aad(&r, Dir::P2c), &f).is_none(), "wrong key");
        assert!(open(&k.p2c, &aad(&"0".repeat(32), Dir::P2c), &f).is_none(), "another room");
        for i in [0, 24, f.len() - 1] {
            let mut t = f.clone();
            t[i] ^= 1;
            assert!(open(&k.p2c, &aad(&r, Dir::P2c), &t).is_none(), "tampered byte {i}");
        }
        assert!(open(&k.p2c, &aad(&r, Dir::P2c), &f[..20]).is_none());
        assert_ne!(seal(&k.p2c, &aad(&r, Dir::P2c), b"hello"), f, "fresh random nonce each time");
        assert_ne!(derive(&seq32(7), "ZZZZ2223", "d").p2c, k.p2c, "the code is part of the salt");
        assert_ne!(derive(&seq32(7), "ZZZZ2222", "e").p2c, k.p2c, "so is the device id");
    }

    #[test]
    fn a_low_order_phone_key_is_refused() {
        assert!(pair_pc(&[0u8; 32], "AB3D9XKQ", "d", seq32(1)).is_none());
        assert!(pair_pc(&[1u8; 31], "AB3D9XKQ", "d", seq32(1)).is_none());
    }

    #[test]
    fn replay_guard_is_per_sid_and_strictly_increasing() {
        let mut r = Replay::default();
        assert!(r.accept("a", 1) && r.accept("a", 2) && r.accept("a", 5));
        assert!(!r.accept("a", 5) && !r.accept("a", 3) && !r.accept("a", 1));
        assert!(r.accept("b", 1), "a new connection starts over");
        assert!(r.accept("a", 6));
        for i in 0..70 {
            r.accept(&format!("s{i}"), 1);
        }
        assert!(r.last.len() <= 64);
    }

    #[test]
    fn fifty_bad_frames_in_a_minute_trip_the_counter() {
        let mut f = AuthFails::new();
        let t0 = std::time::Instant::now();
        for i in 0..49 {
            assert!(!f.record(t0 + std::time::Duration::from_millis(i)));
        }
        assert!(f.record(t0 + std::time::Duration::from_secs(1)));
        let mut g = AuthFails::new();
        for i in 0..200u64 {
            assert!(!g.record(t0 + std::time::Duration::from_secs(i * 2)), "slow noise never trips it");
        }
    }

    #[test]
    fn envelope_roundtrip_and_shape() {
        let e = envelope("0123456789abcdef", 7, &json!({ "t": "x" }));
        assert_eq!(parse_envelope(&e), Some(("0123456789abcdef".into(), 7, json!({ "t": "x" }))));
        assert!(parse_envelope(b"{}").is_none() && parse_envelope(br#"{"sid":"a","seq":1,"m":3}"#).is_none() && parse_envelope(b"nope").is_none());
        assert_eq!(new_sid().len(), 16);
    }

    #[test]
    fn dpapi_roundtrip_through_serde() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct W {
            #[serde(default, with = "stored")]
            k: Option<Keys>,
        }
        let k = derive(&seq32(3), "AB3D9XKQ", "d");
        let text = serde_json::to_string(&W { k: Some(k.clone()) }).unwrap();
        assert!(!text.contains(&hex(&k.p2c)) && !text.contains(&B64.encode(k.p2c)), "no plain key in the file");
        assert_eq!(serde_json::from_str::<W>(&text).unwrap().k, Some(k));
        assert_eq!(serde_json::from_str::<W>("{}").unwrap().k, None);
        assert_eq!(serde_json::from_str::<W>(r#"{"k":"AAAA"}"#).unwrap().k, None, "a blob that is not ours reads as no keys");
    }
}
