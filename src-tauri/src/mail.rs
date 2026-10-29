// New mail -> island card, over IMAP. New file of the Pill app, used by main.rs + src/live-plus.js + src/settings-live-ext.js; user request: «Новое письмо»
// notifications, opt-in. Owner: live-events lane.
//  - live:mail {from, subject, more}   one card per poll: the newest message that arrived since the last poll (+ how many more). Switch live.mail, OFF by default.
// Settings: mail.host, mail.port (993), mail.user, mail.mailbox (INBOX), mail.passwordEnc (the password, DPAPI-encrypted by net::secrets, never plain text,
// never sent to the page, never logged). The state file %APPDATA%\pill\mail-state.json holds {account hash, UIDVALIDITY, highest UID seen}, no mail content.
// What goes over the wire (implicit TLS only, the certificate is checked by the Windows store; port 143 / plain IMAP is refused):
//   LOGIN, EXAMINE (read-only SELECT), UID SEARCH UNSEEN, UID FETCH <newest> (BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)]), LOGOUT.
// Nothing here can mark a message read or download a body or an attachment. Own thread (a slow server never delays liveplus.rs), 2 min between polls,
// 15 s minimum between any two polls, exponential back-off up to 30 min after errors, catch_unwind around every tick.
// net.rs says it is the only module that opens sockets; this is the one exception (WinHTTP cannot speak IMAP), it only talks to mail.host.
// The parsing, RFC 2047 decoding, UID dedup and the scheduler are pure and unit-tested on canned data; the session runs on an in-memory stream.
use crate::ui::bus::{self, Target};
use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle};

const TICK: Duration = Duration::from_secs(2);
const EVERY_MS: u64 = 120_000; // poll interval
const MIN_GAP_MS: u64 = 15_000; // never two polls closer than this (settings changes, off/on toggling)
const MAX_BACKOFF_MS: u64 = 30 * 60_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(20);
const SESSION_MS: u64 = 45_000; // a whole session (connect, login, search, fetch) is cut after this
const MAX_LINE: usize = 1 << 20;
const MAX_LITERAL: usize = 256 << 10;
const MAX_RESPONSE: usize = 4 << 20;

static POLLS: AtomicU64 = AtomicU64::new(0);

// ================= IMAP client =================

struct Imap<S: Read + Write> {
    io: BufReader<S>,
    n: u32,
    deadline: Instant,
}

struct Reply {
    untagged: Vec<Vec<u8>>, // every untagged response with its literals inlined, CRLF included
    status: String,         // OK | NO | BAD (upper case)
    text: String,
}

fn io_text(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        TimedOut | WouldBlock => "сервер не отвечает (таймаут)".into(),
        _ => "соединение с сервером оборвалось".into(),
    }
}

