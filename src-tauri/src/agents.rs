// Coding-agent status (GateGuard note: new file of the Pill app, used by main.rs + src/agents-live.js;
// user request: "agent completion reports in the pill"). Owner: agents lane (A7).
// PRIVACY: the app never edits another tool's config. Hooks the USER pastes write one small JSON file per session into
// %APPDATA%\pill\agents\ ; this module only reads that directory. File content is untrusted: size-capped, parsed
// defensively, titles are text only (control characters stripped, capped), never executed or passed to a shell.
// Events to window "pill": `agents:changed` {sessions, working, waiting}, `agents:done` {session, tool, title, cwd} (once per transition).
use crate::ui::bus;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle};

const MAX_FILE: u64 = 4096;
const MAX_FILES: usize = 64;
const TITLE_MAX: usize = 80;
const CWD_MAX: usize = 260;
const ID_MAX: usize = 64;
const STALE_MS: i64 = 30 * 60_000; // working/waiting without an update -> "stale"
const STALE_DROP_MS: i64 = 24 * 3_600_000; // stale files are removed after a day
const DONE_TTL_MS: i64 = 10 * 60_000; // done files are removed after 10 minutes
const FRESH_MS: i64 = 120_000; // a done file that appears with no earlier state still fires if it is this young
const NO_WINDOW: u32 = 0x0800_0000; // CREATE_NO_WINDOW
const HELPER_NAME: &str = "agent-hook.ps1";

// ---------- pure parts (unit-tested) ----------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Working,
    Waiting,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
struct Session {
    tool: String,
    session: String,
    state: State,
    title: String,
    cwd: String,
    ts: i64,
    /// the session's transcript file (from the hook payload): only its modification time is read, to notice that a prompt was answered
    transcript: String,
}

/// A transcript written this long after the "waiting" mark means the prompt was answered (the tool result / the answer is appended).
const RESUME_GAP_MS: i64 = 2_000;

/// Claude Code has no "resumed" hook: a session marked waiting whose transcript changed afterwards is working again.
fn resume_if_answered(s: &mut Session, transcript_mtime: Option<i64>) {
    if let (State::Waiting, Some(m)) = (s.state, transcript_mtime) {
        if m > s.ts + RESUME_GAP_MS {
            s.state = State::Working;
            s.ts = m;
        }
    }
}

/// mtime (ms) of a regular `.jsonl` file; nothing else is ever stat-ed.
fn transcript_mtime(path: &str) -> Option<i64> {
    let p = std::path::Path::new(path);
    if !p.extension().map_or(false, |x| x.eq_ignore_ascii_case("jsonl")) {
        return None;
    }
    let md = std::fs::metadata(p).ok().filter(|m| m.is_file())?;
    Some(md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
}

/// Text only: control characters become spaces, runs of whitespace collapse, capped at `max` chars.
fn clean_text(s: &str, max: usize) -> String {
    // control chars -> space; zero-width and bidi-override marks (can disguise text) are dropped
    let flat: String = s
        .chars()
        .filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut out = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max {
        out = out.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}";
    }
    out
}

/// Session ids end up in file names and in dismiss(): keep [A-Za-z0-9._-] only.
fn clean_id(s: &str) -> String {
    let id: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(ID_MAX).collect();
    id.trim_matches('.').to_string()
}

/// Parse one status file. `mtime` is the fallback timestamp when `ts` is missing or absurd.
fn parse_status(bytes: &[u8], mtime: i64, now: i64) -> Option<Session> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_FILE {
        return None;
    }
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let o = v.as_object()?;
    let st = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("");
    let state = match st("state") {
        "working" => State::Working,
        "waiting" => State::Waiting,
        "done" => State::Done,
        _ => return None,
    };
    let session = clean_id(st("session"));
    if session.is_empty() {
        return None;
    }
    let tool = match st("tool") {
        t @ ("claude-code" | "codex") => t.to_string(),
        _ => "other".to_string(),
    };
    let ts = o.get("ts").and_then(Value::as_i64).filter(|t| *t > 0 && *t <= now + 300_000).unwrap_or(mtime);
    Some(Session { tool, session, state, title: clean_text(st("title"), TITLE_MAX), cwd: clean_text(st("cwd"), CWD_MAX), ts, transcript: st("transcript").chars().take(CWD_MAX).collect() })
}

