// Command bar: configurable hotkey, Start Menu / Store app index, Everything (es.exe) file search,
// calculator, unit conversion, web search, `>` shell, confirmed system commands. Owner: command agent.
// Window `bar` is built hidden by ui/windows.rs and created on demand by ui/lazy.rs (destroyed after `ui.lazyBarSecs` hidden, 0 = never);
// page = bar.html + bar.js + bar.css, bar-ready.js tells Rust (cmd_ready) when the page has registered its event listeners.
// Events (to window "bar"): bar:shown, bar:hidden. Commands: cmd_toggle, cmd_query, cmd_run, cmd_ready.
// Not done (later slice): timer / remind parsing, currency, runtime hotkey rebind (restart needed).
use crate::ui::bus::{self, Target};
use crate::ui::lazy;
use crate::{settings, store};
use serde::Serialize;
use serde_json::json;
use std::cell::Cell;
use std::collections::HashMap;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewWindow, WindowEvent};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow, SW_SHOWNORMAL};

const LABEL: &str = "bar";
const WIN_W: f64 = 560.0;
const WIN_H: f64 = 350.0;
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
static APPS: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new()); // (name, lnk path | shell:AppsFolder\id, lowercase name)
static APPS_STORE_AT: Mutex<Option<Instant>> = Mutex::new(None);
static APPS_SCAN_AT: Mutex<Option<Instant>> = Mutex::new(None); // end of the last Start Menu walk
static INDEXING: AtomicBool = AtomicBool::new(false);
static LAST_BLUR_HIDE: Mutex<Option<Instant>> = Mutex::new(None);
/// When the bar was last shown: grab_keyboard moves focus between the window and its WebView, which reports a transient Focused(false).
static SHOWN_AT: Mutex<Option<Instant>> = Mutex::new(None);
static PENDING: Mutex<Option<(String, Instant)>> = Mutex::new(None);
static PREV_FG: AtomicIsize = AtomicIsize::new(0); // window that had focus before the bar opened (paste target)
static ICONS: Mutex<Option<Lru>> = Mutex::new(None); // path -> shell icon data URL, bounded; the page also keeps its own per-session map
// Id of the newest cmd_query (and bumped when the bar hides). A query running on an older id sees it moved on and stops early.
static CUR_SEQ: AtomicU64 = AtomicU64::new(0);
thread_local! { static MY_SEQ: Cell<u64> = const { Cell::new(0) }; } // the id the current blocking worker was started with

/// Poison-tolerant lock: one panicking query must not leave the bar empty until restart.
fn lk<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// True once a newer query (or a hide) superseded the one running on this thread.
fn stale() -> bool {
    CUR_SEQ.load(Ordering::Relaxed) != MY_SEQ.with(Cell::get)
}

fn register(list: &[Res]) {
    let mut g = lk(&STORE);
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 500 {
        m.clear();
    }
    for r in list {
        m.insert(r.id.clone(), r.clone());
    }
}

/// Tiny bounded LRU (most recent last). 64 icons x ~10 KB data URL stays under a megabyte.
struct Lru {
    cap: usize,
    v: Vec<(String, String)>,
}

impl Lru {
    fn new(cap: usize) -> Self {
        Lru { cap, v: Vec::new() }
    }
    fn get(&mut self, k: &str) -> Option<String> {
        let i = self.v.iter().position(|e| e.0 == k)?;
        let e = self.v.remove(i);
        let val = e.1.clone();
        self.v.push(e);
        Some(val)
    }
    fn put(&mut self, k: String, val: String) {
        self.v.retain(|e| e.0 != k);
        if self.v.len() >= self.cap {
            self.v.remove(0);
        }
        self.v.push((k, val));
    }
}

// ---- init, hotkey, window ----

pub fn init(app: &tauri::App) {
    index_apps_async();
    // The bar is created on demand (ui/lazy.rs); with `ui.lazyBarSecs` = 0 it is never torn down, so it is built at start like before.
    if lazy_bar_secs() == 0 {
        crate::ui::lazy::get_or_create(app.handle(), LABEL);
    }
    // the command-bar hotkey is registered (and re-registered live) by hotkeys.rs
}

