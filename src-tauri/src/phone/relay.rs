// The relay connector (docs/phone-relay.md): one outbound WebSocket per paired phone that has v2 keys, only while "Phone link" and `phone.relay`
// are on and `phone.relayUrl` is set. Frames are end-to-end encrypted (v2.rs); what the phone sends is fed into the same handler the LAN socket
// uses (`Shared::on_value`) and replies go out through the same `Conn::send_text`, so notifications, replies, status and files work unchanged.
// Threads: one per phone, blocked on its socket (nothing polls); they exist only while the switches are on. Nothing here logs keys or message text.
use super::server::{Conn, Shared};
use super::v2::{self, AuthFails, Dir, Keys, Replay};
use super::wsclient::{is_timeout, Chan, Patient};
use super::ws::{self, Msg};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use super::wsclient::Target;

/// The relay is pinged this often to keep NATs open (also the idle read timeout, which doubles as the tick for housekeeping).
const PING_EVERY: Duration = Duration::from_secs(25);
/// If the relay ever answered a ping, this long without any frame means the path is dead.
const DEAD_AFTER: Duration = Duration::from_secs(80);
const JOIN_WAIT: Duration = Duration::from_secs(10);
const BACKOFF_MIN: u64 = 1;
const BACKOFF_MAX: u64 = 30;

/// `wss://host[:port][/prefix]`. `https://` is read as `wss://`; plain `ws://` / `http://` only for loopback hosts (development, tests).
pub fn parse_url(s: &str) -> Option<Target> {
    let s = s.trim();
    let low = s.to_ascii_lowercase();
    let (tls, http) = if low.starts_with("wss://") {
        (true, format!("https://{}", &s[6..]))
    } else if low.starts_with("https://") {
        (true, s.to_string())
    } else if low.starts_with("ws://") {
        (false, format!("http://{}", &s[5..]))
    } else if low.starts_with("http://") {
        (false, s.to_string())
    } else {
        return None;
    };
    let u = crate::net::parse_url(&http).ok()?;
    if (!tls && !crate::net::is_loopback(&u.host)) || u.path.contains(['?', '#']) {
        return None;
    }
    Some(Target { tls, host: u.host, port: u.port, base: u.path.trim_end_matches('/').to_string() })
}

/// base64 in either alphabet, with or without padding.
pub fn b64_any(s: &str) -> Option<Vec<u8>> {
    let t = s.trim().trim_end_matches('=').replace('+', "-").replace('/', "_");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t).ok()
}

/// The PC's sending half of one relay connection: encrypts v1 messages into frames. Handed to `Conn` as its output.
pub struct Link {
    chan: Arc<Chan>,
    tx: Mutex<Tx>,
}

struct Tx {
    key: [u8; 32],
    aad: Vec<u8>,
    sid: String,
    seq: u64,
}

impl Link {
    fn new(chan: Arc<Chan>, keys: &Keys, room: &str) -> Link {
        Link { chan, tx: Mutex::new(Tx { key: keys.c2p, aad: v2::aad(room, Dir::C2p), sid: v2::new_sid(), seq: 0 }) }
    }

    /// One v1 JSON message (`text` is a JSON object) as one encrypted binary frame. The lock is held across the write so frames leave in `seq` order.
    pub fn send_text(&self, text: &str) -> bool {
        let mut tx = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        tx.seq += 1;
        let env = format!("{{\"sid\":\"{}\",\"seq\":{},\"m\":{}}}", tx.sid, tx.seq, text);
        if env.len() > v2::MAX_PLAIN {
            return false;
        }
        let frame = v2::seal(&tx.key, &tx.aad, env.as_bytes());
        self.chan.send_binary(&frame).is_ok()
    }
}

// ------------------------------------------------------------------------------------------------ the hub

struct Ctl {
    chan: Mutex<Option<Arc<Chan>>>,
    thread: Mutex<Option<std::thread::Thread>>,
    /// the registered channel of the phone, so a stop can unregister it at once (a blocked read may take a while to notice the closed socket)
    conn: Mutex<Option<Arc<Conn>>>,
}

struct Worker {
    stop: Arc<AtomicBool>,
    ctl: Arc<Ctl>,
    /// what this connection was made for: if the settings or the keys change the worker is replaced
    fp: (String, String),
    th: Option<JoinHandle<()>>,
}

impl Worker {
    fn signal(&self, sh: &Shared) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(c) = self.ctl.chan.lock().unwrap_or_else(|e| e.into_inner()).take() {
            c.close(1000); // the relay sees the close; our own blocked read ends when it answers
        }
        if let Some(c) = self.ctl.conn.lock().unwrap_or_else(|e| e.into_inner()).take() {
            sh.detach(&c);
            sh.rf_conn_gone(&c);
        }
        if let Some(t) = self.ctl.thread.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            t.unpark(); // ends a back-off sleep
        }
    }
}

/// All relay connections of one running server.
#[derive(Default)]
pub struct Hub {
    workers: Mutex<HashMap<String, Worker>>,
}

