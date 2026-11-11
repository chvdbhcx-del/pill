// The phone link server: HTTP + WebSocket on a private LAN address, one thread per connection, everything blocks on sockets (no polling).
// Protocol: docs/phone-protocol.md. PRIVACY: tokens and codes are never logged; notification text lives in memory only and is never logged.
use super::info::{self, Stats};
use super::files::{self, Dir, Refused, Status, Transfer, Transfers};
use super::http::{self, Range, Req};
use super::reg::{self, Device, Registry};
use super::ws::{self, Msg};
use super::{relay, v2};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SERVER_VERSION: u32 = 1;
const MAX_CONNS: usize = 64;
const MAX_NOTIFS: usize = 100;
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);
const BODY_TIMEOUT: Duration = Duration::from_secs(60);
/// The app pings every 20 s; a socket silent for this long is dead.
const WS_IDLE: Duration = Duration::from_secs(90);
const CHUNK: usize = 64 * 1024;

/// What the server reads from the settings each time it needs it (so a change applies at once).
#[derive(Clone, Debug)]
pub struct Cfg {
    pub downloads: PathBuf,
    pub max_bytes: u64,
    pub auto_accept: bool,
    pub apk_url: String,
    pub muted: Vec<String>,
    pub pc_name: String,
    /// `phone.relay`: reach paired phones through the internet relay
    pub relay_on: bool,
    /// `phone.relayUrl` as typed (empty = no relay); see `relay::parse_url`
    pub relay_url: String,
    /// `phone.relayMaxFileMB` in bytes: the size cap of a file that travels through the relay
    pub relay_max_bytes: u64,
}

pub struct Opts {
    /// where devices.json lives
    pub dir: PathBuf,
    pub bind: Ipv4Addr,
    pub port: u16,
    /// tests only: accept 127.0.0.1 peers
    pub allow_loopback: bool,
    pub cfg: Arc<dyn Fn() -> Cfg + Send + Sync>,
    /// bus events: `phone:state`, `phone:notif`, `phone:files`
    pub emit: Arc<dyn Fn(&str, Value) + Send + Sync>,
}

#[derive(Clone, Debug, Default)]
struct PhoneStatus {
    connected: bool,
    battery: Option<i64>,
    charging: bool,
    network: String,
    ssid: Option<String>,
    dnd: bool,
    at: i64,
    /// the optional `status` fields (see `info::clean`), `{}` for an older app
    info: Value,
    /// the address the PC saw the socket come from; kept after the phone left
    ip: String,
    connected_at: i64,
    stats: Stats,
}

/// How a phone is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// the LAN WebSocket
    Lan,
    /// the encrypted relay (docs/phone-relay.md)
    Relay,
}

pub(super) enum Out {
    Tcp(Mutex<TcpStream>),
    Relay(Arc<super::relay::Link>),
}

/// One live channel to a phone: a LAN WebSocket or a relay link. The message handlers do not care which.
pub(super) struct Conn {
    pub id: u64,
    pub device: String,
    pub route: Route,
    out: Out,
}

impl Conn {
    fn lan(id: u64, device: &str, s: TcpStream) -> Conn {
        Conn { id, device: device.to_string(), route: Route::Lan, out: Out::Tcp(Mutex::new(s)) }
    }
    pub fn relay(id: u64, device: &str, link: Arc<super::relay::Link>) -> Conn {
        Conn { id, device: device.to_string(), route: Route::Relay, out: Out::Relay(link) }
    }
    /// One v1 JSON message to the phone.
    pub fn send_text(&self, text: &str) -> bool {
        match &self.out {
            Out::Tcp(w) => w.lock().map(|mut s| ws::write_all(&mut *s, &ws::text_frame(text)).is_ok()).unwrap_or(false),
            Out::Relay(l) => l.send_text(text),
        }
    }
    /// Closes a LAN socket with an application code. A relay link is owned by `relay::Hub` (it ends when the device leaves the list).
    fn close(&self, code: u16) {
        if let Out::Tcp(w) = &self.out {
            if let Ok(mut s) = w.lock() {
                let _ = s.write_all(&ws::close_frame(code));
                let _ = s.shutdown(Shutdown::Both);
            }
        }
    }
    fn pong(&self, payload: &[u8]) {
        if let Out::Tcp(w) = &self.out {
            let _ = w.lock().map(|mut s| ws::write_all(&mut *s, &ws::pong_frame(payload)));
        }
    }
}

pub struct Shared {
    opts: Opts,
    pub addr: SocketAddr,
    reg: Registry,
    me: std::sync::Weak<Shared>,
    pub(super) relay: super::relay::Hub,
    pub(super) rf: super::relay_files::State,
    codes: reg::Codes,
    limiter: reg::Limiter,
    pub transfers: Transfers,
    conns: Mutex<Vec<Arc<Conn>>>,
    notifs: Mutex<Vec<Value>>,
    phones: Mutex<HashMap<String, PhoneStatus>>,
    replies: Mutex<HashMap<String, String>>,
    stop: AtomicBool,
    live: AtomicUsize,
    seq: AtomicU64,
}