/// Idle seconds before the hidden bar window is destroyed: settings key `ui.lazyBarSecs`, default 120, 0 = never, otherwise 5 .. 86400.
pub(crate) fn lazy_bar_secs() -> u64 {
    lazy_secs_from(settings::get_f64("ui.lazyBarSecs", 120.0))
}

fn lazy_secs_from(v: f64) -> u64 {
    if !v.is_finite() {
        120
    } else if v < 1.0 {
        0
    } else {
        v.clamp(5.0, 86_400.0) as u64
    }
}

/// Window events of a freshly built bar (ui/lazy.rs calls this for every new window).
pub(crate) fn attach_bar(app: &AppHandle, win: &WebviewWindow) {
    let (w2, h) = (win.clone(), app.clone());
    win.on_window_event(move |e| match e {
        WindowEvent::Focused(false) if w2.is_visible().unwrap_or(false) => {
            if lk(&SHOWN_AT).is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
                return; // our own focus hand-over (grab_keyboard), not the user clicking away
            }
            *lk(&LAST_BLUR_HIDE) = Some(Instant::now());
            hide(&h);
        }
        // Alt+F4 / close on the bar only hides it; the idle teardown (ui/lazy.rs) destroys the window, which does not ask
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(&h);
        }
        _ => {}
    });
}

// The page of a new bar window reports (cmd_ready) once its scripts have run and every event listener is registered; showing earlier would
// lose bar:shown. Forgotten when a window is (re)built.
static READY: Mutex<bool> = Mutex::new(false);
static READY_CV: Condvar = Condvar::new();
static OPENING: AtomicBool = AtomicBool::new(false); // a slow open (window being built / loaded) is in flight: more presses are ignored
const READY_WAIT: Duration = Duration::from_secs(4);

pub(crate) fn bar_reset() {
    *lk(&READY) = false;
}

fn is_ready() -> bool {
    *lk(&READY)
}

/// Blocks until the page reported or `max` passed; true when it reported.
fn wait_ready(max: Duration) -> bool {
    let g = lk(&READY);
    let (g, _) = READY_CV.wait_timeout_while(g, max, |r| !*r).unwrap_or_else(|e| e.into_inner());
    *g
}

/// bar-ready.js: the page is loaded and listening. A report that comes after the open gave up waiting (bar already on screen) replays bar:shown,
/// which that page missed.
#[tauri::command]
pub fn cmd_ready(app: AppHandle) {
    *lk(&READY) = true;
    READY_CV.notify_all();
    if bar_visible(&app) {
        let _ = bus::emit(&app, Target::Label(LABEL), "bar:shown", ());
    }
}

/// tao makes a transparent window by switching DWM blur-behind on with an empty region, once, when the window is created (it even has a FIXME
/// about redoing it). Whatever switches blur-behind off afterwards leaves the window an opaque black slab until it is recreated: a Windhawk mod
/// such as Translucent Windows restoring every window it touched when it is unloaded, a DWM restart. So the windows re-assert it (it is idempotent).
pub fn keep_transparent(win: &WebviewWindow) {
    use windows::Win32::Graphics::Dwm::{DwmEnableBlurBehindWindow, DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND};
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject};
    let Ok(h) = win.hwnd() else { return };
    unsafe {
        let region = CreateRectRgn(0, 0, -1, -1);
        let bb = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION, fEnable: true.into(), hRgnBlur: region, fTransitionOnMaximized: false.into() };
        let _ = DwmEnableBlurBehindWindow(HWND(h.0 as isize as *mut std::ffi::c_void), &bb);
        let _ = DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(region.0));
    }
}

