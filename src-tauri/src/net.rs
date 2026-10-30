// L10 optional network. Owner: net/chat agent. THE ONLY module allowed to open a socket, with one exception: mail.rs (IMAP over TLS to the
// host the user typed, opt-in `live.mail`, off by default; WinHTTP cannot speak IMAP). Everything HTTP goes through here.
//
// * Every feature (weather, ai) is a settings switch, OFF by default (`net.weather`, `net.ai`); request() refuses when off.
// * Hosts are an allow-list per feature; redirects are followed by hand (GET only, max 3), each hop re-checked, never https -> http.
// * HTTP = WinHTTP through the `windows` crate (no extra dependency): Schannel TLS with the system trust store and
//   certificate verification left ON (we never touch WINHTTP_OPTION_SECURITY_FLAGS), system proxy (not for loopback),
//   timeouts, response size caps, User-Agent that names the app. No telemetry, no cookies, no auto-update.
// * Weather: MET Norway Locationforecast 2.0 compact + Nominatim city search; sunrise/sunset, feels-like and the
//   daily/hourly digest are computed locally (pure functions, `cargo test net`).
// * secrets: DPAPI (CryptProtectData, current user) for the AI key; the key is only ever sent to the configured endpoint.
use crate::ui::bus;
use serde_json::{json, Value};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::PCWSTR;
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_DECOMPRESSION,
    WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_RAW_HEADERS_CRLF, WINHTTP_QUERY_STATUS_CODE,
};

pub fn init(app: &tauri::App) {
    // The only background work: a 2 s tick that reads the settings cache and, when weather is on and a city is set,
    // refreshes the forecast for the capsule (see "Current temperature" below). No sockets while the feature is off.
    let _ = APP.set(app.handle().clone());
    std::thread::spawn(bg_loop);
}

// ---------------------------------------------------------------------------------------------------------------
// Features, allow-list, counters
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Feature {
    Weather,
    Ai,
    Voice, // voice.rs: dictation upload, gated by `voice.enabled`, host = exactly the one in `voice.endpoint`
    Setup, // aisetup.rs: user-clicked downloads (fixed host patterns) and loopback calls to Ollama; never goes through request()/enabled()
    Calendar, // liveplus.rs: .ics subscription (https:// or webcal:// in live.calendar.path), gated by `live.calendar`, host = exactly the one in that URL
}

const WEATHER_HOSTS: &[&str] = &["api.met.no", "nominatim.openstreetmap.org"];
static CALLS: [AtomicU64; 5] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

impl Feature {
    fn idx(self) -> usize {
        self as usize
    }
    fn key(self) -> &'static str {
        match self {
            Feature::Weather => "net.weather",
            Feature::Ai => "net.ai",
            Feature::Voice => "voice.enabled",
            Feature::Setup => "aisetup.never", // no such key: request() refuses Setup, aisetup.rs calls request_checked() on a click
            Feature::Calendar => "live.calendar",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Feature::Weather => "weather",
            Feature::Ai => "ai",
            Feature::Voice => "voice",
            Feature::Setup => "setup",
            Feature::Calendar => "calendar",
        }
    }
}

pub fn enabled(f: Feature) -> bool {
    // Settings > Modules > Weather off = no weather traffic at all, whatever net.weather says
    crate::settings::get_bool(f.key(), false) && (f != Feature::Weather || crate::settings::get_bool("module.weather", true))
}

/// Hosts a feature may call right now (weather: fixed list; ai: exactly the host of the configured base URL).
pub fn allowed_hosts(f: Feature) -> Vec<String> {
    match f {
        Feature::Weather => WEATHER_HOSTS.iter().map(|s| s.to_string()).collect(),
        Feature::Ai => parse_url(&crate::settings::get_str("ai.baseUrl", "")).map(|u| vec![u.host]).unwrap_or_default(),
        Feature::Voice => parse_url(&crate::voice::endpoint()).map(|u| vec![u.host]).unwrap_or_default(),
        Feature::Setup => Vec::new(), // aisetup.rs passes its own list per call
        Feature::Calendar => crate::liveplus::ics_url_host().into_iter().collect(),
    }
}

/// Which network features are on and which hosts they may call, plus how many calls were made since start.
pub fn net_features_status() -> Value {
    let mut active: Vec<String> = Vec::new();
    let mut feats: Vec<Value> = [Feature::Weather, Feature::Ai, Feature::Voice, Feature::Calendar]
        .iter()
        .map(|&f| {
            let on = enabled(f);
            let hosts = allowed_hosts(f);
            if on {
                active.extend(hosts.iter().cloned());
            }
            json!({ "id": f.name(), "on": on, "hosts": hosts, "calls": CALLS[f.idx()].load(Ordering::Relaxed) })
        })
        .collect();
    let m = crate::mail::net_feature(); // mail.rs (IMAP) is the one other thing that talks to a server
    if m["on"] == true {
        active.extend(m["hosts"].as_array().into_iter().flatten().filter_map(|h| h.as_str().map(str::to_string)));
    }
    feats.push(m);
    active.sort();
    active.dedup();
    json!({
        "features": feats,
        "active_hosts": active,
        "total_calls": CALLS.iter().map(|c| c.load(Ordering::Relaxed)).sum::<u64>(),
    })
}

fn user_agent() -> String {
    // MET Norway requires a descriptive User-Agent with contact info; the user can set `net.contact` (email or site).
    let c: String = crate::settings::get_str("net.contact", "").chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(80).collect();
    let c = c.trim();
    if c.is_empty() {
        "Pill/0.1 (personal desktop widget; no telemetry)".into()
    } else {
        format!("Pill/0.1 (personal desktop widget; contact: {c})")
    }
}

// ---------------------------------------------------------------------------------------------------------------
// URL handling
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    pub path: String, // path + query, always starts with '/'
}

/// Strict http(s) URL parser: ASCII only, no userinfo, no whitespace/backslashes, optional [v6] host.
pub fn parse_url(s: &str) -> Result<Url, String> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| (0x21..0x7f).contains(&b)) || s.contains('\\') {
        return Err("invalid URL".into());
    }
    let low = s.to_ascii_lowercase();
    let (https, rest) = if low.starts_with("https://") {
        (true, &s[8..])
    } else if low.starts_with("http://") {
        (false, &s[7..])
    } else {
        return Err("URL must start with http:// or https://".into());
    };
    let rest = rest.split('#').next().unwrap_or("");
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (auth, tail) = rest.split_at(end);
    if auth.is_empty() || auth.contains('@') {
        return Err("invalid URL host".into());
    }
    let (host, port_s) = if let Some(r) = auth.strip_prefix('[') {
        let close = r.find(']').ok_or("invalid IPv6 host")?;
        let after = &r[close + 1..];
        (r[..close].to_string(), after.strip_prefix(':').unwrap_or(if after.is_empty() { "" } else { "x" }))
    } else {
        match auth.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p),
            None => (auth.to_string(), ""),
        }
    };
    let port = if port_s.is_empty() {
        if https { 443 } else { 80 }
    } else {
        port_s.parse::<u16>().ok().filter(|p| *p != 0).ok_or("invalid port")?
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':') {
        return Err("invalid URL host".into());
    }
    let path = if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('?') {
        format!("/{tail}")
    } else {
        tail.to_string()
    };
    Ok(Url { https, host, port, path })
}

pub fn is_loopback(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']);
    h == "localhost" || h == "::1" || h.parse::<std::net::Ipv4Addr>().map(|a| a.is_loopback()).unwrap_or(false)
}

/// Allow-list entry vs host. Exact match, or one `*` wildcard whose part may only be [a-z0-9.-] ("cdn-lfs*.huggingface.co",
/// "*.cdn.hf.co"); the text around the `*` keeps its dots, so "evilhuggingface.co" never matches "*.huggingface.co".
pub fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == host,
        Some((pre, suf)) => {
            host.len() >= pre.len() + suf.len()
                && host.starts_with(pre)
                && host.ends_with(suf)
                && host.as_bytes()[pre.len()..host.len() - suf.len()].iter().copied().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        }
    }
}

/// Resolve a Location header against the URL that produced it.
fn resolve_location(base: &Url, loc: &str) -> Result<String, String> {
    let loc = loc.trim();
    let scheme = if base.https { "https" } else { "http" };
    let hostp = if base.host.contains(':') { format!("[{}]", base.host) } else { base.host.clone() };
    if loc.starts_with("http://") || loc.starts_with("https://") {
        Ok(loc.to_string())
    } else if loc.starts_with('/') && !loc.starts_with("//") {
        Ok(format!("{scheme}://{hostp}:{}{loc}", base.port))
    } else {
        Err("unsupported redirect".into())
    }
}

