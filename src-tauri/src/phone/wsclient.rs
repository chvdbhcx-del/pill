// A small blocking WebSocket client for the relay connector: TCP (+ rustls with the Windows certificate store for wss://), the RFC 6455
// handshake, masked frames. Reading and writing may happen on different threads: reads block on the raw socket WITHOUT holding any lock, writes
// take the lock only while they push bytes. `ws://` is accepted by relay.rs for loopback hosts only (tests, local development).
use super::{reg, ws};
use base64::Engine;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// A frame that began must finish within this long (the idle timeout only applies between frames).
const MID_FRAME: Duration = Duration::from_secs(30);

pub struct Chan {
    tcp: TcpStream,
    tls: Option<Mutex<rustls::ClientConnection>>,
    w: Mutex<()>,
}

pub struct Target {
    pub tls: bool,
    pub host: String,
    pub port: u16,
    /// path prefix of the relay (no trailing slash), usually empty
    pub base: String,
}

impl Target {
    /// `wss://host[:port][/prefix]` as the phone is told it.
    pub fn base_url(&self) -> String {
        let default = if self.tls { 443 } else { 80 };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let port = if self.port == default { String::new() } else { format!(":{}", self.port) };
        format!("{}://{host}{port}{}", if self.tls { "wss" } else { "ws" }, self.base)
    }
}

impl Chan {
    /// Connects, finishes TLS when needed and upgrades to a WebSocket at `path` (e.g. `/r/<room>?role=pc`).
    pub fn connect(t: &Target, path: &str) -> Result<Chan, String> {
        let addrs: Vec<_> = (t.host.as_str(), t.port).to_socket_addrs().map_err(|_| "cannot resolve the relay address".to_string())?.collect();
        let tcp = addrs.iter().find_map(|a| TcpStream::connect_timeout(a, CONNECT_TIMEOUT).ok()).ok_or_else(|| "cannot reach the relay".to_string())?;
        let _ = tcp.set_nodelay(true);
        let _ = tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
        let _ = tcp.set_write_timeout(Some(Duration::from_secs(30)));
        let tls = if t.tls {
            let name = rustls::pki_types::ServerName::try_from(t.host.clone()).map_err(|_| "bad relay host name".to_string())?;
            let mut c = rustls::ClientConnection::new(crate::mail::tls_config()?, name).map_err(|_| "TLS could not start".to_string())?;
            let mut s = &tcp;
            while c.is_handshaking() {
                c.complete_io(&mut s).map_err(|_| "TLS handshake with the relay failed".to_string())?;
            }
            Some(Mutex::new(c))
        } else {
            None
        };
        let chan = Chan { tcp, tls, w: Mutex::new(()) };
        chan.upgrade(t, path)?;
        Ok(chan)
    }

    fn upgrade(&self, t: &Target, path: &str) -> Result<(), String> {
        let key = base64::engine::general_purpose::STANDARD.encode(reg::random_bytes(16));
        let default = if t.tls { 443 } else { 80 };
        let host = if t.port == default { t.host.clone() } else { format!("{}:{}", t.host, t.port) };
        let req = format!("GET {}{path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n", t.base);
        self.write_all(req.as_bytes()).map_err(|_| "the relay closed the connection".to_string())?;
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut b = [0u8; 1];
            if head.len() > 8192 || self.read(&mut b).map_err(|_| "the relay did not answer".to_string())? == 0 {
                return Err("the relay closed the connection".into());
            }
            head.push(b[0]);
        }
        let head = String::from_utf8_lossy(&head).to_string();
        let ok = head.starts_with("HTTP/1.1 101") && head.lines().any(|l| l.to_ascii_lowercase().starts_with("sec-websocket-accept:") && l.split_once(':').is_some_and(|(_, v)| v.trim() == ws::accept_key(&key)));
        if ok { Ok(()) } else { Err(format!("the relay refused the WebSocket upgrade ({})", head.lines().next().unwrap_or("").trim())) }
    }

    pub fn set_read_timeout(&self, d: Duration) {
        let _ = self.tcp.set_read_timeout(Some(d));
    }

    /// Plaintext bytes; `Ok(0)` = the peer closed; a `TimedOut` / `WouldBlock` error = nothing arrived within the read timeout.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(tls) = &self.tls else { return (&self.tcp).read(buf) };
        loop {
            match tls.lock().unwrap_or_else(|e| e.into_inner()).reader().read(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            let mut raw = [0u8; 16 * 1024];
            let n = (&self.tcp).read(&mut raw)?; // blocks here with no lock held, so writers are never kept waiting
            if n == 0 {
                return Ok(0);
            }
            let mut c = tls.lock().unwrap_or_else(|e| e.into_inner());
            c.read_tls(&mut &raw[..n])?;
            c.process_new_packets().map_err(io::Error::other)?;
            while c.wants_write() {
                c.write_tls(&mut &self.tcp)?; // alerts, key updates
            }
        }
    }

    pub fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
        match &self.tls {
            None => {
                let _g = self.w.lock().unwrap_or_else(|e| e.into_inner());
                (&self.tcp).write_all(bytes)
            }
            Some(tls) => {
                let mut c = tls.lock().unwrap_or_else(|e| e.into_inner());
                c.writer().write_all(bytes)?;
                while c.wants_write() {
                    c.write_tls(&mut &self.tcp)?;
                }
                Ok(())
            }
        }
    }

    fn frame(&self, op: u8, payload: &[u8]) -> io::Result<()> {
        let m = reg::random_bytes(4);
        self.write_all(&ws::masked_frame(op, payload, [m[0], m[1], m[2], m[3]]))
    }
    pub fn send_text(&self, s: &str) -> io::Result<()> {
        self.frame(0x1, s.as_bytes())
    }
    pub fn send_binary(&self, b: &[u8]) -> io::Result<()> {
        self.frame(0x2, b)
    }
    pub fn send_ping(&self) -> io::Result<()> {
        self.frame(0x9, b"")
    }
    pub fn send_pong(&self, p: &[u8]) -> io::Result<()> {
        self.frame(0xA, &p[..p.len().min(125)])
    }

    /// Best-effort WebSocket close, then the socket is shut so a blocked reader wakes up.
    pub fn close(&self, code: u16) {
        let _ = self.frame(0x8, &code.to_be_bytes());
        self.kill();
    }

    pub fn kill(&self) {
        let _ = self.tcp.shutdown(Shutdown::Both);
    }
}

/// Reads for `ws::read_msg_as`: an idle timeout before a frame starts is reported (`armed()` false), but once the first byte of a message
/// arrived the timeouts are ridden out (up to `MID_FRAME`), so a slow link never splits a frame.
pub struct Patient<'a> {
    chan: &'a Arc<Chan>,
    since: Option<Instant>,
}

impl<'a> Patient<'a> {
    pub fn new(chan: &'a Arc<Chan>) -> Patient<'a> {
        Patient { chan, since: None }
    }
    /// True once part of a message has been read.
    pub fn armed(&self) -> bool {
        self.since.is_some()
    }
}

impl Read for Patient<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.chan.read(buf) {
                Ok(n) => {
                    self.since.get_or_insert_with(Instant::now);
                    return if n == 0 { Err(io::ErrorKind::UnexpectedEof.into()) } else { Ok(n) };
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) && self.since.is_some_and(|s| s.elapsed() < MID_FRAME) => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

pub fn is_timeout(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}
