// Command bar: configurable hotkey, Start Menu / Store app index, Everything (es.exe) file search,
// calculator, unit conversion, web search, `>` shell, confirmed system commands. Owner: command agent.
// Window `bar` is declared hidden in tauri.conf.json; page = bar.html + bar.js + bar.css.
// Events (to window "bar"): bar:shown, bar:hidden. Commands: cmd_toggle, cmd_query, cmd_run.
// Not done (later slice): timer / remind parsing, currency, runtime hotkey rebind (restart needed).
use crate::{settings, store};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow, WindowEvent};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow, SW_SHOWNORMAL};

const LABEL: &str = "bar";
const WIN_W: f64 = 680.0;
const WIN_H: f64 = 420.0;
const LIMIT: usize = 6;
const NO_WINDOW: u32 = 0x0800_0000;
const APPS_PREFIX: &str = r"shell:AppsFolder\";
const CONFIRM_MS: u64 = 3000;

#[derive(Clone, Serialize)]
pub struct Res {
    id: String,
    /// app | file | folder | calc | convert | web | system | shell | hint
    kind: &'static str,
    title: String,
    subtitle: String,
    payload: String,
}

#[derive(Serialize)]
pub struct Outcome {
    /// ok | confirm | error
    status: &'static str,
    message: String,
}

fn res(kind: &'static str, id: String, title: String, subtitle: String, payload: String) -> Res {
    Res { id, kind, title, subtitle, payload }
}
fn ok() -> Outcome {
    Outcome { status: "ok", message: String::new() }
}
fn err(m: impl Into<String>) -> Outcome {
    Outcome { status: "error", message: m.into() }
}

// Results of recent queries by id: cmd_run only ever acts on these, never on paths sent by the page.
static STORE: Mutex<Option<HashMap<String, Res>>> = Mutex::new(None);
static RECENT: Mutex<Vec<Res>> = Mutex::new(Vec::new()); // memory only, never persisted
static APPS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new()); // (name, lnk path | shell:AppsFolder\id)
static APPS_STORE_AT: Mutex<Option<Instant>> = Mutex::new(None);
static INDEXING: AtomicBool = AtomicBool::new(false);
static LAST_BLUR_HIDE: Mutex<Option<Instant>> = Mutex::new(None);
static PENDING: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static PREV_FG: AtomicIsize = AtomicIsize::new(0); // window that had focus before the bar opened (paste target)

fn register(list: &[Res]) {
    let mut g = STORE.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 500 {
        m.clear();
    }
    for r in list {
        m.insert(r.id.clone(), r.clone());
    }
}

// ---- settings (swap the body for settings::get_str once settings.rs lands) ----

fn setting(key: &str, default: &str) -> String {
    let read = || -> Option<String> {
        let p = Path::new(&std::env::var_os("APPDATA")?).join("pill").join("settings.json");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()?;
        let flat = v.get(key).and_then(|x| x.as_str());
        let nested = key.split('.').try_fold(&v, |c, k| c.get(k)).and_then(|x| x.as_str());
        flat.or(nested).map(str::to_string)
    };
    read().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| default.to_string())
}

// ---- init, hotkey, window ----

pub fn init(app: &tauri::App) {
    index_apps_async();
    if let Some(win) = app.get_webview_window(LABEL) {
        let (w2, h) = (win.clone(), app.handle().clone());
        win.on_window_event(move |e| match e {
            WindowEvent::Focused(false) if w2.is_visible().unwrap_or(false) => {
                *LAST_BLUR_HIDE.lock().unwrap() = Some(Instant::now());
                hide(&h);
            }
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide(&h);
            }
            _ => {}
        });
    }
    // the command-bar hotkey is registered (and re-registered live) by hotkeys.rs
}

fn hide(app: &AppHandle) {
    *STORE.lock().unwrap() = None; // in-app rows (clipboard text, notes) live in memory only while the bar is open
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.hide();
        let _ = app.emit_to(LABEL, "bar:hidden", ());
    }
}

/// Centered horizontally, upper third of the monitor under the cursor (physical pixels, DPI aware).
fn place(app: &AppHandle, win: &WebviewWindow) {
    let mon = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(mon) = mon else { return };
    let (mp, ms) = (mon.position(), mon.size());
    // Move first so the window adopts the target monitor's scale factor, then size it logically.
    let _ = win.set_position(PhysicalPosition::new(mp.x + 8, mp.y + 8));
    let _ = win.set_size(LogicalSize::new(WIN_W, WIN_H));
    let w = win.outer_size().map(|s| s.width as i32).unwrap_or((WIN_W * mon.scale_factor()) as i32);
    let x = mp.x + (ms.width as i32 - w) / 2;
    let y = mp.y + (7.0 * mon.scale_factor()) as i32; // hangs just below the top edge, like the vendor's command bar
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

fn toggle(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else { return };
    if win.is_visible().unwrap_or(false) {
        hide(app);
        return;
    }
    // The click that blurred (and hid) the bar must not re-open it.
    if LAST_BLUR_HIDE.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
        return;
    }
    PREV_FG.store(unsafe { GetForegroundWindow() }.0 as isize, Ordering::Relaxed);
    place(app, &win);
    index_apps_async();
    let _ = win.show();
    let _ = win.set_focus();
    let _ = app.emit_to(LABEL, "bar:shown", ());
}

#[tauri::command]
pub fn cmd_toggle(app: AppHandle) {
    toggle(&app);
}

#[tauri::command]
pub async fn cmd_query(text: String) -> Vec<Res> {
    tauri::async_runtime::spawn_blocking(move || query(&text)).await.unwrap_or_default()
}

// ---- query ----

fn query(text: &str) -> Vec<Res> {
    *PENDING.lock().unwrap() = None;
    let q0 = text.trim_start();
    let mut out: Vec<Res> = Vec::new();
    if q0.is_empty() {
        out = RECENT.lock().unwrap().clone();
    } else if let Some(cmd) = q0.strip_prefix('>') {
        let cmd = cmd.trim_start();
        out.push(if cmd.is_empty() {
            res("hint", "hint:shell".into(), "Type a command to run with cmd /c".into(), "Runs exactly what you type".into(), String::new())
        } else {
            res("shell", format!("shell:{cmd}"), format!("Run: {cmd}"), "cmd /c, hidden window".into(), cmd.to_string())
        });
    } else {
        let (scope, rest) = parse_scope(q0);
        let all = scope == Scope::All;
        let lw: Vec<String> = rest.split_whitespace().map(str::to_lowercase).collect();
        let mut special = false;
        if all || scope == Scope::Calc {
            if let Some((t, s, p)) = convert(rest) {
                out.push(res("convert", format!("convert:{p}"), t, s, p));
                special = true;
            } else if let Some((v, shown)) = calc_row(rest) {
                out.push(res("calc", format!("calc:{v}"), format!("= {v}"), format!("{shown}  -  Enter copies"), v));
                special = true;
            } else if scope == Scope::Calc {
                out.push(res("hint", "hint:calc".into(), "Not a valid expression".into(), "Try 12*(3+4) or sqrt(2)".into(), String::new()));
            }
        }
        if all {
            out.extend(system_rows(rest));
        }
        let words: Vec<String> = rest
            .split_whitespace()
            .filter(|w| !w.starts_with('-') && !w.starts_with('/')) // es.exe would read these as switches
            .map(str::to_string)
            .collect();
        if all || scope == Scope::Apps {
            out.extend(match_apps(&words));
            if scope == Scope::Apps && words.is_empty() {
                out.extend(RECENT.lock().unwrap().iter().filter(|r| r.kind == "app").cloned());
            }
        }
        out.extend(app_data(scope, &lw));
        if (all && !special || scope == Scope::Files) && !words.is_empty() && mod_on("module.fileSearch") {
            out.extend(files(&words));
        }
        if all && !rest.is_empty() {
            out.push(web_row(rest));
        }
        if all {
            out.sort_by_key(|r| rank(r, rest)); // stable: order inside a kind is kept
        }
    }
    register(&out);
    out
}