fn urlenc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

// ---------------------------------------------------------------------------------------------------------------
// WinHTTP client
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>, // names lower-cased
    pub body: Vec<u8>,
}

impl Resp {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Lets another thread abort a running request (chat stop button): closing the request handle makes the blocked read fail at once.
#[derive(Clone, Default)]
pub struct Abort(Arc<AbortInner>);
#[derive(Default)]
struct AbortInner {
    flag: AtomicBool,
    handle: AtomicIsize,
}
impl Abort {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_set(&self) -> bool {
        self.0.flag.load(Ordering::SeqCst)
    }
    pub fn abort(&self) {
        self.0.flag.store(true, Ordering::SeqCst);
        let h = self.0.handle.swap(0, Ordering::SeqCst);
        if h != 0 {
            unsafe {
                let _ = WinHttpCloseHandle(h as *mut c_void);
            }
        }
    }
}

struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// The request handle: closed by Drop unless Abort::abort() already closed it (the swap decides who closes).
struct ReqHandle {
    h: *mut c_void,
    abort: Option<Abort>,
}
impl Drop for ReqHandle {
    fn drop(&mut self) {
        let mine = match &self.abort {
            Some(a) => a.0.handle.swap(0, Ordering::SeqCst) == self.h as isize,
            None => true,
        };
        if mine && !self.h.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.h);
            }
        }
    }
}

pub struct Opts<'a> {
    pub feature: Feature,
    pub method: &'a str,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<&'a [u8]>,
    pub max_body: usize,
    pub recv_ms: u32,
    pub decompress: bool,
    pub abort: Option<&'a Abort>,
}

/// Send timeout: 15 s for weather / chat; dictation uploads (up to ~3.8 MB) get 20 s + 1 s per 100 KB, at most 90 s.
fn send_timeout_ms(f: Feature, body_len: usize) -> i32 {
    match f {
        Feature::Voice => (20_000 + (body_len / 102_400) as i32 * 1000).min(90_000),
        _ => 15_000,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn werr(what: &str, e: &windows::core::Error, host: &str) -> String {
    format!("{what} {host}: {}", e.message().trim())
}

type OnData<'a, 'b> = Option<&'a mut (dyn FnMut(&[u8]) -> bool + 'b)>;

/// One HTTP exchange without redirect following. 2xx bodies go to `on_data` when given (return false to stop), everything else is collected.
fn hop(o: &Opts, u: &Url, ua: &str, mut on_data: OnData) -> Result<Resp, String> {
    if o.abort.is_some_and(|a| a.is_set()) {
        return Err("stopped".into());
    }
    unsafe {
        let agent = wide(ua);
        let access = if is_loopback(&u.host) { WINHTTP_ACCESS_TYPE_NO_PROXY } else { WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY };
        let session = Handle(WinHttpOpen(PCWSTR(agent.as_ptr()), access, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err(werr("cannot open network session", &windows::core::Error::from_win32(), ""));
        }
        let send_ms = send_timeout_ms(o.feature, o.body.map_or(0, |b| b.len()));
        let _ = WinHttpSetTimeouts(session.0, 8000, 8000, send_ms, o.recv_ms.min(i32::MAX as u32) as i32);
        let host = wide(&u.host);
        let conn = Handle(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), u.port, 0));
        if conn.0.is_null() {
            return Err(werr("cannot connect to", &windows::core::Error::from_win32(), &u.host));
        }
        let verb = wide(o.method);
        let path = wide(&u.path);
        let flags = if u.https { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let rh = WinHttpOpenRequest(conn.0, PCWSTR(verb.as_ptr()), PCWSTR(path.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags);
        if rh.is_null() {
            return Err(werr("cannot open request to", &windows::core::Error::from_win32(), &u.host));
        }
        let req = ReqHandle { h: rh, abort: o.abort.cloned() };
        if let Some(a) = o.abort {
            a.0.handle.store(rh as isize, Ordering::SeqCst);
            if a.is_set() {
                return Err("stopped".into()); // abort() ran before the handle was registered
            }
        }
        // No automatic redirects (we check every hop against the allow-list ourselves); certificate checks stay at the default (on).
        let _ = WinHttpSetOption(Some(rh as *const c_void), WINHTTP_OPTION_REDIRECT_POLICY, Some(&WINHTTP_OPTION_REDIRECT_POLICY_NEVER.to_le_bytes()));
        if o.decompress {
            let _ = WinHttpSetOption(Some(rh as *const c_void), WINHTTP_OPTION_DECOMPRESSION, Some(&3u32.to_le_bytes())); // gzip + deflate
        }
        let mut hdr = String::new();
        for (k, v) in o.headers {
            let v: String = v.chars().filter(|c| *c != '\r' && *c != '\n').collect();
            hdr.push_str(&format!("{k}: {v}\r\n"));
        }
        let hdr_w: Vec<u16> = hdr.encode_utf16().collect();
        let (bptr, blen) = match o.body {
            Some(b) => (Some(b.as_ptr() as *const c_void), b.len() as u32),
            None => (None, 0),
        };
        let hs = if hdr_w.is_empty() { None } else { Some(&hdr_w[..]) };
        WinHttpSendRequest(rh, hs, bptr, blen, blen, 0).map_err(|e| werr("request to", &e, &u.host))?;
        WinHttpReceiveResponse(rh, std::ptr::null_mut()).map_err(|e| {
            if o.abort.is_some_and(|a| a.is_set()) { "stopped".to_string() } else { werr("no response from", &e, &u.host) }
        })?;

        let mut status: u32 = 0;
        let mut len = 4u32;
        let _ = WinHttpQueryHeaders(rh, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut c_void), &mut len, std::ptr::null_mut());
        let mut hbuf = vec![0u16; 16384];
        let mut hlen = (hbuf.len() * 2) as u32;
        let mut headers = Vec::new();
        if WinHttpQueryHeaders(rh, WINHTTP_QUERY_RAW_HEADERS_CRLF, PCWSTR::null(), Some(hbuf.as_mut_ptr() as *mut c_void), &mut hlen, std::ptr::null_mut()).is_ok() {
            let raw = String::from_utf16_lossy(&hbuf[..(hlen as usize / 2).min(hbuf.len())]);
            headers = parse_raw_headers(&raw);
        }
        let mut resp = Resp { status: status as u16, headers, body: Vec::new() };
        if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(resp); // the caller decides; the body of a redirect is not needed
        }
        let stream = resp.status / 100 == 2 && on_data.is_some();
        let mut buf = vec![0u8; 16384];
        let mut total = 0usize;
        loop {
            if o.abort.is_some_and(|a| a.is_set()) {
                return Err("stopped".into());
            }
            let mut n = 0u32;
            if let Err(e) = WinHttpReadData(rh, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut n) {
                return Err(if o.abort.is_some_and(|a| a.is_set()) { "stopped".to_string() } else { werr("connection to", &e, &u.host) + " dropped" });
            }
            if n == 0 {
                break;
            }
            total += n as usize;
            if total > o.max_body {
                return Err(format!("response from {} is larger than the {} KB cap", u.host, o.max_body / 1024));
            }
            let chunk = &buf[..n as usize];
            if stream {
                if let Some(cb) = on_data.as_mut() {
                    if !cb(chunk) {
                        break;
                    }
                }
            } else {
                resp.body.extend_from_slice(chunk);
            }
        }
        drop(req);
        Ok(resp)
    }
}

fn parse_raw_headers(raw: &str) -> Vec<(String, String)> {
    raw.trim_end_matches('\0')
        .split("\r\n")
        .skip(1) // status line
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect()
}

/// Gate + allow-list + redirect policy around hop(). The only function the rest of the app calls to reach the network.
pub fn request(o: &Opts, mut on_data: OnData) -> Result<Resp, String> {
    if !enabled(o.feature) {
        return Err(format!("network feature '{}' is off", o.feature.name()));
    }
    request_checked(o, &mut on_data, &allowed_hosts(o.feature))
}

