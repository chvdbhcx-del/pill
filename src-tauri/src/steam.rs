// Steam downloads producer for Settings > Live events > Steam (GateGuard note: new file of the Pill app, used by main.rs + src/live-ext.js + src/settings-live-ext.js;
// user request: "делай все оставшееся", part: Live-events producers OBS/Steam/Notifications).
// READ-ONLY: finds Steam through HKCU\\Software\\Valve\\Steam (SteamPath, ActiveProcess\\pid), reads steamapps\\libraryfolders.vdf and every
// steamapps\\appmanifest_*.acf (StateFlags, BytesDownloaded, BytesToDownload). No network, no writes, nothing is sent anywhere.
// Runs only while live.steam is on (no switch = no thread); polls every 3 s while Steam runs, every 10 s to notice Steam starting.
// Event to window "pill": `live:steam` {state: downloading|paused|installing|idle|done, game, appid, percent, speed (B/s), etaSec}.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

const MAX_ACF: u64 = 256 * 1024;

// ---------------------------------------------------------------- pure parts (unit-tested)
#[derive(Debug, Clone, PartialEq)]
pub enum V {
    S(String),
    M(Vec<(String, V)>),
}

enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokens(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = src.chars().peekable();
    while let Some(&c) = it.peek() {
        match c {
            '{' => {
                out.push(Tok::Open);
                it.next();
            }
            '}' => {
                out.push(Tok::Close);
                it.next();
            }
            '"' => {
                it.next();
                let mut s = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(o) => s.push(o), // \\ and \" and anything else: the character itself
                            None => break,
                        },
                        c => s.push(c),
                    }
                }
                out.push(Tok::Str(s));
            }
            '/' => {
                it.next();
                if it.peek() == Some(&'/') {
                    while it.peek().is_some_and(|c| *c != '\n') {
                        it.next();
                    }
                }
            }
            c if c.is_whitespace() => {
                it.next();
            }
            _ => {
                let mut s = String::new();
                while it.peek().is_some_and(|c| !c.is_whitespace() && !matches!(c, '{' | '}' | '"')) {
                    s.push(it.next().unwrap());
                }
                out.push(Tok::Str(s));
            }
        }
    }
    out
}

fn block(t: &mut std::iter::Peekable<std::vec::IntoIter<Tok>>, depth: u32) -> Option<Vec<(String, V)>> {
    if depth > 16 {
        return None;
    }
    let mut v = Vec::new();
    loop {
        match t.next() {
            None | Some(Tok::Close) => return Some(v),
            Some(Tok::Open) => return None,
            Some(Tok::Str(k)) => match t.next()? {
                Tok::Str(s) => v.push((k, V::S(s))),
                Tok::Open => v.push((k, V::M(block(t, depth + 1)?))),
                Tok::Close => return None,
            },
        }
    }
}

/// Valve KeyValues text (appmanifest_*.acf, libraryfolders.vdf) -> key/value tree; None on malformed input.
pub fn parse_vdf(src: &str) -> Option<Vec<(String, V)>> {
    block(&mut tokens(src).into_iter().peekable(), 0)
}

fn find<'a>(m: &'a [(String, V)], key: &str) -> Option<&'a V> {
    m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

fn text<'a>(m: &'a [(String, V)], key: &str) -> Option<&'a str> {
    match find(m, key)? {
        V::S(s) => Some(s),
        _ => None,
    }
}