/// Last folder name of a working directory (either separator, trailing ones ignored): the project a session belongs to.
fn project_name(cwd: &str) -> &str {
    cwd.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or("")
}

/// Order of the sessions list: waiting first (they need you), then working, then the rest; newest first inside a group.
fn list_rank(state: &str) -> u8 {
    match state {
        "waiting" => 0,
        "working" => 1,
        _ => 2,
    }
}

fn key(s: &Session) -> String {
    format!("{}/{}", s.tool, s.session)
}

fn eff_state(s: &Session, now: i64) -> &'static str {
    match s.state {
        State::Done => "done",
        _ if now - s.ts > STALE_MS => "stale",
        State::Working => "working",
        State::Waiting => "waiting",
    }
}

#[derive(Clone)]
struct Entry {
    s: Session,
    file: PathBuf,
}

#[derive(Default)]
struct Out {
    done: Vec<Session>,
    delete: Vec<PathBuf>,
}

struct Tracker {
    map: BTreeMap<String, Entry>,
    primed: bool, // the first scan only sets the baseline: old "done" files never ring on startup
}

impl Tracker {
    const fn new() -> Self {
        Tracker { map: BTreeMap::new(), primed: false }
    }

    /// `listing`: every candidate file in the directory; None = unreadable or malformed (kept as it was, a half-written file heals itself).
    fn update(&mut self, listing: Vec<(PathBuf, Option<Session>)>, now: i64) -> Out {
        let mut out = Out::default();
        let old = std::mem::take(&mut self.map);
        // one winner per key (newest ts); duplicates are removed from disk
        let mut win: HashMap<String, (PathBuf, Session)> = HashMap::new();
        for (p, s) in listing.iter().filter_map(|(p, s)| s.as_ref().map(|s| (p, s))) {
            let k = key(s);
            match win.get(&k) {
                Some((wp, ws)) if ws.ts > s.ts || (ws.ts == s.ts && wp <= p) => out.delete.push(p.clone()),
                Some((wp, _)) => {
                    out.delete.push(wp.clone());
                    win.insert(k, (p.clone(), s.clone()));
                }
                None => {
                    win.insert(k, (p.clone(), s.clone()));
                }
            }
        }
        let mut keys: Vec<_> = win.keys().cloned().collect();
        keys.sort();
        for k in keys {
            let (file, s) = win.remove(&k).unwrap();
            let prev = old.get(&k).map(|e| e.s.state);
            if s.state == State::Done && prev != Some(State::Done) && self.primed && (prev.is_some() || now - s.ts <= FRESH_MS) {
                out.done.push(s.clone());
            }
            let expired = match s.state {
                State::Done => now - s.ts > DONE_TTL_MS,
                _ => now - s.ts > STALE_DROP_MS,
            };
            if expired {
                out.delete.push(file);
            } else {
                self.map.insert(k, Entry { s, file });
            }
        }
        // present but unreadable files keep their previous entry
        let present: HashSet<&PathBuf> = listing.iter().map(|(p, _)| p).collect();
        let mut bad: HashSet<&PathBuf> = HashSet::new();
        for (p, s) in &listing {
            if s.is_none() {
                bad.insert(p);
            }
        }
        for (k, e) in old {
            if present.contains(&e.file) && bad.contains(&e.file) {
                self.map.entry(k).or_insert(e);
            }
        }
        self.primed = true;
        out
    }

    fn snapshot(&self, now: i64) -> Value {
        let mut v: Vec<&Session> = self.map.values().map(|e| &e.s).collect();
        let state = |s: &Session| eff_state(s, now);
        v.sort_by(|a, b| list_rank(state(a)).cmp(&list_rank(state(b))).then_with(|| b.ts.cmp(&a.ts)).then_with(|| a.session.cmp(&b.session)));
        let (working, waiting) = (v.iter().filter(|s| state(s) == "working").count(), v.iter().filter(|s| state(s) == "waiting").count());
        let sessions: Vec<Value> = v
            .iter()
            .map(|s| json!({ "tool": s.tool, "session": s.session, "state": state(s), "title": s.title, "cwd": s.cwd, "project": project_name(&s.cwd), "ts": s.ts }))
            .collect();
        json!({ "sessions": sessions, "working": working, "waiting": waiting })
    }