impl Hub {
    /// Brings the connections in line with the settings, the paired phones and their keys. Cheap when nothing changed.
    pub fn sync(&self, sh: &Arc<Shared>) {
        let cfg = sh.cfg();
        let target = if cfg.relay_on { parse_url(&cfg.relay_url) } else { None };
        let want: HashMap<String, (Keys, String)> = match &target {
            Some(t) => sh.registry().list().into_iter().filter_map(|d| d.keys.map(|k| (d.id, (k, t.base_url())))).collect(),
            None => HashMap::new(),
        };
        let mut ws = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        let gone: Vec<String> = ws.iter().filter(|(id, w)| want.get(*id).map_or(true, |(k, url)| w.fp != (url.clone(), v2::room(k)))).map(|(id, _)| id.clone()).collect();
        let mut old = Vec::new();
        for id in gone {
            if let Some(w) = ws.remove(&id) {
                w.signal(sh);
                old.push(w);
            }
        }
        for (id, (keys, url)) in want {
            if ws.contains_key(&id) {
                continue;
            }
            let Some(t) = target.as_ref().map(|t| Target { tls: t.tls, host: t.host.clone(), port: t.port, base: t.base.clone() }) else { continue };
            let fp = (url, v2::room(&keys));
            let (stop, ctl) = (Arc::new(AtomicBool::new(false)), Arc::new(Ctl { chan: Mutex::new(None), thread: Mutex::new(None), conn: Mutex::new(None) }));
            let (sh2, id2, stop2, ctl2) = (sh.clone(), id.clone(), stop.clone(), ctl.clone());
            let th = std::thread::Builder::new().name("phone-relay".into()).spawn(move || run(sh2, id2, t, keys, stop2, ctl2)).ok();
            ws.insert(id, Worker { stop, ctl, fp, th });
        }
        drop(ws);
        drop(old); // the replaced threads finish on their own; their sockets are already shut
    }

    /// Ends every connection and waits for the threads (the server is going away).
    pub fn stop_all(&self, sh: &Shared) {
        let all: Vec<Worker> = self.workers.lock().unwrap_or_else(|e| e.into_inner()).drain().map(|(_, w)| w).collect();
        for w in &all {
            w.signal(sh);
        }
        // a thread whose socket the relay does not answer is left to finish by itself after 2 s (it holds nothing that matters any more)
        let until = Instant::now() + Duration::from_secs(2);
        for mut w in all {
            if let Some(t) = w.th.take() {
                while !t.is_finished() && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if t.is_finished() {
                    let _ = t.join();
                }
            }
        }
    }

    /// How many phones have a relay connection thread (tests).
    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.workers.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

// ------------------------------------------------------------------------------------------------ one phone's connection

fn run(sh: Arc<Shared>, dev_id: String, target: Target, keys: Keys, stop: Arc<AtomicBool>, ctl: Arc<Ctl>) {
    *ctl.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current());
    let room = v2::room(&keys);
    let join = v2::join(&keys);
    let mut replay = Replay::default(); // survives reconnects: the phone keeps its sid while its own relay socket lives
    let mut back = BACKOFF_MIN;
    while !stop.load(Ordering::SeqCst) {
        let mut s = Session { sh: &sh, dev_id: &dev_id, keys: &keys, room: &room, stop: &stop, replay: &mut replay, ctl: &ctl };
        if s.run(&target, &join) {
            back = BACKOFF_MIN; // it got as far as being joined: start over politely
        }
        *ctl.chan.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let until = Instant::now() + Duration::from_secs(back);
        while !stop.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::park_timeout(until - Instant::now());
        }
        back = (back * 2).min(BACKOFF_MAX);
    }
}

struct Session<'a> {
    sh: &'a Arc<Shared>,
    dev_id: &'a str,
    keys: &'a Keys,
    room: &'a str,
    stop: &'a AtomicBool,
    replay: &'a mut Replay,
    ctl: &'a Ctl,
}