/// A literal announcement `{123}` at the end of a response line.
fn literal_len(line: &[u8]) -> Option<usize> {
    let l = line.strip_suffix(b"\r\n").or_else(|| line.strip_suffix(b"\n")).unwrap_or(line);
    let l = l.strip_suffix(b"}")?;
    let open = l.iter().rposition(|&b| b == b'{')?;
    let digits = &l[open + 1..];
    if digits.is_empty() || digits.len() > 9 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

impl<S: Read + Write> Imap<S> {
    fn new(io: S, deadline: Instant) -> Self {
        Imap { io: BufReader::new(io), n: 0, deadline }
    }

    fn line(&mut self) -> Result<Vec<u8>, String> {
        if Instant::now() > self.deadline {
            return Err("сервер отвечает слишком долго (таймаут)".into());
        }
        let mut v = Vec::new();
        let n = (&mut self.io).take(MAX_LINE as u64).read_until(b'\n', &mut v).map_err(|e| io_text(&e))?;
        if n == 0 || v.last() != Some(&b'\n') {
            return Err("соединение с сервером оборвалось".into());
        }
        Ok(v)
    }

    /// One response: its lines with all literals inlined.
    fn response(&mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        loop {
            let l = self.line()?;
            let lit = literal_len(&l);
            out.extend_from_slice(&l);
            let Some(n) = lit else { return Ok(out) };
            if n > MAX_LITERAL || out.len() + n > MAX_RESPONSE {
                return Err("сервер прислал слишком большой ответ".into());
            }
            let at = out.len();
            out.resize(at + n, 0);
            self.io.read_exact(&mut out[at..]).map_err(|e| io_text(&e))?;
        }
    }

    fn greeting(&mut self) -> Result<(), String> {
        let g = self.response()?;
        if g.starts_with(b"* OK") || g.starts_with(b"* PREAUTH") {
            Ok(())
        } else {
            Err("это не IMAP-сервер или он отказал в соединении".into())
        }
    }

    fn cmd(&mut self, text: &str) -> Result<Reply, String> {
        self.n += 1;
        let tag = format!("a{}", self.n);
        let w = self.io.get_mut();
        w.write_all(format!("{tag} {text}\r\n").as_bytes()).and_then(|_| w.flush()).map_err(|e| io_text(&e))?;
        let (mut untagged, mut size) = (Vec::new(), 0usize);
        loop {
            let r = self.response()?;
            if r.starts_with(tag.as_bytes()) && r.get(tag.len()) == Some(&b' ') {
                let rest = String::from_utf8_lossy(&r[tag.len() + 1..]).trim().to_string();
                let (st, text) = rest.split_once(' ').unwrap_or((rest.as_str(), ""));
                return Ok(Reply { untagged, status: st.to_ascii_uppercase(), text: text.to_string() });
            }
            if r.starts_with(b"* BYE") {
                return Err("сервер закрыл соединение".into());
            }
            size += r.len();
            if size > MAX_RESPONSE {
                return Err("сервер прислал слишком большой ответ".into());
            }
            if r.starts_with(b"* ") {
                untagged.push(r);
            }
        }
    }
}

/// IMAP quoted string. Line breaks / NUL cannot be quoted (and would be command injection), so they are refused.
fn quote(s: &str) -> Result<String, String> {
    if s.bytes().any(|b| b == b'\r' || b == b'\n' || b == 0) {
        return Err("в логине, пароле или названии папки есть недопустимые символы".into());
    }
    Ok(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
}

fn auth_text(server_text: &str) -> String {
    let l = server_text.to_lowercase();
    if l.contains("application-specific") || l.contains("app password") || l.contains("app-specific") || l.contains("webalert") || l.contains("web login") {
        "Сервер требует пароль приложения, а не обычный пароль от почты: создайте его в настройках безопасности аккаунта".into()
    } else {
        "Вход отклонён: неверный логин или пароль, либо в почте выключен доступ по IMAP (для Gmail, Яндекса и Mail.ru нужен пароль приложения)".into()
    }
}

struct Selected {
    validity: Option<u32>,
    uidnext: Option<u32>,
}

/// `[NAME 123]` in the untagged `* OK [...]` lines.
fn resp_code(untagged: &[Vec<u8>], name: &str) -> Option<u32> {
    let key = format!("[{name} ");
    untagged.iter().find_map(|l| {
        let s = String::from_utf8_lossy(l);
        let at = s.find(&key)? + key.len();
        s[at..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
    })
}

/// Greeting, LOGIN, EXAMINE. EXAMINE is SELECT in read-only mode: the server cannot change a flag in it.
fn open<S: Read + Write>(im: &mut Imap<S>, user: &str, pw: &str, mailbox: &str) -> Result<Selected, String> {
    let (u, p, m) = (quote(user)?, quote(pw)?, quote(mailbox)?);
    im.greeting()?;
    let r = im.cmd(&format!("LOGIN {u} {p}"))?;
    if r.status != "OK" {
        return Err(auth_text(&r.text));
    }
    let r = im.cmd(&format!("EXAMINE {m}"))?;
    if r.status != "OK" {
        return Err(format!("на сервере нет папки «{mailbox}»"));
    }
    Ok(Selected { validity: resp_code(&r.untagged, "UIDVALIDITY"), uidnext: resp_code(&r.untagged, "UIDNEXT") })
}

fn parse_search(untagged: &[Vec<u8>]) -> Vec<u32> {
    let mut v: Vec<u32> = untagged
        .iter()
        .filter(|l| l.starts_with(b"* SEARCH"))
        .flat_map(|l| String::from_utf8_lossy(&l[8..]).split_whitespace().filter_map(|t| t.parse::<u32>().ok()).collect::<Vec<_>>())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

fn search_unseen<S: Read + Write>(im: &mut Imap<S>) -> Result<Vec<u32>, String> {
    let r = im.cmd("UID SEARCH UNSEEN")?;
    if r.status != "OK" {
        return Err("сервер не выполнил поиск непрочитанных писем".into());
    }
    Ok(parse_search(&r.untagged))
}

/// `{n}\r\n` literal inside a FETCH response: (position of '{', start of the data, length).
fn find_literal(raw: &[u8]) -> Option<(usize, usize, usize)> {
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'{' {
            let mut j = i + 1;
            while j < raw.len() && raw[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < raw.len() && raw[j] == b'}' && raw[j + 1..].starts_with(b"\r\n") {
                let n: usize = std::str::from_utf8(&raw[i + 1..j]).ok()?.parse().ok()?;
                return Some((i, j + 3, n));
            }
        }
        i += 1;
    }
    None
}

/// `* 3 FETCH (UID 20 BODY[HEADER.FIELDS (FROM SUBJECT)] {n}\r\n<header>)` -> (uid, header bytes). The UID may stand before or after the literal.
fn parse_fetch(raw: &[u8]) -> Option<(u32, Vec<u8>)> {
    let (brace, start, n) = find_literal(raw)?;
    let lit = raw.get(start..start.checked_add(n)?)?.to_vec();
    let mut rest = raw[..brace].to_vec();
    rest.extend_from_slice(&raw[start + n..]);
    let s = String::from_utf8_lossy(&rest).to_ascii_uppercase();
    let at = s.find("UID ")? + 4;
    let uid = s[at..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((uid, lit))
}

/// Sender and subject of one message. PEEK: the message stays unread. Only these two header fields are requested.
fn fetch_headers<S: Read + Write>(im: &mut Imap<S>, uid: u32) -> Result<Option<(String, String)>, String> {
    let r = im.cmd(&format!("UID FETCH {uid} (BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])"))?;
    if r.status != "OK" {
        return Ok(None);
    }
    Ok(r.untagged.iter().filter_map(|l| parse_fetch(l)).find(|(u, _)| *u == uid).map(|(_, h)| shown_headers(&h)))
}

// ================= headers: RFC 2047 =================

const CP1251_HI: &str = "ЂЃ‚ѓ„…†‡€‰Љ‹ЊЌЋЏђ‘’“”•–—\u{FFFD}™љ›њќћџ\u{A0}ЎўЈ¤Ґ¦§Ё©Є«¬\u{AD}®Ї°±Ііґµ¶·ё№є»јЅѕї";
const KOI8_LOW: &str = "юабцдефгхийклмнопярстужвьызшэщчъ";
const KOI8_UP: &str = "ЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";

/// Bytes of a MIME charset -> text. ponytail: utf-8, ascii, latin-1/windows-1252, windows-1251, koi8-r/u only (what real Russian and Western mail uses);
/// anything else is read as UTF-8 with replacement characters. Add a table when a charset shows up in practice.
fn decode_charset(cs: &str, b: &[u8]) -> String {
    let cs = cs.to_ascii_lowercase();
    match cs.as_str() {
        "iso-8859-1" | "latin1" | "us-ascii" | "ascii" | "windows-1252" | "cp1252" | "iso-8859-15" => b.iter().map(|&c| c as char).collect(),
        "windows-1251" | "cp1251" => {
            let hi: Vec<char> = CP1251_HI.chars().collect();
            b.iter()
                .map(|&c| match c {
                    0..=0x7F => c as char,
                    0x80..=0xBF => hi[(c - 0x80) as usize],
                    _ => char::from_u32(0x410 + (c - 0xC0) as u32).unwrap_or('?'),
                })
                .collect()
        }
        "koi8-r" | "koi8-u" | "koi8" => b
            .iter()
            .map(|&c| match c {
                0..=0x7F => c as char,
                0xA3 => 'ё',
                0xB3 => 'Ё',
                0xC0..=0xDF => KOI8_LOW.chars().nth((c - 0xC0) as usize).unwrap_or('?'),
                0xE0..=0xFF => KOI8_UP.chars().nth((c - 0xE0) as usize).unwrap_or('?'),
                _ => '?',
            })
            .collect(),
        _ => String::from_utf8_lossy(b).into_owned(),
    }
}

/// `=?charset?B|Q?text?=` at the start of `t` -> (charset, bytes, bytes used from `t`).
fn parse_word(t: &str) -> Option<(String, Vec<u8>, usize)> {
    let body = t.strip_prefix("=?")?;
    let (cs, rest) = body.split_once('?')?;
    let cs = cs.split('*').next()?.to_string(); // RFC 2231 language suffix
    let (enc, rest) = rest.split_once('?')?;
    let end = rest.find("?=")?;
    let text = &rest[..end];
    if cs.is_empty() || enc.len() != 1 || text.bytes().any(|b| b.is_ascii_whitespace()) {
        return None;
    }
    let bytes = match enc.to_ascii_uppercase().as_str() {
        "B" => STANDARD_NO_PAD.decode(text.trim_end_matches('=')).ok()?,
        "Q" => {
            let b = text.as_bytes();
            let (mut out, mut i) = (Vec::new(), 0);
            while i < b.len() {
                match b[i] {
                    b'_' => out.push(b' '),
                    b'=' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                        out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).ok()?, 16).ok()?);
                        i += 2;
                    }
                    c => out.push(c),
                }
                i += 1;
            }
            out
        }
        _ => return None,
    };
    Some((cs, bytes, 2 + body.len() - rest.len() + end + 2))
}

/// Decodes every encoded word of a header value. White space between two encoded words is dropped, and neighbours of the same charset are joined
/// before decoding (a multi-byte character may be split across two words).
fn decode_words(s: &str) -> String {
    let mut out = String::new();
    let mut pending: Option<(String, Vec<u8>)> = None;
    let flush = |p: &mut Option<(String, Vec<u8>)>, out: &mut String| {
        if let Some((cs, b)) = p.take() {
            out.push_str(&decode_charset(&cs, &b));
        }
    };
    let mut rest = s;
    while let Some(i) = rest.find("=?") {
        let (before, tail) = rest.split_at(i);
        match parse_word(tail) {
            Some((cs, bytes, used)) => {
                let ws = before.chars().all(char::is_whitespace);
                if ws && pending.as_ref().is_some_and(|(c, _)| c.eq_ignore_ascii_case(&cs)) {
                    pending.as_mut().unwrap().1.extend(bytes);
                } else {
                    let had = pending.is_some();
                    flush(&mut pending, &mut out);
                    if !(had && ws) {
                        out.push_str(before);
                    }
                    pending = Some((cs, bytes));
                }
                rest = &tail[used..];
            }
            None => {
                flush(&mut pending, &mut out);
                out.push_str(before);
                out.push_str("=?");
                rest = &tail[2..];
            }
        }
    }
    flush(&mut pending, &mut out);
    out.push_str(rest);
    out
}

/// Control characters -> space, runs of white space collapsed, at most `max` characters (the page inserts it as text only).
fn clean(s: &str, max: usize) -> String {
    let t: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > max {
        t.chars().take(max - 1).collect::<String>() + "…"
    } else {
        t
    }
}

/// "Name <a@b>" -> Name; "<a@b>" or "a@b" -> a@b. The structure is read from the raw value, the name is decoded afterwards.
fn sender(raw: &str) -> String {
    let raw = raw.trim();
    if let (Some(lt), Some(gt)) = (raw.rfind('<'), raw.rfind('>')) {
        if lt < gt {
            let name = decode_words(raw[..lt].trim()).trim().trim_matches('"').trim().to_string();
            return if name.is_empty() { raw[lt + 1..gt].trim().to_string() } else { name };
        }
    }
    decode_words(raw).trim().trim_matches('"').to_string()
}

/// (sender, subject) ready to show, from the raw header block.
fn shown_headers(raw: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(raw);
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        match l.strip_prefix([' ', '\t']) {
            Some(cont) if !lines.is_empty() => {
                let last = lines.last_mut().unwrap();
                last.push(' ');
                last.push_str(cont.trim_start());
            }
            _ => lines.push(l.to_string()),
        }
    }
    let (mut from, mut subject) = (String::new(), String::new());
    for l in &lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "from" if from.is_empty() => from = clean(&sender(v), 60),
            "subject" if subject.is_empty() => subject = clean(&decode_words(v.trim()), 120),
            _ => {}
        }
    }
    (from, subject)
}