    /// The working folder the newest entry of this session id reports (empty = unknown).
    fn cwd_of(&self, session: &str) -> Option<String> {
        self.map.values().filter(|e| e.s.session == session && !e.s.cwd.is_empty()).max_by_key(|e| e.s.ts).map(|e| e.s.cwd.clone())
    }

    /// Forget every session with this id; returns the files to delete.
    fn dismiss(&mut self, session: &str) -> Vec<PathBuf> {
        let ks: Vec<String> = self.map.iter().filter(|(_, e)| e.s.session == session).map(|(k, _)| k.clone()).collect();
        ks.iter().filter_map(|k| self.map.remove(k)).map(|e| e.file).collect()
    }
}

static TRACKER: Mutex<Tracker> = Mutex::new(Tracker::new());

fn lock() -> std::sync::MutexGuard<'static, Tracker> {
    TRACKER.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------- disk ----------
fn status_dir() -> PathBuf {
    let d = crate::store::dir().join("agents");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn hooks_dir() -> PathBuf {
    let d = crate::store::dir().join("hooks");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn scan(dir: &std::path::Path, now: i64) -> Vec<(PathBuf, Option<Session>)> {
    use std::io::Read;
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)) // symlinks and folders are not followed
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |x| x.eq_ignore_ascii_case("json")))
        .collect();
    files.sort();
    files.truncate(MAX_FILES);
    files
        .into_iter()
        .map(|p| {
            let parsed = (|| {
                let f = std::fs::File::open(&p).ok()?;
                let md = f.metadata().ok()?;
                if md.len() > MAX_FILE {
                    return None;
                }
                let mtime = md.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
                let mut buf = Vec::new();
                f.take(MAX_FILE + 1).read_to_end(&mut buf).ok()?;
                let mut s = parse_status(&buf, mtime, now)?;
                if s.state == State::Waiting && !s.transcript.is_empty() {
                    let m = transcript_mtime(&s.transcript);
                    resume_if_answered(&mut s, m);
                }
                Some(s)
            })();
            (p, parsed)
        })
        .collect()
}

// ---------- change notification (FindFirstChangeNotification = the cheap form of ReadDirectoryChangesW) ----------
struct Watcher(windows::Win32::Foundation::HANDLE);

impl Watcher {
    fn new(dir: &std::path::Path) -> Option<Watcher> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::*;
        let w: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let h = unsafe {
            FindFirstChangeNotificationW(PCWSTR(w.as_ptr()), false, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE)
        }
        .ok()?;
        Some(Watcher(h))
    }

    /// Waits up to `ms`; Err = the watch broke (directory removed...), the caller falls back to polling.
    fn wait(&self, ms: u32) -> Result<(), ()> {
        use windows::Win32::Foundation::WAIT_FAILED;
        use windows::Win32::Storage::FileSystem::FindNextChangeNotification;
        use windows::Win32::System::Threading::WaitForSingleObject;
        let r = unsafe { WaitForSingleObject(self.0, ms) };
        if r == WAIT_FAILED {
            return Err(());
        }
        if r.0 == 0 {
            unsafe { FindNextChangeNotification(self.0) }.map_err(|_| ())?;
            std::thread::sleep(std::time::Duration::from_millis(80)); // let a writer finish
        }
        Ok(())
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Storage::FileSystem::FindCloseChangeNotification(self.0);
        }
    }
}

fn run(_app: AppHandle) {
    let mut watcher: Option<Watcher> = None;
    let mut last = String::new();
    loop {
        let dir = status_dir(); // re-created if the user deleted it
        if watcher.is_none() {
            watcher = Watcher::new(&dir);
        }
        let now = crate::store::now_ms();
        let listing = scan(&dir, now);
        let (out, snap) = {
            let mut t = lock();
            let o = t.update(listing, now);
            (o, t.snapshot(now))
        };
        for p in out.delete {
            let _ = std::fs::remove_file(p);
        }
        let idle_ms = if snap["waiting"].as_i64().unwrap_or(0) > 0 { 2_000 } else { 10_000 };
        let txt = snap.to_string();
        if txt != last {
            last = txt;
            let _ = bus::emit("agents:changed", snap);
        }
        for s in out.done {
            let _ = bus::emit("agents:done", json!({ "session": s.session, "tool": s.tool, "title": s.title, "cwd": s.cwd }));
        }
        // 10 s tick with the watcher (stale expiry is time based), 2 s while a session waits (its transcript is not watched), 1.5 s when the watcher failed
        match &watcher {
            Some(w) => {
                if w.wait(idle_ms).is_err() {
                    watcher = None;
                }
            }
            None => std::thread::sleep(std::time::Duration::from_millis(1500)),
        }
    }
}