fn hide(app: &AppHandle) {
    CUR_SEQ.fetch_add(1, Ordering::Relaxed); // a query still running for the closed bar stops early and registers nothing
    *lk(&STORE) = None; // in-app rows (clipboard text, notes) live in memory only while the bar is open
    crate::notes::search_cache_drop(); // so does the note text index
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.hide();
        let _ = bus::emit(&app, Target::Label(LABEL), "bar:hidden", ());
        crate::ui::lazy::note_hidden(app, LABEL); // starts the idle count: the window is destroyed after ui.lazyBarSecs hidden
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
    // hangs just below the top edge, like the vendor's command bar - but never over the pill: under its body when it is showing
    let top = mp.y + (7.0 * mon.scale_factor()) as i32;
    let y = crate::pill::body_bottom(mp.x, mp.y).map_or(top, |b| top.max(b + (8.0 * mon.scale_factor()) as i32));
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

/// Is the command bar on screen right now?
pub(crate) fn bar_visible(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL).is_some_and(|w| w.is_visible().unwrap_or(false))
}

/// Opens the bar when it is closed (the uninstall flow brings it back when the uninstaller is done).
pub(crate) fn show(app: &AppHandle) {
    if !bar_visible(app) {
        toggle(app);
    }
}

fn toggle(app: &AppHandle) {
    if bar_visible(app) {
        hide(app);
        return;
    }
    // The click that blurred (and hid) the bar must not re-open it.
    if lk(&LAST_BLUR_HIDE).is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
        return;
    }
    PREV_FG.store(unsafe { GetForegroundWindow() }.0 as isize, Ordering::Relaxed); // taken now: the slow open below shows the bar later
    lazy::note_shown(LABEL); // the idle teardown must not fire while the bar is being opened
    match lazy::try_get(app, LABEL) {
        Some(win) if is_ready() => show_now(app, &win),
        _ => open_slow(app), // no window yet (idle teardown), or it is still loading / being built or torn down
    }
}

/// The bar window is not there or not ready: build it right now (hidden, so the page loads while nothing is shown), wait for its page to
/// report, then show it exactly like the fast path. The work runs on a worker thread (window creation must not run on the event loop).
fn open_slow(app: &AppHandle) {
    if OPENING.swap(true, Ordering::SeqCst) {
        return; // a second press while the window is still loading
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let win = lazy::get_or_create(&app, LABEL);
        if win.is_some() && !wait_ready(READY_WAIT) {
            eprintln!("command bar: its page did not report within {} s, showing it anyway", READY_WAIT.as_secs());
        }
        let Some(win) = win else {
            OPENING.store(false, Ordering::SeqCst);
            return;
        };
        let a = app.clone();
        let sent = app.run_on_main_thread(move || {
            show_now(&a, &win);
            OPENING.store(false, Ordering::SeqCst);
        });
        if sent.is_err() {
            OPENING.store(false, Ordering::SeqCst);
        }
    });
}

/// Puts the (ready) bar on screen: place, show, bar:shown, take the keyboard.
fn show_now(app: &AppHandle, win: &WebviewWindow) {
    place(app, win);
    index_apps_async();
    keep_transparent(win);
    *lk(&SHOWN_AT) = Some(Instant::now());
    let _ = win.show();
    let _ = bus::emit(&app, Target::Label(LABEL), "bar:shown", ()); // first: the pop animation must not wait for the focus hand-over below
    grab_keyboard(app);
    // the first SetForegroundWindow can lose to the app that still owns the keyboard (the hotkey's own keys are barely released)
    let a = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        let a2 = a.clone();
        let _ = a.run_on_main_thread(move || {
            if a2.get_webview_window(LABEL).is_some_and(|w| w.is_visible().unwrap_or(false)) {
                grab_keyboard(&a2);
            }
        });
    });
}

/// Keyboard for the bar: the OS foreground (borrowing the old foreground thread's input state, like pill_activate) AND the WebView2
/// control inside the window. Without both, the layout hotkeys (Alt+Shift, Ctrl+Shift, Win+Space) keep acting on the app that was in front.
fn grab_keyboard(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else { return };
    if let Ok(h) = win.hwnd() {
        let me = HWND(h.0 as isize as *mut std::ffi::c_void);
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0 != me.0 {
                let (mine, theirs) = (GetCurrentThreadId(), GetWindowThreadProcessId(fg, None));
                let attached = theirs != 0 && theirs != mine && AttachThreadInput(mine, theirs, true).as_bool();
                let _ = SetForegroundWindow(me);
                if attached {
                    let _ = AttachThreadInput(mine, theirs, false);
                }
            }
        }
    }
    let _ = win.set_focus();
    let _ = tauri::Webview::set_focus(win.as_ref());
}

