// Local live activities, the system side (docs/live-activities.md). Any program of this user may put ONE ongoing, time-bound thing on the
// island by writing one-line JSON to the named pipe `\\.\pipe\pill-live-v1`; `pill.exe live start|update|end ...` is the same thing for
// scripts. The rules (schema, limits, three shown + a short queue, dismiss lock, time-out) are pure and live in
// pill_ui_core::live::activity; this file only owns the I/O:
//   * the pipe: current-user ACL (+ SYSTEM) built from the process token, remote clients rejected, no network. Opt-in `live.local`
//     (default off): while it is off no pipe exists and no thread runs. Every thread blocks (accept / read / write / condvar), nothing polls.
//   * the clock thread: sleeps until the next time-out (wake::nap on a condvar) and ends what timed out.
//   * the reply channel: the connection that posted an activity gets `{"event":"action",..}` when the user presses one of its buttons,
//     `{"event":"dismissed",..}` when the user closed it and `{"event":"ended","reason":"ttl|disabled"}` when it ended by itself.
//     A client that wants clicks keeps its connection open; a closed connection simply gets nothing (the click is dropped).
//   * the UI gets the whole state as the event `liveact:state` after every change and calls back `liveact_dismiss` / `liveact_action`.
// PRIVACY: nothing a client sends is logged or stored; it lives in memory until the activity ends. Text is shown as plain text only.
use crate::ui::bus;
use pill_ui_core::live::activity::{self as act, Op, RateLimit, Reason, Registry};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Listener};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, INVALID_HANDLE_VALUE};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW};
use windows::Win32::Security::{GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Console::{AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};

pub const PIPE: &str = r"\\.\pipe\pill-live-v1";
const MAX_CONNS: usize = 8;

#[derive(Default)]
struct State {
    reg: Registry,
    /// activity id -> the connection that posted it last (where its events go)
    owner: HashMap<String, u64>,
    conns: HashMap<u64, Conn>,
    next_conn: u64,
    on: bool,
}

struct Conn {
    tx: SyncSender<String>,
    handle: isize,
}

fn st() -> std::sync::MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner())
}

