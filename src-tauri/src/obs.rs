// OBS producer for Settings > Live events > OBS (GateGuard note: new file of the Pill app, used by main.rs + src/live-ext.js + src/settings-live-ext.js;
// user request: "делай все оставшееся", part: Live-events producers OBS/Steam/Notifications).
// A minimal obs-websocket v5 client: plain RFC 6455 over std::net::TcpStream to 127.0.0.1:<live.obs.port> (default 4455), no new crates.
// LOCALHOST ONLY (the address is hard-coded, only the port is configurable). Runs only while the switch live.obs is on: no switch, no thread, no socket.
// Event to window "pill": `live:obs` {recording, streaming, recMs, streamMs, file?}; `file` (the saved recording) is only present on the stop event.
// The OBS password is typed once into Settings, kept DPAPI-encrypted in settings key `obs.passwordEnc` (net::secrets) and never logged or sent to the page.
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Listener};

const MAX_FRAME: usize = 1 << 20;
const OUTPUTS: u32 = 1 << 6; // eventSubscriptions: Outputs only

// ---------------------------------------------------------------- pure parts (unit-tested)
/// obs-websocket v5: secret = base64(sha256(password + salt)); auth = base64(sha256(secret + challenge)).
pub fn auth_string(password: &str, salt: &str, challenge: &str) -> String {
    let secret = B64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    B64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}

/// One client->server frame (always FIN, always masked as RFC 6455 demands).
pub fn encode_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut v = vec![0x80 | (opcode & 0x0f)];
    match payload.len() {
        n if n < 126 => v.push(0x80 | n as u8),
        n if n <= 0xffff => {
            v.push(0x80 | 126);
            v.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            v.push(0x80 | 127);
            v.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    v.extend_from_slice(&mask);
    v.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    v
}

#[derive(Debug, PartialEq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Decodes one frame from the front of `buf`: Ok(None) = need more bytes, Ok(Some((frame, consumed))). Handles masked and unmasked frames.
pub fn decode_frame(buf: &[u8]) -> Result<Option<(Frame, usize)>, &'static str> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let (fin, opcode) = (buf[0] & 0x80 != 0, buf[0] & 0x0f);
    let masked = buf[1] & 0x80 != 0;
    let mut len = (buf[1] & 0x7f) as usize;
    let mut pos = 2;
    if len == 126 {
        if buf.len() < 4 {
            return Ok(None);
        }
        len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        pos = 4;
    } else if len == 127 {
        if buf.len() < 10 {
            return Ok(None);
        }
        let n = u64::from_be_bytes(buf[2..10].try_into().unwrap());
        if n > MAX_FRAME as u64 {
            return Err("frame too large");
        }
        len = n as usize;
        pos = 10;
    }
    if len > MAX_FRAME {
        return Err("frame too large");
    }
    let mut key = [0u8; 4];
    if masked {
        if buf.len() < pos + 4 {
            return Ok(None);
        }
        key.copy_from_slice(&buf[pos..pos + 4]);
        pos += 4;
    }
    if buf.len() < pos + len {
        return Ok(None);
    }
    let payload: Vec<u8> = buf[pos..pos + len].iter().enumerate().map(|(i, b)| if masked { b ^ key[i % 4] } else { *b }).collect();
    Ok(Some((Frame { fin, opcode, payload }, pos + len)))
}

#[derive(Default, Clone, Debug, PartialEq)]
struct ObsState {
    recording: bool,
    streaming: bool,
    rec_ms: u64,
    stream_ms: u64,
    file: Option<String>, // set only by the event that stops a recording
}