#[tauri::command]
pub fn cmd_toggle(app: AppHandle) {
    toggle(&app);
}

#[tauri::command]
pub async fn cmd_query(text: String) -> Vec<Res> {
    let my = CUR_SEQ.fetch_add(1, Ordering::Relaxed) + 1; // supersedes every query still running
    tauri::async_runtime::spawn_blocking(move || {
        MY_SEQ.with(|c| c.set(my));
        query(&text)
    })
    .await
    .unwrap_or_default()
}

/// Shell icon (PNG data URL) of an app / file / folder row. Looked up by the id of a result of the current query, never by a path from
/// the page; the page asks per row after rendering, so slow shell calls never delay the list itself.
#[tauri::command]
pub async fn cmd_icon(id: String) -> Option<String> {
    let path = {
        let g = lk(&STORE);
        g.as_ref()?.get(&id).filter(|r| matches!(r.kind, "app" | "file" | "folder")).map(|r| r.payload.clone())?
    };
    if let Some(u) = lk(&ICONS).get_or_insert_with(|| Lru::new(64)).get(&path) {
        return Some(u);
    }
    let u = tauri::async_runtime::spawn_blocking({
        let path = path.clone();
        move || crate::shelf::bar_icon(&path)
    })
    .await
    .ok()
    .flatten()?;
    lk(&ICONS).get_or_insert_with(|| Lru::new(64)).put(path, u.clone());
    Some(u)
}

// ---- query ----

fn query(text: &str) -> Vec<Res> {
    *lk(&PENDING) = None;
    let q0 = text.trim_start();
    let mut out: Vec<Res> = Vec::new();
    if q0.is_empty() {
        out = lk(&RECENT).clone();
    } else if let Some(cmd) = q0.strip_prefix('>') {
        let cmd = cmd.trim_start();
        out.push(if cmd.is_empty() {
            res("hint", "hint:shell".into(), "Type a command to run with cmd /c".into(), "Runs exactly what you type".into(), String::new())
        } else {
            res("shell", format!("shell:{cmd}"), format!("Run: {cmd}"), "cmd /c, hidden window".into(), cmd.to_string())
        });
    } else if let Some(rest) = crate::uninstall::trigger(q0).filter(|_| mod_on("module.uninstall")) {
        // `u foo` / `del foo` / `uninstall foo` / `удалить foo`: installed apps to uninstall completely (nothing runs before the confirm card)
        out = crate::uninstall::rows(rest).into_iter().map(|r| res(r.kind, r.id, r.title, r.sub, r.payload)).collect();
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
                out.extend(lk(&RECENT).iter().filter(|r| r.kind == "app").cloned());
            }
        }
        if stale() {
            return vec![];
        }
        out.extend(app_data(scope, &lw));
        // a single character matches half the disk: wait for a second one, unless the `f ` filter asked for files explicitly
        let long_enough = scope == Scope::Files || words.iter().map(|w| w.chars().count()).sum::<usize>() >= 2;
        if (all && !special || scope == Scope::Files) && !words.is_empty() && long_enough && !stale() && mod_on("module.fileSearch") {
            out.extend(files(&words));
        }
        if all && !rest.is_empty() {
            out.push(web_row(rest));
        }
        if all {
            out.sort_by_key(|r| rank(r, rest)); // stable: order inside a kind is kept
        }
    }
    if stale() {
        return vec![]; // superseded: the page drops this answer anyway, and it must not overwrite the newer rows in STORE
    }
    register(&out);
    out
}

fn match_apps(words: &[String]) -> Vec<Res> {
    let ws: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    if ws.is_empty() {
        return vec![];
    }
    let after_space = format!(" {}", ws[0]);
    let mut hits: Vec<(u8, usize, Res)> = lk(&APPS)
        .iter()
        .filter_map(|(name, target, n)| {
            if !ws.iter().all(|w| n.contains(w.as_str())) {
                return None;
            }
            let rank = if n.starts_with(ws[0].as_str()) { 0 } else if n.contains(&after_space) { 1 } else { 2 };
            let sub = if target.starts_with(APPS_PREFIX) { "App" } else { "Application" };
            Some((rank, n.len(), res("app", format!("app:{target}"), name.clone(), sub.into(), target.clone())))
        })
        .collect();
    hits.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    hits.into_iter().take(LIMIT).map(|h| h.2).collect()
}