/// Listener generation: a new one ends the old listener (the switch went off).
static GEN: AtomicU64 = AtomicU64::new(0);
/// Bumped after every registry change: the clock thread re-reads the next time-out.
static TICK: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Tells the island the whole state (the snapshot hands each new activity's card out once), and wakes the clock.
fn publish(s: &mut State) {
    let _ = bus::emit("liveact:state", s.reg.snapshot()); // = act::STATE_EVENT (asserted in the tests)
    crate::wake::bump(&TICK);
}

/// One event line to the connection that owns `id` (dropped when it is gone or does not read).
fn tell(s: &State, id: &str, ev: Value) {
    if let Some(c) = s.owner.get(id).and_then(|c| s.conns.get(c)) {
        let _ = c.tx.try_send(ev.to_string());
    }
}

// ---------------------------------------------------------------- the messages

/// One line from a client -> the one-line reply.
fn handle(conn: u64, line: &str, now: i64) -> String {
    let op = match act::parse_line(line) {
        Ok(o) => o,
        Err(e) => return e.reply(),
    };
    let id = op.id().to_string();
    let ending = matches!(op, Op::End { .. });
    let mut s = st();
    if !s.on {
        return json!({ "ok": false, "error": "disabled", "detail": "Live activities are off (Settings > Live activities)" }).to_string();
    }
    match s.reg.apply(op, now) {
        Ok(done) => {
            if ending {
                s.owner.remove(&id);
            } else {
                s.owner.insert(id, conn);
            }
            publish(&mut s);
            done.reply()
        }
        Err(e) => e.reply(),
    }
}

fn ended(s: &mut State, ids: Vec<String>, reason: Reason) {
    for id in ids {
        tell(s, &id, json!({ "event": "ended", "id": id, "reason": reason.as_str() }));
        s.owner.remove(&id);
    }
}

/// The user pressed the dismiss button of an activity (UI -> backend).
pub fn liveact_dismiss(id: String) -> bool {
    let mut s = st();
    if !s.reg.dismiss(&id) {
        return false;
    }
    tell(&s, &id, json!({ "event": "dismissed", "id": id }));
    publish(&mut s);
    true
}

/// The user pressed a button of an activity (UI -> backend): the id goes back on the reply channel of the connection that posted it.
/// Only a button the activity declared is passed on. False when nobody listens any more.
pub fn liveact_action(id: String, action: String) -> bool {
    let s = st();
    if !s.on || !s.reg.has_action(&id, &action) {
        return false;
    }
    let Some(c) = s.owner.get(&id).and_then(|c| s.conns.get(c)) else { return false };
    c.tx.try_send(json!({ "event": "action", "id": id, "action": action }).to_string()).is_ok()
}

/// Settings > Live activities > "Test activity": posts the sample straight into the registry (no pipe involved). Answers `{ ok, message }`.
pub fn liveact_test() -> Value {
    let ru = crate::ui::capture_view::windows::is_russian();
    let mut s = st();
    if !s.on {
        return json!({ "ok": false, "message": if ru { "Сначала включите переключатель выше" } else { "Turn the switch above on first" } });
    }
    match s.reg.apply(act::sample_start(ru), now_ms()) {
        Ok(_) => {
            publish(&mut s);
            json!({ "ok": true, "message": if ru { "Пример показан на острове" } else { "The sample is on the island" } })
        }
        Err(e) => json!({ "ok": false, "message": e.detail }),
    }
}

// ---------------------------------------------------------------- the clock

fn clock(g: u64) {
    while GEN.load(Ordering::SeqCst) == g {
        let t = TICK.load(Ordering::SeqCst);
        let wait = {
            let s = st();
            s.reg.next_deadline().map(|d| (d - now_ms()).max(0) as u64 + 5).unwrap_or(3_600_000)
        };
        // a full nap = a time-out is due; a bump = the registry changed, look again
        if crate::wake::nap(&TICK, t, wait) {
            let mut s = st();
            let gone = s.reg.expire(now_ms());
            if !gone.is_empty() {
                ended(&mut s, gone, Reason::Ttl);
                publish(&mut s);
            }
        }
    }
}

// ---------------------------------------------------------------- the pipe

/// A security descriptor that lets only this user (and SYSTEM) open the pipe. Built from the token, not from "owner rights", so it also
/// holds when Pill runs elevated. `None` = could not build it: no pipe (fail closed).
fn security() -> Option<SECURITY_ATTRIBUTES> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).ok()?;
        let mut n = 0u32;
        let _ = GetTokenInformation(tok, TokenUser, None, 0, &mut n);
        let mut buf = vec![0u8; n as usize];
        let r = GetTokenInformation(tok, TokenUser, Some(buf.as_mut_ptr() as *mut _), n, &mut n);
        let _ = CloseHandle(tok);
        r.ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
        let sid = s.to_string().ok();
        let _ = LocalFree(HLOCAL(s.0 as *mut _));
        let sddl = format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", sid?);
        let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(wide.as_ptr()), 1, &mut sd, None).ok()?;
        // the descriptor is kept for the life of the process (a few bytes, built once per listener)
        Some(SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() })
    }
}

/// One server end of the pipe, opened OVERLAPPED: a synchronous handle serialises a pending read with every write on the same file object,
/// so the reader thread would block the writer thread for good. Each call here has its own event, so both can wait at once.
struct Pipe(HANDLE);

// the handle is only used through the calls below, each of which is safe to make from any thread
unsafe impl Send for Pipe {}
unsafe impl Sync for Pipe {}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

const ERROR_IO_PENDING: i32 = 997;
const ERROR_PIPE_CONNECTED: i32 = 535;

