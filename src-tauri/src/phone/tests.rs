// End-to-end tests of the phone link: the real server on 127.0.0.1 (a test-only "allow loopback" switch) against a fake phone written here with
// std sockets and a minimal WebSocket client. Every test stops its server; no port stays open.
use super::files::Status;
use super::server::{Cfg, Opts, Server};
use super::ws;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Events = Arc<Mutex<Vec<(String, Value)>>>;

struct Rig {
    server: Option<Server>,
    events: Events,
    dir: PathBuf,
    cfg: Arc<Mutex<Cfg>>,
}

impl Rig {
    fn new(loopback: bool) -> Rig {
        let dir = std::env::temp_dir().join(format!("pill-phone-e2e-{}", super::reg::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = Arc::new(Mutex::new(Cfg { downloads: dir.join("dl"), max_bytes: 10 * 1024 * 1024, auto_accept: true, apk_url: "https://example.org/PillPhone.apk".into(), muted: vec![], pc_name: "TEST-PC".into() }));
        let events: Events = Default::default();
        let (c2, e2) = (cfg.clone(), events.clone());
        let server = Server::start(Opts {
            dir: dir.join("data"),
            bind: Ipv4Addr::LOCALHOST,
            port: 0,
            allow_loopback: loopback,
            cfg: Arc::new(move || c2.lock().unwrap().clone()),
            emit: Arc::new(move |n, v| e2.lock().unwrap().push((n.to_string(), v))),
        })
        .unwrap();
        Rig { server: Some(server), events, dir, cfg }
    }
    fn sh(&self) -> &super::server::Shared {
        &self.server.as_ref().unwrap().shared
    }
    fn addr(&self) -> std::net::SocketAddr {
        self.sh().addr
    }
    /// Waits for an event the predicate accepts (the server thread emits on its own time).
    fn wait(&self, name: &str, pred: impl Fn(&Value) -> bool) -> Value {
        let t = Instant::now();
        loop {
            if let Some(v) = self.events.lock().unwrap().iter().find(|(n, v)| n == name && pred(v)).map(|(_, v)| v.clone()) {
                return v;
            }
            assert!(t.elapsed() < Duration::from_secs(5), "no `{name}` event arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn pair(&self, device: &str) -> String {
        let code = self.sh().pair_code(false)["code"].as_str().unwrap().to_string();
        let (st, _, body) = http(self.addr(), "POST", "/api/pair", &[], json!({ "code": code, "deviceId": device, "name": "Pixel 8", "model": "Pixel 8", "os": "Android 15", "appVersion": "1.0.0" }).to_string().as_bytes());
        assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
        serde_json::from_slice::<Value>(&body).unwrap()["token"].as_str().unwrap().to_string()
    }
    fn stop(mut self) {
        let addr = self.addr();
        self.server.take().unwrap().stop();
        assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err(), "the port must be closed after stop()");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One request, read to the end (the server always closes). Returns (status, lower-case headers as "k: v" lines, body).
fn http(addr: std::net::SocketAddr, method: &str, path: &str, headers: &[(&str, String)], body: &[u8]) -> (u16, Vec<String>, Vec<u8>) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: x\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty() || method == "PUT" || method == "POST" {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut raw = Vec::new();
    let _ = s.read_to_end(&mut raw);
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(raw.len().saturating_sub(4));
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head.lines();
    let code = lines.next().unwrap_or("HTTP/1.1 0").split(' ').nth(1).unwrap().parse().unwrap();
    (code, lines.map(|l| l.to_ascii_lowercase()).collect(), raw[(split + 4).min(raw.len())..].to_vec())
}

fn bearer(t: &str) -> [(&'static str, String); 1] {
    [("Authorization", format!("Bearer {t}"))]
}

struct Phone {
    s: TcpStream,
}

impl Phone {
    fn connect(addr: std::net::SocketAddr, token: &str) -> Phone {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(format!("GET /ws?token={token} HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").as_bytes()).unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut b = [0u8; 1];
            s.read_exact(&mut b).unwrap();
            head.push(b[0]);
        }
        let head = String::from_utf8(head).unwrap();
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        assert!(head.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="), "{head}");
        Phone { s }
    }
    fn send(&mut self, v: Value) {
        self.s.write_all(&ws::client_frame(1, true, v.to_string().as_bytes())).unwrap();
    }
    /// The next server frame: (opcode, payload).
    fn frame(&mut self) -> (u8, Vec<u8>) {
        let mut h = [0u8; 2];
        self.s.read_exact(&mut h).unwrap();
        let mut len = (h[1] & 0x7F) as usize;
        if len == 126 {
            let mut b = [0u8; 2];
            self.s.read_exact(&mut b).unwrap();
            len = u16::from_be_bytes(b) as usize;
        }
        let mut p = vec![0u8; len];
        self.s.read_exact(&mut p).unwrap();
        (h[0] & 0x0F, p)
    }
    fn recv(&mut self) -> Value {
        let (op, p) = self.frame();
        assert_eq!(op, 1);
        serde_json::from_slice(&p).unwrap()
    }
    fn recv_t(&mut self, t: &str) -> Value {
        let v = self.recv();
        assert_eq!(v["t"], t, "{v}");
        v
    }
}

#[test]
fn only_private_peers_may_connect() {
    let r = Rig::new(false); // loopback is not private: the production rule
    let (st, _, _) = http(r.addr(), "GET", "/p", &[], b"");
    assert_eq!(st, 403);
    r.stop();
}

#[test]
fn pairing_page_code_token_and_state() {
    let r = Rig::new(true);
    let code = r.sh().pair_code(false);
    let c = code["code"].as_str().unwrap();
    assert_eq!(c.len(), 8);
    assert!(code["url"].as_str().unwrap().starts_with(&format!("http://127.0.0.1:{}/p?c={c}", r.addr().port())));
    // the page: Russian by Accept-Language, both buttons, the deep link with the code, a restrictive CSP
    let (st, h, body) = http(r.addr(), "GET", &format!("/p?c={c}"), &[("Accept-Language", "ru-RU,ru;q=0.9".into())], b"");
    let html = String::from_utf8(body).unwrap();
    assert_eq!(st, 200);
    assert!(h.iter().any(|l| l.starts_with("content-type: text/html")) && h.iter().any(|l| l.starts_with("content-security-policy")));
    assert!(html.contains("Скачать приложение") && html.contains("Открыть в Pill Phone") && html.contains(&format!("c={c}")) && html.contains("TEST-PC"));
    assert!(html.contains("https://example.org/PillPhone.apk"));
    // a wrong code is refused, the right one pays out once
    let bad = json!({ "code": "ZZZZZZZZ", "deviceId": "dev-1", "name": "Pixel" }).to_string();
    assert_eq!(http(r.addr(), "POST", "/api/pair", &[], bad.as_bytes()).0, 403);
    let tok = r.pair("dev-1");
    assert_eq!(tok.len(), 43);
    let again = json!({ "code": c, "deviceId": "dev-2", "name": "x" }).to_string();
    let (st, _, body) = http(r.addr(), "POST", "/api/pair", &[], again.as_bytes());
    assert_eq!((st, serde_json::from_slice::<Value>(&body).unwrap()["error"].as_str().unwrap().to_string()), (403, "bad_code".to_string()));
    // /api/state needs the token
    let (st, _, body) = http(r.addr(), "GET", "/api/state", &bearer(&tok), b"");
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!((st, v["pcName"].as_str(), v["serverVersion"].as_u64(), v["paired"].as_bool()), (200, Some("TEST-PC"), Some(1), Some(true)));
    assert_eq!(http(r.addr(), "GET", "/api/state", &[], b"").0, 401);
    assert_eq!(http(r.addr(), "GET", "/api/state", &bearer("nope"), b"").0, 401);
    // the phone shows up in the registry (and the state event fired)
    assert_eq!(r.sh().devices()[0]["name"], "Pixel 8");
    r.wait("phone:state", |v| v["devices"].as_array().is_some_and(|d| d.len() == 1));
    // unpairing itself
    assert_eq!(http(r.addr(), "DELETE", "/api/device", &bearer(&tok), b"").0, 200);
    assert_eq!(http(r.addr(), "GET", "/api/state", &bearer(&tok), b"").0, 401);
    assert!(r.sh().devices().is_empty());
    r.stop();
}

#[test]
fn an_expired_code_is_refused() {
    let r = Rig::new(true);
    // a code that ran out (the registry logic itself is unit-tested; here: the server really consults it)
    let c = r.sh().pair_code(true)["code"].as_str().unwrap().to_string();
    r.sh().pair_code(true); // renewing kills the previous one
    let body = json!({ "code": c, "deviceId": "dev-1", "name": "x" }).to_string();
    assert_eq!(http(r.addr(), "POST", "/api/pair", &[], body.as_bytes()).0, 403);
    r.stop();
}

#[test]
fn twenty_bad_tokens_a_minute_end_in_429() {
    let r = Rig::new(true);
    for i in 0..20 {
        assert_eq!(http(r.addr(), "GET", "/api/state", &bearer("wrong"), b"").0, 401, "attempt {i}");
    }
    assert_eq!(http(r.addr(), "GET", "/api/state", &bearer("wrong"), b"").0, 429);
    let tok = {
        // even a good token is refused while the address is blocked
        let code = r.sh().pair_code(false)["code"].as_str().unwrap().to_string();
        let body = json!({ "code": code, "deviceId": "dev-1", "name": "x" }).to_string();
        http(r.addr(), "POST", "/api/pair", &[], body.as_bytes()).0
    };
    assert_eq!(tok, 429);
    r.stop();
}

#[test]
fn websocket_hello_ping_status_and_notifications() {
    let r = Rig::new(true);
    let tok = r.pair("dev-1");
    assert_eq!(http(r.addr(), "GET", "/ws?token=nope", &[("Upgrade", "websocket".into())], b"").0, 401);
    let mut p = Phone::connect(r.addr(), &tok);
    let hello = p.recv_t("hello");
    assert_eq!((hello["pcName"].as_str(), hello["serverVersion"].as_u64()), (Some("TEST-PC"), Some(1)));
    assert!(hello["time"].as_i64().unwrap() > 1_600_000_000_000);
    p.send(json!({ "t": "ping" }));
    p.recv_t("pong");
    // a WebSocket-level ping is answered with a pong frame
    p.s.write_all(&ws::client_frame(0x9, true, b"hi")).unwrap();
    assert_eq!(p.frame(), (0xA, b"hi".to_vec()));
    // status
    p.send(json!({ "t": "status", "battery": 87, "charging": true, "network": "wifi", "ssid": "Home", "dnd": false, "time": 1 }));
    let st = r.wait("phone:state", |v| v["devices"][0]["battery"] == 87);
    assert_eq!((st["connected"].as_u64(), st["devices"][0]["network"].as_str(), st["devices"][0]["ssid"].as_str(), st["devices"][0]["charging"].as_bool()), (Some(1), Some("wifi"), Some("Home"), Some(true)));
    // a notification
    let n = json!({ "t": "notif.posted", "key": "0|org.telegram.messenger|1", "pkg": "org.telegram.messenger", "app": "Telegram", "title": "Вадим", "text": "привет", "sub": "", "when": 1700000000000i64, "category": "msg", "ongoing": false, "progress": null, "icon": "iVBORw0KGgo=", "avatar": "<script>", "canReply": true, "actions": [{ "id": "a1", "title": "Mark as read" }], "group": null });
    p.send(n.clone());
    let ev = r.wait("phone:notif", |v| v["kind"] == "posted");
    assert_eq!((ev["notif"]["title"].as_str(), ev["notif"]["canReply"].as_bool(), ev["notif"]["fresh"].as_bool()), (Some("Вадим"), Some(true), Some(true)));
    assert_eq!(ev["notif"]["icon"], "iVBORw0KGgo=");
    assert!(ev["notif"]["avatar"].is_null(), "an avatar that is not base64 is dropped");
    assert_eq!(r.sh().notifs().len(), 1);
    // the same words again (progress tick): stored once, not fresh
    p.send(n);
    r.wait("phone:notif", |v| v["kind"] == "posted" && v["notif"]["fresh"] == false);
    assert_eq!(r.sh().notifs().len(), 1);
    // Hide / Mark read / Reply travel to the phone
    r.sh().notif_action("0|org.telegram.messenger|1", "read", "").unwrap();
    assert_eq!(p.recv_t("notif.read")["key"], "0|org.telegram.messenger|1");
    assert!(r.sh().notifs().is_empty());
    p.send(json!({ "t": "notif.posted", "key": "k2", "pkg": "org.telegram.messenger", "app": "Telegram", "title": "A", "text": "B", "canReply": true }));
    r.wait("phone:notif", |v| v["notif"]["key"] == "k2");
    assert!(r.sh().notif_action("k2", "reply", "  ").is_err(), "empty replies are refused");
    r.sh().notif_action("k2", "reply", "хорошо").unwrap();
    let rep = p.recv_t("notif.reply");
    assert_eq!((rep["key"].as_str(), rep["text"].as_str()), (Some("k2"), Some("хорошо")));
    p.send(json!({ "t": "reply.result", "req": rep["req"], "ok": true }));
    let done = r.wait("phone:notif", |v| v["kind"] == "reply");
    assert_eq!((done["key"].as_str(), done["ok"].as_bool()), (Some("k2"), Some(true)));
    assert!(r.sh().notifs().is_empty(), "a sent reply takes the notification off the list");
    p.send(json!({ "t": "notif.posted", "key": "k3", "pkg": "p", "app": "A", "title": "t", "text": "x" }));
    r.wait("phone:notif", |v| v["notif"]["key"] == "k3");
    r.sh().notif_action("k3", "dismiss", "").unwrap();
    assert_eq!(p.recv_t("notif.dismiss")["key"], "k3");
    r.wait("phone:notif", |v| v["kind"] == "removed" && v["key"] == "k3");
    p.send(json!({ "t": "notif.posted", "key": "k4", "pkg": "p", "app": "A", "title": "t", "text": "x" }));
    p.send(json!({ "t": "notif.removed", "key": "k4" }));
    r.wait("phone:notif", |v| v["kind"] == "removed" && v["key"] == "k4");
    assert!(r.sh().notif_action("gone", "dismiss", "").is_err());
    // ignoring junk keeps the socket alive
    p.send(json!({ "t": "who-knows" }));
    p.s.write_all(&ws::client_frame(1, true, b"not json")).unwrap();
    p.send(json!({ "t": "ping" }));
    p.recv_t("pong");
    drop(p);
    r.wait("phone:state", |v| v["connected"] == 0);
    r.stop();
}

#[test]
fn muted_apps_are_not_listed_but_are_remembered() {
    let r = Rig::new(true);
    r.cfg.lock().unwrap().muted = vec!["com.spam".into()];
    let tok = r.pair("dev-1");
    let mut p = Phone::connect(r.addr(), &tok);
    p.recv_t("hello");
    p.send(json!({ "t": "notif.posted", "key": "m1", "pkg": "com.spam", "app": "Spam", "title": "buy", "text": "now" }));
    p.send(json!({ "t": "notif.posted", "key": "m2", "pkg": "com.ok", "app": "Ok", "title": "hi", "text": "there" }));
    r.wait("phone:notif", |v| v["notif"]["key"] == "m2");
    assert_eq!(r.sh().notifs().len(), 1);
    assert_eq!(r.sh().registry().apps()["com.spam"], "Spam");
    drop(p);
    r.stop();
}

#[test]
fn revoking_closes_the_socket_with_4001_and_kills_the_token() {
    let r = Rig::new(true);
    let tok = r.pair("dev-1");
    let mut p = Phone::connect(r.addr(), &tok);
    p.recv_t("hello");
    assert!(r.sh().revoke("dev-1"));
    let (op, payload) = p.frame();
    assert_eq!((op, u16::from_be_bytes([payload[0], payload[1]])), (8, 4001));
    assert_eq!(http(r.addr(), "GET", "/api/state", &bearer(&tok), b"").0, 401);
    assert!(r.sh().devices().is_empty());
    r.stop();
}

#[test]
fn a_second_socket_of_the_same_phone_replaces_the_first() {
    let r = Rig::new(true);
    let tok = r.pair("dev-1");
    let mut a = Phone::connect(r.addr(), &tok);
    a.recv_t("hello");
    let mut b = Phone::connect(r.addr(), &tok);
    b.recv_t("hello");
    let (op, payload) = a.frame();
    assert_eq!((op, u16::from_be_bytes([payload[0], payload[1]])), (8, 4000));
    assert_eq!(r.sh().send(Some("dev-1"), &json!({ "t": "find" })), 1);
    b.recv_t("find");
    assert_eq!(r.sh().status()["connected"], 1, "the replaced socket must not mark the phone as gone");
    drop((a, b));
    r.stop();
}

#[test]
fn phone_to_pc_upload() {
    let r = Rig::new(true);
    let tok = r.pair("dev-1");
    let mut p = Phone::connect(r.addr(), &tok);
    p.recv_t("hello");
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    // the name is sanitised, the offer accepted automatically
    p.send(json!({ "t": "file.offer", "id": "f1", "name": "../../evil/..\\photo.jpg", "size": data.len(), "mime": "image/jpeg" }));
    assert_eq!(p.recv_t("file.accept")["id"], "f1");
    // not announced / not accepted: 404; a body that does not match the announced size: refused
    assert_eq!(http(r.addr(), "PUT", "/api/up/zzz", &bearer(&tok), b"x").0, 404);
    let mut big = data.clone();
    big.push(1);
    assert_eq!(http(r.addr(), "PUT", "/api/up/f1", &bearer(&tok), &big).0, 413, "larger than announced");
    assert_eq!(r.sh().transfers.get("f1").unwrap().status, Status::Failed);
    // a failed offer cannot be retried (the phone offers again with a new id)
    assert_eq!(http(r.addr(), "PUT", "/api/up/f1", &bearer(&tok), &data).0, 404);
    p.send(json!({ "t": "file.offer", "id": "f2", "name": "../../evil/..\\photo.jpg", "size": data.len(), "mime": "image/jpeg" }));
    p.recv_t("file.accept");
    assert_eq!(http(r.addr(), "PUT", "/api/up/f2", &[], &data).0, 401, "a token is needed");
    let (st, _, body) = http(r.addr(), "PUT", "/api/up/f2", &bearer(&tok), &data);
    assert_eq!(st, 200, "{}", String::from_utf8_lossy(&body));
    let path = PathBuf::from(serde_json::from_slice::<Value>(&body).unwrap()["path"].as_str().unwrap());
    assert_eq!(path.parent().unwrap(), r.dir.join("dl"), "lands in the downloads folder, never outside it");
    assert_eq!(path.file_name().unwrap(), "photo.jpg");
    assert_eq!(std::fs::read(&path).unwrap(), data);
    let done = r.wait("phone:files", |v| v["kind"] == "done");
    assert_eq!((done["transfer"]["dir"].as_str(), done["transfer"]["name"].as_str()), (Some("in"), Some("photo.jpg")));
    // the same name again gets a suffix, nothing is overwritten
    p.send(json!({ "t": "file.offer", "id": "f3", "name": "photo.jpg", "size": 3 }));
    p.recv_t("file.accept");
    let (_, _, body) = http(r.addr(), "PUT", "/api/up/f3", &bearer(&tok), b"abc");
    assert!(serde_json::from_slice::<Value>(&body).unwrap()["path"].as_str().unwrap().ends_with("photo (1).jpg"));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    // reserved names and streams
    p.send(json!({ "t": "file.offer", "id": "f4", "name": "NUL.txt", "size": 1 }));
    p.recv_t("file.accept");
    let (_, _, body) = http(r.addr(), "PUT", "/api/up/f4", &bearer(&tok), b"z");
    assert!(serde_json::from_slice::<Value>(&body).unwrap()["path"].as_str().unwrap().ends_with("_NUL.txt"));
    drop(p);
    r.stop();
}

#[test]
fn size_cap_and_manual_acceptance() {
    let r = Rig::new(true);
    let tok = r.pair("dev-1");
    let mut p = Phone::connect(r.addr(), &tok);
    p.recv_t("hello");
    // over the cap: rejected at the offer
    p.send(json!({ "t": "file.offer", "id": "big", "name": "movie.mkv", "size": 11u64 * 1024 * 1024 }));
    let rej = p.recv_t("file.reject");
    assert_eq!((rej["id"].as_str(), rej["reason"].as_str()), (Some("big"), Some("too_big")));
    assert!(r.sh().transfers.get("big").is_none());
    // a bad id is rejected too
    p.send(json!({ "t": "file.offer", "id": "../x", "name": "a", "size": 1 }));
    assert_eq!(p.recv_t("file.reject")["reason"], "bad_id");
    // auto-accept off: the offer waits for the user
    r.cfg.lock().unwrap().auto_accept = false;
    p.send(json!({ "t": "file.offer", "id": "w1", "name": "a.txt", "size": 2 }));
    r.wait("phone:files", |v| v["kind"] == "offer" && v["transfer"]["status"] == "pending");
    assert_eq!(http(r.addr(), "PUT", "/api/up/w1", &bearer(&tok), b"hi").0, 404, "not accepted yet");
    r.sh().decide("w1", true).unwrap();
    assert_eq!(p.recv_t("file.accept")["id"], "w1");
    assert_eq!(http(r.addr(), "PUT", "/api/up/w1", &bearer(&tok), b"hi").0, 200);
    p.send(json!({ "t": "file.offer", "id": "w2", "name": "b.txt", "size": 2 }));
    r.wait("phone:files", |v| v["transfer"]["id"] == "w2");
    r.sh().decide("w2", false).unwrap();
    assert_eq!(p.recv_t("file.reject")["reason"], "declined");
    assert!(r.sh().decide("w2", true).is_err(), "decided offers are final");
    drop(p);
    r.stop();
}

#[test]
fn pc_to_phone_download_with_ranges() {
    let r = Rig::new(true);
    let tok = r.pair("dev-1");
    let other = {
        let code = r.sh().pair_code(true)["code"].as_str().unwrap().to_string();
        let (_, _, b) = http(r.addr(), "POST", "/api/pair", &[], json!({ "code": code, "deviceId": "dev-2", "name": "Other" }).to_string().as_bytes());
        serde_json::from_slice::<Value>(&b).unwrap()["token"].as_str().unwrap().to_string()
    };
    let file = r.dir.join("Report.docx");
    std::fs::write(&file, (0..1000u32).map(|i| (i % 256) as u8).collect::<Vec<u8>>()).unwrap();
    assert!(r.sh().send_file(&file, None).is_err(), "no phone connected yet");
    let mut p = Phone::connect(r.addr(), &tok);
    p.recv_t("hello");
    let t = r.sh().send_file(&file, None).unwrap();
    let offer = p.recv_t("file.offer");
    let id = offer["id"].as_str().unwrap().to_string();
    assert_eq!((offer["name"].as_str(), offer["size"].as_u64()), (Some("Report.docx"), Some(1000)));
    assert_eq!(t["id"], offer["id"]);
    let (st, h, body) = http(r.addr(), "GET", &format!("/api/down/{id}"), &bearer(&tok), b"");
    assert_eq!((st, body.len()), (200, 1000));
    assert!(h.iter().any(|l| l == "accept-ranges: bytes"));
    let (st, h, body) = http(r.addr(), "GET", &format!("/api/down/{id}"), &[("Authorization", format!("Bearer {tok}")), ("Range", "bytes=100-199".into())], b"");
    assert_eq!((st, body.len(), body[0]), (206, 100, 100));
    assert!(h.iter().any(|l| l == "content-range: bytes 100-199/1000"), "{h:?}");
    let (st, _, body) = http(r.addr(), "GET", &format!("/api/down/{id}"), &[("Authorization", format!("Bearer {tok}")), ("Range", "bytes=-10".into())], b"");
    assert_eq!((st, body.len()), (206, 10));
    assert_eq!(http(r.addr(), "GET", &format!("/api/down/{id}"), &[("Authorization", format!("Bearer {tok}")), ("Range", "bytes=5000-".into())], b"").0, 416);
    assert_eq!(http(r.addr(), "GET", &format!("/api/down/{id}"), &bearer(&other), b"").0, 404, "another phone cannot fetch it");
    assert_eq!(http(r.addr(), "GET", "/api/down/nope", &bearer(&tok), b"").0, 404);
    p.send(json!({ "t": "file.done", "id": id, "ok": true }));
    let sent = r.wait("phone:files", |v| v["kind"] == "sent");
    assert_eq!((sent["transfer"]["dir"].as_str(), sent["transfer"]["status"].as_str()), (Some("out"), Some("done")));
    // too big for the cap
    let big = r.dir.join("big.bin");
    std::fs::write(&big, vec![0u8; 11 * 1024 * 1024]).unwrap();
    assert!(r.sh().send_file(&big, None).unwrap_err().contains("size cap"));
    assert!(r.sh().send_file(&r.dir, None).is_err(), "folders are not files");
    drop(p);
    r.stop();
}

#[test]
fn the_server_is_gone_after_stop_even_with_open_sockets() {
    let r = Rig::new(true);
    let tok = r.pair("dev-1");
    let mut p = Phone::connect(r.addr(), &tok);
    p.recv_t("hello");
    let addr = r.addr();
    let mut shared_only = r; // stop() asserts the port is closed
    let server = shared_only.server.take().unwrap();
    server.stop();
    let (op, _) = p.frame();
    assert_eq!(op, 8, "the phone gets a close frame");
    assert!(TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err());
    let _ = std::fs::remove_dir_all(&shared_only.dir);
}