fn match_apps(words: &[String]) -> Vec<Res> {
    let ws: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    if ws.is_empty() {
        return vec![];
    }
    let mut hits: Vec<(u8, usize, Res)> = APPS
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(name, target)| {
            let n = name.to_lowercase();
            if !ws.iter().all(|w| n.contains(w.as_str())) {
                return None;
            }
            let rank = if n.starts_with(ws[0].as_str()) { 0 } else if n.contains(&format!(" {}", ws[0])) { 1 } else { 2 };
            let sub = if target.starts_with(APPS_PREFIX) { "App" } else { "Application" };
            Some((rank, n.len(), res("app", format!("app:{target}"), name.clone(), sub.into(), target.clone())))
        })
        .collect();
    hits.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    hits.into_iter().take(LIMIT).map(|h| h.2).collect()
}

fn files(words: &[String]) -> Vec<Res> {
    let (f, d) = std::thread::scope(|s| {
        let f = s.spawn(|| es(words, "/a-d"));
        let d = s.spawn(|| es(words, "/ad"));
        (f.join().unwrap_or(Err("Everything search failed")), d.join().unwrap_or(Err("Everything search failed")))
    });
    let mk = |kind: &'static str, p: String| {
        let name = Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
        res(kind, format!("{kind}:{p}"), name, p.clone(), p)
    };
    match (f, d) {
        (Ok(f), Ok(d)) => d.into_iter().map(|p| mk("folder", p)).chain(f.into_iter().map(|p| mk("file", p))).collect(),
        (Err(e), _) | (_, Err(e)) => vec![res("hint", "hint:es".into(), e.into(), "Files and folders need Everything".into(), String::new())],
    }
}

// ---- Everything (es.exe) ----

fn find_es() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(pf) = std::env::var_os(var) {
            dirs.push(Path::new(&pf).join("Everything"));
            dirs.push(Path::new(&pf).join("Everything 1.5a"));
        }
    }
    if let Some(pd) = std::env::var_os("ProgramData") {
        dirs.push(Path::new(&pd).join("chocolatey").join("bin"));
    }
    if let Some(up) = std::env::var_os("USERPROFILE") {
        dirs.push(Path::new(&up).join("scoop").join("shims"));
    }
    if let Some(la) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(Path::new(&la).join("Microsoft").join("WinGet").join("Links")); // winget install voidtools.Everything.Cli
    }
    dirs.into_iter().map(|d| d.join("es.exe")).find(|p| p.is_file())
}

fn es_answers(exe: &Path) -> bool {
    std::process::Command::new(exe).args(["-n", "1"]).creation_flags(NO_WINDOW).output().is_ok_and(|o| o.status.success())
}

/// The GUI exe of Everything (1.4: Everything.exe, 1.5a: Everything64.exe) in the usual install places.
fn find_everything_gui() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(pf) = std::env::var_os(var) {
            dirs.push(Path::new(&pf).join("Everything"));
            dirs.push(Path::new(&pf).join("Everything 1.5a"));
        }
    }
    if let Some(la) = std::env::var_os("LOCALAPPDATA") {
        // winget install voidtools.Everything: Packages\voidtools.Everything_<source hash>\
        if let Ok(rd) = std::fs::read_dir(Path::new(&la).join("Microsoft").join("WinGet").join("Packages")) {
            dirs.extend(rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with("voidtools.Everything")).map(|e| e.path()));
        }
    }
    dirs.into_iter().flat_map(|d| [d.join("Everything.exe"), d.join("Everything64.exe")]).find(|p| p.is_file())
}

fn everything_running() -> bool {
    std::process::Command::new("tasklist").args(["/FI", "IMAGENAME eq Everything*", "/NH"]).creation_flags(NO_WINDOW).output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("everything"))
}

/// Once per session, before the first file search: when Everything does not answer and its GUI is not running, start it
/// (`-startup` = hidden in the tray) and wait up to 2 s for es.exe to get an answer. Never kills anything.
/// Concurrent callers (files + folders search run in parallel) block in call_once until the first one is done.
fn ensure_everything(es_exe: &Path) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    if !settings::get_bool("everything.autostart", true) {
        return;
    }
    ONCE.call_once(|| {
        if es_answers(es_exe) {
            return;
        }
        if !everything_running() {
            let Some(gui) = find_everything_gui() else { return };
            // a GUI app: no CREATE_NO_WINDOW
            if std::process::Command::new(gui).arg("-startup").spawn().is_err() {
                return;
            }
        }
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(2) && !es_answers(es_exe) {
            std::thread::sleep(Duration::from_millis(150));
        }
    });
}

/// `es.exe -n LIMIT <attr> words...`, one full path per line.
fn es(words: &[String], attr: &str) -> Result<Vec<String>, &'static str> {
    let exe = find_es().ok_or("Everything not found: install Everything + es.exe for file search")?;
    ensure_everything(&exe);
    let out = std::process::Command::new(exe)
        .args(["-n", &LIMIT.to_string(), attr])
        .args(words)
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|_| "es.exe failed to start")?;
    if !out.status.success() {
        return Err("Everything is not running: start it for file search");
    }
    Ok(decode(&out.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

// es.exe writes the console OEM code page when piped, unless it already emits UTF-8.
fn decode(b: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_string();
    }
    unsafe {
        let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
        let n = MultiByteToWideChar(CP_OEMCP, flags, b, None);
        let mut buf = vec![0u16; n.max(0) as usize];
        MultiByteToWideChar(CP_OEMCP, flags, b, Some(&mut buf));
        String::from_utf16_lossy(&buf)
    }
}

// ---- app index: Start Menu .lnk files + shell:AppsFolder (Store apps) ----

fn index_apps_async() {
    if INDEXING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut apps: Vec<(String, String)> = Vec::new();
        for var in ["ProgramData", "APPDATA"] {
            if let Some(v) = std::env::var_os(var) {
                walk(&Path::new(&v).join(r"Microsoft\Windows\Start Menu\Programs"), &mut apps);
            }
        }
        // Get-StartApps is slow (~1 s): refresh it at most every 10 minutes. Constant script, no user input.
        let stale = APPS_STORE_AT.lock().unwrap().is_none_or(|t| t.elapsed() > Duration::from_secs(600));
        let mut store: Vec<(String, String)> = if stale {
            *APPS_STORE_AT.lock().unwrap() = Some(Instant::now());
            start_apps()
        } else {
            APPS.lock().unwrap().iter().filter(|a| a.1.starts_with(APPS_PREFIX)).cloned().collect()
        };
        apps.sort_by_key(|a| a.0.to_lowercase()); // .lnk entries win the name dedupe below
        apps.append(&mut store);
        let mut seen = std::collections::HashSet::new();
        apps.retain(|a| seen.insert(a.0.to_lowercase()));
        *APPS.lock().unwrap() = apps;
        INDEXING.store(false, Ordering::SeqCst);
    });
}