impl Pipe {
    /// Starts one overlapped call with a private event and waits for it. A failure with these codes is the end of the stream, not an error.
    fn run(&self, start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>) -> std::io::Result<usize> {
        unsafe {
            let ev = CreateEventW(None, true, false, None).map_err(std::io::Error::from)?;
            let mut ov = OVERLAPPED { hEvent: ev, ..Default::default() };
            let mut n = 0u32;
            let r = match start(&mut ov) {
                Ok(()) => GetOverlappedResult(self.0, &ov, &mut n, true),
                Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_IO_PENDING as u32) => GetOverlappedResult(self.0, &ov, &mut n, true),
                Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_PIPE_CONNECTED as u32) => Ok(()),
                Err(e) => Err(e),
            };
            let _ = CloseHandle(ev);
            match r {
                Ok(()) => Ok(n as usize),
                // broken pipe (109), operation aborted (995, our own CancelIoEx), no process on the other end (233): the client is gone
                Err(e) if [109, 995, 233].iter().any(|c| e.code() == windows::core::HRESULT::from_win32(*c)) => Ok(0),
                Err(e) => Err(e.into()),
            }
        }
    }
    fn connect(&self) -> std::io::Result<()> {
        self.run(|ov| unsafe { ConnectNamedPipe(self.0, Some(ov)) }).map(|_| ())
    }
}

impl std::io::Read for &Pipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.run(|ov| unsafe { ReadFile(self.0, Some(buf), None, Some(ov)) })
    }
}

impl std::io::Write for &Pipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.run(|ov| unsafe { WriteFile(self.0, Some(buf), None, Some(ov)) })? {
            0 => Err(std::io::ErrorKind::BrokenPipe.into()),
            n => Ok(n),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn create_instance(name: &str, first: bool, sa: &SECURITY_ATTRIBUTES) -> std::io::Result<HANDLE> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut open = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open |= FILE_FLAG_FIRST_PIPE_INSTANCE; // a squatter that already owns the name makes this fail instead of receiving our clients
    }
    let h = unsafe { CreateNamedPipeW(PCWSTR(wide.as_ptr()), open, PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, 4096, 4096, 0, Some(sa as *const SECURITY_ATTRIBUTES)) };
    if h == INVALID_HANDLE_VALUE {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(h)
    }
}

/// Accepts clients until the generation changes. One instance waits at a time; each client gets its own two threads.
fn listen(name: String, g: u64) {
    let Some(sa) = security() else {
        eprintln!("liveact: could not build the pipe's access list; the pipe stays closed");
        return;
    };
    let mut first = true;
    let mut failures = 0;
    while GEN.load(Ordering::SeqCst) == g {
        let h = match create_instance(&name, first, &sa) {
            Ok(h) => h,
            Err(e) => {
                failures += 1;
                eprintln!("liveact: cannot create the pipe ({e})");
                if failures >= 5 || !crate::wake::nap(&GEN, g, 1000) {
                    break;
                }
                continue;
            }
        };
        first = false;
        let pipe = Arc::new(Pipe(h));
        // blocks until a client connects (or `stop` connects to wake us)
        let connected = pipe.connect().is_ok();
        if !connected || GEN.load(Ordering::SeqCst) != g {
            continue; // the pipe is closed on drop
        }
        std::thread::spawn(move || serve(g, pipe));
    }
}

