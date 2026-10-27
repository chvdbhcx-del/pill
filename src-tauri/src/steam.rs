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
use std::time::Instant;
use tauri::{AppHandle, Emitter, Listener};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows::Win32::System::SystemInformation::GetLocalTime;
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

/// Progress 0..100 of the phase `st`, from the manifest counters only (the folder size is no progress: an update's downloading folder already
/// holds copies of the unchanged files). While both run, Steam's own bottom bar follows the file update ("Обновление"), so once bytes are staged so do we.
pub fn percent(m: &Manifest, st: St) -> u32 {
    let by = |a: u64, b: u64| if b > 0 { (a as f64 / b as f64 * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let net = by(m.downloaded, m.to_download);
    let disk = by(m.staged, m.to_stage);
    let p = match st {
        St::Installing if m.to_stage > 0 => disk,
        St::Installing => net,
        _ if m.staged > 0 => disk,
        _ => net,
    };
    p.floor() as u32
}

/// What Steam writes to logs\content_log.txt while it downloads. A big download leaves the manifest counters at 0 for hours and the downloading
/// folder is preallocated to its full size, so the log is the only live source. Verified on a real log: `AppID N update started : download X/Y`
/// opens a session (X = the counter at that moment, a resume starts from the bytes already got); `stats: (Invalid, 0) : B Bytes, S sec` is the
/// traffic of the LAST S seconds (a ~5.5 min window, NOT a session total; windows run across pauses); `Current download rate` comes every ~61 s.
#[derive(Debug, Default, PartialEq)]
pub struct LogInfo {
    /// BytesDownloaded when this update session started
    pub base: u64,
    /// bytes received since then, extrapolated to `now`
    pub received: u64,
    /// bytes per second; 0 when the download is stalled or cancelled
    pub rate: f64,
}

/// Rate lines older than this (relative to `now`) mean a stall: the estimate stops growing instead of extrapolating across a pause.
const RATE_STALE_S: i64 = 150;

/// Seconds since 1970-01-01 of a civil date (Hinnant's days_from_civil) * 86400.
fn epoch_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// `[2026-10-27 00:33:36] ...` -> seconds on a local wall-clock scale (no time zone: log and `now` are both local; a DST switch skews it for 1 h).
pub fn line_time(l: &str) -> Option<i64> {
    let b = l.as_bytes();
    if b.len() < 21 || b[0] != b'[' || b[20] != b']' {
        return None;
    }
    let n = |a: usize, z: usize| l.get(a..z)?.parse::<i64>().ok();
    Some(epoch_days(n(1, 5)?, n(6, 8)?, n(9, 11)?) * 86_400 + n(12, 14)? * 3600 + n(15, 17)? * 60 + n(18, 20)?)
}

fn local_now() -> i64 {
    let t = unsafe { GetLocalTime() };
    epoch_days(t.wYear as i64, t.wMonth as i64, t.wDay as i64) * 86_400 + t.wHour as i64 * 3600 + t.wMinute as i64 * 60 + t.wSecond as i64
}

/// Bytes of app `appid` received in its current update session, from the log tail, at local time `now`. None = no session start in the tail.
/// received = sum of the stats windows since the start + the rate lines after the last window + the last rate up to `now` (not past a stall).
pub fn log_info(tail: &str, appid: u64, now: i64) -> Option<LogInfo> {
    let lines: Vec<(i64, &str)> = tail.lines().filter_map(|l| Some((line_time(l)?, l))).collect();
    let key = format!("AppID {appid} update started : download ");
    let si = lines.iter().rposition(|(_, l)| l.contains(&key))?;
    let (s, sl) = lines[si];
    let base = sl.split(&key).nth(1)?.split('/').next()?.trim().parse::<u64>().ok()?;
    let cancel_key = format!("AppID {appid} update canceled");
    let after = &lines[si + 1..];
    let ci = after.iter().position(|(_, l)| l.contains(&cancel_key));
    let cancel = ci.map(|i| after[i].0);
    let after = &after[..ci.unwrap_or(after.len())];
    let mbps = |l: &str| l.split("Current download rate:").nth(1)?.split_whitespace().next()?.parse::<f64>().ok().map(|v| v * 125_000.0);
    let window = |l: &str| {
        let mut it = l.split("stats: (Invalid, 0) :").nth(1)?.split_whitespace();
        let bytes = it.next()?.parse::<u64>().ok()?;
        Some((bytes as f64, it.nth(1)?.parse::<i64>().ok()?))
    };
    // a window that began before the session start holds the old session's bytes too, but only when something was downloading then (a rate line)
    let busy_before = |from: i64| lines[..si].iter().any(|(t, l)| *t >= from && *t < s && mbps(l).is_some());
    let (mut got, mut at) = (0.0f64, s);
    for (t, l) in after {
        if let Some((b, secs)) = window(l) {
            let start = t - secs.max(1);
            got += if start < s && busy_before(start) { b * (*t - s) as f64 / secs as f64 } else { b };
            at = *t;
        }
    }
    let (mut prev, mut last, mut lt) = (at, 0.0f64, s);
    for (t, l) in after {
        if let Some(r) = mbps(l) {
            if *t > at {
                // the first interval of a session is the ramp-up: half the rate
                got += r * (*t - prev) as f64 * if prev == s { 0.5 } else { 1.0 };
                prev = *t;
            }
            last = r;
            lt = *t;
        }
    }
    let live = cancel.is_none() && now - lt <= RATE_STALE_S;
    let end = cancel.unwrap_or(now).min(lt + RATE_STALE_S);
    got += last * (end - prev).max(0) as f64;
    Some(LogInfo { base, received: got.round() as u64, rate: if live { last } else { 0.0 } })
}

/// BytesDownloaded, or the log estimate while the counters still sit at the session start (stale); counters that moved (a small game) are Steam's own.
pub fn downloaded_with_log(m: &Manifest, log: Option<&LogInfo>) -> u64 {
    match log {
        Some(l) if m.downloaded <= l.base => (l.base + l.received).min(m.to_download).max(m.downloaded),
        _ => m.downloaded,
    }
}

/// The last `max` bytes of a file (read-only; Steam keeps writing to it).
fn read_tail(p: &Path, max: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(p).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(max))).ok()?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// content_log.txt as text, re-read only when its length changed (Steam appends ~ once a minute; polls come every 3 s). The previous file, if Steam
/// rotated the log, is put in front so a session that began before the rotation is still found.
// ponytail: `content_log.previous.txt` follows Steam's `*.previous.txt` naming (seen for gameoverlay_ui only; no rotated content_log on this PC to verify)
fn log_text<'a>(steam: &Path, cache: &'a mut (u64, String)) -> &'a str {
    let dir = steam.join("logs");
    let len = std::fs::metadata(dir.join("content_log.txt")).map_or(0, |m| m.len());
    if len != cache.0 {
        let cur = read_tail(&dir.join("content_log.txt"), 1 << 20).unwrap_or_default();
        let old = if len < 1 << 20 { read_tail(&dir.join("content_log.previous.txt"), 1 << 20).unwrap_or_default() } else { String::new() };
        *cache = (len, old + &cur);
    }
    &cache.1
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
    let g = crate::wake::bump(&GEN);
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
    crate::wake::nap(&GEN, gen, ms)
}