pub fn request_checked(o: &Opts, on_data: &mut OnData, allow: &[String]) -> Result<Resp, String> {
    let ua = user_agent();
    let mut url = o.url.to_string();
    for _ in 0..6 { // ollama.com -> github.com -> github.com -> release CDN is already 4 hops
        let u = parse_url(&url)?;
        if !allow.iter().any(|h| host_matches(h, &u.host)) {
            return Err(format!("host {} is not on the allow-list for '{}'", u.host, o.feature.name()));
        }
        if !u.https && !is_loopback(&u.host) {
            return Err("refusing plain http to a non-local host (use https)".into());
        }
        CALLS[o.feature.idx()].fetch_add(1, Ordering::Relaxed);
        let r = hop(o, &u, &ua, on_data.as_mut().map(|c| &mut **c as &mut dyn FnMut(&[u8]) -> bool))?;
        if matches!(r.status, 301 | 302 | 303 | 307 | 308) {
            if o.method != "GET" {
                return Err(format!("unexpected redirect ({}) from {}", r.status, u.host));
            }
            let loc = r.header("location").ok_or("redirect without Location")?;
            url = resolve_location(&u, loc)?;
            continue;
        }
        return Ok(r);
    }
    Err("too many redirects".into())
}

// ---------------------------------------------------------------------------------------------------------------
// secrets: DPAPI (current user), base64 text blob for settings.json
// ---------------------------------------------------------------------------------------------------------------
pub mod secrets {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::ffi::c_void;
    use windows::core::w;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    const ENTROPY: &[u8] = b"pill.ai.apikey.v1"; // binds the blob to this app's use

    fn blob(b: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: b.len() as u32, pbData: b.as_ptr() as *mut u8 }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        unsafe {
            let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
            let _ = LocalFree(HLOCAL(out.pbData as *mut c_void));
            v
        }
    }

    pub fn protect(plain: &str) -> Result<String, String> {
        let input = blob(plain.as_bytes());
        let ent = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        unsafe { CryptProtectData(&input, w!("pill"), Some(&ent), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|e| format!("cannot encrypt the key: {}", e.message().trim()))?;
        Ok(STANDARD.encode(take(out)))
    }

    pub fn unprotect(b64: &str) -> Result<String, String> {
        let raw = STANDARD.decode(b64.trim()).map_err(|_| "stored key is damaged".to_string())?;
        let input = blob(&raw);
        let ent = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        unsafe { CryptUnprotectData(&input, None, Some(&ent), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|_| "cannot decrypt the stored key (made by another Windows user?). Enter it again.".to_string())?;
        String::from_utf8(take(out)).map_err(|_| "stored key is damaged".to_string())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// AI endpoint (OpenAI-compatible); chat.rs does the orchestration, all sockets stay here
// ---------------------------------------------------------------------------------------------------------------

pub struct AiCfg {
    pub base: String, // base URL without trailing slash, e.g. http://127.0.0.1:11434/v1
    pub model: String,
    pub key: Option<String>,
}

/// Validates the user's endpoint: switch on, URL sane, plain http only for loopback.
pub fn check_ai_base(base: &str) -> Result<String, String> {
    let u = parse_url(base)?;
    if !u.https && !is_loopback(&u.host) {
        return Err("Refused: plain http to a non-local host would send your key unencrypted. Use https:// or a 127.0.0.1 / localhost address.".into());
    }
    Ok(base.trim().trim_end_matches('/').to_string())
}

pub fn ai_cfg(model: Option<&str>, need_model: bool) -> Result<AiCfg, String> {
    if !enabled(Feature::Ai) {
        return Err("AI is switched off (enable it in the chat settings).".into());
    }
    let base = crate::settings::get_str("ai.baseUrl", "");
    if base.trim().is_empty() {
        return Err("Set the API address in the AI settings.".into());
    }
    let base = check_ai_base(&base)?;
    let model = model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string).unwrap_or_else(|| crate::settings::get_str("ai.model", "").trim().to_string());
    if need_model && model.is_empty() {
        return Err("Choose a model in the AI settings.".into());
    }
    let enc = crate::settings::get_str("ai.apiKeyEnc", "");
    let key = if enc.trim().is_empty() { None } else { Some(secrets::unprotect(&enc)?) };
    Ok(AiCfg { base, model, key })
}

fn ai_headers<'a>(key: &'a Option<String>, auth: &'a mut String) -> Vec<(&'a str, &'a str)> {
    let mut h = vec![("Accept", "application/json, text/event-stream")];
    if let Some(k) = key {
        *auth = format!("Bearer {}", k.trim());
        h.push(("Authorization", auth.as_str()));
    }
    h
}

/// POST {base}/chat/completions. With `on_data` the 2xx body is streamed (SSE) to the callback; `abort` can cut it short.
pub fn ai_chat(cfg: &AiCfg, body: &[u8], recv_ms: u32, abort: Option<&Abort>, on_data: OnData) -> Result<Resp, String> {
    let url = if cfg.base.ends_with("/chat/completions") { cfg.base.clone() } else { format!("{}/chat/completions", cfg.base) };
    let mut auth = String::new();
    let mut hdrs = ai_headers(&cfg.key, &mut auth);
    hdrs.push(("Content-Type", "application/json"));
    request(&Opts { feature: Feature::Ai, method: "POST", url: &url, headers: &hdrs, body: Some(body), max_body: 32 << 20, recv_ms, decompress: false, abort }, on_data)
}

/// GET {base}/models (best effort, used to fill the model picker).
pub fn ai_models(cfg: &AiCfg) -> Result<Resp, String> {
    let base = cfg.base.trim_end_matches("/chat/completions");
    let url = format!("{base}/models");
    let mut auth = String::new();
    let hdrs = ai_headers(&cfg.key, &mut auth);
    request(&Opts { feature: Feature::Ai, method: "GET", url: &url, headers: &hdrs, body: None, max_body: 1 << 20, recv_ms: 10_000, decompress: false, abort: None }, None)
}

/// GET {base}/models on a loopback server (local AI discovery). The allow-list is loopback only, so it works
/// whatever base URL is saved; still gated by the AI switch.
pub fn ai_probe(base: &str) -> Result<Resp, String> {
    if !enabled(Feature::Ai) {
        return Err("AI is switched off".into());
    }
    let url = format!("{}/models", base.trim_end_matches('/'));
    let hdrs = [("Accept", "application/json")];
    let o = Opts { feature: Feature::Ai, method: "GET", url: &url, headers: &hdrs, body: None, max_body: 1 << 20, recv_ms: 2500, decompress: false, abort: None };
    request_checked(&o, &mut None, &["127.0.0.1".to_string()])
}

// ---------------------------------------------------------------------------------------------------------------
// Time helpers (no tz database: the page passes the PC's UTC offset)
// ---------------------------------------------------------------------------------------------------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// "2026-10-23T18:00:00Z" (fraction and +00:00 tolerated) -> unix seconds.
fn parse_iso(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se)
}

/// "Fri, 23 Oct 2026 18:30:00 GMT" -> unix seconds.
fn parse_http_date(s: &str) -> Option<i64> {
    let p: Vec<&str> = s.split_whitespace().collect();
    if p.len() < 5 {
        return None;
    }
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| m.eq_ignore_ascii_case(p[2]))? as i64 + 1;
    let hms: Vec<i64> = p[4].split(':').filter_map(|x| x.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    Some(days_from_civil(p[3].parse().ok()?, mon, p[1].parse().ok()?) * 86400 + hms[0] * 3600 + hms[1] * 60 + hms[2])
}

// ---------------------------------------------------------------------------------------------------------------
// Sun + comfort maths (pure)
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum Sun {
    /// minutes after 00:00 UTC of the *location's solar date* (may be < 0 or > 1440 far from Greenwich)
    Normal { rise_min: f64, set_min: f64 },
    PolarDay,
    PolarNight,
}

