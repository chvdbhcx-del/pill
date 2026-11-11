// Tests of protocol v2 (docs/phone-relay.md) against the real server: a FAKE RELAY (a std-socket WebSocket server on 127.0.0.1 with the join /
// forward / presence semantics of the spec) and a FAKE PHONE that implements the phone side of the crypto straight from the RustCrypto crates
// (not through v2.rs), so the framing is checked from both ends. Nothing leaves the machine; every test stops its server.
use super::files::Status;
use super::server::{Route, Shared};
use super::tests::{bearer, http, Phone, Rig};
use super::wsclient::{Chan, Patient, Target};
use super::{http as phttp, reg, ws};
use base64::{engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::BufReader;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------------------------------------ the fake relay

#[derive(Clone)]
struct Peer {
    id: u64,
    w: Arc<Mutex<TcpStream>>,
}

#[derive(Default)]
struct Room {
    verifier: Vec<u8>,
    pc: Option<Peer>,
    phone: Option<Peer>,
}

#[derive(Default)]
struct RelayState {
    rooms: Mutex<HashMap<String, Room>>,
    socks: Mutex<Vec<TcpStream>>,
    /// every binary frame that was forwarded (what an eavesdropper at the relay sees)
    tap: Mutex<Vec<Vec<u8>>>,
    /// sockets that joined successfully, per role
    joins_pc: AtomicUsize,
    next: AtomicUsize,
}

struct FakeRelay {
    addr: SocketAddr,
    st: Arc<RelayState>,
    stop: Arc<AtomicBool>,
}

impl FakeRelay {
    fn start() -> FakeRelay {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let (st, stop) = (Arc::new(RelayState::default()), Arc::new(AtomicBool::new(false)));
        let (st2, stop2) = (st.clone(), stop.clone());
        std::thread::spawn(move || {
            for c in l.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(s) = c else { continue };
                let st3 = st2.clone();
                st2.socks.lock().unwrap().push(s.try_clone().unwrap());
                std::thread::spawn(move || serve(st3, s));
            }
        });
        FakeRelay { addr, st, stop }
    }
    fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }
    /// Cuts every socket (a network drop).
    fn kill_all(&self) {
        for s in self.st.socks.lock().unwrap().drain(..) {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
    fn pc_joins(&self) -> usize {
        self.st.joins_pc.load(Ordering::SeqCst)
    }
    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        self.kill_all();
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(300));
    }
}

fn send_to(p: &Peer, bytes: &[u8]) {
    let _ = ws::write_all(&mut *p.w.lock().unwrap(), bytes);
}

fn serve(st: Arc<RelayState>, s: TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut w = s.try_clone().unwrap();
    let mut r = BufReader::new(s);
    let Ok(req) = phttp::read_head(&mut r) else { return };
    let (Some(room), Some(role)) = (req.path.strip_prefix("/r/").map(String::from), req.query.get("role").cloned()) else { return };
    let Some(key) = req.header("sec-websocket-key").map(String::from) else { return };
    use std::io::Write;
    let _ = w.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", ws::accept_key(&key)).as_bytes());
    // the first message must be the join (within the 5 s read timeout)
    let Ok(ws::Msg::Text(t)) = ws::read_msg(&mut r) else { return };
    let j: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
    let verifier = Sha256::digest(j["join"].as_str().unwrap_or("").as_bytes()).to_vec();
    let me = Peer { id: st.next.fetch_add(1, Ordering::SeqCst) as u64, w: Arc::new(Mutex::new(w)) };
    let (other, old) = {
        let mut rooms = st.rooms.lock().unwrap();
        let rm = rooms.entry(room.clone()).or_default();
        if rm.verifier.is_empty() {
            rm.verifier = verifier; // first claim wins
        } else if rm.verifier != verifier {
            drop(rooms);
            send_to(&me, &ws::close_frame(4401));
            return;
        }
        let slot = if role == "pc" { &mut rm.pc } else { &mut rm.phone };
        let old = slot.replace(me.clone());
        (if role == "pc" { rm.phone.clone() } else { rm.pc.clone() }, old)
    };
    if let Some(o) = old {
        send_to(&o, &ws::close_frame(4409));
        let _ = o.w.lock().unwrap().shutdown(Shutdown::Both);
    }
    if role == "pc" {
        st.joins_pc.fetch_add(1, Ordering::SeqCst);
    }
    send_to(&me, &ws::text_frame(&json!({ "t": "joined", "peer": other.is_some() }).to_string()));
    if let Some(o) = &other {
        send_to(o, &ws::text_frame(&json!({ "t": "peer", "up": true }).to_string()));
    }
    let _ = r.get_ref().set_read_timeout(None);
    loop {
        match ws::read_msg(&mut r) {
            Ok(ws::Msg::Bin(b)) => {
                let o = {
                    let rooms = st.rooms.lock().unwrap();
                    let rm = rooms.get(&room).unwrap();
                    if role == "pc" { rm.phone.clone() } else { rm.pc.clone() }
                };
                if let Some(o) = o {
                    st.tap.lock().unwrap().push(b.clone());
                    send_to(&o, &ws::binary_frame(&b));
                }
            }
            Ok(ws::Msg::Ping(p)) => send_to(&me, &ws::pong_frame(&p)),
            Ok(ws::Msg::Text(_)) | Ok(ws::Msg::Pong) => {}
            Ok(ws::Msg::Close(_)) => {
                send_to(&me, &ws::close_frame(1000)); // like a real server: answer the close, then drop the socket
                let _ = me.w.lock().unwrap().shutdown(Shutdown::Both);
                break;
            }
            Err(_) => break,
        }
    }
    let other = {
        let mut rooms = st.rooms.lock().unwrap();
        let rm = rooms.get_mut(&room).unwrap();
        let slot = if role == "pc" { &mut rm.pc } else { &mut rm.phone };
        if slot.as_ref().is_some_and(|p| p.id == me.id) {
            *slot = None;
            if role == "pc" { rm.phone.clone() } else { rm.pc.clone() }
        } else {
            None // replaced by a newer socket of the same role: nobody is told
        }
    };
    if let Some(o) = other {
        send_to(&o, &ws::text_frame(&json!({ "t": "peer", "up": false }).to_string()));
    }
}