impl Session<'_> {
    /// One connection from connect to close. True when it got as far as `joined`.
    fn run(&mut self, target: &Target, join: &str) -> bool {
        let ctl = self.ctl;
        let Ok(chan) = Chan::connect(target, &format!("/r/{}?role=pc", self.room)).map(Arc::new) else { return false };
        *ctl.chan.lock().unwrap_or_else(|e| e.into_inner()) = Some(chan.clone());
        if self.stop.load(Ordering::SeqCst) {
            chan.kill();
            return false;
        }
        if chan.send_text(&json!({ "t": "join", "join": join, "v": 2 }).to_string()).is_err() {
            chan.kill();
            return false;
        }
        // the relay answers `joined`, or closes with 4401
        chan.set_read_timeout(JOIN_WAIT);
        let peer = loop {
            match ws::read_msg_as(&mut Patient::new(&chan), false) {
                Ok(Msg::Text(t)) => match serde_json::from_str::<Value>(&t) {
                    Ok(v) if v["t"] == "joined" => break v["peer"].as_bool().unwrap_or(false),
                    _ => continue,
                },
                Ok(Msg::Ping(p)) => {
                    let _ = chan.send_pong(&p);
                }
                Ok(Msg::Pong) => {}
                _ => {
                    chan.kill();
                    return false;
                }
            }
        };
        chan.set_read_timeout(PING_EVERY);
        let link = Arc::new(Link::new(chan.clone(), self.keys, self.room));
        let mut conn: Option<Arc<Conn>> = None;
        if peer {
            self.present(&link, &mut conn);
        }
        let aad_in = v2::aad(self.room, Dir::P2c);
        let (mut fails, mut last_rx, mut pong_seen) = (AuthFails::new(), Instant::now(), false);
        while !self.stop.load(Ordering::SeqCst) {
            let mut rd = Patient::new(&chan);
            match ws::read_msg_as(&mut rd, false) {
                Ok(Msg::Bin(frame)) => {
                    last_rx = Instant::now();
                    let Some(plain) = v2::open(&self.keys.p2c, &aad_in, &frame) else {
                        if fails.record(Instant::now()) {
                            break; // 50 frames that do not authenticate within a minute: drop the connection
                        }
                        continue;
                    };
                    let Some((sid, seq, m)) = v2::parse_envelope(&plain) else { continue };
                    if !self.replay.accept(&sid, seq) {
                        continue; // replayed or reordered
                    }
                    self.present(&link, &mut conn); // an authenticated frame proves the phone is there
                    let Some(dev) = self.sh.registry().get(self.dev_id) else { break }; // revoked meanwhile
                    if let Some(c) = &conn {
                        self.sh.on_value(&dev, c, &m);
                    }
                }
                Ok(Msg::Text(t)) => {
                    last_rx = Instant::now();
                    let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
                    if v["t"] == "peer" {
                        if v["up"].as_bool().unwrap_or(false) {
                            // the phone (re)connected: greet it again even if we thought it was there (its earlier socket may have died unnoticed)
                            if conn.is_some() {
                                self.hello(conn.as_ref().expect("checked"));
                            }
                            self.present(&link, &mut conn);
                        } else {
                            self.absent(&mut conn);
                        }
                    }
                }
                Ok(Msg::Ping(p)) => {
                    last_rx = Instant::now();
                    let _ = chan.send_pong(&p);
                }
                Ok(Msg::Pong) => {
                    last_rx = Instant::now();
                    pong_seen = true;
                }
                Ok(Msg::Close(_)) => break,
                Err(e) if is_timeout(&e) && !rd.armed() => {
                    // idle tick: keep the NAT open, notice a dead path, drop stalled file transfers
                    if (pong_seen && last_rx.elapsed() > DEAD_AFTER) || chan.send_ping().is_err() {
                        break;
                    }
                    self.sh.rf_sweep(self.dev_id);
                }
                Err(_) => break,
            }
        }
        self.absent(&mut conn);
        chan.kill();
        true
    }

    /// The phone is on the other end: register a channel (once) and greet it like the LAN server does.
    fn present(&self, link: &Arc<Link>, conn: &mut Option<Arc<Conn>>) {
        if conn.is_some() || self.stop.load(Ordering::SeqCst) || self.sh.registry().get(self.dev_id).is_none() {
            return;
        }
        let c = Arc::new(Conn::relay(self.sh.next_conn_id(), self.dev_id, link.clone()));
        self.sh.attach(&c, None);
        self.hello(&c);
        *self.ctl.conn.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
        *conn = Some(c);
    }

    fn hello(&self, c: &Conn) {
        self.sh.send_conn(c, &self.sh.hello_msg());
    }

    fn absent(&self, conn: &mut Option<Arc<Conn>>) {
        if let Some(c) = conn.take() {
            self.ctl.conn.lock().unwrap_or_else(|e| e.into_inner()).take();
            self.sh.detach(&c);
            self.sh.rf_conn_gone(&c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_addresses() {
        let t = parse_url(" wss://pill-relay.example.workers.dev/ ").unwrap();
        assert_eq!((t.tls, t.host.as_str(), t.port, t.base.as_str()), (true, "pill-relay.example.workers.dev", 443, ""));
        assert_eq!(t.base_url(), "wss://pill-relay.example.workers.dev");
        let t = parse_url("https://r.example.org:8443/pill/").unwrap();
        assert_eq!((t.tls, t.port, t.base.as_str()), (true, 8443, "/pill"));
        assert_eq!(t.base_url(), "wss://r.example.org:8443/pill");
        let t = parse_url("ws://127.0.0.1:9000").unwrap();
        assert_eq!((t.tls, t.port, t.base_url().as_str()), (false, 9000, "ws://127.0.0.1:9000"));
        for bad in ["", "wss://", "ws://example.org", "http://192.168.1.5:8787", "ftp://x", "wss://x.example/?a=1", "wss://u:p@x.example", "relay.example.org"] {
            assert!(parse_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn base64_in_either_alphabet() {
        let raw: Vec<u8> = (0..=255u8).collect();
        let std = base64::engine::general_purpose::STANDARD.encode(&raw);
        let url = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&raw);
        assert_eq!(b64_any(&std).unwrap(), raw);
        assert_eq!(b64_any(&url).unwrap(), raw);
        assert!(b64_any("not base64!").is_none());
    }
}
