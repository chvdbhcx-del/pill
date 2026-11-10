// Answer Claude Code from the island (setting `agents.answerFromIsland`, default OFF).
// Claude Code's PermissionRequest hook runs the small script Pill writes into its OWN folder (%APPDATA%\pill\hooks\answer-hook.ps1,
// shown in Settings; the user pastes the hook entry into their settings.json, the app never edits it). The script POSTs the request
// to this module's local HTTP bridge and prints the answer. A card on the island (live/agents.rs, event `agents:ask`) asks the user;
// the buttons call `agents_answer {id, choice}`. After ANSWER_MS without an answer the bridge replies "no decision" (empty body), so
// Claude Code asks in the terminal as usual; so it does when the bridge is off, full, unreachable or the request is not understood.
// PRIVACY / SECURITY: listens on 127.0.0.1 only, on a random port, with a random per-session token (BCryptGenRandom) that the request must
// present in `X-Pill-Token`; port + token are written to %LOCALAPPDATA%\Pill\agent-bridge.json while the bridge runs (removed when it
// stops). Nothing leaves the machine. Only a bounded body is read, and nothing from it is executed or passed to a shell.
// Idle cost: no thread runs while the switch is off; while on, one thread blocks in accept(); a request waits on a channel.
use crate::ui::bus;
use serde_json::{json, Map, Value};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const SETTING: &str = "agents.answerFromIsland";
/// How long the hook waits for the island before it falls through to the terminal. The card disappears a little earlier (CARD_MS).
pub const ANSWER_MS: u64 = 120_000;
const MAX_BODY: usize = 256 * 1024;
const MAX_HEAD: usize = 16 * 1024;
const MAX_PENDING: usize = 4;
const SCRIPT_NAME: &str = "answer-hook.ps1";

// ---------------------------------------------------------------- the request (pure)
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Permission,
    /// AskUserQuestion with one single-choice question and 2-3 options
    Question { question: String, options: Vec<String> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub kind: Kind,
    pub tool_name: String,
    pub summary: String,
    pub session: String,
    pub cwd: String,
    /// the hook's `tool_input` (a question's answer is returned inside it)
    input: Value,
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// One line saying what is asked: the command, the file, the address...
fn summary(tool: &str, input: &Value) -> String {
    let raw = match tool {
        "Bash" | "PowerShell" => str_of(input, "command"),
        "Write" | "Edit" | "MultiEdit" | "Read" => str_of(input, "file_path"),
        "NotebookEdit" => str_of(input, "notebook_path"),
        "WebFetch" => str_of(input, "url"),
        "WebSearch" => str_of(input, "query"),
        "Glob" | "Grep" => str_of(input, "pattern"),
        "ExitPlanMode" => str_of(input, "plan"),
        _ => None,
    };
    crate::agents::clean_text(raw.unwrap_or(""), 160)
}

/// The hook's stdin JSON. None = not something to ask the island about (the hook then gets "no decision" at once).
pub fn parse_request(body: &[u8]) -> Option<Request> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if str_of(&v, "hook_event_name").is_some_and(|e| e != "PermissionRequest") {
        return None;
    }
    let tool = crate::agents::clean_text(str_of(&v, "tool_name")?, 60);
    if tool.is_empty() {
        return None;
    }
    let input = v.get("tool_input").cloned().unwrap_or(Value::Null);
    let kind = if tool == "AskUserQuestion" {
        let qs = input.get("questions")?.as_array()?;
        let [q] = qs.as_slice() else { return None };
        let opts: Vec<String> = q.get("options")?.as_array()?.iter().filter_map(|o| str_of(o, "label")).map(|l| crate::agents::clean_text(l, 80)).collect();
        if q.get("multiSelect").and_then(Value::as_bool) == Some(true) || !(2..=3).contains(&opts.len()) || opts.iter().any(String::is_empty) || opts.len() != q["options"].as_array()?.len() {
            return None;
        }
        Kind::Question { question: crate::agents::clean_text(str_of(q, "question")?, 200), options: opts }
    } else {
        Kind::Permission
    };
    Some(Request {
        summary: summary(&tool, &input),
        kind,
        tool_name: tool,
        session: crate::agents::clean_id(str_of(&v, "session_id").unwrap_or("")),
        cwd: crate::agents::clean_text(str_of(&v, "cwd").unwrap_or(""), 260),
        input,
    })
}