// ------------------------------------------------------------------------------------------------ the fake phone

#[derive(Clone)]
struct PhoneKeys {
    p2c: [u8; 32],
    c2p: [u8; 32],
    room: String,
    join: String,
    token: String,
}

/// Pairs over the LAN endpoint like the app does and derives everything on its own side with raw RustCrypto calls.
fn pair_v2(r: &Rig, device: &str, code_override: Option<&str>) -> (PhoneKeys, Value) {
    use hkdf::Hkdf;
    use hmac::{Hmac, Mac};
    use x25519_dalek::{PublicKey, StaticSecret};
    let code = code_override.map(String::from).unwrap_or_else(|| r.sh().pair_code(false)["code"].as_str().unwrap().to_string());
    let sk = StaticSecret::from(<[u8; 32]>::try_from(reg::random_bytes(32)).unwrap());
    let pk = PublicKey::from(&sk);
    let body = json!({ "code": code, "deviceId": device, "name": "Pixel 8", "model": "Pixel 8", "os": "Android 15", "appVersion": "1.1.0", "pk": B64.encode(pk.as_bytes()) });
    let (st, _, resp) = http(r.addr(), "POST", "/api/pair", &[], body.to_string().as_bytes());
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&resp));
    let v: Value = serde_json::from_slice(&resp).unwrap();
    let pc_pk: [u8; 32] = B64.decode(v["pk"].as_str().expect("the PC answers with its public key")).unwrap().try_into().unwrap();
    let shared = sk.diffie_hellman(&PublicKey::from(pc_pk));
    let salt = Sha256::digest([b"pill-phone-v2".as_slice(), code.as_bytes(), device.as_bytes()].concat());
    let mut okm = [0u8; 96];
    Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes()).expand(b"keys", &mut okm).unwrap();
    let mac = |m: &[u8]| {
        let mut h = <Hmac<Sha256> as Mac>::new_from_slice(&okm[64..]).unwrap();
        h.update(m);
        h.finalize().into_bytes()
    };
    let room: String = mac(b"room")[..16].iter().map(|b| format!("{b:02x}")).collect();
    let join = B64.encode(mac(b"join"));
    let keys = PhoneKeys { p2c: okm[..32].try_into().unwrap(), c2p: okm[32..64].try_into().unwrap(), room, join, token: v["token"].as_str().unwrap().to_string() };
    (keys, v)
}

struct FakePhone {
    chan: Arc<Chan>,
    k: PhoneKeys,
    sid: String,
    seq: u64,
    /// `peer` notices from the relay, in order
    peers: Vec<bool>,
    joined_peer: Option<bool>,
}

impl FakePhone {
    fn connect(relay: &FakeRelay, k: PhoneKeys) -> FakePhone {
        let t = Target { tls: false, host: "127.0.0.1".into(), port: relay.addr.port(), base: String::new() };
        let chan = Arc::new(Chan::connect(&t, &format!("/r/{}?role=phone", k.room)).expect("the fake relay accepts the phone"));
        chan.send_text(&json!({ "t": "join", "join": k.join, "v": 2 }).to_string()).unwrap();
        chan.set_read_timeout(Duration::from_secs(5));
        let mut p = FakePhone { chan, k, sid: "feedfacecafebeef".into(), seq: 0, peers: vec![], joined_peer: None };
        while p.joined_peer.is_none() {
            assert!(p.next_raw(Duration::from_secs(5)).is_none());
        }
        p
    }

    fn aad(&self, dir: &str) -> Vec<u8> {
        [b"pill-v2".as_slice(), self.k.room.as_bytes(), dir.as_bytes()].concat()
    }