// ================= which messages are new =================

#[derive(Debug, Default, Clone, PartialEq)]
struct MailState {
    acct: String,
    validity: u32,
    uid: u32, // highest UID already accounted for
}

/// Hash of host + user + mailbox: the state of one mailbox is never applied to another (no address in the file).
fn acct_id(host: &str, user: &str, mailbox: &str) -> String {
    let h = Sha256::digest(format!("{}\n{}\n{}", host.to_lowercase(), user.to_lowercase(), mailbox).as_bytes());
    h[..12].iter().map(|b| format!("{b:02x}")).collect()
}

/// UIDs to announce, ascending. The first look at a mailbox (or a changed UIDVALIDITY, i.e. the UIDs were renumbered) is only a baseline: whatever
/// is unread now is not "new". After that: unread messages above the highest UID seen. The highest UID also follows UIDNEXT-1, so a message that
/// was read elsewhere before the next poll does not hold the mark back.
fn fresh(st: &mut MailState, acct: &str, validity: Option<u32>, uidnext: Option<u32>, unseen: &[u32]) -> Vec<u32> {
    let top = unseen.iter().copied().max().unwrap_or(0).max(uidnext.map_or(0, |n| n.saturating_sub(1)));
    let validity = validity.unwrap_or(0);
    if st.acct != acct || st.validity != validity {
        *st = MailState { acct: acct.to_string(), validity, uid: top };
        return Vec::new();
    }
    let mut v: Vec<u32> = unseen.iter().copied().filter(|&u| u > st.uid).collect();
    v.sort_unstable();
    v.dedup();
    st.uid = st.uid.max(top);
    v
}