/// Equation of time (minutes) and solar declination (radians) for Julian century `jc` (NOAA general solar position formulas).
fn eqtime_decl(jc: f64) -> (f64, f64) {
    let l0 = (280.46646 + jc * (36000.76983 + jc * 0.0003032)).rem_euclid(360.0).to_radians();
    let m = (357.52911 + jc * (35999.05029 - 0.0001537 * jc)).to_radians();
    let e = 0.016708634 - jc * (0.000042037 + 0.0000001267 * jc);
    let c = m.sin() * (1.914602 - jc * (0.004817 + 0.000014 * jc)) + (2.0 * m).sin() * (0.019993 - 0.000101 * jc) + (3.0 * m).sin() * 0.000289;
    let true_long = l0.to_degrees() + c;
    let omega = (125.04 - 1934.136 * jc).to_radians();
    let lambda = (true_long - 0.00569 - 0.00478 * omega.sin()).to_radians();
    let seconds = 21.448 - jc * (46.815 + jc * (0.00059 - jc * 0.001813));
    let obliq = (23.0 + (26.0 + seconds / 60.0) / 60.0 + 0.00256 * omega.cos()).to_radians();
    let decl = (obliq.sin() * lambda.sin()).asin();
    let y = (obliq / 2.0).tan().powi(2);
    let eq = y * (2.0 * l0).sin() - 2.0 * e * m.sin() + 4.0 * e * y * m.sin() * (2.0 * l0).cos() - 0.5 * y * y * (4.0 * l0).sin() - 1.25 * e * e * (2.0 * m).sin();
    (4.0 * eq.to_degrees(), decl)
}

/// Sunrise / sunset (NOAA algorithm, 90.833 deg zenith incl. refraction) for the date y-m-d at lat/lon (degrees, east positive).
pub fn sun_times(lat: f64, lon: f64, y: i64, m: i64, d: i64) -> Sun {
    let jd0 = days_from_civil(y, m, d) as f64 + 2440587.5; // JD at 00:00 UT
    let at = |min_utc: f64| ((jd0 + min_utc / 1440.0) - 2451545.0) / 36525.0;
    let ha = |decl: f64| {
        let la = lat.to_radians();
        (90.833f64.to_radians().cos() / (la.cos() * decl.cos()) - la.tan() * decl.tan()).clamp(-2.0, 2.0)
    };
    let (_, decl0) = eqtime_decl(at(720.0 - 4.0 * lon));
    let c = ha(decl0);
    if c > 1.0 {
        return Sun::PolarNight;
    }
    if c < -1.0 {
        return Sun::PolarDay;
    }
    // Two passes: evaluate the sun's position at the event time itself.
    let mut rise = 720.0 - 4.0 * (lon + c.acos().to_degrees());
    let mut set = 720.0 - 4.0 * (lon - c.acos().to_degrees());
    for _ in 0..2 {
        let (eq_r, de_r) = eqtime_decl(at(rise));
        let (eq_s, de_s) = eqtime_decl(at(set));
        let (cr, cs) = (ha(de_r), ha(de_s));
        if cr.abs() > 1.0 || cs.abs() > 1.0 {
            break;
        }
        rise = 720.0 - 4.0 * (lon + cr.acos().to_degrees()) - eq_r;
        set = 720.0 - 4.0 * (lon - cs.acos().to_degrees()) - eq_s;
    }
    Sun::Normal { rise_min: rise, set_min: set }
}

/// "Feels like" in deg C: Environment Canada wind chill at <= 10 C, BoM apparent temperature (humidity + wind) above, blended 10..24 C.
pub fn feels_like(t: f64, wind_ms: f64, rh: f64) -> f64 {
    if t <= 10.0 {
        let v = wind_ms * 3.6;
        return if v > 4.8 { 13.12 + 0.6215 * t - 11.37 * v.powf(0.16) + 0.3965 * t * v.powf(0.16) } else { t };
    }
    let e = rh.clamp(0.0, 100.0) / 100.0 * 6.105 * (17.27 * t / (237.7 + t)).exp();
    let at = t + 0.33 * e - 0.70 * wind_ms - 4.0;
    let w = ((t - 10.0) / 14.0).clamp(0.0, 1.0);
    t * (1.0 - w) + at * w
}

// ---------------------------------------------------------------------------------------------------------------
// MET Norway digest (pure)
// ---------------------------------------------------------------------------------------------------------------

struct Entry {
    t: i64, // unix s
    temp: f64,
    wind: f64,
    dir: f64,
    rh: f64,
    cloud: f64,
    sym: Option<String>,  // best available symbol code (1h, else 6h, else 12h)
    sym6: Option<String>, // 6h symbol (used for the day icon)
    p1: Option<f64>,
    p6: Option<f64>,
}

fn parse_met(body: &str) -> Result<(Vec<Entry>, Option<i64>), String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "forecast is not valid JSON".to_string())?;
    let ts = v.pointer("/properties/timeseries").and_then(Value::as_array).ok_or("forecast has no timeseries")?;
    let sc = |e: &Value, n: &str| e.pointer(&format!("/data/{n}/summary/symbol_code")).and_then(Value::as_str).map(str::to_string);
    let pr = |e: &Value, n: &str| e.pointer(&format!("/data/{n}/details/precipitation_amount")).and_then(Value::as_f64);
    let mut out = Vec::new();
    for e in ts {
        let (Some(t), Some(temp)) = (e.get("time").and_then(Value::as_str).and_then(parse_iso), e.pointer("/data/instant/details/air_temperature").and_then(Value::as_f64)) else { continue };
        let g = |k: &str| e.pointer(&format!("/data/instant/details/{k}")).and_then(Value::as_f64);
        out.push(Entry {
            t,
            temp,
            wind: g("wind_speed").unwrap_or(0.0),
            dir: g("wind_from_direction").unwrap_or(0.0),
            rh: g("relative_humidity").unwrap_or(50.0),
            cloud: g("cloud_area_fraction").unwrap_or(0.0),
            sym: sc(e, "next_1_hours").or_else(|| sc(e, "next_6_hours")).or_else(|| sc(e, "next_12_hours")),
            sym6: sc(e, "next_6_hours").or_else(|| sc(e, "next_12_hours")).or_else(|| sc(e, "next_1_hours")),
            p1: pr(e, "next_1_hours"),
            p6: pr(e, "next_6_hours"),
        });
    }
    if out.is_empty() {
        return Err("forecast is empty".into());
    }
    out.sort_by_key(|e| e.t);
    let upd = v.pointer("/properties/meta/updated_at").and_then(Value::as_str).and_then(parse_iso);
    Ok((out, upd))
}

/// MET symbol code -> our own icon id (the page draws it): clear-day, clear-night, partly-day, partly-night, cloud, fog, rain, sleet, snow, thunder.
pub fn icon_for(code: &str) -> &'static str {
    let night = code.ends_with("_night");
    let base = code.split('_').next().unwrap_or("");
    if base.contains("thunder") {
        "thunder"
    } else if base.contains("sleet") {
        "sleet"
    } else if base.contains("snow") {
        "snow"
    } else if base.contains("rain") {
        "rain"
    } else if base == "fog" {
        "fog"
    } else if base == "partlycloudy" {
        if night { "partly-night" } else { "partly-day" }
    } else if base == "clearsky" || base == "fair" {
        if night { "clear-night" } else { "clear-day" }
    } else {
        "cloud"
    }
}