    fn seal(&self, key: &[u8; 32], dir: &str, plain: &[u8]) -> Vec<u8> {
        let nonce = reg::random_bytes(24);
        let ct = XChaCha20Poly1305::new(key.into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad: &self.aad(dir) }).unwrap();
        [nonce, ct].concat()
    }

    /// An encrypted frame for message `m`, with the next sequence number (not yet sent).
    fn frame(&mut self, m: &Value) -> Vec<u8> {
        self.seq += 1;
        let env = format!("{{\"sid\":\"{}\",\"seq\":{},\"m\":{}}}", self.sid, self.seq, m);
        self.seal(&self.k.p2c.clone(), "p2c", env.as_bytes())
    }

    fn send(&mut self, m: Value) {
        let f = self.frame(&m);
        self.chan.send_binary(&f).unwrap();
    }

    fn send_raw(&self, frame: &[u8]) {
        self.chan.send_binary(frame).unwrap();
    }

    /// The next relay message: control messages are recorded and return None; a binary frame is decrypted (c2p) and returned.
    /// `None` also when nothing came within `wait`. Panics on a frame that does not authenticate.
    fn next_raw(&mut self, wait: Duration) -> Option<Value> {
        self.chan.set_read_timeout(wait);
        let mut rd = Patient::new(&self.chan);
        match ws::read_msg_as(&mut rd, false) {
            Ok(ws::Msg::Bin(b)) => {
                let plain = XChaCha20Poly1305::new((&self.k.c2p).into()).decrypt(XNonce::from_slice(&b[..24]), Payload { msg: &b[24..], aad: &self.aad("c2p") }).expect("a frame from the PC must authenticate");
                let env: Value = serde_json::from_slice(&plain).unwrap();
                assert!(env["sid"].is_string() && env["seq"].is_u64(), "{env}");
                Some(env)
            }
            Ok(ws::Msg::Text(t)) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                match v["t"].as_str() {
                    Some("joined") => {
                        self.joined_peer = v["peer"].as_bool();
                        if self.joined_peer == Some(true) {
                            self.peers.push(true); // the other side was already there
                        }
                    }
                    Some("peer") => self.peers.push(v["up"].as_bool().unwrap()),
                    _ => {}
                }
                None
            }
            Ok(ws::Msg::Close(c)) => panic!("relay closed the phone socket ({c})"),
            Ok(_) => None,
            Err(e) if super::wsclient::is_timeout(&e) => None,
            Err(e) => panic!("phone socket error: {e}"),
        }
    }

    /// The next PC message (an envelope's `m`), waiting up to 5 s.
    fn recv(&mut self) -> Value {
        let t = Instant::now();
        loop {
            if let Some(env) = self.next_raw(Duration::from_millis(200)) {
                return env["m"].clone();
            }
            assert!(t.elapsed() < Duration::from_secs(5), "no message from the PC arrived");
        }
    }

    fn recv_t(&mut self, t: &str) -> Value {
        loop {
            let m = self.recv();
            if m["t"] == t {
                return m;
            }
        }
    }

    /// True when no PC message arrives within `wait`.
    fn quiet(&mut self, wait: Duration) -> bool {
        let t = Instant::now();
        while t.elapsed() < wait {
            if self.next_raw(Duration::from_millis(50)).is_some() {
                return false;
            }
        }
        true
    }

    fn wait_peer(&mut self, up: bool, within: Duration) {
        let t = Instant::now();
        while self.peers.last() != Some(&up) {
            self.next_raw(Duration::from_millis(100));
            assert!(t.elapsed() < within, "the relay never reported peer up={up}");
        }
    }

    /// A round trip that proves everything sent before it was handled (one reader thread handles frames in order).
    fn sync(&mut self) {
        self.send(json!({ "t": "ping" }));
        self.recv_t("pong");
    }
}

fn notif(key: &str) -> Value {
    json!({ "t": "notif.posted", "key": key, "pkg": "org.telegram.messenger", "app": "Telegram", "title": "Вадим", "text": "привет, как дела? секретное слово: ЗЕЛЁНЫЙ-ЯК", "when": 1700000000000i64, "category": "msg", "canReply": true, "actions": [] })
}

/// A rig whose relay is the fake one, one phone paired with v2 keys and joined to the room.
struct Env {
    relay: FakeRelay,
    rig: Rig,
    phone: FakePhone,
}