fn start_apps() -> Vec<(String, String)> {
    let script = "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-StartApps | ForEach-Object { $_.Name + [char]9 + $_.AppID }";
    let Ok(out) = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(NO_WINDOW)
        .output()
    else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(|l| l.trim().split_once('\t'))
        .filter(|(n, id)| !n.is_empty() && !id.is_empty() && !n.to_lowercase().contains("uninstall"))
        .map(|(n, id)| (n.to_string(), format!("{APPS_PREFIX}{id}")))
        .collect()
}

fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
            let name = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            if !name.to_lowercase().contains("uninstall") {
                out.push((name, p.to_string_lossy().into_owned()));
            }
        }
    }
}

// ---- web search ----

fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => o.push(b as char),
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

fn web_row(q: &str) -> Res {
    let (name, base) = match setting("search.engine", "google").to_lowercase().as_str() {
        "bing" => ("Bing", "https://www.bing.com/search?q="),
        "duckduckgo" => ("DuckDuckGo", "https://duckduckgo.com/?q="),
        _ => ("Google", "https://www.google.com/search?q="),
    };
    res("web", format!("web:{q}"), format!("Search {name} for \"{q}\""), "Opens your default browser".into(), format!("{base}{}", pct_encode(q)))
}

// ---- system commands (always need a second Enter within 3 s; never executed by tests) ----

const SYSTEM: &[(&str, &str, &[&str])] = &[
    ("lock", "Lock", &["lock", "lock screen"]),
    ("sleep", "Sleep", &["sleep", "suspend"]),
    ("hibernate", "Hibernate", &["hibernate"]),
    ("restart", "Restart", &["restart", "reboot"]),
    ("shutdown", "Shut down", &["shutdown", "shut down", "power off"]),
    ("signout", "Sign out", &["sign out", "log off", "logout", "log out"]),
];

fn system_rows(q: &str) -> Vec<Res> {
    let q = q.to_lowercase();
    if q.len() < 2 {
        return vec![];
    }
    SYSTEM
        .iter()
        .filter(|(_, _, al)| al.iter().any(|a| a.starts_with(&q)))
        .map(|(a, t, _)| res("system", format!("sys:{a}"), t.to_string(), "Press Enter twice to confirm".into(), a.to_string()))
        .collect()
}

#[link(name = "user32")]
extern "system" {
    fn LockWorkStation() -> i32;
}
#[link(name = "powrprof")]
extern "system" {
    fn SetSuspendState(hibernate: u8, force: u8, wakeup_events_disabled: u8) -> u8;
}

fn power(action: &str) {
    let shutdown = |arg: &str| {
        let _ = std::process::Command::new("shutdown.exe").args([arg, "/t", "0"]).creation_flags(NO_WINDOW).spawn();
    };
    unsafe {
        match action {
            "lock" => {
                LockWorkStation();
            }
            "sleep" => {
                SetSuspendState(0, 0, 0);
            }
            "hibernate" => {
                SetSuspendState(1, 0, 0);
            }
            "restart" => shutdown("/r"),
            "shutdown" => shutdown("/s"),
            "signout" => shutdown("/l"),
            _ => {}
        }
    }
}