fn run(app: AppHandle, gen: u64) {
    let mut seen: HashMap<u64, Seen> = HashMap::new();
    let (mut last, mut shown, mut speed, mut prev_at) = (Value::Null, false, 0.0f64, Instant::now());
    let mut primary = 0u64;
    let mut sent = Instant::now();
    let mut pmax: (u64, St, u32) = (0, St::Idle, 0);
    let mut logc: (u64, String) = (0, String::new());
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
        // a big download leaves the manifest counters at 0: Steam's own log has the bytes received and the live rate
        let log = match cur {
            Some((m, St::Downloading | St::Paused)) => log_info(log_text(&steam, &mut logc), m.appid, local_now()),
            _ => None,
        };
        if let Some((m, _)) = cur {
            if m.appid != primary {
                primary = m.appid;
                speed = 0.0;
            }
            let (prev_dl, prev_live) = seen.get(&m.appid).map_or((m.downloaded, live_of(m.appid)), |p| (p.dl, p.live));
            // the counters when Steam moved them, else the growth of the downloading folder, else the rate from the log
            let delta = m.downloaded.saturating_sub(prev_dl).max(live_of(m.appid).saturating_sub(prev_live)) as f64;
            speed = speed * 0.5 + delta / dt * 0.5;
            if delta == 0.0 {
                if let Some(l) = &log {
                    speed = l.rate;
                }
            }
        }
        seen = now.iter().map(|m| (m.appid, Seen { active: matches!(classify(m, live_of(m.appid)), St::Downloading | St::Installing), dl: m.downloaded, live: live_of(m.appid) })).collect();
        if let Some(done) = ev {
            let _ = app.emit_to("pill", "live:steam", done);
        }
        let p = match cur {
            Some((m, st)) => {
                let mut m = m.clone();
                m.downloaded = downloaded_with_log(&m, log.as_ref());
                let m = &m;
                // the figure never goes backwards inside one phase of one app (Steam rewrites the counters now and then); pausing is the same phase
                let raw = percent(m, st);
                let same = |a: St, b: St| (a == St::Installing) == (b == St::Installing);
                let pct = if pmax.0 == m.appid && same(pmax.1, st) { raw.max(pmax.2) } else { raw };
                pmax = (m.appid, st, pct);
                let left = m.to_download.saturating_sub(m.downloaded);
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
        // re-sent every 10 s even when unchanged: the page may have loaded after the first emit (island restarted mid-download)
        if (p != last || sent.elapsed() > std::time::Duration::from_secs(10)) && (cur.is_some() || shown) {
            shown = cur.is_some();
            sent = Instant::now();
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
    fn percent_follows_the_manifest_counters_like_steam() {
        let m = Manifest { appid: 1, to_download: 200, downloaded: 0, to_stage: 1000, staged: 0, ..Default::default() };
        assert_eq!(percent(&m, St::Downloading), 0);
        let m2 = Manifest { downloaded: 156, staged: 200, ..m.clone() };
        assert_eq!(percent(&m2, St::Downloading), 20); // Steamworks case: 78 % of the data, 20 % of the files -> the bar Steam shows at the bottom
        assert_eq!(percent(&Manifest { staged: 0, ..m2.clone() }, St::Downloading), 78); // nothing staged yet: the network bytes
        assert_eq!(percent(&m2, St::Installing), 20); // unpacking is measured against BytesToStage
        assert_eq!(percent(&Manifest { to_stage: 0, staged: 0, ..m2.clone() }, St::Installing), 78); // no stage counter: the network bytes
        assert_eq!(percent(&Manifest { staged: 10_000, ..m2.clone() }, St::Downloading), 100);
    }

    // real excerpt of Steam's content_log.txt (War Thunder, 2026-10-27), the cache lines of the stats block dropped
    const WT: &str = "[2026-10-27 00:28:53] Current download rate: 83.915 Mbps
[2026-10-27 00:30:17] AppID 236390 update started : download 0/90036629856, store 0/0, reuse 0/0, delta 0/0, stage 0/93104081798
[2026-10-27 00:30:17] Downloading 2215 chunks for depot 236394 (444186442078958183)
[2026-10-27 00:30:25] Current download rate: 60.106 Mbps
[2026-10-27 00:31:26] Current download rate: 94.429 Mbps
[2026-10-27 00:32:12] Downloading 94679 chunks for depot 236391 (8675133900244270658)
[2026-10-27 00:32:27] Current download rate: 93.637 Mbps
[2026-10-27 00:33:28] Current download rate: 95.026 Mbps
[2026-10-27 00:33:36] stats: (SteamCache, 55) cache1-iev-datl.steamcontent.com: 680775947 Bytes, 245 sec (22.22 Mbps). 1620 Hits / 1 Misses (100 %, 100 % bytes), score 0.8082
[2026-10-27 00:33:36] stats: (Invalid, 0) : 2887800176 Bytes, 246 sec (93.73 Mbps). 7587 Hits / 14 Misses (100 %, 100 % bytes), score 0.0000
[2026-10-27 00:34:29] Current download rate: 94.961 Mbps
[2026-10-27 00:35:30] Current download rate: 94.854 Mbps
[2026-10-27 00:36:30] Current download rate: 94.337 Mbps
";
    const T: i64 = 86_400 * 20_753; // 2026-10-27 00:00:00
    const MB: f64 = 125_000.0;

    #[test]
    fn log_timestamps() {
        assert_eq!(line_time("[2026-10-27 00:33:36] x"), Some(T + 33 * 60 + 36));
        assert_eq!(line_time("[2026-10-28 01:00:00] x"), Some(T + 86_400 + 3600));
        assert_eq!(line_time("[1970-01-01 00:00:01] x"), Some(1));
        assert_eq!(line_time("[2024-03-01 00:00:00] x").zip(line_time("[2024-02-28 00:00:00] x")).map(|(a, b)| a - b), Some(2 * 86_400)); // leap year
        assert_eq!(line_time("HTTPS (SteamCache,55) - cache1"), None);
        assert_eq!(line_time("[garbage-garbage-garb] x"), None);
        assert!(local_now() > T); // the Win32 clock is on the same scale
    }

    #[test]
    fn log_extrapolates_from_stats_and_rate_lines() {
        // 10 s after the last rate line: window total + 3 rate intervals + 10 s at the last rate
        let now = T + 36 * 60 + 40;
        let i = log_info(WT, 236390, now).unwrap();
        assert_eq!(i.base, 0);
        let want = 2_887_800_176.0 + 94.961 * MB * 53.0 + 94.854 * MB * 61.0 + 94.337 * MB * 60.0 + 94.337 * MB * 10.0;
        assert!((i.received as f64 - want).abs() < 1.0, "{} vs {}", i.received, want);
        assert_eq!(i.rate, 94.337 * MB);
        // a 90.0 GB download: ~ 4.5 %, the window total alone (what the old code showed) would be 3.2 %
        assert!(i.received as f64 / 90_036_629_856.0 > 0.044);
        // another app's session is not ours
        assert_eq!(log_info(WT, 730, now), None);
        assert_eq!(log_info("", 236390, now), None);
    }

    #[test]
    fn log_before_the_first_stats_window_integrates_the_rates() {
        let head: String = WT.lines().take(5).map(|l| format!("{l}\n")).collect(); // up to the 00:31:26 rate line
        let i = log_info(&head, 236390, T + 31 * 60 + 36).unwrap();
        // ramp-up interval at half rate, then 61 s at 94.429, then 10 s extrapolated
        let want = 60.106 * MB * 8.0 * 0.5 + 94.429 * MB * 61.0 + 94.429 * MB * 10.0;
        assert!((i.received as f64 - want).abs() < 1.0, "{} vs {}", i.received, want);
        // nothing since the session start, not even a rate line yet
        let none = log_info(&head.lines().take(3).collect::<Vec<_>>().join("\n"), 236390, T + 30 * 60 + 20).unwrap();
        assert_eq!(none.received, 0);
    }

    #[test]
    fn log_stall_does_not_extrapolate_across_a_pause() {
        let at_rate = log_info(WT, 236390, T + 36 * 60 + 30).unwrap();
        let later = log_info(WT, 236390, T + 40 * 60).unwrap(); // 210 s after the last rate line
        assert_eq!(later.rate, 0.0);
        // grows by 150 s of the last rate at most (the stale horizon), then stops
        assert!((later.received as f64 - at_rate.received as f64 - 94.337 * MB * 150.0).abs() < 1.0);
        assert_eq!(log_info(WT, 236390, T + 90 * 60).unwrap().received, later.received);
    }

    #[test]
    fn log_cancel_pauses_and_resume_starts_from_the_manifest_counter() {
        // a real pause of another game: cancelled at 00:28:46, the trailing rate line is not its own
        let log = "[2026-10-27 00:22:35] AppID 2073850 update started : download 2969063152/55120902704, store 0/0, reuse 0/0, delta 0/0, stage 3052596112/96722615316
[2026-10-27 00:27:06] AppID 2073850 update canceled : Disabled (Suspended)
[2026-10-27 00:27:08] AppID 2073850 update started : download 6151959664/55120902704, store 0/0, reuse 0/0, delta 0/0, stage 6241894568/96722615316
[2026-10-27 00:27:08] Downloading 86956 chunks for depot 2073851 (4263762589855354033)
[2026-10-27 00:27:52] Current download rate: 94.905 Mbps
[2026-10-27 00:28:06] stats: (Invalid, 0) : 3891505440 Bytes, 331 sec (94.19 Mbps). 3822 Hits / 11 Misses (100 %, 100 % bytes), score 0.0000
[2026-10-27 00:28:46] AppID 2073850 update canceled : Disabled (Suspended)
[2026-10-27 00:28:53] Current download rate: 83.915 Mbps
";
        let now = T + 29 * 60;
        let i = log_info(log, 2073850, now).unwrap();
        assert_eq!(i.base, 6_151_959_664); // the last session start wins
        assert_eq!(i.rate, 0.0);
        // no traffic before the start inside the window (this excerpt has no earlier rate line): the whole window counts; then 40 s at the last rate
        // until the cancel (the rate line is 14 s old at the cancel, the stale horizon is not reached)
        let want = 3_891_505_440.0 + 94.905 * MB * 40.0;
        assert!((i.received as f64 - want).abs() < 1.0, "{} vs {}", i.received, want);
        // with a rate line of the old session inside the window only its share after the start counts: 58/331 ~ 0.68 GB (real: the next
        // resume counter of the old session says 0.71 GB)
        let busy = format!("[2026-10-27 00:25:00] Current download rate: 94.0 Mbps\n{log}");
        let j = log_info(&busy, 2073850, now).unwrap();
        let share = 3_891_505_440.0 * 58.0 / 331.0;
        assert!((j.received as f64 - share - 94.905 * MB * 40.0).abs() < 1.0, "{}", j.received);
    }

    #[test]
    fn stale_counters_are_replaced_by_the_estimate_but_live_ones_are_kept() {
        let est = |m: &Manifest, l: &LogInfo| downloaded_with_log(m, Some(l));
        let l = LogInfo { base: 0, received: 31_000_000_000, rate: 1.0 };
        let big = Manifest { appid: 236390, to_download: 90_036_629_856, to_stage: 93_104_081_798, ..Default::default() };
        let p = |d| percent(&Manifest { downloaded: d, ..big.clone() }, St::Downloading);
        assert_eq!(p(est(&big, &l)), 34); // 31.0 / 90.04 GB
        let small = Manifest { downloaded: 400, to_download: 1000, ..Default::default() };
        assert_eq!(est(&small, &l), 400); // moved past the session start: Steam's own number
        assert_eq!(est(&Manifest { to_download: 20_000_000_000, ..big.clone() }, &l), 20_000_000_000); // never above BytesToDownload
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