impl Env {
    fn new() -> Env {
        let relay = FakeRelay::start();
        let rig = Rig::new(true);
        {
            let mut c = rig.cfg.lock().unwrap();
            c.relay_on = true;
            c.relay_url = relay.url();
        }
        let (keys, _) = pair_v2(&rig, "dev-1", None);
        let mut phone = FakePhone::connect(&relay, keys);
        // the PC's connector joins by itself (pairing synced it) and the phone is greeted
        phone.wait_peer(true, Duration::from_secs(5));
        assert_eq!(phone.recv_t("hello")["pcName"], "TEST-PC");
        Env { relay, rig, phone }
    }
    fn sh(&self) -> &Shared {
        self.rig.sh()
    }
    fn stop(self) {
        let Env { relay, rig, phone } = self;
        drop(phone);
        rig.stop();
        relay.stop();
    }
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(8), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ------------------------------------------------------------------------------------------------ pairing

#[test]
fn pairing_makes_the_keys_on_both_sides_and_protects_them_on_disk() {
    let relay = FakeRelay::start();
    let r = Rig::new(true);
    {
        let mut c = r.cfg.lock().unwrap();
        c.relay_on = true;
        c.relay_url = format!("{}/", relay.url()); // a trailing slash is tolerated
    }
    let (k, resp) = pair_v2(&r, "dev-1", None);
    // the redundant relay block matches what the phone derived on its own
    assert_eq!(resp["relay"], json!({ "url": relay.url(), "room": k.room, "join": k.join }));
    assert_eq!(resp["token"].as_str().unwrap().len(), 43);
    // keys on disk: DPAPI blob, never the key bytes in any common encoding
    let text = std::fs::read_to_string(r.dir.join("data").join("devices.json")).unwrap();
    assert!(text.contains("\"keys\"") && !text.contains(&B64.encode(k.p2c)) && !text.contains(&STANDARD.encode(k.p2c)) && !text.contains(&k.join) && !text.contains(&k.room));
    // they survive a restart of the registry, and a v1 device (no `pk`) has none
    let dev = super::reg::Registry::load(r.dir.join("data")).get("dev-1").unwrap();
    assert_eq!(dev.keys.as_ref().map(|x| (x.p2c, x.c2p)), Some((k.p2c, k.c2p)));
    let code = r.sh().pair_code(true)["code"].as_str().unwrap().to_string();
    let (st, _, b) = http(r.addr(), "POST", "/api/pair", &[], json!({ "code": code, "deviceId": "old-1", "name": "Old phone" }).to_string().as_bytes());
    let v: Value = serde_json::from_slice(&b).unwrap();
    assert_eq!(st, 200);
    assert!(v.get("pk").is_none() && v.get("relay").is_none(), "a v1 pairing gets no v2 fields");
    assert!(r.sh().registry().get("old-1").unwrap().keys.is_none());
    // a bad or low-order key is refused WITHOUT spending the code
    let code = r.sh().pair_code(true)["code"].as_str().unwrap().to_string();
    for bad in [B64.encode([0u8; 32]), B64.encode([7u8; 5]), "!!!".to_string()] {
        let (st, _, _) = http(r.addr(), "POST", "/api/pair", &[], json!({ "code": code, "deviceId": "dev-9", "pk": bad }).to_string().as_bytes());
        assert_eq!(st, 400, "{bad}");
    }
    let (k2, _) = pair_v2(&r, "dev-9", Some(&code));
    assert_eq!(k2.room.len(), 32, "the code was still good");
    // relay switched off: the keys are made anyway but no relay block is returned
    r.cfg.lock().unwrap().relay_on = false;
    let (_, v) = pair_v2(&r, "dev-3", None);
    assert!(v.get("relay").is_none() && v["pk"].is_string());
    r.stop();
    relay.stop();
}

// ------------------------------------------------------------------------------------------------ messages

#[test]
fn encrypted_notification_arrives_and_an_encrypted_reply_goes_back() {
    let mut e = Env::new();
    assert_eq!(e.sh().route_of("dev-1"), Some(Route::Relay));
    e.phone.send(notif("0|org.telegram|1"));
    let n = e.rig.wait("phone:notif", |v| v["kind"] == "posted");
    assert_eq!((n["notif"]["title"].as_str(), n["notif"]["app"].as_str()), (Some("Вадим"), Some("Telegram")));
    // the Phone tab sees the device and the route in use
    let d = e.sh().devices();
    assert_eq!((d[0]["connected"].as_bool(), d[0]["route"].as_str(), d[0]["relay"].as_bool()), (Some(true), Some("relay"), Some(true)));
    // reply: PC -> phone through the relay, encrypted
    e.sh().notif_action("0|org.telegram|1", "reply", "хорошо").unwrap();
    let m = e.phone.recv_t("notif.reply");
    assert_eq!((m["key"].as_str(), m["text"].as_str()), (Some("0|org.telegram|1"), Some("хорошо")));
    e.phone.send(json!({ "t": "reply.result", "req": m["req"], "ok": true }));
    let rep = e.rig.wait("phone:notif", |v| v["kind"] == "reply");
    assert_eq!(rep["ok"], true);
    // status and ping work like on the LAN
    e.phone.send(json!({ "t": "status", "battery": 61, "charging": true, "network": "mobile", "dnd": false, "time": 1 }));
    e.phone.sync();
    assert_eq!(e.sh().devices()[0]["battery"], 61);
    // the relay saw ciphertext only
    let tap = e.relay.st.tap.lock().unwrap().clone();
    assert!(tap.len() >= 5);
    for f in &tap {
        let hay = String::from_utf8_lossy(f);
        assert!(!hay.contains("ЗЕЛЁНЫЙ") && !hay.contains("notif") && !hay.contains("хорошо"), "plaintext leaked to the relay");
        assert!(f.len() >= 24 + 16);
    }
    e.stop();
}

#[test]
fn replayed_reordered_tampered_and_reflected_frames_are_dropped() {
    let mut e = Env::new();
    let f1 = e.phone.frame(&notif("k1"));
    e.phone.send_raw(&f1);
    e.rig.wait("phone:notif", |v| v["kind"] == "posted" && v["notif"]["key"] == "k1");
    // the very same frame again (a replay): ignored, even though it authenticates
    e.phone.send_raw(&f1);
    // an older sequence number under the same sid, freshly encrypted: ignored too
    let f_old = {
        let env = format!("{{\"sid\":\"{}\",\"seq\":1,\"m\":{}}}", e.phone.sid, notif("k-old"));
        let k = e.phone.k.p2c;
        e.phone.seal(&k, "p2c", env.as_bytes())
    };
    e.phone.send_raw(&f_old);
    // a flipped bit anywhere: fails authentication
    let mut f2 = e.phone.frame(&notif("k-tampered"));
    let last = f2.len() - 1;
    f2[last] ^= 0x01;
    e.phone.send_raw(&f2);
    let mut f3 = e.phone.frame(&notif("k-nonce"));
    f3[3] ^= 0x80;
    e.phone.send_raw(&f3);
    // a frame sealed with the other direction's key (reflection of the PC's own traffic)
    let f4 = {
        e.phone.seq += 1;
        let env = format!("{{\"sid\":\"{}\",\"seq\":{},\"m\":{}}}", e.phone.sid, e.phone.seq, notif("k-reflect"));
        let k = e.phone.k.c2p;
        e.phone.seal(&k, "c2p", env.as_bytes())
    };
    e.phone.send_raw(&f4);
    // a frame of a good envelope with a wrong room in the AAD
    let f5 = {
        e.phone.seq += 1;
        let env = format!("{{\"sid\":\"{}\",\"seq\":{},\"m\":{}}}", e.phone.sid, e.phone.seq, notif("k-room"));
        let nonce = reg::random_bytes(24);
        let aad = [b"pill-v2".as_slice(), "0".repeat(32).as_bytes(), b"p2c"].concat();
        let ct = XChaCha20Poly1305::new((&e.phone.k.p2c).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: env.as_bytes(), aad: &aad }).unwrap();
        [nonce, ct].concat()
    };
    e.phone.send_raw(&f5);
    e.phone.send_raw(&[1, 2, 3]); // garbage
    // a good frame after all that still works, and it is the only second notification
    e.phone.send(notif("k2"));
    e.phone.sync();
    let keys: Vec<String> = e.rig.events.lock().unwrap().iter().filter(|(n, v)| n == "phone:notif" && v["kind"] == "posted").map(|(_, v)| v["notif"]["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(keys, ["k1", "k2"], "only the two honest, fresh frames got through");
    // a NEW connection of the phone (a new sid) starts its numbering over
    e.phone.sid = "0123456789abcdef".into();
    e.phone.seq = 0;
    e.phone.send(notif("k3"));
    e.phone.sync();
    assert!(e.sh().notifs().iter().any(|n| n["key"] == "k3"));
    e.stop();
}

#[test]
fn fifty_bad_frames_a_minute_close_the_connection_and_it_comes_back() {
    let mut e = Env::new();
    let before = e.relay.pc_joins();
    for i in 0..50 {
        e.phone.send_raw(&[i as u8; 64]); // random-looking junk that cannot authenticate
    }
    e.phone.wait_peer(false, Duration::from_secs(5)); // the PC hung up
    // the connector reconnects by itself after its back-off (1 s) and the phone is greeted again
    e.phone.wait_peer(true, Duration::from_secs(8));
    assert_eq!(e.phone.recv_t("hello")["serverVersion"], 1);
    assert!(e.relay.pc_joins() > before);
    e.phone.send(notif("after"));
    e.rig.wait("phone:notif", |v| v["kind"] == "posted" && v["notif"]["key"] == "after");
    e.stop();
}

#[test]
fn a_dropped_network_reconnects_with_backoff() {
    let mut e = Env::new();
    let before = e.relay.pc_joins();
    e.relay.kill_all();
    wait_for("the PC joins the relay again", || e.relay.pc_joins() > before);
    // the fake phone's socket was cut too: it rejoins, and both sides see each other
    e.phone = FakePhone::connect(&e.relay, e.phone.k.clone());
    e.phone.wait_peer(true, Duration::from_secs(5));
    assert_eq!(e.phone.recv_t("hello")["t"], "hello");
    e.phone.send(notif("again"));
    e.rig.wait("phone:notif", |v| v["kind"] == "posted" && v["notif"]["key"] == "again");
    e.stop();
}

// ------------------------------------------------------------------------------------------------ routes

#[test]
fn the_lan_is_preferred_and_the_relay_stays_as_standby() {
    let mut e = Env::new();
    let tok = {
        // the token of the paired device: pair again over LAN would replace keys, so read it from the pairing response of Env? the fake phone kept it
        e.phone.k.token.clone()
    };
    assert_eq!(e.sh().route_of("dev-1"), Some(Route::Relay));
    let mut lan = Phone::connect(e.rig.addr(), &tok);
    lan.recv_t("hello");
    // a phone that paired before the relay was set up learns its address over the LAN (optional message)
    let rc = lan.recv_t("relay.config");
    assert_eq!((rc["url"].as_str(), rc["room"].as_str(), rc["join"].as_str()), (Some(e.relay.url().as_str()), Some(e.phone.k.room.as_str()), Some(e.phone.k.join.as_str())));
    wait_for("LAN route", || e.sh().route_of("dev-1") == Some(Route::Lan));
    assert_eq!(e.sh().devices()[0]["route"], "lan");
    // a notification arrives over the relay, a command goes out over the LAN socket only
    e.phone.send(notif("n1"));
    e.rig.wait("phone:notif", |v| v["kind"] == "posted");
    e.sh().notif_action("n1", "dismiss", "").unwrap();
    assert_eq!(lan.recv_t("notif.dismiss")["key"], "n1");
    assert!(e.phone.quiet(Duration::from_millis(300)), "nothing was sent through the standby relay");
    // the LAN socket goes away: the same call now uses the relay, and the phone stays "connected"
    drop(lan);
    wait_for("relay route again", || e.sh().route_of("dev-1") == Some(Route::Relay));
    e.phone.send(notif("n2"));
    e.rig.wait("phone:notif", |v| v["kind"] == "posted" && v["notif"]["key"] == "n2");
    e.sh().notif_action("n2", "read", "").unwrap();
    assert_eq!(e.phone.recv_t("notif.read")["key"], "n2");
    assert_eq!(e.sh().devices()[0]["connected"], true);
    e.stop();
}

#[test]
fn an_old_v1_device_works_over_the_lan_and_never_touches_the_relay() {
    let relay = FakeRelay::start();
    let r = Rig::new(true);
    {
        let mut c = r.cfg.lock().unwrap();
        c.relay_on = true;
        c.relay_url = relay.url();
    }
    let tok = r.pair("old-1"); // no `pk`
    r.sh().relay_sync();
    assert_eq!(r.sh().relay.count(), 0, "no keys, no relay connection");
    let mut p = Phone::connect(r.addr(), &tok);
    p.recv_t("hello");
    p.send(notif("lan1"));
    r.wait("phone:notif", |v| v["kind"] == "posted");
    assert_eq!(r.sh().devices()[0]["route"], "lan");
    assert_eq!(relay.pc_joins(), 0);
    drop(p);
    wait_for("offline", || r.sh().devices()[0]["route"] == "none");
    r.stop();
    relay.stop();
}

#[test]
fn switches_and_revocation_tear_the_connection_down() {
    let mut e = Env::new();
    assert_eq!(e.sh().relay.count(), 1);
    // phone.relay off: the connection ends at once and nothing reconnects
    e.rig.cfg.lock().unwrap().relay_on = false;
    e.sh().relay_sync();
    e.phone.wait_peer(false, Duration::from_secs(3));
    assert_eq!(e.sh().relay.count(), 0);
    wait_for("the phone is offline", || e.sh().route_of("dev-1").is_none());
    let joins = e.relay.pc_joins();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(e.relay.pc_joins(), joins, "no reconnect while the switch is off");
    // back on: it connects again
    e.rig.cfg.lock().unwrap().relay_on = true;
    e.sh().relay_sync();
    e.phone.wait_peer(true, Duration::from_secs(5));
    // an empty URL counts as off
    e.rig.cfg.lock().unwrap().relay_url = String::new();
    e.sh().relay_sync();
    e.phone.wait_peer(false, Duration::from_secs(3));
    e.rig.cfg.lock().unwrap().relay_url = e.relay.url();
    e.sh().relay_sync();
    e.phone.wait_peer(true, Duration::from_secs(5));
    // revoking the device: the socket is closed and never comes back, the keys are gone from disk
    let id = e.sh().devices()[0]["id"].as_str().unwrap().to_string();
    assert!(e.sh().revoke(&id));
    e.phone.wait_peer(false, Duration::from_secs(3));
    assert_eq!(e.sh().relay.count(), 0);
    let joins = e.relay.pc_joins();
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(e.relay.pc_joins(), joins, "a revoked device is not reconnected");
    assert!(super::reg::Registry::load(e.rig.dir.join("data")).get(&id).is_none());
    // frames of the revoked phone are not processed even if the room were still there
    e.phone.send(notif("ghost"));
    assert!(!e.rig.events.lock().unwrap().iter().any(|(_, v)| v["notif"]["key"] == "ghost"));
    e.stop();
}

#[test]
fn stopping_the_server_closes_the_relay_socket_and_a_wrong_join_is_refused() {
    let relay = FakeRelay::start();
    // somebody else already claimed the room with a different join secret: 4401, retried politely
    let r = Rig::new(true);
    {
        let mut c = r.cfg.lock().unwrap();
        c.relay_on = true;
        c.relay_url = relay.url();
    }
    // pair first with the relay off so the connector does not start yet, then squat the room
    r.cfg.lock().unwrap().relay_on = false;
    let (k, _) = pair_v2(&r, "dev-1", None);
    let t = Target { tls: false, host: "127.0.0.1".into(), port: relay.addr.port(), base: String::new() };
    let squat = Chan::connect(&t, &format!("/r/{}?role=phone", k.room)).unwrap();
    squat.send_text(&json!({ "t": "join", "join": "not-the-secret", "v": 2 }).to_string()).unwrap();
    squat.set_read_timeout(Duration::from_secs(5));
    let _ = ws::read_msg_as(&mut Patient::new(&Arc::new(squat)), false);
    r.cfg.lock().unwrap().relay_on = true;
    r.sh().relay_sync();
    wait_for("the PC tried to join", || relay.st.rooms.lock().unwrap().contains_key(&k.room));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(relay.pc_joins(), 0, "wrong secret: never joined");
    // now the real thing: stop() ends the thread and closes its socket
    let relay2 = FakeRelay::start();
    let r2 = Rig::new(true);
    {
        let mut c = r2.cfg.lock().unwrap();
        c.relay_on = true;
        c.relay_url = relay2.url();
    }
    let (k2, _) = pair_v2(&r2, "dev-2", None);
    let mut ph = FakePhone::connect(&relay2, k2);
    ph.wait_peer(true, Duration::from_secs(5));
    r2.stop();
    ph.wait_peer(false, Duration::from_secs(3));
    r.stop();
    relay.stop();
    relay2.stop();
}

// ------------------------------------------------------------------------------------------------ files

fn data(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i * 7 + i / 251) % 256) as u8).collect()
}