pub fn init(app: &tauri::App) {
    write_helper();
    let h = app.handle().clone();
    std::thread::spawn(move || run(h));
}

// ---------- hook snippets (text only: the user pastes them, nothing here touches another tool's files) ----------
/// ASCII only (Windows PowerShell 5.1 reads BOM-less files as ANSI). Always exits 0 and prints nothing:
/// a non-zero exit (2) from a Claude Code hook can block the agent, and UserPromptSubmit stdout is fed to the model.
const HELPER: &str = r#"# Pill agent-status helper. Writes %APPDATA%\pill\agents\<tool>-<session>.json; reads nothing else, sends nothing anywhere.
param([string]$Tool = 'claude-code', [string]$State = 'working')
try {
  $raw = ''
  if ($args.Count -gt 0 -and "$($args[0])".TrimStart().StartsWith('{')) { $raw = "$($args[0])" }
  else { try { $ms = New-Object IO.MemoryStream; [Console]::OpenStandardInput().CopyTo($ms); $raw = [Text.Encoding]::UTF8.GetString($ms.ToArray()) } catch { } }  # stdin is UTF-8 JSON: never go through the console code page
  $j = $null
  try { $j = $raw | ConvertFrom-Json } catch { }
  $sid = ''
  foreach ($k in 'session_id', 'thread-id', 'thread_id', 'conversation_id') { if (-not $sid -and $j -and $j.$k) { $sid = [string]$j.$k } }
  if (-not $sid) { $sid = $env:CLAUDE_SESSION_ID }
  if (-not $sid) { $sid = 'default' }
  $sid = ($sid -replace '[^A-Za-z0-9._-]', '_')
  if ($sid.Length -gt 64) { $sid = $sid.Substring(0, 64) }
  $dir = Join-Path $env:APPDATA 'pill\agents'
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  $file = Join-Path $dir ("$Tool-$sid.json")
  $old = $null
  if (Test-Path -LiteralPath $file) { try { $old = [IO.File]::ReadAllText($file, [Text.Encoding]::UTF8) | ConvertFrom-Json } catch { } }
  if ($State -eq 'waiting' -and $old -and $old.state -eq 'done') { exit 0 }   # idle reminder after the turn ended: stay "done"
  $title = ''
  if ($j) { foreach ($k in 'prompt', 'last-assistant-message', 'last_assistant_message') { if (-not $title -and $j.$k) { $title = [string]$j.$k } } }
  if (-not $title -and $old) { $title = [string]$old.title }
  $title = (($title -replace '\s+', ' ').Trim())
  if ($title.Length -gt 80) { $title = $title.Substring(0, 79) + '...' }
  $cwd = ''
  if ($j -and $j.cwd) { $cwd = [string]$j.cwd } elseif ($old) { $cwd = [string]$old.cwd }
  $tp = ''
  if ($j -and $j.transcript_path) { $tp = [string]$j.transcript_path } elseif ($old -and $old.transcript) { $tp = [string]$old.transcript }
  $o = [ordered]@{ tool = $Tool; session = $sid; state = $State; title = $title; cwd = $cwd; transcript = $tp; ts = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
  $tmp = "$file.tmp"
  [IO.File]::WriteAllText($tmp, ($o | ConvertTo-Json -Compress), (New-Object Text.UTF8Encoding($false)))
  Move-Item -LiteralPath $tmp -Destination $file -Force
} catch { }
exit 0
"#;

fn helper_path() -> PathBuf {
    hooks_dir().join(HELPER_NAME)
}

/// The helper lives in the app's OWN folder; written only when its content differs.
fn write_helper() {
    let p = helper_path();
    if std::fs::read_to_string(&p).ok().as_deref() != Some(HELPER) {
        let _ = std::fs::write(&p, HELPER);
    }
}

fn fwd(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn hook_cmd(path: &str, tool: &str, state: &str) -> String {
    format!("powershell -NoProfile -ExecutionPolicy Bypass -File \"{path}\" -Tool {tool} -State {state}")
}

fn snippets(helper: &str) -> Value {
    let hook = |state: &str| json!([{ "hooks": [{ "type": "command", "command": hook_cmd(helper, "claude-code", state) }] }]);
    let claude = serde_json::to_string_pretty(&json!({
        "hooks": { "UserPromptSubmit": hook("working"), "Notification": hook("waiting"), "Stop": hook("done") }
    }))
    .unwrap_or_default();
    let codex_args: Vec<String> =
        ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", helper, "-Tool", "codex", "-State", "done"].iter().map(|s| s.to_string()).collect();
    let codex = format!("notify = {}", serde_json::to_string(&codex_args).unwrap_or_default()); // JSON string/array syntax is valid TOML here
    json!({
        "dir": fwd(&status_dir()),
        "helper": helper,
        "claude_code": claude,
        "claude_code_where": "Merge the \"hooks\" block into ~/.claude/settings.json (user-wide) or <project>/.claude/settings.json; keep any hooks you already have.",
        "claude_code_optional": serde_json::to_string_pretty(&json!({ "PostToolUse": [{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_cmd(helper, "claude-code", "working") }] }] })).unwrap_or_default(),
        "codex": codex,
        "codex_where": "Add the line to ~/.codex/config.toml, above any [table] header. Codex appends one JSON argument; only the end of a turn is reported (no working/waiting states).",
        "uncertain": [
            "Claude Code passes session_id, transcript_path, cwd and hook_event_name as JSON on stdin; an env var CLAUDE_SESSION_ID is NOT documented (the helper reads stdin first).",
            "Which shell Claude Code uses for hook commands on Windows differs by version; the command has no $ or backslashes, so it should survive cmd, bash and PowerShell.",
            "Notification also fires as an idle reminder about a minute after the turn ended; the helper keeps such a session 'done'.",
            "Codex notify payload (type agent-turn-complete, thread-id, cwd, input-messages, last-assistant-message) is from memory; argument quoting through powershell -File may differ, the helper then falls back to session 'default' and an empty title.",
            "After you approve a permission prompt Claude Code has no 'resumed' hook: the optional PostToolUse hook flips the state back to working (adds ~0.2 s of PowerShell start per tool call)."
        ]
    })
}