// ---- run ----

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn shell_execute(verb: PCWSTR, file: &str) -> Outcome {
    if file.contains('\0') {
        return err("invalid target");
    }
    let f = wide(file);
    let r = unsafe { ShellExecuteW(HWND::default(), verb, PCWSTR(f.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        ok()
    } else {
        err(format!("Could not start (code {}): cancelled or access denied", r.0 as isize))
    }
}

/// Canonical existing path without the `\\?\` prefix (Explorer and ShellExecute dislike it).
fn canonical(p: &str) -> Option<String> {
    let c = std::fs::canonicalize(p).ok()?.to_string_lossy().into_owned();
    Some(match c.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => rest.to_string(),
        Some(rest) => format!(r"\\{}", &rest[4..]),
        None => c,
    })
}

fn remember(r: &Res) {
    let mut rc = RECENT.lock().unwrap();
    rc.retain(|x| x.id != r.id);
    rc.insert(0, r.clone());
    rc.truncate(8);
}

#[tauri::command]
pub fn cmd_run(app: AppHandle, id: String, mode: String) -> Outcome {
    let Some(r) = STORE.lock().unwrap().as_ref().and_then(|m| m.get(&id)).cloned() else {
        return err("Unknown result, type again");
    };
    let out = match r.kind {
        "calc" | "convert" => ok(), // the page already copied the value to the clipboard
        "web" => shell_execute(w!("open"), &r.payload),
        "shell" => {
            if mode != "open" {
                return err("Only plain Enter runs a > command");
            }
            // raw_arg: pass the typed text to cmd verbatim (the one deliberate shell string, typed by the user).
            match std::process::Command::new("cmd.exe").arg("/c").raw_arg(&r.payload).creation_flags(NO_WINDOW).spawn() {
                Ok(_) => ok(),
                Err(e) => err(e.to_string()),
            }
        }
        "system" => {
            if !confirmed(&id) {
                return Outcome { status: "confirm", message: "Press Enter again within 3 s".into() };
            }
            hide(&app);
            power(&r.payload);
            return ok();
        }
        "clip" => {
            let cid: i64 = r.payload.parse().unwrap_or(-1);
            if mode == "copy" {
                // copy only: the page writes the text to the clipboard (text items; images/files need Enter = paste)
                let t = store::with(|c| c.query_row("select text from clip_items where id=?1 and kind='text'", [cid], |r| r.get::<_, Option<String>>(0))).flatten();
                return match t {
                    Some(t) => Outcome { status: "copy", message: t },
                    None => err("Copy only works for text items, Enter pastes"),
                };
            }
            // after the bar hides: give focus back to the previous window, then paste exactly like the Clipboard tab does
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(120));
                let h = PREV_FG.load(Ordering::Relaxed);
                if h != 0 {
                    let _ = unsafe { SetForegroundWindow(HWND(h as *mut _)) };
                }
                std::thread::sleep(Duration::from_millis(80));
                let _ = tauri::async_runtime::block_on(crate::clipboard::clip_paste(cid, None, None));
            });
            ok()
        }
        "note" => {
            open_pill(&app, "note", json!({ "noteId": r.payload.parse::<i64>().unwrap_or(0) }));
            ok()
        }
        "reminder" => {
            open_pill(&app, "note", json!({ "section": "reminders" }));
            ok()
        }
        "tab" => {
            open_pill(&app, &r.payload, json!({}));
            ok()
        }
        "setting" => match SETTINGS.iter().find(|s| s.0 == r.payload) {
            Some((key, _, _, _, 'o')) => {
                open_pill(&app, "settings", json!({ "section": key }));
                ok()
            }
            Some((key, en, _, _, _)) => toggle_setting(&app, key, en),
            None => err("Unknown setting"),
        },
        "action" => match r.payload.as_str() {
            "clip-clear" => {
                if !confirmed(&id) {
                    return Outcome { status: "confirm", message: "Press Enter again within 3 s".into() };
                }
                tauri::async_runtime::block_on(crate::clipboard::clip_wipe());
                ok()
            }
            "clip-pause" => toggle_setting(&app, "private", "Clipboard paused"),
            "timer5" => {
                crate::timers::timer_start(300);
                ok()
            }
            "settings" => {
                open_pill(&app, "settings", json!({}));
                ok()
            }
            "voice" => {
                crate::voice::toggle(&app);
                ok()
            }
            _ => err("Unknown action"),
        },
        "app" | "file" | "folder" | "shelf" => {
            let store_app = r.payload.starts_with(APPS_PREFIX);
            let target = if store_app { Some(r.payload.clone()) } else { canonical(&r.payload) };
            let Some(t) = target else { return err("Path no longer exists") };
            let o = match mode.as_str() {
                _ if store_app && mode != "open" => err("Store apps have no file location or admin mode"),
                "folder" => match std::process::Command::new("explorer.exe").arg("/select,").arg(&t).creation_flags(NO_WINDOW).spawn() {
                    Ok(_) => ok(),
                    Err(e) => err(e.to_string()),
                },
                "admin" => shell_execute(w!("runas"), &t), // UAC prompt is the user's; the app never elevates itself
                _ => shell_execute(w!("open"), &t),
            };
            if o.status == "ok" && r.kind != "shelf" {
                remember(&r);
            }
            o
        }
        _ => err("This row cannot be run"),
    };
    if out.status == "ok" {
        hide(&app);
    }
    out
}

// ---- in-app spotlight: clipboard, notes, shelf, reminders, pill tabs, settings, quick actions ----
// All rows are computed on demand from the local store / static lists and kept only in STORE (cleared when the bar hides).

#[derive(Debug, PartialEq, Clone, Copy)]
enum Scope {
    All,
    Clip,
    Notes,
    Files,
    Apps,
    Settings,
    Calc,
}

/// `c foo` -> (Clip, "foo"), `= 2+2` -> (Calc, "2+2"). Letter prefixes (c n f a s) need a following space.
/// Input is trim_start'ed by the caller; without a prefix the whole trimmed text comes back.
fn parse_scope(q: &str) -> (Scope, &str) {
    if let Some(r) = q.strip_prefix('=') {
        return (Scope::Calc, r.trim());
    }
    let mut it = q.chars();
    if let (Some(c), Some(sp)) = (it.next(), it.next()) {
        if sp.is_whitespace() {
            let s = match c.to_ascii_lowercase() {
                'c' => Scope::Clip,
                'n' => Scope::Notes,
                'f' => Scope::Files,
                'a' => Scope::Apps,
                's' => Scope::Settings,
                _ => Scope::All,
            };
            if s != Scope::All {
                return (s, it.as_str().trim());
            }
        }
    }
    (Scope::All, q.trim())
}

/// Display order of mixed results: calculator/shell, system, exact-ish app, quick actions, pill tabs, settings,
/// clipboard, notes, reminders, shelf, other apps, folders, files, web search, hints.
fn rank(r: &Res, q: &str) -> u8 {
    match r.kind {
        "calc" | "convert" | "shell" => 0,
        "system" => 1,
        "app" if r.title.to_lowercase().starts_with(&q.to_lowercase()) => 2,
        "action" => 3,
        "tab" => 4,
        "setting" => 5,
        "clip" => 6,
        "note" => 7,
        "reminder" => 8,
        "shelf" => 9,
        "app" => 10,
        "folder" => 11,
        "file" => 12,
        "web" => 13,
        _ => 14,
    }
}

/// Every query word occurs in `hay` (case-insensitive substring); no words = everything.
fn hit(hay: &str, lw: &[String]) -> bool {
    let h = hay.to_lowercase();
    lw.iter().all(|w| h.contains(w.as_str()))
}

/// Every query word is the start of some word in `hay` (for the short static lists, so "set" does not match "reset").
fn hit_pre(hay: &str, lw: &[String]) -> bool {
    let h = hay.to_lowercase();
    lw.iter().all(|w| h.split(|c: char| !c.is_alphanumeric()).any(|x| x.starts_with(w.as_str())))
}

fn strip_html(s: &str) -> String {
    let (mut o, mut tag, mut in_tag) = (String::new(), String::new(), false);
    for c in s.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
                if matches!(name.to_ascii_lowercase().as_str(), "p" | "div" | "br" | "li" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "tr") {
                    o.push(' ');
                }
            }
            _ if in_tag => tag.push(c),
            _ => o.push(c),
        }
    }
    let o = o.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    o.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One line, at most `n` chars (+ an ellipsis).
fn preview(s: &str, n: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n {
        one.chars().take(n).collect::<String>() + "…"
    } else {
        one
    }
}

/// Window of `n` chars around the first occurrence of `w` (already lowercase).
fn snippet(s: &str, w: Option<&String>, n: usize) -> String {
    let ch: Vec<char> = s.chars().collect();
    let start = w
        .and_then(|w| {
            let low: Vec<char> = ch.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
            let nd: Vec<char> = w.chars().collect();
            if nd.is_empty() || low.len() < nd.len() {
                return None;
            }
            (0..=low.len() - nd.len()).find(|&i| low[i..i + nd.len()] == nd[..])
        })
        .map_or(0, |p| p.saturating_sub(20));
    let t: String = ch[start..].iter().take(n).collect();
    format!("{}{}{}", if start > 0 { "…" } else { "" }, t, if ch.len() > start + n { "…" } else { "" })
}