#[test]
fn a_file_goes_phone_to_pc_in_chunks_with_acks() {
    let mut e = Env::new();
    let src = data(96 * 1024 * 2 + 5000);
    e.phone.send(json!({ "t": "file.offer", "id": "up1", "name": "../../trip\\photo.jpg", "size": src.len(), "mime": "image/jpeg" }));
    assert_eq!(e.phone.recv_t("file.accept")["id"], "up1");
    // a chunk that skips ahead aborts the transfer (no hole in the file) -- on another id
    e.phone.send(json!({ "t": "file.offer", "id": "gap1", "name": "g.bin", "size": 10 }));
    e.phone.recv_t("file.accept");
    e.phone.send(json!({ "t": "file.chunk", "id": "gap1", "off": 0, "d": STANDARD.encode([1u8; 4]) }));
    e.phone.send(json!({ "t": "file.chunk", "id": "gap1", "off": 6, "d": STANDARD.encode([1u8; 4]) }));
    assert_eq!(e.phone.recv_t("file.abort")["reason"], "gap");
    assert_eq!(e.sh().transfers.get("gap1").unwrap().status, Status::Failed);
    assert!(!std::fs::read_dir(e.rig.dir.join("dl")).unwrap().any(|f| f.unwrap().file_name() == "g.bin"), "the partial file is deleted");
    // the real one: chunks of 96 KiB (the phone's own flow control: 8 in flight), acks come back
    let mut sent = 0usize;
    let mut acked = 0usize;
    while sent < src.len() {
        let n = (src.len() - sent).min(96 * 1024);
        e.phone.send(json!({ "t": "file.chunk", "id": "up1", "off": sent, "d": STANDARD.encode(&src[sent..sent + n]) }));
        sent += n;
        while let Some(env) = e.phone.next_raw(Duration::from_millis(1)) {
            if env["m"]["t"] == "file.ack" {
                acked = acked.max(env["m"]["upto"].as_u64().unwrap() as usize);
            }
        }
    }
    e.phone.send(json!({ "t": "file.end", "id": "up1", "size": src.len() }));
    loop {
        let m = e.phone.recv_t("file.ack");
        acked = acked.max(m["upto"].as_u64().unwrap() as usize);
        if acked == src.len() {
            break;
        }
    }
    let done = e.rig.wait("phone:files", |v| v["kind"] == "done" && v["transfer"]["id"] == "up1");
    let path = std::path::PathBuf::from(done["transfer"]["path"].as_str().unwrap());
    assert_eq!(path.parent().unwrap(), e.rig.dir.join("dl"), "same Downloads\\Pill rules as the LAN: sanitised name, inside the folder");
    assert_eq!(path.file_name().unwrap(), "photo.jpg");
    assert_eq!(std::fs::read(&path).unwrap(), src);
    assert_eq!(e.sh().devices()[0]["stats"]["filesIn"], 1);
    // a wrong final size is refused and the file removed
    e.phone.send(json!({ "t": "file.offer", "id": "up2", "name": "short.bin", "size": 100 }));
    e.phone.recv_t("file.accept");
    e.phone.send(json!({ "t": "file.chunk", "id": "up2", "off": 0, "d": STANDARD.encode([9u8; 60]) }));
    e.phone.send(json!({ "t": "file.end", "id": "up2", "size": 100 }));
    assert_eq!(e.phone.recv_t("file.abort")["reason"], "size_mismatch");
    assert!(!std::fs::read_dir(e.rig.dir.join("dl")).unwrap().any(|f| f.unwrap().file_name() == "short.bin"));
    // an empty file needs no chunk
    e.phone.send(json!({ "t": "file.offer", "id": "up3", "name": "empty.txt", "size": 0 }));
    e.phone.recv_t("file.accept");
    e.phone.send(json!({ "t": "file.end", "id": "up3", "size": 0 }));
    assert_eq!(e.phone.recv_t("file.ack")["upto"], 0);
    assert!(e.rig.dir.join("dl").join("empty.txt").exists());
    e.stop();
}