// ---------- commands ----------
pub fn agents_list() -> Value {
    lock().snapshot(crate::store::now_ms())
}

/// Removes the session from the pill and deletes its status file (only files this module listed itself).
pub fn agents_dismiss(session: String) -> bool {
    let id = clean_id(&session);
    if id.is_empty() {
        return false;
    }
    let files = lock().dismiss(&id);
    let any = !files.is_empty();
    for f in files {
        let _ = std::fs::remove_file(f);
    }
    any
}

/// A session's folder as an existing, canonical, LOCAL directory (`C:\...`): no relative path, no UNC / mapped network drive, no file.
fn local_dir(cwd: &str) -> Option<PathBuf> {
    let c = cwd.trim();
    if c.is_empty() || !std::path::Path::new(c).is_absolute() || c.starts_with("\\\\") || c.starts_with("//") {
        return None;
    }
    let canon = std::fs::canonicalize(c).ok()?;
    let s = canon.to_string_lossy().into_owned();
    let plain = s.strip_prefix("\\\\?\\").unwrap_or(&s); // canonicalize returns \\?\C:\...; a network share comes back as \\?\UNC\...
    let b = plain.as_bytes();
    (canon.is_dir() && b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| PathBuf::from(plain))
}

/// First of `names` that exists as a file in a directory of the PATH-style list `path`.
fn find_in_path(path: &std::ffi::OsStr, names: &[&str]) -> Option<PathBuf> {
    std::env::split_paths(path).find_map(|d| names.iter().map(|n| d.join(n)).find(|p| p.is_file()))
}

/// VS Code's `code` launcher: PATH first, then the per-user install (%LOCALAPPDATA%\Programs\Microsoft VS Code\bin\code.cmd).
fn find_code_in(path: Option<&std::ffi::OsStr>, local_appdata: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    path.and_then(|p| find_in_path(p, &["code.cmd", "code.exe"]))
        .or_else(|| local_appdata.map(|l| PathBuf::from(l).join("Programs\\Microsoft VS Code\\bin\\code.cmd")).filter(|p| p.is_file()))
}

fn find_code() -> Option<PathBuf> {
    find_code_in(std::env::var_os("PATH").as_deref(), std::env::var_os("LOCALAPPDATA").as_deref())
}