/// Single-line secrets are never previewed: known token prefixes, card-like numbers, long mixed-case+digit tokens.
/// ponytail: naive; clipboard.rs has a similar private heuristic (it also skips saving such items by default).
fn secretish(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    if t.chars().any(char::is_whitespace) {
        let d = t.chars().filter(|c| c.is_ascii_digit()).count();
        return (13..=19).contains(&d) && t.chars().all(|c| c.is_ascii_digit() || c == ' ' || c == '-');
    }
    if ["sk-", "ghp_", "gho_", "github_pat_", "AKIA", "ASIA", "xoxb-", "xoxp-", "AIza"].iter().any(|p| t.starts_with(p)) && t.len() >= 16 {
        return true;
    }
    t.len() >= 20
        && t.len() <= 256
        && t.chars().all(|c| c.is_ascii_alphanumeric() || "-_+/=.".contains(c))
        && t.chars().any(|c| c.is_ascii_lowercase())
        && t.chars().any(|c| c.is_ascii_uppercase())
        && t.chars().any(|c| c.is_ascii_digit())
}

fn rel_time(ms: i64) -> String {
    let m = ms.abs() / 60_000;
    let s = if m < 1 { "now".to_string() } else if m < 60 { format!("{m} min") } else if m < 2880 { format!("{} h", m / 60) } else { format!("{} d", m / 1440) };
    match (ms < 0, m < 1) {
        (_, true) => s,
        (true, _) => format!("overdue {s}"),
        _ => format!("in {s}"),
    }
}

const TABS: &[(&str, &str, &str)] = &[
    ("home", "Home", "Главная"),
    ("clipboard", "Clipboard", "Буфер"),
    ("shelf", "Shelf", "Полка"),
    ("text", "Text", "Текст"),
    ("chat", "Chat", "Чат"),
    ("note", "Note", "Заметка"),
    ("weather", "Weather", "Погода"),
    ("controls", "Controls", "Управление"),
    ("settings", "Settings", "Настройки"),
];

// (key, English label, Russian label, extra search words, mode: 't' toggle in settings.json, 'p' clipboard pause, 'o' just open Settings)
const SETTINGS: &[(&str, &str, &str, &str, char)] = &[
    ("theme", "Theme", "Тема", "dark light auto appearance", 'o'),
    ("size", "Pill size", "Размер", "scale zoom", 'o'),
    ("language", "Language", "Язык", "russian english", 'o'),
    ("autoHide", "Auto-hide", "Автоскрытие", "hide pill", 't'),
    ("showInCapture", "Show in screenshots", "Показывать на скриншотах", "capture recording", 't'),
    ("clock24", "24-hour clock", "24-часовой формат", "12 24 time clock format", 't'),
    ("private", "Private clipboard mode", "Приватный режим буфера", "pause clipboard history incognito", 'p'),
    ("hotkeys", "Hotkeys", "Горячие клавиши", "shortcut keys", 'o'),
    ("network", "Network features", "Сетевые функции", "internet weather chat online", 'o'),
];

const ACTIONS: &[(&str, &str, &str, &str)] = &[
    ("clip-clear", "Clear clipboard history", "Очистить историю буфера", "wipe delete"),
    ("clip-pause", "Pause clipboard (private mode)", "Пауза буфера (приватный режим)", "incognito stop recording"),
    ("timer5", "Start timer 5 min", "Таймер на 5 минут", "countdown"),
    ("settings", "Open settings", "Открыть настройки", "preferences"),
    ("voice", "Voice to text", "Голос в текст", "dictate dictation microphone speech запись голос диктовка"),
];

fn is_cyrillic(lw: &[String]) -> bool {
    lw.iter().any(|w| w.chars().any(|c| ('а'..='я').contains(&c) || c == 'ё'))
}

fn clip_paused() -> bool {
    tauri::async_runtime::block_on(crate::clipboard::clip_status())["paused"].as_bool().unwrap_or(false)
}

fn setting_on(key: &str) -> bool {
    match key {
        "private" => clip_paused(),
        "clock24" | "showInCapture" => settings::get_bool(key, true),
        _ => settings::get_bool(key, false),
    }
}

fn setting_sub(key: &str, mode: char) -> String {
    if mode != 'o' {
        return format!("{} - Enter switches it", if setting_on(key) { "On" } else { "Off" });
    }
    match key {
        "theme" => format!("Now: {} - opens Settings", settings::get_str("theme", "dark")),
        "size" => format!("Now: {}% - opens Settings", settings::get_str("size", "100")),
        "language" => format!("Now: {} - opens Settings", settings::get_str("language", "auto")),
        "hotkeys" => format!(
            "Toggle {}, bar {}, clipboard {}",
            settings::get_str("hotkey.toggle", "Ctrl+Alt+I"),
            settings::get_str("hotkey.command", "Alt+Shift+Space"),
            settings::get_str("hotkey.clipboard", "Ctrl+Alt+V")
        ),
        _ => "Weather, chat and other online features - opens Settings".into(),
    }
}

fn setting_rows(lw: &[String], scoped: bool) -> Vec<Res> {
    let cyr = is_cyrillic(lw);
    SETTINGS
        .iter()
        .filter(|(_, en, ru, al, _)| hit_pre(&format!("{en} {ru} {al}"), lw) || (scoped && lw.is_empty()))
        .map(|(k, en, ru, _, m)| res("setting", format!("set:{k}"), (if cyr { ru } else { en }).to_string(), setting_sub(k, *m), k.to_string()))
        .collect()
}

fn static_rows(lw: &[String]) -> Vec<Res> {
    let cyr = is_cyrillic(lw);
    let mut o: Vec<Res> = ACTIONS
        .iter()
        .filter(|(_, en, ru, al)| hit_pre(&format!("{en} {ru} {al}"), lw))
        .map(|(id, en, ru, _)| {
            let sub = match *id {
                "clip-clear" => "Press Enter twice to confirm".to_string(),
                "clip-pause" => format!("Now: {}", if clip_paused() { "paused" } else { "recording" }),
                "timer5" => "Counts down in the pill".into(),
                "voice" => "Press again to stop (must be enabled in Settings)".into(),
                _ => "Pill tab".into(),
            };
            res("action", format!("act:{id}"), (if cyr { ru } else { en }).to_string(), sub, id.to_string())
        })
        .collect();
    o.extend(TABS.iter().filter(|(id, en, ru)| tab_on(id) && hit_pre(&format!("{en} {ru}"), lw)).map(|(id, en, ru)| {
        let (a, b) = if cyr { (ru, en) } else { (en, ru) };
        res("tab", format!("tab:{id}"), a.to_string(), format!("Pill tab, {b}"), id.to_string())
    }));
    o.extend(setting_rows(lw, false));
    o
}