fn base_code(code: &str) -> String {
    code.split('_').next().unwrap_or("").to_string()
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Digest the MET compact forecast into what the weather tab shows. `tz_min` = PC's UTC offset in minutes (day boundaries, weekday).
pub fn build_weather(body: &str, lat: f64, lon: f64, now_ms: i64, tz_min: i64) -> Result<Value, String> {
    let (es, updated) = parse_met(body)?;
    let now_s = now_ms.div_euclid(1000);
    let cur = es.iter().rposition(|e| e.t <= now_s).unwrap_or(0);
    let c = &es[cur];
    let day_of = |t: i64| (t + tz_min * 60).div_euclid(86400);
    let today = day_of(now_s);

    // Daily digest: today (from the current entry on) and the following days.
    struct Day { min: f64, max: f64, precip: f64, pick: Option<(f64, String)> }
    let mut days: std::collections::BTreeMap<i64, Day> = std::collections::BTreeMap::new();
    for e in es.iter().skip(cur) {
        let k = day_of(e.t);
        if k < today {
            continue;
        }
        let d = days.entry(k).or_insert(Day { min: f64::MAX, max: f64::MIN, precip: 0.0, pick: None });
        d.min = d.min.min(e.temp);
        d.max = d.max.max(e.temp);
        d.precip += e.p1.or(e.p6).unwrap_or(0.0);
        let hour = ((e.t + tz_min * 60).rem_euclid(86400)) as f64 / 3600.0;
        if let Some(s) = &e.sym6 {
            let dist = (hour - 13.0).abs();
            if d.pick.as_ref().map_or(true, |(bd, _)| dist < *bd) {
                d.pick = Some((dist, s.clone()));
            }
        }
    }
    if let Some(d) = days.get_mut(&today) {
        d.min = d.min.min(c.temp);
        d.max = d.max.max(c.temp);
    }
    let daily: Vec<Value> = days
        .iter()
        .take(7)
        .map(|(k, d)| {
            let sym = d.pick.as_ref().map(|(_, s)| base_code(s)).unwrap_or_else(|| "cloudy".into());
            json!({
                "t": (k * 86400 - tz_min * 60) * 1000, "dow": (k + 4).rem_euclid(7), "min": r1(d.min), "max": r1(d.max),
                "symbol": sym, "icon": icon_for(&format!("{sym}_day")), "precip": r1(d.precip),
            })
        })
        .collect();

    let hourly: Vec<Value> = es
        .iter()
        .skip(cur)
        .take(8)
        .map(|e| {
            let s = e.sym.clone().unwrap_or_else(|| "cloudy".into());
            json!({ "t": e.t * 1000, "temp": r1(e.temp), "symbol": base_code(&s), "icon": icon_for(&s), "precip": r1(e.p1.unwrap_or(0.0)) })
        })
        .collect();

    let sym = c.sym.clone().unwrap_or_else(|| "cloudy".into());
    let today_d = days.get(&today);
    let now = json!({
        "temp": r1(c.temp), "feels": r1(feels_like(c.temp, c.wind, c.rh)), "symbol": base_code(&sym), "icon": icon_for(&sym),
        "wind": r1(c.wind), "dir": c.dir.round(), "humidity": c.rh.round(), "cloud": c.cloud.round(),
        "precip": r1(c.p1.unwrap_or(0.0)), "min": r1(today_d.map_or(c.temp, |d| d.min)), "max": r1(today_d.map_or(c.temp, |d| d.max)),
    });

    // Sun: the location's own solar date (UTC now shifted by longitude), shown in the PC's time zone by the page.
    let solar_day = (now_s + (lon * 240.0) as i64).div_euclid(86400);
    let (y, m, d) = civil_from_days(solar_day);
    let sun = match sun_times(lat, lon, y, m, d) {
        Sun::Normal { rise_min, set_min } => {
            let base = solar_day * 86400_000;
            json!({ "state": "normal", "rise": base + (rise_min * 60_000.0) as i64, "set": base + (set_min * 60_000.0) as i64 })
        }
        Sun::PolarDay => json!({ "state": "polar_day", "rise": null, "set": null }),
        Sun::PolarNight => json!({ "state": "polar_night", "rise": null, "set": null }),
    };

    Ok(json!({ "updated": updated.map(|u| u * 1000), "now": now, "hourly": hourly, "daily": daily, "sun": sun, "tz": tz_min }))
}

// ---------------------------------------------------------------------------------------------------------------
// Weather commands
// ---------------------------------------------------------------------------------------------------------------

static WEATHER_LOCK: Mutex<()> = Mutex::new(());
static NOMINATIM_LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);

fn cache_path() -> std::path::PathBuf {
    crate::store::dir().join("weather-cache.json")
}

struct Cached {
    lat: f64,
    lon: f64,
    fetched: i64,
    expires: i64,
    last_modified: String,
    body: String,
}

fn load_cache() -> Option<Cached> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(cache_path()).ok()?).ok()?;
    Some(Cached {
        lat: v.get("lat")?.as_f64()?,
        lon: v.get("lon")?.as_f64()?,
        fetched: v.get("fetched")?.as_i64()?,
        expires: v.get("expires")?.as_i64()?,
        last_modified: v.get("last_modified").and_then(Value::as_str).unwrap_or("").to_string(),
        body: v.get("body")?.as_str()?.to_string(),
    })
}

fn save_cache(c: &Cached) {
    let v = json!({ "lat": c.lat, "lon": c.lon, "fetched": c.fetched, "expires": c.expires, "last_modified": c.last_modified, "body": c.body });
    let p = cache_path();
    let tmp = p.with_extension("json.tmp");
    if serde_json::to_vec(&v).ok().is_some_and(|b| std::fs::write(&tmp, b).is_ok()) {
        let _ = std::fs::rename(&tmp, &p);
    }
}

const CACHE_MIN_MS: i64 = 30 * 60 * 1000;

/// Forecast text for lat/lon: disk cache for 30 min (and never before MET's Expires), If-Modified-Since afterwards, stale copy on failure.
/// Returns (body, fetched_ms, stale_error).
fn forecast_body(lat: f64, lon: f64, force: bool) -> Result<(String, i64, Option<String>), String> {
    let (lat, lon) = ((lat * 1e4).round() / 1e4, (lon * 1e4).round() / 1e4); // MET: at most 4 decimals
    let now = crate::store::now_ms();
    let cached = load_cache().filter(|c| (c.lat - lat).abs() < 1e-6 && (c.lon - lon).abs() < 1e-6);
    if let Some(c) = &cached {
        let fresh_until = if force { c.expires } else { c.expires.max(c.fetched + CACHE_MIN_MS) };
        if now < fresh_until && now >= c.fetched - 60_000 {
            return Ok((c.body.clone(), c.fetched, None));
        }
    }
    let url = format!("https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={lat:.4}&lon={lon:.4}");
    let ims = cached.as_ref().map(|c| c.last_modified.clone()).filter(|s| !s.is_empty());
    let mut hdrs: Vec<(&str, &str)> = vec![("Accept", "application/json")];
    if let Some(s) = &ims {
        hdrs.push(("If-Modified-Since", s));
    }
    let res = request(&Opts { feature: Feature::Weather, method: "GET", url: &url, headers: &hdrs, body: None, max_body: 2 << 20, recv_ms: 20_000, decompress: true, abort: None }, None);
    let fail = |msg: String| match &cached {
        Some(c) => Ok((c.body.clone(), c.fetched, Some(msg))),
        None => Err(msg),
    };
    match res {
        Ok(r) if r.status == 200 => {
            let body = r.text();
            parse_met(&body)?; // never cache garbage
            let exp = r.header("expires").and_then(parse_http_date).map(|s| s * 1000).unwrap_or(now);
            save_cache(&Cached { lat, lon, fetched: now, expires: exp, last_modified: r.header("last-modified").unwrap_or("").to_string(), body: body.clone() });
            Ok((body, now, None))
        }
        Ok(r) if r.status == 304 && cached.is_some() => {
            let c = cached.unwrap();
            let exp = r.header("expires").and_then(parse_http_date).map(|s| s * 1000).unwrap_or(now);
            let nc = Cached { fetched: now, expires: exp, ..c };
            save_cache(&nc);
            Ok((nc.body, now, None))
        }
        Ok(r) if r.status == 403 || r.status == 429 => fail(format!("MET Norway refused the request (HTTP {}). Set a contact e-mail or site in the weather settings and try later.", r.status)),
        Ok(r) => fail(format!("MET Norway answered HTTP {}", r.status)),
        Err(e) => fail(e),
    }
}

/// Weather for the stored city. Always returns an object with `status`: off | nocity | ok | error.
/// `tz` = the PC's UTC offset in minutes (JS: -new Date().getTimezoneOffset()).
pub fn weather_get(force: Option<bool>, tz: Option<i64>) -> Value {
    if !enabled(Feature::Weather) {
        return json!({ "status": "off" });
    }
    let city = crate::settings::get_str("weather.city", "");
    let (lat, lon) = (crate::settings::get_f64("weather.lat", 999.0), crate::settings::get_f64("weather.lon", 999.0));
    if city.trim().is_empty() || lat.abs() > 90.0 || lon.abs() > 180.0 {
        return json!({ "status": "nocity" });
    }
    let _g = WEATHER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match forecast_body(lat, lon, force.unwrap_or(false)) {
        Ok((body, fetched, stale)) => match build_weather(&body, lat, lon, crate::store::now_ms(), tz.unwrap_or(0).clamp(-840, 840)) {
            Ok(mut v) => {
                v["status"] = json!("ok");
                v["city"] = json!(city);
                v["lat"] = json!(lat);
                v["lon"] = json!(lon);
                v["fetched"] = json!(fetched);
                v["stale"] = json!(stale.is_some());
                v["warning"] = json!(stale);
                publish(&v, &city, fetched, stale.is_none());
                v
            }
            Err(e) => json!({ "status": "error", "message": e }),
        },
        Err(e) => json!({ "status": "error", "message": e }),
    }
}