/// `\uXXXX` for every non-ASCII char: the script prints this through the console, whose code page must not matter.
fn ascii_json(v: &Value) -> String {
    let mut out = String::new();
    for c in serde_json::to_string(v).unwrap_or_default().chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut b = [0u16; 2];
            for u in c.encode_utf16(&mut b) {
                out.push_str(&format!("\\u{u:04x}"));
            }
        }
    }
    out
}

fn decision(d: Value) -> Option<String> {
    Some(ascii_json(&json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": d } })))
}

/// What the hook prints for this answer (None = print nothing = no decision). `allow` / `deny`, or `opt:N` for a question.
pub fn decide(req: &Request, choice: &str) -> Option<String> {
    match (&req.kind, choice) {
        (Kind::Permission, "allow") => decision(json!({ "behavior": "allow" })),
        (Kind::Permission, "deny") => decision(json!({ "behavior": "deny", "message": "Denied from the Pill island", "interrupt": false })),
        (Kind::Question { question, options }, c) => {
            let label = options.get(c.strip_prefix("opt:")?.parse::<usize>().ok()?)?;
            let mut input = req.input.clone();
            input.as_object_mut()?.insert("answers".into(), json!({ question.as_str(): label }));
            decision(json!({ "behavior": "allow", "updatedInput": input }))
        }
        _ => None,
    }
}

fn valid_choice(req: &Request, choice: &str) -> bool {
    match &req.kind {
        Kind::Permission => matches!(choice, "allow" | "deny"),
        Kind::Question { options, .. } => choice.strip_prefix("opt:").and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| n < options.len()),
    }
}

// ---------------------------------------------------------------- waiting requests
struct Slot {
    id: String,
    tx: mpsc::Sender<String>,
    req: Request,
}

pub struct Bridge {
    pending: Mutex<Vec<Slot>>,
    seq: AtomicU64,
}

impl Bridge {
    pub const fn new() -> Bridge {
        Bridge { pending: Mutex::new(Vec::new()), seq: AtomicU64::new(1) }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Slot>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Registers a request; None when too many are already waiting.
    fn open(&self, req: Request) -> Option<(String, mpsc::Receiver<String>)> {
        let mut g = self.lock();
        if g.len() >= MAX_PENDING {
            return None;
        }
        let id = format!("{:x}-{:x}", crate::store::now_ms(), self.seq.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = mpsc::channel();
        g.push(Slot { id: id.clone(), tx, req });
        Some((id, rx))
    }
    fn close(&self, id: &str) {
        self.lock().retain(|s| s.id != id);
    }
    /// The island's answer. False: unknown or already finished request, or a choice that does not fit it.
    pub fn answer(&self, id: &str, choice: &str) -> bool {
        let mut g = self.lock();
        let Some(i) = g.iter().position(|s| s.id == id && valid_choice(&s.req, choice)) else { return false };
        g.remove(i).tx.send(choice.to_string()).is_ok()
    }
    /// Drops every waiting request (their hooks get "no decision").
    fn clear(&self) {
        self.lock().clear();
    }
}

enum Waited {
    Choice(String),
    Timeout,
    /// the hook went away (the user answered in the terminal)
    Gone,
}

/// True when the peer closed its end (the hook was cancelled). Does not consume data.
fn peer_closed(s: &TcpStream) -> bool {
    let _ = s.set_nonblocking(true);
    let mut b = [0u8; 1];
    let closed = match s.peek(&mut b) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
    };
    let _ = s.set_nonblocking(false);
    closed
}

fn wait_for(rx: &mpsc::Receiver<String>, s: &TcpStream, total: Duration) -> Waited {
    let end = Instant::now() + total;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left.min(Duration::from_millis(500))) {
            Ok(c) => return Waited::Choice(c),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Waited::Timeout,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if peer_closed(s) {
                    return Waited::Gone;
                }
                if Instant::now() >= end {
                    return Waited::Timeout;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- HTTP (just enough for one POST)
struct Http {
    method: String,
    path: String,
    token: String,
    body: Vec<u8>,
}

/// Request line, headers and a Content-Length body. Err = status to answer with.
fn read_http(s: &mut TcpStream) -> Result<Http, u16> {
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        if buf.len() > MAX_HEAD {
            return Err(431);
        }
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, path) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
    let (mut token, mut len) = (String::new(), 0usize);
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "x-pill-token" => token = v.trim().to_string(),
                "content-length" => len = v.trim().parse().map_err(|_| 400u16)?,
                _ => {}
            }
        }
    }
    if len > MAX_BODY {
        return Err(413);
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => return Err(400),
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(len);
    Ok(Http { method, path, token, body })
}

fn reply(s: &mut TcpStream, status: u16, body: &str) {
    let text = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Bad Request",
    };
    let _ = write!(s, "HTTP/1.1 {status} {text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = s.flush();
}

/// Constant-time comparison: the token is a secret.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

type Notify = Arc<dyn Fn(&str, Value) + Send + Sync>;

fn handle(bridge: &Bridge, token: &str, notify: &Notify, wait: Duration, mut s: TcpStream) {
    let http = match read_http(&mut s) {
        Ok(h) => h,
        Err(code) => return reply(&mut s, code, ""),
    };
    if !same(&http.token, token) {
        return reply(&mut s, 401, "");
    }
    if http.method != "POST" || http.path != "/permission" {
        return reply(&mut s, 404, "");
    }
    // anything we cannot or should not ask about: empty body = "no decision", Claude Code asks in the terminal
    let Some(req) = parse_request(&http.body) else { return reply(&mut s, 200, "") };
    let (project, session, cwd) = (crate::agents::project_name(&req.cwd).to_string(), req.session.clone(), req.cwd.clone());
    let (kind, question, options) = match &req.kind {
        Kind::Permission => ("permission", String::new(), Vec::new()),
        Kind::Question { question, options } => ("question", question.clone(), options.clone()),
    };
    let (tool_name, summary) = (req.tool_name.clone(), req.summary.clone());
    let Some((id, rx)) = bridge.open(req.clone()) else { return reply(&mut s, 200, "") };
    notify(
        "agents:ask",
        json!({ "id": id, "kind": kind, "tool": "claude-code", "toolName": tool_name, "summary": summary, "question": question, "options": options,
                "project": project, "session": session, "cwd": cwd }),
    );
    let out = match wait_for(&rx, &s, wait) {
        Waited::Choice(c) => decide(&req, &c),
        Waited::Gone => {
            bridge.close(&id); // first forget the request, then tell the island: an answer that races the event must find nothing
            notify("agents:ask-gone", json!({ "id": id }));
            None
        }
        Waited::Timeout => None,
    };
    bridge.close(&id);
    reply(&mut s, 200, out.as_deref().unwrap_or(""));
}

fn serve(listener: TcpListener, bridge: Arc<Bridge>, token: String, notify: Notify, stop: Arc<AtomicBool>, wait: Duration) {
    for conn in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(s) = conn else { continue };
        let (b, t, n) = (bridge.clone(), token.clone(), notify.clone());
        let _ = std::thread::Builder::new().name("agent-bridge-conn".into()).spawn(move || handle(&b, &t, &n, wait, s));
    }
}