fn clip_rows(lw: &[String], limit: usize) -> Vec<Res> {
    let img = lw.is_empty() || lw.iter().all(|w| w.chars().count() >= 2 && "image".starts_with(w.as_str()));
    let rows = store::with(|c| {
        let mut st = c.prepare("select id, kind, substr(text,1,2000), files_json, coalesce(length(text),0) from clip_items order by pinned desc, used_at desc limit 300")?;
        let v = st
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, i64>(4)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    })
    .unwrap_or_default();
    let mut out = Vec::new();
    for (id, kind, text, files, len) in rows {
        let (title, sub) = match kind.as_str() {
            "text" => {
                let t = text.unwrap_or_default();
                if !hit(&t, lw) {
                    continue;
                }
                (if secretish(&t) { "Sensitive text (hidden)".to_string() } else { preview(&t, 80) }, format!("Text, {len} chars"))
            }
            "image" if img => ("Image".to_string(), "Image".to_string()),
            "files" => {
                let list: Vec<String> = serde_json::from_str(&files.unwrap_or_default()).unwrap_or_default();
                let names: Vec<String> = list.iter().map(|p| Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone())).collect();
                if !hit(&names.join(" "), lw) {
                    continue;
                }
                (preview(&names.join(", "), 80), format!("{} file(s)", list.len()))
            }
            _ => continue,
        };
        out.push(res("clip", format!("clip:{id}"), title, sub, id.to_string()));
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn note_rows(lw: &[String], limit: usize) -> Vec<Res> {
    let rows = store::with(|c| {
        let mut st = c.prepare("select id, title, substr(body_html,1,20000) from notes order by pinned desc, updated_at desc limit 200")?;
        let v = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    })
    .unwrap_or_default();
    let mut out = Vec::new();
    for (id, title, html) in rows {
        let body = strip_html(&html);
        if !hit(&format!("{title} {body}"), lw) {
            continue;
        }
        let t = if title.trim().is_empty() {
            let p = preview(&body, 40);
            if p.is_empty() { "Untitled note".into() } else { p }
        } else {
            title
        };
        out.push(res("note", format!("note:{id}"), t, snippet(&body, lw.first(), 80), id.to_string()));
        if out.len() >= limit {
            break;
        }
    }
    out
}

fn shelf_rows(lw: &[String]) -> Vec<Res> {
    store::with(|c| {
        let mut st = c.prepare("select id, path, name from shelf_items order by added_at desc limit 200")?;
        let v = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    })
    .unwrap_or_default()
    .into_iter()
    .filter(|(_, _, name)| hit(name, lw))
    .take(5)
    .map(|(id, path, name)| res("shelf", format!("shelf:{id}"), name, path.clone(), path))
    .collect()
}

fn reminder_rows(lw: &[String]) -> Vec<Res> {
    let now = store::now_ms();
    store::with(|c| {
        let mut st = c.prepare("select id, text, due_at from reminders where kind = 'reminder' and done = 0 order by due_at limit 100")?;
        let v = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    })
    .unwrap_or_default()
    .into_iter()
    .filter(|(_, text, _)| hit(text, lw))
    .take(5)
    .map(|(id, text, due)| res("reminder", format!("rem:{id}"), if text.is_empty() { "Reminder".into() } else { text }, format!("Reminder, {}", rel_time(due - now)), id.to_string()))
    .collect()
}

/// Settings > Modules: a module that is off contributes no rows to the command bar (and its Pill tab is not offered).
fn mod_on(key: &str) -> bool {
    settings::get_bool(key, true)
}

fn tab_on(id: &str) -> bool {
    match id {
        "home" => mod_on("module.media") || mod_on("module.launcher"),
        "clipboard" | "shelf" | "text" | "chat" | "note" | "weather" | "controls" => mod_on(&format!("module.{id}")),
        _ => true,
    }
}

fn app_data(scope: Scope, lw: &[String]) -> Vec<Res> {
    match scope {
        Scope::Clip if mod_on("module.clipboard") => clip_rows(lw, 8),
        Scope::Notes if mod_on("module.note") => note_rows(lw, 8),
        Scope::Settings => setting_rows(lw, true),
        Scope::All if lw.iter().map(|w| w.chars().count()).sum::<usize>() >= 2 => {
            let mut o = static_rows(lw);
            if mod_on("module.clipboard") {
                o.extend(clip_rows(lw, 4));
            }
            if mod_on("module.note") {
                o.extend(note_rows(lw, 4));
                o.extend(reminder_rows(lw));
            }
            if mod_on("module.shelf") {
                o.extend(shelf_rows(lw));
            }
            o
        }
        _ => vec![],
    }
}

/// Second Enter within 3 s on the same row confirms (system commands, clearing the clipboard).
fn confirmed(id: &str) -> bool {
    let mut p = PENDING.lock().unwrap();
    if p.as_ref().is_some_and(|(pid, t)| pid == id && t.elapsed() < Duration::from_millis(CONFIRM_MS)) {
        *p = None;
        true
    } else {
        *p = Some((id.to_string(), Instant::now()));
        false
    }
}

/// Expand the pill on `tab`; pages that care (tab-note.js) also get `pill:open-tab` with the extra fields.
fn open_pill(app: &AppHandle, tab: &str, extra: serde_json::Value) {
    let mut p = json!({ "tab": tab });
    if let (Some(m), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        m.extend(e.clone());
    }
    let _ = app.emit_to("pill", "pill:open-tab", p);
    crate::pill::expand(app, Some(tab.to_string()), "command");
}

/// Flip a switch through the same path as the Settings tab (settings_set persists, applies and broadcasts it).
fn toggle_setting(app: &AppHandle, key: &str, label: &str) -> Outcome {
    let now = if key == "private" {
        let paused = clip_paused();
        tauri::async_runtime::block_on(crate::clipboard::clip_pause(!paused));
        !paused
    } else {
        let v = !setting_on(key);
        let mut m = serde_json::Map::new();
        m.insert(key.to_string(), json!(v));
        settings::settings_set(app.clone(), m);
        v
    };
    Outcome { status: "updated", message: format!("{label}: {}", if now { "on" } else { "off" }) }
}

// ---- calculator (recursive descent, no eval) ----

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Id(String),
    Op(char),
}

fn lex(s: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let (mut i, mut t) = (0, Vec::new());
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch.is_ascii_digit() || ((ch == '.' || ch == ',') && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let (st, mut sep) = (i, false);
            while i < c.len() && (c[i].is_ascii_digit() || (!sep && (c[i] == '.' || c[i] == ','))) {
                sep |= c[i] == '.' || c[i] == ',';
                i += 1;
            }
            t.push(Tok::Num(c[st..i].iter().map(|&x| if x == ',' { '.' } else { x }).collect::<String>().parse().ok()?));
        } else if ch.is_alphabetic() {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            t.push(Tok::Id(c[st..i].iter().collect::<String>().to_lowercase()));
        } else {
            t.push(Tok::Op(match ch {
                '+' | '-' | '*' | '/' | '^' | '%' | '(' | ')' => ch,
                '×' => '*',
                '÷' => '/',
                _ => return None,
            }));
            i += 1;
        }
    }
    Some(t)
}

struct Parser {
    t: Vec<Tok>,
    i: usize,
    depth: u32,
}