/// City search through Nominatim (1 request per second, only on an explicit search, never as-you-type).
pub fn weather_search_city(query: String, lang: Option<String>) -> Value {
    if !enabled(Feature::Weather) {
        return json!({ "status": "off" });
    }
    let q: String = query.chars().filter(|c| !c.is_control()).take(100).collect();
    if q.trim().len() < 2 {
        return json!({ "status": "ok", "results": [] });
    }
    let al = lang.filter(|l| l.len() <= 5 && l.chars().all(|c| c.is_ascii_alphabetic() || c == '-')).unwrap_or_else(|| "en".into());
    let url = format!("https://nominatim.openstreetmap.org/search?q={}&format=jsonv2&limit=6&addressdetails=1&accept-language={}", urlenc(q.trim()), al);
    let res = {
        let mut last = NOMINATIM_LAST.lock().unwrap_or_else(|e| e.into_inner()); // held while waiting: serialises searches
        if let Some(t) = *last {
            let el = t.elapsed();
            if el < std::time::Duration::from_millis(1100) {
                std::thread::sleep(std::time::Duration::from_millis(1100) - el);
            }
        }
        let r = request(&Opts { feature: Feature::Weather, method: "GET", url: &url, headers: &[("Accept", "application/json")], body: None, max_body: 512 << 10, recv_ms: 15_000, decompress: true, abort: None }, None);
        *last = Some(std::time::Instant::now());
        r
    };
    match res {
        Ok(r) if r.status == 200 => match parse_nominatim(&r.text()) {
            Some(results) => json!({ "status": "ok", "results": results }),
            None => json!({ "status": "error", "message": "unexpected answer from the city search" }),
        },
        Ok(r) => json!({ "status": "error", "message": format!("City search answered HTTP {}", r.status) }),
        Err(e) => json!({ "status": "error", "message": e }),
    }
}

fn parse_nominatim(body: &str) -> Option<Vec<Value>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let a = v.as_array()?;
    Some(
        a.iter()
            .filter_map(|r| {
                let lat = r.get("lat")?.as_str()?.parse::<f64>().ok()?;
                let lon = r.get("lon")?.as_str()?.parse::<f64>().ok()?;
                let addr = r.get("address");
                let ad = |k: &str| addr.and_then(|a| a.get(k)).and_then(Value::as_str);
                let name = r.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).or(ad("city")).or(ad("town")).or(ad("village"))?;
                let region = ad("state").or(ad("county")).unwrap_or("");
                let country = ad("country").unwrap_or("");
                let detail = [region, country].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(", ");
                Some(json!({ "name": name, "detail": detail, "lat": lat, "lon": lon }))
            })
            .collect(),
    )
}

/// Remember the chosen city and its coordinates (settings.json keys weather.city / weather.lat / weather.lon).
pub fn weather_set_city(name: String, lat: f64, lon: f64) -> bool {
    let name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
    if name.trim().is_empty() || !(lat.abs() <= 90.0 && lon.abs() <= 180.0) {
        return false;
    }
    crate::settings::set("weather.city", json!(name.trim()));
    crate::settings::set("weather.lat", json!(lat));
    crate::settings::set("weather.lon", json!(lon));
    let _ = std::fs::remove_file(cache_path());
    true
}

// ---------------------------------------------------------------------------------------------------------------
// Current temperature for the collapsed pill: event `weather:current`, command `weather_current`, background refresh
// ---------------------------------------------------------------------------------------------------------------

static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

#[derive(Clone)]
struct Cur {
    temp_c: f64,
    code: String,
    icon: String,
    city: String,
    updated: i64,
}
static CURRENT: Mutex<Option<Cur>> = Mutex::new(None);

struct Sched {
    key: String,       // city + coordinates the state belongs to
    last: i64,         // last attempt (ms; a successful one uses the fetch time)
    expires: i64,      // MET Expires of the cached forecast (ms)
    errors: u32,       // consecutive failures
    resume_until: i64, // no request before this (machine just woke from sleep)
}
static SCHED: Mutex<Sched> = Mutex::new(Sched { key: String::new(), last: 0, expires: 0, errors: 0, resume_until: 0 });

fn sched() -> std::sync::MutexGuard<'static, Sched> {
    SCHED.lock().unwrap_or_else(|e| e.into_inner())
}

fn current_lock() -> std::sync::MutexGuard<'static, Option<Cur>> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner())
}

/// When the next background request may happen (ms). Pure.
/// Without errors: not before MET's Expires and not within 30 min of the last fetch. After errors: exponential backoff
/// 1, 2, 4, 8, 16, 30 min (capped) from the last attempt, still never before Expires. Never before `resume_until`.
/// An absurd Expires (> 6 h ahead) is clamped so a bad header cannot freeze the capsule.
pub fn next_attempt(now: i64, expires: i64, last: i64, errors: u32, resume_until: i64) -> i64 {
    let wait = if errors == 0 { CACHE_MIN_MS } else { (60_000i64 << (errors - 1).min(5)).min(CACHE_MIN_MS) };
    (last + wait).max(expires.min(now + 6 * 3600_000)).max(resume_until)
}

fn cond_text(sym: &str) -> &'static str {
    if sym.contains("thunder") {
        "Thunderstorm"
    } else if sym.contains("sleet") {
        "Sleet"
    } else if sym.contains("snow") {
        "Snow"
    } else if sym.contains("rain") {
        "Rain"
    } else {
        match sym {
            "clearsky" => "Clear",
            "fair" => "Mostly clear",
            "partlycloudy" => "Partly cloudy",
            "fog" => "Fog",
            _ => "Cloudy",
        }
    }
}

fn current_payload(c: &Cur, imperial: bool) -> Value {
    let t = if imperial { c.temp_c * 9.0 / 5.0 + 32.0 } else { c.temp_c };
    json!({ "temp": t.round(), "unit": if imperial { "F" } else { "C" }, "code": c.code, "icon": c.icon, "text": cond_text(&c.code), "city": c.city, "updatedAt": c.updated })
}

fn imperial() -> bool {
    crate::settings::get_str("weather.units", "metric") == "imperial"
}

fn emit_current(v: Value) {
    if let Some(_a) = APP.get() {
        let _ = bus::emit("weather:current", v);
    }
}

/// Remember the current conditions of a loaded forecast and tell the capsule. `ok` = fresh data, not a stale fallback.
fn publish(v: &Value, city: &str, fetched: i64, ok: bool) {
    let n = &v["now"];
    if let (Some(t), Some(code), Some(icon)) = (n["temp"].as_f64(), n["symbol"].as_str(), n["icon"].as_str()) {
        let c = Cur { temp_c: t, code: code.into(), icon: icon.into(), city: city.into(), updated: fetched };
        let p = current_payload(&c, imperial());
        *current_lock() = Some(c);
        emit_current(p);
    }
    let exp = load_cache().map(|c| c.expires).unwrap_or(0);
    let mut s = sched();
    if ok {
        s.last = fetched;
        s.errors = 0;
    }
    s.expires = exp;
}

/// The cached current conditions for the capsule; null when off, no city, or nothing loaded yet. Never touches the network.
pub fn weather_current() -> Value {
    if !enabled(Feature::Weather) {
        return Value::Null;
    }
    let city = crate::settings::get_str("weather.city", "");
    match current_lock().as_ref() {
        Some(c) if c.city == city && !city.is_empty() => current_payload(c, imperial()),
        _ => Value::Null,
    }
}