/// One es.exe run for both kinds: the first ES_CAP hits are split into folders and files (LIMIT each, es order kept), exactly the rows the
/// two filtered runs (`/ad`, `/a-d`) used to give. Only when the cap was hit and a kind is still short does that kind get its own run.
fn files(words: &[String]) -> Vec<Res> {
    let mk = |kind: &'static str, p: String| {
        let name = Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
        res(kind, format!("{kind}:{p}"), name, p.clone(), p)
    };
    let hint = |e: &str| vec![res("hint", "hint:es".into(), e.into(), "Files and folders need Everything: Settings > File search can download it".into(), String::new())];
    let all = match es(words, None, ES_CAP) {
        Ok(v) => v,
        Err(e) => return hint(e),
    };
    let capped = all.len() >= ES_CAP;
    let (mut d, mut f): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for p in all {
        if d.len() >= LIMIT && f.len() >= LIMIT {
            break;
        }
        let v = if Path::new(&p).is_dir() { &mut d } else { &mut f };
        if v.len() < LIMIT {
            v.push(p);
        }
    }
    if capped {
        for (v, attr) in [(&mut d, "/ad"), (&mut f, "/a-d")] {
            if v.len() < LIMIT {
                match es(words, Some(attr), LIMIT) {
                    Ok(r) => *v = r,
                    Err(e) => return hint(e),
                }
            }
        }
    }
    d.into_iter().map(|p| mk("folder", p)).chain(f.into_iter().map(|p| mk("file", p))).collect()
}

// ---- Everything (es.exe) ----

// Where es.exe lives: found once (a miss is remembered for 30 s so typing on a PC without it does not re-scan PATH every keystroke).
static ES_PATH: Mutex<Option<(Option<PathBuf>, Instant)>> = Mutex::new(None);

pub(crate) fn find_es() -> Option<PathBuf> {
    let mut g = lk(&ES_PATH);
    match g.as_ref() {
        Some((Some(p), _)) if p.is_file() => return Some(p.clone()),
        Some((None, at)) if at.elapsed() < Duration::from_secs(30) => return None,
        _ => {}
    }
    let p = scan_es();
    *g = Some((p.clone(), Instant::now()));
    p
}

/// Forget a remembered miss: es.exe has just been unpacked (everything.rs).
pub(crate) fn forget_es() {
    *lk(&ES_PATH) = None;
}

fn scan_es() -> Option<PathBuf> {
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
    dirs.push(crate::everything::dir()); // the es.exe Pill downloaded itself (Settings > File search); a system-wide one wins
    dirs.into_iter().map(|d| d.join("es.exe")).find(|p| p.is_file())
}

fn es_answers(exe: &Path) -> bool {
    std::process::Command::new(exe).args(["-n", "1"]).creation_flags(NO_WINDOW).output().is_ok_and(|o| o.status.success())
}

/// The GUI exe of Everything (1.4: Everything.exe, 1.5a: Everything64.exe) in the usual install places.
pub(crate) fn find_everything_gui() -> Option<PathBuf> {
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

pub(crate) fn everything_running() -> bool {
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

const ES_CAP: usize = 64; // hard cap of hits per es.exe run
const ES_TIMEOUT: Duration = Duration::from_millis(1500);

/// Runs `cmd` with piped stdout; kills it when `cancel()` turns true or `timeout` passes. Ok((exit success, stdout)).
fn run_capped(mut cmd: std::process::Command, cancel: impl Fn() -> bool, timeout: Duration) -> Result<(bool, Vec<u8>), &'static str> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|_| "es.exe failed to start")?;
    let mut so = child.stdout.take().ok_or("es.exe failed to start")?;
    let (tx, rx) = std::sync::mpsc::channel();
    // the reader ends at EOF (process exit or kill); capped at 1 MB, the child then dies on the closed pipe
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.by_ref().take(1 << 20).read_to_end(&mut b);
        let _ = tx.send(b);
    });
    let t = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(5)) {
            Ok(b) => return Ok((child.wait().is_ok_and(|s| s.success()), b)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let why = if cancel() { Some("Search superseded") } else if t.elapsed() > timeout { Some("Everything did not answer in time") } else { None };
                if let Some(why) = why {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(why);
                }
            }
            Err(_) => return Ok((child.wait().is_ok_and(|s| s.success()), Vec::new())),
        }
    }
}