#[test]
fn the_relay_size_cap_applies_both_ways_but_not_to_the_lan() {
    let mut e = Env::new();
    e.rig.cfg.lock().unwrap().relay_max_bytes = 50_000;
    e.phone.send(json!({ "t": "file.offer", "id": "big", "name": "movie.mkv", "size": 60_000 }));
    let rej = e.phone.recv_t("file.reject");
    assert_eq!((rej["id"].as_str(), rej["reason"].as_str()), (Some("big"), Some("too_big")));
    let f = e.rig.dir.join("pc.bin");
    std::fs::write(&f, data(60_000)).unwrap();
    let err = e.sh().send_file(&f, None).unwrap_err();
    assert!(err.contains("relay"), "{err}");
    // the same file is fine over the LAN (the LAN cap is phone.maxFileMB, much higher)
    let mut lan = Phone::connect(e.rig.addr(), &e.phone.k.token);
    lan.recv_t("hello");
    lan.recv_t("relay.config");
    wait_for("LAN",|| e.sh().route_of("dev-1") == Some(Route::Lan));
    assert!(e.sh().send_file(&f, None).is_ok());
    assert_eq!(lan.recv_t("file.offer")["name"], "pc.bin");
    lan.send(json!({ "t": "file.offer", "id": "lanbig", "name": "ok.bin", "size": 60_000 }));
    assert_eq!(lan.recv_t("file.accept")["id"], "lanbig");
    drop(lan);
    e.stop();
}