fn state_path() -> std::path::PathBuf {
    crate::store::dir().join("mail-state.json")
}

fn load_state() -> MailState {
    let Ok(text) = std::fs::read_to_string(state_path()) else { return MailState::default() };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return MailState::default() };
    MailState {
        acct: v["acct"].as_str().unwrap_or("").to_string(),
        validity: v["validity"].as_u64().unwrap_or(0) as u32,
        uid: v["uid"].as_u64().unwrap_or(0) as u32,
    }
}

fn save_state(s: &MailState) {
    let p = state_path();
    let tmp = p.with_extension("json.tmp");
    let body = json!({ "acct": s.acct, "validity": s.validity, "uid": s.uid }).to_string();
    if std::fs::write(&tmp, body).is_err() || std::fs::rename(&tmp, &p).is_err() {
        eprintln!("mail: could not write the state file");
    }
}

// ================= scheduling =================

#[derive(Default)]
struct Sched {
    sig: Option<u64>,
    next: u64,
    last: Option<u64>,
    fails: u32,
}

impl Sched {
    /// True when a poll should start now: first time, settings changed, or the interval is over. Never closer than MIN_GAP_MS to the previous poll.
    fn due(&mut self, now: u64, on: bool, sig: u64) -> bool {
        if !on {
            self.sig = None;
            self.fails = 0;
            return false;
        }
        if self.sig != Some(sig) {
            self.sig = Some(sig);
            self.fails = 0;
            self.next = self.last.map_or(now, |l| (l + MIN_GAP_MS).max(now));
        }
        if now >= self.next {
            self.last = Some(now);
            true
        } else {
            false
        }
    }

    fn done(&mut self, now: u64, ok: bool) {
        if ok {
            self.fails = 0;
            self.next = now + EVERY_MS;
        } else {
            self.fails += 1;
            self.next = now + (EVERY_MS << self.fails.min(5)).min(MAX_BACKOFF_MS);
        }
    }
}

// ================= settings, TLS, polling =================

struct Cfg {
    host: String,
    port: u16,
    user: String,
    mailbox: String,
    password: String,
}

/// Err((true, text)): not set up yet (not an error); Err((false, text)): settings that cannot work.
fn load_cfg() -> Result<Cfg, (bool, String)> {
    let get = |k: &str| crate::settings::get_str(k, "").trim().to_string();
    let (host, user) = (get("mail.host").to_lowercase(), get("mail.user"));
    if host.is_empty() || user.is_empty() {
        return Err((true, "Укажите сервер и логин".into()));
    }
    if !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
        return Err((false, "Адрес сервера — только имя, например imap.gmail.com (без imaps://, порта и пробелов)".into()));
    }
    let port = crate::settings::get_f64("mail.port", 993.0);
    if port == 143.0 {
        return Err((false, "Порт 143 — незашифрованный IMAP, Pill его не использует. Нужен порт 993 (TLS)".into()));
    }
    if !(1.0..=65535.0).contains(&port) {
        return Err((false, "Порт должен быть от 1 до 65535 (обычно 993)".into()));
    }
    let enc = get("mail.passwordEnc");
    if enc.is_empty() {
        return Err((true, "Введите пароль приложения и нажмите «Сохранить пароль»".into()));
    }
    let password = crate::net::secrets::unprotect(&enc).map_err(|_| (false, "Не удалось расшифровать сохранённый пароль (он сохранён под другим пользователем Windows): введите его заново".to_string()))?;
    let mailbox = match get("mail.mailbox") {
        m if m.is_empty() => "INBOX".to_string(),
        m => m,
    };
    Ok(Cfg { host, port: port as u16, user, mailbox, password })
}

/// What changes the account: a change makes the scheduler poll at once.
fn signature() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for k in ["mail.host", "mail.port", "mail.user", "mail.mailbox", "mail.passwordEnc"] {
        crate::settings::get_str(k, "").trim().hash(&mut h);
    }
    h.finish()
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    static CFG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CFG.get_or_init(|| {
        let b = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| "TLS недоступен".to_string())?
            .with_platform_verifier()
            .map_err(|_| "TLS недоступен: не открылось хранилище сертификатов Windows".to_string())?;
        Ok(Arc::new(b.with_no_client_auth()))
    })
    .clone()
}