fn serve(g: u64, pipe: Arc<Pipe>) {
    let raw = pipe.0 .0 as isize;
    let mut out = &*pipe;
    let (tx, rx) = sync_channel::<String>(64);
    let id = {
        let mut s = st();
        if !s.on || s.conns.len() >= MAX_CONNS || GEN.load(Ordering::SeqCst) != g {
            let _ = writeln!(out, "{}", json!({ "ok": false, "error": "busy", "detail": "too many connections, or Live activities are off" }));
            return;
        }
        s.next_conn += 1;
        let id = s.next_conn;
        s.conns.insert(id, Conn { tx: tx.clone(), handle: raw });
        id
    };
    // every write goes through one thread, so a client that never reads cannot block a command; its events are dropped when the queue is full
    let wp = pipe.clone();
    std::thread::spawn(move || {
        let mut out = &*wp;
        for line in rx {
            if writeln!(out, "{line}").is_err() {
                break;
            }
        }
    });
    let mut rd = BufReader::new(&*pipe);
    let mut limit = RateLimit::new(now_ms());
    loop {
        let mut buf = Vec::new();
        // one line, never more than MAX_LINE bytes of it: a client that sends no newline cannot grow our memory
        let n = (&mut rd).take(act::MAX_LINE as u64 + 1).read_until(b'\n', &mut buf).unwrap_or(0);
        if n == 0 || GEN.load(Ordering::SeqCst) != g {
            break;
        }
        if buf.last() != Some(&b'\n') && buf.len() > act::MAX_LINE {
            let _ = tx.try_send(json!({ "ok": false, "error": "too-long", "detail": format!("a message is at most {} bytes", act::MAX_LINE) }).to_string());
            break;
        }
        let reply = match String::from_utf8(buf) {
            Ok(line) if line.trim().is_empty() => continue,
            Ok(line) if !limit.allow(now_ms()) => {
                let _ = line;
                json!({ "ok": false, "error": "rate-limited", "detail": "too many messages; slow down" }).to_string()
            }
            Ok(line) => handle(id, &line, now_ms()),
            Err(_) => json!({ "ok": false, "error": "bad-json", "detail": "not UTF-8" }).to_string(),
        };
        let _ = tx.try_send(reply);
    }
    let mut s = st();
    s.conns.remove(&id);
    s.owner.retain(|_, c| *c != id);
}

/// Ends everything and closes every connection (the switch went off).
fn stop() {
    let mut s = st();
    s.on = false;
    let ids = s.reg.clear();
    ended(&mut s, ids, Reason::Disabled);
    publish(&mut s);
    for c in s.conns.values() {
        unsafe {
            let h = HANDLE(c.handle as *mut _);
            let _ = CancelIoEx(h, None);
            let _ = DisconnectNamedPipe(h);
        }
    }
    drop(s);
    // wake a listener that sits in ConnectNamedPipe
    let _ = std::fs::OpenOptions::new().read(true).write(true).open(PIPE);
}

fn sync(app: &AppHandle) {
    let _ = app;
    let want = crate::settings::get_bool("live.local", false);
    {
        let mut s = st();
        if s.on == want {
            return;
        }
        s.on = want;
    }
    let g = crate::wake::bump(&GEN);
    if want {
        std::thread::spawn(move || listen(PIPE.to_string(), g));
        std::thread::spawn(move || clock(g));
    } else {
        stop();
        crate::wake::bump(&TICK);
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let h2 = h.clone();
    h.listen("settings:changed", move |_| sync(&h2));
    sync(&h);
}

// ---------------------------------------------------------------- the command line

pub const USAGE: &str = "usage: pill live start|update|end --id ID [--title T] [--subtitle S] [--icon NAME|EMOJI] [--progress 0..1|NN%] [--chip C]\n\
       [--ttl SECONDS] [--action ID:LABEL]... [--clear subtitle|icon|progress|chip|actions] [--wait]\n\
  --wait keeps the connection open and prints each button press as one JSON line until the activity ends";

fn take(inline: Option<String>, it: &mut std::slice::Iter<String>, flag: &str) -> Result<String, String> {
    inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("{flag} needs a value"))
}