/// Library folders from libraryfolders.vdf: new format `"0" { "path" "D:\\Lib" ... }`, old format `"1" "D:\\Lib"`.
pub fn library_paths(src: &str) -> Vec<String> {
    let Some(root) = parse_vdf(src) else { return Vec::new() };
    let Some(V::M(libs)) = find(&root, "libraryfolders") else { return Vec::new() };
    libs.iter()
        .filter(|(k, _)| k.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|(_, v)| match v {
            V::M(m) => text(m, "path").map(str::to_string),
            V::S(s) => Some(s.clone()),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Manifest {
    pub appid: u64,
    pub name: String,
    pub flags: u32,
    pub to_download: u64,
    pub downloaded: u64,
    /// uncompressed bytes to be written to steamapps\downloading\<appid> (BytesToStage / BytesStaged); they advance after the network part
    pub to_stage: u64,
    pub staged: u64,
    /// the library (the folder that holds steamapps) this manifest came from; empty for a parsed string
    pub lib: PathBuf,
}

pub fn parse_manifest(src: &str) -> Option<Manifest> {
    let root = parse_vdf(src)?;
    let V::M(a) = find(&root, "AppState")? else { return None };
    let num = |k: &str| text(a, k).and_then(|s| s.trim().parse::<u64>().ok());
    Some(Manifest {
        appid: num("appid")?,
        name: text(a, "name").unwrap_or("").chars().filter(|c| !c.is_control()).take(80).collect(),
        flags: num("StateFlags")? as u32,
        to_download: num("BytesToDownload").unwrap_or(0),
        downloaded: num("BytesDownloaded").unwrap_or(0),
        to_stage: num("BytesToStage").unwrap_or(0),
        staged: num("BytesStaged").unwrap_or(0),
        lib: PathBuf::new(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum St {
    Downloading,
    Paused,
    Installing,
    Idle,
}

// EAppState bits (steamclientpublic.h): 2 update required, 4 fully installed, 256 update running, 512 update paused, 1024 update started,
// 524288 preallocating, 1048576 downloading, 2097152 staging, 4194304 committing (131072 validating is a file check, not a download)
const UPDATE_RUNNING: u32 = 256;
const UPDATE_PAUSED: u32 = 512;
const UPDATE_STARTED: u32 = 1024;
const PREALLOCATING: u32 = 1 << 19;
const DOWNLOADING: u32 = 1 << 20;
const STAGING: u32 = 1 << 21;
const COMMITTING: u32 = 1 << 22;
const BUSY_BITS: u32 = UPDATE_RUNNING | UPDATE_STARTED | PREALLOCATING | DOWNLOADING | STAGING | COMMITTING;

/// `live` = bytes in steamapps\downloading\<appid> right now: Steam rewrites the manifest counters only now and then, the folder grows all the time.
pub fn classify(m: &Manifest, live: u64) -> St {
    let unfinished = m.to_download > 0 && m.downloaded < m.to_download;
    if m.flags & UPDATE_PAUSED != 0 {
        return if unfinished || live > 0 { St::Paused } else { St::Idle };
    }
    if m.flags & BUSY_BITS == 0 {
        return St::Idle;
    }
    if m.flags & (STAGING | COMMITTING) != 0 {
        return St::Installing; // the bytes are here, they are being unpacked and written
    }
    if m.flags & DOWNLOADING != 0 {
        return St::Downloading; // Steam says so itself: the chip shows even before the first counted byte
    }
    if m.to_download > 0 && m.downloaded >= m.to_download {
        St::Installing
    } else if m.downloaded > 0 || live > 0 {
        St::Downloading
    } else {
        St::Idle // queued, not a byte yet: no chip
    }
}

/// Progress 0..100 of the phase `st`: the manifest counters, the downloading folder as a live second opinion (never lower than the counters).
pub fn percent(m: &Manifest, st: St, live: u64) -> u32 {
    let by = |a: u64, b: u64| if b > 0 { (a as f64 / b as f64 * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let net = by(m.downloaded, m.to_download);
    let disk = if m.to_stage > 0 { by(m.staged.max(live), m.to_stage) } else { 0.0 };
    let p = match st {
        St::Installing if m.to_stage > 0 => disk,
        St::Installing => net,
        _ => net.max(disk),
    };
    p.floor() as u32
}

/// Total size of a folder tree (read-only, capped so a pathological tree cannot stall the poll).
fn dir_size(p: &Path) -> u64 {
    let (mut total, mut stack, mut seen) = (0u64, vec![p.to_path_buf()], 0u32);
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 20_000 {
                return total;
            }
            match e.metadata() {
                Ok(md) if md.is_dir() => stack.push(e.path()),
                Ok(md) => total += md.len(),
                Err(_) => {}
            }
        }
    }
    total
}

// ---------------------------------------------------------------- scanning (read-only)
fn reg_str(sub: PCWSTR, name: PCWSTR) -> Option<String> {
    let mut buf = [0u16; 520];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, sub, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len)) };
    if r.0 != 0 {
        return None;
    }
    let n = (len as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim_end_matches('\0').trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn steam_dir() -> Option<PathBuf> {
    reg_str(w!("Software\\Valve\\Steam"), w!("SteamPath")).map(|s| PathBuf::from(s.replace('/', "\\"))).filter(|p| p.is_dir())
}

fn steam_running() -> bool {
    let mut pid = 0u32;
    let mut len = 4u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Valve\\Steam\\ActiveProcess"), w!("pid"), RRF_RT_REG_DWORD, None, Some(&mut pid as *mut u32 as *mut _), Some(&mut len)) };
    if r.0 != 0 || pid == 0 {
        return false;
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(h, &mut code).is_ok() && code == 259; // STILL_ACTIVE
        let _ = CloseHandle(h);
        alive
    }
}

fn read_small(p: &Path) -> Option<String> {
    (std::fs::metadata(p).ok()?.len() <= MAX_ACF).then(|| std::fs::read_to_string(p).ok()).flatten()
}

fn manifests(steam: &Path) -> Vec<Manifest> {
    let mut libs = vec![steam.to_path_buf()];
    let list = read_small(&steam.join("steamapps").join("libraryfolders.vdf")).or_else(|| read_small(&steam.join("config").join("libraryfolders.vdf")));
    for l in list.map(|t| library_paths(&t)).unwrap_or_default() {
        let p = PathBuf::from(l);
        if !libs.iter().any(|x| x.to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())) {
            libs.push(p);
        }
    }
    // ponytail: every acf is re-read each poll (a few hundred tiny files per 3 s at worst); cache by mtime if that ever shows up in a profile
    let mut out = Vec::new();
    for lib in libs {
        let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            if n.starts_with("appmanifest_") && n.ends_with(".acf") {
                if let Some(mut m) = read_small(&e.path()).and_then(|t| parse_manifest(&t)) {
                    m.lib = lib.clone();
                    out.push(m);
                }
            }
        }
    }
    out.sort_by_key(|m| m.appid);
    out
}

// ---------------------------------------------------------------- worker
static GEN: AtomicU64 = AtomicU64::new(0);
static ACTIVE: Mutex<bool> = Mutex::new(false);

fn sync(app: &AppHandle) {
    let want = crate::settings::get_bool("live.steam", true);
    let mut a = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if *a == want {
        return;
    }
    *a = want;
    let g = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    if want {
        let app = app.clone();
        std::thread::spawn(move || run(app, g));
    } else {
        let _ = app.emit_to("pill", "live:steam", json!({ "state": "idle" }));
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let h2 = h.clone();
    h.listen("settings:changed", move |_| sync(&h2));
    sync(&h);
}

struct Seen {
    active: bool,
    dl: u64,
    live: u64,
}

fn nap(gen: u64, ms: u64) -> bool {
    let mut left = ms / 250;
    while left > 0 {
        if GEN.load(Ordering::SeqCst) != gen {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
        left -= 1;
    }
    GEN.load(Ordering::SeqCst) == gen
}

fn run(app: AppHandle, gen: u64) {
    let mut seen: HashMap<u64, Seen> = HashMap::new();
    let (mut last, mut shown, mut speed, mut prev_at) = (Value::Null, false, 0.0f64, Instant::now());
    let mut primary = 0u64;
    let mut pmax: (u64, St, u32) = (0, St::Idle, 0);
    loop {
        if GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let steam = steam_dir().filter(|_| steam_running());
        let Some(steam) = steam else {
            if shown {
                shown = false;
                last = Value::Null;
                let _ = app.emit_to("pill", "live:steam", json!({ "state": "idle" }));
            }
            seen.clear();
            if !nap(gen, 10_000) {
                return;
            }
            continue;
        };
        let now = manifests(&steam);
        // the downloading folder is only looked at for apps that Steam marks as busy (a handful of stat calls per poll)
        let lives: HashMap<u64, u64> = now
            .iter()
            .filter(|m| m.flags & (BUSY_BITS | UPDATE_PAUSED) != 0)
            .map(|m| (m.appid, dir_size(&m.lib.join("steamapps").join("downloading").join(m.appid.to_string()))))
            .collect();
        let live_of = |id: u64| lives.get(&id).copied().unwrap_or(0);
        let dt = prev_at.elapsed().as_secs_f64().max(0.5);
        prev_at = Instant::now();
        let mut ev: Option<Value> = None;
        let mut best: Option<(bool, &Manifest, St)> = None;
        for m in &now {
            let st = classify(m, live_of(m.appid));
            let prev = seen.get(&m.appid);
            let grew = prev.is_some_and(|p| m.downloaded > p.dl || live_of(m.appid) > p.live);
            if prev.is_some_and(|p| p.active) && st == St::Idle && m.flags & 4 != 0 && ev.is_none() {
                ev = Some(json!({ "state": "done", "game": m.name, "appid": m.appid.to_string(), "percent": 100 }));
            }
            if st != St::Idle && best.as_ref().map_or(true, |b| grew && !b.0) {
                best = Some((grew, m, st));
            }
        }
        let cur = best.map(|(_, m, st)| (m, st));
        if let Some((m, _)) = cur {
            if m.appid != primary {
                primary = m.appid;
                speed = 0.0;
            }
            let (prev_dl, prev_live) = seen.get(&m.appid).map_or((m.downloaded, live_of(m.appid)), |p| (p.dl, p.live));
            // the counters when Steam moved them, else the growth of the downloading folder
            let delta = m.downloaded.saturating_sub(prev_dl).max(live_of(m.appid).saturating_sub(prev_live)) as f64;
            speed = speed * 0.5 + delta / dt * 0.5;
        }
        seen = now.iter().map(|m| (m.appid, Seen { active: matches!(classify(m, live_of(m.appid)), St::Downloading | St::Installing), dl: m.downloaded, live: live_of(m.appid) })).collect();
        if let Some(done) = ev {
            let _ = app.emit_to("pill", "live:steam", done);
        }
        let p = match cur {
            Some((m, st)) => {
                let live = live_of(m.appid);
                // the figure never goes backwards inside one phase of one app (the counters and the folder are two noisy readings)
                let raw = percent(m, st, live);
                let pct = if pmax.0 == m.appid && pmax.1 == st { raw.max(pmax.2) } else { raw };
                pmax = (m.appid, st, pct);
                let left = if m.to_stage > 0 && live > 0 { m.to_stage.saturating_sub(live) } else { m.to_download.saturating_sub(m.downloaded) };
                let eta = if st == St::Downloading && speed > 1024.0 { (left as f64 / speed) as u64 } else { 0 };
                let state = match st {
                    St::Downloading => "downloading",
                    St::Paused => "paused",
                    _ => "installing",
                };
                json!({ "state": state, "game": m.name, "appid": m.appid.to_string(), "percent": pct, "speed": speed.round() as u64, "etaSec": eta })
            }
            None => json!({ "state": "idle" }),
        };
        if p != last && (cur.is_some() || shown) {
            shown = cur.is_some();
            let _ = app.emit_to("pill", "live:steam", p.clone());
        }
        last = p;
        if !nap(gen, 3_000) {
            return;
        }
    }
}

// ---------------------------------------------------------------- command
#[tauri::command]
pub fn steam_status() -> Value {
    let dir = steam_dir();
    json!({ "installed": dir.is_some(), "running": dir.is_some() && steam_running() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACF: &str = "\"AppState\"\n{\n\t\"appid\"\t\t\"570\"\n\t\"name\"\t\t\"Dota \\\"2\\\"\"\n\t\"StateFlags\"\t\t\"1026\"\n\t// c\n\t\"BytesToDownload\"\t\t\"2000\"\n\t\"BytesDownloaded\"\t\t\"500\"\n\t\"UserConfig\"\n\t{\n\t\t\"language\"\t\t\"english\"\n\t}\n}\n";

    #[test]
    fn parses_acf() {
        let m = parse_manifest(ACF).unwrap();
        assert_eq!((m.appid, m.flags, m.to_download, m.downloaded), (570, 1026, 2000, 500));
        assert_eq!(m.name, "Dota \"2\"");
        assert_eq!(classify(&m, 0), St::Downloading);
    }

    #[test]
    fn classifies_states() {
        let mk = |flags, to, dl| Manifest { appid: 1, flags, to_download: to, downloaded: dl, ..Default::default() };
        assert_eq!(classify(&mk(4, 0, 0), 0), St::Idle);
        assert_eq!(classify(&mk(1026, 100, 0), 0), St::Idle); // queued
        assert_eq!(classify(&mk(1026, 100, 100), 0), St::Installing);
        assert_eq!(classify(&mk(1538, 100, 40), 0), St::Paused);
        assert_eq!(classify(&mk(4, 100, 100), 0), St::Idle); // finished, stale byte counters
    }

    #[test]
    fn phases_and_the_live_folder() {
        let mk = |flags, to, dl| Manifest { appid: 1, flags, to_download: to, downloaded: dl, to_stage: 1000, staged: 0, ..Default::default() };
        // Steam's own phase bits win: downloading shows the chip at 0 bytes, staging / committing are the installing phase
        assert_eq!(classify(&mk(4 | (1 << 20), 100, 0), 0), St::Downloading);
        assert_eq!(classify(&mk(4 | (1 << 21), 100, 100), 0), St::Installing);
        assert_eq!(classify(&mk(4 | (1 << 22), 100, 100), 0), St::Installing);
        // validating files is not a download
        assert_eq!(classify(&mk(4 | (1 << 17), 0, 0), 0), St::Idle);
        // a stale manifest (0 bytes counted) but the downloading folder grows: the chip appears
        assert_eq!(classify(&mk(1026, 100, 0), 5_000), St::Downloading);
        // paused with something on disk stays a paused chip
        assert_eq!(classify(&mk(1538, 0, 0), 5_000), St::Paused);
    }

    #[test]
    fn percent_uses_the_better_reading_and_never_exceeds_100() {
        let m = Manifest { appid: 1, to_download: 200, downloaded: 0, to_stage: 1000, staged: 0, ..Default::default() };
        assert_eq!(percent(&m, St::Downloading, 0), 0);
        assert_eq!(percent(&m, St::Downloading, 250), 25); // counters stale, the folder says 25 %
        let m2 = Manifest { downloaded: 100, ..m.clone() };
        assert_eq!(percent(&m2, St::Downloading, 250), 50); // the counters are ahead
        assert_eq!(percent(&m2, St::Installing, 400), 40); // unpacking is measured against BytesToStage
        assert_eq!(percent(&Manifest { to_stage: 0, ..m2.clone() }, St::Installing, 400), 50); // no stage counter: fall back to the network bytes
        assert_eq!(percent(&m2, St::Downloading, 10_000), 100);
    }

    #[test]
    fn libraries_both_formats_and_garbage() {
        let new = "\"libraryfolders\" { \"0\" { \"path\" \"C:\\\\Program Files (x86)\\\\Steam\" \"apps\" { \"570\" \"1\" } } \"1\" { \"path\" \"D:\\\\Games\" } \"contentstatsid\" \"123\" }";
        assert_eq!(library_paths(new), vec!["C:\\Program Files (x86)\\Steam", "D:\\Games"]);
        assert_eq!(library_paths("\"LibraryFolders\" { \"TimeNextStatsReport\" \"1\" \"1\" \"E:\\\\Lib\" }"), vec!["E:\\Lib"]);
        assert!(library_paths("{ { {").is_empty());
        assert!(parse_manifest("\"AppState\" { \"appid\" ").is_none());
    }
}