/// `es.exe -n N [attr] words...`, one full path per line.
fn es(words: &[String], attr: Option<&str>, n: usize) -> Result<Vec<String>, &'static str> {
    let exe = find_es().ok_or("Everything not found: install Everything + es.exe for file search")?;
    ensure_everything(&exe);
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["-n", &n.to_string()]).args(attr).args(words).creation_flags(NO_WINDOW);
    let (success, stdout) = run_capped(cmd, stale, ES_TIMEOUT)?;
    if !success {
        return Err("Everything is not running: start it for file search");
    }
    Ok(decode(&stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
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

/// Clears INDEXING when the indexing thread ends, also when it panics (otherwise the index would never be rebuilt).
struct IndexingDone;
impl Drop for IndexingDone {
    fn drop(&mut self) {
        INDEXING.store(false, Ordering::SeqCst);
    }
}

fn index_apps_async() {
    // opening the bar twice within 30 s does not walk the Start Menu again; typing never does
    if lk(&APPS_SCAN_AT).is_some_and(|t| t.elapsed() < Duration::from_secs(30)) && !lk(&APPS).is_empty() {
        return;
    }
    if INDEXING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _done = IndexingDone;
        let mut apps: Vec<(String, String)> = Vec::new();
        for var in ["ProgramData", "APPDATA"] {
            if let Some(v) = std::env::var_os(var) {
                walk(&Path::new(&v).join(r"Microsoft\Windows\Start Menu\Programs"), &mut apps);
            }
        }
        // Get-StartApps is slow (~1 s): refresh it at most every 10 minutes. Constant script, no user input.
        let due = lk(&APPS_STORE_AT).is_none_or(|t| t.elapsed() > Duration::from_secs(600));
        let mut store: Vec<(String, String)> = if due {
            *lk(&APPS_STORE_AT) = Some(Instant::now());
            start_apps()
        } else {
            lk(&APPS).iter().filter(|a| a.1.starts_with(APPS_PREFIX)).map(|a| (a.0.clone(), a.1.clone())).collect()
        };
        apps.sort_by_key(|a| a.0.to_lowercase()); // .lnk entries win the name dedupe below
        apps.append(&mut store);
        let mut seen = std::collections::HashSet::new();
        apps.retain(|a| seen.insert(a.0.to_lowercase()));
        // the lowercase name is made once here, so a keystroke does not lowercase every app name again
        *lk(&APPS) = apps.into_iter().map(|(n, t)| { let lc = n.to_lowercase(); (n, t, lc) }).collect();
        *lk(&APPS_SCAN_AT) = Some(Instant::now());
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
        // DirEntry::file_type comes with the listing (no stat per entry); only a link needs the stat that follows it
        let dir = e.file_type().map_or_else(|_| p.is_dir(), |t| if t.is_symlink() { p.is_dir() } else { t.is_dir() });
        if dir {
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
    let (name, base) = match settings::get_str("search.engine", "google").to_lowercase().as_str() {
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
    let mut rc = lk(&RECENT);
    rc.retain(|x| x.id != r.id);
    rc.insert(0, r.clone());
    rc.truncate(8);
}

/// Pins an existing file / shortcut / folder, or a Microsoft Store app (`shell:AppsFolder\...`, with the name shown in the bar), to the
/// island's launcher strip (the strip itself skips duplicates and bad paths).
/// "updated" keeps the bar open; the page turns the key in `message` into its own wording.
fn pin_item(app: &AppHandle, target: &str, title: &str, store_app: bool) -> Outcome {
    if !settings::get_bool("module.launcher", true) {
        return err("Turn on the Launcher module in Settings > Modules");
    }
    let added = if store_app { crate::launcher::add_store(app, target, title) } else { crate::launcher::launcher_add(app.clone(), vec![target.to_string()]) };
    Outcome { status: "updated", message: if added > 0 { "pinned" } else { "pinnedAlready" }.into() }
}

#[tauri::command]
pub fn cmd_run(app: AppHandle, id: String, mode: String) -> Outcome {
    let Some(r) = lk(&STORE).as_ref().and_then(|m| m.get(&id)).cloned() else {
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
                "pin" => pin_item(&app, &t, &r.title, store_app),
                _ if store_app && mode != "open" => err("Store apps have no file location or admin mode"),
                "folder" => match std::process::Command::new("explorer.exe").arg("/select,").arg(&t).creation_flags(NO_WINDOW).spawn() {
                    Ok(_) => ok(),
                    Err(e) => err(e.to_string()),
                },
                "path" => Outcome { status: "copy", message: t.clone() }, // the page puts the path on the clipboard and closes the bar
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

pub(crate) fn strip_html(s: &str) -> String {
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
    let mut out = Vec::new();
    // notes.rs keeps each note's stripped, lowercased text: a keystroke only filters it (no DB read of the bodies, no HTML stripping)
    for d in crate::notes::search_index() {
        if stale() {
            break;
        }
        if !lw.iter().all(|w| d.hay.contains(w.as_str())) {
            continue;
        }
        let t = if d.title.trim().is_empty() {
            let p = preview(&d.body, 40);
            if p.is_empty() { "Untitled note".into() } else { p }
        } else {
            d.title.clone()
        };
        out.push(res("note", format!("note:{}", d.id), t, snippet(&d.body, lw.first(), 80), d.id.to_string()));
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
    let mut p = lk(&PENDING);
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
    let _ = bus::emit(&app, Target::Label("pill"), "pill:open-tab", p);
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

// ---- calculator and unit conversion: command/calc.rs ----
mod calc;
use calc::{calc_row, convert};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lazy_bar_secs_setting() {
        assert_eq!(lazy_secs_from(120.0), 120); // default
        assert_eq!(lazy_secs_from(0.0), 0); // never destroy
        assert_eq!(lazy_secs_from(-3.0), 0);
        assert_eq!(lazy_secs_from(1.0), 5); // churn guard
        assert_eq!(lazy_secs_from(1e9), 86_400);
        assert_eq!(lazy_secs_from(f64::NAN), 120);
    }

    #[test]
    fn web_search_encoding() {
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

    #[test]
    fn icon_lru_is_bounded_and_recency_ordered() {
        let mut l = Lru::new(3);
        for k in ["a", "b", "c"] {
            l.put(k.into(), k.to_uppercase());
        }
        assert_eq!(l.get("a").as_deref(), Some("A")); // a is now the newest, b the oldest
        l.put("d".into(), "D".into());
        assert_eq!((l.get("b"), l.get("a").as_deref(), l.get("d").as_deref()), (None, Some("A"), Some("D")));
        l.put("a".into(), "A2".into()); // replacing does not grow it
        assert_eq!(l.v.len(), 3);
        assert_eq!(l.get("a").as_deref(), Some("A2"));
    }

    #[test]
    fn superseded_query_is_stale() {
        // MY_SEQ is per thread, so this does not disturb other tests; CUR_SEQ is only read
        let cur = CUR_SEQ.load(Ordering::Relaxed);
        MY_SEQ.with(|c| c.set(cur));
        assert!(!stale());
        MY_SEQ.with(|c| c.set(cur.wrapping_sub(1)));
        assert!(stale());
    }

    fn cmd_exe(args: &[&str]) -> std::process::Command {
        let mut c = std::process::Command::new("cmd.exe");
        c.args(args).creation_flags(NO_WINDOW);
        c
    }

    #[test]
    fn run_capped_returns_output_and_kills_on_cancel_or_timeout() {
        let (ok, out) = run_capped(cmd_exe(&["/c", "echo", "hello"]), || false, Duration::from_secs(5)).unwrap();
        assert!(ok && String::from_utf8_lossy(&out).trim() == "hello");
        let (ok, _) = run_capped(cmd_exe(&["/c", "exit", "3"]), || false, Duration::from_secs(5)).unwrap();
        assert!(!ok);
        // a 5 s child (ping -n 6) is killed as soon as the query is superseded / the timeout passes
        let slow = || {
            let mut c = std::process::Command::new("ping.exe");
            c.args(["-n", "6", "127.0.0.1"]).creation_flags(NO_WINDOW);
            c
        };
        let t = Instant::now();
        let n = std::cell::Cell::new(0);
        let r = run_capped(slow(), || { n.set(n.get() + 1); n.get() > 10 }, Duration::from_secs(30));
        assert_eq!(r.err(), Some("Search superseded"));
        assert!(t.elapsed() < Duration::from_secs(2), "cancel took {:?}", t.elapsed());
        let t = Instant::now();
        let r = run_capped(slow(), || false, Duration::from_millis(100));
        assert_eq!(r.err(), Some("Everything did not answer in time"));
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(run_capped(std::process::Command::new("no-such-binary-xyz.exe"), || false, Duration::from_secs(1)).err(), Some("es.exe failed to start"));
    }

    #[test]
    fn app_walk_finds_nested_lnk_without_stat_per_entry() {
        let root = std::env::temp_dir().join(format!("pill-walk-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Sub").join("Deep")).unwrap();
        for f in ["Alpha.lnk", "Sub/Beta.LNK", "Sub/Deep/Gamma.lnk", "Sub/Uninstall Beta.lnk", "Sub/readme.txt"] {
            std::fs::write(root.join(f), b"").unwrap();
        }
        let mut v = Vec::new();
        walk(&root, &mut v);
        let mut names: Vec<_> = v.into_iter().map(|a| a.0).collect();
        names.sort();
        assert_eq!(names, ["Alpha", "Beta", "Gamma"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn match_apps_uses_the_prebuilt_lowercase_name() {
        let app = |n: &str, t: &str| (n.to_string(), t.to_string(), n.to_lowercase());
        *lk(&APPS) = vec![app("Google Chrome", "g.lnk"), app("Chrome", "c.lnk"), app("Notepad", &format!("{APPS_PREFIX}np")), app("Хром", "h.lnk")];
        let names = |w: &str| match_apps(&[w.to_string()]).into_iter().map(|r| r.title).collect::<Vec<_>>();
        assert_eq!(names("CHR"), ["Chrome", "Google Chrome"]); // prefix match first, then shorter name
        assert_eq!(names("note"), ["Notepad"]);
        assert_eq!(names("хр"), ["Хром"]);
        assert!(match_apps(&[]).is_empty());
        *lk(&APPS) = Vec::new();
    }

    // Timing, run with --nocapture: the old two parallel es.exe runs against the single capped run. Skipped without a working Everything.
    #[test]
    fn es_single_run_timing() {
        let Some(exe) = scan_es() else { return eprintln!("es timing: es.exe not installed, skipped") };
        let run = |args: &[&str]| {
            let mut c = std::process::Command::new(&exe);
            c.args(args).arg("windows").creation_flags(NO_WINDOW);
            run_capped(c, || false, Duration::from_secs(5))
        };
        match run(&["-n", "1"]) {
            Ok((true, _)) => {}
            _ => return eprintln!("es timing: Everything is not running, skipped"),
        }
        let best = |f: &dyn Fn()| (0..5).map(|_| { let t = Instant::now(); f(); t.elapsed() }).min().unwrap();
        let two = best(&|| {
            std::thread::scope(|s| {
                let a = s.spawn(|| run(&["-n", "6", "/a-d"]));
                let b = s.spawn(|| run(&["-n", "6", "/ad"]));
                let _ = (a.join(), b.join());
            })
        });
        let one = best(&|| { let _ = run(&["-n", &ES_CAP.to_string()]); });
        eprintln!("es.exe per keystroke (best of 5): old two parallel runs {two:?}, new single run {one:?}");
    }
}