/// `pill live ...` arguments (after `live`) -> the one JSON line for the pipe and the --wait flag. Checked with the same parser the
/// island uses, so a bad message is refused here with the same words.
pub fn cli_to_line(args: &[String]) -> Result<(String, bool), String> {
    let op = args.first().map(String::as_str).filter(|o| ["start", "update", "end"].contains(o)).ok_or(USAGE)?;
    let mut o = Map::new();
    o.insert("op".into(), json!(op));
    let (mut actions, mut wait) = (Vec::new(), false);
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        match flag.as_str() {
            "--wait" => wait = true,
            "--id" | "--title" | "--subtitle" | "--icon" | "--chip" => {
                o.insert(flag[2..].to_string(), json!(take(inline, &mut it, &flag)?));
            }
            "--progress" => {
                let v = take(inline, &mut it, &flag)?;
                let p = match v.strip_suffix('%') {
                    Some(n) => n.trim().parse::<f64>().map(|x| x / 100.0),
                    None => v.trim().parse::<f64>(),
                };
                o.insert("progress".into(), json!(p.map_err(|_| format!("--progress: `{v}` is not a number"))?));
            }
            "--ttl" => {
                let v = take(inline, &mut it, &flag)?;
                o.insert("ttl_s".into(), json!(v.trim().parse::<u64>().map_err(|_| format!("--ttl: `{v}` is not a whole number of seconds"))?));
            }
            "--action" => {
                let v = take(inline, &mut it, &flag)?;
                let (id, label) = v.split_once(':').ok_or_else(|| format!("--action: `{v}` is not ID:LABEL"))?;
                actions.push(json!({ "id": id, "label": label }));
            }
            "--clear" => {
                let v = take(inline, &mut it, &flag)?;
                for f in v.split(',') {
                    let key = if f.trim() == "ttl" { "ttl_s" } else { f.trim() };
                    o.insert(key.to_string(), Value::Null);
                }
            }
            _ => return Err(format!("unknown option `{a}`\n{USAGE}")),
        }
    }
    if !actions.is_empty() {
        o.insert("actions".into(), Value::Array(actions));
    }
    let line = Value::Object(o).to_string();
    act::parse_line(&line).map_err(|e| format!("{}: {}", e.code, e.detail))?;
    Ok((line, wait))
}

/// A GUI-subsystem exe has no console: bind the parent's, unless the output was redirected already.
fn attach_console() {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        for (std_id, name) in [(STD_OUTPUT_HANDLE, "CONOUT$"), (STD_ERROR_HANDLE, "CONOUT$")] {
            let missing = GetStdHandle(std_id).map_or(true, |h| h.is_invalid() || h.0.is_null());
            if missing {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(name) {
                    if SetStdHandle(std_id, HANDLE(f.as_raw_handle())).is_ok() {
                        std::mem::forget(f);
                    }
                }
            }
        }
    }
}