/// Feeds one obs-websocket message (text JSON) into the state; true = recording/streaming changed (an event is due).
fn apply(st: &mut ObsState, v: &Value) -> bool {
    let d = &v["d"];
    let before = (st.recording, st.streaming);
    st.file = None;
    match v["op"].as_u64() {
        Some(5) => {
            let e = &d["eventData"];
            let active = e["outputActive"].as_bool().unwrap_or(false);
            match d["eventType"].as_str() {
                Some("RecordStateChanged") => {
                    st.recording = active;
                    st.rec_ms = 0;
                    if !active && e["outputState"].as_str() == Some("OBS_WEBSOCKET_OUTPUT_STOPPED") {
                        st.file = e["outputPath"].as_str().filter(|p| !p.is_empty()).map(str::to_string);
                    }
                }
                Some("StreamStateChanged") => {
                    st.streaming = active;
                    st.stream_ms = 0;
                }
                _ => {}
            }
        }
        Some(7) => {
            let r = &d["responseData"];
            let active = r["outputActive"].as_bool().unwrap_or(false);
            let ms = r["outputDuration"].as_u64().unwrap_or(0);
            match d["requestType"].as_str() {
                Some("GetRecordStatus") => {
                    st.recording = active;
                    st.rec_ms = if active { ms } else { 0 };
                }
                Some("GetStreamStatus") => {
                    st.streaming = active;
                    st.stream_ms = if active { ms } else { 0 };
                }
                _ => {}
            }
        }
        _ => {}
    }
    before != (st.recording, st.streaming) || st.file.is_some()
}

// ---------------------------------------------------------------- lifecycle: no switch = nothing exists
static GEN: AtomicU64 = AtomicU64::new(0);
static ACTIVE: Mutex<Option<u16>> = Mutex::new(None); // port of the running worker
static STATUS: Mutex<&'static str> = Mutex::new("off"); // off | searching | connected | auth
static RECENT: Mutex<Vec<String>> = Mutex::new(Vec::new()); // recordings OBS reported: the only paths obs_reveal accepts

fn set_status(s: &'static str) {
    if let Ok(mut g) = STATUS.lock() {
        *g = s;
    }
}

fn want_port() -> Option<u16> {
    if !crate::settings::get_bool("live.obs", false) {
        return None;
    }
    let p = crate::settings::get_f64("live.obs.port", 4455.0);
    Some(if (1.0..=65535.0).contains(&p) { p as u16 } else { 4455 })
}

fn sync(app: &AppHandle, force: bool) {
    let want = want_port();
    let mut a = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if *a == want && !force {
        return;
    }
    let g = crate::wake::bump(&GEN); // the old worker sees the new generation and ends by itself
    *a = want;
    match want {
        Some(port) => {
            set_status("searching");
            let app = app.clone();
            std::thread::spawn(move || run(app, g, port));
        }
        None => {
            set_status("off");
            let _ = app.emit_to("pill", "live:obs", json!({ "recording": false, "streaming": false, "recMs": 0, "streamMs": 0 }));
        }
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let h2 = h.clone();
    h.listen("settings:changed", move |_| sync(&h2, false));
    sync(&h, false);
}

fn password() -> Option<String> {
    let enc = crate::settings::get_str("obs.passwordEnc", "");
    if enc.trim().is_empty() {
        return None;
    }
    crate::net::secrets::unprotect(&enc).ok().filter(|p| !p.is_empty())
}

// ---------------------------------------------------------------- the connection
enum End {
    NoConnect,
    Closed,
    Auth,
}

fn mask_key(n: u64) -> [u8; 4] {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let h = Sha256::digest(format!("{nanos}:{n}").as_bytes());
    [h[0], h[1], h[2], h[3]]
}

fn send(s: &mut TcpStream, opcode: u8, payload: &[u8], n: &mut u64) -> bool {
    *n += 1;
    s.write_all(&encode_frame(opcode, payload, mask_key(*n))).is_ok()
}

/// Returns the stream and the bytes that arrived behind the HTTP response (already websocket frames).
fn connect(port: u16) -> Option<(TcpStream, Vec<u8>)> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).ok()?;
    let _ = s.set_nodelay(true);
    let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let key = B64.encode(&Sha256::digest(format!("{:?}", std::time::Instant::now()).as_bytes())[..16]);
    let req = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n");
    s.write_all(req.as_bytes()).ok()?;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 8192 {
            return None;
        }
        let n = s.read(&mut tmp).ok().filter(|n| *n > 0)?;
        buf.extend_from_slice(&tmp[..n]);
    };
    if !buf.starts_with(b"HTTP/1.1 101") {
        return None;
    }
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    Some((s, buf[end..].to_vec()))
}