#[test]
fn a_file_goes_pc_to_phone_with_at_most_eight_chunks_in_flight() {
    let mut e = Env::new();
    let src = data(96 * 1024 * 12 + 777);
    let f = e.rig.dir.join("Report.pdf");
    std::fs::write(&f, &src).unwrap();
    let t = e.sh().send_file(&f, None).unwrap();
    let id = t["id"].as_str().unwrap().to_string();
    let offer = e.phone.recv_t("file.offer");
    assert_eq!((offer["id"].as_str(), offer["name"].as_str(), offer["size"].as_u64()), (Some(id.as_str()), Some("Report.pdf"), Some(src.len() as u64)));
    e.phone.send(json!({ "t": "file.accept", "id": id }));
    // the PC sends 8 chunks, then must wait for an ack
    let mut got: Vec<(u64, Vec<u8>)> = vec![];
    for _ in 0..8 {
        let m = e.phone.recv_t("file.chunk");
        got.push((m["off"].as_u64().unwrap(), STANDARD.decode(m["d"].as_str().unwrap()).unwrap()));
    }
    assert_eq!(got[7].0, 7 * 96 * 1024);
    assert!(e.phone.quiet(Duration::from_millis(600)), "a 9th chunk came before any ack: the window is 8");
    // ack the first four chunks: exactly room for four more, then it stops again
    e.phone.send(json!({ "t": "file.ack", "id": id, "upto": 4 * 96 * 1024 }));
    let mut more = 0;
    let mut end = None;
    loop {
        let Some(env) = e.phone.next_raw(Duration::from_millis(600)) else { break };
        let m = env["m"].clone();
        match m["t"].as_str() {
            Some("file.chunk") => {
                got.push((m["off"].as_u64().unwrap(), STANDARD.decode(m["d"].as_str().unwrap()).unwrap()));
                more += 1;
            }
            Some("file.end") => end = Some(m),
            _ => {}
        }
    }
    assert_eq!(more, 4, "four acked chunks free exactly four slots");
    assert!(end.is_none());
    // now ack everything received so far; the rest follows, then the end message
    e.phone.send(json!({ "t": "file.ack", "id": id, "upto": got.iter().map(|(o, d)| o + d.len() as u64).max().unwrap() }));
    let t0 = Instant::now();
    let end = loop {
        assert!(t0.elapsed() < Duration::from_secs(10), "the transfer did not finish");
        let env = e.phone.next_raw(Duration::from_millis(300)).unwrap_or(Value::Null);
        let m = env["m"].clone();
        if m["t"] == "file.chunk" {
            got.push((m["off"].as_u64().unwrap(), STANDARD.decode(m["d"].as_str().unwrap()).unwrap()));
            let upto = got.iter().map(|(o, d)| o + d.len() as u64).max().unwrap();
            e.phone.send(json!({ "t": "file.ack", "id": id, "upto": upto }));
        } else if m["t"] == "file.end" {
            break m;
        }
    };
    assert_eq!(end["size"].as_u64(), Some(src.len() as u64));
    got.sort_by_key(|(o, _)| *o);
    let whole: Vec<u8> = got.iter().flat_map(|(_, d)| d.clone()).collect();
    assert_eq!(whole, src, "the phone reassembled the file byte for byte");
    // the phone confirms; the PC counts it
    e.phone.send(json!({ "t": "file.done", "id": id, "ok": true }));
    e.rig.wait("phone:files", |v| v["kind"] == "sent");
    assert_eq!(e.sh().devices()[0]["stats"]["filesOut"], 1);
    // an abort from the phone stops a running transfer
    let f2 = e.rig.dir.join("Big.bin");
    std::fs::write(&f2, data(96 * 1024 * 30)).unwrap();
    let id2 = e.sh().send_file(&f2, None).unwrap()["id"].as_str().unwrap().to_string();
    e.phone.recv_t("file.offer");
    e.phone.send(json!({ "t": "file.accept", "id": id2 }));
    e.phone.recv_t("file.chunk");
    e.phone.send(json!({ "t": "file.abort", "id": id2, "reason": "user" }));
    wait_for("the PC stopped sending", || e.sh().transfers.get(&id2).unwrap().status == Status::Failed);
    e.stop();
}