fn tls_text(e: &std::io::Error) -> String {
    use rustls::{CertificateError as C, Error as E};
    match e.get_ref().and_then(|i| i.downcast_ref::<E>()) {
        Some(E::InvalidCertificate(C::Expired)) => "TLS: срок действия сертификата сервера истёк".into(),
        Some(E::InvalidCertificate(C::NotValidForName)) => "TLS: сертификат выдан на другое имя сервера. Проверьте адрес сервера".into(),
        Some(E::InvalidCertificate(C::UnknownIssuer)) => "TLS: сертификат сервера не от доверенного центра (Windows ему не доверяет)".into(),
        Some(E::InvalidCertificate(_)) => "TLS: сертификат сервера не принят".into(),
        Some(_) => "TLS: ошибка защищённого соединения".into(),
        None => match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "сервер не отвечает (таймаут)".into(),
            _ => "TLS: сервер не поддерживает шифрованное соединение на этом порту (нужен порт 993)".into(),
        },
    }
}

type Tls = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

/// TCP + TLS handshake (implicit TLS: the very first byte is encrypted). Nothing readable is ever sent before the certificate is accepted.
fn connect(host: &str, port: u16) -> Result<Tls, String> {
    POLLS.fetch_add(1, Ordering::Relaxed);
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|_| format!("не удалось найти сервер {host}: проверьте адрес и интернет"))?.collect();
    let tcp = addrs.iter().find_map(|a| TcpStream::connect_timeout(a, CONNECT_TIMEOUT).ok()).ok_or_else(|| {
        format!("не удалось подключиться к {host}:{port}: сервер недоступен, порт закрыт или его блокирует брандмауэр")
    })?;
    let _ = tcp.set_read_timeout(Some(IO_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(IO_TIMEOUT));
    let _ = tcp.set_nodelay(true);
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| "некорректный адрес сервера".to_string())?;
    let conn = rustls::ClientConnection::new(tls_config()?, name).map_err(|_| "TLS: не удалось начать соединение".to_string())?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock).map_err(|e| tls_text(&e))?;
    }
    Ok(tls)
}

fn with_session<T>(cfg: &Cfg, f: impl FnOnce(&mut Imap<Tls>) -> Result<T, String>) -> Result<T, String> {
    let deadline = Instant::now() + Duration::from_millis(SESSION_MS);
    let mut im = Imap::new(connect(&cfg.host, cfg.port)?, deadline);
    let r = f(&mut im);
    let _ = im.cmd("LOGOUT");
    r
}

#[derive(Debug)]
struct Found {
    unseen: usize,
    fresh: usize,
    newest: Option<(String, String)>,
}

/// One poll on an open stream: login, new-UID decision, headers of the newest new message only.
fn poll_session<S: Read + Write>(im: &mut Imap<S>, cfg: &Cfg, st: &mut MailState) -> Result<Found, String> {
    let sel = open(im, &cfg.user, &cfg.password, &cfg.mailbox)?;
    let unseen = search_unseen(im)?;
    let new = fresh(st, &acct_id(&cfg.host, &cfg.user, &cfg.mailbox), sel.validity, sel.uidnext, &unseen);
    // the card still comes when only the headers cannot be read
    let newest = match new.last() {
        Some(&u) => Some(fetch_headers(im, u).ok().flatten().unwrap_or_default()),
        None => None,
    };
    Ok(Found { unseen: unseen.len(), fresh: new.len(), newest })
}

// ================= supervisor =================

#[derive(Default)]
struct Status {
    state: &'static str, // off | nosetup | ok | error
    message: String,
    unseen: Option<usize>,
    at: u64, // unix seconds of the last poll
}

static STATUS: Mutex<Status> = Mutex::new(Status { state: "off", message: String::new(), unseen: None, at: 0 });

fn set_status(state: &'static str, message: String, unseen: Option<usize>) {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Status { state, message, unseen, at };
}

fn tick(app: &AppHandle, sched: &mut Sched, st: &mut MailState) {
    let on = crate::settings::get_bool("live.mail", false);
    let now = crate::liveplus::mono_ms();
    if !sched.due(now, on, signature()) {
        if !on {
            let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
            if s.state != "off" {
                *s = Status { state: "off", ..Status::default() };
            }
        }
        return;
    }
    let cfg = match load_cfg() {
        Ok(c) => c,
        Err((setup, text)) => {
            set_status(if setup { "nosetup" } else { "error" }, text, None);
            return sched.done(now, true); // nothing to retry: the next settings change polls at once
        }
    };
    let before = st.clone();
    match with_session(&cfg, |im| poll_session(im, &cfg, st)) {
        Ok(f) => {
            if *st != before {
                save_state(st);
            }
            set_status("ok", String::new(), Some(f.unseen));
            if f.fresh > 0 {
                let (from, subject) = f.newest.unwrap_or_default();
                let _ = bus::emit(&app, Target::Label("pill"), "live:mail", json!({ "from": from, "subject": subject, "more": f.fresh - 1 }));
            }
            sched.done(crate::liveplus::mono_ms(), true);
        }
        Err(e) => {
            eprintln!("mail: poll of {} failed", cfg.host); // the reason goes to the settings page, never the account
            set_status("error", e, None);
            sched.done(crate::liveplus::mono_ms(), false);
        }
    }
}

fn run(app: AppHandle) {
    let (mut sched, mut st) = (Sched::default(), load_state());
    loop {
        std::thread::sleep(TICK);
        // a panic in one tick (it should not happen) must not end the producer
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(&app, &mut sched, &mut st)));
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    std::thread::spawn(move || run(h));
}

/// For net_features_status: the one host mail may call while the switch is on.
pub fn net_feature() -> Value {
    let on = crate::settings::get_bool("live.mail", false);
    let host = crate::settings::get_str("mail.host", "").trim().to_lowercase();
    let hosts: Vec<String> = if host.is_empty() { Vec::new() } else { vec![host] };
    json!({ "id": "mail", "on": on, "hosts": hosts, "calls": POLLS.load(Ordering::Relaxed) })
}

// ================= commands =================