fn bg_loop() {
    // let the app settle: the first refresh happens ~5 s after launch so the capsule shows the temperature at once
    std::thread::sleep(std::time::Duration::from_secs(3));
    let mut last_wall = crate::store::now_ms();
    let mut announced = false; // a value (or an off message) was sent for the current state
    let mut shown_unit = imperial();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let now = crate::store::now_ms();
        if now - last_wall > 20_000 {
            sched().resume_until = now + 20_000; // the machine just woke up: the network may not be back yet
        }
        last_wall = now;

        let city = crate::settings::get_str("weather.city", "");
        let (lat, lon) = (crate::settings::get_f64("weather.lat", 999.0), crate::settings::get_f64("weather.lon", 999.0));
        let usable = enabled(Feature::Weather) && !city.trim().is_empty() && lat.abs() <= 90.0 && lon.abs() <= 180.0;
        if !usable {
            // feature switched off (or no city): drop the temperature from the capsule at once
            if announced || current_lock().take().is_some() {
                *current_lock() = None;
                emit_current(json!({ "off": true }));
            }
            announced = false;
            continue;
        }
        announced = true;
        let key = format!("{city}|{lat:.4}|{lon:.4}");
        {
            let mut s = sched();
            if s.key != key {
                let resume = s.resume_until;
                *s = Sched { key, last: 0, expires: 0, errors: 0, resume_until: resume };
            }
        }
        if imperial() != shown_unit {
            // units switched in the tab: re-send the cached value in the new unit
            shown_unit = imperial();
            let p = weather_current();
            if !p.is_null() {
                emit_current(p);
            }
        }
        if crate::pill::is_hidden() {
            continue; // nobody sees the capsule: no fetch; the first tick after pill:show catches up if the forecast is due
        }
        let due = {
            let s = sched();
            now >= next_attempt(now, s.expires, s.last, s.errors, s.resume_until)
        };
        if !due {
            continue;
        }
        let Ok(_g) = WEATHER_LOCK.try_lock() else { continue }; // one request at a time (the tab may be loading right now)
        let r = forecast_body(lat, lon, false).and_then(|(body, fetched, stale)| build_weather(&body, lat, lon, crate::store::now_ms(), 0).map(|v| (v, fetched, stale)));
        match r {
            Ok((v, fetched, None)) => publish(&v, &city, fetched, true),
            Ok((v, _, Some(_))) => {
                // the network failed but an old copy exists: show it and back off
                publish(&v, &city, crate::store::now_ms(), false);
                let mut s = sched();
                s.last = now;
                s.errors += 1;
            }
            Err(_) => {
                let mut s = sched();
                s.last = now;
                s.errors += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tests: synthetic fixtures only, no real network (the one HTTP test talks to a listener on 127.0.0.1 inside the test)
// ---------------------------------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn hm(min: f64) -> (i64, i64) {
        let m = min.rem_euclid(1440.0);
        ((m / 60.0) as i64, (m % 60.0) as i64)
    }

    #[test]
    fn net_sun_kyiv_matches_reference() {
        // 2026-10-23, Kyiv: sunrise ~04:32 UTC (07:32 EEST), sunset ~14:51 UTC (17:51 EEST)
        let Sun::Normal { rise_min, set_min } = sun_times(50.4501, 30.5234, 2026, 10, 23) else { panic!("polar?") };
        assert!((rise_min - (4.0 * 60.0 + 32.0)).abs() < 4.0, "rise {:?}", hm(rise_min));
        assert!((set_min - (14.0 * 60.0 + 51.0)).abs() < 4.0, "set {:?}", hm(set_min));
    }

    #[test]
    fn net_sun_london_solstice_and_polar() {
        // 2026-06-21 London: 03:43 UTC / 20:21 UTC
        let Sun::Normal { rise_min, set_min } = sun_times(51.5074, -0.1278, 2026, 6, 21) else { panic!() };
        assert!((rise_min - (3.0 * 60.0 + 43.0)).abs() < 4.0, "rise {:?}", hm(rise_min));
        assert!((set_min - (20.0 * 60.0 + 21.0)).abs() < 4.0, "set {:?}", hm(set_min));
        assert_eq!(sun_times(69.65, 18.96, 2026, 12, 21), Sun::PolarNight);
        assert_eq!(sun_times(69.65, 18.96, 2026, 6, 21), Sun::PolarDay);
        // equator, equinox: ~12 h of daylight
        let Sun::Normal { rise_min, set_min } = sun_times(0.0, 0.0, 2026, 3, 20) else { panic!() };
        assert!(((set_min - rise_min) - 727.0).abs() < 6.0, "len {}", set_min - rise_min);
    }

    #[test]
    fn net_time_helpers() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-10-23T18:00:00Z"), Some(1792778400));
        assert_eq!(parse_http_date("Fri, 23 Oct 2026 18:00:00 GMT"), Some(1792778400));
        assert_eq!(parse_iso("garbage"), None);
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 23)), (2026, 10, 23));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn net_feels_like() {
        assert!((feels_like(-10.0, 5.0, 80.0) - (-17.5)).abs() < 0.5); // wind chill, 18 km/h
        assert_eq!(feels_like(5.0, 1.0, 50.0), 5.0); // calm cold air: no change
        assert!(feels_like(32.0, 1.0, 70.0) > 32.0); // humid heat feels hotter
        assert!((feels_like(15.0, 3.0, 60.0) - 15.0).abs() < 3.0);
    }

    #[test]
    fn net_icons() {
        assert_eq!(icon_for("clearsky_night"), "clear-night");
        assert_eq!(icon_for("partlycloudy_day"), "partly-day");
        assert_eq!(icon_for("lightrainshowersandthunder_day"), "thunder");
        assert_eq!(icon_for("heavysleet"), "sleet");
        assert_eq!(icon_for("lightsnowshowers_night"), "snow");
        assert_eq!(icon_for("fog"), "fog");
        assert_eq!(icon_for("cloudy"), "cloud");
        assert_eq!(icon_for("something_new"), "cloud");
    }

    #[test]
    fn net_voice_send_timeout_scales() {
        assert_eq!(send_timeout_ms(Feature::Weather, 5 << 20), 15_000);
        assert_eq!(send_timeout_ms(Feature::Ai, 0), 15_000);
        assert_eq!(send_timeout_ms(Feature::Voice, 0), 20_000);
        assert_eq!(send_timeout_ms(Feature::Voice, 3_840_000), 57_000);
        assert_eq!(send_timeout_ms(Feature::Voice, 100 << 20), 90_000);
    }

    #[test]
    fn net_urls_and_policy() {
        let u = parse_url("https://api.met.no/weatherapi/x?lat=1&lon=2").unwrap();
        assert_eq!((u.https, u.host.as_str(), u.port, u.path.as_str()), (true, "api.met.no", 443, "/weatherapi/x?lat=1&lon=2"));
        assert_eq!(parse_url("http://127.0.0.1:11434/v1").unwrap().port, 11434);
        assert_eq!(parse_url("http://[::1]:3001").unwrap().host, "::1");
        assert!(parse_url("https://user:pw@evil.com/").is_err());
        assert!(parse_url("ftp://x").is_err());
        assert!(parse_url("https://a b/").is_err());
        assert!(parse_url("https://host:99999/").is_err());
        assert!(is_loopback("localhost") && is_loopback("127.0.0.1") && is_loopback("::1") && !is_loopback("192.168.1.5") && !is_loopback("example.com"));
        assert!(check_ai_base("http://127.0.0.1:11434/v1/").is_ok());
        assert!(check_ai_base("https://api.example.com/v1").is_ok());
        assert!(check_ai_base("http://api.example.com/v1").unwrap_err().contains("Refused"));
        assert!(check_ai_base("http://192.168.1.10:11434/v1").is_err());
        let base = parse_url("https://api.met.no/a").unwrap();
        assert_eq!(resolve_location(&base, "/b?x=1").unwrap(), "https://api.met.no:443/b?x=1");
        assert_eq!(resolve_location(&base, "https://other.example/z").unwrap(), "https://other.example/z");
        assert!(resolve_location(&base, "//evil.com/").is_err());
    }

    /// Hourly (3 days) then 6-hourly entries from 2026-10-23T18:00Z; rain from 2026-10-24T02:00Z for two hours.
    fn fixture() -> String {
        let t0 = parse_iso("2026-10-23T18:00:00Z").unwrap();
        let mut ts = Vec::new();
        let mut t = t0;
        while t < t0 + 9 * 86400 {
            let h = (t - t0) / 3600;
            let hourly = h < 60;
            let rain = (8..10).contains(&h);
            let night = ((t % 86400) / 3600) as i64 >= 16 || ((t % 86400) / 3600) as i64 <= 4;
            let sfx = if night { "night" } else { "day" };
            let mut data = json!({
                "instant": { "details": { "air_temperature": 10.0 + ((t / 3600) % 24) as f64 / 2.0, "wind_speed": 3.0, "wind_from_direction": 200.0, "relative_humidity": 85.0, "cloud_area_fraction": 90.0 } },
                "next_6_hours": { "summary": { "symbol_code": if rain { "rain".to_string() } else { format!("partlycloudy_{sfx}") } }, "details": { "precipitation_amount": if rain { 1.0 } else { 0.0 } } },
            });
            if hourly {
                data["next_1_hours"] = json!({ "summary": { "symbol_code": if rain { "lightrain".to_string() } else { format!("cloudy") } }, "details": { "precipitation_amount": if rain { 0.5 } else { 0.0 } } });
            }
            ts.push(json!({ "time": format!("{}", fmt_iso(t)), "data": data }));
            t += if hourly { 3600 } else { 6 * 3600 };
        }
        json!({ "type": "Feature", "properties": { "meta": { "updated_at": "2026-10-23T17:50:00Z" }, "timeseries": ts } }).to_string()
    }

    fn fmt_iso(t: i64) -> String {
        let (y, m, d) = civil_from_days(t.div_euclid(86400));
        let s = t.rem_euclid(86400);
        format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, (s % 3600) / 60, s % 60)
    }

    #[test]
    fn net_build_weather_digest() {
        let now = (parse_iso("2026-10-23T18:20:00Z").unwrap()) * 1000;
        let v = build_weather(&fixture(), 50.45, 30.52, now, 180).unwrap();
        assert_eq!(v["hourly"].as_array().unwrap().len(), 8);
        assert_eq!(v["hourly"][0]["t"].as_i64().unwrap(), parse_iso("2026-10-23T18:00:00Z").unwrap() * 1000);
        assert_eq!(v["hourly"][8 - 1]["precip"].as_f64().unwrap(), 0.0);
        let days = v["daily"].as_array().unwrap();
        assert_eq!(days.len(), 7);
        for d in days {
            assert!(d["min"].as_f64().unwrap() <= d["max"].as_f64().unwrap());
        }
        // tz +3: entry 18:00Z is 21:00 local on the 23rd (Friday = dow 5); next local day starts 21:00Z
        assert_eq!(days[0]["dow"], 5);
        assert_eq!(days[1]["dow"], 6);
        // rain: 2 x 0.5 mm hourly entries at 02:00Z and 03:00Z (05:00, 06:00 local on the 24th)
        assert!((days[1]["precip"].as_f64().unwrap() - 1.0).abs() < 0.11, "{}", days[1]["precip"]);
        assert_eq!(days[1]["icon"].as_str().unwrap().is_empty(), false);
        assert_eq!(v["now"]["icon"], "cloud");
        assert_eq!(v["now"]["humidity"], 85.0);
        assert_eq!(v["sun"]["state"], "normal");
        let rise = v["sun"]["rise"].as_i64().unwrap();
        let set = v["sun"]["set"].as_i64().unwrap();
        assert!(rise < set && (set - rise) > 8 * 3600_000 && (set - rise) < 12 * 3600_000);
        assert!(build_weather("{}", 0.0, 0.0, now, 0).is_err());
        assert!(build_weather("not json", 0.0, 0.0, now, 0).is_err());
    }

    #[test]
    fn net_nominatim_parse() {
        let body = r#"[{"lat":"50.4500336","lon":"30.5241361","name":"Kyiv","address":{"city":"Kyiv","state":"Kyiv","country":"Ukraine"}},{"lat":"x","lon":"1"}]"#;
        let r = parse_nominatim(body).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0]["name"], "Kyiv");
        assert_eq!(r[0]["detail"], "Kyiv, Ukraine");
        assert!(parse_nominatim("nope").is_none());
    }

    #[test]
    fn net_header_parse() {
        let h = parse_raw_headers("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nExpires: Fri, 23 Oct 2026 18:00:00 GMT\r\n\r\n\0");
        assert!(h.iter().any(|(k, v)| k == "content-type" && v == "text/plain"));
        assert!(h.iter().any(|(k, _)| k == "expires"));
    }

    /// WinHTTP against a canned server on 127.0.0.1 (bypasses the settings gate: calls hop() directly).
    fn serve_once(reply: &'static [u8]) -> u16 {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply);
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        });
        port
    }

    fn opts<'a>(url: &'a str, method: &'a str, body: Option<&'a [u8]>) -> Opts<'a> {
        Opts { feature: Feature::Ai, method, url, headers: &[("X-Test", "1")], body, max_body: 1 << 20, recv_ms: 5000, decompress: false, abort: None }
    }

    #[test]
    fn net_http_loopback_stream_and_redirect() {
        let port = serve_once(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 19\r\nConnection: close\r\n\r\ndata: hi\n\ndata: x\n\n");
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let u = parse_url(&url).unwrap();
        let mut got = Vec::new();
        let mut cb = |c: &[u8]| {
            got.extend_from_slice(c);
            true
        };
        let r = hop(&opts(&url, "POST", Some(b"{}")), &u, "Pill-test", Some(&mut cb)).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.header("content-type"), Some("text/event-stream"));
        assert!(String::from_utf8_lossy(&got).starts_with("data: hi"));

        let port = serve_once(b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let url = format!("http://127.0.0.1:{port}/");
        let u = parse_url(&url).unwrap();
        let r = hop(&opts(&url, "GET", None), &u, "Pill-test", None).unwrap();
        assert_eq!(r.status, 302);
        // request_checked must refuse to follow it: evil.example is not allow-listed
        let port = serve_once(b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let url = format!("http://127.0.0.1:{port}/");
        let e = request_checked(&opts(&url, "GET", None), &mut None, &["127.0.0.1".to_string()]).unwrap_err();
        assert!(e.contains("allow-list"), "{e}");
        // a host that is not allow-listed is refused before any socket is opened
        let e = request_checked(&opts("http://127.0.0.1:1/", "GET", None), &mut None, &["example.com".to_string()]).unwrap_err();
        assert!(e.contains("allow-list"));
    }

    #[test]
    fn net_next_attempt_schedule() {
        let (now, m) = (10_000_000i64, 60_000i64);
        // never fetched: due immediately
        assert!(next_attempt(now, 0, 0, 0, 0) <= now);
        // fresh success, Expires earlier than the 30 min floor: the floor wins
        assert_eq!(next_attempt(now, now + 10 * m, now, 0, 0), now + 30 * m);
        // Expires later than the floor: Expires wins
        assert_eq!(next_attempt(now, now + 45 * m, now, 0, 0), now + 45 * m);
        // errors: 1, 2, 8 min ... capped at 30 min
        assert_eq!(next_attempt(now, 0, now, 1, 0), now + m);
        assert_eq!(next_attempt(now, 0, now, 2, 0), now + 2 * m);
        assert_eq!(next_attempt(now, 0, now, 4, 0), now + 8 * m);
        assert_eq!(next_attempt(now, 0, now, 30, 0), now + 30 * m);
        // backoff never goes before MET's Expires
        assert_eq!(next_attempt(now, now + 20 * m, now, 1, 0), now + 20 * m);
        // just resumed from sleep: wait until resume_until
        assert_eq!(next_attempt(now, 0, now - 40 * m, 0, now + 20_000), now + 20_000);
        // a silly Expires (a week ahead) is clamped to 6 h
        assert_eq!(next_attempt(now, now + 7 * 24 * 60 * m, now, 0, 0), now + 6 * 60 * m);
    }

    #[test]
    fn net_current_payload_units() {
        let c = Cur { temp_c: 15.4, code: "cloudy".into(), icon: "cloud".into(), city: "Kyiv".into(), updated: 5 };
        let p = current_payload(&c, false);
        assert_eq!((p["temp"].as_f64(), p["unit"].as_str(), p["text"].as_str(), p["city"].as_str()), (Some(15.0), Some("C"), Some("Cloudy"), Some("Kyiv")));
        let p = current_payload(&c, true);
        assert_eq!((p["temp"].as_f64(), p["unit"].as_str(), p["updatedAt"].as_i64()), (Some(60.0), Some("F"), Some(5)));
    }

    #[test]
    fn net_http_abort_unblocks_read() {
        use std::io::Read;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                use std::io::Write;
                let mut b = [0u8; 2048];
                let _ = s.read(&mut b);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5\r\ndata:\r\n");
                std::thread::sleep(std::time::Duration::from_secs(5)); // then silence
            }
        });
        let url = format!("http://127.0.0.1:{port}/");
        let u = parse_url(&url).unwrap();
        let ab = Abort::new();
        let ab2 = ab.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            ab2.abort();
        });
        let t = std::time::Instant::now();
        let mut cb = |_: &[u8]| true;
        let mut o = opts(&url, "GET", None);
        o.abort = Some(&ab);
        o.recv_ms = 30_000;
        let r = hop(&o, &u, "Pill-test", Some(&mut cb));
        assert!(r.is_err());
        assert!(t.elapsed() < std::time::Duration::from_secs(4), "abort did not unblock the read");
    }
}