// ---------------------------------------------------------------- switch on / off
struct Running {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static BRIDGE: OnceLock<Arc<Bridge>> = OnceLock::new();

fn bridge() -> Arc<Bridge> {
    BRIDGE.get_or_init(|| Arc::new(Bridge::new())).clone()
}

fn info_file() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Pill").join("agent-bridge.json")
}

/// 128 random bits (BCryptGenRandom) as 32 hex digits.
fn random_token() -> Option<String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut b = [0u8; 16];
    (unsafe { BCryptGenRandom(None, &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.0 == 0).then(|| b.iter().map(|x| format!("{x:02x}")).collect())
}

fn start() {
    let Some(token) = random_token() else { return };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return };
    let Ok(addr) = listener.local_addr() else { return };
    let file = info_file();
    if let Some(d) = file.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if std::fs::write(&file, json!({ "port": addr.port(), "token": token, "pid": std::process::id() }).to_string()).is_err() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let notify: Notify = Arc::new(|name, p| {
        let _ = bus::emit(name, p);
    });
    let (b, st) = (bridge(), stop.clone());
    let ok = std::thread::Builder::new().name("agent-bridge".into()).spawn(move || serve(listener, b, token, notify, st, Duration::from_millis(ANSWER_MS))).is_ok();
    if ok {
        *RUNNING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Running { addr, stop });
    } else {
        let _ = std::fs::remove_file(file);
    }
}