fn emit(app: &AppHandle, st: &ObsState) {
    let mut p = json!({ "recording": st.recording, "streaming": st.streaming, "recMs": st.rec_ms, "streamMs": st.stream_ms });
    if let Some(f) = &st.file {
        p["file"] = json!(f);
        if let Ok(mut r) = RECENT.lock() {
            r.retain(|x| x != f);
            r.push(f.clone());
            if r.len() > 8 {
                r.remove(0);
            }
        }
    }
    let _ = app.emit_to("pill", "live:obs", p);
}

fn session(app: &AppHandle, gen: u64, port: u16) -> End {
    let Some((mut s, mut buf)) = connect(port) else { return End::NoConnect };
    let live = |g: &AtomicU64| g.load(Ordering::SeqCst) == gen;
    let (mut n, mut st, mut text, mut tmp) = (0u64, ObsState::default(), Vec::<u8>::new(), [0u8; 4096]);
    let mut shown = false;
    let end = 'outer: loop {
        if !live(&GEN) {
            break End::Closed;
        }
        loop {
            match decode_frame(&buf) {
                Err(_) => break 'outer End::Closed,
                Ok(None) => break,
                Ok(Some((f, used))) => {
                    buf.drain(..used);
                    match f.opcode {
                        9 => {
                            send(&mut s, 10, &f.payload, &mut n);
                        }
                        8 => {
                            let code = if f.payload.len() >= 2 { u16::from_be_bytes([f.payload[0], f.payload[1]]) } else { 0 };
                            break 'outer if code == 4009 || code == 4008 { End::Auth } else { End::Closed };
                        }
                        0 | 1 => {
                            text.extend_from_slice(&f.payload);
                            if !f.fin {
                                continue;
                            }
                            let msg: Option<Value> = serde_json::from_slice(&text).ok();
                            text.clear();
                            let Some(v) = msg else { continue };
                            match v["op"].as_u64() {
                                Some(0) => {
                                    let mut d = json!({ "rpcVersion": 1, "eventSubscriptions": OUTPUTS });
                                    if let (Some(ch), Some(salt)) = (v["d"]["authentication"]["challenge"].as_str(), v["d"]["authentication"]["salt"].as_str()) {
                                        let Some(pw) = password() else { break 'outer End::Auth };
                                        d["authentication"] = json!(auth_string(&pw, salt, ch));
                                    }
                                    if !send(&mut s, 1, json!({ "op": 1, "d": d }).to_string().as_bytes(), &mut n) {
                                        break 'outer End::Closed;
                                    }
                                }
                                Some(2) => {
                                    set_status("connected");
                                    for (t, id) in [("GetRecordStatus", "rec"), ("GetStreamStatus", "str")] {
                                        send(&mut s, 1, json!({ "op": 6, "d": { "requestType": t, "requestId": id } }).to_string().as_bytes(), &mut n);
                                    }
                                }
                                _ => {
                                    if apply(&mut st, &v) {
                                        shown = shown || st.recording || st.streaming;
                                        emit(app, &st);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        match s.read(&mut tmp) {
            Ok(0) => break End::Closed,
            Ok(k) => buf.extend_from_slice(&tmp[..k]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break End::Closed,
        }
    };
    let _ = s.shutdown(std::net::Shutdown::Both);
    if shown {
        let _ = app.emit_to("pill", "live:obs", json!({ "recording": false, "streaming": false, "recMs": 0, "streamMs": 0 })); // OBS went away: clear the chip
    }
    end
}

fn run(app: AppHandle, gen: u64, port: u16) {
    let mut backoff = 5u64;
    while GEN.load(Ordering::SeqCst) == gen {
        let end = session(&app, gen, port);
        if GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        set_status(if matches!(end, End::Auth) { "auth" } else { "searching" });
        // not found: stay quiet and back off 5 -> 30 s; closed after a session: retry soon; wrong password: slowly (Settings restarts us on a new one)
        let delay = match end {
            End::NoConnect => {
                let d = backoff;
                backoff = (backoff * 2).min(30);
                d
            }
            End::Closed => {
                backoff = 5;
                5
            }
            End::Auth => 30,
        };
        if !crate::wake::nap(&GEN, gen, delay * 1000) {
            return;
        }
    }
}

// ---------------------------------------------------------------- commands
#[tauri::command]
pub fn obs_status() -> Value {
    let state = *STATUS.lock().unwrap_or_else(|e| e.into_inner());
    json!({ "state": state, "port": want_port().unwrap_or_else(|| crate::settings::get_f64("live.obs.port", 4455.0) as u16), "hasPassword": !crate::settings::get_str("obs.passwordEnc", "").trim().is_empty() })
}

/// Stores the OBS WebSocket password DPAPI-encrypted (empty string clears it) and reconnects. The text never comes back to the page.
#[tauri::command]
pub fn obs_set_password(app: AppHandle, password: String) -> bool {
    let value = if password.is_empty() { Some(String::new()) } else { crate::net::secrets::protect(&password).ok() };
    let Some(v) = value else { return false };
    crate::settings::set("obs.passwordEnc", json!(v));
    sync(&app, true);
    true
}

/// "Show in folder" for a recording OBS reported (any other path is refused).
#[tauri::command]
pub fn obs_reveal(path: String) -> bool {
    use std::os::windows::process::CommandExt;
    let ok = RECENT.lock().map(|r| r.iter().any(|x| *x == path)).unwrap_or(false);
    let win = path.replace('/', "\\");
    if !ok || !std::path::Path::new(&win).exists() {
        return false;
    }
    std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{win}\"")).creation_flags(0x0800_0000).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_hash_matches_reference() {
        // reference values computed independently (python hashlib) for the v5 scheme
        let a = auth_string("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhTOcN2VKBHXR2Q=");
        assert_eq!(a, "l/pGTDimWppx9U8D65Od4vGsfKKFrCwcOVpxq/BEukA=");
    }

    #[test]
    fn frames_round_trip() {
        for len in [0usize, 5, 125, 126, 300, 70000] {
            let p: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let f = encode_frame(1, &p, [1, 2, 3, 4]);
            let (d, used) = decode_frame(&f).unwrap().unwrap();
            assert_eq!(used, f.len());
            assert_eq!((d.fin, d.opcode, d.payload), (true, 1, p));
            assert!(decode_frame(&f[..f.len() - 1]).unwrap().is_none()); // partial frame: wait for more
        }
    }

    #[test]
    fn decodes_unmasked_server_frame_and_rejects_huge() {
        let f = [0x81, 0x02, b'h', b'i', 0x88];
        let (d, used) = decode_frame(&f).unwrap().unwrap();
        assert_eq!((used, d.payload.as_slice()), (4, &b"hi"[..]));
        let mut big = vec![0x81, 127];
        big.extend_from_slice(&(u64::MAX).to_be_bytes());
        assert!(decode_frame(&big).is_err());
    }

    #[test]
    fn events_drive_state_and_file() {
        let mut st = ObsState::default();
        let ev = |t: &str, d: Value| json!({ "op": 5, "d": { "eventType": t, "eventData": d } });
        assert!(apply(&mut st, &ev("RecordStateChanged", json!({ "outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED", "outputPath": null }))));
        assert!(st.recording && st.file.is_none());
        assert!(apply(&mut st, &ev("StreamStateChanged", json!({ "outputActive": true, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED" }))));
        assert!(st.streaming);
        assert!(!apply(&mut st, &ev("RecordFileChanged", json!({ "newOutputPath": "C:/v/b.mkv" })))); // split file: no chip change
        assert!(apply(&mut st, &ev("RecordStateChanged", json!({ "outputActive": false, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED", "outputPath": "C:/v/a.mkv" }))));
        assert!(!st.recording && st.file.as_deref() == Some("C:/v/a.mkv"));
        let r = json!({ "op": 7, "d": { "requestType": "GetRecordStatus", "responseData": { "outputActive": true, "outputDuration": 61000 } } });
        assert!(apply(&mut st, &r) && st.recording && st.rec_ms == 61000);
    }
}