impl Parser {
    fn op(&self) -> Option<char> {
        match self.t.get(self.i) {
            Some(Tok::Op(c)) => Some(*c),
            _ => None,
        }
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(o @ ('+' | '-')) = self.op() {
            self.i += 1;
            let r = self.term()?;
            v = if o == '+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(o @ ('*' | '/' | '%')) = self.op() {
            self.i += 1; // a '%' left here is binary: postfix() already took the percent-sign case
            let r = self.unary()?;
            v = match o {
                '*' => v * r,
                '/' => v / r,
                _ => v % r,
            };
        }
        Some(v)
    }
    fn unary(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let r = match self.op() {
            Some('-') => {
                self.i += 1;
                self.unary().map(|v| -v)
            }
            Some('+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        };
        self.depth -= 1;
        r
    }
    fn power(&mut self) -> Option<f64> {
        let b = self.postfix()?;
        if self.op() == Some('^') {
            self.i += 1;
            return Some(b.powf(self.unary()?)); // right associative, allows 2^-1
        }
        Some(b)
    }
    fn postfix(&mut self) -> Option<f64> {
        let mut v = self.primary()?;
        // `50%` is a percent sign unless an operand follows (`10 % 3` is modulo).
        while self.op() == Some('%') && !matches!(self.t.get(self.i + 1), Some(Tok::Num(_) | Tok::Id(_) | Tok::Op('('))) {
            self.i += 1;
            v /= 100.0;
        }
        Some(v)
    }
    fn primary(&mut self) -> Option<f64> {
        let tok = self.t.get(self.i)?.clone();
        self.i += 1;
        match tok {
            Tok::Num(n) => Some(n),
            Tok::Op('(') => {
                self.depth += 1;
                let v = if self.depth > 64 { None } else { self.expr() };
                self.depth -= 1;
                (self.op() == Some(')')).then(|| self.i += 1).and(v)
            }
            Tok::Id(name) if self.op() == Some('(') => {
                self.i += 1;
                self.depth += 1;
                let v = if self.depth > 64 { None } else { self.expr() };
                self.depth -= 1;
                let v = (self.op() == Some(')')).then(|| self.i += 1).and(v)?;
                Some(match name.as_str() {
                    "sqrt" => v.sqrt(),
                    "sin" => v.sin(),
                    "cos" => v.cos(),
                    "tan" => v.tan(),
                    "log" => v.log10(),
                    "ln" => v.ln(),
                    "abs" => v.abs(),
                    "round" => v.round(),
                    "floor" => v.floor(),
                    "ceil" => v.ceil(),
                    _ => return None,
                })
            }
            Tok::Id(name) => match name.as_str() {
                "pi" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Evaluate an arithmetic expression; None for anything malformed or non-finite.
fn calc(s: &str) -> Option<f64> {
    if s.len() > 200 {
        return None;
    }
    let t = lex(s)?;
    if t.is_empty() {
        return None;
    }
    let mut p = Parser { t, i: 0, depth: 0 };
    let v = p.expr()?;
    (p.i == p.t.len() && v.is_finite()).then_some(v)
}

fn fmt_num(v: f64) -> String {
    if v != 0.0 && !(1e-9..1e15).contains(&v.abs()) {
        return format!("{v:e}");
    }
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_string() }
}

/// (formatted result, expression as typed). A bare number is not a calculation.
fn calc_row(q: &str) -> Option<(String, String)> {
    if matches!(lex(q)?.as_slice(), [Tok::Num(_)]) {
        return None;
    }
    calc(q).map(|v| (fmt_num(v), q.to_string()))
}

// ---- unit conversion (offline) ----

// (names, category, factor to the category base). Temperature is handled separately.
const UNITS: &[(&str, &str, f64)] = &[
    ("mm|millimeter|millimeters", "len", 0.001),
    ("cm|centimeter|centimeters", "len", 0.01),
    ("m|meter|meters|metre|metres", "len", 1.0),
    ("km|kilometer|kilometers|kilometre|kilometres", "len", 1000.0),
    ("in|inch|inches", "len", 0.0254),
    ("ft|foot|feet", "len", 0.3048),
    ("yd|yard|yards", "len", 0.9144),
    ("mi|mile|miles", "len", 1609.344),
    ("mg|milligram|milligrams", "mass", 1e-6),
    ("g|gram|grams", "mass", 1e-3),
    ("kg|kilogram|kilograms", "mass", 1.0),
    ("t|tonne|tonnes", "mass", 1000.0),
    ("oz|ounce|ounces", "mass", 0.028349523125),
    ("lb|lbs|pound|pounds", "mass", 0.45359237),
    ("bit|bits", "data", 0.125),
    ("byte|bytes", "data", 1.0),
    ("kb|kilobyte|kilobytes", "data", 1e3),
    ("mb|megabyte|megabytes", "data", 1e6),
    ("gb|gigabyte|gigabytes", "data", 1e9),
    ("tb|terabyte|terabytes", "data", 1e12),
    ("kib", "data", 1024.0),
    ("mib", "data", 1_048_576.0),
    ("gib", "data", 1_073_741_824.0),
    ("tib", "data", 1_099_511_627_776.0),
    ("ms|millisecond|milliseconds", "time", 0.001),
    ("s|sec|secs|second|seconds", "time", 1.0),
    ("min|mins|minute|minutes", "time", 60.0),
    ("h|hr|hrs|hour|hours", "time", 3600.0),
    ("d|day|days", "time", 86400.0),
    ("wk|week|weeks", "time", 604800.0),
    ("c|celsius", "temp", 1.0),
    ("f|fahrenheit", "temp", 1.0),
    ("k|kelvin", "temp", 1.0),
];

fn unit(name: &str) -> Option<(&'static str, f64, &'static str)> {
    let n = name.to_lowercase();
    UNITS.iter().find(|(names, _, _)| names.split('|').any(|x| x == n)).map(|(names, cat, f)| (*cat, *f, names.split('|').next().unwrap()))
}

/// "5 km in mi" -> (title, subtitle, copy text).
fn convert(q: &str) -> Option<(String, String, String)> {
    let mut tok: Vec<String> = q.split_whitespace().map(str::to_string).collect();
    // glued "5km": split the first token at the first letter
    if let Some(first) = tok.first().cloned() {
        if let Some(p) = first.find(|c: char| c.is_alphabetic()) {
            if p > 0 {
                tok[0] = first[..p].to_string();
                tok.insert(1, first[p..].to_string());
            }
        }
    }
    let [num, from, sep, to] = tok.as_slice() else { return None };
    if !["in", "to", "as"].contains(&sep.to_lowercase().as_str()) {
        return None;
    }
    let v: f64 = num.replace(',', ".").parse().ok().filter(|v: &f64| v.is_finite())?;
    let ((c1, f1, _), (c2, f2, _)) = (unit(from)?, unit(to)?);
    if c1 != c2 {
        return None;
    }
    let r = if c1 == "temp" {
        let (a, b) = (from.to_lowercase(), to.to_lowercase());
        let cel = match a.chars().next()? {
            'f' => (v - 32.0) * 5.0 / 9.0,
            'k' => v - 273.15,
            _ => v,
        };
        match b.chars().next()? {
            'f' => cel * 9.0 / 5.0 + 32.0,
            'k' => cel + 273.15,
            _ => cel,
        }
    } else {
        v * f1 / f2
    };
    let text = format!("{} {}", fmt_num(r), to);
    Some((text.clone(), format!("{} {} in {}", fmt_num(v), from, to), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator() {
        let ok: &[(&str, f64)] = &[
            ("1+2*3", 7.0),
            ("(1+2)*3", 9.0),
            ("2^3^2", 512.0),
            ("-2^2", -4.0),
            ("2^-1", 0.5),
            ("--3", 3.0),
            ("10-4-3", 3.0),
            ("8/4/2", 1.0),
            ("50%", 0.5),
            ("200*10%", 20.0),
            ("10 % 3", 1.0),
            ("2,5+1", 3.5),
            ("1.5*2", 3.0),
            ("sqrt(16)+abs(-2)", 6.0),
            ("round(2.6)+floor(2.6)+ceil(2.1)", 8.0),
            ("ln(e)", 1.0),
            ("log(1000)", 3.0),
            ("2*pi", 2.0 * std::f64::consts::PI),
            ("3 × 4 ÷ 2", 6.0),
        ];
        for (s, want) in ok {
            let got = calc(s).unwrap_or_else(|| panic!("{s} -> None"));
            assert!((got - want).abs() < 1e-9, "{s} = {got}, want {want}");
        }
        for s in ["", "1+", "(1", "1)", "2 3", "1/0", "foo(1)", "abc", "1.2.3", "sqrt", "1+*2", "$5", "sin(", &"(".repeat(500)] {
            assert_eq!(calc(s), None, "{s:?} should be None");
        }
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
        assert_eq!(fmt_num(-0.0), "0");
        assert_eq!(calc_row("42"), None);
        assert!(calc_row("40+2").is_some());
    }

    #[test]
    fn units() {
        let near = |q: &str, want: &str| assert_eq!(convert(q).map(|c| c.2), Some(want.to_string()), "{q}");
        near("1 km in m", "1000 m");
        near("5km to mi", "3.1068559612 mi");
        near("0 c in f", "32 f");
        near("100 C to K", "373.15 K");
        near("-40 f in c", "-40 c");
        near("1,5 h in min", "90 min");
        near("1 gib in mb", "1073.741824 mb");
        for bad in ["5 km in kg", "5 km", "km in mi", "5 foo in m", "5 in in", "hello world", ""] {
            assert!(convert(bad).is_none(), "{bad:?}");
        }
        assert_eq!(pct_encode("a b&ü"), "a+b%26%C3%BC");
    }
    #[test]
    fn scopes_and_ranking() {
        let sc = parse_scope;
        assert_eq!(sc("c hello"), (Scope::Clip, "hello"));
        assert_eq!(sc("C  hello world "), (Scope::Clip, "hello world"));
        assert_eq!(sc("n todo"), (Scope::Notes, "todo"));
        assert_eq!(sc("f report"), (Scope::Files, "report"));
        assert_eq!(sc("a code"), (Scope::Apps, "code"));
        assert_eq!(sc("s theme"), (Scope::Settings, "theme"));
        assert_eq!(sc("c "), (Scope::Clip, ""));
        assert_eq!(sc("= 2+2"), (Scope::Calc, "2+2"));
        assert_eq!(sc("=2+2"), (Scope::Calc, "2+2"));
        assert_eq!(sc("chrome"), (Scope::All, "chrome")); // a letter without a space is plain text
        assert_eq!(sc("c"), (Scope::All, "c"));
        assert_eq!(sc("x foo"), (Scope::All, "x foo"));
        assert_eq!(sc("5 km in mi"), (Scope::All, "5 km in mi"));

        let r = |kind: &'static str, title: &str| res(kind, "i".into(), title.into(), String::new(), String::new());
        let mut v = vec![
            r("web", "Search"), r("file", "chrome.txt"), r("note", "chrome ideas"), r("clip", "chrome"), r("app", "Google Chrome"),
            r("app", "Chrome"), r("tab", "Clipboard"), r("action", "Open settings"), r("calc", "= 4"), r("system", "Lock"),
        ];
        v.sort_by_key(|x| rank(x, "chr"));
        let order: Vec<_> = v.iter().map(|x| (x.kind, x.title.as_str())).collect();
        assert_eq!(order[..4], [("calc", "= 4"), ("system", "Lock"), ("app", "Chrome"), ("action", "Open settings")]);
        assert_eq!(order[order.len() - 1].0, "web");
        let pos = |k: &str, t: &str| order.iter().position(|o| *o == (k, t)).unwrap();
        // exact app match < in-app data < other apps < files < web
        assert!(pos("app", "Chrome") < pos("clip", "chrome") && pos("clip", "chrome") < pos("note", "chrome ideas"));
        assert!(pos("note", "chrome ideas") < pos("app", "Google Chrome") && pos("app", "Google Chrome") < pos("file", "chrome.txt"));
    }

    #[test]
    fn inapp_helpers() {
        let w = |s: &str| s.split_whitespace().map(str::to_lowercase).collect::<Vec<_>>();
        assert!(hit("Hello World", &w("hello wor")) && !hit("Hello", &w("hello x")) && hit("anything", &[]));
        assert!(hit("Привет Мир", &w("мир")));
        assert!(hit_pre("Pill size", &w("si")) && !hit_pre("Reset", &w("set")) && hit_pre("Open settings", &w("set")));
        assert_eq!(strip_html("<p>Hi&nbsp;<b>there</b></p><ul><li>a &amp; b</li></ul>"), "Hi there a & b");
        assert_eq!(preview("a
  b   c", 80), "a b c");
        assert_eq!(preview(&"x".repeat(100), 80).chars().count(), 81);
        assert!(snippet(&"word ".repeat(40), Some(&"word".to_string()), 20).chars().count() <= 22);
        let sn = snippet(&format!("{}needle tail", "x ".repeat(30)), Some(&"needle".to_string()), 40);
        assert!(sn.starts_with('…') && sn.contains("needle"));
        assert!(secretish("ghp_abcdefghijklmnopqrstuvwxyz0123456789") && secretish("aB3dE6gH9jK2mN5pQ8sT1vW4yZ7cF0hJ") && secretish("4111 1111 1111 1111"));
        assert!(!secretish("hello world") && !secretish("https://example.com/a/long/path/index.html") && !secretish("password"));
        assert_eq!(rel_time(5 * 60_000), "in 5 min");
        assert_eq!(rel_time(-3 * 3_600_000), "overdue 3 h");
        assert_eq!(rel_time(10_000), "now");
    }
}