pub struct Server {
    pub shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The PC's private IPv4 address towards the LAN (the source address the OS would use for a private destination; no packet is sent).
pub fn detect_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(a) if a.is_private() || a.is_link_local() => Some(a),
        _ => None,
    }
}

fn clip(v: &Value, max: usize) -> String {
    v.as_str().unwrap_or("").chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect()
}

/// A base64 PNG of the notification (icon / avatar): kept when it looks like base64 and is small, else dropped.
fn image_of(v: &Value) -> Value {
    match v.as_str() {
        Some(s) if !s.is_empty() && s.len() <= 48_000 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_')) => json!(s),
        _ => Value::Null,
    }
}

impl Server {
    pub fn start(opts: Opts) -> std::io::Result<Server> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(opts.bind), opts.port))?;
        let addr = listener.local_addr()?;
        let shared = Arc::new_cyclic(|me| Shared {
            reg: Registry::load(opts.dir.clone()),
            opts,
            addr,
            me: me.clone(),
            relay: Default::default(),
            rf: Default::default(),
            codes: Default::default(),
            limiter: Default::default(),
            transfers: Default::default(),
            conns: Default::default(),
            notifs: Default::default(),
            phones: Default::default(),
            replies: Default::default(),
            stop: AtomicBool::new(false),
            live: AtomicUsize::new(0),
            seq: AtomicU64::new(1),
        });
        let sh = shared.clone();
        let thread = std::thread::Builder::new().name("phone-accept".into()).spawn(move || accept_loop(listener, sh))?;
        shared.relay_sync(); // phones paired earlier reach the relay again (only when phone.relay is on and a URL is set)
        Ok(Server { shared, thread: Some(thread) })
    }

    /// Closes the listener and every phone socket; returns when the accept thread is gone (the port is free again).
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        let sh = &self.shared;
        if sh.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        sh.relay.stop_all(sh); // closes the relay sockets and joins their threads (they detach their own links)
        let lan: Vec<Arc<Conn>> = sh.conns.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
        for c in lan {
            c.close(1001);
        }
        let _ = TcpStream::connect_timeout(&sh.addr, Duration::from_millis(500)); // wakes accept()
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(l: TcpListener, sh: Arc<Shared>) {
    for conn in l.incoming() {
        if sh.stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(s) = conn else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        if sh.live.fetch_add(1, Ordering::SeqCst) >= MAX_CONNS {
            sh.live.fetch_sub(1, Ordering::SeqCst);
            continue; // dropped: too many open connections
        }
        let sh2 = sh.clone();
        let spawned = std::thread::Builder::new().name("phone-conn".into()).spawn(move || {
            let _ = sh2.serve(s);
            sh2.live.fetch_sub(1, Ordering::SeqCst);
        });
        if spawned.is_err() {
            sh.live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

type R = BufReader<TcpStream>;

impl Shared {
    pub fn cfg(&self) -> Cfg {
        (self.opts.cfg)()
    }
    pub fn emit(&self, name: &str, v: Value) {
        (self.opts.emit)(name, v)
    }
    pub fn registry(&self) -> &Registry {
        &self.reg
    }
    pub fn url(&self, code: &str) -> String {
        format!("http://{}:{}/p?c={code}", self.addr.ip(), self.addr.port())
    }

    // ------------------------------------------------------------------------------------------------ one connection

    fn serve(self: &Arc<Self>, s: TcpStream) -> std::io::Result<()> {
        let ip = s.peer_addr()?.ip();
        s.set_read_timeout(Some(HEAD_TIMEOUT))?;
        s.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut w = s.try_clone()?;
        if !http::peer_allowed(ip, self.opts.allow_loopback) {
            return http::err(&mut w, 403, "forbidden");
        }
        let now = now_ms();
        if self.limiter.blocked(ip, now) {
            return http::err(&mut w, 429, "too_many_requests");
        }
        let mut r = BufReader::new(s);
        let Ok(req) = http::read_head(&mut r) else { return http::err(&mut w, 400, "bad_request") };
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/p") => self.page(&mut w, &req),
            ("POST", "/api/pair") => self.pair(&mut w, &mut r, &req, ip),
            ("GET", "/api/state") => match self.auth(&mut w, &req, ip, false)? {
                Some(_) => http::json(&mut w, 200, &json!({ "pcName": self.cfg().pc_name, "serverVersion": SERVER_VERSION, "time": now_ms(), "paired": true })),
                None => Ok(()),
            },
            ("DELETE", "/api/device") => match self.auth(&mut w, &req, ip, false)? {
                Some(d) => {
                    self.revoke(&d.id);
                    http::json(&mut w, 200, &json!({ "ok": true }))
                }
                None => Ok(()),
            },
            ("GET", "/ws") => match self.auth(&mut w, &req, ip, true)? {
                Some(d) => self.ws_session(w, r, &req, d, ip),
                None => Ok(()),
            },
            ("PUT", p) if p.starts_with("/api/up/") => match self.auth(&mut w, &req, ip, false)? {
                Some(d) => self.upload(&mut w, &mut r, &req, &d, &p["/api/up/".len()..]),
                None => Ok(()),
            },
            ("GET", p) if p.starts_with("/api/down/") => match self.auth(&mut w, &req, ip, false)? {
                Some(d) => self.download(&mut w, &req, &d, &p["/api/down/".len()..]),
                None => Ok(()),
            },
            (_, "/p" | "/api/pair" | "/api/state" | "/api/device" | "/ws") => http::err(&mut w, 405, "method_not_allowed"),
            _ => http::err(&mut w, 404, "not_found"),
        }
    }

    /// The device of a valid bearer token (or `?token=` for the WebSocket). Answers 401 itself and counts the miss.
    fn auth(&self, w: &mut TcpStream, req: &Req, ip: IpAddr, allow_query: bool) -> std::io::Result<Option<Device>> {
        let tok = req.bearer().or_else(|| if allow_query { req.query.get("token").map(String::as_str) } else { None });
        match tok.and_then(|t| self.reg.by_token(t)) {
            Some(d) => Ok(Some(d)),
            None => {
                self.limiter.bad(ip, now_ms());
                http::err(w, 401, "unauthorized")?;
                Ok(None)
            }
        }
    }

    fn page(&self, w: &mut TcpStream, req: &Req) -> std::io::Result<()> {
        let c = self.cfg();
        let code = req.query.get("c").map(String::as_str).unwrap_or("");
        let html = super::page::render(req.lang(), &c.pc_name, &self.addr.ip().to_string(), self.addr.port(), code, &c.apk_url);
        http::respond(w, 200, "text/html; charset=utf-8", html.as_bytes(), &[("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'".into()), ("Referrer-Policy", "no-referrer".into())])
    }

    fn pair(&self, w: &mut TcpStream, r: &mut R, req: &Req, ip: IpAddr) -> std::io::Result<()> {
        let Some(len) = req.content_length().filter(|n| *n > 0 && *n <= 16 * 1024) else { return http::err(w, 400, "bad_request") };
        let mut body = vec![0u8; len as usize];
        r.read_exact(&mut body)?;
        let Ok(v) = serde_json::from_slice::<Value>(&body) else { return http::err(w, 400, "bad_request") };
        let id = v["deviceId"].as_str().unwrap_or("");
        if !files::valid_id(id) {
            return http::err(w, 400, "bad_request");
        }
        let now = now_ms();
        let code = v["code"].as_str().unwrap_or("");
        // v2 key agreement (docs/phone-relay.md): checked before the one-time code is spent, so a bad key costs the phone nothing
        let v2 = match v["pk"].as_str() {
            None => None,
            Some(pk) => {
                let Some(raw) = relay::b64_any(pk) else { return http::err(w, 400, "bad_request") };
                let secret: [u8; 32] = reg::random_bytes(32).try_into().expect("32 bytes");
                match v2::pair_pc(&raw, code, id, secret) {
                    Some(x) => Some(x),
                    None => return http::err(w, 400, "bad_request"),
                }
            }
        };
        if !self.codes.take(code, now) {
            self.limiter.bad(ip, now);
            return http::err(w, 403, "bad_code");
        }
        let token = reg::new_token();
        let name = v["name"].as_str().filter(|s| !s.trim().is_empty()).or(v["model"].as_str()).unwrap_or("Phone");
        self.close_device(id, 4001); // a second pairing replaces the token: the old socket must go
        let keys = v2.as_ref().map(|(_, k)| k.clone());
        self.reg.pair(id, name, v["model"].as_str().unwrap_or(""), v["os"].as_str().unwrap_or(""), v["appVersion"].as_str().unwrap_or(""), &token, now, keys.clone());
        let c = self.cfg();
        let mut resp = json!({ "token": token, "pcName": c.pc_name, "serverVersion": SERVER_VERSION });
        if let Some((pk, k)) = &v2 {
            resp["pk"] = json!(B64.encode(pk));
            if let Some(r) = self.relay_block(&c, k) {
                resp["relay"] = r;
            }
        }
        self.relay_sync(); // new keys: (re)connect this phone's room, or a plain v1 pairing: drop any old link
        self.emit("phone:state", self.status());
        http::json(w, 200, &resp)
    }

    /// `{"url","room","join"}` for a phone when the relay is switched on and has a usable address.
    fn relay_block(&self, c: &Cfg, k: &v2::Keys) -> Option<Value> {
        if !c.relay_on {
            return None;
        }
        let t = relay::parse_url(&c.relay_url)?;
        Some(json!({ "url": t.base_url(), "room": v2::room(k), "join": v2::join(k) }))
    }

    /// Makes the relay connections match the settings, the device list and their keys.
    pub fn relay_sync(&self) {
        if let Some(me) = self.me.upgrade() {
            self.relay.sync(&me);
        }
    }

    // ------------------------------------------------------------------------------------------------ files over HTTP

    fn upload(&self, w: &mut TcpStream, r: &mut R, req: &Req, dev: &Device, id: &str) -> std::io::Result<()> {
        if !files::valid_id(id) {
            return http::err(w, 404, "unknown_file");
        }
        let (t, _slot) = match self.transfers.begin_upload(id, &dev.id) {
            Ok(x) => x,
            Err(Refused::Unknown) => return http::err(w, 404, "unknown_file"),
            Err(Refused::Busy) => return http::respond(w, 503, "application/json", br#"{"error":"busy"}"#, &[("Retry-After", "2".into())]),
        };
        let fail = |this: &Self, w: &mut TcpStream, code: u16, what: &str, status: Status| {
            if let Some(t) = this.transfers.update(id, |t| t.status = status) {
                this.emit("phone:files", json!({ "kind": "failed", "transfer": t.public() }));
            }
            http::err(w, code, what)
        };
        let Some(len) = req.content_length() else { return fail(self, w, 411, "length_required", Status::Failed) };
        if len > t.size || t.size > self.cfg().max_bytes {
            return fail(self, w, 413, "too_large", Status::Failed); // a body larger than announced is refused before a byte is read
        }
        if len != t.size {
            return fail(self, w, 400, "size_mismatch", Status::Failed);
        }
        let (mut f, path) = match files::create_unique(&self.cfg().downloads, &t.name) {
            Ok(x) => x,
            Err(_) => return fail(self, w, 503, "disk", Status::Failed),
        };
        r.get_ref().set_read_timeout(Some(BODY_TIMEOUT))?;
        self.emit("phone:files", json!({ "kind": "start", "transfer": self.transfers.get(id).map(|t| t.public()) }));
        let (mut left, mut buf, mut last) = (len, vec![0u8; CHUNK], std::time::Instant::now());
        let mut ok = true;
        while left > 0 {
            let n = (left as usize).min(CHUNK);
            if self.stop.load(Ordering::SeqCst) || r.read_exact(&mut buf[..n]).is_err() || f.write_all(&buf[..n]).is_err() {
                ok = false;
                break;
            }
            left -= n as u64;
            if last.elapsed() > Duration::from_millis(300) {
                last = std::time::Instant::now();
                if let Some(t) = self.transfers.update(id, |t| t.bytes = len - left) {
                    self.emit("phone:files", json!({ "kind": "progress", "transfer": t.public() }));
                }
            }
        }
        drop(f);
        if !ok {
            let _ = std::fs::remove_file(&path);
            return fail(self, w, 400, "interrupted", Status::Failed);
        }
        let t = self.transfers.update(id, |t| {
            t.status = Status::Done;
            t.bytes = len;
            t.path = Some(path.clone());
        });
        if let Some(t) = &t {
            self.bump(&dev.id, |s| {
                s.files_in += 1;
                s.bytes_in += len;
            });
            self.emit("phone:files", json!({ "kind": "done", "transfer": t.public() }));
        }
        http::json(w, 200, &json!({ "ok": true, "path": path.to_string_lossy() }))
    }

    fn download(&self, w: &mut TcpStream, req: &Req, dev: &Device, id: &str) -> std::io::Result<()> {
        if !files::valid_id(id) {
            return http::err(w, 404, "unknown_file");
        }
        let (t, _slot) = match self.transfers.begin_download(id, &dev.id) {
            Ok(x) => x,
            Err(Refused::Unknown) => return http::err(w, 404, "unknown_file"),
            Err(Refused::Busy) => return http::respond(w, 503, "application/json", br#"{"error":"busy"}"#, &[("Retry-After", "2".into())]),
        };
        let Some(mut f) = t.path.as_ref().and_then(|p| std::fs::File::open(p).ok()) else { return http::err(w, 404, "file_gone") };
        let size = f.metadata()?.len();
        let (start, end, code) = match http::parse_range(req.header("range"), size) {
            Range::Whole => (0, size.saturating_sub(1), 200),
            Range::Part(a, b) => (a, b, 206),
            Range::Bad => return http::respond(w, 416, "application/json", br#"{"error":"bad_range"}"#, &[("Content-Range", format!("bytes */{size}"))]),
        };
        let len = if size == 0 { 0 } else { end - start + 1 };
        let mut extra = vec![("Accept-Ranges", "bytes".to_string()), ("Content-Disposition", format!("attachment; filename*=UTF-8''{}", http::pct_encode(&t.name)))];
        if code == 206 {
            extra.push(("Content-Range", format!("bytes {start}-{end}/{size}")));
        }
        w.write_all(&http::head(code, &t.mime, len, &extra))?;
        f.seek(SeekFrom::Start(start))?;
        let (mut left, mut buf) = (len, vec![0u8; CHUNK]);
        self.transfers.update(id, |t| t.status = Status::Active);
        while left > 0 && !self.stop.load(Ordering::SeqCst) {
            let n = f.read(&mut buf[..(left as usize).min(CHUNK)])?;
            if n == 0 {
                break;
            }
            w.write_all(&buf[..n])?;
            left -= n as u64;
        }
        w.flush()
    }

    // ------------------------------------------------------------------------------------------------ the WebSocket

    fn ws_session(self: &Arc<Self>, mut w: TcpStream, mut r: R, req: &Req, dev: Device, peer: IpAddr) -> std::io::Result<()> {
        let Some(key) = req.header("sec-websocket-key").filter(|_| req.header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"))) else {
            return http::err(&mut w, 400, "bad_request");
        };
        w.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", ws::accept_key(key)).as_bytes())?;
        w.flush()?;
        r.get_ref().set_read_timeout(Some(WS_IDLE))?;
        self.close_device(&dev.id, 4000); // one socket per phone: a reconnect replaces the stale one
        let conn = Arc::new(Conn::lan(self.seq.fetch_add(1, Ordering::SeqCst), &dev.id, w.try_clone()?));
        self.attach(&conn, Some(peer.to_string()));
        self.send_conn(&conn, &self.hello_msg());
        if let Some(rc) = self.relay_config(&dev) {
            self.send_conn(&conn, &rc); // lets a phone paired before the relay existed learn its address (optional message, see docs/phone-relay.md)
        }
        loop {
            match ws::read_msg(&mut r) {
                Ok(Msg::Text(t)) => self.on_message(&dev, &conn, &t),
                Ok(Msg::Ping(p)) => conn.pong(&p),
                Ok(Msg::Pong) => {}
                Ok(Msg::Close(_)) | Ok(Msg::Bin(_)) | Err(_) => break,
            }
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
        }
        self.detach(&conn);
        let _ = w.shutdown(Shutdown::Both);
        Ok(())
    }

    /// Registers a live channel of a phone and tells the UI.
    pub(super) fn attach(&self, conn: &Arc<Conn>, ip: Option<String>) {
        self.conns.lock().unwrap_or_else(|e| e.into_inner()).push(conn.clone());
        {
            let mut g = self.phones.lock().unwrap_or_else(|e| e.into_inner());
            let p = g.entry(conn.device.clone()).or_default();
            if !p.connected || conn.route == Route::Lan {
                p.connected_at = now_ms();
            }
            p.connected = true;
            if let Some(ip) = ip {
                p.ip = ip;
            }
        }
        self.reg.touch(&conn.device, now_ms(), false);
        self.emit("phone:state", self.status());
    }

    /// Removes a channel. Only when it was the phone's last one is the phone marked as gone (a replaced socket must not do that).
    pub(super) fn detach(&self, conn: &Conn) {
        let mut cs = self.conns.lock().unwrap_or_else(|e| e.into_inner());
        let had = cs.iter().any(|c| c.id == conn.id);
        cs.retain(|c| c.id != conn.id);
        let still = cs.iter().any(|c| c.device == conn.device);
        drop(cs);
        if had && !still {
            if let Some(p) = self.phones.lock().unwrap_or_else(|e| e.into_inner()).get_mut(&conn.device) {
                p.connected = false;
            }
            self.reg.touch(&conn.device, now_ms(), true);
        }
        if had {
            self.emit("phone:state", self.status()); // the route may have changed too
        }
    }

    /// The optional `relay.config` message for a LAN socket: where the relay is and which room, when it is on and this phone has keys.
    fn relay_config(&self, dev: &Device) -> Option<Value> {
        let k = dev.keys.as_ref()?;
        let b = self.relay_block(&self.cfg(), k)?;
        Some(json!({ "t": "relay.config", "url": b["url"], "room": b["room"], "join": b["join"] }))
    }

    /// Counts something for a phone and tells the UI (the counters travel with `phone:state`).
    pub(super) fn bump(&self, device: &str, f: impl FnOnce(&mut Stats)) {
        f(&mut self.phones.lock().unwrap_or_else(|e| e.into_inner()).entry(device.to_string()).or_default().stats);
        self.emit("phone:state", self.status());
    }

    pub(super) fn send_conn(&self, c: &Conn, v: &Value) -> bool {
        c.send_text(&v.to_string())
    }

    /// The v1 `hello`, plus the optional `relayMaxFileMB` (the phone shows it read-only).
    pub(super) fn hello_msg(&self) -> Value {
        let c = self.cfg();
        json!({ "t": "hello", "pcName": c.pc_name, "serverVersion": SERVER_VERSION, "time": now_ms(), "relayMaxFileMB": c.relay_max_bytes / (1024 * 1024) })
    }

    pub(super) fn me_arc(&self) -> Option<Arc<Shared>> {
        self.me.upgrade()
    }
    pub(super) fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
    pub(super) fn next_conn_id(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }

    /// The channels of a device, the LAN one first (the relay one stays as standby).
    pub(super) fn channels(&self, device: &str) -> Vec<Arc<Conn>> {
        let mut cs: Vec<Arc<Conn>> = self.conns.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|c| c.device == device).cloned().collect();
        cs.sort_by_key(|c| c.route != Route::Lan);
        cs
    }

    /// How a device is reached right now: LAN beats relay, `None` = offline.
    pub fn route_of(&self, device: &str) -> Option<Route> {
        self.channels(device).first().map(|c| c.route)
    }

    /// Sends to one device, or to every connected one when `device` is None: over its LAN socket when it has one, else over the relay
    /// (also when the LAN write fails). Returns how many devices got it.
    pub fn send(&self, device: Option<&str>, v: &Value) -> usize {
        let text = v.to_string();
        let mut ids: Vec<String> = self.conns.lock().unwrap_or_else(|e| e.into_inner()).iter().filter(|c| device.map_or(true, |d| c.device == d)).map(|c| c.device.clone()).collect();
        ids.sort();
        ids.dedup();
        ids.iter().filter(|id| self.channels(id).iter().any(|c| c.send_text(&text))).count()
    }

    /// Closes the LAN sockets of a device with an application close code (4001 = revoked). Its relay link follows `relay_sync`.
    fn close_device(&self, id: &str, code: u16) {
        for c in self.channels(id) {
            c.close(code);
        }
    }

    fn on_message(&self, dev: &Device, conn: &Conn, text: &str) {
        if let Ok(m) = serde_json::from_str::<Value>(text) {
            self.on_value(dev, conn, &m);
        }
    }

    /// One v1 message from a phone, whichever route it came by.
    pub(super) fn on_value(&self, dev: &Device, conn: &Conn, m: &Value) {
        match m["t"].as_str().unwrap_or("") {
            "ping" => {
                self.send_conn(conn, &json!({ "t": "pong" }));
            }
            "notif.posted" => self.notif_posted(dev, &m),
            "notif.removed" => {
                let key = clip(&m["key"], 300);
                let gone = {
                    let mut n = self.notifs.lock().unwrap_or_else(|e| e.into_inner());
                    let l = n.len();
                    n.retain(|x| x["key"] != key.as_str() || x["device"] != dev.id.as_str());
                    n.len() != l
                };
                if gone {
                    self.emit("phone:notif", json!({ "kind": "removed", "key": key }));
                }
            }
            "status" => {
                let mut g = self.phones.lock().unwrap_or_else(|e| e.into_inner());
                let p = g.entry(dev.id.clone()).or_default();
                p.connected = true;
                p.battery = m["battery"].as_i64().map(|b| b.clamp(0, 100));
                p.charging = m["charging"].as_bool().unwrap_or(false);
                p.network = clip(&m["network"], 12);
                p.ssid = m["ssid"].as_str().map(|s| clip(&json!(s), 40));
                p.dnd = m["dnd"].as_bool().unwrap_or(false);
                p.info = info::clean(&m);
                p.at = now_ms();
                drop(g);
                self.emit("phone:state", self.status());
            }
            "reply.result" => {
                let req = clip(&m["req"], 64);
                let key = self.replies.lock().unwrap_or_else(|e| e.into_inner()).remove(&req);
                if let Some(key) = key {
                    let ok = m["ok"].as_bool().unwrap_or(false);
                    if ok {
                        self.notifs.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| x["key"] != key.as_str());
                        self.bump(&dev.id, |s| s.replies += 1);
                    }
                    self.emit("phone:notif", json!({ "kind": "reply", "key": key, "ok": ok, "error": m["error"].as_str().map(|s| clip(&json!(s), 40)) }));
                }
            }
            "file.offer" => self.file_offer(dev, conn, &m),
            "file.done" => {
                let id = clip(&m["id"], 64);
                let ok = m["ok"].as_bool().unwrap_or(false);
                let t = self.transfers.update(&id, |t| {
                    if t.device == dev.id && t.dir == Dir::Out {
                        t.status = if ok { Status::Done } else { Status::Failed };
                        t.bytes = if ok { t.size } else { t.bytes };
                    }
                });
                if let Some(t) = t.filter(|t| t.device == dev.id) {
                    if ok {
                        self.bump(&dev.id, |s| {
                            s.files_out += 1;
                            s.bytes_out += t.size;
                        });
                    }
                    self.emit("phone:files", json!({ "kind": if ok { "sent" } else { "failed" }, "transfer": t.public() }));
                }
            }
            // files through the relay (docs/phone-relay.md); relay_files.rs ignores them when they do not come over a relay link
            "file.accept" => self.rf_accept(dev, conn, m),
            "file.chunk" => self.rf_chunk(dev, conn, m),
            "file.end" => self.rf_end(dev, conn, m),
            "file.ack" => self.rf_ack(dev, conn, m),
            "file.abort" => self.rf_abort(dev, conn, m),
            _ => {}
        }
    }

    fn notif_posted(&self, dev: &Device, m: &Value) {
        let key = clip(&m["key"], 300);
        let pkg = clip(&m["pkg"], 120);
        if key.is_empty() {
            return;
        }
        let app = clip(&m["app"], 60);
        self.reg.note_app(&pkg, &app);
        if self.cfg().muted.iter().any(|p| *p == pkg) {
            return; // the user muted this app: it is neither listed nor shown
        }
        let mut o = Map::new();
        o.insert("key".into(), json!(key));
        o.insert("device".into(), json!(dev.id));
        o.insert("pkg".into(), json!(pkg));
        o.insert("app".into(), json!(app));
        o.insert("title".into(), json!(clip(&m["title"], 200)));
        o.insert("text".into(), json!(clip(&m["text"], 1000)));
        o.insert("sub".into(), json!(clip(&m["sub"], 200)));
        o.insert("when".into(), json!(m["when"].as_i64().unwrap_or_else(now_ms)));
        o.insert("category".into(), json!(clip(&m["category"], 24)));
        o.insert("ongoing".into(), json!(m["ongoing"].as_bool().unwrap_or(false)));
        o.insert("progress".into(), if m["progress"].is_number() { m["progress"].clone() } else { Value::Null });
        o.insert("icon".into(), image_of(&m["icon"]));
        o.insert("avatar".into(), image_of(&m["avatar"]));
        o.insert("canReply".into(), json!(m["canReply"].as_bool().unwrap_or(false)));
        o.insert("actions".into(), json!(m["actions"].as_array().map(|a| a.iter().take(5).map(|x| json!({ "id": clip(&x["id"], 40), "title": clip(&x["title"], 60) })).collect::<Vec<_>>()).unwrap_or_default()));
        o.insert("group".into(), m["group"].as_str().map_or(Value::Null, |g| json!(clip(&json!(g), 200))));
        o.insert("replay".into(), json!(m["replay"].as_bool().unwrap_or(false)));
        o.insert("received".into(), json!(now_ms()));
        // a re-post of the same key with the same words (progress ticks, group updates) is not news
        let fresh = {
            let mut n = self.notifs.lock().unwrap_or_else(|e| e.into_inner());
            let same = n.iter().find(|x| x["key"] == key.as_str() && x["device"] == dev.id.as_str()).is_some_and(|x| x["title"] == o["title"] && x["text"] == o["text"]);
            n.retain(|x| !(x["key"] == key.as_str() && x["device"] == dev.id.as_str()));
            n.insert(0, Value::Object(o.clone()));
            n.truncate(MAX_NOTIFS);
            !same
        };
        o.insert("fresh".into(), json!(fresh));
        if fresh && !m["replay"].as_bool().unwrap_or(false) {
            self.bump(&dev.id, |s| s.notifs += 1);
        }
        self.emit("phone:notif", json!({ "kind": "posted", "notif": Value::Object(o) }));
    }

    fn file_offer(&self, dev: &Device, conn: &Conn, m: &Value) {
        let id = clip(&m["id"], 64);
        let reject = |why: &str| {
            self.send_conn(conn, &json!({ "t": "file.reject", "id": id, "reason": why }));
        };
        if !files::valid_id(&id) {
            return reject("bad_id");
        }
        let c = self.cfg();
        let size = m["size"].as_u64().unwrap_or(u64::MAX);
        if size > c.max_bytes || (conn.route == Route::Relay && size > c.relay_max_bytes) {
            return reject("too_big");
        }
        let name = files::sanitize_name(&clip(&m["name"], 300));
        let auto = c.auto_accept;
        let t = Transfer { id: id.clone(), device: dev.id.clone(), dir: Dir::In, mime: clip(&m["mime"], 80), name, size, status: if auto { Status::Accepted } else { Status::Pending }, path: None, bytes: 0, at: now_ms() };
        self.transfers.add(t.clone());
        if auto {
            self.send_conn(conn, &json!({ "t": "file.accept", "id": id }));
        }
        self.emit("phone:files", json!({ "kind": "offer", "transfer": t.public() }));
    }

    // ------------------------------------------------------------------------------------------------ what the commands use

    /// The user's decision about a pending offer from the phone.
    pub fn decide(&self, id: &str, accept: bool) -> Result<(), String> {
        let t = self.transfers.get(id).filter(|t| t.dir == Dir::In && t.status == Status::Pending).ok_or("no such pending file")?;
        let t = self.transfers.update(id, |t| t.status = if accept { Status::Accepted } else { Status::Rejected }).unwrap_or(t);
        let msg = if accept { json!({ "t": "file.accept", "id": id }) } else { json!({ "t": "file.reject", "id": id, "reason": "declined" }) };
        if self.send(Some(&t.device), &msg) == 0 {
            self.transfers.update(id, |t| t.status = Status::Failed);
            return Err("the phone is not connected".into());
        }
        self.emit("phone:files", json!({ "kind": if accept { "accepted" } else { "rejected" }, "transfer": t.public() }));
        Ok(())
    }

    /// Offers a file of this PC to a phone (the first connected one when `device` is None).
    pub fn send_file(&self, path: &std::path::Path, device: Option<&str>) -> Result<Value, String> {
        let meta = std::fs::metadata(path).map_err(|_| "file not found".to_string())?;
        if !meta.is_file() {
            return Err("not a file".into());
        }
        let c = self.cfg();
        if meta.len() > c.max_bytes {
            return Err("file is larger than the size cap".into());
        }
        let dev = {
            let cs = self.conns.lock().unwrap_or_else(|e| e.into_inner());
            cs.iter().map(|c| c.device.clone()).find(|d| device.map_or(true, |x| x == d)).ok_or("no phone is connected")?
        };
        if self.route_of(&dev) == Some(Route::Relay) && meta.len() > c.relay_max_bytes {
            return Err("file is larger than the size cap for the internet relay".into());
        }
        let name = files::sanitize_name(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        let t = Transfer { id: reg::new_id(), device: dev.clone(), dir: Dir::Out, mime: files::mime_of(&name).into(), name, size: meta.len(), status: Status::Accepted, path: Some(path.to_path_buf()), bytes: 0, at: now_ms() };
        self.transfers.add(t.clone());
        let sent = self.send(Some(&dev), &json!({ "t": "file.offer", "id": t.id, "name": t.name, "size": t.size, "mime": t.mime }));
        if sent == 0 {
            self.transfers.update(&t.id, |t| t.status = Status::Failed);
            return Err("no phone is connected".into());
        }
        self.emit("phone:files", json!({ "kind": "offered", "transfer": t.public() }));
        Ok(t.public())
    }

    /// `dismiss` | `read` | `reply` for a mirrored notification.
    pub fn notif_action(&self, key: &str, action: &str, text: &str) -> Result<(), String> {
        let dev = {
            let n = self.notifs.lock().unwrap_or_else(|e| e.into_inner());
            n.iter().find(|x| x["key"] == key).and_then(|x| x["device"].as_str().map(String::from)).ok_or("no such notification")?
        };
        let msg = match action {
            "dismiss" => json!({ "t": "notif.dismiss", "key": key }),
            "read" => json!({ "t": "notif.read", "key": key }),
            "reply" => {
                let text = text.trim();
                if text.is_empty() || text.chars().count() > 2000 {
                    return Err("empty or too long reply".into());
                }
                let req = reg::new_id();
                let mut r = self.replies.lock().unwrap_or_else(|e| e.into_inner());
                if r.len() >= 50 {
                    r.clear();
                }
                r.insert(req.clone(), key.to_string());
                drop(r);
                json!({ "t": "notif.reply", "req": req, "key": key, "text": text })
            }
            _ => return Err("unknown action".into()),
        };
        if self.send(Some(&dev), &msg) == 0 {
            return Err("the phone is not connected".into());
        }
        if action != "reply" {
            // hide / mark read: the row goes now, the phone's own `notif.removed` (if it follows) finds nothing left
            self.notifs.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| x["key"] != key);
            self.emit("phone:notif", json!({ "kind": if action == "read" { "read" } else { "removed" }, "key": key }));
        }
        Ok(())
    }

    pub fn notifs(&self) -> Vec<Value> {
        self.notifs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn pair_code(&self, renew: bool) -> Value {
        let (code, exp) = self.codes.current(now_ms(), renew);
        json!({ "code": code, "url": self.url(&code), "expires": exp })
    }

    pub fn revoke(&self, id: &str) -> bool {
        let gone = self.reg.revoke(id);
        if gone {
            self.close_device(id, 4001);
            self.relay_sync(); // the device is no longer wanted: its relay link and room connection end
            self.rf_device_gone(id);
            self.phones.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
            self.notifs.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| x["device"] != id);
            self.emit("phone:state", self.status());
        }
        gone
    }

    pub fn devices(&self) -> Vec<Value> {
        let routes: HashMap<String, Route> = {
            let cs = self.conns.lock().unwrap_or_else(|e| e.into_inner());
            let mut m: HashMap<String, Route> = HashMap::new();
            for c in cs.iter() {
                let e = m.entry(c.device.clone()).or_insert(c.route);
                if c.route == Route::Lan {
                    *e = Route::Lan;
                }
            }
            m
        };
        let ph = self.phones.lock().unwrap_or_else(|e| e.into_inner());
        self.reg
            .list()
            .iter()
            .map(|d| {
                let mut v = d.public();
                let p = ph.get(&d.id).cloned().unwrap_or_default();
                v["connected"] = json!(p.connected);
                v["battery"] = json!(p.battery);
                v["charging"] = json!(p.charging);
                v["network"] = json!(p.network);
                v["ssid"] = json!(p.ssid);
                v["dnd"] = json!(p.dnd);
                v["statusAt"] = json!(p.at);
                v["info"] = if p.info.is_object() { p.info.clone() } else { json!({}) };
                v["ip"] = json!(p.ip);
                v["connectedAt"] = json!(p.connected_at);
                v["route"] = json!(match routes.get(&d.id) {
                    Some(Route::Lan) => "lan",
                    Some(Route::Relay) => "relay",
                    None => "none",
                });
                v["relay"] = json!(d.keys.is_some()); // this phone can use the internet relay (paired with v2 keys)
                v["stats"] = p.stats.json();
                v
            })
            .collect()
    }

    pub fn status(&self) -> Value {
        let devs = self.devices();
        let connected = devs.iter().filter(|d| d["connected"] == true).count();
        json!({ "running": true, "ip": self.addr.ip().to_string(), "port": self.addr.port(), "pcName": self.cfg().pc_name, "devices": devs, "connected": connected })
    }
}