/// For the settings page: {on, state: off | nosetup | ok | error, message, unseen, at (unix s), hasPassword}. The password never comes back.
#[tauri::command]
pub fn mail_status() -> Value {
    let on = crate::settings::get_bool("live.mail", false);
    let s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    json!({
        "on": on, "state": if on { s.state } else { "off" }, "message": s.message, "unseen": s.unseen, "at": s.at,
        "hasPassword": !crate::settings::get_str("mail.passwordEnc", "").trim().is_empty(),
    })
}

/// Stores the mailbox password DPAPI-encrypted (empty string clears it). The text never comes back to the page.
#[tauri::command]
pub fn mail_set_password(app: AppHandle, password: String) -> bool {
    let value = if password.is_empty() { Some(String::new()) } else { crate::net::secrets::protect(&password).ok() };
    let ok = value.is_some();
    let mut patch = serde_json::Map::new();
    patch.insert("mail.passwordEnc".into(), json!(value.unwrap_or_default()));
    if ok {
        let _ = crate::settings::apply_patch(&app, patch);
    }
    ok
}

/// "Проверить подключение": connects with the saved settings, logs in, opens the folder and counts unread messages. Reads nothing else, changes nothing.
/// {ok, message}: the message is readable and never contains the login, the password or anything the server said.
#[tauri::command(async)]
pub fn mail_test() -> Value {
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, Ordering::SeqCst) {
        return json!({ "ok": false, "message": "Проверка уже идёт, подождите" });
    }
    let res = std::panic::catch_unwind(|| -> Result<usize, String> {
        let cfg = load_cfg().map_err(|(_, t)| t)?;
        with_session(&cfg, |im| {
            open(im, &cfg.user, &cfg.password, &cfg.mailbox)?;
            Ok(search_unseen(im)?.len())
        })
    })
    .unwrap_or_else(|_| Err("внутренняя ошибка".into()));
    BUSY.store(false, Ordering::SeqCst);
    match res {
        Ok(n) => json!({ "ok": true, "message": format!("Подключение работает: вход выполнен, непрочитанных писем в папке: {n}") }),
        Err(e) => json!({ "ok": false, "message": e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // ---- RFC 2047
    #[test]
    fn rfc2047_base64_q_and_charsets() {
        assert_eq!(decode_words("=?UTF-8?B?0J/RgNC40LLQtdGC?="), "Привет");
        assert_eq!(decode_words("=?koi8-r?Q?=F0=D2=C9=D7=C5=D4?="), "Привет");
        assert_eq!(decode_words("=?windows-1251?Q?=CF=F0=E8=E2=E5=F2?="), "Привет");
        assert_eq!(decode_words("=?utf-8?q?a_b=C3=A9?="), "a bé");
        assert_eq!(decode_words("Re: =?iso-8859-1?Q?caf=E9?= done"), "Re: café done");
        assert_eq!(decode_words("=?UTF-8?B?0J/RgNC40LLQtdGC?= =?UTF-8?B?IG1pcg==?="), "Привет mir"); // blank between encoded words is dropped
        assert_eq!(decode_words("=?utf-8?Q?=D0=9F=D1?= =?utf-8?Q?=80=D0=B8?="), "При"); // one letter split across two words
        assert_eq!(decode_words("=?utf-8*ru?B?0J/RgNC40LLQtdGC?="), "Привет"); // language suffix
        assert_eq!(decode_words("plain =?bad and =?utf-8?X?zz?= stay"), "plain =?bad and =?utf-8?X?zz?= stay");
        assert_eq!(decode_words("=?UTF-8?B?0J/RgNC40LLQtdGC?"), "=?UTF-8?B?0J/RgNC40LLQtdGC?"); // not terminated: left as it is
        assert_eq!(CP1251_HI.chars().count(), 64);
        assert_eq!(KOI8_LOW.chars().count(), 32);
        assert_eq!(KOI8_UP.chars().count(), 32);
    }

    #[test]
    fn sender_and_subject_are_shown_clean() {
        let raw = b"From: =?UTF-8?B?0JjQstCw0L0g0J/QtdGC0YDQvtCy?= <ivan@example.com>\r\nSubject: =?utf-8?Q?=D0=A1=D1=87=D1=91=D1=82?=\r\n  =?utf-8?Q?_=E2=84=96_5?=\r\n\r\n";
        assert_eq!(shown_headers(raw), ("Иван Петров".to_string(), "Счёт № 5".to_string()));
        assert_eq!(shown_headers(b"From: \"Doe, Jane\" <jane@x.org>\r\nSubject: hi\r\n\r\n").0, "Doe, Jane");
        assert_eq!(shown_headers(b"From: <bob@x.org>\r\n\r\n"), ("bob@x.org".to_string(), String::new()));
        assert_eq!(shown_headers(b"from: bob@x.org\r\nsubject: low\x01er\r\n\r\n"), ("bob@x.org".to_string(), "low er".to_string()));
        let long = format!("Subject: {}\r\n", "я".repeat(300));
        assert_eq!(shown_headers(long.as_bytes()).1.chars().count(), 120);
    }

    // ---- response parsing
    #[test]
    fn fetch_response_with_literal_in_either_order() {
        let hdr = "From: a@b.c\r\nSubject: s\r\n\r\n";
        let a = format!("* 3 FETCH (UID 20 BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{hdr})\r\n", hdr.len());
        assert_eq!(parse_fetch(a.as_bytes()), Some((20, hdr.as_bytes().to_vec())));
        let b = format!("* 3 FETCH (BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{hdr} UID 21)\r\n", hdr.len());
        assert_eq!(parse_fetch(b.as_bytes()).map(|x| x.0), Some(21));
        // a header that itself contains "{5}\r\n" or "UID 99" does not confuse it
        let tricky = "Subject: {5}\r\nUID 99\r\n\r\n";
        let c = format!("* 1 FETCH (UID 7 BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{tricky})\r\n", tricky.len());
        assert_eq!(parse_fetch(c.as_bytes()).map(|x| x.0), Some(7));
        assert_eq!(parse_fetch(b"* 1 FETCH (UID 7 BODY[HEADER.FIELDS (FROM SUBJECT)] NIL)\r\n"), None);
    }

    #[test]
    fn search_and_codes() {
        let u = vec![b"* SEARCH 18 20 3\r\n".to_vec(), b"* SEARCH\r\n".to_vec(), b"* 5 EXISTS\r\n".to_vec()];
        assert_eq!(parse_search(&u), vec![3, 18, 20]);
        let sel = vec![b"* OK [UIDVALIDITY 3857529045] UIDs valid\r\n".to_vec(), b"* OK [UIDNEXT 4392] Predicted next UID\r\n".to_vec()];
        assert_eq!(resp_code(&sel, "UIDVALIDITY"), Some(3857529045));
        assert_eq!(resp_code(&sel, "UIDNEXT"), Some(4392));
        assert_eq!(resp_code(&sel, "HIGHESTMODSEQ"), None);
        assert_eq!(literal_len(b"* 1 FETCH (BODY[] {342}\r\n"), Some(342));
        assert_eq!(literal_len(b"* OK no literal\r\n"), None);
        assert_eq!(literal_len(b"* OK {abc}\r\n"), None);
    }

    #[test]
    fn quoting_refuses_line_breaks() {
        assert_eq!(quote("a\"b\\c").unwrap(), "\"a\\\"b\\\\c\"");
        assert!(quote("x\r\nA2 DELETE").is_err());
        assert!(quote("x\0").is_err());
    }

    // ---- which messages are new
    #[test]
    fn dedup_baseline_then_only_higher_uids() {
        let mut st = MailState::default();
        let a = acct_id("imap.x.com", "me@x.com", "INBOX");
        // first look: 3 old unread messages are not news
        assert!(fresh(&mut st, &a, Some(1), Some(31), &[10, 12, 30]).is_empty());
        assert_eq!(st, MailState { acct: a.clone(), validity: 1, uid: 30 });
        assert!(fresh(&mut st, &a, Some(1), Some(31), &[10, 12, 30]).is_empty()); // nothing changed
        assert_eq!(fresh(&mut st, &a, Some(1), Some(33), &[10, 12, 30, 31, 32]), vec![31, 32]);
        assert!(fresh(&mut st, &a, Some(1), Some(33), &[10, 31, 32]).is_empty()); // once per UID
        assert!(fresh(&mut st, &a, Some(1), Some(33), &[]).is_empty()); // all read elsewhere
        assert_eq!(fresh(&mut st, &a, Some(1), Some(34), &[33]), vec![33]);
        // read elsewhere before the poll: the mark still moves past it (UIDNEXT), the next one is news
        assert!(fresh(&mut st, &a, Some(1), Some(40), &[]).is_empty());
        assert_eq!(fresh(&mut st, &a, Some(1), Some(41), &[40]), vec![40]);
        // a server that sends no UIDNEXT
        let mut s2 = MailState::default();
        assert!(fresh(&mut s2, &a, None, None, &[5]).is_empty());
        assert_eq!(fresh(&mut s2, &a, None, None, &[5, 6]), vec![6]);
    }

    #[test]
    fn dedup_resets_on_new_uidvalidity_or_other_account() {
        let (a, b) = (acct_id("imap.x.com", "me@x.com", "INBOX"), acct_id("imap.x.com", "other@x.com", "INBOX"));
        assert_ne!(a, b);
        assert_eq!(a, acct_id("IMAP.X.COM", "ME@x.com", "INBOX"));
        assert!(!a.contains("x.com"));
        let mut st = MailState { acct: a.clone(), validity: 1, uid: 100 };
        assert!(fresh(&mut st, &a, Some(2), Some(5), &[1, 2, 3]).is_empty()); // renumbered: baseline, not 3 cards
        assert_eq!(st.uid, 4);
        assert!(fresh(&mut st, &b, Some(2), Some(5), &[4]).is_empty()); // another account: baseline again
        assert_eq!(st.acct, b);
    }

    #[test]
    fn scheduler_interval_change_gap_and_backoff() {
        let mut s = Sched::default();
        assert!(!s.due(0, false, 1)); // off: nothing
        assert!(s.due(1000, true, 1)); // first poll at once
        s.done(1500, true);
        assert!(!s.due(2000, true, 1));
        assert!(!s.due(1500 + EVERY_MS - 1, true, 1));
        assert!(s.due(1500 + EVERY_MS, true, 1)); // every 2 minutes
        s.done(1500 + EVERY_MS, true);
        // settings changed right after a poll: it waits for the minimum gap, not the 2 minutes
        let t = 1500 + EVERY_MS;
        assert!(!s.due(t + 1000, true, 2));
        assert!(s.due(t + MIN_GAP_MS, true, 2));
        s.done(t + MIN_GAP_MS, false); // error: back-off 2x, 4x, ... 30 min
        assert!(!s.due(t + MIN_GAP_MS + EVERY_MS * 2 - 1, true, 2));
        assert!(s.due(t + MIN_GAP_MS + EVERY_MS * 2, true, 2));
        for _ in 0..10 {
            s.done(0, false);
        }
        assert_eq!(s.next, MAX_BACKOFF_MS);
        // switched off and on again: the gap to the last poll still applies
        let last = s.last.unwrap();
        assert!(!s.due(last + 1, false, 2));
        assert!(!s.due(last + 5, true, 2));
        assert!(s.due(last + MIN_GAP_MS, true, 2));
    }

    // ---- the session on an in-memory stream
    struct Mock {
        rx: Cursor<Vec<u8>>,
        tx: Vec<u8>,
    }
    impl Read for Mock {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.rx.read(b)
        }
    }
    impl Write for Mock {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.tx.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn mock(server: &str) -> Imap<Mock> {
        Imap::new(Mock { rx: Cursor::new(server.as_bytes().to_vec()), tx: Vec::new() }, Instant::now() + Duration::from_secs(5))
    }

    fn cfg() -> Cfg {
        Cfg { host: "imap.example.com".into(), port: 993, user: "me@example.com".into(), mailbox: "INBOX".into(), password: "p\"w\\1".into() }
    }

    fn server(search: &str, fetch_uid: u32) -> String {
        let hdr = "From: =?UTF-8?B?0JjQstCw0L0=?= <ivan@example.com>\r\nSubject: =?UTF-8?B?0J/RgNC40LLQtdGC?=\r\n\r\n";
        format!(
            "* OK IMAP4rev1 ready\r\na1 OK LOGIN completed\r\n* 40 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n* OK [UIDNEXT 21] ok\r\na2 OK [READ-ONLY] EXAMINE done\r\n\
             * SEARCH {search}\r\na3 OK SEARCH done\r\n\
             * 9 FETCH (UID {fetch_uid} BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{hdr})\r\na4 OK FETCH done\r\n* BYE bye\r\na5 OK LOGOUT\r\n",
            hdr.len()
        )
    }

    #[test]
    fn session_announces_the_newest_new_message_and_only_peeks() {
        let a = acct_id("imap.example.com", "me@example.com", "INBOX");
        let mut st = MailState { acct: a, validity: 7, uid: 15 };
        let mut im = mock(&server("12 18 20", 20));
        let f = poll_session(&mut im, &cfg(), &mut st).unwrap();
        assert_eq!((f.unseen, f.fresh), (3, 2));
        assert_eq!(f.newest, Some(("Иван".to_string(), "Привет".to_string())));
        assert_eq!(st.uid, 20);
        let sent = String::from_utf8(im.io.get_ref().tx.clone()).unwrap();
        assert!(sent.contains("a1 LOGIN \"me@example.com\" \"p\\\"w\\\\1\"\r\n"), "{sent}");
        assert!(sent.contains("a2 EXAMINE \"INBOX\"\r\n"));
        assert!(sent.contains("a3 UID SEARCH UNSEEN\r\n"));
        assert!(sent.contains("a4 UID FETCH 20 (BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])\r\n"));
        // nothing that could change or download a message
        for bad in ["STORE", "SELECT", "COPY", "EXPUNGE", "RFC822", "BODY[", "BODYSTRUCTURE", "APPEND", "DELETE"] {
            assert!(!sent.contains(bad), "{bad} in {sent}");
        }
        // the password is only in the LOGIN line
        assert_eq!(sent.matches("p\\\"w").count(), 1);
    }

    #[test]
    fn session_without_new_mail_fetches_nothing() {
        let a = acct_id("imap.example.com", "me@example.com", "INBOX");
        let mut st = MailState { acct: a, validity: 7, uid: 20 };
        let mut im = mock("* OK hi\r\na1 OK ok\r\n* OK [UIDVALIDITY 7] x\r\n* OK [UIDNEXT 21] x\r\na2 OK ok\r\n* SEARCH 12 20\r\na3 OK ok\r\n");
        let f = poll_session(&mut im, &cfg(), &mut st).unwrap();
        assert_eq!((f.fresh, f.newest), (0, None));
        assert!(!String::from_utf8_lossy(&im.io.get_ref().tx).contains("FETCH"));
    }

    #[test]
    fn session_errors_are_readable_and_do_not_echo_the_server() {
        let mut st = MailState::default();
        let e = poll_session(&mut mock("* OK hi\r\na1 NO [AUTHENTICATIONFAILED] Invalid credentials for me@example.com\r\n"), &cfg(), &mut st).unwrap_err();
        assert!(e.contains("Вход отклонён") && !e.contains("me@example.com") && !e.contains("p\"w"), "{e}");
        let e = poll_session(&mut mock("* OK hi\r\na1 NO [ALERT] Application-specific password required: https://support.google.com/x\r\n"), &cfg(), &mut st).unwrap_err();
        assert!(e.contains("пароль приложения") && !e.contains("https"), "{e}");
        let e = poll_session(&mut mock("* OK hi\r\na1 OK ok\r\na2 NO [NONEXISTENT] no such mailbox\r\n"), &cfg(), &mut st).unwrap_err();
        assert!(e.contains("нет папки"), "{e}");
        let e = poll_session(&mut mock("HTTP/1.1 400 Bad Request\r\n"), &cfg(), &mut st).unwrap_err();
        assert!(e.contains("не IMAP"), "{e}");
        let e = poll_session(&mut mock("* OK hi\r\n"), &cfg(), &mut st).unwrap_err(); // the server hangs up
        assert!(e.contains("оборвалось"), "{e}");
        let e = poll_session(&mut mock("* OK hi\r\n* BYE too busy\r\n"), &cfg(), &mut st).unwrap_err();
        assert!(e.contains("закрыл"), "{e}");
        // a huge literal is refused instead of allocated
        let e = poll_session(&mut mock(&format!("* OK hi\r\na1 OK ok\r\n* 1 FETCH {{{}}}\r\n", MAX_LITERAL + 1)), &cfg(), &mut st).unwrap_err();
        assert!(e.contains("слишком большой"), "{e}");
    }

    // ---- DPAPI (needs a Windows user profile, which every Windows test run has)
    #[test]
    fn password_dpapi_round_trip_is_not_plain_text() {
        let pw = "app pässword \"1\"";
        let blob = crate::net::secrets::protect(pw).unwrap();
        assert!(!blob.contains("pässword") && !blob.contains("app"));
        assert_eq!(crate::net::secrets::unprotect(&blob).unwrap(), pw);
        assert!(crate::net::secrets::unprotect("not-a-blob").is_err());
    }
}