/// The folder of a tracked session; the front end only ever names the session, never a path.
fn session_dir(session: &str) -> Option<PathBuf> {
    let id = clean_id(session);
    if id.is_empty() {
        return None;
    }
    let cwd = lock().cwd_of(&id)?;
    local_dir(&cwd)
}

pub fn agents_open_folder(session: String) -> bool {
    let Some(dir) = session_dir(&session) else { return false };
    std::process::Command::new("explorer.exe").arg(&dir).creation_flags(NO_WINDOW).spawn().is_ok() // dir = one argv element, no shell
}

pub fn agents_open_code(session: String) -> bool {
    let (Some(dir), Some(code)) = (session_dir(&session), find_code()) else { return false };
    std::process::Command::new(code).arg(&dir).creation_flags(NO_WINDOW).spawn().is_ok()
}

/// { code: bool }: whether the "VS Code" button on the finished card is offered.
pub fn agents_tools() -> Value {
    json!({ "code": find_code().is_some() })
}

pub fn agents_hook_snippets() -> Value {
    write_helper();
    snippets(&fwd(&helper_path()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn parse(s: &str) -> Option<Session> {
        parse_status(s.as_bytes(), NOW - 1000, NOW)
    }
    fn sess(id: &str, state: State, ts: i64) -> Session {
        Session { tool: "claude-code".into(), session: id.into(), state, title: "t".into(), cwd: String::new(), ts, transcript: String::new() }
    }
    fn file(id: &str) -> PathBuf {
        PathBuf::from(format!("{id}.json"))
    }

    #[test]
    fn snapshot_lists_waiting_first_then_working_then_newest() {
        let mut t = Tracker::new();
        let l = vec![
            (file("d"), Some(sess("d", State::Done, NOW - 1_000))),
            (file("w1"), Some(sess("w1", State::Working, NOW - 5_000))),
            (file("p"), Some(sess("p", State::Waiting, NOW - 60_000))),
            (file("w2"), Some(sess("w2", State::Working, NOW - 2_000))),
        ];
        t.update(l, NOW);
        let snap = t.snapshot(NOW);
        let ids: Vec<&str> = snap["sessions"].as_array().unwrap().iter().map(|s| s["session"].as_str().unwrap()).collect();
        assert_eq!(ids, ["p", "w2", "w1", "d"]);
    }

    #[test]
    fn project_is_the_last_folder() {
        assert_eq!(project_name(r"C:\Users\usr\island-replica"), "island-replica");
        assert_eq!(project_name("/home/me/app/"), "app");
        assert_eq!(project_name(r"D:\"), "D:");
        assert_eq!(project_name(""), "");
    }

    #[test]
    fn answered_prompt_resumes_working() {
        let mut s = sess("a", State::Waiting, 10_000);
        resume_if_answered(&mut s, Some(10_000 + RESUME_GAP_MS)); // written at once after the hook: not an answer
        assert_eq!(s.state, State::Waiting);
        resume_if_answered(&mut s, None);
        assert_eq!(s.state, State::Waiting);
        resume_if_answered(&mut s, Some(20_000));
        assert_eq!((s.state, s.ts), (State::Working, 20_000));
        let mut d = sess("b", State::Done, 10_000);
        resume_if_answered(&mut d, Some(99_000)); // only a waiting session is touched
        assert_eq!(d.state, State::Done);
        assert_eq!(transcript_mtime("C:/nope/x.txt"), None);
        assert_eq!(transcript_mtime("C:/definitely/missing.jsonl"), None);
    }

    #[test]
    fn sanitise() {
        assert_eq!(clean_text("a\u{0}b\nc\t\u{1b}[31md", 80), "a b c [31md");
        let long = "x".repeat(200);
        let c = clean_text(&long, 80);
        assert_eq!(c.chars().count(), 80);
        assert!(c.ends_with('\u{2026}'));
        assert_eq!(clean_text("  \u{202e}hi\u{200b}  there ", 80), "hi there"); // bidi override and zero-width removed
        assert_eq!(clean_id("../../etc/passwd"), "etcpasswd");
        assert_eq!(clean_id("a b\\c:d*e"), "abcde");
        assert_eq!(clean_id("..."), "");
        assert_eq!(clean_id(&"z".repeat(300)).len(), 64);
    }

    #[test]
    fn parse_validates() {
        let ok = parse(r#"{"tool":"codex","session":"s1","state":"done","title":"Fix\nbug","cwd":"C:\\p","ts":1799999999000}"#).unwrap();
        assert_eq!((ok.tool.as_str(), ok.state, ok.title.as_str(), ok.ts), ("codex", State::Done, "Fix bug", 1_799_999_999_000));
        assert_eq!(parse(r#"{"tool":"evil","session":"s","state":"working"}"#).unwrap().tool, "other");
        assert!(parse(r#"{"session":"s","state":"running"}"#).is_none()); // unknown state
        assert!(parse(r#"{"session":"///","state":"done"}"#).is_none()); // id empty after cleaning
        assert!(parse("not json").is_none());
        assert!(parse("[1,2]").is_none());
        assert!(parse("").is_none());
        let big = format!(r#"{{"session":"s","state":"done","title":"{}"}}"#, "a".repeat(5000));
        assert!(parse(&big).is_none()); // size cap
        // missing / future ts falls back to the file time
        assert_eq!(parse(r#"{"session":"s","state":"working"}"#).unwrap().ts, NOW - 1000);
        assert_eq!(parse(&format!(r#"{{"session":"s","state":"working","ts":{}}}"#, NOW + 10_000_000)).unwrap().ts, NOW - 1000);
        // wrong field types do not panic
        assert!(parse(r#"{"session":5,"state":["done"],"title":{}}"#).is_none());
    }

    #[test]
    fn done_fires_once() {
        let mut t = Tracker::new();
        // baseline: an old done file on startup is silent and then cleaned up
        let o = t.update(vec![(file("old"), Some(sess("old", State::Done, NOW - 20 * 60_000)))], NOW);
        assert!(o.done.is_empty());
        assert_eq!(o.delete, vec![file("old")]);
        assert!(t.map.is_empty());
        // working -> done fires exactly once, however often the file is re-read
        t.update(vec![(file("a"), Some(sess("a", State::Working, NOW)))], NOW);
        let d = t.update(vec![(file("a"), Some(sess("a", State::Done, NOW + 1000)))], NOW + 1000);
        assert_eq!(d.done.len(), 1);
        assert!(t.update(vec![(file("a"), Some(sess("a", State::Done, NOW + 1000)))], NOW + 2000).done.is_empty());
        assert!(t.update(vec![(file("a"), Some(sess("a", State::Done, NOW + 5000)))], NOW + 6000).done.is_empty()); // rewritten duplicate
        // a new turn: working again, then done again fires again
        t.update(vec![(file("a"), Some(sess("a", State::Working, NOW + 7000)))], NOW + 7000);
        assert_eq!(t.update(vec![(file("a"), Some(sess("a", State::Done, NOW + 8000)))], NOW + 8000).done.len(), 1);
    }

    #[test]
    fn done_without_earlier_state() {
        let mut t = Tracker::new();
        t.update(vec![], NOW); // baseline
        let fresh = t.update(vec![(file("b"), Some(sess("b", State::Done, NOW - 5000)))], NOW);
        assert_eq!(fresh.done.len(), 1); // Stop-only hook setup
        let mut t2 = Tracker::new();
        t2.update(vec![], NOW);
        assert!(t2.update(vec![(file("c"), Some(sess("c", State::Done, NOW - 300_000)))], NOW).done.is_empty()); // too old to announce
    }

    #[test]
    fn stale_and_expiry() {
        let mut t = Tracker::new();
        t.update(vec![(file("w"), Some(sess("w", State::Working, NOW)))], NOW);
        assert_eq!(t.snapshot(NOW + 60_000)["working"], 1);
        let later = NOW + STALE_MS + 1;
        let snap = t.snapshot(later);
        assert_eq!(snap["working"], 0);
        assert_eq!(snap["sessions"][0]["state"], "stale");
        // a waiting entry goes stale as well and is dropped after a day
        let o = t.update(vec![(file("w"), Some(sess("w", State::Waiting, NOW)))], NOW + STALE_DROP_MS + 1);
        assert_eq!(o.delete, vec![file("w")]);
        assert!(t.map.is_empty());
        // an update revives it
        t.update(vec![(file("w"), Some(sess("w", State::Working, later)))], later);
        assert_eq!(t.snapshot(later)["working"], 1);
    }

    #[test]
    fn vanished_unreadable_and_duplicates() {
        let mut t = Tracker::new();
        t.update(vec![(file("a"), Some(sess("a", State::Working, NOW))), (file("b"), Some(sess("b", State::Waiting, NOW)))], NOW);
        let s = t.snapshot(NOW);
        assert_eq!((s["working"].as_i64(), s["waiting"].as_i64()), (Some(1), Some(1)));
        // a half-written file keeps its entry; a deleted file loses it
        t.update(vec![(file("a"), None)], NOW + 1);
        assert_eq!(t.map.len(), 1);
        assert!(t.map.contains_key("claude-code/a"));
        // two files claiming the same session: newest wins, the other is deleted
        let o = t.update(
            vec![(PathBuf::from("x1.json"), Some(sess("a", State::Working, NOW + 10))), (PathBuf::from("x2.json"), Some(sess("a", State::Waiting, NOW + 20)))],
            NOW + 30,
        );
        assert_eq!(o.delete, vec![PathBuf::from("x1.json")]);
        assert_eq!(t.map["claude-code/a"].s.state, State::Waiting);
    }

    #[test]
    fn dismiss_and_snippets() {
        let mut t = Tracker::new();
        t.update(vec![(file("a"), Some(sess("a", State::Done, NOW)))], NOW);
        assert_eq!(t.dismiss("zzz"), Vec::<PathBuf>::new());
        assert_eq!(t.dismiss("a"), vec![file("a")]);
        assert!(t.map.is_empty());
        let s = snippets("C:/Users/x y/AppData/Roaming/pill/hooks/agent-hook.ps1");
        let c: Value = serde_json::from_str(s["claude_code"].as_str().unwrap()).unwrap(); // valid JSON
        let cmd = c["hooks"]["Stop"][0]["hooks"][0]["command"].as_str().unwrap();
        assert!(cmd.contains("-State done") && cmd.contains("\"C:/Users/x y/"));
        assert!(!cmd.contains('$'));
        assert!(s["codex"].as_str().unwrap().starts_with("notify = [\"powershell\""));
        assert!(HELPER.is_ascii());
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pill-agents-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn local_dir_accepts_only_existing_local_directories() {
        let d = scratch("dir");
        let got = local_dir(d.to_str().unwrap()).unwrap();
        assert!(!got.to_string_lossy().starts_with("\\\\"), "verbatim prefix stripped: {got:?}");
        assert!(got.is_dir());
        let f = d.join("a.txt");
        std::fs::write(&f, "x").unwrap();
        for bad in ["", "  ", "relative\\dir", "\\\\server\\share\\x", "//server/share", "\\\\?\\UNC\\server\\share", d.join("missing").to_str().unwrap(), f.to_str().unwrap()] {
            assert!(local_dir(bad).is_none(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn code_lookup_uses_path_then_local_install() {
        let (a, b) = (scratch("path"), scratch("local"));
        assert!(find_code_in(Some(a.as_os_str()), Some(b.as_os_str())).is_none());
        let bin = b.join("Programs\\Microsoft VS Code\\bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("code.cmd"), "").unwrap();
        assert_eq!(find_code_in(Some(a.as_os_str()), Some(b.as_os_str())), Some(bin.join("code.cmd")));
        std::fs::write(a.join("code.cmd"), "").unwrap();
        let both = std::env::join_paths([&a]).unwrap();
        assert_eq!(find_code_in(Some(&both), Some(b.as_os_str())), Some(a.join("code.cmd"))); // PATH wins
        assert!(find_code_in(None, None).is_none());
        let _ = (std::fs::remove_dir_all(&a), std::fs::remove_dir_all(&b));
    }

    #[test]
    fn cwd_of_picks_the_newest_known_folder() {
        let mut t = Tracker::new();
        let mut a = sess("s1", State::Done, 10);
        a.cwd = "C:\\old".into();
        let mut b = sess("s1", State::Done, 20);
        b.cwd = "C:\\new".into();
        t.map.insert("a".into(), Entry { s: a, file: PathBuf::new() });
        t.map.insert("b".into(), Entry { s: b, file: PathBuf::new() });
        t.map.insert("c".into(), Entry { s: sess("s2", State::Done, 30), file: PathBuf::new() }); // no cwd
        assert_eq!(t.cwd_of("s1").as_deref(), Some("C:\\new"));
        assert_eq!(t.cwd_of("s2"), None);
        assert_eq!(t.cwd_of("nope"), None);
    }
}