fn stop() {
    if let Some(r) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        r.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect_timeout(&r.addr, Duration::from_millis(500)); // wakes accept()
    }
    bridge().clear();
    let _ = std::fs::remove_file(info_file());
}

fn sync(on: bool) {
    let running = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    match (on, running) {
        (true, false) => start(),
        (false, _) => stop(), // also removes a file left behind by a crash
        _ => {}
    }
}

/// At startup: the bridge runs only when the switch is on.
pub fn init() {
    sync(crate::settings::get_bool(SETTING, false));
}

/// `settings_set`: the switch works at once, both ways.
pub fn on_settings(patch: &Map<String, Value>) {
    if let Some(on) = patch.get(SETTING).and_then(Value::as_bool) {
        sync(on);
    }
}

/// The island's button: false = nobody waits for this id any more.
pub fn agents_answer(id: String, choice: String) -> bool {
    bridge().answer(&id, &choice)
}

// ---------------------------------------------------------------- what Settings shows
/// ASCII only. Prints nothing (= no decision) on any problem; always exits 0 (an exit code 2 would block the tool).
const SCRIPT: &str = r#"# Pill: answer Claude Code permission requests from the island.
# Talks only to 127.0.0.1 (port + token from %LOCALAPPDATA%\Pill\agent-bridge.json, written while the switch is on). Prints nothing = no decision.
try {
  $ms = New-Object IO.MemoryStream
  [Console]::OpenStandardInput().CopyTo($ms)
  $bytes = $ms.ToArray()
  $cfg = [IO.File]::ReadAllText((Join-Path $env:LOCALAPPDATA 'Pill\agent-bridge.json')) | ConvertFrom-Json
  [Net.ServicePointManager]::Expect100Continue = $false
  $req = [Net.HttpWebRequest]::Create("http://127.0.0.1:$($cfg.port)/permission")
  $req.Method = 'POST'
  $req.Proxy = $null
  $req.Timeout = 125000
  $req.ReadWriteTimeout = 125000
  $req.ContentType = 'application/json'
  $req.Headers.Add('X-Pill-Token', [string]$cfg.token)
  $req.ContentLength = $bytes.Length
  $st = $req.GetRequestStream(); $st.Write($bytes, 0, $bytes.Length); $st.Close()
  $rs = $req.GetResponse().GetResponseStream()
  $out = (New-Object IO.StreamReader($rs, [Text.Encoding]::UTF8)).ReadToEnd()
  if ($out) { [Console]::Out.Write($out) }
} catch { }
exit 0
"#;

fn script_path() -> PathBuf {
    let d = crate::store::dir().join("hooks");
    let _ = std::fs::create_dir_all(&d);
    d.join(SCRIPT_NAME)
}

/// The script lives in the app's OWN folder; written only when its content differs.
fn write_script() -> PathBuf {
    let p = script_path();
    if std::fs::read_to_string(&p).ok().as_deref() != Some(SCRIPT) {
        let _ = std::fs::write(&p, SCRIPT);
    }
    p
}

/// The settings.json entry for `script` (forward slashes: no backslash escaping to get wrong). Timeout in seconds, a little above ANSWER_MS.
fn hook_json(script: &str) -> String {
    let cmd = format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"{script}\"");
    serde_json::to_string_pretty(&json!({ "hooks": { "PermissionRequest": [{ "hooks": [{ "type": "command", "command": cmd, "timeout": ANSWER_MS / 1000 + 10 }] }] } })).unwrap_or_default()
}