/// `pill.exe live ...` is a client of the pipe: nothing of the app starts. None = not a live command (the normal start).
/// Exit codes: 0 accepted, 1 refused by the rules, 2 bad arguments, 3 Pill is not listening.
pub fn cli_main() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("live") {
        return None;
    }
    attach_console();
    let (line, wait) = match cli_to_line(&args[1..]) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return Some(2);
        }
    };
    let Ok(pipe) = std::fs::OpenOptions::new().read(true).write(true).open(PIPE) else {
        eprintln!("Pill is not running, or Live activities are off (Settings > Live activities)");
        return Some(3);
    };
    let mut w = &pipe;
    if writeln!(w, "{line}").is_err() {
        eprintln!("the pipe closed");
        return Some(3);
    }
    let mut r = BufReader::new(&pipe);
    let mut reply = String::new();
    if r.read_line(&mut reply).unwrap_or(0) == 0 {
        eprintln!("no answer");
        return Some(3);
    }
    println!("{}", reply.trim_end());
    let ok = serde_json::from_str::<Value>(&reply).ok().and_then(|v| v["ok"].as_bool()).unwrap_or(false);
    if ok && wait {
        let mut buf = String::new();
        while r.read_line(&mut buf).unwrap_or(0) > 0 {
            println!("{}", buf.trim_end());
            if serde_json::from_str::<Value>(&buf).is_ok_and(|v| v["event"] == "ended") {
                break;
            }
            buf.clear();
        }
    }
    Some(if ok { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_command_line_becomes_one_valid_message() {
        let (line, wait) = cli_to_line(&a(&["start", "--id", "b1", "--title", "Build", "--progress", "40%", "--ttl=90", "--action", "stop:Stop", "--action", "log:Open log", "--wait"])).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!((v["op"].clone(), v["id"].clone(), v["title"].clone(), v["ttl_s"].clone()), (json!("start"), json!("b1"), json!("Build"), json!(90)));
        assert!((v["progress"].as_f64().unwrap() - 0.4).abs() < 1e-9);
        assert_eq!(v["actions"], json!([{"id": "stop", "label": "Stop"}, {"id": "log", "label": "Open log"}]));
        assert!(wait);
        let (line, wait) = cli_to_line(&a(&["update", "--id", "b1", "--progress", "0.9", "--clear", "chip,subtitle"])).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!((v["chip"].clone(), v["subtitle"].clone(), wait), (Value::Null, Value::Null, false));
        assert_eq!(cli_to_line(&a(&["end", "--id", "b1"])).unwrap().0, r#"{"id":"b1","op":"end"}"#);
    }

    #[test]
    fn the_event_name_is_the_one_the_island_listens_to() {
        assert_eq!(act::STATE_EVENT, "liveact:state");
    }

    #[test]
    fn bad_command_lines_are_refused_before_the_pipe_is_touched() {
        for bad in [&["fly"][..], &[], &["start", "--id", "x"], &["start", "--title", "t"], &["start", "--id", "x", "--title", "t", "--progress", "lots"], &["start", "--id", "x", "--title", "t", "--progress", "2"],
            &["start", "--id", "x", "--title", "t", "--ttl", "99999"], &["start", "--id", "x", "--title", "t", "--action", "nolabel"], &["start", "--id", "x", "--title", "t", "--color", "red"],
            &["start", "--id", "x", "--title"], &["end", "--id", "x", "--title", "t"]] {
            assert!(cli_to_line(&a(bad)).is_err(), "{bad:?}");
        }
    }

    /// The real pipe, end to end on a private name: a start is accepted, the state reaches the bus, a rule refusal comes back as a reply,
    /// a click is delivered on the posting connection, and a closed switch ends everything.
    #[test]
    fn the_pipe_round_trip() {
        use std::time::Duration;
        let name = format!(r"\\.\pipe\pill-live-test-{}", std::process::id());
        let seen = std::sync::Arc::new(Mutex::new(Vec::<Value>::new()));
        let s2 = seen.clone();
        bus::subscribe(act::STATE_EVENT, move |v| s2.lock().unwrap().push(v.clone()));
        st().on = true;
        let g = crate::wake::bump(&GEN);
        let n2 = name.clone();
        std::thread::spawn(move || listen(n2, g));
        let open = || {
            for _ in 0..100 {
                if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(&name) {
                    return f;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("no pipe");
        };
        let c = open();
        let mut r = BufReader::new(&c);
        fn ask(mut c: &std::fs::File, r: &mut BufReader<&std::fs::File>, l: &str) -> Value {
            writeln!(c, "{l}").unwrap();
            let mut s = String::new();
            r.read_line(&mut s).unwrap();
            serde_json::from_str::<Value>(&s).unwrap_or_else(|_| panic!("{s}"))
        }
        assert_eq!(ask(&c, &mut r, r#"{"op":"start","id":"t1","title":"Test","actions":[{"id":"go","label":"Go"}]}"#)["state"], "started");
        assert_eq!(ask(&c, &mut r, r#"{"op":"start","id":"t1","title":"Test","color":"red"}"#)["error"], "unknown-field");
        assert_eq!(ask(&c, &mut r, r#"{"op":"update","id":"nope","progress":0.5}"#)["error"], "unknown-id");
        assert!(seen.lock().unwrap().last().unwrap()["acts"][0]["id"] == "t1");
        // the user presses the button, then dismisses the card
        assert!(liveact_action("t1".into(), "go".into()));
        assert!(!liveact_action("t1".into(), "undeclared".into()));
        let mut ev = String::new();
        r.read_line(&mut ev).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&ev).unwrap(), json!({"event": "action", "id": "t1", "action": "go"}));
        assert!(liveact_dismiss("t1".into()));
        ev.clear();
        r.read_line(&mut ev).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&ev).unwrap()["event"], "dismissed");
        assert_eq!(ask(&c, &mut r, r#"{"op":"update","id":"t1","progress":0.5}"#)["error"], "locked");
        assert_eq!(ask(&c, &mut r, r#"{"op":"end","id":"t1"}"#)["state"], "ended");
        // the switch goes off: the connection is closed
        crate::wake::bump(&GEN);
        stop();
        let mut s = String::new();
        assert_eq!(r.read_line(&mut s).unwrap_or(0), 0, "closed by the server: {s}");
    }
}