#[test]
fn a_lost_relay_link_fails_the_running_transfers() {
    let mut e = Env::new();
    let src = data(96 * 1024 * 4);
    e.phone.send(json!({ "t": "file.offer", "id": "cut", "name": "cut.bin", "size": src.len() }));
    e.phone.recv_t("file.accept");
    e.phone.send(json!({ "t": "file.chunk", "id": "cut", "off": 0, "d": STANDARD.encode(&src[..96 * 1024]) }));
    e.rig.wait("phone:files", |v| v["kind"] == "start");
    let part = std::fs::read_dir(e.rig.dir.join("dl")).unwrap().next().unwrap().unwrap().path();
    assert!(part.exists());
    // the phone vanishes from the relay
    e.phone.chan.kill();
    wait_for("transfer failed", || e.sh().transfers.get("cut").unwrap().status == Status::Failed);
    assert!(!part.exists(), "the partial file is removed");
    e.stop();
}

#[test]
fn http_still_needs_the_token_for_the_lan_file_routes() {
    // the relay does not weaken the LAN rules: a file route without a token is refused as before
    let e = Env::new();
    assert_eq!(http(e.rig.addr(), "GET", "/api/down/abc", &[], b"").0, 401);
    assert_eq!(http(e.rig.addr(), "GET", "/api/down/abc", &bearer(&e.phone.k.token), b"").0, 404);
    e.stop();
}

#[test]
fn the_clipboard_travels_through_the_relay_encrypted_in_both_directions() {
    let mut e = Env::new();
    e.rig.cfg.lock().unwrap().clip = super::clip::ClipCfg { on: true, to_phone: true, from_phone: true };
    e.sh().clip_sync();
    assert!(!e.rig.clip.watching.load(std::sync::atomic::Ordering::SeqCst), "no listener before the phone announced the capability");
    assert_eq!(e.sh().clip_local("before caps ЛИМОННЫЙ"), 0);
    e.phone.send(json!({ "t": "caps", "clip": 1 }));
    wait_for("the listener", || e.rig.clip.watching.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(e.sh().clip_local("пароль-не-для-ретранслятора"), 1);
    assert_eq!(e.phone.recv_t("clip")["text"], "пароль-не-для-ретранслятора");
    e.phone.send(json!({ "t": "clip", "id": "r1", "text": "ЛИМОННЫЙ-ТЕКСТ", "ts": 1 }));
    e.rig.wait("phone:clip", |v| v["name"] == "Pixel 8");
    assert_eq!(e.rig.clip.text.lock().unwrap().as_deref(), Some("ЛИМОННЫЙ-ТЕКСТ"));
    // the relay saw ciphertext only
    for f in e.relay.st.tap.lock().unwrap().iter() {
        let hay = String::from_utf8_lossy(f);
        assert!(!hay.contains("ЛИМОННЫЙ") && !hay.contains("пароль") && !hay.contains("clip"), "plaintext leaked to the relay");
    }
    e.stop();
}