/// Extra keys of `agents_hook_snippets`.
pub fn snippet_fields() -> Value {
    let p = write_script();
    json!({
        "answer_on": crate::settings::get_bool(SETTING, false),
        "answer_hook": hook_json(&p.to_string_lossy().replace('\\', "/")),
        "answer_where": "Merge the \"hooks\" block into ~/.claude/settings.json (keep your other hooks). Needs the switch \"Answer agents from the island\" on. The script is already written to the file named in the command; its full text is below. A question with options is answered as allow + updatedInput.answers (the shape of the Agent SDK's AskUserQuestion); if your Claude Code version ignores that, it simply asks in the terminal.",
        "answer_script": SCRIPT,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn perm(tool: &str, input: Value) -> Vec<u8> {
        json!({ "hook_event_name": "PermissionRequest", "session_id": "s-1", "cwd": "C:\\dev\\app", "tool_name": tool, "tool_input": input }).to_string().into_bytes()
    }
    fn ask(opts: Value, multi: bool) -> Vec<u8> {
        perm("AskUserQuestion", json!({ "questions": [{ "question": "Which db?", "header": "DB", "multiSelect": multi, "options": opts }] }))
    }
    fn labels(l: &[&str]) -> Value {
        Value::Array(l.iter().map(|x| json!({ "label": x, "description": "d" })).collect())
    }

    #[test]
    fn requests_are_parsed_and_summarised() {
        let r = parse_request(&perm("Bash", json!({ "command": "rm -rf\n  build" }))).unwrap();
        assert_eq!((r.kind.clone(), r.tool_name.as_str(), r.summary.as_str(), r.session.as_str(), r.cwd.as_str()), (Kind::Permission, "Bash", "rm -rf build", "s-1", "C:\\dev\\app"));
        assert_eq!(parse_request(&perm("Edit", json!({ "file_path": "C:\\a\\b.rs" }))).unwrap().summary, "C:\\a\\b.rs");
        assert_eq!(parse_request(&perm("Mystery", json!({ "x": 1 }))).unwrap().summary, "");
        assert!(parse_request(b"not json").is_none());
        assert!(parse_request(&json!({ "hook_event_name": "PreToolUse", "tool_name": "Bash" }).to_string().into_bytes()).is_none());
        assert!(parse_request(&json!({ "tool_input": {} }).to_string().into_bytes()).is_none(), "no tool name");
        let long = parse_request(&perm("Bash", json!({ "command": "x".repeat(1000) }))).unwrap();
        assert_eq!(long.summary.chars().count(), 160);
    }

    #[test]
    fn only_simple_questions_go_to_the_island() {
        let r = parse_request(&ask(labels(&["SQLite", "Postgres"]), false)).unwrap();
        assert_eq!(r.kind, Kind::Question { question: "Which db?".into(), options: vec!["SQLite".into(), "Postgres".into()] });
        assert!(parse_request(&ask(labels(&["a", "b", "c"]), false)).is_some());
        assert!(parse_request(&ask(labels(&["a", "b", "c", "d"]), false)).is_none(), "too many buttons for a card");
        assert!(parse_request(&ask(labels(&["only"]), false)).is_none());
        assert!(parse_request(&ask(labels(&["a", "b"]), true)).is_none(), "multi-select");
        assert!(parse_request(&ask(json!([{ "label": "" }, { "label": "b" }]), false)).is_none());
        let two = perm("AskUserQuestion", json!({ "questions": [{ "question": "a", "options": [] }, { "question": "b", "options": [] }] }));
        assert!(parse_request(&two).is_none(), "several questions");
    }

    #[test]
    fn decisions_map_to_the_hook_output() {
        let r = parse_request(&perm("Bash", json!({ "command": "ls" }))).unwrap();
        let allow: Value = serde_json::from_str(&decide(&r, "allow").unwrap()).unwrap();
        assert_eq!(allow["hookSpecificOutput"], json!({ "hookEventName": "PermissionRequest", "decision": { "behavior": "allow" } }));
        let deny: Value = serde_json::from_str(&decide(&r, "deny").unwrap()).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert!(deny["hookSpecificOutput"]["decision"]["message"].as_str().unwrap().contains("Pill"));
        assert!(decide(&r, "opt:0").is_none() && decide(&r, "").is_none() && decide(&r, "maybe").is_none());
        // a question: allow + the tool input with the answer filled in; non-ASCII leaves as \u escapes
        let q = parse_request(&ask(labels(&["SQLite", "\u{41f}\u{43e}\u{441}\u{442}\u{433}\u{440}\u{435}\u{441}"]), false)).unwrap();
        let text = decide(&q, "opt:1").unwrap();
        assert!(text.is_ascii() && text.contains("\\u041f"));
        let v: Value = serde_json::from_str(&text).unwrap();
        let d = &v["hookSpecificOutput"]["decision"];
        assert_eq!((d["behavior"].as_str(), d["updatedInput"]["answers"]["Which db?"].as_str()), (Some("allow"), Some("\u{41f}\u{43e}\u{441}\u{442}\u{433}\u{440}\u{435}\u{441}")));
        assert_eq!(d["updatedInput"]["questions"][0]["question"], "Which db?", "the original input is kept");
        assert!(decide(&q, "opt:2").is_none() && decide(&q, "allow").is_none() && decide(&q, "opt:x").is_none());
    }

    #[test]
    fn answers_are_checked_and_used_once() {
        let b = Bridge::new();
        let r = parse_request(&perm("Bash", json!({ "command": "ls" }))).unwrap();
        let (id, rx) = b.open(r.clone()).unwrap();
        assert!(!b.answer("nope", "allow") && !b.answer(&id, "opt:0") && !b.answer(&id, "yes"));
        assert!(b.answer(&id, "deny"));
        assert_eq!(rx.recv().unwrap(), "deny");
        assert!(!b.answer(&id, "allow"), "already answered");
        for _ in 0..MAX_PENDING {
            assert!(b.open(r.clone()).is_some());
        }
        assert!(b.open(r).is_none(), "full: the hook falls through to the terminal");
        b.clear();
        assert!(b.lock().is_empty());
    }

    // ---- the real socket path, with a short wait and the island played by the test
    struct Rig {
        addr: SocketAddr,
        events: mpsc::Receiver<(String, Value)>,
        bridge: Arc<Bridge>,
        stop: Arc<AtomicBool>,
    }
    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn rig(wait_ms: u64) -> Rig {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, events) = channel();
        let tx = Mutex::new(tx);
        let notify: Notify = Arc::new(move |n, p| {
            let _ = tx.lock().unwrap().send((n.to_string(), p));
        });
        let (bridge, stop) = (Arc::new(Bridge::new()), Arc::new(AtomicBool::new(false)));
        let (b, st) = (bridge.clone(), stop.clone());
        std::thread::spawn(move || serve(listener, b, TOKEN.into(), notify, st, Duration::from_millis(wait_ms)));
        Rig { addr, events, bridge, stop }
    }
    impl Drop for Rig {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(self.addr);
        }
    }
    fn post(addr: SocketAddr, token: Option<&str>, path: &str, body: &[u8]) -> (u16, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        let tok = token.map(|t| format!("X-Pill-Token: {t}\r\n")).unwrap_or_default();
        write!(s, "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{tok}Content-Length: {}\r\n\r\n", body.len()).unwrap();
        s.write_all(body).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let code = out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        (code, out.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
    }

    #[test]
    fn the_token_is_required() {
        let r = rig(300);
        let body = perm("Bash", json!({ "command": "ls" }));
        assert_eq!(post(r.addr, None, "/permission", &body).0, 401);
        assert_eq!(post(r.addr, Some("wrong"), "/permission", &body).0, 401);
        assert_eq!(post(r.addr, Some(&TOKEN[..31]), "/permission", &body).0, 401, "a prefix is not the token");
        assert_eq!(post(r.addr, Some(TOKEN), "/other", &body).0, 404);
        assert!(r.events.try_recv().is_err(), "nothing reached the island");
        assert!(r.bridge.lock().is_empty());
    }

    #[test]
    fn an_unanswered_request_falls_through_after_the_wait() {
        let r = rig(300);
        let t0 = Instant::now();
        let (code, out) = post(r.addr, Some(TOKEN), "/permission", &perm("Bash", json!({ "command": "ls" })));
        assert_eq!((code, out.as_str()), (200, ""), "empty body = no decision");
        assert!(t0.elapsed() >= Duration::from_millis(300) && t0.elapsed() < Duration::from_secs(5));
        let (name, p) = r.events.try_recv().unwrap();
        assert_eq!((name.as_str(), p["kind"].as_str(), p["toolName"].as_str(), p["summary"].as_str(), p["project"].as_str()), ("agents:ask", Some("permission"), Some("Bash"), Some("ls"), Some("app")));
        assert!(r.bridge.lock().is_empty(), "the slot is freed");
        // not understood / not for the island: answered at once, nothing shown
        let t0 = Instant::now();
        assert_eq!(post(r.addr, Some(TOKEN), "/permission", b"{}"), (200, String::new()));
        assert_eq!(post(r.addr, Some(TOKEN), "/permission", &ask(labels(&["a"]), false)), (200, String::new()));
        assert!(t0.elapsed() < Duration::from_millis(250));
        assert!(r.events.try_recv().is_err());
    }

    #[test]
    fn the_island_answers_allow_deny_and_questions() {
        let r = rig(10_000);
        let addr = r.addr;
        let h = std::thread::spawn(move || post(addr, Some(TOKEN), "/permission", &perm("Write", json!({ "file_path": "C:\\a.txt" }))));
        let (_, p) = r.events.recv_timeout(Duration::from_secs(5)).unwrap();
        let id = p["id"].as_str().unwrap().to_string();
        assert!(r.bridge.answer(&id, "allow"));
        let (code, out) = h.join().unwrap();
        assert_eq!(code, 200);
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["hookSpecificOutput"]["decision"]["behavior"], "allow");

        let h = std::thread::spawn(move || post(addr, Some(TOKEN), "/permission", &ask(labels(&["SQLite", "Postgres"]), false)));
        let (_, p) = r.events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((p["kind"].as_str(), p["options"].clone(), p["question"].as_str()), (Some("question"), json!(["SQLite", "Postgres"]), Some("Which db?")));
        assert!(r.bridge.answer(p["id"].as_str().unwrap(), "opt:1"));
        let v: Value = serde_json::from_str(&h.join().unwrap().1).unwrap();
        assert_eq!(v["hookSpecificOutput"]["decision"]["updatedInput"]["answers"]["Which db?"], "Postgres");
    }

    #[test]
    fn a_cancelled_hook_clears_its_card() {
        let r = rig(10_000);
        let mut s = TcpStream::connect(r.addr).unwrap();
        let body = perm("Bash", json!({ "command": "ls" }));
        write!(s, "POST /permission HTTP/1.1\r\nX-Pill-Token: {TOKEN}\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
        s.write_all(&body).unwrap();
        let (_, p) = r.events.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(s); // Claude Code killed the hook: the user answered in the terminal
        let (name, g) = r.events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((name.as_str(), &g["id"]), ("agents:ask-gone", &p["id"]));
        assert!(!r.bridge.answer(p["id"].as_str().unwrap(), "allow"));
    }

    #[test]
    fn snippet_is_valid_json_ascii_and_pastes_nothing_secret() {
        let h: Value = serde_json::from_str(&hook_json("C:/Users/x y/AppData/Roaming/pill/hooks/answer-hook.ps1")).unwrap();
        let e = &h["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert!(e["command"].as_str().unwrap().contains("-File \"C:/Users/x y/"));
        assert_eq!(e["timeout"], 130);
        assert!(SCRIPT.is_ascii() && !SCRIPT.contains("exit 2") && SCRIPT.contains("exit 0"));
        assert!(!hook_json("C:/x.ps1").contains("token"), "the token is read from the file at run time");
        assert!(info_file().ends_with("Pill\\agent-bridge.json"));
    }

    #[test]
    fn ascii_json_escapes_astral_chars() {
        assert_eq!(ascii_json(&json!({ "a": "\u{1f600}" })), "{\"a\":\"\\ud83d\\ude00\"}");
    }
}
